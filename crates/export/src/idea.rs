//! Deterministic Explore session JSON & BibTeX export with F12 coverage and Rule V7
//! verdict recomputation (package X17).
//!
//! Governance: `explore.md` 4.1–4.3, 6 (Rules C3, C8, V7, S5, S8), 9 (X17); `00-overview.md`
//! 2.7, 2.9, 2.11, 3.2 row 20, 7.3.
//!
//! Security & integrity invariants:
//! - Reads sessions exclusively through `Ledger::read_verified_session` (hash-chain checked).
//! - Recomputes F12 `SearchCoverage` and per-part `PriorWorkVerdict` (`idea_verdict::compute`)
//!   on every export and fails closed if any stored `idea.verdict` event in the active revision
//!   diverges, carries unknown keys, or is missing after an `idea.map` (Rule V7).
//! - Re-matches every `evidence.quote` record against the session's own pinned `source.ingested`
//!   files via F4 `verify_quote_ref` (requiring pinned `path`, `file_sha256`, `quote_sha256`),
//!   enforces Rule C3 on `evidence.abstract`, enforces Rule C8 across overlapping PDF page spans,
//!   and clips every exported quote/excerpt to `<= 300` Unicode scalar values.
//! - Prefers the user's `source_item.citekey` from F10/F13 `Library` when `library.db` exists
//!   (resolving across all identifier aliases and merging metadata before deduplication), falling
//!   back cleanly without recreating `library.db` when it is deleted (Rule S5).
//! - Enforces Rule S8 fail-closed over all app-authored wording fields: zero `"verified"`,
//!   `"new"`, `"novel"`, `"novelty"`, `"proved"`, or `"unexplored"`.

#[path = "idea_extract.rs"]
mod extract;

use crate::writeup::BibEntry;
use crate::writeup_bib::{render_bibtex, resolve_bib_entry};
use ledger::{Event, Ledger};
use research_tools::idea_verdict::{self, IdeaPresetLimits, IdeaVerdictPayload};
use research_tools::library::{Library, LIBRARY_DB_NAME, LIBRARY_DIR_NAME};
use research_tools::paper_key::{Mode, PaperKey, PaperRef};
use research_tools::prior_work::{PriorWorkVerdict, IDEA_VERDICT_EVENT};
use research_tools::prior_work_wording::{
    contains_forbidden_prior_work_word, format_coverage_scope,
};
use research_tools::search_coverage::{ExplorePreset, SearchCoverage};
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Export envelope schema version for Explore JSON exports.
pub const IDEA_EXPORT_SCHEMA_VERSION: u32 = 1;

// Event-kind constants aligned with `explore.md` 4.2 & `00-overview.md` 8.2 decision 10.
pub(crate) const EVENT_IDEA_STARTED: &str = "idea.started";
pub(crate) const EVENT_IDEA_STATED: &str = "idea.stated";
pub(crate) const EVENT_IDEA_BRIEF: &str = "idea.brief";
pub(crate) const EVENT_IDEA_ANSWER: &str = "idea.answer";
pub(crate) const EVENT_IDEA_CANDIDATES: &str = "idea.candidates";
pub(crate) const EVENT_IDEA_PAPER: &str = "idea.paper";
pub(crate) const EVENT_IDEA_MAP: &str = "idea.map";
pub(crate) const EVENT_IDEA_DIRECTIONS: &str = "idea.directions";
pub(crate) const EVENT_IDEA_BUDGET_RAISED: &str = "idea.budget_raised";
pub(crate) const EVENT_SOURCE_INGESTED: &str = "source.ingested";

const ALLOWED_VERDICT_KEYS: &[&str] = &["map_seq", "origin_paper", "verdicts", "coverage"];

/// Supported formats for `GET /api/sessions/{id}/idea/export?format=json|bib`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdeaExportFormat {
    Json,
    Bib,
}

impl IdeaExportFormat {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "bib" | "bibtex" => Ok(Self::Bib),
            other => Err(format!(
                "unknown Explore export format `{other}` (expected `json` or `bib`)"
            )),
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Bib => "bib",
        }
    }

    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json; charset=utf-8",
            Self::Bib => "application/x-bibtex; charset=utf-8",
        }
    }
}

/// Original idea statement recorded by the user (`idea.stated`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaStatementExport {
    pub seq: u64,
    pub rev: u64,
    pub text: String,
    pub text_sha256: String,
    pub fields: Vec<String>,
    pub seeds: Vec<String>,
    pub author: String,
}

/// Deterministic paper-reading status entry (`idea.paper`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperReadExport {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seq: Option<u64>,
    pub origin: String,
    pub text_basis: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub lines_sent: u64,
    pub total_lines: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
}

/// F4-matched full-text quote entry (`evidence.quote`, id `xe-<seq>`), clipped to `<= 300` chars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedQuoteExport {
    pub id: String,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<String>,
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_sha256: String,
    pub quote_sha256: String,
    pub quote: String,
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gist: Option<String>,
    pub subclaim_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    pub text_basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_tag: Option<String>,
    pub display_grade: String,
}

/// Verbatim abstract quote entry (`evidence.abstract`, id `xa-<seq>`), clipped to `<= 300` chars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractQuoteExport {
    pub id: String,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<String>,
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
}

/// Complete deterministic Explore JSON export bundle (package X17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaExportJson {
    pub schema_version: u32,
    pub session_id: String,
    pub head_hash: String,
    pub pipeline_version: u64,
    pub preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_paper: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<IdeaStatementExport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharpened_idea: Option<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub question_answers: BTreeMap<String, String>,
    pub candidates: Vec<Value>,
    pub papers_read: Vec<PaperReadExport>,
    pub matched_quotes: Vec<MatchedQuoteExport>,
    pub abstract_quotes: Vec<AbstractQuoteExport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept_map: Option<Value>,
    pub verdicts: Vec<PriorWorkVerdict>,
    pub directions: Vec<Value>,
    pub coverage: SearchCoverage,
    pub coverage_scope_summary: String,
}

/// Build the structured deterministic Explore export model from a hash-chain-checked session.
pub fn build_idea_export(data_dir: &Path, session: &str) -> Result<IdeaExportJson, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger =
        Ledger::open_existing(data_dir).map_err(|e| format!("failed to open ledger: {e}"))?;
    let (report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("ledger integrity check failed for `{session}`: {e}"))?;
    if session_mode_from_events(&events) != Mode::Explore {
        return Err(format!(
            "Explore export requires an Explore session (`{session}` is not in Explore mode)"
        ));
    }

    let stmt_seq = latest_statement_seq(&events);
    let recomputed = recompute_and_check_rule_v7(&events, stmt_seq)?;
    let scope_summary = format_coverage_scope(&recomputed.coverage);

    let pipeline_version = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_STARTED)
        .and_then(|e| e.payload.get("pipeline_version").and_then(Value::as_u64))
        .unwrap_or(1);

    let statement = extract::extract_statement(&events, stmt_seq);
    let (sharpened_idea, question_answers) = extract::fold_brief_and_answers(&events, stmt_seq);
    let candidates = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_CANDIDATES && e.seq > stmt_seq)
        .and_then(|e| e.payload.get("candidates").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let papers_read = extract::collect_papers_read(&events, stmt_seq);
    let matched_quotes =
        extract::collect_and_match_quotes(data_dir, session, &events, stmt_seq, &papers_read)?;
    let abstract_quotes = extract::collect_abstract_quotes(&events, stmt_seq)?;
    check_wording_rule_s8(
        &recomputed.verdicts,
        &recomputed.coverage,
        &scope_summary,
        &matched_quotes,
    )?;

    let concept_map = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq)
        .map(|e| e.payload.clone());
    let directions = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_DIRECTIONS && e.seq > stmt_seq)
        .and_then(|e| e.payload.get("directions").and_then(Value::as_array).cloned())
        .unwrap_or_default();

    Ok(IdeaExportJson {
        schema_version: IDEA_EXPORT_SCHEMA_VERSION,
        session_id: session.to_string(),
        head_hash: report.head_hash,
        pipeline_version,
        preset: recomputed.coverage.preset.as_str().to_string(),
        origin_paper: recomputed.origin_paper,
        statement,
        sharpened_idea,
        question_answers,
        candidates,
        papers_read,
        matched_quotes,
        abstract_quotes,
        concept_map,
        verdicts: recomputed.verdicts,
        directions,
        coverage: recomputed.coverage,
        coverage_scope_summary: scope_summary,
    })
}

/// Render a deterministic Explore JSON export string (`export_idea_json`, package X17).
pub fn export_idea_json(data_dir: &Path, session: &str) -> Result<String, String> {
    let model = build_idea_export(data_dir, session)?;
    serde_json::to_string_pretty(&model)
        .map(|s| format!("{s}\n"))
        .map_err(|e| format!("failed to serialize Explore JSON export: {e}"))
}

/// Render a deterministic Explore BibTeX export string (`export_idea_bibtex`, package X17).
///
/// Prefers the user's `source_item.citekey` from F10/F13 `Library` via lookup when `library.db`
/// exists (resolving across all F2 identifier aliases and merging metadata before deduplication),
/// falling back cleanly without recreating `library.db` when it is deleted (Rule S5).
pub fn export_idea_bibtex(data_dir: &Path, session: &str) -> Result<String, String> {
    let model = build_idea_export(data_dir, session)?;
    let raw_hints = extract::collect_paper_bib_hints(&model);
    let lib = open_library_if_present(data_dir);

    // Resolve each hint to its canonical library key (checking both primary key and `hint.ids`
    // aliases) and merge metadata BEFORE deduplication so richer candidate fields are never lost.
    let mut merged: BTreeMap<PaperKey, extract::PaperBibHint> = BTreeMap::new();
    for (key, hint) in raw_hints {
        let canonical = resolve_canonical_key(lib.as_ref(), &key, &hint);
        match merged.get_mut(&canonical) {
            Some(existing) => extract::merge_bib_hint(existing, &hint),
            None => {
                merged.insert(canonical, hint);
            }
        }
    }

    let mut used_keys = BTreeSet::new();
    let mut entries: Vec<BibEntry> = Vec::with_capacity(merged.len());
    for (canonical, hint) in merged {
        let pref = PaperRef {
            key: canonical,
            version: hint.version,
            ids: hint.ids,
        };
        let entry = resolve_bib_entry(
            lib.as_ref(),
            &pref,
            &hint.title,
            &hint.authors,
            hint.year,
            &mut used_keys,
        )?;
        entries.push(entry);
    }

    let header = extract::render_bibtex_header(&model, entries.len())?;
    let body = render_bibtex(&entries);
    if body.is_empty() {
        Ok(header)
    } else {
        Ok(format!("{header}\n{body}"))
    }
}

/// Export an Explore session into `(filename, bytes)` for `format` (`json` or `bib`).
pub fn export_idea_bytes(
    data_dir: &Path,
    session: &str,
    format: IdeaExportFormat,
) -> Result<(String, Vec<u8>), String> {
    let text = match format {
        IdeaExportFormat::Json => export_idea_json(data_dir, session)?,
        IdeaExportFormat::Bib => export_idea_bibtex(data_dir, session)?,
    };
    Ok((
        format!("{session}-idea.{}", format.extension()),
        text.into_bytes(),
    ))
}

fn resolve_canonical_key(
    lib: Option<&Library>,
    key: &PaperKey,
    hint: &extract::PaperBibHint,
) -> PaperKey {
    let Some(lib) = lib else {
        return key.clone();
    };
    if let Ok(Some(found)) = lib.resolve_key(key) {
        return found;
    }
    let candidates: [Option<Result<PaperKey, String>>; 4] = [
        hint.ids.arxiv.as_deref().map(PaperKey::from_arxiv),
        hint.ids.doi.as_deref().map(PaperKey::from_doi),
        hint.ids.inspire.as_deref().map(PaperKey::from_inspire),
        hint.ids.s2.as_deref().map(PaperKey::from_s2),
    ];
    for alias in candidates.into_iter().flatten().flatten() {
        if let Ok(Some(found)) = lib.resolve_key(&alias) {
            return found;
        }
    }
    key.clone()
}

fn latest_statement_seq(events: &[Event]) -> u64 {
    events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_STATED)
        .map_or(0, |e| e.seq)
}

fn derive_preset_limits(events: &[Event]) -> Option<IdeaPresetLimits> {
    let mut saw_started = false;
    let mut preset = ExplorePreset::Standard;
    let mut max_screened = preset.limits().max_screened;
    let mut max_papers = preset.limits().max_papers;

    for ev in events {
        if ev.kind == EVENT_IDEA_STARTED {
            saw_started = true;
            if let Some(p) = ev
                .payload
                .get("preset")
                .and_then(Value::as_str)
                .and_then(ExplorePreset::parse)
            {
                preset = p;
                max_screened = p.limits().max_screened;
                max_papers = p.limits().max_papers;
            }
            if let Some(b) = ev.payload.get("budget").and_then(Value::as_object) {
                if let Some(v) = b.get("max_screened").and_then(Value::as_u64).filter(|&v| v >= 1) {
                    max_screened = v as usize;
                }
                if let Some(v) = b.get("max_papers").and_then(Value::as_u64) {
                    max_papers = v as usize;
                }
            }
        } else if ev.kind == EVENT_IDEA_BUDGET_RAISED {
            let extra = ev
                .payload
                .get("extra_papers")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            max_papers = max_papers.saturating_add(extra);
        }
    }

    saw_started.then_some(IdeaPresetLimits {
        preset,
        max_screened: Some(max_screened),
        max_papers: Some(max_papers),
    })
}

fn recompute_and_check_rule_v7(
    events: &[Event],
    stmt_seq: u64,
) -> Result<IdeaVerdictPayload, String> {
    let latest_map_seq = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq)
        .map(|e| e.seq);
    let mut latest_verdict_seq: Option<u64> = None;

    // Validate EVERY stored `idea.verdict` in the active revision, not just the final one.
    for (idx, ev) in events.iter().enumerate() {
        if ev.kind != IDEA_VERDICT_EVENT || ev.seq <= stmt_seq {
            continue;
        }
        latest_verdict_seq = Some(ev.seq);
        if let Some(obj) = ev.payload.as_object() {
            for k in obj.keys() {
                if !ALLOWED_VERDICT_KEYS.contains(&k.as_str()) {
                    return Err(format!(
                        "Rule V7 mismatch: stored `{IDEA_VERDICT_EVENT}` at seq {} contains unexpected field `{k}`",
                        ev.seq
                    ));
                }
            }
        }
        let stored: IdeaVerdictPayload = serde_json::from_value(ev.payload.clone()).map_err(|e| {
            format!(
                "Rule V7 mismatch: stored `{IDEA_VERDICT_EVENT}` at seq {} is malformed: {e}",
                ev.seq
            )
        })?;
        let prefix = &events[..=idx];
        let at_verdict = idea_verdict::compute(prefix, derive_preset_limits(prefix));
        if stored.verdicts != at_verdict.verdicts
            || stored.coverage != at_verdict.coverage
            || stored.origin_paper != at_verdict.origin_paper
            || stored.map_seq != at_verdict.map_seq
        {
            return Err(format!(
                "Rule V7 mismatch: stored `{IDEA_VERDICT_EVENT}` at seq {} disagrees with recomputed verdicts/coverage",
                ev.seq
            ));
        }
    }

    if let Some(map_seq) = latest_map_seq {
        if latest_verdict_seq.is_none_or(|vseq| vseq < map_seq) {
            return Err(format!(
                "Rule V7 mismatch: `{EVENT_IDEA_MAP}` at seq {map_seq} has no corresponding stored `{IDEA_VERDICT_EVENT}`"
            ));
        }
    }

    Ok(idea_verdict::compute(events, derive_preset_limits(events)))
}

fn check_wording_rule_s8(
    verdicts: &[PriorWorkVerdict],
    coverage: &SearchCoverage,
    scope_summary: &str,
    matched_quotes: &[MatchedQuoteExport],
) -> Result<(), String> {
    check_text_s8("coverage_scope_summary", scope_summary)?;
    for item in &coverage.not_searched {
        check_text_s8("coverage.not_searched", item)?;
    }
    for v in verdicts {
        check_text_s8(&format!("verdict[{}].chip", v.subclaim_id), &v.chip)?;
        check_text_s8(&format!("verdict[{}].text", v.subclaim_id), &v.text)?;
    }
    for q in matched_quotes {
        check_text_s8(&format!("matched_quotes[{}].display_grade", q.id), &q.display_grade)?;
        if let Some(ref tag) = q.ocr_tag {
            check_text_s8(&format!("matched_quotes[{}].ocr_tag", q.id), tag)?;
        }
    }
    Ok(())
}

pub(crate) fn check_text_s8(field: &str, text: &str) -> Result<(), String> {
    if let Some(hit) = contains_forbidden_prior_work_word(text) {
        return Err(format!(
            "Rule S8 refusal: exported wording field `{field}` contains forbidden term `{hit}`"
        ));
    }
    Ok(())
}

fn open_library_if_present(data_dir: &Path) -> Option<Library> {
    data_dir
        .join(LIBRARY_DIR_NAME)
        .join(LIBRARY_DB_NAME)
        .is_file()
        .then(|| Library::open(data_dir).ok())
        .flatten()
}

#[cfg(test)]
#[path = "idea_tests.rs"]
mod tests;
