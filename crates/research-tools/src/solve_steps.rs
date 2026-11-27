//! Typed `step_record` handler for Solve mode (`solve.md` 5.1, rules R5, R9, R10, R17; package S8).
//!
//! Enforces schema, handle portability & latest-version checks, computation receipt provenance,
//! `depends_on` non-emptiness for derivations/results/final answers, Failed-closure refusal for
//! `final_answer`, F4 `verify_quote_ref` + `lines_sent_with_ledger` on `reference_lookup`, R5 LaTeX
//! rendering strictly from `solve.expr`, host-computed `derived_depends_on`, and atomic hashes.

use crate::latex_repair::repair_under_escaped_latex;
use crate::lines_sent::lines_sent_with_ledger;
use crate::proof_closure::{compute_derived_deps, proper_ancestor_ids};
use crate::proof_dag::tool_reference_matches;
use crate::quote_core::{verify_quote_ref, QuoteRef};
use crate::solve_defs::STEP_RECORD;
use crate::solve_model::{
    compute_claim_sha256, compute_entry_sha256, event_kinds, validate_step_id, Claim, ClaimForm,
    ClaimRelation, HandleRef, PlanItem, SeriesOrder, SolveExpr, StepEntryType, StepOrigin,
};
use crate::solve_receipts::{
    build_receipts, has_successful_sympy_receipt, resolve_cited_receipts,
};
use crate::solve_state::SolveState;
use crate::solve_status::{badge_for_step, Tier};
use crate::ResearchEnv;
use ledger::{Event, Ledger};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tools::{CallCtx, ToolHandler};

const EVENT_SCHEMA_VERSION: u32 = 1;
const MAX_SUMMARY_CHARS: usize = 500;
const MAX_DETAILS_CHARS: usize = 16_000;
const MAX_DOMAIN_HOW_CHARS: usize = 500;
const MAX_PLAN_TITLE_CHARS: usize = 200;
const ALLOWED_TOP_KEYS: &[&str] = &[
    "entry_type",
    "summary",
    "details",
    "depends_on",
    "tool_call_refs",
    "claims",
    "source_ref",
    "plan",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaimInput {
    form: ClaimForm,
    lhs: String,
    rhs: String,
    relation: ClaimRelation,
    #[serde(default)]
    order: Option<SeriesOrder>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    how: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQuoteRefInput {
    source_id: String,
    path: String,
    start_line: usize,
    end_line: usize,
    file_sha256: String,
    quote_sha256: String,
    evidence_seq: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlanItemInput {
    title: String,
}

struct ParsedStepInput {
    entry_type: StepEntryType,
    summary: String,
    details: String,
    depends_on: Vec<String>,
    tool_call_refs: Vec<String>,
    claims: Vec<Claim>,
    source_ref: Option<QuoteRef>,
    plan: Option<Vec<PlanItem>>,
}

/// Real `step_record` handler (`solve.md` 5.1, package S8).
pub fn step_record_handler(env: Arc<ResearchEnv>) -> Arc<dyn ToolHandler> {
    Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        run_step_record(&env, &guard, ctx.session, ctx.caller, args)
    })
}

/// Build a [`crate::tool_profile::SolveCheckHost`] wired with S8's real `step_record` handler.
pub fn solve_check_host_s8(
    check_epoch: Arc<dyn tools::WorkerEpoch>,
    env: Arc<ResearchEnv>,
) -> crate::tool_profile::SolveCheckHost {
    crate::tool_profile::SolveCheckHost::new(check_epoch)
        .with_step_record_handler(step_record_handler(env))
}

/// Execute `step_record` synchronously against `ledger` for `session` and `caller`.
pub fn run_step_record(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    caller: &str,
    args: &Value,
) -> Result<Value, String> {
    let input = parse_step_input(args)?;
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let state = SolveState::from_events(&events);
    let receipts = build_receipts(&events, Some(ledger));

    for dep in &input.depends_on {
        if !state.entry_index.contains_key(dep) {
            return Err(format!("`depends_on` references unknown step `{dep}`"));
        }
    }
    if matches!(
        input.entry_type,
        StepEntryType::Derivation | StepEntryType::Result | StepEntryType::FinalAnswer
    ) && input.depends_on.is_empty()
    {
        return Err(format!(
            "`{}` step requires non-empty `depends_on` (rule R9)",
            input.entry_type.as_str()
        ));
    }

    let cited = resolve_cited_receipts(&receipts, &input.tool_call_refs)?;
    if input.entry_type.is_computation() && !has_successful_sympy_receipt(&cited) {
        return Err(format!(
            "`{}` step requires at least one successful `sympy_eval` receipt in `tool_call_refs` (rule R10)",
            input.entry_type.as_str()
        ));
    }

    let rendered = validate_and_render_claims(&state, &input.claims)?;

    if input.entry_type == StepEntryType::ReferenceLookup {
        let qref = input.source_ref.as_ref().ok_or_else(|| {
            "`reference_lookup` step requires `source_ref` naming a verified `evidence.quote` record (rule R17)"
                .to_string()
        })?;
        verify_reference_quote(env, ledger, session, caller, &events, &state, qref)?;
    } else if let Some(qref) = input.source_ref.as_ref() {
        verify_reference_quote(env, ledger, session, caller, &events, &state, qref)?;
    }

    let entry_id = allocate_next_entry_id(&state);
    let working_handles = handles_with_effective_owners(&state);
    let derived_depends_on = compute_derived_deps(
        &input.claims,
        &input.tool_call_refs,
        &working_handles,
        &state.entries,
        Some(&entry_id),
    );

    if input.entry_type == StepEntryType::FinalAnswer {
        refuse_if_closure_has_failed(&state, &input.depends_on, &derived_depends_on)?;
    }

    let claim_sha256 = if input.claims.is_empty() {
        None
    } else {
        Some(compute_claim_sha256(&input.claims, |h| {
            state.handles.get(h).map(|e| e.srepr_sha256.as_str())
        })?)
    };
    let entry_sha256 =
        compute_entry_sha256(claim_sha256.as_deref(), &input.summary, &input.details);
    let latex = (!rendered.is_empty()).then(|| rendered.join("; "));

    if let Some(items) = input.plan {
        if state.plan.is_none() {
            ledger
                .append_with_hash(session, event_kinds::SOLVE_PLAN, EVENT_SCHEMA_VERSION, &json!({ "items": items }))
                .map_err(|e| format!("failed to write solve.plan: {e}"))?;
        }
    }

    let mut payload = json!({
        "entry_id": entry_id,
        "entry_type": input.entry_type.as_str(),
        "summary": input.summary,
        "details": input.details,
        "depends_on": input.depends_on,
        "derived_depends_on": derived_depends_on,
        "origin": StepOrigin::Solver,
        "entry_sha256": entry_sha256,
    });
    if let Some(lx) = &latex {
        payload["latex"] = json!(lx);
    }
    if !input.tool_call_refs.is_empty() {
        payload["tool_call_refs"] = json!(input.tool_call_refs);
    }
    if !input.claims.is_empty() {
        payload["claims"] = json!(input.claims);
    }
    if let Some(csh) = &claim_sha256 {
        payload["claim_sha256"] = json!(csh);
    }
    if let Some(sref) = &input.source_ref {
        payload["source_ref"] = json!(sref);
    }

    let appended = ledger
        .append_with_hash(session, event_kinds::PROOF_ENTRY, EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| format!("{STEP_RECORD} failed to append proof.entry: {e}"))?;

    let updated_events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let updated_state = SolveState::from_events(&updated_events);
    let badge = badge_for_step(&updated_state, &entry_id);
    let own_tier = updated_state.tier(&entry_id).map(|t| t.own);
    let effective_tier = updated_state.tier(&entry_id).map(|t| t.effective);

    Ok(json!({
        "entry_id": entry_id,
        "seq": appended.seq,
        "entry_type": input.entry_type.as_str(),
        "depends_on": input.depends_on,
        "derived_depends_on": derived_depends_on,
        "rendered": rendered,
        "latex": latex,
        "claim_sha256": claim_sha256,
        "entry_sha256": entry_sha256,
        "own_tier": own_tier,
        "effective_tier": effective_tier,
        "badge": badge,
    }))
}

fn parse_step_input(args: &Value) -> Result<ParsedStepInput, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| format!("{STEP_RECORD} arguments must be a JSON object"))?;
    if obj.contains_key("latex") {
        return Err(
            "step_record never accepts `latex`; formulas are rendered from handles (rule R5)"
                .to_string(),
        );
    }
    for key in obj.keys() {
        if !ALLOWED_TOP_KEYS.contains(&key.as_str()) {
            return Err(format!("unknown field `{key}` in {STEP_RECORD}"));
        }
    }

    let entry_type: StepEntryType = serde_json::from_value(
        obj.get("entry_type")
            .ok_or("`entry_type` is required")?
            .clone(),
    )
    .map_err(|e| format!("invalid `entry_type`: {e}"))?;

    let raw_summary = obj
        .get("summary")
        .and_then(Value::as_str)
        .ok_or("`summary` must be a string")?;
    let summary = repair_under_escaped_latex(raw_summary).trim().to_string();
    if summary.is_empty() {
        return Err("`summary` must not be empty".to_string());
    }
    if summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(format!("`summary` exceeds {MAX_SUMMARY_CHARS} characters"));
    }

    let raw_details = obj
        .get("details")
        .and_then(Value::as_str)
        .ok_or("`details` must be a string")?;
    let details = repair_under_escaped_latex(raw_details);
    if details.chars().count() > MAX_DETAILS_CHARS {
        return Err(format!("`details` exceeds {MAX_DETAILS_CHARS} characters"));
    }

    let depends_on = parse_depends_on(obj.get("depends_on"))?;
    let tool_call_refs = parse_tool_call_refs(obj.get("tool_call_refs"))?;
    let claims = parse_claims(obj.get("claims"))?;
    let source_ref = parse_source_ref(obj.get("source_ref"))?;
    let plan = parse_plan(obj.get("plan"))?;

    Ok(ParsedStepInput {
        entry_type,
        summary,
        details,
        depends_on,
        tool_call_refs,
        claims,
        source_ref,
        plan,
    })
}

fn parse_depends_on(val: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or("`depends_on` must be an array")?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or("`depends_on` entries must be strings")?
            .trim()
            .to_string();
        if s.chars().count() > 64 {
            return Err("`depends_on` step id exceeds 64 characters".to_string());
        }
        validate_step_id(&s)?;
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    Ok(out)
}

fn parse_tool_call_refs(val: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or("`tool_call_refs` must be an array")?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or("`tool_call_refs` entries must be strings")?
            .trim()
            .to_string();
        if s.is_empty() || s.chars().count() > 200 {
            return Err("`tool_call_refs` entries must be 1..=200 characters".to_string());
        }
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    Ok(out)
}

fn parse_claims(val: Option<&Value>) -> Result<Vec<Claim>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or("`claims` must be an array")?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let raw: RawClaimInput = serde_json::from_value(item.clone())
            .map_err(|e| format!("invalid claims[{idx}]: {e}"))?;
        let lhs = HandleRef::parse(raw.lhs.trim())
            .map_err(|e| format!("claims[{idx}].lhs: {e}"))?;
        let rhs = HandleRef::parse(raw.rhs.trim())
            .map_err(|e| format!("claims[{idx}].rhs: {e}"))?;
        let order = match raw.order {
            Some(mut ord) => {
                ord.variable = ord.variable.trim().to_string();
                ord.point = ord.point.trim().to_string();
                if ord.variable.is_empty() || ord.variable.chars().count() > 64 {
                    return Err(format!("claims[{idx}].order.variable must be 1..=64 characters"));
                }
                if ord.point.is_empty() || ord.point.chars().count() > 128 {
                    return Err(format!("claims[{idx}].order.point must be 1..=128 characters"));
                }
                Some(ord)
            }
            None if raw.form == ClaimForm::Series => {
                return Err(format!("claims[{idx}] with form `series` requires `order`"));
            }
            None => None,
        };
        let domain = validate_optional_text(raw.domain, "domain", idx)?;
        let how = validate_optional_text(raw.how, "how", idx)?;
        out.push(Claim {
            form: raw.form,
            lhs,
            rhs,
            relation: raw.relation,
            order,
            domain,
            how,
        });
    }
    Ok(out)
}

fn validate_optional_text(
    raw: Option<String>,
    field: &str,
    idx: usize,
) -> Result<Option<String>, String> {
    let Some(text) = raw else {
        return Ok(None);
    };
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_DOMAIN_HOW_CHARS {
        return Err(format!(
            "claims[{idx}].{field} exceeds {MAX_DOMAIN_HOW_CHARS} characters"
        ));
    }
    Ok(Some(trimmed))
}

fn parse_source_ref(val: Option<&Value>) -> Result<Option<QuoteRef>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let raw: RawQuoteRefInput =
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid `source_ref`: {e}"))?;
    if raw.start_line == 0 || raw.end_line < raw.start_line {
        return Err("`source_ref` must satisfy 1 <= start_line <= end_line".to_string());
    }
    Ok(Some(QuoteRef {
        source_id: raw.source_id,
        path: raw.path,
        start_line: raw.start_line,
        end_line: raw.end_line,
        file_sha256: raw.file_sha256,
        quote_sha256: raw.quote_sha256,
        evidence_seq: Some(raw.evidence_seq),
    }))
}

fn parse_plan(val: Option<&Value>) -> Result<Option<Vec<PlanItem>>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let arr = v.as_array().ok_or("`plan` must be an array")?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let raw: RawPlanItemInput =
            serde_json::from_value(item.clone()).map_err(|e| format!("invalid plan[{idx}]: {e}"))?;
        let title = raw.title.trim().to_string();
        if title.is_empty() || title.chars().count() > MAX_PLAN_TITLE_CHARS {
            return Err(format!("plan[{idx}].title must be 1..={MAX_PLAN_TITLE_CHARS} characters"));
        }
        out.push(PlanItem { title });
    }
    Ok(Some(out))
}

fn validate_and_render_claims(state: &SolveState, claims: &[Claim]) -> Result<Vec<String>, String> {
    let mut latest: BTreeMap<&str, u32> = BTreeMap::new();
    for h in state.handles.keys() {
        latest
            .entry(h.name.as_str())
            .and_modify(|v| *v = (*v).max(h.version))
            .or_insert(h.version);
    }

    let mut rendered = Vec::with_capacity(claims.len());
    for (idx, claim) in claims.iter().enumerate() {
        for (side, href) in [("lhs", &claim.lhs), ("rhs", &claim.rhs)] {
            let expr = state
                .handles
                .get(href)
                .ok_or_else(|| format!("claims[{idx}].{side} names unknown handle `{href}`"))?;
            if !expr.portable {
                return Err(format!(
                    "claims[{idx}].{side} handle `{href}` is non-portable and cannot anchor a claim"
                ));
            }
            if let Some(&max_v) = latest.get(href.name.as_str()) {
                if href.version < max_v {
                    return Err(format!(
                        "claims[{idx}].{side} handle `{href}` is superseded by `{}@{max_v}`",
                        href.name
                    ));
                }
            }
        }
        let lhs_tex = &state.handles[&claim.lhs].latex;
        let rhs_tex = &state.handles[&claim.rhs].latex;
        let line = if claim.form == ClaimForm::Value && claim.lhs == claim.rhs {
            lhs_tex.clone()
        } else {
            let rel = match claim.relation {
                ClaimRelation::Eq => "=",
                ClaimRelation::Le => "\\le",
                ClaimRelation::Ge => "\\ge",
                ClaimRelation::Approx => "\\approx",
            };
            format!("{lhs_tex} {rel} {rhs_tex}")
        };
        rendered.push(line);
    }
    Ok(rendered)
}

fn verify_reference_quote(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    caller: &str,
    events: &[Event],
    state: &SolveState,
    qref: &QuoteRef,
) -> Result<(), String> {
    let seq = qref
        .evidence_seq
        .ok_or_else(|| "`source_ref` requires `evidence_seq`".to_string())?;
    let rec = state
        .quotes
        .get(&seq)
        .filter(|r| r.matches_ref(qref))
        .ok_or_else(|| {
            format!("`source_ref` does not match an `evidence.quote` record at seq {seq}")
        })?;
    let quote_ev = events
        .iter()
        .find(|ev| ev.seq == seq && ev.kind == event_kinds::EVIDENCE_QUOTE)
        .ok_or_else(|| format!("missing `evidence.quote` ledger event at seq {seq}"))?;
    let quote_text = quote_ev
        .payload
        .get("quote")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`evidence.quote` at seq {seq} has no quote text"))?;

    verify_quote_ref(&env.sandbox(), session, events, qref, quote_text)
        .map_err(|fault| format!("`source_ref` quote verification failed: {fault}"))?;

    let sent = lines_sent_with_ledger(events, Some(ledger), None);
    if !sent.covers(
        caller,
        &rec.source_id,
        &rec.path,
        &rec.file_sha256,
        rec.start_line,
        rec.end_line,
    ) {
        return Err(format!(
            "lines {}-{} of `{}` (source `{}`) were never sent to caller `{caller}` (rule R17)",
            rec.start_line, rec.end_line, rec.path, rec.source_id
        ));
    }
    Ok(())
}

fn allocate_next_entry_id(state: &SolveState) -> String {
    let mut next_num = state
        .entries
        .iter()
        .filter_map(|e| e.entry_id.strip_prefix("e-").and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0)
        + 1;
    let mut candidate = format!("e-{next_num}");
    while state.entry_index.contains_key(&candidate) {
        next_num += 1;
        candidate = format!("e-{next_num}");
    }
    candidate
}

fn handles_with_effective_owners(state: &SolveState) -> BTreeMap<HandleRef, SolveExpr> {
    let mut out = state.handles.clone();
    for expr in out.values_mut() {
        if expr.owner_entry.is_some() {
            continue;
        }
        let owner = state.active_entries().into_iter().find(|entry| {
            entry
                .tool_call_ref
                .iter()
                .chain(entry.tool_call_refs.iter())
                .any(|r| {
                    tool_reference_matches(
                        r,
                        &expr.produced_by.call_id,
                        expr.produced_by.result_seq,
                    )
                })
        });
        if let Some(entry) = owner {
            expr.owner_entry = Some(entry.entry_id.clone());
        }
    }
    out
}

fn refuse_if_closure_has_failed(
    state: &SolveState,
    depends_on: &[String],
    derived_depends_on: &[String],
) -> Result<(), String> {
    let mut failed = BTreeSet::new();
    let roots: Vec<&str> = depends_on
        .iter()
        .chain(derived_depends_on.iter())
        .map(String::as_str)
        .collect();
    for root in roots {
        if state
            .tier(root)
            .is_some_and(|t| t.effective == Tier::Failed || t.own == Tier::Failed)
        {
            failed.insert(root.to_string());
        }
        for anc in proper_ancestor_ids(&state.entries, root) {
            if state
                .tier(&anc)
                .is_some_and(|t| t.effective == Tier::Failed || t.own == Tier::Failed)
            {
                failed.insert(anc);
            }
        }
    }
    if !failed.is_empty() {
        let ids: Vec<String> = failed.into_iter().collect();
        return Err(format!(
            "cannot record `final_answer`: closure contains Failed step(s): {}",
            ids.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "solve_steps_tests.rs"]
mod tests;
