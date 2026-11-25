//! Consistency audit tests: `read_paper_lines` cross-crate wiring (`Router` `tool_call_ref`,
//! Proof DAG `evidence.rs` & `linked_sources`, `literature_worker_tools` / `read_only` profile),
//! Investigation `session.start` `system_prompt` contradictions, and OpenAI Orchestrator token stats.

mod common;

use agent::{allowed_tools_for_profile, literature_worker_tools};
use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::paper_lines::READ_PAPER_LINES;
use research_tools::{
    all_research_tool_defs, ingest_bytes, register_research_tools, research_policy, IngestRequest,
    ResearchEnv,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router};

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "audit-test".into(),
        }
    }
}

type Handler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct LoopbackApi {
    base_url: String,
}

impl LoopbackApi {
    fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let h = Arc::clone(&handler);
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut reader = BufReader::new(stream);
                    let mut req_line = String::new();
                    if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = req_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .split('?')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let mut len = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut buf = vec![0u8; len];
                    let _ = reader.read_exact(&mut buf);
                    let body: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                    let bytes = serde_json::to_vec(&h(&path, &body)).unwrap();
                    let mut s = reader.into_inner();
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(&bytes);
                    let _ = s.flush();
                });
            }
        });
        Self {
            base_url: format!("http://{addr}"),
        }
    }
}

fn req(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
    let hdr = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

fn wait_idle(srv: &RunningDesktopServer, sid: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (st, sv) = req(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200);
        if sv["running"] == json!(false)
            && sv["events"]
                .as_array()
                .is_some_and(|evs| evs.iter().any(|e| e["kind"] == "session.end" || e["kind"] == "session.abort"))
        {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for turn: {sv}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn read_paper_lines_populates_tool_call_ref_and_resolves_in_proof_evidence_and_linked_sources() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let sid = "s-rpl-evidence";
    let (st, _) = req(&srv, "POST", "/api/sessions", Some(&json!({ "id": sid })));
    assert_eq!(st, 200);

    let env = Arc::new(ResearchEnv::new(&data_dir));
    let db = Mutex::new(Ledger::open(&data_dir).unwrap());
    let paper_bytes = b"Line 1: Assume fixed endpoints.\nLine 2: The boundary variation vanishes.\n";
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "boundary.txt",
            origin: "upload",
            raw_bytes: paper_bytes,
            source_id_hint: Some("src-boundary"),
            section_filter: None,
            title: Some("Boundary Variations"),
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger
        .append(sid, "session.start", 1, &json!({ "prompt": "Check boundary variation" }))
        .unwrap();

    let mut reg = Registry::new();
    register_research_tools(&mut reg, Arc::clone(&env));
    let router = Router::new(
        reg,
        research_policy(),
        ledger,
        sid,
        Arc::new(AllowAll),
    );

    let out = router
        .try_call(
            "agent:anthropic:s-rpl-evidence",
            READ_PAPER_LINES,
            &json!({
                "source_id": src.source_id,
                "start_line": 2,
                "end_line": 2
            }),
        )
        .unwrap();
    let CallOutcome::Ok(read_val) = out else {
        panic!("expected read_paper_lines Ok, got {out:?}");
    };

    let events = Ledger::open(&data_dir).unwrap().read_session(sid).unwrap();
    let call_id = events
        .iter()
        .find(|e| e.kind == "tool.request" && e.payload["tool"] == READ_PAPER_LINES)
        .and_then(|e| e.payload["call_id"].as_str())
        .unwrap()
        .to_string();

    let ledger_out = router
        .try_call(
            "agent:anthropic:s-rpl-evidence",
            "ledger_append",
            &json!({
                "entry_type": "reference_lookup",
                "summary": "Boundary variation vanishes per line 2.",
                "details": "Read directly with read_paper_lines.",
                "tool_call_ref": call_id,
                "depends_on": []
            }),
        )
        .unwrap();
    let CallOutcome::Ok(appended) = ledger_out else {
        panic!("expected ledger_append Ok, got {ledger_out:?}");
    };
    let eid = appended["entry_id"].as_str().unwrap();

    let (ev_st, ev_body) = req(
        &srv,
        "GET",
        &format!("/api/sessions/{sid}/evidence?entry_id={eid}"),
        None,
    );
    assert_eq!(ev_st, 200, "{ev_body}");
    let support = &ev_body["entries"][0]["support"][0];
    let (_, sess_view) = req(&srv, "GET", &format!("/api/sessions/{sid}"), None);

    assert_eq!(
        (
            &read_val["tool_call_ref"],
            &support["kind"],
            &support["status"],
            sess_view["linked_sources"].as_array().unwrap().len()
        ),
        (&json!(call_id), &json!("source_read"), &json!("recorded"), 1),
        "read_paper_lines missing from Router::call tool_call_ref injection ({read_val}), evidence.rs::describe_tool ({support}), and linked_session_sources ({})",
        sess_view["linked_sources"]
    );
}

#[test]
fn literature_worker_and_read_only_subagent_expose_read_paper_lines() {
    let defs = all_research_tool_defs();
    let all_ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    assert!(all_ids.contains(&READ_PAPER_LINES));

    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env);
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        "s-prof",
        Arc::new(AllowAll),
    );

    let lit = literature_worker_tools();
    let read_only = allowed_tools_for_profile("read_only", &router);

    assert!(
        lit.iter().any(|t| t == READ_PAPER_LINES),
        "literature_worker_tools has fs_read & search_paper_passages but omits read_paper_lines: {lit:?}"
    );
    assert!(
        read_only.iter().any(|t| t == READ_PAPER_LINES),
        "read_only subagent profile has fs_read & search_paper_passages but omits read_paper_lines: {read_only:?}"
    );
}

#[test]
fn investigation_session_start_system_prompt_does_not_contradict_itself_or_math_renderer() {
    let mock = LoopbackApi::start(Box::new(|_, _| {
        json!({
            "id": "msg_q1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-6",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "The boundary term vanishes [L2]." }],
            "usage": { "input_tokens": 20, "output_tokens": 10 }
        })
    }));
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    req(
        &srv,
        "PUT",
        "/api/settings/providers",
        Some(&json!({ "keys": { "anthropic": "test-ant-key" } })),
    );

    let sid = "s-inv-sysprompt";
    req(
        &srv,
        "POST",
        "/api/sessions",
        Some(&json!({ "id": sid, "mode": "investigation" })),
    );
    let env = ResearchEnv::new(&data_dir);
    let db = Mutex::new(Ledger::open(&data_dir).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"1. Assume fixed boundary.\n2. The boundary term vanishes.\n",
            source_id_hint: Some("src-p1"),
            section_filter: None,
            title: Some("Boundary Paper"),
        },
    )
    .unwrap();
    drop(db);

    let (st, _) = req(
        &srv,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "What happens to the boundary term?",
            "investigation_source_id": src.source_id,
            "investigation_scope": "question",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202);
    let sv = wait_idle(&srv, sid);
    let sys = sv["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "session.start")
        .and_then(|e| e["payload"]["system_prompt"].as_str())
        .unwrap();

    assert!(sys.contains("Do not build a Proof DAG"));
    assert!(
        !sys.contains("synthesize the `final_answer`")
            && !sys.contains("`ledger_append` with `entry_type")
            && !sys.contains("single dollar delimiters do not render in the Paper view"),
        "Investigation session.start freezes Research Proof-DAG/final_answer delegation prompt AND stale single-dollar claim:\n{sys}"
    );
}

#[test]
fn openai_orchestrator_turn_records_nonzero_input_and_output_tokens_in_session_stats() {
    let mock = LoopbackApi::start(Box::new(|_, _| {
        json!({
            "id": "chatcmpl_audit",
            "model": "gpt-5.4",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Four." },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 42, "completion_tokens": 17, "total_tokens": 59 }
        })
    }));
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    req(
        &srv,
        "PUT",
        "/api/settings/providers",
        Some(&json!({ "keys": { "openai": "test-oai-key" } })),
    );

    let sid = "s-oai-tokens";
    req(&srv, "POST", "/api/sessions", Some(&json!({ "id": sid })));
    let (st, _) = req(
        &srv,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Compute 2+2",
            "provider": "openai",
            "model": "gpt-5.4",
            "verification_mode": false
        })),
    );
    assert_eq!(st, 202);
    let sv = wait_idle(&srv, sid);

    assert_eq!(
        sv["stats"]["input_tokens"],
        json!(42),
        "session_summary.rs ignores OpenAI prompt_tokens on model.response: {}",
        sv["stats"]
    );
    assert_eq!(
        sv["stats"]["output_tokens"],
        json!(17),
        "session_summary.rs ignores OpenAI completion_tokens on model.response: {}",
        sv["stats"]
    );
}
