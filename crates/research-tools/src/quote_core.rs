//! Shared quote verification, acceptance gating, and stored-quote event schemas (package F4).
//!
//! Wraps [`SourceIndex::verify_pointer`] without requiring an active Investigation marker, pairs
//! quote matching with [`crate::lines_sent`] coverage, and defines the mode-neutral
//! `evidence.quote` (`xe-<seq>`) and `evidence.abstract` (`xa-<seq>`) event payloads.

use crate::investigation::anchor::{fold_with_map, valid_id, Anchor, SourceIndex, MAX_TOTAL_BYTES};
use crate::lines_sent::{is_sha256_hex, lines_sent_with_ledger, SentLines};
use crate::paper_key::PaperKey;
use crate::{resolve_sources_session_id, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Provisional shared event kind for a stored full-text quote (owner decision 10 in
/// `docs/design/modes-2026-11/00-overview.md`, section 8).
pub const EVIDENCE_QUOTE_EVENT: &str = "evidence.quote";
/// Provisional shared event kind for a stored abstract quote (owner decision 10 in
/// `docs/design/modes-2026-11/00-overview.md`, section 8).
pub const EVIDENCE_ABSTRACT_EVENT: &str = "evidence.abstract";
/// Stored full-text quote prefix (`xe-<seq>`), reserved in [`crate::paper_key::ID_PREFIXES`].
pub const QUOTE_ID_PREFIX: &str = "xe-";
/// Stored abstract quote prefix (`xa-<seq>`), reserved in [`crate::paper_key::ID_PREFIXES`].
pub const ABSTRACT_ID_PREFIX: &str = "xa-";
/// Provisional UI word for quotes (owner decision 3 in `docs/design/modes-2026-11/00-overview.md`,
/// section 8): `"verified"` is reserved for Lean on every screen; quotes are `"matched"`.
pub const QUOTE_MATCHED_WORD: &str = "matched";
/// Maximum quote length shown on screen or exported (Unicode scalar values).
pub const MAX_DISPLAY_QUOTE_CHARS: usize = 300;
/// Maximum length of an optional evidence gist (Unicode scalar values).
pub const MAX_GIST_CHARS: usize = 240;
/// Maximum stored excerpt size in UTF-8 bytes.
pub const MAX_EXCERPT_BYTES: usize = 3_200;

/// Hash-pinned pointer to a matched quote within an ingested source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteRef {
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_sha256: String,
    pub quote_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_seq: Option<u64>,
}

/// Matched quote details returned by [`verify_quote`] and [`verify_quote_ref`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedQuote {
    pub quote_ref: QuoteRef,
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub excerpt_window: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cited_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub range_basis: Vec<String>,
    pub source_seq: u64,
}

/// Structured failure returned when a quote cannot be matched against an ingested source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteFault {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub end_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub true_text: Option<String>,
}

impl fmt::Display for QuoteFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for QuoteFault {}

/// Failure returned by [`accept_quote`] and [`accept_quote_with_sent`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuoteRejection {
    VerificationFailed(QuoteFault),
    LinesNotSent {
        caller: String,
        source_id: String,
        path: String,
        file_sha256: String,
        start_line: usize,
        end_line: usize,
    },
}

impl fmt::Display for QuoteRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VerificationFailed(fault) => write!(f, "quote verification failed: {fault}"),
            Self::LinesNotSent { caller, source_id, path, start_line, end_line, .. } => write!(
                f,
                "lines {start_line}-{end_line} of `{path}` (source `{source_id}`) were not sent to caller `{caller}`"
            ),
        }
    }
}

impl std::error::Error for QuoteRejection {}

/// Compute the canonical SHA-256 hex digest of a quote after whitespace folding.
///
/// Uses [`crate::investigation::anchor::fold_with_map`] so every whitespace run (including
/// newlines) collapses to a single ASCII space with ends trimmed, matching
/// `SourceIndex::verify_pointer` and Explore's quote deduplication rule.
pub fn quote_sha256(quote: &str) -> String {
    ledger::sha256_hex(fold_with_map(quote).0.as_bytes())
}

/// Clip `text` to at most `max_chars` Unicode scalar values on a valid UTF-8 character boundary.
pub fn clip_to_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

/// Clip `text` to at most [`MAX_DISPLAY_QUOTE_CHARS`] (300) Unicode scalar values for display or
/// export, always preserving UTF-8 character boundaries.
pub fn clip_quote(text: &str) -> &str { clip_to_chars(text, MAX_DISPLAY_QUOTE_CHARS) }

/// Match `quote` against lines `start_line..=end_line` of an ingested source in `session`.
///
/// Requires only a `source.ingested` receipt in `session_events` (no Investigation marker),
/// delegating matching, relocation, narrowing, trimming, and hash verification to
/// `SourceIndex::verify_pointer`.
#[allow(clippy::too_many_arguments, clippy::result_large_err)]
pub fn verify_quote(
    sandbox: &WorkspaceSandbox, session: &str, session_events: &[Event],
    source_id: &str, path: &str, start_line: usize, end_line: usize, quote: &str,
) -> Result<VerifiedQuote, QuoteFault> {
    verify_quote_inner(sandbox, session, session_events, source_id, path, start_line, end_line, "", quote, None)
}

/// Re-match `quote` against an existing [`QuoteRef`], checking `expected.quote_sha256`,
/// `expected.file_sha256` (when non-empty), and preserving `expected.evidence_seq`.
#[allow(clippy::result_large_err)]
pub fn verify_quote_ref(
    sandbox: &WorkspaceSandbox, session: &str, session_events: &[Event],
    expected: &QuoteRef, quote: &str,
) -> Result<VerifiedQuote, QuoteFault> {
    if quote_sha256(quote) != expected.quote_sha256 {
        return Err(QuoteFault {
            code: "quote_hash_mismatch".into(),
            message: "Quote digest does not match expected quote_sha256.".into(),
            start_line: (expected.start_line > 0).then_some(expected.start_line),
            end_line: (expected.end_line > 0).then_some(expected.end_line),
            true_text: None,
        });
    }
    verify_quote_inner(
        sandbox, session, session_events,
        &expected.source_id, &expected.path, expected.start_line, expected.end_line,
        &expected.file_sha256, quote, expected.evidence_seq,
    )
}

#[allow(clippy::too_many_arguments, clippy::result_large_err)]
fn verify_quote_inner(
    sandbox: &WorkspaceSandbox, session: &str, session_events: &[Event],
    source_id: &str, path: &str, start_line: usize, end_line: usize,
    expected_file_sha256: &str, quote: &str, evidence_seq: Option<u64>,
) -> Result<VerifiedQuote, QuoteFault> {
    let resolved_session = resolve_sources_session_id(session).map_err(|err| QuoteFault {
        code: "invalid_session".into(),
        message: err.to_string(),
        start_line: (start_line > 0).then_some(start_line),
        end_line: (end_line > 0).then_some(end_line),
        true_text: None,
    })?;

    let mut receipts: BTreeMap<&str, &Event> = BTreeMap::new();
    for event in session_events.iter().filter(|e| e.kind == "source.ingested") {
        if let Some(id) = event.payload["source_id"].as_str() {
            receipts.insert(id, event);
        }
    }

    let mut index = SourceIndex {
        sandbox: sandbox.clone(), session: resolved_session, primary_source_id: source_id,
        receipts, files: BTreeMap::new(), bytes_left: MAX_TOTAL_BYTES, pages: None,
        history: session_events, active_marker_seq: None,
    };
    let mut anchor = Anchor {
        source_id: source_id.to_string(), path: path.to_string(), start_line, end_line,
        excerpt: None, quote: Some(quote.to_string()), evidence: None, kind: "passage".into(),
        label: None, file_sha256: expected_file_sha256.to_string(), source_seq: 0,
        excerpt_basis: None, cited_range: None, range_basis: Vec::new(), excerpt_window: None,
    };

    match index.verify_pointer("quote", &mut anchor) {
        Ok(_) => Ok(VerifiedQuote {
            quote_ref: QuoteRef {
                source_id: anchor.source_id, path: anchor.path,
                start_line: anchor.start_line, end_line: anchor.end_line,
                file_sha256: anchor.file_sha256, quote_sha256: quote_sha256(quote),
                evidence_seq,
            },
            excerpt: anchor.excerpt.unwrap_or_default(),
            excerpt_window: anchor.excerpt_window,
            cited_range: anchor.cited_range,
            range_basis: anchor.range_basis,
            source_seq: anchor.source_seq,
        }),
        Err(fault) => Err(QuoteFault {
            code: fault.code.to_string(), message: fault.message,
            start_line: fault.lines.map(|[s, _]| s), end_line: fault.lines.map(|[_, e]| e),
            true_text: fault.true_text,
        }),
    }
}

/// Shared acceptance rule across all modes: a quote is accepted only if it matches the ingested
/// source file **and** its final matched lines were sent to `caller`.
#[allow(clippy::too_many_arguments, clippy::result_large_err)]
pub fn accept_quote(
    sandbox: &WorkspaceSandbox, session: &str, session_events: &[Event],
    ledger: Option<&Ledger>, run_id: Option<&str>, caller: &str,
    source_id: &str, path: &str, start_line: usize, end_line: usize, quote: &str,
) -> Result<VerifiedQuote, QuoteRejection> {
    let verified = verify_quote(sandbox, session, session_events, source_id, path, start_line, end_line, quote)
        .map_err(QuoteRejection::VerificationFailed)?;
    let sent = lines_sent_with_ledger(session_events, ledger, run_id);
    require_lines_sent(&sent, caller, verified)
}

/// Batch-friendly variant of [`accept_quote`] reusing a precomputed [`SentLines`] map.
#[allow(clippy::too_many_arguments, clippy::result_large_err)]
pub fn accept_quote_with_sent(
    sandbox: &WorkspaceSandbox, session: &str, session_events: &[Event],
    sent: &SentLines, caller: &str,
    source_id: &str, path: &str, start_line: usize, end_line: usize, quote: &str,
) -> Result<VerifiedQuote, QuoteRejection> {
    let verified = verify_quote(sandbox, session, session_events, source_id, path, start_line, end_line, quote)
        .map_err(QuoteRejection::VerificationFailed)?;
    require_lines_sent(sent, caller, verified)
}

#[allow(clippy::result_large_err)]
fn require_lines_sent(sent: &SentLines, caller: &str, verified: VerifiedQuote) -> Result<VerifiedQuote, QuoteRejection> {
    let q = &verified.quote_ref;
    if sent.covers(caller, &q.source_id, &q.path, &q.file_sha256, q.start_line, q.end_line) {
        return Ok(verified);
    }
    Err(QuoteRejection::LinesNotSent {
        caller: caller.to_string(), source_id: q.source_id.clone(), path: q.path.clone(),
        file_sha256: q.file_sha256.clone(), start_line: q.start_line, end_line: q.end_line,
    })
}

/// Optional semantic role attached to a stored quote record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    StatesResult, Defines, PosesOpenQuestion, Method, Limitation, ClaimsNovelty, NegativeResult,
}

/// Optional mode-specific metadata carried on [`EvidenceQuotePayload`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceModeMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub paper_key: Option<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub role: Option<EvidenceRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub subclaim_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gist: Option<String>,
}

/// Payload of a shared [`EVIDENCE_QUOTE_EVENT`] (`evidence.quote`) ledger event (`xe-<seq>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceQuotePayload {
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_sha256: String,
    pub quote_sha256: String,
    pub quote: String,
    pub excerpt: String,
    pub source_seq: u64,
    pub caller: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cited_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub range_basis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub excerpt_window: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub paper_key: Option<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub role: Option<EvidenceRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub subclaim_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gist: Option<String>,
}

impl EvidenceQuotePayload {
    /// Construct and validate an `evidence.quote` payload from a [`VerifiedQuote`].
    pub fn from_verified(
        verified: &VerifiedQuote, quote: &str, caller: &str, meta: EvidenceModeMeta,
    ) -> Result<Self, String> {
        let payload = Self {
            source_id: verified.quote_ref.source_id.clone(), path: verified.quote_ref.path.clone(),
            start_line: verified.quote_ref.start_line, end_line: verified.quote_ref.end_line,
            file_sha256: verified.quote_ref.file_sha256.clone(),
            quote_sha256: verified.quote_ref.quote_sha256.clone(),
            quote: quote.to_string(), excerpt: verified.excerpt.clone(),
            source_seq: verified.source_seq, caller: caller.to_string(),
            cited_range: verified.cited_range, range_basis: verified.range_basis.clone(),
            excerpt_window: verified.excerpt_window, paper_key: meta.paper_key,
            role: meta.role, subclaim_ids: meta.subclaim_ids,
            gist: meta.gist.map(|g| g.trim().to_string()),
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Validate all invariants of a stored `evidence.quote` payload.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.source_id, 64) {
            return Err("evidence.quote has invalid source_id".into());
        }
        if self.path.is_empty()
            || self.path.len() > 1024
            || self.path.split_once('/').map(|(id, _)| id) != Some(self.source_id.as_str())
        {
            return Err("evidence.quote path must belong to source_id and be <= 1024 bytes".into());
        }
        if self.start_line == 0 || self.end_line < self.start_line {
            return Err("evidence.quote must satisfy 1 <= start_line <= end_line".into());
        }
        if !is_sha256_hex(&self.file_sha256) {
            return Err("evidence.quote file_sha256 must be 64 lower-case hex characters".into());
        }
        validate_quote_and_hash(&self.quote, &self.quote_sha256, "evidence.quote")?;
        if self.excerpt.is_empty() || self.excerpt.len() > MAX_EXCERPT_BYTES {
            return Err(format!("evidence.quote excerpt must be 1..={MAX_EXCERPT_BYTES} UTF-8 bytes"));
        }
        if self.source_seq == 0 {
            return Err("evidence.quote source_seq must be >= 1".into());
        }
        if self.caller.trim().is_empty() {
            return Err("evidence.quote requires a non-empty caller".into());
        }
        if let Some([cs, ce]) = self.cited_range {
            if cs == 0 || ce < cs {
                return Err("evidence.quote cited_range must satisfy 1 <= start <= end".into());
            }
        }
        if let Some([w0, w1]) = self.excerpt_window {
            if w0 > w1 || w1 - w0 > MAX_EXCERPT_BYTES {
                return Err("evidence.quote excerpt_window is out of bounds".into());
            }
        }
        validate_mode_fields(&self.subclaim_ids, self.gist.as_deref(), "evidence.quote")
    }

    /// Build a [`QuoteRef`] pointing to this stored quote event at `evidence_seq`.
    pub fn to_quote_ref(&self, evidence_seq: u64) -> QuoteRef {
        QuoteRef {
            source_id: self.source_id.clone(), path: self.path.clone(),
            start_line: self.start_line, end_line: self.end_line,
            file_sha256: self.file_sha256.clone(), quote_sha256: self.quote_sha256.clone(),
            evidence_seq: Some(evidence_seq),
        }
    }
}

/// Payload of a shared [`EVIDENCE_ABSTRACT_EVENT`] (`evidence.abstract`) ledger event (`xa-<seq>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceAbstractPayload {
    pub paper_key: PaperKey,
    pub quote: String,
    pub quote_sha256: String,
    pub start: usize,
    pub end: usize,
    pub raw_blob: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub role: Option<EvidenceRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub subclaim_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gist: Option<String>,
}

impl EvidenceAbstractPayload {
    /// Construct and validate an `evidence.abstract` payload.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        paper_key: PaperKey, quote: impl Into<String>, start: usize, end: usize,
        raw_blob: impl Into<String>, provider: impl Into<String>,
        role: Option<EvidenceRole>, subclaim_ids: Vec<String>, gist: Option<String>,
    ) -> Result<Self, String> {
        let quote = quote.into();
        let payload = Self {
            paper_key, quote_sha256: quote_sha256(&quote), quote, start, end,
            raw_blob: raw_blob.into(), provider: provider.into(),
            role, subclaim_ids, gist: gist.map(|g| g.trim().to_string()),
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Validate all invariants of a stored `evidence.abstract` payload.
    pub fn validate(&self) -> Result<(), String> {
        validate_quote_and_hash(&self.quote, &self.quote_sha256, "evidence.abstract")?;
        if self.start >= self.end {
            return Err("evidence.abstract must satisfy start < end".into());
        }
        if !is_sha256_hex(&self.raw_blob) {
            return Err("evidence.abstract raw_blob must be 64 lower-case hex characters".into());
        }
        if self.provider.trim().is_empty() {
            return Err("evidence.abstract requires a non-empty provider".into());
        }
        validate_mode_fields(&self.subclaim_ids, self.gist.as_deref(), "evidence.abstract")
    }
}

/// Format a stored full-text quote identifier (`xe-<seq>`).
pub fn quote_id(seq: u64) -> Result<String, String> {
    if seq == 0 { return Err("quote evidence seq must be >= 1".into()); }
    Ok(format!("{QUOTE_ID_PREFIX}{seq}"))
}

/// Parse a stored full-text quote identifier (`xe-<seq>`), rejecting zero and leading zeros.
pub fn parse_quote_id(id: &str) -> Option<u64> { parse_prefixed_seq(id, QUOTE_ID_PREFIX) }

/// Format a stored abstract quote identifier (`xa-<seq>`).
pub fn abstract_id(seq: u64) -> Result<String, String> {
    if seq == 0 { return Err("abstract evidence seq must be >= 1".into()); }
    Ok(format!("{ABSTRACT_ID_PREFIX}{seq}"))
}

/// Parse a stored abstract quote identifier (`xa-<seq>`), rejecting zero and leading zeros.
pub fn parse_abstract_id(id: &str) -> Option<u64> { parse_prefixed_seq(id, ABSTRACT_ID_PREFIX) }

fn parse_prefixed_seq(id: &str, prefix: &str) -> Option<u64> {
    let digits = id.strip_prefix(prefix)?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok().filter(|&n| n >= 1)
}

fn validate_quote_and_hash(quote: &str, digest: &str, label: &str) -> Result<(), String> {
    if quote.trim().is_empty() || quote.chars().count() > MAX_DISPLAY_QUOTE_CHARS {
        return Err(format!("{label} quote must be non-empty and <= {MAX_DISPLAY_QUOTE_CHARS} characters"));
    }
    if !is_sha256_hex(digest) {
        return Err(format!("{label} quote_sha256 must be 64 lower-case hex characters"));
    }
    if digest != quote_sha256(quote) {
        return Err(format!("{label} quote_sha256 does not match folded quote"));
    }
    Ok(())
}

fn validate_mode_fields(subclaim_ids: &[String], gist: Option<&str>, label: &str) -> Result<(), String> {
    for sc in subclaim_ids {
        if parse_prefixed_seq(sc, "sc-").is_none() {
            return Err(format!("{label} subclaim_id `{sc}` must match `sc-<positive digits>`"));
        }
    }
    if let Some(g) = gist {
        if g.trim().is_empty() || g.chars().count() > MAX_GIST_CHARS {
            return Err(format!("{label} gist must be non-empty and <= {MAX_GIST_CHARS} characters"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "quote_core_tests.rs"]
mod tests;
