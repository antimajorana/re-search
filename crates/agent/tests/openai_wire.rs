//! Integration tests for `OpenAiTransport` (OpenAI Chat Completions and Ollama
//! preset), covering env var precedence, keyless loopback vs fail-closed HTTPS,
//! wire serialization/deserialization, Ollama compatibility, HTTP errors, and
//! secret scrubbing over an in-process `127.0.0.1:0` mock server.

use agent::{
    ContentBlock, Message, ModelRequest, OpenAiConfig, OpenAiTransport, ProviderKind, Role, SecretKey, StopReason,
    ToolSpec, Transport, TransportErrorKind,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvReset(Vec<(&'static str, Option<String>)>);

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
        Self(saved)
    }
}

impl Drop for EnvReset {
    fn drop(&mut self) {
        for (k, prev) in self.0.drain(..) {
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

fn spawn_server(
    status_line: &'static str,
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
        let mut content_len = 0usize;
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
        *cap_clone.lock().unwrap() = Some(CapturedRequest { method, target, headers: req_headers, body: req_body });

        let head = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
    });

    (format!("http://{addr}"), captured, handle)
}

fn minimal_request(model: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        system: None,
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }],
        tools: vec![],
        max_tokens: 64,
        reasoning_effort: None,
    }
}

#[test]
fn env_precedence_keyless_loopback_and_fail_closed_https() {
    let _guard = ENV_LOCK.lock().unwrap();

    for bad_provider in [ProviderKind::Anthropic, ProviderKind::Gemini] {
        let cfg = OpenAiConfig { provider: bad_provider, ..Default::default() };
        let key = Some(SecretKey::new("sk-openai-test-secret-99".into()).unwrap());
        let err = OpenAiTransport::new(cfg, key, "X").err().expect("provider rejected");
        assert_eq!(err.kind, TransportErrorKind::Config);
    }

    // OpenAI HTTPS, new(key=None) on HTTPS, and remote HTTPS Ollama without key all fail closed.
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("OPENAI_API_KEY", None),
            ("OLLAMA_API_KEY", None),
        ]);
        assert_eq!(OpenAiTransport::from_env(OpenAiConfig::default()).err().unwrap().kind, TransportErrorKind::Config);
        assert_eq!(OpenAiTransport::new(OpenAiConfig::default(), None, "none").err().unwrap().kind, TransportErrorKind::Config);
        let remote_ollama = OpenAiConfig {
            provider: ProviderKind::Ollama,
            base_url: "https://ollama.example.com/v1".into(),
            ..Default::default()
        };
        assert_eq!(OpenAiTransport::from_env(remote_ollama).err().unwrap().kind, TransportErrorKind::Config);

        // Keyless loopback works for both Ollama default and loopback OpenAI.
        let ollama = OpenAiTransport::from_env(OpenAiConfig::ollama_default()).unwrap();
        assert_eq!(ollama.info().provider, "ollama-chat");
        assert_eq!(ollama.info().endpoint, "http://127.0.0.1:11434/v1/chat/completions");
        assert_eq!(ollama.info().api_key_env, "none");

        let openai_local = OpenAiTransport::from_env(OpenAiConfig {
            base_url: "http://127.0.0.1:8000".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(openai_local.info().provider, "openai-chat");
        assert_eq!(openai_local.info().endpoint, "http://127.0.0.1:8000/v1/chat/completions");
        assert_eq!(openai_local.info().api_key_env, "none");
    }

    // Precedence: OpenAI (RE_SEARCH > RESEARCH_CORE > OPENAI) and Ollama (+ OLLAMA before OPENAI).
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", Some("sk-openai-test-secret-99")),
            ("RESEARCH_CORE_API_KEY", Some("sk-openai-core-22")),
            ("OPENAI_API_KEY", Some("sk-openai-direct-33")),
        ]);
        assert_eq!(OpenAiTransport::from_env(OpenAiConfig::default()).unwrap().info().api_key_env, "RE_SEARCH_API_KEY");
    }
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", Some("  ")),
            ("RESEARCH_CORE_API_KEY", None),
            ("OPENAI_API_KEY", Some("sk-openai-direct-33")),
        ]);
        assert_eq!(OpenAiTransport::from_env(OpenAiConfig::default()).unwrap().info().api_key_env, "OPENAI_API_KEY");
    }
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("OLLAMA_API_KEY", Some("sk-ollama-key-11")),
            ("OPENAI_API_KEY", Some("sk-openai-fallback-22")),
        ]);
        assert_eq!(OpenAiTransport::from_env(OpenAiConfig::ollama_default()).unwrap().info().api_key_env, "OLLAMA_API_KEY");
    }
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("OLLAMA_API_KEY", None),
            ("OPENAI_API_KEY", Some("sk-openai-fallback-22")),
        ]);
        assert_eq!(OpenAiTransport::from_env(OpenAiConfig::ollama_default()).unwrap().info().api_key_env, "OPENAI_API_KEY");
    }

    for (base, expected) in [
        ("https://api.openai.com", "https://api.openai.com/v1/chat/completions"),
        ("https://api.openai.com/v1", "https://api.openai.com/v1/chat/completions"),
        ("https://api.openai.com/v1/chat/completions", "https://api.openai.com/v1/chat/completions"),
        ("https://proxy.example.com/custom/chat/completions", "https://proxy.example.com/custom/chat/completions"),
        ("https://proxy.example.com/custom", "https://proxy.example.com/custom/v1/chat/completions"),
    ] {
        let cfg = OpenAiConfig { base_url: base.into(), ..Default::default() };
        let key = SecretKey::new("sk-openai-test-secret-99".into()).unwrap();
        assert_eq!(OpenAiTransport::new(cfg, Some(key), "OPENAI_API_KEY").unwrap().info().endpoint, expected);
    }
}

fn send_once(status_line: &'static str, body: Vec<u8>, secret: &str, req: &ModelRequest) -> Result<agent::ModelResponse, agent::TransportError> {
    let (base_url, _, handle) = spawn_server(status_line, body);
    let t = OpenAiTransport::new(
        OpenAiConfig { base_url, ..Default::default() },
        Some(SecretKey::new(secret.into()).unwrap()),
        "OPENAI_API_KEY",
    )
    .unwrap();
    let prep = t.prepare_request(req).unwrap();
    let res = t.send_prepared(&prep);
    handle.join().unwrap();
    res
}

#[test]
fn openai_wire_serialization_tool_roundtrip_and_resumed_trailing_user_order() {
    let secret = "sk-openai-test-secret-99";
    let resp_json = json!({
        "id": "chatcmpl-roundtrip-1",
        "model": "gpt-4.1-mini",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "reasoning_content": "Let me inspect the equation first.",
                "content": "Calling tool now.",
                "tool_calls": [{
                    "id": "call_abc_1",
                    "type": "function",
                    "function": { "name": "ledger_note", "arguments": "{\"text\":\"verified\"}" }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 25, "completion_tokens": 12 }
    });
    let raw_bytes = serde_json::to_vec(&resp_json).unwrap();
    let (base_url, captured, handle) = spawn_server("200 OK", raw_bytes.clone());

    let transport = OpenAiTransport::new(
        OpenAiConfig { base_url, ..Default::default() },
        Some(SecretKey::new(secret.into()).unwrap()),
        "OPENAI_API_KEY",
    )
    .unwrap();

    // Resumed-style history: Assistant(Thinking + Text + ToolUse), User(ToolResult + trailing Text).
    let req = ModelRequest {
        model: "gpt-4.1-mini".into(),
        system: Some("System instructions".into()),
        messages: vec![
            Message { role: Role::User, content: vec![ContentBlock::Text { text: "Compute 1+1".into() }] },
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Thinking { thinking: "Cross-provider thought".into(), signature: Some("sig-1".into()) },
                    ContentBlock::RedactedThinking { data: "opaque".into() },
                    ContentBlock::Other { block_type: "gemini_thought_signature".into(), raw: json!({"x": 1}) },
                    ContentBlock::Text { text: "Checking...".into() },
                    ContentBlock::ToolUse { id: "call_prev".into(), name: "ledger_note".into(), input: json!({ "text": "prior" }) },
                ],
            },
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult { tool_use_id: "call_prev".into(), is_error: false, content: "{\"ok\":true}".into() },
                    ContentBlock::Text { text: "Continue after interruption.".into() },
                ],
            },
        ],
        tools: vec![ToolSpec {
            name: "ledger_note".into(),
            description: "Record note".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false
            }),
        }],
        max_tokens: 512,
        reasoning_effort: None,
    };

    let prepared = transport.prepare_request(&req).unwrap();
    let prep_json: Value = serde_json::from_slice(&prepared.body).unwrap();
    assert!(!String::from_utf8_lossy(&prepared.body).contains(secret));

    // OpenAI uses max_completion_tokens, not max_tokens.
    assert_eq!(prep_json["model"], "gpt-4.1-mini");
    assert_eq!(prep_json["max_completion_tokens"], 512);
    assert!(prep_json.get("max_tokens").is_none());
    assert_eq!(prep_json["tools"][0]["type"], "function");
    assert_eq!(prep_json["tools"][0]["function"]["name"], "ledger_note");
    assert_eq!(prep_json["tools"][0]["function"]["parameters"]["additionalProperties"], false);

    // Messages order: system -> user -> assistant(tool_calls, no thinking/other) -> tool -> trailing user.
    let msgs = prep_json["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 5);
    assert_eq!(msgs[0], json!({"role": "system", "content": "System instructions"}));
    assert_eq!(msgs[1], json!({"role": "user", "content": "Compute 1+1"}));
    assert_eq!(msgs[2]["role"], "assistant");
    assert_eq!(msgs[2]["content"], "Checking...");
    assert_eq!(msgs[2]["tool_calls"][0]["id"], "call_prev");
    assert_eq!(msgs[2]["tool_calls"][0]["type"], "function");
    assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "ledger_note");
    let args_parsed: Value = serde_json::from_str(msgs[2]["tool_calls"][0]["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args_parsed, json!({ "text": "prior" }));
    assert_eq!(msgs[3], json!({"role": "tool", "tool_call_id": "call_prev", "content": "{\"ok\":true}"}));
    assert_eq!(msgs[4], json!({"role": "user", "content": "Continue after interruption."}));

    // Assistant with tool_use and empty text uses content: null.
    let no_text_req = ModelRequest {
        model: "gpt-4.1-mini".into(),
        system: None,
        messages: vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse { id: "c1".into(), name: "ledger_note".into(), input: json!({}) }],
        }],
        tools: vec![],
        max_tokens: 32,
        reasoning_effort: None,
    };
    let no_text_json: Value = serde_json::from_slice(&transport.prepare_request(&no_text_req).unwrap().body).unwrap();
    assert_eq!(no_text_json["messages"][0]["content"], Value::Null);

    let resp = transport.send_prepared(&prepared).unwrap();
    handle.join().unwrap();

    let cap = captured.lock().unwrap().clone().unwrap();
    assert_eq!(cap.method, "POST");
    assert_eq!(cap.target, "/v1/chat/completions");
    assert_eq!(cap.body, prepared.body);
    let auth_hdr = cap.headers.iter().find(|(k, _)| k == "authorization").map(|(_, v)| v.as_str());
    assert_eq!(auth_hdr, Some("Bearer sk-openai-test-secret-99"));

    assert_eq!(resp.id.as_deref(), Some("chatcmpl-roundtrip-1"));
    assert_eq!(resp.model, "gpt-4.1-mini");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(resp.raw_body, raw_bytes);
    assert_eq!(resp.content.len(), 3);
    assert_eq!(
        resp.content[0],
        ContentBlock::Thinking { thinking: "Let me inspect the equation first.".into(), signature: None }
    );
    assert_eq!(resp.content[1], ContentBlock::Text { text: "Calling tool now.".into() });
    assert_eq!(
        resp.content[2],
        ContentBlock::ToolUse { id: "call_abc_1".into(), name: "ledger_note".into(), input: json!({ "text": "verified" }) }
    );
}

#[test]
fn ollama_keyless_compatibility_max_tokens_and_synthesized_tool_id() {
    let resp_json = json!({
        "model": "qwen2.5-coder:7b",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "type": "function",
                    "function": { "name": "sympy_eval", "arguments": { "code": "2+2" } }
                }]
            },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
    });
    let (base_url, captured, handle) = spawn_server("200 OK", serde_json::to_vec(&resp_json).unwrap());

    let transport = OpenAiTransport::new(
        OpenAiConfig { provider: ProviderKind::Ollama, base_url, ..Default::default() },
        None,
        "none",
    )
    .unwrap();

    let req = minimal_request("qwen2.5-coder:7b");
    let prep = transport.prepare_request(&req).unwrap();
    let prep_json: Value = serde_json::from_slice(&prep.body).unwrap();
    // Ollama uses max_tokens, not max_completion_tokens.
    assert_eq!(prep_json["max_tokens"], 64);
    assert!(prep_json.get("max_completion_tokens").is_none());

    let resp = transport.send_prepared(&prep).unwrap();
    handle.join().unwrap();

    let cap = captured.lock().unwrap().clone().unwrap();
    assert!(cap.headers.iter().all(|(k, _)| k != "authorization"), "keyless loopback must omit Authorization");

    // finish_reason: "stop" with tool_calls maps to StopReason::ToolUse; missing id synthesizes call_0.
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(
        resp.content,
        vec![ContentBlock::ToolUse { id: "call_0".into(), name: "sympy_eval".into(), input: json!({ "code": "2+2" }) }]
    );
}

#[test]
fn openai_refusal_malformed_arguments_http_errors_and_secret_scrubbing() {
    let secret = "sk-openai-test-secret-99";
    let req = minimal_request("gpt-4.1-mini");

    // Invalid JSON and non-object JSON in function.arguments -> MalformedResponse.
    for args_json in [json!("{not-json"), json!("[1,2,3]")] {
        let body = json!({
            "id": "c1",
            "model": "gpt-4.1-mini",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "ledger_note", "arguments": args_json } }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let err = send_once("200 OK", serde_json::to_vec(&body).unwrap(), secret, &req).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::MalformedResponse);
    }

    // Refusal via message.refusal (omits tool_calls on refusal).
    let refusal_body = json!({
        "id": "c3",
        "model": "gpt-4.1-mini",
        "choices": [{
            "message": {
                "role": "assistant",
                "refusal": "I cannot assist with that request.",
                "tool_calls": [{ "id": "call_ignored", "type": "function", "function": { "name": "ledger_note", "arguments": "{\"text\":\"x\"}" } }]
            },
            "finish_reason": "stop"
        }]
    });
    let resp = send_once("200 OK", serde_json::to_vec(&refusal_body).unwrap(), secret, &req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::Refusal);
    assert_eq!(resp.content, vec![ContentBlock::Text { text: "I cannot assist with that request.".into() }]);

    // Refusal via finish_reason: "content_filter".
    let filter_body = json!({
        "id": "c4",
        "model": "gpt-4.1-mini",
        "choices": [{
            "message": { "role": "assistant", "content": "Filtered output.", "refusal": null },
            "finish_reason": "content_filter"
        }]
    });
    let resp = send_once("200 OK", serde_json::to_vec(&filter_body).unwrap(), secret, &req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::Refusal);
    assert_eq!(resp.content, vec![ContentBlock::Text { text: "Filtered output.".into() }]);

    // 401 Auth with secret reflection -> scrubbed in message and raw_body.
    let err_401 = format!(r#"{{"error":{{"message":"Incorrect API key provided: {secret}","type":"invalid_request_error"}}}}"#);
    let err = send_once("401 Unauthorized", err_401.into_bytes(), secret, &req).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::Auth);
    assert_eq!(err.status, Some(401));
    assert!(!err.message.contains(secret));
    let raw = String::from_utf8(err.raw_body.unwrap()).unwrap();
    assert!(!raw.contains(secret));
    assert!(raw.contains("[REDACTED]"));

    // 429 RateLimit & 500 HttpStatus.
    for (status_line, expected_kind, expected_code) in [
        ("429 Too Many Requests", TransportErrorKind::RateLimit, 429),
        ("500 Internal Server Error", TransportErrorKind::HttpStatus, 500),
    ] {
        let err = send_once(status_line, br#"{"error":{"message":"server issue"}}"#.to_vec(), secret, &req).unwrap_err();
        assert_eq!(err.kind, expected_kind);
        assert_eq!(err.status, Some(expected_code));
    }
}
