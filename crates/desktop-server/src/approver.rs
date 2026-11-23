//! UI-backed `tools::Approver` implementation with unified diff previews
//! and fail-closed SHA-256 chained ledger recording.

use crate::session_lifecycle::{bind_session_grant, can_grant_for_session, matching_session_grant};
use crate::state::{now_ms, DesktopState, PendingApprovalView, PendingSlot};
use research_tools::{preview_workspace_edit, EditPreview};
use serde_json::json;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tools::canon::random_hex;
use tools::{
    ApprovalOutcome, ApprovalReply, ApprovalRequest, ApprovalScope, Approver, APPROVAL_SCHEMA_VERSION,
    EVENT_SCHEMA_VERSION,
};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

pub struct UiApprover {
    pub state: Arc<DesktopState>,
    pub containment: String,
    pub contained: bool,
    pub allow_session_network_grant: bool,
}

impl UiApprover {
    pub fn new(state: Arc<DesktopState>, containment: String, contained: bool) -> Self {
        Self {
            state,
            containment,
            contained,
            allow_session_network_grant: false,
        }
    }

    pub fn with_session_network_grant(mut self, allow: bool) -> Self {
        self.allow_session_network_grant = allow;
        self
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
        approve_for_session: bool,
        reason: &str,
    ) -> ApprovalOutcome {
        let mut final_approved = approved;
        let mut final_reason = reason.to_string();

        let _lock = self.state.ledger_write_lock.lock();
        match self.state.open_ledger() {
            Ok(ledger) => {
                let payload = json!({
                    "tool": &request.tool_id,
                    "digest": &request.digest,
                    "approved": approved,
                    "approve_for_session": approve_for_session,
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

        ApprovalOutcome {
            reply: ApprovalReply {
                approved: final_approved,
                digest: request.digest.clone(),
                by: final_reason,
            },
            approve_for_session: final_approved && approve_for_session,
        }
    }
}

impl Approver for UiApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        self.decide_ext(request).reply
    }

    fn decide_ext(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        if request.schema_version != APPROVAL_SCHEMA_VERSION {
            return ApprovalOutcome {
                reply: ApprovalReply {
                    approved: false,
                    digest: request.digest.clone(),
                    by: "approval request schema_version mismatch".to_string(),
                },
                approve_for_session: false,
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
                        false,
                        "worker-session approval requested without active containment; failing closed",
                    );
                }
                ApprovalScope::WorkerSession
            }
        };

        if self.state.load_ui_settings().auto_approve_tools {
            let call_id = format!("ap-auto-{}", random_hex(6));
            let outcome = self.record_answer_and_reply(
                request,
                scope,
                true,
                false,
                "auto-approved by desktop setting (auto_approve_tools)",
            );
            self.state.sse.emit(
                &request.session,
                "approval_resolved",
                json!({
                    "call_id": call_id,
                    "session": &request.session,
                    "tool_id": &request.tool_id,
                    "digest": &request.digest,
                    "approved": outcome.reply.approved,
                    "approve_for_session": false,
                    "auto_approved": true,
                    "reason": &outcome.reply.by,
                }),
            );
            return outcome;
        }

        let allow_session_approve =
            self.allow_session_network_grant && can_grant_for_session(request.effect, self.contained);

        if allow_session_approve {
            if let Some(grant) = matching_session_grant(&self.state, &request.session, request.effect, self.contained) {
                let reason = format!(
                    "reused human session grant ({}) in desktop UI",
                    &grant.grant_digest[..grant.grant_digest.len().min(12)]
                );
                return self.record_answer_and_reply(request, scope, true, true, &reason);
            }
        }

        let call_id = format!("ap-{}", random_hex(8));
        let edit_preview = self.compute_preview(request);
        let view = PendingApprovalView {
            call_id: call_id.clone(),
            session: request.session.clone(),
            tool_id: request.tool_id.clone(),
            effect: request.effect.as_str().to_string(),
            allow_session_approve,
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
                return self.record_answer_and_reply(request, scope, false, false, "approval map lock poisoned");
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
                return self.record_answer_and_reply(request, scope, false, false, "approval slot lock poisoned");
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

        let (approved, approve_for_session, reason) = match decision {
            Some(dec) if dec.digest == request.digest => {
                if dec.approved {
                    let sess_flag = dec.approve_for_session && allow_session_approve;
                    let note = if sess_flag {
                        format!("approved for session in desktop UI ({})", dec.responder)
                    } else {
                        format!("approved in desktop UI ({})", dec.responder)
                    };
                    (true, sess_flag, note)
                } else if dec.responder == "desktop-ui" {
                    (false, false, format!("denied in desktop UI ({})", dec.responder))
                } else {
                    (false, false, format!("denied: {}", dec.responder))
                }
            }
            Some(_) => (false, false, "approval digest mismatch; failing closed".to_string()),
            None if self.state.sse.is_shutdown() => {
                (false, false, "desktop server shutting down; denied".to_string())
            }
            None => (false, false, "desktop approval timed out after 600s".to_string()),
        };

        let outcome = self.record_answer_and_reply(request, scope, approved, approve_for_session, &reason);
        if outcome.reply.approved && outcome.approve_for_session {
            bind_session_grant(
                &self.state,
                &request.session,
                request.effect,
                self.contained,
                &request.digest,
            );
        }
        self.state.sse.emit(
            &request.session,
            "approval_resolved",
            json!({
                "call_id": call_id,
                "session": &request.session,
                "tool_id": &request.tool_id,
                "digest": &request.digest,
                "approved": outcome.reply.approved,
                "approve_for_session": outcome.approve_for_session,
                "reason": &outcome.reply.by,
            }),
        );
        outcome
    }
}
