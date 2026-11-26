//! The measures of overview section 7.3 as data, and the gate over them.
//!
//! Every target below is transcribed from the table in 7.3; nothing is
//! added. Where the table says "Recorded" the rule is [`Rule::Recorded`]
//! and the value can never pass or fail. A measure with no value is
//! [`Outcome::NotMeasured`], never a pass. Pure module (std only).

use super::manifest::Set;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    /// Passes when the value is strictly below the bound.
    Below,
    /// Passes when the value is at least the bound.
    AtLeast,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    Recorded,
    MustEqual(f64),
    MustBeZero,
    Threshold(Cmp, f64),
}

impl Rule {
    pub fn kind(&self) -> &'static str {
        match self {
            Rule::Recorded => "recorded",
            Rule::MustEqual(_) => "must_equal",
            Rule::MustBeZero => "must_be_zero",
            Rule::Threshold(..) => "threshold",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Rule::Recorded => "recorded".into(),
            Rule::MustEqual(v) => format!("must equal {v}"),
            Rule::MustBeZero => "must be 0".into(),
            Rule::Threshold(Cmp::Below, b) => format!("below {b}"),
            Rule::Threshold(Cmp::AtLeast, b) => format!("at least {b}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureDef {
    pub id: &'static str,
    /// What the number is, in the sheet's words.
    pub label: &'static str,
    /// `seconds`, `share` (0 to 1), `percent` (0 to 100), `count`, `megabytes`.
    pub unit: &'static str,
    /// Sets the measure is taken on; empty means every set ("All").
    pub applies_to: &'static [Set],
    /// The "Applies to" cell of 7.3, word for word.
    pub spec_applies_to: &'static str,
    pub rule: Rule,
    /// What 7.3 says the number decides or blocks, word for word where it
    /// says anything; empty otherwise.
    pub consequence: &'static str,
}

const ALL: &[Set] = &[];
const INVESTIGATE_AND_EXPLORE: &[Set] = &[Set::P, Set::P2, Set::P3, Set::P4, Set::P5, Set::I];

/// Solve tier wording governed by the 2026-11-25 owner decisions
/// (`00-overview.md` 0.1, 8.1, 8.3 question 11; `solve.md` 4.4 / D1).
/// Kept in one module here (pure `std`, no dependency on `research-tools`) so a
/// wording change updates every Q measure in one place; cross-checked against
/// `research_tools::solve_status::wording` in `eval_support/solve_q_tests.rs`
/// (included by `eval_harness.rs`).
pub mod solve_wording {
    macro_rules! define_solve_wording {
        (
            verified = $verified:literal,
            checked_numerically = $checked_numerically:literal,
            model_agreed = $model_agreed:literal,
            unchecked = $unchecked:literal,
            failed = $failed:literal,
        ) => {
            /// Owner decision (`00-overview.md` 0.1 & 8.1, `solve.md` 4.4 / D1): exact symbolic check passed.
            pub const BADGE_VERIFIED: &str = $verified;
            /// Owner decision (`00-overview.md` 8.3 question 11, `solve.md` 4.4 / D1): numeric check passed.
            pub const BADGE_CHECKED_NUMERICALLY: &str = $checked_numerically;
            /// Owner decision (`00-overview.md` 3.2 row 24; section 8 accepted 2026-11-25).
            pub const BADGE_MODEL_AGREED: &str = $model_agreed;
            pub const BADGE_UNCHECKED: &str = $unchecked;
            pub const BADGE_FAILED: &str = $failed;

            pub const LABEL_PLANTED_ERROR_ENDS_FAILED: &str = concat!("Planted error ends ", $failed);
            pub const LABEL_PLANTED_ERROR_ENDS_VERIFIED: &str = concat!("Planted error ends ", $verified);
            pub const LABEL_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY: &str =
                concat!("Planted error ends ", $checked_numerically);
            pub const LABEL_STEPS_VERIFIED_SHARE: &str = concat!("Share of steps ", $verified);
            pub const LABEL_STEPS_CHECKED_NUMERICALLY_SHARE: &str =
                concat!("Share of steps ", $checked_numerically);
            pub const LABEL_STEPS_MODEL_AGREED_SHARE: &str = concat!("Share of steps ", $model_agreed);
            pub const LABEL_STEPS_UNCHECKED_SHARE: &str = concat!("Share of steps ", $unchecked);

            /// Owner decision (`00-overview.md` 0.1, M3 in 6, 7.3, 8.3 question 11).
            pub const CONSEQUENCE_PLANTED_ERROR_RELEASE_BLOCKER: &str =
                concat!("ending ", $verified, " or ", $checked_numerically, " is a release blocker");
        };
    }

    define_solve_wording! {
        verified = "Verified",
        checked_numerically = "Numerically checked",
        model_agreed = "Model agreed",
        unchecked = "Unchecked",
        failed = "Failed",
    }

    pub const ID_PLANTED_ERROR_ENDS_FAILED_PCT: &str = "planted_error_ends_failed_pct";
    pub const ID_PLANTED_ERROR_ENDS_VERIFIED: &str = "planted_error_ends_verified";
    pub const ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY: &str =
        "planted_error_ends_checked_numerically";
    pub const ID_STEPS_VERIFIED_SHARE: &str = "steps_verified_share";
    pub const ID_STEPS_CHECKED_NUMERICALLY_SHARE: &str = "steps_checked_numerically_share";
    pub const ID_STEPS_MODEL_AGREED_SHARE: &str = "steps_model_agreed_share";
    pub const ID_STEPS_UNCHECKED_SHARE: &str = "steps_unchecked_share";

    pub const CONSEQUENCE_SYMPY_REACH: &str = "decides whether SymPy's reach is enough";

    /// Pre-revision Q measure ids retired by the 2026-11-25 scope revision
    /// (`00-overview.md` 0.1, 7.3). An old `Computed` value does not
    /// distinguish exact symbolic checks (`Verified`) from numeric checks
    /// (`Numerically checked`).
    pub const RETIRED_PLANTED_ERROR_ENDS_COMPUTED: &str = "planted_error_ends_computed";
    pub const RETIRED_STEPS_COMPUTED_SHARE: &str = "steps_computed_share";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredMeasure {
    pub id: &'static str,
    pub replacements: &'static [&'static str],
    pub reason: &'static str,
}

/// Pre-revision measure ids rejected fail-closed by [`evaluate`].
/// A saved sheet carrying one of these must be re-measured rather than
/// silently mapped onto a replacement key, because `Computed` conflated exact
/// symbolic checks (`Verified`) and numeric checks (`Numerically checked`).
pub const RETIRED_MEASURES: &[RetiredMeasure] = &[
    RetiredMeasure {
        id: solve_wording::RETIRED_PLANTED_ERROR_ENDS_COMPUTED,
        replacements: &[
            solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
            solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
        ],
        reason: "retired by the 2026-11-25 Solve tier revision (`00-overview.md` 0.1, 7.3): an old `Computed` count does not distinguish `Verified` from `Checked numerically`",
    },
    RetiredMeasure {
        id: solve_wording::RETIRED_STEPS_COMPUTED_SHARE,
        replacements: &[
            solve_wording::ID_STEPS_VERIFIED_SHARE,
            solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE,
        ],
        reason: "retired by the 2026-11-25 Solve tier revision (`00-overview.md` 0.1, 7.3): an old `Computed` share does not distinguish `Verified` from `Checked numerically`",
    },
];

pub fn retired_measure(id: &str) -> Option<&'static RetiredMeasure> {
    RETIRED_MEASURES.iter().find(|r| r.id == id)
}

const fn m(
    id: &'static str,
    label: &'static str,
    unit: &'static str,
    applies_to: &'static [Set],
    spec_applies_to: &'static str,
    rule: Rule,
    consequence: &'static str,
) -> MeasureDef {
    MeasureDef { id, label, unit, applies_to, spec_applies_to, rule, consequence }
}

/// The table of 7.3, row by row, in its order. A row that states several
/// numbers becomes several entries.
pub const MEASURES: &[MeasureDef] = &[
    // Time from input to first deterministic content.
    m("time_to_skeleton_tex_s", "Investigate: time from input to skeleton, TeX paper", "seconds", &[Set::P], "All modes", Rule::Threshold(Cmp::Below, 10.0), "Investigate TeX under 10 s"),
    m("time_to_candidates_s", "Explore: time from input to candidates", "seconds", &[Set::I], "All modes", Rule::Threshold(Cmp::Below, 6.0), "Explore candidates under 6 s"),
    m("time_to_solve_setup_s", "Solve: time from input to setup", "seconds", &[Set::Q], "All modes", Rule::Threshold(Cmp::Below, 1.0), "Solve setup under 1 s"),
    // Model calls and tokens per run, against the cap.
    m("runs_above_cap", "Runs whose model calls or tokens went above the cap", "count", ALL, "All", Rule::MustBeZero, "Never above the cap"),
    m("model_calls_typical", "Typical model calls per run", "count", ALL, "All", Rule::Recorded, "typical value recorded"),
    m("tokens_typical", "Typical tokens per run", "count", ALL, "All", Rule::Recorded, "typical value recorded"),
    // Theorem-like statement recall; section recall.
    m("statement_recall", "Theorem-like statement recall, all papers of P", "share", &[Set::P], "P", Rule::Recorded, ""),
    m("section_recall", "Section recall, all papers of P", "share", &[Set::P], "P", Rule::Recorded, ""),
    m("statement_recall_tex_theorem_envs", "Theorem-like statement recall, TeX papers with theorem environments", "share", &[Set::P], "P", Rule::Threshold(Cmp::AtLeast, 0.9), "below 0.9 on TeX papers with theorem environments blocks M1"),
    m("section_recall_tex_theorem_envs", "Section recall, TeX papers with theorem environments", "share", &[Set::P], "P", Rule::Threshold(Cmp::AtLeast, 0.9), "below 0.9 on TeX papers with theorem environments blocks M1"),
    // "Uses" edge precision, statement and proof roles separately.
    m("uses_edge_precision_statement", "\"Uses\" edge precision, statement role", "share", &[Set::P], "P", Rule::Recorded, ""),
    m("uses_edge_precision_proof", "\"Uses\" edge precision, proof role", "share", &[Set::P], "P", Rule::Recorded, ""),
    // Tags unchanged for unchanged statements; wrong carry-overs.
    m("tags_unchanged_share", "Tags unchanged for unchanged statements", "share", &[Set::P2], "P2", Rule::Recorded, ""),
    m("wrong_carry_overs", "Wrong carry-overs", "count", &[Set::P2], "P2", Rule::MustBeZero, "Wrong carry-overs must be 0"),
    // Authors' label agreement with the key; share of labels whose quote contains a cue.
    m("author_label_agreement", "Authors' label agreement with the key", "share", &[Set::P3], "P3", Rule::Recorded, ""),
    m("label_quote_cue_share", "Share of labels whose quote contains a cue", "share", &[Set::P3], "P3", Rule::Recorded, ""),
    // Known earlier source found; "none in scope" shown for a result that has one.
    m("earlier_source_found", "Known earlier source found", "share", &[Set::P4, Set::I], "P4, I", Rule::Recorded, ""),
    m("none_in_scope_false_negative_rate", "\"None in scope\" shown for a result that has a known earlier source", "share", &[Set::P4, Set::I], "P4, I", Rule::Recorded, "The second number is the false-negative rate that decides whether the feature is announced"),
    // "Judged same" where the key says different.
    m("judged_same_key_different", "\"Judged same\" where the key says different", "count", &[Set::P4, Set::I], "P4, I", Rule::Recorded, "Recorded; every such case reviewed"),
    m("judged_same_key_different_unreviewed", "Of those, cases not yet reviewed", "count", &[Set::P4, Set::I], "P4, I", Rule::MustBeZero, "every such case reviewed"),
    // Wrong statement shown as located.
    m("wrong_statement_located", "Wrong statement shown as located", "count", &[Set::P5], "P5", Rule::MustBeZero, "Must be 0 (candidates list is acceptable)"),
    // Known-paper recall of the search alone, before any model call.
    m("search_recall_keyless", "Known-paper recall of the search alone, keyless services", "share", &[Set::I], "I", Rule::Recorded, "Recorded per service set (keyless; with keys)"),
    m("search_recall_with_keys", "Known-paper recall of the search alone, services with keys", "share", &[Set::I], "I", Rule::Recorded, "Recorded per service set (keyless; with keys)"),
    // Quotes accepted per model call; quotes rejected and why.
    m("quotes_accepted_per_model_call", "Quotes accepted per model call", "count", INVESTIGATE_AND_EXPLORE, "Investigate tasks, Explore readers", Rule::Recorded, ""),
    m("quotes_rejected", "Quotes rejected (reasons in the note)", "count", INVESTIGATE_AND_EXPLORE, "Investigate tasks, Explore readers", Rule::Recorded, ""),
    // Planted error ends Failed (overview 7.3; 2026-11-25 Solve tier revision).
    m(solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT, solve_wording::LABEL_PLANTED_ERROR_ENDS_FAILED, "percent", &[Set::Q], "Q", Rule::MustEqual(100.0), "Must be 100 percent"),
    m(solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED, solve_wording::LABEL_PLANTED_ERROR_ENDS_VERIFIED, "count", &[Set::Q], "Q", Rule::MustBeZero, solve_wording::CONSEQUENCE_PLANTED_ERROR_RELEASE_BLOCKER),
    m(solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY, solve_wording::LABEL_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY, "count", &[Set::Q], "Q", Rule::MustBeZero, solve_wording::CONSEQUENCE_PLANTED_ERROR_RELEASE_BLOCKER),
    // Share of steps Verified, Numerically checked, Model agreed, Unchecked.
    m(solve_wording::ID_STEPS_VERIFIED_SHARE, solve_wording::LABEL_STEPS_VERIFIED_SHARE, "share", &[Set::Q], "Q", Rule::Recorded, solve_wording::CONSEQUENCE_SYMPY_REACH),
    m(solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE, solve_wording::LABEL_STEPS_CHECKED_NUMERICALLY_SHARE, "share", &[Set::Q], "Q", Rule::Recorded, solve_wording::CONSEQUENCE_SYMPY_REACH),
    m(solve_wording::ID_STEPS_MODEL_AGREED_SHARE, solve_wording::LABEL_STEPS_MODEL_AGREED_SHARE, "share", &[Set::Q], "Q", Rule::Recorded, solve_wording::CONSEQUENCE_SYMPY_REACH),
    m(solve_wording::ID_STEPS_UNCHECKED_SHARE, solve_wording::LABEL_STEPS_UNCHECKED_SHARE, "share", &[Set::Q], "Q", Rule::Recorded, solve_wording::CONSEQUENCE_SYMPY_REACH),
    // Final answer equals the key.
    m("final_answer_equals_key", "Final answer equals the key", "share", &[Set::Q], "Q", Rule::Recorded, ""),
    // Quotes surviving re-matching after a hand-over of unchanged bytes.
    m("quotes_surviving_handover_pct", "Quotes surviving re-matching after a hand-over of unchanged bytes", "percent", &[Set::C], "C", Rule::MustEqual(100.0), "Must be 100 percent"),
    // Target case solved correctly when started from a hand-over.
    m("handover_target_solved", "Target case solved correctly when started from a hand-over", "share", &[Set::C], "C", Rule::Recorded, ""),
    // Old ledgers resume; views equal before and after.
    m("old_ledgers_not_resuming", "Old ledgers that do not resume", "count", &[Set::R], "R", Rule::MustBeZero, "Must pass; in CI"),
    m("old_ledgers_view_differs", "Old ledgers whose views differ before and after", "count", &[Set::R], "R", Rule::MustBeZero, "Must pass; in CI"),
    // Peak memory and build time on the reference laptop.
    m("peak_memory_mb", "Peak memory on the reference laptop", "megabytes", ALL, "All", Rule::Recorded, "Recorded at each milestone"),
    m("build_time_s", "Build time on the reference laptop", "seconds", ALL, "All", Rule::Recorded, "Recorded at each milestone"),
    // A view never shows "new", "novel", "proved" or "verified" outside the allowed places.
    m("forbidden_word_property_failures", "Failing property tests for \"new\", \"novel\", \"proved\", \"verified\" outside the allowed places", "count", ALL, "All", Rule::MustBeZero, "Property tests in CI"),
];

pub fn measure(id: &str) -> Option<&'static MeasureDef> {
    MEASURES.iter().find(|d| d.id == id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    RecordedOnly,
    NotMeasured,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "FAIL",
            Outcome::RecordedOnly => "recorded",
            Outcome::NotMeasured => "not measured",
        }
    }
}

/// Applies one rule to one value. `None` is always `NotMeasured`.
/// `MustEqual` compares exactly: 100 percent means every case, so compute
/// the value with [`percent`] rather than rounding by hand.
pub fn judge(rule: &Rule, value: Option<f64>) -> Outcome {
    let Some(v) = value else { return Outcome::NotMeasured };
    let pass = match rule {
        Rule::Recorded => return Outcome::RecordedOnly,
        Rule::MustEqual(target) => v == *target,
        Rule::MustBeZero => v == 0.0,
        Rule::Threshold(Cmp::Below, bound) => v < *bound,
        Rule::Threshold(Cmp::AtLeast, bound) => v >= *bound,
    };
    if pass {
        Outcome::Pass
    } else {
        Outcome::Fail
    }
}

/// `100 * part / whole`, or `None` when nothing was counted, so that
/// "0 of 0 planted errors ended Failed" stays not measured.
/// Guarantees `Some(100.0)` iff `whole > 0 && part == whole` (exact, with no
/// rounding up of `part < whole` or float overshoot when `part == whole`).
pub fn percent(part: u64, whole: u64) -> Option<f64> {
    if whole == 0 {
        return None;
    }
    if part == whole {
        return Some(100.0);
    }
    let pct = 100.0 * part as f64 / whole as f64;
    if part < whole && pct >= 100.0 {
        Some(f64::from_bits(100.0_f64.to_bits() - 1))
    } else {
        Some(pct)
    }
}

/// `part / whole` in 0 to 1, or `None` when nothing was counted.
pub fn share(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

/// `count` as `f64`, or `None` when nothing was sampled (`sample_size == 0`),
/// so that "0 of 0 planted errors ended Verified" stays not measured rather
/// than passing a `MustBeZero` rule on an empty sample.
pub fn count_in(count: u64, sample_size: u64) -> Option<f64> {
    (sample_size > 0).then_some(count as f64)
}

/// Counts from a set Q run, mapped onto the accepted Solve measures of
/// section 7.3 (`Verified`, `Numerically checked`, `Model agreed`, `Unchecked`,
/// and planted-error `Failed` / `Verified` / `Numerically checked`).
///
/// The four recorded step tiers are mutually exclusive; `steps_total` may also
/// include steps in other tiers (`Given`, `Cited`, `CheckedByYou`,
/// `ModelAgreedSameModel`, `AssumedByModel`, `Failed`, `Stale`), so their sum
/// must satisfy `sum <= steps_total`. Likewise planted-error counts satisfy
/// `planted_failed + planted_verified + planted_checked_numerically <= planted_total`.
///
/// Empty samples (`planted_total == 0` or `steps_total == 0`) leave or clear
/// the corresponding measures so the gate reports `NotMeasured` rather than
/// passing or recording on zero cases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SolveQCounts {
    pub planted_total: u64,
    pub planted_failed: u64,
    pub planted_verified: u64,
    pub planted_checked_numerically: u64,
    pub steps_total: u64,
    pub steps_verified: u64,
    pub steps_checked_numerically: u64,
    pub steps_model_agreed: u64,
    pub steps_unchecked: u64,
}

impl SolveQCounts {
    /// Writes every measured Q metric into `values`, clearing any stale Q keys
    /// whose sample total is 0. Rejects inconsistent counts without mutating `values`.
    pub fn record_into(&self, values: &mut BTreeMap<String, f64>) -> Result<(), String> {
        let planted_sum = self
            .planted_failed
            .checked_add(self.planted_verified)
            .and_then(|s| s.checked_add(self.planted_checked_numerically))
            .ok_or_else(|| "planted-error tier counts overflow u64".to_string())?;
        if planted_sum > self.planted_total {
            return Err(format!(
                "planted-error tier counts ({planted_sum}) exceed planted_total ({})",
                self.planted_total
            ));
        }
        let step_sum = self
            .steps_verified
            .checked_add(self.steps_checked_numerically)
            .and_then(|s| s.checked_add(self.steps_model_agreed))
            .and_then(|s| s.checked_add(self.steps_unchecked))
            .ok_or_else(|| "step tier counts overflow u64".to_string())?;
        if step_sum > self.steps_total {
            return Err(format!(
                "step tier counts ({step_sum}) exceed steps_total ({})",
                self.steps_total
            ));
        }
        let mut set_or_clear = |id: &str, v: Option<f64>| match v {
            Some(val) => {
                values.insert(id.to_string(), val);
            }
            None => {
                values.remove(id);
            }
        };
        set_or_clear(
            solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT,
            percent(self.planted_failed, self.planted_total),
        );
        set_or_clear(
            solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED,
            count_in(self.planted_verified, self.planted_total),
        );
        set_or_clear(
            solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY,
            count_in(self.planted_checked_numerically, self.planted_total),
        );
        set_or_clear(
            solve_wording::ID_STEPS_VERIFIED_SHARE,
            share(self.steps_verified, self.steps_total),
        );
        set_or_clear(
            solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE,
            share(self.steps_checked_numerically, self.steps_total),
        );
        set_or_clear(
            solve_wording::ID_STEPS_MODEL_AGREED_SHARE,
            share(self.steps_model_agreed, self.steps_total),
        );
        set_or_clear(
            solve_wording::ID_STEPS_UNCHECKED_SHARE,
            share(self.steps_unchecked, self.steps_total),
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    /// Every measure that has a pass rule was measured and passed.
    Pass,
    /// At least one measure with a pass rule failed.
    Fail,
    /// Nothing failed, but at least one measure with a pass rule has no value.
    Incomplete,
}

impl GateVerdict {
    pub fn label(self) -> &'static str {
        match self {
            GateVerdict::Pass => "PASS",
            GateVerdict::Fail => "FAIL",
            GateVerdict::Incomplete => "INCOMPLETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasureResult {
    pub def: &'static MeasureDef,
    pub value: Option<f64>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateReport {
    /// One entry per measure, in the order of [`MEASURES`].
    pub results: Vec<MeasureResult>,
    pub verdict: GateVerdict,
}

impl GateReport {
    pub fn outcome_of(&self, id: &str) -> Option<Outcome> {
        self.results.iter().find(|r| r.def.id == id).map(|r| r.outcome)
    }

    pub fn ids_with(&self, outcome: Outcome) -> Vec<&'static str> {
        self.results.iter().filter(|r| r.outcome == outcome).map(|r| r.def.id).collect()
    }

    /// Measures that have a pass rule and no value: what keeps the gate
    /// from being decided.
    pub fn unmeasured_rules(&self) -> Vec<&'static str> {
        self.results
            .iter()
            .filter(|r| r.outcome == Outcome::NotMeasured && r.def.rule != Rule::Recorded)
            .map(|r| r.def.id)
            .collect()
    }
}

/// Judges every measure of 7.3 against the measured values. A value for a
/// retired pre-revision id, an id not in the table, or a value outside its
/// unit domain is an error: a sheet must not carry numbers the gate does not
/// know or conflate retired `Computed` data with exact vs numeric tiers.
pub fn evaluate(values: &BTreeMap<String, f64>) -> Result<GateReport, String> {
    for (id, v) in values {
        if let Some(retired) = retired_measure(id) {
            let repl = retired
                .replacements
                .iter()
                .map(|r| format!("`{r}`"))
                .collect::<Vec<_>>()
                .join(" and ");
            return Err(format!(
                "`{id}` is retired ({}); re-measure and supply {repl}",
                retired.reason
            ));
        }
        let Some(def) = measure(id) else {
            return Err(format!("`{id}` is not a measure of section 7.3"));
        };
        if !v.is_finite() || v.is_sign_negative() {
            return Err(format!("`{id}` has the value {v}; a measure is a finite number, 0 or above"));
        }
        if def.unit == "share" && *v > 1.0 {
            return Err(format!("`{id}` has the value {v}; a share must be between 0 and 1"));
        }
        if def.unit == "percent" && *v > 100.0 {
            return Err(format!("`{id}` has the value {v}; a percent must be between 0 and 100"));
        }
        if def.unit == "count" && def.rule == Rule::MustBeZero && v.fract() != 0.0 {
            return Err(format!("`{id}` has the value {v}; a case-error count must be a whole number"));
        }
    }
    let q_step_share_sum: f64 = [
        solve_wording::ID_STEPS_VERIFIED_SHARE,
        solve_wording::ID_STEPS_CHECKED_NUMERICALLY_SHARE,
        solve_wording::ID_STEPS_MODEL_AGREED_SHARE,
        solve_wording::ID_STEPS_UNCHECKED_SHARE,
    ]
    .iter()
    .filter_map(|id| values.get(*id))
    .sum();
    if q_step_share_sum > 1.0 + 1e-9 {
        return Err(format!(
            "Solve step shares sum to {q_step_share_sum}; mutually exclusive tier shares cannot exceed 1"
        ));
    }
    let results: Vec<MeasureResult> = MEASURES
        .iter()
        .map(|def| {
            let value = values.get(def.id).copied();
            MeasureResult { def, value, outcome: judge(&def.rule, value) }
        })
        .collect();
    let failed = results.iter().any(|r| r.outcome == Outcome::Fail);
    let open = results.iter().any(|r| r.outcome == Outcome::NotMeasured && r.def.rule != Rule::Recorded);
    let verdict = if failed {
        GateVerdict::Fail
    } else if open {
        GateVerdict::Incomplete
    } else {
        GateVerdict::Pass
    };
    Ok(GateReport { results, verdict })
}
