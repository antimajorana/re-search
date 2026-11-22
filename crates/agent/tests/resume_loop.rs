//! Integration tests for verified multi-leg session resume, lossless
//! history rehydration, fresh-approval invariants, tail recovery,
//! cross-provider adaptation, and fail-closed rejections.

mod support;

use agent::{
    inspect_session_for_resume, resume_agent_loop, run_agent_loop, ContentBlock, ModelRequest, ModelResponse, NoopSink,
    PreparedRequest, ProviderKind, ResumeConfig, ResumeRejectKind, Role, SessionOutcome, StopReason, Transport,
    TransportError, TransportInfo, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
};
use ledger::{Ledger, GENESIS_HASH};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use support::{
    assert_secret_absent, default_cfg, make_router, make_transport, ok_msg, MockApprover, MockEpoch, MockServer,
    RecordingSink, TempDir,
};
use tools::approval::ApprovalScope;
use tools::{builtin, Router, ToolDef};

struct ScriptedTransport {
    provider: &'static str,
    endpoint: String,
    replies: Mutex<VecDeque<Result<ModelResponse, TransportError>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedTransport {
    fn new(provider: &'static str, replies: Vec<Result<ModelResponse, TransportError>>) -> Self {
        Self {
            provider,
            endpoint: format!("http://127.0.0.1:0/{provider}"),
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for ScriptedTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo { provider: self.provider, endpoint: self.endpoint.clone(), api_key_env: "RE_SEARCH_API_KEY".into() }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.requests.lock().unwrap().push(req.clone());
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        self.replies.lock().unwrap().pop_front().expect("ScriptedTransport ran out of replies")
    }
}

fn anthropic_resp(id: &str, stop_reason: StopReason, content: Vec<ContentBlock>, raw: Value) -> ModelResponse {
    ModelResponse {
        raw_body: serde_json::to_vec(&raw).unwrap(),
        id: Some(id.into()),
        model: "claude-haiku-4-5".into(),
        stop_reason,
        content,
        usage: json!({ "input_tokens": 10, "output_tokens": 5 }),
    }
}

fn resume_cfg(prompt: &str, model: &str, max_turns: u32, contained: bool) -> ResumeConfig {
    ResumeConfig {
        prompt: prompt.into(),
        model_requested: model.into(),
        model: model.into(),
        system_prompt: None,
        max_turns,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: if contained { "appcontainer".into() } else { "none".into() },
        contained,
        reasoning_effort: None,
    }
}

fn defs_of(router: &Router) -> Vec<ToolDef> {
    router.registry().defs().into_iter().cloned().collect()
}

#[test]
fn multi_leg_resume_preserves_thinking_large_tool_blobs_monotonic_turns_and_verifies() {
    let tmp = TempDir::new("resume-multileg");
    let session = "s-multileg-1";
    let approver1 = Arc::new(MockApprover::new(true));
    let runs = Arc::new(AtomicUsize::new(0));
    let router1 = make_router(&tmp.path, session, approver1.clone(), Arc::new(MockEpoch::new(1, true)), runs.clone());
    let ledger = Ledger::open(&tmp.path).unwrap();
    let expected_defs = defs_of(&router1);

    let big_code = format!("1+1 # {}", "a".repeat(4600));
    let server1 = MockServer::start(vec![
        ok_msg("m1", "tool_use", json!([
            { "type": "thinking", "thinking": "deep leg1 thought", "signature": "sig-leg1-xyz" },
            { "type": "tool_use", "id": "tu_sym_1", "name": "sympy_eval", "input": { "code": big_code } }
        ])),
        ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "leg 1 answer" }])),
    ]);
    let t1 = make_transport(server1.base_url.clone(), "sk-resume-leg1");
    let out1 = run_agent_loop(&t1, &router1, &ledger, &default_cfg("Leg 1 prompt", true), &mut NoopSink).unwrap();
    assert_eq!(out1, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
    server1.finish();
    assert_eq!((approver1.asked(), runs.load(Ordering::SeqCst)), (1, 1));

    let evs1 = ledger.read_session(session).unwrap();
    assert_eq!(evs1.iter().find(|e| e.kind == "model.tool_result").unwrap().payload["truncated"], true);
    assert!(evs1.iter().find(|e| e.kind == "tool.result").unwrap().payload["result"]["blob"].is_string());

    // Leg 2: fresh Router + fresh MockApprover (must re-prompt even on same MockEpoch(1, true)).
    let insp2 = inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &expected_defs).unwrap();
    assert_eq!((insp2.leg, insp2.start_turn, insp2.last_provider), (2, 3, ProviderKind::Anthropic));
    assert_eq!(insp2.prior_outcome, "completed");

    let approver2 = Arc::new(MockApprover::new(true));
    let epoch2 = Arc::new(MockEpoch::new(1, true));
    let router2 = make_router(&tmp.path, session, approver2.clone(), epoch2.clone(), runs.clone());
    let server2 = MockServer::start(vec![
        ok_msg("m3", "tool_use", json!([{ "type": "tool_use", "id": "tu_sym_2", "name": "sympy_eval", "input": { "code": "2+2" } }])),
        ok_msg("m4", "end_turn", json!([{ "type": "text", "text": "leg 2 answer" }])),
    ]);
    let t2 = make_transport(server2.base_url.clone(), "sk-resume-leg2");
    let mut sink2 = RecordingSink::new();
    let out2 = resume_agent_loop(&t2, &router2, &ledger, &resume_cfg("Leg 2 prompt", "claude-haiku-4-5", DEFAULT_MAX_TURNS, true), &mut sink2).unwrap();
    assert_eq!(out2, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
    let reqs2 = server2.requests_received();
    server2.finish();

    assert_eq!((approver2.asked(), runs.load(Ordering::SeqCst)), (1, 2));
    assert_eq!((sink2.events[0].1.as_str(), sink2.events[0].0), ("session.resume", evs1.last().unwrap().seq + 1));

    let wire2: Value = serde_json::from_slice(&reqs2[0]).unwrap();
    let msgs2 = wire2["messages"].as_array().unwrap();
    assert_eq!(msgs2[0]["content"][0]["text"], "Leg 1 prompt");
    assert_eq!(msgs2[1]["content"][0]["thinking"], "deep leg1 thought");
    assert_eq!(msgs2[1]["content"][0]["signature"], "sig-leg1-xyz");
    let tr_wire = msgs2[2]["content"][0]["content"].as_str().unwrap();
    assert!(tr_wire.len() > 4500 && tr_wire.contains(&big_code), "unclipped tool_result len={}", tr_wire.len());
    assert_eq!(msgs2[3]["content"][0]["text"], "leg 1 answer");
    assert_eq!(msgs2[4]["content"][0]["text"], "Leg 2 prompt");

    // Leg 3: resume again with ledger_note, verifying leg=3 and start_turn=5.
    epoch2.set_epoch(2);
    let insp3 = inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &expected_defs).unwrap();
    assert_eq!((insp3.leg, insp3.start_turn), (3, 5));
    let server3 = MockServer::start(vec![
        ok_msg("m5", "tool_use", json!([{ "type": "tool_use", "id": "tu_n3", "name": "ledger_note", "input": { "text": "leg3" } }])),
        ok_msg("m6", "end_turn", json!([{ "type": "text", "text": "leg 3 done" }])),
    ]);
    let t3 = make_transport(server3.base_url.clone(), "sk-resume-leg3");
    let out3 = resume_agent_loop(&t3, &router2, &ledger, &resume_cfg("Leg 3 prompt", "claude-haiku-4-5", DEFAULT_MAX_TURNS, true), &mut NoopSink).unwrap();
    assert_eq!(out3, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
    server3.finish();

    let all_evs = ledger.read_session(session).unwrap();
    for (idx, ev) in all_evs.iter().enumerate() {
        assert_eq!(ev.seq, (idx + 1) as u64);
        assert_eq!(ev.prev_hash, if idx == 0 { GENESIS_HASH.to_string() } else { all_evs[idx - 1].event_hash.clone() });
    }
    let req_turns: Vec<u64> = all_evs.iter().filter(|e| e.kind == "model.request").map(|e| e.payload["turn"].as_u64().unwrap()).collect();
    assert_eq!(req_turns, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    assert_secret_absent(&tmp.path, session, &sink2, "sk-resume-leg2");
}

#[test]
fn tail_recovery_refusal_max_turns_tool_use_and_mid_turn_http_error() {
    // (a) Resume after Refusal (refusal tool_use stripped, leg 2 resumes cleanly).
    {
        let tmp = TempDir::new("resume-refusal");
        let session = "s-tail-refusal";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();

        let s1 = MockServer::start(vec![ok_msg("m1", "refusal", json!([
            { "type": "text", "text": "Cannot do that." },
            { "type": "tool_use", "id": "tu_never", "name": "ledger_note", "input": { "text": "nope" } }
        ]))]);
        let t1 = make_transport(s1.base_url.clone(), "sk-r1");
        assert!(matches!(run_agent_loop(&t1, &router, &ledger, &default_cfg("First", false), &mut NoopSink).unwrap(), SessionOutcome::Refusal { turns: 1, .. }));
        s1.finish();

        let insp = inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &defs_of(&router)).unwrap();
        assert_eq!((insp.prior_outcome.as_str(), insp.start_turn), ("refusal", 2));

        let s2 = MockServer::start(vec![ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "Recovered." }]))]);
        let t2 = make_transport(s2.base_url.clone(), "sk-r2");
        let out2 = resume_agent_loop(&t2, &router, &ledger, &resume_cfg("Safe follow-up", "claude-haiku-4-5", 4, false), &mut NoopSink).unwrap();
        assert_eq!(out2, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
        let reqs2 = s2.requests_received();
        s2.finish();

        let wire: Value = serde_json::from_slice(&reqs2[0]).unwrap();
        let msgs = wire["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["content"], json!([{ "type": "text", "text": "Cannot do that." }]));
        assert_eq!(msgs[2]["content"][0]["text"], "Safe follow-up");
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }

    // (b) Resume after max_turns abort on ToolUse (unanswered ToolUse excluded from history).
    {
        let tmp = TempDir::new("resume-maxturns");
        let session = "s-tail-maxturns";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();

        let s1 = MockServer::start(vec![ok_msg("m1", "tool_use", json!([{ "type": "tool_use", "id": "tu_unanswered", "name": "ledger_note", "input": { "text": "skip" } }]))]);
        let t1 = make_transport(s1.base_url.clone(), "sk-m1");
        let mut cfg1 = default_cfg("Trigger tool at limit", false);
        cfg1.max_turns = 1;
        assert!(matches!(run_agent_loop(&t1, &router, &ledger, &cfg1, &mut NoopSink).unwrap(), SessionOutcome::Aborted { turns: 1, .. }));
        s1.finish();

        let s2 = MockServer::start(vec![ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "Clean resume" }]))]);
        let t2 = make_transport(s2.base_url.clone(), "sk-m2");
        assert_eq!(resume_agent_loop(&t2, &router, &ledger, &resume_cfg("Try again", "claude-haiku-4-5", 4, false), &mut NoopSink).unwrap(), SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
        let reqs2 = s2.requests_received();
        s2.finish();

        let serialized = serde_json::to_string(&serde_json::from_slice::<Value>(&reqs2[0]).unwrap()["messages"]).unwrap();
        assert!(!serialized.contains("tu_unanswered"), "dangling tool_use leaked: {serialized}");
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }

    // (c) Turn 1 ToolUse+ToolResult followed by Turn 2 HTTP 500 -> Turn 1 kept, leg 2 prompt appended to trailing User message.
    {
        let tmp = TempDir::new("resume-http500");
        let session = "s-tail-http500";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();

        let s1 = MockServer::start(vec![
            ok_msg("m1", "tool_use", json!([{ "type": "tool_use", "id": "tu_keep", "name": "ledger_note", "input": { "text": "saved" } }])),
            ("500 Internal Server Error", br#"{"type":"error","error":{"message":"transient 500"}}"#.to_vec()),
        ]);
        let t1 = make_transport(s1.base_url.clone(), "sk-h1");
        assert!(matches!(run_agent_loop(&t1, &router, &ledger, &default_cfg("Do note then crash", false), &mut NoopSink).unwrap(), SessionOutcome::Aborted { turns: 2, .. }));
        s1.finish();

        let insp = inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &defs_of(&router)).unwrap();
        assert_eq!((insp.prior_outcome.as_str(), insp.start_turn), ("aborted", 3));

        let s2 = MockServer::start(vec![ok_msg("m3", "end_turn", json!([{ "type": "text", "text": "Continued!" }]))]);
        let t2 = make_transport(s2.base_url.clone(), "sk-h2");
        assert_eq!(resume_agent_loop(&t2, &router, &ledger, &resume_cfg("Continue after 500", "claude-haiku-4-5", 4, false), &mut NoopSink).unwrap(), SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
        let reqs2 = s2.requests_received();
        s2.finish();

        let wire: Value = serde_json::from_slice(&reqs2[0]).unwrap();
        let msgs = wire["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["content"][0]["id"], "tu_keep");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "tu_keep");
        assert_eq!(msgs[2]["content"][1], json!({ "type": "text", "text": "Continue after 500" }));
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }
}

#[test]
fn cross_provider_resume_strips_thinking_and_preserves_same_provider_gemini_signatures() {
    // Part 1: Anthropic (Thinking + RedactedThinking + Text + ToolUse) -> OpenAI Chat
    {
        let tmp = TempDir::new("resume-cross");
        let session = "s-cross-1";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();

        let ant = ScriptedTransport::new("anthropic-messages", vec![
            Ok(anthropic_resp(
                "m1", StopReason::ToolUse,
                vec![
                    ContentBlock::Thinking { thinking: "secret thought".into(), signature: Some("sig-a".into()) },
                    ContentBlock::RedactedThinking { data: "opaque-redacted".into() },
                    ContentBlock::Text { text: "Calling ledger_note".into() },
                    ContentBlock::ToolUse { id: "tu_c1".into(), name: "ledger_note".into(), input: json!({ "text": "cross" }) },
                ],
                json!({
                    "id": "m1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5", "stop_reason": "tool_use",
                    "content": [
                        { "type": "thinking", "thinking": "secret thought", "signature": "sig-a" },
                        { "type": "redacted_thinking", "data": "opaque-redacted" },
                        { "type": "text", "text": "Calling ledger_note" },
                        { "type": "tool_use", "id": "tu_c1", "name": "ledger_note", "input": { "text": "cross" } }
                    ],
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }),
            )),
            Ok(anthropic_resp(
                "m2", StopReason::EndTurn,
                vec![ContentBlock::Text { text: "Anthropic done".into() }],
                json!({
                    "id": "m2", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                    "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Anthropic done" }],
                    "usage": { "input_tokens": 12, "output_tokens": 4 }
                }),
            )),
        ]);
        assert_eq!(run_agent_loop(&ant, &router, &ledger, &default_cfg("Cross leg 1", false), &mut NoopSink).unwrap(), SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });

        let oai = ScriptedTransport::new("openai-chat", vec![Ok(ModelResponse {
            raw_body: serde_json::to_vec(&json!({
                "id": "chatcmpl_1", "model": "gpt-4.1-mini",
                "choices": [{ "index": 0, "finish_reason": "stop", "message": { "role": "assistant", "content": "OpenAI continued" } }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 4 }
            })).unwrap(),
            id: Some("chatcmpl_1".into()), model: "gpt-4.1-mini".into(), stop_reason: StopReason::EndTurn,
            content: vec![ContentBlock::Text { text: "OpenAI continued".into() }], usage: json!({}),
        })]);
        assert_eq!(resume_agent_loop(&oai, &router, &ledger, &resume_cfg("Cross leg 2", "gpt-4.1-mini", 4, false), &mut NoopSink).unwrap(), SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });

        let reqs = oai.requests();
        assert_eq!(reqs[0].messages[1].role, Role::Assistant);
        assert_eq!(reqs[0].messages[1].content, vec![
            ContentBlock::Text { text: "Calling ledger_note".into() },
            ContentBlock::ToolUse { id: "tu_c1".into(), name: "ledger_note".into(), input: json!({ "text": "cross" }) },
        ]);
        assert_eq!(reqs[0].messages[2].role, Role::User);
        assert!(matches!(&reqs[0].messages[2].content[0], ContentBlock::ToolResult { tool_use_id, is_error: false, .. } if tool_use_id == "tu_c1"));
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }

    // Part 2: Same-provider Gemini resume preserves gemini_thought_signature blocks.
    {
        let tmp = TempDir::new("resume-gem");
        let session = "s-gem-1";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();
        let gem_raw1 = serde_json::to_vec(&json!({
            "responseId": "g1", "modelVersion": "gemini-2.5-flash",
            "candidates": [{ "finishReason": "STOP", "content": { "role": "model", "parts": [
                { "thought": true, "text": "gem think", "thoughtSignature": "gsig-t" },
                { "functionCall": { "id": "gc_1", "name": "ledger_note", "args": { "text": "gem" } }, "thoughtSignature": "gsig-fc" }
            ] } }]
        })).unwrap();
        let gem_raw2 = serde_json::to_vec(&json!({
            "responseId": "g2", "modelVersion": "gemini-2.5-flash",
            "candidates": [{ "finishReason": "STOP", "content": { "role": "model", "parts": [{ "text": "gem leg1" }] } }]
        })).unwrap();
        let g1 = ScriptedTransport::new("gemini-generate-content", vec![
            Ok(ModelResponse {
                raw_body: gem_raw1, id: Some("g1".into()), model: "gemini-2.5-flash".into(), stop_reason: StopReason::ToolUse,
                content: vec![
                    ContentBlock::Thinking { thinking: "gem think".into(), signature: Some("gsig-t".into()) },
                    ContentBlock::ToolUse { id: "gc_1".into(), name: "ledger_note".into(), input: json!({ "text": "gem" }) },
                    ContentBlock::Other { block_type: "gemini_thought_signature".into(), raw: json!({ "tool_use_id": "gc_1", "thought_signature": "gsig-fc" }) },
                ],
                usage: json!({}),
            }),
            Ok(ModelResponse {
                raw_body: gem_raw2, id: Some("g2".into()), model: "gemini-2.5-flash".into(), stop_reason: StopReason::EndTurn,
                content: vec![ContentBlock::Text { text: "gem leg1".into() }], usage: json!({}),
            }),
        ]);
        run_agent_loop(&g1, &router, &ledger, &default_cfg("gem1", false), &mut NoopSink).unwrap();

        let gem_raw3 = serde_json::to_vec(&json!({
            "responseId": "g3", "modelVersion": "gemini-2.5-flash",
            "candidates": [{ "finishReason": "STOP", "content": { "role": "model", "parts": [{ "text": "gem leg2" }] } }]
        })).unwrap();
        let g2 = ScriptedTransport::new("gemini-generate-content", vec![Ok(ModelResponse {
            raw_body: gem_raw3, id: Some("g3".into()), model: "gemini-2.5-flash".into(), stop_reason: StopReason::EndTurn,
            content: vec![ContentBlock::Text { text: "gem leg2".into() }], usage: json!({}),
        })]);
        resume_agent_loop(&g2, &router, &ledger, &resume_cfg("gem2", "gemini-2.5-flash", 3, false), &mut NoopSink).unwrap();
        assert!(g2.requests()[0].messages[1].content.iter().any(|b| matches!(b, ContentBlock::Other { block_type, .. } if block_type == "gemini_thought_signature")));
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }
}

#[test]
fn fail_closed_rejections_write_zero_new_ledger_events() {
    // (a) Tampered blob -> TamperOrMissing
    {
        let tmp = TempDir::new("reject-tamper");
        let session = "s-rej-tamper";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();
        let s1 = MockServer::start(vec![ok_msg("m1", "end_turn", json!([{ "type": "text", "text": "ok" }]))]);
        let t1 = make_transport(s1.base_url.clone(), "sk-t");
        run_agent_loop(&t1, &router, &ledger, &default_cfg("hi", false), &mut NoopSink).unwrap();
        s1.finish();

        let evs = ledger.read_session(session).unwrap();
        let before = evs.len();
        let h = evs.iter().find(|e| e.kind == "model.response").unwrap().payload["raw_blob"].as_str().unwrap().to_string();
        std::fs::write(tmp.path.join("blobs").join(&h[..2]).join(&h), b"corrupt").unwrap();

        assert_eq!(inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &defs_of(&router)).unwrap_err().kind, ResumeRejectKind::TamperOrMissing);
        let t2 = ScriptedTransport::new("anthropic-messages", vec![]);
        assert!(resume_agent_loop(&t2, &router, &ledger, &resume_cfg("next", "claude-haiku-4-5", 2, false), &mut NoopSink).is_err());
        assert_eq!(ledger.read_session(session).unwrap().len(), before);
    }

    // (b) Legacy v0 DB -> LegacySchema
    {
        let tmp = TempDir::new("reject-legacy");
        std::fs::write(tmp.path.join("ledger.db"), support::legacy_v0_db_bytes()).unwrap();
        let ledger = Ledger::open_existing(&tmp.path).unwrap();
        assert_eq!(inspect_session_for_resume(&ledger, "s0", builtin::POLICY_VERSION, &builtin::defs()).unwrap_err().kind, ResumeRejectKind::LegacySchema);
    }

    // (c) route == "claude-code-cli" -> UnsupportedRoute
    {
        let tmp = TempDir::new("reject-cli");
        let session = "s-rej-cli";
        let ledger = Ledger::open(&tmp.path).unwrap();
        ledger.append_with_hash(session, "session.start", 1, &json!({ "route": "claude-code-cli" })).unwrap();
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let err = inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &defs_of(&router)).unwrap_err();
        assert_eq!(err.kind, ResumeRejectKind::UnsupportedRoute);
        assert!(err.message.contains("claude-code-cli"));
        assert_eq!(ledger.read_session(session).unwrap().len(), 1);
    }

    // (d) Drift: policy_version mismatch & modified tool_def hash -> Drift, zero events written.
    {
        let tmp = TempDir::new("reject-drift");
        let session = "s-rej-drift";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();
        let s1 = MockServer::start(vec![ok_msg("m1", "end_turn", json!([{ "type": "text", "text": "ok" }]))]);
        let t1 = make_transport(s1.base_url.clone(), "sk-d");
        run_agent_loop(&t1, &router, &ledger, &default_cfg("hi", false), &mut NoopSink).unwrap();
        s1.finish();
        let before = ledger.read_session(session).unwrap().len();
        let defs = defs_of(&router);

        assert_eq!(inspect_session_for_resume(&ledger, session, "v9.9", &defs).unwrap_err().kind, ResumeRejectKind::Drift);
        let mut mod_defs = defs.clone();
        mod_defs[0].description = "tampered description".into();
        assert_eq!(inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &mod_defs).unwrap_err().kind, ResumeRejectKind::Drift);
        let mut mod_scope = defs;
        mod_scope[0].approval_scope = ApprovalScope::WorkerSession;
        assert_eq!(inspect_session_for_resume(&ledger, session, builtin::POLICY_VERSION, &mod_scope).unwrap_err().kind, ResumeRejectKind::Drift);
        assert_eq!(ledger.read_session(session).unwrap().len(), before);
    }

    // (e) Thinking-only assistant turn resumed onto Anthropic emits non-empty placeholder text (never empty "").
    {
        let tmp = TempDir::new("thinking-only-cross");
        let session = "s-think-only";
        let router = make_router(&tmp.path, session, Arc::new(MockApprover::new(true)), Arc::new(MockEpoch::new(1, false)), Arc::new(AtomicUsize::new(0)));
        let ledger = Ledger::open(&tmp.path).unwrap();
        let gem_only_thought = json!({
            "responseId": "g_th",
            "modelVersion": "gemini-2.5-flash",
            "candidates": [{
                "content": { "role": "model", "parts": [{ "text": "internal thought", "thought": true, "thoughtSignature": "sig1" }] },
                "finishReason": "STOP"
            }]
        });
        let tg = ScriptedTransport::new("gemini-generate-content", vec![Ok(ModelResponse {
            raw_body: serde_json::to_vec(&gem_only_thought).unwrap(),
            id: Some("g_th".into()),
            model: "gemini-2.5-flash".into(),
            stop_reason: agent::StopReason::EndTurn,
            content: vec![ContentBlock::Thinking { thinking: "internal thought".into(), signature: Some("sig1".into()) }],
            usage: Value::Null,
        })]);
        let mut gcfg = default_cfg("think", false);
        gcfg.model = "gemini-2.5-flash".into();
        run_agent_loop(&tg, &router, &ledger, &gcfg, &mut NoopSink).unwrap();

        let s2 = MockServer::start(vec![ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "done" }]))]);
        let t2 = make_transport(s2.base_url.clone(), "sk-anth");
        resume_agent_loop(&t2, &router, &ledger, &resume_cfg("continue", "claude-haiku-4-5", 2, false), &mut NoopSink).unwrap();
        let req2: Value = serde_json::from_slice(&s2.requests_received()[0]).unwrap();
        s2.finish();
        let asst_text = req2["messages"][1]["content"][0]["text"].as_str().unwrap();
        assert!(!asst_text.is_empty(), "Anthropic rejects empty text blocks");
    }
}
