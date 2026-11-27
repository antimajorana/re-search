//! Desktop Solve runtime worker assembly (`solve/1`, `WorkerRole::Check` + solver `exprs_in`).

use crate::approver::UiApprover;
use crate::state::DesktopState;
use agent::SolveWorkerRuntime;
use ledger::Ledger;
use research_tools::solve_checks::{
    CheckExecError, CheckExecResult, CheckProcessRunner, DEFAULT_CHECK_TIMEOUT,
};
use research_tools::solve_exprs::{stage_exprs_in, CanonicalizedExpr, CheckCanonicalizer};
use research_tools::solve_gates::{solve_check_host_v1, solve_gate_chain, SolveGate};
use research_tools::solve_model::{HandleRef, SolveExpr, SymbolSpec, SOLVE_TOOL_PROFILE_V1};
use research_tools::solve_state::SolveState;
use research_tools::tool_profile::{lookup as lookup_profile, register_profile, ProfileHostTools};
use research_tools::ResearchEnv;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{CallCtx, Registry, Router, ToolHandler, WorkerEpoch};
#[cfg(windows)]
use worker::{appcontainer::AppContainerSandbox, sandbox::Sandbox, WorkerConfig, WorkerRole};
use worker::{EpochHandle, Worker};

struct EpochAdapter {
    epoch: Option<EpochHandle>,
    contained: bool,
}

impl WorkerEpoch for EpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

pub(crate) struct DesktopCheckBridge {
    worker: Mutex<Option<Worker>>,
}

impl CheckProcessRunner for DesktopCheckBridge {
    fn run_check(
        &self,
        name: &str,
        handles: &BTreeMap<String, String>,
        params: &Value,
        timeout: Duration,
        required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| CheckExecError::Worker("check worker lock poisoned".into()))?;
        let Some(w) = guard.as_mut() else {
            return Err(CheckExecError::Worker(
                "no contained Python check worker configured (`--python` + Windows AppContainer required)"
                    .into(),
            ));
        };
        let (raw, epoch) = w
            .run_check(name, handles, params.clone(), timeout, required_epoch)
            .map_err(|e| match e {
                worker::WorkerError::Timeout(d) => CheckExecError::Timeout(d),
                other => CheckExecError::Worker(other.to_string()),
            })?;
        Ok(CheckExecResult {
            raw,
            epoch,
            request_id: w.last_request_id(),
            environment: w.hello().cloned(),
        })
    }
}

impl CheckCanonicalizer for DesktopCheckBridge {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "check worker lock poisoned".to_string())?;
        let Some(w) = guard.as_mut() else {
            return Err(
                "no contained Python check worker configured (`--python` + Windows AppContainer required)"
                    .into(),
            );
        };
        let (ce, _) = w
            .canonicalize_expr(raw_srepr, DEFAULT_CHECK_TIMEOUT, None)
            .map_err(|e| e.to_string())?;
        Ok(CanonicalizedExpr {
            portable: ce.portable,
            srepr: ce.srepr,
            srepr_sha256: ce.srepr_sha256,
            latex: ce.latex,
            str_preview: ce.str_preview,
            free_symbols: ce
                .free_symbols
                .into_iter()
                .map(|s| SymbolSpec {
                    name: s.name,
                    assumptions: s.assumptions,
                })
                .collect(),
            error: ce.error,
            rejected_constructor: ce.rejected_constructor,
        })
    }
}

pub(crate) struct DesktopSolverRuntime {
    worker: Arc<Mutex<Option<Worker>>>,
    epoch: Option<EpochHandle>,
    timeout: Duration,
}

impl SolveWorkerRuntime for DesktopSolverRuntime {
    fn worker_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn restage_handles(
        &self,
        ledger: &Ledger,
        session: &str,
        handles: &BTreeMap<HandleRef, SolveExpr>,
    ) -> Result<(), String> {
        let staged = stage_exprs_in(ledger, session, handles)?;
        if staged.is_empty() {
            return Ok(());
        }
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "worker lock poisoned".to_string())?;
        let Some(w) = guard.as_mut() else {
            return Ok(());
        };
        w.evaluate_with_exprs("pass", Some(&staged), self.timeout, None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
fn build_solver_worker(state: &DesktopState, session: &str) -> Result<Option<Worker>, String> {
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
fn build_solver_worker(_state: &DesktopState, _session: &str) -> Result<Option<Worker>, String> {
    Ok(None)
}

#[cfg(windows)]
fn build_check_worker(state: &DesktopState, session: &str) -> Result<Option<Worker>, String> {
    let (Some(python), Some(script)) = (&state.cfg.python, &state.cfg.worker_script) else {
        return Ok(None);
    };
    let sdir = state.cfg.data_dir.join("sessions").join(session);
    let scratch = sdir.join("check_scratch");
    let pycache = sdir.join("check_pycache");
    let sandbox: Box<dyn Sandbox> = Box::new(
        AppContainerSandbox::new(
            state.cfg.data_dir.clone(),
            session,
            python.clone(),
            script.clone(),
            scratch.clone(),
            pycache.clone(),
        )
        .map_err(|e| format!("AppContainer check sandbox setup failed: {e}"))?,
    );
    Ok(Some(Worker::new_with_role(
        WorkerConfig {
            python: python.clone(),
            script: script.clone(),
            scratch,
            pycache,
            startup_timeout: Duration::from_secs(90),
        },
        sandbox,
        WorkerRole::Check,
    )))
}

#[cfg(not(windows))]
fn build_check_worker(_state: &DesktopState, _session: &str) -> Result<Option<Worker>, String> {
    Ok(None)
}

pub(crate) struct SolveRouterBundle {
    pub router: Router,
    pub containment: String,
    pub contained: bool,
    pub gate: Arc<SolveGate>,
    pub check_bridge: Option<Arc<DesktopCheckBridge>>,
    pub worker_runtime: Option<Arc<DesktopSolverRuntime>>,
}

pub(crate) fn build_solve_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
) -> Result<SolveRouterBundle, String> {
    let profile = lookup_profile(SOLVE_TOOL_PROFILE_V1).map_err(|e| e.to_string())?;
    let worker = build_solver_worker(&state, session)?;
    let solver_epoch_handle = worker.as_ref().map(Worker::epoch_handle);
    let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let containment = match &worker {
        Some(w) => w.containment().summary(),
        None => "desktop host (no sandboxed Python worker configured)".to_string(),
    };
    let worker_arc = Arc::new(Mutex::new(worker));
    let timeout = Duration::from_secs(state.cfg.eval_timeout_s.max(30));

    let sympy_epoch: Arc<dyn WorkerEpoch> = Arc::new(EpochAdapter {
        epoch: solver_epoch_handle.clone(),
        contained,
    });
    let w_for_handler = Arc::clone(&worker_arc);
    let session_for_handler = session.to_string();
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args["code"].as_str().ok_or("`code` must be a string")?;
        let staged = {
            let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
            let evs = guard.read_session(&session_for_handler).map_err(|e| e.to_string())?;
            let st = SolveState::from_events(&evs);
            stage_exprs_in(&guard, &session_for_handler, &st.handles)?
        };
        let mut guard = w_for_handler.lock().map_err(|_| "worker lock poisoned".to_string())?;
        let w = guard
            .as_mut()
            .ok_or("no Python compute worker configured (`--python` required)")?;
        let (result, ran_epoch) = w
            .evaluate_with_exprs(code, (!staged.is_empty()).then_some(&staged), timeout, ctx.required_epoch)
            .map_err(|e| e.to_string())?;
        ctx.record_worker_execution(ran_epoch, w.last_request_id());
        if let Some(hello) = w.hello() {
            ctx.record_worker_environment(hello);
        }
        Ok(result)
    });

    let check_worker = build_check_worker(&state, session)?;
    let check_epoch: Arc<dyn WorkerEpoch> = Arc::new(EpochAdapter {
        epoch: check_worker.as_ref().map(Worker::epoch_handle),
        contained: check_worker.as_ref().is_some_and(|w| w.containment().is_contained()),
    });
    let has_check_worker = check_worker.is_some();
    let check_bridge = Arc::new(DesktopCheckBridge {
        worker: Mutex::new(check_worker),
    });
    let runner_dyn: Arc<dyn CheckProcessRunner> = check_bridge.clone();

    let host = ProfileHostTools::new(sympy_epoch, sympy_handler)
        .with_solve_check(solve_check_host_v1(check_epoch, Arc::clone(&env), Arc::clone(&runner_dyn)));
    let mut registry = Registry::new();
    let policy = register_profile(&profile, &mut registry, Arc::clone(&env), host)?;

    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session).map_err(|e| e.to_string())?;
    let run_start_seq = existing
        .iter()
        .find(|e| e.kind == "session.start")
        .map_or(existing.last().map_or(0, |e| e.seq), |e| e.seq.saturating_sub(1));
    let leg_start_seq = existing
        .iter()
        .rfind(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"))
        .map_or(run_start_seq, |e| e.seq);
    let (gate, chain) = solve_gate_chain(env, runner_dyn, run_start_seq, leg_start_seq);
    let approver = Arc::new(
        UiApprover::new(state, containment.clone(), contained).with_session_network_grant(true),
    );
    let router = Router::new(registry, policy, ledger, session, approver)
        .with_tool_profile(profile.name)
        .with_gate(chain);

    Ok(SolveRouterBundle {
        router,
        containment,
        contained,
        gate,
        check_bridge: has_check_worker.then_some(check_bridge),
        worker_runtime: solver_epoch_handle.map(|ep| {
            Arc::new(DesktopSolverRuntime {
                worker: worker_arc,
                epoch: Some(ep),
                timeout,
            })
        }),
    })
}
