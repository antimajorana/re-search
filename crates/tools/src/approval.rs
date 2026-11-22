//! Digest-bound approvals: single-use (`Once`) or bound to a live worker
//! process (`WorkerSession`).
//!
//! Every human approval begins with a SHA-256 digest covering the scope,
//! caller, tool id, tool-definition hash, canonical arguments, policy version
//! and a fresh nonce. For `WorkerSession` tools, once the approved call runs on
//! a live worker process, subsequent calls by the same caller with the same
//! tool definition and policy version may reuse that grant only while that
//! exact worker epoch remains alive.

use crate::canon::hash_fields;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Version of the loopback approval channel wire protocol shared by supervisor
/// and MCP server.
pub const APPROVAL_CHANNEL_VERSION: u32 = 3;

/// Version of [`ApprovalRequest`] payloads.
pub const APPROVAL_SCHEMA_VERSION: u32 = 2;

/// How broadly a single human approval applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalScope {
    /// Approved once for one exact call digest.
    Once,
    /// Approved for the initial call and subsequent calls while the same
    /// worker process epoch stays alive.
    WorkerSession,
}

impl ApprovalScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalScope::Once => "once",
            ApprovalScope::WorkerSession => "worker-session",
        }
    }
}

/// Reports the currently live worker process epoch (`1, 2, ...`), or `None`
/// when no worker process is running, and whether the worker runs inside a
/// filesystem- and network-restricted sandbox (`WorkerSession` reuse is gated
/// on `is_contained()`).
pub trait WorkerEpoch: Send + Sync {
    fn live_epoch(&self) -> Option<u64>;
    fn is_contained(&self) -> bool;
}

pub fn approval_digest(
    scope: ApprovalScope,
    caller: &str,
    tool_id: &str,
    tool_def_hash: &str,
    canonical_args: &str,
    policy_version: &str,
    nonce: &str,
) -> String {
    hash_fields(&[
        "approval-v2",
        scope.as_str(),
        caller,
        tool_id,
        tool_def_hash,
        canonical_args,
        policy_version,
        nonce,
    ])
}

/// What the human is shown. `schema_version` covers this struct and
/// [`ApprovalReply`] on whatever channel carries them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub schema_version: u32,
    pub session: String,
    pub call_id: String,
    pub digest: String,
    pub caller: String,
    pub tool_id: String,
    pub scope: ApprovalScope,
    pub effect: EffectClass,
    pub canonical_args: String,
    pub policy_reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalReply {
    pub approved: bool,
    /// The digest the approver saw and decided on.
    pub digest: String,
    /// Who answered, for the audit record (for example "console").
    pub by: String,
}

/// Extended approval reply allowing an explicit human opt-in to session-scoped
/// `EffectClass::NetworkRead` literature searches (`arxiv_search` / `literature_search`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalOutcome {
    pub reply: ApprovalReply,
    #[serde(default)]
    pub approve_for_session: bool,
}

/// Asks a human, out of band. May block for as long as the human takes.
pub trait Approver: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply;

    fn decide_ext(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        ApprovalOutcome {
            reply: self.decide(request),
            approve_for_session: false,
        }
    }
}

/// An active approval bound to one live worker epoch for `(caller, tool_id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGrant {
    pub tool_def_hash: String,
    pub policy_version: String,
    pub worker_epoch: u64,
    pub grant_digest: String,
    pub granted_call_id: String,
}

/// An active human-approved session grant strictly for `EffectClass::NetworkRead`
/// tools (`arxiv_search`, `literature_search`) within one `Router` session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkReadGrant {
    pub policy_version: String,
    pub grant_digest: String,
    pub granted_call_id: String,
}

/// Granted single-use digests, active worker-session grants, and session-scoped
/// `NetworkRead` grants.
#[derive(Default)]
pub struct ApprovalStore {
    granted: Mutex<HashSet<String>>,
    session_grants: Mutex<HashMap<(String, String), SessionGrant>>,
    network_read_grants: Mutex<HashMap<String, NetworkReadGrant>>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grant(&self, digest: &str) {
        self.granted.lock().unwrap().insert(digest.to_string());
    }

    /// True exactly once per granted digest.
    pub fn consume(&self, digest: &str) -> bool {
        self.granted.lock().unwrap().remove(digest)
    }

    pub fn matching_network_read_grant(&self, caller: &str, policy_version: &str) -> Option<NetworkReadGrant> {
        let grants = self.network_read_grants.lock().unwrap();
        let grant = grants.get(caller)?;
        (grant.policy_version == policy_version).then(|| grant.clone())
    }

    pub fn bind_network_read_grant(&self, caller: &str, grant: NetworkReadGrant) {
        self.network_read_grants
            .lock()
            .unwrap()
            .insert(caller.to_string(), grant);
    }

    pub fn matching_session_grant(
        &self,
        caller: &str,
        tool_id: &str,
        tool_def_hash: &str,
        policy_version: &str,
        live_epoch: u64,
    ) -> Option<SessionGrant> {
        let grants = self.session_grants.lock().unwrap();
        let grant = grants.get(&(caller.to_string(), tool_id.to_string()))?;
        if grant.worker_epoch == live_epoch
            && grant.tool_def_hash == tool_def_hash
            && grant.policy_version == policy_version
        {
            Some(grant.clone())
        } else {
            None
        }
    }

    pub fn bind_session_grant(&self, caller: &str, tool_id: &str, grant: SessionGrant) {
        self.session_grants
            .lock()
            .unwrap()
            .insert((caller.to_string(), tool_id.to_string()), grant);
    }

    pub fn clear_session_grant(&self, caller: &str, tool_id: &str) {
        self.session_grants.lock().unwrap().remove(&(caller.to_string(), tool_id.to_string()));
    }

    pub fn clear_if_epoch_mismatch(&self, caller: &str, tool_id: &str, current: Option<u64>) {
        let mut grants = self.session_grants.lock().unwrap();
        let key = (caller.to_string(), tool_id.to_string());
        if let Some(grant) = grants.get(&key) {
            if current != Some(grant.worker_epoch) {
                grants.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_every_field() {
        let base = approval_digest(ApprovalScope::Once, "c", "t", "h", "{}", "p", "n");
        assert_eq!(base, approval_digest(ApprovalScope::Once, "c", "t", "h", "{}", "p", "n"));
        for other in [
            approval_digest(ApprovalScope::WorkerSession, "c", "t", "h", "{}", "p", "n"),
            approval_digest(ApprovalScope::Once, "c2", "t", "h", "{}", "p", "n"),
            approval_digest(ApprovalScope::Once, "c", "t2", "h", "{}", "p", "n"),
            approval_digest(ApprovalScope::Once, "c", "t", "h2", "{}", "p", "n"),
            approval_digest(ApprovalScope::Once, "c", "t", "h", "{\"a\":1}", "p", "n"),
            approval_digest(ApprovalScope::Once, "c", "t", "h", "{}", "p2", "n"),
            approval_digest(ApprovalScope::Once, "c", "t", "h", "{}", "p", "n2"),
            approval_digest(ApprovalScope::Once, "ct", "", "h", "{}", "p", "n"),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn a_grant_is_single_use() {
        let store = ApprovalStore::new();
        assert!(!store.consume("d"));
        store.grant("d");
        assert!(!store.consume("other"));
        assert!(store.consume("d"));
        assert!(!store.consume("d"));
    }

    #[test]
    fn session_grant_matches_only_same_caller_tool_def_policy_and_epoch() {
        let store = ApprovalStore::new();
        store.bind_session_grant(
            "c1",
            "sympy_eval",
            SessionGrant {
                tool_def_hash: "h1".into(),
                policy_version: "v0.2".into(),
                worker_epoch: 1,
                grant_digest: "d1".into(),
                granted_call_id: "call-1".into(),
            },
        );
        assert!(store.matching_session_grant("c1", "sympy_eval", "h1", "v0.2", 1).is_some());
        assert!(store.matching_session_grant("c2", "sympy_eval", "h1", "v0.2", 1).is_none());
        assert!(store.matching_session_grant("c1", "other", "h1", "v0.2", 1).is_none());
        assert!(store.matching_session_grant("c1", "sympy_eval", "h2", "v0.2", 1).is_none());
        assert!(store.matching_session_grant("c1", "sympy_eval", "h1", "v0.3", 1).is_none());
        assert!(store.matching_session_grant("c1", "sympy_eval", "h1", "v0.2", 2).is_none());

        store.clear_if_epoch_mismatch("c1", "sympy_eval", Some(1));
        assert!(store.matching_session_grant("c1", "sympy_eval", "h1", "v0.2", 1).is_some());
        store.clear_if_epoch_mismatch("c1", "sympy_eval", None);
        assert!(store.matching_session_grant("c1", "sympy_eval", "h1", "v0.2", 1).is_none());
    }
}
