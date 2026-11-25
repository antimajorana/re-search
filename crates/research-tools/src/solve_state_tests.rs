use super::*;
use crate::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, ClaimForm, ClaimRelation,
    ReceiptRef, RoleModel, SetupAssumption, SolveBudgets, SolveRoles, SolveTarget, SymbolSpec,
    TargetForm, TaskKind,
};
use crate::solve_status::Tier;
use serde_json::json;

fn mk_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "sess-test".into(),
        seq,
        ts_ms: (seq as i64) * 1000,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: "0".repeat(64),
        event_hash: "a".repeat(64),
    }
}

fn sample_setup(rev: u32, convention: &str) -> SolveSetup {
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let assumptions = vec![SetupAssumption { id: "a1".into(), text: "x > 0".into() }];
    let math_sha256 = compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, convention);
    SolveSetup {
        revision: rev,
        statement: "Compute f(x)".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "f(x)".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: vec![],
        conventions: convention.into(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "anthropic".into(), model: "solver-m".into() },
            verifier: RoleModel { provider: "openai".into(), model: "verifier-m".into() },
            compute: None,
            formal: None,
        },
        budgets: SolveBudgets::default(),
        review: crate::solve_model::ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256,
    }
}

fn rich_session_events() -> Vec<Event> {
    let setup = sample_setup(1, "hbar = 1");
    let math_sha = setup.math_sha256.clone();

    let lhs = HandleRef::parse("lhs@1").unwrap();
    let rhs = HandleRef::parse("rhs@1").unwrap();
    let claim = Claim {
        form: ClaimForm::Equation,
        lhs: lhs.clone(),
        rhs: rhs.clone(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let claim_hash = compute_claim_sha256(std::slice::from_ref(&claim), |h| match h.name.as_str() {
        "lhs" => Some("srepr_lhs_1"),
        "rhs" => Some("srepr_rhs_1"),
        _ => None,
    })
    .unwrap();
    let e2_hash = compute_entry_sha256(None, "model assumption", "details");
    let e3_hash = compute_entry_sha256(Some(&claim_hash), "derived eq", "details");

    vec![
        mk_event(1, event_kinds::SOLVE_STARTED, json!({})),
        mk_event(2, event_kinds::SOLVE_SETUP, serde_json::to_value(&setup).unwrap()),
        mk_event(3, event_kinds::SOLVE_PLAN, json!({"items": [{"title": "Step 1"}]})),
        mk_event(
            4,
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": "e-1",
                "entry_type": "assumption",
                "summary": "x > 0",
                "details": "from setup",
                "depends_on": [],
                "origin": "setup",
                "entry_sha256": compute_entry_sha256(None, "x > 0", "from setup"),
            }),
        ),
        mk_event(5, event_kinds::TOOL_REQUEST, json!({"call_id": "c-1", "tool": "sympy_eval"})),
        mk_event(
            6,
            event_kinds::TOOL_RESULT,
            json!({
                "call_id": "c-1",
                "ok": true,
                "result": {
                    "inline": serde_json::to_string(&json!({
                        "value": "x**2",
                        "error": null,
                        "code_sha256": "code_1",
                        "wall_ms": 15,
                        "process": "solver",
                        "exprs_out": ["lhs@1", "rhs@1"]
                    })).unwrap()
                }
            }),
        ),
        mk_event(
            7,
            event_kinds::SOLVE_EXPR,
            serde_json::to_value(SolveExpr {
                name: "lhs".into(),
                version: 1,
                srepr_blob: "b1".into(),
                srepr_sha256: "srepr_lhs_1".into(),
                latex: "x^2".into(),
                str_preview: "x**2".into(),
                free_symbols: vec![],
                produced_by: ReceiptRef { call_id: "c-1".into(), result_seq: 6 },
                owner_entry: Some("e-3".into()),
                portable: true,
            })
            .unwrap(),
        ),
        mk_event(
            8,
            event_kinds::SOLVE_EXPR,
            serde_json::to_value(SolveExpr {
                name: "rhs".into(),
                version: 1,
                srepr_blob: "b2".into(),
                srepr_sha256: "srepr_rhs_1".into(),
                latex: "x^2".into(),
                str_preview: "x**2".into(),
                free_symbols: vec![],
                produced_by: ReceiptRef { call_id: "c-1".into(), result_seq: 6 },
                owner_entry: Some("e-3".into()),
                portable: true,
            })
            .unwrap(),
        ),
        mk_event(
            9,
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": "e-2",
                "entry_type": "assumption",
                "summary": "model assumption",
                "details": "details",
                "depends_on": [],
                "origin": "solver",
                "entry_sha256": e2_hash,
            }),
        ),
        mk_event(
            10,
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": "e-3",
                "entry_type": "derivation",
                "summary": "derived eq",
                "details": "details",
                "depends_on": ["e-1", "e-2"],
                "tool_call_refs": ["c-1"],
                "claims": [claim],
                "claim_sha256": claim_hash,
                "entry_sha256": e3_hash,
                "origin": "solver",
            }),
        ),
        mk_event(
            11,
            event_kinds::SOLVE_CHECK,
            serde_json::to_value(SolveCheck {
                check_id: "chk-1".into(),
                claim_sha256: claim_hash,
                claim_index: 0,
                entry_id: Some("e-3".into()),
                entry_sha256: None,
                math_sha256: math_sha,
                name: "identity_symbolic".into(),
                registry_sha256: "reg-1".into(),
                params: json!({}),
                outcome: crate::solve_model::CheckOutcome::Pass,
                asked_by: crate::solve_model::CheckRequester::Host,
                receipt: ReceiptRef { call_id: "c-chk".into(), result_seq: 11 },
                evidence: json!({"residual": "0"}),
                evidence_sha256: "ev-1".into(),
            })
            .unwrap(),
        ),
        mk_event(
            12,
            event_kinds::SOLVE_ACK,
            json!({"entry_id": "e-2", "kind": "assumption", "entry_sha256": e2_hash}),
        ),
        mk_event(
            13,
            event_kinds::GUARDRAIL_BLOCKED,
            json!({"gate": "G1", "tool": "sympy_eval", "call_id": "c-2", "message": "interpret first"}),
        ),
        mk_event(
            14,
            event_kinds::SOLVE_PAUSED,
            json!({"reason": "leg_cap", "leg": 1, "turn": 48, "model_calls_used": 48, "open_items": []}),
        ),
        mk_event(
            15,
            event_kinds::SOLVE_CHECKPOINT,
            json!({"leg": 1, "upto_seq": 14, "brief_blob": "bb", "brief_sha256": "bs", "reason": "continue"}),
        ),
        mk_event(16, event_kinds::SESSION_RESUME, json!({"tool_profile": "solve/1"})),
        mk_event(
            17,
            event_kinds::SOLVE_DONE,
            json!({
                "final_entry_id": "e-3",
                "own_tier": "Computed",
                "effective_tier": "Computed",
                "criteria": [],
                "rests_on": {"given": ["e-1", "e-2"], "cited": [], "assumed_by_model": [], "weakest": []},
                "status": "finished",
                "model_calls": {"solver": 4, "verifier": 0, "workers": 0},
            }),
        ),
    ]
}

#[test]
fn incremental_fold_matches_all_at_once_and_every_prefix() {
    let events = rich_session_events();
    let all_at_once = SolveState::from_events(&events);

    let mut one_by_one = SolveState::new();
    for ev in &events {
        one_by_one.apply(ev);
    }
    assert_eq!(one_by_one, all_at_once);

    // Every prefix split k in 0..=len gives the exact same state when continuing from k,
    // and also when re-applying the whole slice (idempotent on seq <= upto_seq).
    for k in 0..=events.len() {
        let mut from_prefix = SolveState::from_events(&events[..k]);
        from_prefix.apply_slice(&events[k..]);
        assert_eq!(from_prefix, all_at_once, "mismatch continuing from prefix k={k}");

        let mut reapplied = SolveState::from_events(&events[..k]);
        reapplied.apply_slice(&events);
        assert_eq!(reapplied, all_at_once, "mismatch re-applying full slice from k={k}");
    }

    assert!(all_at_once.started);
    assert_eq!(all_at_once.upto_seq, 17);
    assert_eq!(all_at_once.run_status, SolveRunStatus::Done(SolveDoneStatus::Finished));
    assert_eq!(all_at_once.tier("e-2").unwrap().effective, Tier::Given);
    assert_eq!(all_at_once.tier("e-3").unwrap().effective, Tier::Computed);
    assert!(all_at_once.pending_runs.is_empty());
}

#[test]
fn pending_runs_tracks_successful_sympy_citations_and_discard_notes() {
    let mut state = SolveState::new();
    // 1. Successful sympy_eval -> added to pending_runs.
    state.apply(&mk_event(1, event_kinds::TOOL_REQUEST, json!({"call_id": "c-1", "tool": "sympy_eval"})));
    state.apply(&mk_event(
        2,
        event_kinds::TOOL_RESULT,
        json!({"call_id": "c-1", "ok": true, "result": {"value": "1", "error": null}}),
    ));
    assert_eq!(state.pending_runs, vec!["c-1"]);

    // 2. Failed sympy_eval -> NOT added to pending_runs (G1 exempts failed runs).
    state.apply(&mk_event(3, event_kinds::TOOL_REQUEST, json!({"call_id": "c-fail", "tool": "sympy_eval"})));
    state.apply(&mk_event(
        4,
        event_kinds::TOOL_RESULT,
        json!({"call_id": "c-fail", "ok": true, "result": {"value": null, "error": "ZeroDivisionError"}}),
    ));
    assert_eq!(state.pending_runs, vec!["c-1"]);

    // 3. Second and third successful runs.
    for (i, cid) in [(5, "c-2"), (7, "c-3")] {
        state.apply(&mk_event(i, event_kinds::TOOL_REQUEST, json!({"call_id": cid, "tool": "sympy_eval"})));
        state.apply(&mk_event(
            i + 1,
            event_kinds::TOOL_RESULT,
            json!({"call_id": cid, "ok": true, "result": {"value": "42", "error": null}}),
        ));
    }
    assert_eq!(state.pending_runs, vec!["c-1", "c-2", "c-3"]);

    // 4. Citing c-1 via proof.entry retires only c-1.
    state.apply(&mk_event(
        9,
        event_kinds::PROOF_ENTRY,
        json!({"entry_id": "e-1", "entry_type": "symbolic_computation", "summary": "s", "details": "d", "tool_call_refs": ["c-1"]}),
    ));
    assert_eq!(state.pending_runs, vec!["c-2", "c-3"]);

    // 5. Ordinary note does not retire anything.
    state.apply(&mk_event(10, event_kinds::NOTE, json!({"text": "just thinking"})));
    assert_eq!(state.pending_runs, vec!["c-2", "c-3"]);

    // 6. Targeted discard note retires only c-2.
    state.apply(&mk_event(11, event_kinds::NOTE, json!({"text": "discard: c-2 was scratch"})));
    assert_eq!(state.pending_runs, vec!["c-3"]);

    // 7. Blanket discard note retires remaining pending runs.
    state.apply(&mk_event(12, event_kinds::NOTE, json!({"text": "discard: exploratory check"})));
    assert!(state.pending_runs.is_empty());
}

#[test]
fn gate_bounces_increments_and_resets_on_ok_result_or_pause() {
    let mut state = SolveState::new();
    state.apply(&mk_event(1, event_kinds::GUARDRAIL_BLOCKED, json!({"gate": "G1", "tool": "sympy_eval", "call_id": "a", "message": "m"})));
    state.apply(&mk_event(2, event_kinds::GUARDRAIL_BLOCKED, json!({"gate": "G1", "tool": "sympy_eval", "call_id": "b", "message": "m"})));
    assert_eq!(state.gate_bounces, 2);

    state.apply(&mk_event(3, event_kinds::TOOL_RESULT, json!({"call_id": "ok-1", "ok": true, "result": {}})));
    assert_eq!(state.gate_bounces, 0);

    state.apply(&mk_event(4, event_kinds::GUARDRAIL_BLOCKED, json!({"gate": "G1", "tool": "sympy_eval", "call_id": "c", "message": "m"})));
    assert_eq!(state.gate_bounces, 1);

    state.apply(&mk_event(5, event_kinds::SOLVE_PAUSED, json!({"reason": "gate_bounces", "leg": 1, "turn": 5, "model_calls_used": 5})));
    assert_eq!(state.gate_bounces, 0);
    assert_eq!(state.run_status, SolveRunStatus::Paused(PauseReason::GateBounces));
}

#[test]
fn accepted_review_with_counterexample_folds_as_rejected() {
    let mut state = SolveState::new();
    let e_hash = compute_entry_sha256(None, "step", "details");
    state.apply(&mk_event(
        1,
        event_kinds::PROOF_ENTRY,
        json!({"entry_id": "e-1", "entry_type": "derivation", "summary": "step", "details": "details", "entry_sha256": e_hash}),
    ));
    state.apply(&mk_event(
        2,
        event_kinds::PROOF_VERIFICATION,
        json!({
            "target_entry_id": "e-1",
            "entry_sha256": e_hash,
            "verdict": "accepted",
            "independence": "different_provider",
            "counterexample": "fails at x = 0",
        }),
    ));
    assert_eq!(state.reviews[0].verdict, ReviewVerdict::Rejected);
    assert_eq!(state.tier("e-1").unwrap().effective, Tier::Failed);
}
