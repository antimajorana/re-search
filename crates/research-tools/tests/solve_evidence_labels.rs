//! Integration tests for Solve accepted evidence labels (`00-overview.md` 2.10,
//! owner clarification 2026-11-26, `solve.md` 4.4).
//!
//! Verifies through the public `research_tools::solve_{model,state,status}` API that:
//! - a matched source (`cited`) renders as `"Found"` (preserving source/OCR flags),
//! - an exact symbolic check renders as `"Verified"`,
//! - a numeric check renders as `"Numerically checked"`,
//! - serialized `Tier` values and existing event shapes stay unchanged,
//! - citations, model opinions, tool successes without a claim check, stale checks,
//!   and failed checks are never styled as machine-confirmed claims (`DisplayGrade::ByCode`),
//! - inheritance and failure propagation preserve exact labels and wording rules.

use ledger::Event;
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, CheckOutcome,
    CheckRequester, Claim, ClaimForm, ClaimRelation, HandleRef, QuoteRef, ReceiptRef,
    ReviewPolicy, RoleModel, SetupAssumption, SolveBudgets, SolveCheck, SolveExpr, SolveRoles,
    SolveSetup, SolveTarget, StepEntryType, StepOrigin, SymbolSpec, TargetForm, TaskKind,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{
    badge_for_step, rests_on_summary, wording, BorderStyle, DisplayGrade, StepBadge, Tier,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct SessionHarness {
    seq: u64,
    state: SolveState,
}

impl SessionHarness {
    fn new() -> Self {
        let mut h = Self { seq: 0, state: SolveState::new() };
        h.emit(event_kinds::SOLVE_STARTED, json!({}));
        h.set_setup(1, "Euclidean, natural units", "x > 0");
        h
    }

    fn emit(&mut self, kind: &str, payload: Value) -> u64 {
        self.seq += 1;
        let ev = Event {
            session: "s-evidence-labels".into(),
            seq: self.seq,
            ts_ms: (self.seq as i64) * 100,
            kind: kind.into(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "a".repeat(64),
        };
        self.state.apply(&ev);
        self.seq
    }

    fn math_sha(&self) -> String {
        self.state.setup.as_ref().unwrap().math_sha256.clone()
    }

    fn set_setup(&mut self, revision: u32, conventions: &str, assumption: &str) {
        let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["positive".into()] }];
        let assumptions = vec![SetupAssumption { id: "a-1".into(), text: assumption.into() }];
        let math_sha256 =
            compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, conventions);
        let setup = SolveSetup {
            revision,
            statement: "Synthetic identity check".into(),
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
                solver: RoleModel { provider: "provider-a".into(), model: "solver-1".into() },
                verifier: RoleModel { provider: "provider-b".into(), model: "verifier-1".into() },
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
                "call_id": call_id,
                "ok": true,
                "result": {
                    "value": "0",
                    "error": null,
                    "code_sha256": code_sha256,
                    "process": "solver"
                }
            }),
        )
    }

    fn add_expr(&mut self, name: &str, version: u32, srepr_sha: &str, run: (&str, u64)) -> HandleRef {
        let expr = SolveExpr {
            name: name.into(),
            version,
            srepr_blob: format!("blob-{name}-{version}"),
            srepr_sha256: srepr_sha.into(),
            latex: name.into(),
            str_preview: name.into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: run.0.into(), result_seq: run.1 },
            owner_entry: None,
            portable: true,
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
        depends_on: &[&str],
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
        let entry_sha = compute_entry_sha256(claim_sha.as_deref(), summary, "synthetic details");
        self.emit(
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": id,
                "entry_type": entry_type,
                "summary": summary,
                "details": "synthetic details",
                "depends_on": depends_on,
                "claims": claims,
                "claim_sha256": claim_sha,
                "entry_sha256": entry_sha,
                "origin": origin,
            }),
        );
        (claim_sha, entry_sha)
    }

    fn add_check(
        &mut self,
        claim_sha: &str,
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
                claim_index: 0,
                entry_id: Some(entry_id.into()),
                math_sha256: math_sha,
                name: name.into(),
                registry_sha256: "reg-v1".into(),
                params: json!({}),
                outcome,
                asked_by: CheckRequester::Host,
                receipt: ReceiptRef { call_id: format!("c-chk-{}", self.seq + 1), result_seq: self.seq + 1 },
                evidence,
                evidence_sha256: "ev-sha-v1".into(),
            })
            .unwrap(),
        );
    }

    fn add_matched_quote(&mut self, id: &str, source_id: &str, path: &str, ocr: bool, depends_on: &[&str]) {
        let file_sha = format!("{source_id:0>64}");
        let quote_sha = format!("{id:0>64}");
        let q_seq = self.emit(
            event_kinds::EVIDENCE_QUOTE,
            json!({
                "source_id": source_id,
                "path": path,
                "start_line": 12,
                "end_line": 15,
                "file_sha256": file_sha,
                "quote_sha256": quote_sha,
                "ocr": ocr,
            }),
        );
        let qref = QuoteRef {
            source_id: source_id.into(),
            path: path.into(),
            start_line: 12,
            end_line: 15,
            file_sha256: file_sha,
            quote_sha256: quote_sha,
            evidence_seq: Some(q_seq),
        };
        self.emit(
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": id,
                "entry_type": "reference_lookup",
                "summary": format!("matched quote {id}"),
                "details": "synthetic citation",
                "depends_on": depends_on,
                "source_ref": qref,
            }),
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

fn assert_not_machine_confirmed(badge: &StepBadge, context: &str) {
    assert_ne!(badge.grade, DisplayGrade::ByCode, "{context}: must not have ByCode grade");
    assert_ne!(badge.label, "Verified", "{context}: must not be labelled Verified");
    assert_ne!(
        badge.label, "Numerically checked",
        "{context}: must not be labelled Numerically checked"
    );
    assert!(
        !badge.label.to_ascii_lowercase().contains("verified"),
        "{context}: label `{}` must not contain `verified`",
        badge.label
    );
}

#[test]
fn matched_source_exact_symbolic_and_numeric_checks_get_distinct_labels_with_unchanged_serde() {
    let mut h = SessionHarness::new();
    let r1 = h.add_sympy_run("c1", "code-1");
    let a = h.add_expr("A", 1, "sha-a", ("c1", r1));
    let b = h.add_expr("B", 1, "sha-b", ("c1", r1));
    let c = h.add_expr("C", 1, "sha-c", ("c1", r1));

    // 1. Source-backed cited step (full text and OCR).
    h.add_matched_quote("e-1", "src-tex", "paper.tex", false, &[]);
    h.add_matched_quote("e-2", "src-pdf", "scan.pdf", true, &[]);

    // 2. Exact symbolic receipt -> Verified.
    let (c_sym, _) = h.add_step(
        "e-3", StepEntryType::Derivation, Some(StepOrigin::Solver), &[],
        vec![eq_claim(a.clone(), b.clone())], "exact symbolic identity",
    );
    h.add_check(&c_sym.unwrap(), "e-3", "identity_symbolic", CheckOutcome::Pass, json!({"residual": "0"}));

    // 3. Qualifying numeric check -> Numerically checked.
    let (c_num, _) = h.add_step(
        "e-4", StepEntryType::Derivation, Some(StepOrigin::Solver), &[],
        vec![eq_claim(b, c)], "numeric identity",
    );
    h.add_check(
        &c_num.unwrap(), "e-4", "identity_numeric", CheckOutcome::Pass,
        json!({"points": 16, "digits": 50}),
    );

    let b_src = badge_for_step(&h.state, "e-1").unwrap();
    let b_ocr = badge_for_step(&h.state, "e-2").unwrap();
    let b_sym = badge_for_step(&h.state, "e-3").unwrap();
    let b_num = badge_for_step(&h.state, "e-4").unwrap();

    // Distinct accepted labels.
    assert_eq!(b_src.label, "Found");
    assert_eq!(b_ocr.label, "Found");
    assert_eq!(b_sym.label, "Verified");
    assert_eq!(b_num.label, "Numerically checked");
    assert_ne!(b_src.label, b_sym.label);
    assert_ne!(b_src.label, b_num.label);
    assert_ne!(b_sym.label, b_num.label);

    // Distinct display grades & preserved OCR flags/tags.
    assert_eq!(b_src.grade, DisplayGrade::MatchedQuoteFullText);
    assert_eq!(b_src.border, BorderStyle::Solid);
    assert_eq!(b_src.provenance_tag, None);
    assert!(b_src.flags.is_empty());

    assert_eq!(b_ocr.grade, DisplayGrade::MatchedQuoteScannedPage);
    assert_eq!(b_ocr.border, BorderStyle::Solid);
    assert_eq!(b_ocr.provenance_tag.as_deref(), Some(wording::TAG_OCR));
    assert!(b_ocr.flags.contains(&wording::FLAG_OCR_SOURCE.to_string()));

    assert_eq!(b_sym.grade, DisplayGrade::ByCode);
    assert_eq!(b_sym.border, BorderStyle::Solid);
    assert_eq!(b_num.grade, DisplayGrade::ByCode);
    assert_eq!(b_num.border, BorderStyle::Solid);
    assert!(b_num.sublabels.contains(&"16 points, 50 digits".to_string()));

    // Serialized tiers stay unchanged (`cited`, `verified`, `checked_numerically`).
    assert_eq!(serde_json::to_value(b_src.effective).unwrap(), json!("cited"));
    assert_eq!(serde_json::to_value(b_sym.effective).unwrap(), json!("verified"));
    assert_eq!(serde_json::to_value(b_num.effective).unwrap(), json!("checked_numerically"));
    assert_eq!(serde_json::from_value::<Tier>(json!("cited")).unwrap(), Tier::Cited);
    assert_eq!(serde_json::from_value::<Tier>(json!("verified")).unwrap(), Tier::Verified);
    assert_eq!(
        serde_json::from_value::<Tier>(json!("checked_numerically")).unwrap(),
        Tier::CheckedNumerically
    );
}

#[test]
fn non_machine_confirmed_steps_are_never_styled_as_by_code_or_check_badges() {
    let mut h = SessionHarness::new();
    let r1 = h.add_sympy_run("c1", "code-1");
    let a = h.add_expr("A", 1, "sha-a", ("c1", r1));
    let b = h.add_expr("B", 1, "sha-b", ("c1", r1));
    let c = h.add_expr("C", 1, "sha-c", ("c1", r1));

    // 1. Matched citation (full-text and OCR): Found, NOT ByCode.
    h.add_matched_quote("e-cite", "src-1", "paper.tex", false, &[]);
    h.add_matched_quote("e-cite-ocr", "src-2", "scan.pdf", true, &[]);
    assert_not_machine_confirmed(&h.state.badge("e-cite").unwrap(), "matched citation");
    assert_not_machine_confirmed(&h.state.badge("e-cite-ocr").unwrap(), "matched OCR citation");

    // 2. Unmatched citation (no matching evidence.quote) -> Unchecked, NOT Found and NOT ByCode.
    h.emit(
        event_kinds::PROOF_ENTRY,
        json!({
            "entry_id": "e-unmatched-cite",
            "entry_type": "reference_lookup",
            "summary": "unmatched quote",
            "details": "d",
            "depends_on": [],
            "source_ref": {
                "source_id": "src-missing", "path": "x.tex", "start_line": 1, "end_line": 2,
                "file_sha256": "f".repeat(64), "quote_sha256": "q".repeat(64), "evidence_seq": 999
            }
        }),
    );
    let b_unmatched = h.state.badge("e-unmatched-cite").unwrap();
    assert_eq!(b_unmatched.effective, Tier::Unchecked);
    assert_eq!(b_unmatched.label, "Unchecked");
    assert_eq!(b_unmatched.border, BorderStyle::Dashed);
    assert_not_machine_confirmed(&b_unmatched, "unmatched citation");

    // 3. Model opinions: ModelAgreed, ModelAgreedSameModel, AssumedByModel.
    let (_, sha_m1) = h.add_step("e-mod-diff", StepEntryType::Derivation, Some(StepOrigin::Solver), &[], vec![], "prose 1");
    h.emit(event_kinds::PROOF_VERIFICATION, json!({"target_entry_id": "e-mod-diff", "entry_sha256": sha_m1, "verdict": "accepted", "independence": "different_provider"}));
    let (_, sha_m2) = h.add_step("e-mod-same", StepEntryType::Derivation, Some(StepOrigin::Solver), &[], vec![], "prose 2");
    h.emit(event_kinds::PROOF_VERIFICATION, json!({"target_entry_id": "e-mod-same", "entry_sha256": sha_m2, "verdict": "accepted", "independence": "same_model"}));
    h.add_step("e-mod-assume", StepEntryType::Assumption, Some(StepOrigin::Solver), &[], vec![], "model assumption");
    for id in ["e-mod-diff", "e-mod-same", "e-mod-assume"] {
        let badge = h.state.badge(id).unwrap();
        assert_eq!(badge.grade, DisplayGrade::ModelStatement, "{id}");
        assert_eq!(badge.border, BorderStyle::Dashed, "{id}");
        assert_not_machine_confirmed(&badge, id);
    }

    // 4. Tool success without a claim check ("ran"), plus weak/under-threshold checks.
    h.emit(
        event_kinds::PROOF_ENTRY,
        json!({
            "entry_id": "e-ran", "entry_type": "symbolic_computation", "summary": "ran code",
            "details": "d", "depends_on": [], "tool_call_refs": ["c1"]
        }),
    );
    let b_ran = h.state.badge("e-ran").unwrap();
    assert_eq!(b_ran.effective, Tier::Unchecked);
    assert!(b_ran.sublabels.contains(&wording::SUBLABEL_RAN.to_string()));
    assert_eq!(b_ran.border, BorderStyle::Dashed);
    assert_not_machine_confirmed(&b_ran, "tool success without claim check");

    let (c_weak, _) = h.add_step("e-weak", StepEntryType::Result, Some(StepOrigin::Solver), &[], vec![eq_claim(a.clone(), b.clone())], "weak");
    let c_weak = c_weak.unwrap();
    h.add_check(&c_weak, "e-weak", "dimensions", CheckOutcome::Pass, json!({}));
    h.add_check(&c_weak, "e-weak", "identity_numeric", CheckOutcome::Pass, json!({"points": 11, "digits": 50}));
    let b_weak = h.state.badge("e-weak").unwrap();
    assert_eq!(b_weak.effective, Tier::Unchecked);
    assert_eq!(b_weak.border, BorderStyle::Dashed);
    assert_not_machine_confirmed(&b_weak, "weak/under-threshold checks");

    // 5. Failed check (including symbolic pass overridden by failed numeric guard).
    let (c_fail, _) = h.add_step("e-fail", StepEntryType::Result, Some(StepOrigin::Solver), &[], vec![eq_claim(a.clone(), c.clone())], "failed guard");
    let c_fail = c_fail.unwrap();
    h.add_check(&c_fail, "e-fail", "identity_symbolic", CheckOutcome::Pass, json!({}));
    h.add_check(&c_fail, "e-fail", "identity_numeric", CheckOutcome::Fail, json!({"counterexample": "x = -1"}));
    let b_fail = h.state.badge("e-fail").unwrap();
    assert_eq!(b_fail.effective, Tier::Failed);
    assert_eq!(b_fail.label, "Failed");
    assert_eq!(b_fail.grade, DisplayGrade::ModelStatement);
    assert_eq!(b_fail.border, BorderStyle::None);
    assert_not_machine_confirmed(&b_fail, "failed check");

    // 6. Stale check (setup revision alters math_sha256).
    let (c_stale, _) = h.add_step("e-stale", StepEntryType::Result, Some(StepOrigin::Solver), &[], vec![eq_claim(b, c)], "will go stale");
    h.add_check(&c_stale.unwrap(), "e-stale", "identity_symbolic", CheckOutcome::Pass, json!({}));
    assert_eq!(h.state.badge("e-stale").unwrap().label, "Verified");
    h.set_setup(2, "Euclidean, natural units", "x is complex");
    let b_stale = h.state.badge("e-stale").unwrap();
    assert!(b_stale.stale);
    assert_eq!(b_stale.effective, Tier::Stale);
    assert_eq!(b_stale.label, "Stale");
    assert_eq!(b_stale.grade, DisplayGrade::ModelStatement);
    assert_eq!(b_stale.border, BorderStyle::None);
    assert_not_machine_confirmed(&b_stale, "stale check");
}

#[test]
fn inheritance_and_failure_propagation_preserve_distinct_labels_and_wording_rules() {
    let mut h = SessionHarness::new();
    let r1 = h.add_sympy_run("c1", "code-1");
    let a = h.add_expr("A", 1, "sha-a", ("c1", r1));
    let b = h.add_expr("B", 1, "sha-b", ("c1", r1));
    let c = h.add_expr("C", 1, "sha-c", ("c1", r1));
    let d = h.add_expr("D", 1, "sha-d", ("c1", r1));

    // Neutral bases: Given + Found (full-text) + Found (OCR).
    h.add_step("e-given", StepEntryType::Assumption, Some(StepOrigin::Setup), &[], vec![], "setup assumption");
    h.add_matched_quote("e-found", "src-1", "paper.tex", false, &[]);
    h.add_matched_quote("e-found-ocr", "src-2", "scan.pdf", true, &[]);

    // Numeric step resting on neutral bases -> Numerically checked.
    let (c_num, _) = h.add_step(
        "e-num", StepEntryType::Derivation, Some(StepOrigin::Solver),
        &["e-given", "e-found", "e-found-ocr"], vec![eq_claim(a.clone(), b.clone())], "numeric base",
    );
    h.add_check(&c_num.unwrap(), "e-num", "identity_numeric", CheckOutcome::Pass, json!({"points": 12, "digits": 50}));

    // Symbolically verified step resting on e-num -> lowered to Numerically checked with own_check_line.
    let (c_ver, _) = h.add_step(
        "e-ver", StepEntryType::Result, Some(StepOrigin::Solver),
        &["e-num"], vec![eq_claim(b.clone(), c.clone())], "symbolic over numeric",
    );
    h.add_check(&c_ver.unwrap(), "e-ver", "identity_symbolic", CheckOutcome::Pass, json!({}));

    let b_ver = h.state.badge("e-ver").unwrap();
    assert_eq!(b_ver.own, Tier::Verified);
    assert_eq!(b_ver.effective, Tier::CheckedNumerically);
    assert_eq!(b_ver.label, "Numerically checked");
    assert_eq!(b_ver.grade, DisplayGrade::ByCode);
    assert_eq!(b_ver.own_check_line.as_deref(), Some(wording::OWN_CHECK_VERIFIED));
    assert_eq!(b_ver.rests_on_line.as_deref(), Some("rests on e-num (numerically checked)"));
    let summary = rests_on_summary(&h.state, "e-ver");
    assert_eq!(summary.given, vec!["e-given"]);
    assert_eq!(summary.cited, vec!["e-found", "e-found-ocr"]);
    assert_eq!(summary.weakest.len(), 1);
    assert_eq!(summary.weakest[0].entry_id, "e-num");
    assert_eq!(summary.weakest[0].tier, "Numerically checked");

    // Failed step breaks both a downstream Verified step and a downstream Cited step.
    let (c_bad, _) = h.add_step("e-bad", StepEntryType::Derivation, Some(StepOrigin::Solver), &[], vec![eq_claim(c.clone(), d.clone())], "bad");
    h.add_check(&c_bad.unwrap(), "e-bad", "identity_symbolic", CheckOutcome::Fail, json!({"residual": "1"}));

    let (c_down, _) = h.add_step("e-down", StepEntryType::Result, Some(StepOrigin::Solver), &["e-bad"], vec![eq_claim(a, d.clone())], "downstream symbolic");
    h.add_check(&c_down.unwrap(), "e-down", "identity_symbolic", CheckOutcome::Pass, json!({}));
    h.add_matched_quote("e-cite-down", "src-3", "other.tex", true, &["e-bad"]);

    let b_down = h.state.badge("e-down").unwrap();
    assert_eq!(b_down.effective, Tier::Failed);
    assert_eq!(b_down.label, "Broken: rests on failed e-bad");
    assert_eq!(b_down.own_check_line.as_deref(), Some(wording::OWN_CHECK_VERIFIED));
    assert_eq!(b_down.grade, DisplayGrade::ModelStatement);
    assert_eq!(b_down.border, BorderStyle::None);
    assert_not_machine_confirmed(&b_down, "broken verified step");

    let b_cite_down = h.state.badge("e-cite-down").unwrap();
    assert_eq!(b_cite_down.effective, Tier::Failed);
    assert_eq!(b_cite_down.label, "Broken: rests on failed e-bad");
    assert_eq!(b_cite_down.own_check_line, None);
    assert_eq!(b_cite_down.grade, DisplayGrade::ModelStatement);
    assert_eq!(b_cite_down.border, BorderStyle::None);
    assert!(b_cite_down.flags.contains(&wording::FLAG_OCR_SOURCE.to_string()));
    assert_not_machine_confirmed(&b_cite_down, "broken cited step");

    // User statements (`Given`, `Checked by you`) use UserStatement + Plain + "you", never ByCode.
    let b_given = h.state.badge("e-given").unwrap();
    assert_eq!(b_given.label, "Given");
    assert_eq!(b_given.grade, DisplayGrade::UserStatement);
    assert_eq!(b_given.border, BorderStyle::Plain);
    assert_eq!(b_given.provenance_tag.as_deref(), Some(wording::TAG_YOU));
    assert_not_machine_confirmed(&b_given, "given step");

    let (_, sha_hand) = h.add_step("e-hand", StepEntryType::Derivation, Some(StepOrigin::Solver), &[], vec![], "hand checked");
    h.emit(event_kinds::SOLVE_ACK, json!({"entry_id": "e-hand", "kind": "hand_checked", "entry_sha256": sha_hand}));
    let b_hand = h.state.badge("e-hand").unwrap();
    assert_eq!(b_hand.label, "Checked by you");
    assert_eq!(b_hand.grade, DisplayGrade::UserStatement);
    assert_eq!(b_hand.border, BorderStyle::Plain);
    assert_eq!(b_hand.provenance_tag.as_deref(), Some(wording::TAG_YOU));
    assert_not_machine_confirmed(&b_hand, "hand-checked step");

    // CheckedNumerically and OCR Cited lowered by Unchecked lose solid/check/Found styling,
    // keep `own_check_line == None`, and OCR keeps its `"OCR source"` flag.
    h.add_step("e-unc", StepEntryType::Derivation, Some(StepOrigin::Solver), &[], vec![], "unchecked base");
    let (c_num_low, _) = h.add_step(
        "e-num-low", StepEntryType::Result, Some(StepOrigin::Solver),
        &["e-unc"], vec![eq_claim(b.clone(), d.clone())], "numeric over unchecked",
    );
    h.add_check(&c_num_low.unwrap(), "e-num-low", "identity_numeric", CheckOutcome::Pass, json!({"points": 12, "digits": 50}));
    let b_num_low = h.state.badge("e-num-low").unwrap();
    assert_eq!(b_num_low.own, Tier::CheckedNumerically);
    assert_eq!(b_num_low.effective, Tier::Unchecked);
    assert_eq!(b_num_low.own_check_line, None);
    assert_eq!(b_num_low.rests_on_line.as_deref(), Some("rests on e-unc (unchecked)"));
    assert_eq!(b_num_low.grade, DisplayGrade::ModelStatement);
    assert_eq!(b_num_low.border, BorderStyle::Dashed);
    assert_not_machine_confirmed(&b_num_low, "lowered numeric step");

    h.add_matched_quote("e-ocr-low", "src-4", "scan2.pdf", true, &["e-unc"]);
    let b_ocr_low = h.state.badge("e-ocr-low").unwrap();
    assert_eq!(b_ocr_low.own, Tier::Cited);
    assert_eq!(b_ocr_low.effective, Tier::Unchecked);
    assert_eq!(b_ocr_low.label, "Unchecked");
    assert_eq!(b_ocr_low.grade, DisplayGrade::ModelStatement);
    assert_eq!(b_ocr_low.border, BorderStyle::Dashed);
    assert_eq!(b_ocr_low.provenance_tag, None);
    assert!(b_ocr_low.flags.contains(&wording::FLAG_OCR_SOURCE.to_string()));
    assert_not_machine_confirmed(&b_ocr_low, "lowered OCR cited step");
}

#[test]
fn numeric_tier_label_and_ui_badge_use_numerically_checked_everywhere() {
    assert_eq!(Tier::CheckedNumerically.label(), "Numerically checked");
    assert_eq!(wording::BADGE_CHECKED_NUMERICALLY, "Numerically checked");
    let ui_badge_js = include_str!("../../desktop-server/assets/ui_badge.js");
    assert!(ui_badge_js.contains("export const SOLVE_NUMERIC_WORD = 'Numerically checked';"));
    assert!(!ui_badge_js.contains("Checked numerically"));
}
