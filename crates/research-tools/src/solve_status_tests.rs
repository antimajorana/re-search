use super::*;
use crate::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, CheckOutcome,
    CheckRequester, Claim, ClaimForm, ClaimRelation, HandleRef, QuoteRef, ReceiptRef,
    ReviewPolicy, RoleModel, SeriesOrder, SetupAssumption, SolveBudgets, SolveCheck, SolveExpr,
    SolveRoles, SolveSetup, SolveTarget, StepEntryType, StepOrigin, SymbolSpec, TargetForm,
    TaskKind, TranscriptionMeta,
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
        h.set_setup(1, "Euclidean, hbar = c = 1", "m > 0");
        h
    }

    fn emit(&mut self, kind: &str, payload: Value) -> u64 {
        self.seq += 1;
        let ev = Event {
            session: "s-status".into(),
            seq: self.seq,
            ts_ms: (self.seq as i64) * 100,
            kind: kind.into(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "1".repeat(64),
        };
        self.state.apply(&ev);
        self.seq
    }

    fn math_sha(&self) -> String {
        self.state.setup.as_ref().unwrap().math_sha256.clone()
    }

    fn set_setup(&mut self, revision: u32, conventions: &str, assumption_text: &str) {
        let symbols = vec![SymbolSpec { name: "m".into(), assumptions: vec!["positive".into()] }];
        let assumptions = vec![SetupAssumption { id: "a-1".into(), text: assumption_text.into() }];
        let math_sha256 =
            compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, conventions);
        let setup = SolveSetup {
            revision,
            statement: "Effective potential".into(),
            task_kind: TaskKind::Calculate,
            target: SolveTarget {
                description: "V_1".into(),
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

    fn add_sympy_run(&mut self, call_id: &str, code_sha256: &str) -> u64 {
        self.emit(event_kinds::TOOL_REQUEST, json!({"call_id": call_id, "tool": "sympy_eval"}));
        self.emit(
            event_kinds::TOOL_RESULT,
            json!({
                "call_id": call_id, "ok": true,
                "result": {"value": "ok", "error": null, "code_sha256": code_sha256, "process": "solver"}
            }),
        )
    }

    fn add_expr(
        &mut self,
        name: &str,
        version: u32,
        srepr_sha256: &str,
        produced_by: (&str, u64),
        owner_entry: Option<&str>,
        portable: bool,
    ) -> HandleRef {
        let expr = SolveExpr {
            name: name.into(),
            version,
            srepr_blob: format!("blob-{name}-{version}"),
            srepr_sha256: srepr_sha256.into(),
            latex: name.into(),
            str_preview: name.into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: produced_by.0.into(), result_seq: produced_by.1 },
            owner_entry: owner_entry.map(str::to_string),
            portable,
        };
        let handle = expr.handle().unwrap();
        self.emit(event_kinds::SOLVE_EXPR, serde_json::to_value(expr).unwrap());
        handle
    }

    fn add_step(
        &mut self,
        id: &str,
        entry_type: StepEntryType,
        origin: Option<StepOrigin>,
        edges: (&[&str], &[&str]),
        claims: Vec<Claim>,
        summary: &str,
    ) -> (Option<String>, String) {
        let claim_sha = if claims.is_empty() {
            None
        } else {
            Some(
                compute_claim_sha256(&claims, |h| {
                    self.state.handles.get(h).map(|e| e.srepr_sha256.as_str())
                })
                .unwrap(),
            )
        };
        let entry_sha = compute_entry_sha256(claim_sha.as_deref(), summary, "details");
        self.emit(
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": id, "entry_type": entry_type, "summary": summary, "details": "details",
                "depends_on": edges.0, "derived_depends_on": edges.1, "claims": claims,
                "claim_sha256": claim_sha, "entry_sha256": entry_sha, "origin": origin,
            }),
        );
        (claim_sha, entry_sha)
    }

    fn add_check(
        &mut self,
        claim_sha: &str,
        claim_index: usize,
        entry_id: &str,
        name: &str,
        outcome: CheckOutcome,
        evidence: Value,
    ) {
        let math_sha = self.math_sha();
        self.emit(
            event_kinds::SOLVE_CHECK,
            serde_json::to_value(SolveCheck {
                check_id: format!("chk-{}", self.seq + 1),
                claim_sha256: claim_sha.into(),
                claim_index,
                entry_id: Some(entry_id.into()),
                math_sha256: math_sha,
                name: name.into(),
                registry_sha256: "reg-v1".into(),
                params: json!({}),
                outcome,
                asked_by: CheckRequester::Host,
                receipt: ReceiptRef { call_id: "c-chk".into(), result_seq: self.seq + 1 },
                evidence,
                evidence_sha256: "ev-sha".into(),
            })
            .unwrap(),
        );
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

#[test]
fn wording_constants_allow_verified_only_for_verified_badge_and_own_check_line() {
    assert_eq!(wording::BADGE_VERIFIED, "Verified");
    assert_eq!(wording::OWN_CHECK_VERIFIED, "own check: verified");
    assert_eq!(wording::BADGE_CHECKED_NUMERICALLY, "Numerically checked");
    assert_eq!(wording::BADGE_CITED, "Found");
    assert_eq!(serde_json::to_value(Tier::Verified).unwrap(), json!("verified"));
    assert_eq!(serde_json::to_value(Tier::CheckedNumerically).unwrap(), json!("checked_numerically"));
    assert_eq!(serde_json::to_value(Tier::Cited).unwrap(), json!("cited"));
    assert_eq!(serde_json::to_value(Tier::Failed).unwrap(), json!("failed"));
    assert_eq!(serde_json::to_value(Tier::Stale).unwrap(), json!("stale"));

    for &c in wording::NON_VERIFIED_CONSTANTS {
        let lower = c.to_ascii_lowercase();
        assert!(!lower.contains("verified"), "non-verified constant `{c}` contains `verified`");
        for word in lower.split(|ch: char| !ch.is_ascii_alphanumeric()) {
            assert!(
                !matches!(word, "proved" | "new" | "novel" | "computed"),
                "disallowed word `{word}` in constant `{c}`"
            );
        }
    }

    // Only machine-confirmed tiers receive DisplayGrade::ByCode (00-overview.md 2.10).
    for tier in [Tier::Verified, Tier::CheckedNumerically] {
        assert_eq!(tier.display_grade(), DisplayGrade::ByCode);
        assert_eq!(tier.border_style(), BorderStyle::Solid);
    }
    for tier in [
        Tier::Cited, Tier::Given, Tier::CheckedByYou, Tier::ModelAgreed,
        Tier::ModelAgreedSameModel, Tier::AssumedByModel, Tier::Unchecked,
        Tier::Stale, Tier::Failed,
    ] {
        assert_ne!(tier.display_grade(), DisplayGrade::ByCode, "{tier:?} must not use ByCode");
    }

    // Explicit tier order: Unchecked (0) < SameModel (1) < ModelAgreed/CheckedByYou (2)
    // < CheckedNumerically (3) < Verified (4).
    assert_eq!(Tier::Unchecked.ordered_rank(), Some(0));
    assert_eq!(Tier::ModelAgreedSameModel.ordered_rank(), Some(1));
    assert_eq!(Tier::ModelAgreed.ordered_rank(), Some(2));
    assert_eq!(Tier::CheckedByYou.ordered_rank(), Some(2));
    assert_eq!(Tier::CheckedNumerically.ordered_rank(), Some(3));
    assert_eq!(Tier::Verified.ordered_rank(), Some(4));
    assert_eq!(Tier::Failed.ordered_rank(), None);
    assert_eq!(Tier::Given.ordered_rank(), None);
    assert_eq!(Tier::Cited.ordered_rank(), None);
}

#[test]
fn r4_resaving_handle_drops_tier_and_rerecording_same_claim_keeps_checks() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    let a1 = h.add_expr("A", 1, "sha_a1", ("c1", r1), None, true);
    let b1 = h.add_expr("B", 1, "sha_b1", ("c1", r1), None, true);
    let (claim_sha, _) = h.add_step(
        "e-1", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]),
        vec![eq_claim(a1.clone(), b1.clone())], "first wording",
    );
    let claim_sha = claim_sha.unwrap();
    h.add_check(&claim_sha, 0, "e-1", "identity_symbolic", CheckOutcome::Pass, json!({"residual": "0"}));
    assert_eq!(h.state.tier("e-1").unwrap().effective, Tier::Verified);
    assert_eq!(h.state.badge("e-1").unwrap().label, wording::BADGE_VERIFIED);
    assert_eq!(h.state.badge("e-1").unwrap().own_check_line, None);

    // Re-recording the exact same claim on e-2 KEEPS the passing check (R4).
    h.add_step(
        "e-2", StepEntryType::Correction, Some(StepOrigin::Solver), (&[], &[]),
        vec![eq_claim(a1.clone(), b1.clone())], "clearer wording",
    );
    assert_eq!(h.state.tier("e-2").unwrap().effective, Tier::Verified);

    // Re-saving handle A as version 2 drops tier.
    let a2 = h.add_expr("A", 2, "sha_a2", ("c1", r1), None, true);
    h.add_step(
        "e-3", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]),
        vec![eq_claim(a2, b1)], "uses new handle version",
    );
    assert_eq!(h.state.tier("e-3").unwrap().effective, Tier::Unchecked);

    // Re-saving handle A@1 in place with a different srepr_sha256 drops e-2 to Stale.
    h.add_expr("A", 1, "sha_a1_mutated", ("c1", r1), None, true);
    assert_eq!(h.state.tier("e-2").unwrap().effective, Tier::Stale);
    assert!(h.state.tier("e-2").unwrap().stale);
}

#[test]
fn r4_two_claim_step_mixed_tiers_and_setup_revision_staleness() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    let a = h.add_expr("A", 1, "sa", ("c1", r1), None, true);
    let b = h.add_expr("B", 1, "sb", ("c1", r1), None, true);
    let c = h.add_expr("C", 1, "sc", ("c1", r1), None, true);
    let (claim_sha, _) = h.add_step(
        "e-1", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]),
        vec![eq_claim(a, b.clone()), eq_claim(b, c)], "two claims",
    );
    let claim_sha = claim_sha.unwrap();

    // Claim 0 symbolic alone does not lift a two-claim step (R4).
    h.add_check(&claim_sha, 0, "e-1", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::Unchecked);

    // Claim 1 numeric (12 pts, 50 digits) -> mixed symbolic + numeric gives CheckedNumerically (R8).
    h.add_check(&claim_sha, 1, "e-1", "identity_numeric", CheckOutcome::Pass, json!({"points": 12, "digits": 50}));
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::CheckedNumerically);
    assert_eq!(h.state.badge("e-1").unwrap().label, wording::BADGE_CHECKED_NUMERICALLY);

    // Claim 1 also passes symbolic -> best per claim is Verified for both -> step becomes Verified!
    h.add_check(&claim_sha, 1, "e-1", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::Verified);

    // Setup revision changing an assumption changes math_sha256 -> checks become Stale (R4, D15).
    h.set_setup(2, "Euclidean, hbar = c = 1", "m can be negative");
    let tier = h.state.tier("e-1").unwrap();
    assert!(tier.stale);
    assert_eq!(tier.own, Tier::Unchecked);
    assert_eq!(tier.effective, Tier::Stale);
}

#[test]
fn r8_weak_checks_numeric_guard_series_exact_vs_numeric_and_value_runs() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    let a = h.add_expr("A", 1, "sa", ("c1", r1), None, true);
    let b = h.add_expr("B", 1, "sb", ("c1", r1), None, true);
    let c = h.add_expr("C", 1, "sc", ("c1", r1), None, true);

    // 1. Only dimensions + symmetry -> Unchecked with sublabels.
    let (c_eq, _) = h.add_step(
        "e-1", StepEntryType::Result, Some(StepOrigin::Solver), (&[], &[]),
        vec![eq_claim(a.clone(), b.clone())], "eq",
    );
    let c_eq = c_eq.unwrap();
    h.add_check(&c_eq, 0, "e-1", "dimensions", CheckOutcome::Pass, json!({}));
    h.add_check(&c_eq, 0, "e-1", "symmetry", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::Unchecked);
    assert_eq!(h.state.tier("e-1").unwrap().sublabels, vec!["dimensions", "symmetry"]);

    // 2. Inequality with counterexample_search -> Unchecked with sublabel.
    let ineq = Claim {
        form: ClaimForm::Inequality, lhs: a.clone(), rhs: b.clone(), relation: ClaimRelation::Ge,
        order: None, domain: None, how: None,
    };
    let (c_ineq, _) = h.add_step("e-2", StepEntryType::Result, Some(StepOrigin::Solver), (&[], &[]), vec![ineq], "ineq");
    h.add_check(&c_ineq.unwrap(), 0, "e-2", "counterexample_search", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-2").unwrap().own, Tier::Unchecked);
    assert_eq!(h.state.tier("e-2").unwrap().sublabels, vec![wording::SUBLABEL_SAMPLED_NO_COUNTEREXAMPLE]);

    // 3. Equation numeric identity: below 12/50 -> Unchecked; at 12/50 -> CheckedNumerically (never Verified).
    h.add_check(&c_eq, 0, "e-1", "identity_numeric", CheckOutcome::Pass, json!({"points": 11, "digits": 50}));
    assert_eq!(h.state.tier("e-1").unwrap().own, Tier::Unchecked);
    h.add_check(&c_eq, 0, "e-1", "identity_numeric", CheckOutcome::Pass, json!({"points": 12, "digits": 50}));
    let t1 = h.state.tier("e-1").unwrap();
    assert_eq!(t1.own, Tier::CheckedNumerically);
    assert!(t1.sublabels.contains(&wording::SUBLABEL_NUMERIC_DEFAULT.to_string()));

    // 4. Verified equation whose numeric guard fails -> Failed (R8).
    let (c_guard, _) = h.add_step("e-g", StepEntryType::Result, Some(StepOrigin::Solver), (&[], &[]), vec![eq_claim(b.clone(), c.clone())], "branch cut");
    let c_guard = c_guard.unwrap();
    h.add_check(&c_guard, 0, "e-g", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-g").unwrap().own, Tier::Verified);
    h.add_check(&c_guard, 0, "e-g", "identity_numeric", CheckOutcome::Fail, json!({"counterexample": "x = -2"}));
    assert_eq!(h.state.tier("e-g").unwrap().own, Tier::Failed);

    // 5. Value claim two-run check -> CheckedNumerically (never Verified).
    let r2_same = h.add_sympy_run("c2", "code1");
    let r3_diff = h.add_sympy_run("c3", "code_independent");
    let v1 = h.add_expr("V1", 1, "sv1", ("c1", r1), None, true);
    let v2_same = h.add_expr("V2s", 1, "sv2s", ("c2", r2_same), None, true);
    let v2_diff = h.add_expr("V2d", 1, "sv2d", ("c3", r3_diff), None, true);
    let val_claim = |x, y| Claim { form: ClaimForm::Value, lhs: x, rhs: y, relation: ClaimRelation::Approx, order: None, domain: None, how: None };
    let (c_same, _) = h.add_step("e-3", StepEntryType::NumericalComputation, Some(StepOrigin::Solver), (&[], &[]), vec![val_claim(v1.clone(), v2_same)], "same");
    h.add_check(&c_same.unwrap(), 0, "e-3", "identity_numeric", CheckOutcome::Pass, json!({"digits": 12}));
    assert_eq!(h.state.tier("e-3").unwrap().own, Tier::Unchecked);
    let (c_diff, _) = h.add_step("e-4", StepEntryType::NumericalComputation, Some(StepOrigin::Solver), (&[], &[]), vec![val_claim(v1, v2_diff)], "diff");
    h.add_check(&c_diff.unwrap(), 0, "e-4", "identity_numeric", CheckOutcome::Pass, json!({"digits": 12}));
    assert_eq!(h.state.tier("e-4").unwrap().own, Tier::CheckedNumerically);
    assert!(h.state.tier("e-4").unwrap().sublabels.contains(&wording::SUBLABEL_TWO_RUNS_AGREE.to_string()));

    // 6. independent_recompute: explicit numeric -> CheckedNumerically, explicit exact -> Verified.
    let (c_ir, _) = h.add_step("e-5", StepEntryType::Result, Some(StepOrigin::Solver), (&[], &[]), vec![eq_claim(a.clone(), c)], "blind");
    let c_ir = c_ir.unwrap();
    h.add_check(&c_ir, 0, "e-5", "independent_recompute", CheckOutcome::Pass, json!({"comparison": "numeric", "exact": false}));
    assert_eq!(h.state.tier("e-5").unwrap().own, Tier::CheckedNumerically);
    h.add_check(&c_ir, 0, "e-5", "independent_recompute", CheckOutcome::Pass, json!({"exact": true}));
    assert_eq!(h.state.tier("e-5").unwrap().own, Tier::Verified);

    // 7. Non-portable handle can never reach a computed tier.
    let bad_h = h.add_expr("NonPort", 1, "snp", ("c1", r1), None, false);
    let (c_np, _) = h.add_step("e-6", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]), vec![eq_claim(bad_h, a)], "non-portable");
    h.add_check(&c_np.unwrap(), 0, "e-6", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-6").unwrap().own, Tier::Unchecked);
}

#[test]
fn r9_derived_dependency_lowers_verified_and_emits_own_check_verified_line() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    h.add_step("e-3", StepEntryType::Assumption, Some(StepOrigin::Solver), (&[], &[]), vec![], "unacked assumption");
    let a = h.add_expr("A", 1, "sa", ("c1", r1), Some("e-3"), true);
    let b = h.add_expr("B", 1, "sb", ("c1", r1), None, true);
    let (c_sha, _) = h.add_step(
        "e-4", StepEntryType::Result, Some(StepOrigin::Solver), (&[], &["e-3"]),
        vec![eq_claim(a, b)], "uses handle owned by e-3",
    );
    h.add_check(&c_sha.unwrap(), 0, "e-4", "identity_symbolic", CheckOutcome::Pass, json!({}));

    let badge = h.state.badge("e-4").unwrap();
    assert_eq!(badge.own, Tier::Verified);
    assert_eq!(badge.effective, Tier::Unchecked);
    assert_eq!(badge.label, wording::BADGE_UNCHECKED);
    assert_eq!(badge.own_check_line.as_deref(), Some(wording::OWN_CHECK_VERIFIED));
    assert_eq!(badge.lowered_by, vec!["e-3"]);
    assert_eq!(badge.rests_on_line.as_deref(), Some("rests on e-3 (assumed by model)"));
    assert!(badge.flags.contains(&"rests on 1 assumption made by the model".to_string()));
}

#[test]
fn fail_is_permanent_and_propagates_broken_notice_and_supersede_causes_stale() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    let a = h.add_expr("A", 1, "sa", ("c1", r1), None, true);
    let b = h.add_expr("B", 1, "sb", ("c1", r1), None, true);
    let c = h.add_expr("C", 1, "sc", ("c1", r1), None, true);
    let d = h.add_expr("D", 1, "sd", ("c1", r1), None, true);
    let e = h.add_expr("E", 1, "se", ("c1", r1), None, true);

    let (c6, _) = h.add_step("e-6", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]), vec![eq_claim(a, b.clone())], "dropped 1/2");
    let c6 = c6.unwrap();
    let (c7, _) = h.add_step("e-7", StepEntryType::Derivation, Some(StepOrigin::Solver), (&["e-6"], &[]), vec![eq_claim(b, c.clone())], "downstream");
    h.add_check(&c7.unwrap(), 0, "e-7", "identity_symbolic", CheckOutcome::Pass, json!({}));

    h.add_check(&c6, 0, "e-6", "series_match", CheckOutcome::Fail, json!({"counterexample": "0.0312"}));
    h.add_check(&c6, 0, "e-6", "identity_symbolic", CheckOutcome::Pass, json!({}));
    let b6 = h.state.badge("e-6").unwrap();
    assert_eq!(b6.label, wording::BADGE_FAILED);
    assert_eq!(b6.grade, DisplayGrade::ModelStatement);
    assert_eq!(b6.border, BorderStyle::None);
    let b7 = h.state.badge("e-7").unwrap();
    assert_eq!(b7.own, Tier::Verified);
    assert_eq!(b7.effective, Tier::Failed);
    assert_eq!(b7.label, "Broken: rests on failed e-6");
    assert_eq!(b7.own_check_line.as_deref(), Some(wording::OWN_CHECK_VERIFIED));
    assert_eq!(b7.grade, DisplayGrade::ModelStatement);
    assert_eq!(b7.border, BorderStyle::None);

    let (c8, _) = h.add_step("e-8", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]), vec![eq_claim(c, d.clone())], "ok step");
    h.add_check(&c8.unwrap(), 0, "e-8", "identity_symbolic", CheckOutcome::Pass, json!({}));
    let (c9, _) = h.add_step("e-9", StepEntryType::Result, Some(StepOrigin::Solver), (&["e-8"], &[]), vec![eq_claim(d, e)], "uses e-8");
    h.add_check(&c9.unwrap(), 0, "e-9", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-9").unwrap().effective, Tier::Verified);
    h.emit(event_kinds::PROOF_SUPERSEDE, json!({"entry_id": "e-8", "replaced_by": "e-8b", "reason": "cleanup"}));
    let b9 = h.state.badge("e-9").unwrap();
    assert!(b9.stale);
    assert_eq!(b9.effective, Tier::Stale);
    assert_eq!(b9.grade, DisplayGrade::ModelStatement);
    assert_eq!(b9.border, BorderStyle::None);
}

#[test]
fn given_cited_ocr_and_earlier_lean_receipt_and_r24_lessons_invariance() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    h.add_step("e-1", StepEntryType::Assumption, Some(StepOrigin::Setup), (&[], &[]), vec![], "setup");

    // Full-text matched quote -> stored tier Cited, rendered as "Found" (MatchedQuoteFullText).
    let q_seq = h.emit(
        event_kinds::EVIDENCE_QUOTE,
        json!({"source_id": "src-1", "path": "paper.tex", "start_line": 10, "end_line": 12, "file_sha256": "f".repeat(64), "quote_sha256": "q".repeat(64)}),
    );
    let qref = QuoteRef {
        source_id: "src-1".into(), path: "paper.tex".into(), start_line: 10, end_line: 12,
        file_sha256: "f".repeat(64), quote_sha256: "q".repeat(64), evidence_seq: Some(q_seq),
    };
    h.emit(
        event_kinds::PROOF_ENTRY,
        json!({"entry_id": "e-2", "entry_type": "reference_lookup", "summary": "cited", "details": "d", "depends_on": [], "source_ref": qref}),
    );
    let b2 = h.state.badge("e-2").unwrap();
    assert_eq!(b2.effective, Tier::Cited);
    assert_eq!(b2.label, "Found");
    assert_eq!(b2.grade, DisplayGrade::MatchedQuoteFullText);
    assert_eq!(b2.border, BorderStyle::Solid);

    // OCR scanned-page quote -> stored tier Cited, rendered as "Found" + MatchedQuoteScannedPage + "OCR" tag + "OCR source" flag.
    let q_ocr_seq = h.emit(
        event_kinds::EVIDENCE_QUOTE,
        json!({"source_id": "src-2", "path": "scan.pdf", "start_line": 3, "end_line": 4, "file_sha256": "c".repeat(64), "quote_sha256": "d".repeat(64), "ocr": true}),
    );
    let qref_ocr = QuoteRef {
        source_id: "src-2".into(), path: "scan.pdf".into(), start_line: 3, end_line: 4,
        file_sha256: "c".repeat(64), quote_sha256: "d".repeat(64), evidence_seq: Some(q_ocr_seq),
    };
    h.emit(
        event_kinds::PROOF_ENTRY,
        json!({"entry_id": "e-2b", "entry_type": "reference_lookup", "summary": "ocr cited", "details": "d", "depends_on": [], "source_ref": qref_ocr}),
    );
    let b_ocr = h.state.badge("e-2b").unwrap();
    assert_eq!(b_ocr.effective, Tier::Cited);
    assert_eq!(b_ocr.label, "Found");
    assert_eq!(b_ocr.grade, DisplayGrade::MatchedQuoteScannedPage);
    assert_eq!(b_ocr.provenance_tag.as_deref(), Some(wording::TAG_OCR));
    assert!(b_ocr.flags.contains(&wording::FLAG_OCR_SOURCE.to_string()));

    // Verified step depending on Given + Cited + OCR Cited stays Verified.
    let a = h.add_expr("A", 1, "sa", ("c1", r1), None, true);
    let b = h.add_expr("B", 1, "sb", ("c1", r1), None, true);
    let (c3, _) = h.add_step("e-3", StepEntryType::Result, Some(StepOrigin::Solver), (&["e-1", "e-2", "e-2b"], &[]), vec![eq_claim(a, b)], "over neutral");
    h.add_check(&c3.unwrap(), 0, "e-3", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.tier("e-3").unwrap().effective, Tier::Verified);

    // Earlier `lean_check` tool result is stored as a receipt and gives NO tier (D19).
    h.emit(event_kinds::TOOL_REQUEST, json!({"call_id": "c-lean", "tool": "lean_check"}));
    h.emit(event_kinds::TOOL_RESULT, json!({"call_id": "c-lean", "ok": true, "result": {"status": "verified"}}));
    h.add_step("e-4", StepEntryType::Derivation, Some(StepOrigin::Solver), (&[], &[]), vec![], "old lean step");
    assert!(h.state.receipts.contains_key("c-lean"));
    assert_eq!(h.state.tier("e-4").unwrap().effective, Tier::Unchecked);

    // R24: adding `lessons.supplied` changes no tier or badge.
    let tiers_before = h.state.tiers.clone();
    h.emit(event_kinds::LESSONS_SUPPLIED, json!({"run_id": "r1", "lesson_ids": ["l-1", "l-2"], "blob": "b"}));
    assert_eq!(h.state.tiers, tiers_before);
}

#[test]
fn worked_example_series_exact_vs_numeric_and_transcription_and_ran_flags() {
    let mut h = Harness::new();
    let r1 = h.add_sympy_run("c1", "code1");
    h.add_step("e-1", StepEntryType::Assumption, Some(StepOrigin::Setup), (&[], &[]), vec![], "Euclidean");
    let (_, e3_sha) = h.add_step("e-3", StepEntryType::Assumption, Some(StepOrigin::Solver), (&[], &[]), vec![], "one-loop formula");
    let (_, e4_sha) = h.add_step("e-4", StepEntryType::Derivation, Some(StepOrigin::Solver), (&["e-3"], &[]), vec![], "d-dim integral def");

    let lhs = h.add_expr("V1_series", 1, "sv1s", ("c1", r1), Some("e-4"), true);
    let rhs = h.add_expr("V1_msbar", 1, "sv1m", ("c1", r1), None, true);
    let series_claim = Claim {
        form: ClaimForm::Series, lhs, rhs, relation: ClaimRelation::Eq,
        order: Some(SeriesOrder { variable: "eps".into(), point: "0".into(), n: 0 }),
        domain: None, how: None,
    };
    let (c7, _) = h.add_step(
        "e-7", StepEntryType::Result, Some(StepOrigin::Solver), (&["e-1"], &["e-4"]),
        vec![series_claim], "MS-bar potential",
    );
    let c7 = c7.unwrap();
    // Numeric series comparison -> CheckedNumerically + "to O(eps^0), numeric coefficients" + flag "to O(eps^0)".
    h.add_check(&c7, 0, "e-7", "series_match", CheckOutcome::Pass, json!({"order": {"variable": "eps", "point": "0", "n": 0}, "numeric_coefficients": true}));
    assert_eq!(h.state.tier("e-7").unwrap().own, Tier::CheckedNumerically);
    assert!(h.state.tier("e-7").unwrap().sublabels.contains(&"to O(eps^0), numeric coefficients".to_string()));
    assert!(h.state.tier("e-7").unwrap().flags.contains(&"to O(eps^0)".to_string()));

    // Exact series comparison lifts own tier to Verified (best passing check for the claim).
    h.add_check(&c7, 0, "e-7", "series_match", CheckOutcome::Pass, json!({"order": {"variable": "eps", "point": "0", "n": 0}, "exact": true}));
    h.add_check(&c7, 0, "e-7", "dimensions", CheckOutcome::Pass, json!({}));

    let b7 = h.state.badge("e-7").unwrap();
    assert_eq!(b7.own, Tier::Verified);
    assert_eq!(b7.effective, Tier::Unchecked);
    assert_eq!(b7.own_check_line.as_deref(), Some(wording::OWN_CHECK_VERIFIED));
    assert!(b7.sublabels.contains(&"to O(eps^0)".to_string()));
    assert!(b7.sublabels.contains(&"dimensions".to_string()));
    assert!(b7.flags.contains(&"rests on 1 assumption made by the model".to_string()));

    h.emit(event_kinds::SOLVE_ACK, json!({"entry_id": "e-3", "kind": "assumption", "entry_sha256": e3_sha}));
    h.emit(
        event_kinds::PROOF_VERIFICATION,
        json!({"target_entry_id": "e-4", "entry_sha256": e4_sha, "verdict": "accepted", "independence": "different_provider"}),
    );
    assert_eq!(h.state.badge("e-7").unwrap().effective, Tier::ModelAgreed);
    assert_eq!(h.state.badge("e-7").unwrap().flags, vec!["to O(eps^0)"]);
    assert_eq!(rests_on_summary(&h.state, "e-7").given, vec!["e-1", "e-3"]);

    let e8_sha = compute_entry_sha256(None, "ran step", "d");
    h.emit(
        event_kinds::PROOF_ENTRY,
        json!({
            "entry_id": "e-8", "entry_type": "symbolic_computation", "summary": "ran step", "details": "d",
            "tool_call_refs": ["c1"], "entry_sha256": e8_sha,
            "transcription": TranscriptionMeta { source_tex_sha256: "t".repeat(64) },
        }),
    );
    let b8 = h.state.badge("e-8").unwrap();
    assert_eq!(b8.own, Tier::Unchecked);
    assert!(b8.sublabels.contains(&wording::SUBLABEL_RAN.to_string()));
    assert!(b8.flags.contains(&wording::FLAG_TRANSCRIPTION_UNCONFIRMED.to_string()));
    h.emit(event_kinds::SOLVE_ACK, json!({"entry_id": "e-8", "kind": "transcription", "entry_sha256": e8_sha}));
    assert!(!h.state.badge("e-8").unwrap().flags.contains(&wording::FLAG_TRANSCRIPTION_UNCONFIRMED.to_string()));
}
