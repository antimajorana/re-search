//! Stage 14 integration tests:
//! - Worker delegation (`delegate_compute`, `delegate_literature`, `delegate_lean`,
//!   `delegate_explore`, `delegate_subagent`) on separate smaller worker models + resume
//! - Capability whitelist and `final_answer` worker guards
//! - Parallel Ultramath exploration (`explore.*`), honesty linter (`honesty.lint`), and session stats
//! - Lean 4 anti-cheat endpoint (`POST /api/sessions/{id}/lean/check`) and custom subagent delegation

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::{Event, Ledger};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

impl EnvGuard {
    fn clear_providers() -> Self {
        let mut vars = vec![("RE_SEARCH_API_KEY", None), ("RESEARCH_CORE_API_KEY", None)];
        for p in [agent::ProviderKind::Anthropic, agent::ProviderKind::OpenAi, agent::ProviderKind::Gemini, agent::ProviderKind::Ollama] {
            vars.extend(p.key_env_vars().iter().map(|&k| (k, None)));
        }
        Self::set(&vars)
    }

    fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
        let saved = vars.iter().map(|&(k, v)| {
            let prev = std::env::var_os(k);
            match v { Some(val) => std::env::set_var(k, val), None => std::env::remove_var(k) }
            (k, prev)
        }).collect();
        Self(saved)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, prev) in self.0.drain(..).rev() {
            match prev { Some(v) => std::env::set_var(k, v), None => std::env::remove_var(k) }
        }
    }
}

type CapturedHandler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct ScriptedMock {
    base_url: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

impl ScriptedMock {
    fn start(handler: CapturedHandler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (seen_cl, handler) = (Arc::clone(&seen), Arc::new(handler));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (handler, seen_inner) = (Arc::clone(&handler), Arc::clone(&seen_cl));
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut reader = BufReader::new(stream);
                    let mut req_line = String::new();
                    if reader.read_line(&mut req_line).unwrap_or(0) == 0 { return; }
                    let path = req_line.split_whitespace().nth(1).unwrap_or("").split('?').next().unwrap_or("").to_string();
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
                    seen_inner.lock().unwrap().push((path.clone(), body.clone()));
                    let bytes = serde_json::to_vec(&handler(&path, &body)).unwrap();
                    let mut s = reader.into_inner();
                    let hdr = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
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

fn anthropic_text(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_t", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 14, "output_tokens": 22 }
    })
}

fn anthropic_tool(model: &str, id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": "msg_u", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 14, "output_tokens": 22 }
    })
}

fn openai_text(model: &str, text: &str) -> Value {
    json!({
        "id": "chatcmpl_t", "model": model,
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 18, "completion_tokens": 26 }
    })
}

fn openai_tool(model: &str, id: &str, name: &str, args: Value) -> Value {
    json!({
        "id": "chatcmpl_u", "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": null, "tool_calls": [{ "id": id, "type": "function", "function": { "name": name, "arguments": args.to_string() } }] },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 18, "completion_tokens": 26 }
    })
}

fn has_tool(body: &Value, name: &str) -> bool {
    body["tools"].as_array().is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name))
}

fn openai_tool_count(body: &Value) -> usize {
    body["messages"].as_array().map(|a| a.iter().filter(|m| m["role"] == "tool").count()).unwrap_or(0)
}

fn anthropic_tool_count(body: &Value) -> usize {
    body["messages"].as_array().map(|a| {
        a.iter().filter_map(|m| m["content"].as_array()).flatten().filter(|b| b["type"] == "tool_result").count()
    }).unwrap_or(0)
}

fn last_user_text(body: &Value) -> String {
    body["messages"].as_array().and_then(|a| a.iter().rev().find(|m| m["role"] == "user")).map(|m| {
        m["content"].as_str().map(str::to_string)
            .or_else(|| m["content"].as_array().map(|bs| bs.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")))
            .unwrap_or_default()
    }).unwrap_or_default()
}

fn has_ant_tool_result(cap: &[(String, Value)], id: &str, is_err: bool, needle: &str) -> bool {
    cap.iter().filter(|(p, _)| p == "/v1/messages")
        .flat_map(|(_, b)| b["messages"].as_array().into_iter().flatten())
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .any(|r| {
            r["type"] == "tool_result" && r["tool_use_id"] == id
                && r["is_error"] == json!(is_err)
                && r["content"].as_str().is_some_and(|c| c.contains(needle))
        })
}

struct TestClient { port: u16, tok: String }

impl TestClient {
    fn new(srv: &RunningDesktopServer) -> Self { Self { port: srv.addr.port(), tok: srv.info.token.clone() } }

    fn req(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
        let hdr = [("X-ReSearch-Token", self.tok.as_str()), ("Content-Type", "application/json")];
        let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
        let (st, _, raw) = raw_http(self.port, method, path, &hdr, &bytes);
        (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
    }

    fn put_settings(&self, payload: Value) {
        let (st, _, raw) = self.req("PUT", "/api/settings/providers", Some(&payload));
        assert_eq!(st, 200, "PUT settings failed: {raw}");
    }

    fn create_session(&self, title: &str, ws: Option<&Path>) -> String {
        let mut payload = json!({ "title": title });
        if let Some(dir) = ws { payload["workspace_root"] = json!(dir); }
        let (st, v, raw) = self.req("POST", "/api/sessions", Some(&payload));
        assert_eq!(st, 200, "create_session failed: {raw}");
        v["id"].as_str().unwrap().to_string()
    }

    fn send_and_wait(&self, sid: &str, body: Value, expected_msgs: usize) -> Value {
        let (st, _, raw) = self.req("POST", &format!("/api/sessions/{sid}/messages"), Some(&body));
        assert_eq!(st, 202, "POST messages failed: {raw}");
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let (gs, sv, graw) = self.req("GET", &format!("/api/sessions/{sid}"), None);
            assert_eq!(gs, 200, "GET session failed: {graw}");
            if let Some(abort) = sv["events"].as_array().and_then(|evs| evs.iter().rev().find(|e| e["kind"] == "session.abort")) {
                panic!("turn aborted unexpectedly: {abort}");
            }
            if sv["running"] == json!(false) && sv["messages"].as_array().is_some_and(|m| m.len() >= expected_msgs) {
                return sv;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {expected_msgs} messages: {sv}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn verify_ledger_clean(data_dir: &Path, sid: &str) -> (Ledger, Vec<Event>) {
    let ledger = Ledger::open(data_dir).unwrap();
    ledger.verify_session(sid).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    let evs = ledger.read_session(sid).unwrap();
    (ledger, evs)
}

#[test]
fn compute_worker_separate_smaller_model_and_resume() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();

    let mock = ScriptedMock::start(Box::new(|path, body| {
        if path == "/v1/messages" {
            match anthropic_tool_count(body) {
                0 => anthropic_tool("claude-haiku-4-5", "tu_cw1", "check_expression_sanity", json!({ "lhs": "(x+1)^2", "rhs": "x^2 + 2*x + 1", "variables": ["x"] })),
                1 => anthropic_tool("claude-haiku-4-5", "tu_cw2", "ledger_append", json!({ "entry_type": "symbolic_computation", "summary": "Expanded (x+1)^2", "details": "Verified sample points." })),
                _ => anthropic_text("claude-haiku-4-5", "Checked identity and logged e-1."),
            }
        } else if last_user_text(body).contains("Evaluate at x = 3") {
            openai_text("gpt-4.1", "At x = 3, both sides equal 16.")
        } else {
            match openai_tool_count(body) {
                0 => openai_tool("gpt-4.1", "call_dc1", "delegate_compute", json!({ "task": "Verify (x+1)^2 = x^2 + 2*x + 1", "record_proof_entry": true })),
                1 => openai_tool("gpt-4.1", "call_la1", "ledger_append", json!({ "entry_type": "result", "summary": "Identity holds", "details": "Confirmed from e-1.", "depends_on": ["e-1"] })),
                _ => openai_text("gpt-4.1", "Identity confirmed via e-1 and e-2."),
            }
        }
    }));

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("providers.json"), serde_json::to_vec(&json!({
        "providers": {
            "openai": { "api_key": "sk-oai-stage14-t1", "base_url": mock.base_url },
            "anthropic": { "api_key": "sk-ant-stage14-t1", "base_url": mock.base_url }
        }
    })).unwrap()).unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    let client = TestClient::new(&srv);
    client.put_settings(json!({ "agent_models": { "compute_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" } } }));

    let sid = client.create_session("Stage14 Compute Worker", None);
    let sv1 = client.send_and_wait(&sid, json!({ "prompt": "Check (x+1)^2 = x^2 + 2*x + 1", "provider": "openai", "model": "gpt-4.1", "verification_mode": false }), 2);
    assert!(sv1["stats"]["delegation_count"].as_u64().unwrap_or(0) >= 1, "{sv1}");
    assert!(sv1["stats"]["worker_input_tokens"].as_u64().unwrap_or(0) > 0, "{sv1}");
    assert!(sv1["stats"]["worker_output_tokens"].as_u64().unwrap_or(0) > 0, "{sv1}");
    assert_eq!(sv1["stats"]["campaign_phase"], "COMPLETE", "{sv1}");

    let (ledger, evs) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs.iter().any(|e| e.kind == "delegate.start" && e.payload["role"] == "compute_worker"));
    let d_turns: Vec<_> = evs.iter().filter(|e| e.kind == "delegate.turn").collect();
    assert!(!d_turns.is_empty());
    for dt in d_turns { assert!(!ledger.get_blob(dt.payload["raw_blob"].as_str().unwrap()).unwrap().is_empty()); }
    assert!(evs.iter().any(|e| {
        e.kind == "delegate.end" && e.payload["worker"] == "compute_worker" && e.payload["model"] == "claude-haiku-4-5"
            && e.payload["proof_entries"].as_array().is_some_and(|a| a.iter().any(|x| x == "e-1"))
    }));
    assert!(evs.iter().any(|e| {
        e.kind == "tool.request" && e.payload["caller"].as_str().is_some_and(|c| {
            c == format!("worker:compute_worker:anthropic-messages:{sid}") || c == format!("worker:compute_worker:anthropic:{sid}")
        })
    }));

    let sv2 = client.send_and_wait(&sid, json!({ "prompt": "Evaluate at x = 3", "provider": "openai", "model": "gpt-4.1", "verification_mode": false }), 4);
    assert!(sv2["messages"][3]["text"].as_str().unwrap_or("").contains("16"));
    verify_ledger_clean(&data_dir, &sid);

    let cap = mock.captured();
    let ant: Vec<_> = cap.iter().filter(|(p, _)| p == "/v1/messages").collect();
    let oai: Vec<_> = cap.iter().filter(|(p, _)| p == "/v1/chat/completions").collect();
    assert_eq!(ant.len(), 3);
    assert!(ant.iter().all(|(_, b)| b["model"] == "claude-haiku-4-5"));
    assert_eq!(oai.len(), 4);
    assert!(oai.iter().all(|(_, b)| b["model"] == "gpt-4.1"));
}

#[test]
fn literature_worker_and_capability_and_final_answer_guards() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();

    let mock = ScriptedMock::start(Box::new(|path, body| {
        if path == "/v1/messages" {
            if has_tool(body, "fs_read") {
                match anthropic_tool_count(body) {
                    0 => anthropic_tool("claude-haiku-4-5", "tu_lit_deny", "sympy_eval", json!({ "code": "1+1" })),
                    1 => anthropic_tool("claude-haiku-4-5", "tu_lit_read", "fs_read", json!({ "path": "paper.tex" })),
                    _ => anthropic_text("claude-haiku-4-5", "Read paper.tex: Theorem 1 holds for N >= 2."),
                }
            } else if anthropic_tool_count(body) == 0 {
                anthropic_tool("claude-haiku-4-5", "tu_comp_fa", "ledger_append", json!({ "entry_type": "final_answer", "summary": "Sneaky", "details": "Forbidden" }))
            } else {
                anthropic_text("claude-haiku-4-5", "Respecting final_answer guard.")
            }
        } else {
            match openai_tool_count(body) {
                0 => openai_tool("gpt-4.1", "call_dl", "delegate_literature", json!({ "task": "Read paper.tex" })),
                1 => openai_tool("gpt-4.1", "call_dc", "delegate_compute", json!({ "task": "Attempt compute summary" })),
                _ => openai_text("gpt-4.1", "Completed worker guard checks cleanly."),
            }
        }
    }));

    let tmp = TempDir::new().unwrap();
    let (data_dir, ws_dir) = (tmp.path().join("data"), tmp.path().join("ws"));
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("paper.tex"), "\\section{Main Theorem}\nTheorem 1 holds for N >= 2.\n").unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws_dir.clone()), Some(mock.base_url.clone()), None)).unwrap();
    let client = TestClient::new(&srv);
    client.put_settings(json!({
        "keys": { "openai": "sk-oai-stage14-t2", "anthropic": "sk-ant-stage14-t2" },
        "agent_models": {
            "literature_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" },
            "compute_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" }
        }
    }));

    let sid = client.create_session("Stage14 Worker Guards", Some(&ws_dir));
    client.send_and_wait(&sid, json!({ "prompt": "Inspect paper.tex via workers", "provider": "openai", "model": "gpt-4.1", "verification_mode": false }), 2);

    let (st_dag, dag, dag_raw) = client.req("GET", &format!("/api/sessions/{sid}/dag"), None);
    assert_eq!(st_dag, 200, "{dag_raw}");
    assert!(!dag["entries"].as_array().unwrap().iter().any(|e| e["entry_type"] == "final_answer"), "{dag}");

    let cap = mock.captured();
    assert!(has_ant_tool_result(&cap, "tu_lit_deny", true, "DENIED:"));
    assert!(has_ant_tool_result(&cap, "tu_lit_read", false, "Theorem 1 holds for N >= 2."));
    assert!(has_ant_tool_result(&cap, "tu_comp_fa", true, "DENIED:"));
    assert!(has_ant_tool_result(&cap, "tu_comp_fa", true, "final_answer"));
    verify_ledger_clean(&data_dir, &sid);
}

#[test]
fn ultramath_parallel_explore_and_honesty_linter() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();

    let (in_flight, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (in_cl, peak_cl) = (Arc::clone(&in_flight), Arc::clone(&peak));
    let mock = ScriptedMock::start(Box::new(move |path, body| {
        if path == "/v1/messages" {
            let cur = in_cl.fetch_add(1, Ordering::SeqCst) + 1;
            peak_cl.fetch_max(cur, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(80));
            let out = if last_user_text(body).contains("Jordan-Wigner") {
                "Jordan-Wigner branch: free-fermion spectrum gives gap 2|J - h|."
            } else {
                "Majorana pairing branch: Pfaffian determinant yields matching gap."
            };
            in_cl.fetch_sub(1, Ordering::SeqCst);
            anthropic_text("claude-haiku-4-5", out)
        } else if openai_tool_count(body) == 0 {
            openai_tool("gpt-4.1", "call_exp", "delegate_explore", json!({ "problem": "Compare two gap derivations", "approaches": ["Jordan-Wigner", "Majorana pairing"] }))
        } else {
            openai_text("gpt-4.1", "SymPy confirmed this result and we have rigorously proved it.")
        }
    }));

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    let client = TestClient::new(&srv);
    client.put_settings(json!({
        "keys": { "openai": "sk-oai-stage14-t3", "anthropic": "sk-ant-stage14-t3" },
        "agent_models": { "explorer_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" } }
    }));

    let sid = client.create_session("Stage14 Explore & Honesty", None);
    let sv = client.send_and_wait(&sid, json!({ "prompt": "Compare two gap derivations", "provider": "openai", "model": "gpt-4.1", "verification_mode": false }), 2);
    assert!(peak.load(Ordering::SeqCst) >= 2, "expected concurrent branch calls");

    let (ledger, evs) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs.iter().any(|e| e.kind == "explore.start" && e.payload["branch_count"] == 2));
    let branches: Vec<_> = evs.iter().filter(|e| e.kind == "explore.branch").collect();
    assert_eq!(branches.len(), 2);
    for br in branches { assert!(!ledger.get_blob(br.payload["raw_blob"].as_str().unwrap()).unwrap().is_empty()); }
    assert!(evs.iter().any(|e| e.kind == "explore.end" && e.payload["branch_count"] == 2 && e.payload["worker"] == "explorer_worker"));
    assert!(evs.iter().any(|e| e.kind == "honesty.lint" && e.payload["passed"] == json!(false) && e.payload["findings"].as_array().is_some_and(|f| !f.is_empty())));

    assert!(sv["stats"]["honesty_warnings"].as_u64().unwrap_or(0) >= 1, "{sv}");
    let assistant = sv["messages"].as_array().unwrap().iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant["honesty"]["passed"], json!(false), "{assistant}");
    assert!(!assistant["honesty"]["findings"].as_array().unwrap().is_empty(), "{assistant}");
}

#[test]
fn lean_worker_and_api_endpoint_and_custom_subagent_delegation() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();

    let mock = ScriptedMock::start(Box::new(|path, body| {
        if path == "/v1/messages" {
            if has_tool(body, "lean_check") {
                match anthropic_tool_count(body) {
                    0 => anthropic_tool("claude-haiku-4-5", "tu_lean1", "lean_check", json!({ "code": "theorem bad : False := by sorry" })),
                    _ => anthropic_text("claude-haiku-4-5", "Lean check rejected sorry cleanly."),
                }
            } else {
                assert!(!has_tool(body, "fs_write") && !has_tool(body, "sympy_eval") && !has_tool(body, "delegate_subagent"));
                match anthropic_tool_count(body) {
                    0 => anthropic_tool("claude-haiku-4-5", "tu_sa1", "ledger_query", json!({})),
                    _ => anthropic_text("claude-haiku-4-5", "Read-only specialist finished audit."),
                }
            }
        } else {
            match openai_tool_count(body) {
                0 => openai_tool("gpt-4.1", "call_lean", "delegate_lean", json!({ "task": "Verify lemma in Lean" })),
                1 => openai_tool("gpt-4.1", "call_sa", "delegate_subagent", json!({ "subagent_id": "tex-proof-auditor", "task": "Audit session ledger read-only" })),
                _ => openai_text("gpt-4.1", "Completed Lean check and custom subagent delegation."),
            }
        }
    }));

    let tmp = TempDir::new().unwrap();
    let (data_dir, ws_dir) = (tmp.path().join("data"), tmp.path().join("ws"));
    std::fs::create_dir_all(&ws_dir).unwrap();
    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws_dir.clone()), Some(mock.base_url.clone()), None)).unwrap();
    let client = TestClient::new(&srv);

    let (st_sub, sub_v, sub_raw) = client.req("GET", "/api/subagents", None);
    assert_eq!(st_sub, 200, "{sub_raw}");
    let builtins = sub_v["builtin"].as_array().unwrap();
    for (id, rk) in [("compute-worker", "compute_worker"), ("literature-worker", "literature_worker"), ("lean-worker", "lean_worker"), ("explorer-worker", "explorer_worker")] {
        assert!(builtins.iter().any(|b| b["id"] == id && b["role_key"] == rk && b["category"] == "worker"), "{sub_v}");
    }
    for role in ["compute_worker", "literature_worker", "lean_worker", "explorer_worker"] {
        assert_eq!(client.req("PUT", "/api/settings/providers", Some(&json!({ "agent_models": { role: { "provider": "claude-cli", "model": "sonnet" } } }))).0, 400);
    }
    client.put_settings(json!({
        "keys": { "openai": "sk-oai-stage14-t4", "anthropic": "sk-ant-stage14-t4" },
        "agent_models": {
            "compute_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" },
            "literature_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" },
            "lean_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" },
            "explorer_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" }
        }
    }));

    let (st_cs, cs_v, cs_raw) = client.req("POST", "/api/subagents", Some(&json!({
        "name": "Tex Proof Auditor", "description": "Strict read-only TeX and ledger auditor",
        "tools_profile": "read_only", "provider": "anthropic", "model": "claude-haiku-4-5", "system_prompt": "Inspect proofs read-only."
    })));
    assert_eq!(st_cs, 201, "{cs_raw}");
    let sa_id = cs_v["subagent"]["id"].as_str().unwrap().to_string();

    let sid = client.create_session("Stage14 Lean & Custom", Some(&ws_dir));
    let (st_sorry, sorry_v, sorry_raw) = client.req("POST", &format!("/api/sessions/{sid}/lean/check"), Some(&json!({ "code": "theorem bad : False := by sorry" })));
    assert_eq!(st_sorry, 200, "{sorry_raw}");
    assert_eq!((sorry_v["ok"].as_bool(), sorry_v["status"].as_str()), (Some(false), Some("rejected_sorry")));

    let lean_evs = Ledger::open(&data_dir).unwrap().read_session(&sid).unwrap();
    let lean_cid = lean_evs.iter().find(|e| e.kind == "tool.request" && e.payload["tool"] == "lean_check" && e.payload["caller"] == "user:desktop-lean").unwrap().payload["call_id"].as_str().unwrap().to_string();
    assert!(lean_evs.iter().any(|e| e.kind == "tool.result" && e.payload["call_id"] == lean_cid));

    {
        let missing = tmp.path().join("missing_lean_bin_stage14").to_string_lossy().into_owned();
        let _lean_guard = EnvGuard::set(&[("RE_SEARCH_LEAN", Some(missing.as_str()))]);
        let (st_miss, miss_v, miss_raw) = client.req("POST", &format!("/api/sessions/{sid}/lean/check"), Some(&json!({ "code": "theorem ok : True := trivial" })));
        assert_eq!(st_miss, 200, "{miss_raw}");
        assert_eq!((miss_v["ok"].as_bool(), miss_v["status"].as_str()), (Some(false), Some("lean_not_found")));
    }

    client.send_and_wait(&sid, json!({ "prompt": "Check Lean and audit ledger", "provider": "openai", "model": "gpt-4.1", "verification_mode": false }), 2);

    let cap = mock.captured();
    let oai_req = cap.iter().find(|(p, _)| p == "/v1/chat/completions").map(|(_, b)| b).unwrap();
    let sub_tool = oai_req["tools"].as_array().unwrap().iter().find(|t| t["function"]["name"] == "delegate_subagent").expect("delegate_subagent");
    assert!(sub_tool["function"]["description"].as_str().unwrap_or("").contains(&sa_id));

    let ant_msgs: Vec<_> = cap.iter().filter(|(p, _)| p == "/v1/messages").map(|(_, b)| b).collect();
    assert!(ant_msgs.iter().all(|b| b["model"] == "claude-haiku-4-5"));
    assert!(has_ant_tool_result(&cap, "tu_lean1", false, "rejected_sorry"));

    let (_, final_evs) = verify_ledger_clean(&data_dir, &sid);
    assert!(final_evs.iter().any(|e| e.kind == "delegate.end" && e.payload["worker"] == "lean_worker"));
    assert!(final_evs.iter().any(|e| e.kind == "delegate.end" && e.payload["worker"] == format!("subagent:{sa_id}")));
}
