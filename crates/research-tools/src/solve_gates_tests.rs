use super::*;
use crate::solve_checks::{CheckExecError, CheckExecResult};
use crate::solve_model::{
    compute_math_sha256, ReviewIndependence, RoleModel, SolveBudgets, SolveRoles, SolveSetup,
    SolveTarget, SymbolSpec, TargetForm, TaskKind,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use tempfile::TempDir;
use tools::{Approver, ApprovalReply, ApprovalRequest, CallCtx, CallOutcome, Registry, Router, ToolHandler};

struct NoopCheckRunner;
impl CheckProcessRunner for NoopCheckRunner {
    fn run_check(
        &self,
        _name: &str,
        _handles: &BTreeMap<String, String>,
        _params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        Ok(CheckExecResult {
            raw: json!({ "outcome": "pass", "evidence": {}, "registry_sha256": "reg" }),
            epoch: 1,
            request_id: 1,
            environment: None,
        })
    }
}

struct OkEpoch;
impl WorkerEpoch for OkEpoch {
    fn live_epoch(&self) -> Option<u64> { Some(1) }
    fn is_contained(&self) -> bool { true }
}

struct OkApprover;
impl Approver for OkApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "gate-test".into() }
    }
}

fn seed_setup(ledger: &Ledger, sid: &str, budgets: SolveBudgets) -> u64 {
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &[], "");
    let setup = SolveSetup {
        revision: 1,
        statement: "solve".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget { description: "x".into(), form: TargetForm::ClosedForm, order: None, digits: None },
        done: vec![],
        conventions: String::new(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions: vec![],
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "a".into(), model: "m1".into() },
            verifier: RoleModel { provider: "b".into(), model: "m2".into() },
            compute: None,
        },
        budgets,
        review: crate::solve_model::ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha,
    };
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap())
        .unwrap()
        .seq
}

#[test]
fn gate_g1_and_g5_block_uncited_sympy_runs_and_trigger_gate_bounces_pause() {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-gate-g1-g5";
    let seq0 = seed_setup(
        &ledger,
        sid,
        SolveBudgets { max_gate_bounces: 3, ..SolveBudgets::default() },
    );

    let sympy_count = Arc::new(AtomicUsize::new(0));
    let sc = Arc::clone(&sympy_count);
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, _args: &Value| {
        ctx.record_worker_execution(1, 1);
        sc.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "value": "1", "error": null, "process": "solver" }))
    });
    let mut reg = Registry::new();
    let host = crate::tool_profile::ProfileHostTools::new(Arc::new(OkEpoch), sympy_handler)
        .with_solve_check(solve_check_host_v1(
            Arc::new(OkEpoch),
            Arc::clone(&env),
            Arc::new(NoopCheckRunner),
        ));
    let policy = crate::tool_profile::register_profile(
        &crate::tool_profile::solve_v1(),
        &mut reg,
        Arc::clone(&env),
        host,
    )
    .unwrap();
    let (gate, chain) = solve_gate_chain(Arc::clone(&env), Arc::new(NoopCheckRunner), seq0, seq0);
    let router = Router::new(reg, policy, ledger, sid, Arc::new(OkApprover))
        .with_tool_profile(crate::tool_profile::PROFILE_SOLVE_V1)
        .with_gate(chain);

    // 3 exploratory `sympy_eval` runs succeed.
    for _ in 0..3 {
        let out = router.try_call("solver:solve", "sympy_eval", &json!({ "code": "1" })).unwrap();
        assert!(matches!(out, CallOutcome::Ok(_)), "{out:?}");
    }
    assert_eq!(sympy_count.load(Ordering::SeqCst), 3);

    // 4th `sympy_eval` is blocked by G1 without executing handler; `ledger_note` "discard:..." clears it.
    let blocked = router.try_call("solver:solve", "sympy_eval", &json!({ "code": "2" })).unwrap();
    assert!(matches!(&blocked, CallOutcome::Error(m) if m.starts_with("GATE G1:")), "{blocked:?}");
    assert_eq!(sympy_count.load(Ordering::SeqCst), 3);

    let discarded = router
        .try_call("solver:solve", "ledger_note", &json!({ "text": "discard: exploratory runs" }))
        .unwrap();
    assert!(matches!(discarded, CallOutcome::Ok(_)), "{discarded:?}");

    // Now a new `sympy_eval` is allowed again!
    let ok_after = router.try_call("solver:solve", "sympy_eval", &json!({ "code": "3" })).unwrap();
    assert!(matches!(ok_after, CallOutcome::Ok(_)), "{ok_after:?}");

    // Now test G5: 3 consecutive gate bounces (e.g., G7 refusal on missing `final_entry_id`) -> `gate_bounces`.
    for i in 0..3 {
        let b = router
            .try_call(
                "solver:solve",
                SOLVE_FINISH,
                &json!({ "final_entry_id": "e-missing", "summary_markdown": "done" }),
            )
            .unwrap();
        assert!(matches!(&b, CallOutcome::Error(m) if m.starts_with("GATE G7:")), "bounce {i}: {b:?}");
    }
    let next_block = router
        .try_call("solver:solve", "ledger_note", &json!({ "text": "still looping" }))
        .unwrap();
    assert!(matches!(&next_block, CallOutcome::Error(m) if m.starts_with("GATE G5:")), "{next_block:?}");

    let paused = gate.check_and_emit_pause(sid, 1, 8).unwrap().expect("expected G5 pause");
    assert_eq!(paused.reason, PauseReason::GateBounces);
}

#[test]
fn gate_g6_and_g4_detect_verification_stuck_and_model_or_wall_caps() {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-gate-g6-g4";
    let seq0 = seed_setup(
        &ledger,
        sid,
        SolveBudgets {
            max_model_calls: 4,
            max_wall_minutes: 2,
            max_model_calls_per_leg: 2,
            max_review_loops: 6,
            max_gate_bounces: 3,
        },
    );

    // Emit 2 leg model calls -> G4 LegCap triggers cleanly via F9 count_model_calls.
    ledger.append_with_hash(sid, "model.request", 1, &json!({ "turn": 1 })).unwrap();
    ledger.append_with_hash(sid, "model.request", 1, &json!({ "turn": 2 })).unwrap();
    let gate = SolveGate::new(Arc::clone(&env), seq0, seq0);
    let leg_trig = gate.check_pause_trigger(sid).unwrap().expect("expected G4 LegCap");
    assert_eq!(leg_trig.reason, PauseReason::LegCap);

    // Advance leg_start_seq past those 2 calls; 2 more calls hit total budget (`max_model_calls: 4`).
    let cur_seq = ledger.read_session(sid).unwrap().last().unwrap().seq;
    gate.set_leg_start_seq(cur_seq);
    assert!(gate.check_pause_trigger(sid).unwrap().is_none());
    ledger.append_with_hash(sid, "verifier.request", 1, &json!({ "turn": 1 })).unwrap();
    ledger.append_with_hash(sid, "model.request", 1, &json!({ "turn": 3 })).unwrap();
    let budget_trig = gate.check_pause_trigger(sid).unwrap().expect("expected G4 Budget");
    assert_eq!(budget_trig.reason, PauseReason::Budget);

    // Wall-clock gate test without sleeping.
    let gate_wall = SolveGate::new(Arc::clone(&env), cur_seq, cur_seq)
        .with_elapsed_fn(Arc::new(|| Duration::from_secs(121)));
    let events_wall = ledger.read_session(sid).unwrap();
    let st_wall = SolveState::from_events(&events_wall);
    let w_trig = detect_g4_budget(&st_wall, &events_wall, cur_seq, cur_seq, gate_wall.elapsed())
        .expect("expected wall budget pause");
    assert_eq!(w_trig.reason, PauseReason::Budget);

    // G6: 2 rejected reviews across supersede lineage `e-1` -> `e-2`.
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({ "entry_id": "e-1", "entry_type": "derivation", "summary": "a", "details": "d", "depends_on": [] }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({ "entry_id": "e-2", "entry_type": "correction", "summary": "b", "details": "d2", "depends_on": [] }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_SUPERSEDE,
            1,
            &json!({ "entry_id": "e-1", "replaced_by": "e-2", "reason": "retry" }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_VERIFICATION,
            1,
            &json!({ "target_entry_id": "e-1", "verdict": "rejected", "independence": ReviewIndependence::DifferentModel }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_VERIFICATION,
            1,
            &json!({ "target_entry_id": "e-2", "verdict": "rejected", "independence": ReviewIndependence::DifferentModel }),
        )
        .unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let st = SolveState::from_events(&evs);
    let stuck = detect_g6_verification_stuck(&st, &evs).expect("expected G6 verification_stuck");
    assert_eq!(stuck.reason, PauseReason::VerificationStuck);
}

#[test]
fn g3_blind_compute_gate_refuses_leaks_and_records_guardrail_blocked() {
    let dir = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(dir.path()));
    let ledger = Ledger::open(dir.path()).unwrap();
    let sid = "s-g3-blind";
    seed_setup(&ledger, sid, SolveBudgets::default());

    // Valid call passes.
    let valid = json!({
        "inputs": ["poly_a@1"],
        "operation": "Integrate poly_a from 0 to 1 with respect to x.",
        "compare_with": "sum@1"
    });
    assert!(check_g3_blind_compute(&valid).is_ok());

    // Identifier-boundary precision: `sum@1` must NOT false-positive on "assume".
    let boundary_ok = json!({
        "inputs": ["poly_a@1"],
        "operation": "Assume x > 0 and expand poly_a.",
        "compare_with": "sum@1"
    });
    assert!(check_g3_blind_compute(&boundary_ok).is_ok());

    // Exact match in `inputs` -> refused.
    let leak_input_exact = json!({
        "inputs": ["poly_a@1", "ans@1"],
        "operation": "Compute derivative of poly_a.",
        "compare_with": "ans@1"
    });
    assert!(check_g3_blind_compute(&leak_input_exact).is_err());

    // Same base name in `inputs` (different version) -> refused.
    let leak_input_base = json!({
        "inputs": ["ans@2"],
        "operation": "Simplify input.",
        "compare_with": "ans@1"
    });
    assert!(check_g3_blind_compute(&leak_input_base).is_err());

    // `compare_with` handle or base token in `operation` -> refused.
    let leak_op_handle = json!({
        "inputs": ["poly_a@1"],
        "operation": "Recompute ans@1 from poly_a.",
        "compare_with": "ans@1"
    });
    assert!(check_g3_blind_compute(&leak_op_handle).is_err());

    let leak_op_base = json!({
        "inputs": ["poly_a@1"],
        "operation": "Verify that derivative equals `ans`.",
        "compare_with": "ans@1"
    });
    assert!(check_g3_blind_compute(&leak_op_base).is_err());

    // Operation > 600 chars -> refused.
    let too_long = json!({
        "inputs": ["poly_a@1"],
        "operation": "x".repeat(COMPUTE_BLIND_MAX_OPERATION_CHARS + 1),
        "compare_with": "ans@1"
    });
    assert!(check_g3_blind_compute(&too_long).is_err());

    // SolveGate::before returns GateDecision::Block with gate id "G3".
    let gate = SolveGate::new(Arc::clone(&env), 0, 0);
    let req = tools::GateRequest {
        session: sid,
        caller: "solver",
        tool_id: COMPUTE_BLIND,
        call_id: "c-g3-gate",
        args: &leak_op_base,
        canonical_args: "{}",
        effect: policy::EffectClass::ExecuteCode,
        approval_scope: tools::ApprovalScope::Once,
    };
    match gate.before(&req) {
        GateDecision::Block(refusal) => assert_eq!(refusal.gate, "G3"),
        other => panic!("expected G3 block, got {other:?}"),
    }

    // enforce_g3_blind_compute writes `guardrail.blocked` and increments `gate_bounces`.
    let err = enforce_g3_blind_compute(&ledger, sid, "cb-1", &leak_input_exact).unwrap_err();
    assert!(err.starts_with("GATE G3:"), "unexpected err: {err}");
    let evs = ledger.read_session(sid).unwrap();
    assert!(evs.iter().any(|e| {
        e.kind == event_kinds::GUARDRAIL_BLOCKED && e.payload["gate"] == json!("G3")
    }));
    assert_eq!(SolveState::from_events(&evs).gate_bounces, 1);
}
