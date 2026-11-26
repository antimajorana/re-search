//! Pure authors'-side status badge builder and deterministic cue-word detector
//! (package V7, `investigate.md` §3, §4.3, rule 6.6; `00-overview.md` §0.1, §3.2 rows 20 & 24).
//!
//! Computes the left half-badge (`Authors: New / Restated / Background / Not stated (model)`),
//! deterministic cue phrases found in matched quoted lines, and heading-citation wording
//! (`cites [<key>] in its heading`). Never emits the word "verified" and never emits
//! "attributed by the authors to".

use crate::map_defs::{AuthorsClaimLabel, AuthorsClaimRecord, VerifiedMapAnchor};
use crate::quote_core::clip_quote;
use serde::{Deserialize, Serialize};

/// Suffix required on all non-verdict model statements (`00-overview.md` §3.2 row 24, §8).
pub const MODEL_TEXT_SUFFIX: &str = "(model)";
/// Deterministic cue line shown when no cue phrase occurs in the quoted lines (`investigate.md` rule 6.6).
pub const NO_EXPLICIT_CUE_TEXT: &str = "no explicit cue in the quoted lines";
/// Prefix used when one or more cue phrases occur in the quoted lines (`investigate.md` rule 6.6).
pub const CUES_FOUND_PREFIX: &str = "Cues in quoted lines: ";
/// Border style for model-extracted badges (`00-overview.md` §2.6, §8.2 decision 4).
pub const BORDER_DASHED: &str = "dashed";
/// Border style for deterministic code-extracted badges (`00-overview.md` §2.6, §8.2 decision 4).
pub const BORDER_SOLID: &str = "solid";

/// Display text for each authors'-claim label (`investigate.md` §3, §13 decision 2; `00-overview.md` §3.2 row 24).
pub const AUTHORS_LABEL_NEW: &str = "Authors: New (model)";
pub const AUTHORS_LABEL_RESTATED: &str = "Authors: Restated (model)";
pub const AUTHORS_LABEL_BACKGROUND: &str = "Authors: Background (model)";
pub const AUTHORS_LABEL_NOT_STATED: &str = "Authors: Not stated (model)";

/// Canonical deterministic cue phrases scanned against quoted lines (`investigate.md` rule 6.6).
///
/// Ordered from more specific phrases to shorter ones so overlapping shorter cues
/// (e.g., `"known"` after `"well-known"`, or `"restated"` after `"restated from"`) are not duplicated.
pub const AUTHORS_CLAIM_CUE_PHRASES: &[&str] = &[
    // Novelty / main contribution cues
    "here we prove",
    "here we show",
    "in this paper we",
    "in the present paper",
    "we prove",
    "we show",
    "we establish",
    "we demonstrate",
    "we obtain",
    "we derive",
    "we construct",
    "we introduce",
    "we extend",
    "our main",
    "main theorem",
    "main result",
    "new result",
    "new bound",
    "new proof",
    "first time",
    "generalizes",
    "generalises",
    // Restatement / attribution cues
    "recalled from",
    "recall from",
    "we recall",
    "restated from",
    "restated",
    "was proved in",
    "was shown in",
    "proved in",
    "shown in",
    "established in",
    "adapted from",
    "taken from",
    "due to",
    "following",
    "see also",
    // Background / known cues
    "it is known",
    "well-known",
    "well known",
    "known",
    "classical",
    "standard result",
    "standard fact",
    "background",
    "see, e.g.",
    "see e.g.",
    "for a review",
];

/// Clipped, display-safe quote metadata attached to an [`AuthorsStatusBadge`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeQuote {
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub quote: String,
}

/// Pure authors'-side half-badge projection derived from an optional [`AuthorsClaimRecord`]
/// and deterministic skeleton `head_cites`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorsStatusBadge {
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<AuthorsClaimLabel>,
    pub text: String,
    pub icon: String,
    pub tone: String,
    pub border: String,
    #[serde(default)]
    pub cues: Vec<String>,
    pub cue_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_cite_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributed_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quotes: Vec<BadgeQuote>,
}

/// Canonical display label for an [`AuthorsClaimLabel`] (`investigate.md` §3, §13 decision 2).
pub fn authors_claim_display_text(label: AuthorsClaimLabel) -> &'static str {
    match label {
        AuthorsClaimLabel::New => AUTHORS_LABEL_NEW,
        AuthorsClaimLabel::Restated => AUTHORS_LABEL_RESTATED,
        AuthorsClaimLabel::Background => AUTHORS_LABEL_BACKGROUND,
        AuthorsClaimLabel::NotStated => AUTHORS_LABEL_NOT_STATED,
    }
}

/// Canonical F6 badge tone for an [`AuthorsClaimLabel`].
pub fn authors_claim_tone(label: AuthorsClaimLabel) -> &'static str {
    match label {
        AuthorsClaimLabel::New => "info",
        AuthorsClaimLabel::Restated => "neutral",
        AuthorsClaimLabel::Background | AuthorsClaimLabel::NotStated => "muted",
    }
}

/// Formats heading citations found deterministically in a theorem-like environment heading
/// (`investigate.md` §0.2 #2, §3). Always writes `"cites [<keys>] in its heading"`, never
/// `"attributed by the authors to"`.
pub fn format_heading_cites(head_cites: &[String]) -> Option<String> {
    let mut keys: Vec<&str> = Vec::new();
    for raw in head_cites {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && !keys.contains(&trimmed) {
            keys.push(trimmed);
        }
    }
    if keys.is_empty() {
        None
    } else {
        Some(format!("cites [{}] in its heading", keys.join(", ")))
    }
}

/// Formats the deterministic cue summary line for UI display (`investigate.md` rule 6.6).
pub fn format_cue_line(cues: &[String]) -> String {
    let cleaned: Vec<&str> = cues
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if cleaned.is_empty() {
        NO_EXPLICIT_CUE_TEXT.to_string()
    } else {
        format!("{CUES_FOUND_PREFIX}{}", cleaned.join(", "))
    }
}

/// Normalises text for deterministic cue-phrase scanning (ASCII lowercase, TeX `~` and whitespace
/// collapsed to single spaces).
fn normalize_for_cues(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_space = true;
    for ch in raw.chars() {
        if ch.is_whitespace() || ch == '~' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            for lc in ch.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Returns `true` iff `phrase` occurs in `normalized_haystack` on ASCII alphanumeric boundaries.
fn contains_cue_phrase(haystack: &str, phrase: &str) -> bool {
    let bytes = haystack.as_bytes();
    let p_len = phrase.len();
    if p_len == 0 || p_len > bytes.len() {
        return false;
    }
    let mut offset = 0usize;
    while let Some(rel) = haystack[offset..].find(phrase) {
        let start = offset + rel;
        let end = start + p_len;
        let left_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        offset = start + 1;
        while offset < bytes.len() && !haystack.is_char_boundary(offset) {
            offset += 1;
        }
    }
    false
}

/// Returns `true` when `phrase` is strictly subsumed by an already-recorded more specific cue
/// (e.g., `"known"` when `"well-known"` or `"it is known"` already matched).
fn is_subsumed_by_recorded(recorded: &[String], phrase: &str) -> bool {
    recorded.iter().any(|existing| {
        existing.len() > phrase.len() && contains_cue_phrase(existing, phrase)
    })
}

/// Deterministically detects cue phrases present in the given quoted text snippets
/// (`investigate.md` rule 6.6). Returns an empty `Vec` when no cue phrase occurs.
pub fn detect_authors_claim_cues(texts: &[&str]) -> Vec<String> {
    let normalized: Vec<String> = texts
        .iter()
        .map(|t| normalize_for_cues(t))
        .filter(|s| !s.is_empty())
        .collect();
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for &phrase in AUTHORS_CLAIM_CUE_PHRASES {
        if is_subsumed_by_recorded(&found, phrase) {
            continue;
        }
        if normalized.iter().any(|h| contains_cue_phrase(h, phrase)) {
            found.push(phrase.to_string());
        }
    }
    found
}

/// Deterministically detects cue phrases across the matched lines (`quote` and `excerpt`)
/// of `anchors` (`investigate.md` rule 6.6).
pub fn detect_cues_from_anchors(anchors: &[VerifiedMapAnchor]) -> Vec<String> {
    let mut texts: Vec<&str> = Vec::with_capacity(anchors.len() * 2);
    for anchor in anchors {
        if !anchor.quote.trim().is_empty() {
            texts.push(anchor.quote.as_str());
        }
        if !anchor.excerpt.trim().is_empty() {
            texts.push(anchor.excerpt.as_str());
        }
    }
    detect_authors_claim_cues(&texts)
}

/// Pure authors'-side status badge builder (`investigate.md` §4.3, rule 6.6).
///
/// Combines an optional model-extracted [`AuthorsClaimRecord`] with deterministic skeleton
/// `head_cites`. Never computes or merges prior-work verdicts, never emits `"verified"`, and
/// never uses `"attributed by the authors to"`.
pub fn status_badge(
    authors_claim: Option<&AuthorsClaimRecord>,
    head_cites: &[String],
) -> AuthorsStatusBadge {
    let heading_cite_line = format_heading_cites(head_cites);
    match authors_claim {
        Some(ac) => {
            let cues = if ac.cues.is_empty() {
                detect_cues_from_anchors(&ac.anchors)
            } else {
                ac.cues.clone()
            };
            let cue_line = format_cue_line(&cues);
            let quotes = ac
                .anchors
                .iter()
                .map(|a| BadgeQuote {
                    source_id: a.source_id.clone(),
                    path: a.path.clone(),
                    start_line: a.start_line,
                    end_line: a.end_line,
                    quote: clip_quote(a.quote.trim()).to_string(),
                })
                .collect();
            AuthorsStatusBadge {
                present: true,
                label: Some(ac.label),
                text: authors_claim_display_text(ac.label).to_string(),
                icon: "model".to_string(),
                tone: authors_claim_tone(ac.label).to_string(),
                border: BORDER_DASHED.to_string(),
                cues,
                cue_line,
                heading_cite_line,
                relation_note: ac
                    .relation_note
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                attributed_to: ac.attributed_to.clone(),
                quotes,
            }
        }
        None => AuthorsStatusBadge {
            present: false,
            label: None,
            text: String::new(),
            icon: "code".to_string(),
            tone: "neutral".to_string(),
            border: BORDER_SOLID.to_string(),
            cues: Vec::new(),
            cue_line: NO_EXPLICIT_CUE_TEXT.to_string(),
            heading_cite_line,
            relation_note: None,
            attributed_to: Vec::new(),
            quotes: Vec::new(),
        },
    }
}

#[cfg(test)]
#[path = "status_badge_tests.rs"]
mod tests;
