// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! PDF quote-matching normalization (ignoring ligatures, soft hyphens, and line-end hyphenation
//! without mutating stored text) and matching first-page metadata candidates to F2 [`PaperKey`] /
//! [`PaperRef`] offline or via an injectable [`MetadataFetcher`] (package F14a, 00-overview.md 2.12).

use crate::http_client::{fetch_response_with_gate, MAX_METADATA_BYTES};
use crate::net_rate::{NetRateGate, Service};
use crate::paper_fetch::{fetch_with_gate, FetchOutcome, FetchWant};
use crate::paper_key::{canonical_doi, PaperIds, PaperKey, PaperRef};
use crate::pdf_firstpage::{fold_diacritics, normalize_pdf_unicode, FirstPageCandidates};
use crate::pdf_pages::{PageBasis, PageMap, PAGE_BREAK_CHAR};
use crate::ResearchEnv;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Part 1: PDF-normalized quote matching (stored text is never altered)
// ---------------------------------------------------------------------------

/// Result of matching a quote against a PDF reading file without mutating the stored text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfQuoteMatch {
    /// Byte span in the original stored reading file.
    pub start_byte: usize,
    pub end_byte: usize,
    /// 1-based line span in the original stored reading file.
    pub start_line: usize,
    pub end_line: usize,
    /// 1-based physical page numbers looked up from [`PageMap`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_page: Option<usize>,
    /// Quality basis of `start_page` (lets callers tag `scan_text_layer` matches as OCR).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_page_basis: Option<PageBasis>,
    /// Verbatim slice from the stored reading file (never rewritten).
    pub stored_excerpt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEndHyphenMode {
    Drop,
    Keep,
}

struct IndexedNorm {
    text: String,
    raw_starts: Vec<usize>,
    raw_ends: Vec<usize>,
}

fn is_hyphen_char(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}')
}

fn normalize_indexed(input: &str, hyphen_mode: LineEndHyphenMode) -> IndexedNorm {
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut text = String::with_capacity(input.len());
    let mut raw_starts = Vec::with_capacity(input.len());
    let mut raw_ends = Vec::with_capacity(input.len());
    let mut prev_ws = true;
    let mut prev_alpha = false;
    let mut i = 0usize;

    let push_piece = |piece: &str,
                      rs: usize,
                      re: usize,
                      text: &mut String,
                      starts: &mut Vec<usize>,
                      ends: &mut Vec<usize>| {
        text.push_str(piece);
        for _ in piece.bytes() {
            starts.push(rs);
            ends.push(re);
        }
    };

    while i < chars.len() {
        let (byte_pos, ch) = chars[i];
        let byte_end = byte_pos + ch.len_utf8();

        if ch == '\u{00AD}' || ch == '\0' {
            i += 1;
            continue;
        }

        // Handle line-end hyphenation: `<alpha>-[spaces]\n[spaces]<alpha>`.
        if prev_alpha && is_hyphen_char(ch) {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, ' ' | '\t' | '\r') {
                j += 1;
            }
            if j < chars.len() && chars[j].1 == '\n' {
                j += 1;
                while j < chars.len() && matches!(chars[j].1, ' ' | '\t' | '\r') {
                    j += 1;
                }
                if j < chars.len() && chars[j].1.is_alphabetic() {
                    if hyphen_mode == LineEndHyphenMode::Keep {
                        push_piece("-", byte_pos, byte_end, &mut text, &mut raw_starts, &mut raw_ends);
                        prev_ws = false;
                        prev_alpha = false;
                    }
                    i = j;
                    continue;
                }
            }
        }

        if ch == PAGE_BREAK_CHAR || ch.is_whitespace() {
            if !prev_ws {
                push_piece(" ", byte_pos, byte_end, &mut text, &mut raw_starts, &mut raw_ends);
                prev_ws = true;
            }
            prev_alpha = false;
            i += 1;
            continue;
        }

        let mapped = map_match_char(ch);
        for mc in mapped.chars() {
            if mc.is_whitespace() {
                if !prev_ws {
                    push_piece(" ", byte_pos, byte_end, &mut text, &mut raw_starts, &mut raw_ends);
                    prev_ws = true;
                }
                prev_alpha = false;
            } else {
                let mut buf = [0u8; 4];
                let s = mc.encode_utf8(&mut buf);
                push_piece(s, byte_pos, byte_end, &mut text, &mut raw_starts, &mut raw_ends);
                prev_ws = false;
                prev_alpha = mc.is_alphabetic();
            }
        }
        i += 1;
    }

    while text.ends_with(' ') {
        text.pop();
        raw_starts.pop();
        raw_ends.pop();
    }

    IndexedNorm {
        text,
        raw_starts,
        raw_ends,
    }
}

fn map_match_char(c: char) -> String {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' => "'".into(),
        '\u{201C}' | '\u{201D}' | '\u{201E}' => "\"".into(),
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2212}' => "-".into(),
        _ => {
            let mut s = [0u8; 4];
            normalize_pdf_unicode(c.encode_utf8(&mut s))
        }
    }
}

/// Normalize text for PDF quote comparison: expands ligatures/PUA, strips soft hyphens,
/// joins line-end hyphenated words, normalizes typographic quotes/dashes, and collapses whitespace.
pub fn normalize_for_pdf_match(text: &str) -> String {
    normalize_indexed(text, LineEndHyphenMode::Drop).text
}

fn line_number_at_byte(text: &str, byte_offset: usize) -> usize {
    let clamped = byte_offset.min(text.len());
    text[..clamped].bytes().filter(|&b| b == b'\n').count() + 1
}

/// Match `query` against `stored_text` while ignoring PDF extraction artifacts (ligatures,
/// soft hyphens, line-end hyphens, whitespace wrapping). Never modifies `stored_text`; returns
/// the exact byte/line span, [`PageMap`] physical page, and verbatim stored excerpt.
pub fn match_pdf_quote(
    stored_text: &str,
    page_map: &PageMap,
    query: &str,
) -> Option<PdfQuoteMatch> {
    if query.trim().is_empty() || stored_text.is_empty() {
        return None;
    }
    for mode in [LineEndHyphenMode::Drop, LineEndHyphenMode::Keep] {
        let needle = normalize_indexed(query, mode).text;
        if needle.is_empty() {
            continue;
        }
        let haystack = normalize_indexed(stored_text, mode);
        if let Some(pos) = haystack.text.find(&needle) {
            let end = pos + needle.len();
            let start_byte = haystack.raw_starts[pos];
            let end_byte = haystack.raw_ends[end - 1];
            let start_line = line_number_at_byte(stored_text, start_byte);
            let last_char_byte = end_byte.saturating_sub(1).max(start_byte);
            let end_line = line_number_at_byte(stored_text, last_char_byte);
            let start_rec = page_map.record_of_line(start_line);
            let end_page = page_map.page_of(end_line);
            return Some(PdfQuoteMatch {
                start_byte,
                end_byte,
                start_line,
                end_line,
                start_page: start_rec.map(|r| r.page),
                end_page,
                start_page_basis: start_rec.map(|r| r.basis),
                stored_excerpt: stored_text[start_byte..end_byte].to_string(),
            });
        }
    }
    None
}

/// Check whether `query` appears inside `stored_slice` under PDF-normalized matching.
pub fn quotes_match_pdf(stored_slice: &str, query: &str) -> bool {
    if query.trim().is_empty() {
        return false;
    }
    for mode in [LineEndHyphenMode::Drop, LineEndHyphenMode::Keep] {
        let needle = normalize_indexed(query, mode).text;
        if needle.is_empty() {
            continue;
        }
        let haystack = normalize_indexed(stored_slice, mode).text;
        if haystack.contains(&needle) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Part 2: Matching first-page candidates to F2 PaperKey (offline + fetcher)
// ---------------------------------------------------------------------------

/// How a candidate PDF was matched to a [`PaperRef`] / [`PaperKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchBasis {
    ArxivCandidate,
    DoiCandidate,
    FileBytesHash,
    RemoteIdentifierLookup,
    RemoteExactTitleAuthor,
}

/// Match result binding a PDF's candidates to an F2 [`PaperRef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperKeyMatch {
    pub paper: PaperRef,
    pub basis: MatchBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_title: Option<String>,
    /// Always `false` for heuristic / lookup candidates until explicitly confirmed.
    pub confirmed: bool,
}

/// Match first-page metadata candidates to an F2 [`PaperRef`] without any network call.
///
/// Preference order follows `00-overview.md` section 2.3:
/// 1. arXiv id candidate (`arxiv:<base>`), preserving any discovered DOI in `ids.doi`
/// 2. DOI candidate (`doi:<norm>`)
/// 3. SHA-256 of `raw_pdf_bytes` (`file:<16-hex>`) when bytes are provided
pub fn match_candidates_offline(
    candidates: &FirstPageCandidates,
    raw_pdf_bytes: Option<&[u8]>,
) -> Option<PaperKeyMatch> {
    let first_doi = candidates.dois.first().map(|d| d.value.clone());

    if let Some(ax) = candidates.arxiv_ids.first() {
        let ids = PaperIds {
            arxiv: Some(ax.value.id.clone()),
            doi: first_doi,
            ..PaperIds::default()
        };
        if let Ok(paper) = PaperRef::from_ids(ids, ax.value.version.clone()) {
            return Some(PaperKeyMatch {
                paper,
                basis: MatchBasis::ArxivCandidate,
                source_line: ax.line,
                source_page: ax.page,
                matched_title: candidates.title.as_ref().map(|t| t.value.clone()),
                confirmed: false,
            });
        }
    }

    if let Some(doi) = candidates.dois.first() {
        let ids = PaperIds {
            doi: Some(doi.value.clone()),
            ..PaperIds::default()
        };
        if let Ok(paper) = PaperRef::from_ids(ids, None) {
            return Some(PaperKeyMatch {
                paper,
                basis: MatchBasis::DoiCandidate,
                source_line: doi.line,
                source_page: doi.page,
                matched_title: candidates.title.as_ref().map(|t| t.value.clone()),
                confirmed: false,
            });
        }
    }

    let bytes = raw_pdf_bytes?;
    let key = PaperKey::from_file_bytes(bytes);
    let paper = PaperRef::from_ids_or(PaperIds::default(), None, Some(key)).ok()?;
    Some(PaperKeyMatch {
        paper,
        basis: MatchBasis::FileBytesHash,
        source_line: None,
        source_page: None,
        matched_title: candidates.title.as_ref().map(|t| t.value.clone()),
        confirmed: false,
    })
}

/// Query passed to an injectable [`MetadataFetcher`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataLookupQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arxiv_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
}

/// One bibliographic candidate returned by a [`MetadataFetcher`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCandidate {
    pub paper: PaperRef,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
}

/// Injectable network lookup backend for resolving PDF first-page candidates to a [`PaperRef`].
pub trait MetadataFetcher {
    fn lookup(&self, query: &MetadataLookupQuery) -> Result<Vec<RemoteCandidate>, String>;
}

fn title_match_key(s: &str) -> String {
    fold_diacritics(&normalize_pdf_unicode(s))
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn author_surname_key(author: &str) -> Option<String> {
    let trimmed = author.trim();
    if trimmed.is_empty() {
        return None;
    }
    let surname = if let Some((last, _)) = trimmed.split_once(',') {
        last.trim()
    } else {
        trimmed.split_whitespace().last().unwrap_or(trimmed)
    };
    let key = title_match_key(surname);
    (!key.is_empty()).then_some(key)
}

/// Resolve first-page candidates via an injectable [`MetadataFetcher`].
///
/// - When an arXiv id or DOI candidate exists, looks up that identifier and verifies the returned
///   record carries the same identifier.
/// - Otherwise, when a title candidate exists, queries by title + first author and accepts only an
///   exact normalized title match (plus first-author surname match when authors were extracted).
pub fn lookup_candidates_with_fetcher(
    candidates: &FirstPageCandidates,
    fetcher: &dyn MetadataFetcher,
) -> Result<Option<PaperKeyMatch>, String> {
    if let Some(ax) = candidates.arxiv_ids.first() {
        let query = MetadataLookupQuery {
            arxiv_id: Some(ax.value.id.clone()),
            doi: candidates.dois.first().map(|d| d.value.clone()),
            title: candidates.title.as_ref().map(|t| t.value.clone()),
            first_author: candidates
                .authors
                .as_ref()
                .and_then(|a| a.value.first().cloned()),
            year: candidates.year.as_ref().map(|y| y.value.clone()),
        };
        let hits = fetcher.lookup(&query)?;
        let matched = hits.into_iter().find(|h| {
            h.paper.ids.arxiv.as_deref() == Some(ax.value.id.as_str())
                || (h.paper.key.kind() == crate::paper_key::PaperKeyKind::Arxiv
                    && h.paper.key.value() == ax.value.id)
        });
        return Ok(matched.map(|h| PaperKeyMatch {
            paper: h.paper,
            basis: MatchBasis::RemoteIdentifierLookup,
            source_line: ax.line,
            source_page: ax.page,
            matched_title: Some(h.title),
            confirmed: false,
        }));
    }

    if let Some(doi) = candidates.dois.first() {
        let query = MetadataLookupQuery {
            doi: Some(doi.value.clone()),
            title: candidates.title.as_ref().map(|t| t.value.clone()),
            first_author: candidates
                .authors
                .as_ref()
                .and_then(|a| a.value.first().cloned()),
            year: candidates.year.as_ref().map(|y| y.value.clone()),
            ..MetadataLookupQuery::default()
        };
        let hits = fetcher.lookup(&query)?;
        let matched = hits.into_iter().find(|h| {
            h.paper.ids.doi.as_deref() == Some(doi.value.as_str())
                || (h.paper.key.kind() == crate::paper_key::PaperKeyKind::Doi
                    && h.paper.key.value() == doi.value)
        });
        return Ok(matched.map(|h| PaperKeyMatch {
            paper: h.paper,
            basis: MatchBasis::RemoteIdentifierLookup,
            source_line: doi.line,
            source_page: doi.page,
            matched_title: Some(h.title),
            confirmed: false,
        }));
    }

    let Some(title_cand) = &candidates.title else {
        return Ok(None);
    };
    let want_title = title_match_key(&title_cand.value);
    if want_title.len() < 6 {
        return Ok(None);
    }
    let first_author = candidates
        .authors
        .as_ref()
        .and_then(|a| a.value.first().cloned());
    let want_surname = first_author.as_deref().and_then(author_surname_key);

    let query = MetadataLookupQuery {
        title: Some(title_cand.value.clone()),
        first_author,
        year: candidates.year.as_ref().map(|y| y.value.clone()),
        ..MetadataLookupQuery::default()
    };
    let hits = fetcher.lookup(&query)?;
    let matched = hits.into_iter().find(|h| {
        let same_title = title_match_key(&h.title) == want_title;
        let same_author = want_surname.as_ref().is_none_or(|want| {
            h.authors
                .iter()
                .filter_map(|a| author_surname_key(a))
                .any(|s| &s == want)
        });
        same_title && same_author
    });
    Ok(matched.map(|h| PaperKeyMatch {
        paper: h.paper,
        basis: MatchBasis::RemoteExactTitleAuthor,
        source_line: title_cand.line,
        source_page: title_cand.page,
        matched_title: Some(h.title),
        confirmed: false,
    }))
}

/// Real [`MetadataFetcher`] adapter backed by F3's [`crate::paper_fetch::fetch_with_gate`] and
/// gated Crossref title lookup (`00-overview.md` 2.4, 2.11).
///
/// Callers must enforce the Overview 2.4 / 2.11 approval rule before invoking network lookups
/// composed from PDF text.
pub struct PaperFetchMetadataAdapter<'a> {
    env: &'a ResearchEnv,
    gate: &'a NetRateGate,
}

impl<'a> PaperFetchMetadataAdapter<'a> {
    pub fn new(env: &'a ResearchEnv, gate: &'a NetRateGate) -> Self {
        Self { env, gate }
    }
}

impl MetadataFetcher for PaperFetchMetadataAdapter<'_> {
    fn lookup(&self, query: &MetadataLookupQuery) -> Result<Vec<RemoteCandidate>, String> {
        if query.arxiv_id.is_some() || query.doi.is_some() {
            let ids = PaperIds {
                arxiv: query.arxiv_id.clone(),
                doi: query.doi.clone(),
                ..PaperIds::default()
            };
            let paper_ref = PaperRef::from_ids(ids, None)?;
            return match fetch_with_gate(self.env, &paper_ref, FetchWant::Metadata, self.gate)? {
                FetchOutcome::Metadata(meta) => Ok(vec![RemoteCandidate {
                    paper: meta.paper,
                    title: meta.title,
                    authors: meta.authors,
                    year: meta.published.as_deref().and_then(|s| s.get(..4)).map(String::from),
                }]),
                FetchOutcome::NeedsFile { .. } => Ok(Vec::new()),
                other => Err(format!("unexpected fetch outcome: {}", other.status_str())),
            };
        }

        let Some(title) = query.title.as_deref().filter(|s| !s.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let q = match query.first_author.as_deref() {
            Some(a) if !a.trim().is_empty() => format!("{title} {a}"),
            _ => title.to_string(),
        };
        let encoded = utf8_percent_encode(&q, NON_ALPHANUMERIC).to_string();
        let base = self.env.network_endpoints.crossref_api_base.trim_end_matches('/');
        let url = format!("{base}/works?rows=3&select=DOI,title,author,published&query.bibliographic={encoded}");
        let resp = fetch_response_with_gate(
            &url,
            &self.env.network_endpoints,
            MAX_METADATA_BYTES,
            Some(Service::Crossref),
            self.gate,
        )?;
        let body: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| format!("invalid Crossref JSON: {e}"))?;
        let mut out = Vec::new();
        if let Some(items) = body["message"]["items"].as_array() {
            for item in items {
                let Some(doi_raw) = item["DOI"].as_str() else { continue };
                let Ok(doi) = canonical_doi(doi_raw) else { continue };
                let Some(item_title) = item["title"].as_array().and_then(|a| a.first()).and_then(Value::as_str) else {
                    continue;
                };
                let authors: Vec<String> = item["author"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|a| {
                                let fam = a["family"].as_str()?;
                                match a["given"].as_str() {
                                    Some(g) if !g.is_empty() => Some(format!("{g} {fam}")),
                                    _ => Some(fam.to_string()),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let year = item["published"]["date-parts"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(Value::as_array)
                    .and_then(|p| p.first())
                    .and_then(Value::as_u64)
                    .map(|y| y.to_string());
                if let Ok(paper) = PaperRef::from_ids(
                    PaperIds {
                        doi: Some(doi),
                        ..PaperIds::default()
                    },
                    None,
                ) {
                    out.push(RemoteCandidate {
                        paper,
                        title: item_title.to_string(),
                        authors,
                        year,
                    });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "pdf_match_tests.rs"]
mod tests;
