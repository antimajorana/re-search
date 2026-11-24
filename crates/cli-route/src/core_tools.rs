//! The tool set `core.exe` serves, in one place.
//!
//! `core.exe` registers its tools with [`register_core_tools`]; the supervisor computes
//! the list it expects the vendor CLI to offer with [`core_tool_set`], which runs the same
//! registration with stub handlers. So the `system/init` check compares against exactly
//! the registry `core.exe` will serve for the same research setting.

use policy::Policy;
use research_tools::ResearchEnv;
use serde_json::Value;
use std::sync::Arc;
use tools::{builtin, CallCtx, Registry, ToolDef, ToolHandler, WorkerEpoch};

/// Register `sympy_eval` (with the caller's worker), `ledger_note`, and, when `research`
/// is given, the research tools; return the matching policy (`v0.4-research` with
/// research tools, the built-in `v0.2` otherwise).
pub fn register_core_tools(
    registry: &mut Registry,
    sympy_epoch: Arc<dyn WorkerEpoch>,
    sympy_handler: Arc<dyn ToolHandler>,
    research: Option<Arc<ResearchEnv>>,
) -> Policy {
    registry.register_worker_session(builtin::sympy_eval_def(), sympy_epoch, sympy_handler);
    registry.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
    match research {
        Some(env) => {
            research_tools::register_research_tools(registry, env);
            research_tools::research_policy()
        }
        None => builtin::default_policy(),
    }
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

/// What `core.exe` serves with or without research tools (definitions in id order).
pub fn core_tool_set(research: Option<Arc<ResearchEnv>>) -> CoreToolSet {
    let mut registry = Registry::new();
    let stub: Arc<dyn ToolHandler> =
        Arc::new(|_: &CallCtx<'_>, _: &Value| Err("tool is served by core.exe, not here".to_string()));
    let policy = register_core_tools(&mut registry, Arc::new(NoWorker), stub, research);
    CoreToolSet { defs: registry.defs().into_iter().cloned().collect(), policy_version: policy.version }
}

/// Ids of the tools `core.exe` serves for a launch environment: with research tools when
/// `CORE_ENABLE_RESEARCH_TOOLS=1` is in `core_env` (as on every desktop session), else the
/// built-in pair. Used where a tool config must name the tools (Antigravity's
/// `enabledTools`).
pub fn core_tool_ids_for(core_env: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    let research = core_env.get("CORE_ENABLE_RESEARCH_TOOLS").is_some_and(|v| v == "1");
    let env = research.then(|| Arc::new(ResearchEnv::new(std::env::temp_dir())));
    core_tool_set(env).defs.into_iter().map(|d| d.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for id in ["sympy_eval", "ledger_note", "fs_read", "fs_write", "arxiv_search", "literature_search", "tex_audit", "record_paper_map", "record_paper_summary"] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
        assert_eq!(set.policy_version, research_tools::RESEARCH_POLICY_VERSION);
    }
}
