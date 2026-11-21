//! `ChatGptTransport` (Responses API, ChatGPT-plan route) against a loopback mock: the
//! documented request shape (`store: false`, `stream: true`, history in `input`, a
//! `namespace` tool, no unsupported fields), a streamed turn with one tool call through
//! the router, success only on `response.completed`, failures, resume from the recorded
//! stream, and that the bearer token reaches only the `Authorization` header.

#[allow(dead_code)]
mod support;

use agent::{
    resume_agent_loop, run_agent_loop, BearerSource, ChatGptConfig, ChatGptTransport, ResumeConfig, SecretKey,
    SessionOutcome, Transport, TransportError, DEFAULT_MAX_TOKENS,
};
use chatgpt_auth::testkit::{MockAuth, Recorded, Reply};
use ledger::Ledger;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
    let incomplete = sse(&[json!({ "type": "response.incomplete", "response": { "id": "r", "status": "incomplete",
        "model": "gpt-test-sol", "incomplete_details": { "reason": "max_output_tokens" }, "output": [text_item("cut")] } })]);
    let (out, _, _, _d) = run_one(incomplete, "chatgpt-wire-incomplete");
    assert!(matches!(&out, SessionOutcome::Aborted { reason, .. } if reason.contains("max_tokens")), "{out:?}");
}

#[test]
fn only_openai_or_loopback_hosts_are_accepted() {
    let src: Arc<dyn BearerSource> = Arc::new(FixedToken(AtomicUsize::new(0)));
    for bad in ["https://proxy.example", "http://api.openai.com", "https://api.openai.com.evil.example"] {
        let cfg = ChatGptConfig { base_url: bad.into(), ..Default::default() };
        assert!(ChatGptTransport::new(cfg, Arc::clone(&src)).is_err(), "{bad}");
    }
    let t = ChatGptTransport::new(ChatGptConfig::default(), src).unwrap();
    assert_eq!(t.info().endpoint, "https://api.openai.com/v1/responses");
}
