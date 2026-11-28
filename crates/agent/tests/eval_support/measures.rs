//! The measures of overview section 7.3 as data, and the gate over them.
//!
//! Every target below is transcribed from the table in 7.3; nothing is
//! added. Where the table says "Recorded" the rule is [`Rule::Recorded`]
//! and the value can never pass or fail. A measure with no value is
//! [`Outcome::NotMeasured`], never a pass. Pure module (std only).

#[allow(unused_imports)]
pub use super::measures_catalog::{
    measure, retired_measure, solve_wording, Cmp, MeasureDef, RetiredMeasure, Rule, MEASURES,
    RETIRED_MEASURES,
};
use std::collections::BTreeMap;

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
