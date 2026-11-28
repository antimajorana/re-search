//! Solve badge types, wording constants, and the single badge function (`badge_for_step`).
//! Re-exported from `solve_status` so callers import everything from `research_tools::solve_status`.

use crate::proof_closure::proper_ancestor_ids;
use crate::solve_model::{RestsOnSummary, WeakestDep};
use crate::solve_state::SolveState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const IDENTITY_NUMERIC_MIN_POINTS: u64 = 12;
pub const IDENTITY_NUMERIC_MIN_DIGITS: u64 = 50;

/// All on-screen words emitted by the Solve badge function, kept in one table so
/// names governed by owner decisions (`00-overview.md` section 8, `solve.md` section 13)
/// can change in one place.
///
/// Word rule (2026-11-25 revision, R22): `"verified"` may appear ONLY in
/// [`BADGE_VERIFIED`] (effective tier is `Verified`) and [`OWN_CHECK_VERIFIED`]
/// (own tier is `Verified` while effective tier is lower). No other constant here
/// contains the word.
pub mod wording {
    /// Owner decision of 2026-11-25 (`solve.md` D1): exact symbolic check passed.
    pub const BADGE_VERIFIED: &str = "Verified";
    /// Owner decision (`solve.md` 3, R22): shown when `own == Verified` and `effective != Verified`.
    pub const OWN_CHECK_VERIFIED: &str = "own check: verified";
    /// Owner clarification of 2026-11-26 (`00-overview.md` 2.10, superseding the earlier
    /// `"Checked numerically"` spelling in 8.3 question 11): numeric check passed.
    /// The serialized tier remains `checked_numerically`.
    pub const BADGE_CHECKED_NUMERICALLY: &str = "Numerically checked";
    /// Overview 3.2 row 24 (model verdicts start "Model agreed").
    pub const BADGE_MODEL_AGREED: &str = "Model agreed";
    /// `solve.md` decision D2 (same-model review weaker label).
    pub const BADGE_MODEL_AGREED_SAME_MODEL: &str = "Model agreed (same model)";
    /// `solve.md` decision D11 (user hand-checked step).
    pub const BADGE_CHECKED_BY_YOU: &str = "Checked by you";
    pub const BADGE_UNCHECKED: &str = "Unchecked";
    pub const BADGE_FAILED: &str = "Failed";
    pub const BADGE_GIVEN: &str = "Given";
    /// Owner clarification of 2026-11-26 (`00-overview.md` 2.10): source-backed `cited` step
    /// renders as `"Found"` (locates a claim without asserting correctness).
    /// The serialized tier remains `cited`.
    pub const BADGE_CITED: &str = "Found";
    /// `solve.md` decision D10 (unacknowledged solver assumption).
    pub const BADGE_ASSUMED_BY_MODEL: &str = "Assumed by model";
    pub const BADGE_STALE: &str = "Stale";

    pub const BROKEN_RESTS_ON_FAILED_PREFIX: &str = "Broken: rests on failed ";
    pub const RESTS_ON_PREFIX: &str = "rests on ";

    /// Sublabels (`solve.md` 4.4 rule 3 table).
    pub const SUBLABEL_SYMBOLIC: &str = "symbolic";
    pub const SUBLABEL_NUMERIC_DEFAULT: &str = "12 points, 50 digits";
    pub const SUBLABEL_SERIES_FALLBACK: &str = "series";
    pub const SUBLABEL_SERIES_NUMERIC_FALLBACK: &str = "series, numeric coefficients";
    pub const SUBLABEL_TWO_RUNS_AGREE: &str = "two runs agree, same solver";
    pub const SUBLABEL_INDEPENDENT_RECOMPUTE: &str = "independent recompute";
    pub const SUBLABEL_SAMPLED_NO_COUNTEREXAMPLE: &str = "sampled, no counterexample";
    pub const SUBLABEL_TRIVIAL_IDENTICAL: &str = "both sides identical — nothing checked";
    pub const SUBLABEL_RAN: &str = "ran";

    /// Flags shown on the badge and never hidden (`solve.md` 4.4).
    /// `solve.md` decision D13.
    pub const FLAG_TRANSCRIPTION_UNCONFIRMED: &str = "transcription unconfirmed";
    /// `solve.md` decision D2.
    pub const FLAG_SAME_MODEL: &str = "same model";
    /// `00-overview.md` 2.10 & `solve.md` 4.4 (Cited step matched in scanned-page OCR text).
    pub const FLAG_OCR_SOURCE: &str = "OCR source";

    /// Provenance tags (`00-overview.md` 2.10).
    pub const TAG_YOU: &str = "you";
    pub const TAG_OCR: &str = "OCR";
    pub const REASON_SUPERSEDED: &str = "superseded";

    /// Every constant in this module except [`BADGE_VERIFIED`] and [`OWN_CHECK_VERIFIED`].
    /// Tested directly so no other wording constant may contain `"verified"`, `"proved"`,
    /// `"new"`, or `"novel"`.
    pub const NON_VERIFIED_CONSTANTS: &[&str] = &[
        BADGE_CHECKED_NUMERICALLY,
        BADGE_MODEL_AGREED,
        BADGE_MODEL_AGREED_SAME_MODEL,
        BADGE_CHECKED_BY_YOU,
        BADGE_UNCHECKED,
        BADGE_FAILED,
        BADGE_GIVEN,
        BADGE_CITED,
        BADGE_ASSUMED_BY_MODEL,
        BADGE_STALE,
        BROKEN_RESTS_ON_FAILED_PREFIX,
        RESTS_ON_PREFIX,
        SUBLABEL_SYMBOLIC,
        SUBLABEL_NUMERIC_DEFAULT,
        SUBLABEL_SERIES_FALLBACK,
        SUBLABEL_SERIES_NUMERIC_FALLBACK,
        SUBLABEL_TWO_RUNS_AGREE,
        SUBLABEL_INDEPENDENT_RECOMPUTE,
        SUBLABEL_SAMPLED_NO_COUNTEREXAMPLE,
        SUBLABEL_TRIVIAL_IDENTICAL,
        SUBLABEL_RAN,
        FLAG_TRANSCRIPTION_UNCONFIRMED,
        FLAG_SAME_MODEL,
        FLAG_OCR_SOURCE,
        TAG_YOU,
        TAG_OCR,
        REASON_SUPERSEDED,
    ];

    /// Format `"rests on N assumptions made by the model"` (`solve.md` D10).
    pub fn format_model_assumptions_flag(count: usize) -> String {
        if count == 1 {
            "rests on 1 assumption made by the model".to_string()
        } else {
            format!("rests on {count} assumptions made by the model")
        }
    }

    pub fn format_numeric_equation_sublabel(points: u64, digits: u64) -> String {
        format!("{points} points, {digits} digits")
    }

    pub fn format_series_order(variable: &str, n: i32) -> String {
        format!("to O({variable}^{n})")
    }

    pub fn format_series_numeric_sublabel(variable: &str, n: i32) -> String {
        format!("to O({variable}^{n}), numeric coefficients")
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
    MatchedQuoteScannedPage,
    MatchedQuoteAbstract,
    ModelStatement,
    UserStatement,
}

/// Shared node/card border style (`00-overview.md` 2.6, `solve.md` 3; owner decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderStyle {
    Solid,
    Dashed,
    Plain,
    None,
}

/// Checking tier of a Solve step (`solve.md` 3 & 4.4, 2026-11-25 revision: Lean cut,
/// two computed tiers `CheckedNumerically` (3) and `Verified` (4), no `Computed` tier).
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
    CheckedNumerically,
    Verified,
    Given,
    Cited,
}

impl Tier {
    /// Rank in the meet-semilattice used for monotone effective-tier propagation (R7):
    /// `Failed (-1) < Unchecked/AssumedByModel/Stale (0) < ModelAgreedSameModel (1)
    ///  < ModelAgreed/CheckedByYou (2) < CheckedNumerically (3)
    ///  < Verified = Given = Cited (4; Given and Cited are neutral at the top of the lattice)`.
    pub fn lattice_rank(self) -> i8 {
        match self {
            Self::Failed => -1,
            Self::Stale | Self::AssumedByModel | Self::Unchecked => 0,
            Self::ModelAgreedSameModel => 1,
            Self::ModelAgreed | Self::CheckedByYou => 2,
            Self::CheckedNumerically => 3,
            Self::Verified | Self::Given | Self::Cited => 4,
        }
    }

    /// Rank in the explicit 0..=4 tier order (`solve.md` 4.4), `None` for Failed/Given/Cited.
    pub fn ordered_rank(self) -> Option<u8> {
        match self {
            Self::Stale | Self::AssumedByModel | Self::Unchecked => Some(0),
            Self::ModelAgreedSameModel => Some(1),
            Self::ModelAgreed | Self::CheckedByYou => Some(2),
            Self::CheckedNumerically => Some(3),
            Self::Verified => Some(4),
            Self::Failed | Self::Given | Self::Cited => None,
        }
    }

    pub fn is_neutral(self) -> bool {
        matches!(self, Self::Given | Self::Cited)
    }

    /// True for either computed tier (`CheckedNumerically` or `Verified`).
    pub fn is_computed(self) -> bool {
        matches!(self, Self::CheckedNumerically | Self::Verified)
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
            Self::CheckedNumerically => wording::BADGE_CHECKED_NUMERICALLY,
            Self::Verified => wording::BADGE_VERIFIED,
            Self::Given => wording::BADGE_GIVEN,
            Self::Cited => wording::BADGE_CITED,
        }
    }

    /// Shared display grade (`00-overview.md` 2.10, owner clarification 2026-11-26):
    /// `ByCode` is reserved strictly for machine-confirmed claims (`Verified` and
    /// `CheckedNumerically`). A citation, model opinion, unchecked run, stale check, or
    /// failed check must never be styled with `DisplayGrade::ByCode`.
    pub fn display_grade(self) -> DisplayGrade {
        match self {
            Self::Verified | Self::CheckedNumerically => DisplayGrade::ByCode,
            Self::Cited => DisplayGrade::MatchedQuoteFullText,
            Self::Given | Self::CheckedByYou => DisplayGrade::UserStatement,
            Self::Failed
            | Self::ModelAgreed
            | Self::ModelAgreedSameModel
            | Self::AssumedByModel
            | Self::Unchecked
            | Self::Stale => DisplayGrade::ModelStatement,
        }
    }

    pub fn border_style(self) -> BorderStyle {
        match self {
            Self::Verified | Self::CheckedNumerically | Self::Cited => BorderStyle::Solid,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchecked_deps: Vec<String>,
    pub sublabels: Vec<String>,
    pub flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_note: Option<String>,
    pub stale: bool,
}

/// Complete on-screen badge produced by the single badge function [`badge_for_step`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepBadge {
    pub entry_id: String,
    pub own: Tier,
    pub effective: Tier,
    pub label: String,
    /// Set to `"own check: verified"` strictly when `own == Tier::Verified` and
    /// `effective != Tier::Verified` (`solve.md` 3, R22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_check_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rests_on_line: Option<String>,
    pub grade: DisplayGrade,
    pub border: BorderStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_tag: Option<String>,
    pub lowered_by: Vec<String>,
    pub broken_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchecked_deps: Vec<String>,
    pub sublabels: Vec<String>,
    pub flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_note: Option<String>,
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

    let own_check_line = (tier.own == Tier::Verified && tier.effective != Tier::Verified)
        .then(|| wording::OWN_CHECK_VERIFIED.to_string());

    let rests_on_ids: &[String] = if !tier.lowered_by.is_empty() {
        &tier.lowered_by
    } else {
        &tier.unchecked_deps
    };
    let rests_on_line = if tier.effective == Tier::Failed && tier.own != Tier::Failed {
        None
    } else if !rests_on_ids.is_empty() {
        let parts: Vec<String> = rests_on_ids
            .iter()
            .map(|dep_id| {
                let dep_label = if state.entry(dep_id).is_some_and(|e| e.superseded_by.is_some()) {
                    wording::REASON_SUPERSEDED.to_string()
                } else {
                    state
                        .tiers
                        .get(dep_id)
                        .map_or(wording::BADGE_UNCHECKED, |t| t.effective.label())
                        .to_ascii_lowercase()
                };
                format!("{dep_id} ({dep_label})")
            })
            .collect();
        Some(format!("{}{}", wording::RESTS_ON_PREFIX, parts.join(", ")))
    } else {
        None
    };

    let is_ocr_cited = tier.effective == Tier::Cited
        && tier.flags.iter().any(|f| f == wording::FLAG_OCR_SOURCE);
    let grade = if is_ocr_cited {
        DisplayGrade::MatchedQuoteScannedPage
    } else {
        tier.effective.display_grade()
    };
    let provenance_tag = if is_ocr_cited {
        Some(wording::TAG_OCR.to_string())
    } else {
        tier.effective.provenance_tag().map(str::to_string)
    };

    Some(StepBadge {
        entry_id: entry_id.to_string(),
        own: tier.own,
        effective: tier.effective,
        label,
        own_check_line,
        rests_on_line,
        grade,
        border: tier.effective.border_style(),
        provenance_tag,
        lowered_by: tier.lowered_by.clone(),
        broken_by: tier.broken_by.clone(),
        unchecked_deps: tier.unchecked_deps.clone(),
        sublabels: tier.sublabels.clone(),
        flags: tier.flags.clone(),
        check_note: tier.check_note.clone(),
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
