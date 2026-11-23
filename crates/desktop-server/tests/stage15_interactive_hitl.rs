//! Stage 15 integration tests for Interactive Human-in-the-Loop (HITL) control:
//! - Live mid-turn steering (`POST /api/sessions/{id}/steer`, SSE `steer_queued`, `<human_steer>`, resume)
//! - `ask_user` checkpoints, `interactive_plan_mode`, `pending_checkpoints`, and `stop` cancellation
//! - Ultramath `pause_after_explore` interactive branch picker + worker tool isolation
//! - Explain promotion (`POST /api/sessions/{id}/explain/promote` to conventions or steer/prompt)

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::{Event, Ledger};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

impl EnvGuard {
    fn clear_providers() -> Self {
        let mut vars: Vec<(&'static str, Option<&'static str>)> = vec![("RE_SEARCH_API_KEY", None), ("RESEARCH_CORE_API_KEY", None)];
        for p in [agent::ProviderKind::Anthropic, agent::ProviderKind::OpenAi, agent::ProviderKind::Gemini, agent::ProviderKind::Ollama] {
            vars.extend(p.key_env_vars().iter().map(|&k| (k, None)));
        }
        Self(vars.iter().map(|&(k, v)| {
            let prev = std::env::var_os(k);
            match v { Some(val) => std::env::set_var(k, val), None => std::env::remove_var(k) }
            (k, prev)
        }).collect())
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, prev) in self.0.drain(..).rev() {
            match prev { Some(v) => std::env::set_var(k, v), None => std::env::remove_var(k) }
        }
    }
}

#[derive(Default)]
struct Flag(Mutex<bool>, Condvar);

impl Flag {
    fn pair() -> (Arc<Self>, Arc<Self>) { let f = Arc::new(Self::default()); (Arc::clone(&f), f) }
    fn signal(&self) { *self.0.lock().unwrap() = true; self.1.notify_all(); }
    fn wait(&self, label: &str) {
        let mut g = self.0.lock().unwrap();
        let dl = Instant::now() + Duration::from_secs(10);
        while !*g {
            let rem = dl.saturating_duration_since(Instant::now());
            assert!(!rem.is_zero(), "timed out waiting for `{label}`");
            g = self.1.wait_timeout(g, rem).unwrap().0;
        }
    }
}

type CapturedHandler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct ScriptedMock { base_url: String, seen: Arc<Mutex<Vec<(String, Value)>>> }

impl ScriptedMock {
    fn start(handler: CapturedHandler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (seen_cl, handler) = (Arc::clone(&seen), Arc::new(handler));
        std::thread::spawn(move || for stream in listener.incoming().flatten() {
            let (handler, seen_inner) = (Arc::clone(&handler), Arc::clone(&seen_cl));
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let mut r = BufReader::new(stream);
                let mut req = String::new();
                if r.read_line(&mut req).unwrap_or(0) == 0 { return; }
                let path = req.split_whitespace().nth(1).unwrap_or("").split('?').next().unwrap_or("").to_string();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() { break; }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") { len = v.trim().parse().unwrap_or(0); }
                    }
                }
                let mut buf = vec![0u8; len];
                let _ = r.read_exact(&mut buf);
                let body: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                seen_inner.lock().unwrap().push((path.clone(), body.clone()));
                let bytes = serde_json::to_vec(&handler(&path, &body)).unwrap();
                let mut s = r.into_inner();
                let hdr = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                let _ = s.write_all(hdr.as_bytes()); let _ = s.write_all(&bytes); let _ = s.flush();
            });
        });
        Self { base_url: format!("http://{addr}"), seen }
    }
    fn captured(&self) -> Vec<(String, Value)> { self.seen.lock().unwrap().clone() }
}

fn anthropic_text(m: &str, text: &str) -> Value {
    json!({ "id": "msg_t", "type": "message", "role": "assistant", "model": m, "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }], "usage": { "input_tokens": 14, "output_tokens": 20 } })
}
fn anthropic_tool(m: &str, id: &str, name: &str, input: Value) -> Value {
    json!({ "id": "msg_u", "type": "message", "role": "assistant", "model": m, "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }], "usage": { "input_tokens": 14, "output_tokens": 20 } })
}
fn openai_text(m: &str, text: &str) -> Value {
    json!({ "id": "chatcmpl_t", "model": m, "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }], "usage": { "prompt_tokens": 18, "completion_tokens": 24 } })
}
fn openai_tool(m: &str, id: &str, name: &str, args: Value) -> Value {
    json!({ "id": "chatcmpl_u", "model": m, "choices": [{ "index": 0, "message": { "role": "assistant", "content": null, "tool_calls": [{ "id": id, "type": "function", "function": { "name": name, "arguments": args.to_string() } }] }, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 18, "completion_tokens": 24 } })
}

fn has_tool(b: &Value, name: &str) -> bool { b["tools"].as_array().is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name)) }
fn system_text(b: &Value) -> String {
    b.get("system").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
        b["messages"].as_array().map(|a| a.iter().filter(|m| m["role"] == "system").filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
    })
}
fn msg_text(m: &Value) -> String {
    m["content"].as_str().map(str::to_string).or_else(|| m["content"].as_array().map(|bs| bs.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"))).unwrap_or_default()
}
fn all_user_text(b: &Value) -> String { b["messages"].as_array().map(|a| a.iter().filter(|m| m["role"] == "user").map(msg_text).collect::<Vec<_>>().join("\n")).unwrap_or_default() }
fn last_user_text(b: &Value) -> String { b["messages"].as_array().and_then(|a| a.iter().rev().find(|m| m["role"] == "user")).map(msg_text).unwrap_or_default() }
fn anthropic_tool_count(b: &Value) -> usize { b["messages"].as_array().map(|a| a.iter().filter_map(|m| m["content"].as_array()).flatten().filter(|x| x["type"] == "tool_result").count()).unwrap_or(0) }
fn openai_tool_count(b: &Value) -> usize { b["messages"].as_array().map(|a| a.iter().filter(|m| m["role"] == "tool").count()).unwrap_or(0) }
fn ant_tool_result_text(b: &Value, id: &str) -> Option<String> {
    b["messages"].as_array()?.iter().filter_map(|m| m["content"].as_array()).flatten()
        .find(|x| x["type"] == "tool_result" && x["tool_use_id"] == id).and_then(|x| x["content"].as_str().map(str::to_string))
}
fn openai_tool_result_json(b: &Value, id: &str) -> Option<Value> {
    b["messages"].as_array()?.iter().find(|m| m["role"] == "tool" && m["tool_call_id"] == id)
        .and_then(|m| m["content"].as_str()).and_then(|s| serde_json::from_str(s).ok())
}

struct SseStream { reader: BufReader<TcpStream>, seen: Vec<(String, Value)> }

impl SseStream {
    fn connect(port: u16, cookie: &str, sid: &str) -> Self {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        write!(stream, "GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("200"), "SSE connect failed: {line}");
        loop { line.clear(); assert!(reader.read_line(&mut line).unwrap() > 0, "EOF in SSE"); if line.trim() == ": connected" { break; } }
        Self { reader, seen: Vec::new() }
    }
    fn len(&self) -> usize { self.seen.len() }
    fn next_frame(&mut self) -> Option<(String, Value)> {
        let (mut kind, mut data) = (String::new(), Vec::new());
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line).ok()? == 0 { return None; }
            let t = line.trim_end_matches(['\r', '\n']);
            if t.is_empty() {
                if !kind.is_empty() || !data.is_empty() {
                    let raw = data.join("\n");
                    let ev = (kind, serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "raw": raw })));
                    self.seen.push(ev.clone());
                    return Some(ev);
                }
                continue;
            }
            if t.starts_with(':') { continue; }
            if let Some(rest) = t.strip_prefix("event:") { kind = rest.trim().to_string(); }
            else if let Some(rest) = t.strip_prefix("data:") { data.push(rest.trim_start().to_string()); }
        }
    }
    fn wait_event_since(&mut self, from: usize, want: &str, timeout: Duration) -> Value {
        if let Some((_, v)) = self.seen[from..].iter().find(|(k, _)| k == want) { return v.clone(); }
        let dl = Instant::now() + timeout;
        while Instant::now() < dl {
            let Some((k, v)) = self.next_frame() else { break; };
            if k == want { return v; }
            if want == "turn_complete" && k == "turn_error" { panic!("unexpected turn_error: {v}"); }
        }
        panic!("timed out waiting for SSE `{want}` (seen from {from}: {:?})", &self.seen[from..]);
    }
}

struct TestClient { port: u16, tok: String, cookie: String }

impl TestClient {
    fn new(srv: &RunningDesktopServer) -> Self {
        let (port, tok) = (srv.addr.port(), srv.info.token.clone());
        Self { port, tok: tok.clone(), cookie: launch_cookie(port, &tok) }
    }
    fn req(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
        let hdr = [("X-ReSearch-Token", self.tok.as_str()), ("Content-Type", "application/json")];
        let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
        let (st, _, raw) = raw_http(self.port, method, path, &hdr, &bytes);
        (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
    }
    fn sse(&self, sid: &str) -> SseStream { SseStream::connect(self.port, &self.cookie, sid) }
    fn create_session(&self, title: &str) -> String {
        let (st, v, raw) = self.req("POST", "/api/sessions", Some(&json!({ "title": title })));
        assert_eq!(st, 200, "create_session failed: {raw}");
        v["id"].as_str().unwrap().to_string()
    }
    fn post_message(&self, sid: &str, body: Value) {
        let (st, _, raw) = self.req("POST", &format!("/api/sessions/{sid}/messages"), Some(&body));
        assert_eq!(st, 202, "POST messages failed: {raw}");
    }
    fn get_session(&self, sid: &str) -> Value {
        let (st, v, raw) = self.req("GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200, "GET session failed: {raw}");
        v
    }
    fn wait_idle(&self, sid: &str) -> Value {
        let dl = Instant::now() + Duration::from_secs(10);
        loop {
            let sv = self.get_session(sid);
            if sv["running"] == json!(false) { return sv; }
            assert!(Instant::now() < dl, "timed out waiting for idle session: {sv}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn setup_server(mock_url: &str, settings: Value) -> (TempDir, PathBuf, RunningDesktopServer, TestClient) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock_url.to_string()), None)).unwrap();
    let client = TestClient::new(&srv);
    let (st, _, raw) = client.req("PUT", "/api/settings/providers", Some(&settings));
    assert_eq!(st, 200, "PUT settings failed: {raw}");
    (tmp, data_dir, srv, client)
}

fn verify_ledger_clean(data_dir: &Path, sid: &str) -> (Ledger, Vec<Event>) {
    let ledger = Ledger::open(data_dir).unwrap();
    ledger.verify_session(sid).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    let evs = ledger.read_session(sid).unwrap();
    (ledger, evs)
}

#[test]
fn mid_turn_live_steering_injects_into_next_turn_and_resumes() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();
    let (req1_arrived, req1_cl) = Flag::pair();
    let (steer_posted, steer_cl) = Flag::pair();

    let mock = ScriptedMock::start(Box::new(move |_path, body| {
        if all_user_text(body).contains("Continue with dispersion relation") {
            anthropic_text("claude-haiku-4-5", "Dispersion E(k) = -2t cos(ka) under PBC.")
        } else if anthropic_tool_count(body) == 0 {
            req1_cl.signal();
            steer_cl.wait("steer_posted");
            anthropic_tool("claude-haiku-4-5", "tu_s1", "ledger_note", json!({ "note": "Setup tight-binding chain" }))
        } else {
            anthropic_text("claude-haiku-4-5", "Applied PBC steer and finished Turn 1.")
        }
    }));

    let (_tmp, data_dir, srv, client) = setup_server(&mock.base_url, json!({ "keys": { "anthropic": "sk-ant-s15-t1" } }));
    let sid = client.create_session("Stage15 Live Steer");
    assert_eq!(client.req("POST", &format!("/api/sessions/{sid}/steer"), Some(&json!({ "text": "Too early" }))).0, 409);

    let mut sse = client.sse(&sid);
    client.post_message(&sid, json!({ "prompt": "Compute tight-binding spectrum", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    req1_arrived.wait("req1_arrived");

    let (st_steer, steer_v, steer_raw) = client.req("POST", &format!("/api/sessions/{sid}/steer"), Some(&json!({ "text": "Use periodic boundary conditions" })));
    assert_eq!(st_steer, 202, "{steer_raw}");
    assert_eq!(steer_v["ok"], json!(true));
    let steer_id = steer_v["steer"]["steer_id"].as_str().expect("steer_id").to_string();
    assert_eq!(steer_v["steer"]["text"], "Use periodic boundary conditions");
    assert!(steer_v["steer"]["ts_ms"].is_i64());

    steer_posted.signal();
    let queued = sse.wait_event_since(0, "steer_queued", Duration::from_secs(10));
    let queued_s = serde_json::to_string(&queued).unwrap();
    assert!(queued_s.contains(&steer_id) && queued_s.contains("Use periodic boundary conditions"), "{queued}");
    sse.wait_event_since(0, "turn_complete", Duration::from_secs(10));
    client.wait_idle(&sid);

    let (_, evs1) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs1.iter().any(|e| e.kind == "human.steer" && e.payload["steer_id"] == steer_id && e.payload["text"] == "Use periodic boundary conditions" && e.payload["ts_ms"].is_i64()));

    let mark2 = sse.len();
    client.post_message(&sid, json!({ "prompt": "Continue with dispersion relation", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    sse.wait_event_since(mark2, "turn_complete", Duration::from_secs(10));
    let sv2 = client.wait_idle(&sid);
    assert!(sv2["messages"].as_array().unwrap().len() >= 4, "{sv2}");
    verify_ledger_clean(&data_dir, &sid);

    let cap = mock.captured();
    assert_eq!(cap.len(), 3);
    assert!(all_user_text(&cap[1].1).contains("<human_steer>\nUse periodic boundary conditions\n</human_steer>"));
    assert!(all_user_text(&cap[2].1).contains("<human_steer>\nUse periodic boundary conditions\n</human_steer>"));
    srv.shutdown();
}

#[test]
fn ask_user_checkpoint_and_interactive_plan_mode_and_stop_unblock() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();

    let mock = ScriptedMock::start(Box::new(|_path, body| {
        if all_user_text(body).contains("Proceed with Turn 2 checkpoint") {
            if ant_tool_result_text(body, "tu_ask2").is_none() {
                anthropic_tool("claude-haiku-4-5", "tu_ask2", "ask_user", json!({ "kind": "decision", "question": "Continue?", "options": ["Yes", "No"] }))
            } else {
                anthropic_text("claude-haiku-4-5", "Unexpected continuation after stop.")
            }
        } else if anthropic_tool_count(body) == 0 {
            anthropic_tool("claude-haiku-4-5", "tu_ask1", "ask_user", json!({ "kind": "plan", "question": "Approve plan?", "options": ["Approve plan", "Adjust"], "context": "1. Diagonalize H\n2. Check k=0" }))
        } else {
            anthropic_text("claude-haiku-4-5", "Plan approved; also checked k=0.")
        }
    }));

    let (_tmp, data_dir, srv, client) = setup_server(&mock.base_url, json!({ "keys": { "anthropic": "sk-ant-s15-t2" } }));
    let sid = client.create_session("Stage15 AskUser & Plan");
    let mut sse = client.sse(&sid);

    client.post_message(&sid, json!({ "prompt": "Solve model interactively", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false, "interactive_plan_mode": true }));
    let cp1 = sse.wait_event_since(0, "checkpoint_requested", Duration::from_secs(10));
    let cid1 = cp1["checkpoint_id"].as_str().expect("checkpoint_id").to_string();
    assert_eq!((cp1["session"].as_str(), cp1["kind"].as_str(), cp1["question"].as_str()), (Some(sid.as_str()), Some("plan"), Some("Approve plan?")));
    assert_eq!(cp1["options"], json!(["Approve plan", "Adjust"]));

    let sv_pend = client.get_session(&sid);
    let pend = sv_pend["pending_checkpoints"].as_array().expect("pending_checkpoints");
    assert_eq!(pend.len(), 1, "{sv_pend}");
    assert_eq!(pend[0]["checkpoint_id"], cid1);
    assert_eq!(client.req("POST", &format!("/api/sessions/{sid}/checkpoints/missing-cid"), Some(&json!({ "selected_option": "X" }))).0, 404);

    let mark_res1 = sse.len();
    let (st_r1, r1_v, r1_raw) = client.req("POST", &format!("/api/sessions/{sid}/checkpoints/{cid1}"), Some(&json!({ "selected_option": "Approve plan", "text": "Also check k=0" })));
    assert_eq!(st_r1, 200, "{r1_raw}");
    assert_eq!((r1_v["ok"].as_bool(), r1_v["reply"]["selected_option"].as_str()), (Some(true), Some("Approve plan")));

    let sse_r1 = sse.wait_event_since(mark_res1, "checkpoint_resolved", Duration::from_secs(10));
    assert!(sse_r1["checkpoint_id"] == cid1 || sse_r1["reply"]["checkpoint_id"] == cid1, "{sse_r1}");
    sse.wait_event_since(mark_res1, "turn_complete", Duration::from_secs(10));
    client.wait_idle(&sid);

    let cap1 = mock.captured();
    let sys1 = system_text(&cap1[0].1);
    assert!(sys1.contains("Interactive Human-in-the-Loop") && sys1.to_lowercase().contains("plan") && sys1.contains("ask_user"), "{sys1}");
    assert!(has_tool(&cap1[0].1, "ask_user"));
    let tr1 = ant_tool_result_text(&cap1[1].1, "tu_ask1").expect("tu_ask1 tool_result");
    assert!(tr1.contains("Approve plan") && tr1.contains("Also check k=0"), "{tr1}");

    let (_, evs1) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs1.iter().any(|e| e.kind == "human.checkpoint_requested" && e.payload["checkpoint_id"] == cid1));
    assert!(evs1.iter().any(|e| e.kind == "human.decision" && e.payload["checkpoint_id"] == cid1 && e.payload["selected_option"] == "Approve plan" && e.payload["text"] == "Also check k=0" && e.payload["cancelled"] == json!(false)));

    let mark2 = sse.len();
    client.post_message(&sid, json!({ "prompt": "Proceed with Turn 2 checkpoint", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    let cp2 = sse.wait_event_since(mark2, "checkpoint_requested", Duration::from_secs(10));
    let cid2 = cp2["checkpoint_id"].as_str().expect("cid2").to_string();
    assert_eq!(cp2["question"], "Continue?");

    let mark_stop = sse.len();
    let t0 = Instant::now();
    let (st_stop, stop_v, stop_raw) = client.req("POST", &format!("/api/sessions/{sid}/stop"), Some(&json!({})));
    assert_eq!(st_stop, 200, "{stop_raw}");
    assert_eq!(stop_v["stopped"], json!(true));
    let res2 = sse.wait_event_since(mark_stop, "checkpoint_resolved", Duration::from_secs(5));
    assert!(res2["checkpoint_id"] == cid2 || res2["reply"]["checkpoint_id"] == cid2, "{res2}");
    assert!(res2["cancelled"] == json!(true) || res2["reply"]["cancelled"] == json!(true), "{res2}");
    sse.wait_event_since(mark_stop, "turn_error", Duration::from_secs(5));
    assert!(t0.elapsed() <= Duration::from_secs(5), "stop took {:?}", t0.elapsed());

    let sv_after = client.wait_idle(&sid);
    assert!(sv_after["pending_checkpoints"].as_array().is_some_and(|a| a.is_empty()), "{sv_after}");
    let (_, evs2) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs2.iter().any(|e| e.kind == "session.abort"));
    srv.shutdown();
}

#[test]
fn ultramath_interactive_branch_picker_pauses_and_returns_selection() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();

    let mock = ScriptedMock::start(Box::new(|path, body| {
        if path == "/v1/messages" {
            let out = if last_user_text(body).contains("Jordan-Wigner") {
                "Jordan-Wigner branch: free-fermion diagonalization."
            } else {
                "Majorana pairing branch: Pfaffian determinant."
            };
            anthropic_text("claude-haiku-4-5", out)
        } else if openai_tool_count(body) == 0 {
            openai_tool("gpt-4.1", "call_exp_hitl", "delegate_explore", json!({ "problem": "Compare two derivations", "approaches": ["Jordan-Wigner", "Majorana pairing"] }))
        } else {
            openai_text("gpt-4.1", "Pursued Branch 2 with Majorana basis.")
        }
    }));

    let (_tmp, data_dir, srv, client) = setup_server(&mock.base_url, json!({
        "keys": { "openai": "sk-oai-s15-t3", "anthropic": "sk-ant-s15-t3" },
        "agent_models": { "explorer_worker": { "provider": "anthropic", "model": "claude-haiku-4-5" } }
    }));
    let sid = client.create_session("Stage15 Branch Picker");
    let mut sse = client.sse(&sid);

    client.post_message(&sid, json!({ "prompt": "Compare two derivations", "provider": "openai", "model": "gpt-4.1", "verification_mode": false, "pause_after_explore": true }));
    let cp = sse.wait_event_since(0, "checkpoint_requested", Duration::from_secs(10));
    let cid = cp["checkpoint_id"].as_str().expect("checkpoint_id").to_string();
    assert_eq!(cp["kind"], "branch");
    assert_eq!(cp["branches"].as_array().map(|a| a.len()), Some(2), "{cp}");
    let opt2 = cp["options"][1].as_str().expect("Branch 2 option").to_string();
    assert!(opt2.contains("Majorana pairing") || opt2.contains("Branch 2"), "{cp}");
    assert_eq!(client.get_session(&sid)["pending_checkpoints"].as_array().map(|a| a.len()), Some(1));

    let mark = sse.len();
    let (st_res, _, res_raw) = client.req("POST", &format!("/api/sessions/{sid}/checkpoints/{cid}"), Some(&json!({ "selected_option": opt2, "text": "Use Majorana basis" })));
    assert_eq!(st_res, 200, "{res_raw}");
    sse.wait_event_since(mark, "checkpoint_resolved", Duration::from_secs(10));
    sse.wait_event_since(mark, "turn_complete", Duration::from_secs(10));
    let sv_done = client.wait_idle(&sid);
    assert!(sv_done["pending_checkpoints"].as_array().is_some_and(|a| a.is_empty()), "{sv_done}");

    let cap = mock.captured();
    let oai: Vec<_> = cap.iter().filter(|(p, _)| p == "/v1/chat/completions").collect();
    let ant: Vec<_> = cap.iter().filter(|(p, _)| p == "/v1/messages").collect();
    assert_eq!((oai.len(), ant.len()), (2, 2));
    assert!(has_tool(&oai[0].1, "ask_user"));
    assert!(ant.iter().all(|(_, b)| !has_tool(b, "ask_user")));
    for tools_fn in [agent::compute_worker_tools, agent::literature_worker_tools, agent::lean_worker_tools, agent::explore_worker_tools] {
        assert!(!tools_fn().iter().any(|t| t == "ask_user"));
    }

    let tr = openai_tool_result_json(&oai[1].1, "call_exp_hitl").expect("explore tool_result JSON");
    assert_eq!(tr["human_selection"]["selected_option"], opt2);
    assert_eq!(tr["human_selection"]["text"], "Use Majorana basis");

    let (_, evs) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs.iter().any(|e| e.kind == "explore.start" && e.payload["branch_count"] == 2));
    assert_eq!(evs.iter().filter(|e| e.kind == "explore.branch").count(), 2);
    assert!(evs.iter().any(|e| e.kind == "explore.end" && e.payload["branch_count"] == 2));
    assert!(evs.iter().any(|e| e.kind == "human.checkpoint_requested" && e.payload["kind"] == "branch"));
    assert!(evs.iter().any(|e| e.kind == "human.decision" && e.payload["text"] == "Use Majorana basis"));
    srv.shutdown();
}

#[test]
fn explain_promote_updates_conventions_and_steers_or_returns_prompt() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();
    let (req1_arrived, req1_cl) = Flag::pair();
    let (unblock_req1, unblock_cl) = Flag::pair();

    let mock = ScriptedMock::start(Box::new(move |_path, body| {
        if anthropic_tool_count(body) == 0 {
            req1_cl.signal();
            unblock_cl.wait("unblock_req1");
            anthropic_tool("claude-haiku-4-5", "tu_prom1", "ledger_note", json!({ "note": "Noted terms" }))
        } else {
            anthropic_text("claude-haiku-4-5", "Dropped O(a^2) surface terms.")
        }
    }));

    let (_tmp, data_dir, srv, client) = setup_server(&mock.base_url, json!({ "keys": { "anthropic": "sk-ant-s15-t4" } }));
    let sid = client.create_session("Stage15 Explain Promote");

    let (st_c, conv_v, conv_raw) = client.req("POST", &format!("/api/sessions/{sid}/explain/promote"), Some(&json!({ "target": "conventions", "text": "Spinless fermions, t > 0" })));
    assert_eq!(st_c, 200, "{conv_raw}");
    assert_eq!((conv_v["ok"].as_bool(), conv_v["target"].as_str()), (Some(true), Some("conventions")));
    assert!(conv_v["pinned_conventions"].as_str().unwrap_or("").contains("Spinless fermions, t > 0"), "{conv_v}");

    let sv_conv = client.get_session(&sid);
    let pinned = sv_conv["pinned_conventions"].as_str().or_else(|| sv_conv["meta"]["pinned_conventions"].as_str()).unwrap_or("");
    assert!(pinned.contains("Spinless fermions, t > 0"), "{sv_conv}");

    let (st_idle, idle_v, idle_raw) = client.req("POST", &format!("/api/sessions/{sid}/explain/promote"), Some(&json!({ "target": "steer", "text": "Use gauge A_x = 0" })));
    assert_eq!(st_idle, 200, "{idle_raw}");
    assert_eq!((idle_v["ok"].as_bool(), idle_v["mode"].as_str(), idle_v["text"].as_str()), (Some(true), Some("prompt"), Some("Use gauge A_x = 0")));

    let mut sse = client.sse(&sid);
    client.post_message(&sid, json!({ "prompt": "Expand lattice action", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    req1_arrived.wait("req1_arrived");

    let (st_act, act_v, act_raw) = client.req("POST", &format!("/api/sessions/{sid}/explain/promote"), Some(&json!({ "target": "steer", "text": "Drop O(a^2) surface terms" })));
    assert_eq!(st_act, 202, "{act_raw}");
    assert_eq!((act_v["ok"].as_bool(), act_v["mode"].as_str(), act_v["steer"]["text"].as_str()), (Some(true), Some("steer"), Some("Drop O(a^2) surface terms")));

    unblock_req1.signal();
    sse.wait_event_since(0, "steer_queued", Duration::from_secs(10));
    sse.wait_event_since(0, "turn_complete", Duration::from_secs(10));
    client.wait_idle(&sid);

    let cap = mock.captured();
    assert_eq!(cap.len(), 2);
    assert!(system_text(&cap[0].1).contains("Spinless fermions, t > 0"));
    assert!(all_user_text(&cap[1].1).contains("<human_steer>\nDrop O(a^2) surface terms\n</human_steer>"));

    let (_, evs) = verify_ledger_clean(&data_dir, &sid);
    assert!(evs.iter().any(|e| e.kind == "explain.promoted" && e.payload["target"] == "conventions"));
    assert!(evs.iter().any(|e| e.kind == "explain.promoted" && e.payload["target"] == "steer"));
    assert!(evs.iter().any(|e| e.kind == "human.steer" && e.payload["text"] == "Drop O(a^2) surface terms"));
    srv.shutdown();
}

#[test]
fn session_conventions_put_updates_and_clears_resumed_orchestrator() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _env = EnvGuard::clear_providers();
    let mock = ScriptedMock::start(Box::new(|_path, body| {
        let idx = last_user_text(body);
        anthropic_text("claude-haiku-4-5", &format!("Ack: {idx}"))
    }));

    let (_tmp, data_dir, srv, client) = setup_server(&mock.base_url, json!({ "keys": { "anthropic": "sk-ant-s15-t5" } }));
    assert_eq!(desktop_server::subagents::normalize_rigor(Some("interactive")).unwrap().as_deref(), Some("interactive"));

    let sid = client.create_session("Session Conventions Sync");
    let mut sse = client.sse(&sid);
    client.post_message(&sid, json!({ "prompt": "Turn one", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    sse.wait_event_since(0, "turn_complete", Duration::from_secs(10));
    client.wait_idle(&sid);

    let (st_put, put_v, put_raw) = client.req("PUT", &format!("/api/sessions/{sid}/conventions"), Some(&json!({ "pinned_conventions": "Signature (-,+,+,+)" })));
    assert_eq!((st_put, put_v["pinned_conventions"].as_str()), (200, Some("Signature (-,+,+,+)")), "{put_raw}");
    let m2 = sse.len();
    client.post_message(&sid, json!({ "prompt": "Turn two", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    sse.wait_event_since(m2, "turn_complete", Duration::from_secs(10));
    client.wait_idle(&sid);

    let (st_clr, clr_v, _) = client.req("PUT", &format!("/api/sessions/{sid}/conventions"), Some(&json!({ "pinned_conventions": "" })));
    assert_eq!((st_clr, clr_v["pinned_conventions"].as_str()), (200, Some("")));
    let m3 = sse.len();
    client.post_message(&sid, json!({ "prompt": "Turn three", "provider": "anthropic", "model": "claude-haiku-4-5", "verification_mode": false }));
    sse.wait_event_since(m3, "turn_complete", Duration::from_secs(10));
    client.wait_idle(&sid);

    let cap = mock.captured();
    assert_eq!(cap.len(), 3);
    assert!(!system_text(&cap[0].1).contains("<pinned_conventions>"));
    assert!(system_text(&cap[1].1).contains("<pinned_conventions>\nSignature (-,+,+,+)\n</pinned_conventions>"));
    assert!(!system_text(&cap[2].1).contains("<pinned_conventions>"));
    verify_ledger_clean(&data_dir, &sid);
    srv.shutdown();
}
