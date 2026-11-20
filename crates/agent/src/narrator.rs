//! Proof Narrator (F13): read-only walkthrough & compact Derivation Sheet
//! assembled from accepted and pending proof-graph entries.

use crate::guardrails::materialize_proof_entries;
use crate::runner::EVENT_VERSION;
use crate::transport::{ContentBlock, Message, ModelRequest, Role, Transport};
use ledger::Ledger;
use research_tools::{read_proof_dag, ProofEntry, ProofStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const NARRATOR_MAX_TOKENS: u32 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrationReport {
    pub walkthrough: String,
    pub derivation_sheet: String,
    pub entry_count: usize,
    pub raw_blob: String,
    pub event_seq: u64,
}

pub fn build_derivation_sheet(entries: &[ProofEntry]) -> String {
    let active: Vec<&ProofEntry> = entries
        .iter()
        .filter(|e| matches!(e.status, ProofStatus::Accepted | ProofStatus::Pending))
        .collect();
    if active.is_empty() {
        return "## Derivation Sheet\n\n_No accepted or pending proof entries._".to_string();
    }
    let mut lines = vec![
        "## Derivation Sheet".to_string(),
        String::new(),
        "| Step | Type | Status | Dependencies | Summary |".to_string(),
        "|---|---|---|---|---|".to_string(),
    ];
    for e in active {
        let deps = if e.depends_on.is_empty() {
            "—".to_string()
        } else {
            e.depends_on.join(", ")
        };
        let st = match e.status {
            ProofStatus::Accepted => "accepted",
            ProofStatus::Pending => "pending",
            ProofStatus::Rejected => "rejected",
            ProofStatus::Superseded => "superseded",
        };
        lines.push(format!(
            "| `{}` | `{}` | `{}` | {} | {} |",
            e.entry_id,
            e.entry_type,
            st,
            deps,
            e.summary.replace('|', "\\|")
        ));
    }
    lines.join("\n")
}

pub fn narrate_session(
    transport: &dyn Transport,
    ledger: &Ledger,
    session: &str,
    model: &str,
) -> Result<NarrationReport, String> {
    if model.trim().is_empty() {
        return Err("narrator model name must not be empty".into());
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
    let included: Vec<ProofEntry> = entries
        .into_iter()
        .filter(|e| matches!(e.status, ProofStatus::Accepted | ProofStatus::Pending))
        .collect();
    let derivation_sheet = build_derivation_sheet(&included);

    let mut proof_text = String::new();
    for e in &included {
        proof_text.push_str(&format!(
            "- {} ({}, {:?}): {}\n  Details: {}\n  LaTeX: {}\n",
            e.entry_id,
            e.entry_type,
            e.status,
            e.summary,
            e.details,
            e.latex.as_deref().unwrap_or("")
        ));
    }

    let system_prompt = "You are a Proof Narrator producing a clear, rigorous read-only \
        walkthrough and Derivation Sheet from the session's accepted and pending proof steps.";
    let user_prompt = format!(
        "Problem:\n{problem_prompt}\n\n{derivation_sheet}\n\nProof steps:\n{proof_text}"
    );
    let req = ModelRequest {
        model: model.to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }],
        }],
        tools: Vec::new(),
        max_tokens: NARRATOR_MAX_TOKENS,
    };
    let prepared = transport.prepare_request(&req).map_err(|e| e.to_string())?;
    let req_blob = ledger
        .put_blob(&prepared.body)
        .map_err(|e| format!("narrator request put_blob failed: {e}"))?;
    ledger
        .append_with_hash(
            session,
            "narrator.request",
            EVENT_VERSION,
            &json!({ "model": model, "entry_count": included.len(), "raw_blob": req_blob }),
        )
        .map_err(|e| format!("failed to record narrator.request: {e}"))?;

    let resp = transport.send_prepared(&prepared).map_err(|e| e.to_string())?;
    let resp_blob = ledger
        .put_blob(&resp.raw_body)
        .map_err(|e| format!("narrator response put_blob failed: {e}"))?;
    let walkthrough: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let event_seq = ledger
        .append_with_hash(
            session,
            "narrator.report",
            EVENT_VERSION,
            &json!({
                "entry_count": included.len(),
                "walkthrough": walkthrough,
                "derivation_sheet": derivation_sheet,
                "raw_blob": resp_blob,
            }),
        )
        .map_err(|e| format!("failed to record narrator.report: {e}"))?
        .seq;

    Ok(NarrationReport {
        walkthrough,
        derivation_sheet,
        entry_count: included.len(),
        raw_blob: resp_blob,
        event_seq,
    })
}
