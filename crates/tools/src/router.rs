//! The router: the only path from a caller to a tool handler.

use crate::approval::{approval_digest, ApprovalRequest, ApprovalStore, Approver};
use crate::canon::{canonical_json, random_hex};
use crate::registry::{check_args, CallCtx, Registry};
use crate::EVENT_SCHEMA_VERSION;
use ledger::Ledger;
use policy::{Decision, Policy};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// Payload text longer than this goes to the blob directory.
const INLINE_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum CallOutcome {
    Ok(Value),
    /// Refused by policy or by the human, or the approval did not hold.
    Denied(String),
    /// The call was let through but failed (bad arguments, handler error),
    /// or the ledger could not record it.
    Error(String),
}

pub struct Router {
    registry: Registry,
    policy: Policy,
    ledger: Mutex<Ledger>,
    session: String,
    approver: Arc<dyn Approver>,
    store: ApprovalStore,
}

impl Router {
    pub fn new(registry: Registry, policy: Policy, ledger: Ledger, session: &str, approver: Arc<dyn Approver>) -> Self {
        Router {
            registry,
            policy,
            ledger: Mutex::new(ledger),
            session: session.to_string(),
            approver,
            store: ApprovalStore::new(),
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    fn record(&self, kind: &str, payload: Value) -> Result<(), String> {
        let ledger = self.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        ledger
            .append(&self.session, kind, EVENT_SCHEMA_VERSION, &payload)
            .map(|_| ())
            .map_err(|e| format!("ledger could not record `{kind}`: {e}"))
    }

    /// Small text is stored in the event; large text goes to a blob and the
    /// event keeps its hash.
    fn inline_or_blob(&self, text: &str) -> Result<Value, String> {
        if text.len() <= INLINE_LIMIT {
            return Ok(json!({ "inline": text }));
        }
        let ledger = self.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let hash = ledger.put_blob(text.as_bytes()).map_err(|e| e.to_string())?;
        Ok(json!({ "blob": hash, "bytes": text.len() }))
    }

    /// Handle one tool call from `caller`. The caller string must come from
    /// the channel (who is connected), never from the call's arguments.
    pub fn call(&self, caller: &str, tool_id: &str, args: &Value) -> CallOutcome {
        let call_id = random_hex(8);
        match self.call_inner(caller, tool_id, args, &call_id) {
            Ok(outcome) => outcome,
            // Recording failed somewhere: fail closed.
            Err(e) => CallOutcome::Error(e),
        }
    }

    fn call_inner(&self, caller: &str, tool_id: &str, args: &Value, call_id: &str) -> Result<CallOutcome, String> {
        let canonical = canonical_json(args);
        let def = self.registry.get(tool_id).map(|(d, _)| d);
        self.record(
            "tool.request",
            json!({
                "call_id": call_id,
                "caller": caller,
                "tool": tool_id,
                "tool_def_hash": def.map(|d| d.def_hash()),
                "effect": def.map(|d| d.effect.as_str()),
                "args": self.inline_or_blob(&canonical)?,
            }),
        )?;
        let Some(def) = def else {
            return self.finish_denied(call_id, "unknown tool");
        };
        if let Err(e) = check_args(&def.input_schema, args) {
            return self.finish_error(call_id, &format!("invalid arguments: {e}"));
        }

        let request = self.policy_request(caller, tool_id, &canonical);
        let response = policy::evaluate(&self.policy, &request.expect("tool exists"));
        self.record(
            "policy.decision",
            json!({
                "call_id": call_id,
                "decision": response.decision,
                "matched_rule": response.matched_rule,
                "reason": response.reason,
                "policy_version": response.policy_version,
                "policy_schema_version": response.schema_version,
            }),
        )?;

        let nonce = match response.decision {
            Decision::Deny => return self.finish_denied(call_id, &format!("policy: {}", response.reason)),
            Decision::Allow => None,
            Decision::Ask => {
                let nonce = random_hex(16);
                let digest =
                    approval_digest(caller, tool_id, &def.def_hash(), &canonical, &self.policy.version, &nonce);
                self.record("approval.requested", json!({ "call_id": call_id, "digest": digest }))?;
                let reply = self.approver.decide(&ApprovalRequest {
                    schema_version: EVENT_SCHEMA_VERSION,
                    session: self.session.clone(),
                    call_id: call_id.to_string(),
                    digest: digest.clone(),
                    caller: caller.to_string(),
                    tool_id: tool_id.to_string(),
                    effect: def.effect,
                    canonical_args: canonical.clone(),
                    policy_reason: response.reason.clone(),
                });
                if !reply.approved || reply.digest != digest {
                    let why = if reply.approved { "approver answered for a different digest" } else { "declined" };
                    self.record(
                        "approval.denied",
                        json!({ "call_id": call_id, "digest": digest, "by": reply.by, "why": why }),
                    )?;
                    return self.finish_denied(call_id, &format!("approval: {why}"));
                }
                self.store.grant(&digest);
                self.record("approval.granted", json!({ "call_id": call_id, "digest": digest, "by": reply.by }))?;
                Some(nonce)
            }
        };
        self.dispatch(caller, tool_id, args, nonce.as_deref(), call_id)
    }

    fn policy_request(&self, caller: &str, tool_id: &str, canonical: &str) -> Option<policy::Request> {
        let (def, _) = self.registry.get(tool_id)?;
        Some(policy::Request {
            schema_version: policy::SCHEMA_VERSION,
            caller: caller.to_string(),
            tool_id: tool_id.to_string(),
            tool_def_hash: def.def_hash(),
            effect: def.effect,
            canonical_args: canonical.to_string(),
            policy_version: self.policy.version.clone(),
        })
    }

    /// The last step before the handler runs. Everything is looked up and
    /// computed again from what is about to be dispatched: the policy is
    /// re-evaluated, and for an "ask" decision the digest is rebuilt and must
    /// match an unused grant, which is consumed here.
    fn dispatch(
        &self,
        caller: &str,
        tool_id: &str,
        args: &Value,
        nonce: Option<&str>,
        call_id: &str,
    ) -> Result<CallOutcome, String> {
        let canonical = canonical_json(args);
        let Some(request) = self.policy_request(caller, tool_id, &canonical) else {
            return self.finish_denied(call_id, "unknown tool");
        };
        match policy::evaluate(&self.policy, &request).decision {
            Decision::Deny => return self.finish_denied(call_id, "policy denies at dispatch"),
            Decision::Allow => {}
            Decision::Ask => {
                let digest = approval_digest(
                    caller,
                    tool_id,
                    &request.tool_def_hash,
                    &canonical,
                    &self.policy.version,
                    nonce.unwrap_or(""),
                );
                if !self.store.consume(&digest) {
                    self.record("approval.rejected", json!({ "call_id": call_id, "digest": digest }))?;
                    return self.finish_denied(call_id, "approval: no unused approval for exactly this call");
                }
                self.record("approval.consumed", json!({ "call_id": call_id, "digest": digest }))?;
            }
        }
        let (_, handler) = self.registry.get(tool_id).expect("checked above");
        let ctx = CallCtx { session: &self.session, caller, call_id, ledger: &self.ledger };
        match handler.call(&ctx, args) {
            Ok(value) => {
                let stored = self.inline_or_blob(&canonical_json(&value))?;
                self.record("tool.result", json!({ "call_id": call_id, "ok": true, "result": stored }))?;
                Ok(CallOutcome::Ok(value))
            }
            Err(e) => self.finish_error(call_id, &e),
        }
    }

    fn finish_denied(&self, call_id: &str, why: &str) -> Result<CallOutcome, String> {
        self.record("tool.denied", json!({ "call_id": call_id, "why": why }))?;
        Ok(CallOutcome::Denied(why.to_string()))
    }

    fn finish_error(&self, call_id: &str, why: &str) -> Result<CallOutcome, String> {
        let stored = self.inline_or_blob(why)?;
        self.record("tool.result", json!({ "call_id": call_id, "ok": false, "error": stored }))?;
        Ok(CallOutcome::Error(why.to_string()))
    }
}

#[cfg(test)]
mod tests;
