//! Solve badge types, wording constants, and the single badge function (`badge_for_step`).
//! Re-exported from `solve_status` so callers import everything from `research_tools::solve_status`.

use crate::proof_closure::proper_ancestor_ids;
use crate::solve_model::{RestsOnSummary, WeakestDep};
use crate::solve_state::SolveState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const STANDARD_LEAN_AXIOMS: &[&str] = &["propext", "Classical.choice", "Quot.sound"];
pub const IDENTITY_NUMERIC_MIN_POINTS: u64 = 12;
pub const IDENTITY_NUMERIC_MIN_DIGITS: u64 = 50;
pub const FORMAL_CHECK_NAME: &str = "formal_check";

/// All on-screen words emitted by the Solve badge function, kept in one table so
/// provisional owner decisions (`00-overview.md` section 8, `solve.md` section 13)
/// can change in one place.
pub mod wording {
    /// Provisional: owner decision 3 ("verified" reserved for Lean; badge uses "Lean-checked").
    pub const BADGE_LEAN_CHECKED: &str = "Lean-checked";
    /// Provisional: solve.md decision D1 ("Computed"; "proved" never used on computed steps).
    pub const BADGE_COMPUTED: &str = "Computed";
    /// Provisional: overview 3.2 row 24 (model verdicts start "Model agreed").
    pub const BADGE_MODEL_AGREED: &str = "Model agreed";
    /// Provisional: solve.md decision D2 (same-model review weaker label).
    pub const BADGE_MODEL_AGREED_SAME_MODEL: &str = "Model agreed (same model)";
    /// Provisional: solve.md decision D11 (user hand-checked step).
    pub const BADGE_CHECKED_BY_YOU: &str = "Checked by you";
    pub const BADGE_UNCHECKED: &str = "Unchecked";
    pub const BADGE_FAILED: &str = "Failed";
    pub const BADGE_GIVEN: &str = "Given";
    /// Provisional: owner decision 3 (quotes are "matched", never "verified").
    pub const BADGE_CITED: &str = "Cited";
    /// Provisional: solve.md decision D10 (unacknowledged solver assumption).
    pub const BADGE_ASSUMED_BY_MODEL: &str = "Assumed by model";
    pub const BADGE_STALE: &str = "Stale";

    pub const BROKEN_RESTS_ON_FAILED_PREFIX: &str = "Broken: rests on failed ";
    pub const RESTS_ON_PREFIX: &str = "rests on ";

    /// Sublabels (`solve.md` 4.4).
    pub const SUBLABEL_SYMBOLIC: &str = "symbolic";
    pub const SUBLABEL_NUMERIC_ONLY: &str = "numeric only";
    pub const SUBLABEL_SERIES_FALLBACK: &str = "series";
    pub const SUBLABEL_TWO_RUNS_AGREE: &str = "two runs agree, same solver";
    pub const SUBLABEL_INDEPENDENT_RECOMPUTE: &str = "independent recompute";
    pub const SUBLABEL_SAMPLED_NO_COUNTEREXAMPLE: &str = "sampled, no counterexample";
    pub const SUBLABEL_RAN: &str = "ran";

    /// Flags shown on the badge and never hidden (`solve.md` 4.4).
    /// Provisional: solve.md decision D13.
    pub const FLAG_TRANSCRIPTION_UNCONFIRMED: &str = "transcription unconfirmed";
    /// Provisional: solve.md decision D14.
    pub const FLAG_STATEMENT_MATCH_UNCONFIRMED: &str = "statement match unconfirmed";
    /// Provisional: solve.md decision D2.
    pub const FLAG_SAME_MODEL: &str = "same model";
    pub const FLAG_NUMERIC_ONLY: &str = "numeric only";

    /// Provenance tag for user statements (`00-overview.md` 2.10).
    pub const TAG_YOU: &str = "you";

    /// Format `"rests on N assumptions made by the model"` (provisional: solve.md D10).
    pub fn format_model_assumptions_flag(count: usize) -> String {
        if count == 1 {
            "rests on 1 assumption made by the model".to_string()
        } else {
            format!("rests on {count} assumptions made by the model")
        }
    }

    pub fn format_series_sublabel(variable: &str, n: i32) -> String {
        format!("series to O({variable}^{n})")
    }

    pub fn format_broken_label(failed_ids: &[String]) -> String {
        format!("{BROKEN_RESTS_ON_FAILED_PREFIX}{}", failed_ids.join(", "))
    }
}

/// Shared display grade (`00-overview.md` 2.10, `solve.md` 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayGrade {
    ByCode,
    MatchedQuoteFullText,
    MatchedQuoteAbstract,
    ModelStatement,
    UserStatement,
}

/// Shared node/card border style (`00-overview.md` 2.6, `solve.md` 3).
/// Provisional: owner decision 4 (dashed always means not checked by code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderStyle {
    Solid,
    Dashed,
    Plain,
    None,
}

/// Checking tier of a Solve step (`solve.md` 3 & 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Failed,
    Stale,
    AssumedByModel,
    Unchecked,
    ModelAgreedSameModel,
    ModelAgreed,
    CheckedByYou,
    Computed,
    LeanChecked,
    Given,
    Cited,
}

impl Tier {
    /// Rank in the meet-semilattice used for monotone effective-tier propagation (R7):
    /// `Failed (-1) < Unchecked/AssumedByModel/Stale (0) < ModelAgreedSameModel (1)
    ///  < ModelAgreed/CheckedByYou (2) < Computed (3) < LeanChecked (4) < Given/Cited (5, neutral)`.
    pub fn lattice_rank(self) -> i8 {
        match self {
            Self::Failed => -1,
            Self::Stale | Self::AssumedByModel | Self::Unchecked => 0,
            Self::ModelAgreedSameModel => 1,
            Self::ModelAgreed | Self::CheckedByYou => 2,
            Self::Computed => 3,
            Self::LeanChecked => 4,
            Self::Given | Self::Cited => 5,
        }
    }

    /// Rank in the explicit 0..=4 tier order (`solve.md` 4.4), `None` for Failed/Given/Cited.
    pub fn ordered_rank(self) -> Option<u8> {
        match self {
            Self::Stale | Self::AssumedByModel | Self::Unchecked => Some(0),
            Self::ModelAgreedSameModel => Some(1),
            Self::ModelAgreed | Self::CheckedByYou => Some(2),
            Self::Computed => Some(3),
            Self::LeanChecked => Some(4),
            Self::Failed | Self::Given | Self::Cited => None,
        }
    }

    pub fn is_neutral(self) -> bool {
        matches!(self, Self::Given | Self::Cited)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Failed => wording::BADGE_FAILED,
            Self::Stale => wording::BADGE_STALE,
            Self::AssumedByModel => wording::BADGE_ASSUMED_BY_MODEL,
            Self::Unchecked => wording::BADGE_UNCHECKED,
            Self::ModelAgreedSameModel => wording::BADGE_MODEL_AGREED_SAME_MODEL,
            Self::ModelAgreed => wording::BADGE_MODEL_AGREED,
            Self::CheckedByYou => wording::BADGE_CHECKED_BY_YOU,
            Self::Computed => wording::BADGE_COMPUTED,
            Self::LeanChecked => wording::BADGE_LEAN_CHECKED,
            Self::Given => wording::BADGE_GIVEN,
            Self::Cited => wording::BADGE_CITED,
        }
    }

    pub fn display_grade(self) -> DisplayGrade {
        match self {
            Self::LeanChecked | Self::Computed | Self::Failed => DisplayGrade::ByCode,
            Self::Cited => DisplayGrade::MatchedQuoteFullText,
            Self::Given | Self::CheckedByYou => DisplayGrade::UserStatement,
            Self::ModelAgreed
            | Self::ModelAgreedSameModel
            | Self::AssumedByModel
            | Self::Unchecked
            | Self::Stale => DisplayGrade::ModelStatement,
        }
    }

    pub fn border_style(self) -> BorderStyle {
        match self {
            Self::LeanChecked | Self::Computed | Self::Cited => BorderStyle::Solid,
            Self::ModelAgreed
            | Self::ModelAgreedSameModel
            | Self::AssumedByModel
            | Self::Unchecked => BorderStyle::Dashed,
            Self::Given | Self::CheckedByYou => BorderStyle::Plain,
            Self::Failed | Self::Stale => BorderStyle::None,
        }
    }

    pub fn provenance_tag(self) -> Option<&'static str> {
        match self {
            Self::Given | Self::CheckedByYou => Some(wording::TAG_YOU),
            _ => None,
        }
    }
}

/// Derived tier summary stored in [`SolveState::tiers`] (`solve.md` 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepTier {
    pub own: Tier,
    pub effective: Tier,
    pub lowered_by: Vec<String>,
    pub broken_by: Vec<String>,
    pub sublabels: Vec<String>,
    pub flags: Vec<String>,
    pub stale: bool,
}

/// Complete on-screen badge produced by the single badge function [`badge_for_step`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepBadge {
    pub entry_id: String,
    pub own: Tier,
    pub effective: Tier,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rests_on_line: Option<String>,
    pub grade: DisplayGrade,
    pub border: BorderStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_tag: Option<String>,
    pub lowered_by: Vec<String>,
    pub broken_by: Vec<String>,
    pub sublabels: Vec<String>,
    pub flags: Vec<String>,
    pub stale: bool,
}

/// The single badge function from [`SolveState`] to what is shown for `entry_id`.
pub fn badge_for_step(state: &SolveState, entry_id: &str) -> Option<StepBadge> {
    let tier = state.tiers.get(entry_id)?;
    let label =
        if tier.effective == Tier::Failed && tier.own != Tier::Failed && !tier.broken_by.is_empty()
        {
            wording::format_broken_label(&tier.broken_by)
        } else {
            tier.effective.label().to_string()
        };

    let rests_on_line = if tier.effective == Tier::Failed && tier.own != Tier::Failed {
        None
    } else if !tier.lowered_by.is_empty() {
        let parts: Vec<String> = tier
            .lowered_by
            .iter()
            .map(|dep_id| {
                let dep_label = state
                    .tiers
                    .get(dep_id)
                    .map_or(wording::BADGE_UNCHECKED, |t| t.effective.label())
                    .to_ascii_lowercase();
                format!("{dep_id} ({dep_label})")
            })
            .collect();
        Some(format!("{}{}", wording::RESTS_ON_PREFIX, parts.join(", ")))
    } else {
        None
    };

    Some(StepBadge {
        entry_id: entry_id.to_string(),
        own: tier.own,
        effective: tier.effective,
        label,
        rests_on_line,
        grade: tier.effective.display_grade(),
        border: tier.effective.border_style(),
        provenance_tag: tier.effective.provenance_tag().map(str::to_string),
        lowered_by: tier.lowered_by.clone(),
        broken_by: tier.broken_by.clone(),
        sublabels: tier.sublabels.clone(),
        flags: tier.flags.clone(),
        stale: tier.stale,
    })
}

/// Compute on-screen badges for every step in `state`.
pub fn compute_all_badges(state: &SolveState) -> BTreeMap<String, StepBadge> {
    state
        .entries
        .iter()
        .filter_map(|e| badge_for_step(state, &e.entry_id).map(|b| (e.entry_id.clone(), b)))
        .collect()
}

/// Build the `rests_on` summary for `entry_id` across its proper ancestor closure
/// (`solve.md` 4.1 `solve.done`, Why popover).
pub fn rests_on_summary(state: &SolveState, entry_id: &str) -> RestsOnSummary {
    let mut summary = RestsOnSummary::default();
    let Some(target_tier) = state.tiers.get(entry_id) else {
        return summary;
    };
    let target_rank = target_tier.effective.lattice_rank();
    let ancestors = proper_ancestor_ids(&state.entries, entry_id);
    let mut seen_weakest = BTreeSet::new();
    for anc in ancestors {
        let Some(t) = state.tiers.get(&anc) else {
            continue;
        };
        match t.own {
            Tier::Given => summary.given.push(anc.clone()),
            Tier::Cited => summary.cited.push(anc.clone()),
            Tier::AssumedByModel => summary.assumed_by_model.push(anc.clone()),
            _ => {}
        }
        if !t.own.is_neutral()
            && t.own.lattice_rank() <= target_rank
            && seen_weakest.insert(anc.clone())
        {
            summary.weakest.push(WeakestDep {
                entry_id: anc,
                tier: t.own.label().to_string(),
            });
        }
    }
    summary
}
