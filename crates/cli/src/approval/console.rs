//! In-process console [`tools::Approver`] used by `--route api`.

use super::{stdin_hub, wait_for_decision, PromptBanner, PromptOutcome, TerminalInput};
use crate::events::EVENT_VERSION;
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{self, IsTerminal};
use std::sync::Mutex;
use tools::{ApprovalReply, ApprovalRequest, ApprovalScope, Approver, APPROVAL_SCHEMA_VERSION};

/// Prompts the human on stderr/stdin and fails closed unless both a terminal
/// approves the request and `approval.answered` is durably appended to the ledger.
pub struct ConsoleApprover {
    ledger: Mutex<Ledger>,
    session: String,
    containment: String,
    contained: bool,
    terminal: Mutex<TerminalInput>,
}

impl ConsoleApprover {
    pub fn new(ledger: Ledger, session: String, containment: String, contained: bool) -> Self {
        Self::with_terminal(ledger, session, containment, contained, TerminalInput::System)
    }

    pub(super) fn with_terminal(
        ledger: Ledger,
        session: String,
        containment: String,
        contained: bool,
        terminal: TerminalInput,
    ) -> Self {
        if matches!(terminal, TerminalInput::System) && io::stdin().is_terminal() && io::stderr().is_terminal() {
            let _ = stdin_hub();
        }
        Self {
            ledger: Mutex::new(ledger),
            session,
            containment,
            contained,
            terminal: Mutex::new(terminal),
        }
    }
}

impl Approver for ConsoleApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        let scope = request.scope.as_str();
        let (approved, by) = if request.schema_version != APPROVAL_SCHEMA_VERSION {
            (false, "supervisor: unknown approval schema version")
        } else if request.session != self.session {
            (false, "supervisor: request is for another session")
        } else if request.scope == ApprovalScope::WorkerSession && !self.contained {
            (false, "supervisor: worker-session requires a contained sandbox")
        } else {
            let terminal = self.terminal.lock().unwrap_or_else(|e| e.into_inner());
            if !terminal.is_interactive() {
                eprintln!("[supervisor] approval requested but no terminal is attached: answering NO");
                (false, "supervisor: no terminal to ask on")
            } else {
                let req_json = serde_json::to_value(request).unwrap_or(Value::Null);
                let banner = PromptBanner {
                    request: &req_json,
                    containment: &self.containment,
                    contained: self.contained,
                    scope,
                };
                match wait_for_decision(None, &banner, &terminal) {
                    PromptOutcome::Answered { approved, by } => (approved, by),
                    PromptOutcome::Expired(reason) => (false, reason),
                }
            }
        };

        let payload = json!({
            "call_id": request.call_id,
            "digest": request.digest,
            "scope": scope,
            "approved": approved,
            "by": by,
        });
        let ledger = match self.ledger.lock() {
            Ok(g) => g,
            Err(_) => {
                eprintln!("[supervisor] WARNING: approval ledger lock poisoned; answering NO");
                return ApprovalReply {
                    approved: false,
                    digest: request.digest.clone(),
                    by: "supervisor: ledger error".into(),
                };
            }
        };
        if let Err(e) = ledger.append(&self.session, "approval.answered", EVENT_VERSION, &payload) {
            eprintln!("[supervisor] WARNING: ledger could not record approval.answered: {e}");
            return ApprovalReply {
                approved: false,
                digest: request.digest.clone(),
                by: "supervisor: ledger error".into(),
            };
        }
        ApprovalReply {
            approved,
            digest: request.digest.clone(),
            by: by.to_string(),
        }
    }
}
