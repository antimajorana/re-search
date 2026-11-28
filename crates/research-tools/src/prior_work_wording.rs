//! Deterministic app-authored wording for prior-work verdicts, coverage scope, and evidence
//! attribution (package F12).
//!
//! Enforces `00-overview.md` 2.9–2.10, 3.2 (rows 20 and 24), and `explore.md` rules V2, V3, V4,
//! V5, V6, V8, and S8:
//! - Every on-screen label governed by an owner decision lives in [`wording`] as a named constant.
//! - Generated wording discloses what was searched, what was not searched, and every failing gate
//!   condition, distinguishing answered zero-result searches from no-answer/failed services.
//! - Abstract evidence wording stays strictly capped at the recorded verbatim abstract and
//!   possible relevance.
//! - Model judgments stay attributed (`"Model judges:"` until confirmed; `"(model)"` on prose).
//! - No generated wording calls a claim "new", "novel", "verified", or "proved".

#[path = "prior_work_wording_check.rs"]
mod check;

pub use check::{check_model_prose_phrases, contains_forbidden_prior_work_word};

use crate::lit_providers::LitProvider;
use crate::prior_work::{
    AbstractMatch, AcceptedOverlap, BorderStyle, DisplayGrade, OverlapRelation, PriorWorkStatus,
};
use crate::quote_core::{clip_quote, QUOTE_MATCHED_WORD};
use crate::search_coverage::{
    CorePaperGap, ExplorePreset, GateFailure, LibraryCoverage, PerspectiveCoverage,
    ProviderCoverage, ProviderCoverageState, SearchCoverage,
};
use std::collections::BTreeSet;

/// Named on-screen strings governed by owner decisions (`00-overview.md` 2.9, 2.10, 3.2, 8.x;
/// `explore.md` 4.1 & section 6), centralized so any string can change in one place.
pub mod wording {
    /// Model-verdict prefix (`00-overview.md` 3.2 row 24, `explore.md` V6).
    pub const MODEL_JUDGES_PREFIX: &str = "Model judges:";
    /// Model-prose suffix (`00-overview.md` 3.2 row 24, `explore.md` section 3).
    pub const MODEL_TEXT_SUFFIX: &str = "(model)";
    /// Prefix once the reader explicitly confirms a `judged_*` verdict (`explore.md` V6).
    pub const READER_CONFIRMED_PREFIX: &str = "Reader confirmed:";

    /// Status chips (`explore.md` 4.1 table, `00-overview.md` 2.9).
    pub const CHIP_JUDGED_SAME_UNCONFIRMED: &str = "Model judges: same";
    pub const CHIP_JUDGED_SAME_CONFIRMED: &str = "found (same)";
    pub const CHIP_JUDGED_PARTIAL_UNCONFIRMED: &str = "Model judges: partly the same";
    pub const CHIP_JUDGED_PARTIAL_CONFIRMED: &str = "found (partly)";
    pub const CHIP_JUDGED_OTHER_NAME_UNCONFIRMED: &str = "Model judges: same under another name";
    pub const CHIP_JUDGED_OTHER_NAME_CONFIRMED: &str = "found under another name";
    pub const CHIP_ABSTRACT_ONLY: &str = "possibly related (abstracts only)";
    pub const CHIP_NONE_IN_SCOPE: &str = "not found in the searched scope";
    pub const CHIP_INCOMPLETE: &str = "not found (incomplete search)";

    /// Shared evidence display-scale labels (`00-overview.md` 2.10 & 2026-11-26 clarification).
    pub const GRADE_BY_CODE: &str = "by code";
    pub const GRADE_FOUND_FULL_TEXT: &str = "Found";
    pub const GRADE_FOUND_OCR: &str = "Found (OCR)";
    pub const GRADE_ABSTRACT_ONLY: &str = "abstract only";
    pub const GRADE_MODEL_STATEMENT: &str = "(model)";
    pub const GRADE_USER_STATEMENT: &str = "you";
    /// Tag required on scanned/OCR quotes (`00-overview.md` 2.10, 8.3 Q3, `explore.md` C8).
    pub const TAG_OCR: &str = "OCR";
    /// Cap phrase for verbatim abstract evidence (brief F12, `00-overview.md` 2.9).
    pub const ABSTRACT_POSSIBLE_RELEVANCE: &str = "possible relevance";

    /// Banned absolute-claim phrases in model prose (`explore.md` rule V8).
    pub const BANNED_MODEL_PROSE_PHRASES: &[&str] = &[
        "has not been done",
        "has never been",
        "no prior work",
        "nobody has",
        "first to",
        "for the first time",
        "novel",
        "unexplored",
    ];

    /// Words forbidden in app-generated prior-work wording (`00-overview.md` 2.9, 3.2 row 20, V5/S8).
    pub const FORBIDDEN_GENERATED_WORDS: &[&str] = &[
        "new",
        "novel",
        "novelty",
        "verified",
        "proved",
        "unexplored",
    ];

    /// Every static on-screen constant in this module, verified by tests against forbidden words.
    pub const ALL_STATIC_CONSTANTS: &[&str] = &[
        MODEL_JUDGES_PREFIX,
        MODEL_TEXT_SUFFIX,
        READER_CONFIRMED_PREFIX,
        CHIP_JUDGED_SAME_UNCONFIRMED,
        CHIP_JUDGED_SAME_CONFIRMED,
        CHIP_JUDGED_PARTIAL_UNCONFIRMED,
        CHIP_JUDGED_PARTIAL_CONFIRMED,
        CHIP_JUDGED_OTHER_NAME_UNCONFIRMED,
        CHIP_JUDGED_OTHER_NAME_CONFIRMED,
        CHIP_ABSTRACT_ONLY,
        CHIP_NONE_IN_SCOPE,
        CHIP_INCOMPLETE,
        GRADE_BY_CODE,
        GRADE_FOUND_FULL_TEXT,
        GRADE_FOUND_OCR,
        GRADE_ABSTRACT_ONLY,
        GRADE_MODEL_STATEMENT,
        GRADE_USER_STATEMENT,
        TAG_OCR,
        ABSTRACT_POSSIBLE_RELEVANCE,
    ];
}

pub const fn provider_display_name(provider: LitProvider) -> &'static str {
    match provider {
        LitProvider::Arxiv => "arXiv",
        LitProvider::Inspire => "INSPIRE-HEP",
        LitProvider::SemanticScholar => "Semantic Scholar",
        LitProvider::Crossref => "Crossref",
        LitProvider::OpenAlex => "OpenAlex",
        LitProvider::TheoremSearch => "TheoremSearch",
    }
}

pub const fn status_chip(status: PriorWorkStatus, confirmed_by_user: bool) -> &'static str {
    match (status, confirmed_by_user) {
        (PriorWorkStatus::JudgedSame, false) => wording::CHIP_JUDGED_SAME_UNCONFIRMED,
        (PriorWorkStatus::JudgedSame, true) => wording::CHIP_JUDGED_SAME_CONFIRMED,
        (PriorWorkStatus::JudgedPartial, false) => wording::CHIP_JUDGED_PARTIAL_UNCONFIRMED,
        (PriorWorkStatus::JudgedPartial, true) => wording::CHIP_JUDGED_PARTIAL_CONFIRMED,
        (PriorWorkStatus::JudgedOtherName, false) => wording::CHIP_JUDGED_OTHER_NAME_UNCONFIRMED,
        (PriorWorkStatus::JudgedOtherName, true) => wording::CHIP_JUDGED_OTHER_NAME_CONFIRMED,
        (PriorWorkStatus::AbstractOnly, _) => wording::CHIP_ABSTRACT_ONLY,
        (PriorWorkStatus::NoneInScope, _) => wording::CHIP_NONE_IN_SCOPE,
        (PriorWorkStatus::Incomplete, _) => wording::CHIP_INCOMPLETE,
    }
}

pub const fn display_grade_label(grade: DisplayGrade) -> &'static str {
    match grade {
        DisplayGrade::ByCode => wording::GRADE_BY_CODE,
        DisplayGrade::MatchedQuoteFullText => wording::GRADE_FOUND_FULL_TEXT,
        DisplayGrade::MatchedQuoteScannedPage => wording::GRADE_FOUND_OCR,
        DisplayGrade::MatchedQuoteAbstract => wording::GRADE_ABSTRACT_ONLY,
        DisplayGrade::ModelStatement => wording::GRADE_MODEL_STATEMENT,
        DisplayGrade::UserStatement => wording::GRADE_USER_STATEMENT,
    }
}

pub const fn display_grade_border(grade: DisplayGrade) -> BorderStyle {
    match grade {
        DisplayGrade::ByCode
        | DisplayGrade::MatchedQuoteFullText
        | DisplayGrade::MatchedQuoteScannedPage => BorderStyle::Solid,
        DisplayGrade::MatchedQuoteAbstract | DisplayGrade::ModelStatement => BorderStyle::Dashed,
        DisplayGrade::UserStatement => BorderStyle::Plain,
    }
}

/// Format a provider's coverage state, distinguishing answered zero-result responses from
/// failures, rate limits, unconfigured/disabled services, skipped/unsupported queries, and
/// unqueried no-answer states.
pub fn format_provider_coverage(p: &ProviderCoverage) -> String {
    let name = provider_display_name(p.provider);
    match p.state {
        ProviderCoverageState::Ok => {
            let qw = plural(p.queries_ok, "query", "queries");
            if p.hits == 0 {
                format!("{name}: answered (0 results, {} {qw})", p.queries_ok)
            } else {
                let hw = plural(p.hits, "hit", "hits");
                format!("{name}: answered ({} {hw}, {} {qw})", p.hits, p.queries_ok)
            }
        }
        ProviderCoverageState::Partial => format!(
            "{name}: partial ({} answered, {} failed, {} rate-limited, {} {})",
            p.queries_ok,
            p.queries_failed,
            p.queries_rate_limited,
            p.hits,
            plural(p.hits, "hit", "hits"),
        ),
        ProviderCoverageState::Failed => {
            format!("{name}: failed (no answer, {} failed)", p.queries_failed.max(1))
        }
        ProviderCoverageState::RateLimited => format!(
            "{name}: rate-limited (no answer, {} rate-limited)",
            p.queries_rate_limited.max(1)
        ),
        ProviderCoverageState::NotConfigured => format!("{name}: not configured"),
        ProviderCoverageState::Disabled => format!("{name}: disabled"),
        ProviderCoverageState::NotAttempted => format_not_attempted_provider(name, p),
    }
}

fn format_not_attempted_provider(name: &str, p: &ProviderCoverage) -> String {
    let mut notes = Vec::new();
    if p.queries_skipped_budget > 0 {
        notes.push(format!("{} skipped due to budget", p.queries_skipped_budget));
    }
    if p.queries_unsupported > 0 {
        notes.push(format!("{} unsupported", p.queries_unsupported));
    }
    if p.seed_lookup_queries_ok > 0 {
        notes.push(format!(
            "{} seed {}",
            p.seed_lookup_queries_ok,
            plural(p.seed_lookup_queries_ok, "lookup", "lookups")
        ));
    }
    if notes.is_empty() {
        format!("{name}: no answer (not queried)")
    } else {
        format!("{name}: no answer ({})", notes.join(", "))
    }
}

/// Format the private library summary (`00-overview.md` 2.9).
pub fn format_library_coverage(lib: &LibraryCoverage) -> String {
    if !lib.searched {
        return "your library: not searched (local only; never counts toward the external service minimum)".to_string();
    }
    format!(
        "your library: {} {}, {} with full text, {} {} (local only; does not count toward the external service minimum)",
        lib.papers,
        plural(lib.papers, "paper", "papers"),
        lib.with_full_text,
        lib.hits,
        plural(lib.hits, "hit", "hits"),
    )
}

/// Format one unsatisfied coverage gate condition into plain text.
pub fn format_gate_failure(failure: &GateFailure) -> String {
    match failure {
        GateFailure::QuickPresetNoFullText => {
            "Quick preset reads abstracts only and never qualifies for not-found-in-scope".to_string()
        }
        GateFailure::TooFewAnsweredServices { answered, required, answered_providers } => {
            if answered_providers.is_empty() {
                format!(
                    "0 of {required} required external services answered (private library does not count)"
                )
            } else {
                let names: Vec<&str> =
                    answered_providers.iter().map(|p| provider_display_name(*p)).collect();
                format!(
                    "only {answered} of {required} required external services answered ({}; private library does not count)",
                    names.join(", ")
                )
            }
        }
        GateFailure::AttemptedServiceFailed { provider, queries_failed } => format!(
            "attempted service {} failed ({} failed {})",
            provider_display_name(*provider),
            queries_failed,
            plural(*queries_failed, "query", "queries")
        ),
        GateFailure::AttemptedServiceRateLimited { provider, queries_rate_limited } => format!(
            "attempted service {} was rate-limited ({} rate-limited {})",
            provider_display_name(*provider),
            queries_rate_limited,
            plural(*queries_rate_limited, "query", "queries")
        ),
        GateFailure::MissingExactPerspective => {
            "exact-match perspective had no answered query".to_string()
        }
        GateFailure::MissingOtherNamesPerspective => {
            "alternative-name (other_names) perspective had no answered query".to_string()
        }
        GateFailure::InsufficientScreenedCandidates { screened, required, candidates_total } => {
            format!(
                "screened {screened} of {required} required candidates (out of {candidates_total} total)"
            )
        }
        GateFailure::UnresolvedCorePapers { papers } => {
            let items: Vec<String> =
                papers.iter().map(|p| format!("{} ({})", p.key, core_gap_label(p.reason))).collect();
            format!(
                "triage-core {} without full text or reader dismissal: {}",
                plural(papers.len(), "paper", "papers"),
                items.join(", ")
            )
        }
        GateFailure::MissedUserSeeds { missed } => {
            let keys: Vec<&str> = missed.iter().map(|k| k.as_str()).collect();
            format!(
                "user-supplied seed {} not found by search: {}",
                plural(missed.len(), "paper", "papers"),
                keys.join(", ")
            )
        }
    }
}

/// Format the full search-scope disclosure.
pub fn format_coverage_scope(coverage: &SearchCoverage) -> String {
    let services_text = if coverage.providers.is_empty() {
        "none".to_string()
    } else {
        coverage.providers.iter().map(format_provider_coverage).collect::<Vec<_>>().join("; ")
    };
    let answered_persp: Vec<String> =
        coverage.perspectives.iter().filter(|p| p.answered()).map(format_perspective_brief).collect();
    let persp_text = if answered_persp.is_empty() {
        "none answered".to_string()
    } else {
        answered_persp.join(", ")
    };
    let full_text_read =
        coverage.papers.read_full + coverage.papers.read_passages + coverage.papers.nothing_relevant;
    let zero_note = if coverage.zero_candidates_from_answered_search {
        " (answered search returned 0 candidates)"
    } else {
        ""
    };
    let not_searched = if coverage.not_searched.is_empty() {
        String::new()
    } else {
        format!(" Not searched: {}.", coverage.not_searched.join("; "))
    };
    format!(
        "Scope ({preset} preset) — external services: {services_text}. Perspectives answered: {persp_text}. Screened {screened} of {cands} {cand_word}{zero_note}; full text read for {full_text_read} {paper_word} ({rf} full, {rp} passages, {nr} nothing relevant). {lib}.{not_searched}",
        preset = coverage.preset.as_str(),
        screened = coverage.screened,
        cands = coverage.candidates_total,
        cand_word = plural(coverage.candidates_total, "candidate", "candidates"),
        paper_word = plural(full_text_read, "paper", "papers"),
        rf = coverage.papers.read_full,
        rp = coverage.papers.read_passages,
        nr = coverage.papers.nothing_relevant,
        lib = format_library_coverage(&coverage.library),
    )
}

/// Generate the app-authored verdict sentence (`explore.md` V2, V3, V5, V6; `00-overview.md` 2.9).
pub fn format_verdict_sentence(
    status: PriorWorkStatus,
    confirmed_by_user: bool,
    overlaps: &[AcceptedOverlap],
    abstract_matches: &[AbstractMatch],
    coverage: &SearchCoverage,
) -> String {
    let scope = format_coverage_scope(coverage);
    match status {
        PriorWorkStatus::JudgedSame
        | PriorWorkStatus::JudgedPartial
        | PriorWorkStatus::JudgedOtherName => {
            format_judged_with_secondary(status, confirmed_by_user, overlaps, &scope)
        }
        PriorWorkStatus::AbstractOnly => {
            let (papers, eids) = collect_abstract_refs(abstract_matches);
            let incomplete_clause = if coverage.gate.passed {
                String::new()
            } else {
                format!(" Search scope is also incomplete: {}.", format_unsatisfied_list(coverage))
            };
            format!(
                "Possibly related ({rel}) based on recorded verbatim abstract {quote_word} ({eids}) from {papers}; full text was not read. Abstract evidence is limited to the recorded verbatim abstract and {rel}.{incomplete_clause} {scope}",
                rel = wording::ABSTRACT_POSSIBLE_RELEVANCE,
                quote_word = plural(abstract_matches.len(), "quote", "quotes"),
            )
        }
        PriorWorkStatus::NoneInScope => format!("Not found in the searched scope. {scope}"),
        PriorWorkStatus::Incomplete => {
            let failing_clause = format_unsatisfied_list(coverage);
            let quick_clean_screen = coverage.preset == ExplorePreset::Quick
                && coverage.gate.answered_external_services > 0
                && coverage.gate.no_attempted_service_failed
                && (coverage.screened > 0 || coverage.zero_candidates_from_answered_search);
            if quick_clean_screen {
                format!(
                    "No close match among {} {}; nothing was read. Unsatisfied conditions: {failing_clause}. {scope}",
                    coverage.screened,
                    plural(coverage.screened, "abstract", "abstracts"),
                )
            } else {
                format!(
                    "Search incomplete; no conclusion about prior work in scope. Unsatisfied conditions: {failing_clause}. {scope}"
                )
            }
        }
    }
}

/// Format an abstract-evidence snippet strictly capped at the recorded verbatim abstract (<= 300
/// chars) and possible relevance (brief F12, `00-overview.md` 2.5 & 2.9).
pub fn format_abstract_evidence_note(ev: &AbstractMatch) -> String {
    let clipped = clip_quote(ev.quote.trim());
    let prov = ev.provider.map(provider_display_name).unwrap_or("recorded service");
    format!(
        "Recorded verbatim abstract ({}, {prov}, {}): \"{clipped}\" — {} only (full text not read).",
        ev.evidence_id,
        wording::GRADE_ABSTRACT_ONLY,
        wording::ABSTRACT_POSSIBLE_RELEVANCE,
    )
}

/// Attribute model-written prose with the shared `"(model)"` suffix (`00-overview.md` 3.2 row 24).
pub fn format_model_text(label: &str, text: &str) -> String {
    let trimmed_label = label.trim();
    let trimmed_text = text.trim();
    if trimmed_label.ends_with(wording::MODEL_TEXT_SUFFIX) {
        format!("{trimmed_label}: {trimmed_text}")
    } else {
        format!("{trimmed_label} {}: {trimmed_text}", wording::MODEL_TEXT_SUFFIX)
    }
}

fn format_unsatisfied_list(coverage: &SearchCoverage) -> String {
    if coverage.gate.unsatisfied.is_empty() {
        "search scope requirements were not satisfied".to_string()
    } else {
        coverage.gate.unsatisfied.iter().map(format_gate_failure).collect::<Vec<_>>().join("; ")
    }
}

fn relation_phrase(rel: OverlapRelation) -> &'static str {
    match rel {
        OverlapRelation::Same => "same as",
        OverlapRelation::Partial => "partly the same as",
        OverlapRelation::OtherName => "same under another name as",
    }
}

fn format_judged_with_secondary(
    status: PriorWorkStatus,
    confirmed_by_user: bool,
    overlaps: &[AcceptedOverlap],
    scope: &str,
) -> String {
    let primary: Vec<&AcceptedOverlap> =
        overlaps.iter().filter(|o| o.relation.to_status() == status).collect();
    let secondary: Vec<&AcceptedOverlap> =
        overlaps.iter().filter(|o| o.relation.to_status() != status).collect();
    let primary_slice = if primary.is_empty() { overlaps.iter().collect() } else { primary };
    let rel_phrase = match status {
        PriorWorkStatus::JudgedSame => relation_phrase(OverlapRelation::Same),
        PriorWorkStatus::JudgedPartial => relation_phrase(OverlapRelation::Partial),
        _ => relation_phrase(OverlapRelation::OtherName),
    };
    let other_terms: Vec<&str> = if status == PriorWorkStatus::JudgedOtherName {
        primary_slice.iter().filter_map(|o| o.other_name.as_deref()).collect()
    } else {
        Vec::new()
    };
    let (papers, eids, eid_count, any_ocr) = collect_overlap_refs(&primary_slice);
    let name_clause = if other_terms.is_empty() {
        String::new()
    } else {
        format!(" (full-text term: \"{}\")", other_terms.join(", "))
    };
    let ocr_clause = if any_ocr { format!(" [{} source]", wording::TAG_OCR) } else { String::new() };
    let quote_word = plural(eid_count.max(1), "quote", "quotes");
    let secondary_clause = format_secondary_overlaps(&secondary);

    if confirmed_by_user {
        format!(
            "{} {rel_phrase} {papers}{name_clause} based on {QUOTE_MATCHED_WORD} full-text {quote_word} ({eids}){ocr_clause}.{secondary_clause} {scope}",
            wording::READER_CONFIRMED_PREFIX,
        )
    } else {
        format!(
            "{} {rel_phrase} {papers}{name_clause} based on {QUOTE_MATCHED_WORD} full-text {quote_word} ({eids}){ocr_clause}; unconfirmed by reader.{secondary_clause} {scope}",
            wording::MODEL_JUDGES_PREFIX,
        )
    }
}

fn format_secondary_overlaps(secondary: &[&AcceptedOverlap]) -> String {
    if secondary.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = secondary
        .iter()
        .map(|o| {
            let eids = o.evidence_ids.iter().map(|e| e.format_id()).collect::<Vec<_>>().join(", ");
            let ocr = if o.ocr_sourced { format!(" [{} source]", wording::TAG_OCR) } else { String::new() };
            format!("{} {} ({eids}){ocr}", relation_phrase(o.relation), o.paper_key)
        })
        .collect();
    format!(" Additional recorded overlap: {}.", parts.join("; "))
}

fn collect_overlap_refs(overlaps: &[&AcceptedOverlap]) -> (String, String, usize, bool) {
    let mut papers = BTreeSet::new();
    let mut eids = BTreeSet::new();
    let mut any_ocr = false;
    for o in overlaps {
        papers.insert(o.paper_key.as_str());
        for eid in &o.evidence_ids {
            eids.insert(eid.format_id());
        }
        any_ocr |= o.ocr_sourced;
    }
    let p_str = if papers.is_empty() {
        "recorded source".to_string()
    } else {
        papers.into_iter().collect::<Vec<_>>().join(", ")
    };
    let eid_count = eids.len();
    let e_str = if eids.is_empty() {
        "none".to_string()
    } else {
        eids.into_iter().collect::<Vec<_>>().join(", ")
    };
    (p_str, e_str, eid_count, any_ocr)
}

fn collect_abstract_refs(matches: &[AbstractMatch]) -> (String, String) {
    let mut papers = BTreeSet::new();
    let mut eids = BTreeSet::new();
    for m in matches {
        papers.insert(m.paper_key.as_str());
        eids.insert(m.evidence_id.format_id());
    }
    let p_str = if papers.is_empty() {
        "recorded candidate".to_string()
    } else {
        papers.into_iter().collect::<Vec<_>>().join(", ")
    };
    let e_str = if eids.is_empty() {
        "none".to_string()
    } else {
        eids.into_iter().collect::<Vec<_>>().join(", ")
    };
    (p_str, e_str)
}

fn format_perspective_brief(p: &PerspectiveCoverage) -> String {
    if p.hits == 0 {
        format!("{} (0 results)", p.kind.as_str())
    } else {
        format!("{} ({} {})", p.kind.as_str(), p.hits, plural(p.hits, "hit", "hits"))
    }
}

fn core_gap_label(gap: CorePaperGap) -> &'static str {
    match gap {
        CorePaperGap::Unavailable => "full text unavailable",
        CorePaperGap::Failed => "reading failed",
        CorePaperGap::Unread => "unread",
    }
}

fn plural(n: usize, singular: &'static str, plural_word: &'static str) -> &'static str {
    if n == 1 { singular } else { plural_word }
}

#[cfg(test)]
#[path = "prior_work_wording_tests.rs"]
mod tests;
