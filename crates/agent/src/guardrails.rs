//! Research guardrails (F11): conventions, stepped tool-output masking,
//! computation-interpretation check, and full-verification completion gate.

use crate::runner::{
    run_turn_leg, tool_defs_json, Audit, EventSink, LoopParams, SessionOutcome,
};
use crate::transport::{ContentBlock, Message, Role, Transport};
use ledger::{Event, Ledger};
use research_tools::{read_proof_dag, ProofDag, ProofEntry, ProofStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use tools::Router;

pub const MAX_PINNED_CONVENTIONS_CHARS: usize = 2_000;
pub const DEFAULT_STEP_MASK_KEEP: usize = 6;
pub const DEFAULT_STEP_MASK_STRIDE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailConfig {
    pub verification_mode: bool,
    pub calculation_mode: bool,
    pub pinned_conventions: Option<String>,
    pub step_mask_keep: usize,
    pub step_mask_stride: usize,
}

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            verification_mode: false,
            calculation_mode: false,
            pinned_conventions: None,
            step_mask_keep: DEFAULT_STEP_MASK_KEEP,
            step_mask_stride: DEFAULT_STEP_MASK_STRIDE,
        }
    }
}

pub fn validate_pinned_conventions(text: &str) -> Result<(), String> {
    let chars = text.chars().count();
    if chars > MAX_PINNED_CONVENTIONS_CHARS {
        return Err(format!(
            "pinned_conventions exceeds {MAX_PINNED_CONVENTIONS_CHARS} characters (got {chars})"
        ));
    }
    Ok(())
}

pub fn build_research_system_prompt(base_prompt: &str, cfg: &GuardrailConfig) -> Result<String, String> {
    let mut sections = vec![base_prompt.trim_end().to_string()];
    if let Some(conv) = cfg.pinned_conventions.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        validate_pinned_conventions(conv)?;
        sections.push(format!("<pinned_conventions>\n{conv}\n</pinned_conventions>"));
    }
    if cfg.calculation_mode {
        sections.push(
            "Calculation mode is active: prefer explicit symbolic/numerical computations via `sympy_eval` \
             or `check_expression_sanity`. Interpret every successful computation in a `result` proof entry. \
             Record the returned `tool_call_ref` on that result or on a computation entry cited in `depends_on`."
                .to_string(),
        );
    }
    if cfg.verification_mode {
        sections.push(
            "Verification mode is active: every `final_answer` requires an accepted full-solution \
             verification (`proof.verification` with `kind: \"full\"` and `verdict: \"accepted\"`)."
                .to_string(),
        );
    }
    Ok(sections.join("\n\n"))
}

pub fn upsert_pinned_conventions_in_prompt(sys: &mut String, pinned: Option<&str>) -> Result<(), String> {
    const OPEN: &str = "<pinned_conventions>";
    const CLOSE: &str = "</pinned_conventions>";
    let clean = pinned.map(str::trim).filter(|s| !s.is_empty());
    if let Some(conv) = clean {
        validate_pinned_conventions(conv)?;
    }
    if let Some(start) = sys.find(OPEN) {
        if let Some(rel_end) = sys[start..].find(CLOSE) {
            let block_end = start + rel_end + CLOSE.len();
            match clean {
                Some(conv) => sys.replace_range(start..block_end, &format!("{OPEN}\n{conv}\n{CLOSE}")),
                None => {
                    let lo = if sys[..start].ends_with("\n\n") { start - 2 } else { start };
                    let hi = if lo == start && sys[block_end..].starts_with("\n\n") { block_end + 2 } else { block_end };
                    sys.replace_range(lo..hi, "");
                    *sys = sys.trim().to_string();
                }
            }
            return Ok(());
        }
    }
    if let Some(conv) = clean {
        let block = format!("{OPEN}\n{conv}\n{CLOSE}");
        let trimmed = sys.trim_end();
        *sys = if trimmed.is_empty() { block } else { format!("{trimmed}\n\n{block}") };
    }
    Ok(())
}

/// Mask `ContentBlock::ToolResult` payloads older than the newest `keep_recent`
/// tool-result turns so long research conversations bound token growth while
/// preserving full outputs in `ledger.db` and `blobs/`.
pub fn mask_old_tool_outputs(messages: &[Message], keep_recent: usize) -> Vec<Message> {
    mask_old_tool_outputs_stepped(messages, keep_recent, 1)
}

/// Stepped tool-output masking: keeps the newest `keep_recent` tool-result
/// messages intact and masks older tool-result messages in strides of `stride`
/// (with `stride <= 1` masking all older tool-result messages immediately).
pub fn mask_old_tool_outputs_stepped(
    messages: &[Message],
    keep_recent: usize,
    stride: usize,
) -> Vec<Message> {
    let tool_msg_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.role == Role::User
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        })
        .map(|(i, _)| i)
        .collect();

    if tool_msg_indices.len() <= keep_recent {
        return messages.to_vec();
    }
    let oldest_count = tool_msg_indices.len() - keep_recent;
    let step = stride.max(1);
    let mask_count = if step == 1 {
        oldest_count
    } else {
        (oldest_count / step) * step
    };
    if mask_count == 0 {
        return messages.to_vec();
    }
    let to_mask: BTreeSet<usize> = tool_msg_indices.into_iter().take(mask_count).collect();

    messages
        .iter()
        .enumerate()
        .map(|(idx, msg)| {
            if !to_mask.contains(&idx) {
                return msg.clone();
            }
            let content = msg
                .content
                .iter()
                .map(|b| match b {
                    ContentBlock::ToolResult { tool_use_id, is_error, content } => {
                        let chars = content.chars().count();
                        let preview: String = content.chars().take(48).collect();
                        let single_line = preview.replace(['\r', '\n'], " ");
                        ContentBlock::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            is_error: *is_error,
                            content: format!(
                                "[masked old tool output ({chars} chars; stored in ledger): {single_line}...]"
                            ),
                        }
                    }
                    other => other.clone(),
                })
                .collect();
            Message { role: msg.role, content }
        })
        .collect()
}

pub(crate) fn materialize_proof_entries(events: &[Event]) -> Vec<ProofEntry> {
    let mut entries: BTreeMap<String, ProofEntry> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for ev in events {
        match ev.kind.as_str() {
            "proof.entry" => {
                let Some(entry_id) = ev.payload.get("entry_id").and_then(Value::as_str) else {
                    continue;
                };
                let entry_type = ev
                    .payload
                    .get("entry_type")
                    .and_then(Value::as_str)
                    .unwrap_or("derivation")
                    .to_string();
                let summary = ev
                    .payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let details = ev
                    .payload
                    .get("details")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let latex = ev.payload.get("latex").and_then(Value::as_str).map(str::to_string);
                let depends_on = ev
                    .payload
                    .get("depends_on")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let tool_call_ref = ev
                    .payload
                    .get("tool_call_ref")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if !entries.contains_key(entry_id) {
                    order.push(entry_id.to_string());
                }
                entries.insert(
                    entry_id.to_string(),
                    ProofEntry {
                        entry_id: entry_id.to_string(),
                        seq: ev.seq,
                        entry_type,
                        status: ProofStatus::Pending,
                        summary,
                        details,
                        latex,
                        depends_on,
                        tool_call_ref,
                        superseded_by: None,
                        verification: None,
                    },
                );
            }
            "proof.supersede" => {
                let Some(old_id) = ev.payload.get("entry_id").and_then(Value::as_str) else {
                    continue;
                };
                let replaced_by = ev
                    .payload
                    .get("replaced_by")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(entry) = entries.get_mut(old_id) {
                    entry.status = ProofStatus::Superseded;
                    entry.superseded_by = replaced_by;
                }
            }
            "proof.verification" => {
                let Some(target) = ev.payload.get("target_entry_id").and_then(Value::as_str) else {
                    continue;
                };
                let verdict = research_tools::effective_verification_verdict(&ev.payload);
                if let Some(entry) = entries.get_mut(target) {
                    if entry.status != ProofStatus::Superseded {
                        entry.status = match verdict {
                            Some("accepted") => ProofStatus::Accepted,
                            Some("rejected") => ProofStatus::Rejected,
                            _ => ProofStatus::Pending,
                        };
                    }
                    entry.verification = Some(ev.payload.clone());
                }
            }
            _ => {}
        }
    }
    order.into_iter().filter_map(|id| entries.remove(&id)).collect()
}

/// Ensures every active `symbolic_computation` or `numerical_computation` entry
/// is cited in the `depends_on` of an active `result` (or `final_answer`) entry,
/// and every successful `sympy_eval` in the current user turn has a recorded interpretation.
pub fn check_computation_interpretation(ledger: &Ledger, session: &str) -> Result<(), String> {
    load_dag_with_computation_check(ledger, session).map(drop)
}

fn load_dag_with_computation_check(ledger: &Ledger, session: &str) -> Result<ProofDag, String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("failed to read session `{session}`: {e}"))?;
    let turn_start = events.iter().rev()
        .find(|event| matches!(event.kind.as_str(), "session.start" | "session.resume"))
        .map_or(0, |event| event.seq);
    let dag = read_proof_dag(ledger, session)?;

    let mut cited_by_result: BTreeSet<String> = BTreeSet::new();
    for entry in &dag.entries {
        if !matches!(entry.status, ProofStatus::Superseded | ProofStatus::Rejected)
            && matches!(entry.entry_type.as_str(), "result" | "final_answer")
        {
            for dep in &entry.depends_on {
                cited_by_result.insert(dep.clone());
            }
        }
    }

    let uninterpreted: Vec<String> = dag.entries
        .iter()
        .filter(|e| {
            e.status != ProofStatus::Superseded
                && matches!(
                    e.entry_type.as_str(),
                    "symbolic_computation" | "numerical_computation"
                )
                && !cited_by_result.contains(&e.entry_id)
        })
        .map(|e| e.entry_id.clone())
        .collect();
    if !uninterpreted.is_empty() {
        return Err(format!(
            "computation entry(s) {:?} must be interpreted by a `result` proof entry citing them in `depends_on`",
            uninterpreted
        ));
    }

    let uncited: Vec<_> = dag.unverified_computations.iter().copied().filter(|&seq| seq > turn_start).collect();
    if !uncited.is_empty() {
        return Err(format!(
            "successful `sympy_eval` results at ledger sequences {uncited:?} need an interpreting `result` entry. \
             Record the returned `tool_call_ref` on the result or its computation dependency"
        ));
    }
    Ok(dag)
}

#[cfg(test)]
mod computation_tests {
    use super::*;

    #[test]
    fn calculation_gate_requires_linked_successful_results() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let session = "calc";
        let append = |kind, payload| ledger.append(session, kind, 1, &payload).unwrap();
        append("session.start", json!({}));
        append("tool.request", json!({"call_id": "failed", "tool": "sympy_eval"}));
        append("tool.result", json!({"call_id": "failed", "ok": false, "error": {"inline": "worker unavailable"}}));
        append("tool.request", json!({"call_id": "exception", "tool": "sympy_eval"}));
        append("tool.result", json!({"call_id": "exception", "ok": true, "result": {
            "inline": json!({"value": null, "error": {"message": "division by zero"}}).to_string()
        }}));
        assert!(check_computation_interpretation(&ledger, session).is_ok());

        append("proof.entry", json!({"entry_id": "e-early", "entry_type": "result",
            "summary": "Result asserted before evaluation", "tool_call_ref": "actual"}));
        append("tool.request", json!({"call_id": "actual", "tool": "sympy_eval"}));
        let blob = ledger.put_blob(br#"{"value":"4","stdout":""}"#).unwrap();
        let result_seq = append("tool.result", json!({"call_id": "actual", "ok": true, "result": {"blob": blob}}));
        // The substring rule previously credited "actual" inside this unrelated citation.
        append("proof.entry", json!({"entry_id": "e-1", "entry_type": "result",
            "summary": "Unrelated result", "tool_call_ref": "unactualized"}));
        let error = check_computation_interpretation(&ledger, session).unwrap_err();
        assert!(error.contains(&format!("[{result_seq}]")), "{error}");

        append("proof.entry", json!({"entry_id": "e-2", "entry_type": "symbolic_computation",
            "summary": "Evaluate 2+2", "tool_call_ref": "actual"}));
        append("proof.entry", json!({"entry_id": "e-3", "entry_type": "result",
            "summary": "The sum is 4", "depends_on": ["e-2"]}));
        assert!(check_computation_interpretation(&ledger, session).is_ok());
        append("proof.supersede", json!({"entry_id": "e-3", "replaced_by": "e-1"}));
        assert!(check_computation_interpretation(&ledger, session).is_err());
    }
}

/// Checks whether a final answer may be accepted under `verification_mode`.
/// When `verification_mode` is `true`, requires both computation interpretation
/// and an accepted full-solution verification of the current proof.
pub fn check_final_answer_gate(
    ledger: &Ledger,
    session: &str,
    verification_mode: bool,
) -> Result<(), String> {
    if !verification_mode {
        return Ok(());
    }
    if !load_dag_with_computation_check(ledger, session)?.latest_full_verification_accepted {
        return Err(
            "verification_mode requires an accepted full-solution verification of the current proof before finalizing"
                .into(),
        );
    }
    Ok(())
}

/// Research-aware agent loop that enforces [`GuardrailConfig`] conventions,
/// computation-interpretation checks, and `verification_mode` completion gates
/// while preserving `run_agent_loop`'s exact `session.start` / event schema.
pub fn run_research_loop(
    transport: &dyn Transport,
    router: &Router,
    ledger: &Ledger,
    cfg: &crate::runner::AgentConfig,
    guardrails: &GuardrailConfig,
    sink: &mut dyn EventSink,
) -> Result<SessionOutcome, String> {
    if cfg.max_turns == 0 {
        return Err("max_turns must be >= 1".into());
    }
    if cfg.max_tokens == 0 {
        return Err("max_tokens must be >= 1".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("model name must not be empty".into());
    }
    let system_prompt = build_research_system_prompt(&cfg.system_prompt, guardrails)?;
    let session = router.session();
    let info = transport.info();
    let defs = router.registry().defs();
    let existing = ledger.read_session(session).map_err(|e| e.to_string())?;
    let mut audit = Audit { ledger, session, sink, last_seq: existing.last().map_or(0, |e| e.seq) };
    let mut start_payload = json!({
        "route": "api",
        "provenance": "in-core loop, full provenance",
        "provider": info.provider,
        "endpoint": info.endpoint,
        "api_key_env": info.api_key_env,
        "model_requested": cfg.model_requested,
        "model": cfg.model,
        "prompt": cfg.prompt,
        "system_prompt": system_prompt,
        "max_turns": cfg.max_turns,
        "max_tokens": cfg.max_tokens,
        "policy_version": router.policy().version.as_str(),
        "containment": cfg.containment,
        "contained": cfg.contained,
        "verification_mode": guardrails.verification_mode,
        "calculation_mode": guardrails.calculation_mode,
        "pinned_conventions": guardrails.pinned_conventions,
        "tool_defs": tool_defs_json(&defs),
    });
    crate::resume_profile::insert_tool_profile(&mut start_payload, router, &existing);
    audit.emit("session.start", start_payload)?;

    let initial_messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: cfg.prompt.clone() }],
    }];
    let started = Instant::now();
    let leg = run_turn_leg(
        transport,
        router,
        &mut audit,
        LoopParams {
            model: &cfg.model,
            system_prompt: &system_prompt,
            max_turns: cfg.max_turns,
            max_tokens: cfg.max_tokens,
            reasoning_effort: cfg.reasoning_effort.as_deref(),
            start_turn: 1,
            initial_messages,
            mask_keep_recent: Some((guardrails.step_mask_keep, guardrails.step_mask_stride)),
            cancel: None,
            emit_end_event: false,
            delegation: None,
            human: None,
            pause_after_explore: false,
        },
    )?;

    if let SessionOutcome::Completed { turns, stop_reason } = &leg.outcome {
        let gate = if guardrails.verification_mode {
            check_final_answer_gate(ledger, session, true)
        } else if guardrails.calculation_mode {
            check_computation_interpretation(ledger, session)
        } else {
            Ok(())
        };
        if let Err(reason) = gate {
            audit.emit("guardrail.blocked", json!({ "reason": reason }))?;
            audit.emit("session.abort", json!({ "reason": reason }))?;
            return Ok(SessionOutcome::Aborted { turns: *turns, reason });
        }
        audit.emit("session.end", json!({
            "outcome": "completed",
            "stop_reason": stop_reason,
            "num_turns": turns,
            "last_turn": leg.last_turn,
            "duration_ms": started.elapsed().as_millis() as u64,
        }))?;
    }
    Ok(leg.outcome)
}
