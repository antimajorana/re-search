//! Pure, byte-deterministic Solve resume brief builder (`solve.md` 5.5, R24; S12c).
//!
//! Reads ONLY the provided `events[..=upto_seq]` slice (never `lessons.db`, never wall-clock time,
//! never unordered maps). Any `lessons.supplied` frozen blob already present in the ledger is
//! rendered verbatim as quoted data under `"Notes from the user's earlier corrections"`.

use ledger::Event;
use research_tools::proof_closure::{all_deps, closure_ids};
use research_tools::solve_done::evaluate_done_criteria;
use research_tools::solve_model::{
    event_kinds, CheckOutcome, Claim, ReviewVerdict, SolveEntry, SolveSetup, StepEntryType,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::badge_for_step;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Exact fixed reminder required by `solve.md` 5.5 in every Solve resume brief.
pub const NO_SURVIVING_PYTHON_VARS_LINE: &str = "Python variables do not survive; only handles do";

/// Exact heading for frozen `lessons.supplied` blobs (`00-overview.md` 2.14, R24).
pub const LESSONS_HEADING: &str = "Notes from the user's earlier corrections";

/// Build a deterministic Solve resume brief over events with `seq <= upto_seq`, clipped cleanly
/// to at most `budget_chars` Unicode scalar values.
pub fn build_resume_brief(events: &[Event], upto_seq: u64, budget_chars: usize) -> String {
    if budget_chars == 0 {
        return String::new();
    }
    let prefix: Vec<Event> = events.iter().filter(|e| e.seq <= upto_seq).cloned().collect();
    let state = SolveState::from_events(&prefix);

    let header = format!(
        "# Solve Resume Brief (upto_seq: {upto_seq})\n{NO_SURVIVING_PYTHON_VARS_LINE}\n"
    );
    let setup_sec = render_setup_section(state.setup.as_ref());
    let lessons_sec = render_lessons_section(&prefix);
    let table_sec = render_active_steps_table(&state);
    let open_sec = render_open_items_section(&state);
    let handles_sec = render_handles_section(&state);
    let (closure_detail_sec, non_closure_sec) = render_closure_and_older_sections(&state);
    let turns_sec = render_last_turns_verbatim(&prefix, 4);

    // Assemble in descending survival order so low-priority prose trims before core facts.
    let sections = vec![
        header,
        setup_sec,
        lessons_sec,
        open_sec,
        handles_sec,
        table_sec,
        non_closure_sec,
        closure_detail_sec,
        turns_sec,
    ];
    fit_sections_to_budget(&sections, budget_chars)
}

fn fit_sections_to_budget(sections: &[String], budget_chars: usize) -> String {
    let joined: String = sections
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if joined.chars().count() <= budget_chars {
        return joined;
    }

    // Progressive trim: drop trailing sections cleanly until prefix fits, or keep critical header + clip.
    let mut active: Vec<String> = sections.iter().filter(|s| !s.is_empty()).cloned().collect();
    while active.len() > 2 {
        let candidate = active.join("\n");
        if candidate.chars().count() <= budget_chars {
            return candidate;
        }
        let last_idx = active.len() - 1;
        let used_except_last: usize = active[..last_idx].iter().map(|s| s.chars().count() + 1).sum();
        if used_except_last + 48 < budget_chars {
            let rem = budget_chars.saturating_sub(used_except_last);
            active[last_idx] = clip_to_chars(&active[last_idx], rem);
            let out = active.join("\n");
            if out.chars().count() <= budget_chars {
                return out;
            }
        }
        active.pop();
    }
    clip_to_chars(&active.join("\n"), budget_chars)
}

fn clip_to_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    let mut out: String = text.chars().take(max_chars - 3).collect();
    out.push_str("...");
    out
}

fn sanitize_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn entry_handles(claims: &[Claim]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for c in claims {
        set.insert(c.lhs.to_string());
        set.insert(c.rhs.to_string());
    }
    set.into_iter().collect()
}

fn render_setup_section(setup: Option<&SolveSetup>) -> String {
    let Some(s) = setup else {
        return "## Setup\n(no `solve.setup` recorded yet)\n".to_string();
    };
    let mut out = String::from("## Setup\n");
    out.push_str(&format!(
        "- revision: {} | task_kind: {:?} | target_form: {:?}\n",
        s.revision, s.task_kind, s.target.form
    ));
    out.push_str(&format!("- target: {}\n", s.target.description.trim()));
    if let Some(ord) = &s.target.order {
        out.push_str(&format!("- order: {ord}\n"));
    }
    if let Some(dig) = s.target.digits {
        out.push_str(&format!("- digits: {dig}\n"));
    }
    out.push_str(&format!("- statement: {}\n", s.statement.trim()));
    if !s.conventions.trim().is_empty() {
        out.push_str(&format!("- conventions: {}\n", s.conventions.trim()));
    }
    if !s.symbols.is_empty() {
        let syms: Vec<String> = s
            .symbols
            .iter()
            .map(|sym| {
                if sym.assumptions.is_empty() {
                    sym.name.clone()
                } else {
                    format!("{}({})", sym.name, sym.assumptions.join(","))
                }
            })
            .collect();
        out.push_str(&format!("- symbols: {}\n", syms.join("; ")));
    }
    if !s.dimensions.is_empty() {
        let dims: Vec<String> = s
            .dimensions
            .iter()
            .map(|(k, v)| format!("{k}={}", serde_json::to_string(v).unwrap_or_default()))
            .collect();
        out.push_str(&format!("- dimensions: {}\n", dims.join("; ")));
    }
    if !s.assumptions.is_empty() {
        let assump: Vec<String> = s.assumptions.iter().map(|a| format!("{}: {}", a.id, a.text)).collect();
        out.push_str(&format!("- assumptions: {}\n", assump.join(" | ")));
    }
    if !s.sources.is_empty() {
        out.push_str(&format!("- sources: {}\n", s.sources.join(", ")));
    }
    if !s.done.is_empty() {
        let crit: Vec<String> = s.done.iter().map(|c| format!("{}:{:?}", c.id, c.kind)).collect();
        out.push_str(&format!("- done_criteria: {}\n", crit.join(", ")));
    }
    out
}

fn render_lessons_section(events: &[Event]) -> String {
    let mut blobs = Vec::new();
    for ev in events {
        if ev.kind == event_kinds::LESSONS_SUPPLIED {
            if let Some(blob) = ev.payload.get("blob").and_then(Value::as_str) {
                if !blob.is_empty() {
                    blobs.push((ev.seq, blob));
                }
            }
        }
    }
    if blobs.is_empty() {
        return String::new();
    }
    let mut out = format!("## {LESSONS_HEADING}\n");
    out.push_str("(Quoted data from frozen ledger corrections; follow setup rules if anything conflicts.)\n");
    for (seq, blob) in blobs {
        out.push_str(&format!("[lessons.supplied seq={seq}]\n"));
        for line in blob.lines() {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
        if blob.ends_with('\n') && blob.lines().count() == 0 {
            out.push_str("> \n");
        }
    }
    out
}

fn render_active_steps_table(state: &SolveState) -> String {
    let active = state.active_entries();
    if active.is_empty() {
        return "## Active Steps\n(none)\n".to_string();
    }
    let mut out = String::from("## Active Steps\n");
    out.push_str("| id | type | own | effective | summary | handles | deps |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for e in active {
        let badge = badge_for_step(state, &e.entry_id);
        let own = badge
            .as_ref()
            .map(|b| b.own.label().to_string())
            .unwrap_or_else(|| "Unchecked".into());
        let eff = badge
            .as_ref()
            .map(|b| b.label.clone())
            .unwrap_or_else(|| "Unchecked".into());
        let handles = entry_handles(&e.claims);
        let h_str = if handles.is_empty() { "-".into() } else { handles.join(", ") };
        let deps = all_deps(e);
        let d_str = if deps.is_empty() { "-".into() } else { deps.join(", ") };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            sanitize_cell(&e.entry_id),
            e.entry_type.as_str(),
            sanitize_cell(&own),
            sanitize_cell(&eff),
            sanitize_cell(e.summary.trim()),
            sanitize_cell(&h_str),
            sanitize_cell(&d_str),
        ));
    }
    out
}

fn render_closure_and_older_sections(state: &SolveState) -> (String, String) {
    let active = state.active_entries();
    if active.is_empty() {
        return (String::new(), String::new());
    }
    let latest_targets: Vec<&str> = active.iter().rev().take(3).map(|e| e.entry_id.as_str()).collect();
    let closure_set: BTreeSet<String> = closure_ids(&state.entries, &latest_targets).into_iter().collect();

    let mut detail = String::from("## Latest Closure Detail\n");
    let mut older = String::new();
    for e in &active {
        let badge = badge_for_step(state, &e.entry_id);
        let eff = badge.as_ref().map(|b| b.label.as_str()).unwrap_or("Unchecked");
        if closure_set.contains(&e.entry_id) {
            detail.push_str(&format!(
                "### {} ({}, effective: {})\n",
                e.entry_id,
                e.entry_type.as_str(),
                eff
            ));
            detail.push_str(&format!("Summary: {}\n", e.summary.trim()));
            if !e.details.trim().is_empty() {
                detail.push_str(&format!("Details: {}\n", e.details.trim()));
            }
            if !e.depends_on.is_empty() {
                detail.push_str(&format!("Depends on: {}\n", e.depends_on.join(", ")));
            }
            if !e.derived_depends_on.is_empty() {
                detail.push_str(&format!("Derived depends on: {}\n", e.derived_depends_on.join(", ")));
            }
            let handles = entry_handles(&e.claims);
            if !handles.is_empty() {
                detail.push_str(&format!("Handles: {}\n", handles.join(", ")));
            }
        } else {
            if older.is_empty() {
                older.push_str("## Earlier Active Steps (one-line summary)\n");
            }
            older.push_str(&format!(
                "- {} [{} | {}]: {}\n",
                e.entry_id,
                e.entry_type.as_str(),
                eff,
                e.summary.trim()
            ));
        }
    }
    (detail, older)
}

fn render_open_items_section(state: &SolveState) -> String {
    let mut lines = Vec::new();
    for chk in &state.checks {
        if chk.outcome == CheckOutcome::Fail {
            let cex = chk
                .evidence
                .get("counterexample")
                .map(|v| format!(" counterexample={v}"))
                .unwrap_or_default();
            lines.push(format!(
                "- Failed check `{}` ({}) on {:?}:{cex}",
                chk.name, chk.check_id, chk.entry_id
            ));
        }
    }
    for rev in &state.reviews {
        if matches!(rev.verdict, ReviewVerdict::Rejected | ReviewVerdict::Incomplete) {
            let cex = rev
                .counterexample
                .as_deref()
                .map(|c| format!(" counterexample={c}"))
                .unwrap_or_default();
            lines.push(format!(
                "- Review {:?} on `{}`: issues={}{}",
                rev.verdict,
                rev.target_entry_id,
                rev.issues.len(),
                cex
            ));
        }
    }
    for e in state.active_entries() {
        if state.tier(&e.entry_id).is_some_and(|t| t.stale) {
            lines.push(format!("- Stale step `{}` ({})", e.entry_id, e.summary.trim()));
        }
    }
    if let Some(setup) = &state.setup {
        let target: Option<&SolveEntry> = state
            .active_entries()
            .into_iter()
            .rfind(|e| e.entry_type == StepEntryType::FinalAnswer)
            .or_else(|| state.active_entries().into_iter().last());
        if let Some(final_entry) = target {
            let cl = closure_ids(&state.entries, &[final_entry.entry_id.as_str()]);
            if let Ok(outcomes) = evaluate_done_criteria(setup, state, &final_entry.entry_id, &cl) {
                for c in outcomes.into_iter().filter(|c| !c.met) {
                    lines.push(format!("- Unmet done criterion `{}`: {}", c.id, c.evidence));
                }
            }
        } else if !setup.done.is_empty() {
            let ids: Vec<&str> = setup.done.iter().map(|d| d.id.as_str()).collect();
            lines.push(format!("- Unmet done criteria (no steps yet): {}", ids.join(", ")));
        }
    }
    for call_id in &state.pending_runs {
        lines.push(format!("- Uninterpreted `sympy_eval` run `{call_id}` (G1 pending)"));
    }
    if lines.is_empty() {
        return "## Open Items\n(none)\n".to_string();
    }
    format!("## Open Items\n{}\n", lines.join("\n"))
}

fn render_handles_section(state: &SolveState) -> String {
    if state.handles.is_empty() {
        return "## Handles\n(none)\n".to_string();
    }
    let mut out = String::from("## Handles\n");
    for (href, expr) in &state.handles {
        let owner = expr.owner_entry.as_deref().unwrap_or("unassigned");
        let port = if expr.portable { "portable" } else { "non-portable" };
        out.push_str(&format!(
            "- {href} ({port}, owner={owner}): `{}`\n",
            expr.str_preview.trim()
        ));
    }
    out
}

fn render_last_turns_verbatim(events: &[Event], max_turns: usize) -> String {
    let mut by_turn: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let mut current_turn = 0u64;
    for ev in events {
        if let Some(t) = ev.payload.get("turn").and_then(Value::as_u64) {
            current_turn = t;
        }
        let bucket = by_turn.entry(current_turn).or_default();
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                if let Some(p) = ev.payload.get("prompt").and_then(Value::as_str) {
                    if !p.trim().is_empty() {
                        bucket.push(format!("User: {p}"));
                    }
                }
            }
            "human.steer" => {
                if let Some(t) = ev.payload.get("text").and_then(Value::as_str) {
                    bucket.push(format!("User steer: {t}"));
                }
            }
            "model.text" => {
                if let Some(t) = ev.payload.get("text").and_then(Value::as_str) {
                    bucket.push(format!("Assistant: {t}"));
                }
            }
            "model.tool_use" => {
                let name = ev.payload.get("name").and_then(Value::as_str).unwrap_or("?");
                let id = ev.payload.get("id").and_then(Value::as_str).unwrap_or("?");
                let input = &ev.payload["input"];
                bucket.push(format!("ToolUse({id}, {name}): {input}"));
            }
            "model.tool_result" => {
                let id = ev.payload.get("tool_use_id").and_then(Value::as_str).unwrap_or("?");
                let err = ev.payload.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let text = ev.payload.get("text").and_then(Value::as_str).unwrap_or("");
                bucket.push(format!("ToolResult({id}, error={err}): {text}"));
            }
            _ => {}
        }
    }
    let non_empty: Vec<(u64, Vec<String>)> =
        by_turn.into_iter().filter(|(_, lines)| !lines.is_empty()).collect();
    if non_empty.is_empty() {
        return "## Recent Turns\n(none)\n".to_string();
    }
    let start = non_empty.len().saturating_sub(max_turns);
    let mut out = String::from("## Recent Turns\n");
    for (turn, lines) in &non_empty[start..] {
        out.push_str(&format!("### Turn {turn}\n"));
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
#[path = "resume_brief_tests.rs"]
mod tests;
