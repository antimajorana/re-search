//! Done-checklist evaluation helpers for `solve_finish` (`solve.md` 5.1, rule R15; package S11).

use crate::solve_model::{
    AckKind, CheckOutcome, CriterionOutcome, DoneCriterion, DoneCriterionKind, ReviewIndependence,
    ReviewVerdict, SolveEntry, SolveReview, SolveSetup, StepEntryType,
};
use crate::solve_state::SolveState;
use crate::solve_status::{badge_for_step, Tier};
use serde_json::{json, Value};

/// Evaluate every [`DoneCriterion`] in `setup.done` against folded ledger state (`solve.md` 5.1, R15).
pub fn evaluate_done_criteria(
    setup: &SolveSetup,
    state: &SolveState,
    final_entry_id: &str,
    closure_ids: &[String],
) -> Result<Vec<CriterionOutcome>, String> {
    let active_closure: Vec<&SolveEntry> = closure_ids
        .iter()
        .filter_map(|id| state.entry(id))
        .filter(|e| e.superseded_by.is_none())
        .collect();
    let mut outcomes = Vec::with_capacity(setup.done.len());
    for c in &setup.done {
        outcomes.push(eval_one_criterion(c, setup, state, final_entry_id, &active_closure)?);
    }
    Ok(outcomes)
}

fn eval_one_criterion(
    c: &DoneCriterion,
    setup: &SolveSetup,
    state: &SolveState,
    final_entry_id: &str,
    active_closure: &[&SolveEntry],
) -> Result<CriterionOutcome, String> {
    match c.kind {
        DoneCriterionKind::FinalTierAtLeast => {
            let (req_tier, req_diff) = parse_tier_requirement(c)?;
            let actual = state.tier(final_entry_id).map(|t| t.effective).unwrap_or(Tier::Unchecked);
            let met = tier_meets(actual, final_entry_id, req_tier, req_diff, state);
            Ok(CriterionOutcome {
                id: c.id.clone(),
                met,
                evidence: json!({
                    "kind": "final_tier_at_least",
                    "final_entry_id": final_entry_id,
                    "required": req_tier,
                    "require_different_model": req_diff,
                    "actual": actual,
                }),
            })
        }
        DoneCriterionKind::ClosureTierAtLeast => {
            let (req_tier, req_diff) = parse_tier_requirement(c)?;
            let failing: Vec<Value> = active_closure
                .iter()
                .filter_map(|entry| {
                    let eff = state.tier(&entry.entry_id).map(|t| t.effective).unwrap_or(Tier::Unchecked);
                    (!tier_meets(eff, &entry.entry_id, req_tier, req_diff, state)).then(|| {
                        json!({ "entry_id": entry.entry_id, "effective": eff })
                    })
                })
                .collect();
            Ok(CriterionOutcome {
                id: c.id.clone(),
                met: failing.is_empty(),
                evidence: json!({
                    "kind": "closure_tier_at_least",
                    "required": req_tier,
                    "require_different_model": req_diff,
                    "checked_count": active_closure.len(),
                    "below_target": failing,
                }),
            })
        }
        DoneCriterionKind::NamedCheckPassed => {
            let check_name = c
                .params
                .get("check")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("criterion `{}` missing `params.check`", c.id))?;
            let on = c.params.get("on").and_then(Value::as_str).unwrap_or("final");
            let targets: Vec<&SolveEntry> = if on == "all_results" {
                let res: Vec<&SolveEntry> = active_closure
                    .iter()
                    .copied()
                    .filter(|e| matches!(e.entry_type, StepEntryType::Result | StepEntryType::FinalAnswer))
                    .collect();
                if res.is_empty() {
                    state.entry(final_entry_id).into_iter().collect()
                } else {
                    res
                }
            } else {
                state.entry(final_entry_id).into_iter().collect()
            };
            let mut passed_ids = Vec::new();
            let mut missing_entries = Vec::new();
            for entry in &targets {
                if let Some(chk_id) = find_passing_named_check(state, entry, check_name, &setup.math_sha256) {
                    passed_ids.push(json!({ "entry_id": entry.entry_id, "check_id": chk_id }));
                } else {
                    missing_entries.push(entry.entry_id.clone());
                }
            }
            Ok(CriterionOutcome {
                id: c.id.clone(),
                met: !targets.is_empty() && missing_entries.is_empty(),
                evidence: json!({
                    "kind": "named_check_passed",
                    "check": check_name,
                    "on": on,
                    "passed": passed_ids,
                    "missing": missing_entries,
                }),
            })
        }
        DoneCriterionKind::NoOpenFlags => {
            let mut flagged = Vec::new();
            for entry in active_closure {
                let mut entry_flags = badge_for_step(state, &entry.entry_id)
                    .map(|b| b.flags)
                    .unwrap_or_default();
                if let Some(t) = state.tier(&entry.entry_id) {
                    if t.stale && !entry_flags.iter().any(|f| f == "Stale") {
                        entry_flags.push("Stale".into());
                    }
                    if t.effective == Tier::Failed && !entry_flags.iter().any(|f| f == "Failed") {
                        entry_flags.push("Failed".into());
                    }
                }
                if let Some(rev) = latest_active_review(state, entry) {
                    if matches!(rev.verdict, ReviewVerdict::Incomplete | ReviewVerdict::Pending) {
                        let v_str = serde_json::to_string(&rev.verdict).unwrap_or_default();
                        entry_flags.push(format!("Review {}", v_str.trim_matches('"')));
                    }
                }
                if !entry_flags.is_empty() {
                    flagged.push(json!({ "entry_id": entry.entry_id, "flags": entry_flags }));
                }
            }
            Ok(CriterionOutcome {
                id: c.id.clone(),
                met: flagged.is_empty(),
                evidence: json!({
                    "kind": "no_open_flags",
                    "open_flags": flagged,
                }),
            })
        }
        DoneCriterionKind::UserConfirms => {
            let text = c.params.get("text").and_then(Value::as_str).unwrap_or("");
            let transcribed: Vec<&SolveEntry> = active_closure
                .iter()
                .copied()
                .filter(|e| e.transcription.is_some())
                .collect();
            let (met, unconfirmed, ack_seqs) = if !transcribed.is_empty() {
                let mut missing = Vec::new();
                let mut seqs = Vec::new();
                for entry in transcribed {
                    let matching = state.acks.iter().rfind(|a| {
                        a.kind == AckKind::Transcription
                            && a.entry_id == entry.entry_id
                            && entry.entry_sha256.as_deref() == Some(a.entry_sha256.as_str())
                    });
                    match matching {
                        Some(ack) => seqs.push(ack.seq),
                        None => missing.push(entry.entry_id.clone()),
                    }
                }
                (missing.is_empty(), missing, seqs)
            } else {
                let matching: Vec<u64> = state
                    .acks
                    .iter()
                    .filter(|a| {
                        active_closure.iter().any(|e| {
                            e.entry_id == a.entry_id
                                && e.entry_sha256.as_deref() == Some(a.entry_sha256.as_str())
                        })
                    })
                    .map(|a| a.seq)
                    .collect();
                let missing = if matching.is_empty() {
                    vec![final_entry_id.to_string()]
                } else {
                    Vec::new()
                };
                (!matching.is_empty(), missing, matching)
            };
            Ok(CriterionOutcome {
                id: c.id.clone(),
                met,
                evidence: json!({
                    "kind": "user_confirms",
                    "text": text,
                    "ack_seqs": ack_seqs,
                    "unconfirmed": unconfirmed,
                }),
            })
        }
    }
}

fn find_passing_named_check<'a>(
    state: &'a SolveState,
    entry: &SolveEntry,
    check_name: &str,
    math_sha256: &str,
) -> Option<&'a str> {
    let claim_sha = entry.claim_sha256.as_deref()?;
    state
        .checks
        .iter()
        .rev()
        .find(|chk| {
            chk.name == check_name
                && chk.outcome == CheckOutcome::Pass
                && chk.math_sha256 == math_sha256
                && chk.claim_sha256 == claim_sha
        })
        .map(|chk| chk.check_id.as_str())
}

fn latest_active_review<'a>(state: &'a SolveState, entry: &SolveEntry) -> Option<&'a SolveReview> {
    state.reviews.iter().rev().find(|r| {
        r.target_entry_id == entry.entry_id
            && (r.entry_sha256.is_none() || r.entry_sha256.as_deref() == entry.entry_sha256.as_deref())
    })
}

fn parse_tier_requirement(c: &DoneCriterion) -> Result<(Tier, bool), String> {
    let raw = c
        .params
        .get("tier")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("criterion `{}` missing `params.tier`", c.id))?;
    let tier = match raw {
        "verified" => Tier::Verified,
        "checked_numerically" => Tier::CheckedNumerically,
        "model_agreed" => Tier::ModelAgreed,
        "checked_by_you" => Tier::CheckedByYou,
        "model_agreed_same_model" => Tier::ModelAgreedSameModel,
        other => return Err(format!("criterion `{}` has unsupported tier `{other}`", c.id)),
    };
    let req_diff = c
        .params
        .get("require_different_model")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((tier, req_diff))
}

fn tier_rank(t: Tier) -> i32 {
    match t {
        Tier::Failed => -1,
        Tier::Stale | Tier::AssumedByModel | Tier::Unchecked => 0,
        Tier::ModelAgreedSameModel => 1,
        Tier::ModelAgreed | Tier::CheckedByYou => 2,
        Tier::CheckedNumerically => 3,
        Tier::Verified | Tier::Given | Tier::Cited => 4,
    }
}

fn tier_meets(
    actual: Tier,
    entry_id: &str,
    required: Tier,
    require_different_model: bool,
    state: &SolveState,
) -> bool {
    if tier_rank(actual) < tier_rank(required) {
        return false;
    }
    if require_different_model && actual == Tier::ModelAgreed {
        let Some(entry) = state.entry(entry_id) else {
            return false;
        };
        if let Some(rev) = state.reviews.iter().rev().find(|r| {
            r.target_entry_id == entry.entry_id
                && r.verdict == ReviewVerdict::Accepted
                && (r.entry_sha256.is_none()
                    || r.entry_sha256.as_deref() == entry.entry_sha256.as_deref())
        }) {
            return !matches!(rev.independence, None | Some(ReviewIndependence::SameModel));
        }
    }
    true
}
