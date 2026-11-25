//! Honesty-lens audit regression test for `delegate_read_paper` slice coverage accounting.

use agent::{
    paper_reader_tools, run_autonomous_research_loop, AgentConfig, AutonomousConfig,
    AutonomousRoles, ContentBlock, DelegatedWorkerSpec, DelegationConfig, ModelRequest,
    ModelResponse, NoopSink, PaperReaderConfig, PaperReaderSlice, PreparedRequest, StopReason,
    Transport, TransportError, TransportInfo, DELEGATE_READ_PAPER,
};
use ledger::Ledger;
use research_tools::{
    ingest_bytes, register_reader_tools, register_research_tools, research_policy, IngestRequest,
    ResearchEnv,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router};

const SID: &str = "audit-honesty-reader-cov";

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

struct ScriptTransport {
    send: SendFn,
    seen: Mutex<Vec<ModelRequest>>,
}

impl ScriptTransport {
    fn new(
        f: impl Fn(&ModelRequest) -> Result<ModelResponse, TransportError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            send: Box::new(f),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Transport for ScriptTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "ollama",
            endpoint: "http://127.0.0.1".into(),
            api_key_env: "none".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.seen.lock().unwrap().push(req.clone());
        Ok(PreparedRequest {
            body: serde_json::to_vec(req).unwrap(),
        })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let req: ModelRequest = serde_json::from_slice(&prepared.body).unwrap();
        (self.send)(&req)
    }
}

fn msg_resp(stop: StopReason, content: Vec<ContentBlock>) -> ModelResponse {
    let raw = serde_json::to_vec(&json!({
        "id": "chatcmpl_audit",
        "object": "chat.completion",
        "model": "llama3.2",
        "choices": [{
            "index": 0,
            "finish_reason": if stop == StopReason::ToolUse { "tool_calls" } else { "stop" },
            "message": {
                "role": "assistant",
                "content": content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(Value::String(text.clone())),
                    _ => None,
                }).unwrap_or(Value::Null),
                "tool_calls": content.iter().filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => Some(json!({
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": input.to_string() }
                    })),
                    _ => None,
                }).collect::<Vec<_>>()
            }
        }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
    }))
    .unwrap();
    ModelResponse {
        raw_body: raw,
        id: Some("chatcmpl_audit".into()),
        model: "llama3.2".into(),
        stop_reason: stop,
        content,
        usage: json!({ "input_tokens": 10, "output_tokens": 5 }),
    }
}

/// Finding 6 (major): `run_single_reader` pushes `(start_line, end_line)` from any
/// `read_paper_lines` call into `read_ranges` without checking `a_meta.path == pr.primary_path`
/// or `a_meta.line_truncated != true`. Reading an unrelated source (or a truncated single line)
/// spoofs `lines_read_pct: 100` and `gaps: []` for the unread primary paper slice.
#[test]
fn audit_honesty_reader_coverage_must_not_count_other_source_or_truncated_line() {
    let tmp = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());

    // Primary paper: line 1 is 10,000 chars (exceeding 7,000-char reader budget -> line_truncated=true),
    // lines 2..=10 are normal lines that the reader will read from `other` instead of `primary`!
    let mut primary_lines = vec![format!("Line 1 prefix {} end.", "x".repeat(10_000))];
    for i in 2..=10 {
        primary_lines.push(format!("Primary paper line {i} statement."));
    }
    let primary_text = primary_lines.join("\n") + "\n";
    let other_text: String = (1..=10)
        .map(|i| format!("Other reference source line {i}.\n"))
        .collect();

    let primary = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "primary.txt",
            origin: "upload",
            raw_bytes: primary_text.as_bytes(),
            source_id_hint: Some("primary"),
            section_filter: None,
            title: Some("Primary"),
        },
    )
    .unwrap();
    let other = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "other.txt",
            origin: "upload",
            raw_bytes: other_text.as_bytes(),
            source_id_hint: Some("other"),
            section_filter: None,
            title: Some("Other"),
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
            &json!({"source_id": primary.source_id}),
        )
        .unwrap();
    let marker_seq = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": primary.source_id}),
        )
        .unwrap();

    let mut main_reg = Registry::new();
    register_research_tools(&mut main_reg, Arc::clone(&env));
    let main_router = Router::new(
        main_reg,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        SID,
        Arc::new(AllowAll),
    );

    let mut reader_reg = Registry::new();
    register_reader_tools(&mut reader_reg, Arc::clone(&env));
    let reader_router = Router::new(
        reader_reg,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        SID,
        Arc::new(AllowAll),
    );

    let primary_id = primary.source_id.clone();
    let other_id = other.source_id.clone();
    let reader_transport = ScriptTransport::new(move |req| match req.messages.len() {
        1 => Ok(msg_resp(
            StopReason::ToolUse,
            vec![
                // Truncated single line 1 on primary (`line_truncated: true`).
                ContentBlock::ToolUse {
                    id: "r1".into(),
                    name: "read_paper_lines".into(),
                    input: json!({"source_id": primary_id, "start_line": 1, "end_line": 1}),
                },
                // Lines 2..=10 read from the WRONG source (`other_id`), leaving primary lines 2..=10 unread!
                ContentBlock::ToolUse {
                    id: "r2".into(),
                    name: "read_paper_lines".into(),
                    input: json!({"source_id": other_id, "start_line": 2, "end_line": 10}),
                },
            ],
        )),
        _ => Ok(msg_resp(
            StopReason::EndTurn,
            vec![ContentBlock::Text {
                text: "done".into(),
            }],
        )),
    });

    let solver_transport = ScriptTransport::new(|req| {
        if req.messages.len() == 1 {
            Ok(msg_resp(
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: "del_1".into(),
                    name: DELEGATE_READ_PAPER.into(),
                    input: json!({"focus": "main results"}),
                }],
            ))
        } else {
            Ok(msg_resp(
                StopReason::EndTurn,
                vec![ContentBlock::Text {
                    text: "Finished.".into(),
                }],
            ))
        }
    });

    let pr = PaperReaderConfig {
        worker: DelegatedWorkerSpec {
            role: "paper_reader".into(),
            display_name: "Paper Reader".into(),
            transport: &reader_transport,
            model: "llama3.2".into(),
            routing_reason: "test".into(),
            system_prompt: "Read assigned slice.".into(),
            allowed_tools: paper_reader_tools(),
            max_turns: 4,
            max_tokens: 2048,
            reasoning_effort: Some("low".into()),
        },
        reader_router: &reader_router,
        source_id: primary.source_id.clone(),
        primary_path: primary.primary_path.unwrap(),
        marker_seq,
        total_lines: 10,
        page_lines: vec![],
        default_slices: vec![PaperReaderSlice {
            start_line: 1,
            end_line: 10,
            goal: "Read primary lines 1-10".into(),
        }],
        max_parallel: 1,
    };

    let roles = AutonomousRoles {
        verifier: None,
        strategist: None,
        delegation: Some(DelegationConfig {
            paper_reader: Some(pr),
            ..Default::default()
        }),
        human: None,
    };
    let cfg = AgentConfig {
        prompt: "Map paper.".into(),
        model_requested: "llama3.2".into(),
        model: "llama3.2".into(),
        system_prompt: "Reading assistant.".into(),
        max_turns: 4,
        max_tokens: 2048,
        containment: "test".into(),
        contained: false,
        reasoning_effort: None,
    };
    let mut sink = NoopSink;
    run_autonomous_research_loop(
        &solver_transport,
        &roles,
        &main_router,
        &ledger,
        &env,
        &cfg,
        &AutonomousConfig::default(),
        None,
        &mut sink,
    )
    .unwrap();

    let solver_seen = solver_transport.seen.lock().unwrap();
    let tool_out = solver_seen[1]
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .find_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    let report: Value = serde_json::from_str(&tool_out).unwrap();

    // Primary lines 2..=10 were NEVER read (only `other` was read), and primary line 1 had `line_truncated: true`.
    assert_eq!(
        report["coverage"]["lines_read_pct"], 0,
        "reader coverage falsely counted `other` source and truncated line 1 as 100% read: {report}"
    );
    assert_eq!(
        report["readers"][0]["gap_lines"],
        json!([[1, 10]]),
        "unread primary lines 1..=10 must remain in gap_lines: {report}"
    );
}
