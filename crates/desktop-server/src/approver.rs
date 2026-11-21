//! UI-backed `tools::Approver` implementation with unified diff previews
//! and fail-closed SHA-256 chained ledger recording.

use crate::state::{now_ms, DesktopState, PendingApprovalView, PendingSlot};
use research_tools::{preview_workspace_edit, EditPreview};
use serde_json::json;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tools::canon::random_hex;
use tools::{ApprovalReply, ApprovalRequest, ApprovalScope, Approver, APPROVAL_SCHEMA_VERSION, EVENT_SCHEMA_VERSION};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

pub struct UiApprover {
    pub state: Arc<DesktopState>,
    pub containment: String,
    pub contained: bool,
}

impl UiApprover {
    pub fn new(state: Arc<DesktopState>, containment: String, contained: bool) -> Self {
        Self {
            state,
            containment,
            contained,
        }
    }

    fn compute_preview(&self, request: &ApprovalRequest) -> Option<EditPreview> {
        if !matches!(request.tool_id.as_str(), "fs_write" | "latex_patch") {
            return None;
        }
        let env = self.state.research_env_for_session(&request.session).ok()?;
        preview_workspace_edit(&env, &request.tool_id, &request.canonical_args).ok()
    }

    fn record_answer_and_reply(
        &self,
        request: &ApprovalRequest,
        scope: ApprovalScope,
        approved: bool,
        reason: &str,
    ) -> ApprovalReply {
        let mut final_approved = approved;
        let mut final_reason = reason.to_string();

        let _lock = self.state.ledger_write_lock.lock();
        match self.state.open_ledger() {
            Ok(ledger) => {
                let payload = json!({
                    "tool": &request.tool_id,
                    "digest": &request.digest,
                    "approved": approved,
                    "reason": reason,
                    "scope": scope.as_str(),
                });
                if let Err(err) = ledger.append_with_hash(
                    &request.session,
                    "approval.answered",
                    EVENT_SCHEMA_VERSION,
                    &payload,
                ) {
                    final_approved = false;
                    final_reason = format!("ledger could not record approval decision ({err}); failing closed");
                }
            }
            Err(err) => {
                final_approved = false;
                final_reason = format!("ledger open failed ({err}); failing closed");
            }
        }

        ApprovalReply {
            approved: final_approved,
            digest: request.digest.clone(),
            by: final_reason,
        }
    }
}

impl Approver for UiApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        if request.schema_version != APPROVAL_SCHEMA_VERSION {
            return ApprovalReply {
                approved: false,
                digest: request.digest.clone(),
                by: "approval request schema_version mismatch".to_string(),
            };
        }
        let scope = match request.scope {
            ApprovalScope::Once => ApprovalScope::Once,
            ApprovalScope::WorkerSession => {
                if !self.contained {
                    return self.record_answer_and_reply(
                        request,
                        ApprovalScope::Once,
                        false,
                        "worker-session approval requested without active containment; failing closed",
                    );
                }
                ApprovalScope::WorkerSession
            }
        };

        let call_id = format!("ap-{}", random_hex(8));
        let edit_preview = self.compute_preview(request);
        let view = PendingApprovalView {
            call_id: call_id.clone(),
            session: request.session.clone(),
            tool_id: request.tool_id.clone(),
            canonical_args: request.canonical_args.clone(),
            digest: request.digest.clone(),
            scope: scope.as_str().to_string(),
            containment: self.containment.clone(),
            contained: self.contained,
            edit_preview,
            created_ms: now_ms(),
        };

        let slot = Arc::new(PendingSlot {
            view: view.clone(),
            decision: Mutex::new(None),
            cv: Condvar::new(),
        });
        let key = (request.session.clone(), call_id.clone());
        {
            let Ok(mut map) = self.state.pending_approvals.lock() else {
                return self.record_answer_and_reply(request, scope, false, "approval map lock poisoned");
            };
            map.insert(key.clone(), Arc::clone(&slot));
        }

        if let Ok(payload) = serde_json::to_value(&view) {
            self.state.sse.emit(&request.session, "approval_requested", payload);
        }

        let deadline = Instant::now() + APPROVAL_TIMEOUT;
        let decision = {
            let Ok(mut guard) = slot.decision.lock() else {
                if let Ok(mut map) = self.state.pending_approvals.lock() {
                    map.remove(&key);
                }
                return self.record_answer_and_reply(request, scope, false, "approval slot lock poisoned");
            };
            while guard.is_none() {
                if self.state.sse.is_shutdown() {
                    break;
                }
                let rem = deadline.saturating_duration_since(Instant::now());
                if rem.is_zero() {
                    break;
                }
                let wait_step = rem.min(Duration::from_millis(500));
                let (next, _) = match slot.cv.wait_timeout(guard, wait_step) {
                    Ok(v) => v,
                    Err(poisoned) => poisoned.into_inner(),
                };
                guard = next;
            }
            guard.take()
        };

        if let Ok(mut map) = self.state.pending_approvals.lock() {
            map.remove(&key);
        }

        let (approved, reason) = match decision {
            Some(dec) if dec.digest == request.digest => {
                if dec.approved {
                    (true, format!("approved in desktop UI ({})", dec.responder))
                } else if dec.responder == "desktop-ui" {
                    (false, format!("denied in desktop UI ({})", dec.responder))
                } else {
                    // Denied by the server itself (caller gone, turn stopped), not by a person.
                    (false, format!("denied: {}", dec.responder))
                }
            }
            Some(_) => (false, "approval digest mismatch; failing closed".to_string()),
            None if self.state.sse.is_shutdown() => (false, "desktop server shutting down; denied".to_string()),
            None => (false, "desktop approval timed out after 600s".to_string()),
        };

        let reply = self.record_answer_and_reply(request, scope, approved, &reason);
        self.state.sse.emit(
            &request.session,
            "approval_resolved",
            json!({
                "call_id": call_id,
                "session": &request.session,
                "tool_id": &request.tool_id,
                "digest": &request.digest,
                "approved": reply.approved,
                "reason": &reply.by,
            }),
        );
        reply
    }
}
