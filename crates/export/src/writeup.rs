//! Mode-neutral write-up document model, single wording table, Gate G7 enforcement, and
//! F4 `quote_core` quote filtering (package F15 core, `00-overview.md` 2.13).
//!
//! Pure deterministic code: zero model calls. Mode adapters (`writeup_solve.rs`,
//! `writeup_idea.rs`, `writeup_paper.rs`) live with the modes and construct [`Writeup`] values.

use ledger::Event;
use research_tools::paper_key::PaperRef;
use research_tools::quote_core::{quote_sha256, verify_quote_ref, QuoteFault, QuoteRef};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::solve_status::{wording as solve_wording, Tier};
use serde::{Deserialize, Serialize};

/// Single wording table for F15 writers (`00-overview.md` 2.13, 3.2 row 20 & 24, 8.1, 8.3 Q10-11).
///
/// Solve tier labels come directly from `research_tools::solve_status` (`solve_badge.rs`) —
/// never a stronger word: no `"proved"`, no `"new"`, no `"novel"`, and no `"verified"` for
/// anything below `Tier::Verified`.
pub mod writeup_wording {
    use research_tools::solve_status::{wording as solve_wording, Tier};

    /// Mandatory ending table heading (`00-overview.md` 2.13 rule 1, 8.3 question 10).
    pub const CHECK_TABLE_HEADING: &str = "How each statement was checked";
    /// Mandatory Explore coverage heading (`00-overview.md` 2.13 rule 1).
    pub const COVERAGE_HEADING: &str = "Where I looked";
    /// Bibliography section heading.
    pub const REFERENCES_HEADING: &str = "References";
    /// Status line label.
    pub const STATUS_LABEL: &str = "Status";
    /// Tag appended to every quote from a scanned/OCR page (`00-overview.md` 2.13 rule 6).
    pub const OCR_TAG: &str = "(OCR)";
    /// Suffix for model-authored prose (`00-overview.md` 3.2 row 24).
    pub const MODEL_SUFFIX: &str = "(model)";
    /// Notice printed when optional prose polish ran (`00-overview.md` 2.13 rule 5).
    pub const PROSE_EDITED_BY_MODEL: &str = "prose edited by a model";
    /// Maximum quote length allowed in write-ups (`00-overview.md` 2.5 & 2.13).
    pub const MAX_QUOTE_CHARS: usize = 300;

    /// Non-Solve checking labels (`00-overview.md` 2.10 & 2.13). None contain `"verified"`,
    /// `"proved"`, `"new"`, or `"novel"`.
    pub const LABEL_CODE_SKELETON: &str = "Found by code";
    pub const LABEL_MATCHED_FULL_TEXT: &str = "Matched quote (full text)";
    pub const LABEL_MATCHED_OCR: &str = "Matched quote (OCR)";
    pub const LABEL_MATCHED_ABSTRACT: &str = "Matched quote (abstract only)";
    pub const LABEL_MODEL_STATEMENT: &str = "Model statement (model)";
    pub const LABEL_USER_STATEMENT: &str = "User statement (you)";

    /// Exact Solve tier label forwarded directly from `solve_badge.rs`.
    pub fn solve_tier_label(tier: Tier) -> &'static str {
        tier.label()
    }

    /// Own-tier note emitted in the check table when `own == Verified` and `effective != Verified`.
    pub fn solve_own_verified_note(own: Option<Tier>, effective: Tier) -> Option<&'static str> {
        (own == Some(Tier::Verified) && effective != Tier::Verified)
            .then_some(solve_wording::OWN_CHECK_VERIFIED)
    }
}

/// Which mode produced a [`Writeup`] (governs mode-specific rules such as Solve Gate G7
/// and the `"verified"` word restriction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteupMode {
    Solve,
    Explore,
    Investigate,
}

/// Provenance of a prose block (`00-overview.md` 2.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProseBasis {
    App,
    User,
    Model,
}

/// How a statement in a write-up was checked (`00-overview.md` 2.10 & 2.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "tier", rename_all = "snake_case")]
pub enum HowChecked {
    /// Solve step checked at the given effective tier from `solve_badge.rs`.
    Solve(Tier),
    /// Deterministic structural fact extracted by code (Investigate skeleton / Explore coverage).
    CodeSkeleton,
    /// Full-text quote matched via `quote_core`.
    MatchedQuoteFullText,
    /// Scanned-page OCR quote matched via `quote_core` (`(OCR)`).
    MatchedQuoteOcr,
    /// Abstract-only verbatim quote (`xa` evidence).
    MatchedQuoteAbstract,
    /// Statement authored or judged by a model.
    ModelStatement,
    /// Statement supplied or checked by the user.
    UserStatement,
}

impl HowChecked {
    /// Single-table label used by every F15 writer.
    pub fn label(self) -> &'static str {
        match self {
            Self::Solve(tier) => writeup_wording::solve_tier_label(tier),
            Self::CodeSkeleton => writeup_wording::LABEL_CODE_SKELETON,
            Self::MatchedQuoteFullText => writeup_wording::LABEL_MATCHED_FULL_TEXT,
            Self::MatchedQuoteOcr => writeup_wording::LABEL_MATCHED_OCR,
            Self::MatchedQuoteAbstract => writeup_wording::LABEL_MATCHED_ABSTRACT,
            Self::ModelStatement => writeup_wording::LABEL_MODEL_STATEMENT,
            Self::UserStatement => writeup_wording::LABEL_USER_STATEMENT,
        }
    }

    pub fn is_failed_or_stale_solve(self) -> bool {
        matches!(self, Self::Solve(Tier::Failed | Tier::Stale))
    }

    pub fn is_verified_solve(self) -> bool {
        matches!(self, Self::Solve(Tier::Verified))
    }
}

/// Source location displayed beside a matched quote (`00-overview.md` 2.12 & 2.13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuoteLocator {
    Lines {
        start_line: usize,
        end_line: usize,
    },
    Page {
        page: usize,
        start_line: usize,
        end_line: usize,
    },
}

impl QuoteLocator {
    pub fn display_text(&self) -> String {
        match self {
            Self::Lines { start_line, end_line } => {
                if start_line == end_line {
                    format!("line {start_line}")
                } else {
                    format!("lines {start_line}-{end_line}")
                }
            }
            Self::Page {
                page,
                start_line,
                end_line,
            } => {
                if start_line == end_line {
                    format!("p. {page}, line {start_line}")
                } else {
                    format!("p. {page}, lines {start_line}-{end_line}")
                }
            }
        }
    }
}

/// A quote block that must pass `quote_core` (F4) before being written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteupQuote {
    pub text: String,
    pub cite_key: String,
    pub paper: PaperRef,
    pub quote_ref: QuoteRef,
    pub locator: QuoteLocator,
    pub ocr: bool,
}

/// Content blocks inside a [`WriteupSection`] (`00-overview.md` 2.13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "block_kind", rename_all = "snake_case")]
pub enum WriteupBlock {
    Prose {
        text: String,
        basis: ProseBasis,
    },
    Formula {
        /// LaTeX expression rendered by the check process from `handle`.
        latex: String,
        /// Handle reference (`name@version`) tying the written formula to the checked handle.
        handle: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Quote(Box<WriteupQuote>),
    /// Verified verbatim abstract quote (`xa-<seq>`, Rule C3; `00-overview.md` 2.9 & 2.13).
    /// Rendered with a real `\cite{cite_key}` in TeX (without `\begin{quote}`) and `[@cite_key]`
    /// in Markdown, strictly capped at `"abstract only"` and `"possible relevance"`.
    AbstractQuote {
        evidence_id: String,
        cite_key: String,
        provider_label: String,
        text: String,
    },
    Statement {
        id: String,
        text: String,
        how_checked: HowChecked,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        own_tier: Option<Tier>,
        #[serde(default)]
        rests_on: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

/// One titled section of a [`Writeup`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteupSection {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub blocks: Vec<WriteupBlock>,
}

/// One bibliography entry linked to a [`PaperRef`] and resolved citation key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibEntry {
    pub cite_key: String,
    pub paper: PaperRef,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_bibtex: Option<String>,
}

/// One row of the mandatory "How each statement was checked" table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckTableRow {
    pub id: String,
    pub statement: String,
    pub how_checked: HowChecked,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_tier: Option<Tier>,
    #[serde(default)]
    pub rests_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl CheckTableRow {
    /// Formatted badge text from the single wording table (never stronger than its tier).
    pub fn badge_text(&self) -> String {
        let base = self.how_checked.label();
        match self.how_checked {
            HowChecked::Solve(effective) => {
                if let Some(own_note) =
                    writeup_wording::solve_own_verified_note(self.own_tier, effective)
                {
                    format!("{base} ({own_note})")
                } else {
                    base.to_string()
                }
            }
            _ => base.to_string(),
        }
    }
}

/// Mode-neutral write-up document model (`00-overview.md` 2.13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Writeup {
    pub mode: WriteupMode,
    pub title: String,
    pub status_line: String,
    /// For Solve write-ups, the effective tier of the final answer step (`00-overview.md` 2.13 rule 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_effective_tier: Option<Tier>,
    /// Mandatory Explore coverage paragraph ("Where I looked"), written by code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_paragraph: Option<String>,
    /// True only if optional model prose polish was applied (`00-overview.md` 2.13 rule 5).
    #[serde(default)]
    pub prose_edited_by_model: bool,
    pub sections: Vec<WriteupSection>,
    #[serde(default)]
    pub bibliography: Vec<BibEntry>,
    #[serde(default)]
    pub check_table: Vec<CheckTableRow>,
}

/// Options accepted by both TeX and Markdown writers.
///
/// `include_inline_badges` may be disabled by the user; the status line, coverage paragraph,
/// and "How each statement was checked" table can **never** be turned off (`00-overview.md` 8.3 Q10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteupOptions {
    pub include_inline_badges: bool,
    pub bib_stem: String,
}

impl Default for WriteupOptions {
    fn default() -> Self {
        Self {
            include_inline_badges: true,
            bib_stem: "references".to_string(),
        }
    }
}

/// Record of a quote dropped before writing because `quote_core` did not match its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedWriteupQuote {
    pub section_title: String,
    pub cite_key: String,
    pub fault: QuoteFault,
}

impl Writeup {
    /// Ensure every `Statement` block appears in `check_table` in document order.
    pub fn derived_check_table(&self) -> Vec<CheckTableRow> {
        let mut rows = self.check_table.clone();
        for sec in &self.sections {
            for blk in &sec.blocks {
                if let WriteupBlock::Statement {
                    id,
                    text,
                    how_checked,
                    own_tier,
                    rests_on,
                    note,
                } = blk
                {
                    if !rows.iter().any(|r| r.id == *id) {
                        rows.push(CheckTableRow {
                            id: id.clone(),
                            statement: text.clone(),
                            how_checked: *how_checked,
                            own_tier: *own_tier,
                            rests_on: rests_on.clone(),
                            note: note.clone(),
                        });
                    }
                }
            }
        }
        rows
    }

    /// Enforce F15 structural & honesty invariants before writing:
    /// - Non-empty title and status line
    /// - Solve Gate G7 (`00-overview.md` 2.13 rule 2): refuses Failed or Stale steps
    /// - Non-Solve modes may not carry `HowChecked::Solve(Tier::Verified)`
    /// - Every quote satisfies the 300-char cap and carries a matching `quote_sha256`
    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("write-up title must not be empty".into());
        }
        if self.status_line.trim().is_empty() {
            return Err("write-up status_line must not be empty".into());
        }
        let rows = self.derived_check_table();
        if self.mode == WriteupMode::Solve {
            if let Some(bad) = rows.iter().find(|r| r.how_checked.is_failed_or_stale_solve()) {
                return Err(format!(
                    "Solve write-up refused (Gate G7): step `{}` is {}",
                    bad.id,
                    bad.how_checked.label()
                ));
            }
            if matches!(self.final_effective_tier, Some(Tier::Failed | Tier::Stale)) {
                return Err("Solve write-up refused (Gate G7): final answer is Failed or Stale".into());
            }
            if self.final_effective_tier != Some(Tier::Verified)
                && contains_ascii_word(&self.status_line.to_ascii_lowercase(), "verified")
            {
                return Err(
                    "Solve write-up status_line must not call the result \"verified\" unless final_effective_tier is Verified"
                        .into(),
                );
            }
        } else {
            if rows.iter().any(|r| r.how_checked.is_verified_solve()) {
                return Err(format!(
                    "{:?} write-up must not carry Solve Verified statements",
                    self.mode
                ));
            }
            if self.status_line.to_ascii_lowercase().contains("verified") {
                return Err(format!(
                    "{:?} write-up status_line must not contain the word \"verified\"",
                    self.mode
                ));
            }
        }

        // Enforce 300-char quote cap and quote_sha256 binding on every Quote block.
        for sec in &self.sections {
            for blk in &sec.blocks {
                if let WriteupBlock::Quote(q) = blk {
                    if q.text.chars().count() > writeup_wording::MAX_QUOTE_CHARS {
                        return Err(format!(
                            "quote for `{}` exceeds {} character limit",
                            q.cite_key,
                            writeup_wording::MAX_QUOTE_CHARS
                        ));
                    }
                    if quote_sha256(&q.text) != q.quote_ref.quote_sha256 {
                        return Err(format!(
                            "quote for `{}` does not match its QuoteRef sha256 (verify via quote_core first)",
                            q.cite_key
                        ));
                    }
                } else if let WriteupBlock::AbstractQuote {
                    cite_key, text, ..
                } = blk
                {
                    if text.chars().count() > writeup_wording::MAX_QUOTE_CHARS {
                        return Err(format!(
                            "abstract quote for `{cite_key}` exceeds {} character limit",
                            writeup_wording::MAX_QUOTE_CHARS
                        ));
                    }
                    if cite_key.trim().is_empty() {
                        return Err("abstract quote must carry a non-empty cite_key".into());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Verify a single quote through F4 [`verify_quote_ref`] and build a [`WriteupQuote`].
/// Rejects quotes over 300 characters or quotes that fail source matching.
#[allow(clippy::too_many_arguments, clippy::result_large_err)]
pub fn build_verified_quote(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    session_events: &[Event],
    cite_key: String,
    paper: PaperRef,
    expected_ref: &QuoteRef,
    quote_text: &str,
    locator: QuoteLocator,
    ocr: bool,
) -> Result<WriteupQuote, QuoteFault> {
    if quote_text.chars().count() > writeup_wording::MAX_QUOTE_CHARS {
        return Err(QuoteFault {
            code: "quote_too_long".into(),
            message: format!(
                "Quote exceeds {} characters.",
                writeup_wording::MAX_QUOTE_CHARS
            ),
            start_line: Some(expected_ref.start_line),
            end_line: Some(expected_ref.end_line),
            true_text: None,
        });
    }
    let verified = verify_quote_ref(sandbox, session_id, session_events, expected_ref, quote_text)?;
    Ok(WriteupQuote {
        text: verified.excerpt,
        cite_key,
        paper,
        quote_ref: verified.quote_ref,
        locator,
        ocr,
    })
}

/// Filter a [`Writeup`] through F4 [`verify_quote_ref`], dropping any quote block that fails
/// source matching or exceeds 300 characters so unmatched quotes are **never written**.
pub fn filter_verified_quotes(
    doc: &Writeup,
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    session_events: &[Event],
) -> (Writeup, Vec<RejectedWriteupQuote>) {
    let mut cleaned = doc.clone();
    let mut rejected = Vec::new();
    for sec in &mut cleaned.sections {
        let mut kept = Vec::with_capacity(sec.blocks.len());
        for blk in sec.blocks.drain(..) {
            match blk {
                WriteupBlock::Quote(q) => {
                    match build_verified_quote(
                        sandbox,
                        session_id,
                        session_events,
                        q.cite_key.clone(),
                        q.paper.clone(),
                        &q.quote_ref,
                        &q.text,
                        q.locator.clone(),
                        q.ocr,
                    ) {
                        Ok(vq) => kept.push(WriteupBlock::Quote(Box::new(vq))),
                        Err(fault) => rejected.push(RejectedWriteupQuote {
                            section_title: sec.title.clone(),
                            cite_key: q.cite_key,
                            fault,
                        }),
                    }
                }
                other => kept.push(other),
            }
        }
        sec.blocks = kept;
    }
    (cleaned, rejected)
}

/// Check that a non-Solve status/prose string or non-Verified badge never leaks banned words
/// (`"verified"`, `"proved"`, `"new"`, `"novel"`).
pub fn contains_forbidden_word(text: &str, allow_verified: bool) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    for word in ["proved", "new", "novel", "verified"] {
        if word == "verified" && allow_verified {
            continue;
        }
        if contains_ascii_word(&lower, word) {
            return Some(word);
        }
    }
    None
}

fn contains_ascii_word(haystack: &str, needle: &str) -> bool {
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() || nb.len() > hb.len() {
        return false;
    }
    for i in 0..=(hb.len() - nb.len()) {
        if &hb[i..i + nb.len()] == nb {
            let left_ok = i == 0 || !hb[i - 1].is_ascii_alphanumeric();
            let right_ok =
                i + nb.len() == hb.len() || !hb[i + nb.len()].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
        }
    }
    false
}

/// Return all non-verified wording constants from both `writeup_wording` and `solve_badge`.
pub fn all_non_verified_writeup_labels() -> Vec<&'static str> {
    let mut out = vec![
        writeup_wording::CHECK_TABLE_HEADING,
        writeup_wording::COVERAGE_HEADING,
        writeup_wording::REFERENCES_HEADING,
        writeup_wording::STATUS_LABEL,
        writeup_wording::OCR_TAG,
        writeup_wording::MODEL_SUFFIX,
        writeup_wording::PROSE_EDITED_BY_MODEL,
        writeup_wording::LABEL_CODE_SKELETON,
        writeup_wording::LABEL_MATCHED_FULL_TEXT,
        writeup_wording::LABEL_MATCHED_OCR,
        writeup_wording::LABEL_MATCHED_ABSTRACT,
        writeup_wording::LABEL_MODEL_STATEMENT,
        writeup_wording::LABEL_USER_STATEMENT,
    ];
    out.extend_from_slice(solve_wording::NON_VERIFIED_CONSTANTS);
    out
}
