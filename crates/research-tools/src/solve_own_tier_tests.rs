//! Focused regression tests for comparison evidence, check outcomes, and check receipt
//! guards in `solve_own_tier.rs` (`solve.md` 4.4 and "Inspecting a check").

use crate::solve_badge::{wording, Tier};
use crate::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, CheckOutcome,
    CheckRequester, Claim, ClaimForm, ClaimRelation, HandleRef, ReceiptRef, ReviewPolicy,
    RoleModel, SeriesOrder, SetupAssumption, SolveBudgets, SolveCheck, SolveExpr, SolveRoles,
    SolveSetup, SolveTarget, StepEntryType, StepOrigin, SymbolSpec, TargetForm, TaskKind,
};
use crate::solve_state::SolveState;
use ledger::Event;
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Harness {
    seq: u64,
    state: SolveState,
}

impl Harness {
    fn new() -> Self {
        let mut h = Self { seq: 0, state: SolveState::new() };
        h.emit(event_kinds::SOLVE_STARTED, json!({}));
        h.set_setup(1, "Euclidean, hbar = c = 1", "x > 0");
        h
    }

    fn emit(&mut self, kind: &str, payload: Value) -> u64 {
        self.seq += 1;
        let ev = Event {
            session: "s-own-tier".into(),
            seq: self.seq,
            ts_ms: (self.seq as i64) * 100,
            kind: kind.into(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "2".repeat(64),
        };
        self.state.apply(&ev);
        self.seq
    }

    fn math_sha(&self) -> String {
        self.state.setup.as_ref().unwrap().math_sha256.clone()
    }

    fn set_setup(&mut self, revision: u32, conventions: &str, assumption_text: &str) {
        let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["positive".into()] }];
        let assumptions = vec![SetupAssumption { id: "a-1".into(), text: assumption_text.into() }];
        let math_sha256 =
            compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, conventions);
        let setup = SolveSetup {
            revision,
            statement: "Comparison evidence regression".into(),
            task_kind: TaskKind::Calculate,
            target: SolveTarget {
                description: "target".into(),
                form: TargetForm::ClosedForm,
                order: None,
                digits: None,
            },
            done: vec![],
            conventions: conventions.into(),
            symbols,
            dimensions: BTreeMap::new(),
            assumptions,
            given_source: None,
            sources: vec![],
            roles: SolveRoles {
                solver: RoleModel { provider: "anthropic".into(), model: "solver-1".into() },
                verifier: RoleModel { provider: "openai".into(), model: "verifier-1".into() },
                compute: None,
            },
            budgets: SolveBudgets::default(),
            review: ReviewPolicy::ProseOnly,
            tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
            origin: None,
            math_sha256,
        };
        self.emit(event_kinds::SOLVE_SETUP, serde_json::to_value(setup).unwrap());
    }

    fn add_tool_run(
        &mut self,
        call_id: &str,
        tool: &str,
        ok: bool,
        process: &str,
        value: &str,
    ) -> u64 {
        self.emit(event_kinds::TOOL_REQUEST, json!({"call_id": call_id, "tool": tool}));
        self.emit(
            event_kinds::TOOL_RESULT,
            json!({
                "call_id": call_id,
                "ok": ok,
                "result": {
                    "value": value,
                    "error": if ok { Value::Null } else { json!("failed") },
                    "code_sha256": format!("code-{call_id}"),
                    "process": process
                }
            }),
        )
    }

    fn add_expr(&mut self, name: &str, srepr_sha: &str, produced_by: (&str, u64)) -> HandleRef {
        let expr = SolveExpr {
            name: name.into(),
            version: 1,
            srepr_blob: format!("blob-{name}"),
            srepr_sha256: srepr_sha.into(),
            latex: name.into(),
            str_preview: name.into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: produced_by.0.into(), result_seq: produced_by.1 },
            owner_entry: None,
            portable: true,
            unevaluated_srepr: None, unevaluated_srepr_sha256: None, source_expr: None,
        };
        let handle = expr.handle().unwrap();
        self.emit(event_kinds::SOLVE_EXPR, serde_json::to_value(expr).unwrap());
        handle
    }

    fn add_step(
        &mut self,
        id: &str,
        entry_type: StepEntryType,
        deps: &[&str],
        claims: Vec<Claim>,
    ) -> (String, String) {
        let claim_sha = compute_claim_sha256(&claims, |h| {
            self.state.handles.get(h).map(|e| e.srepr_sha256.as_str())
        })
        .unwrap();
        let entry_sha = compute_entry_sha256(Some(&claim_sha), id, "details");
        self.emit(
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": id,
                "entry_type": entry_type,
                "summary": id,
                "details": "details",
                "depends_on": deps,
                "claims": claims,
                "claim_sha256": claim_sha,
                "entry_sha256": entry_sha,
                "origin": StepOrigin::Solver,
            }),
        );
        (claim_sha, entry_sha)
    }

    fn add_check_with_receipt(
        &mut self,
        claim_sha: &str,
        entry_id: &str,
        name: &str,
        outcome: CheckOutcome,
        evidence: Value,
        receipt: ReceiptRef,
    ) {
        let math_sha = self.math_sha();
        self.emit(
            event_kinds::SOLVE_CHECK,
            serde_json::to_value(SolveCheck {
                check_id: format!("chk-{}", self.seq + 1),
                claim_sha256: claim_sha.into(),
                claim_index: 0,
                entry_id: Some(entry_id.into()),
                math_sha256: math_sha,
                name: name.into(),
                registry_sha256: "reg-v1".into(),
                params: json!({}),
                outcome,
                asked_by: CheckRequester::Host,
                receipt,
                evidence,
                evidence_sha256: "ev-sha".into(),
            })
            .unwrap(),
        );
    }

    fn add_check(
        &mut self,
        claim_sha: &str,
        entry_id: &str,
        name: &str,
        outcome: CheckOutcome,
        evidence: Value,
    ) {
        let receipt = ReceiptRef { call_id: format!("c-chk-{}", self.seq + 1), result_seq: self.seq + 1 };
        self.add_check_with_receipt(claim_sha, entry_id, name, outcome, evidence, receipt);
    }
}

fn eq_claim(lhs: HandleRef, rhs: HandleRef) -> Claim {
    Claim {
        form: ClaimForm::Equation,
        lhs,
        rhs,
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    }
}

fn value_claim(lhs: HandleRef, rhs: HandleRef) -> Claim {
    Claim {
        form: ClaimForm::Value,
        lhs,
        rhs,
        relation: ClaimRelation::Approx,
        order: None,
        domain: None,
        how: None,
    }
}

fn series_claim(lhs: HandleRef, rhs: HandleRef) -> Claim {
    Claim {
        form: ClaimForm::Series,
        lhs,
        rhs,
        relation: ClaimRelation::Eq,
        order: Some(SeriesOrder { variable: "eps".into(), point: "0".into(), n: 2 }),
        domain: None,
        how: None,
    }
}

#[test]
fn independent_recompute_requires_positive_non_contradictory_comparison_evidence() {
    let mut h = Harness::new();
    let r1 = h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
    let a = h.add_expr("A", "sa", ("c1", r1));
    let b = h.add_expr("B", "sb", ("c1", r1));

    // Missing, negative-only, unrecognized, malformed, series-only, and contradictory inputs
    // must all remain Unchecked with no "independent recompute" sublabel.
    let rejected_cases = [
        json!({}),
        json!(null),
        json!("numeric"),
        json!({"comparison": null}),
        json!({"exact": false}),
        json!({"comparison": ""}),
        json!({"comparison": "approx"}),
        json!({"comparison": "symbolic"}),
        json!({"comparison": "NUMERIC"}),
        json!({"comparison": "EXACT"}),
        json!({"comparison": 1}),
        json!({"exact": "true"}),
        json!({"numeric_coefficients": true}),
        json!({"comparison": "numeric", "numeric_coefficients": true}),
        json!({"comparison": "numeric", "numeric_coefficients": false}),
        json!({"comparison": "exact", "exact": false}),
        json!({"comparison": "numeric", "exact": true}),
        json!({"comparison": "exact", "numeric_coefficients": true}),
        json!({"comparison": "approx", "exact": true}),
        json!({"exact": true, "numeric_coefficients": "yes"}),
    ];

    for (i, ev) in rejected_cases.iter().enumerate() {
        let mut step_h = Harness::new();
        let sr = step_h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
        let sa = step_h.add_expr("A", "sa", ("c1", sr));
        let sb = step_h.add_expr("B", "sb", ("c1", sr));
        let claim = if i % 2 == 0 { eq_claim(sa, sb) } else { value_claim(sa, sb) };
        let (csha, _) = step_h.add_step("e-1", StepEntryType::Result, &[], vec![claim]);
        step_h.add_check(&csha, "e-1", "independent_recompute", CheckOutcome::Pass, ev.clone());
        let tier = step_h.state.tier("e-1").unwrap();
        assert_eq!(tier.own, Tier::Unchecked, "evidence {ev} should stay Unchecked");
        assert!(
            !tier.sublabels.contains(&wording::SUBLABEL_INDEPENDENT_RECOMPUTE.to_string()),
            "evidence {ev} should not emit independent recompute sublabel"
        );
    }

    // Explicit numeric comparison earns CheckedNumerically for both Equation and Value claims.
    for (idx, (claim, ev)) in [
        (eq_claim(a.clone(), b.clone()), json!({"comparison": "numeric"})),
        (value_claim(a.clone(), b.clone()), json!({"comparison": "numeric", "exact": false})),
    ]
    .into_iter()
    .enumerate()
    {
        let eid = format!("e-num-{idx}");
        let (csha, _) = h.add_step(&eid, StepEntryType::Result, &[], vec![claim]);
        h.add_check(&csha, &eid, "independent_recompute", CheckOutcome::Pass, ev);
        let tier = h.state.tier(&eid).unwrap();
        assert_eq!(tier.own, Tier::CheckedNumerically);
        assert!(tier.sublabels.contains(&wording::SUBLABEL_INDEPENDENT_RECOMPUTE.to_string()));
    }

    // Explicit exact comparison earns Verified.
    for (idx, (claim, ev)) in [
        (eq_claim(a.clone(), b.clone()), json!({"comparison": "exact"})),
        (value_claim(a, b), json!({"comparison": "exact", "exact": true})),
    ]
    .into_iter()
    .enumerate()
    {
        let eid = format!("e-exact-{idx}");
        let (csha, _) = h.add_step(&eid, StepEntryType::Result, &[], vec![claim]);
        h.add_check(&csha, &eid, "independent_recompute", CheckOutcome::Pass, ev);
        let tier = h.state.tier(&eid).unwrap();
        assert_eq!(tier.own, Tier::Verified);
        assert!(tier.sublabels.contains(&wording::SUBLABEL_INDEPENDENT_RECOMPUTE.to_string()));
    }
}

#[test]
fn series_checks_require_positive_non_contradictory_comparison_evidence() {
    let valid_order = json!({"variable": "eps", "point": "0", "n": 2});

    // Missing, negative-only, unrecognized, malformed, and contradictory evidence stay Unchecked.
    let rejected_cases = [
        json!({"order": valid_order}),
        json!({"order": valid_order, "exact": false}),
        json!({"order": valid_order, "numeric_coefficients": false}),
        json!({"order": valid_order, "comparison": null}),
        json!({"order": valid_order, "comparison": "approx"}),
        json!({"order": valid_order, "comparison": "symbolic"}),
        json!({"order": valid_order, "comparison": "NUMERIC"}),
        json!({"order": valid_order, "numeric_coefficients": "true"}),
        json!({"order": valid_order, "exact": true, "numeric_coefficients": true}),
        json!({"order": valid_order, "comparison": "exact", "numeric_coefficients": true}),
        json!({"order": valid_order, "comparison": "numeric", "numeric_coefficients": false}),
        json!({"order": valid_order, "comparison": "numeric", "exact": true}),
        json!({"order": valid_order, "comparison": "exact", "exact": false}),
        json!({"order": valid_order, "comparison": "bogus", "numeric_coefficients": true}),
        // Order too low even with explicit numeric_coefficients must stay Unchecked.
        json!({"order": {"variable": "eps", "point": "0", "n": 1}, "numeric_coefficients": true}),
    ];

    for check_name in ["series_match", "vanishes_to_order"] {
        for ev in &rejected_cases {
            let mut h = Harness::new();
            let r1 = h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
            let a = h.add_expr("S_lhs", "sl", ("c1", r1));
            let b = h.add_expr("S_rhs", "sr", ("c1", r1));
            let (csha, _) =
                h.add_step("e-1", StepEntryType::Result, &[], vec![series_claim(a, b)]);
            h.add_check(&csha, "e-1", check_name, CheckOutcome::Pass, ev.clone());
            let tier = h.state.tier("e-1").unwrap();
            assert_eq!(tier.own, Tier::Unchecked, "{check_name} with {ev} should stay Unchecked");
            assert!(tier.sublabels.is_empty(), "{check_name} with {ev} should emit no sublabels");
            assert!(tier.flags.is_empty(), "{check_name} with {ev} should emit no series flags");
        }

        // Explicit numeric forms earn CheckedNumerically for a valid series claim.
        for ev in [
            json!({"order": valid_order, "numeric_coefficients": true}),
            json!({"order": valid_order, "numeric_coefficients": true, "exact": false}),
            json!({"order": valid_order, "comparison": "numeric"}),
            json!({"order": valid_order, "comparison": "numeric", "numeric_coefficients": true}),
        ] {
            let mut h = Harness::new();
            let r1 = h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
            let a = h.add_expr("S_lhs", "sl", ("c1", r1));
            let b = h.add_expr("S_rhs", "sr", ("c1", r1));
            let (csha, _) =
                h.add_step("e-1", StepEntryType::Result, &[], vec![series_claim(a, b)]);
            h.add_check(&csha, "e-1", check_name, CheckOutcome::Pass, ev.clone());
            let tier = h.state.tier("e-1").unwrap();
            assert_eq!(tier.own, Tier::CheckedNumerically, "{check_name} with {ev}");
            assert!(tier.sublabels.contains(&"to O(eps^2), numeric coefficients".to_string()));
            assert!(tier.flags.contains(&"to O(eps^2)".to_string()));
        }

        // Explicit exact forms earn Verified for a valid series claim.
        for ev in [
            json!({"order": valid_order, "comparison": "exact"}),
            json!({"order": valid_order, "comparison": "exact", "numeric_coefficients": false}),
            json!({"order": valid_order, "exact": true}),
        ] {
            let mut h = Harness::new();
            let r1 = h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
            let a = h.add_expr("S_lhs", "sl", ("c1", r1));
            let b = h.add_expr("S_rhs", "sr", ("c1", r1));
            let (csha, _) =
                h.add_step("e-1", StepEntryType::Result, &[], vec![series_claim(a, b)]);
            h.add_check(&csha, "e-1", check_name, CheckOutcome::Pass, ev.clone());
            let tier = h.state.tier("e-1").unwrap();
            assert_eq!(tier.own, Tier::Verified, "{check_name} with {ev}");
            assert!(tier.sublabels.contains(&"to O(eps^2)".to_string()));
            assert!(tier.flags.contains(&"to O(eps^2)".to_string()));
        }
    }

    // Series claim missing `order` or bare-integer `order` without matching `params` stays Unchecked.
    let mut h = Harness::new();
    let r1 = h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
    let a = h.add_expr("S_lhs", "sl", ("c1", r1));
    let b = h.add_expr("S_rhs", "sr", ("c1", r1));
    let no_order = Claim {
        form: ClaimForm::Series,
        lhs: a.clone(),
        rhs: b.clone(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let (c_none, _) = h.add_step("e-none", StepEntryType::Result, &[], vec![no_order]);
    h.add_check(&c_none, "e-none", "series_match", CheckOutcome::Pass, json!({"comparison": "exact"}));
    assert_eq!(h.state.tier("e-none").unwrap().own, Tier::Unchecked);

    let (c_int, _) = h.add_step("e-int", StepEntryType::Result, &[], vec![series_claim(a, b)]);
    h.add_check(&c_int, "e-int", "series_match", CheckOutcome::Pass, json!({"order": 2, "comparison": "exact"}));
    assert_eq!(h.state.tier("e-int").unwrap().own, Tier::Unchecked);
}

#[test]
fn inconclusive_and_not_run_never_earn_computed_tier() {
    for outcome in [CheckOutcome::Inconclusive, CheckOutcome::NotRun] {
        let mut h = Harness::new();
        let r1 = h.add_tool_run("c1", "sympy_eval", true, "solver", "ok");
        let a = h.add_expr("A", "sa", ("c1", r1));
        let b = h.add_expr("B", "sb", ("c1", r1));
        let (c_eq, _) = h.add_step("e-1", StepEntryType::Result, &[], vec![eq_claim(a.clone(), b.clone())]);
        h.add_check(&c_eq, "e-1", "identity_symbolic", outcome, json!({"reason": "timeout"}));
        h.add_check(&c_eq, "e-1", "independent_recompute", outcome, json!({"comparison": "exact"}));
        h.add_check(&c_eq, "e-1", "independent_recompute", outcome, json!({"comparison": "numeric"}));
        assert_eq!(h.state.tier("e-1").unwrap().own, Tier::Unchecked);
        assert!(h.state.tier("e-1").unwrap().sublabels.is_empty());

        let (c_ser, _) = h.add_step("e-2", StepEntryType::Result, &[], vec![series_claim(a, b)]);
        h.add_check(
            &c_ser,
            "e-2",
            "series_match",
            outcome,
            json!({"order": {"variable": "eps", "point": "0", "n": 2}, "comparison": "exact"}),
        );
        h.add_check(
            &c_ser,
            "e-2",
            "vanishes_to_order",
            outcome,
            json!({"order": {"variable": "eps", "point": "0", "n": 2}, "numeric_coefficients": true}),
        );
        assert_eq!(h.state.tier("e-2").unwrap().own, Tier::Unchecked);
        assert!(h.state.tier("e-2").unwrap().sublabels.is_empty());
        assert!(h.state.tier("e-2").unwrap().flags.is_empty());
    }
}

#[test]
fn receipt_guard_and_model_review_or_solver_exit_never_infer_computed_tier() {
    let mut h = Harness::new();
    let solver_seq = h.add_tool_run("c-solver", "sympy_eval", true, "solver", "True");
    let verifier_sympy_seq = h.add_tool_run("c-ver-sym", "sympy_eval", true, "verifier", "True");
    let verifier_other_seq = h.add_tool_run("c-ver-oth", "check_run", true, "verifier", "ok");
    let note_seq = h.add_tool_run("c-note", "ledger_note", true, "unknown", "ok");
    let inflight_seq = h.emit(event_kinds::TOOL_REQUEST, json!({"call_id": "c-inflight", "tool": "check_run"}));
    let fail_chk_seq = h.add_tool_run("c-chk-fail", "check_run", false, "check", "err");
    let ok_chk_seq = h.add_tool_run("c-chk-ok", "compute_blind", true, "check", "agreed");
    let a = h.add_expr("A", "sa", ("c-solver", solver_seq));
    let b = h.add_expr("B", "sb", ("c-solver", solver_seq));
    let c = h.add_expr("C", "sc", ("c-solver", solver_seq));

    let (c1, e1_sha) = h.add_step(
        "e-1",
        StepEntryType::SymbolicComputation,
        &[],
        vec![eq_claim(a, b.clone())],
    );

    // Blank, failed, in-flight, wrong-seq, solver, verifier, and non-check receipts are rejected.
    for bad_receipt in [
        ReceiptRef { call_id: "".into(), result_seq: 1 },
        ReceiptRef { call_id: " c-solver ".into(), result_seq: solver_seq },
        ReceiptRef { call_id: "c-any".into(), result_seq: 0 },
        ReceiptRef { call_id: "c-inflight".into(), result_seq: inflight_seq },
        ReceiptRef { call_id: "c-chk-fail".into(), result_seq: fail_chk_seq },
        ReceiptRef { call_id: "c-chk-ok".into(), result_seq: ok_chk_seq + 99 },
        ReceiptRef { call_id: "c-solver".into(), result_seq: solver_seq },
        ReceiptRef { call_id: "c-ver-sym".into(), result_seq: verifier_sympy_seq },
        ReceiptRef { call_id: "c-ver-oth".into(), result_seq: verifier_other_seq },
        ReceiptRef { call_id: "c-note".into(), result_seq: note_seq },
    ] {
        h.add_check_with_receipt(
            &c1,
            "e-1",
            "independent_recompute",
            CheckOutcome::Pass,
            json!({"comparison": "exact"}),
            bad_receipt,
        );
    }
    h.add_check_with_receipt(
        &c1,
        "e-1",
        "dimensions",
        CheckOutcome::Pass,
        json!({}),
        ReceiptRef { call_id: "".into(), result_seq: 0 },
    );
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::Unchecked);
    assert!(h.state.tier("e-1").unwrap().sublabels.is_empty());

    // Accepted model review lifts at most to ModelAgreed (never Verified or CheckedNumerically).
    h.emit(
        event_kinds::PROOF_VERIFICATION,
        json!({
            "target_entry_id": "e-1",
            "entry_sha256": e1_sha,
            "verdict": "accepted",
            "independence": "different_provider",
        }),
    );
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::ModelAgreed);

    // A valid check-process receipt with explicit numeric comparison earns CheckedNumerically.
    h.add_check_with_receipt(
        &c1,
        "e-1",
        "independent_recompute",
        CheckOutcome::Pass,
        json!({"comparison": "numeric"}),
        ReceiptRef { call_id: "c-chk-ok".into(), result_seq: ok_chk_seq },
    );
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::CheckedNumerically);

    // Dependency, stale, and failure rules remain intact.
    let (c2, _) = h.add_step("e-2", StepEntryType::Result, &["e-1"], vec![eq_claim(b, c)]);
    h.add_check(&c2, "e-2", "independent_recompute", CheckOutcome::Pass, json!({"comparison": "exact"}));
    assert_eq!(h.state.tier("e-2").unwrap().own, Tier::Verified);
    assert_eq!(h.state.tier("e-2").unwrap().effective, Tier::CheckedNumerically);

    // Setup revision marks both steps Stale (existing stale rule preserved).
    h.set_setup(2, "Euclidean, hbar = c = 1", "x in R");
    assert_eq!(h.state.tier("e-1").unwrap().effective, Tier::Stale);
    assert_eq!(h.state.tier("e-2").unwrap().effective, Tier::Stale);

    // Failing check under the current setup still fails even without comparison kind or receipt,
    // and propagates Failed downstream.
    h.add_check_with_receipt(
        &c1,
        "e-1",
        "independent_recompute",
        CheckOutcome::Fail,
        json!({}),
        ReceiptRef { call_id: "".into(), result_seq: 0 },
    );
    assert_eq!(h.state.tier("e-1").unwrap().effective, Tier::Failed);
    assert_eq!(h.state.tier("e-2").unwrap().effective, Tier::Failed);
}
