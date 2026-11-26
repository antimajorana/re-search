//! Shared prior-work status model and deterministic verdict evaluator (package F12).
//!
//! Implements the six-state [`PriorWorkStatus`] enum (`00-overview.md` 2.9), the shared
//! display-grade mapping (`00-overview.md` 2.10, reusing [`DisplayGrade`] and [`BorderStyle`]
//! from `solve_status`), and Explore's verdict honesty rules V1–V10 (`explore.md` section 6).
//!
//! Pure code: no network calls, no model calls, no routes, no tool definitions.
//! Second-model independence (`different_provider | different_model | same_model`) is left to S14,
//! and the Explore pipeline state machine is left to X7–X11.

#[path = "prior_work_eval.rs"]
mod eval;
#[path = "prior_work_events.rs"]
mod events;

use crate::investigation::anchor::fold_with_map;
use crate::lit_providers::LitProvider;
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use crate::quote_core::{
    abstract_id, parse_abstract_id, parse_quote_id, quote_id, EvidenceRole, QuoteRef,
};
use crate::search_coverage::{
    compute_coverage_from_events, paper_ref_matches_key, PaperReadStatus, SearchCoverage,
};
pub use crate::solve_status::{BorderStyle, DisplayGrade};
use ledger::Event;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Explore map event carrying model-supplied overlaps (`explore.md` 4.2).
pub const IDEA_MAP_EVENT: &str = "idea.map";
/// Explore verdict event kind (`explore.md` 4.2).
pub const IDEA_VERDICT_EVENT: &str = "idea.verdict";
/// Shared hand-off packet event (`00-overview.md` 4.1, section 8.2 decision 10).
pub const HANDOFF_PACKET_EVENT: &str = "handoff.packet";
/// Shared source ingestion event (`explore.md` 4.2, F14 `pdf_pages`).
pub const SOURCE_INGESTED_EVENT: &str = "source.ingested";
/// User note actions governing verdict confirmation (`explore.md` 4.2 & rule V6).
pub const NOTE_ACTION_CONFIRM_SAME: &str = "confirm_same";
pub const NOTE_ACTION_REJECT_SAME: &str = "reject_same";
/// Maximum character length of model-written `difference` prose (`explore.md` 4.1).
pub const MAX_DIFFERENCE_CHARS: usize = 600;

/// Shared six-state prior-work status (`00-overview.md` 2.9).
///
/// No variant means "new" or "novel".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorWorkStatus {
    JudgedSame,
    JudgedPartial,
    JudgedOtherName,
    AbstractOnly,
    NoneInScope,
    Incomplete,
}

impl PriorWorkStatus {
    pub const ALL: [PriorWorkStatus; 6] = [
        Self::JudgedSame,
        Self::JudgedPartial,
        Self::JudgedOtherName,
        Self::AbstractOnly,
        Self::NoneInScope,
        Self::Incomplete,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JudgedSame => "judged_same",
            Self::JudgedPartial => "judged_partial",
            Self::JudgedOtherName => "judged_other_name",
            Self::AbstractOnly => "abstract_only",
            Self::NoneInScope => "none_in_scope",
            Self::Incomplete => "incomplete",
        }
    }

    /// True for statuses backed by a model equivalence/overlap judgment over full text.
    pub const fn is_judged(self) -> bool {
        matches!(self, Self::JudgedSame | Self::JudgedPartial | Self::JudgedOtherName)
    }

    /// Display grade of the verdict status itself (`00-overview.md` 2.10).
    pub const fn display_grade(
        self,
        confirmed_by_user: bool,
        has_ocr_evidence: bool,
    ) -> DisplayGrade {
        match self {
            Self::NoneInScope | Self::Incomplete => DisplayGrade::ByCode,
            Self::AbstractOnly => DisplayGrade::MatchedQuoteAbstract,
            Self::JudgedSame | Self::JudgedPartial | Self::JudgedOtherName => {
                if confirmed_by_user {
                    if has_ocr_evidence {
                        DisplayGrade::MatchedQuoteScannedPage
                    } else {
                        DisplayGrade::MatchedQuoteFullText
                    }
                } else {
                    DisplayGrade::ModelStatement
                }
            }
        }
    }

    /// Border/line style under the shared rule (`00-overview.md` 2.6, 2.10, decision 4).
    pub const fn border_style(self, confirmed_by_user: bool) -> BorderStyle {
        match self {
            Self::NoneInScope | Self::Incomplete => BorderStyle::Solid,
            Self::AbstractOnly => BorderStyle::Dashed,
            Self::JudgedSame | Self::JudgedPartial | Self::JudgedOtherName => {
                if confirmed_by_user {
                    BorderStyle::Solid
                } else {
                    BorderStyle::Dashed
                }
            }
        }
    }
}

impl fmt::Display for PriorWorkStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed F4 evidence identifier: `xe-<seq>` (full text) or `xa-<seq>` (verbatim abstract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EvidenceId {
    FullText(u64),
    Abstract(u64),
}

impl EvidenceId {
    pub fn full_text(seq: u64) -> Result<Self, String> {
        quote_id(seq).map(|_| Self::FullText(seq))
    }

    pub fn abstract_quote(seq: u64) -> Result<Self, String> {
        abstract_id(seq).map(|_| Self::Abstract(seq))
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        if let Some(seq) = parse_quote_id(raw) {
            return Ok(Self::FullText(seq));
        }
        if let Some(seq) = parse_abstract_id(raw) {
            return Ok(Self::Abstract(seq));
        }
        Err(format!("invalid evidence id `{raw}` (expected `xe-<seq>` or `xa-<seq>`)"))
    }

    pub const fn is_valid(self) -> bool {
        self.seq() >= 1
    }

    pub fn format_id(self) -> String {
        match self {
            Self::FullText(seq) => quote_id(seq).unwrap_or_else(|_| "xe-invalid".to_string()),
            Self::Abstract(seq) => abstract_id(seq).unwrap_or_else(|_| "xa-invalid".to_string()),
        }
    }

    pub const fn is_full_text(self) -> bool {
        matches!(self, Self::FullText(_))
    }

    pub const fn is_abstract(self) -> bool {
        matches!(self, Self::Abstract(_))
    }

    pub const fn seq(self) -> u64 {
        match self {
            Self::FullText(s) | Self::Abstract(s) => s,
        }
    }
}

impl fmt::Display for EvidenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_id())
    }
}

impl Serialize for EvidenceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !self.is_valid() {
            return Err(serde::ser::Error::custom("evidence id seq must be >= 1"));
        }
        serializer.serialize_str(&self.format_id())
    }
}

impl<'de> Deserialize<'de> for EvidenceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Source text provenance (`explore.md` 4.1 `PaperRead.text_basis`, rule C8, overview 8.3 Q3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextBasis {
    #[default]
    Tex,
    PdfText,
    ScanTextLayer,
    Ocr,
}

impl TextBasis {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tex" => Some(Self::Tex),
            "pdf_text" | "text_layer" => Some(Self::PdfText),
            "scan_text_layer" | "scan" => Some(Self::ScanTextLayer),
            "ocr" | "none" => Some(Self::Ocr),
            _ => None,
        }
    }

    /// True when the quote came from a scanned page and must be tagged `"OCR"` (rule C8).
    pub const fn is_ocr_source(self) -> bool {
        matches!(self, Self::ScanTextLayer | Self::Ocr)
    }

    pub const fn display_grade(self) -> DisplayGrade {
        if self.is_ocr_source() {
            DisplayGrade::MatchedQuoteScannedPage
        } else {
            DisplayGrade::MatchedQuoteFullText
        }
    }
}

/// Shared display-grade helper for a single piece of evidence (`00-overview.md` 2.10).
pub const fn evidence_display_grade(is_abstract: bool, text_basis: TextBasis) -> DisplayGrade {
    if is_abstract {
        DisplayGrade::MatchedQuoteAbstract
    } else {
        text_basis.display_grade()
    }
}

/// Model-supplied overlap relation (`explore.md` 4.1 `Overlap.relation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapRelation {
    Same,
    Partial,
    OtherName,
}

impl OverlapRelation {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "same" => Some(Self::Same),
            "partial" => Some(Self::Partial),
            "other_name" => Some(Self::OtherName),
            _ => None,
        }
    }

    pub const fn to_status(self) -> PriorWorkStatus {
        match self {
            Self::Same => PriorWorkStatus::JudgedSame,
            Self::Partial => PriorWorkStatus::JudgedPartial,
            Self::OtherName => PriorWorkStatus::JudgedOtherName,
        }
    }

    pub(crate) const fn priority(self) -> u8 {
        match self {
            Self::Same => 3,
            Self::OtherName => 2,
            Self::Partial => 1,
        }
    }
}

/// Verified full-text quote record (`evidence.quote`, id `xe-<seq>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextEvidence {
    pub id: EvidenceId,
    pub paper_key: PaperKey,
    pub quote: String,
    #[serde(default)]
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_ref: Option<QuoteRef>,
    #[serde(default)]
    pub text_basis: TextBasis,
    #[serde(default)]
    pub subclaim_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<EvidenceRole>,
}

/// Recorded verbatim abstract quote (`evidence.abstract`, id `xa-<seq>`, rule C3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractEvidence {
    pub id: EvidenceId,
    pub paper_key: PaperKey,
    pub quote: String,
    /// Must be `true` to count as evidence (rebuilt inverted-index abstracts are never evidence).
    pub verbatim: bool,
    #[serde(default)]
    pub raw_blob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<LitProvider>,
    #[serde(default)]
    pub subclaim_ids: Vec<String>,
}

/// Model-supplied overlap claim before code validation (`explore.md` 4.1 `Overlap`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlapCandidate {
    pub subclaim_id: String,
    pub paper_key: PaperKey,
    pub relation: OverlapRelation,
    pub evidence_ids: Vec<EvidenceId>,
    #[serde(default)]
    pub difference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_name: Option<String>,
}

/// Validated full-text overlap accepted for a [`PriorWorkVerdict`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedOverlap {
    pub paper_key: PaperKey,
    pub relation: OverlapRelation,
    pub evidence_ids: Vec<EvidenceId>,
    pub difference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_name: Option<String>,
    pub ocr_sourced: bool,
}

/// Validated verbatim abstract match supporting at most [`PriorWorkStatus::AbstractOnly`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractMatch {
    pub paper_key: PaperKey,
    pub evidence_id: EvidenceId,
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<LitProvider>,
}

/// Reason a model-supplied [`OverlapCandidate`] was refused by code rules V1/V8/V9.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum OverlapRejectionReason {
    OriginPaperExcluded,
    PaperNotRead {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<PaperReadStatus>,
    },
    NoValidFullTextEvidence,
    UnknownEvidenceId {
        evidence_id: EvidenceId,
    },
    EvidencePaperMismatch {
        evidence_id: EvidenceId,
    },
    OtherNameMissing,
    OtherNameNotInFullTextQuote {
        term: String,
    },
    DifferenceTooLong {
        chars: usize,
        max: usize,
    },
    BannedProsePhrase {
        phrases: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedOverlap {
    pub subclaim_id: String,
    pub paper_key: PaperKey,
    pub relation: OverlapRelation,
    pub reason: OverlapRejectionReason,
}

/// App-computed prior-work verdict for one claim/subclaim (`explore.md` 4.1 `Verdict`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorWorkVerdict {
    pub subclaim_id: String,
    pub status: PriorWorkStatus,
    pub overlaps: Vec<AcceptedOverlap>,
    pub abstract_matches: Vec<AbstractMatch>,
    pub rejected_overlaps: Vec<RejectedOverlap>,
    pub confirmed_by_user: bool,
    pub has_ocr_evidence: bool,
    pub display_grade: DisplayGrade,
    pub border_style: BorderStyle,
    pub chip: String,
    pub text: String,
}

/// Typed input to [`evaluate_prior_work`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorWorkInput {
    pub subclaim_ids: Vec<String>,
    pub coverage: SearchCoverage,
    #[serde(default)]
    pub paper_reads: BTreeMap<PaperKey, PaperReadStatus>,
    #[serde(default)]
    pub paper_text_basis: BTreeMap<PaperKey, TextBasis>,
    #[serde(default)]
    pub full_text_evidence: Vec<FullTextEvidence>,
    #[serde(default)]
    pub abstract_evidence: Vec<AbstractEvidence>,
    #[serde(default)]
    pub overlaps: Vec<OverlapCandidate>,
    /// Subclaim ids explicitly confirmed by the reader (`confirm_same`, not superseded by
    /// `reject_same`, `idea.stated`, or a changed `idea.map`; rule V6).
    #[serde(default)]
    pub confirmed_subclaims: BTreeSet<String>,
    /// Origin paper (with F2 aliases) when opened from `handoff.packet.from.paper` (rule V9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_paper: Option<PaperRef>,
}

/// Combined pure output from [`compute_prior_work_from_events`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorWorkReport {
    pub coverage: SearchCoverage,
    pub verdicts: Vec<PriorWorkVerdict>,
}

/// Convenience helper constructing a [`PaperRef`] from a single canonical [`PaperKey`].
pub fn origin_ref_from_key(key: PaperKey) -> PaperRef {
    PaperRef { key, version: None, ids: PaperIds::default() }
}

/// Rule V9 helper: checks whether `key` matches `origin` by primary key or any F2 alias id.
pub fn is_origin_paper(origin: Option<&PaperRef>, key: &PaperKey) -> bool {
    origin.is_some_and(|orig| paper_ref_matches_key(orig, key))
}

/// Check whether `term` occurs in `text` after whitespace normalization and lowercasing.
/// Rejects empty or punctuation-only terms so `judged_other_name` requires real textual evidence.
pub fn quote_contains_term(text: &str, term: &str) -> bool {
    let norm_term = fold_with_map(term).0.to_lowercase();
    if norm_term.is_empty() || !norm_term.chars().any(char::is_alphanumeric) {
        return false;
    }
    let norm_text = fold_with_map(text).0.to_lowercase();
    norm_text.contains(&norm_term)
}

/// Pure verdict computation for every subclaim in `input` (`explore.md` rules V1–V10).
pub fn evaluate_prior_work(input: &PriorWorkInput) -> Vec<PriorWorkVerdict> {
    let ft_by_id: BTreeMap<EvidenceId, &FullTextEvidence> = input
        .full_text_evidence
        .iter()
        .filter(|ev| ev.id.is_full_text() && ev.id.is_valid() && !ev.quote.trim().is_empty())
        .map(|ev| (ev.id, ev))
        .collect();
    let abs_by_id: BTreeMap<EvidenceId, &AbstractEvidence> = input
        .abstract_evidence
        .iter()
        .filter(|ev| {
            ev.id.is_abstract() && ev.id.is_valid() && ev.verbatim && !ev.quote.trim().is_empty()
        })
        .map(|ev| (ev.id, ev))
        .collect();

    input
        .subclaim_ids
        .iter()
        .map(|sc| eval::evaluate_one_subclaim(sc, input, &ft_by_id, &abs_by_id))
        .collect()
}

/// Pure ledger-to-report recomputation (`explore.md` V7 & V10).
/// Ignores `lessons.supplied` and all non-confirmation F16/ranking marks.
pub fn compute_prior_work_from_events(events: &[Event]) -> PriorWorkReport {
    let coverage = compute_coverage_from_events(events);
    let input = events::extract_prior_work_input(events, coverage.clone());
    let verdicts = evaluate_prior_work(&input);
    PriorWorkReport { coverage, verdicts }
}

/// Rule V7 export/parity check: confirms a stored [`PriorWorkReport`] equals the pure
/// recomputation from `events`.
pub fn report_matches_recomputed(stored: &PriorWorkReport, events: &[Event]) -> bool {
    *stored == compute_prior_work_from_events(events)
}

#[cfg(test)]
#[path = "prior_work_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prior_work_audit_tests.rs"]
mod audit_tests;
