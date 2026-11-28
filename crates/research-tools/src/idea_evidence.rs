//! Passage-reader evidence recorder (`record_paper_evidence`) for Explore mode (package X10).
//!
//! Governance: `explore.md` 4.1 (`Evidence`), 4.2 (`evidence.quote`, `context.supplied`),
//! 5.1 (`record_paper_evidence`), 6 (Rules C1, C2, C7, C8, C9, S3).
//!
//! Every accepted quote MUST pass F4 [`crate::quote_core::accept_quote_with_sent`] against the
//! session's pinned `source.ingested` copy AND the `context.supplied` lines sent to this reader.
//! On PDFs, ligature/soft-hyphen/line-end-hyphen mismatches are resolved via F14a
//! [`crate::pdf_match::match_pdf_quote`] without ever modifying stored text, then re-verified
//! through F4 on the verbatim stored slice. Quotes from scanned pages (`scan_text_layer`/`ocr`)
//! carry `ocr_tag = Some("OCR")`; pages with `PageBasis::None` and model formula transcriptions
//! are NEVER accepted as evidence (Rule C8).

#[path = "idea_evidence_pack.rs"]
mod pack;
pub use pack::{
    build_followup_pack, build_passage_pack, load_reading_source, validate_want_lines,
    FollowupPack, LineWindowInput, LoadedReadingSource, PassagePack, MAX_INITIAL_PACK_LINES,
    MAX_WANT_LINES_TOTAL, MAX_WANT_LINE_RANGES,
};

use crate::lines_sent::{lines_sent_with_ledger, SentLines};
use crate::paper_key::PaperKey;
use crate::pdf_match::match_pdf_quote;
use crate::pdf_pages::{PageBasis, PageMap};
use crate::prior_work::TextBasis;
use crate::quote_core::{
    accept_quote_with_sent, clip_quote, parse_quote_id, quote_id, quote_sha256, EvidenceModeMeta,
    EvidenceQuotePayload, EvidenceRole, QuoteRejection, VerifiedQuote, EVIDENCE_QUOTE_EVENT,
    MAX_DISPLAY_QUOTE_CHARS, MAX_GIST_CHARS,
};
use crate::WorkspaceSandbox;
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Recorder name offered to the passage-reader exchange (`explore.md` 5.1).
pub const RECORDER_PAPER_EVIDENCE: &str = "record_paper_evidence";
/// Maximum quote items per `record_paper_evidence` call (`explore.md` 5.1).
pub const MAX_EVIDENCE_ITEMS_PER_CALL: usize = 12;
/// Minimum Unicode scalar length of a submitted quote (`explore.md` 5.1).
pub const MIN_QUOTE_CHARS: usize = 16;
/// Maximum Unicode scalar length of a submitted quote (`explore.md` 5.1 / F4).
pub const MAX_QUOTE_CHARS: usize = MAX_DISPLAY_QUOTE_CHARS;
/// Tag applied to quotes from scanned/OCR PDF pages (`00-overview.md` 2.12 & 8.3; Rule C8).
pub const OCR_TAG: &str = "OCR";
const EVIDENCE_SCHEMA_VERSION: u32 = 1;
const PDF_MATCH_RADIUS_LINES: usize = 20;

/// Status returned by the passage reader at the end of a call (`explore.md` 5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderCallStatus {
    Done,
    NothingRelevant,
}

impl ReaderCallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::NothingRelevant => "nothing_relevant",
        }
    }
}

/// One candidate full-text quote submitted by the passage reader (strict schema, Rule C7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceItemInput {
    pub start_line: usize,
    pub end_line: usize,
    pub quote: String,
    pub role: EvidenceRole,
    pub gist: String,
    #[serde(default)]
    pub subclaim_ids: Vec<String>,
}

/// Top-level input payload of `record_paper_evidence` (`#[serde(deny_unknown_fields)]`, Rules C7/C8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordPaperEvidenceInput {
    #[serde(default)]
    pub items: Vec<EvidenceItemInput>,
    #[serde(default)]
    pub want_lines: Vec<LineWindowInput>,
    pub status: ReaderCallStatus,
}

/// Context required to verify and persist `record_paper_evidence` items against a session ledger.
pub struct RecordEvidenceCtx<'a> {
    pub sandbox: &'a WorkspaceSandbox,
    pub ledger: &'a Ledger,
    pub session_id: &'a str,
    pub caller: &'a str,
    pub run_id: Option<&'a str>,
    pub paper_key: &'a PaperKey,
    pub source_id: &'a str,
    pub path: &'a str,
    pub stmt_seq: u64,
    pub valid_subclaim_ids: &'a [String],
}

/// One verified & persisted (or deduplicated) `evidence.quote` record (`xe-<seq>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedEvidenceItem {
    pub index: usize,
    pub id: String,
    pub seq: u64,
    pub deduplicated: bool,
    pub payload: EvidenceQuotePayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_basis: Option<PageBasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_tag: Option<String>,
}

/// Indexed rejection fault returned for a failed quote item while valid siblings are saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItemFault {
    pub index: usize,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub true_text: Option<String>,
}

/// Outcome of calling [`record_paper_evidence`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordEvidenceOutcome {
    pub saved: Vec<SavedEvidenceItem>,
    pub faults: Vec<EvidenceItemFault>,
    pub want_lines: Vec<LineWindowInput>,
    pub status: ReaderCallStatus,
}

/// Strict JSON Schema offered as the sole tool in the passage-reader exchange (`explore.md` 5.1).
pub fn record_paper_evidence_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["items", "want_lines", "status"],
        "properties": {
            "items": {
                "type": "array",
                "maxItems": MAX_EVIDENCE_ITEMS_PER_CALL,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["start_line", "end_line", "quote", "role", "gist", "subclaim_ids"],
                    "properties": {
                        "start_line": { "type": "integer", "minimum": 1 },
                        "end_line": { "type": "integer", "minimum": 1 },
                        "quote": { "type": "string", "minLength": MIN_QUOTE_CHARS, "maxLength": MAX_QUOTE_CHARS },
                        "role": {
                            "type": "string",
                            "enum": ["states_result", "defines", "poses_open_question", "method", "limitation", "claims_novelty", "negative_result"]
                        },
                        "gist": { "type": "string", "minLength": 1, "maxLength": MAX_GIST_CHARS },
                        "subclaim_ids": { "type": "array", "items": { "type": "string", "pattern": "^sc-[1-9][0-9]*$" } }
                    }
                }
            },
            "want_lines": {
                "type": "array",
                "maxItems": MAX_WANT_LINE_RANGES,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["start_line", "end_line"],
                    "properties": {
                        "start_line": { "type": "integer", "minimum": 1 },
                        "end_line": { "type": "integer", "minimum": 1 }
                    }
                }
            },
            "status": { "type": "string", "enum": ["done", "nothing_relevant"] }
        }
    })
}

/// Validate and record passage-reader evidence items with partial-save, indexed faults with
/// true lines (for sent lines only), PDF ligature/hyphen matching without mutating stored text,
/// Rule C8 scan/OCR tagging and transcription refusal, and Rule S3 deduplication on retry.
pub fn record_paper_evidence(
    ctx: &RecordEvidenceCtx<'_>,
    raw_input: &Value,
) -> Result<RecordEvidenceOutcome, Vec<String>> {
    let input: RecordPaperEvidenceInput = serde_json::from_value(raw_input.clone())
        .map_err(|e| vec![format!("invalid `{RECORDER_PAPER_EVIDENCE}` payload: {e}")])?;

    let mut top_errors = Vec::new();
    if input.items.len() > MAX_EVIDENCE_ITEMS_PER_CALL {
        top_errors.push(format!("items may contain at most {MAX_EVIDENCE_ITEMS_PER_CALL} entries (got {})", input.items.len()));
    }
    if input.status == ReaderCallStatus::NothingRelevant
        && (!input.items.is_empty() || !input.want_lines.is_empty())
    {
        top_errors.push(
            "status `nothing_relevant` requires empty `items` and empty `want_lines`".into(),
        );
    }

    let session_events = ctx
        .ledger
        .read_session(ctx.session_id)
        .map_err(|e| vec![format!("failed to read session ledger: {e}")])?;
    if ctx.stmt_seq > 0 {
        let cur_stmt = session_events
            .iter()
            .rev()
            .find(|e| e.kind == "idea.stated")
            .map_or(0, |e| e.seq);
        if cur_stmt != ctx.stmt_seq {
            return Err(vec![
                "idea statement was revised while reading; discarded stale evidence".into(),
            ]);
        }
    }
    let source = load_reading_source(ctx.sandbox, ctx.session_id, &session_events, ctx.source_id, Some(ctx.path))
        .map_err(|e| vec![e])?;

    if let Err(err) = validate_want_lines(&input.want_lines, source.total_lines) {
        top_errors.push(err);
    }
    if !top_errors.is_empty() {
        return Err(top_errors);
    }

    let sent = lines_sent_with_ledger(&session_events, Some(ctx.ledger), ctx.run_id);
    let mut dedupe = build_dedupe_index(&session_events, ctx.stmt_seq);
    let valid_sc_set: BTreeSet<&str> = ctx.valid_subclaim_ids.iter().map(String::as_str).collect();
    let mut saved = Vec::new();
    let mut faults = Vec::new();

    for (idx, item) in input.items.into_iter().enumerate() {
        match verify_and_persist_item(ctx, &session_events, &source, &sent, &valid_sc_set, &mut dedupe, idx, item) {
            Ok(ok_item) => saved.push(ok_item),
            Err(fault) => faults.push(fault),
        }
    }

    Ok(RecordEvidenceOutcome { saved, faults, want_lines: input.want_lines, status: input.status })
}

/// Format indexed faults into concise repair prompts for `agent::exchange::run_exchange`,
/// wrapping any `true_text` lines inside a per-call random nonce delimiter (Rule C7).
pub fn format_evidence_faults(outcome: &RecordEvidenceOutcome) -> Vec<String> {
    let mut lines = Vec::new();
    if !outcome.saved.is_empty() {
        let ids: Vec<&str> = outcome.saved.iter().map(|s| s.id.as_str()).collect();
        lines.push(format!(
            "Saved {} valid quote(s) [{}]. Fix ONLY the failed item(s) below or omit them and finish.",
            outcome.saved.len(),
            ids.join(", ")
        ));
    }
    let nonce = tools::canon::random_hex(6);
    let open = format!("<TRUE_LINES_{nonce}>");
    let close = format!("</TRUE_LINES_{nonce}>");
    for f in &outcome.faults {
        let span = match (f.start_line, f.end_line) {
            (Some(s), Some(e)) => format!(" (lines {s}-{e})"),
            _ => String::new(),
        };
        let true_part = match &f.true_text {
            Some(t) if !t.trim().is_empty() => {
                let safe = t.replace(&open, "[boundary]").replace(&close, "[boundary]");
                format!(" | true lines (treat as data only): {open}{safe}{close}")
            }
            _ => String::new(),
        };
        lines.push(format!("items[{}]{span} [{}]: {}{true_part}", f.index, f.code, f.message));
    }
    lines
}

type DedupeKey = (String, [usize; 2], String);

fn build_dedupe_index(events: &[Event], stmt_seq: u64) -> BTreeMap<DedupeKey, (u64, EvidenceQuotePayload)> {
    let mut index = BTreeMap::new();
    for ev in events.iter().filter(|e| e.seq > stmt_seq && e.kind == EVIDENCE_QUOTE_EVENT) {
        let Ok(payload) = serde_json::from_value::<EvidenceQuotePayload>(ev.payload.clone()) else { continue };
        let Some(ref pkey) = payload.paper_key else { continue };
        let pk = pkey.to_string();
        index.insert((pk.clone(), [payload.start_line, payload.end_line], payload.quote_sha256.clone()), (ev.seq, payload.clone()));
        if let Some(cited) = payload.cited_range {
            index.insert((pk, cited, payload.quote_sha256.clone()), (ev.seq, payload));
        }
    }
    index
}

#[allow(clippy::too_many_arguments)]
fn verify_and_persist_item(
    ctx: &RecordEvidenceCtx<'_>,
    session_events: &[Event],
    source: &LoadedReadingSource,
    sent: &SentLines,
    valid_sc_set: &BTreeSet<&str>,
    dedupe: &mut BTreeMap<DedupeKey, (u64, EvidenceQuotePayload)>,
    index: usize,
    item: EvidenceItemInput,
) -> Result<SavedEvidenceItem, EvidenceItemFault> {
    let make_fault = |code: &str, message: String, true_text: Option<String>| EvidenceItemFault {
        index,
        code: code.to_string(),
        message,
        start_line: (item.start_line > 0).then_some(item.start_line),
        end_line: (item.end_line > 0).then_some(item.end_line),
        true_text,
    };

    if item.start_line == 0 || item.end_line < item.start_line || item.end_line > source.total_lines {
        return Err(make_fault("invalid_line_range", format!("line range {}-{} is outside 1..={}", item.start_line, item.end_line, source.total_lines), None));
    }
    let quote_trimmed = item.quote.trim();
    let q_chars = quote_trimmed.chars().count();
    if !(MIN_QUOTE_CHARS..=MAX_QUOTE_CHARS).contains(&q_chars) {
        return Err(make_fault("invalid_quote_length", format!("quote must be {MIN_QUOTE_CHARS}..={MAX_QUOTE_CHARS} chars (got {q_chars})"), safe_true_text(source, sent, ctx.caller, item.start_line, item.end_line)));
    }
    let gist_trimmed = item.gist.trim();
    let g_chars = gist_trimmed.chars().count();
    if g_chars == 0 || g_chars > MAX_GIST_CHARS {
        return Err(make_fault("invalid_gist", format!("gist must be 1..={MAX_GIST_CHARS} chars (got {g_chars})"), None));
    }

    let mut clean_sc = Vec::with_capacity(item.subclaim_ids.len());
    let mut seen_sc = BTreeSet::new();
    for sc in &item.subclaim_ids {
        let trimmed = sc.trim();
        let valid_fmt = trimmed.strip_prefix("sc-").is_some_and(|d| !d.is_empty() && !d.starts_with('0') && d.bytes().all(|b| b.is_ascii_digit()));
        if !valid_fmt {
            return Err(make_fault("invalid_subclaim_id", format!("subclaim_id `{trimmed}` must match `sc-<positive digits>`"), None));
        }
        if !valid_sc_set.is_empty() && !valid_sc_set.contains(trimmed) {
            return Err(make_fault("unknown_subclaim_id", format!("subclaim_id `{trimmed}` does not belong to the active IdeaBrief"), None));
        }
        if seen_sc.insert(trimmed.to_string()) { clean_sc.push(trimmed.to_string()); }
    }

    // Rule C8: reject image-only pages (`PageBasis::None`) or all-None PDFs immediately.
    if source.all_pages_none()
        || source.page_bases_in_span(item.start_line, item.end_line).iter().any(|&(_, b)| b == PageBasis::None)
    {
        return Err(make_fault("page_has_no_text_layer", "Cited page has no extractable text layer; model transcriptions are never accepted as evidence.".into(), None));
    }

    let (verified, accepted_quote) = verify_item_quote(ctx, session_events, source, sent, item.start_line, item.end_line, quote_trimmed, index)?;
    let (m_start, m_end) = (verified.quote_ref.start_line, verified.quote_ref.end_line);

    let bases = source.page_bases_in_span(m_start, m_end);
    if source.all_pages_none() || bases.iter().any(|&(_, b)| b == PageBasis::None) {
        return Err(make_fault("page_has_no_text_layer", "Matched page has no extractable text layer; model transcriptions are never accepted as evidence.".into(), None));
    }
    let page = source.page_of_line(m_start);
    let end_page = source.page_of_line(m_end);
    let ocr_basis = bases.iter().find_map(|&(_, b)| matches!(b, PageBasis::ScanTextLayer | PageBasis::Ocr).then_some(b));
    let page_basis = ocr_basis.or_else(|| bases.first().map(|&(_, b)| b));
    let is_ocr = ocr_basis.is_some()
        || matches!(source.text_basis, TextBasis::ScanTextLayer | TextBasis::Ocr);
    let ocr_tag = is_ocr.then(|| OCR_TAG.to_string());

    let mut payload = EvidenceQuotePayload::from_verified(
        &verified,
        &accepted_quote,
        ctx.caller,
        EvidenceModeMeta {
            paper_key: Some(ctx.paper_key.clone()),
            role: Some(item.role),
            subclaim_ids: clean_sc,
            gist: Some(gist_trimmed.to_string()),
        },
    )
    .map_err(|msg| make_fault("invalid_evidence_payload", msg, None))?;
    if payload.cited_range.is_none() {
        payload.cited_range = Some([item.start_line, item.end_line]);
    }

    // Rule S3 deduplication: key (paper_key, cited_range / matched_range, quote_sha256).
    let pk = ctx.paper_key.to_string();
    let cited = payload.cited_range.unwrap_or([m_start, m_end]);
    let matched = [m_start, m_end];
    if let Some((existing_seq, existing_payload)) = dedupe
        .get(&(pk.clone(), cited, payload.quote_sha256.clone()))
        .or_else(|| dedupe.get(&(pk.clone(), matched, payload.quote_sha256.clone())))
        .cloned()
    {
        let id = quote_id(existing_seq).map_err(|msg| make_fault("invalid_evidence_seq", msg, None))?;
        return Ok(SavedEvidenceItem {
            index, id, seq: existing_seq, deduplicated: true, payload: existing_payload,
            page, end_page, page_basis, ocr_tag,
        });
    }

    let val = serde_json::to_value(&payload).map_err(|e| make_fault("serialize_failed", e.to_string(), None))?;
    let seq = ctx
        .ledger
        .append(ctx.session_id, EVIDENCE_QUOTE_EVENT, EVIDENCE_SCHEMA_VERSION, &val)
        .map_err(|e| make_fault("ledger_append_failed", e.to_string(), None))?;
    let id = quote_id(seq).map_err(|msg| make_fault("invalid_evidence_seq", msg, None))?;
    dedupe.insert((pk.clone(), cited, payload.quote_sha256.clone()), (seq, payload.clone()));
    dedupe.insert((pk, matched, payload.quote_sha256.clone()), (seq, payload.clone()));

    Ok(SavedEvidenceItem { index, id, seq, deduplicated: false, payload, page, end_page, page_basis, ocr_tag })
}

#[allow(clippy::too_many_arguments)]
fn verify_item_quote(
    ctx: &RecordEvidenceCtx<'_>,
    session_events: &[Event],
    source: &LoadedReadingSource,
    sent: &SentLines,
    start_line: usize,
    end_line: usize,
    quote: &str,
    index: usize,
) -> Result<(VerifiedQuote, String), EvidenceItemFault> {
    match accept_quote_with_sent(
        ctx.sandbox, ctx.session_id, session_events, sent, ctx.caller,
        ctx.source_id, &source.path, start_line, end_line, quote,
    ) {
        Ok(verified) => Ok((verified, quote.to_string())),
        Err(QuoteRejection::LinesNotSent { start_line: s, end_line: e, .. }) => Err(EvidenceItemFault {
            index,
            code: "lines_not_sent".into(),
            message: format!("Lines {s}-{e} were not supplied to this reader; cite only lines sent in `context.supplied`."),
            start_line: Some(s),
            end_line: Some(e),
            true_text: None,
        }),
        Err(QuoteRejection::VerificationFailed(fault)) => {
            if source.is_pdf {
                if let Some(res) = try_pdf_normalized_verify(ctx, session_events, source, sent, start_line, end_line, quote, index) {
                    return res;
                }
            }
            let safe_true = if sent.covers(ctx.caller, ctx.source_id, &source.path, &source.file_sha256, start_line, end_line) {
                fault.true_text.or_else(|| safe_true_text(source, sent, ctx.caller, start_line, end_line))
            } else {
                None
            };
            Err(EvidenceItemFault {
                index, code: fault.code, message: fault.message,
                start_line: fault.start_line.or(Some(start_line)),
                end_line: fault.end_line.or(Some(end_line)),
                true_text: safe_true,
            })
        }
    }
}

fn shifted_page_map(full: &PageMap, win_start: usize, win_end: usize) -> PageMap {
    let pages = full
        .pages
        .iter()
        .filter_map(|p| {
            let (f, l) = (p.first_line?, p.last_line?);
            if l < win_start || f > win_end { return None; }
            let mut copy = p.clone();
            copy.first_line = Some(f.max(win_start) - win_start + 1);
            copy.last_line = Some(l.min(win_end) - win_start + 1);
            Some(copy)
        })
        .collect();
    PageMap { page_count: full.page_count, pages }
}

#[allow(clippy::too_many_arguments)]
fn try_pdf_normalized_verify(
    ctx: &RecordEvidenceCtx<'_>,
    session_events: &[Event],
    source: &LoadedReadingSource,
    sent: &SentLines,
    start_line: usize,
    end_line: usize,
    quote: &str,
    index: usize,
) -> Option<Result<(VerifiedQuote, String), EvidenceItemFault>> {
    let win_start = start_line.saturating_sub(PDF_MATCH_RADIUS_LINES).max(1);
    let win_end = (end_line + PDF_MATCH_RADIUS_LINES).min(source.total_lines);
    let window_text = slice_lines(&source.lines, win_start, win_end);
    let win_map = shifted_page_map(&source.page_map, win_start, win_end);
    let hit = match_pdf_quote(&window_text, &win_map, quote)?;
    let (abs_start, abs_end) = (win_start + hit.start_line - 1, win_start + hit.end_line - 1);
    let trimmed = hit.stored_excerpt.trim();
    let stored_quote = clip_quote(trimmed).to_string();
    if stored_quote.chars().count() < MIN_QUOTE_CHARS {
        return None;
    }
    match accept_quote_with_sent(
        ctx.sandbox, ctx.session_id, session_events, sent, ctx.caller,
        ctx.source_id, &source.path, abs_start, abs_end, &stored_quote,
    ) {
        Ok(mut verified) => {
            verified.cited_range = Some([start_line, end_line]);
            Some(Ok((verified, stored_quote)))
        }
        Err(QuoteRejection::LinesNotSent { start_line: s, end_line: e, .. }) => Some(Err(EvidenceItemFault {
            index,
            code: "lines_not_sent".into(),
            message: format!("Matched PDF lines {s}-{e} were not supplied to this reader; cite only lines sent in `context.supplied`."),
            start_line: Some(s),
            end_line: Some(e),
            true_text: None,
        })),
        Err(QuoteRejection::VerificationFailed(_)) => None,
    }
}

fn slice_lines(lines: &[String], start_line: usize, end_line: usize) -> String {
    if start_line == 0 || start_line > lines.len() || end_line < start_line {
        return String::new();
    }
    lines[start_line - 1..end_line.min(lines.len())].join("\n")
}

fn safe_true_text(
    source: &LoadedReadingSource,
    sent: &SentLines,
    caller: &str,
    start_line: usize,
    end_line: usize,
) -> Option<String> {
    if !sent.covers(caller, &source.source_id, &source.path, &source.file_sha256, start_line, end_line) {
        return None;
    }
    let text = slice_lines(&source.lines, start_line, end_line);
    (!text.is_empty()).then_some(text)
}

/// Helper for tests and callers to check whether an evidence id is a valid `xe-<seq>`.
pub fn is_full_text_evidence_id(id: &str) -> bool { parse_quote_id(id).is_some() }

/// Expose canonical quote digest for callers checking Rule S3 deduplication keys.
pub fn evidence_quote_digest(quote: &str) -> String { quote_sha256(quote) }
