//! Isolated Python worker factory (`IsolatedWorkerFactory`) and `POST /api/sessions/{id}/solve/review`
//! route for Solve mode (`solve.md` 4.3, 5.3, rules R13, D2, D3; packages S14 & S20).

use crate::agent_models::resolve_solve_verifier_transport;
use crate::approver::UiApprover;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::session_lifecycle::{is_turn_running, reserve_turn};
use crate::session_mode::session_mode;
use crate::solve::session_solve_view;
use crate::state::DesktopState;
use crate::turn_control::ApiTurnGuard;
use agent::verifier_loop::{
    run_solve_verifier_loop, SolveReviewOutcome, SolveVerifierConfig, SolveVerifierRunCtx,
    SOLVE_VERIFIER_MAX_BATCH_SIZE,
};
use agent::verifier_tools::{IsolatedEvalOutput, IsolatedPythonWorker, IsolatedWorkerFactory};
use agent::{CancelToken, Transport};
use research_tools::paper_key::Mode;
use research_tools::solve_checks::{CheckExecError, CheckExecResult, CheckProcessRunner};
use research_tools::solve_exprs::{CanonicalizedExpr, CheckCanonicalizer};
use research_tools::solve_model::{RoleModel, SymbolSpec};
use research_tools::solve_state::SolveState;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
#[cfg(windows)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{Approver, WorkerEpoch};
use worker::{EpochHandle, Worker};
#[cfg(windows)]
use worker::{appcontainer::AppContainerSandbox, sandbox::Sandbox, WorkerConfig, WorkerRole};

/// Build an ephemeral, collision-free AppContainer profile key (`<= 43` chars, `[a-z0-9-]`)
/// for an isolated Solve worker so it never shares an AppContainer SID or deletes the live
/// solver session's profile on drop (`crates/worker/src/appcontainer.rs`).
pub fn isolated_appcontainer_key(session: &str, role_tag: &str, seq: u64) -> String {
    isolated_appcontainer_key_with_pid(session, role_tag, seq, std::process::id())
}

fn isolated_appcontainer_key_with_pid(
    session: &str,
    role_tag: &str,
    seq: u64,
    pid: u32,
) -> String {
    let tag: String = role_tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .take(12)
        .collect();
    let tag = if tag.is_empty() { "work" } else { tag.as_str() };
    let mut h = Sha256::new();
    h.update(session.as_bytes());
    h.update(b"\0");
    h.update(role_tag.as_bytes());
    h.update(b"\0");
    h.update(seq.to_le_bytes());
    h.update(pid.to_le_bytes());
    let digest = h.finalize();
    let mut hex16 = String::with_capacity(16);
    for b in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(&mut hex16, "{b:02x}");
    }
    format!("iso-{tag}-{hex16}")
}

struct SimpleEpochAdapter {
    epoch: Option<EpochHandle>,
    contained: bool,
}

impl WorkerEpoch for SimpleEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

/// Dedicated isolated Python worker backed by `crates/worker::Worker`.
/// Dropping kills the underlying worker process tree and cleans up its ephemeral dir (`solve.md` 4.3, R13).
pub struct DesktopIsolatedWorker {
    worker: Mutex<Option<Worker>>,
    ephemeral_dir: Option<PathBuf>,
    contained: bool,
    containment: String,
}

impl DesktopIsolatedWorker {
    pub fn new(worker: Worker, ephemeral_dir: Option<PathBuf>) -> Self {
        let contained = worker.containment().is_contained();
        let containment = worker.containment().summary();
        Self {
            worker: Mutex::new(Some(worker)),
            ephemeral_dir,
            contained,
            containment,
        }
    }
}

impl Drop for DesktopIsolatedWorker {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.worker.lock() {
            guard.take();
        }
        if let Some(dir) = self.ephemeral_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl IsolatedPythonWorker for DesktopIsolatedWorker {
    fn evaluate(
        &self,
        code: &str,
        exprs_in: Option<&BTreeMap<String, String>>,
        timeout: Duration,
        required_epoch: Option<u64>,
    ) -> Result<IsolatedEvalOutput, String> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "isolated worker lock poisoned".to_string())?;
        let Some(worker) = guard.as_mut() else {
            return Err("isolated worker already shut down".to_string());
        };
        let (result, epoch) = worker
            .evaluate_with_exprs(code, exprs_in, timeout, required_epoch)
            .map_err(|e| e.to_string())?;
        let request_id = worker.last_request_id();
        let environment = worker.hello().cloned();
        Ok(IsolatedEvalOutput {
            result,
            epoch,
            request_id,
            environment,
        })
    }

    fn live_epoch(&self) -> Option<u64> {
        self.worker
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(Worker::live_epoch))
    }

    fn is_contained(&self) -> bool {
        self.contained
    }

    fn containment_summary(&self) -> String {
        self.containment.clone()
    }
}

struct DesktopCheckBackend {
    worker: Mutex<Option<Worker>>,
    ephemeral_dir: Option<PathBuf>,
    timeout: Duration,
}

impl Drop for DesktopCheckBackend {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.worker.lock() {
            guard.take();
        }
        if let Some(dir) = self.ephemeral_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl CheckProcessRunner for DesktopCheckBackend {
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
        let (raw, epoch) = match w.run_check(name, handles, params.clone(), timeout, required_epoch)
        {
            Ok(ok) => ok,
            Err(worker::WorkerError::Timeout(dur)) => return Err(CheckExecError::Timeout(dur)),
            Err(err) => return Err(CheckExecError::Worker(err.to_string())),
        };
        Ok(CheckExecResult {
            raw,
            epoch,
            request_id: w.last_request_id(),
            environment: w.hello().cloned(),
        })
    }
}

impl CheckCanonicalizer for DesktopCheckBackend {
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
        let (canon, _epoch) = w
            .canonicalize_expr(raw_srepr, self.timeout, None)
            .map_err(|e| e.to_string())?;
        Ok(CanonicalizedExpr {
            portable: canon.portable,
            srepr: canon.srepr,
            srepr_sha256: canon.srepr_sha256,
            latex: canon.latex,
            str_preview: canon.str_preview,
            free_symbols: canon
                .free_symbols
                .into_iter()
                .map(|fs| SymbolSpec {
                    name: fs.name,
                    assumptions: fs.assumptions,
                })
                .collect(),
            error: canon.error,
            rejected_constructor: canon.rejected_constructor,
        })
    }
}

/// Host implementation of [`IsolatedWorkerFactory`] backed by `crates/worker`.
pub struct DesktopIsolatedWorkerFactory {
    state: Arc<DesktopState>,
    seq: AtomicU64,
}

impl DesktopIsolatedWorkerFactory {
    pub fn new(state: Arc<DesktopState>) -> Self {
        Self {
            state,
            seq: AtomicU64::new(1),
        }
    }

    #[cfg(windows)]
    fn build_sandboxed_worker(
        &self,
        session: &str,
        role_tag: &str,
        role: WorkerRole,
    ) -> Result<Option<(Worker, PathBuf)>, String> {
        let (Some(python), Some(script)) = (&self.state.cfg.python, &self.state.cfg.worker_script)
        else {
            return Ok(None);
        };
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let iso_key = isolated_appcontainer_key(session, role_tag, id);
        let base = self.state.cfg.data_dir.join("sessions").join(&iso_key);
        let scratch = base.join("scratch");
        let pycache = base.join("pycache");
        let sandbox: Box<dyn Sandbox> = Box::new(
            AppContainerSandbox::new(
                self.state.cfg.data_dir.clone(),
                &iso_key,
                python.clone(),
                script.clone(),
                scratch.clone(),
                pycache.clone(),
            )
            .map_err(|e| format!("AppContainer isolated worker sandbox setup failed: {e}"))?,
        );
        let worker = Worker::new_with_role(
            WorkerConfig {
                python: python.clone(),
                script: script.clone(),
                scratch,
                pycache,
                startup_timeout: Duration::from_secs(90),
            },
            sandbox,
            role,
        );
        Ok(Some((worker, base)))
    }

    #[cfg(not(windows))]
    fn build_sandboxed_worker_unavailable(&self, purpose: &str) -> String {
        format!(
            "isolated `{purpose}` Python worker requires Windows AppContainer containment (`--python` + Windows host)"
        )
    }
}

impl IsolatedWorkerFactory for DesktopIsolatedWorkerFactory {
    fn spawn_isolated_python_worker(
        &self,
        session: &str,
        purpose: &str,
    ) -> Result<Box<dyn IsolatedPythonWorker>, String> {
        #[cfg(windows)]
        {
            let (worker, dir) = self
                .build_sandboxed_worker(session, purpose, WorkerRole::Solver)?
                .ok_or_else(|| {
                    "no Python compute worker configured (`--python` required)".to_string()
                })?;
            Ok(Box::new(DesktopIsolatedWorker::new(worker, Some(dir))))
        }
        #[cfg(not(windows))]
        {
            let _ = (session, &self.seq);
            Err(self.build_sandboxed_worker_unavailable(purpose))
        }
    }

    fn check_runner(
        &self,
        session: &str,
    ) -> Result<agent::verifier_tools::CheckRunnerBinding, String> {
        #[cfg(windows)]
        let built = self.build_sandboxed_worker(session, "check", WorkerRole::Check)?;
        #[cfg(not(windows))]
        let built: Option<(Worker, PathBuf)> = {
            let _ = session;
            None
        };
        let (worker, ephemeral_dir) = match built {
            Some((w, d)) => (Some(w), Some(d)),
            None => (None, None),
        };
        let epoch = worker.as_ref().map(Worker::epoch_handle);
        let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
        let backend = Arc::new(DesktopCheckBackend {
            worker: Mutex::new(worker),
            ephemeral_dir,
            timeout: Duration::from_secs(self.state.cfg.eval_timeout_s.max(30)),
        });
        let check_epoch: Arc<dyn WorkerEpoch> =
            Arc::new(SimpleEpochAdapter { epoch, contained });
        Ok((backend, check_epoch, contained))
    }

    fn check_canonicalizer(&self, session: &str) -> Result<Arc<dyn CheckCanonicalizer>, String> {
        #[cfg(windows)]
        let built = self.build_sandboxed_worker(session, "canon", WorkerRole::Check)?;
        #[cfg(not(windows))]
        let built: Option<(Worker, PathBuf)> = {
            let _ = session;
            None
        };
        let (worker, ephemeral_dir) = match built {
            Some((w, d)) => (Some(w), Some(d)),
            None => (None, None),
        };
        Ok(Arc::new(DesktopCheckBackend {
            worker: Mutex::new(worker),
            ephemeral_dir,
            timeout: Duration::from_secs(self.state.cfg.eval_timeout_s.max(30)),
        }))
    }

    fn isolated_workers_contained(&self) -> bool {
        #[cfg(windows)]
        {
            self.state.cfg.python.is_some() && self.state.cfg.worker_script.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    fn eval_timeout(&self) -> Duration {
        Duration::from_secs(self.state.cfg.eval_timeout_s.max(30))
    }
}

struct SolveReviewTurnGuard<'a> {
    state: &'a DesktopState,
    session: &'a str,
}

impl Drop for SolveReviewTurnGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.active_turns.lock() {
            active.remove(self.session);
        }
    }
}

/// Auth-guarded `POST /api/sessions/{id}/solve/review` endpoint (`solve.md` 5.3, S14).
/// Accepts `{ entry_id?: "e-4", entry_ids?: ["e-4", ...], verifier?: { provider, model } }`.
pub fn handle_solve_review(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let entry_ids = parse_review_entry_ids(&val)?;

    let _activity = reserve_turn(state, session_id)?;
    {
        let mut active = state
            .active_turns
            .lock()
            .map_err(|_| internal("Session lock unavailable."))?;
        if !active.insert(session_id.to_string()) {
            return Err((409, "Wait for the current turn or review to finish.".into()));
        }
    }
    let reservation = SolveReviewTurnGuard {
        state,
        session: session_id,
    };
    let turn_guard = ApiTurnGuard::new(state, session_id).map_err(internal)?;

    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    if session_mode(&events) != Mode::Solve {
        return Err(bad("This endpoint is only available for Solve sessions."));
    }
    let folded = SolveState::from_events(&events);
    let setup = folded
        .setup
        .as_ref()
        .ok_or_else(|| bad("Solve setup must be recorded before running a review."))?;

    let ui = state.load_ui_settings();
    let target = resolve_solve_verifier_transport(
        state,
        &ui,
        Some(&setup.roles),
        Some(&val),
        Some(&setup.roles.solver),
    )?;
    let factory: Arc<dyn IsolatedWorkerFactory> =
        Arc::new(DesktopIsolatedWorkerFactory::new(Arc::clone(state)));
    let contained = factory.isolated_workers_contained();
    let containment = if contained {
        "AppContainer isolated verifier worker".to_string()
    } else {
        "desktop host (no sandboxed Python verifier worker configured)".to_string()
    };
    let approver: Arc<dyn Approver> =
        Arc::new(UiApprover::new(Arc::clone(state), containment, contained));

    let outcome = run_solve_review_with_deps(
        state,
        session_id,
        &entry_ids,
        target.transport.as_ref(),
        target.role,
        approver,
        &factory,
        Some(&turn_guard.cancel),
    )?;
    drop(turn_guard);
    drop(reservation);

    let updated_events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "solve_updated",
        json!({
            "session": session_id,
            "review_completed": matches!(outcome, SolveReviewOutcome::Completed { .. }),
            "independence": outcome.independence(),
        }),
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
            "outcome": outcome,
            "view": view,
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn run_solve_review_with_deps(
    state: &Arc<DesktopState>,
    session_id: &str,
    entry_ids: &[String],
    transport: &dyn Transport,
    verifier_role: RoleModel,
    approver: Arc<dyn Approver>,
    factory: &Arc<dyn IsolatedWorkerFactory>,
    cancel: Option<&CancelToken>,
) -> Result<SolveReviewOutcome, (u16, String)> {
    let ledger = state.open_ledger().map_err(internal)?;
    let env = Arc::new(state.research_env_for_session(session_id).map_err(internal)?);
    let config = SolveVerifierConfig::new(verifier_role);
    let ctx = SolveVerifierRunCtx {
        transport,
        ledger: &ledger,
        env,
        session: session_id,
        config: &config,
        approver,
        factory,
        cancel,
    };
    run_solve_verifier_loop(&ctx, entry_ids).map_err(bad)
}

fn parse_review_entry_ids(val: &Value) -> Result<Vec<String>, (u16, String)> {
    let obj = val
        .as_object()
        .ok_or_else(|| bad("request body must be a JSON object"))?;
    let mut ids = Vec::new();
    if let Some(arr) = obj.get("entry_ids").and_then(Value::as_array) {
        for item in arr {
            let s = item
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| bad("`entry_ids` must contain non-empty step ID strings"))?;
            ids.push(s.to_string());
        }
    } else if let Some(single) = obj
        .get("entry_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        ids.push(single.to_string());
    }
    if ids.is_empty() || ids.len() > SOLVE_VERIFIER_MAX_BATCH_SIZE {
        return Err(bad(format!(
            "provide 1..={SOLVE_VERIFIER_MAX_BATCH_SIZE} step IDs via `entry_ids` or `entry_id`"
        )));
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_appcontainer_key_is_valid_bounded_and_distinct() {
        let long_session = "s-very-long-session-name-exceeding-43-characters-0123456789";
        let k1 = isolated_appcontainer_key_with_pid(long_session, "verifier_review_extra_long", 1, 42);
        let k2 = isolated_appcontainer_key_with_pid(long_session, "verifier_review_extra_long", 2, 42);
        let k3 = isolated_appcontainer_key_with_pid(long_session, "check", 1, 42);
        let k_empty = isolated_appcontainer_key_with_pid("s-1", "!!!", 1, 42);

        for k in [&k1, &k2, &k3, &k_empty] {
            assert!(!k.is_empty() && k.len() <= 43, "key len invalid: {k}");
            assert!(!k.ends_with('.'), "key ends with dot: {k}");
            assert!(
                k.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "key chars invalid: {k}"
            );
            assert_ne!(k, long_session);
        }
        assert_ne!(k1, k2);
        assert_ne!(k1, k3);
        assert!(k_empty.starts_with("iso-work-"));
    }
}
