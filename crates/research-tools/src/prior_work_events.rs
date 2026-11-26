//! Ledger-event extractor for `crate::prior_work` (`explore.md` 4.2, rules V6, V7, V9, V10, C3, C8).
//!
//! Explicitly ignores `lessons.supplied` and non-confirmation F16/ranking marks.

use super::{
    AbstractEvidence, EvidenceId, FullTextEvidence, OverlapCandidate, OverlapRelation,
    PriorWorkInput, TextBasis, HANDOFF_PACKET_EVENT, IDEA_MAP_EVENT, NOTE_ACTION_CONFIRM_SAME,
    NOTE_ACTION_REJECT_SAME, SOURCE_INGESTED_EVENT,
};
use crate::lines_sent::is_sha256_hex;
use crate::lit_providers::{LitProvider, LITERATURE_FETCHED_EVENT};
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use crate::quote_core::{
    quote_sha256, QuoteRef, EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT,
};
use crate::search_coverage::{
    paper_ref_matches_key, PaperReadStatus, SearchCoverage, IDEA_BRIEF_EVENT,
    IDEA_CANDIDATES_EVENT, IDEA_NOTE_EVENT, IDEA_PAPER_EVENT, IDEA_STATED_EVENT,
};
use ledger::Event;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy)]
struct PdfPageSpan {
    first_line: usize,
    last_line: usize,
    basis: TextBasis,
}

type OverlapSignature = BTreeSet<(PaperKey, OverlapRelation, Vec<EvidenceId>, Option<String>, String)>;

pub(super) fn extract_prior_work_input(
    events: &[Event],
    coverage: SearchCoverage,
) -> PriorWorkInput {
    let mut subclaim_ids: Vec<String> = Vec::new();
    let mut paper_reads: BTreeMap<PaperKey, PaperReadStatus> = BTreeMap::new();
    let mut paper_text_basis: BTreeMap<PaperKey, TextBasis> = BTreeMap::new();
    let mut source_pdf_pages: BTreeMap<String, Vec<PdfPageSpan>> = BTreeMap::new();
    let mut source_file_hashes: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut full_text_evidence: Vec<FullTextEvidence> = Vec::new();
    let mut abstract_evidence: Vec<AbstractEvidence> = Vec::new();
    let mut overlaps: Vec<OverlapCandidate> = Vec::new();
    let mut prev_sigs: BTreeMap<String, OverlapSignature> = BTreeMap::new();
    let mut current_map_seq: Option<u64> = None;
    let mut seen_brief = false;
    let mut confirmed_subclaims: BTreeSet<String> = BTreeSet::new();
    let mut origin_paper: Option<PaperRef> = None;

    for ev in events {
        match ev.kind.as_str() {
            HANDOFF_PACKET_EVENT => {
                if let Some(pref) = parse_origin_paper(&ev.payload) {
                    origin_paper = Some(pref);
                }
            }
            IDEA_STATED_EVENT => {
                // Revising the idea invalidates any pre-revision map and confirmations (R2).
                overlaps.clear();
                prev_sigs.clear();
                confirmed_subclaims.clear();
                current_map_seq = None;
            }
            IDEA_BRIEF_EVENT => {
                if let Some(arr) = ev.payload.get("subclaims").and_then(Value::as_array) {
                    if seen_brief {
                        // Re-sharpening replaces subclaims; clear pre-revision map/confirmations (R2).
                        overlaps.clear();
                        prev_sigs.clear();
                        confirmed_subclaims.clear();
                        current_map_seq = None;
                    }
                    seen_brief = true;
                    subclaim_ids = arr
                        .iter()
                        .filter_map(|sc| sc.get("id").and_then(Value::as_str).map(str::to_string))
                        .collect();
                }
            }
            IDEA_CANDIDATES_EVENT => {
                enrich_origin_from_candidates(&ev.payload, &mut origin_paper);
            }
            LITERATURE_FETCHED_EVENT => {
                enrich_origin_from_lit_fetched(&ev.payload, &mut origin_paper);
            }
            SOURCE_INGESTED_EVENT => {
                parse_source_ingested_pages(&ev.payload, &mut source_pdf_pages, &mut source_file_hashes);
            }
            IDEA_PAPER_EVENT => {
                parse_paper_event(&ev.payload, &mut paper_reads, &mut paper_text_basis);
            }
            EVIDENCE_QUOTE_EVENT => {
                if let Some(rec) = parse_ft_evidence_event(
                    ev,
                    &paper_text_basis,
                    &source_pdf_pages,
                    &source_file_hashes,
                ) {
                    full_text_evidence.push(rec);
                }
            }
            EVIDENCE_ABSTRACT_EVENT => {
                if let Some(rec) = parse_abs_evidence_event(ev) {
                    abstract_evidence.push(rec);
                }
            }
            IDEA_MAP_EVENT => {
                if let Some(arr) = ev.payload.get("overlaps").and_then(Value::as_array) {
                    overlaps = arr.iter().filter_map(parse_overlap_value).collect();
                    let next_sigs = build_overlap_signatures(&overlaps);
                    // Retain confirmation only if the subclaim's exact overlap signature stayed unchanged (H5).
                    confirmed_subclaims.retain(|sc| {
                        prev_sigs.contains_key(sc) && prev_sigs.get(sc) == next_sigs.get(sc)
                    });
                    prev_sigs = next_sigs;
                    current_map_seq = Some(ev.seq);
                }
            }
            IDEA_NOTE_EVENT => {
                parse_confirmation_note(
                    &ev.payload,
                    current_map_seq,
                    &prev_sigs,
                    &mut confirmed_subclaims,
                );
            }
            _ => {}
        }
    }

    for sc in overlaps
        .iter()
        .map(|o| &o.subclaim_id)
        .chain(abstract_evidence.iter().flat_map(|a| a.subclaim_ids.iter()))
    {
        if !subclaim_ids.contains(sc) {
            subclaim_ids.push(sc.clone());
        }
    }

    PriorWorkInput {
        subclaim_ids,
        coverage,
        paper_reads,
        paper_text_basis,
        full_text_evidence,
        abstract_evidence,
        overlaps,
        confirmed_subclaims,
        origin_paper,
    }
}

fn parse_origin_paper(p: &Value) -> Option<PaperRef> {
    let paper = p.get("from")?.get("paper")?;
    let key = paper.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())?;
    let raw_ids: PaperIds = paper
        .get("ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let ids = normalize_ids_with_key(&key, &raw_ids);
    let version = paper.get("version").and_then(Value::as_str).map(str::to_string);
    Some(PaperRef { key, version, ids })
}

fn normalize_ids_with_key(key: &PaperKey, raw: &PaperIds) -> PaperIds {
    let mut ids = PaperIds {
        arxiv: raw.arxiv.as_deref().and_then(|s| PaperKey::from_arxiv(s).ok()).map(|k| k.value().to_string()),
        doi: raw.doi.as_deref().and_then(|s| PaperKey::from_doi(s).ok()).map(|k| k.value().to_string()),
        inspire: raw.inspire.as_deref().and_then(|s| PaperKey::from_inspire(s).ok()).map(|k| k.value().to_string()),
        s2: raw.s2.as_deref().and_then(|s| PaperKey::from_s2(s).ok()).map(|k| k.value().to_string()),
    };
    match key.kind() {
        crate::paper_key::PaperKeyKind::Arxiv if ids.arxiv.is_none() => ids.arxiv = Some(key.value().to_string()),
        crate::paper_key::PaperKeyKind::Doi if ids.doi.is_none() => ids.doi = Some(key.value().to_string()),
        crate::paper_key::PaperKeyKind::Inspire if ids.inspire.is_none() => ids.inspire = Some(key.value().to_string()),
        crate::paper_key::PaperKeyKind::S2 if ids.s2.is_none() => ids.s2 = Some(key.value().to_string()),
        _ => {}
    }
    ids
}

fn refs_share_identity(a: &PaperRef, b: &PaperRef) -> bool {
    paper_ref_matches_key(a, &b.key)
        || paper_ref_matches_key(b, &a.key)
        || (a.ids.arxiv.is_some() && a.ids.arxiv == b.ids.arxiv)
        || (a.ids.doi.is_some() && a.ids.doi == b.ids.doi)
        || (a.ids.inspire.is_some() && a.ids.inspire == b.ids.inspire)
        || (a.ids.s2.is_some() && a.ids.s2 == b.ids.s2)
}

fn merge_origin_aliases(origin: &mut Option<PaperRef>, candidate: &PaperRef) {
    let Some(orig) = origin.as_mut() else { return };
    if !refs_share_identity(orig, candidate) {
        return;
    }
    if orig.ids.arxiv.is_none() { orig.ids.arxiv = candidate.ids.arxiv.clone(); }
    if orig.ids.doi.is_none() { orig.ids.doi = candidate.ids.doi.clone(); }
    if orig.ids.inspire.is_none() { orig.ids.inspire = candidate.ids.inspire.clone(); }
    if orig.ids.s2.is_none() { orig.ids.s2 = candidate.ids.s2.clone(); }
}

fn enrich_origin_from_candidates(p: &Value, origin: &mut Option<PaperRef>) {
    if origin.is_none() { return; }
    let Some(arr) = p.get("candidates").and_then(Value::as_array) else { return };
    for c in arr {
        let Some(key) = c.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) else { continue };
        let raw_ids: PaperIds = c.get("ids").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let pref = PaperRef { ids: normalize_ids_with_key(&key, &raw_ids), key, version: None };
        merge_origin_aliases(origin, &pref);
    }
}

fn enrich_origin_from_lit_fetched(p: &Value, origin: &mut Option<PaperRef>) {
    if origin.is_none() { return; }
    let Some(prov) = p.get("provider").and_then(Value::as_str).and_then(LitProvider::parse) else { return };
    let items = p.get("papers").or_else(|| p.get("hits")).and_then(Value::as_array);
    let Some(arr) = items else { return };
    for item in arr {
        let Some(key) = item.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) else { continue };
        let prov_id = item.get("id").and_then(Value::as_str).map(str::to_string);
        let raw = PaperIds {
            arxiv: item.get("base_id").or_else(|| item.get("arxiv_id")).and_then(Value::as_str).map(str::to_string),
            doi: item.get("doi").and_then(Value::as_str).map(str::to_string),
            inspire: item.get("inspire").and_then(Value::as_str).map(str::to_string).or_else(|| (prov == LitProvider::Inspire).then(|| prov_id.clone()).flatten()),
            s2: item.get("s2").and_then(Value::as_str).map(str::to_string).or_else(|| (prov == LitProvider::SemanticScholar).then(|| prov_id.clone()).flatten()),
        };
        let pref = PaperRef { ids: normalize_ids_with_key(&key, &raw), key, version: None };
        merge_origin_aliases(origin, &pref);
    }
}

fn parse_source_ingested_pages(
    p: &Value,
    map: &mut BTreeMap<String, Vec<PdfPageSpan>>,
    file_hashes: &mut BTreeMap<String, BTreeMap<String, String>>,
) {
    let Some(source_id) = p.get("source_id").and_then(Value::as_str) else { return };
    if let Some(obj) = p.get("files_sha256").and_then(Value::as_object) {
        let mut per_src = BTreeMap::new();
        for (k, v) in obj {
            if let Some(h) = v.as_str() {
                per_src.insert(k.clone(), h.to_string());
            }
        }
        if !per_src.is_empty() {
            file_hashes.insert(source_id.to_string(), per_src);
        }
    }
    let Some(pages) = p.get("pdf_pages").and_then(Value::as_array) else { return };
    let mut spans = Vec::new();
    for page in pages {
        let (Some(first), Some(last), Some(basis)) = (
            page.get("first_line").and_then(Value::as_u64),
            page.get("last_line").and_then(Value::as_u64),
            page.get("basis").and_then(Value::as_str).and_then(TextBasis::parse),
        ) else {
            continue;
        };
        spans.push(PdfPageSpan { first_line: first as usize, last_line: last as usize, basis });
    }
    if !spans.is_empty() {
        map.insert(source_id.to_string(), spans);
    }
}

fn parse_paper_event(
    p: &Value,
    paper_reads: &mut BTreeMap<PaperKey, PaperReadStatus>,
    paper_text_basis: &mut BTreeMap<PaperKey, TextBasis>,
) {
    let Some(key) = p.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) else { return };
    if let Some(st) = p.get("status").and_then(Value::as_str).and_then(PaperReadStatus::parse) {
        paper_reads.insert(key.clone(), st);
    }
    if let Some(tb) = p.get("text_basis").and_then(Value::as_str).and_then(TextBasis::parse) {
        paper_text_basis.insert(key, tb);
    }
}

fn resolve_quote_text_basis(
    p: &Value,
    paper_key: &PaperKey,
    qref: &QuoteRef,
    paper_text_basis: &BTreeMap<PaperKey, TextBasis>,
    source_pdf_pages: &BTreeMap<String, Vec<PdfPageSpan>>,
) -> TextBasis {
    if let Some(tb) = p.get("text_basis").and_then(Value::as_str).and_then(TextBasis::parse) {
        return tb;
    }
    if p.get("ocr").and_then(Value::as_bool) == Some(true)
        || p.get("scanned").and_then(Value::as_bool) == Some(true)
        || p.get("source_kind")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("ocr") || s.eq_ignore_ascii_case("scan"))
    {
        return TextBasis::Ocr;
    }
    if let Some(spans) = source_pdf_pages.get(&qref.source_id) {
        let mut saw_pdf_page = false;
        for sp in spans {
            if sp.first_line <= qref.end_line && sp.last_line >= qref.start_line {
                if sp.basis.is_ocr_source() {
                    return sp.basis;
                }
                saw_pdf_page = true;
            }
        }
        if saw_pdf_page {
            return TextBasis::PdfText;
        }
    }
    paper_text_basis.get(paper_key).copied().unwrap_or_default()
}

fn parse_ft_evidence_event(
    ev: &Event,
    paper_text_basis: &BTreeMap<PaperKey, TextBasis>,
    source_pdf_pages: &BTreeMap<String, Vec<PdfPageSpan>>,
    source_file_hashes: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<FullTextEvidence> {
    if ev.seq == 0 { return None; }
    let p = &ev.payload;
    let paper_key = p.get("paper_key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())?;
    let quote = p.get("quote").and_then(Value::as_str)?.to_string();
    if quote.trim().is_empty() { return None; }
    // Require full F4 QuoteRef provenance with valid line bounds and matching SHA-256 digests (H4, R3b).
    let quote_ref = extract_valid_quote_ref(p, ev.seq, &quote, source_file_hashes)?;
    let excerpt = p.get("excerpt").and_then(Value::as_str).unwrap_or_default().to_string();
    let text_basis = resolve_quote_text_basis(p, &paper_key, &quote_ref, paper_text_basis, source_pdf_pages);
    let subclaim_ids = p
        .get("subclaim_ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let role = p.get("role").and_then(|v| serde_json::from_value(v.clone()).ok());
    Some(FullTextEvidence {
        id: EvidenceId::FullText(ev.seq),
        paper_key,
        quote,
        excerpt,
        quote_ref: Some(quote_ref),
        text_basis,
        subclaim_ids,
        role,
    })
}

fn extract_valid_quote_ref(
    p: &Value,
    seq: u64,
    quote: &str,
    source_file_hashes: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<QuoteRef> {
    let source_id = p.get("source_id")?.as_str()?.to_string();
    let path = p.get("path")?.as_str()?.to_string();
    let start_line = p.get("start_line")?.as_u64()? as usize;
    let end_line = p.get("end_line")?.as_u64()? as usize;
    let file_sha256 = p.get("file_sha256")?.as_str()?.to_string();
    let quote_sha = p.get("quote_sha256")?.as_str()?.to_string();
    if source_id.is_empty()
        || path.split_once('/').map(|(id, _)| id) != Some(source_id.as_str())
        || start_line == 0
        || end_line < start_line
        || !is_sha256_hex(&file_sha256)
        || !is_sha256_hex(&quote_sha)
        || quote_sha != quote_sha256(quote)
    {
        return None;
    }
    if let Some(files) = source_file_hashes.get(&source_id) {
        let rel = path.split_once('/').map(|(_, r)| r).unwrap_or(&path);
        let expected = files.get(&path).or_else(|| files.get(rel));
        if expected.map(String::as_str) != Some(file_sha256.as_str()) {
            return None;
        }
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

fn parse_abs_evidence_event(ev: &Event) -> Option<AbstractEvidence> {
    if ev.seq == 0 { return None; }
    let p = &ev.payload;
    let paper_key = p.get("paper_key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())?;
    let quote = p.get("quote").and_then(Value::as_str)?.to_string();
    if quote.trim().is_empty() { return None; }
    // Explicit `verbatim: false` is rejected; F4 `EvidenceAbstractPayload` omits `verbatim`
    // because only verbatim abstracts may be recorded, so validate all F4 invariants (H4, C3).
    if p.get("verbatim").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let start = p.get("start")?.as_u64()? as usize;
    let end = p.get("end")?.as_u64()? as usize;
    let raw_blob = p.get("raw_blob")?.as_str()?.to_string();
    let quote_sha = p.get("quote_sha256")?.as_str()?;
    let provider_str = p.get("provider")?.as_str()?.trim();
    if start >= end
        || !is_sha256_hex(&raw_blob)
        || !is_sha256_hex(quote_sha)
        || quote_sha != quote_sha256(&quote)
        || provider_str.is_empty()
    {
        return None;
    }
    let provider = LitProvider::parse(provider_str);
    let subclaim_ids = p
        .get("subclaim_ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    Some(AbstractEvidence {
        id: EvidenceId::Abstract(ev.seq),
        paper_key,
        quote,
        verbatim: true,
        raw_blob,
        provider,
        subclaim_ids,
    })
}

fn parse_overlap_value(v: &Value) -> Option<OverlapCandidate> {
    let subclaim_id = v.get("subclaim_id").and_then(Value::as_str)?.to_string();
    let paper_key = v.get("paper_key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())?;
    let relation = v.get("relation").and_then(Value::as_str).and_then(OverlapRelation::parse)?;
    let evidence_ids = v
        .get("evidence")
        .or_else(|| v.get("evidence_ids"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter_map(|s| EvidenceId::parse(s).ok()).collect())
        .unwrap_or_default();
    let difference = v.get("difference").and_then(Value::as_str).unwrap_or_default().to_string();
    let other_name = v
        .get("other_name")
        .and_then(|o| o.as_str().or_else(|| o.get("term").and_then(Value::as_str)))
        .map(str::to_string);
    Some(OverlapCandidate { subclaim_id, paper_key, relation, evidence_ids, difference, other_name })
}

fn build_overlap_signatures(overlaps: &[OverlapCandidate]) -> BTreeMap<String, OverlapSignature> {
    let mut map: BTreeMap<String, OverlapSignature> = BTreeMap::new();
    for o in overlaps {
        let mut eids = o.evidence_ids.clone();
        eids.sort();
        eids.dedup();
        let norm_name = o.other_name.as_deref().map(|s| s.trim().to_lowercase());
        map.entry(o.subclaim_id.clone()).or_default().insert((
            o.paper_key.clone(),
            o.relation,
            eids,
            norm_name,
            o.difference.trim().to_string(),
        ));
    }
    map
}

fn parse_confirmation_note(
    p: &Value,
    current_map_seq: Option<u64>,
    prev_sigs: &BTreeMap<String, OverlapSignature>,
    confirmed: &mut BTreeSet<String>,
) {
    let Some(action) = p.get("action").and_then(Value::as_str) else { return };
    if action != NOTE_ACTION_CONFIRM_SAME && action != NOTE_ACTION_REJECT_SAME {
        return;
    }
    let Some(target) = p.get("subclaim_id").or_else(|| p.get("target")).and_then(Value::as_str)
    else {
        return;
    };
    if let Some(note_map_seq) = p.get("map_seq").and_then(Value::as_u64) {
        if current_map_seq != Some(note_map_seq) {
            return;
        }
    }
    if action == NOTE_ACTION_CONFIRM_SAME {
        if current_map_seq.is_some() && prev_sigs.contains_key(target) {
            confirmed.insert(target.to_string());
        }
    } else {
        confirmed.remove(target);
    }
}
