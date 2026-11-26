//! HTTP API handlers for the shared paper library (`/api/library/..`, package F10).
//!
//! No network calls and no model calls happen here: every read queries `<data_dir>/library/library.db`,
//! user-mark writes mirror to `<data_dir>/library/user_marks.json` (+ `.bak`), `attach` re-ingests
//! verified bytes into a session via `ingest_bytes`, and `rebuild` reconstructs the index from
//! `<data_dir>/ledger.db` and `<data_dir>/blobs/`.
//!
//! Registration note (`http_srv.rs` is over the 600-line ceiling and not edited by F10):
//! add one delegating arm to `dispatch_api` in `crates/desktop-server/src/http_srv.rs`
//! (which already enforces `AuthGuard::authorize_api` header-token and `Origin` checks for
//! all `/api/` paths):
//! ```text
//! (_, ["api", "library", rest @ ..]) => write_api_res(
//!     &mut stream,
//!     crate::routes_library::handle_library(state, method, rest, &req.query, &req.body),
//! ),
//! ```

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::library::{
    Library, LibraryQuery, MarkStatus, SeenRole, DEFAULT_LIBRARY_LIMIT, LIBRARY_SCHEMA_VERSION,
    MAX_LIBRARY_LIMIT,
};
use research_tools::paper_key::PaperKey;
use research_tools::{validate_session_id, WorkspaceSandbox};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// Single dispatcher for all `/api/library/..` endpoints.
///
/// `rest` contains the path segments after `/api/library`. Because `http_srv.rs`
/// percent-decodes the request target before splitting on `'/'`, keys that contain `'/'`
/// (such as `doi:10.1000/xyz` or `arxiv:hep-th/9901001`) arrive split across multiple
/// segments and are re-joined with `'/'`.
pub fn handle_library(
    state: &DesktopState,
    method: &str,
    rest: &[&str],
    query: &HashMap<String, String>,
    body: &[u8],
) -> ApiResult {
    match (method, rest) {
        ("GET", []) => handle_list_or_search(state, query),
        ("POST", ["rebuild"]) => handle_rebuild(state),
        ("GET", ["marks", "backup"]) => handle_export_marks_backup(state),
        ("POST", ["marks", "restore"]) => handle_restore_marks_backup(state, body),
        ("POST", [key_parts @ .., "mark"]) if !key_parts.is_empty() => {
            let raw_key = key_parts.join("/");
            handle_set_mark(state, &raw_key, body)
        }
        ("DELETE", [key_parts @ .., "mark"]) if !key_parts.is_empty() => {
            let raw_key = key_parts.join("/");
            handle_delete_mark(state, &raw_key)
        }
        ("POST", [key_parts @ .., "attach"]) if !key_parts.is_empty() => {
            let raw_key = key_parts.join("/");
            handle_attach(state, &raw_key, body)
        }
        ("GET", key_parts) if !key_parts.is_empty() => {
            let raw_key = key_parts.join("/");
            handle_get_paper(state, &raw_key)
        }
        ("DELETE", key_parts) if !key_parts.is_empty() => {
            let raw_key = key_parts.join("/");
            handle_delete_paper(state, &raw_key)
        }
        _ => {
            let suffix = if rest.is_empty() {
                String::new()
            } else {
                format!("/{}", rest.join("/"))
            };
            Err(not_found(format!(
                "unknown library endpoint: {method} /api/library{suffix}"
            )))
        }
    }
}

/// `GET /api/library?q=&title=&author=&year=&key=&status=&role=&limit=&offset=`
pub fn handle_list_or_search(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let q = non_empty(query.get("q"));
    let title = non_empty(query.get("title"));
    let author = non_empty(query.get("author"));

    let year = match non_empty(query.get("year")) {
        Some(raw) => Some(
            raw.parse::<i32>()
                .map_err(|_| bad(format!("invalid `year` parameter `{raw}`")))?,
        ),
        None => None,
    };
    let key = match non_empty(query.get("key")) {
        Some(raw) => Some(PaperKey::parse(&raw).map_err(bad)?),
        None => None,
    };
    let status = match non_empty(query.get("status")) {
        Some(raw) => Some(MarkStatus::parse(&raw).map_err(bad)?),
        None => None,
    };
    let role = match non_empty(query.get("role")) {
        Some(raw) => Some(SeenRole::parse(&raw).map_err(bad)?),
        None => None,
    };
    let limit = match non_empty(query.get("limit")) {
        Some(raw) => {
            let parsed = raw
                .parse::<usize>()
                .map_err(|_| bad(format!("invalid `limit` parameter `{raw}`")))?;
            if parsed == 0 {
                return Err(bad("`limit` must be at least 1"));
            }
            parsed.min(MAX_LIBRARY_LIMIT)
        }
        None => DEFAULT_LIBRARY_LIMIT,
    };
    let offset = match non_empty(query.get("offset")) {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| bad(format!("invalid `offset` parameter `{raw}`")))?,
        None => 0,
    };

    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let papers = lib
        .search(&LibraryQuery {
            q,
            title,
            author,
            year,
            key,
            status,
            role,
            limit: Some(limit),
            offset,
        })
        .map_err(internal)?;
    Ok((
        200,
        json!({
            "papers": papers,
            "count": papers.len(),
            "limit": limit,
            "offset": offset,
            "schema_version": LIBRARY_SCHEMA_VERSION,
        }),
    ))
}

/// `GET /api/library/{key}`
pub fn handle_get_paper(state: &DesktopState, raw_key: &str) -> ApiResult {
    let key = PaperKey::parse(raw_key.trim()).map_err(bad)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let Some(detail) = lib.get_paper(&key).map_err(internal)? else {
        return Err(not_found(format!("paper `{key}` not found in library")));
    };
    let body = serde_json::to_value(detail).map_err(|e| internal(e.to_string()))?;
    Ok((200, body))
}

/// `POST /api/library/{key}/mark`
pub fn handle_set_mark(state: &DesktopState, raw_key: &str, body: &[u8]) -> ApiResult {
    let key = PaperKey::parse(raw_key.trim()).map_err(bad)?;
    let val = parse_json_body(body)?;

    let status_raw = val
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`status` is required (unread, skimmed, key, or irrelevant)"))?;
    let status = MarkStatus::parse(status_raw).map_err(bad)?;

    let tags = match val.get("tags") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = item
                    .as_str()
                    .ok_or_else(|| bad("`tags` entries must be strings"))?;
                out.push(s.to_string());
            }
            out
        }
        Some(_) => return Err(bad("`tags` must be an array of strings")),
    };
    let note = match val.get("note") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(bad("`note` must be a string")),
    };

    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let mark = lib
        .set_user_mark(&key, status, tags, note)
        .map_err(bad)?;
    Ok((200, json!({ "ok": true, "mark": mark })))
}

/// `DELETE /api/library/{key}/mark`
pub fn handle_delete_mark(state: &DesktopState, raw_key: &str) -> ApiResult {
    let key = PaperKey::parse(raw_key.trim()).map_err(bad)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = lib.delete_user_mark(&key).map_err(internal)?;
    Ok((200, json!({ "ok": true, "deleted": deleted, "key": key })))
}

/// `POST /api/library/{key}/attach`
pub fn handle_attach(state: &DesktopState, raw_key: &str, body: &[u8]) -> ApiResult {
    let key = PaperKey::parse(raw_key.trim()).map_err(bad)?;
    let val = parse_json_body(body)?;
    let session_id = val
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`session_id` is required"))?;
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let version = val
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let section_filter = val
        .get("section_filter")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let sandbox = WorkspaceSandbox::new(env.data_dir.clone(), env.workspace_root.clone());
    let ledger = Mutex::new(state.open_ledger().map_err(internal)?);
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let ingested = lib
        .attach_to_session(&sandbox, &ledger, session_id, &key, version, section_filter)
        .map_err(|e| {
            if e.starts_with("no cached full-text blob") {
                not_found(e)
            } else {
                bad(e)
            }
        })?;
    Ok((200, json!({ "ok": true, "ingested": ingested })))
}

/// `DELETE /api/library/{key}`
pub fn handle_delete_paper(state: &DesktopState, raw_key: &str) -> ApiResult {
    let key = PaperKey::parse(raw_key.trim()).map_err(bad)?;
    let mut lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = lib.delete_paper(&key).map_err(internal)?;
    if !deleted {
        return Err(not_found(format!("paper `{key}` not found in library")));
    }
    Ok((200, json!({ "ok": true, "deleted": key })))
}

/// `POST /api/library/rebuild`
pub fn handle_rebuild(state: &DesktopState) -> ApiResult {
    let mut lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let report = lib.rebuild().map_err(internal)?;
    Ok((200, json!({ "ok": true, "report": report })))
}

/// `GET /api/library/marks/backup`
pub fn handle_export_marks_backup(state: &DesktopState) -> ApiResult {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let backup = lib.export_user_marks_backup().map_err(internal)?;
    let body = serde_json::to_value(backup).map_err(|e| internal(e.to_string()))?;
    Ok((200, body))
}

/// `POST /api/library/marks/restore`
pub fn handle_restore_marks_backup(state: &DesktopState, body: &[u8]) -> ApiResult {
    let mut lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let restored = lib.restore_user_marks_backup(body).map_err(bad)?;
    Ok((200, json!({ "ok": true, "restored": restored })))
}

fn non_empty(opt: Option<&String>) -> Option<String> {
    opt.map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
