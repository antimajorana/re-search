//! Research Strategist (F13): advisory consultations capped at `<= 20` per
//! session, with outputs explicitly marked `"unverified_advice"`.

use crate::guardrails::materialize_proof_entries;
use crate::runner::EVENT_VERSION;
use crate::transport::{ContentBlock, Message, ModelRequest, Role, Transport};
use ledger::Ledger;
use research_tools::{read_proof_dag, ProofStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_STRATEGIST_CONSULTATIONS: usize = 20;
const STRATEGIST_MAX_TOKENS: u32 = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategistAdvice {
    pub consultation_number: usize,
    pub status: String,
    pub advice: String,
    pub raw_blob: String,
    pub event_seq: u64,
}

pub fn count_strategist_consultations(ledger: &Ledger, session: &str) -> Result<usize, String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("failed to read session `{session}`: {e}"))?;
    Ok(events.iter().filter(|e| e.kind == "strategist.consult").count())
}

pub fn consult_strategist(
    transport: &dyn Transport,
    ledger: &Ledger,
    session: &str,
    question: &str,
    model: &str,
) -> Result<StrategistAdvice, String> {
    if question.trim().is_empty() {
        return Err("strategist question must not be empty".into());
    }
    if model.trim().is_empty() {
        return Err("strategist model name must not be empty".into());
    }
    let existing = count_strategist_consultations(ledger, session)?;
    if existing >= MAX_STRATEGIST_CONSULTATIONS {
        return Err(format!(
            "strategist consultation limit ({MAX_STRATEGIST_CONSULTATIONS}) reached for session `{session}`"
        ));
    }

    let events = ledger
        .read_session(session)
        .map_err(|e| format!("failed to read session `{session}`: {e}"))?;
    let problem_prompt = events
        .iter()
        .find(|e| e.kind == "session.start")
        .and_then(|e| e.payload.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or("(no session prompt recorded)");
    let dag = read_proof_dag(ledger, session)?;
    let entries = if !dag.entries.is_empty() {
        dag.entries
    } else {
        materialize_proof_entries(&events)
    };
    let summary_lines: Vec<String> = entries
        .iter()
        .filter(|e| e.status != ProofStatus::Superseded)
        .map(|e| format!("- {} [{} / {:?}]: {}", e.entry_id, e.entry_type, e.status, e.summary))
        .collect();

    let system_prompt = "You are a Research Strategist providing high-level advisory strategy \
        guidance for mathematical and theoretical-physics derivations. Your suggestions are \
        strictly advisory (unverified_advice) and never bypass verification.";
    let user_prompt = format!(
        "Problem:\n{problem_prompt}\n\nCurrent active proof entries:\n{}\n\nQuestion:\n{question}",
        if summary_lines.is_empty() {
            "(none yet)".to_string()
        } else {
            summary_lines.join("\n")
        }
    );

    let req = ModelRequest {
        model: model.to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }],
        }],
        tools: Vec::new(),
        max_tokens: STRATEGIST_MAX_TOKENS,
    };
    let prepared = transport.prepare_request(&req).map_err(|e| e.to_string())?;
    let req_blob = ledger
        .put_blob(&prepared.body)
        .map_err(|e| format!("strategist request put_blob failed: {e}"))?;
    ledger
        .append_with_hash(
            session,
            "strategist.request",
            EVENT_VERSION,
            &json!({
                "consultation_number": existing + 1,
                "model": model,
                "question": question,
                "raw_blob": req_blob,
            }),
        )
        .map_err(|e| format!("failed to record strategist.request: {e}"))?;

    let resp = transport.send_prepared(&prepared).map_err(|e| e.to_string())?;
    let resp_blob = ledger
        .put_blob(&resp.raw_body)
        .map_err(|e| format!("strategist response put_blob failed: {e}"))?;

    let advice: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let consultation_number = existing + 1;
    let status = "unverified_advice".to_string();
    let event_seq = ledger
        .append_with_hash(
            session,
            "strategist.consult",
            EVENT_VERSION,
            &json!({
                "consultation_number": consultation_number,
                "status": status,
                "question": question,
                "advice": advice,
                "raw_blob": resp_blob,
            }),
        )
        .map_err(|e| format!("failed to record strategist.consult: {e}"))?
        .seq;

    Ok(StrategistAdvice {
        consultation_number,
        status,
        advice,
        raw_blob: resp_blob,
        event_seq,
    })
}

/// Returns `true` when the latest `proof.verification` verdict was `"rejected"`
/// and neither a subsequent `"strategist.consult"` nor 3 `"strategist.bypass"`
/// / refusal escape entries have cleared the mandatory strategist lock.
pub fn mandatory_strategist_required(ledger: &Ledger, session: &str) -> Result<bool, String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("failed to read session `{session}`: {e}"))?;
    let mut locked = false;
    let mut bypasses = 0usize;
    for ev in &events {
        match ev.kind.as_str() {
            "proof.verification" => {
                if ev.payload.get("verdict").and_then(Value::as_str) == Some("rejected") {
                    locked = true;
                    bypasses = 0;
                } else if ev.payload.get("verdict").and_then(Value::as_str) == Some("accepted") {
                    locked = false;
                    bypasses = 0;
                }
            }
            "strategist.consult" => {
                locked = false;
                bypasses = 0;
            }
            "strategist.refusal" | "model.refusal" if locked => {
                bypasses += 1;
                if bypasses >= 3 {
                    locked = false;
                }
            }
            _ => {}
        }
    }
    Ok(locked)
}
