//! Solve verifier tools, `IsolatedWorkerFactory` crate boundary, and closure-scoped
//! registry dispatch (`solve.md` 4.3, 5.1–5.3, rules R3 & R13; package S14).
//!
//! `crates/agent` intentionally has NO dependency on `crates/worker`. Both S14 (verifier
//! worker) and S20 (blind compute worker) spawn isolated Python processes through
//! [`IsolatedWorkerFactory`], implemented in `crates/desktop-server/src/solve_workers.rs`
//! (or test harnesses).

use crate::transport::ToolSpec;
use ledger::{sha256_hex, Ledger};
use research_tools::policy_preset::SOLVE_VERIFIER_CALLER;
use research_tools::solve_checks::{check_run_handler, CheckProcessRunner};
use research_tools::solve_defs::{CHECK_RUN, EXPR_SHOW};
use research_tools::solve_exprs::{stage_exprs_in, CheckCanonicalizer};
use research_tools::tool_profile::{
    register_profile, solve_v1, ProfileHostTools, SolveCheckHost,
};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tools::{Approver, CallCtx, CallOutcome, Registry, Router, ToolHandler, WorkerEpoch};

/// Terminal host-handled tool used by the Solve verifier loop to submit its verdict batch.
pub const REPORT_VERDICT: &str = "report_verdict";
/// Host-handled tool exposed to the Solve solver loop to invoke the verifier on 1..=5 steps.
pub const REVIEW_STEPS: &str = "review_steps";

/// Exact `solve/1` registry tools exposed to `verifier:solve` (`solve.md` 5.1 & 5.3).
pub const VERIFIER_REGISTRY_TOOL_IDS: &[&str] = &[
    "ledger_query",
    EXPR_SHOW,
    "sympy_eval",
    CHECK_RUN,
    "read_paper_lines",
    "search_paper_passages",
];

/// Raw execution output returned by an [`IsolatedPythonWorker`].
#[derive(Debug, Clone)]
pub struct IsolatedEvalOutput {
    pub result: Value,
    pub epoch: u64,
    pub request_id: u64,
    pub environment: Option<Value>,
}

/// Handle to a dedicated isolated Python worker process (`solve.md` 4.3, R13, S20).
/// Dropping the handle must terminate the underlying process so no state leaks across loops.
pub trait IsolatedPythonWorker: Send + Sync {
    fn evaluate(
        &self,
        code: &str,
        exprs_in: Option<&BTreeMap<String, String>>,
        timeout: Duration,
        required_epoch: Option<u64>,
    ) -> Result<IsolatedEvalOutput, String>;

    fn live_epoch(&self) -> Option<u64>;
    fn is_contained(&self) -> bool;
    fn containment_summary(&self) -> String;
}

/// Tuple returned by [`IsolatedWorkerFactory::check_runner`]: `(runner, epoch, is_contained)`.
pub type CheckRunnerBinding = (Arc<dyn CheckProcessRunner>, Arc<dyn WorkerEpoch>, bool);

/// Host factory injected into `agent` so S14 and S20 can spawn isolated Python workers
/// and access the dedicated check process without coupling `agent` to `worker`.
pub trait IsolatedWorkerFactory: Send + Sync {
    /// Spawn a brand-new isolated Python worker for `purpose` (`"verifier"` or `"blind_compute"`).
    /// Must never share Python process state with the solver worker.
    fn spawn_isolated_python_worker(
        &self,
        session: &str,
        purpose: &str,
    ) -> Result<Box<dyn IsolatedPythonWorker>, String>;

    /// Access the dedicated check-role process runner and epoch for `session`.
    fn check_runner(&self, session: &str) -> Result<CheckRunnerBinding, String>;

    /// Access the dedicated check-role canonicalizer for harvesting expressions (S20).
    fn check_canonicalizer(&self, session: &str) -> Result<Arc<dyn CheckCanonicalizer>, String>;

    /// Whether isolated Python workers spawned by this factory run inside OS containment.
    fn isolated_workers_contained(&self) -> bool {
        false
    }

    /// Evaluation timeout applied to isolated worker `sympy_eval` calls.
    fn eval_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Lazily spawns one isolated verifier Python worker on the first `sympy_eval` call
/// and tears it down when dropped at the end of the review loop (`solve.md` 4.3).
pub struct LazyIsolatedWorker {
    factory: Arc<dyn IsolatedWorkerFactory>,
    session: String,
    purpose: String,
    inner: Mutex<Option<Box<dyn IsolatedPythonWorker>>>,
}

impl LazyIsolatedWorker {
    pub fn new(
        factory: Arc<dyn IsolatedWorkerFactory>,
        session: impl Into<String>,
        purpose: impl Into<String>,
    ) -> Self {
        Self {
            factory,
            session: session.into(),
            purpose: purpose.into(),
            inner: Mutex::new(None),
        }
    }

    /// Returns `true` if a worker process has actually been spawned.
    pub fn was_spawned(&self) -> bool {
        self.inner.lock().ok().is_some_and(|g| g.is_some())
    }

    /// Explicitly drop the spawned worker process, if any.
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.take();
        }
    }

    pub fn evaluate(
        &self,
        code: &str,
        exprs_in: Option<&BTreeMap<String, String>>,
        required_epoch: Option<u64>,
    ) -> Result<IsolatedEvalOutput, String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "isolated worker lock poisoned".to_string())?;
        if guard.is_none() {
            let spawned = self
                .factory
                .spawn_isolated_python_worker(&self.session, &self.purpose)?;
            *guard = Some(spawned);
        }
        let worker = guard.as_ref().expect("spawned above");
        worker.evaluate(code, exprs_in, self.factory.eval_timeout(), required_epoch)
    }
}

impl Drop for LazyIsolatedWorker {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.take();
        }
    }
}

struct LazyWorkerEpoch {
    worker: Arc<LazyIsolatedWorker>,
}

impl WorkerEpoch for LazyWorkerEpoch {
    fn live_epoch(&self) -> Option<u64> {
        self.worker
            .inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|w| w.live_epoch()))
    }

    fn is_contained(&self) -> bool {
        if let Ok(guard) = self.worker.inner.lock() {
            if let Some(w) = guard.as_ref() {
                return w.is_contained();
            }
        }
        self.worker.factory.isolated_workers_contained()
    }
}

/// Terminal verifier tool spec (`solve.md` 5.3). Lives outside `tool_defs.txt` drift checks.
pub fn report_verdict_tool_spec() -> ToolSpec {
    ToolSpec {
        name: REPORT_VERDICT.to_string(),
        description: "Submit the final verification verdict for every target step in this batch. \
                      Every `checks_cited` id must exist in the session ledger and match the \
                      target step's current claim and setup mathematics."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "verdicts": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 5,
                    "items": {
                        "type": "object",
                        "properties": {
                            "entry_id": { "type": "string", "maxLength": 64 },
                            "verdict": { "type": "string", "enum": ["accepted", "rejected"] },
                            "summary": { "type": "string", "maxLength": 4000 },
                            "counterexample": { "type": ["string", "null"], "maxLength": 2000 },
                            "issues": {
                                "type": "array",
                                "items": {
                                    "oneOf": [
                                        { "type": "string", "maxLength": 1000 },
                                        {
                                            "type": "object",
                                            "properties": {
                                                "severity": {
                                                    "type": "string",
                                                    "enum": ["info", "warning", "error", "blocker"]
                                                },
                                                "message": { "type": "string", "maxLength": 1000 }
                                            },
                                            "required": ["message"],
                                            "additionalProperties": false
                                        }
                                    ]
                                }
                            },
                            "checks_cited": {
                                "type": "array",
                                "items": { "type": "string", "maxLength": 64 }
                            }
                        },
                        "required": ["entry_id", "verdict", "summary"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["verdicts"],
            "additionalProperties": false
        }),
    }
}

/// Host-handled `review_steps` tool spec offered to the Solve solver loop (`solve.md` 5.2).
pub fn review_steps_tool_spec() -> ToolSpec {
    ToolSpec {
        name: REVIEW_STEPS.to_string(),
        description: "Request an independent verifier review of 1 to 5 active derivation steps \
                      over their union dependency closure. Counts against `max_review_loops`."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "entry_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 5,
                    "items": { "type": "string", "maxLength": 64 }
                }
            },
            "required": ["entry_ids"],
            "additionalProperties": false
        }),
    }
}

/// Build provider [`ToolSpec`]s exposed to the Solve verifier loop: the 6 allowed `solve/1`
/// registry tools plus the terminal `report_verdict` host tool (`solve.md` 5.3).
pub fn verifier_model_tool_specs(router: &Router) -> Vec<ToolSpec> {
    let mut specs: Vec<ToolSpec> = router
        .registry()
        .defs()
        .into_iter()
        .filter(|d| VERIFIER_REGISTRY_TOOL_IDS.contains(&d.id.as_str()))
        .map(|d| ToolSpec {
            name: d.id.clone(),
            description: d.description.clone(),
            input_schema: d.input_schema.clone(),
        })
        .collect();
    specs.push(report_verdict_tool_spec());
    specs
}

/// Build a `solve/1` [`Router`] scoped for `verifier:solve` with a lazy isolated verifier
/// Python worker for `sympy_eval` and the session check process for `check_run`.
pub fn build_verifier_router(
    env: Arc<ResearchEnv>,
    ledger: Ledger,
    session: &str,
    approver: Arc<dyn Approver>,
    factory: &Arc<dyn IsolatedWorkerFactory>,
) -> Result<(Router, Arc<LazyIsolatedWorker>), String> {
    let (check_runner, check_epoch, _check_contained) = factory.check_runner(session)?;
    let lazy_worker = Arc::new(LazyIsolatedWorker::new(
        Arc::clone(factory),
        session,
        "verifier",
    ));
    let sympy_epoch: Arc<dyn WorkerEpoch> = Arc::new(LazyWorkerEpoch {
        worker: Arc::clone(&lazy_worker),
    });

    let eval_worker = Arc::clone(&lazy_worker);
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args
            .get("code")
            .and_then(Value::as_str)
            .ok_or("`code` must be a string")?;
        // Stage portable handles from ledger blobs so `expr(name)` works without sharing solver state.
        let staged = {
            let guard = ctx
                .ledger
                .lock()
                .map_err(|_| "ledger lock poisoned".to_string())?;
            stage_exprs_in(&guard, ctx.session)?
        };
        let exprs_arg = (!staged.is_empty()).then_some(&staged);
        let started = Instant::now();
        let output = eval_worker.evaluate(code, exprs_arg, ctx.required_epoch)?;
        let wall_ms = started.elapsed().as_millis() as u64;
        if output.epoch > 0 {
            ctx.record_worker_execution(output.epoch, output.request_id);
        }
        if let Some(env_val) = &output.environment {
            ctx.record_worker_environment(env_val);
        }
        let mut res = output.result;
        if let Some(obj) = res.as_object_mut() {
            obj.entry("code_sha256".to_string())
                .or_insert_with(|| json!(sha256_hex(code.as_bytes())));
            obj.entry("wall_ms".to_string()).or_insert_with(|| json!(wall_ms));
            obj.insert("process".to_string(), json!("verifier"));
        }
        Ok(res)
    });

    let check_handler = check_run_handler(Arc::clone(&env), check_runner);
    let host = ProfileHostTools::new(sympy_epoch, sympy_handler)
        .with_solve_check(SolveCheckHost::new(check_epoch).with_check_handler(check_handler));
    let mut registry = Registry::new();
    let policy = register_profile(&solve_v1(), &mut registry, env, host)?;
    let router = Router::new(registry, policy, ledger, session, approver).with_tool_profile("solve/1");
    Ok((router, lazy_worker))
}

/// Dispatch one registry tool call on behalf of `verifier:solve`, enforcing the verifier
/// allowlist and clamping `ledger_query` strictly to `closure_ids` (`solve.md` 5.1 & 5.3).
pub fn dispatch_verifier_registry_tool(
    router: &Router,
    tool_name: &str,
    args: &Value,
    closure_ids: &BTreeSet<String>,
) -> Result<Value, String> {
    if !VERIFIER_REGISTRY_TOOL_IDS.contains(&tool_name) {
        return Err(format!(
            "tool `{tool_name}` is not permitted for `{SOLVE_VERIFIER_CALLER}`"
        ));
    }
    if tool_name == "ledger_query" || tool_name == CHECK_RUN {
        if let Some(eid) = args
            .get("entry_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !closure_ids.contains(eid) {
                return Err(format!(
                    "{tool_name} refused: step `{eid}` is outside the target review closure"
                ));
            }
        }
    }

    let outcome = router.try_call(SOLVE_VERIFIER_CALLER, tool_name, args)?;
    let mut value = match outcome {
        CallOutcome::Ok(v) => v,
        CallOutcome::Denied(reason) => return Err(format!("denied: {reason}")),
        CallOutcome::Error(err) => return Err(err),
    };

    if tool_name == "ledger_query" {
        clamp_ledger_query_output(&mut value, closure_ids);
    }
    Ok(value)
}

fn clamp_ledger_query_output(value: &mut Value, closure_ids: &BTreeSet<String>) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(entries) = obj.get_mut("entries").and_then(Value::as_array_mut) {
        entries.retain(|e| {
            e.get("entry_id")
                .and_then(Value::as_str)
                .is_some_and(|id| closure_ids.contains(id))
        });
    }
    if let Some(edges) = obj.get_mut("edges").and_then(Value::as_array_mut) {
        edges.retain(|edge| {
            edge.as_array().is_some_and(|pair| {
                pair.len() == 2
                    && pair[0]
                        .as_str()
                        .is_some_and(|a| closure_ids.contains(a))
                    && pair[1]
                        .as_str()
                        .is_some_and(|b| closure_ids.contains(b))
            })
        });
    }
    // Strip session-wide summary fields that reflect steps outside the verifier's closure.
    obj.remove("latest_full_verification_accepted");
    obj.remove("unverified_computations");
    obj.insert("closure_scoped".into(), json!(true));
}

#[cfg(test)]
#[path = "verifier_tools_tests.rs"]
mod tests;
