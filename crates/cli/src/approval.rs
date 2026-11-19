//! The supervisor's side of the approval channel: a loopback listener that
//! the `core` MCP server connects to, and the console prompt.
//!
//! There is no way to approve without a person typing at a terminal: if
//! stdin or stderr is not a terminal, or the token/version/scope is wrong,
//! the answer is "no". When the caller (`core`) times out and drops its
//! socket while a prompt is on screen, the prompt is closed cleanly and
//! `approval.expired` is recorded instead of leaving a stale prompt open.

use crate::events::EVENT_VERSION;
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tools::APPROVAL_CHANNEL_VERSION;

const MAX_REQUEST_BYTES: u64 = 65_536;
const MAX_ACK_BYTES: u64 = 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const ANTI_BUFFER_WINDOW: Duration = Duration::from_millis(150);

pub struct ApprovalServer {
    pub addr: String,
}

enum StdinEvent {
    Line(String),
    Eof,
}

struct StdinHub {
    active: Mutex<Option<mpsc::Sender<StdinEvent>>>,
    eof: AtomicBool,
}

impl StdinHub {
    fn new() -> Self {
        StdinHub { active: Mutex::new(None), eof: AtomicBool::new(false) }
    }

    fn attach(&self) -> Option<mpsc::Receiver<StdinEvent>> {
        let mut active = self.active.lock().unwrap();
        if self.eof.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        *active = Some(tx);
        Some(rx)
    }

    fn mark_eof(&self) {
        self.eof.store(true, Ordering::Release);
        if let Some(tx) = self.active.lock().unwrap().take() {
            let _ = tx.send(StdinEvent::Eof);
        }
    }
}

static STDIN_HUB: OnceLock<StdinHub> = OnceLock::new();

fn stdin_hub() -> &'static StdinHub {
    STDIN_HUB.get_or_init(|| {
        std::thread::Builder::new()
            .name("re-search-stdin".into())
            .spawn(|| {
                let hub = STDIN_HUB.wait();
                let stdin = io::stdin();
                loop {
                    let mut line = String::new();
                    match stdin.read_line(&mut line) {
                        Ok(0) | Err(_) => {
                            hub.mark_eof();
                            break;
                        }
                        Ok(_) => {
                            let active = hub.active.lock().unwrap().clone();
                            if let Some(tx) = active {
                                if tx.send(StdinEvent::Line(line)).is_ok() {
                                    continue;
                                }
                            }
                            eprintln!("[supervisor] no active approval prompt; input ignored");
                        }
                    }
                }
            })
            .expect("could not start stdin reader thread");
        StdinHub::new()
    })
}

enum ActivePromptGuard {
    Static(&'static StdinHub),
    #[cfg(test)]
    Owned(std::sync::Arc<StdinHub>),
}

impl Drop for ActivePromptGuard {
    fn drop(&mut self) {
        let slot = match self {
            ActivePromptGuard::Static(h) => &h.active,
            #[cfg(test)]
            ActivePromptGuard::Owned(h) => &h.active,
        };
        *slot.lock().unwrap() = None;
    }
}

enum TerminalInput {
    System,
    #[cfg(test)]
    Test {
        rx: mpsc::Receiver<StdinEvent>,
        anti_buffer: Duration,
    },
    #[cfg(test)]
    Hub(std::sync::Arc<StdinHub>),
}

impl TerminalInput {
    fn is_interactive(&self) -> bool {
        match self {
            TerminalInput::System => io::stdin().is_terminal() && io::stderr().is_terminal(),
            #[cfg(test)]
            TerminalInput::Test { .. } | TerminalInput::Hub(_) => true,
        }
    }
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

enum PeerState {
    Waiting,
    Closed(&'static str),
    ProtocolError(&'static str),
}

fn peek_peer_state(stream: &TcpStream) -> PeerState {
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

fn handle_stream(
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
    } else if scope == "worker-session" && message.get("contained").is_some_and(|v| v != &json!(true)) {
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
        match wait_for_decision(
            &stream,
            request,
            message["containment"].as_str().unwrap_or("unknown"),
            contained,
            scope,
            terminal,
        ) {
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

enum PromptOutcome {
    Answered { approved: bool, by: &'static str },
    Expired(&'static str),
}

fn wait_for_decision(
    stream: &TcpStream,
    request: &Value,
    containment: &str,
    contained: bool,
    scope: &str,
    terminal: TerminalInput,
) -> PromptOutcome {
    let (_guard, rx, anti_buffer) = match terminal {
        TerminalInput::System => {
            let hub = stdin_hub();
            let Some(rx) = hub.attach() else {
                eprintln!("[supervisor] terminal closed; answering NO");
                return PromptOutcome::Answered { approved: false, by: "supervisor: terminal EOF" };
            };
            (Some(ActivePromptGuard::Static(hub)), rx, ANTI_BUFFER_WINDOW)
        }
        #[cfg(test)]
        TerminalInput::Hub(hub) => {
            let Some(rx) = hub.attach() else {
                eprintln!("[supervisor] terminal closed; answering NO");
                return PromptOutcome::Answered { approved: false, by: "supervisor: terminal EOF" };
            };
            (Some(ActivePromptGuard::Owned(hub)), rx, Duration::ZERO)
        }
        #[cfg(test)]
        TerminalInput::Test { rx, anti_buffer } => (None, rx, anti_buffer),
    };
    print_banner(request, containment, contained, scope);
    let mut printed_at = Instant::now();

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(StdinEvent::Line(answer)) => {
                if printed_at.elapsed() < anti_buffer {
                    eprintln!("[supervisor] buffered input ignored; please confirm this prompt [y/N]: ");
                    printed_at = Instant::now();
                    continue;
                }
                let yes = matches!(answer.trim(), "y" | "Y" | "yes" | "Yes" | "YES");
                eprintln!("[supervisor] {}", if yes { "approved" } else { "declined" });
                return PromptOutcome::Answered { approved: yes, by: "console" };
            }
            Ok(StdinEvent::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("\n[supervisor] terminal closed; answering NO");
                return PromptOutcome::Answered { approved: false, by: "supervisor: terminal EOF" };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => match peek_peer_state(stream) {
                PeerState::Waiting => {}
                PeerState::Closed(reason) | PeerState::ProtocolError(reason) => {
                    return PromptOutcome::Expired(reason);
                }
            },
        }
    }
}

fn sanitize_terminal_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_control() && ch != '\t' {
            out.extend(ch.escape_default());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
fn format_banner(request: &Value, containment: &str, scope: &str) -> String {
    format_banner_with_containment(request, containment, false, scope)
}

fn format_banner_with_containment(request: &Value, containment: &str, contained: bool, scope: &str) -> String {
    use std::fmt::Write as _;

    let args = request["canonical_args"].as_str().unwrap_or("");
    let parsed: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let scope_note = if scope == "worker-session" {
        "worker-session (approves this call and further calls until this worker restarts)"
    } else {
        "once (this call only)"
    };
    let mut out = String::new();
    let _ = writeln!(out, "\n================ APPROVAL NEEDED ================");
    let _ = writeln!(out, "tool:    {}   (effect class: {})", request["tool_id"], request["effect"]);
    let _ = writeln!(out, "scope:   {scope_note}");
    let _ = writeln!(out, "caller:  {}", request["caller"]);
    match parsed["code"].as_str() {
        Some(code) => {
            let _ = writeln!(out, "code to run:");
            for l in code.lines() {
                let _ = writeln!(out, "    | {}", sanitize_terminal_text(l));
            }
        }
        None => {
            let _ = writeln!(out, "arguments: {}", sanitize_terminal_text(args));
        }
    }
    let containment_note = if contained {
        "CONTAINED (appcontainer): network blocked; user files blocked; writes restricted to session scratch (system/AAP-readable files remain readable)."
    } else {
        "NOT CONTAINED: this runs as you. It can read and write your files and use the network."
    };
    let _ = writeln!(out, "{containment_note}");
    let _ = writeln!(out, "({})", sanitize_terminal_text(containment));
    let _ = writeln!(out, "digest:  {}", sanitize_terminal_text(request["digest"].as_str().unwrap_or("")));
    let prompt = if scope == "worker-session" {
        "Approve this worker session and run? [y/N]: "
    } else {
        "Run exactly this, once? [y/N]: "
    };
    let _ = write!(out, "{prompt}");
    out
}

fn print_banner(request: &Value, containment: &str, contained: bool, scope: &str) {
    let banner = format_banner_with_containment(request, containment, contained, scope);
    let mut err = io::stderr().lock();
    let _ = err.write_all(banner.as_bytes());
    let _ = err.flush();
}

#[cfg(test)]
mod tests;
