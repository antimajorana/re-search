//! Approval relay decisions for `claude-cli` turns: the desktop approval card.
//!
//! `core.exe` (started by the Claude CLI) asks over the loopback relay in
//! `cli_route::relay`, which checks the channel version, session token, session, scope and
//! containment first. A request that passes is shown through [`UiApprover::decide`], the
//! same card, digest binding and fail-closed `approval.answered` recording the API route
//! uses. Only a person answering the card through the token-protected
//! `POST /api/sessions/{id}/approvals/{call_id}` can approve. If `core.exe` goes away while
//! the card is open (timeout, turn stopped), the card is denied, never approved.

use crate::approver::UiApprover;
use crate::state::DesktopState;
use cli_route::relay::{peek_peer_state, PeerState, RelayAsk, RelayDecider, RelayDecision};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tools::{ApprovalRequest, Approver};

const PEER_POLL: Duration = Duration::from_millis(200);

pub(crate) struct UiRelay {
    pub state: Arc<DesktopState>,
}

/// Watches the caller's socket while a card is open and denies that card if the caller
/// disconnects.
struct PeerWatch {
    done: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PeerWatch {
    fn start(peer: Option<TcpStream>, state: Arc<DesktopState>, session: String, digest: String) -> PeerWatch {
        let done = Arc::new(AtomicBool::new(false));
        let handle = peer.map(|peer| {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    std::thread::sleep(PEER_POLL);
                    if done.load(Ordering::Acquire) {
                        break;
                    }
                    if let PeerState::Closed(reason) | PeerState::ProtocolError(reason) = peek_peer_state(&peer) {
                        state.deny_pending(&session, Some(&digest), &format!("supervisor: {reason}"));
                        break;
                    }
                }
            })
        });
        PeerWatch { done, handle }
    }

    fn finish(mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl RelayDecider for UiRelay {
    fn decide(&self, ask: &RelayAsk<'_>) -> RelayDecision {
        match peek_peer_state(ask.peer) {
            PeerState::Waiting => {}
            PeerState::Closed(reason) | PeerState::ProtocolError(reason) => return RelayDecision::Expired(reason.into()),
        }
        let request: ApprovalRequest = match serde_json::from_value(ask.request.clone()) {
            Ok(r) => r,
            Err(e) => {
                return RelayDecision::Answered {
                    approved: false,
                    by: format!("supervisor: malformed approval request ({e})"),
                }
            }
        };
        let watch = PeerWatch::start(
            ask.peer.try_clone().ok(),
            Arc::clone(&self.state),
            ask.session.to_string(),
            request.digest.clone(),
        );
        let approver = UiApprover::new(Arc::clone(&self.state), ask.containment.to_string(), ask.contained);
        let reply = approver.decide(&request);
        watch.finish();
        // `UiApprover` already recorded `approval.answered` (and answered "no" if it could not).
        RelayDecision::Recorded { approved: reply.approved, by: reply.by }
    }
}
