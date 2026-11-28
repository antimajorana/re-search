//! Deterministic property tests for Solve rules R4, R7, R8, and R22 (package S2).
//! Uses a small built-in xorshift64 generator (zero new dependencies).

use super::*;
use crate::proof_closure::all_deps;
use crate::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, AckKind,
    CheckOutcome, CheckRequester, Claim, ClaimForm, ClaimRelation, HandleRef, QuoteRef,
    ReceiptRef, ReviewPolicy, RoleModel, SetupAssumption, SolveBudgets, SolveCheck, SolveExpr,
    SolveRoles, SolveSetup, SolveTarget, StepEntryType, StepOrigin, SymbolSpec, TargetForm,
    TaskKind,
};
use crate::solve_state::SolveState;
use ledger::Event;
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn usize(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        (self.next_u64() % den) < num
    }
}

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "prop-sess".into(),
        seq,
        ts_ms: (seq as i64) * 10,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: "0".repeat(64),
        event_hash: "f".repeat(64),
    }
}

fn setup_event(seq: u64, rev: u32, tag: &str) -> (Event, String) {
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let assumptions = vec![SetupAssumption { id: "a".into(), text: tag.into() }];
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, tag);
    let setup = SolveSetup {
        revision: rev,
        statement: "prop".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget { description: "t".into(), form: TargetForm::ClosedForm, order: None, digits: None },
        done: vec![],
        conventions: tag.into(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "p1".into(), model: "m1".into() },
            verifier: RoleModel { provider: "p2".into(), model: "m2".into() },
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha.clone(),
    };
    (ev(seq, event_kinds::SOLVE_SETUP, serde_json::to_value(setup).unwrap()), math_sha)
}

fn expr_ev(seq: u64, name: &str, srepr_sha: &str) -> Event {
    ev(
        seq,
        event_kinds::SOLVE_EXPR,
        serde_json::to_value(SolveExpr {
            name: name.into(),
            version: 1,
            srepr_blob: name.into(),
            srepr_sha256: srepr_sha.into(),
            latex: name.into(),
            str_preview: name.into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: "c0".into(), result_seq: 1 },
            owner_entry: None,
            portable: true,
        })
        .unwrap(),
    )
}

fn check_ev(
    seq: u64,
    claim_sha: &str,
    claim_index: usize,
    math_sha: &str,
    name: &str,
    outcome: CheckOutcome,
    evidence: Value,
) -> Event {
    ev(
        seq,
        event_kinds::SOLVE_CHECK,
        serde_json::to_value(SolveCheck {
            check_id: format!("chk-{seq}"),
            claim_sha256: claim_sha.into(),
            claim_index,
            entry_id: None,
            math_sha256: math_sha.into(),
            name: name.into(),
            registry_sha256: "r".into(),
            params: json!({}),
            outcome,
            asked_by: CheckRequester::Host,
            receipt: ReceiptRef { call_id: "cc".into(), result_seq: seq },
            evidence,
            evidence_sha256: "e".into(),
        })
        .unwrap(),
    )
}

fn random_dag(rng: &mut Rng, n_nodes: usize) -> (SolveState, u64, String) {
    let mut seq = 1u64;
    let mut state = SolveState::new();
    state.apply(&ev(seq, event_kinds::SOLVE_STARTED, json!({})));
    seq += 1;
    let (se, math_sha) = setup_event(seq, 1, "base");
    state.apply(&se);

    seq += 1;
    let quote_seq = seq;
    let ocr = rng.chance(1, 2);
    state.apply(&ev(
        quote_seq,
        event_kinds::EVIDENCE_QUOTE,
        json!({"source_id": "src", "path": "p.tex", "start_line": 1, "end_line": 2, "file_sha256": "a".repeat(64), "quote_sha256": "b".repeat(64), "ocr": ocr}),
    ));
    seq += 1;
    state.apply(&expr_ev(seq, "H_lhs", "sl"));
    seq += 1;
    state.apply(&expr_ev(seq, "H_rhs", "sr"));

    let base_claim = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::parse("H_lhs@1").unwrap(),
        rhs: HandleRef::parse("H_rhs@1").unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };

    for i in 1..=n_nodes {
        let eid = format!("e-{i}");
        let (mut declared, mut derived) = (Vec::new(), Vec::new());
        for prev in 1..i {
            let p_id = format!("e-{prev}");
            match rng.usize(6) {
                0 => declared.push(p_id),
                1 => derived.push(p_id),
                _ => {}
            }
        }
        let kind_roll = rng.usize(10);
        let (etype, origin, claims, claim_sha, source_ref) = match kind_roll {
            0 => (StepEntryType::Assumption, Some(StepOrigin::Setup), vec![], None, None),
            1 => (StepEntryType::Assumption, Some(StepOrigin::Solver), vec![], None, None),
            2 => (
                StepEntryType::ReferenceLookup,
                Some(StepOrigin::Solver),
                vec![],
                None,
                Some(QuoteRef {
                    source_id: "src".into(), path: "p.tex".into(), start_line: 1, end_line: 2,
                    file_sha256: "a".repeat(64), quote_sha256: "b".repeat(64), evidence_seq: Some(quote_seq),
                }),
            ),
            3..=6 => {
                let mut c = base_claim.clone();
                c.domain = Some(format!("dom-{i}"));
                let csha = compute_claim_sha256(std::slice::from_ref(&c), |h| {
                    state.handles.get(h).map(|e| e.srepr_sha256.as_str())
                })
                .unwrap();
                (StepEntryType::Derivation, Some(StepOrigin::Solver), vec![c], Some(csha), None)
            }
            _ => (StepEntryType::Derivation, Some(StepOrigin::Solver), vec![], None, None),
        };
        let entry_sha = compute_entry_sha256(claim_sha.as_deref(), &eid, "d");
        seq += 1;
        state.apply(&ev(
            seq,
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": eid, "entry_type": etype, "summary": eid, "details": "d",
                "depends_on": declared, "derived_depends_on": derived, "claims": claims,
                "claim_sha256": claim_sha, "entry_sha256": entry_sha, "origin": origin, "source_ref": source_ref,
            }),
        ));

        match kind_roll {
            3 => {
                // Verified (exact symbolic)
                seq += 1;
                state.apply(&check_ev(seq, &claim_sha.unwrap(), 0, &math_sha, "identity_symbolic", CheckOutcome::Pass, json!({})));
            }
            4 => {
                // Failed check
                seq += 1;
                state.apply(&check_ev(seq, &claim_sha.unwrap(), 0, &math_sha, "identity_numeric", CheckOutcome::Fail, json!({"counterexample": "1"})));
            }
            5 => {
                // Checked numerically (12 points, 50 digits)
                seq += 1;
                state.apply(&check_ev(seq, &claim_sha.unwrap(), 0, &math_sha, "identity_numeric", CheckOutcome::Pass, json!({"points": 12, "digits": 50})));
            }
            7 => {
                let indep = if rng.chance(1, 2) { "different_provider" } else { "same_model" };
                seq += 1;
                state.apply(&ev(
                    seq,
                    event_kinds::PROOF_VERIFICATION,
                    json!({"target_entry_id": eid, "entry_sha256": entry_sha, "verdict": "accepted", "independence": indep}),
                ));
            }
            8 => {
                seq += 1;
                state.apply(&ev(seq, event_kinds::SOLVE_ACK, json!({"entry_id": eid, "kind": AckKind::HandChecked, "entry_sha256": entry_sha})));
            }
            _ => {}
        }

        if i > 2 && rng.chance(1, 8) {
            let victim = format!("e-{}", 1 + rng.usize(i - 1));
            seq += 1;
            state.apply(&ev(seq, event_kinds::PROOF_SUPERSEDE, json!({"entry_id": victim, "replaced_by": eid, "reason": "prop"})));
        }
    }
    (state, seq, math_sha)
}

fn contains_word(text: &str, target: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == target)
}

fn assert_r22_wording(badge: &StepBadge) {
    // Badge label contains "verified" iff effective tier is Verified.
    if badge.effective == Tier::Verified {
        assert_eq!(badge.label, wording::BADGE_VERIFIED);
    } else {
        assert!(
            !contains_word(&badge.label, "verified"),
            "non-Verified effective {:?} showed `verified` in label `{}`",
            badge.effective,
            badge.label
        );
    }
    // Accepted 2026-11-26 evidence labels and machine-confirmed grade exclusivity (00-overview.md 2.10).
    match badge.effective {
        Tier::Verified => {
            assert_eq!(badge.label, "Verified");
            assert_eq!(badge.grade, DisplayGrade::ByCode);
            assert_eq!(badge.border, BorderStyle::Solid);
        }
        Tier::CheckedNumerically => {
            assert_eq!(badge.label, "Numerically checked");
            assert_eq!(badge.grade, DisplayGrade::ByCode);
            assert_eq!(badge.border, BorderStyle::Solid);
        }
        Tier::Cited => {
            assert_eq!(badge.label, "Found");
            assert!(matches!(
                badge.grade,
                DisplayGrade::MatchedQuoteFullText | DisplayGrade::MatchedQuoteScannedPage
            ));
            assert_eq!(badge.border, BorderStyle::Solid);
        }
        Tier::Given | Tier::CheckedByYou => {
            assert_eq!(badge.grade, DisplayGrade::UserStatement);
            assert_eq!(badge.border, BorderStyle::Plain);
        }
        Tier::Failed | Tier::Stale => {
            assert_eq!(badge.grade, DisplayGrade::ModelStatement);
            assert_eq!(badge.border, BorderStyle::None);
            assert_ne!(badge.label, "Numerically checked");
            assert_ne!(badge.label, "Found");
        }
        Tier::ModelAgreed
        | Tier::ModelAgreedSameModel
        | Tier::AssumedByModel
        | Tier::Unchecked => {
            assert_eq!(badge.grade, DisplayGrade::ModelStatement);
            assert_eq!(badge.border, BorderStyle::Dashed);
            assert_ne!(badge.label, "Numerically checked");
            assert_ne!(badge.label, "Found");
        }
    }
    // `own_check_line` is "own check: verified" iff own == Verified && effective != Verified.
    if badge.own == Tier::Verified && badge.effective != Tier::Verified {
        assert_eq!(badge.own_check_line.as_deref(), Some(wording::OWN_CHECK_VERIFIED));
    } else {
        assert_eq!(badge.own_check_line, None);
    }

    // No other badge string may contain "verified", and no string anywhere may contain
    // "proved", "new", "novel", or "computed".
    let mut others: Vec<&str> = Vec::new();
    if let Some(ref line) = badge.rests_on_line {
        others.push(line);
    }
    if let Some(ref tag) = badge.provenance_tag {
        others.push(tag);
    }
    others.extend(badge.sublabels.iter().map(String::as_str));
    others.extend(badge.flags.iter().map(String::as_str));
    for text in &others {
        assert!(!contains_word(text, "verified"), "unexpected `verified` in `{text}` for {}", badge.entry_id);
    }
    for text in others.into_iter().chain(std::iter::once(badge.label.as_str())).chain(badge.own_check_line.as_deref()) {
        for forbidden in ["proved", "new", "novel", "computed"] {
            assert!(!contains_word(text, forbidden), "forbidden `{forbidden}` in `{text}` for {}", badge.entry_id);
        }
    }
}

#[test]
fn r7_and_r22_property_tiers_monotone_and_wording_clean_on_200_random_dags() {
    let mut rng = Rng::new(0x5017_E001);
    for case in 0..200 {
        let n = 4 + rng.usize(10);
        let (state, mut seq, math_sha) = random_dag(&mut rng, n);

        for entry in &state.entries {
            let t = state.tier(&entry.entry_id).unwrap();
            assert!(t.effective.lattice_rank() <= t.own.lattice_rank(), "case {case} {}", entry.entry_id);
            for dep in all_deps(entry) {
                let dt = state.tier(dep).unwrap();
                assert!(t.effective.lattice_rank() <= dt.effective.lattice_rank(), "case {case} {} -> {dep}", entry.entry_id);
            }
            assert_r22_wording(&state.badge(&entry.entry_id).unwrap());
        }

        let before: BTreeMap<String, i8> =
            state.tiers.iter().map(|(k, v)| (k.clone(), v.effective.lattice_rank())).collect();
        let target = &state.entries[rng.usize(state.entries.len())];
        let mut with_fail = state.clone();
        seq += 1;
        if let Some(ref csha) = target.claim_sha256 {
            with_fail.apply(&check_ev(seq, csha, 0, &math_sha, "dimensions", CheckOutcome::Fail, json!({})));
        } else {
            with_fail.apply(&ev(
                seq,
                event_kinds::PROOF_VERIFICATION,
                json!({"target_entry_id": target.entry_id, "entry_sha256": target.entry_sha256, "verdict": "rejected"}),
            ));
        }
        for (id, &old_rank) in &before {
            assert!(with_fail.tier(id).unwrap().effective.lattice_rank() <= old_rank, "case {case} fail raised {id}");
        }

        if state.entries.len() >= 2 {
            let dst_idx = 1 + rng.usize(state.entries.len() - 1);
            let src_id = state.entries[rng.usize(dst_idx)].entry_id.clone();
            let mut with_edge = state.clone();
            if rng.chance(1, 2) {
                with_edge.entries[dst_idx].depends_on.push(src_id);
            } else {
                with_edge.entries[dst_idx].derived_depends_on.push(src_id);
            }
            with_edge.recompute_tiers();
            for (id, &old_rank) in &before {
                assert!(with_edge.tier(id).unwrap().effective.lattice_rank() <= old_rank, "case {case} edge raised {id}");
            }
        }
    }
}

#[test]
fn r4_property_multi_claim_and_setup_change_and_rerecord() {
    let mut rng = Rng::new(0x0000_0044);
    for case in 0..60 {
        let mut seq = 1u64;
        let mut state = SolveState::new();
        state.apply(&ev(seq, event_kinds::SOLVE_STARTED, json!({})));
        seq += 1;
        let (se, math_sha) = setup_event(seq, 1, &format!("math-{case}"));
        state.apply(&se);

        let num_claims = 1 + rng.usize(4);
        let mut claims = Vec::new();
        for c in 0..num_claims {
            let (ln, rn) = (format!("L_{c}"), format!("R_{c}"));
            seq += 1;
            state.apply(&expr_ev(seq, &ln, &format!("sl_{case}_{c}")));
            seq += 1;
            state.apply(&expr_ev(seq, &rn, &format!("sr_{case}_{c}")));
            claims.push(Claim {
                form: ClaimForm::Equation, lhs: HandleRef::new(ln, 1).unwrap(), rhs: HandleRef::new(rn, 1).unwrap(),
                relation: ClaimRelation::Eq, order: None, domain: None, how: None,
            });
        }
        let claim_sha = compute_claim_sha256(&claims, |h| state.handles.get(h).map(|e| e.srepr_sha256.as_str())).unwrap();
        seq += 1;
        state.apply(&ev(
            seq,
            event_kinds::PROOF_ENTRY,
            json!({"entry_id": "e-1", "entry_type": "derivation", "summary": "s", "details": "d", "claims": claims, "claim_sha256": claim_sha, "entry_sha256": compute_entry_sha256(Some(&claim_sha), "s", "d")}),
        ));

        let omit = rng.usize(num_claims);
        for c in 0..num_claims {
            if c != omit {
                seq += 1;
                state.apply(&check_ev(seq, &claim_sha, c, &math_sha, "identity_symbolic", CheckOutcome::Pass, json!({})));
            }
        }
        assert_eq!(state.tier("e-1").unwrap().own, Tier::Unchecked, "case {case} omitted {omit}");

        seq += 1;
        state.apply(&check_ev(seq, &claim_sha, omit, &math_sha, "identity_symbolic", CheckOutcome::Pass, json!({})));
        assert_eq!(state.tier("e-1").unwrap().own, Tier::Verified);

        seq += 1;
        let (se2, _) = setup_event(seq, 2, &format!("math-changed-{case}"));
        state.apply(&se2);
        assert_eq!(state.tier("e-1").unwrap().effective, Tier::Stale);
    }
}

#[test]
fn r8_property_weak_checks_never_compute_and_numeric_never_verifies() {
    let weak_names = ["dimensions", "limit_case", "symmetry", "conservation", "convergence", "matrix_property"];
    let mut rng = Rng::new(0x0000_0088);
    for case in 0..60 {
        let mut seq = 1u64;
        let mut state = SolveState::new();
        state.apply(&ev(seq, event_kinds::SOLVE_STARTED, json!({})));
        seq += 1;
        let (se, math_sha) = setup_event(seq, 1, "r8");
        state.apply(&se);
        seq += 1;
        state.apply(&expr_ev(seq, "A", "sa"));
        seq += 1;
        state.apply(&expr_ev(seq, "B", "sb"));

        let is_ineq = rng.chance(1, 2);
        let claim = Claim {
            form: if is_ineq { ClaimForm::Inequality } else { ClaimForm::Equation },
            lhs: HandleRef::parse("A@1").unwrap(),
            rhs: HandleRef::parse("B@1").unwrap(),
            relation: if is_ineq { ClaimRelation::Ge } else { ClaimRelation::Eq },
            order: None,
            domain: None,
            how: None,
        };
        let claim_sha = compute_claim_sha256(std::slice::from_ref(&claim), |h| {
            state.handles.get(h).map(|e| e.srepr_sha256.as_str())
        })
        .unwrap();
        seq += 1;
        state.apply(&ev(
            seq,
            event_kinds::PROOF_ENTRY,
            json!({"entry_id": "e-1", "entry_type": "result", "summary": "s", "details": "d", "claims": [claim], "claim_sha256": claim_sha}),
        ));

        for &name in &weak_names {
            if rng.chance(1, 2) {
                seq += 1;
                state.apply(&check_ev(seq, &claim_sha, 0, &math_sha, name, CheckOutcome::Pass, json!({})));
            }
        }
        seq += 1;
        if is_ineq {
            state.apply(&check_ev(seq, &claim_sha, 0, &math_sha, "counterexample_search", CheckOutcome::Pass, json!({"points": 100})));
            assert_eq!(state.tier("e-1").unwrap().own, Tier::Unchecked, "case {case} ineq");
        } else if rng.chance(1, 2) {
            // Under-threshold numeric -> Unchecked.
            let pts = rng.usize(12) as u64;
            state.apply(&check_ev(seq, &claim_sha, 0, &math_sha, "identity_numeric", CheckOutcome::Pass, json!({"points": pts, "digits": 50})));
            assert_eq!(state.tier("e-1").unwrap().own, Tier::Unchecked, "case {case} under-threshold");
        } else {
            // Qualifying numeric (points >= 12, digits >= 50) -> CheckedNumerically, NEVER Verified.
            let pts = 12 + (rng.usize(200) as u64);
            let digs = 50 + (rng.usize(100) as u64);
            state.apply(&check_ev(seq, &claim_sha, 0, &math_sha, "identity_numeric", CheckOutcome::Pass, json!({"points": pts, "digits": digs})));
            assert_eq!(state.tier("e-1").unwrap().own, Tier::CheckedNumerically, "case {case} numeric");
        }
        assert_r22_wording(&state.badge("e-1").unwrap());
    }
}
