//! End-to-end Solve S8 -> S9 -> S11 integration tests:
//! - Real cross-process Python `worker.py --stdio --role solver` & `--role check`
//! - `solve/1` Router + `GateChain(SolveGate, AutoCheckGate)` + `solve_check_host_v1`
//! - G1 (`>3` pending runs), G4 (F9 model-call & wall-clock caps), G5 (`gate_bounces`),
//!   G6 (2 rejected reviews on lineage -> `verification_stuck`), G7 (closure & R22), R15 (`solve.done`)

mod solve_rig;

use ledger::Ledger;
use research_tools::solve_defs::SOLVE_FINISH;
use research_tools::solve_gates::{detect_g4_budget, detect_g6_verification_stuck};
use research_tools::solve_model::{
    event_kinds, DoneCriterion, DoneCriterionKind, PauseReason, ReviewIndependence, SolveBudgets,
    StepEntryType, SymbolSpec, TaskKind,
};
use research_tools::solve_state::SolveState;
use serde_json::json;
use solve_rig::{build_real_rig, eval_and_harvest, record_equation_step};
use std::time::Duration;
use tools::CallOutcome;

#[test]
fn s8_s9_s11_end_to_end_real_python_workers_and_gates() {
    let rig = build_real_rig(
        "s-e2e-full",
        TaskKind::Calculate,
        "Show (x + 1)^2 - (x^2 + 2*x + 1) = 0 for real x",
        vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }],
        vec![],
        vec![
            DoneCriterion { id: "final_ver".into(), kind: DoneCriterionKind::FinalTierAtLeast, params: json!({ "tier": "verified" }) },
            DoneCriterion { id: "clos_ver".into(), kind: DoneCriterionKind::ClosureTierAtLeast, params: json!({ "tier": "verified" }) },
            DoneCriterion { id: "no_flags".into(), kind: DoneCriterionKind::NoOpenFlags, params: json!({}) },
        ],
        SolveBudgets::default(),
    );

    // G1: 3 exploratory sympy_eval runs pass; 4th is blocked until one is cited or discarded.
    let c1 = eval_and_harvest(&rig, "x = assumed_symbol('x', real=True)\nsave_expr('L1', (x + 1)**2)\nsave_expr('R1', x**2 + 2*x + 1)\nInteger(0)");
    let _c2 = eval_and_harvest(&rig, "x = assumed_symbol('x', real=True)\nx + 1");
    let _c3 = eval_and_harvest(&rig, "x = assumed_symbol('x', real=True)\nx + 2");
    let g1_block = rig
        .router
        .try_call("solver:solve", "sympy_eval", &json!({ "code": "1 + 1" }))
        .unwrap();
    assert!(matches!(&g1_block, CallOutcome::Error(m) if m.starts_with("GATE G1:")), "{g1_block:?}");

    // Citing c1 via step_record lowers pending_runs back to 2 and runs AutoCheckGate (Verified!).
    let s1 = record_equation_step(
        &rig,
        StepEntryType::SymbolicComputation,
        "Expand (x+1)^2",
        &[],
        &c1,
        ("L1@1", "R1@1"),
        None,
    );
    assert_eq!(s1["entry_id"], "e-1");
    assert_eq!(s1["effective_tier"], "verified");
    assert_eq!(s1["auto_checks"][0]["outcome"], "pass");

    // G7 R22 refuses retired "Computed" or "proved" even when step IS Verified.
    for bad in [
        "Result has status Computed.",
        "Therefore the identity is proved.",
    ] {
        let refused = rig
            .router
            .try_call(
                "solver:solve",
                SOLVE_FINISH,
                &json!({ "final_entry_id": "e-1", "summary_markdown": bad }),
            )
            .unwrap();
        assert!(matches!(&refused, CallOutcome::Error(m) if m.starts_with("GATE G7:")), "{refused:?}");
    }

    // Valid `solve_finish` commits `solve.done` with status `finished`.
    let fin = rig
        .router
        .try_call(
            "solver:solve",
            SOLVE_FINISH,
            &json!({
                "outcome": "finished",
                "final_entry_id": "e-1",
                "summary_markdown": "Step e-1 is verified via identity_symbolic."
            }),
        )
        .unwrap();
    let fin_val = match fin {
        CallOutcome::Ok(v) => v,
        other => panic!("solve_finish unexpected: {other:?}"),
    };
    assert_eq!(fin_val["status"], "finished");
    assert_eq!(fin_val["effective_tier"], "verified");
    let folded = SolveState::from_events(&Ledger::open(&rig.data_dir).unwrap().read_session(&rig.sid).unwrap());
    assert_eq!(folded.done.unwrap().final_entry_id, "e-1");
}

#[test]
fn s11_g4_g5_g6_pause_triggers_and_r15_finished_open_items() {
    let rig = build_real_rig(
        "s-r15-open",
        TaskKind::Calculate,
        "Verify e-1",
        vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }],
        vec![],
        vec![
            DoneCriterion { id: "named".into(), kind: DoneCriterionKind::NamedCheckPassed, params: json!({ "check": "vanishes_to_order", "on": "final" }) },
        ],
        SolveBudgets { max_model_calls: 2, max_wall_minutes: 10, max_model_calls_per_leg: 1, max_review_loops: 4, max_gate_bounces: 2 },
    );

    let c1 = eval_and_harvest(&rig, "x = assumed_symbol('x', real=True)\nsave_expr('A', x)\nsave_expr('B', x)\nx");
    let s1 = record_equation_step(&rig, StepEntryType::SymbolicComputation, "x = x", &[], &c1, ("A@1", "B@1"), None);
    assert_eq!(s1["entry_id"], "e-1");

    // R15: solver asks for `finished`, missing `vanishes_to_order` forces `finished_open_items`.
    let out = rig
        .router
        .try_call(
            "solver:solve",
            SOLVE_FINISH,
            &json!({ "outcome": "finished", "final_entry_id": "e-1", "summary_markdown": "Step e-1 is verified." }),
        )
        .unwrap();
    let v = match out { CallOutcome::Ok(v) => v, other => panic!("{other:?}") };
    assert_eq!(v["status"], "finished_open_items");

    // G5 (`max_gate_bounces: 2`): 2 consecutive G7 refusals trigger G5 + pause `gate_bounces`.
    for _ in 0..2 {
        let g = rig.router.try_call("solver:solve", SOLVE_FINISH, &json!({ "final_entry_id": "e-missing", "summary_markdown": "x" })).unwrap();
        assert!(matches!(g, CallOutcome::Error(ref m) if m.starts_with("GATE G7:")));
    }
    let g5 = rig.router.try_call("solver:solve", "ledger_note", &json!({ "text": "ping" })).unwrap();
    assert!(matches!(g5, CallOutcome::Error(ref m) if m.starts_with("GATE G5:")));
    let paused = rig.gate.check_and_emit_pause(&rig.sid, 1, 2).unwrap().unwrap();
    assert_eq!(paused.reason, PauseReason::GateBounces);

    // G4 (F9 count_model_calls -> LegCap then Budget) and G6 (2 rejected reviews -> VerificationStuck).
    let ledger = Ledger::open(&rig.data_dir).unwrap();
    let leg_seq = ledger.read_session(&rig.sid).unwrap().iter().rev().find(|e| e.kind == event_kinds::SOLVE_SETUP).unwrap().seq;
    ledger.append_with_hash(&rig.sid, "model.request", 1, &json!({ "turn": 1 })).unwrap();
    let evs = ledger.read_session(&rig.sid).unwrap();
    let st = SolveState::from_events(&evs);
    assert_eq!(
        detect_g4_budget(&st, &evs, leg_seq, leg_seq, Duration::from_secs(1)).unwrap().reason,
        PauseReason::LegCap
    );
    let after_first = evs.last().unwrap().seq;
    ledger.append_with_hash(&rig.sid, "verifier.request", 1, &json!({ "turn": 1 })).unwrap();
    let evs_b = ledger.read_session(&rig.sid).unwrap();
    let st_b = SolveState::from_events(&evs_b);
    assert_eq!(
        detect_g4_budget(&st_b, &evs_b, after_first, leg_seq, Duration::from_secs(1)).unwrap().reason,
        PauseReason::Budget
    );

    for _ in 0..2 {
        ledger
            .append_with_hash(
                &rig.sid,
                event_kinds::PROOF_VERIFICATION,
                1,
                &json!({ "target_entry_id": "e-1", "verdict": "rejected", "independence": ReviewIndependence::DifferentProvider }),
            )
            .unwrap();
    }
    let evs2 = ledger.read_session(&rig.sid).unwrap();
    let st2 = SolveState::from_events(&evs2);
    assert_eq!(detect_g6_verification_stuck(&st2, &evs2).unwrap().reason, PauseReason::VerificationStuck);
}
