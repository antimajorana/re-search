//! External source bibliography/PDF resolution and Investigate wording guards for
//! the F15 Investigate write-up adapter (`writeup_paper.rs`).
//!
//! Resolves external `source.ingested` receipts and `investigation.cited_lookup` records so
//! external V12 `Prerequisite` / `Symbol` / `CitedResult` anchors carry accurate BibTeX
//! citations, physical PDF page locators, and `(OCR)` tags without hardcoding primary metadata.

use crate::writeup::BibEntry;
use crate::writeup_bib::resolve_bib_entry;
use crate::writeup_paper_meta::{PdfPageIndex, EVENT_SOURCE_INGESTED};
use ledger::Event;
use research_tools::library::Library;
use research_tools::map_fold::FoldedMap;
use research_tools::paper_key::{PaperIds, PaperKey, PaperRef};
use research_tools::status_badge::AUTHORS_LABEL_NEW;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const EVENT_INVESTIGATION_CITED_LOOKUP: &str = "investigation.cited_lookup";

/// Resolved bibliography identity for a non-primary `source_id` ingested in the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalSourceBib {
    pub cite_key: String,
    pub paper_ref: PaperRef,
}

/// Build a `source_id -> PdfPageIndex` map across every `source.ingested` event in the session
/// so both primary and external cited PDFs carry physical page locators and `(OCR)` flags.
pub(crate) fn build_pdf_indices(events: &[Event]) -> BTreeMap<String, PdfPageIndex> {
    let mut map = BTreeMap::new();
    for ev in events {
        if ev.kind != EVENT_SOURCE_INGESTED {
            continue;
        }
        if let Some(sid) = ev.payload["source_id"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            map.insert(sid.to_string(), PdfPageIndex::from_receipt(ev));
        }
    }
    map
}

/// Indexed `investigation.cited_lookup` records by bibliography key and external `source_id`.
#[derive(Debug, Default)]
pub(crate) struct CitedLookupMaps {
    pub by_bib_key: BTreeMap<String, PaperRef>,
    pub by_source_id: BTreeMap<String, (Option<String>, PaperRef)>,
}

/// Collect `bib_key -> PaperRef` and `external_source_id -> (Option<bib_key>, PaperRef)`
/// from `investigation.cited_lookup` events in the session.
pub(crate) fn collect_cited_lookups(events: &[Event]) -> CitedLookupMaps {
    let mut out = CitedLookupMaps::default();
    for ev in events {
        if ev.kind != EVENT_INVESTIGATION_CITED_LOOKUP {
            continue;
        }
        let Ok(pref) = serde_json::from_value::<PaperRef>(ev.payload["resolved"].clone()) else {
            continue;
        };
        let bib_key = ev.payload["bib_key"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(ref bk) = bib_key {
            out.by_bib_key.insert(bk.clone(), pref.clone());
        }
        if let Some(ext_sid) = ev.payload["external_source_id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            out.by_source_id.insert(ext_sid.to_string(), (bib_key, pref));
        }
    }
    out
}

/// Collect every non-primary `source_id` referenced by Layer-2 anchors or context cards.
pub(crate) fn collect_external_anchor_source_ids(
    primary_source_id: &str,
    folded: &FoldedMap,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |sid: &str| {
        let s = sid.trim();
        if !s.is_empty() && s != primary_source_id {
            out.insert(s.to_string());
        }
    };
    for r in folded.nodes.values() {
        if let Some(anchors) = &r.anchors.current {
            for a in &anchors.value {
                add(&a.source_id);
            }
        }
        if let Some(ac) = &r.authors_claim.current {
            for a in &ac.value.anchors {
                add(&a.source_id);
            }
        }
        if let Some(hyps) = &r.hypotheses.current {
            for h in &hyps.value {
                add(&h.anchor.source_id);
            }
        }
        for card in &r.context_cards {
            for a in &card.value.anchors {
                add(&a.source_id);
            }
            for h in &card.value.hypotheses {
                add(&h.anchor.source_id);
            }
        }
    }
    out
}

/// Resolve bibliography entries for every external `source_id` referenced by anchors/cards or
/// `investigation.cited_lookup`, appending deduplicated entries into `bib_entries`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_external_source_bibs(
    events: &[Event],
    lib: Option<&Library>,
    primary_source_id: &str,
    folded: &FoldedMap,
    cited_by_source: &BTreeMap<String, (Option<String>, PaperRef)>,
    cite_key_map: &mut BTreeMap<String, String>,
    paper_ref_map: &mut BTreeMap<String, PaperRef>,
    bib_entries: &mut Vec<BibEntry>,
) -> Result<BTreeMap<String, ExternalSourceBib>, String> {
    let mut used_keys: BTreeSet<String> = bib_entries.iter().map(|e| e.cite_key.clone()).collect();
    let mut by_paper_key: BTreeMap<PaperKey, (String, PaperRef)> = bib_entries
        .iter()
        .map(|e| (e.paper.key.clone(), (e.cite_key.clone(), e.paper.clone())))
        .collect();

    let receipts: BTreeMap<&str, &Event> = events
        .iter()
        .filter(|e| e.kind == EVENT_SOURCE_INGESTED)
        .filter_map(|e| {
            let sid = e.payload["source_id"].as_str().map(str::trim).filter(|s| !s.is_empty())?;
            Some((sid, e))
        })
        .collect();

    let mut needed = collect_external_anchor_source_ids(primary_source_id, folded);
    for sid in cited_by_source.keys() {
        if sid != primary_source_id {
            needed.insert(sid.clone());
        }
    }

    let mut source_bib_map = BTreeMap::new();
    for ext_sid in needed {
        let receipt = receipts.get(ext_sid.as_str()).copied();
        let (cited_bk, cited_pref) = cited_by_source
            .get(&ext_sid)
            .map(|(bk, pr)| (bk.clone(), Some(pr.clone())))
            .unwrap_or((None, None));
        let pref = cited_pref
            .or_else(|| receipt.and_then(|r| paper_ref_from_receipt(r, &ext_sid)))
            .unwrap_or_else(|| PaperRef {
                key: PaperKey::from_file_bytes(ext_sid.as_bytes()),
                version: None,
                ids: PaperIds::default(),
            });

        let (cite_key, resolved_pref) = if let Some((existing_ck, existing_pr)) = by_paper_key.get(&pref.key) {
            (existing_ck.clone(), existing_pr.clone())
        } else {
            let title = receipt
                .and_then(|r| r.payload["title"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("External source {ext_sid}"));
            let authors: Vec<String> = receipt
                .and_then(|r| r.payload.get("authors"))
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
            let year = receipt
                .and_then(|r| r.payload.get("year"))
                .and_then(Value::as_i64)
                .map(|y| y as i32);
            let entry = resolve_bib_entry(lib, &pref, &title, &authors, year, &mut used_keys)?;
            let ck = entry.cite_key.clone();
            let pr = entry.paper.clone();
            by_paper_key.insert(pr.key.clone(), (ck.clone(), pr.clone()));
            bib_entries.push(entry);
            (ck, pr)
        };

        if let Some(bk) = cited_bk {
            cite_key_map.entry(bk.clone()).or_insert_with(|| cite_key.clone());
            paper_ref_map.entry(bk).or_insert_with(|| resolved_pref.clone());
        }
        source_bib_map.insert(ext_sid, ExternalSourceBib { cite_key, paper_ref: resolved_pref });
    }
    Ok(source_bib_map)
}

fn paper_ref_from_receipt(receipt: &Event, fallback_seed: &str) -> Option<PaperRef> {
    if let Some(raw_pk) = receipt.payload["paper_key"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(pk) = PaperKey::parse(raw_pk) {
            return Some(PaperRef { key: pk, version: None, ids: PaperIds::default() });
        }
    }
    if let Some(origin) = receipt.payload["origin"].as_str().map(str::trim) {
        if let Some(rest) = origin.strip_prefix("library:") {
            if let Ok(pk) = PaperKey::parse(rest.trim()) {
                return Some(PaperRef { key: pk, version: None, ids: PaperIds::default() });
            }
        }
        if let Some(idx) = origin.find("arxiv.org/") {
            let tail = &origin[idx + "arxiv.org/".len()..];
            if let Some(id_part) = tail
                .strip_prefix("e-print/")
                .or_else(|| tail.strip_prefix("abs/"))
                .or_else(|| tail.strip_prefix("pdf/"))
            {
                let clean = id_part.trim_end_matches(".pdf").split(['?', '#']).next().unwrap_or("");
                if let Ok(pr) = PaperRef::from_ids(
                    PaperIds { arxiv: Some(clean.to_string()), ..Default::default() },
                    None,
                ) {
                    return Some(pr);
                }
            }
        }
    }
    Some(PaperRef {
        key: PaperKey::from_file_bytes(fallback_seed.as_bytes()),
        version: None,
        ids: PaperIds::default(),
    })
}

/// Detect forbidden claim words (`verified`, `proved`, `novel`, and `new` outside exact
/// `"Authors: New (model)"`) at ASCII word boundaries (`00-overview.md` 2.13 rule 4, 7.3).
pub(crate) fn find_forbidden_investigate_word(
    text: &str,
    allow_authors_new: bool,
) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("verified") {
        return Some("verified");
    }
    for word in ["proved", "novel"] {
        if contains_ascii_word(&lower, word) {
            return Some(word);
        }
    }
    let check_new = if allow_authors_new {
        lower.replace(&AUTHORS_LABEL_NEW.to_ascii_lowercase(), "")
    } else {
        lower
    };
    if contains_ascii_word(&check_new, "new") {
        return Some("new");
    }
    None
}

fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    let (hb, nb) = (haystack.as_bytes(), needle.as_bytes());
    if nb.is_empty() || nb.len() > hb.len() {
        return false;
    }
    for i in 0..=(hb.len() - nb.len()) {
        if &hb[i..i + nb.len()] == nb {
            let left_ok = i == 0 || !hb[i - 1].is_ascii_alphanumeric();
            let right_ok = i + nb.len() == hb.len() || !hb[i + nb.len()].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
        }
    }
    false
}
