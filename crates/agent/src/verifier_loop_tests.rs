use super::test_support::*;
use super::*;
use crate::verifier_tools::IsolatedPythonWorker;
use ledger::Event;
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, event_kinds, CheckOutcome, CheckRequester, Claim,
    ClaimForm, ClaimRelation, DoneCriterion, DoneCriterionKind, HandleRef, ReceiptRef,
    ReviewPolicy, SolveCheck, StepEntryType, StepOrigin,
};
use research_tools::solve_status::Tier;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

fn make_ctx<'a>(
    transport: &'a dyn Transport,
    ledger: &'a Ledger,
    env: Arc<ResearchEnv>,
    session: &'a str,
    cfg: &'a SolveVerifierConfig,
    factory: &'a Arc<dyn IsolatedWorkerFactory>,
) -> SolveVerifierRunCtx<'a> {
    SolveVerifierRunCtx {
        transport,
        ledger,
        env,
        session,
        config: cfg,
        approver: Arc::new(AllowAllApprover),
        factory,
        cancel: None,
    }
}

#[test]
fn verifier_loop_rejects_forged_and_cross_step_citations_r3_and_d2_same_model() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-vloop-r13";
    let role = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    let (math_sha, claim_sha) =
        seed_solve_session(&ledger, session, role.clone(), role.clone(), ReviewPolicy::ProseOnly);

    // Add a second claimed step `e-other` with a distinct claim_sha256 and its own passing check.
    let srepr_b = "Symbol('y')";
    let sha_b = ledger.put_blob(srepr_b.as_bytes()).unwrap();
    let claim_b = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::parse("lhs_h@1").unwrap(),
        rhs: HandleRef::parse("rhs_h@1").unwrap(),
        relation: ClaimRelation::Le,
        order: None,
        domain: None,
        how: None,
    };
    let claim_b_sha =
        compute_claim_sha256(std::slice::from_ref(&claim_b), |_| Some(sha_b.as_str())).unwrap();
    let other_sha = compute_entry_sha256(Some(&claim_b_sha), "Other", "Other details");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-other",
                "entry_type": StepEntryType::Derivation.as_str(),
                "summary": "Other",
                "details": "Other details",
                "depends_on": [],
                "claims": [claim_b],
                "claim_sha256": claim_b_sha,
                "entry_sha256": other_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();

    for (cid, csha, msha, eid) in [
        ("chk-good", claim_sha.as_str(), math_sha.as_str(), "e-1"),
        ("chk-stale", claim_sha.as_str(), "old-math", "e-1"),
        ("chk-other", claim_b_sha.as_str(), math_sha.as_str(), "e-other"),
    ] {
        let chk = SolveCheck {
            check_id: cid.into(),
            claim_sha256: csha.into(),
            claim_index: 0,
            entry_id: Some(eid.into()),
            math_sha256: msha.into(),
            name: "identity_symbolic".into(),
            registry_sha256: "reg".into(),
            params: json!({}),
            outcome: CheckOutcome::Pass,
            asked_by: CheckRequester::Host,
            receipt: ReceiptRef {
                call_id: "rc-1".into(),
                result_seq: 99,
            },
            evidence: json!({ "comparison": "exact", "exact": true }),
            evidence_sha256: "ev-sha".into(),
        };
        ledger
            .append_with_hash(
                session,
                event_kinds::SOLVE_CHECK,
                1,
                &serde_json::to_value(&chk).unwrap(),
            )
            .unwrap();
    }

    // Turns 1..6 test forged check_id, stale math_sha, cross-step check_id, R3 independence smuggling,
    // accepted+counterexample rejection, accepted+blocker rejection; Turn 7 succeeds.
    let transport = ScriptedTransport::new(vec![
        tool_use_resp(
            "t1",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "ok", "checks_cited": ["chk-forged"] }] }),
        ),
        tool_use_resp(
            "t2",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "ok", "checks_cited": ["chk-stale"] }] }),
        ),
        tool_use_resp(
            "t3",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "ok", "checks_cited": ["chk-other"] }] }),
        ),
        tool_use_resp(
            "t4",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "ok", "independence": "different_provider" }] }),
        ),
        tool_use_resp(
            "t5",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "ok", "counterexample": "x = -1" }] }),
        ),
        tool_use_resp(
            "t6",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "ok", "issues": [{ "severity": "blocker", "message": "broken" }] }] }),
        ),
        tool_use_resp(
            "t7",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-1", "verdict": "accepted", "summary": "Valid identity.", "checks_cited": ["chk-good"] }] }),
        ),
    ]);

    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: false,
        spawns: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = SolveVerifierConfig::new(role);
    let ctx = make_ctx(&transport, &ledger, env, session, &cfg, &factory);

    let outcome = run_solve_verifier_loop(&ctx, &["e-1".into()]).unwrap();
    match outcome {
        SolveReviewOutcome::Completed {
            verdicts,
            independence,
            model_calls_used,
            worker_spawned,
            ..
        } => {
            assert_eq!(independence, ReviewIndependence::SameModel);
            assert_eq!(model_calls_used, 7);
            assert!(!worker_spawned);
            assert_eq!(verdicts.len(), 1);
            assert_eq!(verdicts[0].checks_cited, vec!["chk-good"]);
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    let evs = ledger.read_session(session).unwrap();
    let reqs: Vec<&Event> = evs.iter().filter(|e| e.kind == "verifier.request").collect();
    assert_eq!(reqs.len(), 7);
    assert!(reqs.iter().all(|r| r.payload["caller"] == json!(SOLVE_VERIFIER_CALLER)));
}

#[test]
fn verifier_loop_exhausting_12_calls_writes_verifier_incomplete_and_leaves_step_unchecked() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-vloop-cap";
    let solver = RoleModel {
        provider: "anthropic".into(),
        model: "claude-opus-4-1".into(),
    };
    let verifier = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    seed_solve_session(&ledger, session, solver, verifier.clone(), ReviewPolicy::ProseOnly);

    let resps = (0..12)
        .map(|i| {
            tool_use_resp(
                &format!("t{i}"),
                REPORT_VERDICT,
                json!({ "verdicts": [{ "entry_id": "e-2", "verdict": "accepted", "summary": "ok", "checks_cited": ["chk-nope"] }] }),
            )
        })
        .collect();
    let transport = ScriptedTransport::new(resps);
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: false,
        spawns: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = SolveVerifierConfig::new(verifier);
    let ctx = make_ctx(&transport, &ledger, env, session, &cfg, &factory);

    let outcome = run_solve_verifier_loop(&ctx, &["e-2".into()]).unwrap();
    match outcome {
        SolveReviewOutcome::Incomplete {
            model_calls_used,
            independence,
            ..
        } => {
            assert_eq!(model_calls_used, 12);
            assert_eq!(independence, ReviewIndependence::DifferentProvider);
        }
        other => panic!("expected Incomplete, got {other:?}"),
    }

    let evs = ledger.read_session(session).unwrap();
    assert!(evs.iter().any(|e| e.kind == EVENT_VERIFIER_INCOMPLETE));
    assert!(!evs.iter().any(|e| e.kind == event_kinds::PROOF_VERIFICATION));
    assert_eq!(SolveState::from_events(&evs).tier("e-2").unwrap().own, Tier::Unchecked);
}

#[test]
fn real_python_worker_isolation_hides_solver_variables_and_does_not_poison_gate_g1() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-vloop-iso";
    let solver = RoleModel {
        provider: "anthropic".into(),
        model: "claude-opus-4-1".into(),
    };
    let verifier = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    seed_solve_session(&ledger, session, solver, verifier.clone(), ReviewPolicy::ProseOnly);

    let solver_worker = RealStdioPythonWorker::spawn().unwrap();
    let solver_out = solver_worker
        .evaluate("solver_only_secret = 777\nsolver_only_secret", None, Duration::from_secs(10), None)
        .unwrap();
    assert_eq!(solver_out.result["value"], json!("777"));

    // Run 4 verifier sympy_eval calls (first probes solver_only_secret) then accept.
    let transport = ScriptedTransport::new(vec![
        tool_use_resp("t1", "sympy_eval", json!({ "code": "solver_only_secret" })),
        tool_use_resp("t2", "sympy_eval", json!({ "code": "2 + 2" })),
        tool_use_resp("t3", "sympy_eval", json!({ "code": "3 + 3" })),
        tool_use_resp("t4", "sympy_eval", json!({ "code": "4 + 4" })),
        tool_use_resp(
            "t5",
            REPORT_VERDICT,
            json!({ "verdicts": [{ "entry_id": "e-2", "verdict": "accepted", "summary": "Isolated worker verified." }] }),
        ),
    ]);
    let spawns = Arc::new(AtomicUsize::new(0));
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: true,
        spawns: Arc::clone(&spawns),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = SolveVerifierConfig::new(verifier);
    let ctx = make_ctx(&transport, &ledger, env, session, &cfg, &factory);

    let outcome = run_solve_verifier_loop(&ctx, &["e-2".into()]).unwrap();
    assert!(matches!(
        outcome,
        SolveReviewOutcome::Completed { worker_spawned: true, .. }
    ));
    assert_eq!(spawns.load(Ordering::SeqCst), 1);

    let reqs = transport.seen_requests();
    let tool_result_text = reqs[1]
        .messages
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .unwrap();
    assert!(
        tool_result_text.contains("NameError") && tool_result_text.contains("solver_only_secret"),
        "expected isolated NameError, got: {tool_result_text}"
    );

    // 4 verifier sympy_eval calls must NOT populate solver pending_runs (Gate G1 non-poisoning).
    let folded = SolveState::from_events(&ledger.read_session(session).unwrap());
    assert!(
        folded.pending_runs.is_empty(),
        "verifier sympy_eval must not poison G1 pending_runs: {:?}",
        folded.pending_runs
    );
}

#[test]
fn g6_verification_stuck_deepest_first_clipping_and_mid_review_mutation_guard() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-vloop-g6";
    let solver = RoleModel {
        provider: "anthropic".into(),
        model: "claude-opus-4-1".into(),
    };
    let verifier = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    seed_solve_session(&ledger, session, solver, verifier.clone(), ReviewPolicy::ProseOnly);

    let e3_sha = compute_entry_sha256(None, "Revised prose", "Revised body");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-3",
                "entry_type": StepEntryType::FinalAnswer.as_str(),
                "summary": "Revised prose",
                "details": "Revised body",
                "depends_on": ["e-1"],
                "entry_sha256": e3_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();

    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: false,
        spawns: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = SolveVerifierConfig::new(verifier);

    let t1 = ScriptedTransport::new(vec![tool_use_resp(
        "r1",
        REPORT_VERDICT,
        json!({ "verdicts": [{ "entry_id": "e-2", "verdict": "rejected", "summary": "Missing sign check.", "issues": ["sign"] }] }),
    )]);
    let out1 =
        run_solve_verifier_loop(&make_ctx(&t1, &ledger, Arc::clone(&env), session, &cfg, &factory), &["e-2".into()])
            .unwrap();
    assert!(matches!(out1, SolveReviewOutcome::Completed { verification_stuck: None, .. }));

    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_SUPERSEDE,
            1,
            &json!({ "entry_id": "e-2", "replaced_by": "e-3", "reason": "retry" }),
        )
        .unwrap();

    let t2 = ScriptedTransport::new(vec![tool_use_resp(
        "r2",
        REPORT_VERDICT,
        json!({ "verdicts": [{ "entry_id": "e-3", "verdict": "rejected", "summary": "Still missing sign check.", "issues": ["sign"] }] }),
    )]);
    let out2 =
        run_solve_verifier_loop(&make_ctx(&t2, &ledger, Arc::clone(&env), session, &cfg, &factory), &["e-3".into()])
            .unwrap();
    match out2 {
        SolveReviewOutcome::Completed {
            verification_stuck: Some(stuck),
            ..
        } => assert_eq!(stuck["reason"], json!("verification_stuck")),
        other => panic!("expected G6 verification_stuck, got {other:?}"),
    }

    // Deepest-first clipping: depth-1 ancestor clipped while depth-0 target stays intact.
    let evs = ledger.read_session(session).unwrap();
    let mut state = SolveState::from_events(&evs);
    let setup = state.setup.clone().unwrap();
    state.entries[0].details = "A".repeat(2000);
    state.entries[2].details = "TARGET_KEEP".into();
    let closure = closure_entries(&state.entries, &["e-3"]);
    let prompt = build_solve_review_prompt(&setup, &state, &["e-3".into()], &closure, 1800);
    let parsed: Value = serde_json::from_str(&prompt).unwrap();
    let steps = parsed["closure_steps"].as_array().unwrap();
    assert_eq!(steps.iter().find(|s| s["entry_id"] == "e-1").unwrap()["details_clipped"], json!(true));
    let e3_json = steps.iter().find(|s| s["entry_id"] == "e-3").unwrap();
    assert_eq!(e3_json["details"], json!("TARGET_KEEP"));
    assert_eq!(e3_json["details_clipped"], json!(false));

    // Mid-review mutation guard: if target sha mismatches current state, validate_report_verdict rejects.
    let mut stale_shas = BTreeMap::new();
    stale_shas.insert("e-3".to_string(), "stale-digest".to_string());
    let err = super::support::validate_report_verdict(
        &state,
        &["e-3".to_string()],
        &stale_shas,
        &json!({ "verdicts": [{ "entry_id": "e-3", "verdict": "accepted", "summary": "ok" }] }),
    )
    .unwrap_err();
    assert!(err.contains("changed during review"), "unexpected err: {err}");
}

#[test]
fn finish_review_policies_batching_assumed_skip_and_loop_caps() {
    let dir = TempDir::new().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let session = "s-vloop-finish";
    let solver = RoleModel {
        provider: "anthropic".into(),
        model: "claude-opus-4-1".into(),
    };
    let verifier = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    seed_solve_session(&ledger, session, solver, verifier.clone(), ReviewPolicy::ProseOnly);

    // Add 6 prose steps (e-p1..e-p6) plus an AssumedByModel step (e-asm) feeding into e-final.
    let mut deps = vec!["e-1".to_string()];
    for i in 1..=6 {
        let id = format!("e-p{i}");
        let sha = compute_entry_sha256(None, &format!("Prose {i}"), "body");
        ledger
            .append_with_hash(
                session,
                event_kinds::PROOF_ENTRY,
                1,
                &json!({
                    "entry_id": id,
                    "entry_type": StepEntryType::Derivation.as_str(),
                    "summary": format!("Prose {i}"),
                    "details": "body",
                    "depends_on": deps,
                    "entry_sha256": sha,
                    "origin": StepOrigin::Solver,
                }),
            )
            .unwrap();
        deps = vec![id];
    }
    let asm_sha = compute_entry_sha256(None, "Model assumption", "assumed");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-asm",
                "entry_type": StepEntryType::Assumption.as_str(),
                "summary": "Model assumption",
                "details": "assumed",
                "depends_on": deps,
                "entry_sha256": asm_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();
    let fin_sha = compute_entry_sha256(None, "Final", "final body");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-final",
                "entry_type": StepEntryType::FinalAnswer.as_str(),
                "summary": "Final",
                "details": "final body",
                "depends_on": ["e-asm"],
                "entry_sha256": fin_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();

    let evs = ledger.read_session(session).unwrap();
    let mut st = SolveState::from_events(&evs);

    // WP1 item 9: ProseOnly skips claim-bearing steps ONLY when own.is_computed();
    // unchecked `e-1` is selected alongside the 7 prose steps (8 total -> 2 batches: 5 + 3),
    // while `e-asm` (AssumedByModel) is still skipped.
    let prose_targets = select_finish_review_targets(&st, "e-final");
    assert_eq!(prose_targets.len(), 8);
    assert!(prose_targets.contains(&"e-1".to_string()));
    assert!(!prose_targets.contains(&"e-asm".to_string()));
    let chunks = batch_review_targets(&prose_targets);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 5);
    assert_eq!(chunks[1].len(), 3);

    // All includes claimed `e-1` as well (8 total), still skipping `e-asm`.
    st.setup.as_mut().unwrap().review = ReviewPolicy::All;
    let all_targets = select_finish_review_targets(&st, "e-final");
    assert_eq!(all_targets.len(), 8);
    assert!(all_targets.contains(&"e-1".to_string()));
    assert!(!all_targets.contains(&"e-asm".to_string()));

    // FinalOnly selects only `e-final`, respecting FinalTierAtLeast.
    st.setup.as_mut().unwrap().review = ReviewPolicy::FinalOnly;
    st.setup.as_mut().unwrap().done = vec![DoneCriterion {
        id: "f1".into(),
        kind: DoneCriterionKind::FinalTierAtLeast,
        params: json!({ "tier": "verified" }),
    }];
    assert_eq!(select_finish_review_targets(&st, "e-final"), vec!["e-final".to_string()]);

    // Off selects nothing.
    st.setup.as_mut().unwrap().review = ReviewPolicy::Off;
    assert!(select_finish_review_targets(&st, "e-final").is_empty());

    // Cap max_review_loops = 1: run_finish_review_batches runs 1 batch of 5, skips 2, and handle_review_steps refuses next batch.
    let mut capped_setup = SolveState::from_events(&evs).setup.unwrap();
    capped_setup.revision = 2;
    capped_setup.budgets.max_review_loops = 1;
    ledger
        .append_with_hash(
            session,
            event_kinds::SOLVE_SETUP,
            1,
            &serde_json::to_value(&capped_setup).unwrap(),
        )
        .unwrap();

    let first5: Vec<Value> = chunks[0]
        .iter()
        .map(|id| json!({ "entry_id": id, "verdict": "accepted", "summary": "ok" }))
        .collect();
    let transport = ScriptedTransport::new(vec![tool_use_resp(
        "b1",
        REPORT_VERDICT,
        json!({ "verdicts": first5 }),
    )]);
    let factory: Arc<dyn IsolatedWorkerFactory> = Arc::new(TestFactory {
        use_real_python: false,
        spawns: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let env = Arc::new(ResearchEnv::new(dir.path().to_path_buf()));
    let cfg = SolveVerifierConfig::new(verifier);
    let ctx = make_ctx(&transport, &ledger, env, session, &cfg, &factory);

    let summary = run_finish_review_batches(&ctx, "e-final").unwrap();
    assert_eq!(summary.batches_run, 1);
    assert_eq!(summary.skipped_due_to_cap, 3);

    // Distinct batches with identical raw payloads must count separately via `batch_id`.
    let cap_err = handle_review_steps(&ctx, &json!({ "entry_ids": ["e-final"] })).unwrap_err();
    assert!(cap_err.contains("review loop cap reached"), "unexpected err: {cap_err}");

    let dup_a = Event {
        session: session.into(),
        seq: 100,
        ts_ms: 0,
        kind: event_kinds::PROOF_VERIFICATION.into(),
        schema_version: 1,
        payload: json!({ "kind": SOLVE_VERIFIER_KIND, "batch_id": "rv-10", "raw_blob": "same-sha" }),
        prev_hash: String::new(),
        event_hash: String::new(),
    };
    let dup_b = Event {
        seq: 101,
        payload: json!({ "kind": SOLVE_VERIFIER_KIND, "batch_id": "rv-20", "raw_blob": "same-sha" }),
        ..dup_a.clone()
    };
    assert_eq!(count_review_loops(&[dup_a, dup_b]), 2);
}
