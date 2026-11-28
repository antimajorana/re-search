//! HTTP route handler for `POST /api/sessions/{id}/idea/read` (packages X9–X10).
//!
//! Kept in a sibling file so `routes_idea.rs` stays strictly <= 600 lines.

use crate::idea_read::{run_reading_queue_with_router, ReadQueueResult};
use crate::idea_view;
use crate::routers::build_idea_router;
use crate::routes_idea::ActiveExploreGuard;
use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::{decode_base64, DesktopState};
use ledger::Ledger;
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::MAX_USER_PDF_BYTES;
use serde_json::{json, Value};
use std::sync::Arc;

/// `POST /api/sessions/{id}/idea/read`:
/// - If `attach_pdf` is present: attaches a user-uploaded PDF/TeX file to `paper_key`
///   strictly through `ingest_bytes` / `ingest_pdf_pages` (Rule C9, zero network).
/// - Otherwise: runs the reading queue through `build_idea_router` -> `Router::try_call`
///   (`idea_fetch_papers`, Rule P5) and returns updated `idea_view`.
pub fn handle_read_op(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let val = if body.is_empty() {
        json!({})
    } else {
        parse_json_body(body)?
    };

    if let Some(attach) = val.get("attach_pdf").filter(|v| !v.is_null()) {
        return handle_attach_pdf(state, ledger, session_id, attach);
    }

    let explicit_keys = parse_optional_keys(val.get("keys"))?;
    let env = Arc::new(state.research_env_for_session(session_id).map_err(bad)?);
    let router = build_idea_router(Arc::clone(state), env, session_id).map_err(internal)?;
    let guard = ActiveExploreGuard::acquire(state, session_id)?;

    let sse = Arc::clone(&state.sse);
    let sid = session_id.to_string();
    let res = run_reading_queue_with_router(
        ledger,
        &router,
        session_id,
        explicit_keys,
        Some(&guard.cancel),
        move |kind, payload| {
            sse.emit(
                &sid,
                "session_event",
                json!({ "session": sid, "kind": kind, "payload": payload }),
            );
        },
    );
    drop(guard);

    let outcome = res.map_err(|e| {
        if e.contains("outside stage") || e.contains("Quick preset") || e.contains("reading list is empty") {
            bad(e)
        } else {
            internal(e)
        }
    })?;
    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let view = idea_view::view(&updated, false);

    match outcome {
        ReadQueueResult::Finished { papers, ready_count, unavailable_count } => Ok((
            200,
            json!({
                "ok": true,
                "status": "finished",
                "papers": papers,
                "ready_count": ready_count,
                "unavailable_count": unavailable_count,
                "idea": view,
            }),
        )),
        ReadQueueResult::Denied { reason } => Ok((
            200,
            json!({
                "ok": false,
                "status": "denied",
                "reason": reason,
                "idea": view,
            }),
        )),
        ReadQueueResult::StoppedAtCap { reason, papers } => Ok((
            200,
            json!({
                "ok": true,
                "status": "stopped_at_cap",
                "reason": reason,
                "papers": papers,
                "idea": view,
            }),
        )),
        ReadQueueResult::Cancelled { papers } => Ok((
            200,
            json!({
                "ok": false,
                "status": "cancelled",
                "papers": papers,
                "idea": view,
            }),
        )),
    }
}

fn handle_attach_pdf(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    attach: &Value,
) -> ApiResult {
    let obj = attach
        .as_object()
        .ok_or_else(|| bad("`attach_pdf` must be an object"))?;
    let raw_key = obj
        .get("paper_key")
        .or_else(|| obj.get("key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`attach_pdf.paper_key` must be a non-empty string"))?;
    let paper_key = PaperKey::parse(raw_key).map_err(|e| bad(format!("invalid paper_key: {e}")))?;

    let existing_source_id = obj
        .get("source_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let decoded_upload = if existing_source_id.is_none() {
        let file_name = obj
            .get("file_name")
            .or_else(|| obj.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad("`attach_pdf.file_name` is required when `source_id` is omitted"))?;
        let b64 = obj
            .get("bytes_base64")
            .or_else(|| obj.get("content_base64"))
            .and_then(Value::as_str)
            .ok_or_else(|| bad("`attach_pdf.bytes_base64` is required when `source_id` is omitted"))?;
        let bytes = decode_base64(b64).map_err(|e| bad(format!("invalid base64 payload: {e}")))?;
        if bytes.is_empty() {
            return Err(bad("attached PDF file is empty"));
        }
        if bytes.len() > MAX_USER_PDF_BYTES {
            return Err((413, format!("attached PDF exceeds {} byte limit", MAX_USER_PDF_BYTES)));
        }
        Some((file_name.to_string(), bytes))
    } else {
        None
    };

    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let _guard = ActiveExploreGuard::acquire(state, session_id)?;
    let sse = Arc::clone(&state.sse);
    let sid = session_id.to_string();
    let paper = crate::idea_read::prepare_attached_pdf(
        &env,
        ledger,
        session_id,
        &paper_key,
        existing_source_id,
        decoded_upload.as_ref().map(|(n, b)| (n.as_str(), b.as_slice())),
        move |kind, payload| {
            sse.emit(
                &sid,
                "session_event",
                json!({ "session": sid, "kind": kind, "payload": payload }),
            );
        },
    )
    .map_err(bad)?;

    {
        let _lock = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("ledger lock poisoned"))?;
        let sse2 = Arc::clone(&state.sse);
        let sid2 = session_id.to_string();
        crate::idea_read::append_idea_paper(ledger, session_id, &paper, &mut move |kind, payload| {
            sse2.emit(
                &sid2,
                "session_event",
                json!({ "session": sid2, "kind": kind, "payload": payload }),
            );
        })
        .map_err(internal)?;
    }

    drop(_guard);
    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    Ok((
        200,
        json!({
            "ok": true,
            "paper": paper,
            "idea": idea_view::view(&updated, false),
        }),
    ))
}

fn parse_optional_keys(val: Option<&Value>) -> Result<Option<Vec<PaperKey>>, (u16, String)> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| bad("`keys` must be an array of canonical PaperKey strings"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad("each entry in `keys` must be a non-empty string"))?;
        out.push(PaperKey::parse(s).map_err(|e| bad(format!("invalid PaperKey `{s}`: {e}")))?);
    }
    Ok(Some(out))
}
