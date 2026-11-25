//! Own-tier evaluation for a single Solve step (`solve.md` 4.4 rules 1, 3–6; rule 2 cut with Lean).
//! Internal helper module for `solve_status.rs`.

use crate::proof_dag::tool_reference_matches;
use crate::solve_badge::{
    wording, Tier, IDENTITY_NUMERIC_MIN_DIGITS, IDENTITY_NUMERIC_MIN_POINTS,
};
use crate::solve_model::{
    compute_claim_sha256, AckKind, CheckOutcome, Claim, ClaimForm, EvidenceQuoteRecord,
    ReviewIndependence, ReviewVerdict, SolveCheck, SolveEntry, SolveReview, StepEntryType,
    StepOrigin,
};
use crate::solve_state::SolveState;
use serde_json::Value;

const NON_IDENTITY_CHECKS: &[&str] = &[
    "dimensions",
    "limit_case",
    "symmetry",
    "conservation",
    "convergence",
    "matrix_property",
];

pub(crate) struct OwnEval {
    pub own: Tier,
    pub sublabels: Vec<String>,
    pub series_flags: Vec<String>,
    pub ocr_source: bool,
    pub own_checks_stale: bool,
    pub has_transcription_ack: bool,
}

fn has_matching_ack(state: &SolveState, entry: &SolveEntry, kind: AckKind) -> bool {
    let Some(ref esh) = entry.entry_sha256 else {
        return false;
    };
    state
        .acks
        .iter()
        .any(|a| a.entry_id == entry.entry_id && a.kind == kind && a.entry_sha256 == *esh)
}

fn latest_matching_review<'a>(
    state: &'a SolveState,
    entry: &SolveEntry,
) -> Option<&'a SolveReview> {
    state
        .reviews
        .iter()
        .filter(|r| {
            r.target_entry_id == entry.entry_id
                && matches!(r.verdict, ReviewVerdict::Accepted | ReviewVerdict::Rejected)
                && match (&entry.entry_sha256, &r.entry_sha256) {
                    (Some(e), Some(rv)) => e == rv,
                    (None, None) => true,
                    _ => false,
                }
        })
        .max_by_key(|r| r.seq)
}

fn matched_cited_quote<'a>(
    state: &'a SolveState,
    entry: &SolveEntry,
) -> Option<&'a EvidenceQuoteRecord> {
    if entry.entry_type != StepEntryType::ReferenceLookup {
        return None;
    }
    let qref = entry.source_ref.as_ref()?;
    let seq = qref.evidence_seq?;
    state.quotes.get(&seq).filter(|rec| rec.matches_ref(qref))
}

fn has_successful_cited_receipt(state: &SolveState, entry: &SolveEntry) -> bool {
    let refs = entry.tool_call_ref.iter().chain(entry.tool_call_refs.iter());
    for r in refs {
        for receipt in state.receipts.values() {
            if receipt.ok && tool_reference_matches(r, &receipt.call_id, receipt.result_seq) {
                return true;
            }
        }
    }
    false
}

/// Returns `(all_handles_portable, handle_redefined_stale)`.
fn handles_status(state: &SolveState, entry: &SolveEntry) -> (bool, bool) {
    let mut all_present = true;
    for claim in &entry.claims {
        for handle in [&claim.lhs, &claim.rhs] {
            match state.handles.get(handle) {
                Some(expr) if !expr.portable => return (false, false),
                Some(_) => {}
                None => all_present = false,
            }
        }
    }
    if all_present && !entry.claims.is_empty() {
        if let Some(ref recorded) = entry.claim_sha256 {
            let recomputed = compute_claim_sha256(&entry.claims, |h| {
                state.handles.get(h).map(|e| e.srepr_sha256.as_str())
            });
            if recomputed.ok().as_deref() != Some(recorded.as_str()) {
                return (true, true);
            }
        }
    }
    (true, false)
}

fn series_order_satisfied(claim: &Claim, check: &SolveCheck) -> bool {
    let Some(ref req) = claim.order else {
        return true;
    };
    let Some(ev_order) = check.evidence.get("order") else {
        return false;
    };
    if let Some(obj) = ev_order.as_object() {
        let var_ok = obj.get("variable").and_then(Value::as_str) == Some(req.variable.as_str());
        let pt_ok = match obj.get("point") {
            Some(Value::String(s)) => s == &req.point,
            Some(Value::Number(n)) => n.to_string() == req.point,
            _ => false,
        };
        let n_ok = obj.get("n").and_then(Value::as_i64).is_some_and(|n| n >= i64::from(req.n));
        return var_ok && pt_ok && n_ok;
    }
    if let Some(n) = ev_order.as_i64() {
        let var_ok = check
            .params
            .get("variable")
            .and_then(Value::as_str)
            .is_none_or(|v| v == req.variable);
        return var_ok && n >= i64::from(req.n);
    }
    false
}

/// Fail-closed check for whether a series or `independent_recompute` check compared
/// coefficients/results symbolically (exact) rather than numerically (`solve.md` 4.4).
fn is_exact_comparison(check: &SolveCheck) -> bool {
    if check.evidence.get("numeric_coefficients").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    check.evidence.get("exact").and_then(Value::as_bool) == Some(true)
        || check.evidence.get("comparison").and_then(Value::as_str) == Some("exact")
}

fn value_two_runs_differ(state: &SolveState, claim: &Claim, check: &SolveCheck) -> bool {
    let code_from_handle = |h| {
        let expr = state.handles.get(h)?;
        let rec = state.receipts.get(&expr.produced_by.call_id)?;
        Some((expr.produced_by.call_id.as_str(), rec.code_sha256.as_deref()?))
    };
    if let (Some((c1, h1)), Some((c2, h2))) =
        (code_from_handle(&claim.lhs), code_from_handle(&claim.rhs))
    {
        return !h1.is_empty() && !h2.is_empty() && c1 != c2 && h1 != h2;
    }
    let h1 = check.evidence.get("lhs_code_sha256").and_then(Value::as_str);
    let h2 = check.evidence.get("rhs_code_sha256").and_then(Value::as_str);
    matches!((h1, h2), (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() && a != b)
}

fn equation_numeric_evidence(check: &SolveCheck) -> Option<(u64, u64)> {
    let points = check.evidence.get("points").and_then(Value::as_u64).unwrap_or(0);
    let digits = check.evidence.get("digits").and_then(Value::as_u64).unwrap_or(0);
    let domain_ok = check.evidence.get("domain_checked").and_then(Value::as_bool) != Some(false);
    (points >= IDENTITY_NUMERIC_MIN_POINTS && digits >= IDENTITY_NUMERIC_MIN_DIGITS && domain_ok)
        .then_some((points, digits))
}

pub(crate) fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

pub(crate) fn evaluate_own(state: &SolveState, entry: &SolveEntry) -> OwnEval {
    let current_math = state.setup.as_ref().map(|s| s.math_sha256.as_str());
    let (all_portable, handle_redefined_stale) = handles_status(state, entry);

    let mut matching_claim_checks: Vec<&SolveCheck> = Vec::new();
    let mut has_stale_claim_check = handle_redefined_stale;

    if let Some(ref c_hash) = entry.claim_sha256 {
        for check in &state.checks {
            if check.claim_sha256 == *c_hash {
                if current_math == Some(check.math_sha256.as_str())
                    && all_portable
                    && !handle_redefined_stale
                {
                    matching_claim_checks.push(check);
                } else if current_math.is_some() && all_portable {
                    has_stale_claim_check = true;
                }
            }
        }
    }

    let own_checks_stale = has_stale_claim_check && matching_claim_checks.is_empty();
    let latest_review = latest_matching_review(state, entry);
    let has_transcription_ack = has_matching_ack(state, entry, AckKind::Transcription);
    let has_hand_checked = has_matching_ack(state, entry, AckKind::HandChecked);
    let has_assumption_ack = has_matching_ack(state, entry, AckKind::Assumption);

    let mut sublabels: Vec<String> = Vec::new();
    let mut series_flags: Vec<String> = Vec::new();
    let mut any_claim_check_failed = false;
    for check in &matching_claim_checks {
        if check.outcome == CheckOutcome::Fail {
            any_claim_check_failed = true;
        } else if check.outcome == CheckOutcome::Pass
            && NON_IDENTITY_CHECKS.contains(&check.name.as_str())
        {
            push_unique(&mut sublabels, check.name.clone());
        }
    }

    let mk = |own: Tier, sublabels: Vec<String>, series_flags: Vec<String>, ocr_source: bool| OwnEval {
        own,
        sublabels,
        series_flags,
        ocr_source,
        own_checks_stale,
        has_transcription_ack,
    };

    // Rule 1: Failed overrides everything and is never cleared by a later passing check.
    // Includes any failed numeric guard on an otherwise symbolically verified equation (R8).
    let review_rejected = latest_review.is_some_and(|r| r.verdict == ReviewVerdict::Rejected);
    if any_claim_check_failed || review_rejected {
        return mk(Tier::Failed, sublabels, series_flags, false);
    }

    // Rule 5 (neutral bases & solver assumptions): Given, Cited, Assumed by model.
    if entry.entry_type == StepEntryType::Assumption {
        let is_given = matches!(
            entry.origin,
            Some(StepOrigin::Setup | StepOrigin::Given | StepOrigin::User)
        ) || has_assumption_ack;
        return mk(
            if is_given { Tier::Given } else { Tier::AssumedByModel },
            sublabels,
            series_flags,
            false,
        );
    }
    if let Some(quote) = matched_cited_quote(state, entry) {
        return mk(Tier::Cited, sublabels, series_flags, quote.ocr);
    }

    // Rule 3: Verified (4) or Checked numerically (3).
    // Each claim gets the tier of its best passing identity check; the step gets the lowest
    // tier among its claims (`solve.md` 4.4 rule 3).
    if let Some(computed_tier) = evaluate_claims_computed_tier(
        state,
        entry,
        &matching_claim_checks,
        all_portable,
        handle_redefined_stale,
        &mut sublabels,
        &mut series_flags,
    ) {
        return mk(computed_tier, sublabels, series_flags, false);
    }

    // Rule 4: Checked by you / Model agreed / Model agreed (same model).
    if has_hand_checked {
        return mk(Tier::CheckedByYou, sublabels, series_flags, false);
    }
    if let Some(review) = latest_review.filter(|r| r.verdict == ReviewVerdict::Accepted) {
        return mk(review_accepted_tier(state, review), sublabels, series_flags, false);
    }

    // Rule 6: Unchecked (with "ran" sublabel for computation step with receipt and no claims).
    if entry.entry_type.is_computation()
        && entry.claims.is_empty()
        && has_successful_cited_receipt(state, entry)
    {
        push_unique(&mut sublabels, wording::SUBLABEL_RAN.to_string());
    }
    mk(Tier::Unchecked, sublabels, series_flags, false)
}

fn evaluate_claims_computed_tier(
    state: &SolveState,
    entry: &SolveEntry,
    matching_checks: &[&SolveCheck],
    all_portable: bool,
    handle_redefined_stale: bool,
    sublabels: &mut Vec<String>,
    series_flags: &mut Vec<String>,
) -> Option<Tier> {
    let eligible = !entry.claims.is_empty()
        && all_portable
        && !handle_redefined_stale
        && entry.claim_sha256.is_some();

    let mut step_tier: Option<Tier> = if eligible { Some(Tier::Verified) } else { None };

    for (idx, claim) in entry.claims.iter().enumerate() {
        let mut best_claim_tier: Option<Tier> = None;

        for check in matching_checks
            .iter()
            .filter(|c| c.claim_index == idx && c.outcome == CheckOutcome::Pass)
        {
            match (claim.form, check.name.as_str()) {
                (ClaimForm::Equation, "identity_symbolic") => {
                    best_claim_tier = best_of(best_claim_tier, Tier::Verified);
                    push_unique(sublabels, wording::SUBLABEL_SYMBOLIC.to_string());
                }
                (ClaimForm::Equation, "identity_numeric") => {
                    if let Some((pts, digs)) = equation_numeric_evidence(check) {
                        best_claim_tier = best_of(best_claim_tier, Tier::CheckedNumerically);
                        push_unique(
                            sublabels,
                            wording::format_numeric_equation_sublabel(pts, digs),
                        );
                    }
                }
                (ClaimForm::Equation | ClaimForm::Value, "independent_recompute") => {
                    let t = if is_exact_comparison(check) {
                        Tier::Verified
                    } else {
                        Tier::CheckedNumerically
                    };
                    best_claim_tier = best_of(best_claim_tier, t);
                    push_unique(sublabels, wording::SUBLABEL_INDEPENDENT_RECOMPUTE.to_string());
                }
                (ClaimForm::Series, "series_match" | "vanishes_to_order")
                    if series_order_satisfied(claim, check) =>
                {
                    let exact = is_exact_comparison(check);
                    let t = if exact { Tier::Verified } else { Tier::CheckedNumerically };
                    best_claim_tier = best_of(best_claim_tier, t);
                    if let Some(ref o) = claim.order {
                        let order_text = wording::format_series_order(&o.variable, o.n);
                        push_unique(series_flags, order_text.clone());
                        let sub = if exact {
                            order_text
                        } else {
                            wording::format_series_numeric_sublabel(&o.variable, o.n)
                        };
                        push_unique(sublabels, sub);
                    } else {
                        let sub = if exact {
                            wording::SUBLABEL_SERIES_FALLBACK
                        } else {
                            wording::SUBLABEL_SERIES_NUMERIC_FALLBACK
                        };
                        push_unique(sublabels, sub.to_string());
                    }
                }
                (ClaimForm::Value, "identity_numeric")
                    if value_two_runs_differ(state, claim, check) =>
                {
                    best_claim_tier = best_of(best_claim_tier, Tier::CheckedNumerically);
                    push_unique(sublabels, wording::SUBLABEL_TWO_RUNS_AGREE.to_string());
                }
                (ClaimForm::Inequality, "counterexample_search") => {
                    // Rule 3 & R8: never grants a computed tier, only adds the sublabel.
                    push_unique(
                        sublabels,
                        wording::SUBLABEL_SAMPLED_NO_COUNTEREXAMPLE.to_string(),
                    );
                }
                _ => {}
            }
        }

        step_tier = match (step_tier, best_claim_tier) {
            (Some(cur), Some(claim_t)) => {
                if claim_t.lattice_rank() < cur.lattice_rank() {
                    Some(claim_t)
                } else {
                    Some(cur)
                }
            }
            _ => None,
        };
    }
    step_tier
}

fn best_of(current: Option<Tier>, candidate: Tier) -> Option<Tier> {
    match current {
        Some(cur) if cur.lattice_rank() >= candidate.lattice_rank() => Some(cur),
        _ => Some(candidate),
    }
}

fn review_accepted_tier(state: &SolveState, review: &SolveReview) -> Tier {
    match review.independence {
        Some(ReviewIndependence::DifferentProvider | ReviewIndependence::DifferentModel) => {
            Tier::ModelAgreed
        }
        Some(ReviewIndependence::SameModel) => Tier::ModelAgreedSameModel,
        None => {
            // Legacy Research review (`solve.md` 8.4 & D19): same model unless different recorded.
            let solver_model = state.setup.as_ref().map(|s| s.roles.solver.model.as_str());
            if review
                .verifier_model
                .as_deref()
                .is_some_and(|vm| solver_model.is_some_and(|sm| vm != sm))
            {
                Tier::ModelAgreed
            } else {
                Tier::ModelAgreedSameModel
            }
        }
    }
}
