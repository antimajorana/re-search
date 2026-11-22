//! Isolated Side-Talk assistant (F13).
//!
//! Reads the base session's proof-graph entries and ingested source list as
//! read-only context, and appends turns exclusively to SQLite session key
//! `format!("sidetalk:{base_session}")` without ever mutating `base_session`.

use crate::guardrails::materialize_proof_entries;
use crate::runner::EVENT_VERSION;
use crate::transport::{ContentBlock, Message, ModelRequest, Role, Transport};
use ledger::Ledger;
use research_tools::{read_proof_dag, validate_session_id, ResearchEnv, SandboxRoot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const SIDETALK_MAX_TOKENS: u32 = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideTalkTurn {
    pub seq: u64,
    pub ts_ms: i64,
    pub question: String,
    pub answer: String,
    pub references: Vec<String>,
    pub raw_blob: Option<String>,
}

pub fn sidetalk_session_key(base_session: &str) -> Result<String, String> {
    validate_session_id(base_session).map_err(|e| e.to_string())?;
    Ok(format!("sidetalk:{base_session}"))
}

pub fn read_sidetalk_thread(ledger: &Ledger, base_session: &str) -> Result<Vec<SideTalkTurn>, String> {
    let st_key = sidetalk_session_key(base_session)?;
    let events = ledger
        .read_session(&st_key)
        .map_err(|e| format!("failed to read sidetalk thread `{st_key}`: {e}"))?;
    let mut turns = Vec::new();
    for ev in events {
        if ev.kind != "sidetalk.turn" {
            continue;
        }
        let question = ev
            .payload
            .get("question")
            .or_else(|| ev.payload.get("q"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let answer = ev
            .payload
            .get("answer")
            .or_else(|| ev.payload.get("a"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let references = ev
            .payload
            .get("references")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let raw_blob = ev
            .payload
            .get("raw_blob")
            .and_then(Value::as_str)
            .map(str::to_string);
        turns.push(SideTalkTurn {
            seq: ev.seq,
            ts_ms: ev.ts_ms,
            question,
            answer,
            references,
            raw_blob,
        });
    }
    Ok(turns)
}

pub fn ask_sidetalk(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    base_session: &str,
    question: &str,
    model: &str,
) -> Result<SideTalkTurn, String> {
    let st_key = sidetalk_session_key(base_session)?;
    if question.trim().is_empty() {
        return Err("side-talk question must not be empty".into());
    }
    if model.trim().is_empty() {
        return Err("side-talk model name must not be empty".into());
    }

    let base_events = ledger
        .read_session(base_session)
        .map_err(|e| format!("failed to read base session `{base_session}`: {e}"))?;
    if base_events.is_empty() {
        return Err(format!("unknown base session `{base_session}`"));
    }

    let problem_prompt = base_events
        .iter()
        .find(|e| e.kind == "session.start")
        .and_then(|e| e.payload.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or("(no prompt)");
    let dag = read_proof_dag(ledger, base_session)?;
    let entries = if !dag.entries.is_empty() {
        dag.entries
    } else {
        materialize_proof_entries(&base_events)
    };
    let entry_ids: Vec<String> = entries.iter().map(|e| e.entry_id.clone()).collect();
    let entry_lines: Vec<String> = entries
        .iter()
        .map(|e| format!("- {} ({}, {:?}): {}", e.entry_id, e.entry_type, e.status, e.summary))
        .collect();

    let sources_list = list_session_sources(env, base_session);
    let context_block = format!(
        "Read-only context for session `{base_session}`:\nProblem: {problem_prompt}\n\n\
         Proof entries:\n{}\n\nIngested sources:\n{}",
        if entry_lines.is_empty() {
            "(none)".to_string()
        } else {
            entry_lines.join("\n")
        },
        if sources_list.is_empty() {
            "(none)".to_string()
        } else {
            sources_list.join("\n")
        }
    );

    let prior_turns = read_sidetalk_thread(ledger, base_session)?;
    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: context_block }],
    }];
    if !prior_turns.is_empty() {
        let mut hist = String::from("Prior side-talk Q&A:\n");
        for t in &prior_turns {
            hist.push_str(&format!("Q: {}\nA: {}\n\n", t.question, t.answer));
        }
        messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: "Understood, ready for follow-up side-talk questions.".into() }],
        });
        messages.push(Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: format!("{hist}New question: {question}") }],
        });
    } else {
        messages[0].content.push(ContentBlock::Text {
            text: format!("\n\nQuestion: {question}"),
        });
    }

    let system_prompt = "You are an isolated Side-Talk research assistant (sidetalk). \
        Answer the user's question clearly using the read-only proof entries and sources, \
        citing relevant step IDs (e.g. e-1) or source paths without modifying the main derivation.";
    let req = ModelRequest {
        model: model.to_string(),
        system: Some(system_prompt.to_string()),
        messages,
        tools: Vec::new(),
        max_tokens: SIDETALK_MAX_TOKENS,
    };
    let prepared = transport.prepare_request(&req).map_err(|e| e.to_string())?;
    let req_blob = ledger
        .put_blob(&prepared.body)
        .map_err(|e| format!("sidetalk request put_blob failed: {e}"))?;
    ledger
        .append_with_hash(
            &st_key,
            "sidetalk.request",
            EVENT_VERSION,
            &json!({ "base_session": base_session, "model": model, "raw_blob": req_blob }),
        )
        .map_err(|e| format!("failed to record sidetalk.request: {e}"))?;

    let resp = transport.send_prepared(&prepared).map_err(|e| e.to_string())?;
    let resp_blob = ledger
        .put_blob(&resp.raw_body)
        .map_err(|e| format!("sidetalk response put_blob failed: {e}"))?;
    let answer: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let references = extract_references(&answer, &entry_ids, &sources_list);
    let receipt = ledger
        .append_with_hash(
            &st_key,
            "sidetalk.turn",
            EVENT_VERSION,
            &json!({
                "base_session": base_session,
                "question": question,
                "answer": answer,
                "references": references,
                "raw_blob": resp_blob,
            }),
        )
        .map_err(|e| format!("failed to record sidetalk.turn: {e}"))?;

    Ok(SideTalkTurn {
        seq: receipt.seq,
        ts_ms: receipt.ts_ms,
        question: question.to_string(),
        answer,
        references,
        raw_blob: Some(resp_blob),
    })
}

fn list_session_sources(env: &ResearchEnv, base_session: &str) -> Vec<String> {
    let Ok(Some(root)) = env.sandbox().canonical_root(SandboxRoot::Sources, base_session, false) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_source_names(&root, &root, &mut out);
    out.sort();
    out.truncate(100);
    out
}

fn collect_source_names(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.filter_map(Result::ok) {
        if out.len() >= 100 {
            break;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if research_tools::is_symlink_or_junction(&path, &meta) {
            continue;
        }
        if meta.is_dir() {
            collect_source_names(root, &path, out);
        } else if meta.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn extract_references(answer: &str, entry_ids: &[String], sources: &[String]) -> Vec<String> {
    let mut refs = BTreeSet::new();
    for id in entry_ids {
        if answer.contains(id.as_str()) {
            refs.insert(id.clone());
        }
    }
    for src in sources {
        if answer.contains(src.as_str()) {
            refs.insert(src.clone());
        }
    }
    refs.into_iter().collect()
}
