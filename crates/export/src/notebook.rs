//! Deterministic Solve mode Jupyter Notebook (`nbformat` 4.5, `.ipynb`) exporter (S19b).
//!
//! Reads the verified ledger directly (`Ledger::read_verified_session` + `SolveState::from_events`),
//! never calling Research `latex::load_session_export_data`. Enforces R5 (Markdown LaTeX strictly
//! from `solve.expr.latex`), D18 (default closure of `done.final_entry_id`; `include_all_runs`
//! opt-in), and R22 wording placement rules.

use crate::notebook_prelude::render_check_code_cell;
use crate::notebook_prelude::render_prelude_cell;
use crate::notebook_render::{
    append_extra_runs_and_checks, render_final_markdown, render_receipt_code_cell,
    render_setup_markdown, render_step_markdown,
};
use ledger::{sha256_hex, Event, Ledger};
use research_tools::paper_key::Mode;
use research_tools::proof_closure::{closure_for_target, topological_order};
use research_tools::quote_core::MAX_DISPLAY_QUOTE_CHARS;
use research_tools::solve_exprs::load_handle_srepr;
use research_tools::solve_model::{event_kinds, HandleRef, SolveCheck, SolveEntry, SolveExpr};
use research_tools::solve_receipts::{build_receipts, find_receipt, ToolReceiptRecord};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{badge_for_step, Tier};
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotebookOptions {
    pub include_all_runs: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SympyRequestInfo {
    pub code: String,
    pub staged_handles: BTreeMap<String, HandleRef>,
}

#[derive(Debug, Clone)]
pub(crate) struct QuoteTextInfo {
    pub excerpt: String,
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Export a verified Solve session from `data_dir` as pretty-printed nbformat 4.5 JSON bytes.
pub fn export_solve_notebook_bytes(
    data_dir: &Path,
    session: &str,
    options: NotebookOptions,
) -> Result<Vec<u8>, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("cannot export unverified session `{session}`: {e}"))?;
    build_solve_notebook_bytes(&ledger, &report.head_hash, session, &events, options)
}

/// Render nbformat 4.5 JSON bytes from already-loaded verified Solve session `events`.
pub fn build_solve_notebook_bytes(
    ledger: &Ledger,
    head_hash: &str,
    session: &str,
    events: &[Event],
    options: NotebookOptions,
) -> Result<Vec<u8>, String> {
    let val = build_solve_notebook_value(ledger, head_hash, session, events, options)?;
    let mut bytes = serde_json::to_vec_pretty(&val).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Build the structured `nbformat` 4.5 JSON document for a verified Solve session.
pub fn build_solve_notebook_value(
    ledger: &Ledger,
    head_hash: &str,
    session: &str,
    events: &[Event],
    options: NotebookOptions,
) -> Result<Value, String> {
    if session_mode_from_events(events) != Mode::Solve {
        return Err(format!("session `{session}` is not a Solve session"));
    }

    let state = SolveState::from_events(events);
    let mut all_handles: BTreeMap<HandleRef, (SolveExpr, String)> = BTreeMap::new();
    for (href, expr) in &state.handles {
        if let Some(srepr) = load_handle_srepr(ledger, expr)? {
            all_handles.insert(href.clone(), (expr.clone(), srepr));
        }
    }

    let receipts = build_receipts(events, Some(ledger));
    let request_info = collect_sympy_requests(ledger, events);
    let quote_texts = collect_quote_texts(events);

    let (ordered_steps, scope_label) = select_ordered_steps(&state, options.include_all_runs)?;
    let scope_ids: BTreeSet<&str> = ordered_steps.iter().map(|e| e.entry_id.as_str()).collect();
    let inlined_handles = select_inlined_handles(
        &all_handles,
        &ordered_steps,
        &scope_ids,
        &receipts,
        &request_info,
        options.include_all_runs,
    );

    let mut cells: Vec<Value> = Vec::new();
    let mut seen_cell_ids = BTreeSet::new();
    let mut exec_counter = 1u64;

    let setup_md = render_setup_markdown(
        &state,
        session,
        head_hash,
        options.include_all_runs,
        &scope_label,
    );
    cells.push(make_markdown_cell(
        &mut seen_cell_ids,
        "setup",
        session,
        &setup_md,
        json!({ "re_search": { "kind": "setup" } }),
    ));

    let prelude_py = render_prelude_cell(state.setup.as_ref(), &inlined_handles);
    cells.push(make_code_cell(
        &mut seen_cell_ids,
        "prelude",
        session,
        &prelude_py,
        None,
        Vec::new(),
        json!({ "re_search": { "kind": "prelude" } }),
    ));

    let current_math_sha = state.setup.as_ref().map(|s| s.math_sha256.as_str());
    let mut emitted_receipts = BTreeSet::new();
    let mut emitted_checks = BTreeSet::new();
    let mut has_any_verified = false;

    for entry in &ordered_steps {
        let badge = badge_for_step(&state, &entry.entry_id);
        if entry.superseded_by.is_none()
            && badge
                .as_ref()
                .is_some_and(|b| !b.stale && (b.own == Tier::Verified || b.effective == Tier::Verified))
        {
            has_any_verified = true;
        }
        let step_md = render_step_markdown(&state, entry, badge.as_ref(), &quote_texts);
        cells.push(make_markdown_cell(
            &mut seen_cell_ids,
            "step",
            &entry.entry_id,
            &step_md,
            json!({ "re_search": { "kind": "step", "entry_id": entry.entry_id } }),
        ));

        for cid in step_cited_call_ids(entry) {
            let Some(rec) = find_receipt(&receipts, cid) else {
                continue;
            };
            if rec.tool != "sympy_eval" || !emitted_receipts.insert(rec.call_id.clone()) {
                continue;
            }
            if let Some(code_cell) = render_receipt_code_cell(
                &mut seen_cell_ids,
                &mut exec_counter,
                rec,
                request_info.get(&rec.call_id),
                &inlined_handles,
                &entry.entry_id,
            ) {
                cells.push(code_cell);
            }
        }

        for (claim_idx, chk) in
            matching_checks_for_step(&state, entry, current_math_sha, options.include_all_runs)
        {
            if !emitted_checks.insert(chk.check_id.clone()) {
                continue;
            }
            let claim = entry.claims.get(claim_idx);
            let check_py = render_check_code_cell(chk, claim, &inlined_handles, &entry.entry_id);
            cells.push(make_code_cell(
                &mut seen_cell_ids,
                "check",
                &chk.check_id,
                &check_py,
                None,
                Vec::new(),
                json!({
                    "re_search": {
                        "kind": "check",
                        "check_id": chk.check_id,
                        "entry_id": entry.entry_id,
                        "outcome": chk.outcome,
                    }
                }),
            ));
        }
    }

    if options.include_all_runs {
        append_extra_runs_and_checks(
            &state,
            &receipts,
            &request_info,
            &inlined_handles,
            &mut emitted_receipts,
            &mut emitted_checks,
            &mut exec_counter,
            &mut seen_cell_ids,
            &mut cells,
        );
    }

    let final_md = render_final_markdown(
        &state,
        events,
        &ordered_steps,
        has_any_verified,
        options.include_all_runs,
    );
    cells.push(make_markdown_cell(
        &mut seen_cell_ids,
        "summary",
        session,
        &final_md,
        json!({ "re_search": { "kind": "done_summary" } }),
    ));

    Ok(json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3",
                "language": "python",
                "name": "python3"
            },
            "language_info": {
                "name": "python",
                "codemirror_mode": { "name": "ipython", "version": 3 },
                "file_extension": ".py",
                "mimetype": "text/x-python",
                "nbconvert_exporter": "python",
                "pygments_lexer": "ipython3"
            },
            "re_search": {
                "schema_version": 1,
                "session": session,
                "mode": "solve",
                "head_hash": head_hash,
                "math_sha256": state.setup.as_ref().map(|s| s.math_sha256.as_str()),
                "include_all_runs": options.include_all_runs
            }
        },
        "cells": cells
    }))
}

/// Deterministically allocate a unique 8-lowercase-hex-char nbformat 4.5 cell id.
pub(crate) fn alloc_cell_id(
    seen: &mut BTreeSet<String>,
    kind: &str,
    seed: &str,
    content: &str,
) -> String {
    let base = format!("re-search-cell:v1:{kind}:{seed}\n{content}");
    let first = sha256_hex(base.as_bytes())[..8].to_string();
    if seen.insert(first.clone()) {
        return first;
    }
    let mut nonce = 1u64;
    loop {
        let salted = format!("re-search-cell:v1:{kind}:{seed}#collision:{nonce}\n{content}");
        let cand = sha256_hex(salted.as_bytes())[..8].to_string();
        if seen.insert(cand.clone()) {
            return cand;
        }
        nonce += 1;
    }
}

pub(crate) fn make_markdown_cell(
    seen: &mut BTreeSet<String>,
    kind: &str,
    seed: &str,
    text: &str,
    metadata: Value,
) -> Value {
    json!({
        "id": alloc_cell_id(seen, kind, seed, text),
        "cell_type": "markdown",
        "metadata": metadata,
        "source": to_source_lines(text),
    })
}

pub(crate) fn make_code_cell(
    seen: &mut BTreeSet<String>,
    kind: &str,
    seed: &str,
    code: &str,
    execution_count: Option<u64>,
    outputs: Vec<Value>,
    metadata: Value,
) -> Value {
    json!({
        "id": alloc_cell_id(seen, kind, seed, code),
        "cell_type": "code",
        "execution_count": execution_count,
        "metadata": metadata,
        "outputs": outputs,
        "source": to_source_lines(code),
    })
}

pub(crate) fn to_source_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').map(str::to_string).collect()
}

pub(crate) fn step_cited_call_ids(entry: &SolveEntry) -> Vec<&str> {
    let mut ids = Vec::new();
    if let Some(c) = entry.tool_call_ref.as_deref() {
        ids.push(c);
    }
    for c in &entry.tool_call_refs {
        if !ids.contains(&c.as_str()) {
            ids.push(c.as_str());
        }
    }
    ids
}

pub(crate) fn matching_checks_for_step<'a>(
    state: &'a SolveState,
    entry: &SolveEntry,
    current_math_sha: Option<&str>,
    include_all_runs: bool,
) -> Vec<(usize, &'a SolveCheck)> {
    let claim_sha = entry.claim_sha256.as_deref();
    state
        .checks
        .iter()
        .filter(|chk| {
            let matches_claim = claim_sha == Some(chk.claim_sha256.as_str());
            let matches_entry =
                include_all_runs && chk.entry_id.as_deref() == Some(entry.entry_id.as_str());
            let matches_math =
                include_all_runs || current_math_sha.is_none_or(|m| m == chk.math_sha256.as_str());
            (matches_claim && matches_math) || matches_entry
        })
        .map(|chk| (chk.claim_index, chk))
        .collect()
}

fn collect_sympy_requests(ledger: &Ledger, events: &[Event]) -> BTreeMap<String, SympyRequestInfo> {
    let mut inc_state = SolveState::new();
    let mut out = BTreeMap::new();
    for ev in events {
        if ev.kind == event_kinds::TOOL_REQUEST
            && ev.payload.get("tool").and_then(Value::as_str) == Some("sympy_eval")
        {
            if let Some(call_id) = ev.payload.get("call_id").and_then(Value::as_str) {
                let raw_args = resolve_slot_text(ledger, ev.payload.get("args"));
                let code = serde_json::from_str::<Value>(&raw_args)
                    .ok()
                    .and_then(|v| v.get("code").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_default();
                let staged_handles = latest_portable_handles(&inc_state);
                out.insert(call_id.to_string(), SympyRequestInfo { code, staged_handles });
            }
        }
        inc_state.apply(ev);
    }
    out
}

fn latest_portable_handles(state: &SolveState) -> BTreeMap<String, HandleRef> {
    let mut latest: BTreeMap<&str, (&HandleRef, &SolveExpr)> = BTreeMap::new();
    for (href, expr) in &state.handles {
        if latest.get(href.name.as_str()).is_none_or(|(_, prev)| prev.version < expr.version) {
            latest.insert(href.name.as_str(), (href, expr));
        }
    }
    latest
        .into_iter()
        .filter(|(_, (_, expr))| expr.portable)
        .map(|(name, (href, _))| (name.to_string(), href.clone()))
        .collect()
}

fn resolve_slot_text(ledger: &Ledger, slot: Option<&Value>) -> String {
    let Some(obj) = slot.and_then(Value::as_object) else {
        return String::new();
    };
    if let Some(s) = obj.get("inline").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(h) = obj.get("blob").and_then(Value::as_str) {
        if let Ok(bytes) = ledger.get_blob(h) {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
    }
    String::new()
}

fn collect_quote_texts(events: &[Event]) -> BTreeMap<u64, QuoteTextInfo> {
    let mut out = BTreeMap::new();
    for ev in events {
        if ev.kind == event_kinds::EVIDENCE_QUOTE {
            let p = &ev.payload;
            let raw_excerpt = p
                .get("excerpt")
                .or_else(|| p.get("quote"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            out.insert(
                ev.seq,
                QuoteTextInfo {
                    excerpt: clip_chars(raw_excerpt, MAX_DISPLAY_QUOTE_CHARS),
                    source_id: p.get("source_id").and_then(Value::as_str).unwrap_or("").to_string(),
                    path: p.get("path").and_then(Value::as_str).unwrap_or("").to_string(),
                    start_line: p.get("start_line").and_then(Value::as_u64).unwrap_or(0) as usize,
                    end_line: p.get("end_line").and_then(Value::as_u64).unwrap_or(0) as usize,
                },
            );
        }
    }
    out
}

fn clip_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn select_ordered_steps(
    state: &SolveState,
    include_all_runs: bool,
) -> Result<(Vec<&SolveEntry>, String), String> {
    let (subset, scope_label): (Vec<&SolveEntry>, String) = if include_all_runs {
        (state.entries.iter().collect(), "All runs (including superseded and discarded)".to_string())
    } else if let Some(done) = &state.done {
        let closure = closure_for_target(&state.entries, &done.final_entry_id)?;
        let wanted: BTreeSet<&str> = closure.iter().map(String::as_str).collect();
        let list = state
            .entries
            .iter()
            .filter(|e| wanted.contains(e.entry_id.as_str()))
            .collect();
        (list, format!("Closure of `{}`", done.final_entry_id))
    } else {
        let list = state.active_entries();
        (list, "Active steps (no solve.done recorded)".to_string())
    };

    let order = topological_order(&subset).unwrap_or_else(|cyclic| cyclic);
    let mut by_id: BTreeMap<&str, &SolveEntry> =
        subset.iter().map(|e| (e.entry_id.as_str(), *e)).collect();
    let mut ordered = Vec::with_capacity(subset.len());
    for id in order {
        if let Some(entry) = by_id.remove(id.as_str()) {
            ordered.push(entry);
        }
    }
    ordered.extend(by_id.into_values());
    Ok((ordered, scope_label))
}

fn select_inlined_handles(
    all_handles: &BTreeMap<HandleRef, (SolveExpr, String)>,
    ordered_steps: &[&SolveEntry],
    scope_ids: &BTreeSet<&str>,
    receipts: &BTreeMap<String, ToolReceiptRecord>,
    request_info: &BTreeMap<String, SympyRequestInfo>,
    include_all_runs: bool,
) -> BTreeMap<HandleRef, (SolveExpr, String)> {
    if include_all_runs {
        return all_handles.clone();
    }
    let mut needed = BTreeSet::new();
    for entry in ordered_steps {
        for claim in &entry.claims {
            needed.insert(claim.lhs.clone());
            needed.insert(claim.rhs.clone());
        }
        for cid in step_cited_call_ids(entry) {
            if let Some(rec) = find_receipt(receipts, cid) {
                for h in &rec.harvested {
                    needed.insert(h.clone());
                }
                if let Some(req) = request_info.get(&rec.call_id) {
                    for h in req.staged_handles.values() {
                        needed.insert(h.clone());
                    }
                }
            }
        }
    }
    for (href, (expr, _)) in all_handles {
        if expr.owner_entry.as_deref().is_some_and(|id| scope_ids.contains(id)) {
            needed.insert(href.clone());
        }
    }
    all_handles
        .iter()
        .filter(|(href, _)| needed.contains(*href))
        .map(|(href, pair)| (href.clone(), pair.clone()))
        .collect()
}

#[cfg(test)]
#[path = "notebook_tests.rs"]
mod tests;
