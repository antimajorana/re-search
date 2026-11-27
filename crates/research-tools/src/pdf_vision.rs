//! Opt-in vision-model equation transcription schema and text-layer agreement gate (package F14c).
//!
//! Owner Decision 2026-11-26 (non-negotiable honesty constraints):
//! 1. Off by default (`DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false`).
//! 2. Every transcription record is unconditionally labelled [`MODEL_TRANSCRIBED_LABEL`]
//!    (`"model-transcribed"`).
//! 3. A `"model-transcribed"` draft NEVER mints a [`crate::quote_core::QuoteRef`], NEVER emits
//!    `evidence.quote`, NEVER credits [`crate::lines_sent::SentLines::covers`] from `page_ranges`,
//!    and NEVER modifies `source.ingested` reading text. Any quote cited from that page MUST
//!    independently match the pinned text layer (`TextLayer`, `ScanTextLayer`, or `Ocr`) with
//!    text lines supplied in `context.supplied` via [`crate::quote_core::accept_quote`].

use crate::investigation::anchor::valid_id;
use crate::lines_sent::is_sha256_hex;
use crate::pdf_match::{match_pdf_quote, normalize_for_pdf_match, quotes_match_pdf, PdfQuoteMatch};
use crate::pdf_pages::{PageBasis, PageMap, PageQualityMetrics, PageRecord};
use crate::quote_core::{accept_quote, QuoteRejection, VerifiedQuote, MAX_DISPLAY_QUOTE_CHARS};
use crate::sandbox::{resolve_sources_session_id, SandboxRoot, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::Read;

/// Opt-in default for sending equation-page images to a user's vision model (Owner Decision 2026-11-26).
pub const DEFAULT_EQUATION_TRANSCRIPTION_ENABLED: bool = false;

/// Mandatory label stored on every `pdf.transcription` record and shown beside the page crop
/// (Owner Decision 2026-11-26).
pub const MODEL_TRANSCRIBED_LABEL: &str = "model-transcribed";

/// Ledger event kind recording one opt-in model-transcribed equation draft (package F14c).
pub const EVENT_PDF_TRANSCRIPTION: &str = "pdf.transcription";

/// Maximum UTF-8 byte length for a single `latex_draft` transcription payload.
pub const MAX_LATEX_DRAFT_BYTES: usize = 16 * 1024;

/// Maximum UTF-8 byte length for a model identifier recorded on a transcription.
pub const MAX_MODEL_ID_BYTES: usize = 160;

const MAX_READING_FILE_BYTES: usize = 16 * 1024 * 1024;
const MIN_SUBSTRING_NON_WS_CHARS: usize = 16;

/// Pinned text-layer provenance classes permitted to support a matched quote on a PDF page.
///
/// `PageBasis::None` (image-only / empty text layer) is intentionally excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextLayerBasis {
    TextLayer,
    ScanTextLayer,
    Ocr,
}

impl TextLayerBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextLayer => "text_layer",
            Self::ScanTextLayer => "scan_text_layer",
            Self::Ocr => "ocr",
        }
    }

    pub fn from_basis_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text_layer" => Some(Self::TextLayer),
            "scan_text_layer" => Some(Self::ScanTextLayer),
            "ocr" => Some(Self::Ocr),
            _ => None,
        }
    }

    /// True when quotes matched from this layer must carry the `"OCR"` / scanned-source tag.
    pub fn is_ocr_or_scan(self) -> bool { matches!(self, Self::ScanTextLayer | Self::Ocr) }

    fn to_page_basis(self) -> PageBasis {
        match self {
            Self::TextLayer => PageBasis::TextLayer,
            Self::ScanTextLayer | Self::Ocr => PageBasis::ScanTextLayer,
        }
    }
}

/// Evaluate the fail-closed opt-in gate for equation-page vision transcription.
///
/// Explicit per-request `Some(false)` always refuses; `Some(true)` opts in; `None` falls back to
/// the persisted setting, defaulting to [`DEFAULT_EQUATION_TRANSCRIPTION_ENABLED`] (`false`).
pub fn is_equation_transcription_opted_in(setting_enabled: Option<bool>, request_opt_in: Option<bool>) -> bool {
    match request_opt_in {
        Some(false) => false,
        Some(true) => true,
        None => setting_enabled.unwrap_or(DEFAULT_EQUATION_TRANSCRIPTION_ENABLED),
    }
}

/// Stored payload of an [`EVENT_PDF_TRANSCRIPTION`] (`pdf.transcription`) ledger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormulaTranscription {
    pub source_id: String,
    pub raw_blob_sha256: String,
    pub page: usize,
    /// Optional `[x, y, width, height]` pixel crop rectangle (`width > 0 && height > 0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[u32; 4]>,
    pub model: String,
    pub latex_draft: String,
    /// Always [`MODEL_TRANSCRIBED_LABEL`] (`"model-transcribed"`).
    pub label: String,
    /// Advisory flag indicating whether the pinned text layer on `page` agreed with `latex_draft`.
    /// Even when `true`, a matched quote still requires [`verify_transcription_against_text_layer`].
    pub agreed_with_text_layer: bool,
}

impl FormulaTranscription {
    /// Construct and validate a [`FormulaTranscription`] unconditionally labelled [`MODEL_TRANSCRIBED_LABEL`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_id: impl Into<String>, raw_blob_sha256: impl Into<String>, page: usize,
        crop: Option<[u32; 4]>, model: impl Into<String>, latex_draft: impl Into<String>,
        agreed_with_text_layer: bool,
    ) -> Result<Self, String> {
        let rec = Self {
            source_id: source_id.into(),
            raw_blob_sha256: raw_blob_sha256.into(),
            page,
            crop,
            model: model.into().trim().to_string(),
            latex_draft: latex_draft.into().trim().to_string(),
            label: MODEL_TRANSCRIBED_LABEL.to_string(),
            agreed_with_text_layer,
        };
        rec.validate()?;
        Ok(rec)
    }

    /// Validate all structural and honesty invariants of a [`FormulaTranscription`].
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.source_id, 64) {
            return Err("pdf.transcription has invalid source_id".into());
        }
        if !is_sha256_hex(&self.raw_blob_sha256) {
            return Err("pdf.transcription raw_blob_sha256 must be 64 lower-case hex characters".into());
        }
        if self.page == 0 {
            return Err("pdf.transcription page must be >= 1".into());
        }
        if let Some([_, _, w, h]) = self.crop {
            if w == 0 || h == 0 {
                return Err("pdf.transcription crop must have width > 0 and height > 0".into());
            }
        }
        if self.model.trim().is_empty() || self.model.len() > MAX_MODEL_ID_BYTES {
            return Err(format!("pdf.transcription model must be non-empty and <= {MAX_MODEL_ID_BYTES} bytes"));
        }
        if self.latex_draft.trim().is_empty() || self.latex_draft.len() > MAX_LATEX_DRAFT_BYTES {
            return Err(format!("pdf.transcription latex_draft must be non-empty and <= {MAX_LATEX_DRAFT_BYTES} bytes"));
        }
        if self.label != MODEL_TRANSCRIBED_LABEL {
            return Err(format!("pdf.transcription label must be `{MODEL_TRANSCRIBED_LABEL}`, got `{}`", self.label));
        }
        Ok(())
    }
}

/// Outcome of a read-only text-layer agreement check on a single PDF page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLayerAgreement {
    pub agreed: bool,
    pub basis: Option<TextLayerBasis>,
    pub primary_path: Option<String>,
    pub matched: Option<PdfQuoteMatch>,
}

/// Verified text-layer quote returned by [`verify_transcription_against_text_layer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLayerVerifiedQuote {
    pub verified: VerifiedQuote,
    pub page: usize,
    pub basis: TextLayerBasis,
}

/// Structured failure returned when the text-layer agreement gate refuses a transcription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptionGateError {
    InvalidTranscription(String),
    SourceNotFound { source_id: String },
    SourceNotPdf { source_id: String },
    RawBlobMismatch { source_id: String, expected: String, actual: String },
    NoTextLayer { source_id: String, page: usize },
    ReadingFileError(String),
    TextLayerDisagrees { source_id: String, page: usize },
    QuoteRejected(QuoteRejection),
}

impl fmt::Display for TranscriptionGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTranscription(msg) => write!(f, "invalid transcription: {msg}"),
            Self::SourceNotFound { source_id } => write!(f, "source `{source_id}` has no source.ingested receipt in session"),
            Self::SourceNotPdf { source_id } => write!(f, "source `{source_id}` is not a PDF"),
            Self::RawBlobMismatch { source_id, .. } => write!(f, "source `{source_id}` raw_blob digest does not match transcription"),
            Self::NoTextLayer { source_id, page } => write!(
                f,
                "source `{source_id}` page {page} has no pinned text layer (TextLayer/ScanTextLayer/Ocr); model-transcribed text cannot be quoted"
            ),
            Self::ReadingFileError(msg) => write!(f, "pinned reading file error: {msg}"),
            Self::TextLayerDisagrees { source_id, page } => write!(
                f,
                "pinned text layer on source `{source_id}` page {page} disagrees with model-transcribed draft; refusing matched quote"
            ),
            Self::QuoteRejected(rej) => write!(f, "{rej}"),
        }
    }
}

impl std::error::Error for TranscriptionGateError {}

struct PinnedPageText {
    primary_path: String,
    basis: TextLayerBasis,
    first_line: usize,
    last_line: usize,
    slice_start_byte: usize,
    page_text: String,
    page_line_count: usize,
}

fn find_pdf_receipt<'a>(events: &'a [Event], source_id: &str) -> Option<&'a Value> {
    events
        .iter()
        .rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id))
        .map(|e| &e.payload)
}

fn resolve_page_basis_and_lines(receipt: &Value, page: usize) -> Option<(TextLayerBasis, usize, usize)> {
    if let Some(pages) = receipt.get("pdf_pages").and_then(Value::as_array) {
        let rec = pages
            .iter()
            .find(|p| p.get("page").and_then(Value::as_u64) == Some(page as u64))?;
        let basis = rec
            .get("basis")
            .and_then(Value::as_str)
            .and_then(TextLayerBasis::from_basis_str)?;
        let first_line = rec.get("first_line").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())?;
        let last_line = rec.get("last_line").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())?;
        if first_line == 0 || last_line < first_line {
            return None;
        }
        return Some((basis, first_line, last_line));
    }

    if receipt.get("status").and_then(Value::as_str) == Some("empty_text_layer") {
        return None;
    }
    let basis = match receipt
        .get("basis")
        .or_else(|| receipt.get("text_basis"))
        .and_then(Value::as_str)
    {
        Some(raw) => TextLayerBasis::from_basis_str(raw)?,
        None => {
            // Conservative scan classification when pre-F14a receipt lacks per-page scan metadata
            // (`00-overview.md` 2.12: when unsure, treat as a scan).
            if receipt.get("status").and_then(Value::as_str) == Some("ok") {
                TextLayerBasis::ScanTextLayer
            } else {
                return None;
            }
        }
    };
    let lines = receipt.get("pdf_page_lines").and_then(Value::as_array)?;
    for entry in lines {
        let arr = entry.as_array()?;
        if arr.len() < 3 {
            continue;
        }
        let p = arr[0].as_u64().and_then(|n| usize::try_from(n).ok())?;
        if p == page {
            let first = arr[1].as_u64().and_then(|n| usize::try_from(n).ok())?;
            let last = arr[2].as_u64().and_then(|n| usize::try_from(n).ok())?;
            if first >= 1 && last >= first {
                return Some((basis, first, last));
            }
            return None;
        }
    }
    None
}

fn slice_lines(text: &str, first_line: usize, last_line: usize) -> Option<(usize, String, usize)> {
    if first_line == 0 || last_line < first_line {
        return None;
    }
    let mut line_no = 1usize;
    let mut start_byte = if first_line == 1 { Some(0usize) } else { None };
    let mut end_byte = None;
    for (idx, b) in text.bytes().enumerate() {
        if b == b'\n' {
            if line_no == last_line {
                end_byte = Some(idx);
                break;
            }
            line_no += 1;
            if line_no == first_line && start_byte.is_none() {
                start_byte = Some(idx + 1);
            }
        }
    }
    let start = start_byte?;
    let end = end_byte.unwrap_or(text.len());
    if start > end || line_no < first_line {
        return None;
    }
    let slice = &text[start..end];
    let count = slice.bytes().filter(|&b| b == b'\n').count() + 1;
    Some((start, slice.to_string(), count))
}

fn load_pinned_page_text(
    sandbox: &WorkspaceSandbox,
    session: &str,
    session_events: &[Event],
    source_id: &str,
    raw_blob_sha256: &str,
    page: usize,
) -> Result<PinnedPageText, TranscriptionGateError> {
    let resolved_session = resolve_sources_session_id(session)
        .map_err(|e| TranscriptionGateError::ReadingFileError(e.to_string()))?;
    let receipt = find_pdf_receipt(session_events, source_id).ok_or_else(|| {
        TranscriptionGateError::SourceNotFound { source_id: source_id.to_string() }
    })?;
    if receipt["format"].as_str() != Some("pdf") {
        return Err(TranscriptionGateError::SourceNotPdf { source_id: source_id.to_string() });
    }
    let actual_blob = receipt["raw_blob"].as_str().unwrap_or("");
    if actual_blob != raw_blob_sha256 {
        return Err(TranscriptionGateError::RawBlobMismatch {
            source_id: source_id.to_string(),
            expected: raw_blob_sha256.to_string(),
            actual: actual_blob.to_string(),
        });
    }
    let (basis, first_line, last_line) = resolve_page_basis_and_lines(receipt, page).ok_or_else(|| {
        TranscriptionGateError::NoTextLayer { source_id: source_id.to_string(), page }
    })?;
    let primary_path = receipt["primary_path"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TranscriptionGateError::NoTextLayer { source_id: source_id.to_string(), page })?
        .to_string();
    let expected_sha = receipt["files_sha256"]
        .get(&primary_path)
        .and_then(Value::as_str)
        .filter(|s| is_sha256_hex(s))
        .ok_or_else(|| {
            TranscriptionGateError::ReadingFileError(format!(
                "source `{source_id}` is missing pinned files_sha256 for `{primary_path}`"
            ))
        })?;
    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, resolved_session, &primary_path, false)
        .map_err(|e| TranscriptionGateError::ReadingFileError(e.to_string()))?;
    if !resolved.metadata.is_file() {
        return Err(TranscriptionGateError::ReadingFileError(format!(
            "reading file `{primary_path}` is not a regular file"
        )));
    }
    let file = fs::File::open(&resolved.canonical_target)
        .map_err(|e| TranscriptionGateError::ReadingFileError(e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_READING_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| TranscriptionGateError::ReadingFileError(e.to_string()))?;
    if bytes.len() > MAX_READING_FILE_BYTES {
        return Err(TranscriptionGateError::ReadingFileError(format!(
            "reading file `{primary_path}` exceeds {MAX_READING_FILE_BYTES} bytes"
        )));
    }
    if ledger::sha256_hex(&bytes) != expected_sha {
        return Err(TranscriptionGateError::ReadingFileError(format!(
            "reading file `{primary_path}` hash mismatch against pinned source.ingested receipt"
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| TranscriptionGateError::ReadingFileError(e.to_string()))?;
    let (slice_start_byte, page_text, page_line_count) =
        slice_lines(&text, first_line, last_line).ok_or_else(|| {
            TranscriptionGateError::NoTextLayer { source_id: source_id.to_string(), page }
        })?;
    if page_text.trim().is_empty() {
        return Err(TranscriptionGateError::NoTextLayer { source_id: source_id.to_string(), page });
    }
    Ok(PinnedPageText {
        primary_path,
        basis,
        first_line,
        last_line,
        slice_start_byte,
        page_text,
        page_line_count,
    })
}

fn strip_outer_math_delimiters(raw: &str) -> &str {
    let s = raw.trim();
    for (open, close) in [("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)"), ("$", "$")] {
        if let Some(inner) = s.strip_prefix(open).and_then(|rest| rest.strip_suffix(close)) {
            let trimmed = inner.trim();
            if !trimmed.is_empty() && !trimmed.contains(open) && !trimmed.contains(close) {
                return trimmed;
            }
        }
    }
    s
}

fn has_token_boundaries(haystack: &str, start: usize, end: usize) -> bool {
    let starts_alnum = haystack[start..end].chars().next().is_some_and(char::is_alphanumeric);
    let ends_alnum = haystack[start..end].chars().next_back().is_some_and(char::is_alphanumeric);
    let prev_ok = !starts_alnum
        || haystack[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
    let next_ok = !ends_alnum
        || haystack[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    prev_ok && next_ok
}

fn match_candidate_on_page(pinned: &PinnedPageText, page: usize, latex_draft: &str) -> Option<PdfQuoteMatch> {
    let raw = latex_draft.trim();
    let stripped = strip_outer_math_delimiters(raw);
    let candidates: Vec<&str> = if stripped != raw { vec![raw, stripped] } else { vec![raw] };
    let dummy_map = PageMap {
        page_count: 1,
        pages: vec![PageRecord {
            page,
            basis: pinned.basis.to_page_basis(),
            first_line: Some(1),
            last_line: Some(pinned.page_line_count),
            metrics: PageQualityMetrics {
                char_count: pinned.page_text.chars().count(),
                alpha_count: 0, word_count: 0, image_coverage_pct: 0,
                has_invisible_text: false, width_pt: 0, height_pt: 0,
            },
            reason: None,
        }],
    };
    for cand in candidates {
        if cand.chars().count() > MAX_DISPLAY_QUOTE_CHARS {
            continue;
        }
        let norm = normalize_for_pdf_match(cand);
        let non_ws = norm.chars().filter(|c| !c.is_whitespace()).count();
        if non_ws < 2 {
            continue;
        }
        let Some(mut m) = match_pdf_quote(&pinned.page_text, &dummy_map, cand) else {
            continue;
        };
        if m.stored_excerpt.chars().count() > MAX_DISPLAY_QUOTE_CHARS
            || !has_token_boundaries(&pinned.page_text, m.start_byte, m.end_byte)
        {
            continue;
        }
        // Keep parity with quote_core's anti-short-substring rule (<16 non-ws chars must cover
        // the entire trimmed cited line span).
        if non_ws < MIN_SUBSTRING_NON_WS_CHARS {
            let Some((_, line_span, _)) = slice_lines(&pinned.page_text, m.start_line, m.end_line) else {
                continue;
            };
            if normalize_for_pdf_match(line_span.trim()) != normalize_for_pdf_match(m.stored_excerpt.trim()) {
                continue;
            }
        }
        // Fail closed on ambiguous duplicate occurrences on the same page.
        if quotes_match_pdf(&pinned.page_text[m.end_byte..], cand) {
            continue;
        }
        m.start_byte += pinned.slice_start_byte;
        m.end_byte += pinned.slice_start_byte;
        m.start_line += pinned.first_line - 1;
        m.end_line += pinned.first_line - 1;
        m.start_page = Some(page);
        m.end_page = Some(page);
        return Some(m);
    }
    None
}

/// Read-only advisory check of whether `latex_draft` agrees with the pinned text layer on `page`.
///
/// Never mutates `source.ingested` or reading files, never mints a `QuoteRef`, and never credits
/// `SentLines`. Returns `agreed: false` whenever the page lacks a pinned text layer or disagrees.
pub fn check_text_layer_agreement(
    sandbox: &WorkspaceSandbox,
    session: &str,
    session_events: &[Event],
    source_id: &str,
    raw_blob_sha256: &str,
    page: usize,
    latex_draft: &str,
) -> TextLayerAgreement {
    let Ok(pinned) = load_pinned_page_text(sandbox, session, session_events, source_id, raw_blob_sha256, page) else {
        return TextLayerAgreement { agreed: false, basis: None, primary_path: None, matched: None };
    };
    let matched = match_candidate_on_page(&pinned, page, latex_draft);
    TextLayerAgreement {
        agreed: matched.is_some(),
        basis: Some(pinned.basis),
        primary_path: Some(pinned.primary_path),
        matched,
    }
}

/// Enforce the F14c text-layer agreement gate (Owner Decision 2026-11-26).
///
/// A `"model-transcribed"` draft NEVER produces a `QuoteRef` / `evidence.quote` on its own.
/// This gate accepts a matched quote ONLY when:
/// 1. `transcription` is valid and labelled `"model-transcribed"`.
/// 2. The pinned `source.ingested` PDF receipt matches `raw_blob_sha256` and has a pinned text
///    layer (`TextLayer`, `ScanTextLayer`, or `Ocr` — never `None`) on `transcription.page`.
/// 3. The pinned text layer on that page agrees with `transcription.latex_draft` under
///    [`crate::pdf_match::match_pdf_quote`].
/// 4. [`crate::quote_core::accept_quote`] independently verifies the stored text-layer excerpt
///    AND confirms its text lines (on `transcription.page`) were supplied to `caller` via
///    `context.supplied` (`ranges`) or a read tool (never via `page_ranges`).
#[allow(clippy::result_large_err)]
pub fn verify_transcription_against_text_layer(
    sandbox: &WorkspaceSandbox,
    session: &str,
    session_events: &[Event],
    ledger: Option<&Ledger>,
    run_id: Option<&str>,
    caller: &str,
    transcription: &FormulaTranscription,
) -> Result<TextLayerVerifiedQuote, TranscriptionGateError> {
    transcription
        .validate()
        .map_err(TranscriptionGateError::InvalidTranscription)?;
    let pinned = load_pinned_page_text(
        sandbox,
        session,
        session_events,
        &transcription.source_id,
        &transcription.raw_blob_sha256,
        transcription.page,
    )?;
    let matched = match_candidate_on_page(&pinned, transcription.page, &transcription.latex_draft)
        .ok_or_else(|| TranscriptionGateError::TextLayerDisagrees {
            source_id: transcription.source_id.clone(),
            page: transcription.page,
        })?;
    let verified = accept_quote(
        sandbox,
        session,
        session_events,
        ledger,
        run_id,
        caller,
        &transcription.source_id,
        &pinned.primary_path,
        matched.start_line,
        matched.end_line,
        &matched.stored_excerpt,
    )
    .map_err(TranscriptionGateError::QuoteRejected)?;
    if verified.quote_ref.start_line < pinned.first_line || verified.quote_ref.end_line > pinned.last_line {
        return Err(TranscriptionGateError::TextLayerDisagrees {
            source_id: transcription.source_id.clone(),
            page: transcription.page,
        });
    }
    Ok(TextLayerVerifiedQuote {
        verified,
        page: transcription.page,
        basis: pinned.basis,
    })
}

#[cfg(test)]
#[path = "pdf_vision_tests.rs"]
mod tests;
