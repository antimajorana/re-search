//! Loopback TCP approval listener for the `--route cli` MCP server.
//!
//! When the caller (`core`) times out and drops its socket while a prompt is
//! on screen, the prompt is closed cleanly and `approval.expired` is recorded
//! instead of leaving a stale prompt open.

use super::{stdin_hub, wait_for_decision, PromptBanner, PromptOutcome, TerminalInput};
use crate::events::EVENT_VERSION;
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;
use tools::APPROVAL_CHANNEL_VERSION;

const MAX_REQUEST_BYTES: u64 = 65_536;
const MAX_ACK_BYTES: u64 = 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ApprovalServer {
    pub addr: String,
}

/// Bind 127.0.0.1 on a free port and answer approval requests on a
/// background thread, one at a time.
pub fn start(data_dir: PathBuf, session: String, token: String) -> io::Result<ApprovalServer> {
    let ledger = Ledger::open(&data_dir).map_err(|e| io::Error::other(e.to_string()))?;
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        let _ = stdin_hub();
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Err(e) = handle_stream(stream, &token, &session, &ledger, TerminalInput::System) {
                eprintln!("[supervisor] approval request failed: {e}");
            }
        }
    });
    Ok(ApprovalServer { addr })
}

pub(super) enum PeerState {
    Waiting,
    Closed(&'static str),
    ProtocolError(&'static str),
}

pub(super) fn peek_peer_state(stream: &TcpStream) -> PeerState {
    if stream.set_nonblocking(true).is_err() {
        return PeerState::Closed("could not inspect approval socket");
    }
    let mut buf = [0u8; 1];
    let res = stream.peek(&mut buf);
    let _ = stream.set_nonblocking(false);
    match res {
        Ok(0) => PeerState::Closed("caller disconnected before answer"),
        Ok(_) => PeerState::ProtocolError("unexpected extra bytes from caller before answer"),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
            ) =>
        {
            PeerState::Waiting
        }
        Err(_) => PeerState::Closed("caller disconnected before answer"),
    }
}

pub(super) fn handle_stream(
    stream: TcpStream,
    token: &str,
    session: &str,
    ledger: &Ledger,
    terminal: TerminalInput,
) -> Result<(), String> {
    stream.set_read_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;

    let mut line = String::new();
    let n = BufReader::new((&stream).take(MAX_REQUEST_BYTES))
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
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

    let (approved, by) = if message["v"] != json!(APPROVAL_CHANNEL_VERSION) {
        (false, "supervisor: unknown channel version")
    } else if message["token"].as_str() != Some(token) {
        (false, "supervisor: wrong session token")
    } else if request["session"].as_str() != Some(session) {
        (false, "supervisor: request is for another session")
    } else if !valid_scope {
        (false, "supervisor: unknown or missing approval scope")
    } else if scope == "worker-session" && !contained {
        (false, "supervisor: worker-session requires a contained sandbox")
    } else if !terminal.is_interactive() {
        eprintln!("[supervisor] approval requested but no terminal is attached: answering NO");
        (false, "supervisor: no terminal to ask on")
    } else {
        match peek_peer_state(&stream) {
            PeerState::Waiting => {}
            PeerState::Closed(reason) | PeerState::ProtocolError(reason) => {
                record_expired(ledger, session, &call_id, &digest, scope, reason)?;
                return Ok(());
            }
        }
        let banner = PromptBanner {
            request,
            containment: message["containment"].as_str().unwrap_or("unknown"),
            contained,
            scope,
        };
        match wait_for_decision(Some(&stream), &banner, &terminal) {
            PromptOutcome::Answered { approved, by } => (approved, by),
            PromptOutcome::Expired(reason) => {
                eprintln!("\n[supervisor] approval prompt closed (request timed out or caller disconnected)");
                record_expired(ledger, session, &call_id, &digest, scope, reason)?;
                return Ok(());
            }
        }
    };

    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT)).map_err(|e| e.to_string())?;

    let payload = json!({
        "call_id": call_id,
        "digest": digest,
        "scope": scope,
        "approved": approved,
        "by": by,
    });
    if let Err(e) = ledger.append(session, "approval.answered", EVENT_VERSION, &payload) {
        let fail_reply = json!({
            "v": APPROVAL_CHANNEL_VERSION,
            "approved": false,
            "digest": digest,
            "by": "supervisor: ledger error",
        });
        let _ = (&stream).write_all(format!("{fail_reply}\n").as_bytes());
        let _ = (&stream).flush();
        return Err(format!("ledger could not record approval.answered: {e}"));
    }

    let reply = json!({
        "v": APPROVAL_CHANNEL_VERSION,
        "approved": approved,
        "digest": digest,
        "by": by,
    });
    if let Err(e) = (&stream).write_all(format!("{reply}\n").as_bytes()).and_then(|_| (&stream).flush()) {
        if approved {
            eprintln!("[supervisor] WARNING: could not deliver approval to core: {e}");
        }
        record_undelivered(ledger, session, &call_id, &digest, "reply write failed")?;
        return Ok(());
    }

    let mut ack_line = String::new();
    let ack_ok = match BufReader::new((&stream).take(MAX_ACK_BYTES)).read_line(&mut ack_line) {
        Ok(0) | Err(_) => false,
        Ok(_) => serde_json::from_str::<Value>(&ack_line).is_ok_and(|v| {
            v["v"] == json!(APPROVAL_CHANNEL_VERSION) && v["ack"] == json!(true)
        }),
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

fn record_expired(
    ledger: &Ledger,
    session: &str,
    call_id: &str,
    digest: &str,
    scope: &str,
    reason: &str,
) -> Result<(), String> {
    let payload = json!({ "call_id": call_id, "digest": digest, "scope": scope, "reason": reason });
    ledger
        .append(session, "approval.expired", EVENT_VERSION, &payload)
        .map(|_| ())
        .map_err(|e| format!("ledger could not record approval.expired: {e}"))
}

fn record_undelivered(
    ledger: &Ledger,
    session: &str,
    call_id: &str,
    digest: &str,
    reason: &str,
) -> Result<(), String> {
    let payload = json!({ "call_id": call_id, "digest": digest, "reason": reason });
    ledger
        .append(session, "approval.undelivered", EVENT_VERSION, &payload)
        .map(|_| ())
        .map_err(|e| format!("ledger could not record approval.undelivered: {e}"))
}
