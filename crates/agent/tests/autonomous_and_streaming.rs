//! Integration tests for Stage 10 autonomous research loop
//! (`run_autonomous_research_loop` / `resume_autonomous_research_loop`),
//! live token/reasoning SSE streaming (`ChatGptTransport::send_prepared_streaming`),
//! and cooperative `CancelToken` mid-stream Stop cancellation.

use agent::{
    explore_worker_tools, resume_autonomous_research_loop, run_autonomous_research_loop,
    AgentConfig, AutonomousConfig, AutonomousRoles, BearerSource, CancelToken, ChatGptConfig,
    ChatGptTransport, CheckpointPrompt, ContentBlock, DelegatedWorkerSpec, DelegationConfig,
    EventSink, GuardrailConfig, HumanChannel, HumanReply, ModelRequest, ModelResponse,
    PreparedRequest, ResumeConfig, SecretKey, SessionOutcome, SteerMessage, StopReason, Transport,
    TransportError, TransportInfo, ASK_USER, DEFAULT_MAX_TOKENS, DELEGATE_EXPLORE,
    EXPLORE_BRANCH_MAX_TURNS, RESEARCH_SYSTEM_PROMPT,
};
use ledger::{AppendedEvent, Ledger};
use research_tools::{register_research_tools, research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router};

struct DenyAsk;
impl Approver for DenyAsk {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: false, digest: req.digest.clone(), by: "test".into() }
    }
}

struct MockTransport {
    provider: &'static str,
    requests: Mutex<Vec<ModelRequest>>,
    replies: Mutex<VecDeque<ModelResponse>>,
}

impl MockTransport {
    fn new(provider: &'static str, replies: Vec<ModelResponse>) -> Self {
        Self { provider, requests: Mutex::new(Vec::new()), replies: Mutex::new(VecDeque::from(replies)) }
    }
    fn seen_requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for MockTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo { provider: self.provider, endpoint: "http://127.0.0.1:1".into(), api_key_env: "MOCK".into() }
    }
    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.requests.lock().unwrap().push(req.clone());
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }
    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        Ok(self.replies.lock().unwrap().pop_front().expect("MockTransport exhausted"))
    }
}

fn anthropic_text_reply(id: &str, text: &str) -> ModelResponse {
    let raw = serde_json::to_vec(&json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 10, "output_tokens": 12 }
    }))
    .unwrap();
    ModelResponse {
        raw_body: raw,
        id: Some(id.into()),
        model: "claude-haiku-4-5".into(),
        stop_reason: StopReason::EndTurn,
        content: vec![ContentBlock::Text { text: text.into() }],
        usage: json!({"input_tokens": 10, "output_tokens": 12}),
    }
}

#[derive(Default)]
struct RecordingSink {
    text_deltas: Vec<String>,
    thinking_deltas: Vec<String>,
    events: Vec<String>,
}

impl EventSink for RecordingSink {
    fn on_event(&mut self, _receipt: &AppendedEvent, kind: &str, _data: &Value) -> Result<(), String> {
        self.events.push(kind.to_string());
        Ok(())
    }
    fn on_text_delta(&mut self, delta: &str) {
        self.text_deltas.push(delta.to_string());
    }
    fn on_thinking_delta(&mut self, delta: &str) {
        self.thinking_deltas.push(delta.to_string());
    }
}

#[test]
fn autonomous_loop_rejects_consults_strategist_revises_accepts_and_resumes_losslessly() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data));
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let session = "s-autoloop01";
    let router = Router::new(reg, research_policy(), Ledger::open(&data).unwrap(), session, Arc::new(DenyAsk));
    let ledger = Ledger::open(&data).unwrap();

    // Solver Round 1 gives a sign-flawed draft; Round 2 fixes the sign after Verifier+Strategist feedback.
    let solver = MockTransport::new(
        "anthropic-messages",
        vec![
            anthropic_text_reply("m-draft1", "Draft 1: gap is $-2|\\Delta|$ (sign error)."),
            anthropic_text_reply("m-rev2", "Revised: energy gap is $+2|\\Delta| > 0$ at $\\mu = 0$."),
        ],
    );
    let verifier = MockTransport::new(
        "anthropic-messages",
        vec![
            anthropic_text_reply(
                "v-1",
                r#"{"verdict":"rejected","summary":"Excitation gap must be non-negative","counterexample":"Delta = 1 gives +2, not -2","issues":[{"severity":"error","message":"Sign of excitation energy inverted"}]}"#,
            ),
            anthropic_text_reply(
                "v-2",
                r#"{"verdict":"accepted","summary":"Corrected Bogoliubov spectrum verified","counterexample":null,"issues":[]}"#,
            ),
        ],
    );
    let strategist = MockTransport::new(
        "anthropic-messages",
        vec![anthropic_text_reply(
            "st-1",
            "Take the positive branch $\\omega(k) = +\\sqrt{\\dots}$ of the Bogoliubov quasiparticle dispersion.",
        )],
    );

    let roles = AutonomousRoles {
        verifier: Some((&verifier, "claude-haiku-4-5")),
        strategist: Some((&strategist, "claude-haiku-4-5")),
        delegation: None,
        human: None,
    };
    let cfg = AgentConfig {
        prompt: "Compute the Kitaev chain excitation gap at mu = 0.".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: RESEARCH_SYSTEM_PROMPT.into(),
        max_turns: 12,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let auto_cfg = AutonomousConfig {
        guardrails: GuardrailConfig { verification_mode: true, calculation_mode: true, ..Default::default() },
        use_strategist: true,
        max_verification_rounds: 3,
        ..Default::default()
    };
    let mut sink = RecordingSink::default();
    let out = run_autonomous_research_loop(
        &solver,
        &roles,
        &router,
        &ledger,
        &env,
        &cfg,
        &auto_cfg,
        None,
        &mut sink,
    )
    .unwrap();
    assert_eq!(out, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });

    // Solver Round 2 must have received Verifier issues + counterexample + Strategist advice!
    let solver_reqs = solver.seen_requests();
    assert_eq!(solver_reqs.len(), 2);
    let round2_last_msg = serde_json::to_string(&solver_reqs[1].messages.last().unwrap()).unwrap();
    assert!(round2_last_msg.contains("Sign of excitation energy inverted"), "{round2_last_msg}");
    assert!(round2_last_msg.contains("Delta = 1 gives +2"), "{round2_last_msg}");
    assert!(round2_last_msg.contains("positive branch"), "{round2_last_msg}");

    // Follow-up Leg 2 resume must rehydrate Round 1 + `autonomous.feedback` + Round 2 losslessly!
    let solver_leg2 = MockTransport::new("anthropic-messages", vec![anthropic_text_reply("m-leg2", "Follow-up verified.")]);
    let verifier_leg2 = MockTransport::new(
        "anthropic-messages",
        vec![anthropic_text_reply("v-3", r#"{"verdict":"accepted","summary":"Follow-up sound","issues":[]}"#)],
    );
    let roles_leg2 = AutonomousRoles { verifier: Some((&verifier_leg2, "claude-haiku-4-5")), strategist: None, delegation: None, human: None };
    let resume_cfg = ResumeConfig {
        prompt: "What happens for |mu| > 2t?".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: None,
        max_turns: 8,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let out2 = resume_autonomous_research_loop(
        &solver_leg2,
        &roles_leg2,
        &router,
        &ledger,
        &env,
        &resume_cfg,
        &auto_cfg,
        None,
        &mut sink,
    )
    .unwrap();
    assert_eq!(out2, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });

    // Alternating roles check on rehydrated history: User -> Assistant -> User (feedback) -> Assistant -> User (leg2).
    let leg2_reqs = solver_leg2.seen_requests();
    assert_eq!(leg2_reqs[0].messages.len(), 5);
    ledger.verify_session(session).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn autonomous_loop_emits_warning_instead_of_discarding_when_verifier_rounds_exhaust() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data));
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let session = "s-exhaust01";
    let router = Router::new(reg, research_policy(), Ledger::open(&data).unwrap(), session, Arc::new(DenyAsk));
    let ledger = Ledger::open(&data).unwrap();

    let solver = MockTransport::new(
        "anthropic-messages",
        vec![anthropic_text_reply("m1", "Draft A"), anthropic_text_reply("m2", "Draft B")],
    );
    let verifier = MockTransport::new(
        "anthropic-messages",
        vec![
            anthropic_text_reply("v1", r#"{"verdict":"rejected","summary":"Gap 1","issues":[{"severity":"error","message":"Missing bound"}]}"#),
            anthropic_text_reply("v2", r#"{"verdict":"rejected","summary":"Still missing bound","issues":[{"severity":"error","message":"Unbounded operator"}]}"#),
        ],
    );
    let roles = AutonomousRoles { verifier: Some((&verifier, "claude-haiku-4-5")), strategist: None, delegation: None, human: None };
    let cfg = AgentConfig {
        prompt: "Prove operator bound.".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: RESEARCH_SYSTEM_PROMPT.into(),
        max_turns: 8,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let auto_cfg = AutonomousConfig {
        guardrails: GuardrailConfig { verification_mode: true, ..Default::default() },
        use_strategist: false,
        max_verification_rounds: 2,
        ..Default::default()
    };
    let mut sink = RecordingSink::default();
    let out = run_autonomous_research_loop(&solver, &roles, &router, &ledger, &env, &cfg, &auto_cfg, None, &mut sink).unwrap();
    assert_eq!(out, SessionOutcome::Completed { turns: 2, stop_reason: "completed_unverified".into() });
    assert!(sink.events.contains(&"guardrail.warning".to_string()));
    ledger.verify_session(session).unwrap();
}

struct FixedBearer;
impl BearerSource for FixedBearer {
    fn bearer(&self) -> Result<SecretKey, TransportError> {
        SecretKey::new("tok-live-stream-test-secret".into())
    }
}

fn drain_http_req(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if len > 0 {
        let mut body = vec![0u8; len];
        let _ = reader.read_exact(&mut body);
    }
}

#[test]
fn chatgpt_streaming_delivers_live_deltas_and_cancel_token_aborts_promptly() {
    // 1. Live reasoning + output_text SSE deltas.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        drain_http_req(&mut s);
        let sse = concat!(
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\"}}\n\n",
            "event: response.reasoning_summary_text.delta\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Checking Matsui split condition...\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Theorem: \"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Split holds.\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"gpt-6.2-sol\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"Theorem: Split holds.\"}]}],\"usage\":{\"total_tokens\":42}}}\n\n",
        );
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", sse.len());
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(sse.as_bytes());
    });

    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data));
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let router = Router::new(reg, research_policy(), Ledger::open(&data).unwrap(), "s-stream01", Arc::new(DenyAsk));
    let ledger = Ledger::open(&data).unwrap();
    let transport = ChatGptTransport::new(
        ChatGptConfig { base_url: format!("http://{addr}"), ..Default::default() },
        Arc::new(FixedBearer),
    )
    .unwrap();
    let cfg = AgentConfig {
        prompt: "State theorem.".into(),
        model_requested: "gpt-6.2-sol".into(),
        model: "gpt-6.2-sol".into(),
        system_prompt: RESEARCH_SYSTEM_PROMPT.into(),
        max_turns: 4,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let mut sink = RecordingSink::default();
    let out = run_autonomous_research_loop(
        &transport,
        &AutonomousRoles::default(),
        &router,
        &ledger,
        &env,
        &cfg,
        &AutonomousConfig::default(),
        None,
        &mut sink,
    )
    .unwrap();
    srv.join().unwrap();
    assert_eq!(out, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
    assert_eq!(sink.text_deltas.join(""), "Theorem: Split holds.");
    assert!(sink.thinking_deltas.iter().any(|d| d.contains("Checking Matsui split condition")));

    // 2. Cooperative CancelToken aborts a stalled HTTP stream in well under 2 seconds!
    let slow_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let slow_addr = slow_listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = slow_listener.accept() {
            drain_http_req(&mut s);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
            std::thread::sleep(Duration::from_secs(15));
        }
    });
    let slow_transport = ChatGptTransport::new(
        ChatGptConfig { base_url: format!("http://{slow_addr}"), ..Default::default() },
        Arc::new(FixedBearer),
    )
    .unwrap();
    let mut reg2 = Registry::new();
    register_research_tools(&mut reg2, env.clone());
    let router2 = Router::new(reg2, research_policy(), Ledger::open(&data).unwrap(), "s-cancel01", Arc::new(DenyAsk));
    let cancel = CancelToken::new();
    let cancel_cl = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        cancel_cl.cancel();
    });
    let t0 = Instant::now();
    let mut sink2 = RecordingSink::default();
    let out_cancel = run_autonomous_research_loop(
        &slow_transport,
        &AutonomousRoles::default(),
        &router2,
        &ledger,
        &env,
        &cfg,
        &AutonomousConfig::default(),
        Some(&cancel),
        &mut sink2,
    )
    .unwrap();
    assert!(t0.elapsed() < Duration::from_secs(3), "cancel took {:?}", t0.elapsed());
    assert_eq!(out_cancel, SessionOutcome::Aborted { turns: 1, reason: "stopped by user".into() });
    let evs = ledger.read_session("s-cancel01").unwrap();
    assert_eq!(evs.last().unwrap().kind, "session.abort");
    assert_eq!(evs.last().unwrap().payload["stopped"], true);
}

fn anthropic_tool_reply(id: &str, call_id: &str, name: &str, input: Value) -> ModelResponse {
    let raw = serde_json::to_vec(&json!({
        "id": id, "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "tool_use",
        "content": [{ "type": "tool_use", "id": call_id, "name": name, "input": input }],
        "usage": { "input_tokens": 10, "output_tokens": 12 }
    })).unwrap();
    ModelResponse {
        raw_body: raw, id: Some(id.into()), model: "claude-haiku-4-5".into(),
        stop_reason: StopReason::ToolUse,
        content: vec![ContentBlock::ToolUse { id: call_id.into(), name: name.into(), input }],
        usage: json!({"input_tokens": 10, "output_tokens": 12}),
    }
}

struct InteractiveStub {
    steers: Mutex<Vec<SteerMessage>>,
    prompts: Mutex<Vec<CheckpointPrompt>>,
}
impl HumanChannel for InteractiveStub {
    fn drain_steers(&self) -> Vec<SteerMessage> {
        std::mem::take(&mut *self.steers.lock().unwrap())
    }
    fn ask_checkpoint(&self, prompt: &CheckpointPrompt, _cancel: Option<&CancelToken>) -> HumanReply {
        self.prompts.lock().unwrap().push(prompt.clone());
        if prompt.kind == "plan" {
            self.steers.lock().unwrap().push(SteerMessage {
                steer_id: "st-1".into(), text: "Use dimensionless units t = 1".into(), ts_ms: 99,
            });
            HumanReply { checkpoint_id: prompt.checkpoint_id.clone(), selected_option: Some("Approve plan".into()), text: "Proceed".into(), cancelled: false }
        } else {
            HumanReply { checkpoint_id: prompt.checkpoint_id.clone(), selected_option: prompt.options.first().cloned(), text: "Use branch 1".into(), cancelled: false }
        }
    }
}

#[test]
fn interactive_hitl_plan_steer_explore_pause_and_resume_work_losslessly() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data));
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let session = "s-hitl01";
    let router = Router::new(reg, research_policy(), Ledger::open(&data).unwrap(), session, Arc::new(DenyAsk));
    let ledger = Ledger::open(&data).unwrap();

    let solver = MockTransport::new("anthropic-messages", vec![
        anthropic_tool_reply("m1", "tu-plan", ASK_USER, json!({ "question": "Approve or edit this research plan before execution?", "kind": "plan", "options": ["Approve plan", "Adjust approach"] })),
        anthropic_tool_reply("m2", "tu-exp", DELEGATE_EXPLORE, json!({ "problem": "Find gap", "approaches": ["Majorana", "Jordan-Wigner"] })),
        anthropic_text_reply("m3", "Gap is 2|Delta|."),
    ]);
    let explorer = MockTransport::new("anthropic-messages", vec![
        anthropic_text_reply("e1", "Branch A viable."),
        anthropic_text_reply("e2", "Branch B viable."),
    ]);
    let human = InteractiveStub { steers: Mutex::new(Vec::new()), prompts: Mutex::new(Vec::new()) };
    let delegation = DelegationConfig {
        explorer_worker: Some(DelegatedWorkerSpec {
            role: "explorer_worker".into(), display_name: "Explorer".into(),
            transport: &explorer, model: "claude-haiku-4-5".into(), routing_reason: "test".into(),
            system_prompt: "Explore.".into(), allowed_tools: explore_worker_tools(),
            max_turns: EXPLORE_BRANCH_MAX_TURNS, max_tokens: 2048, reasoning_effort: None,
        }),
        ..Default::default()
    };
    let roles = AutonomousRoles { verifier: None, strategist: None, delegation: Some(delegation), human: Some(&human) };
    let cfg = AgentConfig {
        prompt: "Find Kitaev gap.".into(), model_requested: "claude-haiku-4-5".into(), model: "claude-haiku-4-5".into(),
        system_prompt: RESEARCH_SYSTEM_PROMPT.into(), max_turns: 8, max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(), contained: false, reasoning_effort: None,
    };
    let auto_cfg = AutonomousConfig { interactive_plan_mode: true, pause_after_explore: true, ..Default::default() };
    let mut sink = RecordingSink::default();
    let out = run_autonomous_research_loop(&solver, &roles, &router, &ledger, &env, &cfg, &auto_cfg, None, &mut sink).unwrap();
    assert_eq!(out, SessionOutcome::Completed { turns: 3, stop_reason: "end_turn".into() });

    let reqs = solver.seen_requests();
    assert!(reqs[0].tools.iter().any(|t| t.name == ASK_USER));
    assert!(reqs[0].system.as_deref().unwrap().contains("Interactive Plan Mode"));
    assert!(reqs[0].system.as_deref().unwrap().contains("Pause After Explore"));
    assert!(explorer.seen_requests().iter().all(|r| r.tools.iter().all(|t| t.name != ASK_USER)));
    assert!(serde_json::to_string(&reqs[1].messages.last().unwrap()).unwrap().contains("<human_steer>\\nUse dimensionless units t = 1\\n</human_steer>"));
    assert!(serde_json::to_string(&reqs[2].messages.last().unwrap()).unwrap().contains("human_selection"));
    {
        let prompts = human.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].kind, "plan");
        assert_eq!(prompts[1].kind, "branch");
    }

    let solver2 = MockTransport::new("anthropic-messages", vec![anthropic_text_reply("m4", "Follow-up complete.")]);
    let roles2 = AutonomousRoles { verifier: None, strategist: None, delegation: None, human: Some(&human) };
    let rcfg = ResumeConfig {
        prompt: "Check mu != 0.".into(), model_requested: "claude-haiku-4-5".into(), model: "claude-haiku-4-5".into(),
        system_prompt: None, max_turns: 4, max_tokens: DEFAULT_MAX_TOKENS,
        containment: "test".into(), contained: false, reasoning_effort: None,
    };
    let auto_cfg2 = AutonomousConfig { interactive_plan_mode: false, pause_after_explore: false, ..Default::default() };
    let out2 = resume_autonomous_research_loop(&solver2, &roles2, &router, &ledger, &env, &rcfg, &auto_cfg2, None, &mut sink).unwrap();
    assert_eq!(out2, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
    let r2 = &solver2.seen_requests()[0];
    let sys2 = r2.system.as_deref().unwrap();
    assert_eq!(sys2.matches("Interactive Human-in-the-Loop").count(), 1);
    assert!(!sys2.contains("Interactive Plan Mode"));
    assert!(serde_json::to_string(&r2.messages).unwrap().contains("<human_steer>\\nUse dimensionless units t = 1\\n</human_steer>"));
    ledger.verify_session(session).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}
