//! Unit tests for deterministic Solve setup validation and prefill (Checkpoint S4b).

use super::*;
use crate::lessons_schema::{ConventionDefault, SymbolDefault};
use crate::solve_model::TargetForm;
use crate::solve_status::wording;
use serde_json::json;
use std::time::Instant;

const WORKED_EXAMPLE: &str = "Compute the one-loop effective potential V_1(phi) of massless \
    lambda phi^4 theory, V_tree = lambda phi^4/4!, in d = 4 - 2 eps with MS-bar, \
    and show mu dV/dmu = 0 at order lambda^2.";

fn base_valid_input() -> SolveSetupInput {
    SolveSetupInput {
        statement: WORKED_EXAMPLE.into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "One-loop effective potential".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: default_done_criteria(TaskKind::Calculate, None, false, false),
        conventions: "Euclidean signature, hbar = c = 1".into(),
        symbols: vec![
            SymbolSpec { name: "lambda".into(), assumptions: vec!["real".into(), "positive".into()] },
            SymbolSpec { name: "phi".into(), assumptions: vec!["real".into()] },
        ],
        dimensions: BTreeMap::from([(
            "phi".into(),
            BTreeMap::from([("mass".into(), json!(1))]),
        )]),
        assumptions: vec![SetupAssumption {
            id: "a-1".into(),
            text: "Massless scalar field in Euclidean d dimensions".into(),
        }],
        given_source: None,
        sources: Vec::new(),
        roles: SolveRoles {
            solver: RoleModel { provider: "openai".into(), model: "gpt-5".into() },
            verifier: RoleModel { provider: "anthropic".into(), model: "claude-sonnet".into() },
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        ask_before_running: false,
    }
}

#[test]
fn prefill_is_deterministic_fast_and_handles_worked_example() {
    let req = PrefillRequest {
        statement: WORKED_EXAMPLE.into(),
        pinned_conventions: "Euclidean signature, hbar = c = 1".into(),
        defaults: None,
        solver: Some(RoleModel { provider: "openai".into(), model: "gpt-5".into() }),
        verifier: Some(RoleModel { provider: "anthropic".into(), model: "claude-sonnet".into() }),
        task_kind_override: None,
        allow_numeric_target: false,
    };

    let t0 = Instant::now();
    let r1 = prefill_solve_setup(&req);
    let r2 = prefill_solve_setup(&req);
    assert!(t0.elapsed().as_millis() < 1_000, "prefill must run well under 1 s");
    assert_eq!(r1, r2, "prefill must be byte-for-byte deterministic");

    assert_eq!(r1.task_kind, TaskKind::Calculate);
    assert_eq!(r1.prove_notice, None);
    assert!(!r1.same_model);
    assert_eq!(r1.same_model_warning, None);

    let sym_names: Vec<&str> = r1.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in ["phi", "lambda", "d", "eps", "mu"] {
        assert!(sym_names.contains(&expected), "missing symbol `{expected}` in {sym_names:?}");
    }
    let lambda = r1.symbols.iter().find(|s| s.name == "lambda").unwrap();
    assert!(lambda.assumptions.contains(&"positive".to_string()));

    let cue = r1.vanishes_cue.as_ref().expect("should detect show ... = 0 at order lambda^2");
    assert_eq!(cue.order_variable, "lambda");
    assert_eq!(cue.order_power, 2);
    assert!(cue.expression.contains("dV/dmu"));
    assert!(r1.done.iter().any(|c| {
        c.kind == DoneCriterionKind::NamedCheckPassed
            && c.params["check"] == "vanishes_to_order"
            && c.params["order_variable"] == "lambda"
            && c.params["order_power"] == 2
    }));
}

#[test]
fn prefill_prove_task_includes_d21_notice_and_different_model_review_criterion() {
    let req = PrefillRequest {
        statement: "Prove that every bounded monotone sequence in \\R converges.".into(),
        pinned_conventions: String::new(),
        defaults: None,
        solver: Some(RoleModel { provider: "openai".into(), model: "gpt-5".into() }),
        verifier: Some(RoleModel { provider: "openai".into(), model: "gpt-5".into() }),
        task_kind_override: None,
        allow_numeric_target: false,
    };
    let out = prefill_solve_setup(&req);
    assert_eq!(out.task_kind, TaskKind::Prove);
    assert_eq!(out.target.form, TargetForm::Proof);
    assert_eq!(out.prove_notice.as_deref(), Some(PROVE_PROSE_NOTICE));
    assert!(out.same_model);
    assert_eq!(out.same_model_warning.as_deref(), Some(SAME_MODEL_WARNING));
    assert!(out.done.iter().any(|c| {
        c.kind == DoneCriterionKind::FinalTierAtLeast
            && c.params["tier"] == CRITERION_TIER_MODEL_AGREED
            && c.params["require_different_model"] == true
    }));
}

#[test]
fn prefill_check_derivation_and_f16a_defaults_override_heuristics() {
    let suggested = SuggestedSolveDefaults {
        symbols: vec![SymbolDefault {
            symbol: "phi".into(),
            domain: "qft".into(),
            assumptions: vec!["real".into(), "positive".into()],
            dimensions: Some("mass^1".into()),
            convention_note: None,
            origin_session: "s-prev".into(),
            updated_ms: 100,
        }],
        conventions: vec![ConventionDefault {
            key: "scheme".into(),
            value: "MS-bar".into(),
            domain: "qft".into(),
            origin_session: "s-prev".into(),
            updated_ms: 100,
        }],
    };
    let defaults = SetupDefaults::from_suggested_lessons(&suggested);
    let req = PrefillRequest {
        statement: "Check this derivation for \\phi and \\lambda.".into(),
        pinned_conventions: "hbar = c = 1".into(),
        defaults: Some(defaults),
        solver: None,
        verifier: None,
        task_kind_override: None,
        allow_numeric_target: true,
    };
    let out = prefill_solve_setup(&req);
    assert_eq!(out.task_kind, TaskKind::CheckDerivation);
    assert_eq!(out.target.form, TargetForm::PerStepVerdict);
    assert!(out.conventions.contains("scheme: MS-bar"));
    let phi = out.symbols.iter().find(|s| s.name == "phi").unwrap();
    assert_eq!(phi.assumptions, vec!["real", "positive"]);
    assert_eq!(out.dimensions["phi"]["mass"], json!(1));
    assert_eq!(out.done[0].params["tier"], CRITERION_TIER_NUMERIC);
}

#[test]
fn prefill_wording_satisfies_r22_rules() {
    for stmt in [
        WORKED_EXAMPLE,
        "Prove that n^2 + n is even for integer n.",
        "Check this derivation of \\alpha + \\beta = 0.",
    ] {
        for allow_num in [false, true] {
            let out = prefill_solve_setup(&PrefillRequest {
                statement: stmt.into(),
                allow_numeric_target: allow_num,
                ..Default::default()
            });
            let json_text = serde_json::to_string(&out).unwrap();
            assert!(!json_text.contains("Computed"), "retired word Computed leaked: {json_text}");
            assert!(
                !json_text.contains("Checked numerically"),
                "pre-2026-11-26 spelling leaked: {json_text}"
            );
            assert_eq!(
                out.target_tier_labels[CRITERION_TIER_NUMERIC],
                wording::BADGE_CHECKED_NUMERICALLY
            );
            assert_eq!(
                out.target_tier_labels[CRITERION_TIER_VERIFIED],
                wording::BADGE_VERIFIED
            );
            // "proved" never appears on computed labels or notices.
            assert!(!json_text.to_ascii_lowercase().contains("proved"));
        }
    }
}

#[test]
fn validation_enforces_size_symbol_dimension_budget_and_profile_limits() {
    let ok = validate_and_build_setup(&base_valid_input(), 0).unwrap();
    assert_eq!(ok.revision, 1);
    assert_eq!(ok.math_sha256.len(), 64);

    // Revision increments and changing conventions changes math_sha256 (D15).
    let mut rev2_in = base_valid_input();
    rev2_in.conventions = "Lorentzian signature (-,+,+,+)".into();
    let rev2 = validate_and_build_setup(&rev2_in, ok.revision).unwrap();
    assert_eq!(rev2.revision, 2);
    assert_ne!(rev2.math_sha256, ok.math_sha256);

    // Empty statement rejected.
    let mut bad = base_valid_input();
    bad.statement = "   ".into();
    assert!(validate_and_build_setup(&bad, 0).is_err());

    // Statement > 16,000 chars rejected.
    let mut bad = base_valid_input();
    bad.statement = "x".repeat(MAX_STATEMENT_CHARS + 1);
    assert!(validate_and_build_setup(&bad, 0).unwrap_err().contains("16000"));

    // Conventions > 2,000 chars rejected.
    let mut bad = base_valid_input();
    bad.conventions = "c".repeat(MAX_CONVENTIONS_CHARS + 1);
    assert!(validate_and_build_setup(&bad, 0).unwrap_err().contains("2000"));

    // Invalid symbol name rejected.
    let mut bad = base_valid_input();
    bad.symbols.push(SymbolSpec { name: "1bad".into(), assumptions: vec!["real".into()] });
    assert!(validate_and_build_setup(&bad, 0).is_err());

    // Duplicate symbol rejected.
    let mut bad = base_valid_input();
    bad.symbols.push(SymbolSpec { name: "phi".into(), assumptions: vec!["real".into()] });
    assert!(validate_and_build_setup(&bad, 0).unwrap_err().contains("duplicate symbol"));

    // Invalid dimension power rejected.
    let mut bad = base_valid_input();
    bad.dimensions.insert("phi".into(), BTreeMap::from([("mass".into(), json!(true))]));
    assert!(validate_and_build_setup(&bad, 0).is_err());

    // Valid rational string dimension accepted.
    let mut good_rat = base_valid_input();
    good_rat.dimensions.insert("phi".into(), BTreeMap::from([("mass".into(), json!("-3/2"))]));
    assert!(validate_and_build_setup(&good_rat, 0).is_ok());

    // Invalid budgets rejected.
    let mut bad = base_valid_input();
    bad.budgets.max_model_calls = 0;
    assert!(validate_and_build_setup(&bad, 0).is_err());

    let mut bad = base_valid_input();
    bad.budgets.max_model_calls = 10;
    bad.budgets.max_model_calls_per_leg = 20;
    assert!(validate_and_build_setup(&bad, 0).is_err());

    // Wrong tool_profile rejected.
    let mut bad = base_valid_input();
    bad.tool_profile = "research/1".into();
    assert!(validate_and_build_setup(&bad, 0).unwrap_err().contains("tool_profile"));

    // Zero-model setup (empty solver/verifier) succeeds cleanly (00-overview.md 5.1).
    let mut no_model = base_valid_input();
    no_model.roles = SolveRoles {
        solver: RoleModel { provider: String::new(), model: String::new() },
        verifier: RoleModel { provider: String::new(), model: String::new() },
        compute: None,
    };
    assert!(validate_and_build_setup(&no_model, 0).is_ok());

    // Non-boolean require_different_model rejected.
    let mut bad_rdm = base_valid_input();
    bad_rdm.done[0].params = json!({ "tier": "verified", "require_different_model": "yes" });
    assert!(validate_and_build_setup(&bad_rdm, 0).unwrap_err().contains("boolean"));

    // Duplicate trimmed dimension key rejected.
    let mut dup_dim = base_valid_input();
    dup_dim.dimensions.insert("phi".into(), BTreeMap::from([("mass".into(), json!(1))]));
    dup_dim.dimensions.insert(" phi ".into(), BTreeMap::from([("mass".into(), json!(2))]));
    assert!(validate_and_build_setup(&dup_dim, 0).unwrap_err().contains("duplicate"));
}

#[test]
fn prefill_avoids_false_positives_and_unwraps_big_o_vanishes_cue() {
    let cue_none = prefill_solve_setup(&PrefillRequest {
        statement: "Compute the overshow term to order eps^2.".into(),
        ..Default::default()
    });
    assert!(
        !cue_none.done.iter().any(|c| c.kind == DoneCriterionKind::NamedCheckPassed),
        "substring 'overshow ' must not trigger show-vanishes cue"
    );

    let cue_big_o = prefill_solve_setup(&PrefillRequest {
        statement: r"Show that $R_2(\lambda)$ vanishes to order $\mathcal{O}(\lambda^2)$.".into(),
        ..Default::default()
    });
    let named = cue_big_o
        .done
        .iter()
        .find(|c| c.kind == DoneCriterionKind::NamedCheckPassed)
        .expect("expected vanishes_to_order criterion");
    assert_eq!(named.params["check"], "vanishes_to_order");
    assert_eq!(named.params["order_variable"], "lambda");
    assert_eq!(named.params["order_power"], 2);
    assert!(!named.params["expression"].as_str().unwrap().contains("vanishes"));

    let scanned = scan_statement_symbols("Take the n-th x-ray peak in part (b) for $k > 0$.", None);
    let names: Vec<&str> = scanned.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"k"));
    assert!(!names.contains(&"n"), "n-th should not be scanned as symbol n");
    assert!(!names.contains(&"x"), "x-ray should not be scanned as symbol x");
    assert!(!names.contains(&"b"), "(b) should not be scanned as symbol b");
}

