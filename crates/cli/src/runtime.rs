//! Shared tool router and sandbox initialization for `--route api` (and
//! future in-process runtime helpers).

use crate::approval::ConsoleApprover;
use ledger::Ledger;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{builtin, CallCtx, Registry, Router, WorkerEpoch};
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

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct WorkerSetup {
    pub python: PathBuf,
    pub worker_script: PathBuf,
    pub scratch: PathBuf,
    pub pycache: PathBuf,
}

pub(crate) struct BuiltRouter {
    pub router: Router,
    pub containment: String,
    pub contained: bool,
}

#[cfg(windows)]
fn build_worker(data_dir: &Path, session: &str, setup: WorkerSetup) -> Result<Worker, String> {
    let sandbox: Box<dyn Sandbox> = Box::new(
        AppContainerSandbox::new(
            data_dir.to_path_buf(),
            session,
            setup.python.clone(),
            setup.worker_script.clone(),
            setup.scratch.clone(),
            setup.pycache.clone(),
        )
        .map_err(|e| format!("AppContainer sandbox setup failed: {e}"))?,
    );
    Ok(Worker::new(
        WorkerConfig {
            python: setup.python,
            script: setup.worker_script,
            scratch: setup.scratch,
            pycache: setup.pycache,
            startup_timeout: Duration::from_secs(90),
        },
        sandbox,
    ))
}

#[cfg(not(windows))]
fn build_worker(_data_dir: &Path, _session: &str, _setup: WorkerSetup) -> Result<Worker, String> {
    Err("AppContainer sandbox requires Windows".to_string())
}

pub(crate) fn build_in_process_router(
    data_dir: &Path,
    session: &str,
    worker_setup: Option<WorkerSetup>,
    eval_timeout_s: u64,
) -> Result<BuiltRouter, String> {
    let worker = match worker_setup {
        Some(setup) => Some(build_worker(data_dir, session, setup)?),
        None => None,
    };
    let epoch_handle = worker.as_ref().map(Worker::epoch_handle);
    let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let containment = match &worker {
        Some(w) => w.containment().summary(),
        None => "no compute worker configured".to_string(),
    };
    let worker = Arc::new(Mutex::new(worker));
    let timeout = Duration::from_secs(eval_timeout_s);

    let mut registry = Registry::new();
    registry.register_worker_session(
        builtin::sympy_eval_def(),
        Arc::new(WorkerEpochAdapter { epoch: epoch_handle, contained }),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let code = args["code"].as_str().ok_or("`code` must be a string")?;
            let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
            let w = guard.as_mut().ok_or("no compute worker is configured for this session")?;
            let (result, ran_epoch) =
                w.evaluate_in_epoch(code, timeout, ctx.required_epoch).map_err(|e| e.to_string())?;
            ctx.record_worker_epoch(ran_epoch);
            Ok(result)
        }),
    );
    registry.register(builtin::ledger_note_def(), builtin::ledger_note_handler());

    let router_ledger = Ledger::open(data_dir).map_err(|e| e.to_string())?;
    let approval_ledger = Ledger::open(data_dir).map_err(|e| e.to_string())?;
    let approver = Arc::new(ConsoleApprover::new(
        approval_ledger,
        session.to_string(),
        containment.clone(),
        contained,
    ));
    let router = Router::new(registry, builtin::default_policy(), router_ledger, session, approver);
    Ok(BuiltRouter { router, containment, contained })
}
