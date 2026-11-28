//! Set Q M3 gate test (`eval-q-cases.md`, `solve.md` section 9):
//! Exercises ALL 3 planted-error `check_derivation` cases from `manifest-q.json` end-to-end
//! through real Python solver/check workers, `step_record`, `AutoCheckGate`, and `solve_finish`.
//! Proves 100% (3/3) end `Failed` and 0% end `Verified` or `Numerically checked`.

mod solve_rig;

use ledger::Ledger;
use research_tools::solve_defs::SOLVE_FINISH;
use research_tools::solve_model::{
    DoneCriterion, DoneCriterionKind, SolveBudgets, StepEntryType, SymbolSpec, TaskKind,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use serde_json::{json, Value};
use solve_rig::{build_real_rig, eval_and_harvest, record_equation_step};
use std::path::PathBuf;
use tools::CallOutcome;

#[test]
fn set_q_m3_gate_all_three_check_derivation_planted_errors_end_failed() {
    let manifest_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/eval/manifest-q.json");
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    for expected_id in [
        "Q-check-01-sampled-domain-sqrt",
        "Q-check-02-branch-cut-log",
        "Q-check-03-gaussian-moment-ibp",
    ] {
        assert!(
            cases.iter().any(|c| c["id"] == expected_id),
            "missing Set Q case `{expected_id}`"
        );
    }

    let mut final_tiers = Vec::new();

    // Case 1: Q-check-01-sampled-domain-sqrt (`sqrt((x - 2)^2) + x = 2*x - 2` fails on `[0, 2)`)
    {
        let rig = build_real_rig(
            "q-check-01",
            TaskKind::CheckDerivation,
            "Check derivation of integral_0^4 (sqrt(x^2 - 4*x + 4) + x) dx",
            vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }],
            vec![],
            vec![DoneCriterion {
                id: "clos".into(),
                kind: DoneCriterionKind::ClosureTierAtLeast,
                params: json!({ "tier": "verified" }),
            }],
            SolveBudgets::default(),
        );
        let c1 = eval_and_harvest(
            &rig,
            "x = assumed_symbol('x', real=True)\nsave_expr('L1', x**2 - 4*x + 4)\nsave_expr('R1', (x - 2)**2)\nInteger(0)",
        );
        let s1 = record_equation_step(
            &rig,
            StepEntryType::SymbolicComputation,
            "Factor radicand",
            &[],
            &c1,
            ("L1@1", "R1@1"),
            Some("x in [0, 4]"),
        );
        assert_eq!(s1["effective_tier"], "verified");

        let c2 = eval_and_harvest(
            &rig,
            "x = assumed_symbol('x', real=True)\nsave_expr('L2', sqrt((x - 2)**2) + x)\nsave_expr('R2', 2*x - 2)\nInteger(0)",
        );
        let s2 = record_equation_step(
            &rig,
            StepEntryType::SymbolicComputation,
            "Drop sqrt -> 2x-2",
            &["e-1"],
            &c2,
            ("L2@1", "R2@1"),
            Some("x in [0, 4]"),
        );
        assert_eq!(s2["effective_tier"], "failed");

        let c3 = eval_and_harvest(
            &rig,
            "x = assumed_symbol('x', real=True)\nsave_expr('L3', Integral(2*x - 2, (x, 0, 4)))\nsave_expr('R3', Integer(8))\nInteger(0)",
        );
        let s3 = record_equation_step(
            &rig,
            StepEntryType::SymbolicComputation,
            "Integrate 2x-2 over [0,4]",
            &["e-2"],
            &c3,
            ("L3@1", "R3@1"),
            Some("x in [0, 4]"),
        );
        assert_eq!(s3["own_tier"], "verified");
        assert_eq!(s3["effective_tier"], "failed");
        assert!(
            s3["badge"]["label"].as_str().unwrap().contains("Broken")
                && s3["badge"]["label"].as_str().unwrap().contains("e-2"),
            "unexpected broken badge label: {}",
            s3["badge"]
        );
        assert_eq!(s3["badge"]["own_check_line"], "own check: verified");

        let g7 = rig
            .router
            .try_call(
                "solver:solve",
                SOLVE_FINISH,
                &json!({ "final_entry_id": "e-3", "summary_markdown": "Integral evaluated." }),
            )
            .unwrap();
        assert!(
            matches!(g7, CallOutcome::Error(ref m) if m.starts_with("GATE G7:") && m.contains("Failed")),
            "{g7:?}"
        );
        let state =
            SolveState::from_events(&Ledger::open(&rig.data_dir).unwrap().read_session(&rig.sid).unwrap());
        assert!(state.done.is_none());
        final_tiers.push(state.tier("e-3").unwrap().effective);
    }

    // Case 2: Q-check-02-branch-cut-log (`Log(z^2) = 2*Log(z)` fails on `theta in (pi/2, pi)`)
    {
        let rig = build_real_rig(
            "q-check-02",
            TaskKind::CheckDerivation,
            "Check derivation of Log(z^2) for z = r*exp(I*theta), theta in (pi/2, pi)",
            vec![
                SymbolSpec { name: "r".into(), assumptions: vec!["real".into(), "positive".into()] },
                SymbolSpec { name: "theta".into(), assumptions: vec!["real".into()] },
            ],
            vec![],
            vec![DoneCriterion {
                id: "clos".into(),
                kind: DoneCriterionKind::ClosureTierAtLeast,
                params: json!({ "tier": "verified" }),
            }],
            SolveBudgets::default(),
        );
        let c1 = eval_and_harvest(
            &rig,
            "r = assumed_symbol('r', real=True, positive=True)\ntheta = assumed_symbol('theta', real=True)\nsave_expr('L1', 'Abs(r*exp(I*theta))', locals={'r': r, 'theta': theta})\nsave_expr('R1', r)\nInteger(0)",
        );
        let s1 = record_equation_step(
            &rig,
            StepEntryType::SymbolicComputation,
            "|z| = r",
            &[],
            &c1,
            ("L1@1", "R1@1"),
            Some("r > 0, theta in (pi/2, pi)"),
        );
        assert_eq!(s1["effective_tier"], "verified");

        let c2 = eval_and_harvest(
            &rig,
            "r = assumed_symbol('r', real=True, positive=True)\ntheta = assumed_symbol('theta', real=True)\nz = r*exp(I*theta)\nsave_expr('L2', log(z**2))\nsave_expr('R2', 2*log(z))\nInteger(0)",
        );
        let s2 = record_equation_step(
            &rig,
            StepEntryType::SymbolicComputation,
            "Claim Log(z^2) = 2 Log(z)",
            &["e-1"],
            &c2,
            ("L2@1", "R2@1"),
            Some("r > 0, theta in (pi/2, pi)"),
        );
        assert_eq!(s2["effective_tier"], "failed");

        let c3 = eval_and_harvest(
            &rig,
            "r = assumed_symbol('r', real=True, positive=True)\ntheta = assumed_symbol('theta', real=True)\nsave_expr('L3', '((2*log(r) + 2*I*theta) - 2*log(r))/I', locals={'r': r, 'theta': theta})\nsave_expr('R3', 2*theta)\nInteger(0)",
        );
        let s3 = record_equation_step(
            &rig,
            StepEntryType::SymbolicComputation,
            "Isolate imaginary coeff",
            &["e-2"],
            &c3,
            ("L3@1", "R3@1"),
            Some("r > 0, theta in (pi/2, pi)"),
        );
        assert_eq!(s3["own_tier"], "verified");
        assert_eq!(s3["effective_tier"], "failed");

        let g7 = rig
            .router
            .try_call(
                "solver:solve",
                SOLVE_FINISH,
                &json!({ "final_entry_id": "e-3", "summary_markdown": "Branch cut derivation checked." }),
            )
            .unwrap();
        assert!(
            matches!(g7, CallOutcome::Error(ref m) if m.starts_with("GATE G7:") && m.contains("Failed")),
            "{g7:?}"
        );
        let state =
            SolveState::from_events(&Ledger::open(&rig.data_dir).unwrap().read_session(&rig.sid).unwrap());
        final_tiers.push(state.tier("e-3").unwrap().effective);
    }

    // Case 3: Q-check-03-gaussian-moment-ibp (dropped factor 1/2 in I_4(alpha) = (3/alpha)*I_2(alpha))
    {
        let rig = build_real_rig(
            "q-check-03",
            TaskKind::CheckDerivation,
            "Check fourth Gaussian moment I_4(alpha) derivation by IBP",
            vec![
                SymbolSpec { name: "alpha".into(), assumptions: vec!["real".into(), "positive".into()] },
                SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] },
            ],
            vec![],
            vec![DoneCriterion {
                id: "clos".into(),
                kind: DoneCriterionKind::ClosureTierAtLeast,
                params: json!({ "tier": "verified" }),
            }],
            SolveBudgets::default(),
        );
        let c1 = eval_and_harvest(
            &rig,
            "alpha = assumed_symbol('alpha', real=True, positive=True)\nx = assumed_symbol('x', real=True)\nsave_expr('L1', Integral(exp(-alpha*x**2), (x, -oo, oo)))\nsave_expr('R1', sqrt(pi/alpha))\nInteger(0)",
        );
        assert_eq!(
            record_equation_step(&rig, StepEntryType::SymbolicComputation, "I_0(alpha)", &[], &c1, ("L1@1", "R1@1"), None)["effective_tier"],
            "verified"
        );

        let c2 = eval_and_harvest(
            &rig,
            "alpha = assumed_symbol('alpha', real=True, positive=True)\nx = assumed_symbol('x', real=True)\nsave_expr('L2', Integral(x**2 * exp(-alpha*x**2), (x, -oo, oo)))\nsave_expr('R2', sqrt(pi)/(2*alpha**Rational(3, 2)))\nInteger(0)",
        );
        assert_eq!(
            record_equation_step(&rig, StepEntryType::SymbolicComputation, "I_2(alpha)", &["e-1"], &c2, ("L2@1", "R2@1"), None)["effective_tier"],
            "verified"
        );

        let c3 = eval_and_harvest(
            &rig,
            "alpha = assumed_symbol('alpha', real=True, positive=True)\nx = assumed_symbol('x', real=True)\nsave_expr('L3', Integral(x**4 * exp(-alpha*x**2), (x, -oo, oo)))\nsave_expr('R3', (3/alpha)*sqrt(pi)/(2*alpha**Rational(3, 2)))\nInteger(0)",
        );
        let s3 = record_equation_step(&rig, StepEntryType::SymbolicComputation, "Claim I_4 = (3/alpha) I_2", &["e-2"], &c3, ("L3@1", "R3@1"), None);
        assert_eq!(s3["effective_tier"], "failed");

        let c4 = eval_and_harvest(
            &rig,
            "alpha = assumed_symbol('alpha', real=True, positive=True)\nsave_expr('L4', '(3/alpha)*(sqrt(pi)/(2*alpha**Rational(3, 2)))', locals={'alpha': alpha})\nsave_expr('R4', 3*sqrt(pi)/(2*alpha**Rational(5, 2)))\nInteger(0)",
        );
        let s4 = record_equation_step(&rig, StepEntryType::SymbolicComputation, "Substitute I_2 into s3", &["e-3", "e-2"], &c4, ("L4@1", "R4@1"), None);
        assert_eq!(s4["own_tier"], "verified");
        assert_eq!(s4["effective_tier"], "failed");

        let g7 = rig
            .router
            .try_call(
                "solver:solve",
                SOLVE_FINISH,
                &json!({ "final_entry_id": "e-4", "summary_markdown": "Gaussian moment checked." }),
            )
            .unwrap();
        assert!(
            matches!(g7, CallOutcome::Error(ref m) if m.starts_with("GATE G7:") && m.contains("Failed")),
            "{g7:?}"
        );
        let state =
            SolveState::from_events(&Ledger::open(&rig.data_dir).unwrap().read_session(&rig.sid).unwrap());
        final_tiers.push(state.tier("e-4").unwrap().effective);
    }

    assert_eq!(final_tiers.len(), 3);
    assert!(
        final_tiers.iter().all(|&t| t == Tier::Failed),
        "100% must end Failed: {final_tiers:?}"
    );
    assert!(
        !final_tiers
            .iter()
            .any(|&t| matches!(t, Tier::Verified | Tier::CheckedNumerically))
    );
}
