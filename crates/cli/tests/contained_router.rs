//! End-to-end `tools::Router` + `AppContainerSandbox` containment-gated `WorkerSession` test.
#![cfg(windows)]

use ledger::Ledger;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{
    builtin, ApprovalReply, ApprovalRequest, ApprovalScope, Approver, CallCtx, CallOutcome, Registry, Router,
    WorkerEpoch,
};
use worker::appcontainer::{delete_session_profile, AppContainerSandbox};
use worker::sandbox::Sandbox;
use worker::{EpochHandle, Worker, WorkerConfig};

fn python_exe() -> Option<String> {
    std::env::var("RE_SEARCH_PYTHON")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("RESEARCH_CORE_PYTHON").ok().filter(|v| !v.is_empty()))
}

struct RouterFixture {
    root: PathBuf,
    data_dir: PathBuf,
    session: String,
}

impl RouterFixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("cli-router-ac-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        Self { root, data_dir, session: format!("s-router-{tag}-{}", std::process::id()) }
    }
}

impl Drop for RouterFixture {
    fn drop(&mut self) {
        delete_session_profile(&self.session);
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct CountingApprover {
    asked: AtomicUsize,
    scopes: Mutex<Vec<ApprovalScope>>,
}

impl Approver for CountingApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.scopes.lock().unwrap().push(request.scope);
        ApprovalReply { approved: true, digest: request.digest.clone(), by: "mock".into() }
    }
}

struct WorkerEpochAdapter {
    epoch: EpochHandle,
    contained: bool,
}

impl WorkerEpoch for WorkerEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.live()
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

#[test]
fn contained_worker_reuses_session_approval_until_timeout_respawn() {
    let Some(python) = python_exe() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON to run contained_router test");
        return;
    };
    let fx = RouterFixture::new("epoch");
    let sdir = fx.data_dir.join("sessions").join(&fx.session);
    let scratch = sdir.join("scratch");
    let pycache = sdir.join("pycache");
    fs::create_dir_all(sdir.join("work")).unwrap();
    fs::create_dir_all(&scratch).unwrap();
    fs::create_dir_all(&pycache).unwrap();

    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python/worker.py");
    let sandbox = AppContainerSandbox::new(
        fx.data_dir.clone(),
        &fx.session,
        PathBuf::from(&python),
        script.clone(),
        scratch.clone(),
        pycache.clone(),
    )
    .unwrap();
    let contained = sandbox.containment().is_contained();
    assert!(contained);

    let worker = Worker::new(
        WorkerConfig {
            python: PathBuf::from(python),
            script,
            scratch,
            pycache,
            startup_timeout: Duration::from_secs(60),
        },
        Box::new(sandbox),
    );
    let epoch_handle = worker.epoch_handle();
    let worker = Arc::new(Mutex::new(worker));
    let timeout_s = Arc::new(AtomicU64::new(60));

    let mut registry = Registry::new();
    let worker_h = worker.clone();
    let timeout_h = timeout_s.clone();
    registry.register_worker_session(
        builtin::sympy_eval_def(),
        Arc::new(WorkerEpochAdapter { epoch: epoch_handle.clone(), contained }),
        Arc::new(move |ctx: &CallCtx<'_>, args: &serde_json::Value| {
            let code = args["code"].as_str().ok_or("`code` must be a string")?;
            let secs = timeout_h.load(Ordering::Acquire);
            let mut w = worker_h.lock().map_err(|_| "worker lock poisoned".to_string())?;
            let (result, ran_epoch) = w
                .evaluate_in_epoch(code, Duration::from_secs(secs), ctx.required_epoch)
                .map_err(|e| e.to_string())?;
            ctx.record_worker_epoch(ran_epoch);
            Ok(result)
        }),
    );

    let ledger = Ledger::open(&fx.data_dir).unwrap();
    let approver = Arc::new(CountingApprover { asked: AtomicUsize::new(0), scopes: Mutex::new(Vec::new()) });
    let router = Router::new(registry, builtin::default_policy(), ledger, &fx.session, approver.clone());
    let caller = format!("claude-code-cli:{}", fx.session);

    // Call 1: asks once with WorkerSession and establishes epoch 1.
    let out1 = router.call(&caller, builtin::SYMPY_EVAL, &json!({ "code": "x = 41; x + 1" }));
    let CallOutcome::Ok(v1) = out1 else { panic!("call 1 failed: {out1:?}") };
    assert_eq!(v1["value"], "42");
    assert_eq!(approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(&*approver.scopes.lock().unwrap(), &[ApprovalScope::WorkerSession]);
    assert_eq!(epoch_handle.live(), Some(1));

    // Call 2: reuses epoch 1 state and session grant without re-asking.
    let out2 = router.call(&caller, builtin::SYMPY_EVAL, &json!({ "code": "x + 9" }));
    let CallOutcome::Ok(v2) = out2 else { panic!("call 2 failed: {out2:?}") };
    assert_eq!(v2["value"], "50");
    assert_eq!(approver.asked.load(Ordering::SeqCst), 1);

    // Timeout kills epoch 1.
    timeout_s.store(1, Ordering::Release);
    let out_to = router.call(&caller, builtin::SYMPY_EVAL, &json!({ "code": "while True: pass" }));
    assert!(matches!(out_to, CallOutcome::Error(_)), "expected timeout error: {out_to:?}");
    assert_eq!(epoch_handle.live(), None);
    assert_eq!(approver.asked.load(Ordering::SeqCst), 1);

    // Call 3: new epoch 2 must ask again.
    timeout_s.store(60, Ordering::Release);
    let out3 = router.call(&caller, builtin::SYMPY_EVAL, &json!({ "code": "1 + 1" }));
    let CallOutcome::Ok(v3) = out3 else { panic!("call 3 failed: {out3:?}") };
    assert_eq!(v3["value"], "2");
    assert_eq!(approver.asked.load(Ordering::SeqCst), 2);
    assert_eq!(
        &*approver.scopes.lock().unwrap(),
        &[ApprovalScope::WorkerSession, ApprovalScope::WorkerSession]
    );
    assert_eq!(epoch_handle.live(), Some(2));

    let ledger_check = Ledger::open(&fx.data_dir).unwrap();
    let events = ledger_check.read_session(&fx.session).unwrap();
    assert!(events.iter().any(|e| e.kind == "approval.session_reused"), "missing approval.session_reused");
}
