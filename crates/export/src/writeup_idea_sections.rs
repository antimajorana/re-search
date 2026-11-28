//! Optional Directions builder, F12 coverage paragraph, F9 status line, and wording validators
//! for the F15 Explore write-up adapter (`writeup_idea.rs`).

use super::explore_writeup_wording;
use crate::writeup::{
    contains_forbidden_word, writeup_wording, CheckTableRow, HowChecked, ProseBasis, WriteupBlock,
    WriteupSection,
};
use ledger::Event;
use research_tools::idea_verdict::IdeaVerdictPayload;
use research_tools::prior_work::DisplayGrade;
use research_tools::prior_work_wording::{
    check_model_prose_phrases, contains_forbidden_prior_work_word, format_coverage_scope,
    format_gate_failure,
};
use research_tools::quote_core::clip_to_chars;
use research_tools::run_budget::{explore_stage_run_state, ExploreStageState, RunState};
use research_tools::search_coverage::SearchCoverage;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const EVENT_IDEA_STAGE: &str = "idea.stage";
const EVENT_IDEA_DIRECTIONS: &str = "idea.directions";
const STAGE_ORDER: [&str; 7] = [
    "idea",
    "sharpen",
    "search",
    "triage",
    "read",
    "map",
    "directions",
];

pub(crate) fn append_directions_sections(
    events: &[Event],
    latest_stmt_seq: u64,
    latest_map_seq: Option<u64>,
    sections: &mut Vec<WriteupSection>,
    check_table: &mut Vec<CheckTableRow>,
) -> Result<(), String> {
    // Audit M3: bind `idea.directions` strictly to the current active map revision.
    let dir_ev = events.iter().rev().find(|e| {
        if e.kind != EVENT_IDEA_DIRECTIONS || e.seq <= latest_stmt_seq {
            return false;
        }
        match (e.payload.get("map_seq").and_then(Value::as_u64), latest_map_seq) {
            (Some(dm), Some(lm)) => dm == lm && e.seq > lm,
            (None, Some(lm)) => e.seq > lm,
            (Some(_), None) => false,
            (None, None) => true,
        }
    });
    let Some(ev) = dir_ev else { return Ok(()) };
    let Some(arr) = ev.payload.get("directions").and_then(Value::as_array) else {
        return Ok(());
    };
    if arr.is_empty() {
        return Ok(());
    }

    let mut used_ids: BTreeSet<String> = check_table.iter().map(|r| r.id.clone()).collect();
    let mut concrete_blocks = Vec::new();
    let mut concrete_rows = Vec::new();
    let mut sketch_blocks = Vec::new();
    let mut sketch_rows = Vec::new();

    for (idx, item) in arr.iter().enumerate() {
        if !item.is_object() {
            continue;
        }
        let is_concrete = item
            .get("concreteness")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("concrete"));
        let (target_blocks, target_rows, label) = if is_concrete {
            (&mut concrete_blocks, &mut concrete_rows, explore_writeup_wording::CONCRETE_LABEL)
        } else {
            (&mut sketch_blocks, &mut sketch_rows, explore_writeup_wording::SKETCH_LABEL)
        };
        append_one_direction(item, idx, label, &mut used_ids, target_blocks, target_rows)?;
    }

    // Emit sections and check-table rows in Concrete-then-Sketch document order (Audit M3).
    if !concrete_blocks.is_empty() {
        sections.push(WriteupSection {
            title: explore_writeup_wording::SECTION_DIRECTIONS.to_string(),
            label: Some(explore_writeup_wording::LABEL_DIRECTIONS.to_string()),
            blocks: concrete_blocks,
        });
        check_table.extend(concrete_rows);
    }
    if !sketch_blocks.is_empty() {
        sections.push(WriteupSection {
            title: explore_writeup_wording::SECTION_LOOSE_IDEAS.to_string(),
            label: Some(explore_writeup_wording::LABEL_LOOSE_IDEAS.to_string()),
            blocks: sketch_blocks,
        });
        check_table.extend(sketch_rows);
    }
    Ok(())
}

fn append_one_direction(
    item: &Value,
    idx: usize,
    concreteness_label: &str,
    used_ids: &mut BTreeSet<String>,
    blocks: &mut Vec<WriteupBlock>,
    rows: &mut Vec<CheckTableRow>,
) -> Result<(), String> {
    let base_id = item
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("r-{}", idx + 1));
    validate_model_prose(&base_id, "direction.id")?;
    let title = item.get("title").and_then(Value::as_str).unwrap_or("").trim();
    let question = item.get("question").and_then(Value::as_str).unwrap_or("").trim();
    validate_model_prose(title, "direction.title")?;
    validate_model_prose(question, "direction.question")?;

    let mut refs = Vec::new();
    let mut prose_lines = Vec::new();
    collect_direction_known(item, &mut refs, &mut prose_lines)?;
    collect_direction_missing(item, &mut refs, &mut prose_lines)?;
    collect_direction_first_calc(item, &mut refs, &mut prose_lines)?;
    collect_direction_risks(item, &mut refs, &mut prose_lines)?;
    if let Some(stop_if) =
        item.get("stop_if").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
    {
        validate_model_prose(stop_if, "direction.stop_if")?;
        prose_lines.push(format!("Stop if: {stop_if}"));
    }

    if title.is_empty() && question.is_empty() && prose_lines.is_empty() {
        return Ok(());
    }

    let id = deduplicate_id(&base_id, used_ids);
    let heading_text = if title.is_empty() && question.is_empty() {
        format!("{id} ({concreteness_label})")
    } else if title.is_empty() {
        format!("{question} ({concreteness_label})")
    } else if question.is_empty() {
        format!("{title} ({concreteness_label})")
    } else {
        format!("{title} ({concreteness_label}) — {question}")
    };
    let note = Some(format!(
        "{concreteness_label}; {}",
        explore_writeup_wording::SUGGESTION_NOTE
    ));
    rows.push(CheckTableRow {
        id: id.clone(),
        statement: if title.is_empty() { id.clone() } else { title.to_string() },
        how_checked: HowChecked::ModelStatement,
        own_tier: None,
        rests_on: refs.clone(),
        note: note.clone(),
    });
    blocks.push(WriteupBlock::Statement {
        id,
        text: heading_text,
        how_checked: HowChecked::ModelStatement,
        own_tier: None,
        rests_on: refs,
        note,
    });
    for line in prose_lines {
        blocks.push(WriteupBlock::Prose {
            text: line,
            basis: ProseBasis::Model,
        });
    }
    Ok(())
}

fn deduplicate_id(base: &str, used: &mut BTreeSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

fn collect_direction_known(
    item: &Value,
    refs: &mut Vec<String>,
    prose_lines: &mut Vec<String>,
) -> Result<(), String> {
    let Some(known) = item.get("known").and_then(Value::as_array) else { return Ok(()) };
    let mut parts = Vec::new();
    for k in known {
        let txt = k.get("text").and_then(Value::as_str).unwrap_or("").trim();
        validate_model_prose(txt, "direction.known")?;
        let nids = extract_string_array(k.get("node_ids"));
        for nid in &nids {
            validate_model_prose(nid, "direction.known.node_ids")?;
            push_unique(refs, nid.clone());
        }
        if !txt.is_empty() {
            if nids.is_empty() {
                parts.push(txt.to_string());
            } else {
                parts.push(format!("{txt} [{}]", nids.join(", ")));
            }
        }
    }
    if !parts.is_empty() {
        prose_lines.push(format!("Known: {}", parts.join("; ")));
    }
    Ok(())
}

fn collect_direction_missing(
    item: &Value,
    refs: &mut Vec<String>,
    prose_lines: &mut Vec<String>,
) -> Result<(), String> {
    let Some(missing) = item.get("missing").and_then(Value::as_array) else { return Ok(()) };
    let mut parts = Vec::new();
    for m in missing {
        let txt = m.get("text").and_then(Value::as_str).unwrap_or("").trim();
        validate_model_prose(txt, "direction.missing")?;
        if let Some(basis) = m.get("basis").and_then(Value::as_str) {
            if let Some(qid) = basis
                .strip_prefix("open_question(")
                .and_then(|s| s.strip_suffix(')'))
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                validate_model_prose(qid, "direction.missing.basis")?;
                push_unique(refs, qid.to_string());
            }
        }
        if !txt.is_empty() {
            parts.push(txt.to_string());
        }
    }
    if !parts.is_empty() {
        prose_lines.push(format!("Missing: {}", parts.join("; ")));
    }
    Ok(())
}

fn collect_direction_first_calc(
    item: &Value,
    refs: &mut Vec<String>,
    prose_lines: &mut Vec<String>,
) -> Result<(), String> {
    let Some(fc) = item.get("first_calculation").and_then(Value::as_object) else { return Ok(()) };
    let setup = fc.get("setup").and_then(Value::as_str).unwrap_or("").trim();
    let method = fc.get("method").and_then(Value::as_str).unwrap_or("").trim();
    let expected = fc.get("expected_form").and_then(Value::as_str).unwrap_or("").trim();
    let effort = fc.get("effort").and_then(Value::as_str).unwrap_or("").trim();
    validate_model_prose(setup, "direction.first_calculation.setup")?;
    validate_model_prose(method, "direction.first_calculation.method")?;
    validate_model_prose(expected, "direction.first_calculation.expected_form")?;
    validate_model_prose(effort, "direction.first_calculation.effort")?;
    for inp in extract_string_array(fc.get("inputs")) {
        validate_model_prose(&inp, "direction.first_calculation.inputs")?;
        push_unique(refs, inp);
    }
    let mut fc_parts = Vec::new();
    if !setup.is_empty() { fc_parts.push(setup.to_string()); }
    if !method.is_empty() { fc_parts.push(format!("method: {method}")); }
    if !expected.is_empty() { fc_parts.push(format!("expected form: {expected}")); }
    if let Some(chk) = fc.get("check").and_then(Value::as_object) {
        let ctxt = chk.get("text").and_then(Value::as_str).unwrap_or("").trim();
        validate_model_prose(ctxt, "direction.first_calculation.check")?;
        if let Some(rep) = chk
            .get("reproduces")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            validate_model_prose(rep, "direction.first_calculation.check.reproduces")?;
            push_unique(refs, rep.to_string());
            if !ctxt.is_empty() {
                fc_parts.push(format!("check first: {ctxt} [{rep}]"));
            }
        } else if !ctxt.is_empty() {
            fc_parts.push(format!("check first: {ctxt}"));
        }
    }
    if !effort.is_empty() { fc_parts.push(format!("effort: {effort}")); }
    if !fc_parts.is_empty() {
        prose_lines.push(format!("First calculation: {}", fc_parts.join("; ")));
    }
    Ok(())
}

fn collect_direction_risks(
    item: &Value,
    refs: &mut Vec<String>,
    prose_lines: &mut Vec<String>,
) -> Result<(), String> {
    let Some(risks) = item.get("risks").and_then(Value::as_array) else { return Ok(()) };
    let mut parts = Vec::new();
    for r in risks {
        let txt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
        validate_model_prose(txt, "direction.risks")?;
        for nid in extract_string_array(r.get("node_ids")) {
            validate_model_prose(&nid, "direction.risks.node_ids")?;
            push_unique(refs, nid);
        }
        if !txt.is_empty() {
            parts.push(txt.to_string());
        }
    }
    if !parts.is_empty() {
        prose_lines.push(format!("Risks: {}", parts.join("; ")));
    }
    Ok(())
}

/// Build the non-removable `"Where I looked"` coverage paragraph from F12 [`SearchCoverage`].
pub(crate) fn build_coverage_paragraph(coverage: &SearchCoverage) -> Result<String, String> {
    let mut parts = vec![format_coverage_scope(coverage)];
    if coverage.papers.unavailable > 0 || coverage.papers.failed > 0 {
        parts.push(format!(
            "Full text not obtained: {} unavailable, {} failed.",
            coverage.papers.unavailable, coverage.papers.failed
        ));
    }
    if coverage.gate.passed {
        parts.push(format!(
            "Coverage gate: {} ({} external services answered).",
            explore_writeup_wording::COVERAGE_COMPLETE,
            coverage.gate.answered_external_services
        ));
    } else {
        let unsat = if coverage.gate.unsatisfied.is_empty() {
            "search scope requirements were not satisfied".to_string()
        } else {
            coverage
                .gate
                .unsatisfied
                .iter()
                .map(format_gate_failure)
                .collect::<Vec<_>>()
                .join("; ")
        };
        parts.push(format!(
            "Coverage gate: {} — unsatisfied conditions: {unsat}.",
            explore_writeup_wording::COVERAGE_INCOMPLETE
        ));
    }
    let paragraph = parts.join(" ");
    if let Some(word) = contains_forbidden_prior_work_word(&paragraph) {
        return Err(format!(
            "internal wording violation: coverage paragraph contained forbidden word `{word}`"
        ));
    }
    Ok(paragraph)
}

/// Build the non-removable opening status line from F9 `explore_stage_run_state` + F12 coverage.
pub(crate) fn build_status_line(
    events: &[Event],
    latest_stmt_seq: u64,
    current: &IdeaVerdictPayload,
) -> String {
    let mut latest_by_stage: BTreeMap<String, (ExploreStageState, Option<String>)> = BTreeMap::new();
    for ev in events {
        if ev.kind != EVENT_IDEA_STAGE || ev.seq < latest_stmt_seq {
            continue;
        }
        let Some(stage) = ev.payload.get("stage").and_then(Value::as_str) else { continue };
        let Some(raw_state) = ev
            .payload
            .get("state")
            .and_then(Value::as_str)
            .and_then(ExploreStageState::from_stored)
        else {
            continue;
        };
        // Rule S3: a `requested` stage left open on disk maps to `Interrupted`.
        let effective = if raw_state == ExploreStageState::Requested {
            ExploreStageState::Interrupted
        } else {
            raw_state
        };
        // Audit H4: drop any reason containing substring "verified" (e.g. "unverified") so
        // `Writeup::validate()`'s status_line substring guard is never self-triggered.
        let reason = ev
            .payload
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| {
                !s.is_empty()
                    && contains_forbidden_word(s, false).is_none()
                    && !s.to_ascii_lowercase().contains("verified")
            })
            .map(str::to_string);
        latest_by_stage.insert(stage.to_ascii_lowercase(), (effective, reason));
    }

    let mut chosen: Option<(&str, RunState, Option<String>)> = None;
    for stage in STAGE_ORDER.iter().rev() {
        if let Some((st, reason)) = latest_by_stage.get(*stage) {
            let rs = explore_stage_run_state(*st);
            if !rs.is_terminal() {
                chosen = Some((*stage, rs, reason.clone()));
                break;
            }
        }
    }

    let stage_part = match chosen {
        Some((stage, rs, Some(reason))) => format!("{}: {reason} (stage: {stage})", rs.label()),
        Some((stage, rs, None)) => format!("{} (stage: {stage})", rs.label()),
        None => explore_stage_run_state(ExploreStageState::Finished).label().to_string(),
    };

    let cov = &current.coverage;
    let n_parts = current.verdicts.len();
    let part_word = if n_parts == 1 { "part" } else { "parts" };
    let scope_part = if cov.gate.passed {
        format!(
            "{} ({} external services answered)",
            explore_writeup_wording::COVERAGE_COMPLETE,
            cov.gate.answered_external_services
        )
    } else {
        let u = cov.gate.unsatisfied.len().max(1);
        let cw = if u == 1 { "condition" } else { "conditions" };
        format!(
            "{} ({u} unsatisfied gate {cw})",
            explore_writeup_wording::COVERAGE_INCOMPLETE
        )
    };

    format!(
        "{stage_part} — {} preset; {scope_part}; {n_parts} {part_word} checked",
        cov.preset.as_str()
    )
}

/// Case-insensitive ASCII word-boundary match (rejects `"verified"`, permits `"unverified"`).
pub(crate) fn contains_ascii_word_ci(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    if n.is_empty() {
        return false;
    }
    let hb = h.as_bytes();
    let nlen = n.len();
    let mut off = 0;
    while let Some(pos) = h[off..].find(&n) {
        let start = off + pos;
        let end = start + nlen;
        let left_ok = start == 0 || !hb[start - 1].is_ascii_alphanumeric();
        let right_ok = end == hb.len() || !hb[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        off = start + 1;
    }
    false
}

/// Enforce Rule V8 banned novelty/proof phrases + Rule S8 word-boundary `"verified"` on model prose
/// (`overlap.difference` and `idea.directions` fields).
pub(crate) fn validate_model_prose(text: &str, field: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }
    if let Err(hits) = check_model_prose_phrases(text) {
        return Err(format!(
            "Explore write-up refused (Rule V8): `{field}` contains banned phrase(s): {}",
            hits.join(", ")
        ));
    }
    validate_no_verified_word(text, field)
}

/// Enforce Rule S8 word-boundary `"verified"` check without applying Rule V8 novelty-phrase rules.
pub(crate) fn validate_no_verified_word(text: &str, field: &str) -> Result<(), String> {
    if !text.trim().is_empty() && contains_ascii_word_ci(text, "verified") {
        return Err(format!(
            "Explore write-up refused (Rule S8): `{field}` must not contain the word \"verified\""
        ));
    }
    Ok(())
}

pub(crate) fn map_display_grade(grade: DisplayGrade) -> HowChecked {
    match grade {
        DisplayGrade::ByCode => HowChecked::CodeSkeleton,
        DisplayGrade::MatchedQuoteFullText => HowChecked::MatchedQuoteFullText,
        DisplayGrade::MatchedQuoteScannedPage => HowChecked::MatchedQuoteOcr,
        DisplayGrade::MatchedQuoteAbstract => HowChecked::MatchedQuoteAbstract,
        DisplayGrade::ModelStatement => HowChecked::ModelStatement,
        DisplayGrade::UserStatement => HowChecked::UserStatement,
    }
}

pub(crate) fn format_ocr_tag_and_pages(any_ocr: bool, pages: &[usize]) -> Option<String> {
    if !any_ocr {
        return None;
    }
    if pages.is_empty() {
        Some(writeup_wording::OCR_TAG.to_string())
    } else {
        let joined = pages.iter().map(|p| format!("p. {p}")).collect::<Vec<_>>().join(", ");
        Some(format!("{} ({joined})", writeup_wording::OCR_TAG))
    }
}

pub(crate) fn derive_title(idea_text: &str, restatement: &str, session: &str) -> String {
    let src = if !restatement.trim().is_empty() {
        restatement.trim()
    } else {
        idea_text.trim()
    };
    let clean = clip_title(src, 72);
    if !clean.is_empty() && !contains_ascii_word_ci(&clean, "verified") {
        return format!("{}: {clean}", explore_writeup_wording::TITLE_PREFIX);
    }
    if !session.trim().is_empty() && !contains_ascii_word_ci(session, "verified") {
        format!("{} ({session})", explore_writeup_wording::TITLE_PREFIX)
    } else {
        explore_writeup_wording::TITLE_PREFIX.to_string()
    }
}

pub(crate) fn clip_title(raw: &str, max_chars: usize) -> String {
    let first_line = raw.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let collapsed: String = first_line.split_whitespace().collect::<Vec<_>>().join(" ");
    clip_to_chars(&collapsed, max_chars).trim().to_string()
}

pub(crate) fn extract_string_array(val: Option<&Value>) -> Vec<String> {
    val.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn push_unique(vec: &mut Vec<String>, item: String) {
    if !vec.contains(&item) {
        vec.push(item);
    }
}
