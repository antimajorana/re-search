//! Solve S9 HTTP routes: `POST /api/sessions/{id}/solve/check` and
//! `POST /api/sessions/{id}/solve/recheck` (`solve.md` 5.1, rules R3, R18, R19).

use crate::approver::UiApprover;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::solve::session_solve_view;
use crate::state::DesktopState;
use research_tools::solve_checks::{
    check_run_handler, evaluate_recheck_diff, AutoCheckGate, CheckExecError, CheckExecResult,
    CheckProcessRunner,
};
use research_tools::solve_defs::CHECK_RUN;
use research_tools::solve_model::SolveCheck;
use research_tools::solve_state::SolveState;
use research_tools::solve_steps::step_record_handler;
use research_tools::tool_profile::{register_profile, solve_v1, ProfileHostTools, SolveCheckHost};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{CallOutcome, Registry, Router, ToolHandler, WorkerEpoch};
use worker::{EpochHandle, Worker};
#[cfg(windows)]
use worker::{appcontainer::AppContainerSandbox, sandbox::Sandbox, WorkerConfig, WorkerRole};

struct CheckEpochAdapter {
    epoch: Option<EpochHandle>,
    contained: bool,
}

impl WorkerEpoch for CheckEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

struct DesktopCheckRunner {
    worker: Mutex<Option<Worker>>,
}

impl CheckProcessRunner for DesktopCheckRunner {
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
        let (raw, ran_epoch) = match w.run_check(name, handles, params.clone(), timeout, required_epoch)
        {
            Ok(ok) => ok,
            Err(worker::WorkerError::Timeout(dur)) => return Err(CheckExecError::Timeout(dur)),
            Err(err) => return Err(CheckExecError::Worker(err.to_string())),
        };
        let request_id = w.last_request_id();
        let environment = w.hello().cloned();
        Ok(CheckExecResult {
            raw,
            epoch: ran_epoch,
            request_id,
            environment,
        })
    }
}

#[cfg(windows)]
fn build_desktop_check_worker(state: &DesktopState, session: &str) -> Result<Option<Worker>, String> {
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
fn build_desktop_check_worker(_state: &DesktopState, _session: &str) -> Result<Option<Worker>, String> {
    Ok(None)
}

fn build_check_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session_id: &str,
    runner: Arc<dyn CheckProcessRunner>,
    check_epoch: Arc<dyn WorkerEpoch>,
    contained: bool,
) -> Result<Router, String> {
    let sympy_epoch: Arc<dyn WorkerEpoch> = Arc::new(CheckEpochAdapter {
        epoch: None,
        contained: false,
    });
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(|_: &tools::CallCtx<'_>, _: &Value| {
        Err("sympy_eval is not used on user check/recheck endpoints".to_string())
    });
    let check_handler = check_run_handler(Arc::clone(&env), Arc::clone(&runner));
    let host = ProfileHostTools::new(sympy_epoch, sympy_handler).with_solve_check(
        SolveCheckHost::new(check_epoch)
            .with_check_handler(check_handler)
            .with_step_record_handler(step_record_handler(Arc::clone(&env))),
    );
    let mut registry = Registry::new();
    let policy = register_profile(&solve_v1(), &mut registry, Arc::clone(&env), host)?;
    let ledger = state.open_ledger()?;
    let containment = if contained {
        "check-role sandbox".to_string()
    } else {
        "desktop host (no sandboxed check worker configured)".to_string()
    };
    let approver = Arc::new(UiApprover::new(state, containment, contained));
    let gate = Arc::new(AutoCheckGate::new(env, runner));
    Ok(Router::new(registry, policy, ledger, session_id, approver)
        .with_tool_profile("solve/1")
        .with_gate(gate))
}

type CheckBindings = (Arc<dyn CheckProcessRunner>, Arc<dyn WorkerEpoch>, bool);

fn desktop_runner_and_epoch(
    state: &DesktopState,
    session_id: &str,
) -> Result<CheckBindings, String> {
    let worker = build_desktop_check_worker(state, session_id)?;
    let epoch = worker.as_ref().map(Worker::epoch_handle);
    let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let runner: Arc<dyn CheckProcessRunner> = Arc::new(DesktopCheckRunner {
        worker: Mutex::new(worker),
    });
    let check_epoch: Arc<dyn WorkerEpoch> = Arc::new(CheckEpochAdapter { epoch, contained });
    Ok((runner, check_epoch, contained))
}

/// S9: `POST /api/sessions/{id}/solve/check` — user runs `check_run` through Solve's Router.
pub fn handle_check(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    let (runner, check_epoch, contained) =
        desktop_runner_and_epoch(state, session_id).map_err(internal)?;
    handle_check_with_runner(state, session_id, body, runner, check_epoch, contained)
}

pub(crate) fn handle_check_with_runner(
    state: &Arc<DesktopState>,
    session_id: &str,
    body: &[u8],
    runner: Arc<dyn CheckProcessRunner>,
    check_epoch: Arc<dyn WorkerEpoch>,
    contained: bool,
) -> ApiResult {
    let val = parse_json_body(body)?;
    if !val.is_object() {
        return Err(bad("request body must be a JSON object"));
    }
    if !contained {
        return Err((
            403,
            "check_run requires an active contained check worker (rule R18)".to_string(),
        ));
    }
    let env = Arc::new(state.research_env_for_session(session_id).map_err(internal)?);
    let router = build_check_router(
        Arc::clone(state),
        env,
        session_id,
        runner,
        check_epoch,
        contained,
    )
    .map_err(internal)?;

    let check_val = match router.try_call("user", CHECK_RUN, &val).map_err(internal)? {
        CallOutcome::Ok(v) => v,
        CallOutcome::Denied(reason) => return Err((403, reason)),
        CallOutcome::Error(error) => return Err(bad(error)),
    };

    let ledger = state.open_ledger().map_err(internal)?;
    let updated_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "solve_updated",
        json!({ "session": session_id, "check_id": check_val.get("check_id") }),
    );
    let view = session_solve_view(
        state,
        session_id,
        &updated_events,
        is_turn_running(state, session_id),
    );
    Ok((
        200,
        json!({
            "ok": true,
            "check": check_val,
            "view": view,
        }),
    ))
}

/// S9: `POST /api/sessions/{id}/solve/recheck` — R19 reproducibility check.
pub fn handle_recheck(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    let (runner, check_epoch, contained) =
        desktop_runner_and_epoch(state, session_id).map_err(internal)?;
    handle_recheck_with_runner(state, session_id, body, runner, check_epoch, contained)
}

pub(crate) fn handle_recheck_with_runner(
    state: &Arc<DesktopState>,
    session_id: &str,
    body: &[u8],
    runner: Arc<dyn CheckProcessRunner>,
    check_epoch: Arc<dyn WorkerEpoch>,
    contained: bool,
) -> ApiResult {
    let val = parse_json_body(body)?;
    let requested_check_id = val
        .get("check_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let filter_entry_id = val
        .get("entry_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let folded = SolveState::from_events(&events);

    let targets: Vec<SolveCheck> = if let Some(ref cid) = requested_check_id {
        let orig = folded
            .checks
            .iter()
            .find(|c| c.check_id == *cid)
            .ok_or_else(|| not_found(format!("unknown check_id `{cid}`")))?
            .clone();
        vec![orig]
    } else {
        let filtered: Vec<SolveCheck> = folded
            .checks
            .iter()
            .filter(|c| match &filter_entry_id {
                Some(eid) => c.entry_id.as_deref() == Some(eid.as_str()),
                None => true,
            })
            .cloned()
            .collect();
        if filtered.is_empty() {
            return Err(not_found("no recorded checks found to re-run"));
        }
        filtered
    };

    if !contained {
        return Err((
            403,
            "check_run requires an active contained check worker (rule R18)".to_string(),
        ));
    }

    let env = Arc::new(state.research_env_for_session(session_id).map_err(internal)?);
    let router = build_check_router(
        Arc::clone(state),
        env,
        session_id,
        runner,
        check_epoch,
        contained,
    )
    .map_err(internal)?;

    let mut results = Vec::with_capacity(targets.len());
    for orig in &targets {
        let entry_id = orig
            .entry_id
            .clone()
            .or_else(|| {
                folded
                    .active_entries()
                    .into_iter()
                    .find(|e| e.claim_sha256.as_deref() == Some(orig.claim_sha256.as_str()))
                    .map(|e| e.entry_id.clone())
            })
            .ok_or_else(|| bad(format!("cannot locate step for check `{}`", orig.check_id)))?;
        let args = json!({
            "entry_id": entry_id,
            "claim_index": orig.claim_index,
            "check": orig.name,
            "params": orig.params,
        });
        let new_check = match router.try_call("user", CHECK_RUN, &args).map_err(internal)? {
            CallOutcome::Ok(v) => v,
            CallOutcome::Denied(reason) => return Err((403, reason)),
            CallOutcome::Error(error) => return Err(bad(error)),
        };
        results.push(evaluate_recheck_diff(orig, &new_check));
    }

    let updated_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "solve_updated",
        json!({ "session": session_id, "rechecked": results.len() }),
    );
    let view = session_solve_view(
        state,
        session_id,
        &updated_events,
        is_turn_running(state, session_id),
    );

    if requested_check_id.is_some() && results.len() == 1 {
        let single = results.into_iter().next().unwrap();
        let mut resp = single.as_object().cloned().unwrap_or_default();
        resp.insert("ok".into(), json!(true));
        resp.insert("view".into(), view);
        return Ok((200, Value::Object(resp)));
    }

    let any_changed = results
        .iter()
        .any(|r| r.get("changed").and_then(Value::as_bool) == Some(true));
    let warning = if any_changed {
        json!("result changed on re-run")
    } else {
        Value::Null
    };
    Ok((
        200,
        json!({
            "ok": true,
            "changed": any_changed,
            "warning": warning,
            "results": results,
            "view": view,
        }),
    ))
}

#[cfg(test)]
#[path = "solve_check_routes_tests.rs"]
mod tests;
