//! Pure deterministic selection & code-reuse rules for F16a (`00-overview.md` 2.14).
//!
//! No model is involved anywhere here, and nothing here ever alters a badge, tier, verdict,
//! coverage gate, or quote-matching result. It changes only:
//! 1. Candidate paper rankings & Explore seed suggestions from F10 library user marks
//! 2. Query alias expansion and rejected-pair suppression from confirmed/rejected term pairs
//! 3. Editable symbol & convention defaults offered on a Solve setup card
//! 4. Non-mutating display overlays for skeleton node overrides and OCR line corrections

use crate::lessons::LessonsStore;
use crate::lessons_schema::{
    canonical_term_pair, normalize_term, ConventionDefault, OcrOverlay, SkeletonOverride,
    SymbolDefault, TermAliasDecision, IRRELEVANT_PAPER_RANK_PENALTY, KEY_PAPER_RANK_BOOST,
    OCR_OVERLAY_LABEL,
};
use crate::library::{Library, MarkStatus, PaperSummary, UserMark};
use crate::paper_key::PaperKey;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One ranked candidate paper input to [`apply_mark_ranking`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedPaperCandidate {
    pub key: PaperKey,
    pub base_score: i32,
    pub adjusted_score: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark_status: Option<MarkStatus>,
    /// True when marked `irrelevant`: demoted in ordering, **still listed** (`00-overview.md` 2.14).
    pub demoted: bool,
    /// True when marked `key`: boosted in ordering.
    pub boosted: bool,
}

/// Apply F10 library `user_mark` adjustments to a candidate list:
/// - `MarkStatus::Key` is boosted (`+KEY_PAPER_RANK_BOOST`)
/// - `MarkStatus::Irrelevant` is demoted (`IRRELEVANT_PAPER_RANK_PENALTY`), **never dropped**
/// - Ties preserve deterministic ordering by `key`.
pub fn apply_mark_ranking(
    candidates: &[(PaperKey, i32)],
    marks: &[UserMark],
) -> Vec<RankedPaperCandidate> {
    let mark_map: BTreeMap<&str, MarkStatus> = marks
        .iter()
        .map(|m| (m.key.as_str(), m.status))
        .collect();

    let mut ranked: Vec<RankedPaperCandidate> = candidates
        .iter()
        .map(|(key, base)| {
            let status = mark_map.get(key.as_str()).copied();
            let (delta, demoted, boosted) = match status {
                Some(MarkStatus::Key) => (KEY_PAPER_RANK_BOOST, false, true),
                Some(MarkStatus::Irrelevant) => (IRRELEVANT_PAPER_RANK_PENALTY, true, false),
                _ => (0, false, false),
            };
            RankedPaperCandidate {
                key: key.clone(),
                base_score: *base,
                adjusted_score: base.saturating_add(delta),
                mark_status: status,
                demoted,
                boosted,
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.adjusted_score
            .cmp(&a.adjusted_score)
            .then_with(|| b.base_score.cmp(&a.base_score))
            .then_with(|| a.key.cmp(&b.key))
    });
    ranked
}

/// Find papers marked `MarkStatus::Key` in `lib` whose title, tags, or user note match any
/// of `query_terms` so Explore can offer them as seeds (`00-overview.md` 2.14, Explore decision 11).
pub fn matching_key_paper_seeds(
    lib: &Library,
    query_terms: &[&str],
    limit: usize,
) -> Result<Vec<PaperSummary>, String> {
    let norm_terms: Vec<String> = query_terms
        .iter()
        .map(|t| normalize_term(t))
        .filter(|t| !t.is_empty())
        .collect();
    if norm_terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let key_papers = lib.search(&crate::library::LibraryQuery {
        status: Some(MarkStatus::Key),
        limit: Some(crate::library::MAX_LIBRARY_LIMIT),
        ..Default::default()
    })?;
    let mut matched = Vec::new();
    for paper in key_papers {
        let detail = lib.get_paper(&paper.paper.key)?;
        let mut haystack = paper.paper.title.to_lowercase();
        if let Some(d) = detail {
            haystack.push(' ');
            haystack.push_str(&d.paper.abstract_text.to_lowercase());
            if let Some(mark) = d.user_mark {
                for t in mark.tags {
                    haystack.push(' ');
                    haystack.push_str(&t.to_lowercase());
                }
                haystack.push(' ');
                haystack.push_str(&mark.note.to_lowercase());
            }
        }
        if norm_terms.iter().any(|term| haystack.contains(term.as_str())) {
            matched.push(paper);
            if matched.len() >= limit {
                break;
            }
        }
    }
    Ok(matched)
}

/// Expand `query_terms` with every user-confirmed alias (`TermAliasDecision::Confirmed`)
/// stored in `lessons`. Rejected pairs are never added.
pub fn expand_query_aliases(
    store: &LessonsStore,
    query_terms: &[&str],
) -> Result<Vec<String>, String> {
    let aliases = store.list_term_aliases()?;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for raw in query_terms {
        let norm = normalize_term(raw);
        if !norm.is_empty() && seen.insert(norm.clone()) {
            out.push(norm);
        }
    }
    let initial = out.clone();
    for term in &initial {
        for rec in &aliases {
            if rec.decision != TermAliasDecision::Confirmed {
                continue;
            }
            if rec.term_a == *term && seen.insert(rec.term_b.clone()) {
                out.push(rec.term_b.clone());
            } else if rec.term_b == *term && seen.insert(rec.term_a.clone()) {
                out.push(rec.term_a.clone());
            }
        }
    }
    Ok(out)
}

/// Returns `true` when the user previously rejected `(term_a, term_b)` as "same under another
/// name", so code will not propose that pair as the same again (`00-overview.md` 2.14).
pub fn is_pair_rejected(store: &LessonsStore, term_a: &str, term_b: &str) -> Result<bool, String> {
    let Ok((a, b)) = canonical_term_pair(term_a, term_b) else {
        return Ok(false);
    };
    let aliases = store.list_term_aliases()?;
    Ok(aliases.iter().any(|r| {
        r.decision == TermAliasDecision::Rejected && r.term_a == a && r.term_b == b
    }))
}

/// Editable Solve setup defaults suggested when matching symbols appear (`00-overview.md` 2.14).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedSolveDefaults {
    pub symbols: Vec<SymbolDefault>,
    pub conventions: Vec<ConventionDefault>,
}

/// Offer stored symbol assumptions/dimensions and conventions as defaults for the next Solve
/// setup when `symbols` appear. Domain-specific entries win over generic (`domain == ""`) ones.
pub fn suggest_solve_defaults(
    store: &LessonsStore,
    symbols: &[&str],
    domain: Option<&str>,
) -> Result<SuggestedSolveDefaults, String> {
    let want_domain = domain.map(|d| d.trim().to_lowercase()).unwrap_or_default();
    let want_syms: BTreeSet<String> = symbols
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let all_syms = store.list_symbol_defaults()?;
    let mut best_sym: BTreeMap<String, SymbolDefault> = BTreeMap::new();
    for s in all_syms {
        if !want_syms.contains(&s.symbol) {
            continue;
        }
        let domain_matches = s.domain.is_empty() || (!want_domain.is_empty() && s.domain == want_domain);
        if !domain_matches {
            continue;
        }
        let replace = match best_sym.get(&s.symbol) {
            None => true,
            Some(prev) => {
                let prev_specific = !prev.domain.is_empty();
                let cur_specific = !s.domain.is_empty();
                (cur_specific && !prev_specific)
                    || (cur_specific == prev_specific && s.updated_ms > prev.updated_ms)
            }
        };
        if replace {
            best_sym.insert(s.symbol.clone(), s);
        }
    }

    let all_convs = store.list_convention_defaults()?;
    let mut best_conv: BTreeMap<String, ConventionDefault> = BTreeMap::new();
    for c in all_convs {
        let domain_matches = c.domain.is_empty() || (!want_domain.is_empty() && c.domain == want_domain);
        if !domain_matches {
            continue;
        }
        let replace = match best_conv.get(&c.key) {
            None => true,
            Some(prev) => {
                let prev_specific = !prev.domain.is_empty();
                let cur_specific = !c.domain.is_empty();
                (cur_specific && !prev_specific)
                    || (cur_specific == prev_specific && c.updated_ms > prev.updated_ms)
            }
        };
        if replace {
            best_conv.insert(c.key.clone(), c);
        }
    }

    Ok(SuggestedSolveDefaults {
        symbols: best_sym.into_values().collect(),
        conventions: best_conv.into_values().collect(),
    })
}

/// Non-mutating display view of a skeleton node with any stored reader override attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkeletonNodeDisplay {
    pub node_tag: String,
    pub pure_kind: String,
    pub effective_kind: String,
    pub pure_end_line: usize,
    pub effective_end_line: usize,
    pub overridden_by_user: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_note: Option<String>,
}

/// Attach stored reader overrides to skeleton nodes for display without mutating the pure skeleton.
pub fn apply_skeleton_overrides(
    store: &LessonsStore,
    nodes: &[(String, String, usize)],
) -> Result<Vec<SkeletonNodeDisplay>, String> {
    let overrides: BTreeMap<String, SkeletonOverride> = store
        .list_skeleton_overrides()?
        .into_iter()
        .map(|o| (o.node_tag.clone(), o))
        .collect();

    Ok(nodes
        .iter()
        .map(|(tag, pure_kind, pure_end)| {
            if let Some(ov) = overrides.get(tag) {
                SkeletonNodeDisplay {
                    node_tag: tag.clone(),
                    pure_kind: pure_kind.clone(),
                    effective_kind: ov
                        .kind_override
                        .clone()
                        .unwrap_or_else(|| pure_kind.clone()),
                    pure_end_line: *pure_end,
                    effective_end_line: ov.end_line_override.unwrap_or(*pure_end),
                    overridden_by_user: true,
                    override_note: (!ov.note.is_empty()).then(|| ov.note.clone()),
                }
            } else {
                SkeletonNodeDisplay {
                    node_tag: tag.clone(),
                    pure_kind: pure_kind.clone(),
                    effective_kind: pure_kind.clone(),
                    pure_end_line: *pure_end,
                    effective_end_line: *pure_end,
                    overridden_by_user: false,
                    override_note: None,
                }
            }
        })
        .collect())
}

/// One display line of an OCR reading file with optional user correction overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrDisplayLine {
    pub line_no: usize,
    pub pinned_text: String,
    pub display_text: String,
    pub corrected_by_you: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Apply stored OCR line overlays for `file_sha256` to `pinned_lines` (1-based) for display.
/// The underlying `pinned_lines` are never mutated, and corrected lines carry `"corrected by you"`.
pub fn apply_ocr_overlays(
    store: &LessonsStore,
    file_sha256: &str,
    pinned_lines: &[&str],
) -> Result<Vec<OcrDisplayLine>, String> {
    let overlays: BTreeMap<usize, OcrOverlay> = store
        .list_ocr_overlays(Some(file_sha256))?
        .into_iter()
        .map(|o| (o.line_no, o))
        .collect();

    Ok(pinned_lines
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            let line_no = idx + 1;
            if let Some(ov) = overlays.get(&line_no) {
                OcrDisplayLine {
                    line_no,
                    pinned_text: (*line).to_string(),
                    display_text: ov.corrected_text.clone(),
                    corrected_by_you: true,
                    label: Some(OCR_OVERLAY_LABEL.to_string()),
                }
            } else {
                OcrDisplayLine {
                    line_no,
                    pinned_text: (*line).to_string(),
                    display_text: (*line).to_string(),
                    corrected_by_you: false,
                    label: None,
                }
            }
        })
        .collect())
}
