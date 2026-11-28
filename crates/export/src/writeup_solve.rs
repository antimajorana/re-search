//! Deterministic F15 Solve write-up adapter (`00-overview.md` 2.13, `solve.md` 5.6, R5, R22, R23).
//!
//! Pure local code with zero model calls:
//! - Verifies the ledger chain via [`Ledger::read_verified_session`] and folds [`SolveState`]
//! - Enforces Gate **G7** refusal when any closure step is `Failed`, `Stale`, or superseded
//! - Orders assumptions (`Given`, `AssumedByModel`, setup conventions) first, then topological
//!   derivation steps ending with the final answer
//! - Reads formulas **exclusively** from check-process `solve.expr.latex` (rule **R5**; ignores
//!   `SolveEntry.latex` and refuses missing or non-portable handles)
//! - Re-verifies every Cited quote via F4 `build_verified_quote` (`<= 300` chars, `(OCR)` + PDF
//!   page when scanned, `.bib` preferring `source_item.citekey`)
//! - Emits unconditional opening status line and closing `"How each statement was checked"` table
//!   with `"verified"` restricted strictly to `Tier::Verified` (rule **R22**).

use crate::writeup::{
    CheckTableRow, HowChecked, ProseBasis, Writeup, WriteupBlock, WriteupMode, WriteupSection,
};
use crate::writeup_solve_cite::SolveCiteResolver;
use ledger::{Event, Ledger};
use research_tools::library::Library;
use research_tools::paper_key::Mode;
use research_tools::proof_closure::{all_deps, closure_for_target, topological_order};
use research_tools::proof_dag::tool_reference_matches;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::solve_model::{
    Claim, ClaimForm, ClaimRelation, HandleRef, SolveDoneStatus, SolveEntry, SolveRunStatus,
    SolveSetup, StepEntryType, StepOrigin,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{badge_for_step, wording as solve_wording, StepTier, Tier};
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use std::collections::BTreeSet;
use std::path::Path;

/// Section headings & status strings governed by `00-overview.md` 2.13 / 3.3 and `solve.md` 5.5–5.6.
pub mod solve_writeup_wording {
    pub const SECTION_ASSUMPTIONS: &str = "Assumptions and Setup";
    pub const SECTION_DERIVATION: &str = "Derivation";
    pub const SECTION_FINAL_ANSWER: &str = "Final Answer";
    pub const LABEL_ASSUMPTIONS: &str = "sec:assumptions";
    pub const LABEL_DERIVATION: &str = "sec:derivation";
    pub const LABEL_FINAL_ANSWER: &str = "sec:final-answer";
    pub const STATUS_DONE: &str = "Done";
    pub const STATUS_DONE_OPEN_ITEMS: &str = "Done with open items";
    pub const STATUS_PAUSED_PREFIX: &str = "Paused: ";
    pub const STATUS_IN_PROGRESS: &str = "In progress";
    pub const STATUS_NOT_FINISHED: &str = "Not finished";
    pub const PROSE_MODEL_NOTE: &str = "prose: (model)";
    pub const DEFAULT_TITLE: &str = "Solve Derivation";
}

/// Build a deterministic [`Writeup`] for a verified Solve session on disk.
pub fn build_solve_writeup(data_dir: &Path, session: &str) -> Result<Writeup, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger =
        Ledger::open_existing(data_dir).map_err(|e| format!("failed to open ledger: {e}"))?;
    let (_report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("ledger verification failed for `{session}`: {e}"))?;
    build_solve_writeup_from_verified(data_dir, session, &events)
}

/// Build a deterministic Solve [`Writeup`] from an already hash-verified session event stream.
pub fn build_solve_writeup_from_verified(
    data_dir: &Path,
    session: &str,
    events: &[Event],
) -> Result<Writeup, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let mode = session_mode_from_events(events);
    if mode != Mode::Solve {
        return Err(format!("session `{session}` is in `{}` mode, not Solve mode", mode.as_str()));
    }

    let state = SolveState::from_events(events);
    let target_id = resolve_target_entry_id(&state)?;
    let closure_ids = closure_for_target(&state.entries, &target_id)?;
    enforce_gate_g7(&state, &closure_ids)?;

    let closure_entries: Vec<&SolveEntry> =
        closure_ids.iter().filter_map(|id| state.entry(id)).collect();
    let ordered_ids = topological_order(&closure_entries).map_err(|cyclic| {
        format!("Solve write-up refused: dependency cycle in closure ({})", cyclic.join(", "))
    })?;
    let closure_set: BTreeSet<String> = ordered_ids.iter().cloned().collect();
    let final_tier = state
        .tier(&target_id)
        .ok_or_else(|| format!("missing tier for final step `{target_id}`"))?;

    // Open F10/F13 library only when already present on disk so write-up stays read-only.
    let lib = data_dir
        .join("library")
        .join("library.db")
        .is_file()
        .then(|| Library::open(data_dir).ok())
        .flatten();
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let mut cite_resolver = SolveCiteResolver::new(session, events, &state, &sandbox, lib.as_ref());

    let mut assumption_ids = Vec::new();
    let mut derivation_ids = Vec::new();
    for id in &ordered_ids {
        if id == &target_id {
            continue;
        }
        let own = state.tier(id).map(|t| t.own).unwrap_or(Tier::Unchecked);
        if matches!(own, Tier::Given | Tier::AssumedByModel) {
            assumption_ids.push(id.clone());
        } else {
            derivation_ids.push(id.clone());
        }
    }

    let assumptions_note = state.setup.as_ref().and_then(summarize_setup_assumptions);
    let mut sections = Vec::new();
    let mut check_table = Vec::new();

    let mut assumption_blocks = state.setup.as_ref().map(build_setup_prose_blocks).unwrap_or_default();
    for id in &assumption_ids {
        append_step_blocks(
            &state, id, &closure_set, assumptions_note.as_deref(),
            &mut cite_resolver, &mut assumption_blocks, &mut check_table,
        )?;
    }
    if !assumption_blocks.is_empty() {
        sections.push(WriteupSection {
            title: solve_writeup_wording::SECTION_ASSUMPTIONS.to_string(),
            label: Some(solve_writeup_wording::LABEL_ASSUMPTIONS.to_string()),
            blocks: assumption_blocks,
        });
    }

    if !derivation_ids.is_empty() {
        let mut deriv_blocks = Vec::new();
        for id in &derivation_ids {
            append_step_blocks(
                &state, id, &closure_set, assumptions_note.as_deref(),
                &mut cite_resolver, &mut deriv_blocks, &mut check_table,
            )?;
        }
        sections.push(WriteupSection {
            title: solve_writeup_wording::SECTION_DERIVATION.to_string(),
            label: Some(solve_writeup_wording::LABEL_DERIVATION.to_string()),
            blocks: deriv_blocks,
        });
    }

    let mut final_blocks = Vec::new();
    append_step_blocks(
        &state, &target_id, &closure_set, assumptions_note.as_deref(),
        &mut cite_resolver, &mut final_blocks, &mut check_table,
    )?;
    sections.push(WriteupSection {
        title: solve_writeup_wording::SECTION_FINAL_ANSWER.to_string(),
        label: Some(solve_writeup_wording::LABEL_FINAL_ANSWER.to_string()),
        blocks: final_blocks,
    });

    let doc = Writeup {
        mode: WriteupMode::Solve,
        title: derive_title(state.setup.as_ref(), session),
        status_line: build_status_line(&state, &target_id, &ordered_ids, final_tier),
        final_effective_tier: Some(final_tier.effective),
        coverage_paragraph: None,
        prose_edited_by_model: false,
        sections,
        bibliography: cite_resolver.into_bibliography(),
        check_table,
    };
    doc.validate()?;
    Ok(doc)
}

fn resolve_target_entry_id(state: &SolveState) -> Result<String, String> {
    if matches!(state.run_status, SolveRunStatus::Done(_)) {
        if let Some(done) = &state.done {
            return Ok(done.final_entry_id.clone());
        }
    }
    for kind in [StepEntryType::FinalAnswer, StepEntryType::Result] {
        if let Some(step) = state
            .entries
            .iter()
            .rev()
            .find(|e| e.superseded_by.is_none() && e.entry_type == kind)
        {
            return Ok(step.entry_id.clone());
        }
    }
    if let Some(done) = &state.done {
        return Ok(done.final_entry_id.clone());
    }
    Err("Solve write-up requires a final answer step or recorded `solve.done`".to_string())
}

/// Gate G7 enforcement (`00-overview.md` 2.13 rule 2, `solve.md` 5.4 G7 & R23).
fn enforce_gate_g7(state: &SolveState, closure_ids: &[String]) -> Result<(), String> {
    let mut failed = BTreeSet::new();
    let mut stale = BTreeSet::new();
    let mut superseded = BTreeSet::new();

    for id in closure_ids {
        let entry = state
            .entry(id)
            .ok_or_else(|| format!("Solve write-up refused (Gate G7): missing step `{id}`"))?;
        if entry.superseded_by.is_some() {
            superseded.insert(id.clone());
        }
        let tier = state.tier(id).ok_or_else(|| {
            format!("Solve write-up refused (Gate G7): missing tier for step `{id}`")
        })?;
        if tier.own == Tier::Failed || tier.effective == Tier::Failed {
            failed.insert(id.clone());
            failed.extend(tier.broken_by.iter().cloned());
        }
        if tier.effective == Tier::Stale || tier.stale {
            stale.insert(id.clone());
        }
    }

    for (label, set) in [("Failed", failed), ("Stale", stale), ("superseded", superseded)] {
        if !set.is_empty() {
            let list: Vec<_> = set.into_iter().collect();
            return Err(format!(
                "Solve write-up refused (Gate G7): closure contains {label} step(s) ({})",
                list.join(", ")
            ));
        }
    }
    Ok(())
}

fn append_step_blocks(
    state: &SolveState,
    entry_id: &str,
    _closure_set: &BTreeSet<String>,
    assumptions_note: Option<&str>,
    cite_resolver: &mut SolveCiteResolver<'_>,
    blocks: &mut Vec<WriteupBlock>,
    check_table: &mut Vec<CheckTableRow>,
) -> Result<(), String> {
    let entry = state.entry(entry_id).ok_or_else(|| format!("unknown step `{entry_id}`"))?;
    let tier = state.tier(entry_id).ok_or_else(|| format!("missing tier for step `{entry_id}`"))?;
    let basis = prose_basis_for_origin(entry.origin);
    // Preserve every declared/derived dependency (including any dangling id that downgraded tier).
    let rests_on: Vec<String> = all_deps(entry).into_iter().map(str::to_string).collect();
    let resolved_cite = entry
        .source_ref
        .as_ref()
        .map(|qref| cite_resolver.resolve_step_citation(entry, qref))
        .transpose()?;
    let note = build_step_note(
        basis, tier, resolved_cite.as_ref().is_some_and(|c| c.ocr), assumptions_note,
    );
    let summary_text = match entry.summary.trim() {
        "" => format!("Step {} ({})", entry.entry_id, entry.entry_type.as_str()),
        s => s.to_string(),
    };
    let details = entry.details.trim();

    // Enforce R22 wording honesty on model-authored step text (`solve_done::lint_solve_summary_r22`).
    if basis == ProseBasis::Model {
        research_tools::solve_done::lint_solve_summary_r22(&summary_text, state, entry_id)?;
        if !details.is_empty() {
            research_tools::solve_done::lint_solve_summary_r22(details, state, entry_id)?;
        }
    }

    blocks.push(WriteupBlock::Statement {
        id: entry.entry_id.clone(),
        text: summary_text.clone(),
        how_checked: HowChecked::Solve(tier.effective),
        own_tier: Some(tier.own),
        rests_on: rests_on.clone(),
        note: note.clone(),
    });
    if !details.is_empty() && details != summary_text {
        blocks.push(WriteupBlock::Prose { text: details.to_string(), basis });
    }
    // Rule R5: read formulas EXCLUSIVELY from check-process `solve.expr.latex` (ignore `entry.latex`).
    blocks.extend(build_step_formulas(state, entry)?);
    if let Some(rc) = resolved_cite {
        blocks.push(WriteupBlock::Quote(Box::new(rc.quote)));
    }
    check_table.push(CheckTableRow {
        id: entry.entry_id.clone(),
        statement: summary_text,
        how_checked: HowChecked::Solve(tier.effective),
        own_tier: Some(tier.own),
        rests_on,
        note,
    });
    Ok(())
}

fn prose_basis_for_origin(origin: Option<StepOrigin>) -> ProseBasis {
    match origin {
        Some(StepOrigin::Setup | StepOrigin::Given | StepOrigin::User) => ProseBasis::User,
        Some(StepOrigin::Solver | StepOrigin::Worker) | None => ProseBasis::Model,
    }
}

/// Build [`WriteupBlock::Formula`] entries strictly from check-process `solve.expr.latex` (R5).
fn build_step_formulas(state: &SolveState, entry: &SolveEntry) -> Result<Vec<WriteupBlock>, String> {
    let mut out = Vec::new();
    if !entry.claims.is_empty() {
        for (idx, claim) in entry.claims.iter().enumerate() {
            let (latex, handle) = render_claim_from_handles(state, &entry.entry_id, idx, claim)?;
            out.push(WriteupBlock::Formula {
                latex,
                handle,
                label: Some(format!("eq:{}-c{}", entry.entry_id, idx + 1)),
            });
        }
        return Ok(out);
    }
    // Claimless steps may emit portable owned handles from `solve.expr.latex`; non-portable
    // handles are skipped here per `solve.md` 4.3 (they may not anchor claims or earn badges).
    for (idx, (href, expr)) in state
        .handles
        .iter()
        .filter(|(_, e)| {
            let owned = match e.owner_entry.as_deref() {
                Some(owner) => owner == entry.entry_id,
                None => entry.tool_call_ref.iter().chain(entry.tool_call_refs.iter()).any(|r| {
                    tool_reference_matches(r, &e.produced_by.call_id, e.produced_by.result_seq)
                }),
            };
            owned && e.portable && !e.latex.trim().is_empty()
        })
        .enumerate()
    {
        out.push(WriteupBlock::Formula {
            latex: expr.latex.trim().to_string(),
            handle: href.to_string(),
            label: Some(format!("eq:{}-h{}", entry.entry_id, idx + 1)),
        });
    }
    Ok(out)
}

fn render_claim_from_handles(
    state: &SolveState,
    entry_id: &str,
    idx: usize,
    claim: &Claim,
) -> Result<(String, String), String> {
    let lhs = resolve_portable_expr(state, entry_id, idx, "lhs", &claim.lhs)?;
    let rhs = resolve_portable_expr(state, entry_id, idx, "rhs", &claim.rhs)?;
    if claim.form == ClaimForm::Value && claim.lhs == claim.rhs {
        return Ok((lhs.to_string(), claim.lhs.to_string()));
    }
    let rel = match claim.relation {
        ClaimRelation::Eq => "=",
        ClaimRelation::Le => "\\le",
        ClaimRelation::Ge => "\\ge",
        ClaimRelation::Approx => "\\approx",
    };
    let handle = if claim.lhs == claim.rhs {
        claim.lhs.to_string()
    } else {
        format!("{}, {}", claim.lhs, claim.rhs)
    };
    Ok((format!("{lhs} {rel} {rhs}"), handle))
}

fn resolve_portable_expr<'a>(
    state: &'a SolveState,
    entry_id: &str,
    idx: usize,
    side: &str,
    href: &HandleRef,
) -> Result<&'a str, String> {
    let expr = state.handles.get(href).ok_or_else(|| {
        format!(
            "Solve write-up refused (R5): step `{entry_id}` claim {idx} {side} references missing handle `{href}`"
        )
    })?;
    if !expr.portable {
        return Err(format!(
            "Solve write-up refused (R5): step `{entry_id}` claim {idx} {side} references non-portable handle `{href}`"
        ));
    }
    let trimmed = expr.latex.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "Solve write-up refused (R5): handle `{href}` on step `{entry_id}` has empty check-process LaTeX"
        ));
    }
    Ok(trimmed)
}

fn build_step_note(
    basis: ProseBasis,
    tier: &StepTier,
    is_ocr_cite: bool,
    assumptions_note: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut push_uniq = |s: &str| {
        if !s.trim().is_empty() && !parts.iter().any(|p| p == s) {
            parts.push(s.to_string());
        }
    };
    if basis == ProseBasis::Model {
        push_uniq(solve_writeup_wording::PROSE_MODEL_NOTE);
    }
    for sub in &tier.sublabels {
        push_uniq(sub);
    }
    for flag in &tier.flags {
        push_uniq(flag);
    }
    if is_ocr_cite {
        push_uniq(solve_wording::FLAG_OCR_SOURCE);
    }
    if tier.own == Tier::Verified {
        if let Some(assump) = assumptions_note {
            push_uniq(&format!("assumptions: {assump}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn build_setup_prose_blocks(setup: &SolveSetup) -> Vec<WriteupBlock> {
    let mut blocks = Vec::new();
    let mut push = |text: String| blocks.push(WriteupBlock::Prose { text, basis: ProseBasis::User });
    if !setup.statement.trim().is_empty() {
        push(format!("Problem statement: {}", setup.statement.trim()));
    }
    if !setup.conventions.trim().is_empty() {
        push(format!("Conventions: {}", setup.conventions.trim()));
    }
    if !setup.assumptions.is_empty() {
        let items: Vec<&str> = setup
            .assumptions
            .iter()
            .map(|a| a.text.trim())
            .filter(|t| !t.is_empty())
            .collect();
        if !items.is_empty() {
            push(format!("Setup assumptions: {}.", items.join("; ")));
        }
    }
    if !setup.symbols.is_empty() {
        let syms: Vec<String> = setup
            .symbols
            .iter()
            .map(|s| {
                if s.assumptions.is_empty() {
                    s.name.clone()
                } else {
                    format!("{} ({})", s.name, s.assumptions.join(", "))
                }
            })
            .collect();
        push(format!("Declared symbols: {}.", syms.join("; ")));
    }
    if !setup.dimensions.is_empty() {
        let dims: Vec<String> = setup
            .dimensions
            .iter()
            .map(|(sym, map)| {
                let expr = map.iter().map(|(b, p)| format!("{b}^{p}")).collect::<Vec<_>>().join(" ");
                format!("{sym}: [{expr}]")
            })
            .collect();
        push(format!("Dimensions: {}.", dims.join("; ")));
    }
    blocks
}

fn summarize_setup_assumptions(setup: &SolveSetup) -> Option<String> {
    let mut items: Vec<String> = setup
        .symbols
        .iter()
        .filter(|s| !s.assumptions.is_empty())
        .map(|s| format!("{} ({})", s.name, s.assumptions.join(", ")))
        .collect();
    for a in &setup.assumptions {
        let t = a.text.trim();
        if !t.is_empty() && !items.iter().any(|existing| existing == t) {
            items.push(t.to_string());
        }
    }
    if !setup.conventions.trim().is_empty() {
        items.push(setup.conventions.trim().to_string());
    }
    (!items.is_empty()).then(|| items.join("; "))
}

fn derive_title(setup: Option<&SolveSetup>, session: &str) -> String {
    if let Some(s) = setup {
        for cand in [s.target.description.trim(), s.statement.trim()] {
            if !cand.is_empty() {
                let first = cand.lines().next().unwrap_or(cand).trim();
                return if first.chars().count() <= 120 {
                    first.to_string()
                } else {
                    let prefix: String = first.chars().take(117).collect();
                    format!("{}...", prefix.trim_end())
                };
            }
        }
    }
    format!("{}: {session}", solve_writeup_wording::DEFAULT_TITLE)
}

/// Format opening status line matching `state.run_status` (and `solve.done.status` when Done)
/// while strictly obeying rule R22 (`"verified"` appears ONLY when `final_tier.effective == Verified`).
fn build_status_line(
    state: &SolveState,
    target_id: &str,
    closure_ids: &[String],
    final_tier: &StepTier,
) -> String {
    let final_label = badge_for_step(state, target_id)
        .map(|b| b.label)
        .unwrap_or_else(|| final_tier.effective.label().to_string());

    match state.run_status {
        SolveRunStatus::Done(SolveDoneStatus::Finished) => {
            format!("{} (final answer: {final_label})", solve_writeup_wording::STATUS_DONE)
        }
        SolveRunStatus::Done(SolveDoneStatus::FinishedOpenItems) => {
            let mut open_parts = Vec::new();
            let unchecked = closure_ids
                .iter()
                .filter(|id| state.tier(id).is_some_and(|t| t.own == Tier::Unchecked))
                .count();
            let model_assump = closure_ids
                .iter()
                .filter(|id| state.tier(id).is_some_and(|t| t.own == Tier::AssumedByModel))
                .count();
            let unmet = state
                .done
                .as_ref()
                .map(|d| d.criteria.iter().filter(|c| !c.met).count())
                .unwrap_or(0);
            if unchecked > 0 {
                open_parts.push(format!("{unchecked} unchecked"));
            }
            if model_assump > 0 {
                open_parts.push(format!("{model_assump} assumed by model"));
            }
            if unmet > 0 {
                open_parts.push(format!("{unmet} unmet criteria"));
            }
            if open_parts.is_empty() {
                format!(
                    "{} (final answer: {final_label})",
                    solve_writeup_wording::STATUS_DONE_OPEN_ITEMS
                )
            } else {
                format!(
                    "{}: {} (final answer: {final_label})",
                    solve_writeup_wording::STATUS_DONE_OPEN_ITEMS,
                    open_parts.join(", ")
                )
            }
        }
        SolveRunStatus::Paused(reason) => format!(
            "{}{} (final answer: {final_label})",
            solve_writeup_wording::STATUS_PAUSED_PREFIX,
            reason.as_str()
        ),
        SolveRunStatus::Running => format!(
            "{} (final answer: {final_label})",
            solve_writeup_wording::STATUS_IN_PROGRESS
        ),
        SolveRunStatus::Idle => format!(
            "{} (final answer: {final_label})",
            solve_writeup_wording::STATUS_NOT_FINISHED
        ),
    }
}
