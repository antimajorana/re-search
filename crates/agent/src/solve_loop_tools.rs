//! Tool dispatch, `sympy_eval` outbox harvesting, checkpoints, and pause helpers for `solve_loop`.

use super::{LegContext, SolveLoopConfig, SOLVE_CALLER, SOLVE_ROUTER_CALLER};
use crate::dispatch::clip_text;
use crate::interaction::{is_human_tool, run_ask_user_tool};
use crate::resume_brief::build_resume_brief;
use crate::runner::{tool_defs_json, Audit, SessionOutcome};
use crate::transport::{CancelToken, ContentBlock, Message, ToolSpec, Transport, TransportError};
use ledger::{sha256_hex, Event};
use research_tools::run_budget::{count_model_calls, RunScope};
use research_tools::solve_defs::SOLVE_FINISH;
use research_tools::solve_exprs::{
    attach_harvest_previews, harvest_from_sympy_result, CheckCanonicalizer,
};
use research_tools::solve_gates::{emit_solve_paused, PauseTrigger, SolveGate};
use research_tools::solve_model::{event_kinds, PauseReason, ReceiptRef, SolveSetup};
use research_tools::solve_state::SolveState;
use research_tools::tool_profile::PROFILE_SOLVE_V1;
use serde_json::{json, Value};
use std::sync::Arc;
use tools::canon::canonical_json;
use tools::{CallOutcome, Router};

const RESUME_MODE_BRIEF: &str = "brief";

pub(super) enum DispatchStep {
    Continue(Vec<ContentBlock>),
    Pause(PauseTrigger),
}

pub(super) fn validate_solve_cfg(router: &Router, cfg: &SolveLoopConfig<'_>) -> Result<(), String> {
    if cfg.max_tokens == 0 {
        return Err("max_tokens must be >= 1".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("model name must not be empty".into());
    }
    if let Some(p) = router.tool_profile() {
        if p != PROFILE_SOLVE_V1 {
            return Err(format!(
                "`solve_loop` requires tool_profile `{PROFILE_SOLVE_V1}`, got `{p}`"
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_brief_resume(
    audit: &mut Audit<'_>,
    transport: &dyn Transport,
    router: &Router,
    cfg: &SolveLoopConfig<'_>,
    setup: &SolveSetup,
    leg: u32,
    run_id: &str,
    brief: &str,
    brief_sha256: &str,
) -> Result<ledger::AppendedEvent, String> {
    let info = transport.info();
    let defs = router.registry().defs();
    let max_turns = setup.budgets.max_model_calls_per_leg;
    let prompt_val = if cfg.prompt.trim().is_empty() {
        brief
    } else {
        cfg.prompt.trim()
    };
    audit.emit(
        "session.resume",
        json!({
            "route": "api",
            "provenance": "in-core loop, full provenance",
            "provider": info.provider,
            "endpoint": info.endpoint,
            "api_key_env": info.api_key_env,
            "model_requested": cfg.model_requested,
            "model": cfg.model,
            "prompt": prompt_val,
            "max_turns": max_turns,
            "max_tokens": cfg.max_tokens,
            "policy_version": router.policy().version.as_str(),
            "tool_profile": PROFILE_SOLVE_V1,
            "resume_mode": RESUME_MODE_BRIEF,
            "brief_sha256": brief_sha256,
            "leg": leg,
            "run_id": run_id,
            "caller": SOLVE_CALLER,
            "tool_defs": tool_defs_json(&defs),
        }),
    )
}

pub(super) fn write_checkpoint(
    audit: &mut Audit<'_>,
    events: &[Event],
    leg: u32,
    reason: &str,
    budget_chars: usize,
) -> Result<(String, String), String> {
    let upto_seq = events.last().map_or(audit.last_seq, |e| e.seq);
    let brief = build_resume_brief(events, upto_seq, budget_chars);
    let brief_sha256 = sha256_hex(brief.as_bytes());
    let brief_blob = audit.put_blob(brief.as_bytes())?;
    audit.emit(
        event_kinds::SOLVE_CHECKPOINT,
        json!({
            "leg": leg,
            "upto_seq": upto_seq,
            "brief_blob": brief_blob,
            "raw_blob": brief_blob,
            "brief_sha256": brief_sha256,
            "reason": reason,
        }),
    )?;
    Ok((brief, brief_sha256))
}

pub(super) fn dispatch_solve_tools(
    audit: &mut Audit<'_>,
    router: &Router,
    gate: &Arc<SolveGate>,
    cfg: &SolveLoopConfig<'_>,
    ctx: &mut LegContext,
    turn: u32,
    content: &[ContentBlock],
) -> Result<DispatchStep, String> {
    let mut results = Vec::new();
    for block in content {
        let ContentBlock::ToolUse { id, name, input } = block else {
            continue;
        };
        if cfg.cancel.is_some_and(CancelToken::is_cancelled) {
            return Ok(DispatchStep::Pause(PauseTrigger {
                reason: PauseReason::User,
                message: "stopped by user".into(),
                open_items: Vec::new(),
            }));
        }
        let (is_error, content_text, pause_after) = if is_human_tool(name) {
            let (err, txt) = run_ask_user_tool(audit, cfg.human, id, input, cfg.cancel)?;
            (err, txt, None)
        } else if let Some(hooks) = cfg.hooks {
            if let Some(res) =
                hooks.handle_host_tool(audit.ledger, audit.session, name, id, input, cfg.cancel)
            {
                audit.flush_interleaved()?;
                let (err, txt) = res?;
                (err, txt, None)
            } else {
                dispatch_router_tool(audit, router, cfg, ctx, name, input)?
            }
        } else {
            dispatch_router_tool(audit, router, cfg, ctx, name, input)?
        };

        let (clipped, truncated) = clip_text(&content_text);
        audit.emit(
            "model.tool_result",
            json!({
                "turn": turn,
                "tool_use_id": id,
                "is_error": is_error,
                "text": clipped,
                "truncated": truncated,
            }),
        )?;
        results.push(ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            is_error,
            content: content_text,
        });

        if let Some(trigger) = pause_after {
            return Ok(DispatchStep::Pause(trigger));
        }
        if let Some(trigger) = gate.check_pause_trigger(audit.session)? {
            return Ok(DispatchStep::Pause(trigger));
        }
    }
    Ok(DispatchStep::Continue(results))
}

fn dispatch_router_tool(
    audit: &mut Audit<'_>,
    router: &Router,
    cfg: &SolveLoopConfig<'_>,
    ctx: &mut LegContext,
    name: &str,
    input: &Value,
) -> Result<(bool, String, Option<PauseTrigger>), String> {
    let prev_epoch = current_sympy_epoch(router, cfg.worker_runtime);
    let outcome = router.try_call(SOLVE_ROUTER_CALLER, name, input)?;
    audit.flush_interleaved()?;

    match outcome {
        CallOutcome::Ok(mut val) => {
            if name == "sympy_eval" {
                if let Some(canon) = cfg.canonicalizer {
                    harvest_sympy(audit, &mut val, canon)?;
                }
            }
            if name == SOLVE_FINISH {
                if let Some(hooks) = cfg.hooks {
                    hooks.after_solve_finish(audit.ledger, audit.session, &val)?;
                }
                if val.get("status").and_then(Value::as_str) == Some("finished_open_items")
                    && !ctx.repair_used
                {
                    ctx.repair_used = true;
                    ctx.pending_repair_prompt = Some(format!(
                        "<solve_repair_round>\n`solve_finish` reported `finished_open_items` ({}). \
                        You have one automatic repair round inside remaining budget to address open \
                        items and call `solve_finish` again.\n</solve_repair_round>",
                        val.get("criteria").cloned().unwrap_or(Value::Null)
                    ));
                }
            }
            Ok((false, canonical_json(&val), None))
        }
        CallOutcome::Denied(why) => {
            let pause = (!cfg.cancel.is_some_and(CancelToken::is_cancelled)).then(|| PauseTrigger {
                reason: PauseReason::ApprovalNeeded,
                message: format!("approval needed for `{name}`: {why}"),
                open_items: vec![json!({ "kind": "approval_needed", "tool": name, "reason": why })],
            });
            Ok((true, format!("DENIED: {why}"), pause))
        }
        CallOutcome::Error(why) => {
            let pause = if name == "sympy_eval" && is_worker_restart_error(&why) {
                let events = audit.ledger.read_session(audit.session).map_err(|e| e.to_string())?;
                let state = SolveState::from_events(&events);
                if let Some(rt) = cfg.worker_runtime {
                    if !state.handles.is_empty() {
                        rt.restage_handles(audit.ledger, audit.session, &state.handles)?;
                    }
                }
                let new_epoch = current_sympy_epoch(router, cfg.worker_runtime);
                let epoch_changed = prev_epoch.is_some() && new_epoch != prev_epoch;
                let reason = if epoch_changed {
                    PauseReason::ApprovalNeeded
                } else {
                    PauseReason::WorkerRestarted
                };
                Some(PauseTrigger {
                    reason,
                    message: format!("worker restart during `sympy_eval`: {why}"),
                    open_items: vec![json!({
                        "kind": reason.as_str(),
                        "handles_restaged": state.handles.len(),
                        "detail": why,
                    })],
                })
            } else {
                None
            };
            Ok((true, format!("ERROR: {why}"), pause))
        }
    }
}

fn harvest_sympy(
    audit: &mut Audit<'_>,
    val: &mut Value,
    canon: &dyn CheckCanonicalizer,
) -> Result<(), String> {
    let Some(call_id) = val.get("tool_call_ref").and_then(Value::as_str).map(str::to_string) else {
        return Ok(());
    };
    let events = audit.ledger.read_session(audit.session).map_err(|e| e.to_string())?;
    let result_seq = events
        .iter()
        .rev()
        .find(|e| {
            e.kind == event_kinds::TOOL_RESULT
                && e.payload.get("call_id").and_then(Value::as_str) == Some(call_id.as_str())
        })
        .map(|e| e.seq)
        .unwrap_or(audit.last_seq);
    let report = harvest_from_sympy_result(
        audit.ledger,
        audit.session,
        ReceiptRef { call_id, result_seq },
        val,
        canon,
    )?;
    audit.flush_interleaved()?;
    attach_harvest_previews(val, &report);
    Ok(())
}

pub(super) fn pause_on_transport(
    audit: &mut Audit<'_>,
    leg: u32,
    turn: u32,
    total_turns: u32,
    err: TransportError,
) -> Result<SessionOutcome, String> {
    let raw_blob = match &err.raw_body {
        Some(bytes) if !bytes.is_empty() => Some(audit.put_blob(bytes)?),
        _ => None,
    };
    let mut err_payload = json!({
        "turn": turn,
        "kind": err.kind.as_str(),
        "status": err.status,
        "message": err.message,
    });
    if let (Some(h), Some(obj)) = (raw_blob, err_payload.as_object_mut()) {
        obj.insert("raw_blob".into(), json!(h));
    }
    audit.emit("model.error", err_payload)?;
    pause_solve(
        audit,
        leg,
        turn,
        total_turns,
        PauseTrigger {
            reason: PauseReason::TransportError,
            message: err.to_string(),
            open_items: Vec::new(),
        },
    )
}

pub(super) fn pause_solve(
    audit: &mut Audit<'_>,
    leg: u32,
    turn: u32,
    turns: u32,
    trigger: PauseTrigger,
) -> Result<SessionOutcome, String> {
    let events = audit.ledger.read_session(audit.session).map_err(|e| e.to_string())?;
    let used = u32::try_from(count_model_calls(&events, RunScope::All)).unwrap_or(u32::MAX);
    emit_solve_paused(audit.ledger, audit.session, &trigger, leg, turn, used)?;
    audit.flush_interleaved()?;
    Ok(SessionOutcome::Paused {
        turns,
        reason: trigger.reason.as_str().to_string(),
    })
}

fn current_sympy_epoch(
    _router: &Router,
    rt: Option<&dyn super::SolveWorkerRuntime>,
) -> Option<u64> {
    rt.and_then(super::SolveWorkerRuntime::worker_epoch)
}

fn is_worker_restart_error(err: &str) -> bool {
    let low = err.to_ascii_lowercase();
    !low.starts_with("invalid arguments")
        && (low.contains("timed out")
            || low.contains("timeout")
            || low.contains("worker exited")
            || low.contains("epoch")
            || low.contains("restarted"))
}

pub(super) fn estimate_prompt_chars(system: &str, messages: &[Message], tools: &[ToolSpec]) -> usize {
    let mut chars = system.chars().count();
    for m in messages {
        for b in &m.content {
            chars += match b {
                ContentBlock::Text { text } => text.chars().count(),
                ContentBlock::Thinking { thinking, .. } => thinking.chars().count(),
                ContentBlock::RedactedThinking { data } => data.chars().count(),
                ContentBlock::ToolUse { name, input, .. } => {
                    name.chars().count() + input.to_string().chars().count()
                }
                ContentBlock::ToolResult { content, .. } => content.chars().count(),
                ContentBlock::Other { raw, .. } => raw.to_string().chars().count(),
            };
        }
    }
    for t in tools {
        chars += t.name.chars().count() + t.description.chars().count();
    }
    chars
}

pub(super) fn latest_leg(events: &[Event]) -> u32 {
    events
        .iter()
        .rev()
        .find_map(|e| e.payload.get("leg").and_then(Value::as_u64))
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(1)
}

pub(super) fn latest_turn(events: &[Event]) -> u32 {
    events
        .iter()
        .rev()
        .find(|e| e.kind == "model.request")
        .and_then(|e| e.payload.get("turn").and_then(Value::as_u64))
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}

pub(super) fn latest_run_id(events: &[Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|e| e.payload.get("run_id").and_then(Value::as_str).map(str::to_string))
}
