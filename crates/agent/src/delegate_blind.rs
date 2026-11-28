//! Blind compute delegation (`compute_blind`, `solve.md` 4.4, 5.2, 5.4, rule G3; package S20).
//!
//! Spawns a fresh isolated Python worker via [`IsolatedWorkerFactory`] that receives ONLY
//! `inputs` and `operation` (never `compare_with`, solver derivation, or solver memory).
//! The host harvests the blind output handle via S7's check canonicalizer, compares it against
//! `compare_with` inside the dedicated check process (`identity_symbolic` with fallback to
//! `identity_numeric`), and records `solve.check { name: "independent_recompute" }`:
//! - Exact symbolic match -> `"comparison": "exact"`, `"exact": true` -> `Tier::Verified`.
//! - Numeric tolerance match -> `"comparison": "numeric"`, `"exact": false` -> `Tier::CheckedNumerically`.

#[path = "delegate_blind_check.rs"]
mod check;

use self::check::{
    build_blind_prompt, build_inputs_exprs_map, load_portable_srepr, parse_blind_args,
    resolve_target_claim, run_host_independent_comparison,
};
use crate::runner::EVENT_VERSION;
use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, Role, StopReason, StreamObserver, ToolSpec,
    Transport,
};
use crate::verifier_tools::IsolatedWorkerFactory;
use ledger::{sha256_hex, Ledger};
use research_tools::solve_exprs::{extract_exprs_out, harvest_exprs_out};
use research_tools::solve_gates::{
    check_g1_pending_runs, detect_g5_gate_bounces, enforce_g3_blind_compute, COMPUTE_BLIND,
    COMPUTE_BLIND_MAX_OPERATION_CHARS,
};
use research_tools::solve_model::{event_kinds, CheckOutcome, ReceiptRef, RoleModel};
use research_tools::solve_state::SolveState;
use research_tools::ResearchEnv;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Maximum model calls permitted for one blind-compute worker run.
pub const BLIND_COMPUTE_MAX_MODEL_CALLS: u32 = 6;
/// Token ceiling per blind-compute worker model call.
pub const BLIND_COMPUTE_MAX_TOKENS: u32 = 8_192;
/// Maximum allowed numeric fallback tolerance (`(0, 1e-3]`) so loose tolerances cannot mask wrong expressions.
pub const BLIND_COMPUTE_MAX_TOLERANCE: f64 = 1e-3;
/// Minimum/maximum numeric probe sample points (matching `python/checks/algebra_probe.py`).
pub const BLIND_COMPUTE_MIN_POINTS: u64 = 12;
pub const BLIND_COMPUTE_MAX_POINTS: u64 = 64;
/// Minimum/maximum numeric probe mpmath digits (matching `python/checks/algebra_probe.py`).
pub const BLIND_COMPUTE_MIN_DIGITS: u64 = 50;
pub const BLIND_COMPUTE_MAX_DIGITS: u64 = 120;
/// Role label recorded on `delegate.turn` / `delegate.end` events for F9 budget accounting.
pub const BLIND_COMPUTE_ROLE: &str = "blind_compute";
/// Caller tag recorded on blind worker `tool.request` events.
pub const BLIND_COMPUTE_CALLER: &str = "worker:blind_compute";
/// Check name recorded on `solve.check` events (`solve.md` 4.4, 5.2, `solve_own_tier.rs`).
pub const INDEPENDENT_RECOMPUTE_CHECK: &str = "independent_recompute";

struct CancelOnlyObserver<'a>(Option<&'a CancelToken>);
impl StreamObserver for CancelOnlyObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_some_and(CancelToken::is_cancelled)
    }
}

/// Configuration for one blind-compute worker run.
#[derive(Debug, Clone)]
pub struct BlindComputeConfig {
    pub role: RoleModel,
    pub max_model_calls: u32,
    pub max_tokens: u32,
    pub run_id: Option<String>,
}

impl BlindComputeConfig {
    pub fn new(role: RoleModel) -> Self {
        Self {
            role,
            max_model_calls: BLIND_COMPUTE_MAX_MODEL_CALLS,
            max_tokens: BLIND_COMPUTE_MAX_TOKENS,
            run_id: None,
        }
    }
}

/// Borrowed host dependencies for running `compute_blind`.
pub struct BlindComputeRunCtx<'a> {
    pub transport: &'a dyn Transport,
    pub ledger: &'a Ledger,
    pub env: Arc<ResearchEnv>,
    pub session: &'a str,
    pub config: &'a BlindComputeConfig,
    pub factory: &'a Arc<dyn IsolatedWorkerFactory>,
    pub cancel: Option<&'a CancelToken>,
}

/// Structured result of one host-handled `compute_blind` call (`solve.md` 5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlindComputeResult {
    pub agreed: bool,
    pub outcome: CheckOutcome,
    pub comparison: Option<String>,
    pub exact: bool,
    pub output: String,
    pub check_id: String,
    pub check_seq: u64,
    pub evidence: Value,
    pub model_calls_used: u32,
}

/// Tool schema advertised to the in-core Solve loop for host-handled `compute_blind` (`solve.md` 5.2).
pub fn compute_blind_tool_spec() -> ToolSpec {
    ToolSpec {
        name: COMPUTE_BLIND.into(),
        description: "Run an independent blind recomputation in a fresh isolated Python worker that \
            sees ONLY `inputs` and `operation` (never `compare_with`). The host compares the blind \
            output against `compare_with` in the check process and writes an `independent_recompute` check."
            .into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["inputs", "operation", "compare_with"],
            "properties": {
                "inputs": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 }
                },
                "operation": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": COMPUTE_BLIND_MAX_OPERATION_CHARS
                },
                "compare_with": { "type": "string", "minLength": 1 },
                "tolerance": {
                    "type": "number",
                    "exclusiveMinimum": 0,
                    "maximum": BLIND_COMPUTE_MAX_TOLERANCE
                },
                "entry_id": { "type": "string", "minLength": 1 },
                "claim_index": { "type": "integer", "minimum": 0 },
                "points": {
                    "type": "integer",
                    "minimum": BLIND_COMPUTE_MIN_POINTS,
                    "maximum": BLIND_COMPUTE_MAX_POINTS
                },
                "digits": {
                    "type": "integer",
                    "minimum": BLIND_COMPUTE_MIN_DIGITS,
                    "maximum": BLIND_COMPUTE_MAX_DIGITS
                }
            }
        }),
    }
}

fn blind_worker_system_prompt() -> String {
    "You are an independent blind compute worker for a mathematical calculation.\n\
     You receive ONLY the listed input handles and a short operation description.\n\
     Use `sympy_eval` to compute the requested expression from the staged inputs via `expr(\"<name>\")`.\n\
     Persist your final SymPy expression with `save_expr(\"blind_result\", result_expr)`.\n\
     Once `save_expr(\"blind_result\", ...)` succeeds without error, stop calling tools and summarize briefly."
        .to_string()
}

fn blind_sympy_tool_spec() -> ToolSpec {
    ToolSpec {
        name: "sympy_eval".into(),
        description: "Evaluate Python/SymPy code in an isolated worker containing ONLY the staged \
            input expressions. Load inputs with `expr(\"name@v\")` or `expr(\"name\")` and save the \
            final expression with `save_expr(\"blind_result\", expr_obj)`."
            .into(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["code"],
            "properties": {
                "code": { "type": "string", "minLength": 1 }
            }
        }),
    }
}

fn record_delegate_end(ctx: &BlindComputeRunCtx<'_>, call_id: &str, turns: u32, status: &str) {
    let mut payload = json!({
        "call_id": call_id,
        "role": BLIND_COMPUTE_ROLE,
        "provider": ctx.config.role.provider,
        "model": ctx.config.role.model,
        "turns": turns,
        "status": status,
    });
    if let Some(ref run_id) = ctx.config.run_id {
        payload["run_id"] = json!(run_id);
    }
    let _ = ctx
        .ledger
        .append_with_hash(ctx.session, "delegate.end", EVENT_VERSION, &payload);
}

/// Execute `compute_blind` end-to-end: Gates G5/G3/G1 -> isolated blind worker -> S7 harvest ->
/// host check-process comparison -> `solve.check { name: "independent_recompute" }`.
pub fn run_compute_blind(
    ctx: &BlindComputeRunCtx<'_>,
    raw_call_id: &str,
    args: &Value,
) -> Result<BlindComputeResult, String> {
    let call_id = raw_call_id.trim();
    if call_id.is_empty() {
        return Err("`compute_blind` requires a non-empty `call_id`".into());
    }
    // 1. Gate G3 runs BEFORE any model call or worker spawn.
    enforce_g3_blind_compute(ctx.ledger, ctx.session, call_id, args)?;
    if ctx.config.role.model.trim().is_empty() {
        return Err("blind compute model name must not be empty".into());
    }
    if ctx.cancel.is_some_and(CancelToken::is_cancelled) {
        return Err("Blind compute cancelled.".into());
    }

    let parsed = parse_blind_args(args)?;
    let events = ctx
        .ledger
        .read_session(ctx.session)
        .map_err(|e| format!("cannot read session `{}`: {e}", ctx.session))?;
    let state = SolveState::from_events(&events);

    // Host-handled enforcement of G5 (`gate_bounces`) and G1 (`pending_runs`).
    if let Some(trigger) = detect_g5_gate_bounces(&state) {
        return Err(format!("GATE G5: {}", trigger.message));
    }
    if let Err(reason) = check_g1_pending_runs(&state, "solver", COMPUTE_BLIND) {
        let message = format!("GATE G1: {reason}");
        ctx.ledger
            .append_with_hash(
                ctx.session,
                event_kinds::GUARDRAIL_BLOCKED,
                EVENT_VERSION,
                &json!({
                    "gate": "G1",
                    "tool": COMPUTE_BLIND,
                    "call_id": call_id,
                    "message": message,
                }),
            )
            .map_err(|e| format!("failed to record G1 guardrail.blocked: {e}"))?;
        return Err(message);
    }

    let setup = state
        .setup
        .as_ref()
        .ok_or("cannot run `compute_blind` before `solve.setup` is recorded")?
        .clone();

    let binding = resolve_target_claim(&state, &parsed)?;
    let compare_with_srepr =
        load_portable_srepr(ctx.ledger, &state, &parsed.compare_with, "compare_with")?;
    let exprs_in = build_inputs_exprs_map(ctx.ledger, &state, &parsed.inputs)?;
    let prompt = build_blind_prompt(&setup, &state, &parsed.inputs, &parsed.operation);

    // 2. Spawn fresh isolated blind Python worker (never sees `compare_with`).
    let blind_worker = ctx
        .factory
        .spawn_isolated_python_worker(ctx.session, BLIND_COMPUTE_ROLE)?;
    let eval_timeout = ctx.factory.eval_timeout();
    let max_calls = ctx
        .config
        .max_model_calls
        .clamp(1, BLIND_COMPUTE_MAX_MODEL_CALLS);
    let max_tokens = ctx.config.max_tokens.clamp(256, BLIND_COMPUTE_MAX_TOKENS);
    let tools = vec![blind_sympy_tool_spec()];

    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: prompt }],
    }];
    let mut model_calls_used = 0u32;
    let mut saved_srepr: Option<String> = None;
    let mut saved_receipt: Option<ReceiptRef> = None;
    let mut saved_explicit_blind_result = false;
    let mut ended_recorded = false;

    let loop_status: Result<(), String> = (|| {
        while model_calls_used < max_calls {
            if ctx.cancel.is_some_and(CancelToken::is_cancelled) {
                record_delegate_end(ctx, call_id, model_calls_used, "cancelled");
                ended_recorded = true;
                return Err("Blind compute cancelled.".into());
            }
            let req = ModelRequest {
                model: ctx.config.role.model.clone(),
                system: Some(blind_worker_system_prompt()),
                messages: messages.clone(),
                tools: tools.clone(),
                max_tokens,
                reasoning_effort: None,
            };
            let prepared = ctx.transport.prepare_request(&req).map_err(|e| e.to_string())?;
            let req_blob = ctx
                .ledger
                .put_blob(&prepared.body)
                .map_err(|e| format!("blind worker request put_blob failed: {e}"))?;

            model_calls_used += 1;
            let mut turn_payload = json!({
                "call_id": call_id,
                "role": BLIND_COMPUTE_ROLE,
                "provider": ctx.config.role.provider,
                "model": ctx.config.role.model,
                "turn": model_calls_used,
                "raw_blob": req_blob,
            });
            if let Some(ref run_id) = ctx.config.run_id {
                turn_payload["run_id"] = json!(run_id);
            }
            ctx.ledger
                .append_with_hash(ctx.session, "delegate.turn", EVENT_VERSION, &turn_payload)
                .map_err(|e| format!("failed to record delegate.turn: {e}"))?;

            let mut obs = CancelOnlyObserver(ctx.cancel);
            let resp = match ctx.transport.send_prepared_streaming(&prepared, &mut obs) {
                Ok(r) => r,
                Err(e) => {
                    if ctx.cancel.is_some_and(CancelToken::is_cancelled) {
                        record_delegate_end(ctx, call_id, model_calls_used, "cancelled");
                        ended_recorded = true;
                        return Err("Blind compute cancelled.".into());
                    }
                    return Err(e.to_string());
                }
            };
            if ctx.cancel.is_some_and(CancelToken::is_cancelled) {
                record_delegate_end(ctx, call_id, model_calls_used, "cancelled");
                ended_recorded = true;
                return Err("Blind compute cancelled.".into());
            }
            if resp.stop_reason == StopReason::Refusal {
                record_delegate_end(ctx, call_id, model_calls_used, "refused");
                ended_recorded = true;
                return Err("blind compute model refused the request".into());
            }

            messages.push(Message {
                role: Role::Assistant,
                content: resp.content.clone(),
            });
            let tool_uses: Vec<(String, String, Value)> = resp
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect();

            if tool_uses.is_empty() {
                if saved_srepr.is_some() {
                    break;
                }
                if model_calls_used < max_calls {
                    messages.push(Message {
                        role: Role::User,
                        content: vec![ContentBlock::Text {
                            text: "Call `sympy_eval` and save your result with \
                                   `save_expr(\"blind_result\", expr_obj)`."
                                .into(),
                        }],
                    });
                }
                continue;
            }

            let mut results = Vec::with_capacity(tool_uses.len());
            for (idx, (tu_id, name, input)) in tool_uses.into_iter().enumerate() {
                if name != "sympy_eval" {
                    results.push(ContentBlock::ToolResult {
                        tool_use_id: tu_id,
                        content: json!({ "error": format!("tool `{name}` is not available to blind compute") }).to_string(),
                        is_error: true,
                    });
                    continue;
                }
                let Some(code) = input
                    .get("code")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                else {
                    results.push(ContentBlock::ToolResult {
                        tool_use_id: tu_id,
                        content: json!({ "error": "`code` must be a non-empty string" }).to_string(),
                        is_error: true,
                    });
                    continue;
                };
                let eval_call_id = format!("{call_id}-eval-{model_calls_used}-{idx}");
                ctx.ledger
                    .append_with_hash(
                        ctx.session,
                        event_kinds::TOOL_REQUEST,
                        EVENT_VERSION,
                        &json!({
                            "call_id": eval_call_id,
                            "tool": "sympy_eval",
                            "caller": BLIND_COMPUTE_CALLER,
                            "args": { "code_sha256": sha256_hex(code.as_bytes()) },
                        }),
                    )
                    .map_err(|e| format!("failed to record blind tool.request: {e}"))?;

                let eval_res = blind_worker.evaluate(
                    code,
                    Some(&exprs_in),
                    eval_timeout,
                    blind_worker.live_epoch(),
                );
                match eval_res {
                    Ok(mut out) => {
                        let out_ok = out.result["error"].is_null();
                        if let Some(obj) = out.result.as_object_mut() {
                            obj.insert("process".to_string(), json!("verifier"));
                        }
                        let res_ev = ctx
                            .ledger
                            .append_with_hash(
                                ctx.session,
                                event_kinds::TOOL_RESULT,
                                EVENT_VERSION,
                                &json!({
                                    "call_id": eval_call_id,
                                    "tool": "sympy_eval",
                                    "ok": out_ok,
                                    "process": "verifier",
                                    "result": out.result,
                                }),
                            )
                            .map_err(|e| format!("failed to record blind tool.result: {e}"))?;
                        let outbox = extract_exprs_out(&out.result);
                        if out_ok {
                            if let Some(raw) = outbox.get("blind_result").cloned() {
                                saved_srepr = Some(raw);
                                saved_receipt = Some(ReceiptRef {
                                    call_id: eval_call_id,
                                    result_seq: res_ev.seq,
                                });
                                saved_explicit_blind_result = true;
                            } else if !saved_explicit_blind_result {
                                if let Some(raw) = outbox.values().next_back().cloned() {
                                    saved_srepr = Some(raw);
                                    saved_receipt = Some(ReceiptRef {
                                        call_id: eval_call_id,
                                        result_seq: res_ev.seq,
                                    });
                                }
                            }
                        }
                        results.push(ContentBlock::ToolResult {
                            tool_use_id: tu_id,
                            content: out.result.to_string(),
                            is_error: !out_ok,
                        });
                    }
                    Err(err) => {
                        let _ = ctx.ledger.append_with_hash(
                            ctx.session,
                            event_kinds::TOOL_RESULT,
                            EVENT_VERSION,
                            &json!({
                                "call_id": eval_call_id,
                                "tool": "sympy_eval",
                                "ok": false,
                                "process": "verifier",
                                "error": err,
                            }),
                        );
                        results.push(ContentBlock::ToolResult {
                            tool_use_id: tu_id,
                            content: json!({ "error": err }).to_string(),
                            is_error: true,
                        });
                    }
                }
            }
            messages.push(Message {
                role: Role::User,
                content: results,
            });
            // Stop early ONLY when the model explicitly saved `"blind_result"`.
            if saved_explicit_blind_result {
                break;
            }
        }
        let status = if saved_srepr.is_some() { "ok" } else { "no_output" };
        record_delegate_end(ctx, call_id, model_calls_used, status);
        ended_recorded = true;
        Ok(())
    })();

    // Always drop the isolated blind worker before harvesting/checking, and close any open delegate turns.
    drop(blind_worker);
    if loop_status.is_err() && !ended_recorded && model_calls_used > 0 {
        record_delegate_end(ctx, call_id, model_calls_used, "error");
    }
    loop_status?;

    let (raw_blind_srepr, produced_by) = match (saved_srepr, saved_receipt) {
        (Some(s), Some(r)) => (s, r),
        _ => {
            return Err(format!(
                "blind compute worker exhausted {model_calls_used} call(s) without calling `save_expr`"
            ));
        }
    };

    // 3. Harvest blind output via S7 check canonicalizer, then drop canonicalizer before check_runner.
    let canonicalizer = ctx.factory.check_canonicalizer(ctx.session)?;
    let mut raw_map = BTreeMap::new();
    raw_map.insert("blind_result".to_string(), raw_blind_srepr);
    let report = harvest_exprs_out(
        ctx.ledger,
        ctx.session,
        produced_by,
        &raw_map,
        canonicalizer.as_ref(),
    )?;
    drop(canonicalizer);

    let harvested = report.harvested.into_iter().next().ok_or_else(|| {
        let skipped = report
            .skipped
            .into_iter()
            .map(|s| format!("{}: {}", s.name, s.reason))
            .collect::<Vec<_>>()
            .join("; ");
        format!("blind compute output was not harvested ({skipped})")
    })?;
    if !harvested.expr.portable {
        return Err(format!(
            "blind compute output `{}` is not portable: {}",
            harvested.handle,
            harvested.error.unwrap_or_else(|| "rejected by check canonicalizer".into())
        ));
    }
    let blind_srepr = String::from_utf8(
        ctx.ledger
            .get_blob(&harvested.expr.srepr_blob)
            .map_err(|e| format!("failed to read harvested blind srepr: {e}"))?,
    )
    .map_err(|e| format!("invalid UTF-8 in blind srepr: {e}"))?;

    // 4. Compare in the dedicated check process (`identity_symbolic`, fallback `identity_numeric`).
    run_host_independent_comparison(
        ctx,
        call_id,
        &setup,
        &parsed,
        &binding,
        &compare_with_srepr,
        &harvested.handle,
        &blind_srepr,
        model_calls_used,
    )
}

/// Host-handled `compute_blind` seam for `solve_loop.rs` (`solve.md` 5.2, S12 + S20).
pub fn handle_compute_blind(
    ctx: &BlindComputeRunCtx<'_>,
    call_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let result = run_compute_blind(ctx, call_id, args)?;
    serde_json::to_value(&result).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "delegate_blind_tests.rs"]
mod tests;
