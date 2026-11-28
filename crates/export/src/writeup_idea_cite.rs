//! Citation re-verification, strict Rule-C3 abstract `raw_blob` checking, and read-only library
//! `.bib` resolution for the F15 Explore write-up adapter (`writeup_idea.rs`).
//!
//! Pure local deterministic code: zero model calls, zero network calls, zero disk writes when
//! `library.db` is absent (`00-overview.md` 2.5, 2.11, 2.12, 2.13; `explore.md` C1, C3, C8, C9, S9).

use super::ocr::{
    build_paper_ref, collect_meta_from_candidates, collect_meta_from_idea_paper,
    collect_meta_from_ingested, collect_meta_from_lit_fetched, detect_ocr_and_page,
    extract_quote_ref, full_quote_present_in_verified_file, is_sha256_hex, make_locator,
    PaperFallbackMeta,
};
use crate::writeup::{writeup_wording, BibEntry, QuoteLocator, RejectedWriteupQuote, WriteupQuote};
use crate::writeup_bib::resolve_bib_entry;
use ledger::{sha256_hex, Event, Ledger};
use research_tools::library::Library;
use research_tools::lit_abstracts::extract_abstract_from_blob;
use research_tools::lit_providers::LitProvider;
use research_tools::paper_key::{split_arxiv_id, PaperKey, PaperRef};
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

/// Pre-verified full-text quote waiting to be committed to the bibliography if its overlap survives.
#[derive(Debug, Clone)]
pub(crate) struct VerifiedFullTextData {
    pub paper_key: PaperKey,
    pub clipped_text: String,
    pub quote_ref: QuoteRef,
    pub locator: QuoteLocator,
    pub ocr: bool,
    pub page: Option<usize>,
}

/// Pre-verified verbatim abstract quote checked strictly against its pinned `raw_blob` (Rule C3).
#[derive(Debug, Clone)]
pub(crate) struct VerifiedAbstractData {
    pub evidence_id: String,
    pub paper_key: PaperKey,
    pub provider_label: String,
    pub text: String,
}

/// Committed full-text quote block plus its resolved OCR status and physical PDF page (if any).
pub(crate) struct ResolvedExploreQuote {
    pub quote: WriteupQuote,
    pub ocr: bool,
    pub page: Option<usize>,
}

/// Committed abstract quote block ready for `WriteupBlock::AbstractQuote`.
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

/// Stateful per-writeup citation resolver that pre-verifies `xe-*` quotes via F4 `verify_quote_ref`
/// (with paper/source ownership & >300-char tail checks) and `xa-*` abstracts strictly via F11
/// `extract_abstract_from_blob` + offset validation, detects PDF pages & OCR provenance, and
/// deduplicates bibliography entries by canonical [`PaperKey`] in deterministic first-cited order.
pub(crate) struct IdeaCiteResolver<'a> {
    session_id: &'a str,
    events: &'a [Event],
    sandbox: &'a WorkspaceSandbox,
    ledger: Option<Ledger>,
    lib: Option<&'a Library>,
    ingested_by_source: BTreeMap<String, &'a Event>,
    source_origin_keys: BTreeMap<String, PaperKey>,
    paper_source_ids: BTreeMap<PaperKey, BTreeSet<String>>,
    quote_events: BTreeMap<u64, &'a Event>,
    abstract_events: BTreeMap<u64, &'a Event>,
    paper_text_basis: BTreeMap<PaperKey, TextBasis>,
    paper_meta: BTreeMap<PaperKey, PaperFallbackMeta>,
    bib_by_key: BTreeMap<PaperKey, BibEntry>,
    key_aliases: BTreeMap<PaperKey, PaperKey>,
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
        let mut source_origin_keys = BTreeMap::new();
        let mut paper_source_ids = BTreeMap::new();
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
                    collect_meta_from_ingested(&ev.payload, &mut source_origin_keys, &mut paper_meta);
                }
                "literature.fetched" if ev.seq >= latest_stmt_seq => {
                    collect_meta_from_lit_fetched(&ev.payload, &mut paper_meta);
                }
                IDEA_CANDIDATES_EVENT if ev.seq >= latest_stmt_seq => {
                    collect_meta_from_candidates(&ev.payload, &mut paper_meta);
                }
                IDEA_PAPER_EVENT if ev.seq >= latest_stmt_seq => {
                    collect_meta_from_idea_paper(
                        &ev.payload,
                        &mut paper_text_basis,
                        &mut paper_source_ids,
                        &mut paper_meta,
                    );
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
            session_id,
            events,
            sandbox,
            ledger: Ledger::open_existing(data_dir).ok(),
            lib,
            ingested_by_source,
            source_origin_keys,
            paper_source_ids,
            quote_events,
            abstract_events,
            paper_text_basis,
            paper_meta,
            bib_by_key: BTreeMap::new(),
            key_aliases: BTreeMap::new(),
            bib_order: Vec::new(),
            used_keys: BTreeSet::new(),
            rejected_quotes: Vec::new(),
        }
    }

    /// Pre-verify an `evidence.quote` (`xe-<seq>`) without registering `.bib` yet (C2/H1).
    pub(crate) fn verify_full_text_quote(
        &mut self,
        sec: &str,
        expected: &PaperKey,
        seq: u64,
    ) -> Option<VerifiedFullTextData> {
        let Some(eq_ev) = self.quote_events.get(&seq).copied() else {
            return self.reject(sec, expected, "missing_evidence_event", format!("`evidence.quote` seq {seq} not found"));
        };
        let p = &eq_ev.payload;
        if !self.event_paper_matches(p, expected) {
            return self.reject(sec, expected, "evidence_paper_mismatch", format!("`evidence.quote` seq {seq} paper_key mismatch"));
        }
        let Some(raw_quote) = p.get("quote").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
        else {
            return self.reject(sec, expected, "empty_quote", format!("`evidence.quote` seq {seq} is empty"));
        };
        let Some(stored_ref) = extract_quote_ref(p, seq) else {
            return self.reject(sec, expected, "malformed_quote_ref", format!("`evidence.quote` seq {seq} has malformed QuoteRef"));
        };
        if !self.source_belongs_to_paper(&stored_ref.source_id, expected) {
            return self.reject(
                sec,
                expected,
                "source_paper_mismatch",
                format!("`evidence.quote` seq {seq} source `{}` does not belong to `{expected}`", stored_ref.source_id),
            );
        }
        if quote_sha256(raw_quote) != stored_ref.quote_sha256 {
            return self.reject(sec, expected, "quote_hash_mismatch", format!("`evidence.quote` seq {seq} quote_sha256 mismatch"));
        }

        let clipped = clip_quote(raw_quote);
        let verify_ref = QuoteRef { quote_sha256: quote_sha256(clipped), ..stored_ref };
        let verified = match verify_quote_ref(self.sandbox, self.session_id, self.events, &verify_ref, clipped) {
            Ok(v) => v,
            Err(fault) => {
                self.rejected_quotes.push(RejectedWriteupQuote {
                    section_title: sec.to_string(),
                    cite_key: expected.to_string(),
                    fault,
                });
                return None;
            }
        };

        // Audit H1: verify unclipped quote tail against the hash-verified physical source file.
        if raw_quote.chars().count() > writeup_wording::MAX_QUOTE_CHARS
            && !full_quote_present_in_verified_file(self.sandbox, self.session_id, &verified.quote_ref, raw_quote)
        {
            return self.reject(
                sec,
                expected,
                "quote_tail_unverified",
                format!("`evidence.quote` seq {seq} text beyond 300 chars did not match pinned source"),
            );
        }

        let final_qref = QuoteRef { quote_sha256: quote_sha256(clipped), ..verified.quote_ref };
        let ingested_ev = self.ingested_by_source.get(&final_qref.source_id).copied();
        let paper_ocr = self.is_paper_ocr_basis(expected);
        let (ocr, page) = detect_ocr_and_page(paper_ocr, p, ingested_ev, final_qref.start_line, final_qref.end_line);
        let locator = make_locator(page, final_qref.start_line, final_qref.end_line);
        Some(VerifiedFullTextData {
            paper_key: expected.clone(),
            clipped_text: clipped.to_string(),
            quote_ref: final_qref,
            locator,
            ocr,
            page,
        })
    }

    /// Commit a pre-verified full-text quote, registering its paper in the bibliography.
    pub(crate) fn commit_full_text_quote(
        &mut self,
        data: VerifiedFullTextData,
    ) -> Result<ResolvedExploreQuote, String> {
        let ingested_ev = self.ingested_by_source.get(&data.quote_ref.source_id).copied();
        let (paper_ref, cite_key) = self.register_paper(&data.paper_key, ingested_ev)?;
        Ok(ResolvedExploreQuote {
            quote: WriteupQuote {
                text: data.clipped_text,
                cite_key,
                paper: paper_ref,
                quote_ref: data.quote_ref,
                locator: data.locator,
                ocr: data.ocr,
            },
            ocr: data.ocr,
            page: data.page,
        })
    }

    /// Pre-verify an `evidence.abstract` (`xa-<seq>`) strictly against its pinned `raw_blob`
    /// using F11 `extract_abstract_from_blob` (Rule C3; no raw-byte fallback; exact `start..end`
    /// slice & `paper_key` verification; Audit C1/H2).
    pub(crate) fn verify_abstract_quote(
        &mut self,
        sec: &str,
        expected: &PaperKey,
        seq: u64,
    ) -> Option<VerifiedAbstractData> {
        let Some(ev) = self.abstract_events.get(&seq).copied() else {
            return self.reject(sec, expected, "missing_abstract_event", format!("`evidence.abstract` seq {seq} not found"));
        };
        let p = &ev.payload;
        if !self.event_paper_matches(p, expected) {
            return self.reject(sec, expected, "abstract_paper_mismatch", format!("`evidence.abstract` seq {seq} paper_key mismatch"));
        }
        if p.get("verbatim").and_then(Value::as_bool) == Some(false) {
            return self.reject(sec, expected, "non_verbatim_abstract", format!("`xa-{seq}` is non-verbatim (Rule C3)"));
        }
        let (Some(raw_quote), Some(qsha), Some(raw_blob), Some(provider_str), Some(start), Some(end)) = (
            p.get("quote").and_then(Value::as_str).filter(|s| !s.trim().is_empty()),
            p.get("quote_sha256").and_then(Value::as_str),
            p.get("raw_blob").and_then(Value::as_str),
            p.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()),
            p.get("start").and_then(Value::as_u64).map(|n| n as usize),
            p.get("end").and_then(Value::as_u64).map(|n| n as usize),
        ) else {
            return self.reject(sec, expected, "malformed_abstract_evidence", format!("`xa-{seq}` is missing required fields"));
        };
        if start >= end || !is_sha256_hex(raw_blob) || !is_sha256_hex(qsha) || qsha != quote_sha256(raw_quote) {
            return self.reject(sec, expected, "abstract_hash_mismatch", format!("`xa-{seq}` failed digest or offset check"));
        }
        let Some(prov) = LitProvider::parse(provider_str) else {
            return self.reject(sec, expected, "unsupported_abstract_provider", format!("`xa-{seq}` provider `{provider_str}` unsupported"));
        };
        let raw_bytes = match self.ledger.as_ref().and_then(|l| l.get_blob(raw_blob).ok()) {
            Some(b) if sha256_hex(&b) == raw_blob => b,
            _ => {
                return self.reject(sec, expected, "missing_or_corrupt_raw_blob", format!("pinned `raw_blob` `{raw_blob}` for `xa-{seq}` missing/corrupt"));
            }
        };

        let aliases = self.all_paper_keys(expected);
        let extracted = aliases
            .iter()
            .find_map(|k| extract_abstract_from_blob(prov, &raw_bytes, k).ok());
        let Some(ext) = extracted.filter(|e| e.verbatim) else {
            return self.reject(sec, expected, "abstract_not_in_blob", format!("`xa-{seq}` did not resolve to a verbatim provider abstract (Rule C3)"));
        };
        let Some(slice) = ext.text.get(start..end) else {
            return self.reject(sec, expected, "abstract_offset_out_of_bounds", format!("`xa-{seq}` offsets {start}..{end} out of bounds"));
        };
        if quote_sha256(slice) != qsha {
            return self.reject(sec, expected, "abstract_slice_mismatch", format!("`xa-{seq}` slice at {start}..{end} does not match quote_sha256"));
        }

        Some(VerifiedAbstractData {
            evidence_id: format!("xa-{seq}"),
            paper_key: expected.clone(),
            provider_label: provider_display_name(prov).to_string(),
            text: clip_quote(slice.trim()).to_string(),
        })
    }

    /// Commit a pre-verified abstract quote, registering its paper in the bibliography.
    pub(crate) fn commit_abstract_quote(
        &mut self,
        data: VerifiedAbstractData,
    ) -> Result<ResolvedAbstractQuote, String> {
        let (_, cite_key) = self.register_paper(&data.paper_key, None)?;
        Ok(ResolvedAbstractQuote {
            evidence_id: data.evidence_id,
            cite_key,
            provider_label: data.provider_label,
            text: data.text,
        })
    }

    /// Return the already-registered citekey for `paper_key` (resolving aliases, Audit M2).
    pub(crate) fn existing_cite_key(&self, paper_key: &PaperKey) -> Option<&str> {
        if let Some(canonical) = self.key_aliases.get(paper_key) {
            if let Some(entry) = self.bib_by_key.get(canonical) {
                return Some(entry.cite_key.as_str());
            }
        }
        if let Some(entry) = self.bib_by_key.get(paper_key) {
            return Some(entry.cite_key.as_str());
        }
        for k in self.all_paper_keys(paper_key) {
            let target = self.key_aliases.get(&k).unwrap_or(&k);
            if let Some(entry) = self.bib_by_key.get(target) {
                return Some(entry.cite_key.as_str());
            }
        }
        None
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

    fn all_paper_keys(&self, key: &PaperKey) -> Vec<PaperKey> {
        let mut out = vec![key.clone()];
        if let Some(m) = self.paper_meta.get(key) {
            for ak in &m.alias_keys {
                if !out.contains(ak) {
                    out.push(ak.clone());
                }
            }
        }
        for (k, m) in &self.paper_meta {
            if m.alias_keys.contains(key) {
                if !out.contains(k) {
                    out.push(k.clone());
                }
                for ak in &m.alias_keys {
                    if !out.contains(ak) {
                        out.push(ak.clone());
                    }
                }
            }
        }
        out
    }

    fn lookup_meta(&self, key: &PaperKey) -> PaperFallbackMeta {
        if let Some(m) = self.paper_meta.get(key) {
            return m.clone();
        }
        self.paper_meta
            .values()
            .find(|m| m.alias_keys.contains(key))
            .cloned()
            .unwrap_or_default()
    }

    fn keys_equivalent(&self, a: &PaperKey, b: &PaperKey) -> bool {
        a == b || self.all_paper_keys(a).contains(b)
    }

    fn event_paper_matches(&self, p: &Value, expected: &PaperKey) -> bool {
        p.get("paper_key")
            .and_then(Value::as_str)
            .and_then(|s| PaperKey::parse(s).ok())
            .is_some_and(|pk| self.keys_equivalent(&pk, expected))
    }

    fn source_belongs_to_paper(&self, source_id: &str, expected: &PaperKey) -> bool {
        if let Some(origin_key) = self.source_origin_keys.get(source_id) {
            if !self.keys_equivalent(origin_key, expected) {
                return false;
            }
        }
        let mut expected_sources: BTreeSet<&str> = BTreeSet::new();
        for k in self.all_paper_keys(expected) {
            if let Some(set) = self.paper_source_ids.get(&k) {
                expected_sources.extend(set.iter().map(String::as_str));
            }
        }
        expected_sources.is_empty() || expected_sources.contains(source_id)
    }

    fn is_paper_ocr_basis(&self, key: &PaperKey) -> bool {
        self.all_paper_keys(key)
            .iter()
            .any(|k| self.paper_text_basis.get(k).is_some_and(|tb| tb.is_ocr_source()))
    }

    fn register_paper(
        &mut self,
        candidate_key: &PaperKey,
        ingested_ev: Option<&Event>,
    ) -> Result<(PaperRef, String), String> {
        let meta = self.lookup_meta(candidate_key);
        let version = meta.version.clone().or_else(|| {
            ingested_ev
                .and_then(|ev| ev.payload.get("arxiv_cover_id").and_then(Value::as_str))
                .and_then(|cid| split_arxiv_id(cid).ok())
                .and_then(|(_, v)| v)
        });
        let paper_ref = build_paper_ref(self.lib, candidate_key, &meta.ids, version)?;
        let canonical = paper_ref.key.clone();
        self.key_aliases.insert(candidate_key.clone(), canonical.clone());
        for ak in self.all_paper_keys(candidate_key) {
            self.key_aliases.insert(ak, canonical.clone());
        }

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

    fn reject<T>(&mut self, sec: &str, paper: &PaperKey, code: &str, message: String) -> Option<T> {
        self.rejected_quotes.push(RejectedWriteupQuote {
            section_title: sec.to_string(),
            cite_key: paper.to_string(),
            fault: QuoteFault {
                code: code.to_string(),
                message,
                start_line: None,
                end_line: None,
                true_text: None,
            },
        });
        None
    }
}
