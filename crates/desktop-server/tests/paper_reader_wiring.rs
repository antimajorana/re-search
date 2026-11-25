//! WP8 integration tests: desktop wiring for isolated parallel paper readers (`delegate_read_paper`),
//! opt-in/default gating, `isolated_primary_model` routing, `"low"` wire effort vs no effort on
//! existing workers, end-to-end Investigation mapping with evidence anchors, and poll sanitization.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::paper_reader::{default_max_parallel, plan_slices, plan_slices_for_ranges};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv, EVIDENCE_EVENT};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
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
        "usage": { "prompt_tokens": 15, "completion_tokens": 20 }
    })
}

fn oai_tool(model: &str, id: &str, name: &str, args: Value) -> Value {
    json!({
        "id": "chat_u", "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": null, "tool_calls": [{ "id": id, "type": "function", "function": { "name": name, "arguments": args.to_string() } }] },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 15, "completion_tokens": 20 }
    })
}

fn has_tool(body: &Value, name: &str) -> bool {
    body["tools"].as_array().is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name))
}

fn ant_tool_results(body: &Value) -> Vec<Value> {
    body["messages"].as_array().into_iter().flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_result")
        .cloned().collect()
}

fn oai_tool_results(body: &Value) -> Vec<String> {
    body["messages"].as_array().into_iter().flatten()
        .filter(|m| m["role"] == "tool")
        .filter_map(|m| m["content"].as_str().map(str::to_string))
        .collect()
}

struct Client { port: u16, tok: String }

impl Client {
    fn new(srv: &RunningDesktopServer) -> Self {
        Self { port: srv.addr.port(), tok: srv.info.token.clone() }
    }

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

    fn wait_done(&self, sid: &str, min_msgs: usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (st, sv, raw) = self.req("GET", &format!("/api/sessions/{sid}"), None);
            assert_eq!(st, 200, "GET session failed: {raw}");
            if let Some(abort) = sv["events"].as_array()
                .and_then(|evs| evs.iter().rev().find(|e| e["kind"] == "session.abort"))
            {
                panic!("turn aborted unexpectedly: {abort}");
            }
            if sv["running"] == json!(false)
                && sv["messages"].as_array().is_some_and(|m| m.len() >= min_msgs)
            {
                return sv;
            }
            assert!(Instant::now() < deadline, "timed out waiting for turn: {sv}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

const PAPER_TEXT: &str = "\
Title: Chiral Edge Modes in One Dimension\n\
We study a chiral tight-binding chain to classify its protected boundary modes.\n\
Our method computes the winding number via contour integration over the Brillouin zone.\n\
Convention: throughout this paper hbar = 1 and lattice spacing a = 1.\n\
Theorem 1: The bulk winding number nu equals the number of zero-energy boundary states.\n";

fn ingest_paper(data_dir: &std::path::Path, sid: &str) -> String {
    let env = ResearchEnv::new(data_dir.to_path_buf());
    let db = Mutex::new(Ledger::open(data_dir).unwrap());
    ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "chiral.txt",
            origin: "upload",
            raw_bytes: PAPER_TEXT.as_bytes(),
            source_id_hint: Some("chiral-1"),
            section_filter: None,
            title: Some("Chiral Edge Modes"),
        },
    )
    .unwrap()
    .source_id
}

fn boot(mock_url: Option<String>) -> (TempDir, std::path::PathBuf, RunningDesktopServer, Client) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, mock_url, None)).unwrap();
    let client = Client::new(&srv);
    (tmp, data_dir, srv, client)
}

#[test]
fn offered_only_on_paper_recording_turns_in_core() {
    let mock = MockServer::start(Box::new(|_, _| ant_text("claude-sonnet-4-6", "Done.")));
    let (_tmp, data_dir, _srv, client) = boot(Some(mock.base_url.clone()));
    client.req("PUT", "/api/settings/providers", Some(&json!({
        "keys": { "anthropic": "sk-ant-wp8-t1" }, "paper_readers_enabled": true
    })));

    // 1. Plain Research turn must NEVER receive delegate_read_paper even when enabled.
    let (_, s_res, _) = client.req("POST", "/api/sessions", Some(&json!({ "title": "Plain" })));
    let sid_res = s_res["id"].as_str().unwrap();
    client.post_msg(sid_res, json!({
        "prompt": "Compute 2+2", "provider": "anthropic", "model": "claude-sonnet-4-6", "verification_mode": false
    }));
    client.wait_done(sid_res, 2);

    // 2. Investigation turn with default settings (opt-in unset, no smaller model) -> NOT offered.
    client.req("PUT", "/api/settings/providers", Some(&json!({ "paper_readers_enabled": null })));
    let (_, s_inv1, _) = client.req("POST", "/api/sessions", Some(&json!({ "id": "inv-off", "mode": "investigation" })));
    let sid_inv1 = s_inv1["id"].as_str().unwrap();
    let src1 = ingest_paper(&data_dir, sid_inv1);
    client.post_msg(sid_inv1, json!({
        "prompt": "Map paper", "investigation_source_id": src1, "provider": "anthropic", "model": "claude-sonnet-4-6"
    }));
    client.wait_done(sid_inv1, 2);

    // 3. Investigation turn with opt-in enabled -> IS offered.
    client.req("PUT", "/api/settings/providers", Some(&json!({ "paper_readers_enabled": true })));
    let (_, s_inv2, _) = client.req("POST", "/api/sessions", Some(&json!({ "id": "inv-on", "mode": "investigation" })));
    let sid_inv2 = s_inv2["id"].as_str().unwrap();
    let src2 = ingest_paper(&data_dir, sid_inv2);
    client.post_msg(sid_inv2, json!({
        "prompt": "Map paper", "investigation_source_id": src2, "provider": "anthropic", "model": "claude-sonnet-4-6"
    }));
    client.wait_done(sid_inv2, 2);

    let cap = mock.captured();
    assert_eq!(cap.len(), 3);
    assert!(!has_tool(&cap[0].1, "delegate_read_paper"), "Research turn must not offer delegate_read_paper");
    assert!(!has_tool(&cap[1].1, "delegate_read_paper"), "Default Investigation turn must have readers off");
    assert!(has_tool(&cap[2].1, "delegate_read_paper"), "Opt-in Investigation mapping turn must offer delegate_read_paper");
    assert!(cap[2].1["messages"][0]["content"].to_string().contains("delegate_read_paper"));

    assert_eq!(default_max_parallel(agent::ProviderKind::ChatGpt), 2);
    assert_eq!(default_max_parallel(agent::ProviderKind::Anthropic), 4);
    assert_eq!(plan_slices(PAPER_TEXT, &[], None, None, 4).len(), 1);
}

#[test]
fn routing_reason_isolated_primary_model() {
    let mock = MockServer::start(Box::new(|_, body| {
        if has_tool(body, "note_evidence") {
            ant_text("claude-sonnet-4-6", "Reader done.")
        } else if ant_tool_results(body).is_empty() {
            ant_tool("claude-sonnet-4-6", "call_drp", "delegate_read_paper", json!({ "focus": "Theorem 1" }))
        } else {
            ant_text("claude-sonnet-4-6", "Orchestrator done.")
        }
    }));
    let (_tmp, data_dir, _srv, client) = boot(Some(mock.base_url.clone()));
    let (st_set, set_v, raw_set) = client.req("PUT", "/api/settings/providers", Some(&json!({
        "keys": { "anthropic": "sk-ant-wp8-t2" },
        "paper_readers_enabled": true,
        "paper_reader_max_parallel": 3
    })));
    assert_eq!(st_set, 200, "{raw_set}");
    assert_eq!(set_v["paper_readers_enabled"], true);
    assert_eq!(set_v["paper_reader_max_parallel"], 3);

    let (st_sa, sa_v, _) = client.req("GET", "/api/subagents", None);
    assert_eq!(st_sa, 200);
    assert!(sa_v["builtin"].as_array().unwrap().iter().any(|b| b["id"] == "paper-reader" && b["role_key"] == "paper_reader"));

    client.req("POST", "/api/sessions", Some(&json!({ "id": "inv-iso", "mode": "investigation" })));
    let src = ingest_paper(&data_dir, "inv-iso");
    client.post_msg("inv-iso", json!({
        "prompt": "Read paper", "investigation_source_id": src, "provider": "anthropic", "model": "claude-sonnet-4-6"
    }));
    client.wait_done("inv-iso", 2);

    let ledger = Ledger::open(&data_dir).unwrap();
    let evs = ledger.read_session("inv-iso").unwrap();
    let d_start = evs.iter().find(|e| e.kind == "delegate.start" && e.payload["role"] == "paper_reader").expect("delegate.start");
    assert_eq!(d_start.payload["routing_reason"], "isolated_primary_model");
    assert_eq!(d_start.payload["model"], "claude-sonnet-4-6");
    let d_end = evs.iter().find(|e| e.kind == "delegate.end" && e.payload["worker"] == "paper_reader").expect("delegate.end");
    assert_eq!(d_end.payload["routing_reason"], "isolated_primary_model");
    ledger.verify_session("inv-iso").unwrap();
}

#[test]
fn reader_wire_effort_low() {
    let mock = MockServer::start(Box::new(|_, body| {
        if has_tool(body, "note_evidence") {
            ant_text("claude-haiku-4-5", "Reader finished.")
        } else if ant_tool_results(body).is_empty() {
            ant_tool("claude-sonnet-4-6", "call_drp_eff", "delegate_read_paper", json!({ "focus": "Conventions" }))
        } else {
            ant_text("claude-sonnet-4-6", "Done.")
        }
    }));
    let (_tmp, data_dir, _srv, client) = boot(Some(mock.base_url.clone()));
    client.req("PUT", "/api/settings/providers", Some(&json!({
        "keys": { "anthropic": "sk-ant-wp8-t3" },
        "agent_models": { "paper_reader": { "provider": "anthropic", "model": "claude-haiku-4-5" } }
    })));
    client.req("POST", "/api/sessions", Some(&json!({ "id": "inv-eff", "mode": "investigation" })));
    let src = ingest_paper(&data_dir, "inv-eff");
    client.post_msg("inv-eff", json!({
        "prompt": "Map conventions", "investigation_source_id": src, "provider": "anthropic", "model": "claude-sonnet-4-6"
    }));
    client.wait_done("inv-eff", 2);

    let cap = mock.captured();
    let reader_reqs: Vec<_> = cap.iter().filter(|(_, b)| has_tool(b, "note_evidence")).collect();
    assert!(!reader_reqs.is_empty(), "expected at least one reader wire request");
    for (_, body) in reader_reqs {
        assert_eq!(body["model"], "claude-haiku-4-5");
        assert_eq!(body["output_config"], json!({ "effort": "low" }), "reader request must carry effort=low: {body}");
    }
}

#[test]
fn existing_roles_no_effort() {
    let mock = MockServer::start(Box::new(|path, body| {
        if path == "/v1/messages" {
            ant_text("claude-haiku-4-5", "Worker finished.")
        } else {
            match oai_tool_results(body).len() {
                0 => oai_tool("gpt-4.1", "c_comp", "delegate_compute", json!({ "task": "Check 2+2" })),
                1 => oai_tool("gpt-4.1", "c_lit", "delegate_literature", json!({ "task": "Find paper" })),
                _ => oai_text("gpt-4.1", "Both workers done."),
            }
        }
    }));
    let (_tmp, _data_dir, _srv, client) = boot(Some(mock.base_url.clone()));
    client.req("PUT", "/api/settings/providers", Some(&json!({
        "keys": { "openai": "sk-oai-wp8-t4", "anthropic": "sk-ant-wp8-t4" },
        "agent_models": {
            "compute_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" },
            "literature_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" }
        }
    })));
    let (_, sess, _) = client.req("POST", "/api/sessions", Some(&json!({ "title": "No effort" })));
    let sid = sess["id"].as_str().unwrap();
    client.post_msg(sid, json!({
        "prompt": "Delegate to compute and literature", "provider": "openai", "model": "gpt-4.1", "verification_mode": false
    }));
    client.wait_done(sid, 2);

    let cap = mock.captured();
    let worker_reqs: Vec<_> = cap.iter().filter(|(p, _)| p == "/v1/messages").collect();
    assert_eq!(worker_reqs.len(), 2);
    for (_, body) in worker_reqs {
        assert!(body.get("output_config").is_none(), "existing worker role must NOT send effort: {body}");
    }
}

#[test]
fn e2e_mock_reader_then_ev_anchors_finish_mapped() {
    let ev_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_cl = Arc::clone(&ev_ids);
    let src_shared: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let src_cl = Arc::clone(&src_shared);

    let mock = MockServer::start(Box::new(move |_, body| {
        let src_id = src_cl.lock().unwrap().clone();
        if has_tool(body, "note_evidence") {
            match ant_tool_results(body).len() {
                0 => ant_tool("claude-haiku-4-5", "r_read", "read_paper_lines",
                    json!({ "source_id": src_id, "start_line": 1, "end_line": 5 })),
                1 => ant_tool("claude-haiku-4-5", "r_note", "note_evidence", json!({ "items": [
                    { "source_id": src_id, "start_line": 2, "end_line": 2,
                      "quote": "We study a chiral tight-binding chain to classify its protected boundary modes.",
                      "kind": "passage", "role": "motivation", "gist": "Study chiral chain boundary modes." },
                    { "source_id": src_id, "start_line": 3, "end_line": 3,
                      "quote": "Our method computes the winding number via contour integration over the Brillouin zone.",
                      "kind": "passage", "role": "method", "gist": "Contour integration over Brillouin zone." },
                    { "source_id": src_id, "start_line": 4, "end_line": 4,
                      "quote": "throughout this paper hbar = 1 and lattice spacing a = 1.",
                      "kind": "passage", "role": "convention", "gist": "hbar=1 and a=1." },
                    { "source_id": src_id, "start_line": 5, "end_line": 5,
                      "quote": "The bulk winding number nu equals the number of zero-energy boundary states.",
                      "kind": "passage", "role": "result", "gist": "Theorem 1 bulk-boundary correspondence." }
                ] })),
                _ => ant_text("claude-haiku-4-5", "Noted 4 evidence pointers."),
            }
        } else {
            let results = ant_tool_results(body);
            match results.len() {
                0 => ant_tool("claude-sonnet-4-6", "o_drp", "delegate_read_paper",
                    json!({ "focus": "Summary, conventions, and Theorem 1" })),
                1 => {
                    let drp: Value = serde_json::from_str(results[0]["content"].as_str().unwrap()).unwrap();
                    let evs: Vec<String> = drp["evidence"].as_array().unwrap().iter()
                        .map(|e| e["e"].as_str().unwrap().to_string()).collect();
                    assert_eq!(evs.len(), 4, "{drp}");
                    *ev_cl.lock().unwrap() = evs.clone();
                    ant_tool("claude-sonnet-4-6", "o_sum", "record_paper_summary", json!({
                        "source_id": src_id, "title": "Chiral Edge Modes in One Dimension",
                        "what": { "text": "Classifies protected boundary modes of chiral 1D chains.",
                            "anchors": [{ "source_id": src_id, "evidence": evs[0], "quote": "We study a chiral tight-binding chain to classify its protected boundary modes.", "kind": "passage" }] },
                        "why": { "text": "To classify protected boundary modes.",
                            "anchors": [{ "source_id": src_id, "evidence": evs[0], "quote": "classify its protected boundary modes.", "kind": "passage" }] },
                        "how": { "text": "Contour integration over the Brillouin zone.",
                            "anchors": [{ "source_id": src_id, "evidence": evs[1], "quote": "computes the winding number via contour integration over the Brillouin zone.", "kind": "passage" }] }
                    }))
                }
                2 => {
                    let evs = ev_cl.lock().unwrap().clone();
                    ant_tool("claude-sonnet-4-6", "o_map", "record_paper_map", json!({
                        "source_id": src_id, "title": "Chiral Edge Modes in One Dimension",
                        "coverage": "paper", "limitations": [], "edges": [],
                        "nodes": [{
                            "id": "thm1", "sentence": "The bulk winding number equals the count of zero-energy boundary states.",
                            "kind": "result", "main": true,
                            "anchors": [{ "source_id": src_id, "evidence": evs[3],
                                "quote": "The bulk winding number nu equals the number of zero-energy boundary states.", "kind": "passage" }]
                        }]
                    }))
                }
                3 => {
                    let evs = ev_cl.lock().unwrap().clone();
                    let req_seq = results
                        .get(2)
                        .and_then(|r| r["content"].as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .and_then(|v| v["request_seq"].as_u64())
                        .or_else(|| {
                            body["messages"][0]["content"]
                                .to_string()
                                .split("request_seq=")
                                .nth(1)
                                .and_then(|s| {
                                    s.chars()
                                        .take_while(|c| c.is_ascii_digit())
                                        .collect::<String>()
                                        .parse::<u64>()
                                        .ok()
                                })
                        })
                        .unwrap();
                    ant_tool("claude-sonnet-4-6", "o_conv", "record_paper_conventions", json!({
                        "request_seq": req_seq, "source_id": src_id, "batch_index": 1, "done": true,
                        "coverage": "paper", "limitations": [],
                        "items": [{
                            "id": "units-hbar", "category": "units", "label": "hbar = 1, a = 1",
                            "meaning": "Reduced Planck constant and lattice spacing set to unity.",
                            "scope": "Throughout the paper", "basis": "explicit",
                            "anchors": [{ "source_id": src_id, "evidence": evs[2],
                                "quote": "throughout this paper hbar = 1 and lattice spacing a = 1.", "kind": "passage" }]
                        }]
                    }))
                }
                _ => ant_text("claude-sonnet-4-6", "Paper mapped using reader evidence."),
            }
        }
    }));

    let (_tmp, data_dir, _srv, client) = boot(Some(mock.base_url.clone()));
    client.req("PUT", "/api/settings/providers", Some(&json!({
        "keys": { "anthropic": "sk-ant-wp8-e2e" },
        "agent_models": { "paper_reader": { "provider": "anthropic", "model": "claude-haiku-4-5" } }
    })));
    client.req("POST", "/api/sessions", Some(&json!({ "id": "inv-e2e", "mode": "investigation" })));
    let src = ingest_paper(&data_dir, "inv-e2e");
    *src_shared.lock().unwrap() = src.clone();
    client.post_msg("inv-e2e", json!({
        "prompt": "Identify the main results and conventions.", "investigation_source_id": src,
        "provider": "anthropic", "model": "claude-sonnet-4-6"
    }));
    let sv = client.wait_done("inv-e2e", 2);
    assert_eq!(sv["investigation"]["mapping_status"], "mapped", "{sv}");
    let map_anchor = &sv["investigation"]["map"]["nodes"][0]["anchors"][0];
    assert_eq!(map_anchor["excerpt_basis"], "evidence", "{map_anchor}");
    assert!(map_anchor["excerpt"].as_str().unwrap().contains("bulk winding number nu"));

    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.verify_session("inv-e2e").unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn poll_view_omits_evidence_excerpt() {
    let (_tmp, data_dir, _srv, client) = boot(None);
    client.req("POST", "/api/sessions", Some(&json!({ "id": "inv-poll", "mode": "investigation" })));
    let ledger = Ledger::open(&data_dir).unwrap();
    let sample_excerpt = "The bulk winding number nu equals the number of zero-energy boundary states.";
    ledger.append("inv-poll", EVIDENCE_EVENT, 1, &json!({
        "marker_seq": 2, "source_id": "chiral-1", "path": "chiral-1/chiral.txt",
        "start_line": 5, "end_line": 5, "range_basis": [], "kind": "passage", "role": "result",
        "gist": "Theorem 1 statement", "quote": sample_excerpt, "excerpt": sample_excerpt,
        "file_sha256": "0".repeat(64), "source_seq": 1,
        "caller": "worker:paper_reader:anthropic:inv-poll:r0"
    })).unwrap();

    let (st, sv, raw) = client.req("GET", "/api/sessions/inv-poll", None);
    assert_eq!(st, 200, "{raw}");
    let ev = sv["events"].as_array().unwrap().iter()
        .find(|e| e["kind"] == EVIDENCE_EVENT).expect("investigation.evidence in poll view");
    assert!(ev["payload"].get("excerpt").is_none(), "excerpt must be stripped: {ev}");
    assert!(ev["payload"].get("quote").is_none(), "quote must be stripped: {ev}");
    assert_eq!(ev["payload"]["excerpt_chars"].as_u64(), Some(sample_excerpt.chars().count() as u64));
    assert_eq!(ev["payload"]["gist"], "Theorem 1 statement");
}

#[test]
fn review3_plan_slices_merges_smallest_gaps_to_n_max() {
    let text: String = (1..=300).map(|i| format!("Line {i}: sample paper text.\n")).collect();

    // 3 disjoint ranges with n_max = 2: gaps are (15-10=5) vs (100-20=80) -> merges first two.
    let slices = plan_slices_for_ranges(&text, &[], None, &[(1, 10), (15, 20), (100, 110)], 2);
    assert!(slices.len() <= 2, "expected <= 2 slices, got {slices:?}");
    assert_eq!(slices.len(), 2);
    assert_eq!((slices[0].start_line, slices[0].end_line), (1, 20));
    assert_eq!((slices[1].start_line, slices[1].end_line), (100, 110));

    // 10 disjoint ranges with n_max = 12: capped at MAX_PAPER_READER_SLICES (8).
    let ten_ranges: Vec<(usize, usize)> = (0..10).map(|i| (i * 25 + 1, i * 25 + 5)).collect();
    let capped = plan_slices_for_ranges(&text, &[], None, &ten_ranges, 12);
    assert!(
        capped.len() <= 8,
        "expected <= 8 slices (MAX_PAPER_READER_SLICES), got {}",
        capped.len()
    );
}
