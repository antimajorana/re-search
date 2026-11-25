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
    // Planted error ends Failed.
    m("planted_error_ends_failed_pct", "Planted error ends Failed", "percent", &[Set::Q], "Q", Rule::MustEqual(100.0), "Must be 100 percent"),
    m("planted_error_ends_computed", "Planted error ends Computed", "count", &[Set::Q], "Q", Rule::MustBeZero, "ending Computed is a release blocker"),
    // Share of steps Computed, Model agreed, Unchecked.
    m("steps_computed_share", "Share of steps Computed", "share", &[Set::Q], "Q", Rule::Recorded, "decides whether SymPy's reach is enough"),
    m("steps_model_agreed_share", "Share of steps Model agreed", "share", &[Set::Q], "Q", Rule::Recorded, "decides whether SymPy's reach is enough"),
    m("steps_unchecked_share", "Share of steps Unchecked", "share", &[Set::Q], "Q", Rule::Recorded, "decides whether SymPy's reach is enough"),
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
pub fn percent(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| 100.0 * part as f64 / whole as f64)
}

/// `part / whole` in 0 to 1, or `None` when nothing was counted.
pub fn share(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
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

/// Judges every measure of 7.3 against the measured values. A value for an
/// id that is not in the table, or a value that is not a finite number, is
/// an error: a sheet must not carry numbers the gate does not know.
pub fn evaluate(values: &BTreeMap<String, f64>) -> Result<GateReport, String> {
    for (id, v) in values {
        if measure(id).is_none() {
            return Err(format!("`{id}` is not a measure of section 7.3"));
        }
        if !v.is_finite() || *v < 0.0 {
            return Err(format!("`{id}` has the value {v}; a measure is a finite number, 0 or above"));
        }
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
