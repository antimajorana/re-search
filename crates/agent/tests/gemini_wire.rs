//! Integration tests for `GeminiTransport`, covering env var precedence,
//! model path-injection rejection, `x-goog-api-key` header (zero `?key=`),
//! `parametersJsonSchema` preservation, `thought` + `thoughtSignature` lossless
//! roundtrip, safety/promptFeedback refusals, `MALFORMED_FUNCTION_CALL`, HTTP
//! errors, and secret scrubbing over an in-process `127.0.0.1:0` mock server.

use agent::{
    ContentBlock, GeminiConfig, GeminiTransport, Message, ModelRequest, Role, SecretKey, StopReason, ToolSpec,
    Transport, TransportErrorKind,
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

fn minimal_req(model: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        system: None,
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }],
        tools: vec![],
        max_tokens: 64,
    }
}

#[test]
fn env_precedence_endpoint_construction_and_model_validation() {
    let _guard = ENV_LOCK.lock().unwrap();

    // Missing all env keys -> Config error.
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("GEMINI_API_KEY", None),
            ("GOOGLE_API_KEY", None),
        ]);
        let err = match GeminiTransport::from_env(GeminiConfig::default()) {
            Ok(_) => panic!("expected Config error without key"),
            Err(e) => e,
        };
        assert_eq!(err.kind, TransportErrorKind::Config);
    }

    // Precedence: RE_SEARCH_API_KEY > RESEARCH_CORE_API_KEY > GEMINI_API_KEY > GOOGLE_API_KEY.
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("GEMINI_API_KEY", Some("AIza-gemini-test-secret-99")),
            ("GOOGLE_API_KEY", Some("AIza-google-fallback-11")),
        ]);
        let t = GeminiTransport::from_env(GeminiConfig::default()).unwrap();
        assert_eq!(t.info().api_key_env, "GEMINI_API_KEY");
        assert_eq!(t.info().provider, "gemini-generate-content");
        assert_eq!(
            t.info().endpoint,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
        assert!(!t.info().endpoint.contains("?key="));
    }
    {
        let _env = EnvReset::set(&[
            ("RE_SEARCH_API_KEY", None),
            ("RESEARCH_CORE_API_KEY", None),
            ("GEMINI_API_KEY", Some("  ")),
            ("GOOGLE_API_KEY", Some("AIza-google-fallback-11")),
        ]);
        let t = GeminiTransport::from_env(GeminiConfig::default()).unwrap();
        assert_eq!(t.info().api_key_env, "GOOGLE_API_KEY");
    }

    // Reject path-injection model names.
    for bad_model in [
        "",
        "   ",
        "../models/gemini",
        "gemini/2.5",
        "gemini\\2.5",
        "gemini?key=x",
        "gemini#frag",
        "gemini:batch",
        "gemini%2f",
        "gemini@host",
        "gemini 2.5",
        "gemini\n2.5",
    ] {
        let cfg = GeminiConfig { model: bad_model.into(), ..Default::default() };
        let key = SecretKey::new("AIza-gemini-test-secret-99".into()).unwrap();
        let err = match GeminiTransport::new(cfg, key, "GEMINI_API_KEY") {
            Ok(_) => panic!("expected rejection for model {bad_model:?}"),
            Err(e) => e,
        };
        assert_eq!(err.kind, TransportErrorKind::Config, "bad model {bad_model:?}");
    }

    // Endpoint base_path normalization.
    for (base, expected) in [
        (
            "https://generativelanguage.googleapis.com",
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
        ),
        (
            "https://generativelanguage.googleapis.com/v1beta",
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
        ),
        (
            "https://proxy.example.com/gw",
            "https://proxy.example.com/gw/v1beta/models/gemini-2.5-flash:generateContent",
        ),
    ] {
        let cfg = GeminiConfig { base_url: base.into(), ..Default::default() };
        let key = SecretKey::new("AIza-gemini-test-secret-99".into()).unwrap();
        let t = GeminiTransport::new(cfg, key, "GEMINI_API_KEY").unwrap();
        assert_eq!(t.info().endpoint, expected);
    }

    // Model mismatch between transport config and ModelRequest fails closed.
    let t = GeminiTransport::new(
        GeminiConfig::default(),
        SecretKey::new("AIza-gemini-test-secret-99".into()).unwrap(),
        "GEMINI_API_KEY",
    )
    .unwrap();
    let mismatch_err = t.prepare_request(&minimal_req("gemini-2.5-pro")).unwrap_err();
    assert_eq!(mismatch_err.kind, TransportErrorKind::Config);
}

fn send_once(
    status_line: &'static str,
    body: Vec<u8>,
    secret: &str,
    req: &ModelRequest,
) -> Result<agent::ModelResponse, agent::TransportError> {
    let (base_url, _, handle) = spawn_server(status_line, body);
    let t = GeminiTransport::new(
        GeminiConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "GEMINI_API_KEY",
    )
    .unwrap();
    let prep = t.prepare_request(req).unwrap();
    let res = t.send_prepared(&prep);
    handle.join().unwrap();
    res
}

#[test]
fn gemini_wire_roundtrip_thought_signature_parameters_json_schema_and_header_auth() {
    let secret = "AIza-gemini-test-secret-99";
    let resp_json = json!({
        "responseId": "resp_gem_1",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [
                    { "text": "Reasoning trace", "thought": true, "thoughtSignature": "sig-think-0" },
                    { "text": "Calling tool now." },
                    {
                        "functionCall": { "id": "fc_next", "name": "ledger_note", "args": { "text": "check" } },
                        "thoughtSignature": "sig-fc-next"
                    }
                ]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": { "promptTokenCount": 30, "candidatesTokenCount": 14 }
    });
    let raw_bytes = serde_json::to_vec(&resp_json).unwrap();
    let (base_url, captured, handle) = spawn_server("200 OK", raw_bytes.clone());

    let transport = GeminiTransport::new(
        GeminiConfig { base_url, model: "gemini-2.5-flash".into(), ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "GEMINI_API_KEY",
    )
    .unwrap();

    // Multi-turn history carrying prior thoughtSignature for functionCall + ToolResult.
    let req = ModelRequest {
        model: "gemini-2.5-flash".into(),
        system: Some("System instructions".into()),
        messages: vec![
            Message { role: Role::User, content: vec![ContentBlock::Text { text: "Take a note".into() }] },
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Thinking { thinking: "Prior thought".into(), signature: Some("sig-prior-t".into()) },
                    ContentBlock::RedactedThinking { data: "skip-me".into() },
                    ContentBlock::ToolUse { id: "fc_prev".into(), name: "ledger_note".into(), input: json!({ "text": "prior" }) },
                    ContentBlock::Other {
                        block_type: "gemini_thought_signature".into(),
                        raw: json!({ "tool_use_id": "fc_prev", "thought_signature": "sig-fc-prev" }),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult { tool_use_id: "fc_prev".into(), is_error: false, content: "{\"ok\":true}".into() },
                    ContentBlock::Text { text: "Follow up".into() },
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
        max_tokens: 256,
    };

    let prepared = transport.prepare_request(&req).unwrap();
    let prep_json: Value = serde_json::from_slice(&prepared.body).unwrap();
    assert!(!String::from_utf8_lossy(&prepared.body).contains(secret));

    assert_eq!(prep_json["systemInstruction"], json!({ "parts": [{ "text": "System instructions" }] }));
    assert_eq!(prep_json["generationConfig"], json!({ "maxOutputTokens": 256 }));
    assert_eq!(prep_json["tools"][0]["functionDeclarations"][0]["name"], "ledger_note");
    assert_eq!(
        prep_json["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"]["additionalProperties"],
        false
    );

    let contents = prep_json["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[0]["role"], "user");
    assert_eq!(contents[1]["role"], "model");
    let model_parts = contents[1]["parts"].as_array().unwrap();
    assert_eq!(model_parts.len(), 2);
    assert_eq!(model_parts[0], json!({ "text": "Prior thought", "thought": true, "thoughtSignature": "sig-prior-t" }));
    assert_eq!(
        model_parts[1],
        json!({
            "functionCall": { "id": "fc_prev", "name": "ledger_note", "args": { "text": "prior" } },
            "thoughtSignature": "sig-fc-prev"
        })
    );
    assert_eq!(contents[2]["role"], "user");
    assert_eq!(
        contents[2]["parts"][0],
        json!({
            "functionResponse": { "id": "fc_prev", "name": "ledger_note", "response": { "result": { "ok": true } } }
        })
    );
    assert_eq!(contents[2]["parts"][1], json!({ "text": "Follow up" }));

    let resp = transport.send_prepared(&prepared).unwrap();
    handle.join().unwrap();

    let cap = captured.lock().unwrap().clone().unwrap();
    assert_eq!(cap.method, "POST");
    assert_eq!(cap.target, "/v1beta/models/gemini-2.5-flash:generateContent");
    assert_eq!(cap.body, prepared.body);
    assert!(!cap.target.contains("?key="), "secret must not be in query string");
    let api_key_hdr = cap.headers.iter().find(|(k, _)| k == "x-goog-api-key").map(|(_, v)| v.as_str());
    assert_eq!(api_key_hdr, Some(secret));

    assert_eq!(resp.id.as_deref(), Some("resp_gem_1"));
    assert_eq!(resp.model, "gemini-2.5-flash");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(resp.raw_body, raw_bytes);
    assert_eq!(resp.content.len(), 4);
    assert_eq!(
        resp.content[0],
        ContentBlock::Thinking { thinking: "Reasoning trace".into(), signature: Some("sig-think-0".into()) }
    );
    assert_eq!(resp.content[1], ContentBlock::Text { text: "Calling tool now.".into() });
    assert_eq!(
        resp.content[2],
        ContentBlock::ToolUse { id: "fc_next".into(), name: "ledger_note".into(), input: json!({ "text": "check" }) }
    );
    assert_eq!(
        resp.content[3],
        ContentBlock::Other {
            block_type: "gemini_thought_signature".into(),
            raw: json!({ "tool_use_id": "fc_next", "thought_signature": "sig-fc-next" }),
        }
    );
}

#[test]
fn gemini_refusals_malformed_function_call_http_errors_and_scrubbing() {
    let secret = "AIza-gemini-test-secret-99";
    let req = minimal_req("gemini-2.5-flash");

    // Unknown tool_use_id for ToolResult fails closed in prepare_request.
    let bad_tool_res_req = ModelRequest {
        model: "gemini-2.5-flash".into(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult { tool_use_id: "missing_id".into(), is_error: true, content: "boom".into() }],
        }],
        tools: vec![],
        max_tokens: 64,
    };
    let t_local = GeminiTransport::new(
        GeminiConfig::default(),
        SecretKey::new(secret.into()).unwrap(),
        "GEMINI_API_KEY",
    )
    .unwrap();
    assert_eq!(t_local.prepare_request(&bad_tool_res_req).unwrap_err().kind, TransportErrorKind::Config);

    // promptFeedback.blockReason -> StopReason::Refusal.
    let prompt_blocked = json!({
        "responseId": "r_block",
        "modelVersion": "gemini-2.5-flash",
        "promptFeedback": { "blockReason": "PROHIBITED_CONTENT" }
    });
    let resp = send_once("200 OK", serde_json::to_vec(&prompt_blocked).unwrap(), secret, &req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::Refusal);
    assert_eq!(
        resp.content,
        vec![ContentBlock::Text { text: "blocked by promptFeedback: PROHIBITED_CONTENT".into() }]
    );

    // finishReason: SAFETY -> StopReason::Refusal with zero ToolUse blocks.
    let safety_blocked = json!({
        "responseId": "r_safety",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [
                    { "functionCall": { "name": "ledger_note", "args": {} } },
                    { "text": "Safety triggered." }
                ]
            },
            "finishReason": "SAFETY"
        }]
    });
    let resp = send_once("200 OK", serde_json::to_vec(&safety_blocked).unwrap(), secret, &req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::Refusal);
    assert_eq!(resp.content, vec![ContentBlock::Text { text: "Safety triggered.".into() }]);

    // MALFORMED_FUNCTION_CALL -> StopReason::Other("malformed_function_call").
    let malformed_fc = json!({
        "responseId": "r_mfc",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{ "content": { "role": "model", "parts": [] }, "finishReason": "MALFORMED_FUNCTION_CALL" }]
    });
    let resp = send_once("200 OK", serde_json::to_vec(&malformed_fc).unwrap(), secret, &req).unwrap();
    assert_eq!(resp.stop_reason, StopReason::Other("malformed_function_call".into()));

    // HTTP 401/429/500 + secret scrubbing.
    let err_401 = format!(r#"{{"error":{{"code":401,"message":"API key not valid: {secret}","status":"UNAUTHENTICATED"}}}}"#);
    let err = send_once("401 Unauthorized", err_401.into_bytes(), secret, &req).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::Auth);
    assert_eq!(err.status, Some(401));
    assert!(!err.message.contains(secret));
    let raw = String::from_utf8(err.raw_body.unwrap()).unwrap();
    assert!(!raw.contains(secret));
    assert!(raw.contains("[REDACTED]"));

    for (status_line, expected_kind, expected_code) in [
        ("429 Too Many Requests", TransportErrorKind::RateLimit, 429),
        ("500 Internal Server Error", TransportErrorKind::HttpStatus, 500),
    ] {
        let err = send_once(status_line, br#"{"error":{"message":"backend failure"}}"#.to_vec(), secret, &req).unwrap_err();
        assert_eq!(err.kind, expected_kind);
        assert_eq!(err.status, Some(expected_code));
    }
}

#[test]
fn repeated_tool_use_id_across_turns_resolves_per_turn_function_response_name() {
    let transport = GeminiTransport::new(
        GeminiConfig { base_url: "https://generativelanguage.googleapis.com".into(), ..Default::default() },
        SecretKey::new("AIza-test-key".into()).unwrap(),
        "GEMINI_API_KEY",
    )
    .unwrap();
    let req = ModelRequest {
        model: "gemini-2.5-flash".into(),
        system: None,
        messages: vec![
            Message { role: Role::User, content: vec![ContentBlock::Text { text: "step 1".into() }] },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse { id: "call_0".into(), name: "sympy_eval".into(), input: json!({"code": "1+1"}) }],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult { tool_use_id: "call_0".into(), is_error: false, content: r#"{"value":"2"}"#.into() }],
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse { id: "call_0".into(), name: "ledger_note".into(), input: json!({"text": "done"}) }],
            },
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult { tool_use_id: "call_0".into(), is_error: false, content: r#"{"recorded":true}"#.into() },
                    ContentBlock::Text { text: "summarize".into() },
                ],
            },
        ],
        tools: vec![],
        max_tokens: 256,
    };
    let prepared = transport.prepare_request(&req).unwrap();
    let wire: Value = serde_json::from_slice(&prepared.body).unwrap();
    assert_eq!(wire["contents"][2]["parts"][0]["functionResponse"]["name"], "sympy_eval");
    assert_eq!(wire["contents"][4]["parts"][0]["functionResponse"]["name"], "ledger_note");
}
