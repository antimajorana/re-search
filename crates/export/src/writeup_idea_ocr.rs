//! PDF page / OCR provenance detection and ledger paper metadata extraction for the F15 Explore
//! write-up adapter (`writeup_idea.rs`, `00-overview.md` 2.12 & 2.13 rule 6, 8.3 Q3, `explore.md` C8).

use crate::writeup::QuoteLocator;
use ledger::{sha256_hex, Event};
use research_tools::library::Library;
use research_tools::paper_key::{split_arxiv_id, PaperIds, PaperKey, PaperKeyKind, PaperRef};
use research_tools::pdf_pages::{page_of_line, PageRecord};
use research_tools::prior_work::TextBasis;
use research_tools::quote_core::QuoteRef;
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

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
///
/// When a multi-page quote spans both clean and scanned pages, prefers the scanned physical page
/// so the `(OCR)` tag always names a scanned page (Audit M1).
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
    let mut scanned_pages: Vec<usize> = Vec::new();
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
                        if !scanned_pages.contains(&pg.page) {
                            scanned_pages.push(pg.page);
                        }
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
                        if !scanned_pages.contains(&p) {
                            scanned_pages.push(p);
                        }
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
            for pg in pages {
                if pages_to_check.contains(&pg.page) && pg.basis.is_scan() {
                    any_scan = true;
                    if !scanned_pages.contains(&pg.page) {
                        scanned_pages.push(pg.page);
                    }
                }
            }
        } else if let Some(arr) = raw_pages_arr {
            for item in arr {
                let page = item.get("page").and_then(Value::as_u64).map(|n| n as usize);
                if let Some(p) = page.filter(|p| pages_to_check.contains(p)) {
                    if item.get("basis").and_then(Value::as_str).is_some_and(is_scan_basis_str) {
                        any_scan = true;
                        if !scanned_pages.contains(&p) {
                            scanned_pages.push(p);
                        }
                    }
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

    let chosen_page = if any_scan {
        scanned_pages.first().copied().or_else(|| matched_pages.first().copied())
    } else {
        matched_pages.first().copied()
    };
    (any_scan, chosen_page)
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
    paper_source_ids: &mut BTreeMap<PaperKey, BTreeSet<String>>,
    map: &mut BTreeMap<PaperKey, PaperFallbackMeta>,
) {
    let Some(key) = p.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())
    else {
        return;
    };
    if let Some(tb) = p.get("text_basis").and_then(Value::as_str).and_then(TextBasis::parse) {
        text_basis.insert(key.clone(), tb);
    }
    if let Some(sid) = p.get("source_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        paper_source_ids.entry(key.clone()).or_default().insert(sid.to_string());
    }
    if let Some(ver) = p.get("version").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        map.entry(key).or_default().version = Some(ver.to_string());
    }
}

pub(crate) fn collect_meta_from_ingested(
    p: &Value,
    source_origin_keys: &mut BTreeMap<String, PaperKey>,
    map: &mut BTreeMap<PaperKey, PaperFallbackMeta>,
) {
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
    if let Some(ref k) = key {
        if let Some(sid) = p.get("source_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
            source_origin_keys.insert(sid.to_string(), k.clone());
        }
        if let Some(t) = title {
            let entry = map.entry(k.clone()).or_default();
            if entry.title.is_empty() {
                entry.title = t.to_string();
            }
        }
    }
}

pub(crate) fn merge_ids(dst: &mut PaperIds, src: &PaperIds) {
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

pub(crate) fn extract_quote_ref(p: &Value, seq: u64) -> Option<QuoteRef> {
    let source_id = p.get("source_id")?.as_str()?.to_string();
    let path = p.get("path")?.as_str()?.to_string();
    let start_line = p.get("start_line")?.as_u64()? as usize;
    let end_line = p.get("end_line")?.as_u64()? as usize;
    let file_sha256 = p.get("file_sha256")?.as_str()?.to_string();
    let quote_sha = p.get("quote_sha256")?.as_str()?.to_string();
    if source_id.is_empty()
        || start_line == 0
        || end_line < start_line
        || !is_sha256_hex(&file_sha256)
        || !is_sha256_hex(&quote_sha)
    {
        return None;
    }
    Some(QuoteRef {
        source_id,
        path,
        start_line,
        end_line,
        file_sha256,
        quote_sha256: quote_sha,
        evidence_seq: Some(seq),
    })
}

pub(crate) fn full_quote_present_in_verified_file(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    qref: &QuoteRef,
    raw_quote: &str,
) -> bool {
    let Ok(resolved) = sandbox.resolve_existing(SandboxRoot::Sources, session_id, &qref.path, false)
    else {
        return false;
    };
    let Ok(bytes) = fs::read(&resolved.canonical_target) else {
        return false;
    };
    if sha256_hex(&bytes) != qref.file_sha256 {
        return false;
    }
    let text = String::from_utf8_lossy(&bytes);
    let folded_file: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let folded_quote: String = raw_quote.split_whitespace().collect::<Vec<_>>().join(" ");
    !folded_quote.is_empty() && folded_file.contains(&folded_quote)
}

pub(crate) fn build_paper_ref(
    lib: Option<&Library>,
    candidate: &PaperKey,
    fallback_ids: &PaperIds,
    version: Option<String>,
) -> Result<PaperRef, String> {
    if let Some(l) = lib {
        if let Some(detail) = l.get_paper(candidate)? {
            let ids = PaperIds {
                arxiv: detail.paper.ids.arxiv.clone().or_else(|| fallback_ids.arxiv.clone()),
                doi: detail.paper.ids.doi.clone().or_else(|| fallback_ids.doi.clone()),
                inspire: detail.paper.ids.inspire.clone().or_else(|| fallback_ids.inspire.clone()),
                s2: detail.paper.ids.s2.clone().or_else(|| fallback_ids.s2.clone()),
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

    let mut ids = fallback_ids.clone();
    let fallback = match candidate.kind() {
        PaperKeyKind::Arxiv => {
            if ids.arxiv.is_none() {
                ids.arxiv = Some(candidate.value().to_string());
            }
            None
        }
        PaperKeyKind::Doi => {
            if ids.doi.is_none() {
                ids.doi = Some(candidate.value().to_string());
            }
            None
        }
        PaperKeyKind::Inspire => {
            if ids.inspire.is_none() {
                ids.inspire = Some(candidate.value().to_string());
            }
            None
        }
        PaperKeyKind::S2 => {
            if ids.s2.is_none() {
                ids.s2 = Some(candidate.value().to_string());
            }
            None
        }
        PaperKeyKind::Openalex | PaperKeyKind::File => Some(candidate.clone()),
    };
    PaperRef::from_ids_or(ids, version, fallback)
}

pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
