//! End-to-end binary tests for `re-search run --route api` against a local mock
//! `POST /v1/messages` server (runs on Linux and Windows without Python or
//! external network access).

use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const TEST_API_KEY: &str = "sk-test-never-record-998877";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cli-api-route-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

enum MockReply {
    Json(u16, Value),
}

struct MockServer {
    base_url: String,
    rx: mpsc::Receiver<Value>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let req_body = read_http_request(&mut stream);
                let _ = tx.send(req_body);
                match reply {
                    MockReply::Json(status, body) => write_http_json(&mut stream, status, &body),
                }
            }
        });
        Self { base_url: format!("http://{addr}"), rx, handle: Some(handle) }
    }

    fn take_requests(mut self) -> Vec<Value> {
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
        self.rx.try_iter().collect()
    }
}

fn read_http_request(stream: &mut TcpStream) -> Value {
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    assert!(request_line.starts_with("POST /v1/messages "), "unexpected request line: {request_line:?}");
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap();
            }
        }
    }
    let mut body = vec![0u8; content_len];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn write_http_json(stream: &mut TcpStream, status: u16, body: &Value) {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        _ => "Error",
    };
    let payload = serde_json::to_vec(body).unwrap();
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(&payload).unwrap();
    stream.flush().unwrap();
}

fn base_cmd() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_re-search"));
    for (k, _) in std::env::vars_os() {
        let upper = k.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("RE_SEARCH_")
            || upper.starts_with("RESEARCH_CORE_")
            || matches!(
                upper.as_str(),
                "ANTHROPIC_API_KEY" | "OPENAI_API_KEY" | "OLLAMA_API_KEY" | "GEMINI_API_KEY" | "GOOGLE_API_KEY"
            )
        {
            cmd.env_remove(k);
        }
    }
    cmd.stdin(Stdio::null());
    cmd
}

fn run_api(data_dir: &Path, base_url: &str, prompt: &str) -> Output {
    base_cmd()
        .env("RE_SEARCH_API_KEY", TEST_API_KEY)
        .args([
            "run",
            "--route",
            "api",
            "--prompt",
            prompt,
            "--data-dir",
            &data_dir.to_string_lossy(),
            "--api-base",
            base_url,
        ])
        .output()
        .unwrap()
}

fn verify_session_cli(data_dir: &Path, session: &str) -> Output {
    base_cmd()
        .args(["verify", session, "--data-dir", &data_dir.to_string_lossy()])
        .output()
        .unwrap()
}

fn scan_dir_for_secret(dir: &Path, secret: &[u8]) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan_dir_for_secret(&path, secret);
        } else if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret),
                "secret leaked into {}",
                path.display()
            );
        }
    }
}

#[test]
fn api_route_runs_ledger_note_denies_non_tty_code_and_verifies() {
    let tmp = TempDir::new("ok");
    let turn1 = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "tool_use",
        "content": [
            { "type": "tool_use", "id": "tu_note", "name": "ledger_note", "input": { "text": "stage4 note" } },
            { "type": "tool_use", "id": "tu_eval", "name": "sympy_eval", "input": { "code": "1+1" } }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 20 }
    });
    let turn2 = json!({
        "id": "msg_2",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": "done" }],
        "usage": { "input_tokens": 30, "output_tokens": 5 }
    });
    let server = MockServer::start(vec![MockReply::Json(200, turn1), MockReply::Json(200, turn2)]);
    let out = run_api(&tmp.0, &server.base_url, "record note and try eval");
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));

    let stdout_s = String::from_utf8(out.stdout.clone()).unwrap();
    let stderr_s = String::from_utf8(out.stderr.clone()).unwrap();
    assert!(!stdout_s.contains(TEST_API_KEY));
    assert!(!stderr_s.contains(TEST_API_KEY));

    let ndjson: Vec<Value> = stdout_s.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(!ndjson.is_empty());
    for (idx, line) in ndjson.iter().enumerate() {
        assert_eq!(line["seq"], (idx + 1) as u64);
        if idx == 0 {
            assert_eq!(line["prev_hash"], ledger::GENESIS_HASH);
        } else {
            assert_eq!(line["prev_hash"], ndjson[idx - 1]["event_hash"]);
        }
    }

    let reqs = server.take_requests();
    assert_eq!(reqs.len(), 2);
    let turn2_msgs = reqs[1]["messages"].as_array().unwrap();
    let tool_results = turn2_msgs.last().unwrap()["content"].as_array().unwrap();
    assert_eq!(tool_results.len(), 2);
    assert_eq!(tool_results[0]["tool_use_id"], "tu_note");
    assert_eq!(tool_results[0]["is_error"], false);
    assert!(tool_results[0]["content"].as_str().unwrap().contains("\"recorded\":true"));
    assert_eq!(tool_results[1]["tool_use_id"], "tu_eval");
    assert_eq!(tool_results[1]["is_error"], true);
    assert_eq!(tool_results[1]["content"], "DENIED: approval: declined");

    let ledger = Ledger::open(&tmp.0).unwrap();
    let sessions = ledger.sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0].0;
    let events = ledger.read_session(session).unwrap();
    let answered = events.iter().find(|e| e.kind == "approval.answered").expect("approval.answered");
    assert_eq!(answered.payload["approved"], false);
    assert_eq!(answered.payload["by"], "supervisor: no terminal to ask on");
    assert!(events.iter().any(|e| e.kind == "note" && e.payload["text"] == "stage4 note"));

    let ver = verify_session_cli(&tmp.0, session);
    assert_eq!(ver.status.code(), Some(0), "verify failed:\n{}", String::from_utf8_lossy(&ver.stderr));
    scan_dir_for_secret(&tmp.0, TEST_API_KEY.as_bytes());
}

#[test]
fn api_route_exits_5_on_model_refusal_and_3_on_http_error() {
    let refusal_dir = TempDir::new("refusal");
    let refusal_body = json!({
        "id": "msg_refuse",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "refusal",
        "content": [{ "type": "text", "text": "cannot assist" }],
        "usage": { "input_tokens": 5, "output_tokens": 3 }
    });
    let server = MockServer::start(vec![MockReply::Json(200, refusal_body)]);
    let out = run_api(&refusal_dir.0, &server.base_url, "refuse this");
    assert_eq!(out.status.code(), Some(5), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(server.take_requests().len(), 1);
    let ledger = Ledger::open(&refusal_dir.0).unwrap();
    let s_ref = ledger.sessions().unwrap()[0].0.clone();
    assert_eq!(verify_session_cli(&refusal_dir.0, &s_ref).status.code(), Some(0));

    let err_dir = TempDir::new("http401");
    let err_body = json!({
        "type": "error",
        "error": { "type": "authentication_error", "message": "invalid x-api-key" }
    });
    let server = MockServer::start(vec![MockReply::Json(401, err_body)]);
    let out = run_api(&err_dir.0, &server.base_url, "auth fail");
    assert_eq!(out.status.code(), Some(3), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(server.take_requests().len(), 1);
    let ledger = Ledger::open(&err_dir.0).unwrap();
    let s_err = ledger.sessions().unwrap()[0].0.clone();
    assert_eq!(verify_session_cli(&err_dir.0, &s_err).status.code(), Some(0));

    // Missing API key with scrubbed child environment -> deterministic exit 2 and no ledger created.
    let nokey_dir = TempDir::new("nokey");
    let nokey_out = base_cmd()
        .args([
            "run",
            "--route",
            "api",
            "--prompt",
            "no key",
            "--data-dir",
            &nokey_dir.0.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert_eq!(nokey_out.status.code(), Some(2));
    assert!(!nokey_dir.0.join("ledger.db").exists());
}
