//! Deterministic F15 Explore write-up adapter (Cited Summary): pure ledger-to-[`Writeup`]
//! projection with zero model calls and zero network calls (`00-overview.md` 0.1, 2.5, 2.9–2.13,
//! 3.2 rows 17–24, 7.2 Set W, 7.3, 8.1, 8.3 Q3 & Q10; `explore.md` 4.1–4.4, 6 C1–C9/V1–V10/
//! D1–D3/S8–S9, 9 X17).
//!
//! - Recomputes F12 [`research_tools::search_coverage::SearchCoverage`] and per-subclaim
//!   [`research_tools::prior_work::PriorWorkVerdict`] via `idea_verdict::compute` (Rules V7, V9, V10)
//!   and fails closed if a stored `idea.verdict` disagrees or if `idea.stated`/`idea.brief` is absent.
//! - Re-verifies every `xe-*` quote via F4 `verify_quote_ref` and every `xa-*` abstract quote
//!   against its pinned `raw_blob`, dropping unverified quotes and clipping to `<= 300` UTF-8 chars.
//! - Tags every OCR/scanned quote and OCR-backed `judged_*` verdict with `"(OCR)"` and physical page.
//! - Emits unconditional status line, `"Where I looked"` coverage paragraph, and closing
//!   `"How each statement was checked"` table with zero `"verified"` anywhere in Explore output.

#[path = "writeup_idea_cite.rs"]
mod cite;
#[path = "writeup_idea_ocr.rs"]
mod ocr;
#[path = "writeup_idea_sections.rs"]
mod sections;

use crate::writeup::{
    writeup_wording, CheckTableRow, HowChecked, ProseBasis, RejectedWriteupQuote, Writeup,
    WriteupBlock, WriteupMode, WriteupSection,
};
use cite::{format_verified_abstract_line, open_existing_library, IdeaCiteResolver};
use ledger::{Event, Ledger};
use research_tools::idea_brief::{EVENT_IDEA_ANSWER, EVENT_IDEA_BRIEF, UI_SHARPENED_IDEA_LABEL};
use research_tools::idea_verdict::{self, IdeaVerdictPayload};
use research_tools::paper_key::Mode;
use research_tools::prior_work::{EvidenceId, PriorWorkVerdict, IDEA_VERDICT_EVENT};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::search_coverage::IDEA_STATED_EVENT;
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use sections::{
    append_directions_sections, build_coverage_paragraph, build_status_line, clip_title,
    derive_title, format_ocr_tag_and_pages, map_display_grade, push_unique, validate_model_prose,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Centralized Explore write-up wording constants governed by accepted owner decisions
/// (`00-overview.md` 0.1, 2.9–2.10, 2.13, 3.2 rows 17–24, 8.1, 8.3 Q3 & Q10; `explore.md` 3 & S8).
/// None contain `"verified"`, `"new"`, `"novel"`, or `"proved"`.
pub mod explore_writeup_wording {
    /// Default title prefix (`00-overview.md` 2.13).
    pub const TITLE_PREFIX: &str = "Explore Cited Summary";
    /// Opening idea & sharpened-parts section (`00-overview.md` 3.2 row 18: never `"brief"` on screen).
    pub const SECTION_IDEA: &str = "Idea and Sharpened Parts";
    pub const LABEL_IDEA: &str = "sec:idea";
    /// Concrete directions section (`explore.md` 3 & Rule D2).
    pub const SECTION_DIRECTIONS: &str = "Directions";
    pub const LABEL_DIRECTIONS: &str = "sec:directions";
    /// Sketch directions section (`explore.md` 3 & Rule D2).
    pub const SECTION_LOOSE_IDEAS: &str = "Loose ideas";
    pub const LABEL_LOOSE_IDEAS: &str = "sec:loose-ideas";
    /// Model-authored difference label (`explore.md` 3, `00-overview.md` 2.13 & 3.2 row 24).
    pub const WHAT_DIFFERS_LABEL: &str = "What differs";
    /// Sharpened restatement label (`00-overview.md` 3.2 row 18).
    pub const SHARPENED_RESTATEMENT_LABEL: &str = "Sharpened idea";
    /// Assumptions label.
    pub const ASSUMPTIONS_LABEL: &str = "Assumptions";
    /// Origin-paper disclosure (Rule V9).
    pub const ORIGIN_EXCLUDED_LABEL: &str =
        "Origin paper excluded from prior-work verdicts (Rule V9)";
    /// Direction concreteness labels (`explore.md` 3 & Rule D2).
    pub const CONCRETE_LABEL: &str = "Concrete";
    pub const SKETCH_LABEL: &str = "Loose idea";
    /// Direction free-text provenance note (`explore.md` Rule D3).
    pub const SUGGESTION_NOTE: &str = "suggestion (model)";
    /// Coverage completeness summaries for status line & coverage paragraph.
    pub const COVERAGE_COMPLETE: &str = "search scope complete";
    pub const COVERAGE_INCOMPLETE: &str = "search scope incomplete";
    /// Omitted-quote phrasing (`00-overview.md` 3.2 row 20: quotes are `"matched"`, never `"verified"`).
    pub const QUOTE_OMITTED_PHRASE: &str = "could not be matched and omitted";

    /// All static Explore write-up constants, checked by Set W wording tests.
    pub const ALL_STATIC_CONSTANTS: &[&str] = &[
        TITLE_PREFIX,
        SECTION_IDEA,
        SECTION_DIRECTIONS,
        SECTION_LOOSE_IDEAS,
        WHAT_DIFFERS_LABEL,
        SHARPENED_RESTATEMENT_LABEL,
        ASSUMPTIONS_LABEL,
        ORIGIN_EXCLUDED_LABEL,
        CONCRETE_LABEL,
        SKETCH_LABEL,
        SUGGESTION_NOTE,
        COVERAGE_COMPLETE,
        COVERAGE_INCOMPLETE,
        QUOTE_OMITTED_PHRASE,
    ];
}

/// Detailed result of building an Explore cited-summary [`Writeup`], including any quotes
/// dropped because F4 / pinned `raw_blob` re-verification failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExploreWriteupBuild {
    pub writeup: Writeup,
    pub rejected_quotes: Vec<RejectedWriteupQuote>,
}

/// Build a deterministic Explore cited-summary [`Writeup`] for a verified session on disk.
pub fn build_explore_writeup(data_dir: &Path, session: &str) -> Result<Writeup, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger =
        Ledger::open_existing(data_dir).map_err(|e| format!("failed to open ledger: {e}"))?;
    let (_report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("ledger verification failed for `{session}`: {e}"))?;
    build_explore_writeup_from_verified(data_dir, session, &events)
}

/// Build a deterministic Explore cited-summary [`Writeup`] from an already hash-verified session
/// event stream (called by `export::build_session_writeup`).
pub fn build_explore_writeup_from_verified(
    data_dir: &Path,
    session: &str,
    events: &[Event],
) -> Result<Writeup, String> {
    build_explore_writeup_detailed(data_dir, session, events).map(|b| b.writeup)
}

/// Build an Explore cited-summary [`Writeup`] and return any dropped quotes for inspection/tests.
pub fn build_explore_writeup_detailed(
    data_dir: &Path,
    session: &str,
    events: &[Event],
) -> Result<ExploreWriteupBuild, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let mode = session_mode_from_events(events);
    if mode != Mode::Explore {
        return Err(format!(
            "session `{session}` is in `{}` mode, not Explore mode",
            mode.as_str()
        ));
    }

    let stmt_ev = events
        .iter()
        .rev()
        .find(|e| e.kind == IDEA_STATED_EVENT)
        .ok_or_else(|| {
            format!("Explore write-up refused: session `{session}` has no `idea.stated` event")
        })?;
    let latest_stmt_seq = stmt_ev.seq;
    let idea_text = stmt_ev
        .payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!("Explore write-up refused: latest `idea.stated` event in `{session}` has empty text")
        })?;

    let brief_ev = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > latest_stmt_seq)
        .ok_or_else(|| {
            format!(
                "Explore write-up refused: active revision in `{session}` has no `idea.brief` event"
            )
        })?;
    let brief_view = fold_active_brief(events, latest_stmt_seq, brief_ev);

    // Rules V7 & V10 fail-closed check: verify stored `idea.verdict` at its own ledger head.
    verify_stored_verdict_parity(events, latest_stmt_seq)?;

    // Recompute current verdicts & coverage purely over verified events (reflecting any
    // post-verdict Rule V6 `confirm_same` / `reject_same` notes while ignoring F16 lessons).
    let current = idea_verdict::compute(events, None);
    let coverage_paragraph = build_coverage_paragraph(&current.coverage)?;

    let lib = open_existing_library(data_dir);
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let mut cite_resolver = IdeaCiteResolver::new(
        data_dir,
        session,
        events,
        latest_stmt_seq,
        &sandbox,
        lib.as_ref(),
    );

    let mut sections = Vec::new();
    let mut check_table = Vec::new();

    sections.push(build_idea_section(
        idea_text,
        &brief_view,
        current.origin_paper.as_deref(),
    )?);

    for verdict in &current.verdicts {
        let subclaim_text = brief_view
            .subclaim_texts
            .get(&verdict.subclaim_id)
            .map(String::as_str)
            .unwrap_or(verdict.subclaim_id.as_str());
        let sec = build_subclaim_section(
            verdict,
            subclaim_text,
            &mut cite_resolver,
            &mut check_table,
        )?;
        sections.push(sec);
    }

    append_directions_sections(
        events,
        latest_stmt_seq,
        current.map_seq,
        &mut sections,
        &mut check_table,
    )?;

    let status_line = build_status_line(events, latest_stmt_seq, &current);
    let title = derive_title(idea_text, &brief_view.restatement, session);
    let rejected_quotes = std::mem::take(&mut cite_resolver.rejected_quotes);
    let bibliography = cite_resolver.into_bibliography();

    let doc = Writeup {
        mode: WriteupMode::Explore,
        title,
        status_line,
        final_effective_tier: None,
        coverage_paragraph: Some(coverage_paragraph),
        prose_edited_by_model: false,
        sections,
        bibliography,
        check_table,
    };
    enforce_explore_honesty(&doc)?;
    doc.validate()?;

    Ok(ExploreWriteupBuild {
        writeup: doc,
        rejected_quotes,
    })
}

struct ActiveBriefView {
    restatement: String,
    assumptions: Vec<String>,
    subclaim_texts: BTreeMap<String, String>,
}

fn fold_active_brief(events: &[Event], latest_stmt_seq: u64, brief_ev: &Event) -> ActiveBriefView {
    let mut restatement = brief_ev
        .payload
        .get("restatement")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let mut assumptions: Vec<String> = brief_ev
        .payload
        .get("assumptions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut subclaim_texts = BTreeMap::new();
    if let Some(arr) = brief_ev.payload.get("subclaims").and_then(Value::as_array) {
        for sc in arr {
            if let (Some(id), Some(txt)) = (
                sc.get("id").and_then(Value::as_str),
                sc.get("text").and_then(Value::as_str),
            ) {
                subclaim_texts.insert(id.to_string(), txt.trim().to_string());
            }
        }
    }
    for ev in events {
        if ev.kind != EVENT_IDEA_ANSWER || ev.seq <= latest_stmt_seq || ev.seq < brief_ev.seq {
            continue;
        }
        if let Some(r) = ev.payload.get("restatement").and_then(Value::as_str) {
            restatement = r.trim().to_string();
        }
        if let Some(arr) = ev.payload.get("assumptions").and_then(Value::as_array) {
            assumptions = arr
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    ActiveBriefView {
        restatement,
        assumptions,
        subclaim_texts,
    }
}

/// Rule V7 & V10 fail-closed verification: if an `idea.verdict` exists in the active revision,
/// its stored payload must exactly match pure ledger recomputation at that event's ledger head.
fn verify_stored_verdict_parity(events: &[Event], latest_stmt_seq: u64) -> Result<(), String> {
    let Some((verdict_idx, verdict_ev)) = events
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| e.kind == IDEA_VERDICT_EVENT && e.seq > latest_stmt_seq)
    else {
        return Ok(());
    };
    let stored: IdeaVerdictPayload =
        serde_json::from_value(verdict_ev.payload.clone()).map_err(|e| {
            format!("Explore write-up refused (Rule V7): stored `idea.verdict` is malformed ({e})")
        })?;
    let recomputed = idea_verdict::compute(&events[..verdict_idx], None);
    let map_seq_ok = stored.map_seq.is_none_or(|ms| Some(ms) == recomputed.map_seq);
    if stored.verdicts != recomputed.verdicts
        || stored.coverage != recomputed.coverage
        || stored.origin_paper != recomputed.origin_paper
        || !map_seq_ok
    {
        return Err(
            "Explore write-up refused (Rule V7): stored `idea.verdict` disagrees with pure ledger recomputation"
                .to_string(),
        );
    }
    Ok(())
}

fn build_idea_section(
    idea_text: &str,
    brief: &ActiveBriefView,
    origin_paper: Option<&str>,
) -> Result<WriteupSection, String> {
    let mut blocks = vec![WriteupBlock::Prose {
        text: idea_text.to_string(),
        basis: ProseBasis::User,
    }];
    if !brief.restatement.is_empty() {
        validate_model_prose(&brief.restatement, "idea.brief.restatement")?;
        blocks.push(WriteupBlock::Prose {
            text: format!("{UI_SHARPENED_IDEA_LABEL}: {}", brief.restatement),
            basis: ProseBasis::Model,
        });
    }
    if !brief.assumptions.is_empty() {
        for assumption in &brief.assumptions {
            validate_model_prose(assumption, "idea.brief.assumptions")?;
        }
        blocks.push(WriteupBlock::Prose {
            text: format!(
                "{}: {}",
                explore_writeup_wording::ASSUMPTIONS_LABEL,
                brief.assumptions.join("; ")
            ),
            basis: ProseBasis::Model,
        });
    }
    if let Some(orig) = origin_paper.filter(|s| !s.trim().is_empty()) {
        blocks.push(WriteupBlock::Prose {
            text: format!("{}: {orig}", explore_writeup_wording::ORIGIN_EXCLUDED_LABEL),
            basis: ProseBasis::App,
        });
    }
    Ok(WriteupSection {
        title: explore_writeup_wording::SECTION_IDEA.to_string(),
        label: Some(explore_writeup_wording::LABEL_IDEA.to_string()),
        blocks,
    })
}

fn build_subclaim_section(
    verdict: &PriorWorkVerdict,
    subclaim_text: &str,
    cite_resolver: &mut IdeaCiteResolver<'_>,
    check_table: &mut Vec<CheckTableRow>,
) -> Result<WriteupSection, String> {
    let sec_title = format!(
        "Part {}: {}",
        verdict.subclaim_id,
        clip_title(subclaim_text, 64)
    );
    let mut quote_blocks: Vec<WriteupBlock> = Vec::new();
    let mut diff_rows: Vec<CheckTableRow> = Vec::new();
    let mut rests_on: Vec<String> = Vec::new();
    let mut any_ocr = verdict.has_ocr_evidence;
    let mut ocr_pages: Vec<usize> = Vec::new();
    let mut dropped_count = 0usize;

    for (ov_idx, overlap) in verdict.overlaps.iter().enumerate() {
        for eid in &overlap.evidence_ids {
            if let EvidenceId::FullText(seq) = *eid {
                match cite_resolver.resolve_full_text_quote(&sec_title, &overlap.paper_key, seq)? {
                    Some(resolved) => {
                        if resolved.ocr {
                            any_ocr = true;
                            if let Some(pg) = resolved.page {
                                if !ocr_pages.contains(&pg) {
                                    ocr_pages.push(pg);
                                }
                            }
                        }
                        push_unique(&mut rests_on, format!("xe-{seq}"));
                        push_unique(&mut rests_on, resolved.quote.cite_key.clone());
                        quote_blocks.push(WriteupBlock::Quote(Box::new(resolved.quote)));
                    }
                    None => dropped_count += 1,
                }
            }
        }

        let diff_trimmed = overlap.difference.trim();
        if !diff_trimmed.is_empty() {
            validate_model_prose(diff_trimmed, "overlap.difference")?;
            let paper_label = cite_resolver
                .existing_cite_key(&overlap.paper_key)
                .unwrap_or_else(|| overlap.paper_key.as_str())
                .to_string();
            quote_blocks.push(WriteupBlock::Prose {
                text: format!(
                    "{} ({paper_label}): {diff_trimmed}",
                    explore_writeup_wording::WHAT_DIFFERS_LABEL
                ),
                basis: ProseBasis::Model,
            });
            diff_rows.push(CheckTableRow {
                id: format!("{}-diff-{}", verdict.subclaim_id, ov_idx + 1),
                statement: format!(
                    "{} ({paper_label})",
                    explore_writeup_wording::WHAT_DIFFERS_LABEL
                ),
                how_checked: HowChecked::ModelStatement,
                own_tier: None,
                rests_on: vec![paper_label],
                note: Some(format!(
                    "{} {}",
                    explore_writeup_wording::WHAT_DIFFERS_LABEL,
                    writeup_wording::MODEL_SUFFIX
                )),
            });
        }
    }

    for abs in &verdict.abstract_matches {
        if let EvidenceId::Abstract(seq) = abs.evidence_id {
            match cite_resolver.resolve_abstract_quote(&sec_title, &abs.paper_key, seq)? {
                Some(resolved_abs) => {
                    push_unique(&mut rests_on, resolved_abs.evidence_id.clone());
                    push_unique(&mut rests_on, resolved_abs.cite_key.clone());
                    quote_blocks.push(WriteupBlock::Prose {
                        text: format_verified_abstract_line(&resolved_abs),
                        basis: ProseBasis::App,
                    });
                }
                None => dropped_count += 1,
            }
        }
    }

    if rests_on.is_empty() {
        rests_on.push("coverage gate".to_string());
    }

    let how_checked = map_display_grade(
        verdict.status.display_grade(verdict.confirmed_by_user, any_ocr),
    );
    let ocr_suffix = format_ocr_tag_and_pages(any_ocr, &ocr_pages);
    let mut note_parts = vec![verdict.chip.clone()];
    if let Some(ref ocr_str) = ocr_suffix {
        note_parts.push(ocr_str.clone());
    }
    if dropped_count > 0 {
        let qw = if dropped_count == 1 { "quote" } else { "quotes" };
        note_parts.push(format!(
            "{dropped_count} {qw} {}",
            explore_writeup_wording::QUOTE_OMITTED_PHRASE
        ));
    }
    let note = Some(note_parts.join("; "));
    let stmt_text = match &ocr_suffix {
        Some(ocr_str) => format!("{subclaim_text} — {} {ocr_str}", verdict.chip),
        None => format!("{subclaim_text} — {}", verdict.chip),
    };

    check_table.push(CheckTableRow {
        id: verdict.subclaim_id.clone(),
        statement: subclaim_text.to_string(),
        how_checked,
        own_tier: None,
        rests_on: rests_on.clone(),
        note: note.clone(),
    });
    check_table.extend(diff_rows);

    let mut blocks = Vec::with_capacity(2 + quote_blocks.len());
    blocks.push(WriteupBlock::Statement {
        id: verdict.subclaim_id.clone(),
        text: stmt_text,
        how_checked,
        own_tier: None,
        rests_on,
        note,
    });
    // Rule S9: emit the exact F12 app-authored verdict sentence.
    blocks.push(WriteupBlock::Prose {
        text: verdict.text.clone(),
        basis: ProseBasis::App,
    });
    blocks.extend(quote_blocks);

    Ok(WriteupSection {
        title: sec_title,
        label: Some(format!("sec:{}", verdict.subclaim_id)),
        blocks,
    })
}

fn enforce_explore_honesty(doc: &Writeup) -> Result<(), String> {
    if doc.status_line.to_ascii_lowercase().contains("verified") {
        return Err("Explore write-up status_line must not contain \"verified\" (Rule S8)".into());
    }
    if let Some(cov) = doc.coverage_paragraph.as_deref() {
        if cov.to_ascii_lowercase().contains("verified") {
            return Err("Explore coverage paragraph must not contain \"verified\" (Rule S8)".into());
        }
    }
    for row in doc.derived_check_table() {
        if row.badge_text().to_ascii_lowercase().contains("verified") {
            return Err(format!(
                "Explore check table row `{}` must not contain \"verified\" (Rule S8)",
                row.id
            ));
        }
    }
    Ok(())
}
