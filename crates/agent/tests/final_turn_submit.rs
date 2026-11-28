//! Integration tests for opt-in `final_turn_tools`, last-call harness notice,
//! final-turn allowlist dispatch + resume preservation, and additive
//! `model.request` tool-surface provenance (`tool_names` / `host_tools`).

use agent::{
    resume_agent_loop, resume_autonomous_research_loop, run_autonomous_research_loop, AgentConfig,
    AutonomousConfig, AutonomousRoles, CancelToken, CheckpointPrompt, ContentBlock, EventSink,
    HumanChannel, HumanReply, ModelRequest, ModelResponse, NoopSink, PreparedRequest,
    ProviderKind, ResumeConfig, Role, SessionOutcome, SteerMessage, StopReason, Transport,
    TransportError, TransportInfo, DEFAULT_MAX_TOKENS,
};
use ledger::{AppendedEvent, Ledger};
use policy::EffectClass;
use research_tools::{research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, ApprovalScope, Approver, CallCtx, Registry, Router, ToolDef};

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "test".into() }
    }
}

struct NoopHuman;
impl HumanChannel for NoopHuman {
    fn ask_checkpoint(&self, prompt: &CheckpointPrompt, _cancel: Option<&CancelToken>) -> HumanReply {
        HumanReply {
            checkpoint_id: prompt.checkpoint_id.clone(),
            selected_option: None,
            text: "ok".into(),
            cancelled: false,
        }
    }
    fn drain_steers(&self) -> Vec<SteerMessage> {
        Vec::new()
    }
}

struct TitleWantingSink;
impl EventSink for TitleWantingSink {
    fn on_event(&mut self, _receipt: &AppendedEvent, _kind: &str, _data: &Value) -> Result<(), String> {
        Ok(())
    }
    fn wants_session_title(&self) -> bool {
        true
    }
}

struct QueueTransport {
    provider: &'static str,
    seen: Mutex<Vec<ModelRequest>>,
    replies: Mutex<VecDeque<ModelResponse>>,
}

impl QueueTransport {
    fn new(provider: &'static str, replies: Vec<ModelResponse>) -> Self {
        Self { provider, seen: Mutex::new(Vec::new()), replies: Mutex::new(replies.into()) }
    }

    fn seen_requests(&self) -> Vec<ModelRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl Transport for QueueTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: self.provider,
            endpoint: "http://127.0.0.1/mock".into(),
            api_key_env: "MOCK_KEY".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.seen.lock().unwrap().push(req.clone());
        let body = serde_json::to_vec(req).unwrap();
        Ok(PreparedRequest { body })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        Ok(self.replies.lock().unwrap().pop_front().expect("queued response"))
    }
}

fn anthropic_reply(id: &str, stop_reason: StopReason, content: Vec<ContentBlock>) -> ModelResponse {
    let wire_blocks: Vec<Value> = content
        .iter()
        .map(|b| match b {
            ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
            ContentBlock::ToolUse { id, name, input } => {
                json!({ "type": "tool_use", "id": id, "name": name, "input": input })
            }
            other => panic!("unexpected test block: {other:?}"),
        })
        .collect();
    let raw = json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": stop_reason.as_str(),
        "content": wire_blocks,
        "usage": { "input_tokens": 10, "output_tokens": 5 },
    });
    ModelResponse {
        raw_body: serde_json::to_vec(&raw).unwrap(),
        id: Some(id.into()),
        model: "claude-haiku-4-5".into(),
        stop_reason,
        content,
        usage: raw["usage"].clone(),
    }
}

fn tool_def(id: &str, effect: EffectClass) -> ToolDef {
    ToolDef {
        id: id.into(),
        description: format!("test tool {id}"),
        input_schema: json!({
            "type": "object",
            "properties": { "done": { "type": "boolean" }, "text": { "type": "string" } },
            "additionalProperties": false
        }),
        effect,
        approval_scope: ApprovalScope::Once,
    }
}

fn build_test_router(
    dir: &std::path::Path,
    session: &str,
    submit_runs: Arc<AtomicUsize>,
    probe_runs: Arc<AtomicUsize>,
) -> Router {
    let mut reg = Registry::new();
    let s1 = Arc::clone(&submit_runs);
    reg.register(
        tool_def("record_map_reading", EffectClass::ObserveLocal),
        Arc::new(move |_ctx: &CallCtx<'_>, args: &Value| {
            s1.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "recorded": true, "done": args["done"] }))
        }),
    );
    let s2 = Arc::clone(&submit_runs);
    reg.register(
        tool_def("record_context_cards", EffectClass::ObserveLocal),
        Arc::new(move |_ctx: &CallCtx<'_>, args: &Value| {
            s2.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "cards_recorded": true, "done": args["done"] }))
        }),
    );
    let p = Arc::clone(&probe_runs);
    reg.register(
        tool_def("read_probe", EffectClass::ReadWorkspace),
        Arc::new(move |_ctx: &CallCtx<'_>, args: &Value| {
            p.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "probe": args["text"] }))
        }),
    );
    Router::new(reg, research_policy(), Ledger::open(dir).unwrap(), session, Arc::new(AllowAll))
}

fn agent_cfg(prompt: &str, max_turns: u32) -> AgentConfig {
    AgentConfig {
        prompt: prompt.into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: "Test system prompt.".into(),
        max_turns,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    }
}

#[test]
fn max_turns_1_mixed_tools_dispatches_only_allowlisted_and_resume_keeps_turn() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = ResearchEnv::new(&data);
    let session = "s-final-turn-1";
    let submit_runs = Arc::new(AtomicUsize::new(0));
    let probe_runs = Arc::new(AtomicUsize::new(0));
    let router = build_test_router(&data, session, Arc::clone(&submit_runs), Arc::clone(&probe_runs));
    let ledger = Ledger::open(&data).unwrap();

    let transport = QueueTransport::new(
        ProviderKind::Anthropic.wire_id(),
        vec![anthropic_reply(
            "m-1",
            StopReason::ToolUse,
            vec![
                ContentBlock::ToolUse {
                    id: "tu_submit".into(),
                    name: "record_map_reading".into(),
                    input: json!({ "done": true }),
                },
                ContentBlock::ToolUse {
                    id: "tu_probe".into(),
                    name: "read_probe".into(),
                    input: json!({ "text": "must not run" }),
                },
            ],
        )],
    );
    let auto_cfg = AutonomousConfig {
        final_turn_tools: vec!["record_map_reading".into(), "record_context_cards".into()],
        ..Default::default()
    };
    let out = run_autonomous_research_loop(
        &transport,
        &AutonomousRoles::default(),
        &router,
        &ledger,
        &env,
        &agent_cfg("Map the paper.", 1),
        &auto_cfg,
        None,
        &mut NoopSink,
    )
    .unwrap();
    assert_eq!(
        out,
        SessionOutcome::Aborted {
            turns: 1,
            reason: "reached max_turns (1) before model completed".into(),
        }
    );
    assert_eq!(submit_runs.load(Ordering::SeqCst), 1, "allowlisted submit tool must run once");
    assert_eq!(probe_runs.load(Ordering::SeqCst), 0, "non-allowlisted tool must never run");

    let events = ledger.read_session(session).unwrap();
    let tool_reqs: Vec<_> = events.iter().filter(|e| e.kind == "tool.request").collect();
    assert_eq!(tool_reqs.len(), 1, "zero tool.request for non-submit tool");
    assert_eq!(tool_reqs[0].payload["tool"], json!("record_map_reading"));
    let tool_results: Vec<_> = events.iter().filter(|e| e.kind == "tool.result").collect();
    assert_eq!(tool_results.len(), 1, "zero tool.result for non-submit tool");
    assert_eq!(tool_results[0].payload["call_id"], tool_reqs[0].payload["call_id"]);

    let mtr_submit = events
        .iter()
        .find(|e| e.kind == "model.tool_result" && e.payload["tool_use_id"] == "tu_submit")
        .expect("model.tool_result for submit");
    assert_eq!(mtr_submit.payload["is_error"], json!(false));

    let mtr_probe = events
        .iter()
        .find(|e| e.kind == "model.tool_result" && e.payload["tool_use_id"] == "tu_probe")
        .expect("host model.tool_result for non-submit");
    assert_eq!(mtr_probe.payload["is_error"], json!(true));
    assert!(
        mtr_probe.payload["text"].as_str().unwrap().contains("NOT RUN"),
        "expected NOT RUN marker: {}",
        mtr_probe.payload["text"]
    );
    assert_eq!(events.last().unwrap().kind, "session.abort");

    // Resuming the aborted session must preserve the committed turn-1 assistant & tool_result messages.
    let resume_transport = QueueTransport::new(
        ProviderKind::Anthropic.wire_id(),
        vec![anthropic_reply("m-2", StopReason::EndTurn, vec![ContentBlock::Text { text: "Resumed.".into() }])],
    );
    let rcfg = ResumeConfig {
        prompt: "Continue.".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: None,
        max_turns: 2,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let resumed = resume_agent_loop(&resume_transport, &router, &ledger, &rcfg, &mut NoopSink).unwrap();
    assert_eq!(resumed, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });

    let seen = resume_transport.seen_requests();
    assert_eq!(seen.len(), 1);
    let msgs = &seen[0].messages;
    let has_turn1_assistant = msgs.iter().any(|m| {
        m.role == Role::Assistant
            && m.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == "tu_submit"))
            && m.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == "tu_probe"))
    });
    assert!(has_turn1_assistant, "resume must keep turn-1 assistant tool_use blocks: {msgs:?}");
    let has_turn1_results = msgs.iter().any(|m| {
        m.role == Role::User
            && m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { tool_use_id, is_error: false, .. } if tool_use_id == "tu_submit"))
            && m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { tool_use_id, is_error: true, content } if tool_use_id == "tu_probe" && content.contains("NOT RUN")))
    });
    assert!(has_turn1_results, "resume must keep both turn-1 tool results: {msgs:?}");
    ledger.verify_session(session).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn max_turns_2_emits_final_call_notice_without_human_steer_and_filters_turn_2_tools() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = ResearchEnv::new(&data);
    let session = "s-final-turn-2";
    let submit_runs = Arc::new(AtomicUsize::new(0));
    let probe_runs = Arc::new(AtomicUsize::new(0));
    let router = build_test_router(&data, session, Arc::clone(&submit_runs), Arc::clone(&probe_runs));
    let ledger = Ledger::open(&data).unwrap();

    let transport = QueueTransport::new(
        ProviderKind::Anthropic.wire_id(),
        vec![
            anthropic_reply(
                "m-1",
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: "tu_1".into(),
                    name: "read_probe".into(),
                    input: json!({ "text": "scan" }),
                }],
            ),
            anthropic_reply(
                "m-2",
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: "tu_2".into(),
                    name: "record_map_reading".into(),
                    input: json!({ "done": true }),
                }],
            ),
        ],
    );
    let auto_cfg = AutonomousConfig {
        final_turn_tools: vec!["record_map_reading".into(), "record_context_cards".into()],
        ..Default::default()
    };
    let out = run_autonomous_research_loop(
        &transport,
        &AutonomousRoles::default(),
        &router,
        &ledger,
        &env,
        &agent_cfg("Investigate.", 2),
        &auto_cfg,
        None,
        &mut NoopSink,
    )
    .unwrap();
    assert_eq!(
        out,
        SessionOutcome::Aborted {
            turns: 2,
            reason: "reached max_turns (2) before model completed".into(),
        }
    );
    assert_eq!(probe_runs.load(Ordering::SeqCst), 1);
    assert_eq!(submit_runs.load(Ordering::SeqCst), 1);

    let events = ledger.read_session(session).unwrap();
    assert!(!events.iter().any(|e| e.kind == "human.steer"), "host notice must never emit human.steer");
    let notice = events
        .iter()
        .find(|e| e.kind == "session.notice" && e.payload["kind"] == "final_call_next")
        .expect("session.notice final_call_next emitted");
    assert_eq!(notice.payload["turn"], json!(2));

    let reqs: Vec<_> = events.iter().filter(|e| e.kind == "model.request").collect();
    assert_eq!(reqs.len(), 2);
    assert!(notice.seq < reqs[1].seq, "session.notice must precede turn-2 model.request");
    assert_eq!(
        reqs[1].payload["tool_names"],
        json!(["record_context_cards", "record_map_reading"])
    );

    let seen = transport.seen_requests();
    assert_eq!(seen.len(), 2);
    let turn1_tools: Vec<&str> = seen[0].tools.iter().map(|t| t.name.as_str()).collect();
    assert!(turn1_tools.contains(&"read_probe"), "turn 1 should expose full tool surface: {turn1_tools:?}");
    let turn2_tools: Vec<&str> = seen[1].tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        turn2_tools,
        vec!["record_context_cards", "record_map_reading"],
        "turn 2 tools must be filtered strictly to final_turn_tools"
    );

    let turn2_json = serde_json::to_string(&seen[1]).unwrap();
    assert!(
        turn2_json.contains("[harness notice, not from the user]"),
        "turn-2 request must include honest harness notice: {turn2_json}"
    );
    assert!(
        !turn2_json.contains("<human_steer>"),
        "turn-2 request must never masquerade as <human_steer>: {turn2_json}"
    );
    ledger.verify_session(session).unwrap();
}

#[test]
fn model_request_records_tool_names_and_host_tools_without_changing_tool_defs_or_resume() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = ResearchEnv::new(&data);
    let session = "s-tool-surface";
    let submit_runs = Arc::new(AtomicUsize::new(0));
    let probe_runs = Arc::new(AtomicUsize::new(0));
    let router = build_test_router(&data, session, Arc::clone(&submit_runs), Arc::clone(&probe_runs));
    let ledger = Ledger::open(&data).unwrap();

    let human = NoopHuman;
    let roles = AutonomousRoles { human: Some(&human), ..Default::default() };
    let transport = QueueTransport::new(
        ProviderKind::Anthropic.wire_id(),
        vec![anthropic_reply("m-1", StopReason::EndTurn, vec![ContentBlock::Text { text: "Done.".into() }])],
    );
    let out = run_autonomous_research_loop(
        &transport,
        &roles,
        &router,
        &ledger,
        &env,
        &agent_cfg("Check tools.", 4),
        &AutonomousConfig::default(),
        None,
        &mut TitleWantingSink,
    )
    .unwrap();
    assert_eq!(out, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });

    let events = ledger.read_session(session).unwrap();
    let start = events.iter().find(|e| e.kind == "session.start").unwrap();
    let start_def_ids: Vec<&str> = start.payload["tool_defs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        start_def_ids,
        vec!["read_probe", "record_context_cards", "record_map_reading"],
        "session.start.tool_defs must stay registry-only for resume drift checks"
    );

    let req_ev = events.iter().find(|e| e.kind == "model.request").unwrap();
    let req_tool_names: Vec<&str> = req_ev.payload["tool_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let req_host_tools: Vec<&str> = req_ev.payload["host_tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        req_tool_names,
        vec!["read_probe", "record_context_cards", "record_map_reading", "ask_user", "set_session_title"]
    );
    assert_eq!(req_host_tools, vec!["ask_user", "set_session_title"]);

    // Resuming with the same registry must pass drift checks cleanly.
    let resume_transport = QueueTransport::new(
        ProviderKind::Anthropic.wire_id(),
        vec![anthropic_reply("m-2", StopReason::EndTurn, vec![ContentBlock::Text { text: "Still done.".into() }])],
    );
    let rcfg = ResumeConfig {
        prompt: "Follow up.".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: None,
        max_turns: 2,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let resumed = resume_autonomous_research_loop(
        &resume_transport,
        &roles,
        &router,
        &ledger,
        &env,
        &rcfg,
        &AutonomousConfig::default(),
        None,
        &mut TitleWantingSink,
    )
    .unwrap();
    assert_eq!(resumed, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
    ledger.verify_session(session).unwrap();
}
