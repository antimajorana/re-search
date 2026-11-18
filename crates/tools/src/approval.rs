//! Digest-bound, single-use approvals.
//!
//! An approval is for one exact call: the digest covers the caller, the tool
//! id, the tool-definition hash, the canonical arguments, the policy version
//! and a fresh nonce. The router grants it only if the approver echoes that
//! digest, and consumes it (once) immediately before dispatch.

use crate::canon::hash_fields;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;

pub fn approval_digest(
    caller: &str,
    tool_id: &str,
    tool_def_hash: &str,
    canonical_args: &str,
    policy_version: &str,
    nonce: &str,
) -> String {
    hash_fields(&["approval-v1", caller, tool_id, tool_def_hash, canonical_args, policy_version, nonce])
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

/// Asks a human, out of band. May block for as long as the human takes.
pub trait Approver: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply;
}

/// Granted digests that have not been used yet.
#[derive(Default)]
pub struct ApprovalStore {
    granted: Mutex<HashSet<String>>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_every_field() {
        let base = approval_digest("c", "t", "h", "{}", "p", "n");
        assert_eq!(base, approval_digest("c", "t", "h", "{}", "p", "n"));
        for other in [
            approval_digest("c2", "t", "h", "{}", "p", "n"),
            approval_digest("c", "t2", "h", "{}", "p", "n"),
            approval_digest("c", "t", "h2", "{}", "p", "n"),
            approval_digest("c", "t", "h", "{\"a\":1}", "p", "n"),
            approval_digest("c", "t", "h", "{}", "p2", "n"),
            approval_digest("c", "t", "h", "{}", "p", "n2"),
            approval_digest("ct", "", "h", "{}", "p", "n"),
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
}
