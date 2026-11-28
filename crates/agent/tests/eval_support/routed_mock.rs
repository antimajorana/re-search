//! Loopback mock for recorded service responses (fixed tier, section 7.1).
//!
//! Routes by method + path + query to a recorded body. A request that no
//! route matches gets 404 and is kept as "unexpected", so a test can
//! assert that the code under test asked for nothing outside the
//! recording. Point the literature tools at it with
//! `research_tools::LiteratureEndpoints::loopback_mock(&mock.base_url)`.
//!
//! `support::MockServer` (a queue of bodies in arrival order) stays the
//! model-side mock; it cannot serve this purpose because it does not look
//! at the request line.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const RECORDING_FORMAT_VERSION: u32 = 1;

/// One recording file: `tests/fixtures/eval/recorded/<name>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub format_version: u32,
    /// Which service, when and how it was recorded, and by whom.
    pub notes: String,
    pub routes: Vec<RecordedRoute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedRoute {
    pub method: String,
    pub path: String,
    /// Query string without `?`; the order of its `&` parts does not matter.
    #[serde(default)]
    pub query: String,
    pub status: u16,
    pub content_type: String,
    /// The body inline, or
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// a file next to the recording (relative to the recording's directory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenRequest {
    pub method: String,
    /// Path and query exactly as sent.
    pub target: String,
}

#[derive(Debug, Clone)]
struct Reply {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

type RouteKey = (String, String, String);

fn normal_query(query: &str) -> String {
    let mut parts: Vec<&str> = query.split('&').filter(|p| !p.is_empty()).collect();
    parts.sort_unstable();
    parts.join("&")
}

fn key(method: &str, target: &str) -> RouteKey {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (method.to_ascii_uppercase(), path.to_string(), normal_query(query))
}

/// A set of routes, built in code or loaded from a recording file.
#[derive(Debug, Clone, Default)]
pub struct Routes {
    table: BTreeMap<RouteKey, Reply>,
}

impl Routes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one route; `target` is the path with its query, such as
    /// `/api/query?id_list=1&max_results=1`. Adding the same route twice is an error.
    pub fn add(&mut self, method: &str, target: &str, status: u16, content_type: &str, body: &[u8]) -> Result<(), String> {
        if !target.starts_with('/') {
            return Err(format!("route `{target}` must start with `/`"));
        }
        let reply = Reply { status, content_type: content_type.to_string(), body: body.to_vec() };
        match self.table.insert(key(method, target), reply) {
            None => Ok(()),
            Some(_) => Err(format!("route `{method} {target}` is recorded twice")),
        }
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Loads a recording file; `body_file` entries are read relative to its directory.
    pub fn load(recording_path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(recording_path).map_err(|e| format!("cannot read {}: {e}", recording_path.display()))?;
        let rec: Recording = serde_json::from_str(&text).map_err(|e| format!("{} does not parse: {e}", recording_path.display()))?;
        if rec.format_version != RECORDING_FORMAT_VERSION {
            return Err(format!("{}: format_version {} is not {RECORDING_FORMAT_VERSION}", recording_path.display(), rec.format_version));
        }
        let dir = recording_path.parent().unwrap_or(Path::new("."));
        let mut routes = Routes::new();
        for r in &rec.routes {
            let body = match (&r.body, &r.body_file) {
                (Some(b), None) => b.clone().into_bytes(),
                (None, Some(f)) => {
                    if f.contains("..") || f.contains('\\') || f.contains(':') || f.starts_with('/') {
                        return Err(format!("body_file `{f}` must be a plain relative path"));
                    }
                    std::fs::read(dir.join(f)).map_err(|e| format!("cannot read body_file `{f}`: {e}"))?
                }
                _ => return Err(format!("route `{} {}` needs exactly one of `body` and `body_file`", r.method, r.path)),
            };
            let target = if r.query.is_empty() { r.path.clone() } else { format!("{}?{}", r.path, r.query) };
            routes.add(&r.method, &target, r.status, &r.content_type, &body)?;
        }
        Ok(routes)
    }
}

pub struct RoutedMock {
    pub base_url: String,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
    unexpected: Arc<Mutex<Vec<SeenRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RoutedMock {
    pub fn start(routes: Routes) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let unexpected = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_t, unexpected_t, stop_t) = (seen.clone(), unexpected.clone(), stop.clone());

        let handle = thread::spawn(move || {
            while !stop_t.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = serve_one(stream, &routes, &seen_t, &unexpected_t);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });
        Self { base_url: format!("http://{addr}"), seen, unexpected, stop, handle: Some(handle) }
    }

    /// Every request received, in order.
    pub fn requests(&self) -> Vec<SeenRequest> {
        self.seen.lock().unwrap().clone()
    }

    /// Requests that matched no route (each was answered 404).
    pub fn unexpected(&self) -> Vec<SeenRequest> {
        self.unexpected.lock().unwrap().clone()
    }

    pub fn assert_no_unexpected(&self) {
        let u = self.unexpected();
        assert!(u.is_empty(), "requests outside the recording: {u:?}");
    }
}

impl Drop for RoutedMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve_one(
    mut stream: TcpStream,
    routes: &Routes,
    seen: &Mutex<Vec<SeenRequest>>,
    unexpected: &Mutex<Vec<SeenRequest>>,
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or("").to_string();
    let target = words.next().unwrap_or("/").to_string();
    let mut content_len = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.trim_end().split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_len];
    reader.read_exact(&mut body)?;

    let request = SeenRequest { method: method.clone(), target: target.clone() };
    seen.lock().unwrap().push(request.clone());
    let reply = match routes.table.get(&key(&method, &target)) {
        Some(r) => r.clone(),
        None => {
            unexpected.lock().unwrap().push(request);
            Reply { status: 404, content_type: "text/plain".into(), body: b"not in the recording".to_vec() }
        }
    };
    let head = format!(
        "HTTP/1.1 {} Recorded\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&reply.body)?;
    stream.flush()
}
