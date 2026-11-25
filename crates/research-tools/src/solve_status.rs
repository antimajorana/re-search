//! Solve mode effective-tier propagation and badge API (package S2).
//!
//! Combines own-tier evaluation (`solve_own_tier`), monotone effective-tier propagation
//! over declared plus derived edges (`proof_closure`), stale/fail rules, flags, and the
//! single badge function (`solve_badge::badge_for_step`). Pure code, no I/O.

use crate::proof_closure::{all_deps, proper_ancestor_ids, topological_order};
pub use crate::solve_badge::*;
use crate::solve_model::SolveEntry;
use crate::solve_own_tier::{evaluate_own, push_unique, OwnEval};
use crate::solve_state::SolveState;
use std::collections::BTreeMap;

/// Compute own and effective tiers for every step in `state` over `depends_on ∪ derived_depends_on`.
pub fn compute_all_tiers(state: &SolveState) -> BTreeMap<String, StepTier> {
    let mut own_map: BTreeMap<String, OwnEval> = BTreeMap::new();
    for entry in &state.entries {
        own_map.insert(entry.entry_id.clone(), evaluate_own(state, entry));
    }

    let topo = topological_order(&state.entries);
    let has_cycle = topo.is_err();
    let order: Vec<String> = match topo {
        Ok(ids) => ids,
        Err(cyclic) => {
            let mut ids: Vec<String> = state
                .entries
                .iter()
                .map(|e| e.entry_id.clone())
                .filter(|id| !cyclic.contains(id))
                .collect();
            ids.extend(cyclic);
            ids
        }
    };

    let mut result: BTreeMap<String, StepTier> = BTreeMap::new();
    for id in &order {
        let Some(&idx) = state.entry_index.get(id) else {
            continue;
        };
        let tier = propagate_step(state, &state.entries[idx], &own_map[id], &result);
        result.insert(id.clone(), tier);
    }

    // Fixed-point relaxation in case a malformed ledger contained cycles.
    if has_cycle {
        for _ in 0..state.entries.len() {
            let mut changed = false;
            for id in &order {
                let Some(&idx) = state.entry_index.get(id) else {
                    continue;
                };
                let updated = propagate_step(state, &state.entries[idx], &own_map[id], &result);
                if result.get(id) != Some(&updated) {
                    result.insert(id.clone(), updated);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    attach_flags(state, &own_map, &mut result);
    result
}

fn propagate_step(
    state: &SolveState,
    entry: &SolveEntry,
    eval: &OwnEval,
    resolved: &BTreeMap<String, StepTier>,
) -> StepTier {
    let deps = all_deps(entry);
    let mut min_rank = eval.own.lattice_rank();
    let mut dep_stale = false;
    let mut broken_by: Vec<String> = Vec::new();
    let mut dep_ranks: Vec<(&str, i8, Tier, bool)> = Vec::with_capacity(deps.len());

    for dep_id in deps {
        let Some(&dep_idx) = state.entry_index.get(dep_id) else {
            // Missing dependency: fail-closed to Unchecked (rank 0).
            min_rank = min_rank.min(0);
            dep_ranks.push((dep_id, 0, Tier::Unchecked, false));
            continue;
        };
        let dep_entry = &state.entries[dep_idx];
        let dep_superseded = dep_entry.superseded_by.is_some();
        let dep_tier = resolved.get(dep_id);
        let dep_eff = dep_tier.map_or(Tier::Unchecked, |t| t.effective);
        let dep_is_stale = dep_superseded || dep_tier.is_some_and(|t| t.stale);

        if dep_eff == Tier::Failed {
            if dep_tier.is_some_and(|t| t.own == Tier::Failed) {
                push_unique(&mut broken_by, dep_id.to_string());
            } else if let Some(t) = dep_tier {
                if t.broken_by.is_empty() {
                    push_unique(&mut broken_by, dep_id.to_string());
                } else {
                    for root in &t.broken_by {
                        push_unique(&mut broken_by, root.clone());
                    }
                }
            }
        }
        if dep_is_stale {
            dep_stale = true;
        }
        let eff_rank = if dep_eff == Tier::Failed {
            -1
        } else if dep_is_stale {
            0
        } else {
            dep_eff.lattice_rank()
        };
        min_rank = min_rank.min(eff_rank);
        dep_ranks.push((dep_id, eff_rank, dep_eff, dep_is_stale));
    }

    let stale = eval.own_checks_stale || dep_stale;
    let effective = if min_rank == -1 {
        Tier::Failed
    } else if stale {
        Tier::Stale
    } else {
        match min_rank {
            5 => eval.own,
            4 => Tier::LeanChecked,
            3 => Tier::Computed,
            2 => {
                let any_model_agreed = eval.own == Tier::ModelAgreed
                    || dep_ranks.iter().any(|(_, r, t, _)| *r == 2 && *t == Tier::ModelAgreed);
                if any_model_agreed {
                    Tier::ModelAgreed
                } else {
                    Tier::CheckedByYou
                }
            }
            1 => Tier::ModelAgreedSameModel,
            _ => {
                if eval.own == Tier::AssumedByModel && dep_ranks.iter().all(|(_, r, _, _)| *r > 0) {
                    Tier::AssumedByModel
                } else {
                    Tier::Unchecked
                }
            }
        }
    };

    let own_rank = eval.own.lattice_rank();
    let eff_rank = effective.lattice_rank();
    let lowered_by: Vec<String> = if eff_rank < own_rank {
        dep_ranks
            .iter()
            .filter(|(_, r, _, _)| *r == eff_rank)
            .map(|(id, _, _, _)| (*id).to_string())
            .collect()
    } else if dep_stale && effective == Tier::Stale {
        dep_ranks
            .iter()
            .filter(|(_, _, _, s)| *s)
            .map(|(id, _, _, _)| (*id).to_string())
            .collect()
    } else {
        Vec::new()
    };

    // A step that failed on its own does not report `broken_by` deps.
    if eval.own == Tier::Failed {
        broken_by.clear();
    }

    StepTier {
        own: eval.own,
        effective,
        lowered_by,
        broken_by,
        sublabels: eval.sublabels.clone(),
        flags: Vec::new(),
        stale,
    }
}

fn attach_flags(
    state: &SolveState,
    own_map: &BTreeMap<String, OwnEval>,
    result: &mut BTreeMap<String, StepTier>,
) {
    for entry in &state.entries {
        let id = &entry.entry_id;
        let eval = &own_map[id];
        let model_assumptions_count = proper_ancestor_ids(&state.entries, id)
            .into_iter()
            .filter(|anc| own_map.get(anc).is_some_and(|e| e.own == Tier::AssumedByModel))
            .count();
        if let Some(step_tier) = result.get_mut(id) {
            let mut flags = Vec::new();
            if entry.transcription.is_some() && !eval.has_transcription_ack {
                flags.push(wording::FLAG_TRANSCRIPTION_UNCONFIRMED.to_string());
            }
            if eval.own == Tier::LeanChecked && !eval.has_formal_stmt_ack {
                flags.push(wording::FLAG_STATEMENT_MATCH_UNCONFIRMED.to_string());
            }
            if eval.own == Tier::ModelAgreedSameModel
                || step_tier.effective == Tier::ModelAgreedSameModel
            {
                flags.push(wording::FLAG_SAME_MODEL.to_string());
            }
            if eval.own == Tier::Computed && eval.used_numeric_only {
                flags.push(wording::FLAG_NUMERIC_ONLY.to_string());
            }
            if model_assumptions_count > 0 {
                flags.push(wording::format_model_assumptions_flag(model_assumptions_count));
            }
            step_tier.flags = flags;
        }
    }
}

#[cfg(test)]
#[path = "solve_status_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "solve_status_props_tests.rs"]
mod prop_tests;
