//! Finish-time batching, `review_steps` host seam, and review-loop counting for
//! Solve verification (`solve.md` 5.2–5.3, D3; package S14).

use crate::verifier_loop::{
    run_solve_verifier_loop, SolveReviewOutcome, SolveVerifierRunCtx, EVENT_VERIFIER_INCOMPLETE,
    SOLVE_VERIFIER_KIND, SOLVE_VERIFIER_MAX_BATCH_SIZE,
};
use ledger::Event;
use research_tools::proof_closure::{closure_entries, topological_order};
use research_tools::solve_model::{
    compute_review_independence, event_kinds, DoneCriterionKind, ReviewIndependence, ReviewPolicy,
    ReviewVerdict, SolveEntry, SolveSetup,
};
use research_tools::solve_setup::is_same_model;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{review_cites_trivial_check, Tier};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

/// Result of finish-time batched review (`review: prose_only | all | final_only | off`).
#[derive(Debug, Clone, Serialize)]
pub struct FinishReviewSummary {
    pub batches_run: u32,
    pub skipped_due_to_cap: usize,
    pub outcomes: Vec<SolveReviewOutcome>,
    pub verification_stuck: Option<Value>,
}

/// Count prior Solve verifier loops in `events` to enforce `budgets.max_review_loops` (`solve.md` 5.2).
pub fn count_review_loops(events: &[Event]) -> u32 {
    let mut batches: BTreeSet<String> = BTreeSet::new();
    for ev in events {
        if ev.payload.get("kind").and_then(Value::as_str) != Some(SOLVE_VERIFIER_KIND) {
            continue;
        }
        if ev.kind != EVENT_VERIFIER_INCOMPLETE && ev.kind != event_kinds::PROOF_VERIFICATION {
            continue;
        }
        if let Some(bid) = ev
            .payload
            .get("batch_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            batches.insert(bid.to_string());
        } else if ev.kind == EVENT_VERIFIER_INCOMPLETE {
            batches.insert(format!("inc-{}", ev.seq));
        } else {
            let key = ev
                .payload
                .get("raw_blob")
                .and_then(Value::as_str)
                .map(|b| format!("blob-{b}"))
                .unwrap_or_else(|| format!("seq-{}", ev.seq));
            batches.insert(key);
        }
    }
    u32::try_from(batches.len()).unwrap_or(u32::MAX)
}

fn independence_rank(ind: ReviewIndependence) -> u8 {
    match ind {
        ReviewIndependence::SameModel => 1,
        ReviewIndependence::DifferentModel => 2,
        ReviewIndependence::DifferentProvider => 3,
    }
}

fn has_valid_accepted_review_at(
    state: &SolveState,
    entry: &SolveEntry,
    min_independence: ReviewIndependence,
) -> bool {
    let min_rank = independence_rank(min_independence);
    state.reviews.iter().any(|r| {
        r.target_entry_id == entry.entry_id
            && r.verdict == ReviewVerdict::Accepted
            && match (&entry.entry_sha256, &r.entry_sha256) {
                (Some(e), Some(rv)) => e == rv,
                (None, None) => true,
                _ => false,
            }
            && r.independence
                .map(independence_rank)
                .unwrap_or(1)
                >= min_rank
            && !review_cites_trivial_check(state, r)
    })
}

/// Compute target step ids that still need review at finish time (`solve.md` 5.3, D3).
pub fn select_finish_review_targets(state: &SolveState, final_entry_id: &str) -> Vec<String> {
    let Some(setup) = state.setup.as_ref() else {
        return Vec::new();
    };
    if setup.review == ReviewPolicy::Off {
        return Vec::new();
    }
    let closure = closure_entries(&state.entries, &[final_entry_id]);
    let ordered_ids: Vec<String> = match topological_order(&closure) {
        Ok(ids) => ids,
        Err(_) => closure.iter().map(|e| e.entry_id.clone()).collect(),
    };
    let (closure_req, final_req) = required_review_ranks(setup);
    let same_model = is_same_model(Some(&setup.roles.solver), Some(&setup.roles.verifier));
    let current_independence = if same_model {
        ReviewIndependence::SameModel
    } else {
        compute_review_independence(&setup.roles.solver, &setup.roles.verifier)
    };
    let max_review_lift = if same_model {
        Tier::ModelAgreedSameModel.ordered_rank().unwrap_or(1)
    } else {
        Tier::ModelAgreed.ordered_rank().unwrap_or(2)
    };

    let mut out = Vec::new();
    for id in ordered_ids {
        let Some(entry) = state.entry(&id) else {
            continue;
        };
        if entry.superseded_by.is_some() {
            continue;
        }
        if setup.review == ReviewPolicy::FinalOnly && entry.entry_id != final_entry_id {
            continue;
        }
        let own = state.tier(&id).map_or(Tier::Unchecked, |t| t.own);
        // Under ProseOnly, skip claim-bearing steps ONLY when they already earned a computed tier.
        if setup.review == ReviewPolicy::ProseOnly && !entry.claims.is_empty() && own.is_computed() {
            continue;
        }
        // Verifier cannot lift Given, Cited, Failed, or AssumedByModel (only user ack can).
        if matches!(
            own,
            Tier::Given | Tier::Cited | Tier::Failed | Tier::AssumedByModel
        ) {
            continue;
        }
        // Skip entries that already have a valid Accepted review for their current entry_sha256
        // at the verifier's current independence (unless that review cited trivial checks).
        if has_valid_accepted_review_at(state, entry, current_independence) {
            continue;
        }
        let required = if entry.entry_id == final_entry_id {
            closure_req.max(final_req)
        } else {
            closure_req
        };
        let effective_goal = if entry.claims.is_empty() || same_model {
            required.min(max_review_lift)
        } else {
            required
        };
        if own.ordered_rank().unwrap_or(0) < effective_goal {
            out.push(id);
        }
    }
    out
}

fn parse_tier_rank(tier_str: &str) -> Option<u8> {
    match tier_str {
        "model_agreed_same_model" => Some(1),
        "model_agreed" | "checked_by_you" => Some(2),
        "checked_numerically" | "numerically_checked" => Some(3),
        "verified" => Some(4),
        _ => None,
    }
}

fn required_review_ranks(setup: &SolveSetup) -> (u8, u8) {
    let default_rank = Tier::ModelAgreed.ordered_rank().unwrap_or(2);
    let mut closure_rank: Option<u8> = None;
    let mut final_rank: Option<u8> = None;
    for c in &setup.done {
        let Some(rank) = c
            .params
            .get("tier")
            .and_then(Value::as_str)
            .and_then(parse_tier_rank)
        else {
            continue;
        };
        match c.kind {
            DoneCriterionKind::ClosureTierAtLeast => {
                closure_rank = Some(closure_rank.map_or(rank, |cur| cur.max(rank)));
            }
            DoneCriterionKind::FinalTierAtLeast => {
                final_rank = Some(final_rank.map_or(rank, |cur| cur.max(rank)));
            }
            _ => {}
        }
    }
    let cr = closure_rank.unwrap_or(default_rank);
    let fr = final_rank.unwrap_or(cr);
    (cr, fr)
}

/// Chunk target step ids into verifier batches of at most 5 steps (`solve.md` 5.2 & 5.3).
pub fn batch_review_targets(targets: &[String]) -> Vec<Vec<String>> {
    targets
        .chunks(SOLVE_VERIFIER_MAX_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

/// Host-handled `review_steps { entry_ids: [1..5] }` seam for `solve_loop.rs` (S12 + S14).
pub fn handle_review_steps(ctx: &SolveVerifierRunCtx<'_>, args: &Value) -> Result<Value, String> {
    let raw_ids = args
        .get("entry_ids")
        .and_then(Value::as_array)
        .ok_or("`entry_ids` must be an array of 1 to 5 step IDs")?;
    let mut entry_ids = Vec::with_capacity(raw_ids.len());
    for v in raw_ids {
        let s = v
            .as_str()
            .ok_or("each item in `entry_ids` must be a string")?
            .trim();
        entry_ids.push(s.to_string());
    }
    let events = ctx.ledger.read_session(ctx.session).map_err(|e| e.to_string())?;
    let state = SolveState::from_events(&events);
    let max_loops = state.setup.as_ref().map(|s| s.budgets.max_review_loops).unwrap_or(6);
    let used_loops = count_review_loops(&events);
    if used_loops >= max_loops {
        return Err(format!(
            "review loop cap reached ({used_loops} / {max_loops}); cannot run another `review_steps` batch"
        ));
    }
    let outcome = run_solve_verifier_loop(ctx, &entry_ids)?;
    serde_json::to_value(&outcome).map_err(|e| e.to_string())
}

/// Run finish-time review batches according to `solve.setup.review` (`prose_only` default,
/// `all`, `final_only`, `off`), stopping early on `max_review_loops` or G6 `verification_stuck`.
pub fn run_finish_review_batches(
    ctx: &SolveVerifierRunCtx<'_>,
    final_entry_id: &str,
) -> Result<FinishReviewSummary, String> {
    let events = ctx.ledger.read_session(ctx.session).map_err(|e| e.to_string())?;
    let state = SolveState::from_events(&events);
    let max_loops = state.setup.as_ref().map(|s| s.budgets.max_review_loops).unwrap_or(6);
    let mut used_loops = count_review_loops(&events);
    let targets = select_finish_review_targets(&state, final_entry_id);
    let batches = batch_review_targets(&targets);

    let mut outcomes = Vec::new();
    let mut skipped_due_to_cap = 0usize;
    let mut verification_stuck = None;

    for batch in batches {
        if used_loops >= max_loops {
            skipped_due_to_cap += batch.len();
            continue;
        }
        let outcome = run_solve_verifier_loop(ctx, &batch)?;
        used_loops += 1;
        if let SolveReviewOutcome::Completed {
            verification_stuck: Some(ref stuck),
            ..
        } = outcome
        {
            verification_stuck = Some(stuck.clone());
            outcomes.push(outcome);
            break;
        }
        outcomes.push(outcome);
    }

    Ok(FinishReviewSummary {
        batches_run: u32::try_from(outcomes.len()).unwrap_or(u32::MAX),
        skipped_due_to_cap,
        outcomes,
        verification_stuck,
    })
}
