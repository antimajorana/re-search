//! Regression tests for WP1 agent changes: finish review target selection, handle source
//! provenance, trivial check citation rejection, repair-round gating, and solver prompts.

use crate::solve_loop::loop_tools::repairable_open_items;
use crate::solve_prompts::build_solve_system_prompt;
use crate::verifier_loop::handle_sources::{
    build_handle_sources, extract_sympy_code_by_call, reject_trivial_check_citation,
};
use crate::verifier_loop::{build_solve_review_prompt, select_finish_review_targets};
use ledger::Event;
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, CheckOutcome,
    CheckRequester, Claim, ClaimForm, ClaimRelation, DoneCriterion, DoneCriterionKind, HandleRef,
    ReceiptRef, ReviewIndependence, ReviewPolicy, RoleModel, SetupAssumption, SolveBudgets,
    SolveCheck, SolveExpr, SolveRoles, SolveSetup, SolveTarget, StepEntryType, StepOrigin,
    SymbolSpec, TargetForm, TaskKind,
};
use research_tools::solve_state::SolveState;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-vf".into(),
        seq,
        ts_ms: 0,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: String::new(),
    }
}

fn make_setup(same_model: bool, review: ReviewPolicy) -> SolveSetup {
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
        statement: "Prove X".into(),
        task_kind: TaskKind::Prove,
        target: SolveTarget {
            description: "Proof".into(),
            form: TargetForm::Proof,
            order: None,
            digits: None,
        },
        done: vec![
            DoneCriterion {
                id: "final_review".into(),
                kind: DoneCriterionKind::FinalTierAtLeast,
                params: json!({ "tier": if same_model { "model_agreed_same_model" } else { "model_agreed" } }),
            },
            DoneCriterion {
                id: "closure_review".into(),
                kind: DoneCriterionKind::ClosureTierAtLeast,
                params: json!({ "tier": "verified" }),
            },
        ],
        conventions: String::new(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles { solver, verifier, compute: None },
        budgets: SolveBudgets::default(),
        review,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha,
    }
}

#[test]
fn prose_only_selects_unchecked_claim_steps_and_skips_computed_and_already_accepted_same_model() {
    let setup = make_setup(true, ReviewPolicy::ProseOnly);
    let math_sha = setup.math_sha256.clone();

    let expr_a = SolveExpr {
        name: "a".into(), version: 1, srepr_blob: "sha-a".into(), srepr_sha256: "sha-a".into(),
        latex: "(x+1)^2".into(), str_preview: "(x+1)**2".into(), free_symbols: vec![],
        produced_by: ReceiptRef { call_id: "c-1".into(), result_seq: 4 }, owner_entry: None,
        portable: true, unevaluated_srepr: None, unevaluated_srepr_sha256: None,
        source_expr: Some("(x + 1)**2".into()),
    };
    let expr_b = SolveExpr {
        name: "b".into(), version: 1, srepr_blob: "sha-b".into(), srepr_sha256: "sha-b".into(),
        latex: "x^2+2x+1".into(), str_preview: "x**2+2*x+1".into(), free_symbols: vec![],
        produced_by: ReceiptRef { call_id: "c-1".into(), result_seq: 4 }, owner_entry: None,
        portable: true, unevaluated_srepr: None, unevaluated_srepr_sha256: None,
        source_expr: Some("x**2 + 2*x + 1".into()),
    };
    let expr_same1 = SolveExpr {
        name: "s1".into(), version: 1, srepr_blob: "sha-same".into(), srepr_sha256: "sha-same".into(),
        latex: "0".into(), str_preview: "0".into(), free_symbols: vec![],
        produced_by: ReceiptRef { call_id: "c-1".into(), result_seq: 4 }, owner_entry: None,
        portable: true, unevaluated_srepr: None, unevaluated_srepr_sha256: None,
        source_expr: Some("0".into()),
    };
    let expr_same2 = SolveExpr { name: "s2".into(), ..expr_same1.clone() };

    let c_good = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("a", 1).unwrap(), rhs: HandleRef::new("b", 1).unwrap(),
        relation: ClaimRelation::Eq, order: None, domain: None, how: None,
    };
    let c_triv = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("s1", 1).unwrap(), rhs: HandleRef::new("s2", 1).unwrap(),
        relation: ClaimRelation::Eq, order: None, domain: None, how: None,
    };
    let sha_good = compute_claim_sha256(std::slice::from_ref(&c_good), |h| Some(if h.name == "a" { "sha-a" } else { "sha-b" })).unwrap();
    let sha_triv = compute_claim_sha256(std::slice::from_ref(&c_triv), |_| Some("sha-same")).unwrap();
    let e1_sha = compute_entry_sha256(Some(&sha_good), "Verified step", "d");
    let e2_sha = compute_entry_sha256(Some(&sha_triv), "Unchecked trivial step", "d");
    let e3_sha = compute_entry_sha256(None, "Already reviewed prose step", "d");
    let e4_sha = compute_entry_sha256(None, "Final prose step", "d");

    let events = vec![
        ev(1, event_kinds::SOLVE_STARTED, json!({})),
        ev(2, event_kinds::SOLVE_SETUP, serde_json::to_value(&setup).unwrap()),
        ev(3, event_kinds::TOOL_REQUEST, json!({
            "call_id": "c-1", "tool": "sympy_eval",
            "args": { "inline": "{\"code\":\"save_expr('a', (x+1)**2)\\nsave_expr('b', x**2+2*x+1)\"}" }
        })),
        ev(4, event_kinds::TOOL_RESULT, json!({ "call_id": "c-1", "ok": true, "result": { "value": "0", "error": null, "process": "solver" } })),
        ev(5, event_kinds::SOLVE_EXPR, serde_json::to_value(&expr_a).unwrap()),
        ev(6, event_kinds::SOLVE_EXPR, serde_json::to_value(&expr_b).unwrap()),
        ev(7, event_kinds::SOLVE_EXPR, serde_json::to_value(&expr_same1).unwrap()),
        ev(8, event_kinds::SOLVE_EXPR, serde_json::to_value(&expr_same2).unwrap()),
        // e-1: claim-bearing step that IS Verified -> skipped under ProseOnly.
        ev(9, event_kinds::PROOF_ENTRY, json!({
            "entry_id": "e-1", "entry_type": StepEntryType::SymbolicComputation, "summary": "Verified step", "details": "d",
            "depends_on": [], "tool_call_refs": ["c-1"], "claims": [c_good], "claim_sha256": sha_good, "entry_sha256": e1_sha, "origin": StepOrigin::Solver,
        })),
        ev(10, event_kinds::TOOL_RESULT, json!({ "call_id": "chk-1", "ok": true, "result": { "process": "check" } })),
        ev(11, event_kinds::SOLVE_CHECK, serde_json::to_value(SolveCheck {
            check_id: "chk-1".into(), claim_sha256: sha_good.clone(), claim_index: 0, entry_id: Some("e-1".into()),
            math_sha256: math_sha.clone(), name: "identity_symbolic".into(), registry_sha256: "r".into(),
            params: json!({}), outcome: CheckOutcome::Pass, asked_by: CheckRequester::Host,
            receipt: ReceiptRef { call_id: "chk-1".into(), result_seq: 10 }, evidence: json!({"exact": true}), evidence_sha256: "e".into(),
        }).unwrap()),
        // e-2: claim-bearing step with trivial check + Accepted review citing that trivial check -> MUST STILL be selected!
        ev(12, event_kinds::PROOF_ENTRY, json!({
            "entry_id": "e-2", "entry_type": StepEntryType::SymbolicComputation, "summary": "Unchecked trivial step", "details": "d",
            "depends_on": ["e-1"], "tool_call_refs": ["c-1"], "claims": [c_triv], "claim_sha256": sha_triv, "entry_sha256": e2_sha, "origin": StepOrigin::Solver,
        })),
        ev(13, event_kinds::SOLVE_CHECK, serde_json::to_value(SolveCheck {
            check_id: "chk-triv".into(), claim_sha256: sha_triv.clone(), claim_index: 0, entry_id: Some("e-2".into()),
            math_sha256: math_sha.clone(), name: "identity_symbolic".into(), registry_sha256: "r".into(),
            params: json!({}), outcome: CheckOutcome::Trivial, asked_by: CheckRequester::Host,
            receipt: ReceiptRef { call_id: "chk-1".into(), result_seq: 10 }, evidence: json!({"identical_sides": true}), evidence_sha256: "et".into(),
        }).unwrap()),
        ev(14, event_kinds::PROOF_VERIFICATION, json!({
            "target_entry_id": "e-2", "verdict": "accepted", "entry_sha256": e2_sha,
            "independence": ReviewIndependence::SameModel, "checks_cited": ["chk-triv"],
        })),
        // e-3: prose step already validly accepted at same-model -> MUST BE SKIPPED (no redundant re-review)!
        ev(15, event_kinds::PROOF_ENTRY, json!({
            "entry_id": "e-3", "entry_type": StepEntryType::Derivation, "summary": "Already reviewed prose step", "details": "d",
            "depends_on": ["e-2"], "entry_sha256": e3_sha, "origin": StepOrigin::Solver,
        })),
        ev(16, event_kinds::PROOF_VERIFICATION, json!({
            "target_entry_id": "e-3", "verdict": "accepted", "entry_sha256": e3_sha,
            "independence": ReviewIndependence::SameModel, "checks_cited": [],
        })),
        // e-4: unreviewed final step -> selected.
        ev(17, event_kinds::PROOF_ENTRY, json!({
            "entry_id": "e-4", "entry_type": StepEntryType::FinalAnswer, "summary": "Final prose step", "details": "d",
            "depends_on": ["e-3"], "entry_sha256": e4_sha, "origin": StepOrigin::Solver,
        })),
    ];

    let state = SolveState::from_events(&events);
    let targets = select_finish_review_targets(&state, "e-4");
    assert_eq!(targets, vec!["e-2", "e-4"], "expected e-2 (unchecked/invalidated) and e-4; skipping e-1 (Verified) and e-3 (already accepted)");

    // Verify handle_sources extraction and trivial check citation rejection.
    let code_map = extract_sympy_code_by_call(&events, None);
    assert!(code_map.get("c-1").is_some_and(|c| c.contains("save_expr('a'")));
    let closure: Vec<_> = state.entries.iter().collect();
    let hs = build_handle_sources(&state, &closure, &code_map);
    assert!(hs["sympy_code_by_call"]["c-1"].as_str().is_some_and(|s| s.contains("save_expr")));
    let prompt = build_solve_review_prompt(&setup, &state, &["e-2".into()], &closure, 48_000);
    assert!(prompt.contains("\"sides_identical\": true"));
    let triv_chk = state.checks.iter().find(|c| c.check_id == "chk-triv").unwrap();
    let err = reject_trivial_check_citation(&state, "e-2", triv_chk).unwrap_err();
    assert!(err.contains("trivial"), "got: {err}");
}

#[test]
fn repairable_open_items_skips_user_only_items_and_names_below_target_for_solver_items() {
    // Only user-clearable items (assumed_by_model + same model flag) -> None (skip repair round).
    let user_only = json!({
        "status": "finished_open_items",
        "final_entry_id": "e-7",
        "criteria": [
            { "id": "c1", "met": false, "evidence": { "kind": "closure_tier_at_least", "required": "model_agreed_same_model", "require_different_model": false, "below_target": [{ "entry_id": "e-1", "effective": "assumed_by_model" }] } },
            { "id": "c2", "met": false, "evidence": { "kind": "no_open_flags", "open_flags": [{ "entry_id": "e-7", "flags": ["same model", "rests on 1 assumption made by the model"] }] } },
        ]
    });
    assert!(repairable_open_items(&user_only).is_none());

    // Unchecked step e-3 -> repairable, returns below_target = ["e-3"].
    let solver_fixable = json!({
        "status": "finished_open_items",
        "final_entry_id": "e-7",
        "criteria": [
            { "id": "c1", "met": false, "evidence": { "kind": "closure_tier_at_least", "required": "verified", "require_different_model": false, "below_target": [{ "entry_id": "e-3", "effective": "unchecked" }] } },
        ]
    });
    let (below, unmet) = repairable_open_items(&solver_fixable).expect("must trigger repair round");
    assert_eq!(below, vec!["e-3"]);
    assert_eq!(unmet, vec!["c1"]);

    // Verify updated Solve system prompts enforce independent sides and unevaluated save_expr.
    let prompt = build_solve_system_prompt(&make_setup(false, ReviewPolicy::ProseOnly), &[]);
    assert!(prompt.contains("UnevaluatedExpr") && prompt.contains("trivial"));
}
