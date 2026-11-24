//! Tests-first integration tests for `desktop-server`:
//! - Auth & CSRF (single-use `GET /?launch=`, rejection of `GET /?token=`, `Set-Cookie`,
//!   `<meta name="re-search-csrf">`, `Host`/`Origin` enforcement, `401`/`403` rejections)
//! - Session CRUD, prompt execution + SSE event streaming
//! - Live `UiApprover` diff preview (`fs_write`/`latex_patch`) + fail-closed ledger recording
//! - File upload (`base64` JSON + `multipart/form-data`), workspace read/patch/audit,
//!   literature search, proof DAG, verify, fork/diff, narrate, sidetalk, export,
//!   and redacted `0600` provider settings.

mod common;

use common::{launch_cookie, raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::{preview_workspace_edit, ResearchEnv};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn json_upload_accepts_five_files_at_the_advertised_limit() {
    use std::fmt::Write as _;

    const FILE_BYTES: usize = 10 * 1024 * 1024;
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let headers = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];
    let (status, _, created) = raw_http(port, "POST", "/api/sessions", &headers, b"{}");
    assert_eq!(status, 200);
    let created: Value = serde_json::from_slice(&created).unwrap();
    let sid = created["id"].as_str().unwrap();

    // Each YWFh decodes to three bytes and YQ== supplies the final byte.
    let encoded = "YWFh".repeat(FILE_BYTES / 3);
    let mut body = String::with_capacity(5 * encoded.len() + 512);
    body.push_str(r#"{"files":["#);
    for index in 0..5 {
        if index > 0 {
            body.push(',');
        }
        write!(body, "{{\"name\":\"notes{index}.txt\",\"content_base64\":\"").unwrap();
        body.push_str(&encoded);
        body.push_str("YQ==\"}");
    }
    body.push_str("]}");

    let (status, _, result) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/upload"),
        &headers,
        body.as_bytes(),
    );
    assert_eq!(status, 200);
    let result: Value = serde_json::from_slice(&result).unwrap();
    let ingested = result["ingested"].as_array().unwrap();
    assert_eq!(ingested.len(), 5);
    assert!(ingested.iter().all(|report| report["bytes"] == FILE_BYTES));
    assert_eq!(result["failed"], json!([]));

    // The upload allowance does not change the limit on other endpoints.
    let oversized = body.len().to_string();
    let limited_headers = [headers[0], ("Content-Length", oversized.as_str())];
    assert_eq!(raw_http(port, "POST", "/api/sessions", &limited_headers, b"").0, 413);
    srv.shutdown();
}

#[test]
fn mixed_upload_reports_each_success_and_indexes_failures() {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let headers = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];
    let (status, _, created) = raw_http(port, "POST", "/api/sessions", &headers, b"{}");
    assert_eq!(status, 200);
    let created: Value = serde_json::from_slice(&created).unwrap();
    let sid = created["id"].as_str().unwrap();

    let (status, _, result) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/upload"),
        &headers,
        br#"{"files":[{"name":"notes.txt","bytes_base64":"YQ=="},{"name":"notes.txt","bytes_base64":"AA=="},{"name":"extra.txt","bytes_base64":"Yg=="}]}"#,
    );
    assert_eq!(status, 400);
    let result: Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(result["ok"], false);
    let ingested = result["ingested"].as_array().unwrap();
    assert_eq!(ingested.len(), 2);
    assert_eq!(ingested[0]["origin"], "notes.txt");
    assert_eq!(ingested[1]["origin"], "extra.txt");
    assert_eq!(
        result["failed"],
        json!([{ "index": 1, "name": "notes.txt", "error": result["error"] }])
    );
    assert!(result["error"].as_str().unwrap().contains("NUL"));
    let events = Ledger::open_existing(tmp.path()).unwrap().read_session(sid).unwrap();
    let sources = events.iter().filter(|event| event.kind == "source.ingested").count();
    assert_eq!(sources, 2);
    srv.shutdown();
}

#[test]
fn auth_bootstrap_assets_host_origin_and_csrf_enforced() {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    assert_eq!(tok.len(), 64);

    // Launch URL carries only the single-use launch nonce, never the live API token.
    assert!(srv.info.url.starts_with(&format!("{}/?launch=", srv.info.base_url)), "{}", srv.info.url);
    assert!(!srv.info.url.contains(&tok), "launch URL must never contain the API token");
    let launch_path = srv.info.url.strip_prefix(&srv.info.base_url).unwrap().to_string();
    let nonce = launch_path.strip_prefix("/?launch=").unwrap();
    assert_eq!(nonce.len(), 64);
    assert_ne!(nonce, tok);

    // Health check works on loopback Host.
    let (st, _, body) = raw_http(port, "GET", "/api/health", &[], b"");
    assert_eq!(st, 200);
    let h: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(h["ok"], true);

    // Bad Host rejected with 403.
    let (st_host, _, _) = raw_http(port, "GET", "/api/health", &[("Host", "evil.example:8080")], b"");
    assert_eq!(st_host, 403);

    // Real API token is NEVER accepted as a URL query parameter on GET /.
    let (st_real_tok_q, tok_q_hdrs, tok_q_body) = raw_http(port, "GET", &format!("/?token={tok}"), &[], b"");
    assert_eq!(st_real_tok_q, 403);
    assert!(!String::from_utf8_lossy(&tok_q_body).contains(&tok));
    assert!(!tok_q_hdrs.iter().any(|(k, _)| k == "set-cookie"));

    let (st_bad_q, _, _) = raw_http(port, "GET", "/?token=wrong", &[], b"");
    assert_eq!(st_bad_q, 403);

    // Wrong launch nonce is 403 and must NOT burn the legitimate launch nonce.
    let (st_bad_launch, bad_launch_hdrs, bad_launch_body) = raw_http(port, "GET", "/?launch=wrong", &[], b"");
    assert_eq!(st_bad_launch, 403);
    assert!(!String::from_utf8_lossy(&bad_launch_body).contains(&tok));
    assert!(!bad_launch_hdrs.iter().any(|(k, _)| k == "set-cookie"));

    // First use of the valid launch nonce returns 200, injects <meta>, and sets the stream cookie.
    let (st_root, hdrs, html_bytes) = raw_http(port, "GET", &launch_path, &[], b"");
    assert_eq!(st_root, 200);
    let html = String::from_utf8(html_bytes).unwrap();
    assert!(html.contains(&format!("<meta name=\"re-search-csrf\" content=\"{tok}\">")));
    let set_cookie = hdrs
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.as_str())
        .unwrap();
    // The stream cookie is per-port and is independent of both the token and the launch nonce.
    assert!(set_cookie.starts_with(&format!("__research_stream_{port}=")));
    assert!(!set_cookie.contains(&tok));
    assert!(!set_cookie.contains(nonce));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    let cookie_hdr = set_cookie.split(';').next().unwrap().trim().to_string();

    // Replay of the same launch nonce (without cookie) is 403, leaks no token, and sets no cookie.
    let (st_replay, replay_hdrs, replay_body) = raw_http(port, "GET", &launch_path, &[], b"");
    assert_eq!(st_replay, 403);
    assert!(!String::from_utf8_lossy(&replay_body).contains(&tok));
    assert!(!replay_hdrs.iter().any(|(k, _)| k == "set-cookie"));

    // A reload (cookie alone) gets the page WITHOUT the token: the cookie cannot be traded for it.
    let (st_reload, _, reload_html) = raw_http(port, "GET", "/", &[("Cookie", &cookie_hdr)], b"");
    assert_eq!(st_reload, 200);
    let reload_str = String::from_utf8_lossy(&reload_html);
    assert!(!reload_str.contains(&tok));
    assert!(reload_str.contains(r#"<meta name="re-search-csrf" content="">"#));

    // The page refuses to be framed, and `/` with no credential at all is refused.
    for (name, want) in [("x-frame-options", "DENY"), ("content-security-policy", "frame-ancestors 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'")] {
        assert!(hdrs.iter().any(|(k, v)| k == name && v == want), "missing {name}: {hdrs:?}");
    }
    assert_eq!(raw_http(port, "GET", "/", &[], b"").0, 403);
    // The token-for-cookie exchange route is gone.
    for hdrs in [vec![("Cookie", cookie_hdr.as_str())], vec![("X-ReSearch-Token", tok.as_str())]] {
        let (st, _, body) = raw_http(port, "GET", "/api/bootstrap", &hdrs, b"");
        assert!(matches!(st, 401 | 404), "{st}");
        assert!(!String::from_utf8_lossy(&body).contains(&tok));
    }
    // Host must be this loopback server: other hosts and other ports are refused.
    for host in ["evil.example", format!("127.0.0.1:{}", port.wrapping_add(1)).as_str(), "127.0.0.1.evil.example"] {
        assert_eq!(raw_http(port, "GET", "/api/health", &[("Host", host)], b"").0, 403, "Host {host}");
    }
    let localhost = format!("localhost:{port}");
    assert_eq!(raw_http(port, "GET", "/api/health", &[("Host", &localhost)], b"").0, 200);

    // Vendor KaTeX font & CSS served accurately, traversal blocked.
    let (st_kcss, kcss_hdrs, _) = raw_http(port, "GET", "/vendor/katex/katex.min.css", &[], b"");
    assert_eq!(st_kcss, 200);
    assert!(kcss_hdrs.iter().any(|(k, v)| k == "content-type" && v.contains("text/css")));
    let (st_woff, woff_hdrs, _) =
        raw_http(port, "GET", "/vendor/katex/fonts/KaTeX_Main-Regular.woff2", &[], b"");
    assert_eq!(st_woff, 200);
    assert!(woff_hdrs.iter().any(|(k, v)| k == "content-type" && v.contains("font/woff2")));
    let (st_trav, _, _) = raw_http(port, "GET", "/assets/../Cargo.toml", &[], b"");
    assert!(matches!(st_trav, 403 | 404));

    // Mutating POST requires token AND valid Origin if present.
    let (st_nocsrf, _, _) = raw_http(port, "POST", "/api/sessions", &[], b"{}");
    assert_eq!(st_nocsrf, 401);
    let (st_bad_orig, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[("X-ReSearch-Token", &tok), ("Origin", "http://evil.invalid")],
        b"{}",
    );
    assert_eq!(st_bad_orig, 403);

    srv.shutdown();
}

#[test]
fn sessions_sse_approval_card_tools_fork_export_and_settings() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\nHi\n\\end{document}\n").unwrap();

    // Scripted Anthropic mock: 1) fs_write (triggers UiApprover!), 2) final text.
    let turn1 = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "tool_use",
        "content": [
            { "type": "thinking", "thinking": "Preparing write", "signature": "sig1" },
            { "type": "tool_use", "id": "tu_w1", "name": "fs_write", "input": { "path": "notes.tex", "content": "Hello $\\alpha$\n" } }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 12 }
    });
    let turn2 = json!({
        "id": "msg_2",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": "Wrote notes.tex cleanly." }],
        "usage": { "input_tokens": 20, "output_tokens": 8 }
    });
    let llm = MockHttp::start(vec![
        MockReply { status: 200, content_type: "application/json", body: serde_json::to_vec(&turn1).unwrap() },
        MockReply { status: 200, content_type: "application/json", body: serde_json::to_vec(&turn2).unwrap() },
    ]);

    let arxiv_xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>http://arxiv.org/abs/2403.09876v1</id>
    <title>Test Paper</title>
    <summary>A short abstract.</summary>
    <published>2024-03-14T00:00:00Z</published>
    <updated>2024-03-15T00:00:00Z</updated>
    <author><name>Alice Researcher</name></author>
  </entry>
</feed>"#;
    let lit = MockHttp::start(vec![MockReply {
        status: 200,
        content_type: "application/atom+xml",
        body: arxiv_xml.to_vec(),
    }]);

    std::env::set_var("RE_SEARCH_API_KEY", "sk-desktop-test-secret-0001");
    let srv = start_desktop_server(test_cfg(
        &data_dir,
        Some(ws_dir.clone()),
        Some(llm.base_url.clone()),
        Some(&lit.base_url),
    ))
    .unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    common::disable_auto_approve(port, &tok);
    let auth_hdr = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];

    // Create idle session -> visible immediately in GET /api/sessions.
    let (st_c, _, body_c) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &auth_hdr,
        serde_json::to_vec(&json!({ "title": "Desktop Test", "workspace_root": ws_dir })).unwrap().as_slice(),
    );
    assert_eq!(st_c, 200);
    let created: Value = serde_json::from_slice(&body_c).unwrap();
    let sid = created["id"].as_str().unwrap().to_string();

    let cookie_hdr = launch_cookie(port, &tok);
    let (st_l, _, body_l) = raw_http(port, "GET", "/api/sessions", &auth_hdr, b"");
    assert_eq!(st_l, 200);
    let list: Value = serde_json::from_slice(&body_l).unwrap();
    assert!(list["sessions"].as_array().unwrap().iter().any(|s| s["id"] == sid));

    // Open SSE stream before triggering message turn.
    let mut sse_stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse_stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let sse_req = format!("GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie_hdr}\r\n\r\n");
    sse_stream.write_all(sse_req.as_bytes()).unwrap();
    sse_stream.flush().unwrap();

    // Spawn SSE reader collecting frames.
    let (tx_ev, rx_ev) = mpsc::channel::<(String, Value)>();
    let sse_thread = thread::spawn(move || {
        let mut reader = BufReader::new(sse_stream);
        let mut cur_ev = String::new();
        let mut cur_data = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let t = line.trim_end_matches(&['\r', '\n'][..]);
                    if t.is_empty() {
                        if !cur_ev.is_empty() && !cur_data.is_empty() {
                            if let Ok(v) = serde_json::from_str::<Value>(&cur_data) {
                                let _ = tx_ev.send((cur_ev.clone(), v));
                            }
                        }
                        cur_ev.clear();
                        cur_data.clear();
                    } else if let Some(rest) = t.strip_prefix("event:") {
                        cur_ev = rest.trim().to_string();
                    } else if let Some(rest) = t.strip_prefix("data:") {
                        cur_data.push_str(rest.trim());
                    }
                }
            }
        }
    });

    // Trigger solver message -> will block on UiApprover for fs_write.
    let (st_m, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        &auth_hdr,
        serde_json::to_vec(&json!({ "prompt": "Write notes.tex", "provider": "anthropic" })).unwrap().as_slice(),
    );
    assert!(matches!(st_m, 200 | 202));

    // Wait for approval_requested SSE event and verify edit_preview.
    let mut approval_data = None;
    for _ in 0..50 {
        if let Ok((ev, data)) = rx_ev.recv_timeout(Duration::from_secs(5)) {
            if ev == "approval_requested" {
                approval_data = Some(data);
                break;
            }
        }
    }
    let ap = approval_data.expect("expected approval_requested SSE event");
    let call_id = ap["call_id"].as_str().unwrap();
    let digest = ap["digest"].as_str().unwrap();
    assert!(ap["edit_preview"]["unified_diff"].as_str().unwrap().contains("notes.tex"));

    // Approve via REST endpoint.
    let (st_ap, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/approvals/{call_id}"),
        &auth_hdr,
        serde_json::to_vec(&json!({ "digest": digest, "approved": true })).unwrap().as_slice(),
    );
    assert_eq!(st_ap, 200);

    // Wait for turn_complete SSE event.
    let mut saw_complete = false;
    for _ in 0..80 {
        if let Ok((ev, _)) = rx_ev.recv_timeout(Duration::from_secs(5)) {
            if ev == "turn_complete" {
                saw_complete = true;
                break;
            }
        }
    }
    assert!(saw_complete, "expected turn_complete SSE event");
    assert_eq!(std::fs::read_to_string(ws_dir.join("notes.tex")).unwrap(), "Hello $\\alpha$\n");

    // Upload JSON base64 file into session sources.
    let b64 = "XHNlY3Rpb257VXBsb2FkZWR9CkhlbGxvCg=="; // \section{Uploaded}\nHello\n
    let (st_up, _, up_body) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/upload"),
        &auth_hdr,
        serde_json::to_vec(&json!({ "files": [{ "name": "uploaded.tex", "bytes_base64": b64 }] })).unwrap().as_slice(),
    );
    assert_eq!(st_up, 200);
    let up_v: Value = serde_json::from_slice(&up_body).unwrap();
    assert_eq!(up_v["ingested"].as_array().unwrap().len(), 1);

    // Workspace list & file read + tex-audit + literature + dag + verify + settings redaction.
    let (st_ws, _, ws_b) = raw_http(port, "GET", &format!("/api/sessions/{sid}/workspace"), &auth_hdr, b"");
    assert_eq!(st_ws, 200);
    let ws_v: Value = serde_json::from_slice(&ws_b).unwrap();
    assert!(ws_v["files"].as_array().unwrap().iter().any(|f| f["path"] == "notes.tex"));

    let (st_ta, _, ta_b) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/tex-audit"),
        &auth_hdr,
        b"{}",
    );
    assert_eq!(st_ta, 200);
    assert_eq!(serde_json::from_slice::<Value>(&ta_b).unwrap()["ok"], true);

    let (st_lit, _, lit_b) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/literature"),
        &auth_hdr,
        serde_json::to_vec(&json!({ "source": "arxiv", "query": "2403.09876" })).unwrap().as_slice(),
    );
    assert_eq!(st_lit, 200);
    assert_eq!(serde_json::from_slice::<Value>(&lit_b).unwrap()["count"], 1);

    let (st_ver, _, ver_b) = raw_http(port, "POST", &format!("/api/sessions/{sid}/verify"), &auth_hdr, b"{}");
    assert_eq!(st_ver, 200);
    assert_eq!(serde_json::from_slice::<Value>(&ver_b).unwrap()["ok"], true);

    // Settings PUT/GET redacts keys.
    let secret_key = "sk-ant-never-return-in-get-9999";
    let (st_put, _, put_b) = raw_http(
        port,
        "PUT",
        "/api/settings/providers",
        &auth_hdr,
        serde_json::to_vec(&json!({ "keys": { "anthropic": secret_key } })).unwrap().as_slice(),
    );
    assert_eq!(st_put, 200);
    assert!(!String::from_utf8_lossy(&put_b).contains(secret_key));
    let (st_get_set, _, get_set_b) = raw_http(port, "GET", "/api/settings/providers", &auth_hdr, b"");
    assert_eq!(st_get_set, 200);
    let set_str = String::from_utf8_lossy(&get_set_b);
    assert!(!set_str.contains(secret_key));
    let set_v: Value = serde_json::from_slice(&get_set_b).unwrap();
    assert_eq!(set_v["has_anthropic_key"], true);

    // Ledger clean!
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);

    // Sanity check edit preview helper directly.
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let prev = preview_workspace_edit(&env, "fs_write", r#"{"content":"x","path":"notes.tex"}"#).unwrap();
    assert!(prev.sha256_before.is_some());

    srv.shutdown();
    drop(sse_thread);
    llm.finish();
    lit.finish();
}
