//! Failing audit proof tests (`audit-flow`) for Investigation turn flow, prompts,
//! section-scoped conventions, checked line citations, and session-view status in `desktop-server`.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::{
    ingest_bytes,
    investigation::{record, record_conventions},
    IngestRequest, ResearchEnv,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

type Handler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct MockServer {
    base_url: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

impl MockServer {
    fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (seen_cl, handler) = (Arc::clone(&seen), Arc::new(handler));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (h, s_inner) = (Arc::clone(&handler), Arc::clone(&seen_cl));
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
                    s_inner.lock().unwrap().push((path.clone(), body.clone()));
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
            seen,
        }
    }

    fn captured(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

fn ant_text(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_t", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn ant_tools(model: &str, calls: Vec<(&str, &str, Value)>) -> Value {
    let content: Vec<Value> = calls
        .into_iter()
        .map(|(id, name, input)| json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
        .collect();
    json!({
        "id": "msg_u", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "tool_use", "content": content,
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn ant_tool_results(body: &Value) -> Vec<Value> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_result")
        .cloned()
        .collect()
}

fn has_tool(body: &Value, name: &str) -> bool {
    body["tools"]
        .as_array()
        .is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name))
}

struct Client {
    port: u16,
    tok: String,
}

impl Client {
    fn new(srv: &RunningDesktopServer) -> Self {
        Self {
            port: srv.addr.port(),
            tok: srv.info.token.clone(),
        }
    }

    fn req(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
        let hdr = [
            ("X-ReSearch-Token", self.tok.as_str()),
            ("Content-Type", "application/json"),
        ];
        let bytes = body
            .map(|v| serde_json::to_vec(v).unwrap())
            .unwrap_or_default();
        let (st, _, raw) = raw_http(self.port, method, path, &hdr, &bytes);
        (
            st,
            serde_json::from_slice(&raw).unwrap_or(Value::Null),
            String::from_utf8_lossy(&raw).into_owned(),
        )
    }

    fn wait_idle(&self, sid: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (st, sv, raw) = self.req("GET", &format!("/api/sessions/{sid}"), None);
            assert_eq!(st, 200, "GET session failed: {raw}");
            if sv["running"] == json!(false) {
                return sv;
            }
            assert!(Instant::now() < deadline, "timed out waiting for idle: {sv}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn boot(mock_url: String) -> (TempDir, std::path::PathBuf, RunningDesktopServer, Client) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock_url), None)).unwrap();
    let client = Client::new(&srv);
    let (st, _, raw) = client.req(
        "PUT",
        "/api/settings/providers",
        Some(&json!({ "keys": { "anthropic": "placeholder-offline-test-value" } })),
    );
    assert_eq!(st, 200, "{raw}");
    (tmp, data_dir, srv, client)
}

const APS_SHORT_PAPER: &str = "\
Title: Crystalline Defects in Chiral Chains
Abstract
We analyze lattice defects and boundary zero modes in chiral tight-binding chains.

I. INTRODUCTION
We introduce the chiral tight-binding chain on an open interval.

II. DEFECT MODEL
We study coupling modulation across a crystalline defect.

A. Microscopic Coupling
Throughout this section we set hbar = 1 and lattice spacing a = 1.
Theorem 1: A single bond defect traps an exact zero-energy state.

B. Bulk Winding
Away from the defect the bulk winding number remains quantized.

III. CONCLUSIONS
The zero mode is topologically protected against chiral-preserving disorder.";

fn ingest_aps(data_dir: &Path, sid: &str) -> (String, String) {
    let env = ResearchEnv::new(data_dir.to_path_buf());
    let db = Mutex::new(Ledger::open(data_dir).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "aps.txt",
            origin: "upload",
            raw_bytes: APS_SHORT_PAPER.as_bytes(),
            source_id_hint: Some("aps-1"),
            section_filter: None,
            title: Some("Crystalline Defects in Chiral Chains"),
        },
    )
    .unwrap();
    (src.source_id, src.primary_path.unwrap())
}

fn seed_section_scoped_map(data_dir: &Path, sid: &str, src_id: &str) -> (u64, u64) {
    let env = ResearchEnv::new(data_dir.to_path_buf());
    let ledger = Ledger::open(data_dir).unwrap();
    ledger
        .append(
            sid,
            "investigation.paper",
            1,
            &json!({"source_id": src_id, "question": "Section II.A"}),
        )
        .unwrap();
    let req_seq = ledger
        .append(
            sid,
            "investigation.mapping_requested",
            1,
            &json!({
                "source_id": src_id,
                "question": "Section II.A",
                "summary_requested": false,
                "conventions_requested": false,
                "conventions_scope": "section"
            }),
        )
        .unwrap();
    let map_res = record(
        &env,
        &ledger,
        sid,
        &json!({
            "source_id": src_id,
            "title": "Crystalline Defects in Chiral Chains",
            "coverage": "paper",
            "limitations": [],
            "nodes": [{
                "id": "r1", "kind": "result", "main": true,
                "sentence": "A single bond defect traps an exact zero-energy state.",
                "anchors": [{
                    "source_id": src_id, "start_line": 13, "end_line": 13,
                    "quote": "A single bond defect traps an exact zero-energy state.", "kind": "passage"
                }]
            }],
            "edges": []
        }),
    )
    .unwrap();
    (req_seq, map_res["map_seq"].as_u64().unwrap())
}

#[test]
fn investigation_turn_system_prompt_excludes_final_answer_and_proof_dag_lead() {
    let mock = MockServer::start(Box::new(|_, _| {
        ant_text("claude-sonnet-4-6", "Theorem 1 traps a zero mode.")
    }));
    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "inv-sys-prompt";
    client.req(
        "POST",
        "/api/sessions",
        Some(&json!({ "id": sid, "mode": "investigation" })),
    );
    let (src_id, _) = ingest_aps(&data_dir, sid);

    let (st, _, raw) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "What does Theorem 1 establish?",
            "investigation_source_id": src_id,
            "investigation_scope": "question",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202, "{raw}");
    client.wait_idle(sid);

    let ledger = Ledger::open(&data_dir).unwrap();
    let events = ledger.read_session(sid).unwrap();
    let start = events
        .iter()
        .find(|e| e.kind == "session.start")
        .expect("session.start must be recorded");
    let frozen_sys = start.payload["system_prompt"].as_str().unwrap_or("");
    assert!(
        !frozen_sys.contains("final_answer")
            && !frozen_sys.contains("`ledger_append` with `entry_type"),
        "Investigation session.start froze Research Proof-DAG/final_answer synthesis instructions into system_prompt: {frozen_sys}"
    );
}

#[test]
fn section_scoped_map_prompt_does_not_ask_for_conventions_when_conventions_off() {
    let mock = MockServer::start(Box::new(|_, _| ant_text("claude-sonnet-4-6", "Done.")));
    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "inv-sec-map-prompt";
    client.req(
        "POST",
        "/api/sessions",
        Some(&json!({ "id": sid, "mode": "investigation" })),
    );
    let (src_id, _) = ingest_aps(&data_dir, sid);

    let (st, _, raw) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Help me understand section II.A",
            "investigation_source_id": src_id,
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202, "{raw}");
    client.wait_idle(sid);

    let first_user = mock.captured()[0].1["messages"][0]["content"].to_string();
    assert!(!first_user.contains("record_paper_conventions"));
    assert!(
        !first_user.contains("record conventions for the scope"),
        "section-scoped map with conventions_requested=false still instructs model to record conventions without request_seq: {first_user}"
    );
}

#[test]
fn first_conventions_run_on_section_map_starts_at_section_scope_not_paper_expansion() {
    let mock = MockServer::start(Box::new(|_, _| ant_text("claude-sonnet-4-6", "Done.")));
    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "inv-sec-conv-first";
    client.req(
        "POST",
        "/api/sessions",
        Some(&json!({ "id": sid, "mode": "investigation" })),
    );
    let (src_id, _) = ingest_aps(&data_dir, sid);
    let (_, map_seq) = seed_section_scoped_map(&data_dir, sid, &src_id);

    let (st_c, _, raw_c) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/investigation/conventions"),
        Some(&json!({ "map_seq": map_seq, "provider": "anthropic", "model": "claude-sonnet-4-6" })),
    );
    assert_eq!(st_c, 202, "{raw_c}");
    client.wait_idle(sid);

    let ledger = Ledger::open(&data_dir).unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let resumed = evs
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.conventions_resumed")
        .expect("conventions_resumed must be recorded");
    assert_eq!(
        resumed.payload["scope"],
        "section",
        "with null conventions catalog on a section-scoped map, POST /investigation/conventions skipped section scope and jumped straight to paper expansion: {}",
        resumed.payload
    );
}

#[test]
fn incomplete_section_conventions_resume_stays_section_and_failed_expansion_clears_flag() {
    let mock = MockServer::start(Box::new(|_, _| ant_text("claude-sonnet-4-6", "Stopped early.")));
    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "inv-sec-conv-partial";
    client.req(
        "POST",
        "/api/sessions",
        Some(&json!({ "id": sid, "mode": "investigation" })),
    );
    let (src_id, _) = ingest_aps(&data_dir, sid);
    let (req_seq, map_seq) = seed_section_scoped_map(&data_dir, sid, &src_id);

    let env = ResearchEnv::new(data_dir.clone());
    let ledger = Ledger::open(&data_dir).unwrap();
    let conv_res = record_conventions(
        &env,
        &ledger,
        sid,
        &json!({
            "request_seq": req_seq,
            "source_id": src_id,
            "batch_index": 1,
            "done": true,
            "coverage": "paper",
            "limitations": [],
            "items": [
                {
                    "id": "c-ok", "category": "units", "label": "hbar = 1",
                    "meaning": "Natural units", "scope": "Section II.A", "basis": "explicit",
                    "anchors": [{
                        "source_id": src_id, "start_line": 12, "end_line": 12,
                        "quote": "hbar = 1 and lattice spacing a = 1", "kind": "passage"
                    }]
                },
                {
                    "id": "c-bad", "category": "notation", "label": "eta",
                    "meaning": "Nonexistent symbol", "scope": "Section II.A", "basis": "explicit",
                    "anchors": [{
                        "source_id": src_id, "start_line": 12, "end_line": 12,
                        "quote": "quote that does not appear anywhere in the paper", "kind": "passage"
                    }]
                }
            ]
        }),
    )
    .unwrap();
    assert_eq!(conv_res["done"], false);

    let (st_c, _, raw_c) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/investigation/conventions"),
        Some(&json!({ "map_seq": map_seq, "provider": "anthropic", "model": "claude-sonnet-4-6" })),
    );
    assert_eq!(st_c, 202, "{raw_c}");
    let view = client.wait_idle(sid);

    let evs = ledger.read_session(sid).unwrap();
    let resumed = evs.iter().rev().find(|e| e.kind == "investigation.conventions_resumed").unwrap();
    assert!(
        resumed.payload["scope"] == "section" && view["investigation"]["conventions_expanding"] == false,
        "incomplete section catalog (done=false) upgraded to scope={} and left conventions_expanding={} after turn finished (running={}): resumed={}",
        resumed.payload["scope"], view["investigation"]["conventions_expanding"], view["running"], resumed.payload
    );
}

#[test]
fn checked_answer_cites_ignore_isolated_worker_reads_and_eof_zero_char_fs_read() {
    let sid = "inv-cite-leak";
    let tmp_holder: Arc<Mutex<(String, String)>> =
        Arc::new(Mutex::new((String::new(), String::new())));
    let ids = Arc::clone(&tmp_holder);

    let mock = MockServer::start(Box::new(move |_, body| {
        let (src_id, rel_path) = ids.lock().unwrap().clone();
        if !has_tool(body, "delegate_literature") {
            if ant_tool_results(body).is_empty() {
                ant_tools(
                    "claude-haiku-4-5",
                    vec![(
                        "w_read",
                        "fs_read",
                        json!({ "root": "sources", "path": rel_path, "offset": 0, "max_chars": 60 }),
                    )],
                )
            } else {
                ant_text("claude-haiku-4-5", "Located the defect section.")
            }
        } else if ant_tool_results(body).is_empty() {
            ant_tools(
                "claude-sonnet-4-6",
                vec![
                    (
                        "o_del",
                        "delegate_literature",
                        json!({ "task": format!("Check source {src_id}") }),
                    ),
                    (
                        "o_eof",
                        "fs_read",
                        json!({ "root": "sources", "path": rel_path, "offset": 99_999, "max_chars": 200 }),
                    ),
                ],
            )
        } else {
            ant_text(
                "claude-sonnet-4-6",
                "The chain is introduced [L1] and protected against disorder [L19].",
            )
        }
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    client.req(
        "PUT",
        "/api/settings/providers",
        Some(&json!({
            "agent_models": { "literature_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" } }
        })),
    );
    client.req(
        "POST",
        "/api/sessions",
        Some(&json!({ "id": sid, "mode": "investigation" })),
    );
    let (src_id, rel_path) = ingest_aps(&data_dir, sid);
    *tmp_holder.lock().unwrap() = (src_id.clone(), rel_path);

    let (st, _, raw) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Summarize lines 1 and 19.",
            "investigation_source_id": src_id,
            "investigation_scope": "question",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202, "{raw}");
    let sv = client.wait_idle(sid);

    let cites = sv["investigation"]["answers"][0]["cites"]
        .as_array()
        .expect("cites array");
    assert_eq!(cites.len(), 2, "expected 2 parsed citations: {cites:?}");
    assert_eq!(
        cites[0]["read"], false,
        "line 1 was read ONLY inside isolated literature_worker (never received by Orchestrator), yet marked read=true: {cites:?}"
    );
    assert_eq!(
        cites[1]["read"], false,
        "line 19 came from a 0-char past-EOF fs_read (content=\"\"), yet marked read=true: {cites:?}"
    );
}

#[test]
fn interrupted_question_does_not_resurrect_as_running_during_later_mapping_turn() {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rx = Arc::new(Mutex::new(rx));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let entered_tx = Arc::new(Mutex::new(Some(entered_tx)));

    let mock = MockServer::start(Box::new(move |_, _| {
        if let Some(s) = entered_tx.lock().unwrap().take() {
            let _ = s.send(());
        }
        let _ = rx.lock().unwrap().recv_timeout(Duration::from_secs(5));
        ant_text("claude-sonnet-4-6", "Mapping complete.")
    }));
    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "inv-stale-q";
    client.req("POST", "/api/sessions", Some(&json!({ "id": sid, "mode": "investigation" })));
    let (src_id, _) = ingest_aps(&data_dir, sid);

    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append(sid, "investigation.paper", 1, &json!({"source_id": src_id, "question": "Old question"})).unwrap();
    ledger.append(sid, "investigation.question_requested", 1, &json!({"source_id": src_id, "question": "Crashed earlier question?"})).unwrap();

    let (st, _, raw) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Map paper",
            "investigation_source_id": src_id,
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202, "{raw}");
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let (_, sv_mid, _) = client.req("GET", &format!("/api/sessions/{sid}"), None);
    let _ = tx.send(());
    client.wait_idle(sid);

    assert_eq!(
        sv_mid["investigation"]["answers"][0]["status"], "interrupted",
        "earlier crashed question turn resurrected as `running` during a subsequent mapping turn: {}",
        sv_mid["investigation"]["answers"]
    );
}
