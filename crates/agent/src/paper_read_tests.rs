//! WP6 tests for `delegate_read_paper` parallel fan-out, whitelist, wrap-up, trim, and resume.

use super::*;
use crate::resume::{inspect_session_for_resume, rehydrate_history};
use crate::runner::{run_turn_loop, LoopParams, NoopSink};
use crate::transport::{
    ModelResponse, PreparedRequest, ProviderKind, Transport, TransportError, TransportInfo,
};
use ledger::Ledger;
use research_tools::{
    ingest_bytes, register_reader_tools, register_research_tools, research_policy, IngestRequest,
    ResearchEnv,
};
use std::sync::atomic::{AtomicUsize, Ordering as AOrd};
use std::sync::{Arc, Barrier};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router, ToolDef};

const SID: &str = "s-paper-read";

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "test".into() }
    }
}

type SendFn = Box<dyn Fn(&ModelRequest) -> Result<ModelResponse, TransportError> + Send + Sync>;

struct FnTransport {
    send: SendFn,
    seen: Mutex<Vec<ModelRequest>>,
}
impl FnTransport {
    fn new(f: impl Fn(&ModelRequest) -> Result<ModelResponse, TransportError> + Send + Sync + 'static) -> Self {
        Self { send: Box::new(f), seen: Mutex::new(Vec::new()) }
    }
    fn seen(&self) -> Vec<ModelRequest> { self.seen.lock().unwrap().clone() }
}
impl Transport for FnTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo { provider: "anthropic-messages", endpoint: "http://127.0.0.1".into(), api_key_env: "TEST".into() }
    }
    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.seen.lock().unwrap().push(req.clone());
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }
    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let req: ModelRequest = serde_json::from_slice(&prepared.body).unwrap();
        (self.send)(&req)
    }
}

fn resp(stop: StopReason, content: Vec<ContentBlock>) -> ModelResponse {
    let raw = serde_json::to_vec(&json!({
        "id": "msg_r", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": stop.as_str(),
        "content": content.iter().map(|b| match b {
            ContentBlock::Text { text } => json!({"type":"text","text":text}),
            ContentBlock::ToolUse { id, name, input } => json!({"type":"tool_use","id":id,"name":name,"input":input}),
            _ => json!({"type":"text","text":""}),
        }).collect::<Vec<_>>(),
        "usage": { "input_tokens": 11, "output_tokens": 7 }
    })).unwrap();
    ModelResponse {
        raw_body: raw, id: Some("msg_r".into()), model: "claude-haiku-4-5".into(),
        stop_reason: stop, content, usage: json!({ "input_tokens": 11, "output_tokens": 7 }),
    }
}

struct EnvFixture {
    tmp: TempDir,
    env: Arc<ResearchEnv>,
    source_id: String,
    primary_path: String,
    marker_seq: u64,
    total_lines: usize,
}

fn setup(lines: &[String]) -> EnvFixture {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let text = lines.join("\n") + "\n";
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let src = ingest_bytes(&env.sandbox(), &db, &IngestRequest {
        session_id: SID, file_name: "paper.txt", origin: "upload", raw_bytes: text.as_bytes(),
        source_id_hint: Some("paper1"), section_filter: None, title: Some("Test Paper"),
    }).unwrap();
    let ledger = db.into_inner().unwrap();
    ledger.append(SID, "investigation.started", 1, &json!({})).unwrap();
    ledger.append(SID, "investigation.paper", 1, &json!({"source_id": src.source_id})).unwrap();
    let marker_seq = ledger.append(SID, "investigation.mapping_requested", 1, &json!({"source_id": src.source_id})).unwrap();
    EnvFixture {
        tmp, env, source_id: src.source_id, primary_path: src.primary_path.unwrap(),
        marker_seq, total_lines: lines.len(),
    }
}

fn sample_lines(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("Line {i:03}: Main physical statement number {i} with distinct symbols alpha_{i} and beta_{i}.")).collect()
}

fn build_reader_router(f: &EnvFixture) -> Router {
    let mut reg = Registry::new();
    register_reader_tools(&mut reg, Arc::clone(&f.env));
    Router::new(reg, research_policy(), Ledger::open(f.tmp.path()).unwrap(), SID, Arc::new(AllowAll))
}

fn make_pr<'a>(f: &EnvFixture, t: &'a dyn Transport, rr: &'a Router, slices: Vec<PaperReaderSlice>, max_turns: u32) -> PaperReaderConfig<'a> {
    PaperReaderConfig {
        worker: DelegatedWorkerSpec {
            role: "paper_reader".into(), display_name: "Paper Reader".into(), transport: t,
            model: "claude-haiku-4-5".into(), routing_reason: "test".into(),
            system_prompt: "Read paper.".into(), allowed_tools: paper_reader_tools(),
            max_turns, max_tokens: 2048, reasoning_effort: Some("low".into()),
        },
        reader_router: rr, source_id: f.source_id.clone(), primary_path: f.primary_path.clone(),
        marker_seq: f.marker_seq, total_lines: f.total_lines, page_lines: vec![],
        default_slices: slices, max_parallel: 4,
    }
}

fn two_slices() -> Vec<PaperReaderSlice> {
    vec![
        PaperReaderSlice { start_line: 1, end_line: 10, goal: "Part A".into() },
        PaperReaderSlice { start_line: 11, end_line: 20, goal: "Part B".into() },
    ]
}

fn first_user_text(req: &ModelRequest) -> String {
    match &req.messages[0].content[0] { ContentBlock::Text { text } => text.clone(), _ => String::new() }
}

#[test]
fn readers_run_concurrently_barrier() {
    let f = setup(&sample_lines(20));
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (b2, inf2, pk2) = (Arc::clone(&barrier), Arc::clone(&in_flight), Arc::clone(&peak));
    let t = FnTransport::new(move |_req| {
        let c = inf2.fetch_add(1, AOrd::SeqCst) + 1;
        pk2.fetch_max(c, AOrd::SeqCst);
        b2.wait();
        inf2.fetch_sub(1, AOrd::SeqCst);
        Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "done".into() }]))
    });
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, two_slices(), 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let (is_err, out) = run_paper_readers(&mut audit, &cfg, "c_bar", &json!({"focus":"all"}), None).unwrap();
    assert!(!is_err, "{out}");
    assert_eq!(peak.load(AOrd::SeqCst), 2);
    ledger.verify_session(SID).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn transport_error_is_gap_others_kept() {
    let lines = sample_lines(20);
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let sid_src = f.source_id.clone();
    let q1 = lines[0].clone();
    let t = FnTransport::new(move |req| {
        if first_user_text(req).contains("Part B") {
            return Err(TransportError { kind: TransportErrorKind::HttpStatus, status: Some(429), message: "rate limited".into(), raw_body: None });
        }
        if req.messages.len() == 1 {
            Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
                id: "n1".into(), name: NOTE_EVIDENCE.into(),
                input: json!({"items":[{"source_id":sid_src,"start_line":1,"end_line":1,"quote":q1,"kind":"passage","role":"result","gist":"Result 1"}]}),
            }]))
        } else {
            Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "ok".into() }]))
        }
    });
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, two_slices(), 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let (is_err, out) = run_paper_readers(&mut audit, &cfg, "c_gap", &json!({"focus":"results"}), None).unwrap();
    assert!(!is_err);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["readers"][0]["ok"], true);
    assert_eq!(v["readers"][0]["pointers"], 1);
    assert_eq!(v["readers"][1]["ok"], false);
    assert_eq!(v["readers"][1]["gap_lines"], json!([[11, 20]]));
    assert_eq!(v["evidence"].as_array().unwrap().len(), 1);
    ledger.verify_session(SID).unwrap();
}

#[test]
fn try_call_err_aborts_all_and_fails_turn() {
    let f = setup(&sample_lines(20));
    let broken_dir = f.tmp.path().join("broken_ledger");
    std::fs::create_dir_all(&broken_dir).unwrap();
    let broken_ledger = Ledger::open(&broken_dir).unwrap();
    // Corrupt the db file permissions/path by deleting db after open? Better: open a read-only sqlite file.
    drop(broken_ledger);
    let db_path = broken_dir.join("ledger.db");
    let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&db_path, perms).unwrap();
    let ro_ledger = Ledger::open(&broken_dir).unwrap();
    let mut reg = Registry::new();
    register_reader_tools(&mut reg, Arc::clone(&f.env));
    let bad_rr = Router::new(reg, research_policy(), ro_ledger, SID, Arc::new(AllowAll));

    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let src_id = f.source_id.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_cl = Arc::clone(&calls);
    let t = FnTransport::new(move |_req| {
        calls_cl.fetch_add(1, AOrd::SeqCst);
        Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
            id: "r1".into(), name: "read_paper_lines".into(),
            input: json!({"source_id": src_id, "start_line": 1, "end_line": 5}),
        }]))
    });
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &bad_rr, two_slices(), 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let res = run_paper_readers(&mut audit, &cfg, "c_abort", &json!({"focus":"all"}), None);
    assert!(res.is_err(), "try_call Err must fail the turn: {res:?}");
    // Restore permissions so TempDir cleans up cleanly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    {
        let mut p2 = std::fs::metadata(&db_path).unwrap().permissions();
        p2.set_readonly(false);
        let _ = std::fs::set_permissions(&db_path, p2);
    }
}

#[test]
fn max_turns_keeps_pointers_wrapup_only_note_evidence() {
    let lines = sample_lines(10);
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let (src_id, q1) = (f.source_id.clone(), lines[0].clone());
    let t = FnTransport::new(move |req| {
        if req.messages.len() == 1 {
            Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
                id: "r1".into(), name: "read_paper_lines".into(),
                input: json!({"source_id": src_id, "start_line": 1, "end_line": 5}),
            }]))
        } else {
            Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
                id: "n1".into(), name: NOTE_EVIDENCE.into(),
                input: json!({"items":[{"source_id":src_id,"start_line":1,"end_line":1,"quote":q1,"kind":"passage","role":"result","gist":"Kept on final turn"}]}),
            }]))
        }
    });
    let slices = vec![PaperReaderSlice { start_line: 1, end_line: 5, goal: "Slice".into() }];
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, slices, 2)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let (is_err, out) = run_paper_readers(&mut audit, &cfg, "c_max", &json!({"focus":"main"}), None).unwrap();
    assert!(!is_err, "{out}");
    let seen = t.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].tools.len(), 3);
    assert_eq!(seen[1].tools.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec![NOTE_EVIDENCE]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(v["readers"][0]["pointers"], 1);
    ledger.verify_session(SID).unwrap();
}

#[test]
fn wrapup_dispatch_denies_other_tools() {
    let lines = sample_lines(10);
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let src_id = f.source_id.clone();
    // max_turns = 1 => turn 1 is immediately the wrap-up turn.
    let t = FnTransport::new(move |_req| {
        Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
            id: "bad_last".into(), name: "read_paper_lines".into(),
            input: json!({"source_id": src_id, "start_line": 1, "end_line": 5}),
        }]))
    });
    let slices = vec![PaperReaderSlice { start_line: 1, end_line: 5, goal: "Slice".into() }];
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, slices, 1)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let _ = run_paper_readers(&mut audit, &cfg, "c_wrap", &json!({"focus":"main"}), None).unwrap();
    // Router must never have received `read_paper_lines` on the wrap-up turn!
    let evs = ledger.read_session(SID).unwrap();
    assert!(!evs.iter().any(|e| e.kind == "tool.request" && e.payload["tool"] == "read_paper_lines"));
    assert!(evs.iter().any(|e| e.kind == "delegate.end" && e.payload["call_id"] == "c_wrap"));
}

#[test]
fn cancel_returns_denied_and_writes_delegate_end() {
    let f = setup(&sample_lines(10));
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let t = FnTransport::new(|_| panic!("transport should not be called when already cancelled"));
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, two_slices(), 4)), ..Default::default() };
    let cancel = CancelToken::new();
    cancel.cancel();
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let (is_err, out) = run_paper_readers(&mut audit, &cfg, "c_can", &json!({"focus":"all"}), Some(&cancel)).unwrap();
    assert!(is_err);
    assert_eq!(out, "DENIED: stopped by user");
    let evs = ledger.read_session(SID).unwrap();
    assert!(evs.iter().any(|e| e.kind == "delegate.start" && e.payload["call_id"] == "c_can"));
    let end = evs.iter().find(|e| e.kind == "delegate.end" && e.payload["call_id"] == "c_can").unwrap();
    assert_eq!(String::from_utf8(ledger.get_blob(end.payload["raw_blob"].as_str().unwrap()).unwrap()).unwrap(), out);
    ledger.verify_session(SID).unwrap();
}

#[test]
fn whitelist_exactly_three_and_ledger_append_delegate_record_denied() {
    let f = setup(&sample_lines(10));
    // Even if a router mistakenly had extra tools registered, paper_read.rs hard-whitelists ONLY the 3 tools.
    let mut reg = Registry::new();
    register_research_tools(&mut reg, Arc::clone(&f.env));
    register_reader_tools(&mut reg, Arc::clone(&f.env));
    let rr = Router::new(reg, research_policy(), Ledger::open(f.tmp.path()).unwrap(), SID, Arc::new(AllowAll));
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let t = FnTransport::new(|req| {
        if req.messages.len() == 1 {
            Ok(resp(StopReason::ToolUse, vec![
                ContentBlock::ToolUse { id: "t1".into(), name: "ledger_append".into(), input: json!({"entry_type":"final_answer","summary":"s","details":"d"}) },
                ContentBlock::ToolUse { id: "t2".into(), name: "delegate_compute".into(), input: json!({"task":"x"}) },
                ContentBlock::ToolUse { id: "t3".into(), name: "record_paper_map".into(), input: json!({}) },
                ContentBlock::ToolUse { id: "t4".into(), name: "ask_user".into(), input: json!({}) },
            ]))
        } else {
            Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "done".into() }]))
        }
    });
    let slices = vec![PaperReaderSlice { start_line: 1, end_line: 5, goal: "Slice".into() }];
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, slices, 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let _ = run_paper_readers(&mut audit, &cfg, "c_wl", &json!({"focus":"all"}), None).unwrap();
    let seen = t.seen();
    let mut offered: Vec<&str> = seen[0].tools.iter().map(|s| s.name.as_str()).collect();
    offered.sort();
    assert_eq!(offered, vec!["note_evidence", "read_paper_lines", "search_paper_passages"]);
    for b in &seen[1].messages[2].content {
        assert!(matches!(b, ContentBlock::ToolResult { is_error: true, content, .. } if content.starts_with("DENIED:")), "{b:?}");
    }
    let evs = ledger.read_session(SID).unwrap();
    assert!(!evs.iter().any(|e| e.kind == "tool.request"));
}

#[test]
fn return_le_24kb_deterministic_trim() {
    let lines: Vec<String> = (1..=140).map(|i| format!("Line {i:03}: Detailed scientific claim number {i} discussing topological invariants and boundary modes thoroughly.")).collect();
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let roles = ["scope", "method", "motivation", "result"];
    for (idx, line) in lines.iter().enumerate() {
        let role = roles[idx % roles.len()];
        let gist = format!("Gist {idx:03} for {role}: {}", "x".repeat(180));
        let out = rr.try_call("worker:paper_reader:anthropic:s-paper-read:r0", NOTE_EVIDENCE, &json!({
            "items": [{
                "source_id": f.source_id, "start_line": idx + 1, "end_line": idx + 1,
                "quote": line, "kind": "passage", "role": role, "gist": gist
            }]
        })).unwrap();
        assert!(matches!(out, CallOutcome::Ok(_)));
    }
    let t = FnTransport::new(|_| panic!("list mode must not call transport"));
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, two_slices(), 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let (e1, out1) = run_paper_readers(&mut audit, &cfg, "c_trim1", &json!({"focus":"topological invariants","mode":"list"}), None).unwrap();
    let (e2, out2) = run_paper_readers(&mut audit, &cfg, "c_trim2", &json!({"focus":"topological invariants","mode":"list"}), None).unwrap();
    assert!(!e1 && !e2);
    assert!(out1.len() <= PAPER_READER_RETURN_MAX_BYTES, "len={}", out1.len());
    assert_eq!(out1, out2, "trimming must be deterministic");
    let v: Value = serde_json::from_str(&out1).unwrap();
    assert!(v["omitted"].as_u64().unwrap_or(0) > 0, "expected omitted > 0");
    // Remaining scope/method/motivation items should have had `g` dropped before results were dropped.
    for ev in v["evidence"].as_array().unwrap() {
        if matches!(ev["r"].as_str(), Some("scope" | "method" | "motivation")) {
            assert!(ev.get("g").is_none(), "expected gist stripped for {}: {ev}", ev["r"]);
        }
    }
}

#[test]
fn one_delegate_turn_with_blob_per_reader_turn() {
    let lines = sample_lines(10);
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let src_id = f.source_id.clone();
    let t = FnTransport::new(move |req| {
        if req.messages.len() == 1 {
            Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
                id: "r1".into(), name: "read_paper_lines".into(),
                input: json!({"source_id": src_id, "start_line": 1, "end_line": 3}),
            }]))
        } else {
            Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "ok".into() }]))
        }
    });
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, two_slices(), 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    run_paper_readers(&mut audit, &cfg, "c_turns", &json!({"focus":"all"}), None).unwrap();
    let evs = ledger.read_session(SID).unwrap();
    let dturns: Vec<_> = evs.iter().filter(|e| e.kind == "delegate.turn" && e.payload["call_id"] == "c_turns").collect();
    assert_eq!(dturns.len(), 4, "2 readers * 2 turns = 4 delegate.turn events");
    for dt in dturns {
        assert!(dt.payload["reader"].as_u64().is_some());
        assert!(!ledger.get_blob(dt.payload["raw_blob"].as_str().unwrap()).unwrap().is_empty());
    }
    ledger.verify_session(SID).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn resume_rebuilds_20kb_with_interleaved_reader_tool_results() {
    let lines: Vec<String> = (1..=80).map(|i| format!("Line {i:03}: Important equation and theorem statement number {i} with exact notation gamma_{i}.")).collect();
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let mut main_reg = Registry::new();
    register_research_tools(&mut main_reg, Arc::clone(&f.env));
    let main_router = Router::new(main_reg, research_policy(), Ledger::open(f.tmp.path()).unwrap(), SID, Arc::new(AllowAll));
    let ledger = Ledger::open(f.tmp.path()).unwrap();

    // Emit 51 pointers inside the reader turn so mode="read" returns ~18-22 KB while testing interleaved tool results.
    let (src_id, reader_lines) = (f.source_id.clone(), lines[..51].to_vec());
    let reader_t = FnTransport::new(move |req| {
        if req.messages.len() == 1 {
            let calls: Vec<ContentBlock> = reader_lines
                .chunks(10)
                .enumerate()
                .map(|(chunk_idx, chunk)| {
                    let items: Vec<Value> = chunk
                        .iter()
                        .enumerate()
                        .map(|(offset, line)| {
                            let idx = chunk_idx * 10 + offset;
                            let gist = if idx < 50 {
                                format!("Important result {idx:02}: {}", "y".repeat(170))
                            } else {
                                "Extra interleaved note.".to_string()
                            };
                            json!({"source_id": src_id, "start_line": idx + 1, "end_line": idx + 1, "quote": line, "kind": "passage", "role": "result", "gist": gist})
                        })
                        .collect();
                    ContentBlock::ToolUse {
                        id: format!("rn{chunk_idx}"),
                        name: NOTE_EVIDENCE.into(),
                        input: json!({"items": items}),
                    }
                })
                .collect();
            Ok(resp(StopReason::ToolUse, calls))
        } else {
            Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "done".into() }]))
        }
    });
    let main_t = FnTransport::new(|req| {
        if req.messages.len() == 1 {
            Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
                id: "drp1".into(), name: DELEGATE_READ_PAPER.into(), input: json!({"focus":"notation"}),
            }]))
        } else {
            Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "Mapped.".into() }]))
        }
    });
    let slices = vec![PaperReaderSlice { start_line: 1, end_line: 80, goal: "All".into() }];
    let dcfg = DelegationConfig { paper_reader: Some(make_pr(&f, &reader_t, &rr, slices, 4)), ..Default::default() };
    let defs = main_router.registry().defs();
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    audit.emit("session.start", json!({
        "route": "api", "provider": "anthropic-messages", "model": "claude-sonnet-4-6",
        "prompt": "Read paper", "system_prompt": "Sys", "policy_version": main_router.policy().version,
        "tool_defs": crate::runner::tool_defs_json(&defs),
    })).unwrap();
    run_turn_loop(&main_t, &main_router, &mut audit, LoopParams {
        model: "claude-sonnet-4-6", system_prompt: "Sys", max_turns: 4, max_tokens: 2048,
        reasoning_effort: None, start_turn: 1,
        initial_messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "Read paper".into() }] }],
        mask_keep_recent: None, cancel: None, emit_end_event: true, delegation: Some(&dcfg), human: None, pause_after_explore: false,
    }).unwrap();

    let expected_defs: Vec<ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
    inspect_session_for_resume(&ledger, SID, &main_router.policy().version, &expected_defs).unwrap();
    let evs = ledger.read_session(SID).unwrap();
    let hist = rehydrate_history(&ledger, &evs, ProviderKind::Anthropic).unwrap();
    let tr_text = hist.iter().flat_map(|m| &m.content).find_map(|b| match b {
        ContentBlock::ToolResult { tool_use_id, content, .. } if tool_use_id == "drp1" => Some(content.as_str()),
        _ => None,
    }).unwrap();
    assert!(tr_text.len() > 12_000 && tr_text.len() <= PAPER_READER_RETURN_MAX_BYTES, "len={}", tr_text.len());
    let parsed: Value = serde_json::from_str(tr_text).unwrap();
    assert!(parsed["evidence"].as_array().unwrap().len() >= 40);
    ledger.verify_session(SID).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn list_mode_over_4kb_resumes() {
    let lines: Vec<String> = (1..=30).map(|i| format!("Line {i:03}: Convention statement number {i} establishing units and sign choice sigma_{i}.")).collect();
    let f = setup(&lines);
    let rr = build_reader_router(&f);
    let mut main_reg = Registry::new();
    register_research_tools(&mut main_reg, Arc::clone(&f.env));
    let main_router = Router::new(main_reg, research_policy(), Ledger::open(f.tmp.path()).unwrap(), SID, Arc::new(AllowAll));
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    for (idx, line) in lines.iter().enumerate() {
        rr.try_call("worker:paper_reader:anthropic-messages:s-paper-read:r0", NOTE_EVIDENCE, &json!({
            "items": [{"source_id": f.source_id, "start_line": idx + 1, "end_line": idx + 1, "quote": line, "kind": "passage", "role": "convention", "gist": format!("Convention {idx:02}: {}", "z".repeat(120))}]
        })).unwrap();
    }
    let reader_t = FnTransport::new(|_| panic!("list mode must not call reader model"));
    let main_t = FnTransport::new(|req| {
        if req.messages.len() == 1 {
            Ok(resp(StopReason::ToolUse, vec![ContentBlock::ToolUse {
                id: "drp_list".into(), name: DELEGATE_READ_PAPER.into(), input: json!({"focus":"conventions","mode":"list"}),
            }]))
        } else {
            Ok(resp(StopReason::EndTurn, vec![ContentBlock::Text { text: "Listed.".into() }]))
        }
    });
    let dcfg = DelegationConfig { paper_reader: Some(make_pr(&f, &reader_t, &rr, two_slices(), 4)), ..Default::default() };
    let defs = main_router.registry().defs();
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    audit.emit("session.start", json!({
        "route": "api", "provider": "anthropic-messages", "model": "claude-sonnet-4-6",
        "prompt": "List evidence", "system_prompt": "Sys", "policy_version": main_router.policy().version,
        "tool_defs": crate::runner::tool_defs_json(&defs),
    })).unwrap();
    run_turn_loop(&main_t, &main_router, &mut audit, LoopParams {
        model: "claude-sonnet-4-6", system_prompt: "Sys", max_turns: 4, max_tokens: 2048,
        reasoning_effort: None, start_turn: 1,
        initial_messages: vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "List evidence".into() }] }],
        mask_keep_recent: None, cancel: None, emit_end_event: true, delegation: Some(&dcfg), human: None, pause_after_explore: false,
    }).unwrap();

    let expected_defs: Vec<ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
    inspect_session_for_resume(&ledger, SID, &main_router.policy().version, &expected_defs).unwrap();
    let evs = ledger.read_session(SID).unwrap();
    let hist = rehydrate_history(&ledger, &evs, ProviderKind::Anthropic).unwrap();
    let tr_text = hist.iter().flat_map(|m| &m.content).find_map(|b| match b {
        ContentBlock::ToolResult { tool_use_id, content, .. } if tool_use_id == "drp_list" => Some(content.as_str()),
        _ => None,
    }).unwrap();
    assert!(tr_text.len() > 4096, "expected > 4KB list payload, got {}", tr_text.len());
    let parsed: Value = serde_json::from_str(tr_text).unwrap();
    assert_eq!(parsed["evidence"].as_array().unwrap().len(), 30);
    ledger.verify_session(SID).unwrap();
}

#[test]
fn arg_error_writes_delegate_end() {
    let f = setup(&sample_lines(10));
    let rr = build_reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let t = FnTransport::new(|_| panic!("arg error must not invoke transport"));
    let cfg = DelegationConfig { paper_reader: Some(make_pr(&f, &t, &rr, two_slices(), 4)), ..Default::default() };
    let mut sink = NoopSink;
    let mut audit = Audit { ledger: &ledger, session: SID, sink: &mut sink, last_seq: ledger.read_session(SID).unwrap().last().unwrap().seq };
    let (is_err, out) = run_paper_readers(&mut audit, &cfg, "c_argerr", &json!({"focus": "", "mode": "read"}), None).unwrap();
    assert!(is_err && out.starts_with("ERROR:"));
    let evs = ledger.read_session(SID).unwrap();
    assert!(evs.iter().any(|e| e.kind == "delegate.start" && e.payload["call_id"] == "c_argerr"));
    let end = evs.iter().find(|e| e.kind == "delegate.end" && e.payload["call_id"] == "c_argerr").unwrap();
    assert_eq!(end.payload["ok"], false);
    assert_eq!(String::from_utf8(ledger.get_blob(end.payload["raw_blob"].as_str().unwrap()).unwrap()).unwrap(), out);
    ledger.verify_session(SID).unwrap();
}
