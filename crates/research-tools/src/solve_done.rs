//! Host-evaluated `solve_finish` handler, G7 closure guard, and Solve-tier-aware R22 wording lint
//! (`solve.md` 5.1, 5.4, rules R15, R22; package S11).

#[path = "solve_done_criteria.rs"]
mod criteria;

pub use self::criteria::evaluate_done_criteria;

use crate::proof_closure::closure_for_target;
use crate::run_budget::{count_model_calls_breakdown, RunScope};
use crate::solve_model::{event_kinds, ModelCallsBreakdown, SolveDoneStatus};
use crate::solve_state::SolveState;
use crate::solve_status::{badge_for_step, rests_on_summary, Tier};
use crate::ResearchEnv;
use ledger::Ledger;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use tools::{CallCtx, ToolHandler};

const EVENT_SCHEMA_VERSION: u32 = 1;
const MAX_SUMMARY_CHARS: usize = 16_000;
const MAX_OPEN_ITEM_CHARS: usize = 500;
const ALLOWED_FINISH_KEYS: &[&str] = &["outcome", "final_entry_id", "summary_markdown", "open_items"];

/// Real `solve_finish` handler (`solve.md` 5.1, rules R15 & R22; package S11).
pub fn solve_finish_handler(env: Arc<ResearchEnv>) -> Arc<dyn ToolHandler> {
    Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        run_solve_finish(&env, &guard, ctx.session, ctx.caller, args)
    })
}

/// Execute `solve_finish` synchronously against `ledger` for `session`.
pub fn run_solve_finish(
    _env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    _caller: &str,
    args: &Value,
) -> Result<Value, String> {
    let parsed = parse_finish_args(args)?;
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let state = SolveState::from_events(&events);
    let setup = state
        .setup
        .as_ref()
        .ok_or_else(|| "cannot run `solve_finish` before `solve.setup` has been recorded".to_string())?;

    let closure_ids = validate_solve_finish_preconditions(
        &state,
        &parsed.final_entry_id,
        &parsed.summary_markdown,
    )?;

    let criteria = evaluate_done_criteria(setup, &state, &parsed.final_entry_id, &closure_ids)?;
    let all_met = criteria.iter().all(|c| c.met);
    // Rule R15: host computes authoritative `status` strictly from ledger criteria.
    let status = if all_met {
        SolveDoneStatus::Finished
    } else {
        SolveDoneStatus::FinishedOpenItems
    };

    let final_tier = state
        .tier(&parsed.final_entry_id)
        .ok_or_else(|| format!("missing tier for `{}`", parsed.final_entry_id))?;
    let rests_on = rests_on_summary(&state, &parsed.final_entry_id);
    let mc = count_model_calls_breakdown(&events, RunScope::All);
    let model_calls = ModelCallsBreakdown {
        solver: u32::try_from(mc.main_loop).unwrap_or(u32::MAX),
        verifier: u32::try_from(mc.verifier).unwrap_or(u32::MAX),
        workers: u32::try_from(mc.delegated).unwrap_or(u32::MAX),
    };

    let mut payload = json!({
        "final_entry_id": parsed.final_entry_id,
        "own_tier": final_tier.own,
        "effective_tier": final_tier.effective,
        "criteria": criteria,
        "rests_on": rests_on,
        "status": status,
        "model_calls": model_calls,
        "summary_markdown": parsed.summary_markdown,
        "open_items": parsed.open_items,
    });
    if let Some(proposed) = parsed.proposed_outcome {
        payload["model_proposed_outcome"] = json!(proposed);
    }

    let appended = ledger
        .append_with_hash(session, event_kinds::SOLVE_DONE, EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| format!("failed to record `solve.done`: {e}"))?;

    let mut out = payload;
    out["seq"] = json!(appended.seq);
    if let Some(badge) = badge_for_step(&state, &parsed.final_entry_id) {
        out["badge"] = json!(badge);
    }
    Ok(out)
}

/// Shared Gate G7 validation: refuse `solve_finish` if closure contains `Failed`/`Stale` steps
/// or if `summary_markdown` violates Solve R22 wording rules. Returns the closure id list.
pub fn validate_solve_finish_preconditions(
    state: &SolveState,
    final_entry_id: &str,
    summary_markdown: &str,
) -> Result<Vec<String>, String> {
    if state.setup.is_none() {
        return Err("cannot run `solve_finish` before `solve.setup` has been recorded".into());
    }
    let entry = state
        .entry(final_entry_id)
        .ok_or_else(|| format!("unknown `final_entry_id` `{final_entry_id}`"))?;
    if let Some(rep) = &entry.superseded_by {
        return Err(format!(
            "cannot finish on superseded step `{final_entry_id}` (replaced by `{rep}`)"
        ));
    }

    let closure = closure_for_target(&state.entries, final_entry_id)?;
    let mut failed_ids = Vec::new();
    let mut stale_ids = Vec::new();
    let mut seen_failed = BTreeSet::new();
    let mut seen_stale = BTreeSet::new();

    for id in &closure {
        let Some(step) = state.entry(id) else {
            continue;
        };
        if step.superseded_by.is_some() {
            continue;
        }
        let Some(tier) = state.tier(id) else {
            continue;
        };
        if tier.effective == Tier::Failed || tier.own == Tier::Failed {
            if seen_failed.insert(id.clone()) {
                failed_ids.push(id.clone());
            }
            for b in &tier.broken_by {
                if seen_failed.insert(b.clone()) {
                    failed_ids.push(b.clone());
                }
            }
        }
        if (tier.effective == Tier::Stale || tier.stale) && seen_stale.insert(id.clone()) {
            stale_ids.push(id.clone());
        }
    }

    if !failed_ids.is_empty() {
        return Err(format!(
            "closure contains Failed step(s) ({}); fix or supersede failed steps before finishing",
            failed_ids.join(", ")
        ));
    }
    if !stale_ids.is_empty() {
        return Err(format!(
            "closure contains Stale step(s) ({}); re-check stale steps before finishing",
            stale_ids.join(", ")
        ));
    }

    lint_solve_summary_r22(summary_markdown, state, final_entry_id)?;
    Ok(closure)
}

/// Solve-tier-aware R22 wording lint on `summary_markdown` (`solve.md` 5.1, rule R22).
///
/// Never delegates to `agent::honesty` (which knows only Research verdicts).
/// - Disallows retired `"Computed"` badge claims and non-negated `"proved"`/`"proven"`.
/// - Allows `"verified"` next to named step ids ONLY when every named step has `effective == Verified`.
/// - Allows unqualified whole-answer `"verified"` ONLY when `final_entry_id` has `effective == Verified`.
pub fn lint_solve_summary_r22(
    summary: &str,
    state: &SolveState,
    final_entry_id: &str,
) -> Result<(), String> {
    let final_is_verified = state
        .tier(final_entry_id)
        .is_some_and(|t| t.effective == Tier::Verified);

    for raw_seg in summary.split(['\n', '.', '!', '?', ';']) {
        let seg = raw_seg.trim();
        if seg.is_empty() {
            continue;
        }
        let words = extract_words(seg);
        if words.is_empty() {
            continue;
        }

        for (idx, w) in words.iter().enumerate() {
            if *w == "Computed" {
                return Err(
                    "R22 wording violation: retired label `Computed` is disallowed; use `Verified` or `Numerically checked`"
                        .into(),
                );
            }
            let lower = w.to_ascii_lowercase();
            if lower == "computed" && idx > 0 {
                let prev = words[idx - 1].to_ascii_lowercase();
                if matches!(prev.as_str(), "is" | "are" | "was" | "status" | "tier" | "marked" | "badge") {
                    return Err(
                        "R22 wording violation: status label `computed` is retired; use `Verified` or `Numerically checked`"
                            .into(),
                    );
                }
            }
            if matches!(lower.as_str(), "proved" | "proven") && !is_negated_word(&words, idx) {
                return Err(
                    "R22 wording violation: `proved`/`proven` is disallowed; state exact Solve tier (`Verified`, `Numerically checked`, or `Model agreed`)"
                        .into(),
                );
            }
            if lower == "verified" && !is_negated_word(&words, idx) {
                let step_refs = extract_step_refs(seg, state);
                if step_refs.is_empty() {
                    if !final_is_verified {
                        let label = badge_for_step(state, final_entry_id)
                            .map(|b| b.label)
                            .unwrap_or_else(|| "Unchecked".into());
                        return Err(format!(
                            "R22 wording violation: summary describes answer as `verified`, but `{final_entry_id}` is `{label}`"
                        ));
                    }
                } else {
                    for sid in step_refs {
                        let Some(tier) = state.tier(&sid) else {
                            return Err(format!(
                                "R22 wording violation: `verified` claim cites unknown step `{sid}`"
                            ));
                        };
                        if tier.effective != Tier::Verified {
                            let label = badge_for_step(state, &sid)
                                .map(|b| b.label)
                                .unwrap_or_else(|| "Unchecked".into());
                            return Err(format!(
                                "R22 wording violation: step `{sid}` has effective tier `{label}`, so summary may not describe it as `verified`"
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn is_negated_word(words: &[&str], idx: usize) -> bool {
    let start = idx.saturating_sub(3);
    words[start..idx].iter().any(|w| {
        matches!(
            w.to_ascii_lowercase().as_str(),
            "not" | "never" | "no" | "neither" | "nor" | "without" | "cannot" | "unverified"
        )
    })
}

fn extract_words(seg: &str) -> Vec<&str> {
    seg.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect()
}

fn extract_step_refs(seg: &str, state: &SolveState) -> Vec<String> {
    let mut refs = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in seg.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | '.' | ':' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '`' | '\'' | '"' | '*' | '!'
            )
    }) {
        if raw.is_empty() {
            continue;
        }
        let is_step_pattern = raw.starts_with("e-")
            && raw.len() > 2
            && raw[2..].bytes().all(|b| b.is_ascii_digit());
        if (is_step_pattern || state.entry_index.contains_key(raw)) && seen.insert(raw.to_string()) {
            refs.push(raw.to_string());
        }
    }
    refs
}

struct ParsedFinishArgs {
    final_entry_id: String,
    summary_markdown: String,
    proposed_outcome: Option<SolveDoneStatus>,
    open_items: Vec<String>,
}

fn parse_finish_args(args: &Value) -> Result<ParsedFinishArgs, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "solve_finish arguments must be a JSON object".to_string())?;
    for k in obj.keys() {
        if !ALLOWED_FINISH_KEYS.contains(&k.as_str()) {
            return Err(format!("unknown argument `{k}` on solve_finish"));
        }
    }
    let final_entry_id = obj
        .get("final_entry_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "`final_entry_id` must be a non-empty string".to_string())?
        .to_string();
    if final_entry_id.chars().count() > 64 || final_entry_id.chars().any(char::is_control) {
        return Err("invalid `final_entry_id`".into());
    }

    let summary_markdown = obj
        .get("summary_markdown")
        .and_then(Value::as_str)
        .ok_or_else(|| "`summary_markdown` must be a string".to_string())?
        .to_string();
    if summary_markdown.chars().count() > MAX_SUMMARY_CHARS {
        return Err(format!("`summary_markdown` exceeds {MAX_SUMMARY_CHARS} chars"));
    }

    let proposed_outcome = match obj.get("outcome") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value::<SolveDoneStatus>(v.clone())
                .map_err(|e| format!("invalid `outcome`: {e}"))?,
        ),
    };

    let mut open_items = Vec::new();
    if let Some(arr) = obj.get("open_items").and_then(Value::as_array) {
        for item in arr {
            let text = item
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "every entry in `open_items` must be a non-empty string".to_string())?;
            if text.chars().count() > MAX_OPEN_ITEM_CHARS {
                return Err(format!("open_item exceeds {MAX_OPEN_ITEM_CHARS} chars"));
            }
            open_items.push(text.to_string());
        }
    }

    Ok(ParsedFinishArgs {
        final_entry_id,
        summary_markdown,
        proposed_outcome,
        open_items,
    })
}

#[cfg(test)]
#[path = "solve_done_tests.rs"]
mod tests;
