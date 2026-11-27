//! Citation re-verification, OCR / physical PDF page detection, and library-backed `.bib`
//! resolution for the F15 Solve write-up adapter (`writeup_solve.rs`).
//!
//! Pure local deterministic code: zero model calls, zero network calls. Every Cited quote is
//! re-verified through F4 [`crate::writeup::build_verified_quote`] (`<= 300` chars) and every
//! `.bib` key prefers the user's own `source_item.citekey` from F10/F13 (`00-overview.md` 2.13).

use crate::writeup::{
    build_verified_quote, writeup_wording, BibEntry, QuoteLocator, WriteupQuote,
};
use crate::writeup_bib::resolve_bib_entry;
use ledger::Event;
use research_tools::library::Library;
use research_tools::paper_key::{split_arxiv_id, PaperIds, PaperKey, PaperKeyKind, PaperRef};
use research_tools::pdf_pages::{page_of_line, PageRecord};
use research_tools::quote_core::{quote_sha256, QuoteRef};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::solve_model::{event_kinds, SolveEntry};
use research_tools::solve_state::SolveState;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Verified quote block plus its resolved OCR status for one Cited Solve step.
pub(crate) struct ResolvedCitation {
    pub quote: WriteupQuote,
    pub ocr: bool,
}

/// Stateful per-writeup citation resolver that deduplicates bibliography entries by [`PaperKey`]
/// and preserves deterministic first-cited order.
pub(crate) struct SolveCiteResolver<'a> {
    session_id: &'a str,
    events: &'a [Event],
    state: &'a SolveState,
    sandbox: &'a WorkspaceSandbox,
    lib: Option<&'a Library>,
    ingested_by_source: BTreeMap<String, &'a Event>,
    quote_events: BTreeMap<u64, &'a Event>,
    bib_by_key: BTreeMap<PaperKey, BibEntry>,
    bib_order: Vec<PaperKey>,
    used_keys: BTreeSet<String>,
}

impl<'a> SolveCiteResolver<'a> {
    pub(crate) fn new(
        session_id: &'a str,
        events: &'a [Event],
        state: &'a SolveState,
        sandbox: &'a WorkspaceSandbox,
        lib: Option<&'a Library>,
    ) -> Self {
        let mut ingested_by_source = BTreeMap::new();
        let mut quote_events = BTreeMap::new();
        for ev in events {
            match ev.kind.as_str() {
                "source.ingested" => {
                    if let Some(sid) = ev.payload.get("source_id").and_then(Value::as_str) {
                        ingested_by_source.insert(sid.to_string(), ev);
                    }
                }
                event_kinds::EVIDENCE_QUOTE => {
                    quote_events.insert(ev.seq, ev);
                }
                _ => {}
            }
        }
        Self {
            session_id,
            events,
            state,
            sandbox,
            lib,
            ingested_by_source,
            quote_events,
            bib_by_key: BTreeMap::new(),
            bib_order: Vec::new(),
            used_keys: BTreeSet::new(),
        }
    }

    /// Re-verify a Cited step's [`QuoteRef`] through F4 `build_verified_quote`, resolve its
    /// physical PDF page and OCR status, and register its [`BibEntry`] (preferring
    /// `source_item.citekey` from the user's library).
    pub(crate) fn resolve_step_citation(
        &mut self,
        entry: &SolveEntry,
        qref: &QuoteRef,
    ) -> Result<ResolvedCitation, String> {
        let seq = qref.evidence_seq.ok_or_else(|| {
            format!(
                "Solve write-up refused: cited step `{}` source_ref is missing `evidence_seq`",
                entry.entry_id
            )
        })?;
        let folded_rec = self
            .state
            .quotes
            .get(&seq)
            .filter(|r| r.matches_ref(qref))
            .ok_or_else(|| {
                format!(
                    "Solve write-up refused: cited step `{}` references missing or mismatched `evidence.quote` seq {seq}",
                    entry.entry_id
                )
            })?;
        let eq_ev = self.quote_events.get(&seq).copied().ok_or_else(|| {
            format!(
                "Solve write-up refused: `evidence.quote` event seq {seq} not found for step `{}`",
                entry.entry_id
            )
        })?;
        let quote_text = extract_quote_text(&eq_ev.payload).ok_or_else(|| {
            format!(
                "Solve write-up refused: `evidence.quote` seq {seq} for step `{}` has empty quote text",
                entry.entry_id
            )
        })?;

        let ingested_ev = self.ingested_by_source.get(&qref.source_id).copied();
        let (paper_ref, cite_key) = self.resolve_paper_and_citekey(eq_ev, ingested_ev, qref)?;

        let (initial_ocr, initial_page) = detect_ocr_and_page(
            folded_rec.ocr,
            &eq_ev.payload,
            ingested_ev,
            qref.start_line,
            qref.end_line,
        );
        let initial_locator = make_locator(initial_page, qref.start_line, qref.end_line);

        let mut verified = build_verified_quote(
            self.sandbox,
            self.session_id,
            self.events,
            cite_key,
            paper_ref,
            qref,
            quote_text,
            initial_locator,
            initial_ocr,
        )
        .map_err(|fault| {
            format!(
                "Solve write-up refused: cited quote on step `{}` failed F4 re-verification ({fault})",
                entry.entry_id
            )
        })?;

        // Recompute page & OCR on the final verified line span (in case F4 narrowed/relocated it).
        let (final_ocr, final_page) = detect_ocr_and_page(
            folded_rec.ocr,
            &eq_ev.payload,
            ingested_ev,
            verified.quote_ref.start_line,
            verified.quote_ref.end_line,
        );
        verified.ocr = final_ocr;
        verified.locator = make_locator(
            final_page,
            verified.quote_ref.start_line,
            verified.quote_ref.end_line,
        );

        // When `quote_text` is a matched substring of a longer line, `verify_quote_ref` returns
        // the full line as `excerpt` while binding `quote_ref.quote_sha256` to `quote_text`.
        // Keep the exact verified `quote_text` so Writeup::validate()'s sha256 and 300-char
        // invariants hold without ever writing unverified text.
        if verified.text.chars().count() > writeup_wording::MAX_QUOTE_CHARS
            || quote_sha256(&verified.text) != verified.quote_ref.quote_sha256
        {
            verified.text = quote_text.to_string();
        }

        Ok(ResolvedCitation {
            quote: verified,
            ocr: final_ocr,
        })
    }

    pub(crate) fn into_bibliography(mut self) -> Vec<BibEntry> {
        let mut out = Vec::with_capacity(self.bib_order.len());
        for key in self.bib_order {
            if let Some(entry) = self.bib_by_key.remove(&key) {
                out.push(entry);
            }
        }
        out
    }

    fn resolve_paper_and_citekey(
        &mut self,
        eq_ev: &Event,
        ingested_ev: Option<&Event>,
        qref: &QuoteRef,
    ) -> Result<(PaperRef, String), String> {
        let (candidate_key, version) = extract_paper_key_and_version(eq_ev, ingested_ev, qref)?;
        let paper_ref = build_paper_ref(self.lib, &candidate_key, version)?;
        let canonical_key = paper_ref.key.clone();

        if let Some(existing) = self.bib_by_key.get(&canonical_key) {
            return Ok((existing.paper.clone(), existing.cite_key.clone()));
        }

        let fallback_title = ingested_ev
            .and_then(|ev| ev.payload.get("title").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&qref.source_id);

        let entry = resolve_bib_entry(
            self.lib,
            &paper_ref,
            fallback_title,
            &[],
            None,
            &mut self.used_keys,
        )?;
        let cite_key = entry.cite_key.clone();
        let resolved_paper = entry.paper.clone();
        self.bib_order.push(canonical_key.clone());
        self.bib_by_key.insert(canonical_key, entry);
        Ok((resolved_paper, cite_key))
    }
}

fn extract_quote_text(payload: &Value) -> Option<&str> {
    payload
        .get("quote")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            payload
                .get("excerpt")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        })
}

fn make_locator(page: Option<usize>, start_line: usize, end_line: usize) -> QuoteLocator {
    match page {
        Some(p) if p >= 1 => QuoteLocator::Page {
            page: p,
            start_line,
            end_line,
        },
        _ => QuoteLocator::Lines {
            start_line,
            end_line,
        },
    }
}

/// Detect OCR provenance and physical 1-based PDF page across all recorded signals:
/// - `EvidenceQuoteRecord.ocr` (folded from `evidence.quote`)
/// - raw `evidence.quote` fields (`ocr`, `scanned`, `source_kind: "ocr"`, `page_basis`, `page`)
/// - `source.ingested` `pdf_pages` (`PageBasis::ScanTextLayer` / `PageBasis::None` / raw `"ocr"`),
///   `pdf_page_lines`, and PDF scan status fields.
pub(crate) fn detect_ocr_and_page(
    folded_ocr: bool,
    eq_payload: &Value,
    ingested_ev: Option<&Event>,
    start_line: usize,
    end_line: usize,
) -> (bool, Option<usize>) {
    let raw_eq_ocr = eq_payload
        .get("ocr")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || eq_payload
            .get("scanned")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || eq_payload
            .get("source_kind")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("ocr"))
        || eq_payload
            .get("page_basis")
            .and_then(Value::as_str)
            .is_some_and(|s| matches!(s, "scan_text_layer" | "none" | "ocr"));

    let raw_eq_page = eq_payload
        .get("page")
        .and_then(Value::as_u64)
        .filter(|&p| p >= 1)
        .map(|p| p as usize);

    let (ingested_ocr, ingested_page) = ingested_ev
        .map(|ev| detect_ingested_ocr_and_page(&ev.payload, start_line, end_line))
        .unwrap_or((false, None));

    (
        folded_ocr || raw_eq_ocr || ingested_ocr,
        ingested_page.or(raw_eq_page),
    )
}

fn detect_ingested_ocr_and_page(
    payload: &Value,
    start_line: usize,
    end_line: usize,
) -> (bool, Option<usize>) {
    let mut matched_page: Option<usize> = None;
    let mut any_scan = false;

    if let Some(raw_pages) = payload.get("pdf_pages") {
        if let Ok(pages) = serde_json::from_value::<Vec<PageRecord>>(raw_pages.clone()) {
            for pg in &pages {
                if let (Some(first), Some(last)) = (pg.first_line, pg.last_line) {
                    if first <= end_line && last >= start_line {
                        matched_page.get_or_insert(pg.page);
                        if pg.basis.is_scan() {
                            any_scan = true;
                        }
                    }
                }
            }
        } else if let Some(arr) = raw_pages.as_array() {
            // Tolerate minimal / raw `"basis": "ocr"` page objects written in fixtures or older events.
            for item in arr {
                let page = item.get("page").and_then(Value::as_u64).map(|n| n as usize);
                let first = item
                    .get("first_line")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize);
                let last = item
                    .get("last_line")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize);
                if let (Some(p), Some(f), Some(l)) = (page, first, last) {
                    if f <= end_line && l >= start_line {
                        matched_page.get_or_insert(p);
                        if item
                            .get("basis")
                            .and_then(Value::as_str)
                            .is_some_and(|b| matches!(b, "scan_text_layer" | "none" | "ocr"))
                        {
                            any_scan = true;
                        }
                    }
                }
            }
        }
    }

    if matched_page.is_none() {
        if let Some(raw_lines) = payload.get("pdf_page_lines") {
            if let Ok(tuples) = serde_json::from_value::<Vec<(usize, usize, usize)>>(raw_lines.clone())
            {
                matched_page = page_of_line(&tuples, start_line);
            }
        }
    }

    if !any_scan {
        let format_pdf = payload.get("format").and_then(Value::as_str) == Some("pdf");
        let status_scan = payload
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|s| matches!(s, "scan_only" | "ocr" | "scan_text_layer"));
        let source_kind_ocr = payload
            .get("source_kind")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("ocr"));
        let explicit_ocr = payload.get("ocr").and_then(Value::as_bool).unwrap_or(false);
        if (format_pdf && status_scan) || source_kind_ocr || explicit_ocr {
            any_scan = true;
        }
    }

    (any_scan, matched_page)
}

fn extract_paper_key_and_version(
    eq_ev: &Event,
    ingested_ev: Option<&Event>,
    qref: &QuoteRef,
) -> Result<(PaperKey, Option<String>), String> {
    if let Some(raw) = eq_ev.payload.get("paper_key").and_then(Value::as_str) {
        if let Ok(key) = PaperKey::parse(raw.trim()) {
            let ver = ingested_ev
                .and_then(|ev| ev.payload.get("arxiv_cover_id").and_then(Value::as_str))
                .and_then(|cid| split_arxiv_id(cid).ok())
                .and_then(|(_, v)| v);
            return Ok((key, ver));
        }
    }

    if let Some(ev) = ingested_ev {
        if let Some(origin) = ev.payload.get("origin").and_then(Value::as_str) {
            let trimmed = origin.trim();
            if let Some(rest) = trimmed.strip_prefix("library:") {
                if let Ok(key) = PaperKey::parse(rest.trim()) {
                    return Ok((key, None));
                }
            }
            if let Ok((base, ver)) = split_arxiv_id(trimmed) {
                if let Ok(key) = PaperKey::from_arxiv(&base) {
                    return Ok((key, ver));
                }
            }
            if let Ok(key) = PaperKey::from_doi(trimmed) {
                return Ok((key, None));
            }
            if let Ok(key) = PaperKey::parse(trimmed) {
                return Ok((key, None));
            }
        }
        if let Some(cover) = ev.payload.get("arxiv_cover_id").and_then(Value::as_str) {
            if let Ok((base, ver)) = split_arxiv_id(cover) {
                if let Ok(key) = PaperKey::from_arxiv(&base) {
                    return Ok((key, ver));
                }
            }
        }
        if let Some(raw_blob) = ev.payload.get("raw_blob").and_then(Value::as_str) {
            if raw_blob.len() >= 16 {
                let candidate = format!("file:{}", raw_blob[..16].to_ascii_lowercase());
                if let Ok(key) = PaperKey::parse(&candidate) {
                    return Ok((key, None));
                }
            }
        }
    }

    if qref.file_sha256.len() >= 16 {
        let candidate = format!("file:{}", qref.file_sha256[..16].to_ascii_lowercase());
        if let Ok(key) = PaperKey::parse(&candidate) {
            return Ok((key, None));
        }
    }

    Err(format!(
        "could not derive a canonical PaperKey for source `{}`",
        qref.source_id
    ))
}

fn build_paper_ref(
    lib: Option<&Library>,
    candidate: &PaperKey,
    version: Option<String>,
) -> Result<PaperRef, String> {
    if let Some(l) = lib {
        if let Some(detail) = l.get_paper(candidate)? {
            let ids = PaperIds {
                arxiv: detail.paper.ids.arxiv.clone(),
                doi: detail.paper.ids.doi.clone(),
                inspire: detail.paper.ids.inspire.clone(),
                s2: detail.paper.ids.s2.clone(),
            };
            let ver = version.or_else(|| {
                detail
                    .source_items
                    .iter()
                    .find_map(|it| it.version.clone())
                    .filter(|v| !v.is_empty())
            });
            return PaperRef::from_ids_or(ids, ver, Some(detail.paper.key));
        }
    }

    let mut ids = PaperIds::default();
    let fallback = match candidate.kind() {
        PaperKeyKind::Arxiv => {
            ids.arxiv = Some(candidate.value().to_string());
            None
        }
        PaperKeyKind::Doi => {
            ids.doi = Some(candidate.value().to_string());
            None
        }
        PaperKeyKind::Inspire => {
            ids.inspire = Some(candidate.value().to_string());
            None
        }
        PaperKeyKind::S2 => {
            ids.s2 = Some(candidate.value().to_string());
            None
        }
        PaperKeyKind::Openalex | PaperKeyKind::File => Some(candidate.clone()),
    };
    PaperRef::from_ids_or(ids, version, fallback)
}
