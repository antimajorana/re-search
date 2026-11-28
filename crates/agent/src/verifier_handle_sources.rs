//! Producing-code extraction, handle provenance enrichment, and trivial-check citation
//! rejection for the Solve verifier loop (`verifier_loop_support.rs`).

use ledger::{Event, Ledger};
use research_tools::solve_exprs::clip_utf8_chars;
use research_tools::solve_model::{event_kinds, Claim, SolveCheck, SolveEntry};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{check_is_trivial, claim_sides_identical};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Maximum characters of producing `sympy_eval` Python code included per call in verifier prompts.
pub(crate) const MAX_HANDLE_SOURCE_CODE_CHARS: usize = 1_800;

/// Extract `{ call_id -> sympy_eval Python code }` from session `tool.request` events.
pub(crate) fn extract_sympy_code_by_call(
    events: &[Event],
    ledger: Option<&Ledger>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for ev in events {
        if ev.kind != event_kinds::TOOL_REQUEST {
            continue;
        }
        if ev.payload.get("tool").and_then(Value::as_str) != Some("sympy_eval") {
            continue;
        }
        let Some(call_id) = ev.payload.get("call_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(code) = decode_sympy_request_code(&ev.payload, ledger) {
            out.insert(call_id.to_string(), code);
        }
    }
    out
}

fn decode_sympy_request_code(payload: &Value, ledger: Option<&Ledger>) -> Option<String> {
    let args = payload.get("args")?;
    let parsed = if let Some(inline) = args.get("inline").and_then(Value::as_str) {
        serde_json::from_str::<Value>(inline).ok()?
    } else if let Some(blob) = args.get("blob").and_then(Value::as_str) {
        let bytes = ledger?.get_blob(blob).ok()?;
        serde_json::from_slice::<Value>(&bytes).ok()?
    } else {
        args.clone()
    };
    parsed
        .get("code")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Render one claim for the verifier prompt with check-process LaTeX, `sides_identical`,
/// `source_expr`, and producing `call_id` provenance.
pub(crate) fn render_claim_with_provenance(
    state: &SolveState,
    idx: usize,
    c: &Claim,
) -> Value {
    let lhs_expr = state.handles.get(&c.lhs);
    let rhs_expr = state.handles.get(&c.rhs);
    let lhs_latex = lhs_expr.filter(|e| e.portable).map(|e| e.latex.as_str()).unwrap_or("");
    let rhs_latex = rhs_expr.filter(|e| e.portable).map(|e| e.latex.as_str()).unwrap_or("");
    let sides_identical = claim_sides_identical(state, c);
    json!({
        "claim_index": idx,
        "form": c.form,
        "relation": c.relation,
        "lhs_handle": c.lhs,
        "lhs_latex": lhs_latex,
        "lhs_source_expr": lhs_expr.and_then(|e| e.source_expr.as_deref()),
        "lhs_call_id": lhs_expr.map(|e| e.produced_by.call_id.as_str()),
        "rhs_handle": c.rhs,
        "rhs_latex": rhs_latex,
        "rhs_source_expr": rhs_expr.and_then(|e| e.source_expr.as_deref()),
        "rhs_call_id": rhs_expr.map(|e| e.produced_by.call_id.as_str()),
        "sides_identical": sides_identical,
        "domain": c.domain,
        "order": c.order,
    })
}

/// Build the `handle_sources` block for a verifier prompt: per-handle `source_expr` and
/// producing `call_id`, plus clipped producing `sympy_eval` code (~1800 chars each).
pub(crate) fn build_handle_sources(
    state: &SolveState,
    closure: &[&SolveEntry],
    code_by_call: &BTreeMap<String, String>,
) -> Value {
    let mut handle_entries = Vec::new();
    let mut seen_handles = BTreeSet::new();
    let mut cited_calls = BTreeSet::new();

    for entry in closure {
        for r in entry.tool_call_ref.iter().chain(entry.tool_call_refs.iter()) {
            let trimmed = r.trim();
            if !trimmed.is_empty() {
                cited_calls.insert(trimmed.to_string());
            }
        }
        for claim in &entry.claims {
            for href in [&claim.lhs, &claim.rhs] {
                let key = href.to_string();
                if !seen_handles.insert(key.clone()) {
                    continue;
                }
                if let Some(expr) = state.handles.get(href) {
                    let cid = expr.produced_by.call_id.clone();
                    cited_calls.insert(cid.clone());
                    let code_clipped = code_by_call.get(&cid).map(|raw| {
                        let (clipped, was_cut) = clip_utf8_chars(raw, MAX_HANDLE_SOURCE_CODE_CHARS);
                        if was_cut { format!("{clipped}\n... [clipped]") } else { clipped }
                    });
                    handle_entries.push(json!({
                        "handle": key,
                        "source_expr": expr.source_expr,
                        "call_id": cid,
                        "code": code_clipped,
                    }));
                }
            }
        }
    }

    let mut receipt_code = BTreeMap::new();
    for cid in cited_calls {
        if let Some(raw) = code_by_call.get(&cid) {
            let (clipped, was_cut) = clip_utf8_chars(raw, MAX_HANDLE_SOURCE_CODE_CHARS);
            let text = if was_cut { format!("{clipped}\n... [clipped]") } else { clipped };
            receipt_code.insert(cid, Value::String(text));
        }
    }

    json!({
        "handles": handle_entries,
        "sympy_code_by_call": receipt_code,
    })
}

/// Reject citing a `Trivial` or identical-side check as evidence in `report_verdict.checks_cited`.
pub(crate) fn reject_trivial_check_citation(
    state: &SolveState,
    entry_id: &str,
    check: &SolveCheck,
) -> Result<(), String> {
    if check_is_trivial(state, check) {
        return Err(format!(
            "cited check `{}` for `{entry_id}` is trivial (both sides identical before simplification) and is not valid evidence; do not cite trivial checks",
            check.check_id
        ));
    }
    Ok(())
}
