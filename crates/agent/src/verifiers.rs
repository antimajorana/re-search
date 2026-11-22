//! Stateless Step Verifier & Full-Solution Verifier (F12).
//!
//! Verifiers run with zero main-solver bias: prompts contain only the original
//! problem prompt, the relevant `depends_on` proof-graph slice, and the
//! `<deterministic_tex_audit>` XML report. Every `put_blob` is referenced by a
//! ledger event (`verifier.request` / `proof.verification`) carrying `"raw_blob"`.

use crate::guardrails::materialize_proof_entries;
use crate::runner::EVENT_VERSION;
use crate::transport::{CancelToken, ContentBlock, Message, ModelRequest, Role, StopReason, StreamObserver, Transport};
use ledger::Ledger;
use research_tools::{
    format_deterministic_tex_audit_xml, read_proof_dag, run_tex_audit, AuditOptions, AuditReport,
    ProofEntry, ProofStatus, ResearchEnv,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const VERIFIER_MAX_TOKENS: u32 = 8192;

struct CancelOnlyObserver<'a>(Option<&'a CancelToken>);
impl StreamObserver for CancelOnlyObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_some_and(CancelToken::is_cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationVerdict {
    Accepted,
    Rejected,
}

impl VerificationVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            VerificationVerdict::Accepted => "accepted",
            VerificationVerdict::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationIssue {
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub target_entry_id: String,
    pub kind: String,
    pub verdict: VerificationVerdict,
    pub summary: String,
    pub counterexample: Option<String>,
    pub issues: Vec<VerificationIssue>,
    pub tex_audit: AuditReport,
    pub raw_blob: String,
    pub event_seq: u64,
}

fn load_problem_and_entries(
    ledger: &Ledger,
    session: &str,
) -> Result<(String, Vec<ProofEntry>), String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("cannot read session `{session}`: {e}"))?;
    let problem_prompt = events
        .iter()
        .find(|e| e.kind == "session.start")
        .and_then(|e| e.payload.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or("(no session prompt recorded)")
        .to_string();
    let dag = read_proof_dag(ledger, session)?;
    let entries = if !dag.entries.is_empty() {
        dag.entries
    } else {
        materialize_proof_entries(&events)
    };
    Ok((problem_prompt, entries))
}

fn collect_ancestor_slice(entries: &[ProofEntry], target_id: &str) -> Result<Vec<ProofEntry>, String> {
    let by_id: BTreeMap<&str, &ProofEntry> =
        entries.iter().map(|e| (e.entry_id.as_str(), e)).collect();
    if !by_id.contains_key(target_id) {
        return Err(format!("proof entry `{target_id}` not found"));
    }
    let mut visited = BTreeSet::new();
    let mut stack = vec![target_id.to_string()];
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur.clone()) {
            continue;
        }
        if let Some(entry) = by_id.get(cur.as_str()) {
            for dep in &entry.depends_on {
                if !visited.contains(dep) {
                    stack.push(dep.clone());
                }
            }
        }
    }
    Ok(entries
        .iter()
        .filter(|e| visited.contains(&e.entry_id))
        .cloned()
        .collect())
}

fn format_entries_xml(entries: &[ProofEntry]) -> String {
    let mut out = String::from("<proof_entries>\n");
    for e in entries {
        let deps = e.depends_on.join(", ");
        let latex = e.latex.as_deref().unwrap_or("");
        out.push_str(&format!(
            "  <entry id=\"{}\" type=\"{}\" status=\"{:?}\" depends_on=\"{}\">\n    \
             <summary>{}</summary>\n    <details>{}</details>\n    <latex>{}</latex>\n  </entry>\n",
            e.entry_id, e.entry_type, e.status, deps, e.summary, e.details, latex
        ));
    }
    out.push_str("</proof_entries>");
    out
}

fn run_candidate_tex_audit(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    entries: &[ProofEntry],
) -> Result<AuditReport, String> {
    let candidate_texts = entries
        .iter()
        .map(|e| {
            let body = match &e.latex {
                Some(l) if !l.trim().is_empty() => format!("{}\n{}\n{}", e.summary, e.details, l),
                _ => format!("{}\n{}", e.summary, e.details),
            };
            (format!("proof_entry:{}", e.entry_id), body)
        })
        .collect();
    let opts = AuditOptions { candidate_texts, ..AuditOptions::default() };
    run_tex_audit(env, session, Some(ledger), &opts)
}

#[derive(Debug, Deserialize)]
struct RawModelVerdict {
    verdict: Option<String>,
    summary: Option<String>,
    counterexample: Option<String>,
    issues: Option<Vec<Value>>,
}

fn parse_verifier_response(text: &str) -> (VerificationVerdict, String, Option<String>, Vec<VerificationIssue>) {
    let trimmed = text.trim();
    let json_slice = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim().trim_end_matches("```").trim())
        .unwrap_or(trimmed);
    if let Ok(parsed) = serde_json::from_str::<RawModelVerdict>(json_slice) {
        let verdict = match parsed.verdict.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("accepted") => VerificationVerdict::Accepted,
            _ => VerificationVerdict::Rejected,
        };
        let summary = parsed
            .summary
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Verifier completed.".to_string());
        let counterexample = parsed.counterexample.filter(|s| !s.trim().is_empty());
        let issues = parsed
            .issues
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| {
                if let Some(s) = v.as_str() {
                    Some(VerificationIssue { severity: "error".into(), message: s.to_string() })
                } else {
                    let msg = v.get("message").and_then(Value::as_str)?.to_string();
                    let sev = v.get("severity").and_then(Value::as_str).unwrap_or("error").to_string();
                    Some(VerificationIssue { severity: sev, message: msg })
                }
            })
            .collect();
        return (verdict, summary, counterexample, issues);
    }
    let lower = trimmed.to_ascii_lowercase();
    let verdict = if lower.contains("\"accepted\"") || lower.starts_with("accepted") {
        VerificationVerdict::Accepted
    } else {
        VerificationVerdict::Rejected
    };
    (verdict, trimmed.chars().take(500).collect(), None, Vec::new())
}

struct VerifierParams<'a> {
    target_entry_id: &'a str,
    kind: &'a str,
    entries_slice: &'a [ProofEntry],
    problem_prompt: &'a str,
    model: &'a str,
    cancel: Option<&'a CancelToken>,
}

fn execute_verifier(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    params: VerifierParams<'_>,
) -> Result<VerificationReport, String> {
    let target_entry_id = params.target_entry_id;
    let kind = params.kind;
    let entries_slice = params.entries_slice;
    let problem_prompt = params.problem_prompt;
    let model = params.model;
    if model.trim().is_empty() {
        return Err("verifier model name must not be empty".into());
    }
    // Durable pre-call attempt barrier BEFORE contacting transport.
    ledger
        .append_with_hash(
            session,
            "proof.verification_attempt",
            EVENT_VERSION,
            &json!({
                "target_entry_id": target_entry_id,
                "kind": kind,
                "phase": "started",
            }),
        )
        .map_err(|e| format!("failed to record proof.verification_attempt: {e}"))?;

    let tex_audit = run_candidate_tex_audit(env, ledger, session, entries_slice)?;
    let tex_audit_xml = format_deterministic_tex_audit_xml(&tex_audit);
    let system_prompt = if kind == "full" {
        "You are an independent stateless full-solution verifier (full_verification). \
         Judge mathematical soundness strictly from the problem statement, proof entries, and \
         <deterministic_tex_audit>. Respond in JSON with {verdict: \"accepted\"|\"rejected\", \
         summary, counterexample, issues}."
    } else {
        "You are an independent stateless step verifier (step_verification). \
         Judge the target proof step strictly from the problem statement, its depends_on ancestor \
         entries, and <deterministic_tex_audit>. Respond in JSON with {verdict: \"accepted\"|\"rejected\", \
         summary, counterexample, issues}."
    };
    let user_prompt = format!(
        "target_entry_id: {target_entry_id}\nverification_kind: {kind}\n\n\
         <problem_prompt>\n{problem_prompt}\n</problem_prompt>\n\n\
         {}\n\n{tex_audit_xml}",
        format_entries_xml(entries_slice)
    );

    let req = ModelRequest {
        model: model.to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }],
        }],
        tools: Vec::new(),
        max_tokens: VERIFIER_MAX_TOKENS,
    };
    let prepared = transport.prepare_request(&req).map_err(|e| e.to_string())?;
    let req_blob = ledger
        .put_blob(&prepared.body)
        .map_err(|e| format!("verifier request put_blob failed: {e}"))?;
    ledger
        .append_with_hash(
            session,
            "verifier.request",
            EVENT_VERSION,
            &json!({
                "target_entry_id": target_entry_id,
                "kind": kind,
                "model": model,
                "raw_blob": req_blob,
            }),
        )
        .map_err(|e| format!("failed to record verifier.request: {e}"))?;

    let mut obs = CancelOnlyObserver(params.cancel);
    let resp = transport.send_prepared_streaming(&prepared, &mut obs).map_err(|e| e.to_string())?;
    let resp_blob = ledger
        .put_blob(&resp.raw_body)
        .map_err(|e| format!("verifier response put_blob failed: {e}"))?;

    let text: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let (mut verdict, mut summary, counterexample, mut issues) = if resp.stop_reason == StopReason::Refusal {
        (VerificationVerdict::Rejected, "Verifier refused to validate step.".to_string(), None, Vec::new())
    } else {
        parse_verifier_response(&text)
    };

    if tex_audit.blocker_count > 0 {
        verdict = VerificationVerdict::Rejected;
        summary = format!(
            "Blocked by deterministic tex_audit ({} blocker(s)): {summary}",
            tex_audit.blocker_count
        );
        for issue in tex_audit.issues.iter().filter(|i| i.severity.as_str() == "blocker") {
            issues.push(VerificationIssue {
                severity: "blocker".into(),
                message: issue.message.clone(),
            });
        }
    }

    let event_seq = ledger
        .append_with_hash(
            session,
            "proof.verification",
            EVENT_VERSION,
            &json!({
                "target_entry_id": target_entry_id,
                "kind": kind,
                "verdict": verdict.as_str(),
                "summary": summary,
                "counterexample": counterexample,
                "issues": issues,
                "raw_blob": resp_blob,
            }),
        )
        .map_err(|e| format!("failed to record proof.verification: {e}"))?
        .seq;

    Ok(VerificationReport {
        target_entry_id: target_entry_id.to_string(),
        kind: kind.to_string(),
        verdict,
        summary,
        counterexample,
        issues,
        tex_audit,
        raw_blob: resp_blob,
        event_seq,
    })
}

/// Stateless step verifier: isolates `entry_id` + its `depends_on` ancestors
/// and runs deterministic `tex_audit` before invoking the verifier model.
pub fn verify_step(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    entry_id: &str,
    model: &str,
) -> Result<VerificationReport, String> {
    let (problem_prompt, entries) = load_problem_and_entries(ledger, session)?;
    let slice = collect_ancestor_slice(&entries, entry_id)?;
    execute_verifier(
        transport,
        ledger,
        env,
        session,
        VerifierParams {
            target_entry_id: entry_id,
            kind: "step",
            entries_slice: &slice,
            problem_prompt: &problem_prompt,
            model,
            cancel: None,
        },
    )
}

/// Stateless full-solution verifier: slims superseded entries out of the proof DAG
/// and enforces deterministic `tex_audit` blockers over the entire active proof.
pub fn verify_full_solution(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    model: &str,
) -> Result<VerificationReport, String> {
    verify_full_solution_cancellable(transport, ledger, env, session, model, None)
}

pub fn verify_full_solution_cancellable(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    model: &str,
    cancel: Option<&CancelToken>,
) -> Result<VerificationReport, String> {
    let (problem_prompt, entries) = load_problem_and_entries(ledger, session)?;
    let active: Vec<ProofEntry> = entries
        .into_iter()
        .filter(|e| e.status != ProofStatus::Superseded)
        .collect();
    let target_entry_id = active
        .iter()
        .rev()
        .find(|e| e.entry_type == "final_answer")
        .or_else(|| active.last())
        .map(|e| e.entry_id.clone())
        .ok_or_else(|| format!("session `{session}` has no active proof entries to verify"))?;

    execute_verifier(
        transport,
        ledger,
        env,
        session,
        VerifierParams {
            target_entry_id: &target_entry_id,
            kind: "full",
            entries_slice: &active,
            problem_prompt: &problem_prompt,
            model,
            cancel,
        },
    )
}
