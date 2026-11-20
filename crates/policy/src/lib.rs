//! Pure allow / ask / deny policy engine.
//!
//! `evaluate` is a pure function of a [`Policy`] and a [`Request`]. It does no
//! I/O and keeps no state. It is policy and audit, not a security boundary:
//! it decides what the core is willing to dispatch, it does not contain code.
//!
//! Precedence among matching rules is deny > ask > allow. If no rule matches,
//! the policy's default applies (deny unless stated otherwise).
//!
//! Built-in invariants that cannot be configured away: tools of effect class
//! `execute-code`, `write-workspace` or `network-read` are never allowed without
//! asking a human.

use serde::{Deserialize, Serialize};

/// Version of the request/response wire types in this crate.
pub const SCHEMA_VERSION: u32 = 1;

/// What a tool can do to the world. Ordered roughly by how hard it is to undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectClass {
    ObserveLocal,
    NetworkEgress,
    MutateReversible,
    MutateIrreversible,
    ExecuteCode,
    ReadWorkspace,
    WriteWorkspace,
    NetworkRead,
}

impl EffectClass {
    pub fn as_str(self) -> &'static str {
        match self {
            EffectClass::ObserveLocal => "observe-local",
            EffectClass::NetworkEgress => "network-egress",
            EffectClass::MutateReversible => "mutate-reversible",
            EffectClass::MutateIrreversible => "mutate-irreversible",
            EffectClass::ExecuteCode => "execute-code",
            EffectClass::ReadWorkspace => "read-workspace",
            EffectClass::WriteWorkspace => "write-workspace",
            EffectClass::NetworkRead => "network-read",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    // Order matters: the strictest decision is the greatest.
    Allow,
    Ask,
    Deny,
}

/// One policy question. The caller identity must come from the channel the
/// request arrived on, never from the arguments of the request itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub schema_version: u32,
    pub caller: String,
    pub tool_id: String,
    /// SHA-256 (hex) of the tool definition the caller was shown.
    pub tool_def_hash: String,
    pub effect: EffectClass,
    /// Canonical JSON text of the arguments. Carried for audit and approval
    /// binding; v0 rules do not match on argument content.
    pub canonical_args: String,
    pub policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub schema_version: u32,
    pub decision: Decision,
    /// Id of the rule that fixed the decision, if any.
    pub matched_rule: Option<String>,
    pub reason: String,
    pub policy_version: String,
}

/// String matcher used for callers and tool ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Match {
    Any,
    Exact(String),
    Prefix(String),
}

impl Match {
    fn matches(&self, s: &str) -> bool {
        match self {
            Match::Any => true,
            Match::Exact(e) => s == e,
            Match::Prefix(p) => s.starts_with(p.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub caller: Match,
    pub tool: Match,
    /// If set, the rule applies only to tools of this effect class.
    pub effect: Option<EffectClass>,
    /// If set, the rule applies only while the tool definition has this hash,
    /// so a changed definition falls back to the default.
    pub tool_def_hash: Option<String>,
    pub decision: Decision,
}

impl Rule {
    fn matches(&self, r: &Request) -> bool {
        self.caller.matches(&r.caller)
            && self.tool.matches(&r.tool_id)
            && self.effect.is_none_or(|e| e == r.effect)
            && self.tool_def_hash.as_ref().is_none_or(|h| *h == r.tool_def_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub version: String,
    pub rules: Vec<Rule>,
    /// Applied when no rule matches.
    pub default: Decision,
}

impl Policy {
    /// A policy with no rules that denies everything.
    pub fn deny_all(version: &str) -> Self {
        Policy { version: version.to_string(), rules: Vec::new(), default: Decision::Deny }
    }
}

fn respond(p: &Policy, decision: Decision, rule: Option<&str>, reason: &str) -> Response {
    Response {
        schema_version: SCHEMA_VERSION,
        decision,
        matched_rule: rule.map(str::to_string),
        reason: reason.to_string(),
        policy_version: p.version.clone(),
    }
}

/// Decide one request. Pure: same inputs, same output.
pub fn evaluate(policy: &Policy, req: &Request) -> Response {
    if req.schema_version != SCHEMA_VERSION {
        return respond(policy, Decision::Deny, None, "unsupported request schema version");
    }
    if req.policy_version != policy.version {
        return respond(policy, Decision::Deny, None, "request names a different policy version");
    }
    // Strictest matching rule wins; among equals, the first listed.
    let mut best: Option<&Rule> = None;
    for rule in policy.rules.iter().filter(|r| r.matches(req)) {
        if best.is_none_or(|b| rule.decision > b.decision) {
            best = Some(rule);
        }
    }
    let (mut decision, rule_id, mut reason) = match best {
        Some(r) => (r.decision, Some(r.id.as_str()), "matched rule"),
        None => (policy.default, None, "no rule matched; policy default"),
    };
    if decision == Decision::Allow {
        match req.effect {
            EffectClass::ExecuteCode => {
                decision = Decision::Ask;
                reason = "execute-code is never allowed without asking in v0 (no sandbox)";
            }
            EffectClass::WriteWorkspace => {
                decision = Decision::Ask;
                reason = "write-workspace is never allowed without human approval";
            }
            EffectClass::NetworkRead => {
                decision = Decision::Ask;
                reason = "network-read is never allowed without asking: the model chooses the query text";
            }
            _ => {}
        }
    }
    respond(policy, decision, rule_id, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, tool: Match, decision: Decision) -> Rule {
        Rule { id: id.into(), caller: Match::Any, tool, effect: None, tool_def_hash: None, decision }
    }

    fn req(tool: &str, effect: EffectClass) -> Request {
        Request {
            schema_version: SCHEMA_VERSION,
            caller: "cli:session-1".into(),
            tool_id: tool.into(),
            tool_def_hash: "abc".into(),
            effect,
            canonical_args: "{}".into(),
            policy_version: "p1".into(),
        }
    }

    fn policy(rules: Vec<Rule>) -> Policy {
        Policy { version: "p1".into(), rules, default: Decision::Deny }
    }

    #[test]
    fn default_is_deny() {
        let r = evaluate(&policy(vec![]), &req("x", EffectClass::ObserveLocal));
        assert_eq!(r.decision, Decision::Deny);
        assert_eq!(r.matched_rule, None);
    }

    #[test]
    fn deny_beats_ask_beats_allow_regardless_of_order() {
        let allow = rule("a", Match::Any, Decision::Allow);
        let ask = rule("k", Match::Prefix("no".into()), Decision::Ask);
        let deny = rule("d", Match::Exact("note".into()), Decision::Deny);
        for rules in [
            vec![allow.clone(), ask.clone(), deny.clone()],
            vec![deny.clone(), ask.clone(), allow.clone()],
        ] {
            let p = policy(rules);
            let r = evaluate(&p, &req("note", EffectClass::ObserveLocal));
            assert_eq!((r.decision, r.matched_rule.as_deref()), (Decision::Deny, Some("d")));
            let r = evaluate(&p, &req("nope", EffectClass::ObserveLocal));
            assert_eq!((r.decision, r.matched_rule.as_deref()), (Decision::Ask, Some("k")));
            let r = evaluate(&p, &req("other", EffectClass::ObserveLocal));
            assert_eq!((r.decision, r.matched_rule.as_deref()), (Decision::Allow, Some("a")));
        }
    }

    #[test]
    fn execute_code_is_clamped_to_ask() {
        let p = policy(vec![rule("a", Match::Any, Decision::Allow)]);
        let r = evaluate(&p, &req("sympy_eval", EffectClass::ExecuteCode));
        assert_eq!(r.decision, Decision::Ask);
        // A deny rule is not weakened by the clamp.
        let p = policy(vec![rule("d", Match::Any, Decision::Deny)]);
        assert_eq!(evaluate(&p, &req("sympy_eval", EffectClass::ExecuteCode)).decision, Decision::Deny);
    }

    #[test]
    fn version_mismatches_deny() {
        let p = policy(vec![rule("a", Match::Any, Decision::Allow)]);
        let mut r = req("x", EffectClass::ObserveLocal);
        r.policy_version = "p0".into();
        assert_eq!(evaluate(&p, &r).decision, Decision::Deny);
        let mut r = req("x", EffectClass::ObserveLocal);
        r.schema_version = 99;
        assert_eq!(evaluate(&p, &r).decision, Decision::Deny);
    }

    #[test]
    fn rule_pinned_to_definition_hash_stops_matching_when_definition_changes() {
        let mut pinned = rule("a", Match::Exact("x".into()), Decision::Allow);
        pinned.tool_def_hash = Some("abc".into());
        let p = policy(vec![pinned]);
        assert_eq!(evaluate(&p, &req("x", EffectClass::ObserveLocal)).decision, Decision::Allow);
        let mut changed = req("x", EffectClass::ObserveLocal);
        changed.tool_def_hash = "def".into();
        assert_eq!(evaluate(&p, &changed).decision, Decision::Deny);
    }

    #[test]
    fn effect_and_caller_filters() {
        let mut r1 = rule("net", Match::Any, Decision::Ask);
        r1.effect = Some(EffectClass::NetworkEgress);
        let mut r2 = rule("cli", Match::Any, Decision::Allow);
        r2.caller = Match::Prefix("cli:".into());
        let p = policy(vec![r1, r2]);
        assert_eq!(evaluate(&p, &req("x", EffectClass::NetworkEgress)).decision, Decision::Ask);
        assert_eq!(evaluate(&p, &req("x", EffectClass::ObserveLocal)).decision, Decision::Allow);
        let mut other = req("x", EffectClass::ObserveLocal);
        other.caller = "unknown".into();
        assert_eq!(evaluate(&p, &other).decision, Decision::Deny);
    }

    #[test]
    fn evaluate_is_deterministic() {
        let p = policy(vec![rule("a", Match::Any, Decision::Ask)]);
        let q = req("x", EffectClass::ObserveLocal);
        assert_eq!(evaluate(&p, &q), evaluate(&p, &q));
    }

    #[test]
    fn write_workspace_is_clamped_to_ask_while_deny_stays_deny() {
        let p = policy(vec![rule("a", Match::Any, Decision::Allow)]);
        let r = evaluate(&p, &req("fs_write", EffectClass::WriteWorkspace));
        assert_eq!(r.decision, Decision::Ask);
        assert_eq!(r.matched_rule.as_deref(), Some("a"));

        let p_ask = policy(vec![rule("k", Match::Any, Decision::Ask)]);
        let r_ask = evaluate(&p_ask, &req("latex_patch", EffectClass::WriteWorkspace));
        assert_eq!((r_ask.decision, r_ask.matched_rule.as_deref()), (Decision::Ask, Some("k")));

        let p_deny = policy(vec![rule("d", Match::Any, Decision::Deny)]);
        assert_eq!(evaluate(&p_deny, &req("fs_write", EffectClass::WriteWorkspace)).decision, Decision::Deny);
    }

    #[test]
    fn stage6_effect_classes_allow_read_but_clamp_network_read_to_ask() {
        assert_eq!(EffectClass::ReadWorkspace.as_str(), "read-workspace");
        assert_eq!(EffectClass::WriteWorkspace.as_str(), "write-workspace");
        assert_eq!(EffectClass::NetworkRead.as_str(), "network-read");

        let mut r_read = rule("rw", Match::Any, Decision::Allow);
        r_read.effect = Some(EffectClass::ReadWorkspace);
        let mut r_net = rule("nr", Match::Any, Decision::Allow);
        r_net.effect = Some(EffectClass::NetworkRead);
        let p = policy(vec![r_read, r_net]);

        assert_eq!(evaluate(&p, &req("fs_read", EffectClass::ReadWorkspace)).decision, Decision::Allow);
        // An allow rule for network-read is clamped to ask (user decision 2026-11-20: the
        // model-chosen query text can carry workspace content off the machine).
        let r = evaluate(&p, &req("arxiv_search", EffectClass::NetworkRead));
        assert_eq!((r.decision, r.matched_rule.as_deref()), (Decision::Ask, Some("nr")));
        assert_eq!(evaluate(&p, &req("other_net", EffectClass::NetworkEgress)).decision, Decision::Deny);
        // A deny rule is not weakened by the clamp.
        let p_deny = policy(vec![rule("d", Match::Any, Decision::Deny)]);
        assert_eq!(evaluate(&p_deny, &req("arxiv_search", EffectClass::NetworkRead)).decision, Decision::Deny);
    }
}
