//! Unit and integration regression tests for Work Package 1 (research-tools).

use ledger::Ledger;
use research_tools::proof_closure::closure_for_target;
use research_tools::proof_dag;
use research_tools::solve_checks::{
    check_run_handler, AutoCheckGate, CheckExecError, CheckExecResult, CheckProcessRunner,
};
use research_tools::solve_defs::STEP_RECORD;
use research_tools::solve_done::evaluate_done_criteria;
use research_tools::solve_exprs::{harvest_from_sympy_result, CanonicalizedExpr, CheckCanonicalizer};
use research_tools::solve_model::{
    compute_entry_sha256, compute_math_sha256, event_kinds, Claim, ClaimForm, ClaimRelation,
    DoneCriterion, DoneCriterionKind, HandleRef, ReceiptRef, ReviewIndependence, ReviewPolicy,
    RoleModel, SetupAssumption, SolveBudgets, SolveRoles, SolveSetup, SolveTarget, StepOrigin,
    SymbolSpec, TargetForm, TaskKind,
};
use research_tools::solve_setup::{prefill_solve_setup, PrefillRequest};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{badge_for_step, claim_sides_identical, wording, Tier};
use research_tools::solve_steps::{run_step_record, step_record_handler};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router, WorkerEpoch};

struct FixedEpoch;
impl WorkerEpoch for FixedEpoch {
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
}

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "test-ok".into(),
        }
    }
}

struct EchoCanon;
impl CheckCanonicalizer for EchoCanon {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let sha = ledger::sha256_hex(raw_srepr.as_bytes());
        Ok(CanonicalizedExpr {
            portable: true,
            srepr: raw_srepr.to_string(),
            srepr_sha256: sha,
            latex: raw_srepr.to_string(),
            str_preview: raw_srepr.to_string(),
            free_symbols: vec![],
            error: None,
            rejected_constructor: None,
        })
    }
}

struct CountingTrivialRunner {
    numeric_calls: AtomicUsize,
}
impl CheckProcessRunner for CountingTrivialRunner {
    fn run_check(
        &self,
        name: &str,
        _handles: &BTreeMap<String, String>,
        _params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        if name == "identity_numeric" {
            self.numeric_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(CheckExecResult {
            raw: json!({
                "outcome": "trivial",
                "registry_sha256": "reg-1",
                "wall_ms": 1,
                "evidence": {
                    "identical_sides": true,
                    "reason": "both sides are structurally identical before simplification; nothing was checked"
                }
            }),
            epoch: 1,
            request_id: 1,
            environment: None,
        })
    }
}

fn sample_setup(done: Vec<DoneCriterion>, same_model: bool, task_kind: TaskKind) -> SolveSetup {
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let assumptions = vec![SetupAssumption { id: "a1".into(), text: "x real".into() }];
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, "");
    let solver = RoleModel { provider: "openai".into(), model: "gpt-5".into() };
    let verifier = if same_model {
        solver.clone()
    } else {
        RoleModel { provider: "anthropic".into(), model: "claude-sonnet-4-6".into() }
    };
    SolveSetup {
        revision: 1,
        statement: "Prove identity".into(),
        task_kind,
        target: SolveTarget {
            description: "Structured proof".into(),
            form: TargetForm::Proof,
            order: None,
            digits: None,
        },
        done,
        conventions: String::new(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles { solver, verifier, compute: None },
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha,
    }
}

#[test]
fn trivial_check_never_triggers_numeric_fallback_and_emits_attention_and_warnings() {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-trivial-gate";
    let setup = sample_setup(vec![], false, TaskKind::Calculate);
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap())
        .unwrap();
    let rseq = {
        ledger
            .append_with_hash(
                sid,
                event_kinds::TOOL_REQUEST,
                1,
                &json!({ "call_id": "c-1", "tool": "sympy_eval", "caller": "solver:solve", "args": {} }),
            )
            .unwrap();
        ledger
            .append_with_hash(
                sid,
                event_kinds::TOOL_RESULT,
                1,
                &json!({ "call_id": "c-1", "ok": true, "result": { "value": "8", "error": null, "process": "solver" } }),
            )
            .unwrap()
            .seq
    };
    let sympy_res = json!({
        "exprs_out": { "a": "Integer(8)", "b": "Integer(8)" },
        "exprs_out_meta": {
            "a": { "source_expr": "Integer(8)" },
            "b": { "source_expr": "Integer(8)" }
        }
    });
    let rep = harvest_from_sympy_result(
        &ledger,
        sid,
        ReceiptRef { call_id: "c-1".into(), result_seq: rseq },
        &sympy_res,
        &EchoCanon,
    )
    .unwrap();
    assert_eq!(rep.harvested.len(), 2);
    assert_eq!(rep.harvested[0].expr.source_expr.as_deref(), Some("Integer(8)"));

    let runner = Arc::new(CountingTrivialRunner { numeric_calls: AtomicUsize::new(0) });
    let mut reg = Registry::new();
    reg.register(
        research_tools::solve_defs::step_record_def(),
        step_record_handler(Arc::clone(&env)),
    );
    reg.register_worker_session(
        research_tools::solve_defs::check_run_def(),
        Arc::new(FixedEpoch),
        check_run_handler(Arc::clone(&env), runner.clone()),
    );
    let router = Router::new(
        reg,
        research_tools::policy_preset::solve_policy(),
        Ledger::open(tmp.path()).unwrap(),
        sid,
        Arc::new(AllowAll),
    )
    .with_gate(Arc::new(AutoCheckGate::new(Arc::clone(&env), runner.clone())));

    let out = router
        .try_call(
            "solver:solve",
            STEP_RECORD,
            &json!({
                "entry_type": "symbolic_computation",
                "summary": "Vacuous equality",
                "details": "8 = 8",
                "tool_call_refs": ["c-1"],
                "claims": [{ "form": "equation", "lhs": "a@1", "rhs": "b@1", "relation": "eq" }]
            }),
        )
        .unwrap();
    let val = match out {
        CallOutcome::Ok(v) => v,
        other => panic!("unexpected outcome: {other:?}"),
    };
    assert_eq!(runner.numeric_calls.load(Ordering::SeqCst), 0, "trivial must NOT invoke identity_numeric");
    assert_eq!(val["own_tier"], "unchecked");
    assert!(val["warnings"].as_array().is_some_and(|w| !w.is_empty()), "expected identical-side warning");
    assert!(val["attention"].as_str().is_some_and(|a| a.contains("trivial")), "expected attention field: {val}");
    assert!(val["own_tier_reason"].as_str().is_some_and(|r| r.contains("both sides identical")), "expected own_tier_reason: {val}");
    assert_eq!(val["auto_checks"][0]["outcome"], "trivial");
    assert!(val["badge"]["sublabels"].as_array().unwrap().iter().any(|s| s == wording::SUBLABEL_TRIVIAL_IDENTICAL));
}

#[test]
fn unevaluated_srepr_distinguishes_sides_even_when_evaluated_hashes_match() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-uneval";
    let rseq = ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": "c-1", "ok": true, "result": { "value": "8", "error": null, "process": "solver" } }),
        )
        .unwrap()
        .seq;
    let sympy_res = json!({
        "exprs_out": { "lhs": "Integer(8)", "rhs": "Integer(8)" },
        "exprs_out_meta": {
            "lhs": { "unevaluated_srepr": "Integral(Add(Mul(Integer(2), Symbol('x')), Integer(-2)), Tuple(Symbol('x'), Integer(0), Integer(4)))", "source_expr": "Integral(2*x - 2, (x, 0, 4))" },
            "rhs": { "source_expr": "Integer(8)" }
        }
    });
    harvest_from_sympy_result(
        &ledger,
        sid,
        ReceiptRef { call_id: "c-1".into(), result_seq: rseq },
        &sympy_res,
        &EchoCanon,
    )
    .unwrap();
    let state = SolveState::from_events(&ledger.read_session(sid).unwrap());
    let claim = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new("lhs", 1).unwrap(),
        rhs: HandleRef::new("rhs", 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    assert!(!claim_sides_identical(&state, &claim), "distinct unevaluated_srepr_sha256 must NOT be flagged identical");
}

#[test]
fn subset_claim_duplicate_is_refused_and_like_for_like_supersede_does_not_stale_replacement() {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-subset-sup";
    let setup = sample_setup(vec![], false, TaskKind::Calculate);
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap())
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({ "call_id": "c-1", "tool": "sympy_eval", "caller": "solver:solve", "args": {} }),
        )
        .unwrap();
    let rseq = ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": "c-1", "ok": true, "result": { "value": "0", "error": null, "process": "solver" } }),
        )
        .unwrap()
        .seq;
    let sympy_res = json!({ "exprs_out": { "l1": "Pow(Add(Symbol('x'), Integer(1)), Integer(2))", "r1": "Add(Pow(Symbol('x'), Integer(2)), Mul(Integer(2), Symbol('x')), Integer(1))", "l2": "Symbol('x')", "r2": "Integer(0)" } });
    harvest_from_sympy_result(&ledger, sid, ReceiptRef { call_id: "c-1".into(), result_seq: rseq }, &sympy_res, &EchoCanon).unwrap();

    let e1 = run_step_record(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "Two claims",
            "details": "Eq + Ge",
            "tool_call_refs": ["c-1"],
            "claims": [
                { "form": "equation", "lhs": "l1@1", "rhs": "r1@1", "relation": "eq" },
                { "form": "inequality", "lhs": "l2@1", "rhs": "r2@1", "relation": "ge" }
            ]
        }),
    )
    .unwrap();
    assert_eq!(e1["entry_id"], "e-1");
    assert!(e1["own_tier_reason"].as_str().is_some_and(|r| r.contains("inequality")));

    // Strict subset claim set must be refused!
    let err = run_step_record(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "Subset claim",
            "details": "Only eq",
            "tool_call_refs": ["c-1"],
            "claims": [{ "form": "equation", "lhs": "l1@1", "rhs": "r1@1", "relation": "eq" }]
        }),
    )
    .unwrap_err();
    assert!(err.contains("e-1") && err.contains("ledger_supersede"), "got: {err}");

    // Like-for-like supersede (`symbolic_computation` -> `symbolic_computation`) is allowed in Solve sessions
    // and replacement step sharing receipt `c-1` does NOT become Stale!
    let e2 = run_step_record(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "Replacement",
            "details": "Distinct claim",
            "tool_call_refs": ["c-1"],
            "claims": [{ "form": "equation", "lhs": "r1@1", "rhs": "l1@1", "relation": "eq" }]
        }),
    )
    .unwrap();
    assert_eq!(e2["entry_id"], "e-2");

    let mut reg = Registry::new();
    proof_dag::register(&mut reg);
    let router = Router::new(
        reg,
        research_tools::policy_preset::solve_policy(),
        Ledger::open(tmp.path()).unwrap(),
        sid,
        Arc::new(AllowAll),
    );
    let sup = router
        .try_call(
            "solver:solve",
            "ledger_supersede",
            &json!({ "entry_id": "e-1", "replaced_by": "e-2", "reason": "split cleanly" }),
        )
        .unwrap();
    assert!(matches!(sup, CallOutcome::Ok(_)), "like-for-like supersede must succeed in Solve: {sup:?}");

    let state = SolveState::from_events(&ledger.read_session(sid).unwrap());
    let t2 = state.tier("e-2").unwrap();
    assert!(!t2.stale && t2.effective != Tier::Stale, "replacement e-2 must not be marked Stale by superseded e-1: {t2:?}");
}

#[test]
fn unchecked_deps_rests_on_line_and_no_open_flags_same_model_and_prefill_description() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-flags-deps";
    let setup = sample_setup(
        vec![
            DoneCriterion {
                id: "final_model_review".into(),
                kind: DoneCriterionKind::FinalTierAtLeast,
                params: json!({ "tier": "model_agreed_same_model", "require_different_model": false }),
            },
            DoneCriterion {
                id: "no_open_flags".into(),
                kind: DoneCriterionKind::NoOpenFlags,
                params: json!({}),
            },
        ],
        true,
        TaskKind::Prove,
    );
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap())
        .unwrap();

    // e-1: assumed by model; e-2: unchecked dependent on e-1 -> rests_on_line names e-1 (assumed by model)!
    let e1_sha = compute_entry_sha256(None, "Model assumption", "Assume A");
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-1", "entry_type": "assumption", "summary": "Model assumption", "details": "Assume A",
        "depends_on": [], "entry_sha256": e1_sha, "origin": StepOrigin::Solver,
    })).unwrap();
    let e2_sha = compute_entry_sha256(None, "Unchecked step", "Follows from e-1");
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-2", "entry_type": "derivation", "summary": "Unchecked step", "details": "Follows from e-1",
        "depends_on": ["e-1"], "entry_sha256": e2_sha, "origin": StepOrigin::Solver,
    })).unwrap();

    // e-3: final_answer depending only on given e-0, accepted by same-model review -> NoOpenFlags met!
    let e0_sha = compute_entry_sha256(None, "Given", "Setup");
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-0", "entry_type": "assumption", "summary": "Given", "details": "Setup",
        "depends_on": [], "entry_sha256": e0_sha, "origin": StepOrigin::Given,
    })).unwrap();
    let e3_sha = compute_entry_sha256(None, "Final", "Done");
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-3", "entry_type": "final_answer", "summary": "Final", "details": "Done",
        "depends_on": ["e-0"], "entry_sha256": e3_sha, "origin": StepOrigin::Solver,
    })).unwrap();
    ledger.append_with_hash(sid, event_kinds::PROOF_VERIFICATION, 1, &json!({
        "target_entry_id": "e-3", "verdict": "accepted", "entry_sha256": e3_sha,
        "independence": ReviewIndependence::SameModel, "checks_cited": [],
    })).unwrap();

    let state = SolveState::from_events(&ledger.read_session(sid).unwrap());
    let b2 = badge_for_step(&state, "e-2").unwrap();
    assert_eq!(b2.own, Tier::Unchecked);
    assert_eq!(b2.unchecked_deps, vec!["e-1"]);
    assert_eq!(b2.rests_on_line.as_deref(), Some("rests on e-1 (assumed by model)"));

    // e-3 keeps FLAG_SAME_MODEL on badge, yet NoOpenFlags criterion is met when setup accepts same-model!
    let b3 = badge_for_step(&state, "e-3").unwrap();
    assert!(b3.flags.iter().any(|f| f == wording::FLAG_SAME_MODEL));
    let closure_e3 = closure_for_target(&state.entries, "e-3").unwrap();
    let outcomes = evaluate_done_criteria(&setup, &state, "e-3", &closure_e3).unwrap();
    assert!(outcomes.iter().all(|c| c.met), "Done should be reachable when setup accepts same-model: {outcomes:?}");

    // Prefill omits "independent model" from target.description when solver == verifier.
    let pre = prefill_solve_setup(&PrefillRequest {
        statement: "Prove theorem X".into(),
        task_kind_override: Some(TaskKind::Prove),
        pinned_conventions: String::new(),
        defaults: None,
        solver: Some(RoleModel { provider: "openai".into(), model: "gpt-5".into() }),
        verifier: Some(RoleModel { provider: "openai".into(), model: "gpt-5".into() }),
        allow_numeric_target: false,
    });
    assert!(!pre.target.description.to_ascii_lowercase().contains("independent model"), "got: {}", pre.target.description);
}
