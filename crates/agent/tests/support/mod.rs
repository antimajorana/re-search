//! Shared local mock server, approver, worker epoch, and ledger helpers for
//! `crates/agent` integration tests.

use agent::{
    AgentConfig, AnthropicConfig, AnthropicTransport, EventSink, SecretKey, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
    DEFAULT_SYSTEM_PROMPT,
};
use ledger::{AppendedEvent, Ledger};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tools::builtin;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallCtx, Registry, Router, WorkerEpoch};

pub struct MockServer {
    pub base_url: String,
    received: Arc<Mutex<Vec<Vec<u8>>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    pub fn start(responses: Vec<(&'static str, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let rec_clone = received.clone();
        let mut queue = VecDeque::from(responses);

        let handle = thread::spawn(move || {
            while let Some((status, body)) = queue.pop_front() {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut content_len: usize = 0;
                loop {
                    line.clear();
                    let n = reader.read_line(&mut line).unwrap();
                    if n == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some((k, v)) = line.trim_end_matches(['\r', '\n']).split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut req_body = vec![0u8; content_len];
                if content_len > 0 {
                    reader.read_exact(&mut req_body).unwrap();
                }
                rec_clone.lock().unwrap().push(req_body);

                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });

        Self { base_url: format!("http://{addr}"), received, handle: Some(handle) }
    }

    pub fn requests_received(&self) -> Vec<Vec<u8>> {
        self.received.lock().unwrap().clone()
    }

    pub fn finish(mut self) {
        if let Some(h) = self.handle.take() {
            h.join().unwrap();
        }
    }
}

pub struct MockApprover {
    approve: bool,
    asked: AtomicUsize,
}

impl MockApprover {
    pub fn new(approve: bool) -> Self {
        Self { approve, asked: AtomicUsize::new(0) }
    }

    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

impl Approver for MockApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        self.asked.fetch_add(1, Ordering::SeqCst);
        ApprovalReply {
            approved: self.approve,
            digest: request.digest.clone(),
            by: "mock-approver".into(),
        }
    }
}

pub struct MockEpoch {
    epoch: AtomicU64,
    contained: AtomicBool,
}

impl MockEpoch {
    pub fn new(epoch: u64, contained: bool) -> Self {
        Self { epoch: AtomicU64::new(epoch), contained: AtomicBool::new(contained) }
    }

    pub fn set_epoch(&self, epoch: u64) {
        self.epoch.store(epoch, Ordering::Release);
    }
}

impl WorkerEpoch for MockEpoch {
    fn live_epoch(&self) -> Option<u64> {
        match self.epoch.load(Ordering::Acquire) {
            0 => None,
            n => Some(n),
        }
    }

    fn is_contained(&self) -> bool {
        self.contained.load(Ordering::Acquire)
    }
}

pub struct RecordingSink {
    pub events: Vec<(u64, String, Value)>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl EventSink for RecordingSink {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String> {
        self.events.push((receipt.seq, kind.to_string(), data.clone()));
        Ok(())
    }
}

pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("agent-loop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let blobs = self.path.join("blobs");
        if blobs.is_dir() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&blobs, std::fs::Permissions::from_mode(0o755));
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn default_cfg(prompt: &str, contained: bool) -> AgentConfig {
    AgentConfig {
        prompt: prompt.into(),
        model_requested: "haiku".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
        max_turns: DEFAULT_MAX_TURNS,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: if contained { "appcontainer".into() } else { "none".into() },
        contained,
        reasoning_effort: None,
    }
}

pub fn make_transport(base_url: String, secret: &str) -> AnthropicTransport {
    AnthropicTransport::new(
        AnthropicConfig { base_url, ..Default::default() },
        SecretKey::new(secret.into()).unwrap(),
        "RE_SEARCH_API_KEY",
    )
    .unwrap()
}

pub fn make_router(
    dir: &Path,
    session: &str,
    approver: Arc<dyn Approver>,
    epoch: Arc<MockEpoch>,
    runs: Arc<AtomicUsize>,
) -> Router {
    let mut reg = Registry::new();
    reg.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
    reg.register_worker_session(
        builtin::sympy_eval_def(),
        epoch.clone(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            runs.fetch_add(1, Ordering::SeqCst);
            let ep = epoch.live_epoch().ok_or("no live worker epoch")?;
            if let Some(req) = ctx.required_epoch {
                if req != ep {
                    return Err(format!("epoch mismatch: {req} != {ep}"));
                }
            }
            ctx.record_worker_epoch(ep);
            Ok(json!({ "value": format!("ok:{}", args["code"].as_str().unwrap_or("")) }))
        }),
    );
    Router::new(reg, builtin::default_policy(), Ledger::open(dir).unwrap(), session, approver)
}

pub fn ok_msg(id: &str, stop_reason: &str, content: Value) -> (&'static str, Vec<u8>) {
    let body = json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": stop_reason,
        "content": content,
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    });
    ("200 OK", serde_json::to_vec(&body).unwrap())
}

pub fn assert_secret_absent(dir: &Path, session: &str, sink: &RecordingSink, secret: &str) {
    let ledger = Ledger::open_existing(dir).unwrap();
    for ev in ledger.read_session(session).unwrap() {
        let s = ev.payload.to_string();
        assert!(!s.contains(secret), "secret leaked in event {} ({}): {s}", ev.seq, ev.kind);
    }
    for db_name in ["ledger.db", "ledger.db-wal"] {
        let p = dir.join(db_name);
        if p.is_file() {
            let bytes = std::fs::read(&p).unwrap();
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "secret leaked in {db_name}"
            );
        }
    }
    let blobs_dir = dir.join("blobs");
    if blobs_dir.is_dir() {
        for entry in std::fs::read_dir(&blobs_dir).unwrap() {
            let sub = entry.unwrap().path();
            if sub.is_dir() {
                for f in std::fs::read_dir(sub).unwrap() {
                    let bytes = std::fs::read(f.unwrap().path()).unwrap();
                    assert!(
                        !String::from_utf8_lossy(&bytes).contains(secret),
                        "secret leaked in blob"
                    );
                }
            }
        }
    }
    for (seq, kind, data) in &sink.events {
        assert!(!data.to_string().contains(secret), "secret leaked in sink seq {seq} ({kind})");
    }
}

pub fn legacy_v0_db_bytes() -> Vec<u8> {
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
    buf
}
