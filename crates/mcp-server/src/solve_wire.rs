//! Solve (`solve/1`) wiring for the `core` MCP server (`solve.md` 5.1, 5.4, 9 S15; rules R11/R18).
//!
//! Binds the separate check-role Python worker, protocol-2 handle staging and harvesting,
//! `research_tools::solve_gates::SolveGate` (G1/G4/G5/G6/G7), and S9's `AutoCheckGate`
//! (`host:check` through `TcpApprover`, zero auto-approve) without coupling `mcp-server` to `agent`.

use ledger::{Event, Ledger};
use research_tools::solve_checks::{
    AutoCheckGate, CheckExecError, CheckExecResult, CheckProcessRunner, DEFAULT_CHECK_TIMEOUT,
};
use research_tools::solve_exprs::{
    attach_harvest_previews, harvest_from_sympy_result, stage_exprs_in, CanonicalizedExpr,
    CheckCanonicalizer, HarvestReport,
};
use research_tools::solve_gates::{solve_check_host_v1, SolveGate};
use research_tools::solve_model::{event_kinds, ReceiptRef, SymbolSpec};
use research_tools::tool_profile::PROFILE_SOLVE_V1;
use research_tools::ResearchEnv;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{
    Approver, CallCtx, CallOutcome, Gate, GateChain, GateDecision, GateRequest, Registry, Router,
    ToolHandler, WorkerEpoch,
};
use worker::{EpochHandle, Worker};
#[cfg(windows)]
use worker::{appcontainer::AppContainerSandbox, sandbox::Sandbox, WorkerConfig, WorkerRole};

/// Epoch adapter matching `mcp-server/src/main.rs`'s solver epoch wrapper.
pub struct WorkerEpochAdapter {
    pub epoch: Option<EpochHandle>,
    pub contained: bool,
}

impl WorkerEpoch for WorkerEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

/// Bridges `worker::Worker` (`WorkerRole::Check`) into both S9 `CheckProcessRunner`
/// and S7 `CheckCanonicalizer` so model code never alters checks or rendered LaTeX (R5/R6).
pub struct McpCheckRunner {
    worker: Mutex<Option<Worker>>,
    canonicalize_timeout: Duration,
}

impl McpCheckRunner {
    pub fn new(worker: Option<Worker>, canonicalize_timeout: Duration) -> Self {
        Self { worker: Mutex::new(worker), canonicalize_timeout }
    }
}

impl CheckProcessRunner for McpCheckRunner {
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
                "no contained Python check worker configured (`CORE_PYTHON` + Windows AppContainer required)"
                    .into(),
            ));
        };
        let (raw, ran_epoch) = match w.run_check(name, handles, params.clone(), timeout, required_epoch) {
            Ok(ok) => ok,
            Err(worker::WorkerError::Timeout(dur)) => return Err(CheckExecError::Timeout(dur)),
            Err(err) => return Err(CheckExecError::Worker(err.to_string())),
        };
        Ok(CheckExecResult {
            raw,
            epoch: ran_epoch,
            request_id: w.last_request_id(),
            environment: w.hello().cloned(),
        })
    }
}

impl CheckCanonicalizer for McpCheckRunner {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "check worker lock poisoned".to_string())?;
        let Some(w) = guard.as_mut() else {
            return Err("no contained Python check worker configured".to_string());
        };
        let (canon, _epoch) = w
            .canonicalize_expr(raw_srepr, self.canonicalize_timeout, None)
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
                .map(|s| SymbolSpec { name: s.name, assumptions: s.assumptions })
                .collect(),
            error: canon.error,
            rejected_constructor: canon.rejected_constructor,
        })
    }
}

/// Shared side-table of harvested handle previews and errors keyed by `call_id`, populated by
/// [`SympyHarvestGate::on_outcome`] after `tool.result` is recorded and consumed by
/// `CoreServer::call_tool` to enrich the MCP tool response.
#[derive(Clone, Default)]
pub struct HarvestPreviews {
    reports: Arc<Mutex<HashMap<String, Result<HarvestReport, String>>>>,
}

impl HarvestPreviews {
    pub fn new() -> Self {
        Self::default()
    }

    fn store(&self, call_id: String, outcome: Result<HarvestReport, String>) {
        if let Ok(mut map) = self.reports.lock() {
            map.insert(call_id, outcome);
        }
    }

    /// Attach any harvested handle previews (or harvest failure warning) for `value["tool_call_ref"]`.
    pub fn attach_to_result(&self, value: &mut Value) {
        let Some(call_id) = value.get("tool_call_ref").and_then(Value::as_str).map(str::to_string) else {
            return;
        };
        let outcome = self
            .reports
            .lock()
            .ok()
            .and_then(|mut map| map.remove(&call_id));
        match outcome {
            Some(Ok(report)) => attach_harvest_previews(value, &report),
            Some(Err(err)) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("expr_harvest_error".into(), Value::String(err));
                }
            }
            None => {}
        }
    }
}

/// Post-outcome `Gate` that harvests protocol-2 `exprs_out` from successful `sympy_eval` calls
/// through `CheckCanonicalizer` once `tool.result` has been recorded in the ledger.
pub struct SympyHarvestGate {
    data_dir: PathBuf,
    canonicalizer: Arc<dyn CheckCanonicalizer>,
    previews: HarvestPreviews,
}

impl SympyHarvestGate {
    pub fn new(
        data_dir: PathBuf,
        canonicalizer: Arc<dyn CheckCanonicalizer>,
        previews: HarvestPreviews,
    ) -> Self {
        Self { data_dir, canonicalizer, previews }
    }
}

impl Gate for SympyHarvestGate {
    fn before(&self, _req: &GateRequest<'_>) -> GateDecision {
        GateDecision::Allow
    }

    fn on_outcome(&self, req: &GateRequest<'_>, outcome: &CallOutcome) {
        if req.tool_id != "sympy_eval" {
            return;
        }
        let CallOutcome::Ok(val) = outcome else {
            return;
        };
        if !val.get("error").is_none_or(Value::is_null) {
            return;
        }
        // Trigger whenever `exprs_out` is present and non-null (including malformed entries so
        // `SkippedExpr` warnings are preserved rather than silently dropped).
        let has_exprs_out = match val.get("exprs_out") {
            None | Some(Value::Null) => false,
            Some(Value::Object(map)) => !map.is_empty(),
            Some(_) => true,
        };
        if !has_exprs_out {
            return;
        }
        let harvest_res = (|| -> Result<HarvestReport, String> {
            let ledger = Ledger::open(&self.data_dir).map_err(|e| e.to_string())?;
            let events = ledger.read_session(req.session).map_err(|e| e.to_string())?;
            let result_seq = events
                .iter()
                .rev()
                .find(|e| e.kind == event_kinds::TOOL_RESULT && e.payload["call_id"] == req.call_id)
                .map(|e| e.seq)
                .ok_or_else(|| format!("missing tool.result seq for sympy_eval call `{}`", req.call_id))?;
            harvest_from_sympy_result(
                &ledger,
                req.session,
                ReceiptRef { call_id: req.call_id.to_string(), result_seq },
                val,
                self.canonicalizer.as_ref(),
            )
        })();
        if let Err(ref err) = harvest_res {
            eprintln!("core: sympy expression harvest failed for call {}: {err}", req.call_id);
        }
        self.previews.store(req.call_id.to_string(), harvest_res);
    }
}

/// Derive a distinct AppContainer sandbox session id (`1..=43` lowercase ASCII `[a-z0-9._-]`)
/// for the isolated `WorkerRole::Check` worker so it never shares an AppContainer SID, ACL grant,
/// profile lifecycle, or `.worker.lock` with the untrusted solver worker (rule R6).
pub fn check_sandbox_id(session: &str) -> String {
    let lower = session.to_ascii_lowercase();
    let valid_chars = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
    };
    let candidate = format!("{lower}-chk");
    if valid_chars(&candidate) && candidate.len() <= 43 && candidate != lower {
        candidate
    } else {
        let digest = ledger::sha256_hex(session.as_bytes());
        format!("chk-{}", &digest[..16])
    }
}

/// Build the isolated Windows AppContainer check-role Python worker for `core.exe`.
/// Uses [`check_sandbox_id`] so solver and check AppContainers have separate SIDs and locks.
#[cfg(windows)]
pub fn build_check_worker(
    data_dir: &Path,
    session: &str,
    python: &str,
    script: &str,
    check_scratch: Option<String>,
    check_pycache: Option<String>,
) -> Result<Option<Worker>, String> {
    let check_sid = check_sandbox_id(session);
    let sdir = data_dir.join("sessions").join(&check_sid);
    let scratch = check_scratch
        .map(PathBuf::from)
        .filter(|p| p.starts_with(&sdir))
        .unwrap_or_else(|| sdir.join("scratch"));
    let pycache = check_pycache
        .map(PathBuf::from)
        .filter(|p| p.starts_with(&sdir))
        .unwrap_or_else(|| sdir.join("pycache"));
    let sandbox: Box<dyn Sandbox> = Box::new(
        AppContainerSandbox::new(
            data_dir.to_path_buf(),
            &check_sid,
            PathBuf::from(python),
            PathBuf::from(script),
            scratch.clone(),
            pycache.clone(),
        )
        .map_err(|e| format!("AppContainer check sandbox setup failed: {e}"))?,
    );
    Ok(Some(Worker::new_with_role(
        WorkerConfig {
            python: PathBuf::from(python),
            script: PathBuf::from(script),
            scratch,
            pycache,
            startup_timeout: Duration::from_secs(90),
            job_memory_limit_bytes: None,
        },
        sandbox,
        WorkerRole::Check,
    )))
}

#[cfg(not(windows))]
pub fn build_check_worker(
    _data_dir: &Path,
    _session: &str,
    _python: &str,
    _script: &str,
    _check_scratch: Option<String>,
    _check_pycache: Option<String>,
) -> Result<Option<Worker>, String> {
    Ok(None)
}

/// Build the Solve `sympy_eval` tool handler that re-stages portable ledger handles via
/// protocol-2 `exprs_in` before invoking the solver worker.
pub fn solve_sympy_handler(
    worker: Arc<Mutex<Option<Worker>>>,
    data_dir: PathBuf,
    timeout: Duration,
) -> Arc<dyn ToolHandler> {
    Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args["code"].as_str().ok_or("`code` must be a string")?;
        let staged = {
            let l = Ledger::open(&data_dir).map_err(|e| e.to_string())?;
            stage_exprs_in(&l, ctx.session)?
        };
        let exprs_arg = (!staged.is_empty()).then_some(&staged);
        let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
        let w = guard.as_mut().ok_or("no compute worker is configured for this session")?;
        let (result, ran_epoch) = w
            .evaluate_with_exprs(code, exprs_arg, timeout, ctx.required_epoch)
            .map_err(|e| e.to_string())?;
        ctx.record_worker_execution(ran_epoch, w.last_request_id());
        if let Some(hello) = w.hello() {
            ctx.record_worker_environment(hello);
        }
        Ok(result)
    })
}

/// Derive `(run_start_seq, leg_start_seq)` for `SolveGate` G4 budget counting from ledger events.
pub fn solve_gate_anchors(events: &[Event]) -> (u64, u64) {
    let run_start = events
        .iter()
        .find(|e| e.kind == event_kinds::SOLVE_SETUP || e.kind == event_kinds::SOLVE_STARTED)
        .map_or(0, |e| e.seq);
    let leg_start = events
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind.as_str(),
                "session.start" | "session.resume" | event_kinds::SOLVE_CHECKPOINT
            )
        })
        .map_or(run_start, |e| e.seq);
    (run_start, leg_start)
}

/// Fully wired Solve (`solve/1`) MCP router plus shared harvest-preview store.
pub struct WiredSolveRouter {
    pub router: Router,
    pub solve_gate: Arc<SolveGate>,
    pub previews: HarvestPreviews,
}

/// Assemble a `solve/1` MCP `Router` from explicit solver/check bindings and an `Approver`
/// (`TcpApprover`, `DenyAll`, or a test approver). Used by both `core` startup and
/// `tests/solve_mcp_gates.rs`.
#[allow(clippy::too_many_arguments)]
pub fn build_solve_router_with_bindings(
    env: Arc<ResearchEnv>,
    ledger: Ledger,
    session: &str,
    sympy_epoch: Arc<dyn WorkerEpoch>,
    sympy_handler: Arc<dyn ToolHandler>,
    check_epoch: Arc<dyn WorkerEpoch>,
    check_runner: Arc<dyn CheckProcessRunner>,
    canonicalizer: Arc<dyn CheckCanonicalizer>,
    approver: Arc<dyn Approver>,
) -> Result<WiredSolveRouter, String> {
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let (run_start_seq, leg_start_seq) = solve_gate_anchors(&events);

    let mut registry = Registry::new();
    let solve_host = solve_check_host_v1(check_epoch, Arc::clone(&env), Arc::clone(&check_runner));
    let policy = cli_route::core_tools::register_core_tools_for_profile_with_solve(
        &mut registry,
        sympy_epoch,
        sympy_handler,
        Arc::clone(&env),
        PROFILE_SOLVE_V1,
        Some(solve_host),
    )?;

    let previews = HarvestPreviews::new();
    let solve_gate = Arc::new(SolveGate::new(Arc::clone(&env), run_start_seq, leg_start_seq));
    let auto_check = Arc::new(AutoCheckGate::new(Arc::clone(&env), check_runner));
    let harvest_gate = Arc::new(SympyHarvestGate::new(
        env.data_dir.clone(),
        canonicalizer,
        previews.clone(),
    ));
    let chain = Arc::new(GateChain::new(vec![
        solve_gate.clone() as Arc<dyn Gate>,
        auto_check as Arc<dyn Gate>,
        harvest_gate as Arc<dyn Gate>,
    ]));

    let router = Router::new(registry, policy, ledger, session, approver)
        .with_tool_profile(PROFILE_SOLVE_V1)
        .with_gate(chain);
    Ok(WiredSolveRouter { router, solve_gate, previews })
}

/// Check timeout configured via `CORE_CHECK_TIMEOUT_S`, clamped to `[1, 60]` seconds.
pub fn parse_check_timeout(raw: Option<String>) -> Duration {
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .map(|secs| Duration::from_secs(secs.clamp(1, 60)))
        .unwrap_or(DEFAULT_CHECK_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_check_timeout_defaults_and_clamps() {
        assert_eq!(parse_check_timeout(None), DEFAULT_CHECK_TIMEOUT);
        assert_eq!(parse_check_timeout(Some("bad".into())), DEFAULT_CHECK_TIMEOUT);
        assert_eq!(parse_check_timeout(Some("0".into())), Duration::from_secs(1));
        assert_eq!(parse_check_timeout(Some("15".into())), Duration::from_secs(15));
        assert_eq!(parse_check_timeout(Some("999".into())), Duration::from_secs(60));
    }

    #[test]
    fn check_sandbox_id_is_distinct_and_within_appcontainer_bounds() {
        for sid in ["s-1", "solve_demo.1", &"a".repeat(43), &"b".repeat(60)] {
            let cid = check_sandbox_id(sid);
            assert_ne!(cid, sid.to_ascii_lowercase());
            assert!(!cid.is_empty() && cid.len() <= 43, "invalid len for {cid}");
            assert!(
                cid.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')),
                "invalid chars in {cid}"
            );
        }
    }

    struct UnusedCanon;
    impl CheckCanonicalizer for UnusedCanon {
        fn canonicalize(&self, _raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
            Err("should not canonicalize malformed non-array exprs_out".into())
        }
    }

    #[test]
    fn harvest_gate_surfaces_warnings_and_errors_on_malformed_exprs_out() {
        let dir = std::env::temp_dir().join(format!("solve_wire_malformed_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = Ledger::open(&dir).unwrap();
        ledger.append_with_hash("s-mal", event_kinds::TOOL_RESULT, 1, &serde_json::json!({ "call_id": "c-1" })).unwrap();

        let previews = HarvestPreviews::new();
        let gate = SympyHarvestGate::new(dir.clone(), Arc::new(UnusedCanon), previews.clone());
        let args = serde_json::json!({});
        let req = |call_id| GateRequest {
            session: "s-mal", caller: "claude-cli", tool_id: "sympy_eval", call_id,
            args: &args, canonical_args: "{}", effect: policy::EffectClass::ExecuteCode,
            approval_scope: tools::ApprovalScope::WorkerSession,
        };

        // Malformed non-object `exprs_out` with valid tool.result must still attach `expr_warnings`.
        let mut out1 = serde_json::json!({ "ok": true, "tool_call_ref": "c-1", "exprs_out": "not-an-object" });
        gate.on_outcome(&req("c-1"), &CallOutcome::Ok(out1.clone()));
        previews.attach_to_result(&mut out1);
        assert!(out1["expr_warnings"].as_array().is_some_and(|a| !a.is_empty()), "expected expr_warnings: {out1}");

        // Missing tool.result must surface `expr_harvest_error` rather than failing silently.
        let mut out2 = serde_json::json!({ "ok": true, "tool_call_ref": "c-missing", "exprs_out": "not-an-object" });
        gate.on_outcome(&req("c-missing"), &CallOutcome::Ok(out2.clone()));
        previews.attach_to_result(&mut out2);
        assert!(out2["expr_harvest_error"].as_str().is_some_and(|s| !s.is_empty()), "expected expr_harvest_error: {out2}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
