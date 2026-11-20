//! Research policy preset (`v0.4-research`) for workspace, LaTeX, and literature tools.

use policy::{Decision, EffectClass, Match, Policy, Rule};

pub const RESEARCH_POLICY_VERSION: &str = "v0.4-research";

fn effect_rule(id: &str, effect: EffectClass, decision: Decision) -> Rule {
    Rule {
        id: id.to_string(),
        caller: Match::Any,
        tool: Match::Any,
        effect: Some(effect),
        tool_def_hash: None,
        decision,
    }
}

/// Policy preset for Stage 6+ research sessions:
/// - `ObserveLocal`, `ReadWorkspace` => `Allow`
/// - `WriteWorkspace`, `ExecuteCode`, `NetworkRead` => `Ask` (also hard-clamped in `policy::evaluate`)
/// - Everything else (`NetworkEgress`, `MutateReversible`, `MutateIrreversible`) => `Deny`
pub fn research_policy() -> Policy {
    Policy {
        version: RESEARCH_POLICY_VERSION.to_string(),
        rules: vec![
            effect_rule("observe-local-allow", EffectClass::ObserveLocal, Decision::Allow),
            effect_rule("read-workspace-allow", EffectClass::ReadWorkspace, Decision::Allow),
            // The engine clamps `network-read` to ask anyway (query text is model-chosen and could
            // carry workspace content off the machine); the rule states that intent explicitly.
            effect_rule("network-read-ask", EffectClass::NetworkRead, Decision::Ask),
            effect_rule("write-workspace-ask", EffectClass::WriteWorkspace, Decision::Ask),
            effect_rule("execute-code-ask", EffectClass::ExecuteCode, Decision::Ask),
        ],
        default: Decision::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use policy::{evaluate, Request, SCHEMA_VERSION};

    fn req(tool: &str, effect: EffectClass) -> Request {
        Request {
            schema_version: SCHEMA_VERSION,
            caller: "agent:test:s-1".into(),
            tool_id: tool.into(),
            tool_def_hash: "0".repeat(64),
            effect,
            canonical_args: "{}".into(),
            policy_version: RESEARCH_POLICY_VERSION.into(),
        }
    }

    #[test]
    fn research_policy_decisions_and_serde_kebab_case() {
        let p = research_policy();
        assert_eq!(p.version, "v0.4-research");
        assert_eq!(evaluate(&p, &req("ledger_note", EffectClass::ObserveLocal)).decision, Decision::Allow);
        assert_eq!(evaluate(&p, &req("fs_read", EffectClass::ReadWorkspace)).decision, Decision::Allow);
        assert_eq!(evaluate(&p, &req("arxiv_search", EffectClass::NetworkRead)).decision, Decision::Ask);
        assert_eq!(evaluate(&p, &req("fs_write", EffectClass::WriteWorkspace)).decision, Decision::Ask);
        assert_eq!(evaluate(&p, &req("sympy_eval", EffectClass::ExecuteCode)).decision, Decision::Ask);
        assert_eq!(evaluate(&p, &req("raw_post", EffectClass::NetworkEgress)).decision, Decision::Deny);

        for (variant, wire) in [
            (EffectClass::ReadWorkspace, "\"read-workspace\""),
            (EffectClass::WriteWorkspace, "\"write-workspace\""),
            (EffectClass::NetworkRead, "\"network-read\""),
        ] {
            let encoded = serde_json::to_string(&variant).unwrap();
            assert_eq!(encoded, wire);
            let decoded: EffectClass = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, variant);
        }
    }
}
