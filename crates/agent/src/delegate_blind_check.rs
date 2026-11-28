//! Argument parsing, claim binding, blind prompt construction, and host check-process
//! comparison (`independent_recompute`) for [`crate::delegate_blind`] (package S20).

use crate::delegate_blind::{
    BlindComputeResult, BlindComputeRunCtx, BLIND_COMPUTE_MAX_DIGITS, BLIND_COMPUTE_MAX_POINTS,
    BLIND_COMPUTE_MAX_TOLERANCE, BLIND_COMPUTE_MIN_DIGITS, BLIND_COMPUTE_MIN_POINTS,
    INDEPENDENT_RECOMPUTE_CHECK,
};
use crate::runner::EVENT_VERSION;
use ledger::{canonical_json, sha256_hex, Ledger};
use research_tools::policy_preset::SOLVE_CHECK_CALLER;
use research_tools::solve_checks::CheckExecError;
use research_tools::solve_exprs::load_handle_srepr;
use research_tools::solve_gates::COMPUTE_BLIND;
use research_tools::solve_model::{
    event_kinds, CheckOutcome, CheckRequester, Claim, ClaimForm, HandleRef, ReceiptRef, SolveCheck,
    SolveSetup,
};
use research_tools::solve_state::SolveState;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const ALLOWED_ARG_KEYS: &[&str] = &[
    "inputs",
    "operation",
    "compare_with",
    "tolerance",
    "entry_id",
    "claim_index",
    "points",
    "digits",
];

pub(crate) struct ParsedBlindArgs {
    pub(crate) inputs: Vec<HandleRef>,
    pub(crate) operation: String,
    pub(crate) compare_with: HandleRef,
    pub(crate) tolerance: Option<f64>,
    pub(crate) entry_id: Option<String>,
    pub(crate) claim_index: Option<usize>,
    pub(crate) points: Option<u64>,
    pub(crate) digits: Option<u64>,
}

pub(crate) struct TargetClaimBinding {
    pub(crate) entry_id: String,
    pub(crate) claim_index: usize,
    pub(crate) claim_sha256: String,
    pub(crate) domain: Option<String>,
}

pub(crate) fn parse_blind_args(args: &Value) -> Result<ParsedBlindArgs, String> {
    let obj = args
        .as_object()
        .ok_or("`compute_blind` arguments must be a JSON object")?;
    for key in obj.keys() {
        if !ALLOWED_ARG_KEYS.contains(&key.as_str()) {
            return Err(format!("unknown argument `{key}` for `{COMPUTE_BLIND}`"));
        }
    }
    let raw_inputs = obj
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or("`inputs` must be an array of handle strings")?;
    let mut inputs = Vec::with_capacity(raw_inputs.len());
    for item in raw_inputs {
        let s = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("each input handle must be a non-empty string")?;
        inputs.push(HandleRef::parse(s).map_err(|e| format!("invalid input handle `{s}`: {e}"))?);
    }
    let operation = obj
        .get("operation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("`operation` must be a non-empty string")?
        .to_string();
    let compare_raw = obj
        .get("compare_with")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("`compare_with` must be a non-empty handle string")?;
    let compare_with = HandleRef::parse(compare_raw)
        .map_err(|e| format!("invalid `compare_with` handle `{compare_raw}`: {e}"))?;

    let tolerance = match obj.get("tolerance") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let t = v
                .as_f64()
                .ok_or("`tolerance` must be a positive finite number")?;
            if !t.is_finite() || t <= 0.0 || t > BLIND_COMPUTE_MAX_TOLERANCE {
                return Err(format!(
                    "`tolerance` must be in (0, {BLIND_COMPUTE_MAX_TOLERANCE}]"
                ));
            }
            Some(t)
        }
    };
    let entry_id = obj
        .get("entry_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let claim_index = match obj.get("claim_index") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let idx = v
                .as_u64()
                .ok_or("`claim_index` must be a non-negative integer")?;
            Some(usize::try_from(idx).map_err(|_| "`claim_index` out of range")?)
        }
    };
    let points = match obj.get("points") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_u64().ok_or("`points` must be an integer")?;
            if !(BLIND_COMPUTE_MIN_POINTS..=BLIND_COMPUTE_MAX_POINTS).contains(&n) {
                return Err(format!(
                    "`points` must be in {BLIND_COMPUTE_MIN_POINTS}..={BLIND_COMPUTE_MAX_POINTS}"
                ));
            }
            Some(n)
        }
    };
    let digits = match obj.get("digits") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_u64().ok_or("`digits` must be an integer")?;
            if !(BLIND_COMPUTE_MIN_DIGITS..=BLIND_COMPUTE_MAX_DIGITS).contains(&n) {
                return Err(format!(
                    "`digits` must be in {BLIND_COMPUTE_MIN_DIGITS}..={BLIND_COMPUTE_MAX_DIGITS}"
                ));
            }
            Some(n)
        }
    };

    Ok(ParsedBlindArgs {
        inputs,
        operation,
        compare_with,
        tolerance,
        entry_id,
        claim_index,
        points,
        digits,
    })
}

fn validate_claim_for_blind(
    entry_id: &str,
    claim_index: usize,
    claim: &Claim,
    compare_with: &HandleRef,
) -> Result<(), String> {
    if claim.lhs != *compare_with && claim.rhs != *compare_with {
        return Err(format!(
            "claim #{claim_index} on step `{entry_id}` does not reference `compare_with` handle `{compare_with}`"
        ));
    }
    if !matches!(claim.form, ClaimForm::Equation | ClaimForm::Value) {
        return Err(format!(
            "`independent_recompute` applies only to `equation` or `value` claims (step `{entry_id}` claim #{claim_index} is `{:?}`)",
            claim.form
        ));
    }
    Ok(())
}

pub(crate) fn resolve_target_claim(
    state: &SolveState,
    parsed: &ParsedBlindArgs,
) -> Result<TargetClaimBinding, String> {
    if let Some(ref eid) = parsed.entry_id {
        let entry = state
            .entry(eid)
            .ok_or_else(|| format!("target step `{eid}` not found"))?;
        if entry.superseded_by.is_some() {
            return Err(format!("target step `{eid}` is superseded"));
        }
        if entry.claims.is_empty() {
            return Err(format!(
                "target step `{eid}` has no claims to attach `independent_recompute`"
            ));
        }
        let claim_sha = entry
            .claim_sha256
            .clone()
            .ok_or_else(|| format!("target step `{eid}` is missing `claim_sha256`"))?;
        let (idx, claim) = if let Some(ci) = parsed.claim_index {
            let c = entry.claims.get(ci).ok_or_else(|| {
                format!(
                    "`claim_index` {ci} out of bounds for step `{eid}` ({} claim(s))",
                    entry.claims.len()
                )
            })?;
            validate_claim_for_blind(eid, ci, c, &parsed.compare_with)?;
            (ci, c)
        } else {
            let ci = entry
                .claims
                .iter()
                .position(|c| c.lhs == parsed.compare_with || c.rhs == parsed.compare_with)
                .ok_or_else(|| {
                    format!(
                        "step `{eid}` has no claim referencing `compare_with` handle `{}`",
                        parsed.compare_with
                    )
                })?;
            let c = &entry.claims[ci];
            validate_claim_for_blind(eid, ci, c, &parsed.compare_with)?;
            (ci, c)
        };
        return Ok(TargetClaimBinding {
            entry_id: eid.clone(),
            claim_index: idx,
            claim_sha256: claim_sha,
            domain: claim.domain.clone(),
        });
    }

    for entry in state.entries.iter().rev() {
        if entry.superseded_by.is_some() || entry.claims.is_empty() {
            continue;
        }
        let Some(ref claim_sha) = entry.claim_sha256 else {
            continue;
        };
        if let Some(req_idx) = parsed.claim_index {
            if let Some(c) = entry.claims.get(req_idx) {
                if validate_claim_for_blind(&entry.entry_id, req_idx, c, &parsed.compare_with).is_ok() {
                    return Ok(TargetClaimBinding {
                        entry_id: entry.entry_id.clone(),
                        claim_index: req_idx,
                        claim_sha256: claim_sha.clone(),
                        domain: c.domain.clone(),
                    });
                }
            }
            continue;
        }
        if let Some((idx, c)) = entry.claims.iter().enumerate().find(|(_, c)| {
            (c.lhs == parsed.compare_with || c.rhs == parsed.compare_with)
                && matches!(c.form, ClaimForm::Equation | ClaimForm::Value)
        }) {
            return Ok(TargetClaimBinding {
                entry_id: entry.entry_id.clone(),
                claim_index: idx,
                claim_sha256: claim_sha.clone(),
                domain: c.domain.clone(),
            });
        }
    }
    Err(format!(
        "no active `equation`/`value` claim references `compare_with` handle `{}`; record the step claim first or pass `entry_id`",
        parsed.compare_with
    ))
}

pub(crate) fn load_portable_srepr(
    ledger: &Ledger,
    state: &SolveState,
    handle: &HandleRef,
    label: &str,
) -> Result<String, String> {
    let expr = state
        .handles
        .get(handle)
        .ok_or_else(|| format!("{label} handle `{handle}` not found in session ledger"))?;
    load_handle_srepr(ledger, expr)?
        .ok_or_else(|| format!("{label} handle `{handle}` is not portable"))
}

pub(crate) fn build_blind_prompt(
    setup: &SolveSetup,
    state: &SolveState,
    inputs: &[HandleRef],
    operation: &str,
) -> String {
    let mut input_items = Vec::with_capacity(inputs.len());
    for h in inputs {
        if let Some(expr) = state.handles.get(h) {
            input_items.push(json!({
                "handle": h.to_string(),
                "name": h.name,
                "latex": expr.latex,
                "str_preview": expr.str_preview,
                "free_symbols": expr.free_symbols,
            }));
        }
    }
    json!({
        "conventions": setup.conventions,
        "symbols": setup.symbols,
        "dimensions": setup.dimensions,
        "assumptions": setup.assumptions,
        "inputs": input_items,
        "operation": operation,
    })
    .to_string()
}

pub(crate) fn build_inputs_exprs_map(
    ledger: &Ledger,
    state: &SolveState,
    inputs: &[HandleRef],
) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut dup_names: BTreeSet<String> = BTreeSet::new();
    for h in inputs {
        if !seen_names.insert(h.name.clone()) {
            dup_names.insert(h.name.clone());
        }
    }
    for h in inputs {
        let srepr = load_portable_srepr(ledger, state, h, "input")?;
        map.insert(h.to_string(), srepr.clone());
        if !dup_names.contains(&h.name) {
            map.insert(h.name.clone(), srepr);
        }
    }
    Ok(map)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_host_independent_comparison(
    ctx: &BlindComputeRunCtx<'_>,
    call_id: &str,
    setup: &SolveSetup,
    parsed: &ParsedBlindArgs,
    binding: &TargetClaimBinding,
    compare_with_srepr: &str,
    blind_handle: &HandleRef,
    blind_srepr: &str,
    model_calls_used: u32,
) -> Result<BlindComputeResult, String> {
    let (check_runner, check_epoch, _contained) = ctx.factory.check_runner(ctx.session)?;
    let timeout = ctx.factory.eval_timeout();
    let check_call_id = format!("{call_id}-recompute-check");

    let mut handles = BTreeMap::new();
    handles.insert("lhs".to_string(), compare_with_srepr.to_string());
    handles.insert("rhs".to_string(), blind_srepr.to_string());

    // Guard (b) on `identity_symbolic` MUST run at full precision (never pass user `tolerance`!).
    let mut sym_params = json!({
        "symbols": setup.symbols,
        "compare_with": parsed.compare_with.to_string(),
        "blind_output": blind_handle.to_string(),
    });
    if let Some(ref dom) = binding.domain {
        sym_params["domain"] = json!(dom);
    }
    if let Some(pts) = parsed.points {
        sym_params["points"] = json!(pts);
    }
    if let Some(digs) = parsed.digits {
        sym_params["digits"] = json!(digs);
    }
    let mut num_params = sym_params.clone();
    if let Some(tol) = parsed.tolerance {
        num_params["tolerance"] = json!(tol);
    }

    ctx.ledger
        .append_with_hash(
            ctx.session,
            event_kinds::TOOL_REQUEST,
            EVENT_VERSION,
            &json!({
                "call_id": check_call_id,
                "tool": INDEPENDENT_RECOMPUTE_CHECK,
                "caller": SOLVE_CHECK_CALLER,
                "args": num_params,
            }),
        )
        .map_err(|e| format!("failed to record check tool.request: {e}"))?;

    let record_check_err = |err_msg: &str| {
        let _ = ctx.ledger.append_with_hash(
            ctx.session,
            event_kinds::TOOL_RESULT,
            EVENT_VERSION,
            &json!({
                "call_id": check_call_id,
                "tool": INDEPENDENT_RECOMPUTE_CHECK,
                "ok": false,
                "process": "check",
                "error": err_msg,
            }),
        );
    };

    let (sym_outcome, sym_exact, sym_guard_failed, sym_ev, mut registry_sha) = match check_runner
        .run_check(
            "identity_symbolic",
            &handles,
            &sym_params,
            timeout,
            check_epoch.live_epoch(),
        ) {
        Ok(r) => {
            let outcome = r.raw.get("outcome").and_then(Value::as_str).map(String::from);
            let ev = r.raw.get("evidence").cloned().unwrap_or(Value::Null);
            let exact = ev.get("exact").and_then(Value::as_bool) == Some(true);
            let guard_failed = ev.get("guard_failed").and_then(Value::as_bool) == Some(true);
            let rsha = r
                .raw
                .get("registry_sha256")
                .and_then(Value::as_str)
                .unwrap_or("host:independent_recompute")
                .to_string();
            (outcome, exact, guard_failed, ev, rsha)
        }
        Err(CheckExecError::Timeout(dur)) => (
            Some("inconclusive".to_string()),
            false,
            false,
            json!({ "reason": format!("identity_symbolic timed out after {}s", dur.as_secs()) }),
            "host:independent_recompute".to_string(),
        ),
        Err(e) => {
            let msg = format!("check process `identity_symbolic` failed: {e:?}");
            record_check_err(&msg);
            return Err(msg);
        }
    };

    let exact_match = (sym_outcome.as_deref() == Some("pass") && sym_exact)
        || (sym_outcome.as_deref() == Some("trivial") && parsed.compare_with != *blind_handle);
    let (outcome, comparison, exact, evidence) = if exact_match {
        (
            CheckOutcome::Pass,
            Some("exact".to_string()),
            true,
            json!({
                "comparison": "exact",
                "exact": true,
                "method": "identity_symbolic",
                "compare_with": parsed.compare_with.to_string(),
                "blind_output": blind_handle.to_string(),
                "symbolic_evidence": sym_ev,
            }),
        )
    } else if sym_outcome.as_deref() == Some("fail")
        && (sym_guard_failed || parsed.tolerance.is_none())
    {
        // Guard (b) branch-cut failure ALWAYS fails immediately; non-zero diff without caller tolerance fails.
        (
            CheckOutcome::Fail,
            Some("exact".to_string()),
            false,
            json!({
                "comparison": "exact",
                "exact": false,
                "method": "identity_symbolic",
                "compare_with": parsed.compare_with.to_string(),
                "blind_output": blind_handle.to_string(),
                "symbolic_evidence": sym_ev,
            }),
        )
    } else {
        // Symbolic check was inconclusive (or non-guard diff with bounded caller tolerance) -> numeric fallback.
        let num_res = match check_runner.run_check(
            "identity_numeric",
            &handles,
            &num_params,
            timeout,
            check_epoch.live_epoch(),
        ) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("check process `identity_numeric` failed: {e:?}");
                record_check_err(&msg);
                return Err(msg);
            }
        };
        if let Some(rsha) = num_res.raw.get("registry_sha256").and_then(Value::as_str) {
            registry_sha = rsha.to_string();
        }
        let num_outcome = match num_res.raw.get("outcome").and_then(Value::as_str) {
            Some("pass") => CheckOutcome::Pass,
            Some("fail") => CheckOutcome::Fail,
            _ => CheckOutcome::Inconclusive,
        };
        let num_ev = num_res.raw.get("evidence").cloned().unwrap_or(Value::Null);
        let mut merged = json!({
            "comparison": "numeric",
            "exact": false,
            "method": "identity_numeric",
            "compare_with": parsed.compare_with.to_string(),
            "blind_output": blind_handle.to_string(),
            "symbolic_evidence": sym_ev,
            "numeric_evidence": num_ev,
        });
        for field in ["points", "digits", "tolerance", "max_residual", "counterexample"] {
            if let Some(v) = num_res.raw.get("evidence").and_then(|e| e.get(field)) {
                merged[field] = v.clone();
            }
        }
        (num_outcome, Some("numeric".to_string()), false, merged)
    };

    let res_ev = ctx
        .ledger
        .append_with_hash(
            ctx.session,
            event_kinds::TOOL_RESULT,
            EVENT_VERSION,
            &json!({
                "call_id": check_call_id,
                "tool": INDEPENDENT_RECOMPUTE_CHECK,
                "ok": true,
                "process": "check",
                "result": {
                    "outcome": outcome,
                    "evidence": evidence,
                    "process": "check",
                },
            }),
        )
        .map_err(|e| format!("failed to record check tool.result: {e}"))?;

    let evidence_sha256 = sha256_hex(canonical_json(&evidence).as_bytes());
    let check_id = format!("chk-blind-{}", res_ev.seq);
    let solve_check = SolveCheck {
        check_id: check_id.clone(),
        claim_sha256: binding.claim_sha256.clone(),
        claim_index: binding.claim_index,
        entry_id: Some(binding.entry_id.clone()),
        math_sha256: setup.math_sha256.clone(),
        name: INDEPENDENT_RECOMPUTE_CHECK.into(),
        registry_sha256: registry_sha,
        params: num_params,
        outcome,
        asked_by: CheckRequester::Host,
        receipt: ReceiptRef {
            call_id: check_call_id,
            result_seq: res_ev.seq,
        },
        evidence: evidence.clone(),
        evidence_sha256,
    };
    let check_ev = ctx
        .ledger
        .append_with_hash(
            ctx.session,
            event_kinds::SOLVE_CHECK,
            EVENT_VERSION,
            &serde_json::to_value(&solve_check).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("failed to record solve.check: {e}"))?;

    Ok(BlindComputeResult {
        agreed: outcome == CheckOutcome::Pass,
        outcome,
        comparison,
        exact,
        output: blind_handle.to_string(),
        check_id,
        check_seq: check_ev.seq,
        evidence,
        model_calls_used,
    })
}
