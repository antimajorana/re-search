//! Loopback TCP approval relay between `core.exe` and the person.
//!
//! `core.exe` is a child of the vendor CLI with no console of its own, so its router asks
//! for approvals over a loopback connection to the supervisor. This module owns the wire
//! handling: version, session token, session, scope and containment checks; the
//! `approval.answered` / `approval.delivered` / `approval.undelivered` /
//! `approval.expired` bookkeeping; and the ACK. Who decides is a [`RelayDecider`]: the
//! console prompt (`crates/cli`) or the desktop approval card (`crates/desktop-server`).
//! There is no decider that approves on its own.

use crate::EVENT_VERSION;
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tools::APPROVAL_CHANNEL_VERSION;

const MAX_REQUEST_BYTES: u64 = 65_536;
const MAX_ACK_BYTES: u64 = 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

/// A request that passed the channel checks and needs a person's decision.
pub struct RelayAsk<'a> {
    /// The caller's connection; poll it with [`peek_peer_state`] while waiting.
    pub peer: &'a TcpStream,
    pub session: &'a str,
    /// The `tools::ApprovalRequest` as sent by `core.exe`.
    pub request: &'a Value,
    pub scope: &'a str,
    pub containment: &'a str,
    pub contained: bool,
}

/// What a decider concluded.
pub enum RelayDecision {
    /// A decision the relay records as `approval.answered` before replying.
    Answered { approved: bool, by: String },
    /// A decision the decider already recorded as `approval.answered` itself, failing
    /// closed (`approved: false`) if it could not.
    Recorded { approved: bool, by: String },
    /// The caller went away (or broke protocol) before a decision: `approval.expired`.
    Expired(String),
}

/// Takes the decision for one validated request.
pub trait RelayDecider {
    fn decide(&self, ask: &RelayAsk<'_>) -> RelayDecision;
}

/// The running listener. Dropping it stops accepting new connections (a request already
/// being decided finishes first).
pub struct RelayServer {
    pub addr: String,
    stop: Arc<AtomicBool>,
}

impl Drop for RelayServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake the blocking accept so the thread can see the flag.
        let _ = TcpStream::connect_timeout(&self.addr.parse().expect("relay address"), Duration::from_millis(200));
    }
}

/// Bind `127.0.0.1` on a free port and answer approval requests on a background thread,
/// one at a time.
pub fn start(
    ledger: Ledger,
    session: String,
    token: String,
    decider: Arc<dyn RelayDecider + Send + Sync>,
) -> io::Result<RelayServer> {
    start_inner(ledger, session, token, None, decider)
}

/// Bind `127.0.0.1` on a free port like [`start`], and additionally verify that every
/// request's `caller` matches `expected_caller`.
pub fn start_for_caller(
    ledger: Ledger,
    session: String,
    token: String,
    expected_caller: String,
    decider: Arc<dyn RelayDecider + Send + Sync>,
) -> io::Result<RelayServer> {
    start_inner(ledger, session, token, Some(vec![expected_caller]), decider)
}

/// Bind `127.0.0.1` on a free port like [`start`], and require every request's `caller`
/// to appear in `expected_callers` (for example, the vendor CLI caller plus `host:check`
/// for Solve S9 auto-checks).
pub fn start_for_callers(
    ledger: Ledger,
    session: String,
    token: String,
    expected_callers: Vec<String>,
    decider: Arc<dyn RelayDecider + Send + Sync>,
) -> io::Result<RelayServer> {
    start_inner(ledger, session, token, Some(expected_callers), decider)
}

fn start_inner(
    ledger: Ledger,
    session: String,
    token: String,
    expected_callers: Option<Vec<String>>,
    decider: Arc<dyn RelayDecider + Send + Sync>,
) -> io::Result<RelayServer> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    std::thread::Builder::new().name(format!("approval-relay-{session}")).spawn(move || {
        for stream in listener.incoming() {
            if thread_stop.load(Ordering::Acquire) {
                break;
            }
            let Ok(stream) = stream else { continue };
            if let Err(e) = handle_stream_checked_callers(
                stream,
                &token,
                &session,
                expected_callers.as_deref(),
                &ledger,
                &*decider,
            ) {
                eprintln!("[supervisor] approval request failed: {e}");
            }
        }
    })?;
    Ok(RelayServer { addr, stop })
}

pub enum PeerState {
    Waiting,
    Closed(&'static str),
    ProtocolError(&'static str),
}

/// Non-blocking look at the caller's socket: still waiting, gone, or sending bytes it
/// should not send before an answer.
pub fn peek_peer_state(stream: &TcpStream) -> PeerState {
    if stream.set_nonblocking(true).is_err() {
        return PeerState::Closed("could not inspect approval socket");
    }
    let mut buf = [0u8; 1];
    let res = stream.peek(&mut buf);
    let _ = stream.set_nonblocking(false);
    match res {
        Ok(0) => PeerState::Closed("caller disconnected before answer"),
        Ok(_) => PeerState::ProtocolError("unexpected extra bytes from caller before answer"),
        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {
            PeerState::Waiting
        }
        Err(_) => PeerState::Closed("caller disconnected before answer"),
    }
}

/// Handle one connection: read the request, check it, ask `decider` only if every check
/// passes, record, reply, and record whether `core.exe` acknowledged the reply.
pub fn handle_stream(
    stream: TcpStream,
    token: &str,
    session: &str,
    ledger: &Ledger,
    decider: &dyn RelayDecider,
) -> Result<(), String> {
    handle_stream_checked(stream, token, session, None, ledger, decider)
}

/// Like [`handle_stream`], and when `expected_caller` is `Some`, also require
/// `request["caller"]` to equal it.
pub fn handle_stream_checked(
    stream: TcpStream,
    token: &str,
    session: &str,
    expected_caller: Option<&str>,
    ledger: &Ledger,
    decider: &dyn RelayDecider,
) -> Result<(), String> {
    let callers = expected_caller.map(|c| vec![c.to_string()]);
    handle_stream_checked_callers(stream, token, session, callers.as_deref(), ledger, decider)
}

/// Like [`handle_stream`], and when `expected_callers` is `Some`, require
/// `request["caller"]` to equal one of the listed callers.
pub fn handle_stream_checked_callers(
    stream: TcpStream,
    token: &str,
    session: &str,
    expected_callers: Option<&[String]>,
    ledger: &Ledger,
    decider: &dyn RelayDecider,
) -> Result<(), String> {
    stream.set_read_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;

    let mut line = String::new();
    let n = BufReader::new((&stream).take(MAX_REQUEST_BYTES)).read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("connection closed before request".into());
    }
    if !line.ends_with('\n') && n as u64 >= MAX_REQUEST_BYTES {
        return Err("approval request exceeded 64 KiB limit".into());
    }
    let message: Value = serde_json::from_str(&line).map_err(|e| format!("bad request: {e}"))?;
    let request = &message["request"];
    let call_id = request["call_id"].as_str().unwrap_or_default().to_string();
    let digest = request["digest"].as_str().unwrap_or_default().to_string();
    let scope = request["scope"].as_str().unwrap_or("");
    let valid_scope = matches!(scope, "once" | "worker-session");
    let contained = message["contained"] == json!(true);

    let caller_ok = expected_callers.is_none_or(|allowed| {
        request["caller"]
            .as_str()
            .is_some_and(|c| allowed.iter().any(|a| a == c))
    });

    let refuse = |by: &str| RelayDecision::Answered { approved: false, by: by.to_string() };
    let decision = if message["v"] != json!(APPROVAL_CHANNEL_VERSION) {
        refuse("supervisor: unknown channel version")
    } else if message["token"].as_str() != Some(token) {
        refuse("supervisor: wrong session token")
    } else if request["session"].as_str() != Some(session) {
        refuse("supervisor: request is for another session")
    } else if !caller_ok {
        refuse("supervisor: caller identity mismatch")
    } else if !valid_scope {
        refuse("supervisor: unknown or missing approval scope")
    } else if scope == "worker-session" && !contained {
        refuse("supervisor: worker-session requires a contained sandbox")
    } else {
        let ask = RelayAsk {
            peer: &stream,
            session,
            request,
            scope,
            containment: message["containment"].as_str().unwrap_or("unknown"),
            contained,
        };
        decider.decide(&ask)
    };

    let (approved, by) = match decision {
        RelayDecision::Expired(reason) => return record_expired(ledger, session, &call_id, &digest, scope, &reason),
        RelayDecision::Recorded { approved, by } => (approved, by),
        RelayDecision::Answered { approved, by } => {
            let payload = json!({ "call_id": call_id, "digest": digest, "scope": scope, "approved": approved, "by": by });
            if let Err(e) = ledger.append(session, "approval.answered", EVENT_VERSION, &payload) {
                let fail = json!({ "v": APPROVAL_CHANNEL_VERSION, "approved": false, "digest": digest, "by": "supervisor: ledger error" });
                let _ = (&stream).write_all(format!("{fail}\n").as_bytes());
                let _ = (&stream).flush();
                return Err(format!("ledger could not record approval.answered: {e}"));
            }
            (approved, by)
        }
    };

    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;

    let reply = json!({ "v": APPROVAL_CHANNEL_VERSION, "approved": approved, "digest": digest, "by": by });
    if let Err(e) = (&stream).write_all(format!("{reply}\n").as_bytes()).and_then(|_| (&stream).flush()) {
        if approved {
            eprintln!("[supervisor] WARNING: could not deliver approval to core: {e}");
        }
        return record_undelivered(ledger, session, &call_id, &digest, "reply write failed");
    }

    let mut ack_line = String::new();
    let ack_ok = match BufReader::new((&stream).take(MAX_ACK_BYTES)).read_line(&mut ack_line) {
        Ok(0) | Err(_) => false,
        Ok(_) => serde_json::from_str::<Value>(&ack_line)
            .is_ok_and(|v| v["v"] == json!(APPROVAL_CHANNEL_VERSION) && v["ack"] == json!(true)),
    };
    if ack_ok {
        let p = json!({ "call_id": request["call_id"], "digest": digest });
        ledger
            .append(session, "approval.delivered", EVENT_VERSION, &p)
            .map_err(|e| format!("ledger could not record approval.delivered: {e}"))?;
    } else {
        if approved {
            eprintln!("[supervisor] WARNING: approval was not acknowledged by core (request may have timed out)");
        }
        record_undelivered(ledger, session, &call_id, &digest, "missing or invalid ack from caller")?;
    }
    Ok(())
}

fn record_expired(ledger: &Ledger, session: &str, call_id: &str, digest: &str, scope: &str, reason: &str) -> Result<(), String> {
    let payload = json!({ "call_id": call_id, "digest": digest, "scope": scope, "reason": reason });
    ledger
        .append(session, "approval.expired", EVENT_VERSION, &payload)
        .map(|_| ())
        .map_err(|e| format!("ledger could not record approval.expired: {e}"))
}

fn record_undelivered(ledger: &Ledger, session: &str, call_id: &str, digest: &str, reason: &str) -> Result<(), String> {
    let payload = json!({ "call_id": call_id, "digest": digest, "reason": reason });
    ledger
        .append(session, "approval.undelivered", EVENT_VERSION, &payload)
        .map(|_| ())
        .map_err(|e| format!("ledger could not record approval.undelivered: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Counting(Mutex<u32>, RelayDecisionKind);
    #[derive(Clone, Copy)]
    enum RelayDecisionKind {
        Yes,
        RecordedNo,
    }
    impl RelayDecider for Counting {
        fn decide(&self, _ask: &RelayAsk<'_>) -> RelayDecision {
            *self.0.lock().unwrap() += 1;
            match self.1 {
                RelayDecisionKind::Yes => RelayDecision::Answered { approved: true, by: "test person".into() },
                RelayDecisionKind::RecordedNo => RelayDecision::Recorded { approved: false, by: "recorded elsewhere".into() },
            }
        }
    }

    fn request(session: &str) -> Value {
        json!({ "schema_version": tools::APPROVAL_SCHEMA_VERSION, "session": session, "call_id": "c1",
            "digest": "d".repeat(64), "caller": "x", "tool_id": "sympy_eval", "scope": "once",
            "effect": "execute-code", "canonical_args": "{}", "policy_reason": "r" })
    }

    /// One exchange; then the ledger's kinds once the relay recorded the delivery outcome
    /// (polled up to a deadline, not a fixed sleep).
    fn exchange(msg: Value, decider: Arc<Counting>) -> (Value, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("cli-route-relay-{}-{}", std::process::id(), rand_tag()));
        let _ = std::fs::remove_dir_all(&dir);
        let srv = start(Ledger::open(&dir).unwrap(), "s1".into(), "tok".into(), decider).unwrap();
        let mut c = TcpStream::connect(&srv.addr).unwrap();
        writeln!(c, "{msg}").unwrap();
        let mut reply = String::new();
        BufReader::new(&mut c).read_line(&mut reply).unwrap();
        writeln!(c, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
        drop(c);
        let read = || -> Vec<String> { Ledger::open(&dir).unwrap().read_session("s1").unwrap().into_iter().map(|e| e.kind).collect() };
        let done = |k: &[String]| k.iter().any(|k| k == "approval.delivered" || k == "approval.undelivered");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut kinds = read();
        while !done(&kinds) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            kinds = read();
        }
        drop(srv);
        let _ = std::fs::remove_dir_all(&dir);
        (serde_json::from_str(&reply).unwrap(), kinds)
    }

    fn rand_tag() -> String {
        tools::canon::random_hex(4)
    }

    #[test]
    fn decider_is_asked_only_after_the_checks_pass() {
        let d = Arc::new(Counting(Mutex::new(0), RelayDecisionKind::Yes));
        let bad = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "wrong", "request": request("s1") });
        let (reply, kinds) = exchange(bad, Arc::clone(&d));
        assert_eq!((reply["approved"].clone(), reply["by"].clone()), (json!(false), json!("supervisor: wrong session token")));
        assert_eq!(kinds, ["approval.answered", "approval.delivered"]);
        assert_eq!(*d.0.lock().unwrap(), 0);

        let good = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": request("s1") });
        let (reply, kinds) = exchange(good, Arc::clone(&d));
        assert_eq!((reply["approved"].clone(), reply["by"].clone()), (json!(true), json!("test person")));
        assert_eq!(kinds, ["approval.answered", "approval.delivered"]);
        assert_eq!(*d.0.lock().unwrap(), 1);
    }

    #[test]
    fn recorded_decisions_are_not_recorded_twice() {
        let d = Arc::new(Counting(Mutex::new(0), RelayDecisionKind::RecordedNo));
        let msg = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": request("s1") });
        let (reply, kinds) = exchange(msg, d);
        assert_eq!(reply["approved"], false);
        assert_eq!(kinds, ["approval.delivered"]);
    }

    fn exchange_checked(msg: Value, expected_caller: Option<&str>, decider: &Counting) -> Value {
        let dir = std::env::temp_dir().join(format!("cli-route-relay-chk-{}-{}", std::process::id(), rand_tag()));
        let _ = std::fs::remove_dir_all(&dir);
        let ledger = Ledger::open(&dir).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            writeln!(c, "{msg}").unwrap();
            let mut reply = String::new();
            BufReader::new(&mut c).read_line(&mut reply).unwrap();
            writeln!(c, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
            serde_json::from_str::<Value>(&reply).unwrap()
        });
        let (server_stream, _) = listener.accept().unwrap();
        handle_stream_checked(server_stream, "tok", "s1", expected_caller, &ledger, decider).unwrap();
        let reply = client.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        reply
    }

    #[test]
    fn handle_stream_checked_enforces_expected_caller() {
        let d = Counting(Mutex::new(0), RelayDecisionKind::Yes);
        let mut req = request("s1");
        req["caller"] = json!("claude-code-cli:s1");

        // Matching caller reaches decider and is approved.
        let ok = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": req });
        let reply = exchange_checked(ok, Some("claude-code-cli:s1"), &d);
        assert_eq!(reply["approved"], true);
        assert_eq!(*d.0.lock().unwrap(), 1);

        // Mismatched caller is refused without asking decider.
        let mut wrong_req = request("s1");
        wrong_req["caller"] = json!("antigravity:s1");
        let wrong = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": wrong_req });
        let reply = exchange_checked(wrong, Some("claude-code-cli:s1"), &d);
        assert_eq!(reply["approved"], false);
        assert_eq!(reply["by"], "supervisor: caller identity mismatch");
        assert_eq!(*d.0.lock().unwrap(), 1);

        // Missing caller is refused without asking decider.
        let mut missing_req = request("s1");
        missing_req.as_object_mut().unwrap().remove("caller");
        let missing = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": missing_req });
        let reply = exchange_checked(missing, Some("claude-code-cli:s1"), &d);
        assert_eq!(reply["approved"], false);
        assert_eq!(reply["by"], "supervisor: caller identity mismatch");
        assert_eq!(*d.0.lock().unwrap(), 1);
    }

    #[test]
    fn handle_stream_checked_callers_allows_solve_host_check_and_rejects_others() {
        let d = Counting(Mutex::new(0), RelayDecisionKind::Yes);
        let allowed = vec!["claude-code-cli:s1".to_string(), "host:check".to_string()];
        let run_one = |caller: &str| -> Value {
            let dir = std::env::temp_dir().join(format!("cli-route-relay-multi-{}-{}", std::process::id(), rand_tag()));
            let _ = std::fs::remove_dir_all(&dir);
            let ledger = Ledger::open(&dir).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let mut req = request("s1");
            req["caller"] = json!(caller);
            let msg = json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": req });
            let client = std::thread::spawn(move || {
                let mut c = TcpStream::connect(addr).unwrap();
                writeln!(c, "{msg}").unwrap();
                let mut reply = String::new();
                BufReader::new(&mut c).read_line(&mut reply).unwrap();
                writeln!(c, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
                serde_json::from_str::<Value>(&reply).unwrap()
            });
            let (server_stream, _) = listener.accept().unwrap();
            handle_stream_checked_callers(server_stream, "tok", "s1", Some(&allowed), &ledger, &d).unwrap();
            let reply = client.join().unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            reply
        };

        assert_eq!(run_one("claude-code-cli:s1")["approved"], true);
        assert_eq!(run_one("host:check")["approved"], true);
        let bad = run_one("spoofed:s1");
        assert_eq!(bad["approved"], false);
        assert_eq!(bad["by"], "supervisor: caller identity mismatch");
        assert_eq!(*d.0.lock().unwrap(), 2);
    }
}
