use super::*;
use crate::verifier_loop::test_support::{
    RealStdioPythonWorker, ScriptedTransport, TestFactory,
};
use crate::verifier_tools::IsolatedPythonWorker;
use research_tools::run_budget::{count_model_calls_breakdown, RunScope};
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, Claim, ClaimForm,
    ClaimRelation, DoneCriterion, DoneCriterionKind, HandleRef, ReviewPolicy, SolveBudgets,
    SolveExpr, SolveRoles, SolveSetup, SolveTarget, StepEntryType, StepOrigin, SymbolSpec,
    TargetForm, TaskKind,
};
use research_tools::solve_status::Tier;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

fn tool_use_blind(id: &str, code: &str) -> crate::transport::ModelResponse {
    crate::verifier_loop::test_support::tool_use_resp(id, "sympy_eval", json!({ "code": code }))
}

fn seed_blind_session(ledger: &Ledger, session: &str) -> String {
    ledger
        .append_with_hash(session, event_kinds::SOLVE_STARTED, 1, &json!({}))
        .unwrap();
    let symbols = vec![SymbolSpec {
        name: "x".into(),
        assumptions: vec!["real".into()],
    }];
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &[], "Euclidean");
    let role = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    let setup = SolveSetup {
        revision: 1,
        statement: "Compute expansions and values".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "exact expression".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: vec![DoneCriterion {
            id: "c1".into(),
            kind: DoneCriterionKind::FinalTierAtLeast,
            params: json!({ "tier": "verified" }),
        }],
        conventions: "Euclidean".into(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions: vec![],
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: role.clone(),
            verifier: role,
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha.clone(),
    };
    ledger
        .append_with_hash(
            session,
            event_kinds::SOLVE_SETUP,
            1,
            &serde_json::to_value(&setup).unwrap(),
        )
        .unwrap();
    math_sha
}

fn store_expr(
    ledger: &Ledger,
    session: &str,
    name: &str,
    srepr: &str,
    latex: &str,
    preview: &str,
    owner: &str,
) -> (HandleRef, String) {
    let sha = ledger.put_blob(srepr.as_bytes()).unwrap();
    let expr = SolveExpr {
        name: name.into(),
        version: 1,
        srepr_blob: sha.clone(),
        srepr_sha256: sha.clone(),
        latex: latex.into(),
        str_preview: preview.into(),
        free_symbols: vec![],
        produced_by: ReceiptRef {
            call_id: format!("seed-{name}"),
            result_seq: 1,
        },
        owner_entry: Some(owner.into()),
        portable: true,
        unevaluated_srepr: None, unevaluated_srepr_sha256: None, source_expr: None,
    };
    ledger
        .append_with_hash(
            session,
            event_kinds::SOLVE_EXPR,
            1,
            &serde_json::to_value(&expr).unwrap(),
        )
        .unwrap();
    (HandleRef::parse(&format!("{name}@1")).unwrap(), sha)
}

#[allow(clippy::too_many_arguments)]
fn store_claim_step(
    ledger: &Ledger,
    session: &str,
    entry_id: &str,
    form: ClaimForm,
    lhs: HandleRef,
    lhs_sha: &str,
    rhs: HandleRef,
    rhs_sha: &str,
) {
    let claim = Claim {
        form,
        lhs: lhs.clone(),
        rhs: rhs.clone(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let claim_sha = compute_claim_sha256(std::slice::from_ref(&claim), |h| {
        if *h == lhs {
            Some(lhs_sha)
        } else if *h == rhs {
            Some(rhs_sha)
        } else {
            None
        }
    })
    .unwrap();
    let entry_sha = compute_entry_sha256(Some(&claim_sha), "Claim step", "Details");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": entry_id,
                "entry_type": StepEntryType::Derivation.as_str(),
                "summary": "Claim step",
                "details": "Details",
                "depends_on": [],
                "claims": [claim],
                "claim_sha256": claim_sha,
                "entry_sha256": entry_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();
}

#[test]
fn compute_blind_exact_symbolic_match_yields_verified_and_isolates_compare_with() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-blind-exact";
    seed_blind_session(&ledger, session);

    // Separate solver Python worker defines a secret variable.
    let solver_worker = RealStdioPythonWorker::spawn().unwrap();
    solver_worker
        .evaluate("blind_forbidden_var = 999", None, Duration::from_secs(10), None)
        .unwrap();

    // Seed input `(x + 1)**3` and solver claimed answer `x**3 + 3*x**2 + 3*x + 1`.
    let (h_in, sha_in) = store_expr(
        &ledger,
        session,
        "poly_in",
        "Pow(Add(Symbol('x', real=True), Integer(1)), Integer(3))",
        "(x + 1)^3",
        "(x + 1)**3",
        "e-1",
    );
    let (h_ans, sha_ans) = store_expr(
        &ledger,
        session,
        "secret_target_ans",
        "Add(Pow(Symbol('x', real=True), Integer(3)), Mul(Integer(3), Pow(Symbol('x', real=True), Integer(2))), Mul(Integer(3), Symbol('x', real=True)), Integer(1))",
        "x^3 + 3 x^2 + 3 x + 1",
        "x**3 + 3*x**2 + 3*x + 1",
        "e-1",
    );
    store_claim_step(
        &ledger,
        session,
        "e-1",
        ClaimForm::Equation,
        h_in,
        &sha_in,
        h_ans,
        &sha_ans,
    );

    // Turn 1: blind worker tries to read `secret_target_ans` & `blind_forbidden_var` -> both must fail!
    // Turn 2: blind worker honestly expands `expr("poly_in")` and calls `save_expr("blind_result", ...)`.
    let transport = ScriptedTransport::new(vec![
        tool_use_blind(
            "b1",
            "try:\n    expr('secret_target_ans@1')\n    leaked = True\nexcept KeyError:\n    leaked = False\nassert not leaked\nblind_forbidden_var",
        ),
        tool_use_blind(
            "b2",
            "import sympy as sp\nout = sp.expand(expr('poly_in'))\nsave_expr('blind_result', out)\nout",
        ),
    ]);
    let spawns = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: true,
        spawns: Arc::clone(&spawns),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = BlindComputeConfig::new(RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1-mini".into(),
    });
    let ctx = BlindComputeRunCtx {
        transport: &transport,
        ledger: &ledger,
        env,
        session,
        config: &cfg,
        factory: &factory,
        cancel: None,
    };

    let res_val = handle_compute_blind(
        &ctx,
        "cb-exact",
        &json!({
            "inputs": ["poly_in@1"],
            "operation": "Expand poly_in completely in powers of x.",
            "compare_with": "secret_target_ans@1"
        }),
    )
    .unwrap();
    let res: BlindComputeResult = serde_json::from_value(res_val).unwrap();
    assert!(res.agreed);
    assert_eq!(res.outcome, CheckOutcome::Pass);
    assert_eq!(res.comparison.as_deref(), Some("exact"));
    assert!(res.exact);
    assert_eq!(res.output, "blind_result@1");
    assert_eq!(res.model_calls_used, 2);
    assert_eq!(spawns.load(Ordering::SeqCst), 1);

    // Blind model prompt must NEVER contain `secret_target_ans` or its LaTeX/preview.
    let reqs = transport.seen_requests();
    let prompt_json = serde_json::to_string(&reqs[0]).unwrap();
    assert!(!prompt_json.contains("secret_target_ans"), "compare_with leaked into blind prompt!");
    assert!(!prompt_json.contains("3*x**2"), "compare_with preview leaked into blind prompt!");

    // Turn 1 tool result confirmed `blind_forbidden_var` is NameError (isolated process).
    let turn1_err = reqs[1]
        .messages
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .unwrap();
    assert!(
        turn1_err.contains("NameError") && turn1_err.contains("blind_forbidden_var"),
        "expected isolated NameError, got: {turn1_err}"
    );

    // Fold ledger: step `e-1` must reach Tier::Verified, F9 delegated == 2, G1 pending_runs empty!
    let evs = ledger.read_session(session).unwrap();
    let folded = SolveState::from_events(&evs);
    let tier = folded.tier("e-1").unwrap();
    assert_eq!(tier.own, Tier::Verified);
    assert_eq!(tier.effective, Tier::Verified);
    assert!(tier.sublabels.iter().any(|s| s == "independent recompute"));
    assert!(folded.pending_runs.is_empty());
    let mc = count_model_calls_breakdown(&evs, RunScope::All);
    assert_eq!(mc.delegated, 2);
}

#[test]
fn compute_blind_numeric_fallback_yields_checked_numerically_never_verified() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-blind-numeric";
    seed_blind_session(&ledger, session);

    // Seed Value claim: `lhs` = `pi` (exact), `rhs` = `pi_approx` (35-digit Float close to pi).
    // Symbolic diff `pi - Float(...)` is non-zero, so `identity_symbolic` is inconclusive with
    // tolerance 1e-25, then `identity_numeric` passes -> Tier::CheckedNumerically (NEVER Verified!).
    let (h_lhs, sha_lhs) = store_expr(
        &ledger,
        session,
        "pi_sym",
        "pi",
        "\\pi",
        "pi",
        "e-num",
    );
    let (h_rhs, sha_rhs) = store_expr(
        &ledger,
        session,
        "pi_approx",
        "Float('3.14159265358979323846264338327950288', precision=120)",
        "3.14159265358979323846264338327950288",
        "3.14159265358979323846264338327950288",
        "e-num",
    );
    store_claim_step(
        &ledger,
        session,
        "e-num",
        ClaimForm::Value,
        h_lhs,
        &sha_lhs,
        h_rhs,
        &sha_rhs,
    );

    // Blind worker computes exact `sp.pi` (symbolic difference vs 35-digit Float is non-zero,
    // numeric difference < 1e-25).
    let transport = ScriptedTransport::new(vec![tool_use_blind(
        "n1",
        "import sympy as sp\nsave_expr('blind_result', sp.pi)\nsp.pi",
    )]);
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: true,
        spawns: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = BlindComputeConfig::new(RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1-mini".into(),
    });
    let ctx = BlindComputeRunCtx {
        transport: &transport,
        ledger: &ledger,
        env,
        session,
        config: &cfg,
        factory: &factory,
        cancel: None,
    };

    let res = run_compute_blind(
        &ctx,
        "cb-num",
        &json!({
            "inputs": [],
            "operation": "Compute the constant pi symbolically.",
            "compare_with": "pi_approx@1",
            "tolerance": 1e-25,
            "entry_id": "e-num"
        }),
    )
    .unwrap();
    assert!(res.agreed);
    assert_eq!(res.outcome, CheckOutcome::Pass);
    assert_eq!(res.comparison.as_deref(), Some("numeric"));
    assert!(!res.exact);
    assert!(res.evidence.get("numeric_coefficients").is_none());

    let evs = ledger.read_session(session).unwrap();
    let folded = SolveState::from_events(&evs);
    let tier = folded.tier("e-num").unwrap();
    assert_eq!(tier.own, Tier::CheckedNumerically);
    assert_ne!(tier.own, Tier::Verified);
}

#[test]
fn compute_blind_mismatch_fails_step_and_g3_refusal_spends_zero_calls_and_workers() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-blind-fail-g3";
    seed_blind_session(&ledger, session);

    let (h_in, sha_in) = store_expr(
        &ledger,
        session,
        "x_in",
        "Symbol('x', real=True)",
        "x",
        "x",
        "e-bad",
    );
    let (h_claim, sha_claim) = store_expr(
        &ledger,
        session,
        "claimed_sq",
        "Add(Pow(Symbol('x', real=True), Integer(2)), Integer(1))",
        "x^2 + 1",
        "x**2 + 1",
        "e-bad",
    );
    store_claim_step(
        &ledger,
        session,
        "e-bad",
        ClaimForm::Equation,
        h_in,
        &sha_in,
        h_claim,
        &sha_claim,
    );

    // 1. G3 refusal must block BEFORE any model call or worker spawn.
    let zero_transport = ScriptedTransport::new(vec![]);
    let spawns = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: true,
        spawns: Arc::clone(&spawns),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = BlindComputeConfig::new(RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1-mini".into(),
    });
    let g3_ctx = BlindComputeRunCtx {
        transport: &zero_transport,
        ledger: &ledger,
        env: Arc::clone(&env),
        session,
        config: &cfg,
        factory: &factory,
        cancel: None,
    };
    let g3_err = run_compute_blind(
        &g3_ctx,
        "cb-g3",
        &json!({
            "inputs": ["x_in@1", "claimed_sq@1"],
            "operation": "Square x_in and add 1.",
            "compare_with": "claimed_sq@1"
        }),
    )
    .unwrap_err();
    assert!(g3_err.starts_with("GATE G3:"), "unexpected G3 err: {g3_err}");
    assert!(zero_transport.seen_requests().is_empty());
    assert_eq!(spawns.load(Ordering::SeqCst), 0);

    // 2. Disagreement: blind worker returns `x**2 + 2` vs claimed `x**2 + 1` -> CheckOutcome::Fail -> Tier::Failed.
    let fail_transport = ScriptedTransport::new(vec![tool_use_blind(
        "f1",
        "x = expr('x_in')\nsave_expr('blind_result', x**2 + 2)\nx**2 + 2",
    )]);
    let fail_ctx = BlindComputeRunCtx {
        transport: &fail_transport,
        ledger: &ledger,
        env,
        session,
        config: &cfg,
        factory: &factory,
        cancel: None,
    };
    let res = run_compute_blind(
        &fail_ctx,
        "cb-fail",
        &json!({
            "inputs": ["x_in@1"],
            "operation": "Compute x_in**2 + 2.",
            "compare_with": "claimed_sq@1"
        }),
    )
    .unwrap();
    assert!(!res.agreed);
    assert_eq!(res.outcome, CheckOutcome::Fail);

    let evs = ledger.read_session(session).unwrap();
    let folded = SolveState::from_events(&evs);
    assert_eq!(folded.tier("e-bad").unwrap().own, Tier::Failed);
}

#[test]
fn compute_blind_hardening_rejects_forged_claim_binding_loose_tol_and_handles_cancel_drop() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-blind-hard";
    seed_blind_session(&ledger, session);

    let (h_a, sha_a) = store_expr(&ledger, session, "wrong_a", "Integer(1)", "1", "1", "e-false");
    let (h_b, sha_b) = store_expr(&ledger, session, "wrong_b", "Integer(2)", "2", "2", "e-false");
    let (_h_z, _sha_z) = store_expr(&ledger, session, "zero_h", "Integer(0)", "0", "0", "e-other");
    store_claim_step(&ledger, session, "e-false", ClaimForm::Equation, h_a, &sha_a, h_b, &sha_b);

    let spawns = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: false,
        spawns: Arc::clone(&spawns),
        dropped: Arc::clone(&dropped),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = BlindComputeConfig::new(RoleModel { provider: "openai".into(), model: "gpt-4.1".into() });
    let empty_t = ScriptedTransport::new(vec![]);
    let ctx = BlindComputeRunCtx {
        transport: &empty_t, ledger: &ledger, env: Arc::clone(&env), session, config: &cfg, factory: &factory, cancel: None,
    };

    // 1. Forged claim binding: explicit `entry_id` with unrelated `compare_with` MUST be rejected (with or without claim_index).
    for extra in [json!({ "entry_id": "e-false" }), json!({ "entry_id": "e-false", "claim_index": 0 })] {
        let mut args = json!({ "inputs": [], "operation": "Compute 0.", "compare_with": "zero_h@1" });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let err = run_compute_blind(&ctx, "cb-forge", &args).unwrap_err();
        assert!(err.contains("does not reference") || err.contains("no claim referencing"), "got: {err}");
    }

    // 2. Unbounded tolerance, points, and digits MUST be rejected.
    for bad in [
        json!({ "inputs": [], "operation": "Compute 1.", "compare_with": "wrong_a@1", "tolerance": 1e6 }),
        json!({ "inputs": [], "operation": "Compute 1.", "compare_with": "wrong_a@1", "points": 2 }),
        json!({ "inputs": [], "operation": "Compute 1.", "compare_with": "wrong_a@1", "digits": 10 }),
    ] {
        assert!(run_compute_blind(&ctx, "cb-bounds", &bad).is_err(), "expected error for {bad}");
    }
    assert_eq!(spawns.load(Ordering::SeqCst), 0);

    // 3. Pre-cancelled token refuses BEFORE spawning worker.
    let cancel = CancelToken::new();
    cancel.cancel();
    let cancel_ctx = BlindComputeRunCtx { cancel: Some(&cancel), ..ctx };
    let cerr = run_compute_blind(
        &cancel_ctx,
        "cb-cancel",
        &json!({ "inputs": [], "operation": "Compute 1.", "compare_with": "wrong_a@1", "entry_id": "e-false" }),
    )
    .unwrap_err();
    assert!(cerr.contains("cancelled"), "got: {cerr}");
    assert_eq!(spawns.load(Ordering::SeqCst), 0);

    // 4. Transport exhaustion drops worker AND records delegate.end with status "error".
    let err_ctx = BlindComputeRunCtx {
        transport: &empty_t, ledger: &ledger, env, session, config: &cfg, factory: &factory, cancel: None,
    };
    assert!(run_compute_blind(
        &err_ctx,
        "cb-err-drop",
        &json!({ "inputs": [], "operation": "Compute 1.", "compare_with": "wrong_a@1", "entry_id": "e-false" }),
    )
    .is_err());
    assert_eq!(spawns.load(Ordering::SeqCst), 1);
    assert!(dropped.load(Ordering::SeqCst), "isolated worker must be dropped on error");
    let evs = ledger.read_session(session).unwrap();
    assert!(evs.iter().any(|e| {
        e.kind == "delegate.end"
            && e.payload["call_id"] == json!("cb-err-drop")
            && e.payload["status"] == json!("error")
    }));
}
