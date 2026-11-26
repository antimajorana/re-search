//! Shared literature search coverage record and completeness gate (package F12).
//!
//! Pure, deterministic computation over typed search records or ledger events (`00-overview.md`
//! 2.9, `explore.md` 4.1 & 6 rules V2, V3, V4, V7, V10). Makes no network calls, no model calls,
//! and reads no F16 lessons or ranking marks.
//!
//! Downstream interfaces:
//! - X7–X11 call [`compute_coverage`] or [`compute_coverage_from_events`] when building
//!   `idea.verdict` and rendering "Where I looked".
//! - [`crate::prior_work`] reads [`SearchCoverage::gate`] to decide `none_in_scope` vs
//!   `incomplete`.

#[path = "search_coverage_eval.rs"]
mod eval;
#[path = "search_coverage_events.rs"]
mod events;
#[path = "search_coverage_types.rs"]
mod types;

pub use types::*;

use crate::lit_providers::LitProvider;
use crate::paper_key::{PaperKey, PaperRef};
use ledger::Event;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Minimum number of answered external literature services (`00-overview.md` 2.9, `explore.md` V3).
pub const MIN_ANSWERED_EXTERNAL_SERVICES: usize = 2;

/// Explore ledger event kinds read by [`compute_coverage_from_events`] (`explore.md` 4.2;
/// `00-overview.md` section 8.2 decision 10 keeps `idea.*` for Explore-only events).
pub const IDEA_STARTED_EVENT: &str = "idea.started";
pub const IDEA_STATED_EVENT: &str = "idea.stated";
pub const IDEA_BRIEF_EVENT: &str = "idea.brief";
pub const IDEA_SEARCH_PLANNED_EVENT: &str = "idea.search_planned";
pub const IDEA_SEARCH_FINISHED_EVENT: &str = "idea.search_finished";
pub const IDEA_CANDIDATES_EVENT: &str = "idea.candidates";
pub const IDEA_CITATION_NET_EVENT: &str = "idea.citation_net";
pub const IDEA_TRIAGE_EVENT: &str = "idea.triage";
pub const IDEA_PAPER_EVENT: &str = "idea.paper";
pub const IDEA_NOTE_EVENT: &str = "idea.note";
/// Explicit per-session reader dismissal of a triage-core paper (`explore.md` V3). Kept distinct
/// from F16 ranking marks (`not_relevant`, `pin`, `key_paper`) so rule V10 holds unconditionally.
pub const IDEA_CORE_DISMISSED_EVENT: &str = "idea.core_dismissed";
pub const NOTE_ACTION_DISMISS_CORE: &str = "dismiss_core";
pub const NOTE_ACTION_UNDISMISS_CORE: &str = "undismiss_core";

/// Fixed scope disclosures always attached to [`SearchCoverage::not_searched`] (`explore.md` 4.1).
pub const NOT_SEARCHED_DEFAULT: &[&str] = &[
    "books and monographs",
    "theses not on arXiv",
    "talks and unpublished notes",
    "non-English literature",
    "most work before 1991 without a DOI unless it is in your library",
    "paywalled full text that is not in your library",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryCoverageRecord {
    pub qid: String,
    pub provider: LitProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective_kind: Option<PerspectiveKind>,
    pub status: QueryOutcomeStatus,
    #[serde(default)]
    pub hits: usize,
    #[serde(default)]
    pub cached: bool,
    /// True for direct `origin: app` seed lookups; never credited toward search recall.
    #[serde(default)]
    pub is_seed_lookup: bool,
    #[serde(default)]
    pub found_papers: Vec<PaperRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateCoverageRecord {
    pub paper: PaperRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default)]
    pub found_by: Vec<String>,
    #[serde(default)]
    pub user_seed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageCoverageRecord {
    pub key: PaperKey,
    pub relevance: TriageRelevance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperReadCoverageRecord {
    pub key: PaperKey,
    pub status: PaperReadStatus,
}

/// Typed input for pure coverage computation via [`compute_coverage`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageInput {
    pub preset: ExplorePreset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screened_limit_override: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screened_count_override: Option<usize>,
    #[serde(default)]
    pub provider_states: Vec<(LitProvider, ProviderCoverageState)>,
    #[serde(default)]
    pub perspectives: Vec<(String, PerspectiveKind)>,
    #[serde(default)]
    pub queries: Vec<QueryCoverageRecord>,
    #[serde(default)]
    pub library: LibraryCoverage,
    #[serde(default)]
    pub candidates: Vec<CandidateCoverageRecord>,
    #[serde(default)]
    pub triage: Vec<TriageCoverageRecord>,
    #[serde(default)]
    pub paper_reads: Vec<PaperReadCoverageRecord>,
    #[serde(default)]
    pub dismissed_core: BTreeSet<PaperKey>,
    #[serde(default)]
    pub user_seeds: Vec<PaperKey>,
    #[serde(default)]
    pub citation_expansion: CitationExpansionCoverage,
}

/// Pure function computing [`SearchCoverage`] and evaluating every gate condition.
pub fn compute_coverage(input: &CoverageInput) -> SearchCoverage {
    let providers = eval::aggregate_providers(input);
    let perspectives = eval::aggregate_perspectives(input);
    let (candidates_total, year_range) = eval::summarize_candidates(&input.candidates);
    let rt = eval::summarize_reads_and_triage(input);

    let screened = input.screened_count_override.unwrap_or(rt.triaged_count);
    let screened_limit = input
        .screened_limit_override
        .filter(|&n| n >= 1)
        .unwrap_or_else(|| input.preset.limits().max_screened);
    let screened_required = screened_limit.min(candidates_total);
    let known_paper_recall = eval::compute_seed_recall(input);

    let gate = eval::evaluate_gate(
        input.preset,
        &providers,
        &perspectives,
        candidates_total,
        screened,
        screened_required,
        &rt.unresolved_core,
        &known_paper_recall,
    );
    let zero_candidates_from_answered_search =
        candidates_total == 0 && gate.answered_external_services > 0;

    SearchCoverage {
        preset: input.preset,
        providers,
        library: input.library.clone(),
        perspectives,
        candidates_total,
        screened,
        screened_required,
        zero_candidates_from_answered_search,
        papers: rt.papers,
        unavailable_core: rt.unavailable_core,
        unresolved_core: rt.unresolved_core,
        dismissed_core: rt.dismissed_core,
        year_range,
        citation_expansion: input.citation_expansion.clone(),
        known_paper_recall,
        not_searched: NOT_SEARCHED_DEFAULT.iter().map(|s| (*s).to_string()).collect(),
        gate,
    }
}

/// Recompute [`SearchCoverage`] purely from ledger events (`explore.md` V7 & V10).
/// Reads events through `serde_json::Value` and ignores F16 lessons and ranking marks.
pub fn compute_coverage_from_events(events: &[Event]) -> SearchCoverage {
    let input = events::extract_coverage_input(events);
    compute_coverage(&input)
}

/// Rule V7 export/parity check: confirms a stored [`SearchCoverage`] equals the pure
/// recomputation from `events`.
pub fn coverage_matches_recomputed(stored: &SearchCoverage, events: &[Event]) -> bool {
    *stored == compute_coverage_from_events(events)
}

/// Match a [`PaperRef`] against a canonical [`PaperKey`] by primary key or F2 alias ids.
/// Reuses F11's [`crate::lit_abstracts::paper_matches_key`].
pub fn paper_ref_matches_key(paper: &PaperRef, target: &PaperKey) -> bool {
    crate::lit_abstracts::paper_matches_key(paper, target)
}

#[cfg(test)]
#[path = "search_coverage_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "search_coverage_audit_tests.rs"]
mod audit_tests;
