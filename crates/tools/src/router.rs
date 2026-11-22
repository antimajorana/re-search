//! The router: the only path from a caller to a tool handler.

use crate::approval::{
    approval_digest, ApprovalRequest, ApprovalScope, ApprovalStore, Approver, NetworkReadGrant, SessionGrant,
    WorkerEpoch, APPROVAL_SCHEMA_VERSION,
};
use crate::canon::{canonical_json, random_hex};
use crate::registry::{check_args, CallCtx, Registry, ToolDef};
use crate::EVENT_SCHEMA_VERSION;
use ledger::Ledger;
use policy::{Decision, EffectClass, Policy};
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

enum DispatchAuth<'a> {
    Allow,
    Once {
        nonce: &'a str,
        bind_session_digest: Option<String>,
        bind_network_read_digest: Option<String>,
    },
    Session(u64),
    NetworkSession,
}

/// `WorkerSession` reuse is allowed only when the tool's epoch provider
/// reports an active filesystem- and network-restricted sandbox.
fn effective_scope(def: &ToolDef, epoch_src: Option<&Arc<dyn WorkerEpoch>>) -> ApprovalScope {
    if def.approval_scope == ApprovalScope::WorkerSession && epoch_src.is_some_and(|e| e.is_contained()) {
        ApprovalScope::WorkerSession
    } else {
        ApprovalScope::Once
    }
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

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn seed_network_read_grant(&self, caller: &str, grant: NetworkReadGrant) {
        self.store.bind_network_read_grant(caller, grant);
    }

    pub fn active_network_read_grant(&self, caller: &str) -> Option<NetworkReadGrant> {
        self.store.matching_network_read_grant(caller, &self.policy.version)
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

    /// Handle one tool call from `caller`, returning `Err` if and only if the
    /// ledger or blob store could not record a required audit step.
    ///
    /// The caller string must come from the channel (who is connected), never
    /// from the call's arguments.
    pub fn try_call(&self, caller: &str, tool_id: &str, args: &Value) -> Result<CallOutcome, String> {
        let call_id = random_hex(8);
        self.call_inner(caller, tool_id, args, &call_id)
    }

    /// Handle one tool call from `caller`, collapsing ledger failures into
    /// [`CallOutcome::Error`] for single-call RPC servers.
    pub fn call(&self, caller: &str, tool_id: &str, args: &Value) -> CallOutcome {
        self.try_call(caller, tool_id, args).unwrap_or_else(CallOutcome::Error)
    }

    fn call_inner(&self, caller: &str, tool_id: &str, args: &Value, call_id: &str) -> Result<CallOutcome, String> {
        let canonical = canonical_json(args);
        let entry = self.registry.get_entry(tool_id);
        let def = entry.map(|(d, _, _)| d);
        let scope = entry.map(|(d, _, epoch_src)| effective_scope(d, epoch_src));
        self.record(
            "tool.request",
            json!({
                "call_id": call_id,
                "caller": caller,
                "tool": tool_id,
                "tool_def_hash": def.map(|d| d.def_hash()),
                "effect": def.map(|d| d.effect.as_str()),
                "approval_scope": scope.map(|s| s.as_str()),
                "args": self.inline_or_blob(&canonical)?,
            }),
        )?;
        let Some((def, _, epoch_src)) = entry else {
            return self.finish_denied(call_id, "unknown tool");
        };
        let scope = effective_scope(def, epoch_src);
        if let Err(e) = check_args(&def.input_schema, args) {
            return self.finish_error(call_id, &format!("invalid arguments: {e}"));
        }

        let request = self.policy_request(caller, tool_id, &canonical).expect("tool exists");
        let response = policy::evaluate(&self.policy, &request);
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

        let def_hash = def.def_hash();
        match response.decision {
            Decision::Deny => self.finish_denied(call_id, &format!("policy: {}", response.reason)),
            Decision::Allow => self.dispatch_auth(caller, tool_id, args, DispatchAuth::Allow, call_id),
            Decision::Ask => {
                if scope == ApprovalScope::WorkerSession {
                    let Some(epoch_provider) = epoch_src else {
                        return self.finish_denied(call_id, "approval: worker-session tool has no epoch provider");
                    };
                    let live = epoch_provider.live_epoch();
                    self.store.clear_if_epoch_mismatch(caller, tool_id, live);
                    if let Some(ep) = live {
                        if let Some(grant) = self.store.matching_session_grant(
                            caller,
                            tool_id,
                            &def_hash,
                            &self.policy.version,
                            ep,
                        ) {
                            self.record(
                                "approval.session_reused",
                                json!({
                                    "call_id": call_id,
                                    "grant_digest": grant.grant_digest,
                                    "granted_call_id": grant.granted_call_id,
                                    "worker_epoch": ep,
                                }),
                            )?;
                            return self.dispatch_auth(caller, tool_id, args, DispatchAuth::Session(ep), call_id);
                        }
                    }
                }

                if def.effect == EffectClass::NetworkRead {
                    if let Some(grant) = self.store.matching_network_read_grant(caller, &self.policy.version) {
                        self.record(
                            "approval.session_reused",
                            json!({
                                "call_id": call_id,
                                "grant_digest": grant.grant_digest,
                                "granted_call_id": grant.granted_call_id,
                                "effect": "network-read",
                            }),
                        )?;
                        return self.dispatch_auth(caller, tool_id, args, DispatchAuth::NetworkSession, call_id);
                    }
                }

                let nonce = random_hex(16);
                let digest = approval_digest(
                    scope,
                    caller,
                    tool_id,
                    &def_hash,
                    &canonical,
                    &self.policy.version,
                    &nonce,
                );
                self.record(
                    "approval.requested",
                    json!({ "call_id": call_id, "digest": digest, "scope": scope.as_str() }),
                )?;
                let outcome = self.approver.decide_ext(&ApprovalRequest {
                    schema_version: APPROVAL_SCHEMA_VERSION,
                    session: self.session.clone(),
                    call_id: call_id.to_string(),
                    digest: digest.clone(),
                    caller: caller.to_string(),
                    tool_id: tool_id.to_string(),
                    scope,
                    effect: def.effect,
                    canonical_args: canonical.clone(),
                    policy_reason: response.reason.clone(),
                });
                let reply = outcome.reply;
                if !reply.approved || reply.digest != digest {
                    let why = if reply.approved { "approver answered for a different digest" } else { "declined" };
                    self.record(
                        "approval.denied",
                        json!({ "call_id": call_id, "digest": digest, "by": reply.by, "why": why }),
                    )?;
                    return self.finish_denied(call_id, &format!("approval: {why}"));
                }
                self.store.grant(&digest);
                let effective_scope_str = if outcome.approve_for_session && def.effect == EffectClass::NetworkRead {
                    "network-session"
                } else {
                    scope.as_str()
                };
                self.record(
                    "approval.granted",
                    json!({
                        "call_id": call_id,
                        "digest": digest,
                        "scope": effective_scope_str,
                        "by": reply.by,
                    }),
                )?;
                let bind_session_digest = (scope == ApprovalScope::WorkerSession).then(|| digest.clone());
                let bind_network_read_digest =
                    (outcome.approve_for_session && def.effect == EffectClass::NetworkRead).then(|| digest.clone());
                self.dispatch_auth(
                    caller,
                    tool_id,
                    args,
                    DispatchAuth::Once {
                        nonce: &nonce,
                        bind_session_digest,
                        bind_network_read_digest,
                    },
                    call_id,
                )
            }
        }
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

    /// Re-verify policy and authorization immediately before calling the handler.
    fn dispatch_auth(
        &self,
        caller: &str,
        tool_id: &str,
        args: &Value,
        auth: DispatchAuth<'_>,
        call_id: &str,
    ) -> Result<CallOutcome, String> {
        let canonical = canonical_json(args);
        let Some(request) = self.policy_request(caller, tool_id, &canonical) else {
            return self.finish_denied(call_id, "unknown tool");
        };
        let (def, handler, epoch_src) = self.registry.get_entry(tool_id).expect("checked above");
        let scope = effective_scope(def, epoch_src);
        let (required_epoch, bind_session_digest) = match policy::evaluate(&self.policy, &request).decision {
            Decision::Deny => return self.finish_denied(call_id, "policy denies at dispatch"),
            Decision::Allow => (None, None),
            Decision::Ask => match auth {
                DispatchAuth::Allow => {
                    return self.finish_denied(call_id, "approval: missing authorization");
                }
                DispatchAuth::Once {
                    nonce,
                    bind_session_digest,
                    bind_network_read_digest,
                } => {
                    let digest = approval_digest(
                        scope,
                        caller,
                        tool_id,
                        &request.tool_def_hash,
                        &canonical,
                        &self.policy.version,
                        nonce,
                    );
                    if !self.store.consume(&digest) {
                        self.record("approval.rejected", json!({ "call_id": call_id, "digest": digest }))?;
                        return self.finish_denied(call_id, "approval: no unused approval for exactly this call");
                    }
                    self.record("approval.consumed", json!({ "call_id": call_id, "digest": digest }))?;
                    if def.effect == EffectClass::NetworkRead {
                        if let Some(grant_digest) = bind_network_read_digest {
                            self.record(
                                "approval.session_bound",
                                json!({
                                    "call_id": call_id,
                                    "grant_digest": grant_digest,
                                    "effect": "network-read",
                                }),
                            )?;
                            self.store.bind_network_read_grant(
                                caller,
                                NetworkReadGrant {
                                    policy_version: self.policy.version.clone(),
                                    grant_digest,
                                    granted_call_id: call_id.to_string(),
                                },
                            );
                        }
                    }
                    let bind = (scope == ApprovalScope::WorkerSession)
                        .then_some(bind_session_digest)
                        .flatten();
                    (None, bind)
                }
                DispatchAuth::NetworkSession => {
                    let valid = def.effect == EffectClass::NetworkRead
                        && self
                            .store
                            .matching_network_read_grant(caller, &self.policy.version)
                            .is_some();
                    if !valid {
                        self.record(
                            "approval.rejected",
                            json!({ "call_id": call_id, "effect": def.effect.as_str() }),
                        )?;
                        return self.finish_denied(call_id, "approval: network-read session grant is not valid");
                    }
                    (None, None)
                }
                DispatchAuth::Session(expected_ep) => {
                    let live = epoch_src.and_then(|e| e.live_epoch());
                    let valid = scope == ApprovalScope::WorkerSession
                        && live == Some(expected_ep)
                        && self
                            .store
                            .matching_session_grant(caller, tool_id, &request.tool_def_hash, &self.policy.version, expected_ep)
                            .is_some();
                    if !valid {
                        self.store.clear_session_grant(caller, tool_id);
                        self.record(
                            "approval.rejected",
                            json!({ "call_id": call_id, "expected_epoch": expected_ep, "actual_epoch": live }),
                        )?;
                        return self.finish_denied(call_id, "approval: worker session grant is no longer valid");
                    }
                    (Some(expected_ep), None)
                }
            },
        };

        let ctx = CallCtx::new(&self.session, caller, call_id, &self.ledger, required_epoch);
        match handler.call(&ctx, args) {
            Ok(value) => {
                let stored = self.inline_or_blob(&canonical_json(&value))?;
                self.record("tool.result", json!({ "call_id": call_id, "ok": true, "result": stored }))?;
                if scope == ApprovalScope::WorkerSession {
                    if let (Some(grant_digest), Some(ran_ep)) = (bind_session_digest, ctx.worker_epoch()) {
                        let live_now = epoch_src.and_then(|e| e.live_epoch());
                        if live_now == Some(ran_ep) {
                            self.record(
                                "approval.session_bound",
                                json!({ "call_id": call_id, "grant_digest": grant_digest, "worker_epoch": ran_ep }),
                            )?;
                            self.store.bind_session_grant(
                                caller,
                                tool_id,
                                SessionGrant {
                                    tool_def_hash: request.tool_def_hash,
                                    policy_version: self.policy.version.clone(),
                                    worker_epoch: ran_ep,
                                    grant_digest,
                                    granted_call_id: call_id.to_string(),
                                },
                            );
                        }
                    }
                }
                Ok(CallOutcome::Ok(value))
            }
            Err(e) => {
                self.store
                    .clear_if_epoch_mismatch(caller, tool_id, epoch_src.and_then(|s| s.live_epoch()));
                self.finish_error(call_id, &e)
            }
        }
    }

    #[cfg(test)]
    fn dispatch(
        &self,
        caller: &str,
        tool_id: &str,
        args: &Value,
        nonce: Option<&str>,
        call_id: &str,
    ) -> Result<CallOutcome, String> {
        self.dispatch_auth(
            caller,
            tool_id,
            args,
            DispatchAuth::Once {
                nonce: nonce.unwrap_or(""),
                bind_session_digest: None,
                bind_network_read_digest: None,
            },
            call_id,
        )
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
