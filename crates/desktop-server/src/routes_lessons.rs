//! HTTP API handlers for F16a corrections reused by code (`/api/lessons/..`).
//!
//! Backed by `<data_dir>/library/lessons.db`. Zero model calls, zero network calls, and
//! nothing here ever alters a badge, tier, verdict, or gate (`00-overview.md` 2.14).
//!
//! Registration note (`http_srv.rs` is over the 600-line ceiling and not edited by F16a):
//! add one delegating arm to `dispatch_api` in `crates/desktop-server/src/http_srv.rs`
//! (which already enforces `AuthGuard::authorize_api` header-token and `Origin` checks for
//! all `/api/` paths):
//! ```text
//! (_, ["api", "lessons", rest @ ..]) => write_api_res(
//!     &mut stream,
//!     crate::routes_lessons::handle_lessons(state, method, rest, &req.query, &req.body),
//! ),
//! ```

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::lessons::{
    ConventionDefault, LessonsStore, OcrOverlay, SkeletonOverride, SymbolDefault,
    TermAliasDecision, TermAliasRecord, LESSONS_SCHEMA_VERSION, OCR_OVERLAY_LABEL,
};
use research_tools::lessons_select::{
    expand_query_aliases, is_pair_rejected, suggest_solve_defaults,
};
use research_tools::paper_key::PaperKey;
use research_tools::paper_sources::now_ms;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Single dispatcher for all `/api/lessons/..` endpoints.
pub fn handle_lessons(
    state: &DesktopState,
    method: &str,
    rest: &[&str],
    query: &HashMap<String, String>,
    body: &[u8],
) -> ApiResult {
    match (method, rest) {
        ("GET", []) => handle_overview(state),
        ("POST", ["ingest", session_id]) => handle_ingest_session(state, session_id),
        ("GET", ["aliases"]) => handle_list_aliases(state, query),
        ("POST", ["aliases"]) => handle_upsert_alias(state, body),
        ("DELETE", ["aliases"]) => handle_delete_alias(state, body),
        ("GET", ["defaults"]) => handle_suggest_defaults(state, query),
        ("POST", ["defaults", "symbols"]) => handle_upsert_symbol_default(state, body),
        ("DELETE", ["defaults", "symbols", symbol]) => {
            handle_delete_symbol_default(state, symbol, query)
        }
        ("POST", ["defaults", "conventions"]) => handle_upsert_convention_default(state, body),
        ("DELETE", ["defaults", "conventions", key]) => {
            handle_delete_convention_default(state, key, query)
        }
        ("GET", ["skeleton-overrides"]) => handle_list_skeleton_overrides(state),
        ("POST", ["skeleton-overrides"]) => handle_upsert_skeleton_override(state, body),
        ("DELETE", ["skeleton-overrides", node_tag]) => {
            handle_delete_skeleton_override(state, node_tag)
        }
        ("GET", ["ocr-overlays"]) => handle_list_ocr_overlays(state, query),
        ("POST", ["ocr-overlays"]) => handle_upsert_ocr_overlay(state, body),
        ("DELETE", ["ocr-overlays"]) => handle_delete_ocr_overlay(state, body),
        ("GET", ["backup"]) => handle_export_backup(state, query),
        ("POST", ["restore"]) => handle_restore_backup(state, body),
        _ => {
            let suffix = if rest.is_empty() {
                String::new()
            } else {
                format!("/{}", rest.join("/"))
            };
            Err(not_found(format!(
                "unknown lessons endpoint: {method} /api/lessons{suffix}"
            )))
        }
    }
}

/// `GET /api/lessons`
pub fn handle_overview(state: &DesktopState) -> ApiResult {
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    Ok((
        200,
        json!({
            "schema_version": LESSONS_SCHEMA_VERSION,
            "term_aliases": store.list_term_aliases().map_err(internal)?,
            "symbol_defaults": store.list_symbol_defaults().map_err(internal)?,
            "convention_defaults": store.list_convention_defaults().map_err(internal)?,
            "skeleton_overrides": store.list_skeleton_overrides().map_err(internal)?,
            "ocr_overlays": store.list_ocr_overlays(None).map_err(internal)?,
        }),
    ))
}

/// `POST /api/lessons/ingest/{session_id}`
pub fn handle_ingest_session(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let report = store
        .ingest_session_events(session_id, &events)
        .map_err(internal)?;
    Ok((200, json!({ "ok": true, "report": report })))
}

/// `GET /api/lessons/aliases?expand=term1,term2` or `?check_a=...&check_b=...`
pub fn handle_list_aliases(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let aliases = store.list_term_aliases().map_err(internal)?;
    let expanded = query.get("expand").map(|raw| {
        let terms: Vec<&str> = raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        expand_query_aliases(&store, &terms)
    });
    let expanded = match expanded {
        Some(Ok(v)) => Some(v),
        Some(Err(e)) => return Err(internal(e)),
        None => None,
    };
    let rejected = match (query.get("check_a"), query.get("check_b")) {
        (Some(a), Some(b)) => Some(is_pair_rejected(&store, a, b).map_err(internal)?),
        _ => None,
    };
    Ok((
        200,
        json!({
            "aliases": aliases,
            "expanded": expanded,
            "pair_rejected": rejected,
        }),
    ))
}

/// `POST /api/lessons/aliases`
pub fn handle_upsert_alias(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let term_a = req_str(&val, "term_a")?;
    let term_b = req_str(&val, "term_b")?;
    let decision = TermAliasDecision::parse(&req_str(&val, "decision")?).map_err(bad)?;
    let origin_session = opt_str(&val, "origin_session").unwrap_or_default();
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let rec = TermAliasRecord {
        term_a,
        term_b,
        decision,
        origin_session,
        origin_node: None,
        updated_ms: now_ms(),
    };
    store.record_term_alias(&rec).map_err(bad)?;
    Ok((200, json!({ "ok": true })))
}

/// `DELETE /api/lessons/aliases`
pub fn handle_delete_alias(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let term_a = req_str(&val, "term_a")?;
    let term_b = req_str(&val, "term_b")?;
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = store.delete_term_alias(&term_a, &term_b).map_err(bad)?;
    Ok((200, json!({ "ok": true, "deleted": deleted })))
}

/// `GET /api/lessons/defaults?symbols=m,g&domain=qft`
pub fn handle_suggest_defaults(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let domain = query
        .get("domain")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let syms_vec: Vec<&str> = query
        .get("symbols")
        .map(|raw| raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let suggested = suggest_solve_defaults(&store, &syms_vec, domain).map_err(internal)?;
    Ok((200, json!({ "defaults": suggested })))
}

/// `POST /api/lessons/defaults/symbols`
pub fn handle_upsert_symbol_default(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let symbol = req_str(&val, "symbol")?;
    let domain = opt_str(&val, "domain").unwrap_or_default();
    let assumptions = val
        .get("assumptions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    store
        .record_symbol_default(&SymbolDefault {
            symbol,
            domain,
            assumptions,
            dimensions: opt_str(&val, "dimensions"),
            convention_note: opt_str(&val, "convention_note"),
            origin_session: opt_str(&val, "origin_session").unwrap_or_default(),
            updated_ms: now_ms(),
        })
        .map_err(bad)?;
    Ok((200, json!({ "ok": true })))
}

/// `DELETE /api/lessons/defaults/symbols/{symbol}?domain=...`
pub fn handle_delete_symbol_default(
    state: &DesktopState,
    symbol: &str,
    query: &HashMap<String, String>,
) -> ApiResult {
    let domain = query.get("domain").map(String::as_str).unwrap_or("");
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = store
        .delete_symbol_default(symbol, domain)
        .map_err(internal)?;
    Ok((200, json!({ "ok": true, "deleted": deleted })))
}

/// `POST /api/lessons/defaults/conventions`
pub fn handle_upsert_convention_default(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let key = req_str(&val, "key")?;
    let value = req_str(&val, "value")?;
    let domain = opt_str(&val, "domain").unwrap_or_default();
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    store
        .record_convention_default(&ConventionDefault {
            key,
            value,
            domain,
            origin_session: opt_str(&val, "origin_session").unwrap_or_default(),
            updated_ms: now_ms(),
        })
        .map_err(bad)?;
    Ok((200, json!({ "ok": true })))
}

/// `DELETE /api/lessons/defaults/conventions/{key}?domain=...`
pub fn handle_delete_convention_default(
    state: &DesktopState,
    key: &str,
    query: &HashMap<String, String>,
) -> ApiResult {
    let domain = query.get("domain").map(String::as_str).unwrap_or("");
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = store
        .delete_convention_default(key, domain)
        .map_err(internal)?;
    Ok((200, json!({ "ok": true, "deleted": deleted })))
}

/// `GET /api/lessons/skeleton-overrides`
pub fn handle_list_skeleton_overrides(state: &DesktopState) -> ApiResult {
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let overrides = store.list_skeleton_overrides().map_err(internal)?;
    Ok((200, json!({ "overrides": overrides })))
}

/// `POST /api/lessons/skeleton-overrides`
pub fn handle_upsert_skeleton_override(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let node_tag = req_str(&val, "node_tag")?;
    let paper_key = opt_str(&val, "paper_key")
        .map(|k| PaperKey::parse(&k))
        .transpose()
        .map_err(bad)?;
    let end_line_override = val
        .get("end_line_override")
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    store
        .record_skeleton_override(&SkeletonOverride {
            node_tag,
            paper_key,
            kind_override: opt_str(&val, "kind_override"),
            end_line_override,
            note: opt_str(&val, "note").unwrap_or_default(),
            origin_session: opt_str(&val, "origin_session").unwrap_or_default(),
            updated_ms: now_ms(),
        })
        .map_err(bad)?;
    Ok((200, json!({ "ok": true })))
}

/// `DELETE /api/lessons/skeleton-overrides/{node_tag}`
pub fn handle_delete_skeleton_override(state: &DesktopState, node_tag: &str) -> ApiResult {
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = store
        .delete_skeleton_override(node_tag)
        .map_err(internal)?;
    Ok((200, json!({ "ok": true, "deleted": deleted })))
}

/// `GET /api/lessons/ocr-overlays?file_sha256=...`
pub fn handle_list_ocr_overlays(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let sha = query
        .get("file_sha256")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty());
    let overlays = store.list_ocr_overlays(sha).map_err(internal)?;
    Ok((200, json!({ "overlays": overlays })))
}

/// `POST /api/lessons/ocr-overlays`
pub fn handle_upsert_ocr_overlay(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let file_sha256 = req_str(&val, "file_sha256")?;
    let line_no = val
        .get("line_no")
        .and_then(Value::as_u64)
        .filter(|&n| n >= 1)
        .ok_or_else(|| bad("`line_no` must be a positive integer (>= 1)"))? as usize;
    let corrected_text = req_str(&val, "corrected_text")?;
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    store
        .record_ocr_overlay(&OcrOverlay {
            file_sha256,
            line_no,
            corrected_text,
            original_sha256: opt_str(&val, "original_sha256"),
            origin_session: opt_str(&val, "origin_session").unwrap_or_default(),
            updated_ms: now_ms(),
            label: OCR_OVERLAY_LABEL.to_string(),
        })
        .map_err(bad)?;
    Ok((200, json!({ "ok": true })))
}

/// `DELETE /api/lessons/ocr-overlays`
pub fn handle_delete_ocr_overlay(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let file_sha256 = req_str(&val, "file_sha256")?;
    let line_no = val
        .get("line_no")
        .and_then(Value::as_u64)
        .filter(|&n| n >= 1)
        .ok_or_else(|| bad("`line_no` must be a positive integer (>= 1)"))? as usize;
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = store
        .delete_ocr_overlay(&file_sha256, line_no)
        .map_err(internal)?;
    Ok((200, json!({ "ok": true, "deleted": deleted })))
}

/// `GET /api/lessons/backup?confirm=true` (opt-in export per `00-overview.md` 2.14)
pub fn handle_export_backup(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    if query.get("confirm").map(String::as_str) != Some("true") {
        return Err(bad(
            "lessons export is opt-in: pass `?confirm=true` to export user corrections",
        ));
    }
    let store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let backup = store.export_backup().map_err(internal)?;
    let body = serde_json::to_value(backup).map_err(|e| internal(e.to_string()))?;
    Ok((200, body))
}

/// `POST /api/lessons/restore`
pub fn handle_restore_backup(state: &DesktopState, body: &[u8]) -> ApiResult {
    let mut store = LessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let restored = store.restore_backup_bytes(body).map_err(bad)?;
    Ok((200, json!({ "ok": true, "restored": restored })))
}

fn req_str(val: &Value, key: &str) -> Result<String, (u16, String)> {
    val.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| bad(format!("`{key}` is required and must be a non-empty string")))
}

fn opt_str(val: &Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
