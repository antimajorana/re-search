//! Loopback TCP approval listener for the `--route cli` MCP server, answered on the
//! terminal.
//!
//! The wire handling (token, version, session, scope and containment checks; ledger
//! bookkeeping; ACK) lives in `cli_route::relay`. This module supplies the decision: the
//! console prompt. When the caller (`core`) times out and drops its socket while a prompt
//! is on screen, the prompt is closed cleanly and `approval.expired` is recorded instead
//! of leaving a stale prompt open.

use super::{stdin_hub, wait_for_decision, PromptBanner, PromptOutcome, TerminalInput};
use cli_route::relay::{RelayAsk, RelayDecider, RelayDecision, RelayServer};
use ledger::Ledger;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(super) use cli_route::relay::{peek_peer_state, PeerState};

/// Asks the person at the terminal. Never approves without a typed answer.
struct ConsoleRelay {
    terminal: Mutex<TerminalInput>,
}

impl RelayDecider for ConsoleRelay {
    fn decide(&self, ask: &RelayAsk<'_>) -> RelayDecision {
        let terminal = self.terminal.lock().unwrap_or_else(|e| e.into_inner());
        if !terminal.is_interactive() {
            eprintln!("[supervisor] approval requested but no terminal is attached: answering NO");
            return RelayDecision::Answered { approved: false, by: "supervisor: no terminal to ask on".into() };
        }
        match peek_peer_state(ask.peer) {
            PeerState::Waiting => {}
            PeerState::Closed(reason) | PeerState::ProtocolError(reason) => return RelayDecision::Expired(reason.into()),
        }
        let banner = PromptBanner {
            request: ask.request,
            containment: ask.containment,
            contained: ask.contained,
            scope: ask.scope,
        };
        match wait_for_decision(Some(ask.peer), &banner, &terminal) {
            PromptOutcome::Answered { approved, by } => RelayDecision::Answered { approved, by: by.into() },
            PromptOutcome::Expired(reason) => {
                eprintln!("\n[supervisor] approval prompt closed (request timed out or caller disconnected)");
                RelayDecision::Expired(reason.into())
            }
        }
    }
}

/// Bind 127.0.0.1 on a free port and answer approval requests on a background thread,
/// one at a time, on the terminal.
pub fn start(data_dir: PathBuf, session: String, token: String) -> io::Result<RelayServer> {
    let ledger = Ledger::open(&data_dir).map_err(|e| io::Error::other(e.to_string()))?;
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        let _ = stdin_hub();
    }
    let decider = Arc::new(ConsoleRelay { terminal: Mutex::new(TerminalInput::System) });
    cli_route::relay::start(ledger, session, token, decider)
}

#[cfg(test)]
pub(super) fn handle_stream(
    stream: std::net::TcpStream,
    token: &str,
    session: &str,
    ledger: &Ledger,
    terminal: TerminalInput,
) -> Result<(), String> {
    let decider = ConsoleRelay { terminal: Mutex::new(terminal) };
    cli_route::relay::handle_stream(stream, token, session, ledger, &decider)
}
