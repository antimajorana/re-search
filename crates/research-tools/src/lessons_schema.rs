//! Typed schema and constants for F16a corrections reused by code (`00-overview.md` 2.14).
//!
//! All user-visible strings and event kinds tied to `00-overview.md` section 8.2 decision 5
//! and section 8.3 decision 7 are kept in named constants here so they can change in one place.

use crate::paper_key::{NodeRef, PaperKey};
use ledger::lessons_db::{
    ConventionDefaultRow, OcrOverlayRow, SkeletonOverrideRow, SymbolDefaultRow, TermAliasRow,
};
pub use ledger::{LESSONS_DB_NAME, LESSONS_SCHEMA_VERSION};
use serde::{Deserialize, Serialize};

/// Portable JSON backup format version for `lessons.db` (`00-overview.md` section 8.2 decision 5).
pub const LESSONS_BACKUP_SCHEMA_VERSION: u32 = 1;

/// Display label shown on OCR line overlays (`00-overview.md` 2.14).
pub const OCR_OVERLAY_LABEL: &str = "corrected by you";

/// Term alias decision strings (`00-overview.md` 2.14).
pub const TERM_DECISION_CONFIRMED: &str = "confirmed";
pub const TERM_DECISION_REJECTED: &str = "rejected";

/// Existing event kinds that F16a reads (F16a adds no new way to correct; `00-overview.md` 2.14).
pub const EVENT_IDEA_VERDICT_CONFIRMED: &str = "idea.verdict_confirmed";
pub const EVENT_IDEA_ALIAS_CONFIRMED: &str = "idea.alias_confirmed";
pub const EVENT_IDEA_ALIAS_REJECTED: &str = "idea.alias_rejected";
pub const EVENT_IDEA_CORRECTION: &str = "idea.correction";
pub const EVENT_SOLVE_SETUP: &str = "solve.setup";
pub const EVENT_SOLVE_SETUP_REVISED: &str = "solve.setup_revised";
pub const EVENT_SESSION_CONVENTIONS: &str = "session.conventions";
pub const EVENT_INVESTIGATION_NODE_OVERRIDE: &str = "investigation.node_override";
pub const EVENT_INVESTIGATION_PIN: &str = "investigation.pin";
pub const EVENT_OCR_LINE_CORRECTED: &str = "ocr.line_corrected";
pub const EVENT_PAPER_OCR_CORRECTED: &str = "paper.ocr_corrected";

/// Ranking score adjustments for library user marks (`explore.md` decision 11, `00-overview.md` 2.14).
/// Marks change only rankings and seeds — never verdicts, tiers, or gates.
pub const KEY_PAPER_RANK_BOOST: i32 = 250;
pub const IRRELEVANT_PAPER_RANK_PENALTY: i32 = -500;

/// Maximum backup payload size (16 MiB).
pub const MAX_LESSONS_BACKUP_BYTES: usize = 16 * 1024 * 1024;

/// Whether a pair of names ("same under another name") was confirmed or rejected by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermAliasDecision {
    Confirmed,
    Rejected,
}

impl TermAliasDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => TERM_DECISION_CONFIRMED,
            Self::Rejected => TERM_DECISION_REJECTED,
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            TERM_DECISION_CONFIRMED => Ok(Self::Confirmed),
            TERM_DECISION_REJECTED => Ok(Self::Rejected),
            other => Err(format!(
                "invalid term alias decision `{other}` (expected `confirmed` or `rejected`)"
            )),
        }
    }
}

/// Normalize and canonically order a pair of terms so `(A, B)` and `(B, A)` share one key.
pub fn canonical_term_pair(a: &str, b: &str) -> Result<(String, String), String> {
    let na = normalize_term(a);
    let nb = normalize_term(b);
    if na.is_empty() || nb.is_empty() {
        return Err("both terms in an alias pair must be non-empty".into());
    }
    if na == nb {
        return Err("an alias pair must name two distinct terms".into());
    }
    if na <= nb {
        Ok((na, nb))
    } else {
        Ok((nb, na))
    }
}

pub fn normalize_term(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Confirmed or rejected "same under another name" pair (`00-overview.md` 2.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermAliasRecord {
    pub term_a: String,
    pub term_b: String,
    pub decision: TermAliasDecision,
    #[serde(default)]
    pub origin_session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_node: Option<NodeRef>,
    pub updated_ms: i64,
}

impl TermAliasRecord {
    pub(crate) fn to_row(&self) -> Result<TermAliasRow, String> {
        let (a, b) = canonical_term_pair(&self.term_a, &self.term_b)?;
        let origin_node_json = match &self.origin_node {
            Some(node) => serde_json::to_string(node).map_err(|e| e.to_string())?,
            None => "null".to_string(),
        };
        Ok(TermAliasRow {
            term_a: a,
            term_b: b,
            decision: self.decision.as_str().to_string(),
            origin_session: self.origin_session.clone(),
            origin_node_json,
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(r: TermAliasRow) -> Result<Self, String> {
        let origin_node = if r.origin_node_json == "null" {
            None
        } else {
            serde_json::from_str(&r.origin_node_json).ok()
        };
        Ok(Self {
            term_a: r.term_a,
            term_b: r.term_b,
            decision: TermAliasDecision::parse(&r.decision)?,
            origin_session: r.origin_session,
            origin_node,
            updated_ms: r.updated_ms,
        })
    }
}

/// Reusable Solve setup symbol assumption & dimension default (`00-overview.md` 2.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDefault {
    pub symbol: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convention_note: Option<String>,
    #[serde(default)]
    pub origin_session: String,
    pub updated_ms: i64,
}

impl SymbolDefault {
    pub(crate) fn to_row(&self) -> Result<SymbolDefaultRow, String> {
        let sym = self.symbol.trim();
        if sym.is_empty() {
            return Err("symbol name must not be empty".into());
        }
        Ok(SymbolDefaultRow {
            symbol: sym.to_string(),
            domain: self.domain.trim().to_lowercase(),
            assumptions_json: serde_json::to_string(&self.assumptions).map_err(|e| e.to_string())?,
            dimensions: self.dimensions.clone(),
            convention_note: self.convention_note.clone(),
            origin_session: self.origin_session.clone(),
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(r: SymbolDefaultRow) -> Result<Self, String> {
        Ok(Self {
            symbol: r.symbol,
            domain: r.domain,
            assumptions: serde_json::from_str(&r.assumptions_json).unwrap_or_default(),
            dimensions: r.dimensions,
            convention_note: r.convention_note,
            origin_session: r.origin_session,
            updated_ms: r.updated_ms,
        })
    }
}

/// Reusable Solve convention default (`00-overview.md` 2.14), e.g. `metric_signature = (-,+,+,+)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConventionDefault {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub origin_session: String,
    pub updated_ms: i64,
}

impl ConventionDefault {
    pub(crate) fn to_row(&self) -> Result<ConventionDefaultRow, String> {
        let key = self.key.trim();
        if key.is_empty() {
            return Err("convention key must not be empty".into());
        }
        Ok(ConventionDefaultRow {
            conv_key: key.to_string(),
            conv_value: self.value.trim().to_string(),
            domain: self.domain.trim().to_lowercase(),
            origin_session: self.origin_session.clone(),
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(r: ConventionDefaultRow) -> Self {
        Self {
            key: r.conv_key,
            value: r.conv_value,
            domain: r.domain,
            origin_session: r.origin_session,
            updated_ms: r.updated_ms,
        }
    }
}

/// Reader override keyed by permanent Investigate node tag (`n-...`).
/// The underlying skeleton remains a pure function of the paper files (`00-overview.md` 2.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkeletonOverride {
    pub node_tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line_override: Option<usize>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub origin_session: String,
    pub updated_ms: i64,
}

impl SkeletonOverride {
    pub(crate) fn to_row(&self) -> Result<SkeletonOverrideRow, String> {
        let tag = self.node_tag.trim();
        if tag.is_empty() {
            return Err("node_tag must not be empty".into());
        }
        if self.end_line_override == Some(0) {
            return Err("end_line_override must be 1-based (>= 1)".into());
        }
        Ok(SkeletonOverrideRow {
            node_tag: tag.to_string(),
            paper_key: self.paper_key.as_ref().map(ToString::to_string),
            kind_override: self.kind_override.clone(),
            end_line_override: self.end_line_override.map(|v| v as i64),
            note: self.note.clone(),
            origin_session: self.origin_session.clone(),
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(r: SkeletonOverrideRow) -> Result<Self, String> {
        let paper_key = r.paper_key.as_deref().map(PaperKey::parse).transpose()?;
        Ok(Self {
            node_tag: r.node_tag,
            paper_key,
            kind_override: r.kind_override,
            end_line_override: r.end_line_override.map(|v| v as usize),
            note: r.note,
            origin_session: r.origin_session,
            updated_ms: r.updated_ms,
        })
    }
}

/// Display overlay for a reader-corrected OCR line (`00-overview.md` 2.14).
/// Pinned text in the ledger and reading file is never mutated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrOverlay {
    pub file_sha256: String,
    pub line_no: usize,
    pub corrected_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_sha256: Option<String>,
    #[serde(default)]
    pub origin_session: String,
    pub updated_ms: i64,
    #[serde(default = "default_ocr_overlay_label")]
    pub label: String,
}

fn default_ocr_overlay_label() -> String {
    OCR_OVERLAY_LABEL.to_string()
}

impl OcrOverlay {
    pub(crate) fn to_row(&self) -> Result<OcrOverlayRow, String> {
        if self.line_no == 0 {
            return Err("line_no must be 1-based (>= 1)".into());
        }
        let sha = self.file_sha256.trim().to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!("invalid file_sha256 `{}`", self.file_sha256));
        }
        Ok(OcrOverlayRow {
            file_sha256: sha,
            line_no: self.line_no as i64,
            corrected_text: self.corrected_text.clone(),
            original_sha256: self.original_sha256.as_ref().map(|s| s.trim().to_ascii_lowercase()),
            origin_session: self.origin_session.clone(),
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(r: OcrOverlayRow) -> Self {
        Self {
            file_sha256: r.file_sha256,
            line_no: r.line_no as usize,
            corrected_text: r.corrected_text,
            original_sha256: r.original_sha256,
            origin_session: r.origin_session,
            updated_ms: r.updated_ms,
            label: OCR_OVERLAY_LABEL.to_string(),
        }
    }
}

/// Summary returned after scanning a session's events for existing correction events.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestCorrectionsReport {
    pub session_id: String,
    pub events_scanned: usize,
    pub term_aliases_recorded: usize,
    pub symbol_defaults_recorded: usize,
    pub convention_defaults_recorded: usize,
    pub skeleton_overrides_recorded: usize,
    pub ocr_overlays_recorded: usize,
}

/// Portable, SHA-256-checksummed backup payload for `lessons.db` (`00-overview.md` 2.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonsBackup {
    pub schema_version: u32,
    pub exported_ms: i64,
    pub sha256: String,
    pub term_aliases: Vec<TermAliasRecord>,
    pub symbol_defaults: Vec<SymbolDefault>,
    pub convention_defaults: Vec<ConventionDefault>,
    pub skeleton_overrides: Vec<SkeletonOverride>,
    pub ocr_overlays: Vec<OcrOverlay>,
}
