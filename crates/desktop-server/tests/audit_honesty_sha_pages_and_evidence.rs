//! Honesty-lens audit proof tests for:
//! - F16 (`critical`): tampered on-disk source files failing `source.ingested` `files_sha256`
//!   still producing `read: true` Answer-card citations via `fs_read`.
//! - F17 (`critical`): unselected PDF pages (`investigation.paper` `page_start..=page_end`)
//!   still producing `read: true` Answer-card citations in `answer_cites_for_session`.
//! - F20 (`major`): `read_paper_lines` Proof-DAG `tool_call_ref` downgraded to
//!   `"unsupported_reference"` in `evidence.rs` and omitted from `linked_sources`.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::{sha256_hex, Ledger};
use research_tools::{
    ingest_bytes, register_research_tools, research_policy, IngestRequest, ResearchEnv,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router};

type Handler = Box<dyn Fn(&Value) -> Value + Send + Sync>;

struct LoopbackOllamaMock {
    base_url: String,
}

impl LoopbackOllamaMock {
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
                    let reply = serde_json::to_vec(&h(&body)).unwrap();
                    let mut s = reader.into_inner();
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(&reply);
                    let _ = s.flush();
                });
            }
        });
        Self {
            base_url: format!("http://{addr}"),
        }
    }
}

fn oai_text(text: &str) -> Value {
    json!({
        "id": "chat_t",
        "model": "llama3.2",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 8 }
    })
}

fn oai_tools(calls: Vec<(&str, &str, Value)>) -> Value {
    let tc: Vec<Value> = calls
        .into_iter()
        .map(|(id, name, args)| {
            json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": args.to_string() }
            })
        })
        .collect();
    json!({
        "id": "chat_u",
        "model": "llama3.2",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": null, "tool_calls": tc },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 8 }
    })
}

fn call_api(
    server: &RunningDesktopServer,
    method: &str,
    path: &str,
    body: Value,
    expected: u16,
) -> Value {
    let (status, _, bytes) = raw_http(
        server.addr.port(),
        method,
        path,
        &[("X-ReSearch-Token", &server.info.token)],
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(
        status,
        expected,
        "{path}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

fn wait_idle(server: &RunningDesktopServer, sid: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let view = call_api(server, "GET", &format!("/api/sessions/{sid}"), Value::Null, 200);
        if view["running"] != true {
            return view;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("session {sid} stayed running");
}

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

/// Finding F16 (critical): `fs_read` does not check `source.ingested` `files_sha256`, and
/// `collect_received_ranges` in `investigation_cites.rs:132-154` never compares `val["file_sha256"]`
/// against `source.ingested.payload["files_sha256"][primary_path]`. Overwriting a 3-line
/// paper on disk with 50 fabricated lines marks `[L40-45]` as `read: true`.
#[test]
fn audit_honesty_tampered_source_file_must_not_produce_checked_read_true_cites() {
    let temp = tempfile::tempdir().unwrap();
    let sid = "s-tamper-cite";
    let shared_path: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let path_for_mock = Arc::clone(&shared_path);

    let mock = LoopbackOllamaMock::start(Box::new(move |body| {
        let p = path_for_mock.lock().unwrap().clone();
        let has_tool_msg = body["messages"]
            .as_array()
            .is_some_and(|m| m.iter().any(|x| x["role"] == "tool"));
        if !has_tool_msg {
            oai_tools(vec![(
                "c_fs",
                "fs_read",
                json!({"root": "sources", "path": p, "offset": 0, "max_chars": 8000}),
            )])
        } else {
            oai_text("Tampered claim [L40-45].")
        }
    }));

    let mut cfg = test_cfg(temp.path(), None, None, None);
    cfg.api_base_override = Some(format!("{}/v1", mock.base_url));
    let server = start_desktop_server(cfg).unwrap();
    call_api(
        &server,
        "POST",
        "/api/sessions",
        json!({"id": sid, "mode": "investigation"}),
        200,
    );

    let env = ResearchEnv::new(temp.path().to_path_buf());
    let db = Mutex::new(Ledger::open(temp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"Original line 1.\nOriginal line 2.\nOriginal line 3.\n",
            source_id_hint: Some("paper"),
            section_filter: None,
            title: Some("Original Paper"),
        },
    )
    .unwrap();
    drop(db);
    let primary_path = src.primary_path.clone().unwrap();
    *shared_path.lock().unwrap() = primary_path.clone();

    let tampered: String = (1..=50).map(|i| format!("Tampered line {i}.\n")).collect();
    let disk_path = temp
        .path()
        .join("sessions")
        .join(sid)
        .join("sources")
        .join(&primary_path);
    std::fs::write(&disk_path, tampered.as_bytes()).unwrap();

    call_api(
        &server,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        json!({
            "prompt": "What is on lines 40-45?",
            "provider": "ollama",
            "model": "llama3.2",
            "investigation_source_id": src.source_id,
            "investigation_scope": "question"
        }),
        202,
    );
    let view = wait_idle(&server, sid);
    server.shutdown();

    let cites = &view["investigation"]["answers"][0]["cites"];
    let any_read = cites
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["read"] == true);
    assert!(
        !any_read,
        "tampered source file failed ingestion SHA-256 pin yet produced read=true citations: {cites}"
    );
}

/// Finding F17 (critical): when `investigation.paper` pins PDF pages `page_start: 2, page_end: 2`
/// (lines 11..=20), `answer_cites_for_session` (`investigation_cites.rs:292-324`) ignores
/// `SelectedPages` and marks citations `[L3-4]` on unselected Page 1 as `read: true`.
#[test]
fn audit_honesty_unselected_pdf_pages_must_not_produce_checked_read_true_cites() {
    let temp = tempfile::tempdir().unwrap();
    let sid = "s-pdf-cites";
    let mut lines = Vec::new();
    for i in 1..=10 {
        lines.push(format!("Page one unselected prose line {i} goes here."));
    }
    for i in 11..=20 {
        lines.push(format!("Page two selected prose line {i} goes here."));
    }
    let extracted = lines.join("\n") + "\n";

    let rel_path = "pdf1/extracted.txt";
    let full_dir = temp
        .path()
        .join("sessions")
        .join(sid)
        .join("sources")
        .join("pdf1");
    std::fs::create_dir_all(&full_dir).unwrap();
    std::fs::write(full_dir.join("extracted.txt"), extracted.as_bytes()).unwrap();

    let ledger = Ledger::open(temp.path()).unwrap();
    let raw_blob = ledger.put_blob(b"%PDF-1.4 synthetic fixture").unwrap();
    ledger
        .append(sid, "investigation.started", 1, &json!({}))
        .unwrap();
    ledger
        .append(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": "pdf1",
                "format": "pdf",
                "status": "ok",
                "title": "Bound Volume",
                "page_count": 2,
                "pdf_page_lines": [[1, 1, 10], [2, 11, 20]],
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { rel_path: sha256_hex(extracted.as_bytes()) },
                "raw_blob": raw_blob
            }),
        )
        .unwrap();
    drop(ledger);

    let mock = LoopbackOllamaMock::start(Box::new(|body| {
        let has_tool_msg = body["messages"]
            .as_array()
            .is_some_and(|m| m.iter().any(|x| x["role"] == "tool"));
        if !has_tool_msg {
            oai_tools(vec![(
                "r_p1",
                "read_paper_lines",
                json!({"source_id": "pdf1", "start_line": 1, "end_line": 5}),
            )])
        } else {
            oai_text("Unselected page 1 claim [L3-4].")
        }
    }));

    let mut cfg = test_cfg(temp.path(), None, None, None);
    cfg.api_base_override = Some(format!("{}/v1", mock.base_url));
    let server = start_desktop_server(cfg).unwrap();
    call_api(
        &server,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        json!({
            "prompt": "What does the selected paper say?",
            "provider": "ollama",
            "model": "llama3.2",
            "investigation_source_id": "pdf1",
            "investigation_page_start": 2,
            "investigation_page_end": 2,
            "investigation_scope": "question"
        }),
        202,
    );
    let view = wait_idle(&server, sid);
    server.shutdown();

    let cites = &view["investigation"]["answers"][0]["cites"];
    assert_eq!(
        cites,
        &json!([{"start": 3, "end": 4, "read": false}]),
        "unselected PDF page 1 produced read=true answer citations despite page_start=2..=2: {cites}"
    );
}

/// Finding F20 (major): `crates/desktop-server/src/evidence.rs` (`describe_tool` and
/// `linked_session_sources`) has no arm for `"read_paper_lines"`, downgrading Proof-DAG
/// steps backed by `read_paper_lines` to `"status": "unsupported_reference"` and omitting
/// the paper from `linked_sources` (`Sources Used`).
#[test]
fn audit_honesty_proof_evidence_and_linked_sources_must_support_read_paper_lines() {
    let temp = tempfile::tempdir().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let res = call_api(
        &server,
        "POST",
        "/api/sessions",
        json!({"title": "RPL Evidence"}),
        200,
    );
    let sid = res["id"].as_str().unwrap().to_string();

    let env = Arc::new(ResearchEnv::new(temp.path().to_path_buf()));
    let db = Mutex::new(Ledger::open(temp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: &sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"Theorem 1: E = mc^2 holds in flat spacetime.\n",
            source_id_hint: Some("paper"),
            section_filter: None,
            title: Some("Relativity Note"),
        },
    )
    .unwrap();
    {
        let l = db.lock().unwrap();
        l.append(&sid, "session.start", 1, &json!({"prompt": "Check Theorem 1"}))
            .unwrap();
    }

    let mut reg = Registry::new();
    register_research_tools(&mut reg, Arc::clone(&env));
    let router = Router::new(
        reg,
        research_policy(),
        db.into_inner().unwrap(),
        &sid,
        Arc::new(AllowAll),
    );
    let rpl_out = router
        .try_call(
            "agent:ollama:test",
            "read_paper_lines",
            &json!({"source_id": src.source_id, "start_line": 1, "end_line": 1}),
        )
        .unwrap();
    assert!(matches!(rpl_out, CallOutcome::Ok(_)));

    let ledger = Ledger::open(temp.path()).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let call_id = events
        .iter()
        .rfind(|e| e.kind == "tool.request" && e.payload["tool"] == "read_paper_lines")
        .and_then(|e| e.payload["call_id"].as_str())
        .unwrap()
        .to_string();
    drop(events);
    let dag_out = router
        .try_call(
            "agent:ollama:test",
            "ledger_append",
            &json!({
                "entry_type": "reference_lookup",
                "summary": "Theorem 1 states E = mc^2.",
                "details": "Read directly via read_paper_lines.",
                "tool_call_ref": call_id,
                "depends_on": []
            }),
        )
        .unwrap();
    assert!(matches!(dag_out, CallOutcome::Ok(_)), "{dag_out:?}");
    ledger
        .append(&sid, "session.end", 1, &json!({"outcome": "completed"}))
        .unwrap();

    let ev_resp = call_api(
        &server,
        "GET",
        &format!("/api/sessions/{sid}/evidence"),
        Value::Null,
        200,
    );
    let session_view = call_api(&server, "GET", &format!("/api/sessions/{sid}"), Value::Null, 200);
    server.shutdown();

    let support = &ev_resp["entries"][0]["support"];
    assert!(
        support[0]["status"] != "unsupported_reference"
            && !session_view["linked_sources"].as_array().unwrap().is_empty(),
        "read_paper_lines Proof-DAG reference rejected: support={support} linked_sources={}",
        session_view["linked_sources"]
    );
}
