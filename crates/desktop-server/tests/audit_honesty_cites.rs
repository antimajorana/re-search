//! Honesty audit failing integration tests for `desktop-server`:
//! - Answer-card checked line citations (`investigation_cites.rs`) crediting subagent reads,
//!   0-char past-EOF `fs_read`, and `line_truncated` `read_paper_lines`
//! - Checked `[Lstart–end]` citations >513 lines failing `routes_tools.rs` on click
//! - Selected PDF pages (`page_start..=page_end`) leaking out-of-page lines in `investigation_scope.rs`
//!   and `paper_reader.rs`
//! - Unverified disk file reads in `source_total_lines` overriding hash-verified `tool.result` metadata
//! - Request-body `paper_reader` override routing paper slices to an unassigned provider

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::{ingest_bytes, IngestRequest, IngestedSource, ResearchEnv};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

type Handler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct MockServer { base_url: String, seen: Arc<Mutex<Vec<(String, Value)>>> }

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
                    if reader.read_line(&mut req_line).unwrap_or(0) == 0 { return; }
                    let path = req_line.split_whitespace().nth(1).unwrap_or("")
                        .split('?').next().unwrap_or("").to_string();
                    let mut len = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() { break; }
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") { len = v.trim().parse().unwrap_or(0); }
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
        Self { base_url: format!("http://{addr}"), seen }
    }

    fn captured(&self) -> Vec<(String, Value)> { self.seen.lock().unwrap().clone() }
}

fn ant_text(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_t", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn ant_tool(model: &str, id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": "msg_u", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn oai_text(model: &str, text: &str) -> Value {
    json!({
        "id": "chat_t", "model": model,
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 10 }
    })
}

fn ant_tool_results(body: &Value) -> Vec<Value> {
    body["messages"].as_array().into_iter().flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_result").cloned().collect()
}

fn has_tool(body: &Value, name: &str) -> bool {
    body["tools"].as_array().is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name))
}

struct Client { port: u16, tok: String }

impl Client {
    fn new(srv: &RunningDesktopServer) -> Self { Self { port: srv.addr.port(), tok: srv.info.token.clone() } }

    fn req(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
        let hdr = [("X-ReSearch-Token", self.tok.as_str()), ("Content-Type", "application/json")];
        let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
        let (st, _, raw) = raw_http(self.port, method, path, &hdr, &bytes);
        (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
    }

    fn post_msg(&self, sid: &str, body: Value) {
        let (st, _, raw) = self.req("POST", &format!("/api/sessions/{sid}/messages"), Some(&body));
        assert_eq!(st, 202, "{raw}");
    }

    fn wait_idle(&self, sid: &str, min_events: usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (st, sv, raw) = self.req("GET", &format!("/api/sessions/{sid}"), None);
            assert_eq!(st, 200, "GET session failed: {raw}");
            let ev_len = sv["events"].as_array().map_or(0, Vec::len);
            if sv["running"] == json!(false) && ev_len >= min_events { return sv; }
            assert!(Instant::now() < deadline, "timed out waiting for idle: {sv}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn boot(mock_url: String) -> (TempDir, PathBuf, RunningDesktopServer, Client) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock_url), None)).unwrap();
    let client = Client::new(&srv);
    client.req("PUT", "/api/settings/providers", Some(&json!({
        "keys": { "anthropic": "fixture-ant", "openai": "fixture-oai" }
    })));
    (tmp, data_dir, srv, client)
}

fn create_inv_session(client: &Client, id: &str) {
    let (st, _, raw) = client.req("POST", "/api/sessions", Some(&json!({ "id": id, "mode": "investigation" })));
    assert_eq!(st, 200, "{raw}");
}

fn ingest_text(data_dir: &Path, sid: &str, name: &str, text: &str) -> IngestedSource {
    let env = ResearchEnv::new(data_dir.to_path_buf());
    let db = Mutex::new(Ledger::open(data_dir).unwrap());
    ingest_bytes(&env.sandbox(), &db, &IngestRequest {
        session_id: sid, file_name: name, origin: "upload", raw_bytes: text.as_bytes(),
        source_id_hint: None, section_filter: None, title: Some("Audit Paper"),
    }).unwrap()
}

#[test]
fn subagent_read_must_not_mark_orchestrator_answer_citation_as_read() {
    let primary_slot: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let primary_for_mock = Arc::clone(&primary_slot);

    let mock = MockServer::start(Box::new(move |_, body| {
        let sys = body["system"].as_str().unwrap_or("");
        let results = ant_tool_results(body);
        if sys.contains("Literature Worker") {
            if results.is_empty() {
                let p = primary_for_mock.lock().unwrap().clone();
                return ant_tool(
                    "claude-sonnet-4-6",
                    "tu_lit_read",
                    "fs_read",
                    json!({ "root": "sources", "path": p, "offset": 0, "max_chars": 4000 }),
                );
            }
            // Worker returns only a vague summary without quoting line 20.
            return ant_text("claude-sonnet-4-6", "Found relevant discussion near the end.");
        }
        // Orchestrator delegates to literature_worker and NEVER reads the paper itself.
        if results.is_empty() {
            ant_tool(
                "claude-sonnet-4-6",
                "tu_del",
                "delegate_literature",
                json!({ "task": "Check whether topological protection is discussed." }),
            )
        } else {
            ant_text(
                "claude-sonnet-4-6",
                "Topological protection is established in [L20].",
            )
        }
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "s-honesty-subagent-cite";
    create_inv_session(&client, sid);

    let paper_lines: String = (1..=25)
        .map(|i| format!("Paper statement on line {i}.\n"))
        .collect();
    let src = ingest_text(&data_dir, sid, "topo.txt", &paper_lines);
    *primary_slot.lock().unwrap() = src.primary_path.clone().unwrap();

    client.post_msg(
        sid,
        json!({
            "prompt": "Where is topological protection established?",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "investigation_source_id": src.source_id,
            "investigation_scope": "question"
        }),
    );
    let sv = client.wait_idle(sid, 6);
    let cites = &sv["investigation"]["answers"][0]["cites"];
    assert_eq!(
        cites[0]["read"],
        json!(false),
        "orchestrator never received line 20 (only literature_worker ran fs_read), yet cites={cites}"
    );
}

#[test]
fn zero_char_past_eof_fs_read_and_truncated_read_paper_lines_must_not_mark_cites_read() {
    let src_slot: Arc<Mutex<(String, String)>> = Arc::new(Mutex::new((String::new(), String::new())));
    let src_for_mock = Arc::clone(&src_slot);

    let mock = MockServer::start(Box::new(move |_, body| {
        let results = ant_tool_results(body);
        let (source_id, primary_path) = src_for_mock.lock().unwrap().clone();
        match results.len() {
            0 => ant_tool(
                "claude-sonnet-4-6",
                "tu_rpl",
                "read_paper_lines",
                json!({ "source_id": source_id, "start_line": 1, "end_line": 1 }),
            ),
            1 => ant_tool(
                "claude-sonnet-4-6",
                "tu_eof",
                "fs_read",
                json!({ "root": "sources", "path": primary_path, "offset": 999_999 }),
            ),
            _ => ant_text(
                "claude-sonnet-4-6",
                "The truncated tail [L1] and unread EOF line [L3] establish the claim.",
            ),
        }
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "s-honesty-partial-eof";
    create_inv_session(&client, sid);

    // Line 1 > 14,000 chars triggers line_truncated=true; line 3 has no trailing newline.
    let paper = format!(
        "Prefix: {}UNSEEN_TAIL_CLAIM\nMiddle line.\nFinal line with zero chars read",
        "x ".repeat(7_500)
    );
    let src = ingest_text(&data_dir, sid, "long.txt", &paper);
    *src_slot.lock().unwrap() = (src.source_id.clone(), src.primary_path.clone().unwrap());

    client.post_msg(
        sid,
        json!({
            "prompt": "What is on lines 1 and 3?",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "investigation_source_id": src.source_id,
            "investigation_scope": "question"
        }),
    );
    let sv = client.wait_idle(sid, 6);
    let cites = sv["investigation"]["answers"][0]["cites"]
        .as_array()
        .unwrap();
    let l1_read = cites.iter().find(|c| c["start"] == 1).and_then(|c| c["read"].as_bool());
    let l3_read = cites.iter().find(|c| c["start"] == 3).and_then(|c| c["read"].as_bool());
    assert_eq!(
        (l1_read, l3_read),
        (Some(false), Some(false)),
        "line_truncated=true on L1 and 0-char past-EOF fs_read on L3 were marked read:true: {cites:?}"
    );
}

#[test]
fn checked_cite_exceeding_513_lines_must_either_open_or_not_be_marked_read() {
    let src_id_slot: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let src_for_mock = Arc::clone(&src_id_slot);

    let mock = MockServer::start(Box::new(move |_, body| {
        let results = ant_tool_results(body);
        let source_id = src_for_mock.lock().unwrap().clone();
        match results.len() {
            0 => ant_tool(
                "claude-sonnet-4-6",
                "tu_1",
                "read_paper_lines",
                json!({ "source_id": source_id, "start_line": 1, "end_line": 350 }),
            ),
            1 => ant_tool(
                "claude-sonnet-4-6",
                "tu_2",
                "read_paper_lines",
                json!({ "source_id": source_id, "start_line": 351, "end_line": 600 }),
            ),
            _ => ant_text(
                "claude-sonnet-4-6",
                "Across the entire derivation [L1–550], energy is conserved.",
            ),
        }
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "s-honesty-wide-cite";
    create_inv_session(&client, sid);

    let paper: String = (1..=600).map(|i| format!("Line {i} equation.\n")).collect();
    let src = ingest_text(&data_dir, sid, "wide.txt", &paper);
    *src_id_slot.lock().unwrap() = src.source_id.clone();

    client.post_msg(
        sid,
        json!({
            "prompt": "Summarize the derivation.",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "investigation_source_id": src.source_id,
            "investigation_scope": "question"
        }),
    );
    let sv = client.wait_idle(sid, 6);
    let cite = &sv["investigation"]["answers"][0]["cites"][0];
    if cite["read"] == json!(true) {
        let path = src.primary_path.unwrap();
        let (st, _, raw) = client.req(
            "GET",
            &format!("/api/sessions/{sid}/workspace/file?root=sources&path={path}&start_line=1&end_line=550"),
            None,
        );
        assert_eq!(
            st, 200,
            "[L1–550] was marked read:true (clickable .line-cite button), but clicking it fails with HTTP {st}: {raw}"
        );
    }
}

#[test]
fn selected_pdf_pages_must_clip_resolved_section_scope_and_reader_slices() {
    let mock = MockServer::start(Box::new(move |_, body| {
        let sys = body["system"].as_str().unwrap_or("");
        let results = ant_tool_results(body);
        if sys.contains("Paper Reader") {
            return ant_text("claude-sonnet-4-6", "Slice read.");
        }
        if results.is_empty() && has_tool(body, "delegate_read_paper") {
            return ant_tool(
                "claude-sonnet-4-6",
                "tu_drp",
                "delegate_read_paper",
                json!({ "focus": "Section II derivation" }),
            );
        }
        ant_text("claude-sonnet-4-6", "Finished attempt.")
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    client.req(
        "PUT",
        "/api/settings/providers",
        Some(&json!({ "paper_readers_enabled": true })),
    );
    let sid = "s-honesty-pdf-scope";
    create_inv_session(&client, sid);

    // Build a 120-line APS-style paper across 4 physical PDF pages:
    // Page 1: L1..=20 (Abstract), Page 2: L21..=50 (I. INTRODUCTION),
    // Page 3: L51..=80 (start of II. MAIN DERIVATION), Page 4: L81..=120 (rest of II, III. CONCLUSION).
    let mut lines = vec!["Abstract".to_string()];
    for i in 2..=20 {
        lines.push(format!("Abstract sentence on line {i}."));
    }
    lines.push("I. INTRODUCTION".to_string()); // L21
    for i in 22..=50 {
        lines.push(format!("Introduction prose on line {i}."));
    }
    lines.push("II. MAIN DERIVATION".to_string()); // L51 (page 3)
    for i in 52..=110 {
        lines.push(format!("Derivation detail on line {i}."));
    }
    lines.push("III. CONCLUSION".to_string()); // L111 (page 4)
    for i in 112..=120 {
        lines.push(format!("Conclusion prose on line {i}."));
    }
    let text = lines.join("\n") + "\n";

    // Write verified source file and single PDF source.ingested receipt.
    let source_id = "pdf-scoped-1";
    let rel_path = format!("{source_id}/extracted.txt");
    let sources_dir = data_dir.join("sessions").join(sid).join("sources").join(source_id);
    std::fs::create_dir_all(&sources_dir).unwrap();
    std::fs::write(sources_dir.join("extracted.txt"), text.as_bytes()).unwrap();
    let sha = ledger::sha256_hex(text.as_bytes());
    let ledger = Ledger::open(&data_dir).unwrap();
    let raw_blob = ledger.put_blob(text.as_bytes()).unwrap();
    ledger
        .append(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": source_id,
                "origin": "upload",
                "format": "pdf",
                "status": "ok",
                "title": "Scoped PDF",
                "page_count": 4,
                "pdf_page_lines": [[1, 1, 20], [2, 21, 50], [3, 51, 80], [4, 81, 120]],
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { rel_path: sha },
                "raw_blob": raw_blob
            }),
        )
        .unwrap();

    // User restricts Investigation strictly to physical PDF pages 2..=3 (lines 21..=80).
    client.post_msg(
        sid,
        json!({
            "prompt": "Explain section II",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "investigation_source_id": source_id,
            "investigation_scope": "map",
            "investigation_page_start": 2,
            "investigation_page_end": 3
        }),
    );
    let sv = client.wait_idle(sid, 6);
    let scope = &sv["investigation"]["scope"];
    let ranges = scope["ranges"].as_array().expect("resolved scope ranges");
    for r in ranges {
        let start = r[0].as_u64().unwrap();
        let end = r[1].as_u64().unwrap();
        assert!(
            start >= 21 && end <= 80,
            "user selected PDF pages 2–3 (lines 21–80), yet investigation.scope resolved unclipped range {start}..={end} (scope={scope})"
        );
    }
}

#[test]
fn unverified_disk_mutation_must_not_override_verified_total_lines_in_answer_cites() {
    let info_slot: Arc<Mutex<(String, PathBuf)>> =
        Arc::new(Mutex::new((String::new(), PathBuf::new())));
    let info_for_mock = Arc::clone(&info_slot);

    let mock = MockServer::start(Box::new(move |_, body| {
        let results = ant_tool_results(body);
        let (source_id, disk_file) = info_for_mock.lock().unwrap().clone();
        if results.is_empty() {
            ant_tool(
                "claude-sonnet-4-6",
                "tu_read",
                "read_paper_lines",
                json!({ "source_id": source_id, "start_line": 1, "end_line": 20 }),
            )
        } else {
            // Corrupt/truncate the on-disk file after verified read_paper_lines succeeded.
            std::fs::write(&disk_file, b"corrupted single line\n").unwrap();
            ant_text("claude-sonnet-4-6", "The theorem is proved in [L15].")
        }
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    let sid = "s-honesty-disk-tamper";
    create_inv_session(&client, sid);

    let paper: String = (1..=20).map(|i| format!("Verified line {i}.\n")).collect();
    let src = ingest_text(&data_dir, sid, "verified.txt", &paper);
    let rel = src.primary_path.clone().unwrap();
    let disk_path = data_dir.join("sessions").join(sid).join("sources").join(&rel);
    *info_slot.lock().unwrap() = (src.source_id.clone(), disk_path);

    client.post_msg(
        sid,
        json!({
            "prompt": "Where is the theorem proved?",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "investigation_source_id": src.source_id,
            "investigation_scope": "question"
        }),
    );
    let sv = client.wait_idle(sid, 5);
    let cites = &sv["investigation"]["answers"][0]["cites"];
    assert_eq!(
        cites,
        &json!([{ "start": 15, "end": 15, "read": true }]),
        "unverified on-disk truncation overrode hash-verified tool.result total_lines=20 and dropped [L15]: cites={cites}"
    );
}

#[test]
fn request_body_paper_reader_override_must_not_send_paper_to_unassigned_provider() {
    let mock = MockServer::start(Box::new(move |path, body| {
        if path.contains("/chat/completions") {
            return oai_text("gpt-4o-mini", "OpenAI received paper text.");
        }
        let results = ant_tool_results(body);
        if results.is_empty() && has_tool(body, "delegate_read_paper") {
            return ant_tool(
                "claude-sonnet-4-6",
                "tu_drp",
                "delegate_read_paper",
                json!({ "focus": "main results" }),
            );
        }
        ant_text("claude-sonnet-4-6", "Done.")
    }));

    let (_tmp, data_dir, _srv, client) = boot(mock.base_url.clone());
    // Explicitly leave paper_reader unassigned in settings and paper_readers_enabled unset.
    client.req(
        "PUT",
        "/api/settings/providers",
        Some(&json!({
            "agent_models": { "paper_reader": null },
            "paper_readers_enabled": null
        })),
    );
    let sid = "s-honesty-reader-override";
    create_inv_session(&client, sid);
    let src = ingest_text(
        &data_dir,
        sid,
        "secret.txt",
        "Title: Private Draft\nResult: confidential derivation.\n",
    );

    client.post_msg(
        sid,
        json!({
            "prompt": "Identify the paper's main results.",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6",
            "investigation_source_id": src.source_id,
            "investigation_scope": "map",
            "paper_reader": { "provider": "openai", "model": "gpt-4o-mini" }
        }),
    );
    let _sv = client.wait_idle(sid, 4);
    let openai_calls: Vec<_> = mock
        .captured()
        .into_iter()
        .filter(|(path, _)| path.contains("/chat/completions"))
        .collect();
    assert!(
        openai_calls.is_empty(),
        "paper_reader was NOT assigned in settings, yet request-body override auto-enabled readers and sent {} call(s) to OpenAI",
        openai_calls.len()
    );
}
