//! Integration tests for `AnthropicTransport`, `SecretKey`, URL validation,
//! wire serialization/deserialization, HTTP status mapping, and secret scrubbing
//! over an in-process `127.0.0.1:0` mock server (zero external network calls).

use agent::{
    resolve_api_model, AnthropicConfig, AnthropicTransport, ContentBlock, Message, ModelRequest, Role, SecretKey,
    StopReason, ToolSpec, Transport, TransportErrorKind, ANTHROPIC_VERSION, DEFAULT_API_MODEL,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvReset {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvReset {
    fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
        let mut saved = Vec::with_capacity(vars.len());
        for &(k, v) in vars {
            saved.push((k, std::env::var(k).ok()));
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        Self { saved }
    }
}

impl Drop for EnvReset {
    fn drop(&mut self) {
        for (k, prev) in self.saved.drain(..) {
            match prev {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn spawn_single_response_server(
    status_line: &'static str,
    headers: &'static [(&'static str, &'static str)],
    body: Vec<u8>,
) -> (String, Arc<Mutex<Option<CapturedRequest>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(None));
    let cap_clone = captured.clone();

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());

        let mut req_line = String::new();
        reader.read_line(&mut req_line).unwrap();
        let mut parts = req_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("").to_string();

        let mut req_headers = Vec::new();
        let mut content_len: usize = 0;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).unwrap();
            if n == 0 || line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((k, v)) = line.trim_end_matches(['\r', '\n']).split_once(':') {
                let name = k.trim().to_ascii_lowercase();
                let val = v.trim().to_string();
                if name == "content-length" {
                    content_len = val.parse().unwrap_or(0);
                }
                req_headers.push((name, val));
            }
        }

        let mut req_body = vec![0u8; content_len];
        if content_len > 0 {
            reader.read_exact(&mut req_body).unwrap();
        }
        *cap_clone.lock().unwrap() = Some(CapturedRequest {
            method,
            target,
            headers: req_headers,
            body: req_body,
        });

        let mut head = format!("HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
        for (k, v) in headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
    });

    (format!("http://{addr}"), captured, handle)
}

#[test]
fn secret_key_precedence_validation_and_redacted_debug() {
    let _guard = ENV_LOCK.lock().unwrap();

    // Missing all three env vars -> Config error.
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("ANTHROPIC_API_KEY", None),
        ]);
        let err = SecretKey::from_env().unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config);
    }

    // Precedence: RE_SEARCH_API_KEY > RESEARCH_CORE_API_KEY > ANTHROPIC_API_KEY, skipping empty/whitespace.
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", Some("sk-re-search")),
            ("RESEARCH_CORE_API_KEY", Some("sk-research-core")),
            ("ANTHROPIC_API_KEY", Some("sk-anthropic")),
        ]);
        let (key, env_name) = SecretKey::from_env().unwrap();
        assert_eq!(env_name, "RE_SEARCH_API_KEY");
        let dbg = format!("{key:?}");
        assert!(dbg.contains("[REDACTED]"), "{dbg}");
        assert!(!dbg.contains("sk-re-search"), "{dbg}");
    }
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", Some("   ")),
            ("RESEARCH_CORE_API_KEY", Some("sk-research-core")),
            ("ANTHROPIC_API_KEY", Some("sk-anthropic")),
        ]);
        let (_key, env_name) = SecretKey::from_env().unwrap();
        assert_eq!(env_name, "RESEARCH_CORE_API_KEY");
    }
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", Some("")),
            ("RESEARCH_CORE_API_KEY", None),
            ("ANTHROPIC_API_KEY", Some("sk-anthropic")),
        ]);
        let (_key, env_name) = SecretKey::from_env().unwrap();
        assert_eq!(env_name, "ANTHROPIC_API_KEY");
    }

    // Reject empty, whitespace, and control chars (\r, \n, \0, \x7f).
    for bad in ["", "   \t ", "sk-\rbad", "sk-\nbad", "sk-\0bad", "sk-\x7fbad", "sk-\x01bad"] {
        let err = SecretKey::new(bad.to_string()).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "expected Config error for {bad:?}");
    }

    assert_eq!(resolve_api_model(""), DEFAULT_API_MODEL);
    assert_eq!(resolve_api_model("haiku"), DEFAULT_API_MODEL);
    assert_eq!(resolve_api_model(" haiku "), DEFAULT_API_MODEL);
    assert_eq!(resolve_api_model("claude-sonnet-4-5"), "claude-sonnet-4-5");
}

#[test]
fn base_url_validation_rejects_unsafe_urls_and_accepts_https_and_loopback() {
    let key = || SecretKey::new("sk-test-url".into()).unwrap();

    for bad in [
        "http://example.com",
        "http://api.anthropic.com",
        "https://user:pass@api.anthropic.com",
        "https://user@api.anthropic.com",
        "https://api.anthropic.com?foo=1",
        "https://api.anthropic.com/v1#frag",
        "ftp://api.anthropic.com",
        "not-a-url",
        "",
    ] {
        let cfg = AnthropicConfig { base_url: bad.to_string(), ..Default::default() };
        let err = match AnthropicTransport::new(cfg, key(), "RE_SEARCH_API_KEY") {
            Ok(_) => panic!("expected rejection for URL {bad:?}"),
            Err(e) => e,
        };
        assert_eq!(err.kind, TransportErrorKind::Config, "bad url {bad:?}: {err}");
    }

    for (base, expected_endpoint) in [
        ("https://api.anthropic.com", "https://api.anthropic.com/v1/messages"),
        ("https://api.anthropic.com/", "https://api.anthropic.com/v1/messages"),
        ("http://127.0.0.1:8080", "http://127.0.0.1:8080/v1/messages"),
        ("http://localhost:9000/", "http://localhost:9000/v1/messages"),
        ("http://[::1]:8181", "http://[::1]:8181/v1/messages"),
    ] {
        let cfg = AnthropicConfig { base_url: base.to_string(), ..Default::default() };
        let t = AnthropicTransport::new(cfg, key(), "RE_SEARCH_API_KEY")
            .unwrap_or_else(|e| panic!("expected acceptance for {base:?}: {e}"));
        let info = t.info();
        assert_eq!(info.provider, "anthropic-messages");
        assert_eq!(info.endpoint, expected_endpoint);
        assert_eq!(info.api_key_env, "RE_SEARCH_API_KEY");
    }
}

#[test]
fn wire_request_response_roundtrip_with_thinking_tool_use_and_tool_result() {
    let secret = "sk-wire-secret-token-12345";
    let resp_json = json!({
        "id": "msg_roundtrip_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "tool_use",
        "content": [
            { "type": "thinking", "thinking": "Step 1", "signature": "sig-abc" },
            { "type": "redacted_thinking", "data": "opaque-blob" },
            { "type": "text", "text": "Calling tool now." },
            { "type": "tool_use", "id": "toolu_01", "name": "ledger_note", "input": { "text": "check" } },
            { "type": "server_tool_use", "id": "srv_1", "name": "future_block" }
        ],
        "usage": { "input_tokens": 42, "output_tokens": 17 }
    });
    let raw_resp_bytes = serde_json::to_vec(&resp_json).unwrap();

    let (base_url, captured, handle) = spawn_single_response_server(
        "200 OK",
        &[("Content-Type", "application/json")],
        raw_resp_bytes.clone(),
    );
    let transport = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "RE_SEARCH_API_KEY",
    )
    .unwrap();

    let req = ModelRequest {
        model: "claude-haiku-4-5".into(),
        system: Some("System instructions".into()),
        messages: vec![
            Message { role: Role::User, content: vec![ContentBlock::Text { text: "Compute 1+1".into() }] },
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Thinking { thinking: "Initial".into(), signature: Some("sig-prev".into()) },
                    ContentBlock::RedactedThinking { data: "redacted-prev".into() },
                    ContentBlock::ToolUse {
                        id: "toolu_prev".into(),
                        name: "ledger_note".into(),
                        input: json!({ "text": "prior" }),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "toolu_prev".into(),
                    is_error: false,
                    content: "{\"recorded\":true}".into(),
                }],
            },
        ],
        tools: vec![ToolSpec {
            name: "ledger_note".into(),
            description: "Record note".into(),
            input_schema: json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
        }],
        max_tokens: 1024,
        reasoning_effort: None,
    };

    let prepared = transport.prepare_request(&req).unwrap();
    let prepared_json: Value = serde_json::from_slice(&prepared.body).unwrap();
    assert!(!String::from_utf8_lossy(&prepared.body).contains(secret));

    let resp = transport.send_prepared(&prepared).unwrap();
    handle.join().unwrap();

    let cap = captured.lock().unwrap().clone().expect("server captured request");
    assert_eq!(cap.method, "POST");
    assert_eq!(cap.target, "/v1/messages");
    assert_eq!(cap.body, prepared.body);

    let header_val = |name: &str| {
        cap.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(header_val("x-api-key"), Some(secret));
    assert_eq!(header_val("anthropic-version"), Some(ANTHROPIC_VERSION));
    assert_eq!(header_val("content-type"), Some("application/json"));

    assert_eq!(prepared_json["model"], "claude-haiku-4-5");
    assert_eq!(prepared_json["max_tokens"], 1024);
    assert_eq!(prepared_json["system"], "System instructions");
    assert_eq!(prepared_json["tools"][0]["name"], "ledger_note");
    assert_eq!(prepared_json["messages"][1]["content"][0]["type"], "thinking");
    assert_eq!(prepared_json["messages"][1]["content"][0]["signature"], "sig-prev");
    assert_eq!(prepared_json["messages"][1]["content"][1]["type"], "redacted_thinking");
    assert_eq!(prepared_json["messages"][1]["content"][2]["type"], "tool_use");
    assert_eq!(prepared_json["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(prepared_json["messages"][2]["content"][0]["tool_use_id"], "toolu_prev");
    assert_eq!(prepared_json["messages"][2]["content"][0]["is_error"], false);

    assert_eq!(resp.raw_body, raw_resp_bytes);
    assert_eq!(resp.id.as_deref(), Some("msg_roundtrip_1"));
    assert_eq!(resp.model, "claude-haiku-4-5");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(resp.usage, json!({ "input_tokens": 42, "output_tokens": 17 }));
    assert_eq!(resp.content.len(), 5);
    assert_eq!(
        resp.content[0],
        ContentBlock::Thinking { thinking: "Step 1".into(), signature: Some("sig-abc".into()) }
    );
    assert_eq!(resp.content[1], ContentBlock::RedactedThinking { data: "opaque-blob".into() });
    assert_eq!(resp.content[2], ContentBlock::Text { text: "Calling tool now.".into() });
    assert_eq!(
        resp.content[3],
        ContentBlock::ToolUse {
            id: "toolu_01".into(),
            name: "ledger_note".into(),
            input: json!({ "text": "check" }),
        }
    );
    assert!(matches!(
        &resp.content[4],
        ContentBlock::Other { block_type, .. } if block_type == "server_tool_use"
    ));

    // Request with system=None and tools=[] omits optional fields cleanly.
    let min_req = ModelRequest {
        model: "claude-haiku-4-5".into(),
        system: None,
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "ping".into() }] }],
        tools: vec![],
        max_tokens: 16,
        reasoning_effort: None,
    };
    let min_prep = transport.prepare_request(&min_req).unwrap();
    let min_json: Value = serde_json::from_slice(&min_prep.body).unwrap();
    assert!(min_json.get("system").is_none());
    assert!(min_json.get("tools").is_none());
}

#[test]
fn http_errors_malformed_responses_redirects_and_secret_scrubbing() {
    let secret = "sk-scrub-me-xyz987";

    let simple_req = ModelRequest {
        model: "claude-haiku-4-5".into(),
        system: None,
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }],
        tools: vec![],
        max_tokens: 64,
        reasoning_effort: None,
    };

    // 401 Auth + secret reflection scrubbed from raw_body and error message.
    let err_401 = format!(r#"{{"type":"error","error":{{"type":"authentication_error","message":"bad key {secret}"}}}}"#);
    let (base_url, _, handle) = spawn_single_response_server(
        "401 Unauthorized",
        &[("Content-Type", "application/json")],
        err_401.into_bytes(),
    );
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let prep = t.prepare_request(&simple_req).unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::Auth);
    assert_eq!(err.status, Some(401));
    assert!(!err.message.contains(secret), "secret leaked in message: {}", err.message);
    let raw = String::from_utf8(err.raw_body.clone().unwrap()).unwrap();
    assert!(!raw.contains(secret), "secret leaked in raw_body: {raw}");
    assert!(raw.contains("[REDACTED]"), "expected [REDACTED] in raw_body: {raw}");

    // 429 RateLimit.
    let err_429 = br#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#.to_vec();
    let (base_url, _, handle) = spawn_single_response_server(
        "429 Too Many Requests",
        &[("Content-Type", "application/json")],
        err_429,
    );
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::RateLimit);
    assert_eq!(err.status, Some(429));

    // 500 HttpStatus preserves JSON error body in raw_body.
    let err_500 = br#"{"type":"error","error":{"type":"api_error","message":"overloaded"}}"#.to_vec();
    let (base_url, _, handle) = spawn_single_response_server(
        "500 Internal Server Error",
        &[("Content-Type", "application/json")],
        err_500.clone(),
    );
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::HttpStatus);
    assert_eq!(err.status, Some(500));
    assert_eq!(err.raw_body.as_deref(), Some(err_500.as_slice()));

    // 200 OK reflecting the secret in response body must also scrub raw_body and parsed content.
    let reflected_200 = format!(
        r#"{{"id":"m2","role":"assistant","model":"claude-haiku-4-5","stop_reason":"end_turn","content":[{{"type":"text","text":"echo {secret}"}}],"usage":{{}}}}"#
    );
    let (base_url, _, handle) = spawn_single_response_server(
        "200 OK",
        &[("Content-Type", "application/json")],
        reflected_200.into_bytes(),
    );
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let resp = t.send_prepared(&prep).unwrap();
    handle.join().unwrap();
    assert!(!String::from_utf8_lossy(&resp.raw_body).contains(secret));
    assert_eq!(resp.content, vec![ContentBlock::Text { text: "echo [REDACTED]".into() }]);

    // Malformed JSON -> MalformedResponse.
    let (base_url, _, handle) = spawn_single_response_server(
        "200 OK",
        &[("Content-Type", "application/json")],
        b"{not valid json".to_vec(),
    );
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::MalformedResponse);

    // Wrong role -> MalformedResponse.
    let wrong_role = br#"{"id":"m3","role":"user","model":"claude-haiku-4-5","stop_reason":"end_turn","content":[],"usage":{}}"#.to_vec();
    let (base_url, _, handle) = spawn_single_response_server(
        "200 OK",
        &[("Content-Type", "application/json")],
        wrong_role,
    );
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::MalformedResponse);

    // Redirect 302 must NOT be followed (second server should never be contacted).
    let (target_url, target_cap, _target_handle) = spawn_single_response_server(
        "200 OK",
        &[("Content-Type", "application/json")],
        br#"{"id":"m4","role":"assistant","model":"m","stop_reason":"end_turn","content":[],"usage":{}}"#.to_vec(),
    );
    let loc: &'static str = Box::leak(format!("{target_url}/v1/messages").into_boxed_str());
    let loc_header: &'static [(&'static str, &'static str)] =
        Box::leak(vec![("Location", loc)].into_boxed_slice());
    let (redirect_base, _, handle) = spawn_single_response_server("302 Found", loc_header, b"redirect".to_vec());
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url: redirect_base, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::HttpStatus);
    assert_eq!(err.status, Some(302));
    assert!(target_cap.lock().unwrap().is_none(), "302 redirect must not be followed");

    // Error body larger than 1 MiB -> OversizedResponse.
    let big_err = vec![b'x'; 1024 * 1024 + 64];
    let (base_url, _, handle) = spawn_single_response_server("502 Bad Gateway", &[], big_err);
    let t = AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "ANTHROPIC_API_KEY",
    )
    .unwrap();
    let err = t.send_prepared(&prep).unwrap_err();
    handle.join().unwrap();
    assert_eq!(err.kind, TransportErrorKind::OversizedResponse);
    assert_eq!(err.status, Some(502));
}
