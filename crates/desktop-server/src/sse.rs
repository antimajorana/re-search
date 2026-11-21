//! Thread-safe per-session SSE pub/sub hub and TCP stream writer.

use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const POLL_STEP: Duration = Duration::from_millis(250);

/// A single Server-Sent Events frame or control signal.
#[derive(Debug, Clone)]
pub enum SseFrame {
    Event {
        id: u64,
        kind: String,
        data: String,
    },
    Shutdown,
}

impl SseFrame {
    /// Formats this frame on the wire according to the SSE specification.
    pub fn format_wire(&self) -> Option<String> {
        match self {
            SseFrame::Event { id, kind, data } => {
                let mut out = format!("id: {id}\nevent: {kind}\n");
                for line in data.lines() {
                    out.push_str("data: ");
                    out.push_str(line);
                    out.push('\n');
                }
                if data.is_empty() {
                    out.push_str("data: {}\n");
                }
                out.push('\n');
                Some(out)
            }
            SseFrame::Shutdown => None,
        }
    }
}

struct Subscriber {
    sub_id: u64,
    tx: Sender<SseFrame>,
}

/// Per-session broadcast hub for live SSE updates.
pub struct SseHub {
    next_event_id: AtomicU64,
    next_sub_id: AtomicU64,
    shutdown: AtomicBool,
    subs: Mutex<HashMap<String, Vec<Subscriber>>>,
}

impl Default for SseHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SseHub {
    /// Creates an empty SSE hub.
    pub fn new() -> Self {
        Self {
            next_event_id: AtomicU64::new(1),
            next_sub_id: AtomicU64::new(1),
            shutdown: AtomicBool::new(false),
            subs: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribes to live events for `session_id`, returning `(sub_id, rx)`.
    pub fn subscribe(&self, session_id: &str) -> (u64, Receiver<SseFrame>) {
        let sub_id = self.next_sub_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        if self.shutdown.load(Ordering::Acquire) {
            let _ = tx.send(SseFrame::Shutdown);
            return (sub_id, rx);
        }
        if let Ok(mut map) = self.subs.lock() {
            map.entry(session_id.to_string())
                .or_default()
                .push(Subscriber { sub_id, tx });
        }
        (sub_id, rx)
    }

    /// Removes a specific subscriber handle.
    pub fn unsubscribe(&self, session_id: &str, sub_id: u64) {
        if let Ok(mut map) = self.subs.lock() {
            if let Some(list) = map.get_mut(session_id) {
                list.retain(|s| s.sub_id != sub_id);
                if list.is_empty() {
                    map.remove(session_id);
                }
            }
        }
    }

    /// Broadcasts an SSE event with JSON payload `data` to subscribers of `session_id`.
    pub fn emit(&self, session_id: &str, kind: &str, data: Value) {
        let id = self.next_event_id.fetch_add(1, Ordering::Relaxed);
        let serialized = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
        let frame = SseFrame::Event {
            id,
            kind: kind.to_string(),
            data: serialized,
        };
        let Ok(mut map) = self.subs.lock() else {
            return;
        };
        if let Some(list) = map.get_mut(session_id) {
            list.retain(|s| s.tx.send(frame.clone()).is_ok());
            if list.is_empty() {
                map.remove(session_id);
            }
        }
    }

    /// Signals all connected SSE streams to close cleanly.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut map) = self.subs.lock() {
            for (_, list) in map.drain() {
                for sub in list {
                    let _ = sub.tx.send(SseFrame::Shutdown);
                }
            }
        }
    }

    /// Returns true once shutdown has been triggered.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
}

/// Streams HTTP SSE headers, an initial comment, live frames, and periodic keepalives.
pub fn stream_session_sse(
    mut stream: TcpStream,
    hub: &Arc<SseHub>,
    session_id: &str,
    shutdown: &Arc<AtomicBool>,
) {
    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream; charset=utf-8\r\n",
        "Cache-Control: no-cache\r\n",
        "X-Accel-Buffering: no\r\n",
        "Connection: close\r\n\r\n",
        ": connected\n\n"
    );
    // Subscribe first: once a client has read ": connected", no later event is missed.
    let (sub_id, rx) = hub.subscribe(session_id);
    if stream.write_all(headers.as_bytes()).is_err() || stream.flush().is_err() {
        hub.unsubscribe(session_id, sub_id);
        return;
    }
    let mut last_ping = Instant::now();

    loop {
        if shutdown.load(Ordering::Acquire) || hub.is_shutdown() {
            break;
        }
        match rx.recv_timeout(POLL_STEP) {
            Ok(SseFrame::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(frame) => {
                if let Some(wire) = frame.format_wire() {
                    if stream.write_all(wire.as_bytes()).is_err() || stream.flush().is_err() {
                        break;
                    }
                    last_ping = Instant::now();
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if last_ping.elapsed() >= KEEPALIVE_INTERVAL {
                    if stream.write_all(b": keepalive\n\n").is_err() || stream.flush().is_err() {
                        break;
                    }
                    last_ping = Instant::now();
                }
            }
        }
    }

    hub.unsubscribe(session_id, sub_id);
}
