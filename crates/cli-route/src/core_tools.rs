//! The tool set `core.exe` serves, in one place.
//!
//! `core.exe` registers its tools with [`register_core_tools`]; the supervisor computes
//! the list it expects the vendor CLI to offer with [`core_tool_set`], which runs the same
//! registration with stub handlers. So the `system/init` check compares against exactly
//! the registry `core.exe` will serve for the same research setting.

use policy::Policy;
use research_tools::tool_profile::{
    lookup as lookup_profile, register_profile, ProfileHostTools, SolveCheckHost,
    PROFILE_RESEARCH_V1,
};
use research_tools::ResearchEnv;
use serde_json::Value;
use std::sync::Arc;
use tools::{builtin, CallCtx, Registry, ToolDef, ToolHandler, WorkerEpoch};

/// Environment variable naming the frozen tool profile served by `core.exe` (`00-overview.md` 2.2, F1).
pub const CORE_TOOL_PROFILE_ENV: &str = "CORE_TOOL_PROFILE";

fn cli_title_handler() -> Arc<dyn ToolHandler> {
    Arc::new(|ctx: &CallCtx<'_>, input: &Value| {
        let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
        record_cli_title(&ledger, ctx.session, input)
    })
}

/// Register the tools for `profile_name` plus MCP `set_session_title`, with optional Solve check
/// host bindings, returning the profile's policy. Fails closed if `profile_name` is unbuilt or unknown.
pub fn register_core_tools_for_profile_with_solve(
    registry: &mut Registry,
    sympy_epoch: Arc<dyn WorkerEpoch>,
    sympy_handler: Arc<dyn ToolHandler>,
    env: Arc<ResearchEnv>,
    profile_name: &str,
    solve_check: Option<SolveCheckHost>,
) -> Result<Policy, String> {
    let profile = lookup_profile(profile_name).map_err(|e| e.to_string())?;
    register_profile(
        &profile,
        registry,
        env,
        ProfileHostTools {
            sympy_epoch,
            sympy_handler,
            session_title_handler: Some(cli_title_handler()),
            solve_check,
        },
    )
}

/// Register the tools for `profile_name` plus MCP `set_session_title`, returning the profile's
/// policy. Fails closed if `profile_name` is unbuilt or unknown.
pub fn register_core_tools_for_profile(
    registry: &mut Registry,
    sympy_epoch: Arc<dyn WorkerEpoch>,
    sympy_handler: Arc<dyn ToolHandler>,
    env: Arc<ResearchEnv>,
    profile_name: &str,
) -> Result<Policy, String> {
    register_core_tools_for_profile_with_solve(
        registry,
        sympy_epoch,
        sympy_handler,
        env,
        profile_name,
        None,
    )
}

/// Register `sympy_eval` (with the caller's worker), `ledger_note`, and, when `research`
/// is given, the `research/1` profile tools; return the matching policy (`v0.4-research` with
/// research tools, the built-in `v0.2` otherwise).
pub fn register_core_tools(
    registry: &mut Registry,
    sympy_epoch: Arc<dyn WorkerEpoch>,
    sympy_handler: Arc<dyn ToolHandler>,
    research: Option<Arc<ResearchEnv>>,
) -> Policy {
    match research {
        Some(env) => register_core_tools_for_profile(
            registry,
            sympy_epoch,
            sympy_handler,
            env,
            PROFILE_RESEARCH_V1,
        )
        .unwrap_or_else(|e| panic!("built-in profile `{PROFILE_RESEARCH_V1}` failed to register: {e}")),
        None => {
            registry.register_worker_session(builtin::sympy_eval_def(), sympy_epoch, sympy_handler);
            registry.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
            builtin::default_policy()
        }
    }
}

fn record_cli_title(ledger: &ledger::Ledger, session: &str, input: &Value) -> Result<Value, String> {
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let turn = events.iter().rev().find(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"));
    if !turn.is_some_and(|e| e.payload["allow_model_title"] == true)
        || events.iter().any(|e| e.kind == builtin::SESSION_TITLE_EVENT) {
        return Ok(serde_json::json!({"recorded":false,"reason":"The session title is already fixed."}));
    }
    let title = builtin::normalize_session_title(input)?;
    let head = &events.last().ok_or("Start the session first.")?.event_hash;
    let receipt = ledger.append_at_head(session, builtin::SESSION_TITLE_EVENT, tools::EVENT_SCHEMA_VERSION,
        &serde_json::json!({"title":title}), head).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"recorded":true,"seq":receipt.seq,"title":title}))
}

/// Tool definitions and policy version `core.exe` will serve.
#[derive(Debug, Clone)]
pub struct CoreToolSet {
    pub defs: Vec<ToolDef>,
    pub policy_version: String,
}

struct NoWorker;

impl WorkerEpoch for NoWorker {
    fn live_epoch(&self) -> Option<u64> {
        None
    }

    fn is_contained(&self) -> bool {
        false
    }
}

/// What `core.exe` serves for a named tool profile (definitions in id order).
pub fn core_tool_set_for_profile(
    env: Arc<ResearchEnv>,
    profile_name: &str,
) -> Result<CoreToolSet, String> {
    let mut registry = Registry::new();
    let stub: Arc<dyn ToolHandler> =
        Arc::new(|_: &CallCtx<'_>, _: &Value| Err("tool is served by core.exe, not here".to_string()));
    let policy = register_core_tools_for_profile(&mut registry, Arc::new(NoWorker), stub, env, profile_name)?;
    Ok(CoreToolSet { defs: registry.defs().into_iter().cloned().collect(), policy_version: policy.version })
}

/// What `core.exe` serves with or without research tools (definitions in id order).
pub fn core_tool_set(research: Option<Arc<ResearchEnv>>) -> CoreToolSet {
    let mut registry = Registry::new();
    let stub: Arc<dyn ToolHandler> =
        Arc::new(|_: &CallCtx<'_>, _: &Value| Err("tool is served by core.exe, not here".to_string()));
    let policy = register_core_tools(&mut registry, Arc::new(NoWorker), stub, research);
    CoreToolSet { defs: registry.defs().into_iter().cloned().collect(), policy_version: policy.version }
}

/// Ids of the tools `core.exe` serves for a launch environment:
/// - If `CORE_TOOL_PROFILE` is set and non-empty, resolves via [`core_tool_set_for_profile`]
///   (tolerating `NoWorker` cleanly during tool-ID enumeration); falls back fail-closed to the
///   minimal built-in pair (`ledger_note`, `sympy_eval`) without panicking on unknown/unbuilt
///   profiles so it never widens permissions.
/// - Otherwise honours legacy `CORE_ENABLE_RESEARCH_TOOLS=1`, else the built-in pair.
///
/// Used where a tool config must name the tools (Antigravity's `enabledTools`).
pub fn core_tool_ids_for(core_env: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    if let Some(profile_name) = core_env
        .get(CORE_TOOL_PROFILE_ENV)
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let env = Arc::new(ResearchEnv::new(std::env::temp_dir()));
        return match core_tool_set_for_profile(env, profile_name) {
            Ok(set) => set.defs.into_iter().map(|d| d.id).collect(),
            Err(_) => core_tool_set(None).defs.into_iter().map(|d| d.id).collect(),
        };
    }
    let research = core_env.get("CORE_ENABLE_RESEARCH_TOOLS").is_some_and(|v| v == "1");
    let env = research.then(|| Arc::new(ResearchEnv::new(std::env::temp_dir())));
    core_tool_set(env).defs.into_iter().map(|d| d.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn cli_can_title_an_eligible_session_once() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = ledger::Ledger::open(dir.path()).unwrap();
        let args = serde_json::json!({"title":"  Boundary   conditions "});
        let id = "s-title";
        ledger.append(id, "session.start", 1, &serde_json::json!({})).unwrap();
        assert_eq!(record_cli_title(&ledger, id, &args).unwrap()["recorded"], false);
        ledger.append(id, "session.resume", 1, &serde_json::json!({"allow_model_title":true})).unwrap();
        assert!(record_cli_title(&ledger, id, &serde_json::json!({"title":"  "})).is_err());
        assert_eq!(record_cli_title(&ledger, id, &args).unwrap()["title"], "Boundary conditions");
        assert_eq!(record_cli_title(&ledger, id, &args).unwrap()["recorded"], false);
        assert_eq!(ledger.read_verified_session(id).unwrap().1.iter().filter(|e| e.kind == builtin::SESSION_TITLE_EVENT).count(), 1);
    }

    #[test]
    fn builtin_set_matches_the_v0_definitions() {
        let set = core_tool_set(None);
        let ids: Vec<&str> = set.defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["ledger_note", "sympy_eval"]);
        assert_eq!(set.policy_version, builtin::POLICY_VERSION);
    }

    #[test]
    fn research_set_adds_the_research_tools_and_policy() {
        let set = core_tool_set(Some(Arc::new(ResearchEnv::new(std::env::temp_dir()))));
        let ids: Vec<&str> = set.defs.iter().map(|d| d.id.as_str()).collect();
        for id in ["sympy_eval", "ledger_note", "fs_read", "fs_write", "arxiv_search", "literature_search", "tex_audit", "record_paper_map", "record_paper_summary", "record_paper_branch", "record_paper_conventions"] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
        assert_eq!(set.policy_version, research_tools::RESEARCH_POLICY_VERSION);
    }

    #[test]
    fn profile_set_builds_known_profiles_and_rejects_unbuilt() {
        let env = Arc::new(ResearchEnv::new(std::env::temp_dir()));
        let r1 = core_tool_set_for_profile(Arc::clone(&env), PROFILE_RESEARCH_V1).unwrap();
        let i1 = core_tool_set_for_profile(
            Arc::clone(&env),
            research_tools::tool_profile::PROFILE_INVESTIGATION_V1,
        )
        .unwrap();
        assert_eq!(r1.defs, i1.defs);
        assert_eq!(r1.defs, core_tool_set(Some(Arc::clone(&env))).defs);
        let s1 = core_tool_set_for_profile(
            Arc::clone(&env),
            research_tools::tool_profile::PROFILE_SOLVE_V1,
        )
        .unwrap();
        assert_eq!(s1.defs.len(), 17);
        assert_eq!(s1.policy_version, research_tools::SOLVE_POLICY_VERSION);
        let err = core_tool_set_for_profile(env, research_tools::tool_profile::PROFILE_EXPLORE_V1)
            .unwrap_err();
        assert!(err.contains("profile not built yet"), "{err}");
    }

    #[test]
    fn core_tool_ids_for_honors_solve_v1_profile_and_excludes_non_solve_tools() {
        let env = BTreeMap::from([(
            CORE_TOOL_PROFILE_ENV.to_string(),
            research_tools::tool_profile::PROFILE_SOLVE_V1.to_string(),
        )]);
        let ids = core_tool_ids_for(&env);
        let expected: Vec<String> = research_tools::tool_profile::solve_v1()
            .mcp_defs()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids.len(), 17);
        assert_eq!(ids, expected);
        assert!(ids.iter().any(|id| id == builtin::SET_SESSION_TITLE));
        for forbidden in ["doc_ingest", "ledger_append", "lean_check", "fs_write", "search_formal_library"] {
            assert!(!ids.iter().any(|id| id == forbidden), "solve/1 must exclude {forbidden}");
        }
    }

    #[test]
    fn core_tool_ids_for_falls_back_cleanly_on_invalid_or_unbuilt_profile() {
        for bad in ["explore/1", "unknown/99"] {
            let env = BTreeMap::from([
                (CORE_TOOL_PROFILE_ENV.to_string(), bad.to_string()),
                ("CORE_ENABLE_RESEARCH_TOOLS".to_string(), "1".to_string()),
            ]);
            let ids = core_tool_ids_for(&env);
            assert_eq!(ids, vec!["ledger_note".to_string(), "sympy_eval".to_string()]);
        }
    }

    #[test]
    fn core_tool_ids_for_preserves_legacy_research_flag_when_profile_absent() {
        let legacy_on = BTreeMap::from([("CORE_ENABLE_RESEARCH_TOOLS".to_string(), "1".to_string())]);
        let ids_on = core_tool_ids_for(&legacy_on);
        assert!(ids_on.iter().any(|id| id == "fs_write"));
        assert!(ids_on.iter().any(|id| id == builtin::SET_SESSION_TITLE));

        let legacy_off = BTreeMap::new();
        assert_eq!(core_tool_ids_for(&legacy_off), vec!["ledger_note".to_string(), "sympy_eval".to_string()]);
    }
}
