//! Own-tier evaluation for a single Solve step (`solve.md` 4.4 rules 1–6).
//! Internal helper module for `solve_status.rs`.

use crate::proof_dag::tool_reference_matches;
use crate::solve_badge::{
    wording, Tier, FORMAL_CHECK_NAME, IDENTITY_NUMERIC_MIN_DIGITS, IDENTITY_NUMERIC_MIN_POINTS,
    STANDARD_LEAN_AXIOMS,
};
use crate::solve_model::{
    compute_claim_sha256, AckKind, CheckOutcome, Claim, ClaimForm, ReviewIndependence,
    ReviewVerdict, SolveCheck, SolveEntry, SolveReview, StepEntryType, StepOrigin,
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
    pub used_numeric_only: bool,
    pub own_checks_stale: bool,
    pub has_transcription_ack: bool,
    pub has_formal_stmt_ack: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormalOutcome {
    Verified,
    Rejected,
    Other,
}

fn classify_formal_check(check: &SolveCheck) -> FormalOutcome {
    let status = check.evidence.get("status").and_then(Value::as_str);
    if check.outcome == CheckOutcome::Fail || status == Some("rejected") {
        return FormalOutcome::Rejected;
    }
    if status == Some("verified") {
        let Some(axioms) = check.evidence.get("axioms").and_then(Value::as_array) else {
            // R14: exit 0 / verified without axiom confirmation is rejected.
            return FormalOutcome::Rejected;
        };
        let all_standard = axioms
            .iter()
            .all(|a| a.as_str().is_some_and(|name| STANDARD_LEAN_AXIOMS.contains(&name)));
        if all_standard && check.outcome == CheckOutcome::Pass {
            return FormalOutcome::Verified;
        }
        // R14: sorryAx or any non-standard axiom is rejected.
        return FormalOutcome::Rejected;
    }
    FormalOutcome::Other
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

fn is_cited_entry(state: &SolveState, entry: &SolveEntry) -> bool {
    if entry.entry_type != StepEntryType::ReferenceLookup {
        return false;
    }
    let Some(ref qref) = entry.source_ref else {
        return false;
    };
    let Some(seq) = qref.evidence_seq else {
        return false;
    };
    state.quotes.get(&seq).is_some_and(|rec| rec.matches_ref(qref))
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

fn check_matches_formal_binding(check: &SolveCheck, entry: &SolveEntry) -> bool {
    if let Some(ref c_esh) = check.entry_sha256 {
        return entry.entry_sha256.as_deref() == Some(c_esh.as_str());
    }
    if check.claim_sha256.is_empty() {
        return false;
    }
    if entry.claim_sha256.as_deref() == Some(check.claim_sha256.as_str()) {
        return true;
    }
    entry.claims.is_empty() && entry.entry_sha256.as_deref() == Some(check.claim_sha256.as_str())
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

fn equation_numeric_qualifies(check: &SolveCheck) -> bool {
    let points = check.evidence.get("points").and_then(Value::as_u64).unwrap_or(0);
    let digits = check.evidence.get("digits").and_then(Value::as_u64).unwrap_or(0);
    let domain_ok = check.evidence.get("domain_checked").and_then(Value::as_bool) != Some(false);
    points >= IDENTITY_NUMERIC_MIN_POINTS && digits >= IDENTITY_NUMERIC_MIN_DIGITS && domain_ok
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
    let mut matching_formal_checks: Vec<&SolveCheck> = Vec::new();
    let mut has_stale_claim_check = handle_redefined_stale;
    let mut has_stale_formal_check = false;

    for check in &state.checks {
        if check.name == FORMAL_CHECK_NAME {
            if check_matches_formal_binding(check, entry) {
                if current_math == Some(check.math_sha256.as_str()) {
                    matching_formal_checks.push(check);
                } else if current_math.is_some() {
                    has_stale_formal_check = true;
                }
            }
            continue;
        }
        if let Some(ref c_hash) = entry.claim_sha256 {
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

    let own_checks_stale = (has_stale_claim_check && matching_claim_checks.is_empty())
        || (has_stale_formal_check && matching_formal_checks.is_empty());

    let latest_review = latest_matching_review(state, entry);
    let has_transcription_ack = has_matching_ack(state, entry, AckKind::Transcription);
    let has_formal_stmt_ack = has_matching_ack(state, entry, AckKind::FormalStatement);
    let has_hand_checked = has_matching_ack(state, entry, AckKind::HandChecked);
    let has_assumption_ack = has_matching_ack(state, entry, AckKind::Assumption);

    let mut sublabels: Vec<String> = Vec::new();
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

    let any_formal_rejected = matching_formal_checks
        .iter()
        .any(|c| classify_formal_check(c) == FormalOutcome::Rejected);
    let any_formal_verified = matching_formal_checks
        .iter()
        .any(|c| classify_formal_check(c) == FormalOutcome::Verified);

    let mk = |own: Tier, sublabels: Vec<String>, used_numeric_only: bool| OwnEval {
        own,
        sublabels,
        used_numeric_only,
        own_checks_stale,
        has_transcription_ack,
        has_formal_stmt_ack,
    };

    // Rule 1: Failed overrides everything and is never cleared by a later passing check.
    let review_rejected = latest_review.is_some_and(|r| r.verdict == ReviewVerdict::Rejected);
    if any_claim_check_failed || any_formal_rejected || review_rejected {
        return mk(Tier::Failed, sublabels, false);
    }

    // Rule 5 (neutral bases & solver assumptions): Given, Cited, Assumed by model.
    if entry.entry_type == StepEntryType::Assumption {
        let is_given = matches!(
            entry.origin,
            Some(StepOrigin::Setup | StepOrigin::Given | StepOrigin::User)
        ) || has_assumption_ack;
        return mk(if is_given { Tier::Given } else { Tier::AssumedByModel }, sublabels, false);
    }
    if is_cited_entry(state, entry) {
        return mk(Tier::Cited, sublabels, false);
    }

    // Rule 2: Lean-checked.
    if any_formal_verified {
        return mk(Tier::LeanChecked, sublabels, false);
    }

    // Rule 3: Computed — every claim must have a passing identity-class check.
    let (all_claims_computed, used_numeric_only) = evaluate_claims_computed(
        state,
        entry,
        &matching_claim_checks,
        all_portable,
        handle_redefined_stale,
        &mut sublabels,
    );
    if all_claims_computed {
        return mk(Tier::Computed, sublabels, used_numeric_only);
    }

    // Rule 4: Checked by you / Model agreed / Model agreed (same model).
    if has_hand_checked {
        return mk(Tier::CheckedByYou, sublabels, false);
    }
    if let Some(review) = latest_review.filter(|r| r.verdict == ReviewVerdict::Accepted) {
        let tier = review_accepted_tier(state, review);
        return mk(tier, sublabels, false);
    }

    // Rule 6: Unchecked (with "ran" sublabel for computation step with receipt and no claims).
    if entry.entry_type.is_computation()
        && entry.claims.is_empty()
        && has_successful_cited_receipt(state, entry)
    {
        push_unique(&mut sublabels, wording::SUBLABEL_RAN.to_string());
    }
    mk(Tier::Unchecked, sublabels, false)
}

fn evaluate_claims_computed(
    state: &SolveState,
    entry: &SolveEntry,
    matching_checks: &[&SolveCheck],
    all_portable: bool,
    handle_redefined_stale: bool,
    sublabels: &mut Vec<String>,
) -> (bool, bool) {
    let mut all_computed = !entry.claims.is_empty()
        && all_portable
        && !handle_redefined_stale
        && entry.claim_sha256.is_some();
    let mut used_numeric_only = false;

    for (idx, claim) in entry.claims.iter().enumerate() {
        let mut claim_passed = false;
        let mut claim_strong = false;
        let mut claim_numeric = false;

        for check in matching_checks
            .iter()
            .filter(|c| c.claim_index == idx && c.outcome == CheckOutcome::Pass)
        {
            match (claim.form, check.name.as_str()) {
                (ClaimForm::Equation, "identity_symbolic") => {
                    claim_passed = true;
                    claim_strong = true;
                    push_unique(sublabels, wording::SUBLABEL_SYMBOLIC.to_string());
                }
                (ClaimForm::Equation, "identity_numeric") if equation_numeric_qualifies(check) => {
                    claim_passed = true;
                    claim_numeric = true;
                    push_unique(sublabels, wording::SUBLABEL_NUMERIC_ONLY.to_string());
                }
                (ClaimForm::Equation | ClaimForm::Value, "independent_recompute") => {
                    claim_passed = true;
                    claim_strong = true;
                    push_unique(sublabels, wording::SUBLABEL_INDEPENDENT_RECOMPUTE.to_string());
                }
                (ClaimForm::Series, "series_match" | "vanishes_to_order")
                    if series_order_satisfied(claim, check) =>
                {
                    claim_passed = true;
                    let sub = match &claim.order {
                        Some(o) => wording::format_series_sublabel(&o.variable, o.n),
                        None => wording::SUBLABEL_SERIES_FALLBACK.to_string(),
                    };
                    push_unique(sublabels, sub);
                }
                (ClaimForm::Value, "identity_numeric")
                    if value_two_runs_differ(state, claim, check) =>
                {
                    claim_passed = true;
                    push_unique(sublabels, wording::SUBLABEL_TWO_RUNS_AGREE.to_string());
                }
                (ClaimForm::Inequality, "counterexample_search") => {
                    // Rule 3 & R8: never grants Computed, only adds the sublabel.
                    push_unique(
                        sublabels,
                        wording::SUBLABEL_SAMPLED_NO_COUNTEREXAMPLE.to_string(),
                    );
                }
                _ => {}
            }
        }
        if claim_numeric && !claim_strong {
            used_numeric_only = true;
        }
        if !claim_passed {
            all_computed = false;
        }
    }
    (all_computed, used_numeric_only)
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
