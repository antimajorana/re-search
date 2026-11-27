//! Catalog of section 7.3 measures and retired pre-revision measure ids.
//!
//! Split out of `measures.rs` to stay well under the 600-line limit while
//! keeping all definitions pure (`std` + `super::manifest::Set` only).

use super::manifest::Set;

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
/// Sets producing Investigate-task or Explore-reader quotes (`P4` replaced by `P6` and `C`).
const INVESTIGATE_AND_EXPLORE: &[Set] = &[
    Set::P,
    Set::P2,
    Set::P3,
    Set::P5,
    Set::P6,
    Set::I,
    Set::C,
];

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

/// Pre-revision measure ids rejected fail-closed by [`super::measures::evaluate`].
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

/// The table of 7.3 (post-2026-11-25 revision), row by row, in its order.
/// A row that states several numbers becomes several entries.
pub const MEASURES: &[MeasureDef] = &[
    // Time from input to first deterministic content.
    m("time_to_skeleton_tex_s", "Investigate: time from input to skeleton, TeX paper", "seconds", &[Set::P], "All modes", Rule::Threshold(Cmp::Below, 10.0), "Investigate TeX under 10 s"),
    m("time_to_candidates_s", "Explore: time from input to candidates", "seconds", &[Set::I], "All modes", Rule::Threshold(Cmp::Below, 6.0), "Explore candidates under 6 s"),
    m("time_to_solve_setup_s", "Solve: time from input to setup", "seconds", &[Set::Q], "All modes", Rule::Threshold(Cmp::Below, 1.0), "Solve setup under 1 s"),
    m("time_to_skeleton_pdf_s", "Investigate: time from input to skeleton, PDF with text layer", "seconds", &[Set::P6], "All modes", Rule::Recorded, "Investigate from a PDF with a text layer: recorded; a target is set after the first F14a sheet"),
    // Model calls and tokens per run, against the cap.
    m("runs_above_cap", "Runs whose model calls or tokens went above the cap", "count", ALL, "All", Rule::MustBeZero, "Never above the cap"),
    m("model_calls_typical", "Typical model calls per run", "count", ALL, "All", Rule::Recorded, "typical value recorded"),
    m("tokens_typical", "Typical tokens per run", "count", ALL, "All", Rule::Recorded, "typical value recorded"),
    // Theorem-like statement recall; section recall.
    m("statement_recall", "Theorem-like statement recall, all papers of P and P6", "share", &[Set::P, Set::P6], "P, P6", Rule::Recorded, "For PDF the figure is recorded and shown beside the quality line; no blocking value before the first sheet"),
    m("section_recall", "Section recall, all papers of P and P6", "share", &[Set::P, Set::P6], "P, P6", Rule::Recorded, "For PDF the figure is recorded and shown beside the quality line; no blocking value before the first sheet"),
    m("statement_recall_tex_theorem_envs", "Theorem-like statement recall, TeX papers with theorem environments", "share", &[Set::P], "P", Rule::Threshold(Cmp::AtLeast, 0.9), "below 0.9 on TeX papers with theorem environments blocks M1"),
    m("section_recall_tex_theorem_envs", "Section recall, TeX papers with theorem environments", "share", &[Set::P], "P", Rule::Threshold(Cmp::AtLeast, 0.9), "below 0.9 on TeX papers with theorem environments blocks M1"),
    // "Uses" edge precision, statement and proof roles separately.
    m("uses_edge_precision_statement", "\"Uses\" edge precision, statement role", "share", &[Set::P], "P", Rule::Recorded, ""),
    m("uses_edge_precision_proof", "\"Uses\" edge precision, proof role", "share", &[Set::P], "P", Rule::Recorded, ""),
    // Tags unchanged for unchanged statements; wrong carry-overs (Set P2, deferred in 0.1).
    m("tags_unchanged_share", "Tags unchanged for unchanged statements", "share", &[Set::P2], "P2", Rule::Recorded, ""),
    m("wrong_carry_overs", "Wrong carry-overs", "count", &[Set::P2], "P2", Rule::MustBeZero, "Wrong carry-overs must be 0"),
    // Authors' label agreement with the key; share of labels whose quote contains a cue.
    m("author_label_agreement", "Authors' label agreement with the key", "share", &[Set::P3], "P3", Rule::Recorded, ""),
    m("label_quote_cue_share", "Share of labels whose quote contains a cue", "share", &[Set::P3], "P3", Rule::Recorded, ""),
    // Known earlier source found; "none in scope" shown for a result that has one (P4 moved to C P-to-I).
    m("earlier_source_found", "Known earlier source found", "share", &[Set::I, Set::C], "I, and the P-to-I chains of C", Rule::Recorded, ""),
    m("none_in_scope_false_negative_rate", "\"None in scope\" shown for a result that has a known earlier source", "share", &[Set::I, Set::C], "I, and the P-to-I chains of C", Rule::Recorded, "The second number is the false-negative rate that decides whether the feature is announced"),
    // "Judged same" where the key says different (P4 moved to C).
    m("judged_same_key_different", "\"Judged same\" where the key says different", "count", &[Set::I, Set::C], "I, C", Rule::Recorded, "Recorded; every such case reviewed"),
    m("judged_same_key_different_unreviewed", "Of those, cases not yet reviewed", "count", &[Set::I, Set::C], "I, C", Rule::MustBeZero, "every such case reviewed"),
    // Wrong statement shown as located.
    m("wrong_statement_located", "Wrong statement shown as located", "count", &[Set::P5], "P5", Rule::MustBeZero, "Must be 0 (candidates list is acceptable)"),
    // Known-paper recall of the search alone, before any model call.
    m("search_recall_keyless", "Known-paper recall of the search alone, keyless services", "share", &[Set::I], "I", Rule::Recorded, "Recorded per service set (keyless; with keys; with and without the library)"),
    m("search_recall_with_keys", "Known-paper recall of the search alone, services with keys", "share", &[Set::I], "I", Rule::Recorded, "Recorded per service set (keyless; with keys; with and without the library)"),
    m("search_recall_with_library", "Known-paper recall of the search alone, with library", "share", &[Set::I], "I", Rule::Recorded, "Recorded per service set (keyless; with keys; with and without the library)"),
    m("search_recall_without_library", "Known-paper recall of the search alone, without library", "share", &[Set::I], "I", Rule::Recorded, "Recorded per service set (keyless; with keys; with and without the library)"),
    // Quotes accepted per model call; quotes rejected and why.
    m("quotes_accepted_per_model_call", "Quotes accepted per model call", "count", INVESTIGATE_AND_EXPLORE, "Investigate tasks, Explore readers", Rule::Recorded, ""),
    m("quotes_rejected", "Quotes rejected (reasons in the note)", "count", INVESTIGATE_AND_EXPLORE, "Investigate tasks, Explore readers", Rule::Recorded, ""),
    // PDF and old papers (Set P6).
    m("pdf_sentence_match_share", "Sentence copied by hand from the page matches in stored text", "share", &[Set::P6], "P6", Rule::Recorded, "Recorded per page basis (text layer, scan with text layer, own OCR)"),
    m("pdf_wrong_page_chip", "Quote chip opens the wrong page", "count", &[Set::P6], "P6", Rule::MustBeZero, "Must be 0"),
    m("pdf_scanned_reported_plain_text", "Scanned page reported as a plain text layer", "count", &[Set::P6], "P6 and the PDF fixtures", Rule::MustBeZero, "Must be 0"),
    m("pdf_scan_char_errors_per_100", "Character errors per 100 characters on sampled lines of scans", "count", &[Set::P6], "P6", Rule::Recorded, ""),
    m("pdf_two_col_wrong_order_lines", "Lines in the wrong reading order on two-column pages", "count", &[Set::P6], "P6", Rule::Recorded, ""),
    // Own library sync (Set B).
    m("sync_second_changes", "Sync: rows changed by a second sync", "count", &[Set::B], "B", Rule::MustBeZero, "Must pass; in CI"),
    m("sync_planted_case_mismatches", "Sync: planted cases not ending in their expected state", "count", &[Set::B], "B", Rule::MustBeZero, "Must pass; in CI"),
    m("sync_source_folder_diffs", "Sync: source folder bytes differing after sync", "count", &[Set::B], "B", Rule::MustBeZero, "Must pass; in CI"),
    m("sync_removed_source_quote_breaks", "Sync: quote checks broken when a source is removed", "count", &[Set::B], "B", Rule::MustBeZero, "Must pass; in CI"),
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
    // Write-ups (Set W).
    m("writeup_failures", "Write-up: cases failing TeX compile, formula handle, citation, check table, or status checks", "count", &[Set::W], "W", Rule::MustBeZero, "Must pass"),
    // Corrections and lessons (Set K).
    m("lessons_gate_drift", "Corrections: runs whose tier, verdict, or gate differs with vs without lessons for the same events", "count", &[Set::K], "K", Rule::MustBeZero, "Must pass; in CI"),
    m("lessons_error_recurrence", "Corrections: corrected error recurrence across runs with vs without lesson", "count", &[Set::K], "K", Rule::Recorded, ""),
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
