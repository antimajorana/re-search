//! Verified PDF receipt loading and Rule 6.21 honesty helpers for `pdf-1` (Investigate V17).
//!
//! Keeps physical-page lookup strictly derived from physical file lines (`PageMap::page_of`),
//! enforces OCR display metadata and non-math rendering for scanned pages, preserves image-only
//! page availability, rejects ambiguous multi-page quote locations and `PageBasis::None` hits,
//! and loads hash-verified reading files fail-closed from `source.ingested` receipts.

use super::model::{compute_files_root_sha256, SkNode, Span};
use crate::pdf::{MAX_PDF_EXTRACTED_BYTES, MAX_PDF_PAGES};
use crate::pdf_match::{match_pdf_quote, PdfQuoteMatch};
use crate::pdf_pages::{PageBasis, PageMap, PageQualityMetrics, PageRecord};
use crate::sandbox::{SandboxRoot, WorkspaceSandbox};
use ledger::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;

/// Base quality line for PDF skeletons (`investigate.md` section 3, Stage C', rule 6.21;
/// owner scope decision 2026-11-25 in `00-overview.md` 8.1).
pub const PDF_QUALITY_REASON_BASE: &str = "estimated from PDF text; theorem boundaries may be wrong";

/// Scan marker included in `quality_reason` whenever any extracted page is scanned or image-only
/// (`investigate.md` section 3, Stage C' and V17).
pub const PDF_QUALITY_SCAN_TAG: &str = "(scan)";

/// Quote display prefix for matches on `scan_text_layer` / OCR pages (`00-overview.md` 2.12,
/// `investigate.md` rule 6.21; owner decision 8.3 #3).
pub const PDF_OCR_QUOTE_LABEL: &str = "matched in OCR text";

/// Display label for a formula transcribed from a page image by a model (`00-overview.md` 2.12,
/// `investigate.md` rule 6.21; never a matched quote).
pub const PDF_MODEL_TRANSCRIPTION_LABEL: &str = "transcription (model), unconfirmed";

/// Provenance class of text shown for a PDF quote or formula (rule 6.21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextOrigin {
    DigitalTextLayer,
    ScanOcrText,
    ModelImageTranscription,
}

impl PdfTextOrigin {
    /// Only digital text-layer and scan/OCR reading-file matches can back a matched anchor;
    /// model image transcriptions are never matched quotes (rule 6.21).
    pub fn can_be_matched_anchor(self) -> bool {
        matches!(self, Self::DigitalTextLayer | Self::ScanOcrText)
    }

    /// Formulas from OCR text or unconfirmed image transcriptions are never drawn as mathematics
    /// (rule 6.21, `00-overview.md` 2.12).
    pub fn render_formulas_as_math(self) -> bool {
        matches!(self, Self::DigitalTextLayer)
    }

    /// Canonical display prefix for this origin (`00-overview.md` 2.12, `investigate.md` 6.21).
    pub fn display_prefix(self) -> &'static str {
        match self {
            Self::DigitalTextLayer => "matched",
            Self::ScanOcrText => PDF_OCR_QUOTE_LABEL,
            Self::ModelImageTranscription => PDF_MODEL_TRANSCRIPTION_LABEL,
        }
    }
}

/// Per-page availability summary preserved alongside a PDF skeleton.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfPageAvailability {
    pub page: usize,
    pub basis: PageBasis,
    pub has_text: bool,
    /// True whenever the original physical PDF page remains available to view as a page image.
    pub image_available: bool,
    /// False for `ScanTextLayer` and `None` (rule 6.21: formulas from OCR stay text).
    pub render_math: bool,
}

/// Verified PDF quote display metadata derived without mutating the pinned reading file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfQuoteDisplay {
    pub span: Span,
    pub start_page: usize,
    pub end_page: usize,
    pub basis: PageBasis,
    pub origin: PdfTextOrigin,
    pub is_ocr: bool,
    pub render_math: bool,
    pub page_image_available: bool,
    pub display_label: String,
    pub stored_excerpt: String,
    pub raw_match: PdfQuoteMatch,
}

/// Verified PDF reading file and page map loaded from a `source.ingested` ledger receipt.
#[derive(Debug, Clone)]
pub struct IngestedPdfSource {
    pub source_id: String,
    pub source_seq: u64,
    pub primary_path: String,
    pub text: String,
    pub page_map: PageMap,
    pub extractor_id: Option<String>,
    pub files_sha256: BTreeMap<String, String>,
    pub files_root_sha256: String,
    pub extra_quality_notes: Vec<String>,
}

/// Splits reading-file text into 1-based physical lines matching `VerifiedFile` and `PageMap`.
pub(crate) fn split_physical_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// Loads a hash-verified PDF reading file and its physical [`PageMap`] from a `source.ingested`
/// ledger receipt under `sandbox`, failing closed on hash mismatch, invalid status, missing
/// `files` membership, or missing/invalid page maps.
pub fn load_ingested_pdf_source(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
) -> Result<IngestedPdfSource, String> {
    if receipt.kind != "source.ingested" {
        return Err(format!("expected `source.ingested` event, got `{}`", receipt.kind));
    }
    let data = &receipt.payload;
    let status = data["status"].as_str().unwrap_or("");
    let is_pdf = data["format"].as_str() == Some("pdf")
        || status == "pdf_only"
        || data.get("pdf_extractor").is_some_and(|v| v.is_string())
        || data.get("pdf_pages").is_some_and(|v| v.is_array());
    if !is_pdf {
        return Err("source.ingested receipt is not a PDF source".into());
    }
    if !matches!(status, "ok" | "pdf_only" | "empty_text_layer") {
        return Err(format!("unsupported PDF receipt status `{status}`"));
    }

    let source_id = data["source_id"]
        .as_str()
        .ok_or_else(|| "source.ingested receipt missing `source_id`".to_string())?
        .to_string();
    let primary_path = data["primary_path"]
        .as_str()
        .ok_or_else(|| "source.ingested receipt missing `primary_path`".to_string())?
        .to_string();
    let files_arr = data["files"]
        .as_array()
        .ok_or_else(|| "source.ingested receipt missing `files` array".to_string())?;
    if !files_arr.iter().any(|v| v.as_str() == Some(primary_path.as_str())) {
        return Err(format!("`primary_path` `{primary_path}` is not listed in receipt `files`"));
    }

    let hashes_obj = data["files_sha256"]
        .as_object()
        .ok_or_else(|| "source.ingested receipt missing `files_sha256` map".to_string())?;
    let mut files_sha256 = BTreeMap::new();
    for (k, v) in hashes_obj {
        let digest = v
            .as_str()
            .ok_or_else(|| format!("non-string digest for `{k}` in files_sha256"))?;
        if !is_valid_sha256_hex(digest) {
            return Err(format!("invalid sha256 digest for `{k}` in files_sha256"));
        }
        files_sha256.insert(k.clone(), digest.to_string());
    }
    let expected_sha = files_sha256
        .get(&primary_path)
        .ok_or_else(|| format!("files_sha256 missing digest for `{primary_path}`"))?;

    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, session_id, &primary_path, false)
        .map_err(|e| format!("cannot resolve `{primary_path}`: {e}"))?;
    if resolved.rel_path != primary_path {
        return Err(format!(
            "resolved path `{}` does not match receipt `primary_path` `{primary_path}`",
            resolved.rel_path
        ));
    }
    if !resolved.metadata.is_file() {
        return Err(format!("`{primary_path}` is not a regular file"));
    }
    let mut raw_bytes = Vec::new();
    let f = File::open(&resolved.canonical_target)
        .map_err(|e| format!("cannot open `{primary_path}`: {e}"))?;
    f.take((MAX_PDF_EXTRACTED_BYTES as u64) + 1)
        .read_to_end(&mut raw_bytes)
        .map_err(|e| format!("cannot read `{primary_path}`: {e}"))?;
    if raw_bytes.len() > MAX_PDF_EXTRACTED_BYTES {
        return Err(format!("`{primary_path}` exceeds {MAX_PDF_EXTRACTED_BYTES}-byte reading cap"));
    }
    let actual_sha = ledger::sha256_hex(&raw_bytes);
    if &actual_sha != expected_sha {
        return Err(format!(
            "hash mismatch for `{primary_path}`: expected {expected_sha}, got {actual_sha}"
        ));
    }
    let text = String::from_utf8(raw_bytes)
        .map_err(|_| format!("`{primary_path}` is not valid UTF-8"))?;
    let line_count = split_physical_lines(&text).len();

    let (page_map, extra_quality_notes) = parse_receipt_page_map(data, status, line_count)?;
    let extractor_id = data["pdf_extractor"].as_str().map(str::to_string);
    let files_root_sha256 = compute_files_root_sha256(&files_sha256);

    Ok(IngestedPdfSource {
        source_id,
        source_seq: receipt.seq,
        primary_path,
        text,
        page_map,
        extractor_id,
        files_sha256,
        files_root_sha256,
        extra_quality_notes,
    })
}

/// Computes the 1-based physical `(start_page, end_page)` of a span strictly from its physical
/// file lines via [`PageMap::record_of_line`], skipping `PageBasis::None` (image-only) pages.
pub fn page_range_for_span(page_map: &PageMap, span: &Span) -> Option<(usize, usize)> {
    if span.start_line == 0 || span.end_line < span.start_line {
        return None;
    }
    let mut min_p: Option<usize> = None;
    let mut max_p: Option<usize> = None;
    for line in span.start_line..=span.end_line {
        if let Some(rec) = page_map.record_of_line(line) {
            if rec.basis == PageBasis::None {
                continue;
            }
            min_p = Some(min_p.map_or(rec.page, |m| m.min(rec.page)));
            max_p = Some(max_p.map_or(rec.page, |m| m.max(rec.page)));
        }
    }
    Some((min_p?, max_p?))
}

/// Computes the 1-based physical `(start_page, end_page)` of a skeleton node from its spans.
pub fn page_range_for_node(page_map: &PageMap, node: &SkNode) -> Option<(usize, usize)> {
    let mut min_p: Option<usize> = None;
    let mut max_p: Option<usize> = None;
    for span in &node.spans {
        if let Some((a, b)) = page_range_for_span(page_map, span) {
            min_p = Some(min_p.map_or(a, |m| m.min(a)));
            max_p = Some(max_p.map_or(b, |m| m.max(b)));
        }
    }
    Some((min_p?, max_p?))
}

/// Conservatively computes the [`PageBasis`] covering a node's physical lines (`None` > `ScanTextLayer` > `TextLayer`).
pub fn node_page_basis(page_map: &PageMap, node: &SkNode) -> Option<PageBasis> {
    let mut worst: Option<PageBasis> = None;
    for span in &node.spans {
        if span.start_line == 0 || span.end_line < span.start_line {
            continue;
        }
        for line in span.start_line..=span.end_line {
            let Some(rec) = page_map.record_of_line(line) else {
                continue;
            };
            worst = Some(match (worst, rec.basis) {
                (_, PageBasis::None) | (Some(PageBasis::None), _) => PageBasis::None,
                (_, PageBasis::ScanTextLayer) | (Some(PageBasis::ScanTextLayer), _) => {
                    PageBasis::ScanTextLayer
                }
                _ => PageBasis::TextLayer,
            });
        }
    }
    worst
}

/// Returns `true` only for digital [`PageBasis::TextLayer`]; formulas from scanned / OCR or
/// image-only pages are left as plain text (rule 6.21, `00-overview.md` 2.12).
pub fn render_formulas_as_math(basis: PageBasis) -> bool {
    matches!(basis, PageBasis::TextLayer)
}

/// Matches `query` against the pinned PDF reading file without mutating `stored_text`, returning
/// physical page links and rule-6.21 OCR display metadata.
///
/// - Skips matches inside `PageBasis::None` (image-only) pages so stray OCR glyphs on an
///   image-only page never shadow a real match or create a confirmed location.
/// - Returns `None` if the quote matches on more than one distinct physical page (ambiguous
///   location must never become a confirmed page link).
pub fn match_pdf_quote_for_display(
    primary_path: &str,
    stored_text: &str,
    page_map: &PageMap,
    query: &str,
) -> Option<PdfQuoteDisplay> {
    if query.trim().is_empty() || stored_text.is_empty() {
        return None;
    }

    // Collect every non-`PageBasis::None` match by scanning forward through `stored_text` slices
    // without mutating `stored_text`.
    let mut valid_matches: Vec<(PdfQuoteMatch, usize, usize, PageBasis)> = Vec::new();
    let mut offset = 0usize;
    while offset < stored_text.len() {
        let sub = &stored_text[offset..];
        let Some(mut m) = match_pdf_quote(sub, &PageMap { page_count: 0, pages: Vec::new() }, query)
        else {
            break;
        };
        let abs_start = offset + m.start_byte;
        let abs_end = offset + m.end_byte;
        let start_line = line_at_byte(stored_text, abs_start);
        let end_line = line_at_byte(stored_text, abs_end.saturating_sub(1).max(abs_start));
        m.start_byte = abs_start;
        m.end_byte = abs_end;
        m.start_line = start_line;
        m.end_line = end_line;

        if let Some((sp, ep, basis)) = classify_line_range(page_map, start_line, end_line) {
            m.start_page = Some(sp);
            m.end_page = Some(ep);
            m.start_page_basis = Some(basis);
            valid_matches.push((m, sp, ep, basis));
        }

        // Advance past the start of this hit on a valid UTF-8 boundary.
        let mut next = abs_start + 1;
        while next < stored_text.len() && !stored_text.is_char_boundary(next) {
            next += 1;
        }
        if next <= offset {
            break;
        }
        offset = next;
    }

    if valid_matches.is_empty() {
        return None;
    }
    // Reject ambiguous quotes that match on more than one distinct physical page.
    let distinct_pages: BTreeSet<usize> = valid_matches.iter().map(|(_, sp, _, _)| *sp).collect();
    if distinct_pages.len() > 1 {
        return None;
    }

    let (raw, start_page, end_page, worst_basis) = valid_matches.into_iter().next()?;
    let origin = match worst_basis {
        PageBasis::TextLayer => PdfTextOrigin::DigitalTextLayer,
        PageBasis::ScanTextLayer => PdfTextOrigin::ScanOcrText,
        PageBasis::None => return None,
    };
    let page_suffix = if start_page == end_page {
        format!("p. {start_page}")
    } else {
        format!("pp. {start_page}-{end_page}")
    };
    let display_label = format!("{}, {page_suffix}", origin.display_prefix());
    let span = Span::new(primary_path, raw.start_line, raw.end_line);
    let stored_excerpt = raw.stored_excerpt.clone();

    Some(PdfQuoteDisplay {
        span,
        start_page,
        end_page,
        basis: worst_basis,
        origin,
        is_ocr: worst_basis == PageBasis::ScanTextLayer,
        render_math: render_formulas_as_math(worst_basis),
        page_image_available: true,
        display_label,
        stored_excerpt,
        raw_match: raw,
    })
}

fn line_at_byte(text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(text.len());
    text[..clamped].bytes().filter(|&b| b == b'\n').count() + 1
}

fn classify_line_range(
    page_map: &PageMap,
    start_line: usize,
    end_line: usize,
) -> Option<(usize, usize, PageBasis)> {
    let start_rec = page_map.record_of_line(start_line)?;
    if start_rec.basis == PageBasis::None {
        return None;
    }
    let mut worst = start_rec.basis;
    let mut end_page = start_rec.page;
    for ln in start_line..=end_line {
        if let Some(rec) = page_map.record_of_line(ln) {
            if rec.basis == PageBasis::None {
                return None;
            }
            if rec.basis == PageBasis::ScanTextLayer {
                worst = PageBasis::ScanTextLayer;
            }
            end_page = end_page.max(rec.page);
        }
    }
    Some((start_rec.page, end_page, worst))
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn parse_receipt_page_map(
    data: &serde_json::Value,
    status: &str,
    line_count: usize,
) -> Result<(PageMap, Vec<String>), String> {
    let page_count = data["page_count"]
        .as_u64()
        .ok_or_else(|| "PDF source.ingested receipt missing `page_count`".to_string())?
        as usize;

    if let Some(raw_pages) = data.get("pdf_pages") {
        let records: Vec<PageRecord> = serde_json::from_value(raw_pages.clone())
            .map_err(|e| format!("invalid `pdf_pages` in receipt: {e}"))?;
        validate_page_records(&records, page_count, line_count)?;
        return Ok((PageMap { page_count, pages: records }, Vec::new()));
    }

    if status == "empty_text_layer" {
        let capped = page_count.min(MAX_PDF_PAGES);
        let records = (1..=capped)
            .map(|page| PageRecord {
                page,
                basis: PageBasis::None,
                first_line: None,
                last_line: None,
                metrics: empty_metrics(),
                reason: Some("empty_text_layer receipt".into()),
            })
            .collect();
        return Ok((PageMap { page_count, pages: records }, Vec::new()));
    }

    let raw_lines = data.get("pdf_page_lines").ok_or_else(|| {
        "pre-F14a PDF receipt missing both `pdf_pages` and `pdf_page_lines`; verify page ranges from raw_blob first"
            .to_string()
    })?;
    let ranges: Vec<(usize, usize, usize)> = serde_json::from_value(raw_lines.clone())
        .map_err(|e| format!("invalid `pdf_page_lines` in receipt: {e}"))?;
    if ranges.is_empty() && line_count > 0 {
        return Err("PDF receipt has non-empty reading text but empty `pdf_page_lines`".into());
    }
    if ranges.len() > MAX_PDF_PAGES {
        return Err("invalid `pdf_page_lines` count in receipt".into());
    }
    let records: Vec<PageRecord> = ranges
        .into_iter()
        .map(|(page, first, last)| PageRecord {
            page,
            basis: PageBasis::TextLayer,
            first_line: Some(first),
            last_line: Some(last),
            metrics: empty_metrics(),
            reason: None,
        })
        .collect();
    validate_page_records(&records, page_count, line_count)?;
    Ok((
        PageMap { page_count, pages: records },
        vec!["page scan status unknown (pre-F14a receipt without pdf_pages)".into()],
    ))
}

fn empty_metrics() -> PageQualityMetrics {
    PageQualityMetrics {
        char_count: 0,
        alpha_count: 0,
        word_count: 0,
        image_coverage_pct: 0,
        has_invisible_text: false,
        width_pt: 0,
        height_pt: 0,
    }
}

fn validate_page_records(
    records: &[PageRecord],
    page_count: usize,
    line_count: usize,
) -> Result<(), String> {
    if records.len() > MAX_PDF_PAGES {
        return Err(format!(
            "pdf_pages length ({}) exceeds MAX_PDF_PAGES ({MAX_PDF_PAGES})",
            records.len()
        ));
    }
    let mut prev_page = 0usize;
    let mut prev_last = 0usize;
    for rec in records {
        if rec.page == 0 || rec.page <= prev_page || rec.page > page_count {
            return Err(format!(
                "invalid physical page number {} (page_count={page_count})",
                rec.page
            ));
        }
        prev_page = rec.page;
        match (rec.first_line, rec.last_line) {
            (None, None) => {}
            (Some(first), Some(last)) => {
                if first == 0 || last < first || first <= prev_last || last > line_count {
                    return Err(format!(
                        "invalid line range {first}..={last} for page {} (line_count={line_count})",
                        rec.page
                    ));
                }
                prev_last = last;
            }
            _ => {
                return Err(format!("inconsistent first_line/last_line for page {}", rec.page));
            }
        }
    }
    Ok(())
}
