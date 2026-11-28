//! Solve view enrichment helpers (`solve.md` 4.1, 5.4, WP1 provenance honesty).
//!
//! Extracted from `solve_view.rs` to keep every source module <= 600 lines.

use ledger::Event;
use research_tools::proof_closure::closure_for_target;
use research_tools::solve_model::event_kinds;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Extract `sympy_eval` Python `code` strings from `tool.request` ledger events,
/// keyed by `call_id` (handling inline JSON `args.inline` or direct `args.code`).
pub fn extract_sympy_request_code(events: &[Event]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for ev in events {
        if ev.kind != event_kinds::TOOL_REQUEST {
            continue;
        }
        let p = &ev.payload;
        if p.get("tool").and_then(Value::as_str) != Some("sympy_eval") {
            continue;
        }
        let Some(call_id) = p.get("call_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(args) = p.get("args") else {
            continue;
        };
        if let Some(code) = extract_code_from_args(args) {
            if !code.trim().is_empty() {
                map.insert(call_id.to_string(), code);
            }
        }
    }
    map
}

fn extract_code_from_args(args: &Value) -> Option<String> {
    if let Some(code) = args.get("code").and_then(Value::as_str) {
        return Some(code.to_string());
    }
    if let Some(inline) = args.get("inline").and_then(Value::as_str) {
        if let Ok(parsed) = serde_json::from_str::<Value>(inline) {
            if let Some(code) = parsed.get("code").and_then(Value::as_str) {
                return Some(code.to_string());
            }
        }
    }
    None
}

/// Build `view.handles` JSON enriched with `"source_expr"`.
pub fn build_handles_json(state: &SolveState) -> Vec<Value> {
    state
        .handles
        .iter()
        .map(|(h, expr)| {
            json!({
                "handle": h.to_string(),
                "name": expr.name,
                "version": expr.version,
                "latex": expr.latex,
                "str_preview": expr.str_preview,
                "srepr_sha256": expr.srepr_sha256,
                "free_symbols": expr.free_symbols,
                "produced_by": expr.produced_by,
                "owner_entry": expr.owner_entry,
                "portable": expr.portable,
                "source_expr": expr.source_expr,
            })
        })
        .collect()
}

/// Build `view.receipts` JSON enriched with `"code"` from `tool.request` when available.
pub fn build_receipts_json(state: &SolveState, events: &[Event]) -> Vec<Value> {
    let code_by_call = extract_sympy_request_code(events);
    state
        .receipts
        .values()
        .filter_map(|r| {
            let mut val = serde_json::to_value(r).ok()?;
            if let Some(code) = code_by_call.get(&r.call_id) {
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("code".into(), Value::String(code.clone()));
                }
            }
            Some(val)
        })
        .collect()
}

/// Build `run_state.open_items` for a finished Solve session, including unmet criteria,
/// unacknowledged `AssumedByModel` steps in `done.rests_on`, and any active
/// `AssumedByModel` steps outside the final closure.
pub fn done_open_items(state: &SolveState) -> Value {
    let Some(done) = &state.done else {
        return json!([]);
    };
    let mut items = Vec::new();
    for c in &done.criteria {
        if !c.met {
            items.push(json!({ "kind": "unmet_criterion", "id": c.id }));
        }
    }
    let closure_ids: BTreeSet<String> =
        closure_for_target(&state.entries, &done.final_entry_id)
            .unwrap_or_default()
            .into_iter()
            .collect();
    let mut seen_assumed: BTreeSet<String> = BTreeSet::new();
    for eid in &done.rests_on.assumed_by_model {
        seen_assumed.insert(eid.clone());
        items.push(json!({ "kind": "unacknowledged_assumption", "entry_id": eid }));
    }
    for entry in &state.entries {
        if entry.superseded_by.is_some() || seen_assumed.contains(&entry.entry_id) {
            continue;
        }
        let own_is_assumed = state
            .tier(&entry.entry_id)
            .is_some_and(|t| t.own == Tier::AssumedByModel);
        if own_is_assumed {
            let outside = !closure_ids.contains(&entry.entry_id);
            let mut item = json!({
                "kind": "unacknowledged_assumption",
                "entry_id": entry.entry_id,
            });
            if outside {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("outside_final_closure".into(), Value::Bool(true));
                }
            }
            items.push(item);
        }
    }
    Value::Array(items)
}
