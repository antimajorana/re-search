//! Failing audit proof tests (`audit-flow`) for `delegate_read_paper` in `crates/agent`.

use agent::{
    paper_reader_tools, run_autonomous_research_loop, AgentConfig, AutonomousConfig,
    AutonomousRoles, ContentBlock, DelegatedWorkerSpec, DelegationConfig, ModelRequest,
    ModelResponse, NoopSink, PaperReaderConfig, PaperReaderSlice, PreparedRequest, StopReason,
    Transport, TransportError, TransportErrorKind, TransportInfo, DEFAULT_MAX_TOKENS,
    RESEARCH_SYSTEM_PROMPT,
};
use ledger::Ledger;
use research_tools::{
    ingest_bytes, register_reader_tools, register_research_tools, research_policy, IngestRequest,
    ResearchEnv,
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router};

const SID: &str = "s-audit-flow-reader";

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "test".into(),
        }
    }
}

type SendFn = Box<dyn Fn(&ModelRequest) -> Result<ModelResponse, TransportError> + Send + Sync>;

struct FnTransport {
    send: SendFn,
}

impl FnTransport {
    fn new(
        f: impl Fn(&ModelRequest) -> Result<ModelResponse, TransportError> + Send + Sync + 'static,
    ) -> Self {
        Self { send: Box::new(f) }
    }
}

impl Transport for FnTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "anthropic-messages",
            endpoint: "http://127.0.0.1:9".into(),
            api_key_env: "TEST".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest {
            body: serde_json::to_vec(req).unwrap(),
        })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let req: ModelRequest = serde_json::from_slice(&prepared.body).unwrap();
        (self.send)(&req)
    }
}

fn model_reply(stop: StopReason, content: Vec<ContentBlock>) -> ModelResponse {
    let raw = serde_json::to_vec(&json!({
        "id": "msg_audit",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": stop.as_str(),
        "content": content.iter().map(|b| match b {
            ContentBlock::Text { text } => json!({"type": "text", "text": text}),
            ContentBlock::ToolUse { id, name, input } => json!({"type": "tool_use", "id": id, "name": name, "input": input}),
            _ => json!({"type": "text", "text": ""}),
        }).collect::<Vec<_>>(),
        "usage": { "input_tokens": 10, "output_tokens": 8 }
    }))
    .unwrap();
    ModelResponse {
        raw_body: raw,
        id: Some("msg_audit".into()),
        model: "claude-haiku-4-5".into(),
        stop_reason: stop,
        content,
        usage: json!({ "input_tokens": 10, "output_tokens": 8 }),
    }
}

struct ReaderFixture {
    tmp: TempDir,
    env: Arc<ResearchEnv>,
    source_id: String,
    primary_path: String,
    marker_seq: u64,
    total_lines: usize,
}

fn build_fixture(n_lines: usize) -> ReaderFixture {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let lines: Vec<String> = (1..=n_lines)
        .map(|i| format!("Line {i:03}: Verified statement {i} with coupling g_{i} = 1."))
        .collect();
    let text = lines.join("\n") + "\n";
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: text.as_bytes(),
            source_id_hint: Some("paper-audit"),
            section_filter: None,
            title: Some("Audit Paper"),
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger
        .append(SID, "investigation.started", 1, &json!({}))
        .unwrap();
    ledger
        .append(
            SID,
            "investigation.paper",
            1,
            &json!({"source_id": src.source_id}),
        )
        .unwrap();
    let marker_seq = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": src.source_id}),
        )
        .unwrap();
    ReaderFixture {
        tmp,
        env,
        source_id: src.source_id,
        primary_path: src.primary_path.unwrap(),
        marker_seq,
        total_lines: n_lines,
    }
}

fn make_routers(f: &ReaderFixture) -> (Router, Router) {
    let mut main_reg = Registry::new();
    register_research_tools(&mut main_reg, Arc::clone(&f.env));
    let main_router = Router::new(
        main_reg,
        research_policy(),
        Ledger::open(f.tmp.path()).unwrap(),
        SID,
        Arc::new(AllowAll),
    );
    let mut r_reg = Registry::new();
    register_reader_tools(&mut r_reg, Arc::clone(&f.env));
    let reader_router = Router::new(
        r_reg,
        research_policy(),
        Ledger::open(f.tmp.path()).unwrap(),
        SID,
        Arc::new(AllowAll),
    );
    (main_router, reader_router)
}

#[test]
fn delegate_read_paper_caps_explicit_sections_fan_out() {
    let f = build_fixture(60);
    let (main_router, reader_router) = make_routers(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();

    let reader_calls = Arc::new(AtomicUsize::new(0));
    let rc_cl = Arc::clone(&reader_calls);
    let reader_transport = FnTransport::new(move |_req| {
        rc_cl.fetch_add(1, Ordering::SeqCst);
        Ok(model_reply(
            StopReason::EndTurn,
            vec![ContentBlock::Text {
                text: "done".into(),
            }],
        ))
    });

    let sections: Vec<_> = (0..25)
        .map(|i| json!({"start_line": i * 2 + 1, "end_line": i * 2 + 2, "goal": format!("s-{i}")}))
        .collect();
    let orch_step = AtomicUsize::new(0);
    let orch_transport = FnTransport::new(move |_req| {
        if orch_step.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(model_reply(
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: "call_many".into(),
                    name: "delegate_read_paper".into(),
                    input: json!({"focus": "all", "sections": sections}),
                }],
            ))
        } else {
            Ok(model_reply(
                StopReason::EndTurn,
                vec![ContentBlock::Text {
                    text: "Finished.".into(),
                }],
            ))
        }
    });

    let roles = AutonomousRoles {
        delegation: Some(DelegationConfig {
            paper_reader: Some(PaperReaderConfig {
                worker: DelegatedWorkerSpec {
                    role: "paper_reader".into(),
                    display_name: "Paper Reader".into(),
                    transport: &reader_transport,
                    model: "claude-haiku-4-5".into(),
                    routing_reason: "test".into(),
                    system_prompt: "Read paper.".into(),
                    allowed_tools: paper_reader_tools(),
                    max_turns: 4,
                    max_tokens: 1024,
                    reasoning_effort: Some("low".into()),
                },
                reader_router: &reader_router,
                source_id: f.source_id.clone(),
                primary_path: f.primary_path.clone(),
                marker_seq: f.marker_seq,
                total_lines: f.total_lines,
                page_lines: vec![],
                default_slices: vec![PaperReaderSlice {
                    start_line: 1,
                    end_line: 60,
                    goal: "Full".into(),
                }],
                max_parallel: 2,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = AgentConfig {
        prompt: "Map paper".into(),
        model_requested: "claude-sonnet-4-6".into(),
        model: "claude-sonnet-4-6".into(),
        system_prompt: RESEARCH_SYSTEM_PROMPT.into(),
        max_turns: 4,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(),
        contained: false,
        reasoning_effort: None,
    };
    let mut sink = NoopSink;
    run_autonomous_research_loop(
        &orch_transport,
        &roles,
        &main_router,
        &ledger,
        &f.env,
        &cfg,
        &AutonomousConfig::default(),
        None,
        &mut sink,
    )
    .unwrap();

    let executed = reader_calls.load(Ordering::SeqCst);
    assert!(
        executed <= 8,
        "explicit `sections` array with 25 slices was neither rejected nor capped (ran {executed} reader loops in one tool call)"
    );
}

#[test]
fn second_delegate_read_paper_call_reports_error_when_all_its_slices_fail() {
    let f = build_fixture(20);
    let (main_router, reader_router) = make_routers(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();

    let fail_now = Arc::new(AtomicBool::new(false));
    let fail_cl = Arc::clone(&fail_now);
    let src_id = f.source_id.clone();

    let reader_transport = FnTransport::new(move |req| {
        if fail_cl.load(Ordering::SeqCst) {
            return Err(TransportError {
                kind: TransportErrorKind::HttpStatus,
                status: Some(503),
                message: "upstream reader unavailable".into(),
                raw_body: None,
            });
        }
        if req.messages.len() == 1 {
            Ok(model_reply(
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: "tu_note_1".into(),
                    name: "note_evidence".into(),
                    input: json!({
                        "items": [{
                            "source_id": src_id,
                            "start_line": 1,
                            "end_line": 1,
                            "quote": "Verified statement 1 with coupling g_1 = 1.",
                            "kind": "passage",
                            "role": "result",
                            "gist": "Statement 1"
                        }]
                    }),
                }],
            ))
        } else {
            Ok(model_reply(
                StopReason::EndTurn,
                vec![ContentBlock::Text {
                    text: "Slice 1 complete.".into(),
                }],
            ))
        }
    });

    let fail_trigger = Arc::clone(&fail_now);
    let orch_step = AtomicUsize::new(0);
    let orch_transport = FnTransport::new(move |_req| match orch_step.fetch_add(1, Ordering::SeqCst)
    {
        0 => Ok(model_reply(
            StopReason::ToolUse,
            vec![ContentBlock::ToolUse {
                id: "call_slice_1".into(),
                name: "delegate_read_paper".into(),
                input: json!({"focus": "Section I", "sections": [{"start_line": 1, "end_line": 10, "goal": "Section I"}]}),
            }],
        )),
        1 => {
            fail_trigger.store(true, Ordering::SeqCst);
            Ok(model_reply(
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: "call_slice_2".into(),
                    name: "delegate_read_paper".into(),
                    input: json!({"focus": "Section II", "sections": [{"start_line": 11, "end_line": 20, "goal": "Section II"}]}),
                }],
            ))
        }
        _ => Ok(model_reply(
            StopReason::EndTurn,
            vec![ContentBlock::Text {
                text: "Done.".into(),
            }],
        )),
    });

    let roles = AutonomousRoles {
        delegation: Some(DelegationConfig {
            paper_reader: Some(PaperReaderConfig {
                worker: DelegatedWorkerSpec {
                    role: "paper_reader".into(),
                    display_name: "Paper Reader".into(),
                    transport: &reader_transport,
                    model: "claude-haiku-4-5".into(),
                    routing_reason: "test".into(),
                    system_prompt: "Read paper.".into(),
                    allowed_tools: paper_reader_tools(),
                    max_turns: 4,
                    max_tokens: 1024,
                    reasoning_effort: Some("low".into()),
                },
                reader_router: &reader_router,
                source_id: f.source_id.clone(),
                primary_path: f.primary_path.clone(),
                marker_seq: f.marker_seq,
                total_lines: f.total_lines,
                page_lines: vec![],
                default_slices: vec![PaperReaderSlice {
                    start_line: 1,
                    end_line: 10,
                    goal: "Slice 1".into(),
                }],
                max_parallel: 2,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let cfg = AgentConfig {
        prompt: "Map paper".into(),
        model_requested: "claude-sonnet-4-6".into(),
        model: "claude-sonnet-4-6".into(),
        system_prompt: RESEARCH_SYSTEM_PROMPT.into(),
        max_turns: 5,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(),
        contained: false,
        reasoning_effort: None,
    };
    let mut sink = NoopSink;
    run_autonomous_research_loop(
        &orch_transport,
        &roles,
        &main_router,
        &ledger,
        &f.env,
        &cfg,
        &AutonomousConfig::default(),
        None,
        &mut sink,
    )
    .unwrap();

    let events = ledger.read_session(SID).unwrap();
    let ends: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "delegate.end" && e.payload["worker"] == "paper_reader")
        .collect();
    assert_eq!(ends.len(), 2);
    assert_eq!(ends[0].payload["ok"], true);
    assert_eq!(
        ends[1].payload["ok"], false,
        "second delegate_read_paper call had 0/1 slices succeed, but prior evidence under marker_seq masked total failure as ok=true: {}",
        ends[1].payload
    );
}
