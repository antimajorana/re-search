//! Integration tests for `/api/lessons/..` route handlers (`routes_lessons::handle_lessons`)
//! and `AuthGuard::authorize_api` coverage of lessons endpoints (package F16a).

mod common;

use common::test_cfg;
use desktop_server::auth::{ApiCredentials, AuthGuard};
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::routes_lessons::handle_lessons;
use desktop_server::routes_session::handle_create_session;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use research_tools::lessons::EVENT_IDEA_ALIAS_CONFIRMED;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn make_state(tmp: &TempDir) -> Arc<DesktopState> {
    let cfg = test_cfg(tmp.path(), None, None, None);
    let auth = AuthGuard::new(18_422).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    Arc::new(DesktopState::new(cfg, auth, sse, chatgpt))
}

fn qmap(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn lessons_routes_crud_ingest_defaults_overlays_and_opt_in_backup() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    // Create a real session and append an existing alias confirmation event.
    let (st, created) = handle_create_session(&state, b"{}").unwrap();
    assert_eq!(st, 200);
    let sid = created["id"].as_str().unwrap().to_string();
    {
        let ledger = state.open_ledger().unwrap();
        ledger
            .append(
                &sid,
                EVENT_IDEA_ALIAS_CONFIRMED,
                1,
                &json!({ "term_a": "Winding Number", "term_b": "Topological Charge" }),
            )
            .unwrap();
    }

    // 1. Ingest corrections from session ledger.
    let (code, ing) =
        handle_lessons(&state, "POST", &["ingest", &sid], &qmap(&[]), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(ing["report"]["term_aliases_recorded"], json!(1));

    // 2. Query alias expansion.
    let (code, aliases) = handle_lessons(
        &state,
        "GET",
        &["aliases"],
        &qmap(&[("expand", "winding number")]),
        b"",
    )
    .unwrap();
    assert_eq!(code, 200);
    assert_eq!(
        aliases["expanded"],
        json!(["winding number", "topological charge"])
    );

    // 3. Upsert symbol & convention defaults, then suggest defaults.
    let sym_body = serde_json::to_vec(&json!({
        "symbol": "g",
        "domain": "qft",
        "assumptions": ["real", "positive"],
        "dimensions": "dimensionless"
    }))
    .unwrap();
    let (code, _) =
        handle_lessons(&state, "POST", &["defaults", "symbols"], &qmap(&[]), &sym_body).unwrap();
    assert_eq!(code, 200);

    let conv_body = serde_json::to_vec(&json!({
        "key": "metric",
        "value": "(+,-,-,-)",
        "domain": "qft"
    }))
    .unwrap();
    let (code, _) = handle_lessons(
        &state,
        "POST",
        &["defaults", "conventions"],
        &qmap(&[]),
        &conv_body,
    )
    .unwrap();
    assert_eq!(code, 200);

    let (code, sug) = handle_lessons(
        &state,
        "GET",
        &["defaults"],
        &qmap(&[("symbols", "g"), ("domain", "qft")]),
        b"",
    )
    .unwrap();
    assert_eq!(code, 200);
    assert_eq!(sug["defaults"]["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(sug["defaults"]["conventions"].as_array().unwrap().len(), 1);

    // 4. Skeleton override & OCR overlay CRUD.
    let skel_body = serde_json::to_vec(&json!({
        "node_tag": "n-aabbccddee",
        "kind_override": "theorem",
        "end_line_override": 33,
        "note": "fixed span"
    }))
    .unwrap();
    assert_eq!(
        handle_lessons(&state, "POST", &["skeleton-overrides"], &qmap(&[]), &skel_body)
            .unwrap()
            .0,
        200
    );

    let sha = "d".repeat(64);
    let ocr_body = serde_json::to_vec(&json!({
        "file_sha256": sha,
        "line_no": 4,
        "corrected_text": "Z = Tr exp(-beta H)"
    }))
    .unwrap();
    assert_eq!(
        handle_lessons(&state, "POST", &["ocr-overlays"], &qmap(&[]), &ocr_body)
            .unwrap()
            .0,
        200
    );

    // 5. Overview returns all stored entries.
    let (code, ov) = handle_lessons(&state, "GET", &[], &qmap(&[]), b"").unwrap();
    assert_eq!(code, 200);
    assert_eq!(ov["term_aliases"].as_array().unwrap().len(), 1);
    assert_eq!(ov["skeleton_overrides"].as_array().unwrap().len(), 1);
    assert_eq!(ov["ocr_overlays"].as_array().unwrap().len(), 1);

    // 6. Export backup requires opt-in `?confirm=true`, then restore roundtrips cleanly.
    let err = handle_lessons(&state, "GET", &["backup"], &qmap(&[]), b"").unwrap_err();
    assert_eq!(err.0, 400);

    let (code, backup) =
        handle_lessons(&state, "GET", &["backup"], &qmap(&[("confirm", "true")]), b"").unwrap();
    assert_eq!(code, 200);
    let backup_bytes = serde_json::to_vec(&backup).unwrap();

    // Delete skeleton override, verify gone, then restore backup.
    let (code, del) = handle_lessons(
        &state,
        "DELETE",
        &["skeleton-overrides", "n-aabbccddee"],
        &qmap(&[]),
        b"",
    )
    .unwrap();
    assert_eq!(code, 200);
    assert_eq!(del["deleted"], Value::Bool(true));

    let (code, res) =
        handle_lessons(&state, "POST", &["restore"], &qmap(&[]), &backup_bytes).unwrap();
    assert_eq!(code, 200);
    assert_eq!(res["restored"], json!(5));
}

#[test]
fn auth_guard_protects_lessons_endpoints() {
    let auth = AuthGuard::new(18_422).unwrap();
    let unauth = ApiCredentials {
        method: "GET",
        path: "/api/lessons",
        cookie: None,
        token_hdr: None,
        auth_hdr: None,
        origin: None,
        sec_fetch_site: None,
    };
    assert_eq!(auth.authorize_api(&unauth).unwrap_err().0, 401);

    let authed = ApiCredentials {
        method: "GET",
        path: "/api/lessons",
        cookie: None,
        token_hdr: Some(auth.token()),
        auth_hdr: None,
        origin: None,
        sec_fetch_site: None,
    };
    assert!(auth.authorize_api(&authed).is_ok());
}
