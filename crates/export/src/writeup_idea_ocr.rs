//! PDF page / OCR provenance detection and ledger paper metadata extraction for the F15 Explore
//! write-up adapter (`writeup_idea.rs`, `00-overview.md` 2.12 & 2.13 rule 6, 8.3 Q3, `explore.md` C8).

use crate::writeup::QuoteLocator;
use ledger::Event;
use research_tools::paper_key::{split_arxiv_id, PaperIds, PaperKey};
use research_tools::pdf_pages::{page_of_line, PageRecord};
use research_tools::prior_work::TextBasis;
use serde_json::Value;
use std::collections::BTreeMap;

/// Candidate/literature fallback metadata folded from session ledger events for `.bib` entries.
#[derive(Debug, Clone, Default)]
pub(crate) struct PaperFallbackMeta {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub version: Option<String>,
    pub ids: PaperIds,
    pub alias_keys: Vec<PaperKey>,
}

pub(crate) fn make_locator(page: Option<usize>, start_line: usize, end_line: usize) -> QuoteLocator {
    match page {
        Some(p) if p >= 1 => QuoteLocator::Page { page: p, start_line, end_line },
        _ => QuoteLocator::Lines { start_line, end_line },
    }
}

/// Detect OCR provenance and physical 1-based PDF page across `idea.paper.text_basis`,
/// `evidence.quote`, and `source.ingested` (`pdf_pages` with `PageBasis::Ocr`/`ScanTextLayer`/
/// `None` OR raw strings, `pdf_page_lines`, and PDF scan flags).
pub(crate) fn detect_ocr_and_page(
    paper_ocr: bool,
    eq_payload: &Value,
    ingested_ev: Option<&Event>,
    start_line: usize,
    end_line: usize,
) -> (bool, Option<usize>) {
    let raw_eq_ocr = eq_payload.get("ocr").and_then(Value::as_bool).unwrap_or(false)
        || eq_payload.get("scanned").and_then(Value::as_bool).unwrap_or(false)
        || eq_payload
            .get("source_kind")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("ocr") || s.eq_ignore_ascii_case("scan"))
        || eq_payload.get("page_basis").and_then(Value::as_str).is_some_and(is_scan_basis_str)
        || eq_payload.get("text_basis").and_then(Value::as_str).is_some_and(is_scan_basis_str);

    let raw_eq_page = eq_payload
        .get("page")
        .and_then(Value::as_u64)
        .filter(|&p| p >= 1)
        .map(|p| p as usize);

    let (ingested_ocr, ingested_page) = ingested_ev
        .map(|ev| detect_ingested_ocr_and_page(&ev.payload, start_line, end_line, raw_eq_page))
        .unwrap_or((false, None));

    (paper_ocr || raw_eq_ocr || ingested_ocr, ingested_page.or(raw_eq_page))
}

fn is_scan_basis_str(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "scan_text_layer" | "none" | "ocr" | "scan"
    )
}

fn detect_ingested_ocr_and_page(
    payload: &Value,
    start_line: usize,
    end_line: usize,
    fallback_page: Option<usize>,
) -> (bool, Option<usize>) {
    let mut matched_pages: Vec<usize> = Vec::new();
    let mut any_scan = false;

    let parsed_pages = payload
        .get("pdf_pages")
        .and_then(|v| serde_json::from_value::<Vec<PageRecord>>(v.clone()).ok());
    let raw_pages_arr = payload.get("pdf_pages").and_then(Value::as_array);

    if let Some(pages) = &parsed_pages {
        for pg in pages {
            if let (Some(first), Some(last)) = (pg.first_line, pg.last_line) {
                if first <= end_line && last >= start_line {
                    if !matched_pages.contains(&pg.page) {
                        matched_pages.push(pg.page);
                    }
                    if pg.basis.is_scan() {
                        any_scan = true;
                    }
                }
            }
        }
    } else if let Some(arr) = raw_pages_arr {
        for item in arr {
            let page = item.get("page").and_then(Value::as_u64).map(|n| n as usize);
            let first = item.get("first_line").and_then(Value::as_u64).map(|n| n as usize);
            let last = item.get("last_line").and_then(Value::as_u64).map(|n| n as usize);
            if let (Some(p), Some(f), Some(l)) = (page, first, last) {
                if f <= end_line && l >= start_line {
                    if !matched_pages.contains(&p) {
                        matched_pages.push(p);
                    }
                    if item.get("basis").and_then(Value::as_str).is_some_and(is_scan_basis_str) {
                        any_scan = true;
                    }
                }
            }
        }
    }

    if matched_pages.is_empty() {
        if let Some(raw_lines) = payload.get("pdf_page_lines") {
            if let Ok(tuples) =
                serde_json::from_value::<Vec<(usize, usize, usize)>>(raw_lines.clone())
            {
                for &(page, first, last) in &tuples {
                    if first <= end_line && last >= start_line && !matched_pages.contains(&page) {
                        matched_pages.push(page);
                    }
                }
                if matched_pages.is_empty() {
                    if let Some(p) = page_of_line(&tuples, start_line) {
                        matched_pages.push(p);
                    }
                }
            }
        }
    }

    let pages_to_check: Vec<usize> = if matched_pages.is_empty() {
        fallback_page.into_iter().collect()
    } else {
        matched_pages.clone()
    };
    if !any_scan && !pages_to_check.is_empty() {
        if let Some(pages) = &parsed_pages {
            if pages.iter().any(|pg| pages_to_check.contains(&pg.page) && pg.basis.is_scan()) {
                any_scan = true;
            }
        } else if let Some(arr) = raw_pages_arr {
            for item in arr {
                let page = item.get("page").and_then(Value::as_u64).map(|n| n as usize);
                if page.is_some_and(|p| pages_to_check.contains(&p))
                    && item.get("basis").and_then(Value::as_str).is_some_and(is_scan_basis_str)
                {
                    any_scan = true;
                }
            }
        }
    }

    if !any_scan {
        let format_pdf = payload
            .get("format")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("pdf"));
        let status_scan = payload.get("status").and_then(Value::as_str).is_some_and(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "scan_only" | "ocr" | "scan_text_layer" | "empty_text_layer"
            )
        });
        let source_kind_ocr = payload
            .get("source_kind")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("ocr"));
        let explicit_ocr = payload.get("ocr").and_then(Value::as_bool).unwrap_or(false);
        if (format_pdf && status_scan) || source_kind_ocr || explicit_ocr {
            any_scan = true;
        }
    }

    (any_scan, matched_pages.first().copied())
}

pub(crate) fn collect_meta_from_candidates(p: &Value, map: &mut BTreeMap<PaperKey, PaperFallbackMeta>) {
    let Some(arr) = p.get("candidates").and_then(Value::as_array) else { return };
    for c in arr {
        let Some(key) = c.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())
        else {
            continue;
        };
        let entry = map.entry(key).or_default();
        if let Some(t) = c.get("title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
            entry.title = t.to_string();
        }
        if let Some(a) = c.get("authors").and_then(Value::as_array) {
            let names: Vec<String> = a
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !names.is_empty() {
                entry.authors = names;
            }
        }
        if let Some(y) = c.get("year").and_then(Value::as_i64) {
            entry.year = Some(y as i32);
        }
        if let Some(ids) = c.get("ids").and_then(|v| serde_json::from_value::<PaperIds>(v.clone()).ok()) {
            merge_ids(&mut entry.ids, &ids);
            push_alias_keys(&ids, &mut entry.alias_keys);
        }
    }
}

pub(crate) fn collect_meta_from_lit_fetched(p: &Value, map: &mut BTreeMap<PaperKey, PaperFallbackMeta>) {
    let items = p.get("papers").or_else(|| p.get("hits")).and_then(Value::as_array);
    let Some(arr) = items else { return };
    for item in arr {
        let Some(key) = item.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())
        else {
            continue;
        };
        let entry = map.entry(key).or_default();
        if entry.title.is_empty() {
            if let Some(t) = item.get("title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                entry.title = t.to_string();
            }
        }
        if entry.authors.is_empty() {
            if let Some(a) = item.get("authors").and_then(Value::as_array) {
                entry.authors = a
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
        }
        if entry.year.is_none() {
            entry.year = item.get("year").and_then(Value::as_i64).map(|y| y as i32);
        }
    }
}

pub(crate) fn collect_meta_from_idea_paper(
    p: &Value,
    text_basis: &mut BTreeMap<PaperKey, TextBasis>,
    map: &mut BTreeMap<PaperKey, PaperFallbackMeta>,
) {
    let Some(key) = p.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())
    else {
        return;
    };
    if let Some(tb) = p.get("text_basis").and_then(Value::as_str).and_then(TextBasis::parse) {
        text_basis.insert(key.clone(), tb);
    }
    if let Some(ver) = p.get("version").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        map.entry(key).or_default().version = Some(ver.to_string());
    }
}

pub(crate) fn collect_meta_from_ingested(p: &Value, map: &mut BTreeMap<PaperKey, PaperFallbackMeta>) {
    let title = p.get("title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let key = p
        .get("origin")
        .and_then(Value::as_str)
        .and_then(|o| o.trim().strip_prefix("library:").and_then(|r| PaperKey::parse(r.trim()).ok()))
        .or_else(|| {
            p.get("arxiv_cover_id")
                .and_then(Value::as_str)
                .and_then(|cid| split_arxiv_id(cid).ok())
                .and_then(|(base, _)| PaperKey::from_arxiv(&base).ok())
        });
    if let (Some(k), Some(t)) = (key, title) {
        let entry = map.entry(k).or_default();
        if entry.title.is_empty() {
            entry.title = t.to_string();
        }
    }
}

fn merge_ids(dst: &mut PaperIds, src: &PaperIds) {
    if dst.arxiv.is_none() { dst.arxiv = src.arxiv.clone(); }
    if dst.doi.is_none() { dst.doi = src.doi.clone(); }
    if dst.inspire.is_none() { dst.inspire = src.inspire.clone(); }
    if dst.s2.is_none() { dst.s2 = src.s2.clone(); }
}

fn push_alias_keys(ids: &PaperIds, out: &mut Vec<PaperKey>) {
    for k in [
        ids.arxiv.as_deref().and_then(|s| PaperKey::from_arxiv(s).ok()),
        ids.doi.as_deref().and_then(|s| PaperKey::from_doi(s).ok()),
        ids.inspire.as_deref().and_then(|s| PaperKey::from_inspire(s).ok()),
        ids.s2.as_deref().and_then(|s| PaperKey::from_s2(s).ok()),
    ]
    .into_iter()
    .flatten()
    {
        if !out.contains(&k) {
            out.push(k);
        }
    }
}
