//! Proof Narrator (F13): read-only walkthrough & compact Derivation Sheet
//! assembled from accepted and pending proof-graph entries.

use crate::guardrails::materialize_proof_entries;
use crate::runner::EVENT_VERSION;
use crate::transport::{ContentBlock, Message, ModelRequest, Role, Transport};
use ledger::Ledger;
use research_tools::{read_proof_dag, ProofEntry, ProofStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const NARRATOR_MAX_TOKENS: u32 = 8192;
const NARRATOR_FALLBACK_MAX_CHARS: usize = 24_000;

const PEDAGOGICAL_NARRATOR_PROMPT: &str = "\
You are a deeply pedagogical Proof Narrator producing a self-contained, textbook-quality \
walkthrough for maximum human comprehension.\n\n\
How to structure and write your exposition:\n\
1. **Intuition & Roadmap First:** Before introducing dense notation, explain the core physical or \
mathematical idea in plain language and outline how the key steps fit together.\n\
2. **Fill In Omitted Intermediate Steps:** Never gloss over transitions with \"it is easy to see\" \
or \"after some algebra\". Explicitly write out intermediate substitutions, commutators, sign \
checks, integration-by-parts boundary terms, and limiting arguments so a careful reader can follow \
every line without guessing.\n\
3. **Trace Every Hypothesis & Dependency:** Show explicitly where each assumption is needed and why \
the argument would fail without it. Cite recorded Proof-DAG steps inline as `(e-k)`, while cleanly \
distinguishing explanatory bridge steps you fill in from recorded ledger claims.\n\
4. **Conventions, Edge Cases & Honesty:** Restate active conventions consistently. Never invent \
unverified theorems or hide gaps—if a step is `pending` or unverified, note that candidly.\n\
5. **Typography & Summary:** Use clear Markdown sections and clean LaTeX (`$...$` inline, `$$...$$` \
display), ending with a concise **Takeaway & Hypotheses** recap.";

fn format_problem_prompts(events: &[ledger::Event]) -> String {
    let prompts: Vec<&str> = events
        .iter()
        .filter(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"))
        .filter_map(|e| e.payload.get("prompt").and_then(Value::as_str))
        .filter(|p| !p.trim().is_empty())
        .collect();
    match prompts.as_slice() {
        [] => "(no session prompt recorded)".to_string(),
        [single] => (*single).to_string(),
        [first, followups @ ..] => {
            let numbered = followups
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {p}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            format!("Initial prompt:\n{first}\n\nFollow-up requests:\n{numbered}")
        }
    }
}

fn collect_fallback_derivations(events: &[ledger::Event]) -> String {
    let joined = events
        .iter()
        .filter(|ev| ev.kind == "model.text")
        .filter_map(|ev| ev.payload.get("text").and_then(Value::as_str))
        .filter(|t| !t.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    match joined.char_indices().nth(NARRATOR_FALLBACK_MAX_CHARS) {
        Some((idx, _)) => joined[..idx].to_string(),
        None => joined,
    }
}

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
    let has_session_start = events.iter().any(|e| e.kind == "session.start");
    let problem_prompt = format_problem_prompts(&events);
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
    let fallback_text = collect_fallback_derivations(&events);

    if !has_session_start && included.is_empty() && fallback_text.is_empty() {
        return Err("Run at least one derivation turn before narrating the proof.".into());
    }

    let derivation_sheet = build_derivation_sheet(&included);

    let (system_prompt, user_prompt) = if included.is_empty() {
        (
            format!(
                "{PEDAGOGICAL_NARRATOR_PROMPT}\n\n\
                 Work strictly from the session's recorded derivation messages and problem prompts below."
            ),
            format!(
                "Problem:\n{problem_prompt}\n\n{derivation_sheet}\n\n<session_derivations>\n{fallback_text}\n</session_derivations>"
            ),
        )
    } else {
        let mut proof_text = String::new();
        for e in &included {
            let deps = if e.depends_on.is_empty() {
                "none".to_string()
            } else {
                e.depends_on.join(", ")
            };
            proof_text.push_str(&format!(
                "- {} ({}, {:?}, depends_on: [{}]): {}\n  Details: {}\n  LaTeX: {}\n",
                e.entry_id,
                e.entry_type,
                e.status,
                deps,
                e.summary,
                e.details,
                e.latex.as_deref().unwrap_or("")
            ));
        }
        (
            format!(
                "{PEDAGOGICAL_NARRATOR_PROMPT}\n\n\
                 Base your walkthrough strictly on the accepted and pending Proof-DAG steps below."
            ),
            format!("Problem:\n{problem_prompt}\n\n{derivation_sheet}\n\nProof steps:\n{proof_text}"),
        )
    };
    let req = ModelRequest {
        model: model.to_string(),
        system: Some(system_prompt),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }],
        }],
        tools: Vec::new(),
        max_tokens: NARRATOR_MAX_TOKENS,
        reasoning_effort: None,
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
