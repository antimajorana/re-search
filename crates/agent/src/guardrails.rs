//! Research guardrails (F11): conventions, stepped tool-output masking,
//! computation-interpretation check, and full-verification completion gate.

use crate::runner::{
    run_turn_loop, tool_defs_json, Audit, EventSink, LoopParams, SessionOutcome, EVENT_VERSION,
};
use crate::transport::{ContentBlock, Message, Role, Transport};
use ledger::{Event, Ledger};
use research_tools::{read_proof_dag, ProofEntry, ProofStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
             or `check_expression_sanity`, and interpret every computation in a `result` proof entry \
             citing its entry ID via `depends_on`."
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
                let verdict = ev.payload.get("verdict").and_then(Value::as_str);
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
/// and every `sympy_eval` tool call in the session is interpreted by a `result` entry.
pub fn check_computation_interpretation(ledger: &Ledger, session: &str) -> Result<(), String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("failed to read session `{session}`: {e}"))?;
    let dag = read_proof_dag(ledger, session)?;
    let entries = if !dag.entries.is_empty() {
        dag.entries
    } else {
        materialize_proof_entries(&events)
    };

    let mut cited_by_result: BTreeSet<String> = BTreeSet::new();
    let mut has_any_result = false;
    for entry in &entries {
        if entry.status != ProofStatus::Superseded
            && matches!(entry.entry_type.as_str(), "result" | "final_answer")
        {
            has_any_result = true;
            for dep in &entry.depends_on {
                cited_by_result.insert(dep.clone());
            }
        }
    }

    let uninterpreted: Vec<String> = entries
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

    if !dag.unverified_computations.is_empty() && !has_any_result {
        return Err(format!(
            "session executed `sympy_eval` (seq {:?}) without recording an interpreting `result` proof entry",
            dag.unverified_computations
        ));
    }
    Ok(())
}

/// Checks whether a final answer may be accepted under `verification_mode`.
/// When `verification_mode` is `true`, requires both computation interpretation
/// and an accepted full-solution verification (`kind == "full"`, `verdict == "accepted"`)
/// that occurred after the latest `proof.entry` / `proof.supersede` /
/// `proof.verification_attempt` mutation.
pub fn check_final_answer_gate(
    ledger: &Ledger,
    session: &str,
    verification_mode: bool,
) -> Result<(), String> {
    if !verification_mode {
        return Ok(());
    }
    check_computation_interpretation(ledger, session)?;

    let events = ledger
        .read_session(session)
        .map_err(|e| format!("failed to read session `{session}`: {e}"))?;
    let mut latest_full_accepted_seq: Option<u64> = None;
    let mut latest_mutation_or_attempt_seq: Option<u64> = None;

    for ev in &events {
        match ev.kind.as_str() {
            "proof.entry" | "proof.supersede" => {
                latest_mutation_or_attempt_seq = Some(ev.seq);
            }
            "proof.verification_attempt"
                if ev.payload.get("kind").and_then(Value::as_str) == Some("full") =>
            {
                latest_mutation_or_attempt_seq = Some(ev.seq);
            }
            "proof.verification"
                if ev.payload.get("kind").and_then(Value::as_str) == Some("full") =>
            {
                if ev.payload.get("verdict").and_then(Value::as_str) == Some("accepted") {
                    latest_full_accepted_seq = Some(ev.seq);
                } else {
                    latest_full_accepted_seq = None;
                }
            }
            _ => {}
        }
    }

    let Some(acc_seq) = latest_full_accepted_seq else {
        return Err(
            "verification_mode requires an accepted full-solution verification before finalizing"
                .into(),
        );
    };
    if let Some(mut_seq) = latest_mutation_or_attempt_seq {
        if mut_seq > acc_seq {
            return Err(
                "proof graph or verification attempt occurred after the last accepted full verification; re-run full verification"
                    .into(),
            );
        }
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
    let mut audit = Audit { ledger, session, sink, last_seq: 0 };
    audit.emit(
        "session.start",
        json!({
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
        }),
    )?;

    let initial_messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: cfg.prompt.clone() }],
    }];
    let outcome = run_turn_loop(
        transport,
        router,
        &mut audit,
        LoopParams {
            model: &cfg.model,
            system_prompt: &system_prompt,
            max_turns: cfg.max_turns,
            max_tokens: cfg.max_tokens,
            start_turn: 1,
            initial_messages,
            mask_keep_recent: Some((guardrails.step_mask_keep, guardrails.step_mask_stride)),
        },
    )?;

    if let SessionOutcome::Completed { turns, .. } = &outcome {
        if guardrails.calculation_mode {
            if let Err(calc_err) = check_computation_interpretation(ledger, session) {
                let _ = ledger.append_with_hash(
                    session,
                    "guardrail.blocked",
                    EVENT_VERSION,
                    &json!({ "reason": calc_err }),
                );
                return Ok(SessionOutcome::Aborted {
                    turns: *turns,
                    reason: calc_err,
                });
            }
        }
        if let Err(gate_err) = check_final_answer_gate(ledger, session, guardrails.verification_mode)
        {
            let _ = ledger.append_with_hash(
                session,
                "guardrail.blocked",
                EVENT_VERSION,
                &json!({ "reason": gate_err }),
            );
            return Ok(SessionOutcome::Aborted {
                turns: *turns,
                reason: gate_err,
            });
        }
    }
    Ok(outcome)
}
