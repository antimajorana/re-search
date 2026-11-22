//! Credential rule of the desktop server, route by route:
//! - every `/api/` route except `/api/health` answers 401 without the header token;
//! - the stream cookie opens only `GET .../events` and `GET .../export`, only same-origin,
//!   and can never be traded for the token or used to mutate;
//! - a valid token with a foreign `Origin` cannot mutate;
//! - refused requests have no effect, and an unauthenticated body is not read.

mod common;

use common::{launch_cookie, raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::start_desktop_server;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    data_dir: std::path::PathBuf,
    srv: desktop_server::RunningDesktopServer,
    port: u16,
    tok: String,
    cookie: String,
    sid: String,
}

fn fixture(api_base: Option<String>) -> Fixture {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("main.tex"), "\\documentclass{article}\n").unwrap();
    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws_dir.clone()), api_base, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let body = serde_json::to_vec(&json!({ "title": "auth", "workspace_root": ws_dir })).unwrap();
    let (st, _, created) = raw_http(port, "POST", "/api/sessions", &[("X-ReSearch-Token", &tok)], &body);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&created));
    let sid = serde_json::from_slice::<Value>(&created).unwrap()["id"].as_str().unwrap().to_string();
    Fixture { _tmp: tmp, data_dir, srv, port, tok, cookie, sid }
}

fn session_count(f: &Fixture) -> usize {
    let (st, _, list) = raw_http(f.port, "GET", "/api/sessions", &[("X-ReSearch-Token", &f.tok)], b"");
    assert_eq!(st, 200);
    serde_json::from_slice::<Value>(&list).unwrap()["sessions"].as_array().unwrap().len()
}

fn sse_status(port: u16, path: &str, headers: &str) -> String {
    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(sse, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{headers}\r\n").unwrap();
    let mut line = String::new();
    BufReader::new(&sse).read_line(&mut line).unwrap();
    line
}

#[test]
fn every_api_route_except_health_requires_the_token() {
    let f = fixture(None);
    let (port, sid) = (f.port, f.sid.as_str());
    let s = format!("/api/sessions/{sid}");
    let token = [("X-ReSearch-Token", f.tok.as_str())];
    let cookie = [("Cookie", f.cookie.as_str())];

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
        "/api/connections".to_string(),
        "/api/connections/chatgpt/login".to_string(),
        "/api/models".to_string(),
        "/api/models?refresh=1".to_string(),
        "/api/no-such-route".to_string(),
        // Path tricks must not reach an open route: only the exact `/api/health` is open.
        "/api/health/".to_string(),
        "/api/health%2F..%2Fsessions".to_string(),
        "/%61pi/sessions".to_string(),
    ];
    let mutations = [
        ("POST", "/api/sessions".to_string()),
        ("POST", format!("{s}/messages")),
        ("POST", format!("{s}/approvals/c-1")),
        ("POST", format!("{s}/stop")),
        ("POST", format!("{s}/upload")),
        ("POST", format!("{s}/latex/patch")),
        ("POST", format!("{s}/tex-audit")),
        ("POST", format!("{s}/literature")),
        ("POST", format!("{s}/verify")),
        ("POST", format!("{s}/fork")),
        ("POST", format!("{s}/narrate")),
        ("POST", format!("{s}/sidetalk")),
        ("POST", "/api/connections/chatgpt/login".to_string()),
        ("POST", "/api/connections/chatgpt/open".to_string()),
        ("POST", "/api/connections/chatgpt/logout".to_string()),
        ("POST", "/api/connections/antigravity/acknowledge".to_string()),
        ("POST", "/api/connections/antigravity/withdraw".to_string()),
        ("PUT", "/api/settings/providers".to_string()),
        ("DELETE", s.clone()),
        ("PATCH", s.clone()),
        ("OPTIONS", "/api/sessions".to_string()),
        // A mutation aimed at a cookie route is still a mutation.
        ("POST", format!("{s}/export")),
        ("POST", format!("{s}/events")),
    ];
    // The token is not accepted as a cookie value, nor the cookie secret as a token.
    let cookie_value = f.cookie.split_once('=').unwrap().1.to_string();
    let token_as_cookie = format!("{}={}", f.cookie.split_once('=').unwrap().0, f.tok);
    let bad_cookie = [("Cookie", "__research_stream_1=0000")];
    let bad_token = [("X-ReSearch-Token", "0000")];
    let cross_site = [("Origin", "http://evil.invalid")];
    let swapped_a = [("Cookie", token_as_cookie.as_str())];
    let swapped_b = [("X-ReSearch-Token", cookie_value.as_str())];
    let no_creds: [&[(&str, &str)]; 6] = [&[], &bad_cookie, &bad_token, &cross_site, &swapped_a, &swapped_b];

    for path in &gets {
        for hdrs in no_creds {
            let (st, _, _) = raw_http(port, "GET", path, hdrs, b"");
            assert_eq!(st, 401, "GET {path} with {hdrs:?} must be 401");
        }
        // The cookie alone opens nothing except the two stream routes.
        if !(path.ends_with("/events") || path.contains("/export?")) {
            assert_eq!(raw_http(port, "GET", path, &cookie, b"").0, 401, "GET {path} with cookie only");
        }
    }
    for (method, path) in &mutations {
        for hdrs in no_creds.iter().copied().chain([&cookie[..]]) {
            let (st, _, _) = raw_http(port, method, path, hdrs, b"{}");
            assert_eq!(st, 401, "{method} {path} with {hdrs:?} must be 401");
        }
    }
    // HEAD is not a way in either.
    assert_eq!(raw_http(port, "HEAD", "/api/sessions", &[], b"").0, 401);

    // Refused mutations had no effect: still one session, no provider file written.
    assert_eq!(session_count(&f), 1);
    assert!(!f.data_dir.join("providers.json").exists());

    // Health stays open and says nothing beyond ok + version.
    let (st, _, health) = raw_http(port, "GET", "/api/health", &[], b"");
    assert_eq!(st, 200);
    let health: Value = serde_json::from_slice(&health).unwrap();
    assert_eq!(health.as_object().unwrap().len(), 2, "{health}");

    // Positive control: with the header token every real GET route answers (not 401/403).
    for path in gets.iter().filter(|p| !p.ends_with("/events")) {
        let (st, _, b) = raw_http(port, "GET", path, &token, b"");
        assert!(!matches!(st, 401 | 403), "GET {path} with token got {st}: {}", String::from_utf8_lossy(&b));
    }
    for path in ["/api/sessions", s.as_str(), "/api/settings/providers"] {
        assert_eq!(raw_http(port, "GET", path, &token, b"").0, 200, "GET {path} with header token");
    }
    let bearer = format!("Bearer {}", f.tok);
    assert_eq!(raw_http(port, "GET", "/api/sessions", &[("Authorization", &bearer)], b"").0, 200);
    f.srv.shutdown();
}

#[test]
fn stream_cookie_opens_only_same_origin_events_and_export() {
    let f = fixture(None);
    let (port, sid) = (f.port, f.sid.as_str());
    let events = format!("/api/sessions/{sid}/events");
    let export = format!("/api/sessions/{sid}/export?format=tex");
    let own_origin = format!("http://127.0.0.1:{port}");

    // SSE: opens with the cookie, with or without the browser's same-origin marker.
    for extra in ["", "Sec-Fetch-Site: same-origin\r\n"] {
        let line = sse_status(port, &events, &format!("Cookie: {}\r\n{extra}", f.cookie));
        assert!(line.starts_with("HTTP/1.1 200"), "SSE with cookie ({extra:?}): {line}");
    }
    // Export with the cookie authenticates (the body may be an export error, never 401/403).
    for hdrs in [
        vec![("Cookie", f.cookie.as_str())],
        vec![("Cookie", f.cookie.as_str()), ("Sec-Fetch-Site", "same-origin"), ("Origin", own_origin.as_str())],
    ] {
        let (st, _, _) = raw_http(port, "GET", &export, &hdrs, b"");
        assert!(!matches!(st, 401 | 403), "export with cookie {hdrs:?} got {st}");
    }
    // A page on another 127.0.0.1 port is "same-site", a web page elsewhere "cross-site", a
    // typed URL "none": none of them may use the cookie.
    for site in ["same-site", "cross-site", "none"] {
        for path in [&events, &export] {
            let hdrs = [("Cookie", f.cookie.as_str()), ("Sec-Fetch-Site", site)];
            assert_eq!(raw_http(port, "GET", path, &hdrs, b"").0, 401, "GET {path} Sec-Fetch-Site: {site}");
        }
    }
    let foreign = [("Cookie", f.cookie.as_str()), ("Origin", "http://127.0.0.1:1")];
    assert_eq!(raw_http(port, "GET", &export, &foreign, b"").0, 401);
    // The cookie of another port's instance (different name) is not accepted.
    let other_port = format!("__research_stream_{}={}", port.wrapping_add(1), f.cookie.split_once('=').unwrap().1);
    assert_eq!(raw_http(port, "GET", &export, &[("Cookie", &other_port)], b"").0, 401);
    f.srv.shutdown();
}

#[test]
fn valid_token_with_foreign_origin_cannot_mutate_and_refused_bodies_are_not_read() {
    let f = fixture(None);
    let port = f.port;
    let own_origin = format!("http://127.0.0.1:{port}");
    let stop_path = format!("/api/sessions/{}/stop", f.sid);
    let guarded_posts = [
        "/api/sessions",
        "/api/connections/chatgpt/login",
        "/api/connections/chatgpt/open",
        "/api/connections/chatgpt/logout",
        "/api/connections/antigravity/acknowledge",
        "/api/connections/antigravity/withdraw",
        stop_path.as_str(),
    ];
    for origin in ["http://evil.invalid", "http://127.0.0.1:1", "null"] {
        let hdrs = [("X-ReSearch-Token", f.tok.as_str()), ("Origin", origin)];
        for path in guarded_posts {
            assert_eq!(raw_http(port, "POST", path, &hdrs, b"{}").0, 403, "POST {path} Origin {origin}");
        }
    }
    assert_eq!(session_count(&f), 1);
    let ok = [("X-ReSearch-Token", f.tok.as_str()), ("Origin", own_origin.as_str())];
    assert_eq!(raw_http(port, "POST", "/api/sessions", &ok, b"{}").0, 200);
    assert_eq!(session_count(&f), 2);
    // `Authorization: Bearer` is the same credential, for mutations too.
    let bearer = format!("Bearer {}", f.tok);
    assert_eq!(raw_http(port, "POST", "/api/sessions", &[("Authorization", &bearer)], b"{}").0, 200);
    assert_eq!(session_count(&f), 3);
    // The cookie plus the browser's same-origin marker is still not a mutation credential,
    // and does not open a non-stream GET.
    let cookie_same_origin = [("Cookie", f.cookie.as_str()), ("Sec-Fetch-Site", "same-origin")];
    assert_eq!(raw_http(port, "POST", "/api/sessions", &cookie_same_origin, b"{}").0, 401);
    assert_eq!(raw_http(port, "GET", "/api/sessions", &cookie_same_origin, b"").0, 401);
    assert_eq!(session_count(&f), 3);

    // An unauthenticated request announcing a 50 MiB body is refused from its headers alone:
    // the 401 arrives although the body is never sent.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let t = Instant::now();
    write!(s, "POST /api/sessions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 52428800\r\n\r\n").unwrap();
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).unwrap();
    assert!(line.starts_with("HTTP/1.1 401"), "{line}");
    assert!(t.elapsed() < Duration::from_secs(5), "refusal must not wait for the body");
    // And the server closes the connection after a bounded wait instead of holding it open.
    let mut rest = Vec::new();
    let _ = s.read_to_end(&mut rest);
    assert!(t.elapsed() < Duration::from_secs(8), "connection held open for {:?}", t.elapsed());
    f.srv.shutdown();
}

#[test]
fn workspace_overlapping_data_dir_is_refused_and_base_url_cannot_carry_secrets() {
    let f = fixture(None);
    let port = f.port;
    let token = [("X-ReSearch-Token", f.tok.as_str())];
    // The data directory itself, a folder inside it, and a folder containing it.
    let inside = f.data_dir.join("sessions");
    std::fs::create_dir_all(&inside).unwrap();
    for ws in [f.data_dir.clone(), inside, f.data_dir.parent().unwrap().to_path_buf()] {
        let body = serde_json::to_vec(&json!({ "workspace_root": ws })).unwrap();
        let (st, _, b) = raw_http(port, "POST", "/api/sessions", &token, &body);
        assert_eq!(st, 400, "workspace {} must be refused: {}", ws.display(), String::from_utf8_lossy(&b));
        assert!(String::from_utf8_lossy(&b).contains("overlaps"), "{}", String::from_utf8_lossy(&b));
    }
    assert_eq!(session_count(&f), 1);

    for bad in ["https://user:secret@example.org/v1", "https://example.org/v1?key=secret", "https://example.org/v1#frag"] {
        let body = serde_json::to_vec(&json!({ "base_urls": { "openai": bad } })).unwrap();
        let (st, _, b) = raw_http(port, "PUT", "/api/settings/providers", &token, &body);
        assert_eq!(st, 400, "base_url {bad}: {}", String::from_utf8_lossy(&b));
        assert!(!String::from_utf8_lossy(&b).contains("secret"), "error must not echo the URL");
    }
    let (_, _, settings) = raw_http(port, "GET", "/api/settings/providers", &token, b"");
    assert!(!String::from_utf8_lossy(&settings).contains("secret"));
    let ok = serde_json::to_vec(&json!({ "base_urls": { "openai": "https://example.org/v1" } })).unwrap();
    assert_eq!(raw_http(port, "PUT", "/api/settings/providers", &token, &ok).0, 200);
    f.srv.shutdown();
}

/// A request body can no longer redirect the provider call (and with it the stored key):
/// `api_base` in the body is ignored and the configured base is used.
#[test]
fn request_body_api_base_is_ignored() {
    let reply = json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": "A short answer." }],
        "usage": { "input_tokens": 5, "output_tokens": 5 }
    });
    let configured = MockHttp::start(vec![MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&reply).unwrap(),
    }]);
    // The decoy would receive the request if the body override were honoured.
    let decoy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    decoy.set_nonblocking(true).unwrap();
    let decoy_url = format!("http://{}", decoy.local_addr().unwrap());

    std::env::set_var("RE_SEARCH_API_KEY", "sk-desktop-test-secret-0002");
    let f = fixture(Some(configured.base_url.clone()));
    let token = [("X-ReSearch-Token", f.tok.as_str())];
    let body = serde_json::to_vec(&json!({ "prompt": "Why?", "provider": "anthropic", "api_base": decoy_url })).unwrap();
    let (st, _, b) = raw_http(f.port, "POST", &format!("/api/sessions/{}/messages", f.sid), &token, &body);
    assert!(matches!(st, 200 | 202), "{st}: {}", String::from_utf8_lossy(&b));
    // Positive control: the turn's model call reaches the configured base. `finish()` returns
    // once the mock has served it; run it on a thread so a regression fails instead of hanging.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        configured.finish();
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(30)).expect("the configured base was never contacted");
    // ... and nothing ever connects to the base named in the request body.
    std::thread::sleep(Duration::from_millis(500));
    assert!(decoy.accept().is_err(), "the body's api_base must not be contacted");
    f.srv.shutdown();
}

/// The shared test configuration can never open a browser or reach OpenAI: its opener
/// refuses and its endpoints are a closed loopback port, even when a test drives
/// sign-in and "Open in browser" with a valid token.
#[test]
fn shared_test_config_never_opens_a_browser_or_reaches_openai() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_cfg(tmp.path(), None, None, None);
    let hooks = cfg.chatgpt_test.clone().expect("test_cfg must carry ChatGPT test hooks");
    assert!(hooks.mock_base.starts_with("http://127.0.0.1:"), "{}", hooks.mock_base);
    let err = (hooks.opener)("https://auth.openai.com/api/accounts/authorize?client_id=x").unwrap_err();
    assert_eq!(err, desktop_server::chatgpt::TEST_OPENER_REFUSAL);
    let hub = desktop_server::chatgpt::ChatGptHub::new(&cfg).unwrap();
    assert!(hub.api_base().starts_with("http://127.0.0.1:"), "{}", hub.api_base());
    assert!(!hub.auth.endpoints().is_production());

    let f = fixture(None);
    let token = [("X-ReSearch-Token", f.tok.as_str())];
    let (st, _, body) = raw_http(f.port, "POST", "/api/connections/chatgpt/login", &token, b"");
    if cfg!(windows) {
        assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
        let url = serde_json::from_slice::<Value>(&body).unwrap()["authorize_url"].as_str().unwrap().to_string();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        let (st, _, body) = raw_http(f.port, "POST", "/api/connections/chatgpt/open", &token, b"");
        assert_eq!(st, 500, "{}", String::from_utf8_lossy(&body));
        assert!(String::from_utf8_lossy(&body).contains("browser opening disabled"));
    } else {
        assert_eq!(st, 400, "sign-in is not supported off Windows");
    }
    f.srv.shutdown();
}
