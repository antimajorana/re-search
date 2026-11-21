//! `ChatGptTransport` (Responses API, ChatGPT-plan route) against a loopback mock: the
//! documented request shape (`store: false`, `stream: true`, history in `input`, a
//! `namespace` tool, no unsupported fields), a streamed turn with one tool call through
//! the router, success only on `response.completed`, failures, resume from the recorded
//! stream, and that the bearer token reaches only the `Authorization` header.

#[allow(dead_code)]
mod support;

use agent::model_list::{list_models, models_url};
use agent::responses::{parse_responses_stream, serialize_responses_request};
use agent::{
    resume_agent_loop, run_agent_loop, BearerSource, ChatGptConfig, ChatGptTransport, ContentBlock, Message,
    ModelRequest, ProviderKind, ResumeConfig, Role, SecretKey, SessionOutcome, StopReason, ToolSpec, Transport,
    TransportError, TransportErrorKind, DEFAULT_MAX_TOKENS,
};
use chatgpt_auth::testkit::{MockAuth, Recorded, Reply};
use ledger::Ledger;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::*;

const TOKEN: &str = "chatgpt-access-token-SECRET-77";

struct FixedToken(AtomicUsize);

impl BearerSource for FixedToken {
    fn bearer(&self) -> Result<SecretKey, TransportError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        SecretKey::new(TOKEN.to_string())
    }
}

fn sse(events: &[Value]) -> Vec<u8> {
    let mut out = String::new();
    for e in events {
        out.push_str(&format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e));
    }
    out.into_bytes()
}

fn completed(id: &str, output: Value) -> Value {
    json!({ "type": "response.completed", "response": {
        "id": id, "object": "response", "status": "completed", "model": "gpt-test-sol",
        "output": output, "usage": { "input_tokens": 12, "output_tokens": 3 } } })
}

fn call_item() -> Value {
    json!({ "type": "function_call", "id": "fc_1", "call_id": "call_abc", "namespace": "research",
            "name": "sympy_eval", "arguments": "{\"code\":\"1+1\"}", "status": "completed" })
}

fn text_item(t: &str) -> Value {
    json!({ "type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
            "content": [{ "type": "output_text", "text": t, "annotations": [] }] })
}

/// A mock answering `POST /v1/responses` with the queued streams in order.
fn mock(streams: Vec<Vec<u8>>) -> MockAuth {
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(streams)));
    MockAuth::start(Arc::new(move |r: &Recorded| {
        if r.method != "POST" || r.path != "/v1/responses" {
            return Reply::json(404, &json!({}));
        }
        match queue.lock().unwrap().pop_front() {
            Some(body) => Reply { status: 200, content_type: "text/event-stream", body },
            None => Reply::json(500, &json!({ "error": { "code": "exhausted" } })),
        }
    }))
}

fn transport(base: &str, source: Arc<FixedToken>) -> ChatGptTransport {
    ChatGptTransport::new(ChatGptConfig { base_url: base.to_string(), ..Default::default() }, source).unwrap()
}

fn bodies(m: &MockAuth) -> Vec<Value> {
    m.requests().iter().filter(|r| r.path == "/v1/responses").map(|r| serde_json::from_str(&r.body).unwrap()).collect()
}

const FORBIDDEN: &[&str] = &[
    "background", "conversation", "max_output_tokens", "max_tool_calls", "metadata", "moderation", "multi_agent",
    "prompt", "prompt_cache_retention", "safety_identifier", "temperature", "top_logprobs", "top_p", "truncation",
    "user", "previous_response_id",
];

#[test]
fn streamed_turn_with_one_tool_call_then_resume() {
    let dir = TempDir::new("chatgpt-wire-ok");
    let session = "s-chatgpt-wire";
    let m = mock(vec![
        sse(&[
            json!({ "type": "response.created", "response": { "id": "resp_1", "status": "in_progress" } }),
            json!({ "type": "response.output_item.done", "output_index": 0, "item": call_item() }),
            completed("resp_1", json!([call_item()])),
        ]),
        sse(&[
            json!({ "type": "response.output_text.delta", "delta": "The answer is 2." }),
            json!({ "type": "response.output_item.done", "output_index": 0, "item": text_item("The answer is 2.") }),
            completed("resp_2", json!([text_item("The answer is 2.")])),
        ]),
        sse(&[completed("resp_3", json!([text_item("Still 2.")]))]),
    ]);
    let source = Arc::new(FixedToken(AtomicUsize::new(0)));
    let t = transport(&m.base, Arc::clone(&source));
    assert_eq!(t.info().provider, "chatgpt-responses");
    assert_eq!(t.info().endpoint, format!("{}/v1/responses", m.base));
    assert_eq!(t.info().api_key_env, "chatgpt-sign-in");

    let approver = Arc::new(MockApprover::new(true));
    let runs = Arc::new(AtomicUsize::new(0));
    let router = make_router(&dir.path, session, approver.clone(), Arc::new(MockEpoch::new(1, true)), runs.clone());
    let ledger = Ledger::open(&dir.path).unwrap();
    let mut sink = RecordingSink::new();
    let mut cfg = default_cfg("What is 1+1? Use sympy_eval.", true);
    cfg.model = "gpt-test-sol".into();
    let out = run_agent_loop(&t, &router, &ledger, &cfg, &mut sink).unwrap();
    assert!(matches!(out, SessionOutcome::Completed { turns: 2, .. }), "{out:?}");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the tool ran once, through the router");
    assert_eq!(approver.asked(), 1, "and was approved by the approver");
    assert_eq!(source.0.load(Ordering::SeqCst), 2, "a token is fetched for each request");

    let b = bodies(&m);
    for body in &b {
        assert_eq!(body["store"], json!(false));
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["model"], json!("gpt-test-sol"));
        assert!(body["instructions"].as_str().is_some_and(|s| !s.is_empty()));
        for f in FORBIDDEN {
            assert!(body.get(*f).is_none(), "unsupported field `{f}` sent");
        }
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], json!("namespace"));
        assert_eq!(tools[0]["name"], json!("research"));
        assert!(tools[0]["tools"].as_array().unwrap().iter().any(|f| f["type"] == json!("function") && f["name"] == json!("sympy_eval")));
        assert!(!body["input"].as_array().unwrap().iter().any(|i| i["role"] == json!("system")), "no system items");
    }
    // The second request carries the whole history: the prompt, the call and its output.
    let input = b[1]["input"].as_array().unwrap();
    assert_eq!(input[0], json!({ "role": "user", "content": "What is 1+1? Use sympy_eval." }));
    assert_eq!(input[1]["type"], json!("function_call"));
    assert_eq!(input[1]["call_id"], json!("call_abc"));
    assert_eq!(input[1]["namespace"], json!("research"));
    assert_eq!(input[2]["type"], json!("function_call_output"));
    assert_eq!(input[2]["call_id"], json!("call_abc"));
    assert!(input[2]["output"].as_str().unwrap().contains("ok:1+1"));
    for r in m.requests() {
        assert_eq!(r.header("authorization"), Some(format!("Bearer {TOKEN}").as_str()));
        assert!(!r.body.contains(TOKEN) && !r.path.contains(TOKEN) && !r.query.contains(TOKEN));
    }

    // Provenance: the route, endpoint and model, never the token.
    let events = Ledger::open_existing(&dir.path).unwrap().read_session(session).unwrap();
    let start = &events[0].payload;
    assert_eq!(start["route"], json!("api"));
    assert_eq!(start["provider"], json!("chatgpt-responses"));
    assert_eq!(start["api_key_env"], json!("chatgpt-sign-in"));
    assert_eq!(start["model"], json!("gpt-test-sol"));

    // A later turn rehydrates the history from the recorded stream.
    let rcfg = ResumeConfig {
        prompt: "And 2+2?".into(),
        model_requested: "gpt-test-sol".into(),
        model: "gpt-test-sol".into(),
        system_prompt: None,
        max_turns: 3,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "appcontainer".into(),
        contained: true,
    };
    let out = resume_agent_loop(&t, &router, &ledger, &rcfg, &mut sink).unwrap();
    assert!(matches!(out, SessionOutcome::Completed { .. }), "{out:?}");
    let input = bodies(&m)[2]["input"].as_array().unwrap().clone();
    let kinds: Vec<String> = input.iter().map(|i| i["type"].as_str().or(i["role"].as_str()).unwrap().to_string()).collect();
    assert_eq!(kinds, ["user", "function_call", "function_call_output", "assistant", "user"], "{input:?}");
    assert_eq!(input[4]["content"], json!("And 2+2?"));
    drop(router);
    assert_secret_absent(&dir.path, session, &sink, TOKEN);
}

fn run_one(stream: Vec<u8>, tag: &str) -> (SessionOutcome, Vec<ledger::Event>, RecordingSink, TempDir) {
    let dir = TempDir::new(tag);
    let session = "s-chatgpt-fail";
    let m = mock(vec![stream]);
    let t = transport(&m.base, Arc::new(FixedToken(AtomicUsize::new(0))));
    let router = make_router(&dir.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, true)), Arc::new(AtomicUsize::new(0)));
    let ledger = Ledger::open(&dir.path).unwrap();
    let mut sink = RecordingSink::new();
    let mut cfg = default_cfg("hi", true);
    cfg.model = "gpt-test-sol".into();
    let out = run_agent_loop(&t, &router, &ledger, &cfg, &mut sink).unwrap();
    drop(router);
    let events = Ledger::open_existing(&dir.path).unwrap().read_session(session).unwrap();
    (out, events, sink, dir)
}

#[test]
fn a_stream_without_response_completed_is_an_error() {
    let stream = sse(&[
        json!({ "type": "response.created", "response": { "id": "resp_x", "status": "in_progress" } }),
        json!({ "type": "response.output_text.delta", "delta": "partial" }),
        json!({ "type": "response.output_item.done", "output_index": 0, "item": text_item("partial") }),
    ]);
    let (out, events, sink, dir) = run_one(stream, "chatgpt-wire-cut");
    match out {
        SessionOutcome::Aborted { reason, .. } => assert!(reason.contains("without response.completed"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(events.iter().any(|e| e.kind == "model.error"));
    assert!(!events.iter().any(|e| e.kind == "model.text"), "partial text is not a reply");
    assert_secret_absent(&dir.path, "s-chatgpt-fail", &sink, TOKEN);
}

#[test]
fn response_failed_and_incomplete_end_the_turn_with_the_documented_code() {
    let failed = sse(&[json!({ "type": "response.failed", "response": { "id": "r", "status": "failed",
        "error": { "code": "subscription_sharing_usage_limit_exceeded", "message": "limit" } } })]);
    let (out, _, _, _d) = run_one(failed, "chatgpt-wire-failed");
    match out {
        SessionOutcome::Aborted { reason, .. } => {
            assert!(reason.contains("subscription_sharing_usage_limit_exceeded"), "{reason}");
            assert!(reason.contains("rate_limit"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    // Even with truncated function_call JSON, response.incomplete returns Err carrying max_output_tokens.
    let truncated_call = json!({ "type": "function_call", "call_id": "c1", "name": "sympy_eval", "arguments": "{\"code\":" });
    let incomplete = sse(&[json!({ "type": "response.incomplete", "response": { "id": "r", "status": "incomplete",
        "model": "gpt-test-sol", "incomplete_details": { "reason": "max_output_tokens" },
        "output": [text_item("cut"), truncated_call] } })]);
    let parsed_err = parse_responses_stream(incomplete.clone(), TOKEN).unwrap_err();
    assert!(parsed_err.message.contains("max_output_tokens"), "{parsed_err}");
    assert_ne!(parsed_err.kind, TransportErrorKind::MalformedResponse);
    let (out, events, _, _d) = run_one(incomplete, "chatgpt-wire-incomplete");
    assert!(matches!(&out, SessionOutcome::Aborted { reason, .. } if reason.contains("max_output_tokens")), "{out:?}");
    assert!(events.iter().any(|e| e.kind == "session.abort"));
    assert!(!events.iter().any(|e| e.kind == "model.text"), "incomplete stream must not emit model.text");
}

#[test]
fn only_openai_or_loopback_hosts_are_accepted() {
    let src: Arc<dyn BearerSource> = Arc::new(FixedToken(AtomicUsize::new(0)));
    for bad in [
        "https://proxy.example",
        "http://api.openai.com",
        "https://api.openai.com.evil.example",
        "http://localhost:9",
        "http://[::1]:9",
        "https://api.openai.com/evil/prefix",
        "http://127.0.0.1:9/evil",
    ] {
        let cfg = ChatGptConfig { base_url: bad.into(), ..Default::default() };
        let err = ChatGptTransport::new(cfg, Arc::clone(&src)).err().expect(bad);
        assert_eq!(err.kind, TransportErrorKind::Config, "{bad}: {err}");
    }
    for ok_url in [
        "https://api.openai.com",
        "https://api.openai.com/v1",
        "https://api.openai.com/v1/responses",
        "http://127.0.0.1:9",
        "http://127.0.0.1:9/v1",
        "http://127.0.0.1:9/v1/responses",
    ] {
        ChatGptTransport::new(ChatGptConfig { base_url: ok_url.into(), ..Default::default() }, Arc::clone(&src))
            .unwrap_or_else(|e| panic!("{ok_url}: {e}"));
    }
    let t = ChatGptTransport::new(ChatGptConfig::default(), src).unwrap();
    assert_eq!(t.info().endpoint, "https://api.openai.com/v1/responses");
}

#[test]
fn tool_encoding_sets_strict_false_and_normalizes_array_items() {
    let original_schema = json!({
        "type": "object",
        "properties": {
            "quotes": { "type": "array" },
            "withdrawn_arxiv_ids": { "type": "array" },
            "retracted_dois": { "type": "array" },
            "scores": { "type": "array", "items": { "type": "integer" } },
            "nested": { "type": "object", "properties": { "tags": { "type": "array" } } }
        }
    });
    let req = ModelRequest {
        model: "gpt-test-sol".into(),
        system: Some("sys".into()),
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }],
        tools: vec![ToolSpec { name: "tex_audit".into(), description: "audit".into(), input_schema: original_schema.clone() }],
        max_tokens: DEFAULT_MAX_TOKENS,
    };
    let wire: Value = serde_json::from_slice(&serialize_responses_request(&req).unwrap()).unwrap();
    let fns = wire["tools"][0]["tools"].as_array().unwrap();
    assert_eq!(fns.len(), 1);
    assert_eq!(fns[0]["strict"], json!(false));
    let props = &fns[0]["parameters"]["properties"];
    for k in ["quotes", "withdrawn_arxiv_ids", "retracted_dois"] {
        assert_eq!(props[k]["items"], json!({ "type": "string" }), "{k}");
    }
    assert_eq!(props["scores"]["items"], json!({ "type": "integer" }));
    assert_eq!(props["nested"]["properties"]["tags"]["items"], json!({ "type": "string" }));
    // Input ToolSpec schema itself was not mutated.
    assert!(req.tools[0].input_schema["properties"]["quotes"].get("items").is_none());
}

#[test]
fn sse_error_events_bom_keepalive_unterminated_and_status_checks() {
    // Error event with param and documented recovery hint.
    let err_sse = sse(&[json!({
        "type": "error",
        "error": { "code": "subscription_sharing_unsupported_capability", "param": "tools[0].type" }
    })]);
    let err = parse_responses_stream(err_sse, TOKEN).unwrap_err();
    assert!(err.message.contains("subscription_sharing_unsupported_capability"), "{err}");
    assert!(err.message.contains("(param tools[0].type)"), "{err}");
    assert!(err.message.contains("does not support"), "{err}");

    // rate_limit_exceeded maps to RateLimit.
    let rl_sse = sse(&[json!({ "type": "error", "code": "rate_limit_exceeded" })]);
    let err = parse_responses_stream(rl_sse, TOKEN).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::RateLimit);

    // BOM + empty data: keepalive + properly terminated completed (with absent status) succeeds.
    let mut bom_stream = "\u{FEFF}data:\n\ndata:   \n\n".as_bytes().to_vec();
    bom_stream.extend_from_slice(&sse(&[json!({ "type": "response.completed", "response": {
        "id": "r_bom", "model": "gpt-test-sol", "output": [text_item("ok")] } })]));
    let resp = parse_responses_stream(bom_stream, TOKEN).unwrap();
    assert_eq!(resp.id.as_deref(), Some("r_bom"));

    // Unterminated trailing response.completed without blank line fails.
    let unterminated = format!("event: response.completed\ndata: {}\n", completed("r_cut", json!([text_item("ok")])));
    let err = parse_responses_stream(unterminated.into_bytes(), TOKEN).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::MalformedResponse);
    assert!(err.message.contains("without response.completed"), "{err}");

    // response.completed with status != "completed" fails.
    let bad_status = sse(&[json!({ "type": "response.completed", "response": {
        "id": "r_bad", "status": "incomplete", "model": "gpt-test-sol", "output": [text_item("x")] } })]);
    let err = parse_responses_stream(bad_status, TOKEN).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::MalformedResponse);
}

#[test]
fn token_scrubbing_in_sse_and_strict_http_error_sanitization() {
    // Token echoed inside SSE stream is scrubbed from raw_body.
    let echoed = sse(&[completed("r_echo", json!([text_item(&format!("echo {TOKEN} end"))]))]);
    let resp = parse_responses_stream(echoed, TOKEN).unwrap();
    let raw_text = String::from_utf8_lossy(&resp.raw_body);
    assert!(!raw_text.contains(TOKEN), "{raw_text}");
    assert!(raw_text.contains("[REDACTED]"), "{raw_text}");

    // HTTP errors scrub TOKEN from raw_body and never leak raw message/detail/body text into err.message.
    let replies = Arc::new(Mutex::new(std::collections::VecDeque::from(vec![
        Reply::json(400, &json!({ "error": { "code": "invalid_request_error", "param": "input", "message": format!("bad {TOKEN} partial-secret") } })),
        Reply::json(401, &json!({ "detail": format!("detail-leak {TOKEN}") })),
        Reply { status: 500, content_type: "text/plain", body: format!("crash {TOKEN} raw-dump").into_bytes() },
    ])));
    let m = MockAuth::start(Arc::new(move |_| replies.lock().unwrap().pop_front().unwrap()));
    let t = transport(&m.base, Arc::new(FixedToken(AtomicUsize::new(0))));
    let prep = t.prepare_request(&ModelRequest {
        model: "gpt-test-sol".into(),
        system: None,
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }],
        tools: vec![],
        max_tokens: DEFAULT_MAX_TOKENS,
    }).unwrap();

    let e400 = t.send_prepared(&prep).unwrap_err();
    assert_eq!(e400.message, "HTTP 400 invalid_request_error (param input)");
    assert!(!String::from_utf8_lossy(e400.raw_body.as_deref().unwrap()).contains(TOKEN));

    let e401 = t.send_prepared(&prep).unwrap_err();
    assert!(e401.message.contains("sign in again") && !e401.message.contains("detail-leak") && !e401.message.contains(TOKEN), "{e401}");
    assert!(!String::from_utf8_lossy(e401.raw_body.as_deref().unwrap()).contains(TOKEN));

    let e500 = t.send_prepared(&prep).unwrap_err();
    assert_eq!(e500.message, "HTTP 500");
    assert!(!String::from_utf8_lossy(e500.raw_body.as_deref().unwrap()).contains(TOKEN));
}

#[test]
fn reactive_401_refresh_retries_once_and_skips_not_eligible() {
    struct RefreshingToken {
        refreshes: AtomicUsize,
        failed_seen: Mutex<Vec<String>>,
    }
    impl BearerSource for RefreshingToken {
        fn bearer(&self) -> Result<SecretKey, TransportError> {
            SecretKey::new("old-tok-SECRET-1".into())
        }
        fn force_refresh(&self, failed_token: &str) -> Result<SecretKey, TransportError> {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            self.failed_seen.lock().unwrap().push(failed_token.to_string());
            SecretKey::new("new-tok-SECRET-2".into())
        }
    }

    let m = MockAuth::start(Arc::new(|r: &Recorded| {
        if r.header("authorization") == Some("Bearer new-tok-SECRET-2") {
            Reply { status: 200, content_type: "text/event-stream", body: sse(&[completed("r_ok", json!([text_item("refreshed")]))]) }
        } else {
            Reply::json(401, &json!({ "error": { "code": "subscription_sharing_invalid_user" } }))
        }
    }));
    let src = Arc::new(RefreshingToken { refreshes: AtomicUsize::new(0), failed_seen: Mutex::new(Vec::new()) });
    let t = ChatGptTransport::new(ChatGptConfig { base_url: m.base.clone(), ..Default::default() }, src.clone()).unwrap();
    let prep = t.prepare_request(&ModelRequest {
        model: "gpt-test-sol".into(),
        system: None,
        messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }],
        tools: vec![],
        max_tokens: DEFAULT_MAX_TOKENS,
    }).unwrap();
    let resp = t.send_prepared(&prep).unwrap();
    assert_eq!(resp.id.as_deref(), Some("r_ok"));
    assert_eq!(src.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(src.failed_seen.lock().unwrap().as_slice(), ["old-tok-SECRET-1"]);
    assert_eq!(m.requests().len(), 2);

    // Non-recoverable subscription_sharing_user_not_eligible does NOT trigger force_refresh.
    let m2 = MockAuth::start(Arc::new(|_| {
        Reply::json(401, &json!({ "error": { "code": "subscription_sharing_user_not_eligible" } }))
    }));
    let src2 = Arc::new(RefreshingToken { refreshes: AtomicUsize::new(0), failed_seen: Mutex::new(Vec::new()) });
    let t2 = ChatGptTransport::new(ChatGptConfig { base_url: m2.base.clone(), ..Default::default() }, src2.clone()).unwrap();
    let err = t2.send_prepared(&prep).unwrap_err();
    assert!(err.message.contains("subscription_sharing_user_not_eligible"), "{err}");
    assert_eq!(src2.refreshes.load(Ordering::SeqCst), 0);
    assert_eq!(m2.requests().len(), 1);

    // If the retry also returns 401, stop after 1 retry and return scrubbed error.
    let m3 = MockAuth::start(Arc::new(|_| {
        Reply::json(401, &json!({ "error": { "code": "subscription_sharing_invalid_user", "message": "new-tok-SECRET-2" } }))
    }));
    let src3 = Arc::new(RefreshingToken { refreshes: AtomicUsize::new(0), failed_seen: Mutex::new(Vec::new()) });
    let t3 = ChatGptTransport::new(ChatGptConfig { base_url: m3.base.clone(), ..Default::default() }, src3.clone()).unwrap();
    let err = t3.send_prepared(&prep).unwrap_err();
    assert_eq!(src3.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(m3.requests().len(), 2);
    assert!(!err.message.contains("new-tok-SECRET-2"), "{err}");
    assert!(!String::from_utf8_lossy(err.raw_body.as_deref().unwrap()).contains("new-tok-SECRET-2"));
}

#[test]
fn reasoning_is_parsed_not_replayed_and_refusal_sets_stop_reason() {
    let reasoning = json!({ "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "think step" }] });
    let m = mock(vec![
        sse(&[completed("r1", json!([reasoning, call_item()]))]),
        sse(&[completed("r2", json!([text_item("done")]))]),
    ]);
    let dir = TempDir::new("chatgpt-wire-reason");
    let t = transport(&m.base, Arc::new(FixedToken(AtomicUsize::new(0))));
    let router = make_router(&dir.path, "s-reason", Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, true)), Arc::new(AtomicUsize::new(0)));
    let ledger = Ledger::open(&dir.path).unwrap();
    let mut sink = RecordingSink::new();
    let mut cfg = default_cfg("compute", true);
    cfg.model = "gpt-test-sol".into();
    let out = run_agent_loop(&t, &router, &ledger, &cfg, &mut sink).unwrap();
    assert!(matches!(out, SessionOutcome::Completed { turns: 2, .. }), "{out:?}");
    assert!(sink.events.iter().any(|(_, k, _)| k == "model.thinking"));
    let turn2_input = bodies(&m)[1]["input"].as_array().unwrap().clone();
    assert!(!turn2_input.iter().any(|i| i["type"] == json!("reasoning")), "reasoning must not be replayed: {turn2_input:?}");

    // Refusal output part sets StopReason::Refusal.
    let refusal = json!({ "type": "message", "id": "m_ref", "role": "assistant", "status": "completed",
        "content": [{ "type": "refusal", "refusal": "cannot comply" }] });
    let resp = parse_responses_stream(sse(&[completed("r_ref", json!([refusal]))]), TOKEN).unwrap();
    assert_eq!(resp.stop_reason, StopReason::Refusal);
    assert!(resp.content.iter().any(|b| matches!(b, ContentBlock::Text { text } if text == "cannot comply")));
}

#[test]
fn models_url_enforces_chatgpt_host_and_strips_v1_responses() {
    assert_eq!(
        models_url(ProviderKind::ChatGpt, "https://api.openai.com/v1/responses").unwrap(),
        "https://api.openai.com/v1/models"
    );
    assert_eq!(
        models_url(ProviderKind::ChatGpt, "http://127.0.0.1:9/v1/responses").unwrap(),
        "http://127.0.0.1:9/v1/models"
    );
    for bad in ["https://proxy.example", "http://localhost:9", "http://[::1]:9", "https://api.openai.com/evil/prefix"] {
        let err = models_url(ProviderKind::ChatGpt, bad).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "{bad}: {err}");
    }
    let err = list_models(ProviderKind::ChatGpt, "https://api.openai.com", None, Duration::from_millis(200)).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::Config);
    assert_eq!(err.message, "not signed in with ChatGPT");
}
