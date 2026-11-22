//! Integration tests for custom subagents, built-in agents metadata, the in-app Subagent Builder
//! draft endpoint (`POST /api/subagents/draft`), and reasoning_effort + subagent_id turn plumbing.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::start_desktop_server;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

struct CapturingHttp {
    base_url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CapturingHttp {
    fn start(replies: Vec<(u16, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let b_clone = Arc::clone(&bodies);
        let handle = thread::spawn(move || {
            for (status, reply_body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(&mut stream);
                let mut content_len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                if content_len > 0 {
                    let mut buf = vec![0u8; content_len];
                    if reader.read_exact(&mut buf).is_ok() {
                        if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                            b_clone.lock().unwrap().push(v);
                        }
                    }
                }
                let hdr = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply_body.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                let _ = stream.write_all(&reply_body);
                let _ = stream.flush();
            }
        });
        Self { base_url: format!("http://{addr}"), bodies, handle: Some(handle) }
    }

    fn finish(mut self) -> Vec<Value> {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.bodies.lock().unwrap().clone()
    }
}

fn anthropic_text_body(id: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-6",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 10, "output_tokens": 20 }
    }))
    .unwrap()
}

#[test]
fn subagents_crud_and_builder_draft_and_turn_wiring() {
    let builder_draft_json = json!({
        "name": "QFT Loop Checker",
        "description": "Checks dimensional regularization poles and counterterms with SymPy.",
        "tools_profile": "compute",
        "reasoning_effort": "high",
        "rigor": "deep",
        "system_prompt": "Always verify UV pole cancellations in d = 4 - 2*epsilon using SymPy."
    })
    .to_string();

    let mock = CapturingHttp::start(vec![
        (200, anthropic_text_body("msg_draft", &format!("```json\n{builder_draft_json}\n```"))),
        (200, anthropic_text_body("msg_turn1", "Solved with subagent and effort.")),
    ]);

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let mut cfg = test_cfg(&data_dir, None, Some(mock.base_url.clone()), None);
    cfg.connection_env = Some(BTreeMap::from([(
        "ANTHROPIC_API_KEY".to_string(),
        "sk-ant-test-subagent-key".to_string(),
    )]));
    let srv = start_desktop_server(cfg).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();

    // Also save Anthropic key in providers.json because `state.build_transport` reads process env / KeyStore.
    let key_payload = serde_json::to_vec(&json!({ "keys": { "anthropic": "sk-ant-test-subagent-key" } })).unwrap();
    let (st, _, _) = raw_http(port, "PUT", "/api/settings/providers", &[("X-ReSearch-Token", &tok)], &key_payload);
    assert_eq!(st, 200);

    // 1. GET /api/subagents lists built-in roles (including solver & subagent-builder).
    let (st, _, body) = raw_http(port, "GET", "/api/subagents", &[("X-ReSearch-Token", &tok)], b"");
    assert_eq!(st, 200);
    let list: Value = serde_json::from_slice(&body).unwrap();
    let builtins = list["builtin"].as_array().unwrap();
    assert!(builtins.iter().any(|b| b["id"] == "solver"));
    assert!(builtins.iter().any(|b| b["id"] == "full-verifier"));
    assert!(builtins.iter().any(|b| b["id"] == "subagent-builder"));
    assert_eq!(list["custom"].as_array().unwrap().len(), 0);

    // 2. Reject deleting or overwriting reserved built-in IDs.
    let (st, _, _) = raw_http(port, "DELETE", "/api/subagents/solver", &[("X-ReSearch-Token", &tok)], b"");
    assert_eq!(st, 400);
    let reserved = serde_json::to_vec(&json!({
        "id": "solver",
        "name": "Fake Solver",
        "system_prompt": "override"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/subagents", &[("X-ReSearch-Token", &tok)], &reserved);
    assert_eq!(st, 400);

    // 3. POST /api/subagents/draft invokes the in-app Subagent Builder Agent.
    let draft_req = serde_json::to_vec(&json!({
        "goal": "Audit 1-loop dimensional regularization integrals in QFT",
        "provider": "anthropic",
        "model": "claude-sonnet-4-6"
    }))
    .unwrap();
    let (st, _, draft_body) = raw_http(port, "POST", "/api/subagents/draft", &[("X-ReSearch-Token", &tok)], &draft_req);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&draft_body));
    let draft_val: Value = serde_json::from_slice(&draft_body).unwrap();
    assert_eq!(draft_val["draft"]["name"], "QFT Loop Checker");
    assert_eq!(draft_val["draft"]["tools_profile"], "compute");
    assert_eq!(draft_val["draft"]["reasoning_effort"], "high");

    // 4. Save the drafted custom subagent via POST /api/subagents.
    let (st, _, save_body) = raw_http(
        port,
        "POST",
        "/api/subagents",
        &[("X-ReSearch-Token", &tok)],
        serde_json::to_vec(&draft_val["draft"]).unwrap().as_slice(),
    );
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&save_body));
    let saved: Value = serde_json::from_slice(&save_body).unwrap();
    let sa_id = saved["subagent"]["id"].as_str().unwrap().to_string();
    assert_eq!(sa_id, "qft-loop-checker");

    // 5. Run a Direct session turn specifying reasoning_effort + subagent_id; wait for turn_complete.
    let s_req = serde_json::to_vec(&json!({ "title": "subagent test" })).unwrap();
    let (st, _, s_body) = raw_http(port, "POST", "/api/sessions", &[("X-ReSearch-Token", &tok)], &s_req);
    assert_eq!(st, 200);
    let sid = serde_json::from_slice::<Value>(&s_body).unwrap()["id"].as_str().unwrap().to_string();

    let msg_req = serde_json::to_vec(&json!({
        "prompt": "Check UV divergence",
        "provider": "anthropic",
        "model": "claude-sonnet-4-6",
        "reasoning_effort": "max",
        "subagent_id": sa_id,
        "verification_mode": false
    }))
    .unwrap();
    let (st, _, msg_body) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &[("X-ReSearch-Token", &tok)], &msg_req);
    assert_eq!(st, 202, "{}", String::from_utf8_lossy(&msg_body));

    // Poll GET /api/sessions/{sid} until `running` becomes false.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (st, _, sdata) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &[("X-ReSearch-Token", &tok)], b"");
        assert_eq!(st, 200);
        let v: Value = serde_json::from_slice(&sdata).unwrap();
        if v["running"] == json!(false) && !v["messages"].as_array().unwrap().is_empty() {
            assert_eq!(v["meta"]["reasoning_effort"], "max");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for turn");
        thread::sleep(Duration::from_millis(25));
    }

    // 6. Verify mock HTTP requests received both the Subagent system prompt & Anthropic `output_config.effort`.
    let captured = mock.finish();
    assert_eq!(captured.len(), 2);
    let turn_req = &captured[1];
    assert_eq!(turn_req["output_config"], json!({ "effort": "max" }));
    assert!(turn_req["system"].as_str().unwrap().contains("QFT Loop Checker"));
    assert!(turn_req["system"].as_str().unwrap().contains("UV pole cancellations"));

    // 7. Delete custom subagent via DELETE /api/subagents/{id}.
    let (st, _, _) = raw_http(port, "DELETE", &format!("/api/subagents/{sa_id}"), &[("X-ReSearch-Token", &tok)], b"");
    assert_eq!(st, 200);
    let (_, _, list2) = raw_http(port, "GET", "/api/subagents", &[("X-ReSearch-Token", &tok)], b"");
    let v2: Value = serde_json::from_slice(&list2).unwrap();
    assert_eq!(v2["custom"].as_array().unwrap().len(), 0);

    srv.shutdown();
}
