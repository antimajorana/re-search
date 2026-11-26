//! Tool routers for a desktop session: the main research router and the Lean router.

use crate::approver::UiApprover;
use crate::state::DesktopState;
use research_tools::tool_profile::{
    lookup as lookup_profile, profile_for_session_events, register_profile, ProfileHostTools,
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
    let profile = profile_for_session_events(&existing)?;
    build_desktop_router_for_profile(state, env, session, profile.name)
}

pub(crate) fn build_desktop_router_for_profile(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
    profile_name: &str,
) -> Result<(Router, String, bool), String> {
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
