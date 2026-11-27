use super::*;
use crate::policy_preset::{solve_policy, SOLVE_CHECK_CALLER};
use crate::solve_defs::{expr_show_def, solve_finish_def, step_record_def, EXPR_SHOW, SOLVE_FINISH};
use crate::solve_exprs::expr_show_handler;
use crate::solve_steps::step_record_handler;
use crate::tool_profile::{ProfileHostTools, SolveCheckHost, register_profile, solve_v1};
use ledger::sha256_hex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router, WorkerEpoch};

struct StaticEpoch {
    epoch: u64,
    contained: bool,
}

impl WorkerEpoch for StaticEpoch {
    fn live_epoch(&self) -> Option<u64> {
        Some(self.epoch)
    }
    fn is_contained(&self) -> bool {
        self.contained
    }
}

struct MockCheckRunner {
    timeout_next: AtomicBool,
    calls: AtomicU64,
    outcome: std::sync::Mutex<(&'static str, Value)>,
}

impl CheckProcessRunner for MockCheckRunner {
    fn run_check(
        &self,
        name: &str,
        _handles: &BTreeMap<String, String>,
        _params: &Value,
        timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        let req_id = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if self.timeout_next.swap(false, Ordering::SeqCst) {
            return Err(CheckExecError::Timeout(timeout));
        }
        let (out, ev) = self.outcome.lock().unwrap().clone();
        Ok(CheckExecResult {
            raw: json!({
                "name": name,
                "registry_sha256": "r".repeat(64),
                "process": "check",
                "wall_ms": 5,
                "outcome": out,
                "evidence": ev,
            }),
            epoch: 1,
            request_id: req_id,
            environment: Some(json!({ "role": "check", "protocol": 2 })),
        })
    }
}

struct ConditionalApprover {
    allow_check: AtomicBool,
}

impl Approver for ConditionalApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        let approved = !(req.tool_id == CHECK_RUN && !self.allow_check.load(Ordering::SeqCst));
        ApprovalReply {
            approved,
            digest: req.digest.clone(),
            by: if approved { "test-ok".into() } else { "denied-in-test".into() },
        }
    }
}

fn setup_router(
    allow_check: bool,
    runner: Arc<MockCheckRunner>,
) -> (TempDir, Arc<ResearchEnv>, Ledger, Router, String) {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    let env = Arc::new(ResearchEnv::with_workspace(data.clone(), ws).unwrap());
    let ledger = Ledger::open(&data).unwrap();
    let sid = "s-chk-1".to_string();
    ledger
        .append(
            &sid,
            event_kinds::SOLVE_SETUP,
            1,
            &json!({
                "revision": 1,
                "statement": "Test identity",
                "task_kind": "calculate",
                "target": { "description": "V", "form": "closed_form" },
                "symbols": [{ "name": "x", "assumptions": ["real"] }],
                "dimensions": { "x": { "length": 1 } },
                "roles": {
                    "solver": { "provider": "anthropic", "model": "claude-sonnet-4-6" },
                    "verifier": { "provider": "openai", "model": "gpt-5" }
                },
                "tool_profile": "solve/1",
                "math_sha256": "m".repeat(64),
            }),
        )
        .unwrap();

    // Seed solver sympy receipt + portable handles.
    let srepr_a = "Pow(Symbol('x', real=True), Integer(2))";
    let srepr_b = "Mul(Symbol('x', real=True), Symbol('x', real=True))";
    let sha_a = sha256_hex(srepr_a.as_bytes());
    let sha_b = sha256_hex(srepr_b.as_bytes());
    ledger.put_blob(srepr_a.as_bytes()).unwrap();
    ledger.put_blob(srepr_b.as_bytes()).unwrap();
    ledger
        .append(&sid, event_kinds::TOOL_REQUEST, 1, &json!({ "call_id": "sym-1", "tool": "sympy_eval", "caller": "solver" }))
        .unwrap();
    let rseq = ledger
        .append(&sid, event_kinds::TOOL_RESULT, 1, &json!({ "call_id": "sym-1", "ok": true, "result": { "inline": "{\"value\":\"x**2\",\"error\":null,\"code_sha256\":\"c1\",\"process\":\"solver\"}" } }))
        .unwrap();
    for (name, sha, latex) in [("lhs", sha_a, "x^2"), ("rhs", sha_b, "x \\cdot x")] {
        ledger
            .append(
                &sid,
                event_kinds::SOLVE_EXPR,
                1,
                &json!({
                    "name": name,
                    "version": 1,
                    "srepr_blob": sha,
                    "srepr_sha256": sha,
                    "latex": latex,
                    "str_preview": "x^2",
                    "free_symbols": [{ "name": "x", "assumptions": ["real"] }],
                    "produced_by": { "call_id": "sym-1", "result_seq": rseq },
                    "portable": true
                }),
            )
            .unwrap();
    }

    let epoch = Arc::new(StaticEpoch { epoch: 1, contained: true });
    let sympy_handler: Arc<dyn ToolHandler> =
        Arc::new(|_: &tools::CallCtx<'_>, _: &Value| Ok(json!({})));
    let check_handler = check_run_handler(Arc::clone(&env), runner.clone());
    let host = ProfileHostTools::new(epoch.clone(), sympy_handler).with_solve_check(
        SolveCheckHost::new(epoch)
            .with_check_handler(check_handler)
            .with_step_record_handler(step_record_handler(Arc::clone(&env))),
    );
    let mut reg = Registry::new();
    let policy = register_profile(&solve_v1(), &mut reg, Arc::clone(&env), host).unwrap();
    // Ensure imported defs compile cleanly cleanly alongside register_profile.
    let _ = (expr_show_def, solve_finish_def, step_record_def, EXPR_SHOW, SOLVE_FINISH, expr_show_handler, solve_policy);

    let approver = Arc::new(ConditionalApprover {
        allow_check: AtomicBool::new(allow_check),
    });
    let gate = Arc::new(AutoCheckGate::new(Arc::clone(&env), runner));
    let router = Router::new(reg, policy, Ledger::open(&data).unwrap(), &sid, approver)
        .with_tool_profile("solve/1")
        .with_gate(gate);

    (tmp, env, ledger, router, sid)
}

#[test]
fn auto_check_gate_runs_and_handles_denied_and_timeout() {
    // 1. Passing symbolic check updates step_record result with auto_checks & Verified badge.
    let runner_ok = Arc::new(MockCheckRunner {
        timeout_next: AtomicBool::new(false),
        calls: AtomicU64::new(0),
        outcome: std::sync::Mutex::new(("pass", json!({ "comparison": "exact", "exact": true }))),
    });
    let (_tmp, _env, ledger, router, sid) = setup_router(true, runner_ok);
    let out = router
        .try_call(
            "solver:solve",
            STEP_RECORD,
            &json!({
                "entry_type": "symbolic_computation",
                "summary": "Check x^2 = x*x",
                "details": "Identity check",
                "tool_call_refs": ["sym-1"],
                "claims": [{ "form": "equation", "lhs": "lhs@1", "rhs": "rhs@1", "relation": "eq" }]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else {
        panic!("expected Ok, got {out:?}");
    };
    assert_eq!(val["own_tier"], "verified");
    let autos = val["auto_checks"].as_array().unwrap();
    assert_eq!(autos.len(), 2, "expected identity_symbolic + S10 dimensions not_run: {autos:?}");
    assert_eq!(autos[0]["check"], "identity_symbolic");
    assert_eq!(autos[0]["outcome"], "pass");
    assert_eq!(autos[1]["check"], "dimensions");
    assert_eq!(autos[1]["outcome"], "not_run");

    // 2. Denied host:check grant produces `not_run` (never auto-approved, R18).
    let runner_den = Arc::new(MockCheckRunner {
        timeout_next: AtomicBool::new(false),
        calls: AtomicU64::new(0),
        outcome: std::sync::Mutex::new(("pass", json!({ "comparison": "exact", "exact": true }))),
    });
    let (_tmp2, _env2, _ledger2, router_den, _sid2) = setup_router(false, runner_den);
    let out_den = router_den
        .try_call(
            "solver:solve",
            STEP_RECORD,
            &json!({
                "entry_type": "symbolic_computation",
                "summary": "Denied check",
                "details": "No grant",
                "tool_call_refs": ["sym-1"],
                "claims": [{ "form": "equation", "lhs": "lhs@1", "rhs": "rhs@1", "relation": "eq" }]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val_den) = out_den else {
        panic!("expected Ok even when check denied: {out_den:?}");
    };
    assert_eq!(val_den["own_tier"], "unchecked");
    assert_eq!(val_den["auto_checks"][0]["outcome"], "not_run");

    // 3. Timeout records `inconclusive` solve.check without crashing, plus R3 rejects `code` arg.
    let runner_to = Arc::new(MockCheckRunner {
        timeout_next: AtomicBool::new(true),
        calls: AtomicU64::new(0),
        outcome: std::sync::Mutex::new(("pass", json!({}))),
    });
    let (_tmp3, _env3, ledger3, router_to, sid3) = setup_router(true, runner_to);
    let _ = router_to
        .try_call(
            "solver:solve",
            STEP_RECORD,
            &json!({
                "entry_type": "symbolic_computation",
                "summary": "Timeout step",
                "details": "Runs timeout",
                "tool_call_refs": ["sym-1"],
                "claims": [{ "form": "equation", "lhs": "lhs@1", "rhs": "rhs@1", "relation": "eq" }]
            }),
        )
        .unwrap();
    let evs = ledger3.read_session(&sid3).unwrap();
    let checks: Vec<_> = evs.iter().filter(|e| e.kind == event_kinds::SOLVE_CHECK).collect();
    assert!(checks.iter().any(|c| c.payload["outcome"] == "inconclusive"));
    assert_eq!(ledger.read_session(&sid).unwrap().iter().filter(|e| e.kind == event_kinds::SOLVE_CHECK).count(), 1);
    let bad = router_to
        .try_call(
            SOLVE_CHECK_CALLER,
            CHECK_RUN,
            &json!({ "entry_id": "e-1", "check": "identity_symbolic", "code": "bad" }),
        )
        .unwrap();
    assert!(matches!(bad, CallOutcome::Error { .. }));
}
