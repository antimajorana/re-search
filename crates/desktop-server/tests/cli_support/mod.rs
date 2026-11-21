//! Helpers for the Claude subscription route (`claude-cli`) tests: a desktop server whose
//! `claude.exe` is the scripted fake from `cli_route::testkit`, an SSE reader, and
//! stream-json lines. No real CLI and no model is ever started.
#![allow(dead_code)]

use crate::common::{launch_cookie, raw_http, test_cfg};
use cli_route::testkit::FakeCli;
use desktop_server::{start_desktop_server, RunningDesktopServer};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub struct CliHarness {
    pub tmp: TempDir,
    pub data: PathBuf,
    pub ws: PathBuf,
    pub fake: FakeCli,
    pub srv: RunningDesktopServer,
    pub port: u16,
    pub tok: String,
}

impl CliHarness {
    pub fn start() -> CliHarness {
        Self::start_with(|_| {})
    }

    /// [`CliHarness::start`] with a last change to the configuration.
    pub fn start_with(edit: impl FnOnce(&mut desktop_server::DesktopConfig)) -> CliHarness {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("notes.tex"), "old\n").unwrap();
        let fake = FakeCli::new(tmp.path(), "fake-cli");
        let mut cfg = test_cfg(&data, Some(ws.clone()), None, None);
        cfg.claude_exe = Some(fake.exe.clone());
        // Never started by the fake CLI; it only has to exist.
        cfg.core_exe = Some(fake.exe.clone());
        edit(&mut cfg);
        let srv = start_desktop_server(cfg).unwrap();
        let port = srv.addr.port();
        let tok = srv.info.token.clone();
        CliHarness { tmp, data, ws, fake, srv, port, tok }
    }

    pub fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        let hdrs = [("X-ReSearch-Token", self.tok.as_str()), ("Content-Type", "application/json")];
        let (st, _, b) = raw_http(self.port, "POST", path, &hdrs, serde_json::to_vec(body).unwrap().as_slice());
        (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    pub fn get(&self, path: &str) -> (u16, Value) {
        let (st, _, b) = raw_http(self.port, "GET", path, &[("X-ReSearch-Token", self.tok.as_str())], b"");
        (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    /// Create an idle session bound to the Claude subscription route.
    pub fn create_cli_session(&self) -> String {
        let (st, v) = self.post("/api/sessions", &json!({ "title": "cli", "provider": "claude-cli", "model": "haiku" }));
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["route"], "claude-cli");
        v["id"].as_str().unwrap().to_string()
    }

    pub fn send(&self, sid: &str, prompt: &str) {
        let (st, v) = self.post(&format!("/api/sessions/{sid}/messages"), &json!({ "prompt": prompt, "provider": "claude-cli" }));
        assert_eq!(st, 202, "{v}");
    }

    pub fn sse(&self, sid: &str) -> Sse {
        let cookie = launch_cookie(self.port, &self.tok);
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let req = format!("GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: {cookie}\r\n\r\n", self.port);
        s.write_all(req.as_bytes()).unwrap();
        let (tx, rx) = mpsc::channel::<(String, Value)>();
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(s);
            let (mut ev, mut data) = (String::new(), String::new());
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let t = line.trim_end_matches(['\r', '\n']);
                        if t == ": connected" {
                            // The server subscribes before it writes this line.
                            let _ = ready_tx.send(());
                        } else if t.is_empty() {
                            if !ev.is_empty() && !data.is_empty() {
                                if let Ok(v) = serde_json::from_str::<Value>(&data) {
                                    let _ = tx.send((ev.clone(), v));
                                }
                            }
                            ev.clear();
                            data.clear();
                        } else if let Some(rest) = t.strip_prefix("event:") {
                            ev = rest.trim().to_string();
                        } else if let Some(rest) = t.strip_prefix("data:") {
                            data.push_str(rest.trim());
                        }
                    }
                }
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(30)).expect("SSE stream did not connect");
        Sse { rx, seen: Vec::new() }
    }
}

pub struct Sse {
    rx: Receiver<(String, Value)>,
    pub seen: Vec<(String, Value)>,
}

impl Sse {
    /// Wait for the first event named `kind` (keeping everything seen on the way).
    pub fn wait_for(&mut self, kind: &str, timeout: Duration) -> Value {
        let end = Instant::now() + timeout;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if let Ok((ev, data)) = self.rx.recv_timeout(left) {
                self.seen.push((ev.clone(), data.clone()));
                if ev == kind {
                    return data;
                }
            }
        }
        panic!("no `{kind}` SSE event within {timeout:?}; saw {:?}", self.seen.iter().map(|(k, _)| k).collect::<Vec<_>>());
    }

    /// Wait for the end of a turn: `Ok(turn_complete)` or `Err(turn_error)`.
    pub fn wait_turn_end(&mut self, timeout: Duration) -> Result<Value, Value> {
        let end = Instant::now() + timeout;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if let Ok((ev, data)) = self.rx.recv_timeout(left) {
                self.seen.push((ev.clone(), data.clone()));
                match ev.as_str() {
                    "turn_complete" => return Ok(data),
                    "turn_error" => return Err(data),
                    _ => {}
                }
            }
        }
        panic!("turn did not end within {timeout:?}; saw {:?}", self.seen.iter().map(|(k, _)| k).collect::<Vec<_>>());
    }

    /// `kind` values of the `session_event` frames seen so far.
    pub fn session_kinds(&self) -> Vec<String> {
        self.seen
            .iter()
            .filter(|(ev, _)| ev == "session_event")
            .filter_map(|(_, d)| d["kind"].as_str().map(str::to_string))
            .collect()
    }
}

/// The tool names `core.exe` serves on a desktop session (research tools on).
pub fn expected_tools(data: &std::path::Path) -> Vec<String> {
    let env = std::sync::Arc::new(research_tools::ResearchEnv::new(data));
    cli_route::core_tools::core_tool_set(Some(env)).defs.iter().map(|d| format!("mcp__core__{}", d.id)).collect()
}

pub fn init_line(tools: &[String], servers: Value) -> String {
    json!({ "type": "system", "subtype": "init", "tools": tools, "mcp_servers": servers,
        "model": "claude-haiku", "claude_code_version": "2.1.318", "apiKeySource": "none", "session_id": "cli-1" })
    .to_string()
}

pub fn core_only() -> Value {
    json!([{ "name": "core", "status": "connected" }])
}

pub fn assistant(blocks: Value) -> String {
    json!({ "type": "assistant", "message": { "content": blocks } }).to_string()
}

pub fn tool_result(id: &str, text: &str) -> String {
    json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "content": [{ "type": "text", "text": text }] }] } })
        .to_string()
}

pub fn result(text: &str) -> String {
    json!({ "type": "result", "subtype": "success", "is_error": false, "result": text, "num_turns": 2, "duration_ms": 5 }).to_string()
}

/// Make the test process's environment free of credentials the refusal looks for.
pub fn clear_env_auth() {
    for v in cli_route::claude::CLAUDE_ENV_AUTH_VARS {
        std::env::remove_var(v);
    }
    std::env::remove_var(cli_route::env_auth::ALLOW_ENV_AUTH_VAR);
}
