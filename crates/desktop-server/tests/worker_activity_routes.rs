//! Live HTTP dispatch + `AuthGuard` integration tests for F9b Core worker activity
//! endpoints (`/api/sessions/{id}/workers`, `/workers/{job_id}`, `/run-control`, and
//! static `/assets/worker_activity.js`).

mod common;

use common::{raw_http, test_cfg};
use desktop_server::start_desktop_server;
use serde_json::{json, Value};
use tempfile::TempDir;

#[test]
fn worker_activity_routes_require_token_and_serve_lazy_detail_and_asset() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // Create a session through the real HTTP server.
    let (st, _, created) = raw_http(port, "POST", "/api/sessions", &auth, b"{}");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&created));
    let sid = serde_json::from_slice::<Value>(&created).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Seed one worker call + blob directly in the session's ledger.
    let ledger = ledger::Ledger::open(&data_dir).unwrap();
    let raw_blob = ledger.put_blob(b"Candidate derivation summary").unwrap();
    ledger
        .append(
            &sid,
            "delegate.start",
            1,
            &json!({
                "call_id": "job-http-1",
                "worker": "compute_worker",
                "role": "compute_worker",
                "provider": "openai",
                "model": "gpt-4.1-mini",
                "routing_reason": "auto_smaller_tier"
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "delegate.end",
            1,
            &json!({
                "call_id": "job-http-1",
                "ok": true,
                "turns": 1,
                "raw_blob": raw_blob,
                "usage": { "input_tokens": 15, "output_tokens": 9 }
            }),
        )
        .unwrap();

    let workers_path = format!("/api/sessions/{sid}/workers");
    let detail_path = format!("/api/sessions/{sid}/workers/job-http-1");
    let control_path = format!("/api/sessions/{sid}/run-control");

    // Missing token -> 401 on all 3 API endpoints.
    for (method, path, body) in [
        ("GET", workers_path.as_str(), b"".as_slice()),
        ("GET", detail_path.as_str(), b"".as_slice()),
        ("POST", control_path.as_str(), br#"{"action":"focus_note","note":"hi"}"#.as_slice()),
    ] {
        let (code, _, _) = raw_http(port, method, path, &[], body);
        assert_eq!(code, 401, "{method} {path} should refuse unauthenticated calls");
    }

    // Foreign Origin on POST -> 403 even with valid token.
    let foreign = [("X-ReSearch-Token", tok.as_str()), ("Origin", "https://evil.example")];
    let (code, _, _) = raw_http(
        port,
        "POST",
        &control_path,
        &foreign,
        br#"{"action":"pause_after_stage","enabled":true}"#,
    );
    assert_eq!(code, 403);

    // Authorized GET list + POST run-control + GET detail succeed with 200.
    let (code, _, list_bytes) = raw_http(port, "GET", &workers_path, &auth, b"");
    assert_eq!(code, 200);
    let list: Value = serde_json::from_slice(&list_bytes).unwrap();
    assert_eq!(list["jobs"].as_array().unwrap().len(), 1);

    let (code, _, ctrl_bytes) = raw_http(
        port,
        "POST",
        &control_path,
        &auth,
        br#"{"action":"focus_note","note":"Verify asymptotics"}"#,
    );
    assert_eq!(code, 200);
    let ctrl: Value = serde_json::from_slice(&ctrl_bytes).unwrap();
    assert_eq!(ctrl["controls"]["focus_notes"][0]["note"], "Verify asymptotics");

    let (code, _, detail_bytes) = raw_http(port, "GET", &detail_path, &auth, b"");
    assert_eq!(code, 200);
    let detail: Value = serde_json::from_slice(&detail_bytes).unwrap();
    assert_eq!(detail["artifact_missing"], false);
    assert_eq!(
        detail["worker_summary"]["text"],
        "Candidate derivation summary"
    );

    // Static module asset is served cleanly.
    let (code, hdrs, asset_bytes) = raw_http(port, "GET", "/assets/worker_activity.js", &[], b"");
    assert_eq!(code, 200);
    assert!(
        hdrs.iter().any(|(k, v)| k.eq_ignore_ascii_case("content-type")
            && v.to_ascii_lowercase().contains("application/javascript")),
        "unexpected headers: {hdrs:?}"
    );
    let asset_src = String::from_utf8_lossy(&asset_bytes);
    assert!(asset_src.contains("mountWorkerActivityButton"));

    drop(srv);
}
