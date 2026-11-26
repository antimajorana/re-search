//! Pure deterministic citation-locator matcher enforcing Rule 6.9 (Investigate package V11).
//!
//! Zero model calls and zero network calls. Enforces `investigate.md` rule 6.9 and
//! `00-overview.md` §7.3 (Evaluation Set P5: wrong statement shown as located MUST be 0):
//! - `Found` is emitted ONLY on non-PDF skeletons (`tex-1` or `html-1`) with a unique
//!   kind + number match whose `number_basis` is confirmed (`Confirmed`, `Html`, `Printed`,
//!   or `Computed` proven unique under a single shared counter with no counter drift).
//! - PDF skeletons (`pdf-1`) ALWAYS return `Candidates` (never `Found`) carrying physical
//!   page numbers derived on demand via `PageMap::page_of` / `page_of_line`, plus OCR flags.

use super::locate_parse::{build_ranked_candidates, collect_manual_candidates};
use super::model::{
    NumberBasis, PaperSkeleton, SkConfidence, SkNode, SkNodeKind, SkeletonBuilder, SkeletonQuality,
    Span,
};
use crate::paper_fetch::STATUS_NEEDS_FILE;
use crate::paper_key::PaperKeyKind;
use crate::pdf_pages::{PageBasis, PageMap};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use super::locate_parse::{detect_tex_counter_drift, format_candidate_label, parse_locator};

// Named status & provenance constants (`investigate.md` §6.9, `00-overview.md` §8 accepted 2026-11-25).
pub const CITED_STATUS_FOUND: &str = "found";
pub const CITED_STATUS_CANDIDATES: &str = "candidates";
pub const CITED_STATUS_LOCATOR_NOT_FOUND: &str = "paper_found_locator_not_found";
pub const CITED_STATUS_NO_TEX: &str = "no_tex";
pub const CITED_STATUS_UNRESOLVED: &str = "unresolved";
pub const NO_TEX_REASON_NEEDS_FILE: &str = STATUS_NEEDS_FILE;
pub const NO_TEX_REASON_EMPTY_TEXT_LAYER: &str = "empty_text_layer";
pub const PROVENANCE_OUTSIDE_PAPER: &str = "outside_paper";
pub const MAX_LOCATE_CANDIDATES: usize = 50;

/// Normalized representation of a citation locator such as `"Theorem 2.1"` or `"Eq. (3.4)"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedLocator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<SkNodeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    pub raw_locator: String,
    #[serde(default)]
    pub ambiguous: bool,
}

/// One selectable candidate statement in a cited paper skeleton.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateCandidate {
    pub tag: String,
    pub kind: SkNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_basis: Option<NumberBasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub label: String,
    pub spans: Vec<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_basis: Option<PageBasis>,
    #[serde(default)]
    pub ocr: bool,
    #[serde(default)]
    pub render_math: bool,
}

/// Five named deterministic cited-lookup outcomes (`investigate.md` §6.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocateStatus {
    Found {
        tag: String,
        number_basis: NumberBasis,
    },
    Candidates {
        candidates: Vec<LocateCandidate>,
        reason: String,
    },
    PaperFoundLocatorNotFound {
        candidates: Vec<LocateCandidate>,
        reason: String,
    },
    NoTex {
        reason: String,
    },
    Unresolved {
        reason: String,
    },
}

impl LocateStatus {
    pub fn status_str(&self) -> &'static str {
        match self {
            Self::Found { .. } => CITED_STATUS_FOUND,
            Self::Candidates { .. } => CITED_STATUS_CANDIDATES,
            Self::PaperFoundLocatorNotFound { .. } => CITED_STATUS_LOCATOR_NOT_FOUND,
            Self::NoTex { .. } => CITED_STATUS_NO_TEX,
            Self::Unresolved { .. } => CITED_STATUS_UNRESOLVED,
        }
    }
}

/// Full outcome returned by [`locate_in_skeleton`] / [`locate_in_skeleton_with_context`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateResult {
    pub parsed: ParsedLocator,
    #[serde(flatten)]
    pub outcome: LocateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_caveat: Option<String>,
}

/// Optional contextual data supplying PDF physical page lookup and TeX counter-drift signals.
#[derive(Debug, Clone, Default)]
pub struct LocateContext<'a> {
    pub page_map: Option<&'a PageMap>,
    pub page_lines: Option<&'a [(usize, usize, usize)]>,
    pub counter_drift_reason: Option<&'a str>,
    /// True when TeX preamble declarations were parsed and confirmed a single shared theorem counter.
    pub single_counter_confirmed: bool,
}

/// Formats the standard version numbering caveat (`investigate.md` §2 2:30).
pub fn format_version_caveat(skeleton: &PaperSkeleton) -> String {
    let ver = skeleton
        .version
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (skeleton.paper_key.kind() == PaperKeyKind::Arxiv, ver) {
        (true, Some(v)) => {
            format!("numbering taken from arXiv {v}; the journal version may differ")
        }
        (true, None) => {
            "numbering taken from arXiv source; the journal version may differ".into()
        }
        (false, Some(v)) => {
            format!("numbering taken from source {v}; the journal version may differ")
        }
        (false, None) => {
            "numbering taken from the available source; the journal version may differ".into()
        }
    }
}

/// Pure locator lookup using only `skeleton` (no page map or extra TeX scan context).
pub fn locate_in_skeleton(skeleton: &PaperSkeleton, locator: &str) -> LocateResult {
    locate_in_skeleton_with_context(skeleton, locator, &LocateContext::default())
}

/// Strict Rule 6.9 locator lookup with optional PDF page map / line ranges and TeX counter-drift context.
pub fn locate_in_skeleton_with_context(
    skeleton: &PaperSkeleton,
    locator: &str,
    ctx: &LocateContext<'_>,
) -> LocateResult {
    let parsed = parse_locator(locator);
    let version_caveat = Some(format_version_caveat(skeleton));

    if skeleton.builder == SkeletonBuilder::Pdf1
        && skeleton.nodes.is_empty()
        && ctx.page_map.is_some_and(|pm| {
            !pm.pages.is_empty() && pm.pages.iter().all(|p| p.basis == PageBasis::None)
        })
    {
        return LocateResult {
            parsed,
            outcome: LocateStatus::NoTex {
                reason: NO_TEX_REASON_EMPTY_TEXT_LAYER.into(),
            },
            version_caveat,
        };
    }

    let Some(target_num) = parsed.number.as_deref() else {
        let candidates = collect_manual_candidates(skeleton, parsed.kind, ctx);
        let reason =
            "citation locator did not name a numbered statement; pick from the paper below".into();
        return LocateResult {
            parsed,
            outcome: LocateStatus::PaperFoundLocatorNotFound { candidates, reason },
            version_caveat,
        };
    };

    let exact_matches: Vec<&SkNode> = skeleton
        .nodes
        .iter()
        .filter(|n| {
            parsed.kind == Some(n.kind)
                && n.number
                    .as_deref()
                    .is_some_and(|num| numbers_equal(num, target_num))
        })
        .collect();

    if exact_matches.len() == 1 {
        let matched = exact_matches[0];
        if parsed.ambiguous {
            let candidates =
                build_ranked_candidates(skeleton, &exact_matches, Some(matched.kind), ctx);
            return LocateResult {
                parsed,
                outcome: LocateStatus::Candidates {
                    candidates,
                    reason: "compound or indirect citation locator is ambiguous (rule 6.9); pick the intended statement".into(),
                },
                version_caveat,
            };
        }
        if skeleton.builder == SkeletonBuilder::Pdf1 {
            let candidates =
                build_ranked_candidates(skeleton, &exact_matches, Some(matched.kind), ctx);
            let reason = "PDF statement boundaries are estimated (rule 6.9); select the matching page and statement".into();
            return LocateResult {
                parsed,
                outcome: LocateStatus::Candidates { candidates, reason },
                version_caveat,
            };
        }
        return match confirm_match_basis(skeleton, matched, ctx) {
            Ok((tag, number_basis)) => LocateResult {
                parsed,
                outcome: LocateStatus::Found { tag, number_basis },
                version_caveat,
            },
            Err(reason) => LocateResult {
                parsed,
                outcome: LocateStatus::Candidates {
                    candidates: build_ranked_candidates(
                        skeleton,
                        &exact_matches,
                        Some(matched.kind),
                        ctx,
                    ),
                    reason,
                },
                version_caveat,
            },
        };
    }

    if exact_matches.len() > 1 {
        let candidates = build_ranked_candidates(skeleton, &exact_matches, parsed.kind, ctx);
        return LocateResult {
            parsed,
            outcome: LocateStatus::Candidates {
                candidates,
                reason: "multiple statements match this locator; pick the intended one".into(),
            },
            version_caveat,
        };
    }

    let number_matches: Vec<&SkNode> = skeleton
        .nodes
        .iter()
        .filter(|n| {
            n.number
                .as_deref()
                .is_some_and(|num| numbers_equal(num, target_num))
        })
        .collect();

    if !number_matches.is_empty() {
        let candidates = build_ranked_candidates(skeleton, &number_matches, parsed.kind, ctx);
        let reason = if parsed.kind.is_some() {
            "locator number matched a statement of a different kind; pick the intended statement"
        } else {
            "locator did not specify a statement kind; pick the matching statement"
        };
        return LocateResult {
            parsed,
            outcome: LocateStatus::Candidates {
                candidates,
                reason: reason.into(),
            },
            version_caveat,
        };
    }

    let candidates = collect_manual_candidates(skeleton, parsed.kind, ctx);
    let reason = format!(
        "no statement matching `{}` was found in this paper",
        locator.trim()
    );
    LocateResult {
        parsed,
        outcome: LocateStatus::PaperFoundLocatorNotFound { candidates, reason },
        version_caveat,
    }
}

fn numbers_equal(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn confirm_match_basis(
    skeleton: &PaperSkeleton,
    node: &SkNode,
    ctx: &LocateContext<'_>,
) -> Result<(String, NumberBasis), String> {
    let tag = node
        .tag
        .clone()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "matched statement has no stable tag".to_string())?;
    if node.confidence == SkConfidence::PdfHeuristic {
        return Err("statement was detected heuristically from PDF text".into());
    }
    if skeleton.quality == SkeletonQuality::Weak {
        return Err("skeleton quality is weak; pick the statement from candidates".into());
    }
    match node.number_basis {
        Some(b @ (NumberBasis::Confirmed | NumberBasis::Html)) => Ok((tag, b)),
        Some(NumberBasis::Printed) => {
            if skeleton.quality != SkeletonQuality::Full {
                return Err("skeleton quality is not full; pick the statement from candidates".into());
            }
            Ok((tag, NumberBasis::Printed))
        }
        Some(NumberBasis::Computed) => {
            confirm_computed_number(skeleton, node, ctx)?;
            Ok((tag, NumberBasis::Computed))
        }
        None => Err("matched statement has no confirmed number basis".into()),
    }
}

fn confirm_computed_number(
    skeleton: &PaperSkeleton,
    node: &SkNode,
    ctx: &LocateContext<'_>,
) -> Result<(), String> {
    if let Some(why) = ctx.counter_drift_reason {
        return Err(format!(
            "computed number unconfirmed ({why}); pick from candidates"
        ));
    }
    if skeleton.quality != SkeletonQuality::Full {
        return Err(
            "skeleton quality is not full; computed numbering may be shifted (pick from candidates)"
                .into(),
        );
    }
    if node.kind.is_theorem_like() {
        confirm_single_shared_theorem_counter(&skeleton.nodes, ctx.single_counter_confirmed)
    } else if node.kind == SkNodeKind::Equation {
        confirm_computed_equation_counter(&skeleton.nodes)
    } else {
        let label = if node.kind == SkNodeKind::Section {
            "section"
        } else {
            "statement"
        };
        confirm_unique_contiguous_series(
            skeleton.nodes.iter().filter(|n| n.kind == node.kind),
            label,
            node.kind == SkNodeKind::Section,
        )
    }
}

fn confirm_single_shared_theorem_counter(
    nodes: &[SkNode],
    preamble_confirmed: bool,
) -> Result<(), String> {
    let numbered: Vec<&SkNode> = nodes
        .iter()
        .filter(|n| n.kind.is_theorem_like() && n.number.is_some())
        .collect();
    if numbered.is_empty() {
        return Err("no numbered theorem-like nodes available to confirm counter".into());
    }
    let mut seen = BTreeSet::new();
    let mut expected_depth: Option<usize> = None;
    let mut by_prefix: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    let mut distinct_kinds: BTreeSet<(SkNodeKind, Option<&str>)> = BTreeSet::new();
    let mut prefix_kinds: BTreeMap<String, BTreeSet<(SkNodeKind, Option<&str>)>> = BTreeMap::new();

    for n in numbered {
        let num = n.number.as_deref().unwrap_or("").trim();
        if !seen.insert(num.to_ascii_uppercase()) {
            return Err("theorem-like environments use separate or colliding counters (not a single shared counter)".into());
        }
        if n.number_basis == Some(NumberBasis::Printed) {
            continue;
        }
        let depth = num.bytes().filter(|&b| b == b'.').count();
        if expected_depth.is_some_and(|prev| prev != depth) {
            return Err("theorem-like environments mix numbering depths (not a single shared counter)".into());
        }
        expected_depth = Some(depth);
        let (prefix, seq) = split_counter_number(num)
            .ok_or_else(|| format!("computed theorem number `{num}` uses non-standard formatting"))?;
        let key = (n.kind, n.env_name.as_deref());
        distinct_kinds.insert(key);
        prefix_kinds.entry(prefix.clone()).or_default().insert(key);
        by_prefix.entry(prefix).or_default().push(seq);
    }
    if distinct_kinds.len() > 1
        && !preamble_confirmed
        && !prefix_kinds.values().any(|ks| ks.len() > 1)
    {
        return Err(
            "multiple theorem-like environments appear only in disjoint sections; single shared counter is unconfirmed"
                .into(),
        );
    }
    check_contiguous(by_prefix, "theorem-like")
}

fn confirm_computed_equation_counter(nodes: &[SkNode]) -> Result<(), String> {
    if nodes.iter().any(|n| n.eq_count > 0) {
        return Err(
            "paper contains unlabelled display equations; computed equation numbers may be off"
                .into(),
        );
    }
    let eqs: Vec<&SkNode> = nodes
        .iter()
        .filter(|n| n.kind == SkNodeKind::Equation && n.number.is_some())
        .collect();
    for eq in &eqs {
        let env = eq.env_name.as_deref().unwrap_or("").to_ascii_lowercase();
        if matches!(
            env.as_str(),
            "align" | "gather" | "eqnarray" | "multline" | "flalign" | "alignat"
        ) {
            return Err(format!(
                "multi-line math environment `{env}` can shift computed equation numbers"
            ));
        }
    }
    confirm_unique_contiguous_series(eqs.into_iter(), "equation", false)
}

fn confirm_unique_contiguous_series<'a>(
    iter: impl Iterator<Item = &'a SkNode>,
    label: &str,
    allow_alpha_top: bool,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut by_prefix: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for n in iter {
        let Some(num) = n
            .number
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !seen.insert(num.to_ascii_uppercase()) {
            return Err(format!("duplicate computed {label} number `{num}`"));
        }
        if n.number_basis == Some(NumberBasis::Printed) {
            continue;
        }
        if let Some((prefix, seq)) = split_counter_number(num) {
            by_prefix.entry(prefix).or_default().push(seq);
        } else if allow_alpha_top
            && num.len() == 1
            && num.as_bytes()[0].is_ascii_uppercase()
        {
            let seq = u32::from(num.as_bytes()[0] - b'A') + 1;
            by_prefix.entry("__ALPHA__".to_string()).or_default().push(seq);
        } else {
            return Err(format!(
                "computed {label} number `{num}` uses non-numeric formatting"
            ));
        }
    }
    check_contiguous(by_prefix, label)
}

fn check_contiguous(by_prefix: BTreeMap<String, Vec<u32>>, label: &str) -> Result<(), String> {
    for (prefix, seqs) in by_prefix {
        for (idx, seq) in seqs.iter().enumerate() {
            let expected = (idx as u32) + 1;
            if *seq != expected {
                return Err(format!(
                    "non-contiguous computed {label} numbering in scope `{prefix}` (expected {expected}, got {seq})"
                ));
            }
        }
    }
    Ok(())
}

fn split_counter_number(num: &str) -> Option<(String, u32)> {
    match num.rsplit_once('.') {
        Some((prefix, tail)) => tail
            .parse::<u32>()
            .ok()
            .map(|n| (prefix.to_ascii_uppercase(), n)),
        None => num.parse::<u32>().ok().map(|n| (String::new(), n)),
    }
}

#[cfg(test)]
#[path = "locate_tests.rs"]
mod tests;
