//! Policy presets for Research (`v0.4-research`) and Solve (`v0.5-solve`).

use crate::{
    arxiv, fs_tools, literature, proof_dag, sanity, solve_defs, tex_audit,
};
use policy::{Decision, EffectClass, Match, Policy, Rule};
use tools::builtin;

pub const RESEARCH_POLICY_VERSION: &str = "v0.4-research";

/// Policy version belonging to tool profile `solve/1` (`solve.md` 5.1, rule R18;
/// `00-overview.md` 3.1 row 12, 8.2 decision 1).
pub const SOLVE_POLICY_VERSION: &str = "v0.5-solve";

/// Stable host caller id for automatic and host-initiated `check_run` invocations (`solve.md` 5.1, R18).
pub const SOLVE_CHECK_CALLER: &str = "host:check";

/// Stable verifier caller id for the Solve verifier loop (`solve.md` 0.3, 5.3).
pub const SOLVE_VERIFIER_CALLER: &str = "verifier:solve";

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

fn solve_tool_rule(tool_id: &str, effect: EffectClass, decision: Decision) -> Rule {
    let suffix = match decision {
        Decision::Allow => "allow",
        Decision::Ask => "ask",
        Decision::Deny => "deny",
    };
    Rule {
        id: format!("solve-{tool_id}-{suffix}"),
        caller: Match::Any,
        tool: Match::Exact(tool_id.to_string()),
        effect: Some(effect),
        tool_def_hash: None,
        decision,
    }
}

fn deny_caller_tool(prefix: &str, caller: &str, tool_id: &str) -> Rule {
    Rule {
        id: format!("{prefix}-deny-{tool_id}"),
        caller: Match::Exact(caller.to_string()),
        tool: Match::Exact(tool_id.to_string()),
        effect: None,
        tool_def_hash: None,
        decision: Decision::Deny,
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

/// Policy preset for Solve mode profile `solve/1` (`v0.5-solve`, `solve.md` 5.1, rule R18):
/// - Default `Deny`: only the 16 `solve/1` tool ids (plus MCP-only `set_session_title`) have explicit rules.
/// - Every `ExecuteCode` tool (`sympy_eval`, `check_run`) is `Ask` (with per-caller
///   `ApprovalScope::WorkerSession` on its respective worker epoch).
/// - Every `NetworkRead` tool (`arxiv_search`, `literature_search`) is `Ask`.
/// - Read/observe tools in `solve/1` are `Allow`.
/// - `host:check` is restricted to `check_run` (`Ask`) and denied all other tools.
/// - `verifier:solve` is restricted to verifier tools (`expr_show`, `ledger_query`,
///   `read_paper_lines`, `search_paper_passages`, `sympy_eval`, `check_run`) and denied
///   solver-only tools (`solve.md` 5.1 & 5.3).
pub fn solve_policy() -> Policy {
    let mut rules = vec![
        // ExecuteCode -> Ask (also hard-clamped in policy::evaluate)
        solve_tool_rule(builtin::SYMPY_EVAL, EffectClass::ExecuteCode, Decision::Ask),
        solve_tool_rule(solve_defs::CHECK_RUN, EffectClass::ExecuteCode, Decision::Ask),
        // NetworkRead -> Ask (also hard-clamped in policy::evaluate)
        solve_tool_rule(arxiv::ARXIV_SEARCH, EffectClass::NetworkRead, Decision::Ask),
        solve_tool_rule(literature::LITERATURE_SEARCH, EffectClass::NetworkRead, Decision::Ask),
        // ObserveLocal -> Allow
        solve_tool_rule(solve_defs::EXPR_SHOW, EffectClass::ObserveLocal, Decision::Allow),
        solve_tool_rule(solve_defs::STEP_RECORD, EffectClass::ObserveLocal, Decision::Allow),
        solve_tool_rule(solve_defs::SOLVE_FINISH, EffectClass::ObserveLocal, Decision::Allow),
        solve_tool_rule(builtin::LEDGER_NOTE, EffectClass::ObserveLocal, Decision::Allow),
        solve_tool_rule(proof_dag::LEDGER_QUERY, EffectClass::ObserveLocal, Decision::Allow),
        solve_tool_rule(proof_dag::LEDGER_SUPERSEDE, EffectClass::ObserveLocal, Decision::Allow),
        solve_tool_rule(sanity::CHECK_EXPRESSION_SANITY, EffectClass::ObserveLocal, Decision::Allow),
        // ReadWorkspace -> Allow
        solve_tool_rule(fs_tools::FS_READ, EffectClass::ReadWorkspace, Decision::Allow),
        solve_tool_rule(fs_tools::FS_LIST, EffectClass::ReadWorkspace, Decision::Allow),
        solve_tool_rule("read_paper_lines", EffectClass::ReadWorkspace, Decision::Allow),
        solve_tool_rule("search_paper_passages", EffectClass::ReadWorkspace, Decision::Allow),
        solve_tool_rule(tex_audit::TEX_AUDIT, EffectClass::ReadWorkspace, Decision::Allow),
        // MCP-only host-handled title tool registered via `ToolProfile::mcp_defs()` (`core.exe`)
        solve_tool_rule(builtin::SET_SESSION_TITLE, EffectClass::ObserveLocal, Decision::Allow),
    ];

    // `host:check` may only call `check_run` (`solve.md` 5.1).
    for &tool_id in solve_defs::SOLVE_V1_TOOL_IDS {
        if tool_id != solve_defs::CHECK_RUN {
            rules.push(deny_caller_tool("host-check", SOLVE_CHECK_CALLER, tool_id));
        }
    }
    rules.push(deny_caller_tool("host-check", SOLVE_CHECK_CALLER, builtin::SET_SESSION_TITLE));

    // `verifier:solve` may not call solver-only tools (`solve.md` 5.1 & 5.3).
    const VERIFIER_ALLOWED: &[&str] = &[
        solve_defs::EXPR_SHOW,
        proof_dag::LEDGER_QUERY,
        "read_paper_lines",
        "search_paper_passages",
        builtin::SYMPY_EVAL,
        solve_defs::CHECK_RUN,
    ];
    for &tool_id in solve_defs::SOLVE_V1_TOOL_IDS {
        if !VERIFIER_ALLOWED.contains(&tool_id) {
            rules.push(deny_caller_tool("verifier-solve", SOLVE_VERIFIER_CALLER, tool_id));
        }
    }
    rules.push(deny_caller_tool("verifier-solve", SOLVE_VERIFIER_CALLER, builtin::SET_SESSION_TITLE));

    Policy {
        version: SOLVE_POLICY_VERSION.to_string(),
        rules,
        default: Decision::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use policy::{evaluate, Request, SCHEMA_VERSION};

    fn req(tool: &str, effect: EffectClass) -> Request {
        req_with("agent:test:s-1", tool, effect, RESEARCH_POLICY_VERSION)
    }

    fn req_with(caller: &str, tool: &str, effect: EffectClass, version: &str) -> Request {
        Request {
            schema_version: SCHEMA_VERSION,
            caller: caller.into(),
            tool_id: tool.into(),
            tool_def_hash: "0".repeat(64),
            effect,
            canonical_args: "{}".into(),
            policy_version: version.into(),
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

    #[test]
    fn solve_policy_covers_all_16_tools_denies_excluded_and_scopes_callers() {
        let p = solve_policy();
        assert_eq!(p.version, SOLVE_POLICY_VERSION);
        assert_eq!(p.default, Decision::Deny);

        let defs = solve_defs::solve_v1_defs();
        assert_eq!(defs.len(), 16);
        for def in &defs {
            let resp = evaluate(&p, &req_with("solver:solve", &def.id, def.effect, SOLVE_POLICY_VERSION));
            let expected = match def.effect {
                EffectClass::ExecuteCode | EffectClass::NetworkRead => Decision::Ask,
                EffectClass::ObserveLocal | EffectClass::ReadWorkspace => Decision::Allow,
                _ => Decision::Deny,
            };
            assert_eq!(resp.decision, expected, "solver decision for `{}`", def.id);
            assert!(resp.matched_rule.is_some(), "missing explicit rule for `{}`", def.id);
        }

        // MCP `set_session_title` is allowed for solver, denied for `host:check` and `verifier:solve`.
        assert_eq!(
            evaluate(&p, &req_with("solver:solve", builtin::SET_SESSION_TITLE, EffectClass::ObserveLocal, SOLVE_POLICY_VERSION)).decision,
            Decision::Allow
        );
        assert_eq!(
            evaluate(&p, &req_with(SOLVE_CHECK_CALLER, builtin::SET_SESSION_TITLE, EffectClass::ObserveLocal, SOLVE_POLICY_VERSION)).decision,
            Decision::Deny
        );
        assert_eq!(
            evaluate(&p, &req_with(SOLVE_VERIFIER_CALLER, builtin::SET_SESSION_TITLE, EffectClass::ObserveLocal, SOLVE_POLICY_VERSION)).decision,
            Decision::Deny
        );

        // Excluded tools are denied by default.
        for (excluded, effect) in [
            ("doc_ingest", EffectClass::ReadWorkspace),
            ("ledger_append", EffectClass::ObserveLocal),
            ("lean_check", EffectClass::ExecuteCode),
            ("search_formal_library", EffectClass::ReadWorkspace),
            ("fs_write", EffectClass::WriteWorkspace),
            ("latex_patch", EffectClass::WriteWorkspace),
            ("latex_compile", EffectClass::ExecuteCode),
            ("record_paper_map", EffectClass::ReadWorkspace),
            ("read_map_node", EffectClass::ReadWorkspace),
            ("idea_search", EffectClass::NetworkRead),
        ] {
            let resp = evaluate(&p, &req_with("solver:solve", excluded, effect, SOLVE_POLICY_VERSION));
            assert_eq!(resp.decision, Decision::Deny, "excluded tool `{excluded}` must be Deny");
        }

        // `host:check` may only reach `Ask` on `check_run`; everything else is Deny.
        for def in &defs {
            let resp = evaluate(&p, &req_with(SOLVE_CHECK_CALLER, &def.id, def.effect, SOLVE_POLICY_VERSION));
            let expected = if def.id == solve_defs::CHECK_RUN { Decision::Ask } else { Decision::Deny };
            assert_eq!(resp.decision, expected, "host:check decision for `{}`", def.id);
        }

        // `verifier:solve` gets only verifier tools.
        for def in &defs {
            let resp = evaluate(&p, &req_with(SOLVE_VERIFIER_CALLER, &def.id, def.effect, SOLVE_POLICY_VERSION));
            let expected = match def.id.as_str() {
                "expr_show" | "ledger_query" | "read_paper_lines" | "search_paper_passages" => Decision::Allow,
                "sympy_eval" | "check_run" => Decision::Ask,
                _ => Decision::Deny,
            };
            assert_eq!(resp.decision, expected, "verifier:solve decision for `{}`", def.id);
        }
    }
}
