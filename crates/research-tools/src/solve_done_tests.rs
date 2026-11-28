use super::*;
use crate::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, CheckOutcome, CheckRequester,
    Claim, ClaimForm, ClaimRelation, DoneCriterion, DoneCriterionKind, HandleRef, ReceiptRef,
    RoleModel, SetupAssumption, SolveBudgets, SolveCheck, SolveExpr, SolveRoles, SolveSetup,
    SolveTarget, StepEntryType, StepOrigin, SymbolSpec, TargetForm, TaskKind, TranscriptionMeta,
};
use serde_json::json;
use std::collections::BTreeMap;
use tempfile::TempDir;

fn setup_session(ledger: &Ledger, sid: &str, done: Vec<DoneCriterion>) -> (SolveSetup, String) {
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let assumptions = vec![SetupAssumption { id: "a1".into(), text: "x real".into() }];
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, "real x");
    let setup = SolveSetup {
        revision: 1,
        statement: "Compute f(x)".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "closed form".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done,
        conventions: "real x".into(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "anthropic".into(), model: "claude-sonnet-4-6".into() },
            verifier: RoleModel { provider: "openai".into(), model: "gpt-5".into() },
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review: crate::solve_model::ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha.clone(),
    };
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap())
        .unwrap();
    (setup, math_sha)
}

fn record_expr_and_step(
    ledger: &Ledger,
    sid: &str,
    eid: &str,
    deps: Vec<String>,
    lhs_name: &str,
    rhs_name: &str,
    transcription: Option<TranscriptionMeta>,
) -> (String, String) {
    let rseq = ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": format!("sym-{eid}"), "ok": true, "result": { "value": "x", "error": null, "process": "solver" } }),
        )
        .unwrap()
        .seq;
    let mut shas = BTreeMap::new();
    for name in [lhs_name, rhs_name] {
        let sha = format!("sha-{name}");
        ledger
            .append_with_hash(
                sid,
                event_kinds::SOLVE_EXPR,
                1,
                &serde_json::to_value(SolveExpr {
                    name: name.into(),
                    version: 1,
                    srepr_blob: format!("blob-{name}"),
                    srepr_sha256: sha.clone(),
                    latex: name.into(),
                    str_preview: name.into(),
                    free_symbols: vec![],
                    produced_by: ReceiptRef { call_id: format!("sym-{eid}"), result_seq: rseq },
                    owner_entry: Some(eid.into()),
                    portable: true,
                    unevaluated_srepr: None, unevaluated_srepr_sha256: None, source_expr: None,
                })
                .unwrap(),
            )
            .unwrap();
        shas.insert(name.to_string(), sha);
    }
    let claim = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new(lhs_name, 1).unwrap(),
        rhs: HandleRef::new(rhs_name, 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |h| shas.get(&h.name).map(String::as_str)).unwrap();
    let esha = compute_entry_sha256(Some(&csha), "step summary", "step details");
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": eid,
                "entry_type": StepEntryType::SymbolicComputation,
                "summary": "step summary",
                "details": "step details",
                "depends_on": deps,
                "tool_call_refs": [format!("sym-{eid}")],
                "claims": [claim],
                "claim_sha256": csha,
                "entry_sha256": esha,
                "origin": StepOrigin::Solver,
                "transcription": transcription,
            }),
        )
        .unwrap();
    (csha, esha)
}

fn append_check(
    ledger: &Ledger,
    sid: &str,
    eid: &str,
    csha: &str,
    msha: &str,
    name: &str,
    res: (CheckOutcome, Value),
) {
    let (outcome, evidence) = res;
    let rseq = ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": format!("chk-{eid}-{name}"), "ok": true, "result": { "process": "check" } }),
        )
        .unwrap()
        .seq;
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_CHECK,
            1,
            &serde_json::to_value(SolveCheck {
                check_id: format!("c-{eid}-{name}"),
                claim_sha256: csha.into(),
                claim_index: 0,
                entry_id: Some(eid.into()),
                math_sha256: msha.into(),
                name: name.into(),
                registry_sha256: "reg-1".into(),
                params: json!({}),
                outcome,
                asked_by: CheckRequester::Host,
                receipt: ReceiptRef { call_id: format!("chk-{eid}-{name}"), result_seq: rseq },
                evidence,
                evidence_sha256: "ev-1".into(),
            })
            .unwrap(),
        )
        .unwrap();
}

#[test]
fn solve_done_evaluates_all_five_criteria_r15_and_closure_guards() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-done-all-5";
    let done_criteria = vec![
        DoneCriterion {
            id: "c_final".into(),
            kind: DoneCriterionKind::FinalTierAtLeast,
            params: json!({ "tier": "verified" }),
        },
        DoneCriterion {
            id: "c_closure".into(),
            kind: DoneCriterionKind::ClosureTierAtLeast,
            params: json!({ "tier": "checked_numerically" }),
        },
        DoneCriterion {
            id: "c_named".into(),
            kind: DoneCriterionKind::NamedCheckPassed,
            params: json!({ "check": "vanishes_to_order", "on": "final" }),
        },
        DoneCriterion {
            id: "c_flags".into(),
            kind: DoneCriterionKind::NoOpenFlags,
            params: json!({}),
        },
        DoneCriterion {
            id: "c_confirm".into(),
            kind: DoneCriterionKind::UserConfirms,
            params: json!({ "text": "Confirm TeX step" }),
        },
    ];
    let (_, math_sha) = setup_session(&ledger, sid, done_criteria);

    // Step e-1 has a numeric check + transcription (not yet confirmed).
    let (c1, e1_sha) = record_expr_and_step(
        &ledger,
        sid,
        "e-1",
        vec![],
        "a1",
        "b1",
        Some(TranscriptionMeta {
            source_tex_sha256: "a".repeat(64),
        }),
    );
    append_check(
        &ledger,
        sid,
        "e-1",
        &c1,
        &math_sha,
        "identity_numeric",
        (CheckOutcome::Pass, json!({ "points": 12, "digits": 50 })),
    );
    // Step e-2 only checked numerically first -> numeric-only R15 + open flags yield finished_open_items.
    let (c2, _) = record_expr_and_step(&ledger, sid, "e-2", vec!["e-1".into()], "a2", "b2", None);
    append_check(
        &ledger,
        sid,
        "e-2",
        &c2,
        &math_sha,
        "identity_numeric",
        (CheckOutcome::Pass, json!({ "points": 12, "digits": 50 })),
    );

    // R15: even when solver proposes `finished`, unmet criteria force `finished_open_items`.
    let res_open = run_solve_finish(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({
            "outcome": "finished",
            "final_entry_id": "e-2",
            "summary_markdown": "Step e-2 is Numerically checked; confirmation still needed."
        }),
    )
    .unwrap();
    assert_eq!(res_open["status"], "finished_open_items");
    let crit_map: BTreeMap<String, bool> = res_open["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap().to_string(), c["met"].as_bool().unwrap()))
        .collect();
    assert!(!crit_map["c_final"]);
    assert!(crit_map["c_closure"]);
    assert!(!crit_map["c_named"]);
    assert!(!crit_map["c_flags"]);
    assert!(!crit_map["c_confirm"]);

    // Upgrade e-2 to Verified + vanishes_to_order + confirm transcription of e-1 -> Finished.
    append_check(&ledger, sid, "e-2", &c2, &math_sha, "identity_symbolic", (CheckOutcome::Pass, json!({})));
    append_check(&ledger, sid, "e-2", &c2, &math_sha, "vanishes_to_order", (CheckOutcome::Pass, json!({})));
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_ACK,
            1,
            &json!({ "entry_id": "e-1", "kind": "transcription", "entry_sha256": e1_sha }),
        )
        .unwrap();

    // Closure on e-2 still rests on e-1 (CheckedNumerically), so e-2 effective is CheckedNumerically.
    // Upgrade e-1 to Verified too so effective tier of e-2 reaches Verified!
    append_check(&ledger, sid, "e-1", &c1, &math_sha, "identity_symbolic", (CheckOutcome::Pass, json!({})));

    let res_done = run_solve_finish(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({
            "final_entry_id": "e-2",
            "summary_markdown": "Step e-2 is verified and confirmed."
        }),
    )
    .unwrap();
    assert_eq!(res_done["status"], "finished");
    assert_eq!(res_done["effective_tier"], "verified");
    assert!(res_done["criteria"].as_array().unwrap().iter().all(|c| c["met"] == true));
}

#[test]
fn solve_done_r22_lint_and_g7_refuse_dishonest_prose_and_broken_closures() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-done-r22";
    let (_, math_sha) = setup_session(&ledger, sid, vec![]);

    let (c1, _) = record_expr_and_step(&ledger, sid, "e-1", vec![], "l1", "r1", None);
    append_check(
        &ledger,
        sid,
        "e-1",
        &c1,
        &math_sha,
        "identity_numeric",
        (CheckOutcome::Pass, json!({ "points": 12, "digits": 50 })),
    );
    let (c2, _) = record_expr_and_step(&ledger, sid, "e-2", vec!["e-1".into()], "l2", "r2", None);
    // e-2 passes symbolic own check, but effective tier is CheckedNumerically due to e-1!
    append_check(&ledger, sid, "e-2", &c2, &math_sha, "identity_symbolic", (CheckOutcome::Pass, json!({})));

    // Unqualified "verified", named non-Verified step, retired "Computed", and "proved" all fail G7/R22.
    for bad_prose in [
        "The final result is verified.",
        "Step e-1 is verified.",
        "Step e-2 is verified.",
        "Step e-2 has badge Computed.",
        "Therefore the identity is proved.",
    ] {
        let err = run_solve_finish(
            &env,
            &ledger,
            sid,
            "solver:solve",
            &json!({ "final_entry_id": "e-2", "summary_markdown": bad_prose }),
        )
        .unwrap_err();
        assert!(err.contains("R22 wording"), "expected R22 error for `{bad_prose}`, got `{err}`");
    }

    // Negated "not verified" and accurate "Numerically checked" are accepted!
    let ok = run_solve_finish(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({
            "final_entry_id": "e-2",
            "summary_markdown": "Step e-2 rests on e-1 (Numerically checked), so effective status is not verified."
        }),
    );
    assert!(ok.is_ok(), "honest wording must pass R22: {ok:?}");

    // Stale refusal when setup changes math_sha256.
    let symbols2 = vec![SymbolSpec { name: "x".into(), assumptions: vec!["positive".into()] }];
    let m2 = compute_math_sha256(&symbols2, &BTreeMap::new(), &[], "positive x");
    let mut setup2 = serde_json::from_value::<SolveSetup>(
        ledger
            .read_session(sid)
            .unwrap()
            .into_iter()
            .rev()
            .find(|e| e.kind == event_kinds::SOLVE_SETUP)
            .unwrap()
            .payload,
    )
    .unwrap();
    setup2.revision = 2;
    setup2.symbols = symbols2;
    setup2.math_sha256 = m2;
    ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup2).unwrap())
        .unwrap();
    let err_stale = run_solve_finish(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({ "final_entry_id": "e-2", "summary_markdown": "Step e-2 summary." }),
    )
    .unwrap_err();
    assert!(err_stale.contains("Stale"), "{err_stale}");

    // Failed step in closure refuses finish too.
    append_check(&ledger, sid, "e-1", &c1, &setup2.math_sha256, "identity_symbolic", (CheckOutcome::Fail, json!({})));
    let err_fail = run_solve_finish(
        &env,
        &ledger,
        sid,
        "solver:solve",
        &json!({ "final_entry_id": "e-2", "summary_markdown": "Step e-2 summary." }),
    )
    .unwrap_err();
    assert!(err_fail.contains("Failed"), "{err_fail}");
}
