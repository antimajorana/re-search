//! Integration tests for `/api/sources/..` route handlers (`routes_sources::handle_sources`)
//! and `AuthGuard::authorize_api` coverage of source endpoints (package F13a).

mod common;

use common::test_cfg;
use desktop_server::auth::{ApiCredentials, AuthGuard};
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::routes_sources::handle_sources;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn make_state(tmp: &TempDir) -> Arc<DesktopState> {
    let cfg = test_cfg(tmp.path(), None, None, None);
    let auth = AuthGuard::new(18_421).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    Arc::new(DesktopState::new(cfg, auth, sse, chatgpt))
}

fn empty_q() -> HashMap<String, String> {
    HashMap::new()
}

#[test]
fn sources_routes_preview_accept_list_sync_duplicates_and_delete() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let folder = tmp.path().join("my_papers");
    fs::create_dir_all(folder.join("algebra")).unwrap();
    fs::write(
        folder.join("algebra/2405.00101v1.tex"),
        "\\title{Synthetic Route TeX Paper}\n\\author{Ada Route}\n",
    )
    .unwrap();
    fs::write(
        folder.join("algebra/dup_copy.tex"),
        "% arXiv:2405.00101v2\n\\title{Synthetic Route TeX Paper}\n\\author{Ada Route}\n",
    )
    .unwrap();

    // 1. Preview (accept: false) returns report without committing source.
    let preview_body = serde_json::to_vec(&json!({
        "kind": "folder",
        "root": folder.to_string_lossy(),
        "accept": false,
    }))
    .unwrap();
    let (code, body) = handle_sources(&state, "POST", &[], &empty_q(), &preview_body).unwrap();
    assert_eq!(code, 200);
    assert_eq!(body["accepted"], Value::Bool(false));
    assert_eq!(body["report"]["dry_run"], Value::Bool(true));
    assert_eq!(body["report"]["total_items"], json!(2));

    let (code, list_before) = handle_sources(&state, "GET", &[], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(list_before["count"], json!(0));

    // 2. Accept (accept: true) registers and syncs.
    let accept_body = serde_json::to_vec(&json!({
        "kind": "folder",
        "root": folder.to_string_lossy(),
        "id": "my-folder",
        "accept": true,
    }))
    .unwrap();
    let (code, body) = handle_sources(&state, "POST", &[], &empty_q(), &accept_body).unwrap();
    assert_eq!(code, 201);
    assert_eq!(body["accepted"], Value::Bool(true));
    assert_eq!(body["source"]["id"], json!("my-folder"));
    assert_eq!(body["report"]["added"], json!(2));
    assert_eq!(body["report"]["exact_duplicates"], json!(1));

    // 3. GET /api/sources, /api/sources/{id}, /api/sources/{id}/items, /api/sources/duplicates
    let (code, list_after) = handle_sources(&state, "GET", &[], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(list_after["count"], json!(1));

    let (code, detail) =
        handle_sources(&state, "GET", &["my-folder"], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(detail["count"], json!(2));

    let (code, items) =
        handle_sources(&state, "GET", &["my-folder", "items"], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(items["count"], json!(2));

    let (code, dups) =
        handle_sources(&state, "GET", &["duplicates"], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(dups["duplicates"]["exact_groups"].as_array().unwrap().len(), 1);

    // 4. POST /api/sources/{id}/sync & POST /api/sources/sync (second sync changes nothing)
    let (code, resync) =
        handle_sources(&state, "POST", &["my-folder", "sync"], &empty_q(), b"{}").unwrap();
    assert_eq!(code, 200);
    assert_eq!(resync["report"]["unchanged"], json!(2));
    assert_eq!(resync["report"]["added"], json!(0));

    let (code, all_sync) =
        handle_sources(&state, "POST", &["sync"], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(all_sync["count"], json!(1));

    // 5. Unsupported F13b kind returns 400.
    let bad_kind = serde_json::to_vec(&json!({
        "kind": "zotero",
        "root": folder.to_string_lossy(),
        "accept": true,
    }))
    .unwrap();
    let err = handle_sources(&state, "POST", &[], &empty_q(), &bad_kind).unwrap_err();
    assert_eq!(err.0, 400);

    // 6. DELETE /api/sources/{id} removes index rows, leaves disk untouched.
    let (code, del) =
        handle_sources(&state, "DELETE", &["my-folder"], &empty_q(), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(del["deleted"], json!("my-folder"));
    assert!(folder.join("algebra/2405.00101v1.tex").is_file());
}

#[test]
fn auth_guard_protects_sources_endpoints() {
    let auth = AuthGuard::new(18_421).unwrap();
    let missing = ApiCredentials {
        method: "GET",
        path: "/api/sources",
        cookie: None,
        token_hdr: None,
        auth_hdr: None,
        origin: None,
        sec_fetch_site: None,
    };
    assert_eq!(auth.authorize_api(&missing).unwrap_err().0, 401);

    let valid = ApiCredentials {
        method: "GET",
        path: "/api/sources",
        cookie: None,
        token_hdr: Some(auth.token()),
        auth_hdr: None,
        origin: None,
        sec_fetch_site: None,
    };
    assert!(auth.authorize_api(&valid).is_ok());

    let cross_origin = ApiCredentials {
        method: "POST",
        path: "/api/sources",
        cookie: None,
        token_hdr: Some(auth.token()),
        auth_hdr: None,
        origin: Some("https://evil.example"),
        sec_fetch_site: None,
    };
    assert_eq!(auth.authorize_api(&cross_origin).unwrap_err().0, 403);
}
