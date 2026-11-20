//! The supervisor's approval channel and terminal console prompt.
//!
//! There is no way to approve without a person typing at a terminal: if
//! stdin or stderr is not a terminal, or the token/version/scope/containment
//! check fails, the answer is "no".

mod console;
mod server;
#[cfg(test)]
mod tests;

pub use console::ConsoleApprover;
pub use server::start;

use serde_json::Value;
#[cfg(test)]
use server::handle_stream;
use server::{peek_peer_state, PeerState};
use std::io::{self, IsTerminal, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(200);
pub(super) const ANTI_BUFFER_WINDOW: Duration = Duration::from_millis(150);

pub(super) enum StdinEvent {
    Line(String),
    Eof,
}

pub(super) struct StdinHub {
    active: Mutex<Option<mpsc::Sender<StdinEvent>>>,
    eof: AtomicBool,
}

impl StdinHub {
    pub(super) fn new() -> Self {
        StdinHub { active: Mutex::new(None), eof: AtomicBool::new(false) }
    }

    pub(super) fn attach(&self) -> Option<mpsc::Receiver<StdinEvent>> {
        let mut active = self.active.lock().unwrap();
        if self.eof.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        *active = Some(tx);
        Some(rx)
    }

    pub(super) fn mark_eof(&self) {
        self.eof.store(true, Ordering::Release);
        if let Some(tx) = self.active.lock().unwrap().take() {
            let _ = tx.send(StdinEvent::Eof);
        }
    }
}

static STDIN_HUB: OnceLock<StdinHub> = OnceLock::new();

pub(super) fn stdin_hub() -> &'static StdinHub {
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

pub(super) enum TerminalInput {
    System,
    #[cfg(test)]
    NonInteractive,
    #[cfg(test)]
    Test {
        rx: mpsc::Receiver<StdinEvent>,
        anti_buffer: Duration,
    },
    #[cfg(test)]
    Hub(std::sync::Arc<StdinHub>),
}

impl TerminalInput {
    pub(super) fn is_interactive(&self) -> bool {
        match self {
            TerminalInput::System => io::stdin().is_terminal() && io::stderr().is_terminal(),
            #[cfg(test)]
            TerminalInput::NonInteractive => false,
            #[cfg(test)]
            TerminalInput::Test { .. } | TerminalInput::Hub(_) => true,
        }
    }
}

pub(super) enum PromptOutcome {
    Answered { approved: bool, by: &'static str },
    Expired(&'static str),
}

pub(super) struct PromptBanner<'a> {
    pub request: &'a Value,
    pub containment: &'a str,
    pub contained: bool,
    pub scope: &'a str,
}

pub(super) fn wait_for_decision(
    peer: Option<&TcpStream>,
    banner: &PromptBanner<'_>,
    terminal: &TerminalInput,
) -> PromptOutcome {
    enum Rx<'a> {
        Owned(mpsc::Receiver<StdinEvent>),
        #[cfg(test)]
        Ref(&'a mpsc::Receiver<StdinEvent>),
        #[cfg(not(test))]
        _Phantom(std::marker::PhantomData<&'a ()>),
    }
    impl<'a> Rx<'a> {
        fn get(&self) -> &mpsc::Receiver<StdinEvent> {
            match self {
                Rx::Owned(rx) => rx,
                #[cfg(test)]
                Rx::Ref(rx) => rx,
                #[cfg(not(test))]
                Rx::_Phantom(_) => unreachable!(),
            }
        }
    }

    let (_guard, rx, anti_buffer) = match terminal {
        TerminalInput::System => {
            let hub = stdin_hub();
            let Some(rx) = hub.attach() else {
                eprintln!("[supervisor] terminal closed; answering NO");
                return PromptOutcome::Answered { approved: false, by: "supervisor: terminal EOF" };
            };
            (Some(ActivePromptGuard::Static(hub)), Rx::Owned(rx), ANTI_BUFFER_WINDOW)
        }
        #[cfg(test)]
        TerminalInput::NonInteractive => {
            return PromptOutcome::Answered { approved: false, by: "supervisor: no terminal to ask on" };
        }
        #[cfg(test)]
        TerminalInput::Hub(hub) => {
            let Some(rx) = hub.attach() else {
                eprintln!("[supervisor] terminal closed; answering NO");
                return PromptOutcome::Answered { approved: false, by: "supervisor: terminal EOF" };
            };
            (Some(ActivePromptGuard::Owned(hub.clone())), Rx::Owned(rx), Duration::ZERO)
        }
        #[cfg(test)]
        TerminalInput::Test { rx, anti_buffer } => (None, Rx::Ref(rx), *anti_buffer),
    };
    let rx = rx.get();
    print_banner(banner);
    let mut printed_at = Instant::now();

    loop {
        let outcome = match peer {
            Some(_) => rx.recv_timeout(POLL_INTERVAL),
            None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match outcome {
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
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(stream) = peer {
                    match peek_peer_state(stream) {
                        PeerState::Waiting => {}
                        PeerState::Closed(reason) | PeerState::ProtocolError(reason) => {
                            return PromptOutcome::Expired(reason);
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn sanitize_terminal_text(s: &str) -> String {
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
pub(super) fn format_banner(request: &Value, containment: &str, scope: &str) -> String {
    format_banner_with_containment(request, containment, false, scope)
}

pub(super) fn format_banner_with_containment(
    request: &Value,
    containment: &str,
    contained: bool,
    scope: &str,
) -> String {
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
    let _ = writeln!(
        out,
        "tool:    {}   (effect class: {})",
        sanitize_terminal_text(&request["tool_id"].to_string()),
        sanitize_terminal_text(&request["effect"].to_string()),
    );
    let _ = writeln!(out, "scope:   {scope_note}");
    let _ = writeln!(out, "caller:  {}", sanitize_terminal_text(&request["caller"].to_string()));
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

fn print_banner(banner: &PromptBanner<'_>) {
    let text = format_banner_with_containment(banner.request, banner.containment, banner.contained, banner.scope);
    let mut err = io::stderr().lock();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
}
