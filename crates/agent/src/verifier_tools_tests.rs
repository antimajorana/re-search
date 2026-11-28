use super::*;
use research_tools::solve_checks::{CheckExecError, CheckExecResult};
use research_tools::solve_exprs::CanonicalizedExpr;
use research_tools::solve_model::{compute_entry_sha256, event_kinds, StepEntryType};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest};

struct AllowAllApprover;
impl Approver for AllowAllApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "test".into(),
        }
    }
}

struct StubCheckEpoch;
impl WorkerEpoch for StubCheckEpoch {
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
}

struct StubCheckRunner;
impl CheckProcessRunner for StubCheckRunner {
    fn run_check(
        &self,
        _name: &str,
        _handles: &BTreeMap<String, String>,
        _params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        Ok(CheckExecResult {
            raw: json!({ "outcome": "pass", "evidence": { "comparison": "exact", "exact": true } }),
            epoch: 1,
            request_id: 1,
            environment: None,
        })
    }
}

struct StubCanonicalizer;
impl CheckCanonicalizer for StubCanonicalizer {
    fn canonicalize(&self, _raw: &str) -> Result<CanonicalizedExpr, String> {
        Err("unused".into())
    }
}

struct CountingIsolatedWorker {
    dropped: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl IsolatedPythonWorker for CountingIsolatedWorker {
    fn evaluate(
        &self,
        code: &str,
        _exprs_in: Option<&BTreeMap<String, String>>,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<IsolatedEvalOutput, String> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(IsolatedEvalOutput {
            result: json!({ "value": format!("ran:{code}"), "stdout": "", "stderr": "", "error": null }),
            epoch: 1,
            request_id: n as u64,
            environment: Some(json!({ "protocol": 2, "role": "solver" })),
        })
    }
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
    fn containment_summary(&self) -> String {
        "stub-isolated".into()
    }
}

impl Drop for CountingIsolatedWorker {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct CountingFactory {
    spawns: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl IsolatedWorkerFactory for CountingFactory {
    fn spawn_isolated_python_worker(
        &self,
        _session: &str,
        _purpose: &str,
    ) -> Result<Box<dyn IsolatedPythonWorker>, String> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountingIsolatedWorker {
            dropped: Arc::clone(&self.dropped),
            calls: Arc::clone(&self.calls),
        }))
    }
    fn check_runner(&self, _session: &str) -> Result<CheckRunnerBinding, String> {
        Ok((Arc::new(StubCheckRunner), Arc::new(StubCheckEpoch), true))
    }
    fn check_canonicalizer(&self, _session: &str) -> Result<Arc<dyn CheckCanonicalizer>, String> {
        Ok(Arc::new(StubCanonicalizer))
    }
    fn isolated_workers_contained(&self) -> bool {
        true
    }
}

#[test]
fn verifier_router_exposes_only_allowed_tools_and_clamps_ledger_query_to_closure() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-vtools";
    for (id, deps) in [("e-1", vec![]), ("e-2", vec!["e-1"]), ("e-3", vec![])] {
        let sha = compute_entry_sha256(None, id, "details");
        ledger
            .append_with_hash(
                session,
                event_kinds::PROOF_ENTRY,
                1,
                &json!({
                    "entry_id": id,
                    "entry_type": StepEntryType::Derivation.as_str(),
                    "summary": format!("Step {id}"),
                    "details": format!("Body {id}"),
                    "depends_on": deps,
                    "entry_sha256": sha,
                }),
            )
            .unwrap();
    }

    let spawns = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(CountingFactory {
        spawns: Arc::clone(&spawns),
        dropped: Arc::clone(&dropped),
        calls: Arc::clone(&calls),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let router_ledger = Ledger::open(dir.path()).unwrap();
    let (router, lazy_worker) = build_verifier_router(
        env,
        router_ledger,
        session,
        Arc::new(AllowAllApprover),
        &factory,
    )
    .unwrap();

    // Specs contain exactly the 6 allowed registry tools + report_verdict.
    let spec_names: BTreeSet<String> = verifier_model_tool_specs(&router)
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(spec_names.len(), 7);
    assert!(spec_names.contains(REPORT_VERDICT));
    assert!(!spec_names.contains("step_record"));
    assert!(!spec_names.contains("solve_finish"));
    assert!(!spec_names.contains("fs_read"));

    // Closure contains only e-1 and e-2 (e-3 is outside closure).
    let closure: BTreeSet<String> = ["e-1".to_string(), "e-2".to_string()].into_iter().collect();

    // Refuses solver-only tool.
    let err = dispatch_verifier_registry_tool(&router, "step_record", &json!({}), &closure).unwrap_err();
    assert!(err.contains("not permitted"));

    // Refuses direct ledger_query for outside-closure step e-3.
    let err = dispatch_verifier_registry_tool(
        &router,
        "ledger_query",
        &json!({ "entry_id": "e-3" }),
        &closure,
    )
    .unwrap_err();
    assert!(err.contains("outside the target review closure"));

    // Unfiltered ledger_query strips e-3 and session-wide summary fields.
    let res = dispatch_verifier_registry_tool(&router, "ledger_query", &json!({}), &closure).unwrap();
    let ids: Vec<&str> = res["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["entry_id"].as_str())
        .collect();
    assert_eq!(ids, vec!["e-1", "e-2"]);
    assert!(res.get("latest_full_verification_accepted").is_none());
    assert_eq!(res["closure_scoped"], json!(true));

    // Lazy worker was NOT spawned by ledger_query; spawns on first sympy_eval and drops on shutdown.
    assert_eq!(spawns.load(Ordering::SeqCst), 0);
    assert!(!lazy_worker.was_spawned());

    let eval_res = dispatch_verifier_registry_tool(
        &router,
        "sympy_eval",
        &json!({ "code": "2 + 2" }),
        &closure,
    )
    .unwrap();
    assert_eq!(eval_res["value"], json!("ran:2 + 2"));
    assert_eq!(eval_res["process"], json!("verifier"));
    assert_eq!(spawns.load(Ordering::SeqCst), 1);
    assert!(lazy_worker.was_spawned());

    lazy_worker.shutdown();
    drop(router);
    assert!(dropped.load(Ordering::SeqCst));
}
