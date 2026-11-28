//! Tool routers for a desktop session: the main research router and the Lean router.

use crate::approver::UiApprover;
use crate::state::DesktopState;
use research_tools::tool_profile::{
    lookup as lookup_profile, register_profile, ProfileHostTools, PROFILE_SOLVE_V1,
};
use research_tools::{register_lean_tool, research_policy, ResearchEnv};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{CallCtx, Registry, Router, ToolHandler, WorkerEpoch};
#[cfg(windows)]
use worker::appcontainer::AppContainerSandbox;
#[cfg(windows)]
use worker::sandbox::Sandbox;
use worker::{EpochHandle, Worker};
#[cfg(windows)]
use worker::WorkerConfig;

struct WorkerEpochAdapter {
    epoch: Option<EpochHandle>,
    contained: bool,
}

impl WorkerEpoch for WorkerEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

#[cfg(windows)]
fn build_desktop_worker(state: &DesktopState, session: &str) -> Result<Option<Worker>, String> {
    let (Some(python), Some(script)) = (&state.cfg.python, &state.cfg.worker_script) else {
        return Ok(None);
    };
    let sdir = state.cfg.data_dir.join("sessions").join(session);
    let scratch = sdir.join("scratch");
    let pycache = sdir.join("pycache");
    let sandbox: Box<dyn Sandbox> = Box::new(
        AppContainerSandbox::new(
            state.cfg.data_dir.clone(),
            session,
            python.clone(),
            script.clone(),
            scratch.clone(),
            pycache.clone(),
        )
        .map_err(|e| format!("AppContainer sandbox setup failed: {e}"))?,
    );
    Ok(Some(Worker::new(
        WorkerConfig {
            python: python.clone(),
            script: script.clone(),
            scratch,
            pycache,
            startup_timeout: Duration::from_secs(90),
            job_memory_limit_bytes: None,
        },
        sandbox,
    )))
}

#[cfg(not(windows))]
fn build_desktop_worker(_state: &DesktopState, _session: &str) -> Result<Option<Worker>, String> {
    Ok(None)
}

pub(crate) fn build_desktop_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
) -> Result<(Router, String, bool), String> {
    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session).map_err(|e| e.to_string())?;
    let profile = crate::session_mode::profile_for_session(&existing)?;
    build_desktop_router_for_profile(state, env, session, profile.name)
}

pub(crate) fn build_desktop_router_for_profile(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
    profile_name: &str,
) -> Result<(Router, String, bool), String> {
    if profile_name == PROFILE_SOLVE_V1 {
        let bundle = crate::solve_runtime::build_solve_router(state, env, session)?;
        return Ok((bundle.router, bundle.containment, bundle.contained));
    }
    let profile = lookup_profile(profile_name).map_err(|e| e.to_string())?;
    let worker = build_desktop_worker(&state, session)?;
    let epoch_handle = worker.as_ref().map(Worker::epoch_handle);
    let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let containment = match &worker {
        Some(w) => w.containment().summary(),
        None => "desktop host (no sandboxed Python worker configured)".to_string(),
    };
    let worker = Arc::new(Mutex::new(worker));
    let timeout = Duration::from_secs(state.cfg.eval_timeout_s.max(30));

    let sympy_epoch: Arc<dyn WorkerEpoch> = Arc::new(WorkerEpochAdapter {
        epoch: epoch_handle,
        contained,
    });
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args["code"].as_str().ok_or("`code` must be a string")?;
        let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
        let w = guard
            .as_mut()
            .ok_or("no Python compute worker configured (`--python` required)")?;
        let (result, ran_epoch) = w
            .evaluate_in_epoch(code, timeout, ctx.required_epoch)
            .map_err(|e| e.to_string())?;
        ctx.record_worker_execution(ran_epoch, w.last_request_id());
        if let Some(hello) = w.hello() {
            ctx.record_worker_environment(hello);
        }
        Ok(result)
    });

    let mut registry = Registry::new();
    let policy = register_profile(
        &profile,
        &mut registry,
        env,
        ProfileHostTools {
            sympy_epoch,
            sympy_handler,
            session_title_handler: None,
            solve_check: None,
        },
    )?;

    let ledger = state.open_ledger()?;
    let approver = Arc::new(
        UiApprover::new(state, containment.clone(), contained).with_session_network_grant(true),
    );
    let router = Router::new(registry, policy, ledger, session, approver).with_tool_profile(profile.name);
    Ok((router, containment, contained))
}

pub(crate) fn build_lean_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
) -> Result<Router, String> {
    let mut registry = Registry::new();
    register_lean_tool(&mut registry, env);
    let ledger = state.open_ledger()?;
    let approver = Arc::new(UiApprover::new(state, "lean4".into(), false));
    Ok(Router::new(registry, research_policy(), ledger, session, approver))
}

/// Resolve the effective F11/X19 [`research_tools::lit_providers::ProviderConfig`] for Explore
/// literature requests: saved OpenAlex sidecar + `OPENALEX_API_KEY` env fallback via X19
/// [`crate::lit_settings::resolve_provider_config`], plus hermetic `SEMANTIC_SCHOLAR_API_KEY`.
pub(crate) fn resolve_explore_provider_config(
    state: &DesktopState,
) -> research_tools::lit_providers::ProviderConfig {
    let mut cfg = crate::lit_settings::resolve_provider_config(state);
    cfg.semantic_scholar_api_key = state
        .env_var("SEMANTIC_SCHOLAR_API_KEY")
        .and_then(|v| v.into_string().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    cfg
}

/// Build the Explore router strictly from frozen profile `explore/1` (`explore.md` 5.2,
/// Rule P5 & Rule P7). Verifies zero `ExecuteCode` or `WriteWorkspace` tools are registered
/// and binds `UiApprover::with_session_network_grant(true)` for batched network approval.
pub fn build_idea_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
) -> Result<Router, String> {
    let profile = lookup_profile(research_tools::tool_profile::PROFILE_EXPLORE_V1)
        .map_err(|e| e.to_string())?;
    for def in &profile.defs {
        if matches!(
            def.effect,
            policy::EffectClass::ExecuteCode | policy::EffectClass::WriteWorkspace
        ) {
            return Err(format!(
                "Rule P7 violation: explore/1 must not register `{}` ({:?})",
                def.id, def.effect
            ));
        }
    }
    let provider_cfg = resolve_explore_provider_config(&state);
    let mut registry = Registry::new();
    let noop_epoch: Arc<dyn WorkerEpoch> = Arc::new(WorkerEpochAdapter {
        epoch: None,
        contained: false,
    });
    let noop_sympy: Arc<dyn ToolHandler> =
        Arc::new(|_: &CallCtx<'_>, _: &Value| Err("sympy_eval is not registered on explore/1".into()));
    let policy = register_profile(
        &profile,
        &mut registry,
        Arc::clone(&env),
        ProfileHostTools::new(noop_epoch, noop_sympy),
    )?;
    // Bind hydrated sidecar + hermetic env ProviderConfig into `idea_search` with identical ToolDef.
    registry.register(
        research_tools::idea_tools::idea_search_def(),
        research_tools::idea_tools::idea_search_handler_with_gate_and_config(
            env,
            None,
            Some(provider_cfg),
        ),
    );
    let ledger = state.open_ledger()?;
    let approver = Arc::new(
        UiApprover::new(state, "explore host".into(), false).with_session_network_grant(true),
    );
    Ok(Router::new(registry, policy, ledger, session, approver).with_tool_profile(profile.name))
}

