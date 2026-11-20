//! End-to-end binary tests for multi-provider `--route api`, `re-search resume`,
//! session lock enforcement, and fail-closed resume rejections against local
//! `127.0.0.1:0` mock HTTP servers.

use ledger::{Ledger, GENESIS_HASH};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const TEST_KEY: &str = "sk-stage5-test-never-record-112233";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cli-resume-route-{tag}-{}", std::process::id()));
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

struct CapturedHttp {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

struct MockReply {
    status: u16,
    body: Value,
}

struct MockServer {
    base_url: String,
    rx: mpsc::Receiver<CapturedHttp>,
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
                let captured = read_http_request(&mut stream);
                let _ = tx.send(captured);
                write_http_json(&mut stream, reply.status, &reply.body);
            }
        });
        Self { base_url: format!("http://{addr}"), rx, handle: Some(handle) }
    }

    fn take_requests(mut self) -> Vec<CapturedHttp> {
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
        self.rx.try_iter().collect()
    }
}

fn read_http_request(stream: &mut TcpStream) -> CapturedHttp {
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut parts = request_line.split_whitespace();
    assert_eq!(parts.next().unwrap_or(""), "POST", "unexpected HTTP method in {request_line:?}");
    let path = parts.next().unwrap_or("").to_string();

    let mut headers = Vec::new();
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let name = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if name == "content-length" {
                content_len = val.parse().unwrap();
            }
            headers.push((name, val));
        }
    }
    let mut body = vec![0u8; content_len];
    reader.read_exact(&mut body).unwrap();
    CapturedHttp { path, headers, body: serde_json::from_slice(&body).unwrap() }
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

fn run_cli(env: Option<(&str, &str)>, args: &[&str]) -> Output {
    let mut cmd = base_cmd();
    if let Some((k, v)) = env {
        cmd.env(k, v);
    }
    cmd.args(args).output().unwrap()
}

fn verify_cli(data_dir: &Path, session: Option<&str>) -> Output {
    let d = data_dir.to_string_lossy().into_owned();
    let mut args = vec!["verify"];
    if let Some(s) = session {
        args.push(s);
    }
    args.extend(["--data-dir", &d]);
    run_cli(None, &args)
}

fn scan_dir_for_secret(dir: &Path, secret: &[u8]) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan_dir_for_secret(&path, secret);
        } else if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(!bytes.windows(secret.len()).any(|w| w == secret), "secret leaked into {}", path.display());
        }
    }
}

fn parse_ndjson(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn header_val<'a>(req: &'a CapturedHttp, name: &str) -> Option<&'a str> {
    req.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn anthropic_text(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_ant_1",
        "type": "message",
        "role": "assistant",
        "model": model,
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 10, "output_tokens": 4 }
    })
}

fn openai_chat_ok(id: &str, model: &str, text: &str) -> MockReply {
    MockReply {
        status: 200,
        body: json!({
            "id": id,
            "model": model,
            "choices": [{ "index": 0, "finish_reason": "stop", "message": { "role": "assistant", "content": text } }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 4 }
        }),
    }
}

fn openai_tool_and_text(model: &str) -> Vec<MockReply> {
    let t1 = json!({
        "id": "chatcmpl_1",
        "model": model,
        "choices": [{
            "index": 0,
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_note_1",
                    "type": "function",
                    "function": { "name": "ledger_note", "arguments": "{\"text\":\"openai note\"}" }
                }]
            }
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 8 }
    });
    vec![MockReply { status: 200, body: t1 }, openai_chat_ok("chatcmpl_2", model, "openai done")]
}

fn gemini_text(text: &str) -> Value {
    json!({
        "responseId": "resp_gem_1",
        "modelVersion": "gemini-2.5-flash",
        "candidates": [{ "finishReason": "STOP", "content": { "role": "model", "parts": [{ "text": text }] } }],
        "usageMetadata": { "promptTokenCount": 9, "candidatesTokenCount": 4 }
    })
}

fn write_minimal_legacy_v0_db(path: &Path) {
    let mut buf = vec![0u8; 1536];
    let patches: &[(usize, &[u8])] = &[
        (0, b"SQLite format 3\x00\x02\x00\x01\x01\x00@  \x00\x00\x00\x02\x00\x00\x00\x03"),
        (43, b"\x01\x00\x00\x00\x04"),
        (59, b"\x01"),
        (95, b"\x02\x00.\x95\xc9\r\x01\xf8\x00\x02\x00\xf3\x00\x00\xf3\x01\xcd"),
        (243, b"\x81W\x01\x07\x17\x17\x17\x01\x83\rtableeventevent\x02CREATE TABLE event (session TEXT NOT NULL, seq INTEGER NOT NULL, ts_ms INTEGER NOT NULL, kind TEXT NOT NULL, schema_version INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (session, seq)))\x02\x06\x17=\x17\x01\x00indexsqlite_autoindex_event_1event\x03\x00\x00\x00\x08"),
        (512, b"\r\x00\x00\x00\x01\x01\xef\x00\x01\xef"),
        (1007, b"\x0f\x01\x07\x11\t\x08\x15\t\x11s0note{}\n\x00\x00\x00\x01\x01\xf9\x00\x01\xf9"),
        (1529, b"\x06\x04\x11\t\ts0"),
    ];
    for (off, bytes) in patches {
        buf[*off..*off + bytes.len()].copy_from_slice(bytes);
    }
    std::fs::write(path, buf).unwrap();
}

#[test]
fn multi_provider_run_openai_ollama_keyless_and_gemini_header_auth() {
    // 1. OpenAI with OPENAI_API_KEY + tool round-trip
    let openai_dir = TempDir::new("openai");
    let openai_srv = MockServer::start(openai_tool_and_text("gpt-4.1-mini"));
    let d_oai = openai_dir.0.to_string_lossy().into_owned();
    let out = run_cli(
        Some(("OPENAI_API_KEY", TEST_KEY)),
        &["run", "--route", "api", "--provider", "openai", "--prompt", "note it", "--data-dir", &d_oai, "--api-base", &openai_srv.base_url],
    );
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    let reqs = openai_srv.take_requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    assert_eq!(header_val(&reqs[0], "authorization"), Some("Bearer sk-stage5-test-never-record-112233"));
    let ver = verify_cli(&openai_dir.0, None);
    assert_eq!(ver.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&ver.stdout).contains("(0 unreferenced)"));
    scan_dir_for_secret(&openai_dir.0, TEST_KEY.as_bytes());

    // 2. Ollama keyless loopback (zero API key env vars set!)
    let ollama_dir = TempDir::new("ollama");
    let ollama_srv = MockServer::start(vec![openai_chat_ok("chatcmpl_ol", "qwen2.5-coder:7b", "local ok")]);
    let d_ol = ollama_dir.0.to_string_lossy().into_owned();
    let out = run_cli(
        None,
        &["run", "--route", "api", "--provider", "ollama", "--prompt", "local test", "--data-dir", &d_ol, "--api-base", &ollama_srv.base_url],
    );
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    let reqs = ollama_srv.take_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    assert!(header_val(&reqs[0], "authorization").is_none());
    let ledger = Ledger::open(&ollama_dir.0).unwrap();
    let sess = ledger.sessions().unwrap()[0].0.clone();
    let start = ledger.read_session(&sess).unwrap().into_iter().find(|e| e.kind == "session.start").unwrap();
    assert_eq!(start.payload["api_key_env"], "none");
    assert_eq!(verify_cli(&ollama_dir.0, None).status.code(), Some(0));

    // 3. Gemini header auth (`x-goog-api-key`, zero `?key=` in path)
    let gemini_dir = TempDir::new("gemini");
    let gemini_srv = MockServer::start(vec![MockReply { status: 200, body: gemini_text("gem ok") }]);
    let d_gem = gemini_dir.0.to_string_lossy().into_owned();
    let out = run_cli(
        Some(("GEMINI_API_KEY", TEST_KEY)),
        &["run", "--route", "api", "--provider", "gemini", "--prompt", "gem test", "--data-dir", &d_gem, "--api-base", &gemini_srv.base_url],
    );
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&out.stderr));
    let reqs = gemini_srv.take_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/v1beta/models/gemini-2.5-flash:generateContent");
    assert!(!reqs[0].path.contains("key="));
    assert_eq!(header_val(&reqs[0], "x-goog-api-key"), Some(TEST_KEY));
    assert_eq!(verify_cli(&gemini_dir.0, None).status.code(), Some(0));
    scan_dir_for_secret(&gemini_dir.0, TEST_KEY.as_bytes());
}

#[test]
fn resume_inherits_provider_and_model_produces_contiguous_chain_and_verifies() {
    let dir = TempDir::new("inherit");
    let d = dir.0.to_string_lossy().into_owned();
    let srv1 = MockServer::start(openai_tool_and_text("gpt-4.1-mini"));
    let run1 = run_cli(
        Some(("OPENAI_API_KEY", TEST_KEY)),
        &["run", "--route", "api", "--provider", "openai", "--model", "gpt-4.1-mini", "--prompt", "first prompt", "--data-dir", &d, "--api-base", &srv1.base_url],
    );
    assert_eq!(run1.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&run1.stderr));
    let _ = srv1.take_requests();

    let leg1 = parse_ndjson(&run1.stdout);
    let session = leg1[0]["session"].as_str().unwrap().to_string();
    let last_leg1 = leg1.last().unwrap();
    assert_eq!(leg1[0]["prev_hash"], GENESIS_HASH);

    let srv2 = MockServer::start(vec![openai_chat_ok("chatcmpl_resume", "gpt-4.1-mini", "resumed ok")]);
    let run2 = run_cli(
        Some(("OPENAI_API_KEY", TEST_KEY)),
        &["resume", &session, "--prompt", "second prompt", "--data-dir", &d, "--api-base", &srv2.base_url],
    );
    assert_eq!(run2.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&run2.stderr));
    let reqs2 = srv2.take_requests();
    assert_eq!(reqs2.len(), 1);
    assert_eq!(reqs2[0].path, "/v1/chat/completions");
    assert_eq!(reqs2[0].body["model"], "gpt-4.1-mini");
    let msgs = reqs2[0].body["messages"].as_array().unwrap();
    assert!(msgs.iter().any(|m| m["content"] == "first prompt"));
    assert!(msgs.iter().any(|m| m["content"] == "second prompt"));

    let leg2 = parse_ndjson(&run2.stdout);
    assert_eq!(leg2[0]["kind"], "session.resume");
    assert_eq!(leg2[0]["seq"], last_leg1["seq"].as_u64().unwrap() + 1);
    assert_eq!(leg2[0]["prev_hash"], last_leg1["event_hash"]);

    let ver = verify_cli(&dir.0, None);
    assert_eq!(ver.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&ver.stderr));
    assert!(String::from_utf8_lossy(&ver.stdout).contains("(0 unreferenced)"));
}

#[test]
fn cross_provider_resume_anthropic_to_openai_verifies_cleanly() {
    let dir = TempDir::new("cross");
    let d = dir.0.to_string_lossy().into_owned();
    let ant_srv = MockServer::start(vec![MockReply {
        status: 200,
        body: json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-haiku-4-5",
            "stop_reason": "end_turn",
            "content": [
                { "type": "thinking", "thinking": "internal trace", "signature": "sig1" },
                { "type": "text", "text": "leg 1 done" }
            ],
            "usage": { "input_tokens": 5, "output_tokens": 3 }
        }),
    }]);
    let run1 = run_cli(
        Some(("ANTHROPIC_API_KEY", TEST_KEY)),
        &["run", "--route", "api", "--provider", "anthropic", "--prompt", "start anthropic", "--data-dir", &d, "--api-base", &ant_srv.base_url],
    );
    assert_eq!(run1.status.code(), Some(0));
    let _ = ant_srv.take_requests();
    let session = parse_ndjson(&run1.stdout)[0]["session"].as_str().unwrap().to_string();

    let oai_srv = MockServer::start(vec![openai_chat_ok("chatcmpl_x", "gpt-4.1-mini", "continued on openai")]);
    let run2 = run_cli(
        Some(("OPENAI_API_KEY", TEST_KEY)),
        &["resume", &session, "--provider", "openai", "--prompt", "continue on openai", "--data-dir", &d, "--api-base", &oai_srv.base_url],
    );
    assert_eq!(run2.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&run2.stderr));
    let reqs = oai_srv.take_requests();
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    assert_eq!(reqs[0].body["model"], "gpt-4.1-mini");
    assert!(!serde_json::to_string(&reqs[0].body).unwrap().contains("internal trace"));
    assert_eq!(verify_cli(&dir.0, Some(&session)).status.code(), Some(0));
}

#[test]
fn resume_rejects_tamper_unknown_legacy_cli_route_and_live_lock_but_recovers_stale_lock() {
    // 1. Tampered blob -> exit 1
    let tamper_dir = TempDir::new("tamper");
    let d_t = tamper_dir.0.to_string_lossy().into_owned();
    let srv = MockServer::start(vec![MockReply { status: 200, body: anthropic_text("claude-haiku-4-5", "ok") }]);
    let run = run_cli(
        Some(("ANTHROPIC_API_KEY", TEST_KEY)),
        &["run", "--route", "api", "--prompt", "hi", "--data-dir", &d_t, "--api-base", &srv.base_url],
    );
    assert_eq!(run.status.code(), Some(0));
    let _ = srv.take_requests();
    let leg1 = parse_ndjson(&run.stdout);
    let session = leg1[0]["session"].as_str().unwrap().to_string();
    let req_blob = leg1.iter().find(|e| e["kind"] == "model.request").unwrap()["data"]["raw_blob"]
        .as_str()
        .unwrap()
        .to_string();
    let count_before = Ledger::open_existing(&tamper_dir.0).unwrap().read_session(&session).unwrap().len();
    std::fs::write(tamper_dir.0.join("blobs").join(&req_blob[..2]).join(&req_blob), b"tampered").unwrap();

    let res = run_cli(Some(("ANTHROPIC_API_KEY", TEST_KEY)), &["resume", &session, "--prompt", "next", "--data-dir", &d_t]);
    assert_eq!(res.status.code(), Some(1));
    assert_eq!(Ledger::open_existing(&tamper_dir.0).unwrap().read_session(&session).unwrap().len(), count_before);

    // 2. Unknown session -> exit 1
    let res_unknown = run_cli(Some(("ANTHROPIC_API_KEY", TEST_KEY)), &["resume", "s-doesnotexist", "--prompt", "next", "--data-dir", &d_t]);
    assert_eq!(res_unknown.status.code(), Some(1));

    // 3. Legacy v0 DB -> exit 4
    let legacy_dir = TempDir::new("legacy");
    write_minimal_legacy_v0_db(&legacy_dir.0.join("ledger.db"));
    let d_leg = legacy_dir.0.to_string_lossy().into_owned();
    assert_eq!(
        run_cli(Some(("ANTHROPIC_API_KEY", TEST_KEY)), &["resume", "s0", "--prompt", "next", "--data-dir", &d_leg]).status.code(),
        Some(4)
    );

    // 4. --route cli session -> exit 2 and zero ledger mutations
    let cli_dir = TempDir::new("cli-route");
    let ledger = Ledger::open(&cli_dir.0).unwrap();
    ledger.append_with_hash("s-cli1", "session.start", 1, &json!({ "route": "claude-code-cli" })).unwrap();
    let d_cli = cli_dir.0.to_string_lossy().into_owned();
    let res_cli = run_cli(Some(("ANTHROPIC_API_KEY", TEST_KEY)), &["resume", "s-cli1", "--prompt", "next", "--data-dir", &d_cli]);
    assert_eq!(res_cli.status.code(), Some(2));
    assert_eq!(ledger.read_session("s-cli1").unwrap().len(), 1);

    // 5. Live .active.lock (held by current test PID) -> exit 2, then stale PID lock -> recovers cleanly (exit 0)
    let lock_dir = TempDir::new("locks");
    let d_lk = lock_dir.0.to_string_lossy().into_owned();
    let srv_init = MockServer::start(vec![MockReply { status: 200, body: anthropic_text("claude-haiku-4-5", "ok") }]);
    let run_l = run_cli(
        Some(("ANTHROPIC_API_KEY", TEST_KEY)),
        &["run", "--route", "api", "--prompt", "init", "--data-dir", &d_lk, "--api-base", &srv_init.base_url],
    );
    assert_eq!(run_l.status.code(), Some(0));
    let _ = srv_init.take_requests();
    let s_lock = parse_ndjson(&run_l.stdout)[0]["session"].as_str().unwrap().to_string();
    let lock_path = lock_dir.0.join("sessions").join(&s_lock).join(".active.lock");
    std::fs::write(&lock_path, std::process::id().to_string()).unwrap();

    let count_before_lock = Ledger::open_existing(&lock_dir.0).unwrap().read_session(&s_lock).unwrap().len();
    let res_live = run_cli(
        Some(("ANTHROPIC_API_KEY", TEST_KEY)),
        &["resume", &s_lock, "--prompt", "blocked", "--data-dir", &d_lk, "--api-base", "http://127.0.0.1:1"],
    );
    assert_eq!(res_live.status.code(), Some(2));
    assert_eq!(Ledger::open_existing(&lock_dir.0).unwrap().read_session(&s_lock).unwrap().len(), count_before_lock);

    std::fs::write(&lock_path, u32::MAX.to_string()).unwrap();
    let srv_res = MockServer::start(vec![MockReply { status: 200, body: anthropic_text("claude-haiku-4-5", "recovered") }]);
    let res_stale = run_cli(
        Some(("ANTHROPIC_API_KEY", TEST_KEY)),
        &["resume", &s_lock, "--prompt", "recover", "--data-dir", &d_lk, "--api-base", &srv_res.base_url],
    );
    assert_eq!(res_stale.status.code(), Some(0), "stderr:\n{}", String::from_utf8_lossy(&res_stale.stderr));
    let _ = srv_res.take_requests();
    assert!(!lock_path.exists());
    assert_eq!(verify_cli(&lock_dir.0, Some(&s_lock)).status.code(), Some(0));
}
