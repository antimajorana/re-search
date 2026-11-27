//! Solve mode S8/S9 HTTP routes split out of `solve.rs` so `solve.rs` remains compact.
//!
//! Routes handled here:
//! - S8: `POST /api/sessions/{id}/solve/attach-source`, `POST /api/sessions/{id}/solve/ack`
//! - S9: `POST /api/sessions/{id}/solve/check`, `POST /api/sessions/{id}/solve/recheck` (wired in S9)

use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::session_mode::session_mode;
use crate::solve::session_solve_view;
use crate::state::{now_ms, DesktopState};
use ledger::Event;
use research_tools::library::{Library, SeenRecord, SeenRole};
use research_tools::paper_fetch::{fetch, parse_paper_input, FetchOutcome, FetchWant};
use research_tools::paper_key::{Mode, PaperKey};
use research_tools::pdf_pages::PageRecord;
use research_tools::quote_core::{
    clip_quote, verify_quote, EvidenceModeMeta, EvidenceQuotePayload, QuoteRef,
};
use research_tools::solve_model::{event_kinds, AckKind, StepEntryType};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::badge_for_step;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::sync::Mutex;

/// S8: `POST /api/sessions/{id}/solve/attach-source`.
///
/// Attaches a cited source via F10 library (`library_key`/`paper_key`), F3 fetch (`arxiv_id`),
/// or existing session `source_id`, runs F4 `verify_quote`, flags `ocr: true` on scanned PDF pages,
/// appends `evidence.quote` (max 300-char excerpt), and returns the pinned `QuoteRef`.
pub fn handle_attach_source(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let quote = val
        .get("quote")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`quote` must be a non-empty string"))?
        .to_string();
    let start_line = val
        .get("start_line")
        .and_then(Value::as_u64)
        .filter(|&n| n >= 1)
        .ok_or_else(|| bad("`start_line` must be a positive integer"))? as usize;
    let end_line = val
        .get("end_line")
        .and_then(Value::as_u64)
        .filter(|&n| n as usize >= start_line)
        .ok_or_else(|| bad("`end_line` must be an integer >= `start_line`"))?
        as usize;

    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let source_id = ensure_session_source(state, &env, session_id, &val)?;

    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let ingested_ev = events
        .iter()
        .rev()
        .find(|ev| {
            ev.kind == "source.ingested"
                && ev.payload.get("source_id").and_then(Value::as_str) == Some(source_id.as_str())
        })
        .ok_or_else(|| not_found(format!("source `{source_id}` is not ingested in session `{session_id}`")))?;

    let path = resolve_source_path(ingested_ev, &source_id, val.get("path").and_then(Value::as_str))?;
    let verified = verify_quote(
        &env.sandbox(),
        session_id,
        &events,
        &source_id,
        &path,
        start_line,
        end_line,
        &quote,
    )
    .map_err(|fault| bad(fault.to_string()))?;

    let (ocr, matched_page) =
        detect_ocr_and_page(ingested_ev, verified.quote_ref.start_line, verified.quote_ref.end_line);
    let mut eq_payload =
        EvidenceQuotePayload::from_verified(&verified, &quote, "user", EvidenceModeMeta::default())
            .map_err(bad)?;
    eq_payload.excerpt = clip_quote(&eq_payload.excerpt).to_string();
    let excerpt_out = eq_payload.excerpt.clone();

    let mut payload_val = serde_json::to_value(&eq_payload).map_err(|e| internal(e.to_string()))?;
    if let Some(obj) = payload_val.as_object_mut() {
        if ocr {
            obj.insert("ocr".into(), json!(true));
        }
        if let Some(page) = matched_page {
            obj.insert("page".into(), json!(page));
        }
    }

    let _guard = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger write lock poisoned"))?;
    let seq = ledger
        .append_with_hash(session_id, event_kinds::EVIDENCE_QUOTE, 1, &payload_val)
        .map_err(|e| internal(format!("failed to write evidence.quote: {e}")))?
        .seq;
    let updated_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    drop(_guard);

    state.sse.emit(
        session_id,
        "solve_updated",
        json!({ "session": session_id, "evidence_seq": seq, "source_id": source_id }),
    );

    let quote_ref = QuoteRef {
        evidence_seq: Some(seq),
        ..verified.quote_ref
    };
    let view = session_solve_view(
        state,
        session_id,
        &updated_events,
        is_turn_running(state, session_id),
    );
    Ok((
        200,
        json!({
            "ok": true,
            "evidence_seq": seq,
            "quote_ref": quote_ref,
            "ocr": ocr,
            "page": matched_page,
            "excerpt": excerpt_out,
            "view": view,
        }),
    ))
}

/// S8: `POST /api/sessions/{id}/solve/ack`.
///
/// Host-only acknowledgement for `assumption`, `hand_checked`, or `transcription` (`solve.md` D10/D11/D13),
/// hash-bound to the active step's `entry_sha256`.
pub fn handle_ack(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let entry_id = val
        .get("entry_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`entry_id` must be a non-empty string"))?
        .to_string();
    let kind: AckKind = serde_json::from_value(
        val.get("kind")
            .cloned()
            .ok_or_else(|| bad("`kind` is required (`assumption`, `hand_checked`, or `transcription`)"))?,
    )
    .map_err(|e| bad(format!("invalid `kind`: {e}")))?;
    let entry_sha256 = val
        .get("entry_sha256")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`entry_sha256` must be a non-empty string"))?
        .to_string();

    let _guard = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger write lock poisoned"))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    if session_mode(&events) != Mode::Solve {
        return Err(bad("This endpoint is only available for Solve sessions."));
    }
    let folded = SolveState::from_events(&events);
    let entry = folded
        .entry(&entry_id)
        .ok_or_else(|| not_found(format!("unknown step `{entry_id}`")))?;
    if entry.superseded_by.is_some() {
        return Err(conflict(format!(
            "cannot acknowledge superseded step `{entry_id}`"
        )));
    }
    if entry.entry_sha256.as_deref() != Some(entry_sha256.as_str()) {
        return Err(conflict(format!(
            "`entry_sha256` does not match step `{entry_id}`"
        )));
    }
    if kind == AckKind::Assumption && entry.entry_type != StepEntryType::Assumption {
        return Err(bad(format!(
            "`kind: \"assumption\"` applies only to `assumption` steps (got `{}`)",
            entry.entry_type.as_str()
        )));
    }
    if kind == AckKind::Transcription && entry.transcription.is_none() {
        return Err(bad(format!(
            "`kind: \"transcription\"` requires a transcribed step (`{entry_id}` has no transcription metadata)"
        )));
    }

    let payload = json!({
        "entry_id": entry_id,
        "kind": kind,
        "entry_sha256": entry_sha256,
    });
    let seq = ledger
        .append_with_hash(session_id, event_kinds::SOLVE_ACK, 1, &payload)
        .map_err(|e| internal(format!("failed to write solve.ack: {e}")))?
        .seq;
    let updated_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    drop(_guard);

    let updated_state = SolveState::from_events(&updated_events);
    let badge = badge_for_step(&updated_state, &entry_id);
    state.sse.emit(
        session_id,
        "solve_updated",
        json!({ "session": session_id, "ack_seq": seq, "entry_id": entry_id }),
    );

    let view = session_solve_view(
        state,
        session_id,
        &updated_events,
        is_turn_running(state, session_id),
    );
    Ok((
        200,
        json!({
            "ok": true,
            "seq": seq,
            "ack": {
                "seq": seq,
                "entry_id": entry_id,
                "kind": kind,
                "entry_sha256": entry_sha256,
            },
            "badge": badge,
            "view": view,
        }),
    ))
}

fn ensure_session_source(
    state: &DesktopState,
    env: &ResearchEnv,
    session_id: &str,
    val: &Value,
) -> Result<String, (u16, String)> {
    if let Some(sid) = non_empty_str(val.get("source_id")) {
        return Ok(sid.to_string());
    }

    let version = non_empty_str(val.get("version"));
    if let Some(raw_key) = non_empty_str(val.get("library_key")).or_else(|| non_empty_str(val.get("paper_key"))) {
        let parsed = PaperKey::parse(raw_key).map_err(bad)?;
        let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
        let canonical = lib.resolve_key(&parsed).map_err(internal)?.unwrap_or(parsed);

        let expected_origin = format!("library:{canonical}");
        let events = state
            .open_ledger()
            .map_err(internal)?
            .read_session(session_id)
            .map_err(|e| internal(e.to_string()))?;
        if let Some(existing) = events.iter().rev().find(|ev| {
            ev.kind == "source.ingested"
                && ev.payload.get("origin").and_then(Value::as_str) == Some(expected_origin.as_str())
        }) {
            if let Some(sid) = existing.payload.get("source_id").and_then(Value::as_str) {
                let _ = lib.record_seen(&SeenRecord {
                    key: canonical,
                    session_id: session_id.to_string(),
                    role: SeenRole::SolveSource,
                    seen_ms: now_ms(),
                });
                return Ok(sid.to_string());
            }
        }

        let ledger_mutex = Mutex::new(state.open_ledger().map_err(internal)?);
        let _lock = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("ledger write lock poisoned"))?;
        match lib.attach_to_session(&env.sandbox(), &ledger_mutex, session_id, &canonical, version, None) {
            Ok(ing) => {
                let _ = lib.record_seen(&SeenRecord {
                    key: canonical,
                    session_id: session_id.to_string(),
                    role: SeenRole::SolveSource,
                    seen_ms: now_ms(),
                });
                return Ok(ing.source_id);
            }
            Err(err) if err.starts_with("no cached full-text blob") => {
                if non_empty_str(val.get("arxiv_id")).is_none() {
                    return Err(not_found(err));
                }
            }
            Err(err) => return Err(internal(err)),
        }
    }

    if let Some(raw_arxiv) = non_empty_str(val.get("arxiv_id")).or_else(|| non_empty_str(val.get("paper"))) {
        let mut pref = parse_paper_input(raw_arxiv).map_err(bad)?;
        if pref.version.is_none() {
            pref.version = version.map(str::to_string);
        }
        match fetch(env, &pref, FetchWant::Source).map_err(bad)? {
            FetchOutcome::Source(fetched) => {
                let ledger_mutex = Mutex::new(state.open_ledger().map_err(internal)?);
                let _lock = state
                    .ledger_write_lock
                    .lock()
                    .map_err(|_| internal("ledger write lock poisoned"))?;
                let ing = fetched
                    .ingest_into_session(&env.sandbox(), &ledger_mutex, session_id, None)
                    .map_err(internal)?;
                if let Ok(lib) = Library::open(&state.cfg.data_dir) {
                    let _ = lib.record_seen(&SeenRecord {
                        key: fetched.paper.key,
                        session_id: session_id.to_string(),
                        role: SeenRole::SolveSource,
                        seen_ms: now_ms(),
                    });
                }
                return Ok(ing.source_id);
            }
            FetchOutcome::NeedsFile { reason, .. } => return Err(bad(reason)),
            other => return Err(internal(format!("unexpected fetch outcome `{}`", other.status_str()))),
        }
    }

    Err(bad(
        "attach-source requires `source_id`, `library_key`, or `arxiv_id`",
    ))
}

fn resolve_source_path(
    ingested_ev: &Event,
    source_id: &str,
    requested: Option<&str>,
) -> Result<String, (u16, String)> {
    let files: Vec<&str> = ingested_ev
        .payload
        .get("files")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let Some(req) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return ingested_ev
            .payload
            .get("primary_path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| files.first().map(|s| (*s).to_string()))
            .ok_or_else(|| bad(format!("source `{source_id}` has no primary readable file")));
    };

    if req.split_once('/').map(|(id, _)| id) == Some(source_id) {
        return Ok(req.to_string());
    }
    let prefixed = format!("{source_id}/{req}");
    if files.iter().any(|f| *f == prefixed) {
        return Ok(prefixed);
    }
    Ok(req.to_string())
}

fn detect_ocr_and_page(ingested_ev: &Event, start_line: usize, end_line: usize) -> (bool, Option<usize>) {
    let Some(raw_pages) = ingested_ev.payload.get("pdf_pages") else {
        let format_pdf = ingested_ev.payload.get("format").and_then(Value::as_str) == Some("pdf");
        let status_scan = matches!(
            ingested_ev.payload.get("status").and_then(Value::as_str),
            Some("scan_only" | "ocr")
        );
        return (format_pdf && status_scan, None);
    };
    let Ok(pages) = serde_json::from_value::<Vec<PageRecord>>(raw_pages.clone()) else {
        return (false, None);
    };
    let mut matched_page: Option<usize> = None;
    let mut any_scan = false;
    for pg in &pages {
        if let (Some(first), Some(last)) = (pg.first_line, pg.last_line) {
            if first <= end_line && last >= start_line {
                matched_page.get_or_insert(pg.page);
                if pg.basis.is_scan() {
                    any_scan = true;
                }
            }
        }
    }
    (any_scan, matched_page)
}

fn non_empty_str(val: Option<&Value>) -> Option<&str> {
    val.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}
