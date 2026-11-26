//! Desktop HTTP routes for cross-mode hand-overs (package F7, `00-overview.md` section 4).
//!
//! Enforces the five receiving-route rules of `00-overview.md` 4.1:
//! 1. Nothing arrives as checked: every `QuoteRef` is re-matched in the child against its own
//!    copied files and `source.ingested` receipts (`export::handoff::execute_handoff`).
//! 2. Badges and tiers never travel (`#[serde(deny_unknown_fields)]` on `HandoffPacket` and
//!    `HandoffRequest`; Solve prefill creates only `Cited` or `Given` steps).
//! 3. Packet text is data (consumers wrap it with `export::handoff::wrap_packet_as_quoted_data`).
//! 4. Zero model calls and zero network calls; returns the deterministic target setup prefill so
//!    the user can inspect and edit it before any run.
//! 5. Result flows back by reading, never writing (`GET /api/sessions/{id}/handoffs` reads each
//!    child's latest status without modifying the origin session).

use crate::routes_session::{bad, internal, not_found, ApiResult};
use crate::session_mode::{api_mode_name, is_mode_built, mode_label};
use crate::state::{now_ms, DesktopState};
use export::handoff::{
    build_prefill, execute_handoff, read_session_handoffs, HandoffRequest,
};
use research_tools::validate_session_id;
use serde_json::json;

/// `POST /api/sessions/{id}/handoff`: execute a verified cross-mode hand-over from `session_id`.
///
/// Creates a new child session with `[source.ingested..., handoff.packet, <target_marker>]`,
/// records `handoff.sent` in `session_id`, writes child `SessionMeta`, attaches the child to
/// the origin session's project if any, and returns the deterministic target setup prefill.
/// Costs zero model calls and zero network calls.
pub fn handle_create_handoff(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    if body.is_empty() {
        return Err(bad("handoff request body is required"));
    }
    let req: HandoffRequest =
        serde_json::from_slice(body).map_err(|e| bad(format!("invalid handoff packet: {e}")))?;
    if req.packet.from.session != session_id {
        return Err(bad(format!(
            "handoff packet `from.session` (`{}`) must match route session `{session_id}`",
            req.packet.from.session
        )));
    }

    let receipt = execute_handoff(&state.cfg.data_dir, &req).map_err(bad)?;
    let prefill = build_prefill(&receipt.packet, receipt.quotes_dropped).map_err(internal)?;

    let mut meta = state.load_meta(session_id).unwrap_or_default();
    meta.id = receipt.child_session.clone();
    meta.title = receipt.packet.title.chars().take(80).collect();
    meta.allow_model_title = Some(false);
    meta.archived = false;
    meta.fork_info = None;
    meta.created_ms = now_ms();
    state.save_meta(&meta).map_err(internal)?;

    if let Some(project) = crate::projects::project_for_session(state, session_id)? {
        crate::projects::attach_session(state, &project.id, &receipt.child_session)?;
    }

    Ok((
        201,
        json!({
            "session_id": receipt.child_session,
            "mode": api_mode_name(receipt.target_mode),
            "mode_label": mode_label(receipt.target_mode),
            "target_mode_built": is_mode_built(receipt.target_mode),
            "receipt": receipt,
            "prefill": prefill,
        }),
    ))
}

/// `GET /api/sessions/{id}/handoffs`: read incoming hand-off packet/prefill and live read-only
/// status of every outgoing hand-off child session (Rule 5: strictly read-only).
pub fn handle_get_handoffs(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let view = read_session_handoffs(&ledger, session_id).map_err(internal)?;
    let value = serde_json::to_value(&view).map_err(|e| internal(e.to_string()))?;
    Ok((200, value))
}
