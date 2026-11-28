//! Host-registered `check_run` tool and S3 `AutoCheckGate` (`solve.md` 4.4, 5.1, rules R3/R8/R18/R19).

use crate::policy_preset::SOLVE_CHECK_CALLER;
use crate::solve_defs::{CHECK_RUN, SOLVE_V1_CHECK_NAMES, STEP_RECORD};
use crate::solve_model::{event_kinds, CheckOutcome, ClaimForm, ReceiptRef, SolveCheck};
use crate::solve_state::SolveState;
use crate::solve_status::badge_for_step;
use crate::ResearchEnv;
use ledger::{canonical_json, sha256_hex, Ledger};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tools::{CallCtx, CallOutcome, Gate, GateDecision, GateRequest, PostCallCtx, ToolHandler};

#[path = "solve_checks_params.rs"]
mod params;
use self::params::{
    attach_code_hashes, parse_check_outcome, parse_check_run_args, prepare_check_invocation,
    requester_for_caller,
};

/// Host cap for automatic and interactive `check_run` executions (`solve.md` 5.1).
pub const DEFAULT_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_SCHEMA_VERSION: u32 = 1;

/// Raw execution result returned by a [`CheckProcessRunner`].
#[derive(Debug, Clone)]
pub struct CheckExecResult {
    pub raw: Value,
    pub epoch: u64,
    pub request_id: u64,
    pub environment: Option<Value>,
}

/// Failure modes when invoking the check process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckExecError {
    Timeout(Duration),
    Worker(String),
}

/// Boundary to S6's `--role check` worker (`Worker::run_check` in desktop/tests).
pub trait CheckProcessRunner: Send + Sync {
    fn run_check(
        &self,
        name: &str,
        handles: &BTreeMap<String, String>,
        params: &Value,
        timeout: Duration,
        required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError>;

    /// Whether `name` is registered in the host check process (`solve.md` 5.1, S9 + S10).
    fn supports_check(&self, name: &str) -> bool {
        name == "symbolic_zero_smoke" || SOLVE_V1_CHECK_NAMES.contains(&name)
    }
}

/// Real `check_run` tool handler (`solve.md` 5.1, rules R3 & R18; package S9).
pub fn check_run_handler(
    env: Arc<ResearchEnv>,
    runner: Arc<dyn CheckProcessRunner>,
) -> Arc<dyn ToolHandler> {
    Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        run_check_tool(&env, runner.as_ref(), ctx, args, DEFAULT_CHECK_TIMEOUT)
    })
}

/// Execute `check_run` for `ctx` with a caller-provided timeout (10 s default).
pub fn run_check_tool(
    _env: &ResearchEnv,
    runner: &dyn CheckProcessRunner,
    ctx: &CallCtx<'_>,
    args: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    let parsed = parse_check_run_args(args)?;

    // Snapshot ledger inputs and drop the lock before executing the child check process.
    let prepared = {
        let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let events = guard.read_session(ctx.session).map_err(|e| e.to_string())?;
        let state = SolveState::from_events(&events);
        prepare_check_invocation(&guard, &state, &parsed)?
    };

    let started = Instant::now();
    let exec_res = runner.run_check(
        &parsed.check,
        &prepared.handles,
        &prepared.full_params,
        timeout,
        ctx.required_epoch,
    );
    let fallback_wall = started.elapsed().as_millis() as u64;

    let (outcome, evidence, registry_sha256, wall_ms) = match exec_res {
        Ok(res) => {
            if res.epoch > 0 {
                ctx.record_worker_execution(res.epoch, res.request_id);
            }
            if let Some(env_val) = &res.environment {
                ctx.record_worker_environment(env_val);
            }
            let outcome = parse_check_outcome(res.raw.get("outcome"))?;
            let mut evidence = res.raw.get("evidence").cloned().unwrap_or_else(|| json!({}));
            attach_code_hashes(&prepared, &mut evidence);
            let reg_sha = res
                .raw
                .get("registry_sha256")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let wms = res
                .raw
                .get("wall_ms")
                .and_then(Value::as_u64)
                .unwrap_or(fallback_wall);
            (outcome, evidence, reg_sha, wms)
        }
        Err(CheckExecError::Timeout(dur)) => (
            CheckOutcome::Inconclusive,
            json!({
                "reason": format!("check timed out after {}s", dur.as_secs()),
                "timeout_s": dur.as_secs(),
            }),
            String::new(),
            fallback_wall,
        ),
        Err(CheckExecError::Worker(err)) => {
            return Err(format!("check process failed for `{}`: {err}", parsed.check));
        }
    };

    let evidence_sha256 = sha256_hex(canonical_json(&evidence).as_bytes());
    let asked_by = requester_for_caller(ctx.caller);

    let (check_rec, _seq) = {
        let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let events = guard.read_session(ctx.session).map_err(|e| e.to_string())?;
        let expected_check_seq = events.last().map_or(0, |e| e.seq) + 1;
        // Router writes `tool.result` as the immediate next event once handler + after_success finish.
        let expected_result_seq = expected_check_seq + 1;
        let check = SolveCheck {
            check_id: format!("chk-{expected_check_seq}"),
            claim_sha256: prepared.claim_sha256.clone(),
            claim_index: parsed.claim_index,
            entry_id: Some(parsed.entry_id.clone()),
            math_sha256: prepared.math_sha256.clone(),
            name: parsed.check.clone(),
            registry_sha256,
            params: parsed.user_params.clone(),
            outcome,
            asked_by,
            receipt: ReceiptRef {
                call_id: ctx.call_id.to_string(),
                result_seq: expected_result_seq,
            },
            evidence,
            evidence_sha256,
        };
        let payload = serde_json::to_value(&check).map_err(|e| e.to_string())?;
        let appended = guard
            .append_with_hash(ctx.session, event_kinds::SOLVE_CHECK, EVENT_SCHEMA_VERSION, &payload)
            .map_err(|e| format!("failed to append solve.check: {e}"))?;
        if appended.seq != expected_check_seq {
            return Err(format!(
                "concurrent session modification during {CHECK_RUN} (expected seq {expected_check_seq}, got {})",
                appended.seq
            ));
        }
        (check, appended.seq)
    };

    let mut out = serde_json::to_value(&check_rec).map_err(|e| e.to_string())?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("process".into(), json!("check"));
        obj.insert("wall_ms".into(), json!(wall_ms));
    }
    Ok(out)
}

/// Re-run an existing `solve.check` request and report whether `evidence_sha256` matches (R19).
pub fn evaluate_recheck_diff(original: &SolveCheck, new_check: &Value) -> Value {
    let new_sha = new_check
        .get("evidence_sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new_outcome = new_check
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let orig_outcome = serde_json::to_value(original.outcome)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();

    let matched = new_sha == original.evidence_sha256 && new_outcome == orig_outcome;
    let warning = if matched {
        Value::Null
    } else {
        json!("result changed on re-run")
    };
    json!({
        "original_check_id": original.check_id,
        "new_check_id": new_check.get("check_id").cloned().unwrap_or(Value::Null),
        "matched": matched,
        "changed": !matched,
        "original_evidence_sha256": original.evidence_sha256,
        "new_evidence_sha256": new_sha,
        "warning": warning,
        "new_check": new_check,
    })
}

/// S3 Router post-call hook issuing `check_run` automatically as `host:check` after `step_record`
/// (`solve.md` 5.1, rule R18).
pub struct AutoCheckGate {
    env: Arc<ResearchEnv>,
    runner: Arc<dyn CheckProcessRunner>,
}

impl AutoCheckGate {
    pub fn new(env: Arc<ResearchEnv>, runner: Arc<dyn CheckProcessRunner>) -> Self {
        Self { env, runner }
    }
}

impl Gate for AutoCheckGate {
    fn before(&self, _req: &GateRequest<'_>) -> GateDecision {
        GateDecision::Allow
    }

    fn after_success(&self, ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
        if ctx.tool_id != STEP_RECORD {
            return Ok(());
        }
        let Some(entry_id) = ctx.result.get("entry_id").and_then(Value::as_str).map(str::to_string) else {
            return Ok(());
        };
        let Some(claims) = ctx.args.get("claims").and_then(Value::as_array) else {
            attach_auto_checks_only(ctx.result, Vec::new());
            return Ok(());
        };
        if claims.is_empty() {
            attach_auto_checks_only(ctx.result, Vec::new());
            return Ok(());
        }

        let ledger = Ledger::open(&self.env.data_dir).map_err(|e| e.to_string())?;
        let events = ledger.read_session(ctx.session).map_err(|e| e.to_string())?;
        let state = SolveState::from_events(&events);
        let has_dimensions = state.setup.as_ref().is_some_and(|s| !s.dimensions.is_empty());

        let mut auto_checks: Vec<Value> = Vec::new();
        let mut grant_denied = false;

        for (claim_index, raw_claim) in claims.iter().enumerate() {
            if grant_denied {
                break;
            }
            let form: ClaimForm = serde_json::from_value(
                raw_claim.get("form").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| format!("auto-check invalid claim form: {e}"))?;

            let primary = match form {
                ClaimForm::Equation | ClaimForm::Value => "identity_symbolic",
                ClaimForm::Series => "series_match",
                ClaimForm::Inequality => "counterexample_search",
            };

            let outcome = invoke_auto_check(
                ctx,
                &entry_id,
                claim_index,
                primary,
                &mut auto_checks,
            )?;
            if outcome.as_deref() == Some("not_run") {
                grant_denied = true;
                continue;
            }

            if matches!(form, ClaimForm::Equation | ClaimForm::Value)
                && outcome.as_deref() == Some("inconclusive")
            {
                let num_out = invoke_auto_check(
                    ctx,
                    &entry_id,
                    claim_index,
                    "identity_numeric",
                    &mut auto_checks,
                )?;
                if num_out.as_deref() == Some("not_run") {
                    grant_denied = true;
                    continue;
                }
            }

            if has_dimensions && !grant_denied {
                if self.runner.supports_check("dimensions") {
                    let dim_out = invoke_auto_check(
                        ctx,
                        &entry_id,
                        claim_index,
                        "dimensions",
                        &mut auto_checks,
                    )?;
                    if dim_out.as_deref() == Some("not_run") {
                        grant_denied = true;
                    }
                } else {
                    auto_checks.push(json!({
                        "claim_index": claim_index,
                        "check": "dimensions",
                        "outcome": "not_run",
                        "reason": "dimensions check is not registered yet (package S10)"
                    }));
                }
            }
        }

        let updated_events = ledger.read_session(ctx.session).map_err(|e| e.to_string())?;
        let updated_state = SolveState::from_events(&updated_events);
        attach_auto_checks_only(ctx.result, auto_checks);
        if let Some(obj) = ctx.result.as_object_mut() {
            if let Some(badge) = badge_for_step(&updated_state, &entry_id) {
                obj.insert("badge".into(), json!(badge));
            }
            if let Some(tier) = updated_state.tier(&entry_id) {
                obj.insert("own_tier".into(), json!(tier.own));
                obj.insert("effective_tier".into(), json!(tier.effective));
            }
        }
        Ok(())
    }
}

fn invoke_auto_check(
    ctx: &mut PostCallCtx<'_>,
    entry_id: &str,
    claim_index: usize,
    check_name: &str,
    auto_checks: &mut Vec<Value>,
) -> Result<Option<String>, String> {
    let args = json!({
        "entry_id": entry_id,
        "claim_index": claim_index,
        "check": check_name,
        "params": {},
    });
    match ctx.router.try_call(SOLVE_CHECK_CALLER, CHECK_RUN, &args)? {
        CallOutcome::Ok(val) => {
            let outcome = val
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("inconclusive")
                .to_string();
            auto_checks.push(json!({
                "claim_index": claim_index,
                "check": check_name,
                "check_id": val.get("check_id").cloned().unwrap_or(Value::Null),
                "outcome": outcome,
                "evidence": val.get("evidence").cloned().unwrap_or_else(|| json!({})),
            }));
            Ok(Some(outcome))
        }
        CallOutcome::Denied(reason) => {
            auto_checks.push(json!({
                "claim_index": claim_index,
                "check": check_name,
                "outcome": "not_run",
                "reason": reason,
            }));
            Ok(Some("not_run".to_string()))
        }
        CallOutcome::Error(error) => {
            auto_checks.push(json!({
                "claim_index": claim_index,
                "check": check_name,
                "outcome": "inconclusive",
                "reason": error,
            }));
            Ok(Some("inconclusive".to_string()))
        }
    }
}

fn attach_auto_checks_only(result: &mut Value, auto_checks: Vec<Value>) {
    if let Some(obj) = result.as_object_mut() {
        obj.insert("auto_checks".into(), Value::Array(auto_checks));
    }
}

#[cfg(test)]
#[path = "solve_checks_tests.rs"]
mod tests;
