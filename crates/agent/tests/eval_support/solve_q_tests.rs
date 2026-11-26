//! Focused tests for the F8 set Q Solve tier alignment (`00-overview.md` 0.1,
//! M3 in 6, 7.3; `solve.md` 4.4; `solve_badge.rs`). Included by `eval_harness.rs`.

use super::eval_support::manifest::{Set, Tier};
use super::eval_support::measures::{
    count_in, evaluate, judge, measure, percent, retired_measure, share, solve_wording, Cmp,
    GateVerdict, Outcome, Rule, SolveQCounts, MEASURES,
};
use super::eval_support::sheet::{render_markdown, validate_sheet, Build, Sheet};
use research_tools::solve_status::wording as solve_badge_wording;
use std::collections::BTreeMap;

const Q_MEASURE_IDS: [&str; 7] = [
    solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT,
    solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
    solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
    solve_wording::ID_STEPS_VERIFIED_SHARE,
    solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE,
    solve_wording::ID_STEPS_MODEL_AGREED_SHARE,
    solve_wording::ID_STEPS_UNCHECKED_SHARE,
];

fn all_passing_rules() -> BTreeMap<String, f64> {
    let mut all = BTreeMap::new();
    for d in MEASURES {
        match d.rule {
            Rule::Recorded => {}
            Rule::MustEqual(v) => drop(all.insert(d.id.into(), v)),
            Rule::MustBeZero => drop(all.insert(d.id.into(), 0.0)),
            Rule::Threshold(Cmp::Below, b) => drop(all.insert(d.id.into(), b / 2.0)),
            Rule::Threshold(Cmp::AtLeast, b) => drop(all.insert(d.id.into(), b)),
        }
    }
    all
}

fn dated_sheet() -> Sheet {
    let mut s = Sheet::blank(Tier::Fixed);
    s.date = "2026-11-26".into();
    s.build = Build {
        git_commit: Some("0123abc".into()),
        crate_version: Some(env!("CARGO_PKG_VERSION").into()),
    };
    s
}

#[test]
fn q_measures_match_s2_badge_wording_and_retire_computed() {
    assert_eq!(solve_wording::BADGE_VERIFIED, solve_badge_wording::BADGE_VERIFIED);
    assert_eq!(
        solve_wording::BADGE_CHECKED_NUMERICALLY,
        solve_badge_wording::BADGE_CHECKED_NUMERICALLY
    );
    assert_eq!(solve_wording::BADGE_MODEL_AGREED, solve_badge_wording::BADGE_MODEL_AGREED);
    assert_eq!(solve_wording::BADGE_UNCHECKED, solve_badge_wording::BADGE_UNCHECKED);
    assert_eq!(solve_wording::BADGE_FAILED, solve_badge_wording::BADGE_FAILED);

    for d in MEASURES {
        assert!(
            !d.id.to_ascii_lowercase().contains("computed")
                && !d.label.to_ascii_lowercase().contains("computed")
                && !d.consequence.to_ascii_lowercase().contains("computed"),
            "measure `{}` must not use retired `Computed` wording",
            d.id
        );
    }

    for id in Q_MEASURE_IDS {
        let def = measure(id).unwrap_or_else(|| panic!("missing Q measure `{id}`"));
        assert_eq!(def.applies_to, [Set::Q], "{id}");
        assert_eq!(def.spec_applies_to, "Q", "{id}");
    }

    assert_eq!(
        measure(solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT).unwrap().label,
        format!("Planted error ends {}", solve_badge_wording::BADGE_FAILED)
    );
    assert_eq!(
        measure(solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED).unwrap().label,
        format!("Planted error ends {}", solve_badge_wording::BADGE_VERIFIED)
    );
    assert_eq!(
        measure(solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY).unwrap().label,
        format!("Planted error ends {}", solve_badge_wording::BADGE_CHECKED_NUMERICALLY)
    );
    assert_eq!(
        measure(solve_wording::ID_STEPS_VERIFIED_SHARE).unwrap().label,
        format!("Share of steps {}", solve_badge_wording::BADGE_VERIFIED)
    );
    assert_eq!(
        measure(solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE).unwrap().label,
        format!("Share of steps {}", solve_badge_wording::BADGE_CHECKED_NUMERICALLY)
    );
    assert_eq!(
        measure(solve_wording::ID_STEPS_MODEL_AGREED_SHARE).unwrap().label,
        format!("Share of steps {}", solve_badge_wording::BADGE_MODEL_AGREED)
    );
    assert_eq!(
        measure(solve_wording::ID_STEPS_UNCHECKED_SHARE).unwrap().label,
        format!("Share of steps {}", solve_badge_wording::BADGE_UNCHECKED)
    );

    assert!(measure(solve_wording::RETIRED_PLANTED_ERROR_ENDS_COMPUTED).is_none());
    assert!(measure(solve_wording::RETIRED_STEPS_COMPUTED_SHARE).is_none());
    assert!(retired_measure(solve_wording::RETIRED_PLANTED_ERROR_ENDS_COMPUTED).is_some());
    assert!(retired_measure(solve_wording::RETIRED_STEPS_COMPUTED_SHARE).is_some());
}

#[test]
fn q_gate_enforces_failed_and_blocks_verified_or_checked_numerically_planted_errors() {
    // Full passing Q sample: 100% planted errors Failed, 0 Verified, 0 Checked numerically,
    // and separate step shares for Verified, Checked numerically, Model agreed, Unchecked.
    let mut values = all_passing_rules();
    SolveQCounts {
        planted_total: 3,
        planted_failed: 3,
        planted_verified: 0,
        planted_checked_numerically: 0,
        steps_total: 10,
        steps_verified: 4,
        steps_checked_numerically: 2,
        steps_model_agreed: 3,
        steps_unchecked: 1,
    }
    .record_into(&mut values)
    .unwrap();
    let report = evaluate(&values).unwrap();
    assert_eq!(report.verdict, GateVerdict::Pass);
    assert_eq!(values[solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT], 100.0);
    assert_eq!(values[solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED], 0.0);
    assert_eq!(values[solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY], 0.0);
    assert_eq!(values[solve_wording::ID_STEPS_VERIFIED_SHARE], 0.4);
    assert_eq!(values[solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE], 0.2);
    assert_eq!(values[solve_wording::ID_STEPS_MODEL_AGREED_SHARE], 0.3);
    assert_eq!(values[solve_wording::ID_STEPS_UNCHECKED_SHARE], 0.1);
    for id in &Q_MEASURE_IDS[..3] {
        assert_eq!(report.outcome_of(id), Some(Outcome::Pass), "{id}");
    }
    for id in &Q_MEASURE_IDS[3..] {
        assert_eq!(report.outcome_of(id), Some(Outcome::RecordedOnly), "{id}");
    }

    // Planted error ending Verified, Checked numerically, or Unchecked all fail the gate.
    for (counts, expected_fail) in [
        (
            SolveQCounts { planted_total: 3, planted_failed: 2, planted_verified: 1, ..Default::default() },
            vec![
                solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT,
                solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
            ],
        ),
        (
            SolveQCounts {
                planted_total: 3,
                planted_failed: 2,
                planted_checked_numerically: 1,
                ..Default::default()
            },
            vec![
                solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT,
                solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
            ],
        ),
        (
            SolveQCounts { planted_total: 3, planted_failed: 2, ..Default::default() },
            vec![solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT],
        ),
    ] {
        let mut v = all_passing_rules();
        counts.record_into(&mut v).unwrap();
        let rep = evaluate(&v).unwrap();
        assert_eq!(rep.verdict, GateVerdict::Fail);
        assert_eq!(rep.ids_with(Outcome::Fail), expected_fail);
    }

    // Even in isolation, any planted error ending Verified or Checked numerically fails;
    // leaving either count unmeasured keeps the gate INCOMPLETE.
    for id in [
        solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
        solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
    ] {
        let rule = measure(id).unwrap().rule;
        assert_eq!(judge(&rule, Some(0.0)), Outcome::Pass);
        assert_eq!(judge(&rule, Some(1.0)), Outcome::Fail);
        assert_eq!(judge(&rule, None), Outcome::NotMeasured);

        let mut v = all_passing_rules();
        v.insert(id.into(), 1.0);
        let rep = evaluate(&v).unwrap();
        assert_eq!(rep.verdict, GateVerdict::Fail);
        assert_eq!(rep.ids_with(Outcome::Fail), vec![id]);

        v.remove(id);
        let rep = evaluate(&v).unwrap();
        assert_eq!(rep.verdict, GateVerdict::Incomplete);
        assert_eq!(rep.unmeasured_rules(), vec![id]);
    }
}

#[test]
fn q_zero_case_path_stays_unmeasured_and_rejects_inconsistent_counts() {
    assert_eq!(percent(0, 0), None);
    assert_eq!(share(0, 0), None);
    assert_eq!(count_in(0, 0), None);
    assert_eq!(count_in(0, 3), Some(0.0));
    assert_eq!(count_in(1, 3), Some(1.0));

    // Empty sample: nothing written, every Q measure stays NotMeasured, gate stays INCOMPLETE.
    let mut empty = BTreeMap::new();
    SolveQCounts::default().record_into(&mut empty).unwrap();
    assert!(empty.is_empty());
    let rep = evaluate(&empty).unwrap();
    assert_eq!(rep.verdict, GateVerdict::Incomplete);
    for id in Q_MEASURE_IDS {
        assert_eq!(rep.outcome_of(id), Some(Outcome::NotMeasured), "{id}");
    }
    let open = rep.unmeasured_rules();
    for id in &Q_MEASURE_IDS[..3] {
        assert!(open.contains(id), "expected `{id}` in unmeasured_rules");
    }

    // Steps measured while 0 planted-error cases ran: record_into clears stale
    // planted-error keys without touching unrelated keys, keeping the gate INCOMPLETE.
    let mut steps_only = all_passing_rules();
    steps_only.insert("final_answer_equals_key".into(), 1.0);
    SolveQCounts {
        steps_total: 4,
        steps_verified: 2,
        steps_checked_numerically: 1,
        steps_model_agreed: 1,
        ..Default::default()
    }
    .record_into(&mut steps_only)
    .unwrap();
    assert_eq!(steps_only.get("final_answer_equals_key"), Some(&1.0));
    let rep = evaluate(&steps_only).unwrap();
    assert_eq!(rep.verdict, GateVerdict::Incomplete);
    assert_eq!(rep.unmeasured_rules(), Q_MEASURE_IDS[..3].to_vec());
    for id in &Q_MEASURE_IDS[3..] {
        assert_eq!(rep.outcome_of(id), Some(Outcome::RecordedOnly), "{id}");
    }

    // Planted errors measured while 0 steps counted: planted-error rules pass, step shares stay NotMeasured.
    let mut planted_only = all_passing_rules();
    planted_only.insert(solve_wording::ID_STEPS_VERIFIED_SHARE.into(), 0.5);
    SolveQCounts { planted_total: 3, planted_failed: 3, ..Default::default() }
        .record_into(&mut planted_only)
        .unwrap();
    let rep = evaluate(&planted_only).unwrap();
    assert_eq!(rep.verdict, GateVerdict::Pass);
    for id in &Q_MEASURE_IDS[3..] {
        assert_eq!(rep.outcome_of(id), Some(Outcome::NotMeasured), "{id}");
    }

    // Exact percent boundary behavior: part == whole is always 100.0; part < whole is always < 100.0.
    assert_eq!(percent(5_928_434_753_763_435, 5_928_434_753_763_435), Some(100.0));
    assert!(percent(1u64 << 53, (1u64 << 53) + 1).unwrap() < 100.0);

    // Non-zero counts on an empty sample, or mutually exclusive tier counts exceeding totals, fail closed.
    for bad in [
        SolveQCounts { planted_total: 0, planted_failed: 1, ..Default::default() },
        SolveQCounts { planted_total: 0, planted_verified: 1, ..Default::default() },
        SolveQCounts { planted_total: 2, planted_failed: 2, planted_checked_numerically: 1, ..Default::default() },
        SolveQCounts { steps_total: 0, steps_verified: 1, ..Default::default() },
        SolveQCounts { steps_total: 2, steps_verified: 2, steps_checked_numerically: 1, ..Default::default() },
        SolveQCounts { planted_total: u64::MAX, planted_failed: u64::MAX, planted_verified: 1, ..Default::default() },
    ] {
        let mut target = BTreeMap::from([(solve_wording::ID_STEPS_VERIFIED_SHARE.to_string(), 0.5)]);
        assert!(bad.record_into(&mut target).is_err());
        assert_eq!(
            target.get(solve_wording::ID_STEPS_VERIFIED_SHARE),
            Some(&0.5),
            "failed record_into must not mutate values"
        );
    }
}

#[test]
fn q_evaluate_rejects_out_of_domain_values_and_double_counted_step_shares() {
    for (id, bad_val) in [
        (solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED, -0.0),
        (solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED, f64::INFINITY),
        (solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED, 0.5),
        (solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY, 1.5),
        (solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT, 100.1),
        (solve_wording::ID_STEPS_VERIFIED_SHARE, 1.01),
        (solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE, -0.0),
    ] {
        assert!(
            evaluate(&BTreeMap::from([(id.to_string(), bad_val)])).is_err(),
            "expected `{id}` = {bad_val} to be rejected"
        );
    }

    // Mutually exclusive Solve step shares cannot sum above 1.
    let double_counted = BTreeMap::from([
        (solve_wording::ID_STEPS_VERIFIED_SHARE.to_string(), 0.7),
        (solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE.to_string(), 0.5),
    ]);
    let err = evaluate(&double_counted).unwrap_err();
    assert!(err.contains("cannot exceed 1"), "{err}");
}

#[test]
fn saved_pre_revision_computed_sheets_fail_closed_with_migration_guidance() {
    for (old_key, val, repl_a, repl_b) in [
        (
            solve_wording::RETIRED_PLANTED_ERROR_ENDS_COMPUTED,
            0.0,
            solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
            solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
        ),
        (
            solve_wording::RETIRED_PLANTED_ERROR_ENDS_COMPUTED,
            1.0,
            solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
            solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
        ),
        (
            solve_wording::RETIRED_STEPS_COMPUTED_SHARE,
            0.75,
            solve_wording::ID_STEPS_VERIFIED_SHARE,
            solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE,
        ),
    ] {
        // Realistic saved pre-revision sheet containing valid Q keys alongside the retired key.
        let mut mixed = BTreeMap::from([
            ("final_answer_equals_key".to_string(), 1.0),
            (solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT.to_string(), 100.0),
            (solve_wording::ID_STEPS_MODEL_AGREED_SHARE.to_string(), 0.15),
            (solve_wording::ID_STEPS_UNCHECKED_SHARE.to_string(), 0.1),
            (old_key.to_string(), val),
        ]);
        let err = evaluate(&mixed).unwrap_err();
        assert!(
            err.contains(old_key) && err.contains(repl_a) && err.contains(repl_b),
            "retired-key error should name old key and both replacements, got: {err}"
        );

        let mut sheet = dated_sheet();
        sheet.values = std::mem::take(&mut mixed);
        let errs = validate_sheet(&sheet);
        assert!(
            errs.iter().any(|e| e.contains(old_key) && e.contains(repl_a) && e.contains(repl_b)),
            "validate_sheet should reject retired key `{old_key}`, got: {errs:?}"
        );
        assert!(render_markdown(&sheet).is_err());
    }

    // Retired keys in measure_notes are rejected by validate_sheet as well.
    for old_key in [
        solve_wording::RETIRED_PLANTED_ERROR_ENDS_COMPUTED,
        solve_wording::RETIRED_STEPS_COMPUTED_SHARE,
    ] {
        let mut sheet = dated_sheet();
        sheet.measure_notes.insert(old_key.into(), "historical note".into());
        let errs = validate_sheet(&sheet);
        assert!(errs.iter().any(|e| e.contains(old_key)), "{errs:?}");
    }
}
