//! HTTP API handlers for user paper sources and sync (`/api/sources/..`, package F13a).
//!
//! Read-only against user folders and Kandinsky vaults: no file inside a user's library is
//! ever created, modified, moved, or deleted. No network calls and no model calls happen here.
//!
//! Registration note (`http_srv.rs` is over the 600-line ceiling and not edited by F13a):
//! add one delegating arm to `dispatch_api` in `crates/desktop-server/src/http_srv.rs`
//! (which already enforces `AuthGuard::authorize_api` header-token and `Origin` checks for
//! all `/api/` paths):
//! ```text
//! (_, ["api", "sources", rest @ ..]) => write_api_res(
//!     &mut stream,
//!     crate::routes_sources::handle_sources(state, method, rest, &req.query, &req.body),
//! ),
//! ```

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::library::{Library, SourceKind};
use research_tools::source_dedupe::find_duplicates;
use research_tools::source_sync::{
    register_source, remove_source, sync_all_sources, sync_source, SyncOptions,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// Single dispatcher for all `/api/sources/..` endpoints.
pub fn handle_sources(
    state: &DesktopState,
    method: &str,
    rest: &[&str],
    _query: &HashMap<String, String>,
    body: &[u8],
) -> ApiResult {
    match (method, rest) {
        ("GET", []) => handle_list_sources(state),
        ("POST", []) => handle_add_source(state, body),
        ("GET", ["duplicates"]) => handle_list_duplicates(state),
        ("POST", ["sync"]) => handle_sync_all(state, body),
        ("GET", [source_id]) => handle_get_source(state, source_id),
        ("DELETE", [source_id]) => handle_delete_source(state, source_id),
        ("POST", [source_id, "sync"]) => handle_sync_one(state, source_id, body),
        ("GET", [source_id, "items"]) => handle_list_source_items(state, source_id),
        _ => {
            let suffix = if rest.is_empty() {
                String::new()
            } else {
                format!("/{}", rest.join("/"))
            };
            Err(not_found(format!(
                "unknown sources endpoint: {method} /api/sources{suffix}"
            )))
        }
    }
}

/// `GET /api/sources`
pub fn handle_list_sources(state: &DesktopState) -> ApiResult {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let sources = lib.list_sources().map_err(internal)?;
    let mut rows = Vec::with_capacity(sources.len());
    for src in sources {
        let item_count = lib
            .list_source_items_for_source(&src.id)
            .map(|v| v.len())
            .unwrap_or(0);
        rows.push(json!({
            "source": src,
            "item_count": item_count,
        }));
    }
    Ok((200, json!({ "sources": rows, "count": rows.len() })))
}

/// `POST /api/sources`
///
/// Body: `{ "kind": "kandinsky_vault" | "folder", "root": "/path/to/dir",
///          "settings"?: {}, "id"?: "custom", "accept"?: false, "hash_files"?: false }`.
///
/// Follows `00-overview.md` 2.11 sync rule 5: by default (`accept: false`) registers the source
/// only if `accept: true` or runs a dry-run preview report first (`preview_only: true` when
/// `accept` is false and `dry_run` is true so nothing is committed to the index before acceptance).
pub fn handle_add_source(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let kind_raw = val
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`kind` is required (`kandinsky_vault` or `folder`)"))?;
    let kind = SourceKind::parse(kind_raw).map_err(bad)?;
    let root_raw = val
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`root` directory path is required"))?;
    let settings = val.get("settings").cloned().unwrap_or_else(|| json!({}));
    let custom_id = val
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let accept = val.get("accept").and_then(Value::as_bool).unwrap_or(false);
    let hash_files = val
        .get("hash_files")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let root_path = PathBuf::from(root_raw);
    let source = register_source(&lib, kind, &root_path, settings, custom_id).map_err(bad)?;
    let report = sync_source(
        &lib,
        &source.id,
        &SyncOptions {
            dry_run: !accept,
            hash_files,
        },
    )
    .map_err(bad)?;
    if !accept {
        // First-sync preview: remove the uncommitted pending source record until user accepts.
        let _ = remove_source(&lib, &source.id);
    }
    Ok((
        if accept { 201 } else { 200 },
        json!({
            "ok": true,
            "accepted": accept,
            "source": source,
            "report": report,
        }),
    ))
}

/// `GET /api/sources/{id}`
pub fn handle_get_source(state: &DesktopState, source_id: &str) -> ApiResult {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let Some(source) = lib
        .list_sources()
        .map_err(internal)?
        .into_iter()
        .find(|s| s.id == source_id)
    else {
        return Err(not_found(format!("paper source `{source_id}` not found")));
    };
    let items = lib
        .list_source_items_for_source(source_id)
        .map_err(internal)?;
    let dedupe = find_duplicates(&items);
    Ok((
        200,
        json!({
            "source": source,
            "items": items,
            "count": items.len(),
            "duplicates": dedupe,
        }),
    ))
}

/// `GET /api/sources/{id}/items`
pub fn handle_list_source_items(state: &DesktopState, source_id: &str) -> ApiResult {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let exists = lib
        .list_sources()
        .map_err(internal)?
        .iter()
        .any(|s| s.id == source_id);
    if !exists {
        return Err(not_found(format!("paper source `{source_id}` not found")));
    }
    let items = lib
        .list_source_items_for_source(source_id)
        .map_err(internal)?;
    Ok((
        200,
        json!({
            "source_id": source_id,
            "items": items,
            "count": items.len(),
        }),
    ))
}

/// `DELETE /api/sources/{id}`
pub fn handle_delete_source(state: &DesktopState, source_id: &str) -> ApiResult {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = remove_source(&lib, source_id).map_err(internal)?;
    if !deleted {
        return Err(not_found(format!("paper source `{source_id}` not found")));
    }
    Ok((200, json!({ "ok": true, "deleted": source_id })))
}

/// `POST /api/sources/{id}/sync`
pub fn handle_sync_one(state: &DesktopState, source_id: &str, body: &[u8]) -> ApiResult {
    let opts = parse_sync_opts(body)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let report = sync_source(&lib, source_id, &opts).map_err(|e| {
        if e.contains("not found") {
            not_found(e)
        } else {
            bad(e)
        }
    })?;
    Ok((200, json!({ "ok": true, "report": report })))
}

/// `POST /api/sources/sync`
pub fn handle_sync_all(state: &DesktopState, body: &[u8]) -> ApiResult {
    let opts = parse_sync_opts(body)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let reports = sync_all_sources(&lib, &opts).map_err(bad)?;
    Ok((
        200,
        json!({
            "ok": true,
            "reports": reports,
            "count": reports.len(),
        }),
    ))
}

/// `GET /api/sources/duplicates`
pub fn handle_list_duplicates(state: &DesktopState) -> ApiResult {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let items = lib.list_all_source_items().map_err(internal)?;
    let analysis = find_duplicates(&items);
    Ok((200, json!({ "duplicates": analysis })))
}

fn parse_sync_opts(body: &[u8]) -> Result<SyncOptions, (u16, String)> {
    if body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(SyncOptions::default());
    }
    let val = parse_json_body(body)?;
    let dry_run = val
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let hash_files = val
        .get("hash_files")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(SyncOptions {
        dry_run,
        hash_files,
    })
}
