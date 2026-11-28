//! Citation re-verification, pinned abstract `raw_blob` checking, and read-only library `.bib`
//! resolution for the F15 Explore write-up adapter (`writeup_idea.rs`).
//!
//! Pure local deterministic code: zero model calls, zero network calls, zero disk writes when
//! `library.db` is absent (`00-overview.md` 2.5, 2.11, 2.12, 2.13; `explore.md` C1, C3, C8, C9, S9).

use super::ocr::{
    collect_meta_from_candidates, collect_meta_from_idea_paper, collect_meta_from_ingested,
    collect_meta_from_lit_fetched, detect_ocr_and_page, make_locator, PaperFallbackMeta,
};
use crate::writeup::{BibEntry, RejectedWriteupQuote, WriteupQuote};
use crate::writeup_bib::resolve_bib_entry;
use ledger::{sha256_hex, Event, Ledger};
use research_tools::library::Library;
use research_tools::lit_abstracts::{locate_abstract_quote, locate_abstract_quote_in_blob};
use research_tools::lit_providers::{parse_provider_response, LitProvider, QueryRelation};
use research_tools::paper_key::{split_arxiv_id, PaperIds, PaperKey, PaperKeyKind, PaperRef};
use research_tools::prior_work::TextBasis;
use research_tools::prior_work_wording::provider_display_name;
use research_tools::quote_core::{
    clip_quote, quote_sha256, verify_quote_ref, QuoteFault, QuoteRef, EVIDENCE_ABSTRACT_EVENT,
    EVIDENCE_QUOTE_EVENT,
};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::search_coverage::{IDEA_CANDIDATES_EVENT, IDEA_PAPER_EVENT};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const LIBRARY_DB_NAME: &str = "library.db";

/// Verified full-text quote block plus its resolved OCR status and physical PDF page (if any).
pub(crate) struct ResolvedExploreQuote {
    pub quote: WriteupQuote,
    pub ocr: bool,
    pub page: Option<usize>,
}

/// Verified verbatim abstract quote checked against its pinned `raw_blob` (Rule C3).
pub(crate) struct ResolvedAbstractQuote {
    pub evidence_id: String,
    pub cite_key: String,
    pub provider_label: String,
    pub text: String,
}

/// Open `<data_dir>/library/library.db` ONLY if the file already exists (zero writes if absent).
pub(crate) fn open_existing_library(data_dir: &Path) -> Option<Library> {
    data_dir
        .join("library")
        .join(LIBRARY_DB_NAME)
        .is_file()
        .then(|| Library::open(data_dir).ok())
        .flatten()
}

/// Stateful per-writeup citation resolver that verifies `xe-*` quotes via F4 `verify_quote_ref`,
/// verifies `xa-*` abstracts via pinned `raw_blob`, detects PDF pages & OCR provenance, and
/// deduplicates bibliography entries by canonical [`PaperKey`] in deterministic first-cited order.
pub(crate) struct IdeaCiteResolver<'a> {
    data_dir: &'a Path,
    session_id: &'a str,
    events: &'a [Event],
    sandbox: &'a WorkspaceSandbox,
    lib: Option<&'a Library>,
    ingested_by_source: BTreeMap<String, &'a Event>,
    quote_events: BTreeMap<u64, &'a Event>,
    abstract_events: BTreeMap<u64, &'a Event>,
    paper_text_basis: BTreeMap<PaperKey, TextBasis>,
    paper_meta: BTreeMap<PaperKey, PaperFallbackMeta>,
    bib_by_key: BTreeMap<PaperKey, BibEntry>,
    bib_order: Vec<PaperKey>,
    used_keys: BTreeSet<String>,
    pub rejected_quotes: Vec<RejectedWriteupQuote>,
}

impl<'a> IdeaCiteResolver<'a> {
    pub(crate) fn new(
        data_dir: &'a Path,
        session_id: &'a str,
        events: &'a [Event],
        latest_stmt_seq: u64,
        sandbox: &'a WorkspaceSandbox,
        lib: Option<&'a Library>,
    ) -> Self {
        let mut ingested_by_source = BTreeMap::new();
        let mut quote_events = BTreeMap::new();
        let mut abstract_events = BTreeMap::new();
        let mut paper_text_basis = BTreeMap::new();
        let mut paper_meta = BTreeMap::new();

        for ev in events {
            match ev.kind.as_str() {
                "source.ingested" => {
                    if let Some(sid) = ev.payload.get("source_id").and_then(Value::as_str) {
                        ingested_by_source.insert(sid.to_string(), ev);
                    }
                    collect_meta_from_ingested(&ev.payload, &mut paper_meta);
                }
                "literature.fetched" if ev.seq >= latest_stmt_seq => {
                    collect_meta_from_lit_fetched(&ev.payload, &mut paper_meta);
                }
                IDEA_CANDIDATES_EVENT if ev.seq >= latest_stmt_seq => {
                    collect_meta_from_candidates(&ev.payload, &mut paper_meta);
                }
                IDEA_PAPER_EVENT if ev.seq >= latest_stmt_seq => {
                    collect_meta_from_idea_paper(&ev.payload, &mut paper_text_basis, &mut paper_meta);
                }
                EVIDENCE_QUOTE_EVENT if ev.seq > latest_stmt_seq => {
                    quote_events.insert(ev.seq, ev);
                }
                EVIDENCE_ABSTRACT_EVENT if ev.seq > latest_stmt_seq => {
                    abstract_events.insert(ev.seq, ev);
                }
                _ => {}
            }
        }

        Self {
            data_dir,
            session_id,
            events,
            sandbox,
            lib,
            ingested_by_source,
            quote_events,
            abstract_events,
            paper_text_basis,
            paper_meta,
            bib_by_key: BTreeMap::new(),
            bib_order: Vec::new(),
            used_keys: BTreeSet::new(),
            rejected_quotes: Vec::new(),
        }
    }

    /// Re-verify a stored `evidence.quote` (`xe-<seq>`) via F4 `verify_quote_ref`, clip verified
    /// text to `<= 300` UTF-8 scalar chars, detect PDF page & `(OCR)`, and register its `.bib`
    /// entry. Returns `Ok(None)` (and records a [`RejectedWriteupQuote`]) if verification fails.
    pub(crate) fn resolve_full_text_quote(
        &mut self,
        section_title: &str,
        expected_paper: &PaperKey,
        seq: u64,
    ) -> Result<Option<ResolvedExploreQuote>, String> {
        let Some(eq_ev) = self.quote_events.get(&seq).copied() else {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "missing_evidence_event",
                format!("`evidence.quote` event seq {seq} not found in active revision"),
            );
            return Ok(None);
        };
        let p = &eq_ev.payload;
        let Some(raw_quote) = p.get("quote").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
        else {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "empty_quote",
                format!("`evidence.quote` seq {seq} has empty quote text"),
            );
            return Ok(None);
        };
        let Some(stored_ref) = extract_quote_ref(p, seq) else {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "malformed_quote_ref",
                format!("`evidence.quote` seq {seq} has malformed QuoteRef fields"),
            );
            return Ok(None);
        };

        if quote_sha256(raw_quote) != stored_ref.quote_sha256 {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "quote_hash_mismatch",
                format!("`evidence.quote` seq {seq} quote text does not match stored quote_sha256"),
            );
            return Ok(None);
        }

        let clipped = clip_quote(raw_quote);
        let verify_ref = QuoteRef {
            quote_sha256: quote_sha256(clipped),
            ..stored_ref
        };
        let verified = match verify_quote_ref(
            self.sandbox,
            self.session_id,
            self.events,
            &verify_ref,
            clipped,
        ) {
            Ok(v) => v,
            Err(fault) => {
                self.rejected_quotes.push(RejectedWriteupQuote {
                    section_title: section_title.to_string(),
                    cite_key: expected_paper.to_string(),
                    fault,
                });
                return Ok(None);
            }
        };
        let final_qref = QuoteRef {
            quote_sha256: quote_sha256(clipped),
            ..verified.quote_ref
        };

        let ingested_ev = self.ingested_by_source.get(&final_qref.source_id).copied();
        let (paper_ref, cite_key) = self.register_paper(expected_paper, ingested_ev)?;
        let paper_ocr = self
            .paper_text_basis
            .get(&paper_ref.key)
            .or_else(|| self.paper_text_basis.get(expected_paper))
            .is_some_and(|tb| tb.is_ocr_source());
        let (ocr, page) = detect_ocr_and_page(
            paper_ocr,
            p,
            ingested_ev,
            final_qref.start_line,
            final_qref.end_line,
        );
        let locator = make_locator(page, final_qref.start_line, final_qref.end_line);

        Ok(Some(ResolvedExploreQuote {
            quote: WriteupQuote {
                text: clipped.to_string(),
                cite_key,
                paper: paper_ref,
                quote_ref: final_qref,
                locator,
                ocr,
            },
            ocr,
            page,
        }))
    }

    /// Re-verify a stored `evidence.abstract` (`xa-<seq>`) against its pinned `raw_blob` (Rule C3),
    /// rejecting non-verbatim (rebuilt) abstracts or tampered blobs, clipping to `<= 300` UTF-8
    /// chars, and registering its `.bib` entry.
    pub(crate) fn resolve_abstract_quote(
        &mut self,
        section_title: &str,
        expected_paper: &PaperKey,
        seq: u64,
    ) -> Result<Option<ResolvedAbstractQuote>, String> {
        let Some(ev) = self.abstract_events.get(&seq).copied() else {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "missing_abstract_event",
                format!("`evidence.abstract` event seq {seq} not found in active revision"),
            );
            return Ok(None);
        };
        let p = &ev.payload;
        if p.get("verbatim").and_then(Value::as_bool) == Some(false) {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "non_verbatim_abstract",
                format!("`evidence.abstract` seq {seq} is non-verbatim (Rule C3)"),
            );
            return Ok(None);
        }
        let (Some(raw_quote), Some(qsha), Some(raw_blob), Some(provider_str), Some(start), Some(end)) = (
            p.get("quote").and_then(Value::as_str).filter(|s| !s.trim().is_empty()),
            p.get("quote_sha256").and_then(Value::as_str),
            p.get("raw_blob").and_then(Value::as_str),
            p.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()),
            p.get("start").and_then(Value::as_u64),
            p.get("end").and_then(Value::as_u64),
        ) else {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "malformed_abstract_evidence",
                format!("`evidence.abstract` seq {seq} is missing required fields"),
            );
            return Ok(None);
        };
        if start >= end
            || !is_sha256_hex(raw_blob)
            || !is_sha256_hex(qsha)
            || qsha != quote_sha256(raw_quote)
        {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "abstract_hash_mismatch",
                format!("`evidence.abstract` seq {seq} failed digest or offset validation"),
            );
            return Ok(None);
        }

        let raw_bytes = match Ledger::open_existing(self.data_dir).and_then(|l| l.get_blob(raw_blob))
        {
            Ok(b) if sha256_hex(&b) == raw_blob => b,
            _ => {
                self.record_rejection(
                    section_title,
                    expected_paper.as_str(),
                    "missing_or_corrupt_raw_blob",
                    format!("pinned `raw_blob` `{raw_blob}` for `xa-{seq}` is missing or corrupted"),
                );
                return Ok(None);
            }
        };

        let aliases: Vec<PaperKey> = self
            .paper_meta
            .get(expected_paper)
            .map(|m| m.alias_keys.clone())
            .unwrap_or_default();
        let Some(matched_text) = verify_abstract_in_raw_blob(
            provider_str,
            &raw_bytes,
            expected_paper,
            &aliases,
            raw_quote,
        ) else {
            self.record_rejection(
                section_title,
                expected_paper.as_str(),
                "abstract_not_in_blob",
                format!("`xa-{seq}` quote did not match verbatim abstract in pinned `raw_blob` (Rule C3)"),
            );
            return Ok(None);
        };

        let (_, cite_key) = self.register_paper(expected_paper, None)?;
        let provider_label = LitProvider::parse(provider_str)
            .map(provider_display_name)
            .unwrap_or(provider_str)
            .to_string();
        Ok(Some(ResolvedAbstractQuote {
            evidence_id: format!("xa-{seq}"),
            cite_key,
            provider_label,
            text: clip_quote(matched_text.trim()).to_string(),
        }))
    }

    /// Return the already-registered citekey for `paper_key`, if cited by a verified quote.
    pub(crate) fn existing_cite_key(&self, paper_key: &PaperKey) -> Option<&str> {
        self.bib_by_key.get(paper_key).map(|e| e.cite_key.as_str())
    }

    /// Return deduplicated bibliography entries in deterministic first-cited order.
    pub(crate) fn into_bibliography(mut self) -> Vec<BibEntry> {
        let mut out = Vec::with_capacity(self.bib_order.len());
        for key in self.bib_order {
            if let Some(entry) = self.bib_by_key.remove(&key) {
                out.push(entry);
            }
        }
        out
    }

    fn register_paper(
        &mut self,
        candidate_key: &PaperKey,
        ingested_ev: Option<&Event>,
    ) -> Result<(PaperRef, String), String> {
        let meta = self.paper_meta.get(candidate_key).cloned().unwrap_or_default();
        let version = meta.version.clone().or_else(|| {
            ingested_ev
                .and_then(|ev| ev.payload.get("arxiv_cover_id").and_then(Value::as_str))
                .and_then(|cid| split_arxiv_id(cid).ok())
                .and_then(|(_, v)| v)
        });
        let paper_ref = build_paper_ref(self.lib, candidate_key, &meta.ids, version)?;
        let canonical = paper_ref.key.clone();

        if let Some(existing) = self.bib_by_key.get(&canonical) {
            return Ok((existing.paper.clone(), existing.cite_key.clone()));
        }

        let fallback_title = if !meta.title.trim().is_empty() {
            meta.title.trim().to_string()
        } else {
            ingested_ev
                .and_then(|ev| ev.payload.get("title").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(canonical.as_str())
                .to_string()
        };

        let entry = resolve_bib_entry(
            self.lib,
            &paper_ref,
            &fallback_title,
            &meta.authors,
            meta.year,
            &mut self.used_keys,
        )?;
        let cite_key = entry.cite_key.clone();
        let resolved_paper = entry.paper.clone();
        self.bib_order.push(canonical.clone());
        self.bib_by_key.insert(canonical, entry);
        Ok((resolved_paper, cite_key))
    }

    fn record_rejection(&mut self, section: &str, cite_key: &str, code: &str, message: String) {
        self.rejected_quotes.push(RejectedWriteupQuote {
            section_title: section.to_string(),
            cite_key: cite_key.to_string(),
            fault: QuoteFault {
                code: code.to_string(),
                message,
                start_line: None,
                end_line: None,
                true_text: None,
            },
        });
    }
}

fn verify_abstract_in_raw_blob(
    provider_str: &str,
    raw_bytes: &[u8],
    paper_key: &PaperKey,
    alias_keys: &[PaperKey],
    quote: &str,
) -> Option<String> {
    if let Some(prov) = LitProvider::parse(provider_str) {
        if parse_provider_response(prov, raw_bytes, &QueryRelation::None, 0, None).is_ok() {
            return std::iter::once(paper_key)
                .chain(alias_keys.iter())
                .find_map(|k| locate_abstract_quote_in_blob(prov, raw_bytes, k, quote).ok())
                .map(|m| m.matched_text);
        }
    }
    let text = String::from_utf8_lossy(raw_bytes);
    locate_abstract_quote(&text, quote, true).ok().map(|m| m.matched_text)
}

fn extract_quote_ref(p: &Value, seq: u64) -> Option<QuoteRef> {
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

fn build_paper_ref(
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
        PaperKeyKind::Arxiv => { if ids.arxiv.is_none() { ids.arxiv = Some(candidate.value().to_string()); } None }
        PaperKeyKind::Doi => { if ids.doi.is_none() { ids.doi = Some(candidate.value().to_string()); } None }
        PaperKeyKind::Inspire => { if ids.inspire.is_none() { ids.inspire = Some(candidate.value().to_string()); } None }
        PaperKeyKind::S2 => { if ids.s2.is_none() { ids.s2 = Some(candidate.value().to_string()); } None }
        PaperKeyKind::Openalex | PaperKeyKind::File => Some(candidate.clone()),
    };
    PaperRef::from_ids_or(ids, version, fallback)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Format an abstract-evidence prose block strictly capped at `"abstract only"` and
/// `"possible relevance"` (`00-overview.md` 2.9, `explore.md` C3 & V2).
pub(crate) fn format_verified_abstract_line(abs: &ResolvedAbstractQuote) -> String {
    format!(
        "Recorded verbatim abstract ({}, [@{}], {}, {}): \"{}\" — {} only (full text not read).",
        abs.evidence_id,
        abs.cite_key,
        abs.provider_label,
        research_tools::prior_work_wording::wording::GRADE_ABSTRACT_ONLY,
        abs.text,
        research_tools::prior_work_wording::wording::ABSTRACT_POSSIBLE_RELEVANCE,
    )
}
