//! Narrate proof and side questions choose their provider from the request (the model
//! picker's choice) when it is an API provider; with `claude-cli` (or nothing) they use the
//! first API provider that has a key, else a clear 400. The only "provider" here is a
//! loopback mock that records what it was asked; no real network call is made.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::start_desktop_server;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// Loopback mock answering 400 to everything; records `(request line, body)`.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

fn mock() -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            log.lock().unwrap().push((line.trim().to_string(), String::from_utf8_lossy(&body).to_string()));
            let reply = b"{\"error\":{\"message\":\"mock refuses\",\"type\":\"invalid_request_error\"}}";
            let mut s = reader.into_inner();
            let head = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(reply);
            let _ = s.flush();
        }
    });
    (base, seen)
}

#[test]
fn narrate_and_sidetalk_use_an_api_provider_or_say_they_need_one() {
    // This test binary's own environment: no provider key may reach even the mock.
    for p in [agent::ProviderKind::Anthropic, agent::ProviderKind::OpenAi, agent::ProviderKind::Gemini, agent::ProviderKind::Ollama] {
        for name in p.key_env_vars() {
            std::env::remove_var(name);
        }
    }
    let tmp = TempDir::new().unwrap();
    let cfg = test_cfg(&tmp.path().join("data"), None, None, None);
    let srv = start_desktop_server(cfg).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let call = |method: &str, path: &str, body: Value| -> (u16, Value, String) {
        let hdrs = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];
        let (st, _, b) = raw_http(port, method, path, &hdrs, body.to_string().as_bytes());
        (st, serde_json::from_slice(&b).unwrap_or(Value::Null), String::from_utf8_lossy(&b).to_string())
    };
    let (st, v, _) = call("POST", "/api/sessions", json!({ "title": "side" }));
    assert_eq!(st, 200, "{v}");
    let sid = v["id"].as_str().unwrap().to_string();
    let narrate = format!("/api/sessions/{sid}/narrate");
    let sidetalk = format!("/api/sessions/{sid}/sidetalk");

    // No API provider connected: a clear 400 for the subscription choice and for none.
    for body in [json!({ "provider": "claude-cli", "model": "sonnet" }), json!({})] {
        let (st, _, text) = call("POST", &narrate, body.clone());
        assert_eq!(st, 400, "{text}");
        assert!(text.contains("need an API-key provider"), "{text}");
        let mut q = body.clone();
        q["question"] = json!("why?");
        let (st, _, text) = call("POST", &sidetalk, q);
        assert_eq!(st, 400, "{text}");
        assert!(text.contains("need an API-key provider"), "{text}");
    }

    // An OpenAI key (saved, pointed at the mock): the subscription choice falls back to it
    // with its default model; an explicit API choice is used as given.
    let (base, seen) = mock();
    let (st, _, text) = call(
        "PUT",
        "/api/settings/providers",
        json!({ "keys": { "openai": "sk-test-not-a-key-0001" }, "base_urls": { "openai": base } }),
    );
    assert_eq!(st, 200, "{text}");
    // Narrate and side questions need a started session (empty pre-start sessions reject before hitting providers).
    ledger::Ledger::open(&tmp.path().join("data"))
        .unwrap()
        .append(&sid, "session.start", 1, &json!({ "prompt": "P" }))
        .unwrap();
    let (st, _, text) = call("POST", &narrate, json!({ "provider": "claude-cli", "model": "sonnet" }));
    assert_eq!(st, 400, "the mock refuses: {text}");
    assert!(!text.contains("need an API-key provider"), "{text}");
    let default_model = agent::ProviderKind::OpenAi.default_model();
    {
        let reqs = seen.lock().unwrap();
        assert_eq!(reqs.len(), 1, "{reqs:?}");
        assert!(reqs[0].0.contains("chat/completions"), "{reqs:?}");
        assert!(reqs[0].1.contains(&format!("\"model\":\"{default_model}\"")), "{reqs:?}");
        assert!(!reqs[0].1.contains("\"sonnet\""), "{reqs:?}");
    }
    let (st, _, text) = call("POST", &sidetalk, json!({ "question": "why?", "provider": "openai", "model": "gpt-4.1-mini" }));
    assert_eq!(st, 400, "{text}");
    let want = agent::ProviderKind::OpenAi.resolve_model("gpt-4.1-mini");
    let reqs = seen.lock().unwrap().clone();
    assert!(reqs.iter().any(|(_, b)| b.contains(&format!("\"model\":\"{want}\""))), "{reqs:?}");
    srv.shutdown();
}
