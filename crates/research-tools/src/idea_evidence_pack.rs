//! Passage-pack construction and follow-up line loading for Explore passage readers (package X10).
//!
//! Governance: `explore.md` 4.1 (`PaperRead`), 5.1 (`record_paper_evidence`, passage reader),
//! 6 (Rules C1, C2, C8, C9). Calls `investigation_outline::build_outline` and
//! `paper_passages::run_search` unchanged.

use crate::idea_fetch::derive_text_basis_from_receipt;
use crate::investigation_outline::{build_outline, PaperOutline};
use crate::lines_sent::SuppliedRange;
use crate::paper_passages;
use crate::pdf_pages::{PageBasis, PageMap, PageQualityMetrics, PageRecord};
use crate::prior_work::TextBasis;
use crate::sandbox::{resolve_sources_session_id, SandboxRoot, WorkspaceSandbox};
use crate::ResearchEnv;
use ledger::{sha256_hex, Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;

/// Maximum number of lines sent in an initial passage pack (`explore.md` 5.1).
pub const MAX_INITIAL_PACK_LINES: usize = 500;
/// Maximum number of follow-up line ranges requested in a single `want_lines` field (`explore.md` 5.1).
pub const MAX_WANT_LINE_RANGES: usize = 3;
/// Maximum total lines across all follow-up `want_lines` ranges (`explore.md` 5.1).
pub const MAX_WANT_LINES_TOTAL: usize = 400;

const MAX_SOURCE_TEXT_BYTES: u64 = 16 * 1024 * 1024;
const FRONT_MATTER_LINES: usize = 60;
const PASSAGE_CONTEXT_BEFORE: usize = 15;
const PASSAGE_CONTEXT_AFTER: usize = 25;
const MAX_TERMS_PER_QUERY: usize = 12;

/// Inclusive 1-based line window requested via `want_lines`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineWindowInput {
    pub start_line: usize,
    pub end_line: usize,
}

/// Verified, hash-pinned reading file loaded from a session's `source.ingested` receipt.
#[derive(Debug, Clone)]
pub struct LoadedReadingSource {
    pub source_id: String,
    pub source_seq: u64,
    pub path: String,
    pub file_sha256: String,
    pub text: String,
    pub lines: Vec<String>,
    pub total_lines: usize,
    pub is_pdf: bool,
    pub text_basis: TextBasis,
    pub page_map: PageMap,
    pub page_lines: Vec<(usize, usize, usize)>,
}

impl LoadedReadingSource {
    /// Return 1-based physical PDF page number containing `line`, if this source is a PDF.
    pub fn page_of_line(&self, line: usize) -> Option<usize> {
        if !self.is_pdf {
            return None;
        }
        self.page_map
            .page_of(line)
            .or_else(|| crate::pdf_pages::page_of_line(&self.page_lines, line))
    }

    /// True when this is a PDF whose pages all lack an extractable text layer (`PageBasis::None`).
    pub fn all_pages_none(&self) -> bool {
        self.is_pdf
            && !self.page_map.pages.is_empty()
            && self.page_map.pages.iter().all(|p| p.basis == PageBasis::None)
    }

    /// Return the [`PageBasis`] for every PDF page overlapping `start_line..=end_line`.
    pub fn page_bases_in_span(&self, start_line: usize, end_line: usize) -> Vec<(usize, PageBasis)> {
        if !self.is_pdf || start_line == 0 || end_line < start_line {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for rec in &self.page_map.pages {
            if let (Some(first), Some(last)) = (rec.first_line, rec.last_line) {
                if first <= end_line && last >= start_line && seen.insert(rec.page) {
                    out.push((rec.page, rec.basis));
                }
            }
        }
        if out.is_empty() && !self.page_lines.is_empty() {
            let fallback_basis = match self.text_basis {
                TextBasis::ScanTextLayer => PageBasis::ScanTextLayer,
                TextBasis::Ocr => PageBasis::Ocr,
                TextBasis::PdfText => PageBasis::TextLayer,
                TextBasis::Tex => PageBasis::None,
            };
            for &(pg, first, last) in &self.page_lines {
                if first <= end_line && last >= start_line && seen.insert(pg) {
                    out.push((pg, fallback_basis));
                }
            }
        }
        out
    }

    /// Slice `start_line..=end_line` (1-based inclusive) as numbered lines (`L<n>: ...`).
    pub fn render_ranges(&self, ranges: &[(usize, usize)]) -> String {
        let mut blocks = Vec::with_capacity(ranges.len());
        for &(s, e) in ranges {
            if s == 0 || s > self.total_lines || e < s {
                continue;
            }
            let ce = e.min(self.total_lines);
            let page_note = if self.is_pdf {
                match (self.page_of_line(s), self.page_of_line(ce)) {
                    (Some(p1), Some(p2)) if p1 == p2 => format!(" (p. {p1})"),
                    (Some(p1), Some(p2)) => format!(" (pp. {p1}-{p2})"),
                    _ => String::new(),
                }
            } else {
                String::new()
            };
            let mut body = format!("--- lines {s}-{ce}{page_note} ---\n");
            for ln in s..=ce {
                if let Some(line) = self.lines.get(ln - 1) {
                    body.push_str(&format!("L{ln}: {line}\n"));
                }
            }
            blocks.push(body);
        }
        blocks.join("\n")
    }
}

/// Initial reading pack handed to a passage reader in its first model message.
#[derive(Debug, Clone)]
pub struct PassagePack {
    pub source: LoadedReadingSource,
    pub outline: PaperOutline,
    pub outline_text: String,
    pub rendered_passages: String,
    pub supplied_ranges: Vec<SuppliedRange>,
    pub lines_count: usize,
    pub is_full_file: bool,
}

/// Follow-up passage pack produced from validated `want_lines` ranges.
#[derive(Debug, Clone)]
pub struct FollowupPack {
    pub rendered_passages: String,
    pub supplied_ranges: Vec<SuppliedRange>,
    pub lines_count: usize,
}

/// Load and SHA-256-verify the reading file for `source_id` (and optional `path`) from the
/// session's `source.ingested` ledger receipt.
pub fn load_reading_source(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    session_events: &[Event],
    source_id: &str,
    explicit_path: Option<&str>,
) -> Result<LoadedReadingSource, String> {
    let resolved_session = resolve_sources_session_id(session_id).map_err(|e| e.to_string())?;
    let receipt = session_events
        .iter()
        .rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id))
        .ok_or_else(|| format!("source `{source_id}` has no `source.ingested` receipt in session"))?;

    let path = explicit_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| receipt.payload["assembled_path"].as_str())
        .or_else(|| receipt.payload["primary_path"].as_str())
        .or_else(|| receipt.payload["files"].as_array()?.first()?.as_str())
        .ok_or_else(|| format!("source `{source_id}` receipt has no reading path"))?
        .to_string();

    let rel_path = path.split_once('/').map(|(_, r)| r).unwrap_or(path.as_str());
    let expected_sha = receipt.payload["files_sha256"]
        .as_object()
        .and_then(|obj| obj.get(&path).or_else(|| obj.get(rel_path)).and_then(Value::as_str))
        .ok_or_else(|| format!("source `{source_id}` receipt has no pinned sha256 for `{path}`"))?
        .to_string();

    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, resolved_session, &path, false)
        .map_err(|e| format!("cannot resolve source file `{path}`: {e}"))?;
    let mut raw = Vec::new();
    File::open(&resolved.canonical_target)
        .map_err(|e| format!("cannot open `{path}`: {e}"))?
        .take(MAX_SOURCE_TEXT_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("cannot read `{path}`: {e}"))?;
    if raw.len() as u64 > MAX_SOURCE_TEXT_BYTES {
        return Err(format!("source file `{path}` exceeds text size cap"));
    }
    let actual_sha = sha256_hex(&raw);
    if actual_sha != expected_sha {
        return Err(format!("source file `{path}` hash mismatch against `source.ingested` receipt"));
    }
    let text = String::from_utf8(raw).map_err(|_| format!("source file `{path}` is not valid UTF-8"))?;
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let total_lines = lines.len();

    let format = receipt.payload["format"].as_str().unwrap_or("tex");
    let is_pdf = format.eq_ignore_ascii_case("pdf");
    let text_basis = derive_text_basis_from_receipt(Some(receipt), format);
    let (page_map, page_lines) = parse_receipt_page_map(&receipt.payload, text_basis);

    Ok(LoadedReadingSource {
        source_id: source_id.to_string(),
        source_seq: receipt.seq,
        path,
        file_sha256: actual_sha,
        text,
        lines,
        total_lines,
        is_pdf,
        text_basis,
        page_map,
        page_lines,
    })
}

fn empty_page_metrics() -> PageQualityMetrics {
    PageQualityMetrics {
        char_count: 0, alpha_count: 0, word_count: 0,
        image_coverage_pct: 0, has_invisible_text: false, width_pt: 0, height_pt: 0,
    }
}

fn parse_receipt_page_map(payload: &Value, text_basis: TextBasis) -> (PageMap, Vec<(usize, usize, usize)>) {
    let page_count = payload["page_count"].as_u64().unwrap_or(0) as usize;
    let mut records: Vec<PageRecord> = Vec::new();

    if let Some(arr) = payload["pdf_pages"].as_array() {
        for item in arr {
            if let Ok(rec) = serde_json::from_value::<PageRecord>(item.clone()) {
                records.push(rec);
                continue;
            }
            let Some(page) = item["page"].as_u64().map(|n| n as usize) else { continue };
            let basis = match item["basis"].as_str().unwrap_or("").trim().to_ascii_lowercase().as_str() {
                "text_layer" | "pdf_text" => PageBasis::TextLayer,
                "scan_text_layer" => PageBasis::ScanTextLayer,
                "ocr" => PageBasis::Ocr,
                _ => PageBasis::None,
            };
            records.push(PageRecord {
                page,
                basis,
                first_line: item["first_line"].as_u64().map(|n| n as usize),
                last_line: item["last_line"].as_u64().map(|n| n as usize),
                metrics: empty_page_metrics(),
                reason: item["reason"].as_str().map(str::to_string),
            });
        }
    }

    let mut page_lines: Vec<(usize, usize, usize)> = payload["pdf_page_lines"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|row| {
                    let a = row.as_array()?;
                    Some((a.first()?.as_u64()? as usize, a.get(1)?.as_u64()? as usize, a.get(2)?.as_u64()? as usize))
                })
                .collect()
        })
        .unwrap_or_default();

    if page_lines.is_empty() && !records.is_empty() {
        page_lines = records.iter().filter_map(|r| Some((r.page, r.first_line?, r.last_line?))).collect();
    } else if records.is_empty() && !page_lines.is_empty() {
        let fb = match text_basis {
            TextBasis::ScanTextLayer => PageBasis::ScanTextLayer,
            TextBasis::Ocr => PageBasis::Ocr,
            TextBasis::PdfText => PageBasis::TextLayer,
            TextBasis::Tex => PageBasis::None,
        };
        records = page_lines
            .iter()
            .map(|&(page, first, last)| PageRecord {
                page, basis: fb, first_line: Some(first), last_line: Some(last),
                metrics: empty_page_metrics(), reason: None,
            })
            .collect();
    }

    let eff_count = page_count.max(records.len()).max(page_lines.len());
    (PageMap { page_count: eff_count, pages: records }, page_lines)
}

/// Build the initial passage pack for a single paper using `investigation_outline::build_outline`
/// and `paper_passages::run_search` unchanged (`explore.md` 5.1).
pub fn build_passage_pack(
    env: &ResearchEnv,
    ledger: &Ledger,
    session_id: &str,
    source_id: &str,
    terms: &[String],
    max_lines: usize,
) -> Result<PassagePack, String> {
    let cap = max_lines.clamp(1, MAX_INITIAL_PACK_LINES);
    let session_events = ledger
        .read_session(session_id)
        .map_err(|e| format!("cannot read session `{session_id}`: {e}"))?;
    let source = load_reading_source(&env.sandbox(), session_id, &session_events, source_id, None)?;
    if source.total_lines == 0 || source.all_pages_none() {
        return Err(format!("source `{source_id}` has no extractable text layer"));
    }

    let outline = build_outline(&source.text, &source.page_lines);
    let outline_text = format_outline(&outline, source.is_pdf);
    let (ranges, is_full_file) = if source.total_lines <= cap {
        (vec![(1usize, source.total_lines)], true)
    } else {
        let selected = select_passages(env, ledger, session_id, &source, &outline, terms, cap);
        let full = selected.len() == 1 && selected[0] == (1, source.total_lines);
        (selected, full)
    };

    let lines_count = count_lines(&ranges);
    let rendered_passages = source.render_ranges(&ranges);
    let supplied_ranges = ranges
        .into_iter()
        .map(|(start_line, end_line)| SuppliedRange {
            source_id: source.source_id.clone(),
            path: source.path.clone(),
            file_sha256: source.file_sha256.clone(),
            start_line,
            end_line,
        })
        .collect();

    Ok(PassagePack {
        source, outline, outline_text, rendered_passages, supplied_ranges, lines_count, is_full_file,
    })
}

/// Validate and load follow-up `want_lines` windows (`<= 3` ranges, `<= 400` total lines).
pub fn build_followup_pack(
    source: &LoadedReadingSource,
    want_lines: &[LineWindowInput],
) -> Result<FollowupPack, String> {
    validate_want_lines(want_lines, source.total_lines)?;
    let mut raw: Vec<(usize, usize)> = want_lines.iter().map(|w| (w.start_line, w.end_line)).collect();
    let merged = merge_intervals(&mut raw);
    let lines_count = count_lines(&merged);
    let rendered_passages = source.render_ranges(&merged);
    let supplied_ranges = merged
        .into_iter()
        .map(|(start_line, end_line)| SuppliedRange {
            source_id: source.source_id.clone(),
            path: source.path.clone(),
            file_sha256: source.file_sha256.clone(),
            start_line,
            end_line,
        })
        .collect();
    Ok(FollowupPack { rendered_passages, supplied_ranges, lines_count })
}

/// Validate `want_lines` count, 1-based bounds against `total_lines`, and total line cap (400).
pub fn validate_want_lines(want_lines: &[LineWindowInput], total_lines: usize) -> Result<(), String> {
    if want_lines.len() > MAX_WANT_LINE_RANGES {
        return Err(format!("want_lines may contain at most {MAX_WANT_LINE_RANGES} ranges (got {})", want_lines.len()));
    }
    let mut total = 0usize;
    for (idx, w) in want_lines.iter().enumerate() {
        if w.start_line == 0 || w.end_line < w.start_line || w.end_line > total_lines {
            return Err(format!("want_lines[{idx}] ({}-{}) is outside valid lines 1..={total_lines}", w.start_line, w.end_line));
        }
        total = total.saturating_add(w.end_line - w.start_line + 1);
    }
    if total > MAX_WANT_LINES_TOTAL {
        return Err(format!("want_lines requests {total} lines, exceeding the {MAX_WANT_LINES_TOTAL}-line cap"));
    }
    Ok(())
}

fn select_passages(
    env: &ResearchEnv,
    ledger: &Ledger,
    session_id: &str,
    source: &LoadedReadingSource,
    outline: &PaperOutline,
    terms: &[String],
    cap: usize,
) -> Vec<(usize, usize)> {
    let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
    for q in build_passage_queries(terms) {
        let args = json!({ "query": q, "source_id": source.source_id, "limit": 10 });
        if let Ok(val) = paper_passages::run_search(env, ledger, session_id, &args) {
            if let Some(arr) = val["passages"].as_array() {
                for p in arr {
                    if p["path"].as_str() != Some(source.path.as_str()) { continue; }
                    let (Some(s), Some(e)) = (p["start_line"].as_u64(), p["end_line"].as_u64()) else { continue };
                    let score = p["score"].as_u64().unwrap_or(1) as usize;
                    let ws = (s as usize).saturating_sub(PASSAGE_CONTEXT_BEFORE).max(1);
                    let we = (e as usize + PASSAGE_CONTEXT_AFTER).min(source.total_lines);
                    candidates.push((score, ws, we));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut chosen: Vec<(usize, usize)> = Vec::new();
    let front_end = FRONT_MATTER_LINES.min(source.total_lines).min(cap);
    if front_end >= 1 { chosen.push((1, front_end)); }
    for (_, s, e) in candidates {
        if !try_add_window(&mut chosen, (s, e), cap) { break; }
    }
    let mut used = count_lines(&chosen);
    if used < cap {
        for sec in outline.sections.iter().rev() {
            if sec.kind == "back_matter" && sec.key == "References" { continue; }
            let rem = cap.saturating_sub(used);
            if rem == 0 { break; }
            let we = sec.end_line.min(sec.start_line + rem - 1).min(source.total_lines);
            if sec.start_line >= 1 && sec.start_line <= we {
                let _ = try_add_window(&mut chosen, (sec.start_line, we), cap);
                used = count_lines(&chosen);
            }
        }
    }
    merge_intervals(&mut chosen)
}

fn build_passage_queries(terms: &[String]) -> Vec<String> {
    let mut clean = Vec::new();
    let mut seen = BTreeSet::new();
    for t in terms {
        let tr = t.trim();
        if tr.len() >= 2 && seen.insert(tr.to_ascii_lowercase()) {
            clean.push(tr.to_string());
        }
    }
    clean
        .chunks(MAX_TERMS_PER_QUERY)
        .filter_map(|chunk| {
            let mut q = chunk.join(" ");
            if q.chars().count() > 480 { q = q.chars().take(480).collect(); }
            (!q.trim().is_empty()).then_some(q)
        })
        .collect()
}

fn try_add_window(chosen: &mut Vec<(usize, usize)>, window: (usize, usize), cap: usize) -> bool {
    let cur = count_lines(chosen);
    if cur >= cap { return false; }
    let mut trial = chosen.clone();
    trial.push(window);
    let merged = merge_intervals(&mut trial);
    if count_lines(&merged) <= cap {
        *chosen = merged;
        return true;
    }
    let left = cap.saturating_sub(cur);
    if left == 0 { return false; }
    let mut trial2 = chosen.clone();
    trial2.push((window.0, (window.0 + left - 1).min(window.1)));
    let merged2 = merge_intervals(&mut trial2);
    if count_lines(&merged2) <= cap { *chosen = merged2; }
    count_lines(chosen) < cap
}

fn count_lines(intervals: &[(usize, usize)]) -> usize {
    intervals.iter().map(|&(s, e)| if e >= s && s >= 1 { e - s + 1 } else { 0 }).sum()
}

fn merge_intervals(intervals: &mut Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    intervals.retain(|&(s, e)| s >= 1 && e >= s);
    intervals.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(intervals.len());
    for (s, e) in intervals.drain(..) {
        if let Some(last) = merged.last_mut() {
            if last.1.saturating_add(1) >= s {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    merged
}

fn format_outline(outline: &PaperOutline, is_pdf: bool) -> String {
    let mut lines = vec![format!("Total lines: {} (outline style: {})", outline.total_lines, outline.style)];
    if !outline.sections.is_empty() {
        lines.push("Sections:".into());
        for s in outline.sections.iter().take(40) {
            let pg = if is_pdf {
                match (s.page_start, s.page_end) {
                    (Some(p1), Some(p2)) if p1 == p2 => format!(" [p. {p1}]"),
                    (Some(p1), Some(p2)) => format!(" [pp. {p1}-{p2}]"),
                    _ => String::new(),
                }
            } else { String::new() };
            lines.push(format!("  - {} ({}): lines {}-{}{pg}", s.title, s.kind, s.start_line, s.end_line));
        }
    }
    if !outline.equations.is_empty() {
        let eqs: Vec<String> = outline.equations.iter().take(20).map(|e| format!("{}@L{}", e.label, e.line)).collect();
        lines.push(format!("Equations: {}", eqs.join(", ")));
    }
    lines.join("\n")
}
