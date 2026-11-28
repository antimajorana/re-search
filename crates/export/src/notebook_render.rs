//! Markdown and code cell renderers for Solve `.ipynb` exports (S19b).

use crate::notebook::{
    make_code_cell, make_markdown_cell, to_source_lines, QuoteTextInfo, SympyRequestInfo,
};
use crate::notebook_prelude::{py_str_literal, render_check_code_cell};
use ledger::Event;
use research_tools::solve_done::{evaluate_done_criteria, lint_solve_summary_r22};
use research_tools::solve_model::{
    event_kinds, ClaimForm, ClaimRelation, CriterionOutcome, DoneCriterion, DoneCriterionKind,
    HandleRef, ModelCallsBreakdown, RestsOnSummary, SolveDoneStatus, SolveEntry, SolveExpr,
    SolveSetup, StepOrigin,
};
use research_tools::solve_receipts::ToolReceiptRecord;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{badge_for_step, rests_on_summary, wording, StepBadge};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const LEGEND_VERIFIED_NOTE: &str =
    "Verified: the two sides are equal as exact expressions under the stated assumptions, \
     as decided by SymPy; this does not show that the expression means what the text says.";

pub(crate) fn render_setup_markdown(
    state: &SolveState,
    session: &str,
    head_hash: &str,
    include_all_runs: bool,
    scope_label: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Solve Notebook: `{session}`\n");
    let _ = writeln!(out, "- **Session:** `{session}`");
    let _ = writeln!(out, "- **Scope:** {scope_label} (`include_all_runs={include_all_runs}`)");
    let _ = writeln!(out, "- **Head SHA-256:** `{head_hash}`");

    let Some(setup) = &state.setup else {
        out.push_str("\n*No `solve.setup` event recorded.*\n");
        return out;
    };

    let task_kind = serde_json::to_value(setup.task_kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let target_form = serde_json::to_value(setup.target.form)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();

    let _ = writeln!(out, "- **Setup revision:** {}", setup.revision);
    let _ = writeln!(out, "- **Math SHA-256:** `{}`", setup.math_sha256);
    let _ = writeln!(out, "- **Task kind:** `{task_kind}`");
    let _ = writeln!(
        out,
        "- **Target (`{target_form}`):** {}",
        setup.target.description
    );
    if !setup.conventions.trim().is_empty() {
        let _ = writeln!(out, "- **Conventions:** {}", setup.conventions);
    }
    if !setup.sources.is_empty() {
        let _ = writeln!(out, "- **Sources:** {}", setup.sources.join(", "));
    }
    let _ = writeln!(out, "\n## Problem Statement\n\n{}", setup.statement);

    if !setup.symbols.is_empty() {
        out.push_str("\n### Symbols & Assumptions\n\n");
        for sym in &setup.symbols {
            let assump = if sym.assumptions.is_empty() {
                "unconstrained".to_string()
            } else {
                sym.assumptions.join(", ")
            };
            let _ = writeln!(out, "- `{}` ({assump})", sym.name);
        }
    }
    if !setup.dimensions.is_empty() {
        out.push_str("\n### Dimensions\n\n");
        for (sym, dims) in &setup.dimensions {
            let _ = writeln!(out, "- `{sym}`: {}", Value::Object(dims.clone().into_iter().collect()));
        }
    }
    if !setup.assumptions.is_empty() {
        out.push_str("\n### Stated Assumptions\n\n");
        for a in &setup.assumptions {
            let _ = writeln!(out, "- **{}:** {}", a.id, a.text);
        }
    }
    out
}

pub(crate) fn render_step_markdown(
    state: &SolveState,
    entry: &SolveEntry,
    badge: Option<&StepBadge>,
    quote_texts: &BTreeMap<u64, QuoteTextInfo>,
) -> String {
    let mut out = String::new();
    let sup = entry
        .superseded_by
        .as_deref()
        .map(|rep| format!(" *(superseded by `{rep}`)*"))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "## Step `{}` (`{}`){sup}\n",
        entry.entry_id,
        entry.entry_type.as_str()
    );

    if let Some(b) = badge {
        let _ = writeln!(out, "- **Badge:** {}", b.label);
        if let Some(own_line) = &b.own_check_line {
            let _ = writeln!(out, "- **Own check:** {own_line}");
        }
        if let Some(rests) = &b.rests_on_line {
            let _ = writeln!(out, "- **Rests on:** {rests}");
        }
        if let Some(tag) = &b.provenance_tag {
            let _ = writeln!(out, "- **Provenance:** {tag}");
        }
        if !b.sublabels.is_empty() {
            let _ = writeln!(out, "- **Checks:** {}", b.sublabels.join("; "));
        }
        if !b.flags.is_empty() {
            let _ = writeln!(out, "- **Flags:** {}", b.flags.join("; "));
        }
    }

    if !entry.depends_on.is_empty() {
        let _ = writeln!(out, "- **Depends on:** {}", entry.depends_on.join(", "));
    }
    if !entry.derived_depends_on.is_empty() {
        let _ = writeln!(out, "- **Derived depends on:** {}", entry.derived_depends_on.join(", "));
    }

    if let Some(qref) = &entry.source_ref {
        let ocr = qref
            .evidence_seq
            .and_then(|seq| state.quotes.get(&seq))
            .is_some_and(|rec| rec.ocr);
        let qtext = qref.evidence_seq.and_then(|seq| quote_texts.get(&seq));
        let tag = if ocr { " [OCR]" } else { "" };
        if let Some(qt) = qtext {
            let _ = writeln!(
                out,
                "- **Matched quote (`{}` `{}` L{}-L{}){tag}:** > {}",
                qt.source_id, qt.path, qt.start_line, qt.end_line, qt.excerpt
            );
        } else {
            let _ = writeln!(
                out,
                "- **Matched quote (`{}` `{}` L{}-L{}){tag}**",
                qref.source_id, qref.path, qref.start_line, qref.end_line
            );
        }
    }

    let origin_tag = match entry.origin {
        Some(StepOrigin::Solver | StepOrigin::Worker) | None => " (model)",
        Some(StepOrigin::Given | StepOrigin::User) => " (you)",
        Some(StepOrigin::Setup) => " (setup)",
    };
    let _ = writeln!(out, "\n### Summary{origin_tag}\n\n{}", entry.summary);
    if !entry.details.trim().is_empty() {
        let _ = writeln!(out, "\n{}\n", entry.details);
    }

    // Rule R5: mathematical formulas come strictly from check-canonicalized `solve.expr.latex`,
    // never trusting `entry.latex`.
    if !entry.claims.is_empty() {
        out.push_str("\n### Claims (canonical LaTeX from `solve.expr`)\n\n");
        for (idx, claim) in entry.claims.iter().enumerate() {
            let op = match claim.relation {
                ClaimRelation::Eq => "=",
                ClaimRelation::Le => r"\le",
                ClaimRelation::Ge => r"\ge",
                ClaimRelation::Approx => r"\approx",
            };
            let form = match claim.form {
                ClaimForm::Equation => "equation",
                ClaimForm::Inequality => "inequality",
                ClaimForm::Value => "value",
                ClaimForm::Series => "series",
            };
            let lhs = state.handles.get(&claim.lhs);
            let rhs = state.handles.get(&claim.rhs);
            let lhs_tex = lhs.filter(|e| e.portable).map(|e| e.latex.as_str());
            let rhs_tex = rhs.filter(|e| e.portable).map(|e| e.latex.as_str());
            let _ = writeln!(out, "- Claim {idx} (`{form}`, `{}` {op} `{}`):", claim.lhs, claim.rhs);
            match (lhs_tex, rhs_tex) {
                (Some(lt), Some(rt)) => {
                    let _ = writeln!(out, "\n$$\n{lt} {op} {rt}\n$$\n");
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "  *(non-portable handle preview: `{}` vs `{}`)*",
                        lhs.map_or("", |e| e.str_preview.as_str()),
                        rhs.map_or("", |e| e.str_preview.as_str())
                    );
                }
            }
            if let Some(how) = claim.how.as_deref().filter(|s| !s.trim().is_empty()) {
                let _ = writeln!(out, "  How{origin_tag}: {how}");
            }
        }
    }
    out
}

pub(crate) fn render_receipt_code_cell(
    seen_ids: &mut BTreeSet<String>,
    exec_counter: &mut u64,
    rec: &ToolReceiptRecord,
    req_info: Option<&SympyRequestInfo>,
    inlined_handles: &BTreeMap<HandleRef, (SolveExpr, String)>,
    step_label: &str,
) -> Option<Value> {
    let raw_code = req_info.map(|r| r.code.trim()).unwrap_or("");
    if raw_code.is_empty() {
        return None;
    }

    let mut code = format!(
        "# Receipt `{}` ({step_label}, result_seq={}, ok={})\n",
        rec.call_id, rec.result_seq, rec.ok
    );
    if let Some(info) = req_info {
        let mut staged_pairs = Vec::new();
        for (name, href) in &info.staged_handles {
            if inlined_handles.contains_key(href) {
                staged_pairs.push(format!("{}: {}", py_str_literal(name), py_str_literal(&href.to_string())));
            }
        }
        if !staged_pairs.is_empty() {
            let _ = writeln!(code, "_stage_handles({{{}}})", staged_pairs.join(", "));
        }
    }
    code.push_str(raw_code);
    if !code.ends_with('\n') {
        code.push('\n');
    }

    let mut outputs = Vec::new();
    let mut cell_exec_count = None;
    if let Some(decoded) = &rec.decoded_result {
        if let Some(stdout) = decoded.get("stdout").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            outputs.push(json!({
                "output_type": "stream",
                "name": "stdout",
                "text": to_source_lines(stdout),
            }));
        }
        if let Some(stderr) = decoded.get("stderr").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            outputs.push(json!({
                "output_type": "stream",
                "name": "stderr",
                "text": to_source_lines(stderr),
            }));
        }
        if let Some(val_str) = decoded.get("value").and_then(Value::as_str) {
            let count = *exec_counter;
            *exec_counter += 1;
            cell_exec_count = Some(count);
            let mut data = serde_json::Map::new();
            data.insert("text/plain".into(), json!(to_source_lines(val_str)));
            if let Some(tex) = decoded.get("latex").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                data.insert("text/latex".into(), json!(to_source_lines(&format!("${tex}$"))));
            }
            outputs.push(json!({
                "output_type": "execute_result",
                "execution_count": count,
                "data": Value::Object(data),
                "metadata": {},
            }));
        }
    }

    Some(make_code_cell(
        seen_ids,
        "receipt",
        &rec.call_id,
        &code,
        cell_exec_count,
        outputs,
        json!({
            "re_search": {
                "kind": "receipt",
                "call_id": rec.call_id,
                "result_seq": rec.result_seq,
                "ok": rec.ok,
            }
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_extra_runs_and_checks(
    state: &SolveState,
    receipts: &BTreeMap<String, ToolReceiptRecord>,
    request_info: &BTreeMap<String, SympyRequestInfo>,
    inlined_handles: &BTreeMap<HandleRef, (SolveExpr, String)>,
    emitted_receipts: &mut BTreeSet<String>,
    emitted_checks: &mut BTreeSet<String>,
    exec_counter: &mut u64,
    seen_ids: &mut BTreeSet<String>,
    cells: &mut Vec<Value>,
) {
    let mut extras: Vec<Value> = Vec::new();
    let mut ordered_recs: Vec<&ToolReceiptRecord> =
        receipts.values().filter(|r| r.tool == "sympy_eval").collect();
    ordered_recs.sort_by_key(|r| r.result_seq);
    for rec in ordered_recs {
        if emitted_receipts.insert(rec.call_id.clone()) {
            if let Some(cell) = render_receipt_code_cell(
                seen_ids,
                exec_counter,
                rec,
                request_info.get(&rec.call_id),
                inlined_handles,
                "uncited run",
            ) {
                extras.push(cell);
            }
        }
    }
    for chk in &state.checks {
        if emitted_checks.insert(chk.check_id.clone()) {
            let entry = chk.entry_id.as_deref().and_then(|id| state.entry(id));
            let claim = entry.and_then(|e| e.claims.get(chk.claim_index));
            let step_label = chk.entry_id.as_deref().unwrap_or("unattached");
            let py = render_check_code_cell(chk, claim, inlined_handles, step_label);
            extras.push(make_code_cell(
                seen_ids,
                "check",
                &chk.check_id,
                &py,
                None,
                Vec::new(),
                json!({
                    "re_search": {
                        "kind": "check",
                        "check_id": chk.check_id,
                        "entry_id": step_label,
                        "outcome": chk.outcome,
                    }
                }),
            ));
        }
    }
    if !extras.is_empty() {
        cells.push(make_markdown_cell(
            seen_ids,
            "extra_header",
            "all_runs",
            "## Additional Runs & Checks (`include_all_runs=true`)\n",
            json!({ "re_search": { "kind": "extra_header" } }),
        ));
        cells.extend(extras);
    }
}

pub(crate) fn render_final_markdown(
    state: &SolveState,
    events: &[Event],
    ordered_steps: &[&SolveEntry],
    has_any_verified: bool,
    include_all_runs: bool,
) -> String {
    let mut out = String::from("## Outcome & Done Checklist\n\n");
    if let Some(done) = &state.done {
        let status_line = match done.status {
            SolveDoneStatus::Finished => "Done",
            SolveDoneStatus::FinishedOpenItems => "Done with open items",
        };
        let _ = writeln!(out, "- **Status:** {status_line}");
        if let Some(b) = badge_for_step(state, &done.final_entry_id) {
            let _ = writeln!(out, "- **Final step `{}`:** {}", done.final_entry_id, b.label);
            if let Some(own_line) = &b.own_check_line {
                let _ = writeln!(out, "  - {own_line} (for `{}`)", done.final_entry_id);
            }
        }
        render_criteria_list(&mut out, &done.criteria, state.setup.as_ref());
        render_rests_on(&mut out, &done.rests_on);
        render_model_calls(&mut out, done.model_calls);
        if let Some(summary_md) = events
            .iter()
            .rev()
            .find(|e| e.kind == event_kinds::SOLVE_DONE)
            .and_then(|e| e.payload.get("summary_markdown"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            if lint_solve_summary_r22(summary_md, state, &done.final_entry_id).is_ok() {
                let _ = writeln!(out, "\n### Finish Summary (model)\n\n{summary_md}");
            }
        }
    } else {
        out.push_str("- **Status:** In progress / no `solve.done` recorded\n");
        if let Some(setup) = &state.setup {
            let target_id = ordered_steps.last().map(|e| e.entry_id.as_str()).unwrap_or("");
            if !target_id.is_empty() {
                let ids: Vec<String> = ordered_steps.iter().map(|e| e.entry_id.clone()).collect();
                if let Ok(criteria) = evaluate_done_criteria(setup, state, target_id, &ids) {
                    render_criteria_list(&mut out, &criteria, Some(setup));
                }
                render_rests_on(&mut out, &rests_on_summary(state, target_id));
            }
        }
    }

    if has_any_verified {
        let _ = writeln!(out, "\n> {LEGEND_VERIFIED_NOTE}");
    }
    if include_all_runs {
        out.push_str("\n*Exported with `include_all_runs=true`.*\n");
    }
    out
}

fn render_criteria_list(
    out: &mut String,
    criteria: &[CriterionOutcome],
    setup: Option<&SolveSetup>,
) {
    if criteria.is_empty() {
        return;
    }
    out.push_str("\n### Done Checklist\n\n");
    for c in criteria {
        let mark = if c.met { "x" } else { " " };
        let label = setup
            .and_then(|s| s.done.iter().find(|d| d.id == c.id))
            .map(describe_criterion)
            .unwrap_or_else(|| c.id.clone());
        let state_word = if c.met { "met" } else { "open" };
        let _ = writeln!(out, "- [{mark}] `{}` — {label} ({state_word})", c.id);
    }
}

fn fmt_req_tier(raw: Option<&str>) -> &str {
    match raw.unwrap_or("") {
        "verified" => "exact symbolic check (tier 4)",
        "checked_numerically" => wording::BADGE_CHECKED_NUMERICALLY,
        "model_agreed" => wording::BADGE_MODEL_AGREED,
        "checked_by_you" => wording::BADGE_CHECKED_BY_YOU,
        "model_agreed_same_model" => wording::BADGE_MODEL_AGREED_SAME_MODEL,
        other => other,
    }
}

fn describe_criterion(c: &DoneCriterion) -> String {
    match c.kind {
        DoneCriterionKind::FinalTierAtLeast => {
            let tier = fmt_req_tier(c.params.get("tier").and_then(Value::as_str));
            format!("final step reaches {tier}")
        }
        DoneCriterionKind::ClosureTierAtLeast => {
            let tier = fmt_req_tier(c.params.get("tier").and_then(Value::as_str));
            format!("all closure steps reach {tier}")
        }
        DoneCriterionKind::NamedCheckPassed => {
            let name = c.params.get("check").and_then(Value::as_str).unwrap_or("check");
            format!("named check `{name}` passed")
        }
        DoneCriterionKind::NoOpenFlags => "no open flags on closure steps".to_string(),
        DoneCriterionKind::UserConfirms => "user confirmation recorded".to_string(),
    }
}

fn render_rests_on(out: &mut String, rests_on: &RestsOnSummary) {
    out.push_str("\n### Rests On\n\n");
    let list_or_none = |items: &[String]| {
        if items.is_empty() { "none".to_string() } else { items.join(", ") }
    };
    let _ = writeln!(out, "- **Given:** {}", list_or_none(&rests_on.given));
    let _ = writeln!(out, "- **Found in sources:** {}", list_or_none(&rests_on.cited));
    let _ = writeln!(out, "- **Assumed by model:** {}", list_or_none(&rests_on.assumed_by_model));
    let weakest: Vec<String> = rests_on
        .weakest
        .iter()
        .map(|w| {
            let tier_display = if w.tier.eq_ignore_ascii_case("verified") {
                "exact symbolic check (tier 4)".to_string()
            } else {
                w.tier.clone()
            };
            format!("{} ({tier_display})", w.entry_id)
        })
        .collect();
    let _ = writeln!(
        out,
        "- **Weakest closure dependencies:** {}",
        if weakest.is_empty() { "none".to_string() } else { weakest.join(", ") }
    );
}

fn render_model_calls(out: &mut String, calls: ModelCallsBreakdown) {
    let _ = writeln!(
        out,
        "\n- **Model calls:** solver={}, verifier={}, workers={}",
        calls.solver, calls.verifier, calls.workers
    );
}
