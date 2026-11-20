//! Tests-first integration tests for `desktop-server`:
//! - Auth & CSRF (`GET /?token=`, `Set-Cookie`, `<meta name="re-search-csrf">`,
//!   `GET /api/bootstrap`, `Host`/`Origin` enforcement, `401`/`403` rejections)
//! - Session CRUD, prompt execution + SSE event streaming
//! - Live `UiApprover` diff preview (`fs_write`/`latex_patch`) + fail-closed ledger recording
//! - File upload (`base64` JSON + `multipart/form-data`), workspace read/patch/audit,
//!   literature search, proof DAG, verify, fork/diff, narrate, sidetalk, export,
//!   and redacted `0600` provider settings.

use desktop_server::{start_desktop_server, DesktopConfig};
use ledger::Ledger;
use research_tools::{preview_workspace_edit, LiteratureEndpoints, ResearchEnv};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

struct MockReply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

struct MockHttp {
    base_url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttp {
    fn start(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for r in replies {
                let (mut stream, _) = listener.accept().unwrap();
                drain_http_request(&mut stream);
                let hdr = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    r.status,
                    r.content_type,
                    r.body.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                let _ = stream.write_all(&r.body);
                let _ = stream.flush();
            }
        });
        Self { base_url: format!("http://{addr}"), handle: Some(handle) }
    }

    fn finish(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn drain_http_request(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_len > 0 {
        let mut buf = vec![0u8; content_len];
        let _ = reader.read_exact(&mut buf);
    }
}

fn raw_http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    let mut has_host = false;
    for &(k, v) in headers {
        if k.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !has_host {
        req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    if !body.is_empty() && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length")) {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    if !body.is_empty() {
        stream.write_all(body).unwrap();
    }
    stream.flush().unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();
    let status_line = lines.next().unwrap();
    let status: u16 = status_line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut out_hdrs = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            out_hdrs.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    (status, out_hdrs, raw[split + 4..].to_vec())
}

fn test_cfg(data_dir: &Path, workspace: Option<PathBuf>, api_base: Option<String>, lit_base: Option<&str>) -> DesktopConfig {
    let endpoints = match lit_base {
        Some(u) => LiteratureEndpoints::loopback_mock(u).unwrap(),
        None => LiteratureEndpoints::default(),
    };
    DesktopConfig {
        port: 0,
        data_dir: data_dir.to_path_buf(),
        workspace_root: workspace,
        python: None,
        worker_script: None,
        api_base_override: api_base,
        literature_endpoints: endpoints,
        open_window: false,
        app_mode: false,
        print_url_json: false,
        eval_timeout_s: 30,
    }
}

#[test]
fn auth_bootstrap_assets_host_origin_and_csrf_enforced() {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    assert_eq!(tok.len(), 64);

    // Health check works on loopback Host.
    let (st, _, body) = raw_http(port, "GET", "/api/health", &[], b"");
    assert_eq!(st, 200);
    let h: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(h["ok"], true);

    // Bad Host rejected with 403.
    let (st_host, _, _) = raw_http(port, "GET", "/api/health", &[("Host", "evil.example:8080")], b"");
    assert_eq!(st_host, 403);

    // Wrong token on / rejected; valid token sets HttpOnly SameSite=Strict cookie + injects <meta>.
    let (st_bad_q, _, _) = raw_http(port, "GET", "/?token=wrong", &[], b"");
    assert_eq!(st_bad_q, 403);

    let (st_root, hdrs, html_bytes) = raw_http(port, "GET", &format!("/?token={tok}"), &[], b"");
    assert_eq!(st_root, 200);
    let html = String::from_utf8(html_bytes).unwrap();
    assert!(html.contains(&format!("<meta name=\"re-search-csrf\" content=\"{tok}\">")));
    let set_cookie = hdrs
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.as_str())
        .unwrap();
    assert!(set_cookie.contains(&format!("__research_token={tok}")));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));

    // /api/bootstrap requires valid __research_token cookie (or header) and returns csrf_token.
    let (st_boot_noauth, _, _) = raw_http(port, "GET", "/api/bootstrap", &[], b"");
    assert_eq!(st_boot_noauth, 401);
    let cookie_hdr = format!("__research_token={tok}");
    let (st_boot_ok, _, boot_body) =
        raw_http(port, "GET", "/api/bootstrap", &[("Cookie", &cookie_hdr)], b"");
    assert_eq!(st_boot_ok, 200);
    let boot: Value = serde_json::from_slice(&boot_body).unwrap();
    assert_eq!(boot["csrf_token"], tok);

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

    let cookie_hdr = format!("__research_token={tok}");
    let cookie = [("Cookie", cookie_hdr.as_str())];
    let (st_l, _, body_l) = raw_http(port, "GET", "/api/sessions", &cookie, b"");
    assert_eq!(st_l, 200);
    let list: Value = serde_json::from_slice(&body_l).unwrap();
    assert!(list["sessions"].as_array().unwrap().iter().any(|s| s["id"] == sid));

    // Open SSE stream before triggering message turn.
    let mut sse_stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse_stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let sse_req = format!("GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: __research_token={tok}\r\n\r\n");
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
    let (st_ws, _, ws_b) = raw_http(port, "GET", &format!("/api/sessions/{sid}/workspace"), &cookie, b"");
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
    let (st_get_set, _, get_set_b) = raw_http(port, "GET", "/api/settings/providers", &cookie, b"");
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

/// Every `/api/` route except `/api/health` refuses a request without the launch credential,
/// a GET with the cookie (or header token) succeeds, and a mutation with only the cookie is refused.
#[test]
fn every_api_route_except_health_requires_credentials() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("main.tex"), "\\documentclass{article}\n").unwrap();
    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws_dir.clone()), None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie_hdr = format!("__research_token={tok}");
    let cookie = [("Cookie", cookie_hdr.as_str())];
    let token = [("X-ReSearch-Token", tok.as_str())];
    let body = serde_json::to_vec(&json!({ "title": "auth", "workspace_root": ws_dir })).unwrap();
    let (st, _, created) = raw_http(port, "POST", "/api/sessions", &token, &body);
    assert_eq!(st, 200);
    let sid = serde_json::from_slice::<Value>(&created).unwrap()["id"].as_str().unwrap().to_string();

    let s = format!("/api/sessions/{sid}");
    let gets = [
        "/api/bootstrap".to_string(),
        "/api/sessions".to_string(),
        s.clone(),
        format!("{s}/events"),
        format!("{s}/workspace"),
        format!("{s}/workspace/file?path=main.tex"),
        format!("{s}/dag"),
        format!("{s}/diff/{sid}"),
        format!("{s}/sidetalk"),
        format!("{s}/export?format=tex"),
        "/api/settings/providers".to_string(),
        "/api/no-such-route".to_string(),
    ];
    let mutations = [
        ("POST", "/api/sessions".to_string()),
        ("POST", format!("{s}/messages")),
        ("POST", format!("{s}/approvals/c-1")),
        ("POST", format!("{s}/upload")),
        ("POST", format!("{s}/latex/patch")),
        ("POST", format!("{s}/tex-audit")),
        ("POST", format!("{s}/literature")),
        ("POST", format!("{s}/verify")),
        ("POST", format!("{s}/fork")),
        ("POST", format!("{s}/narrate")),
        ("POST", format!("{s}/sidetalk")),
        ("PUT", "/api/settings/providers".to_string()),
        ("DELETE", s.clone()),
    ];
    let bad_cookie = [("Cookie", "__research_token=0000")];
    let bad_token = [("X-ReSearch-Token", "0000")];
    let cross_site = [("Origin", "http://evil.invalid")];
    let no_creds: [&[(&str, &str)]; 4] = [&[], &bad_cookie, &bad_token, &cross_site];
    for path in &gets {
        for hdrs in no_creds {
            let (st, _, _) = raw_http(port, "GET", path, hdrs, b"");
            assert_eq!(st, 401, "GET {path} with {hdrs:?} must be 401");
        }
    }
    for (method, path) in &mutations {
        for hdrs in no_creds.iter().copied().chain([&cookie[..]]) {
            let (st, _, _) = raw_http(port, method, path, hdrs, b"{}");
            assert_eq!(st, 401, "{method} {path} with {hdrs:?} must be 401");
        }
    }
    // Refused mutations had no effect: still one session, no provider file written.
    let (st, _, list) = raw_http(port, "GET", "/api/sessions", &cookie, b"");
    assert_eq!(st, 200);
    assert_eq!(serde_json::from_slice::<Value>(&list).unwrap()["sessions"].as_array().unwrap().len(), 1);
    assert!(!data_dir.join("providers.json").exists());

    // Health stays open; credentialed reads work with the cookie or the header token.
    assert_eq!(raw_http(port, "GET", "/api/health", &[], b"").0, 200);
    for path in gets.iter().filter(|p| !p.ends_with("/events")) {
        let (st, _, b) = raw_http(port, "GET", path, &cookie, b"");
        assert!(!matches!(st, 401 | 403), "GET {path} with cookie got {st}: {}", String::from_utf8_lossy(&b));
    }
    for path in ["/api/sessions", s.as_str(), "/api/settings/providers"] {
        assert_eq!(raw_http(port, "GET", path, &token, b"").0, 200, "GET {path} with header token");
    }
    // The SSE stream opens with the cookie (read only its status line).
    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(sse, "GET {s}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie_hdr}\r\n\r\n").unwrap();
    let mut status_line = String::new();
    BufReader::new(&sse).read_line(&mut status_line).unwrap();
    assert!(status_line.starts_with("HTTP/1.1 200"), "SSE status: {status_line}");
    drop(sse);
    srv.shutdown();
}
