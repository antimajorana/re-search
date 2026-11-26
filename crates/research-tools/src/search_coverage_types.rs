//! Public coverage, preset, provider, perspective, and gate types (re-exported by
//! `crate::search_coverage`).

use crate::lit_providers::{LitProvider, QueryStatus};
use crate::paper_key::PaperKey;
use serde::{Deserialize, Serialize};

/// Explore preset (`explore.md` section 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorePreset {
    Quick,
    #[default]
    Standard,
    Thorough,
}

/// Hard ceilings and typical call estimates for an [`ExplorePreset`] (`explore.md` section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetLimits {
    pub max_queries: usize,
    pub max_screened: usize,
    pub max_papers: usize,
    pub calls_per_paper: usize,
    pub typical_model_calls: usize,
    pub max_model_calls: usize,
}

impl ExplorePreset {
    pub const ALL: [ExplorePreset; 3] = [Self::Quick, Self::Standard, Self::Thorough];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Thorough => "thorough",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "quick" => Some(Self::Quick),
            "standard" => Some(Self::Standard),
            "thorough" => Some(Self::Thorough),
            _ => None,
        }
    }

    pub const fn limits(self) -> PresetLimits {
        match self {
            Self::Quick => PresetLimits {
                max_queries: 8,
                max_screened: 25,
                max_papers: 0,
                calls_per_paper: 0,
                typical_model_calls: 3,
                max_model_calls: 5,
            },
            Self::Standard => PresetLimits {
                max_queries: 18,
                max_screened: 40,
                max_papers: 6,
                calls_per_paper: 3,
                typical_model_calls: 15,
                max_model_calls: 24,
            },
            Self::Thorough => PresetLimits {
                max_queries: 36,
                max_screened: 80,
                max_papers: 12,
                calls_per_paper: 4,
                typical_model_calls: 35,
                max_model_calls: 55,
            },
        }
    }

    /// Quick mode reads abstracts only and never qualifies for `none_in_scope` (`explore.md` V2).
    pub const fn allows_none_in_scope(self) -> bool {
        !matches!(self, Self::Quick)
    }
}

/// Perspective category (`explore.md` 4.1 `Perspective.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerspectiveKind {
    Exact,
    OtherNames,
    Generalisation,
    SpecialCase,
    Method,
    Obstruction,
    AdjacentField,
}

impl PerspectiveKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::OtherNames => "other_names",
            Self::Generalisation => "generalisation",
            Self::SpecialCase => "special_case",
            Self::Method => "method",
            Self::Obstruction => "obstruction",
            Self::AdjacentField => "adjacent_field",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "exact" => Some(Self::Exact),
            "other_names" | "alternative_names" => Some(Self::OtherNames),
            "generalisation" | "generalization" => Some(Self::Generalisation),
            "special_case" => Some(Self::SpecialCase),
            "method" => Some(Self::Method),
            "obstruction" => Some(Self::Obstruction),
            "adjacent_field" => Some(Self::AdjacentField),
            _ => None,
        }
    }
}

/// Per-query execution status combining F11 [`QueryStatus`] with Explore's `skipped_budget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOutcomeStatus {
    Ok,
    Failed,
    RateLimited,
    NotConfigured,
    Disabled,
    Unsupported,
    SkippedBudget,
}

impl From<QueryStatus> for QueryOutcomeStatus {
    fn from(status: QueryStatus) -> Self {
        match status {
            QueryStatus::Ok => Self::Ok,
            QueryStatus::Failed => Self::Failed,
            QueryStatus::RateLimited => Self::RateLimited,
            QueryStatus::NotConfigured => Self::NotConfigured,
            QueryStatus::Disabled => Self::Disabled,
            QueryStatus::Unsupported => Self::Unsupported,
        }
    }
}

impl QueryOutcomeStatus {
    pub const fn is_answered(self) -> bool {
        matches!(self, Self::Ok)
    }

    pub const fn is_attempt_failure(self) -> bool {
        matches!(self, Self::Failed | Self::RateLimited)
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ok" => Some(Self::Ok),
            "failed" => Some(Self::Failed),
            "rate_limited" => Some(Self::RateLimited),
            "not_configured" => Some(Self::NotConfigured),
            "disabled" => Some(Self::Disabled),
            "unsupported" => Some(Self::Unsupported),
            "skipped_budget" => Some(Self::SkippedBudget),
            _ => None,
        }
    }
}

/// Aggregated status of an external literature service (`explore.md` 4.1 `Coverage.providers`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCoverageState {
    Ok,
    Partial,
    Failed,
    RateLimited,
    NotConfigured,
    Disabled,
    NotAttempted,
}

impl ProviderCoverageState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::RateLimited => "rate_limited",
            Self::NotConfigured => "not_configured",
            Self::Disabled => "disabled",
            Self::NotAttempted => "not_attempted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ok" => Some(Self::Ok),
            "partial" => Some(Self::Partial),
            "failed" => Some(Self::Failed),
            "rate_limited" => Some(Self::RateLimited),
            "not_configured" => Some(Self::NotConfigured),
            "disabled" => Some(Self::Disabled),
            "not_attempted" => Some(Self::NotAttempted),
            _ => None,
        }
    }

    pub const fn is_answered(self) -> bool {
        matches!(self, Self::Ok | Self::Partial)
    }

    pub const fn has_attempt_failure(self) -> bool {
        matches!(self, Self::Failed | Self::RateLimited | Self::Partial)
    }
}

/// Per-provider coverage counters, explicitly distinguishing answered zero-result queries from
/// no-answer states (`failed`, `rate_limited`, `not_configured`, `disabled`, `not_attempted`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCoverage {
    pub provider: LitProvider,
    pub state: ProviderCoverageState,
    pub queries_ok: usize,
    pub queries_zero_results: usize,
    pub queries_failed: usize,
    pub queries_rate_limited: usize,
    pub queries_skipped_budget: usize,
    pub queries_unsupported: usize,
    #[serde(default)]
    pub seed_lookup_queries_ok: usize,
    pub cached: usize,
    pub hits: usize,
}

impl ProviderCoverage {
    /// True when at least one search query (excluding direct seed lookups) answered cleanly.
    pub const fn answered(&self) -> bool {
        self.queries_ok > 0
    }

    /// True when the provider answered cleanly with zero results (distinct from no answer or partial failure).
    pub const fn answered_with_zero_results(&self) -> bool {
        matches!(self.state, ProviderCoverageState::Ok) && self.queries_ok > 0 && self.hits == 0
    }

    pub const fn has_no_answer(&self) -> bool {
        self.queries_ok == 0
    }
}

/// Local user library summary (`00-overview.md` 2.9, `explore.md` 4.1).
/// Always listed for the reader; never counted toward the external service minimum.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCoverage {
    pub searched: bool,
    pub papers: usize,
    pub with_full_text: usize,
    pub hits: usize,
}

/// Per-perspective query coverage (`explore.md` 4.1 `Coverage.perspectives`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveCoverage {
    pub id: String,
    pub kind: PerspectiveKind,
    pub queries_planned: usize,
    pub queries_ok: usize,
    pub queries_zero_results: usize,
    pub queries_failed: usize,
    pub hits: usize,
}

impl PerspectiveCoverage {
    pub const fn answered(&self) -> bool {
        self.queries_ok > 0
    }
}

/// Triage relevance label (`explore.md` 4.1 `Triage.items[].relevance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageRelevance {
    Core,
    Related,
    Background,
    Unrelated,
}

impl TriageRelevance {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "core" => Some(Self::Core),
            "related" => Some(Self::Related),
            "background" => Some(Self::Background),
            "unrelated" => Some(Self::Unrelated),
            _ => None,
        }
    }
}

/// Reading status of a paper (`explore.md` 4.1 `PaperRead.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperReadStatus {
    ReadFull,
    ReadPassages,
    NothingRelevant,
    Unavailable,
    Failed,
}

impl PaperReadStatus {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "read_full" => Some(Self::ReadFull),
            "read_passages" => Some(Self::ReadPassages),
            "nothing_relevant" => Some(Self::NothingRelevant),
            "unavailable" => Some(Self::Unavailable),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// True when full text was obtained and read (satisfies rule V3 for a triage-core paper).
    pub const fn has_full_text_read(self) -> bool {
        matches!(self, Self::ReadFull | Self::ReadPassages | Self::NothingRelevant)
    }

    /// True when full text can back a `judged_*` overlap (rule V1: `read_full` or `read_passages`).
    pub const fn can_support_judged_overlap(self) -> bool {
        matches!(self, Self::ReadFull | Self::ReadPassages)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperReadCounts {
    pub read_full: usize,
    pub read_passages: usize,
    pub nothing_relevant: usize,
    pub unavailable: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorePaperGap {
    Unavailable,
    Failed,
    Unread,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedCorePaper {
    pub key: PaperKey,
    pub reason: CorePaperGap,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationExpansionCoverage {
    pub seeds: usize,
    pub hops: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownPaperRecall {
    pub given: Vec<PaperKey>,
    pub found_by_search: Vec<PaperKey>,
    pub missed: Vec<PaperKey>,
}

/// Every unsatisfied condition of the coverage gate is named individually (never short-circuited).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum GateFailure {
    QuickPresetNoFullText,
    TooFewAnsweredServices {
        answered: usize,
        required: usize,
        answered_providers: Vec<LitProvider>,
    },
    AttemptedServiceFailed {
        provider: LitProvider,
        queries_failed: usize,
    },
    AttemptedServiceRateLimited {
        provider: LitProvider,
        queries_rate_limited: usize,
    },
    MissingExactPerspective,
    MissingOtherNamesPerspective,
    InsufficientScreenedCandidates {
        screened: usize,
        required: usize,
        candidates_total: usize,
    },
    UnresolvedCorePapers {
        papers: Vec<UnresolvedCorePaper>,
    },
    MissedUserSeeds {
        missed: Vec<PaperKey>,
    },
}

/// Structured result of evaluating every coverage gate condition (`explore.md` V2, V3, V4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGate {
    pub passed: bool,
    pub preset_allows_none_in_scope: bool,
    pub answered_external_services: usize,
    pub min_external_services: usize,
    pub services_minimum_met: bool,
    pub no_attempted_service_failed: bool,
    pub exact_perspective_answered: bool,
    pub other_names_perspective_answered: bool,
    pub enough_candidates_screened: bool,
    pub all_core_papers_resolved: bool,
    pub all_seeds_found: bool,
    pub unsatisfied: Vec<GateFailure>,
}

/// Complete app-computed search coverage record (`explore.md` 4.1 `Coverage`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCoverage {
    pub preset: ExplorePreset,
    pub providers: Vec<ProviderCoverage>,
    pub library: LibraryCoverage,
    pub perspectives: Vec<PerspectiveCoverage>,
    pub candidates_total: usize,
    pub screened: usize,
    pub screened_required: usize,
    pub zero_candidates_from_answered_search: bool,
    pub papers: PaperReadCounts,
    pub unavailable_core: Vec<PaperKey>,
    pub unresolved_core: Vec<UnresolvedCorePaper>,
    pub dismissed_core: Vec<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year_range: Option<[i32; 2]>,
    pub citation_expansion: CitationExpansionCoverage,
    pub known_paper_recall: KnownPaperRecall,
    pub not_searched: Vec<String>,
    pub gate: CoverageGate,
}
