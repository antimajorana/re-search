//! Shared hand-off packet types and validation (package F7, `00-overview.md` section 4).
//!
//! One packet schema (`handoff.packet` v1) with three target-specific `ask` payloads
//! (`solve`, `explore`, `investigate`). Every input struct uses `#[serde(deny_unknown_fields)]`
//! so badges, tiers, and verdicts can never travel across a hand-over (Rule 2).

use research_tools::paper_key::{Mode, NodeRef, PaperRef};
use research_tools::quote_core::QuoteRef;
use research_tools::solve_model::{DoneCriterion, SolveTarget, SymbolSpec, TaskKind};
use research_tools::tool_profile::{
    MARKER_IDEA_STARTED, MARKER_INVESTIGATION_STARTED, MARKER_SOLVE_STARTED,
};
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Shared event kind written into the child session (`00-overview.md` 4.1, 8.2 decision 10).
pub const HANDOFF_PACKET_EVENT: &str = "handoff.packet";
/// Shared event kind written into the origin session (`00-overview.md` 4.1, 8.2 decision 10).
pub const HANDOFF_SENT_EVENT: &str = "handoff.sent";
/// Schema version of `handoff.packet` and `handoff.sent` events.
pub const HANDOFF_SCHEMA_VERSION: u32 = 1;
/// Suffix required on model-authored text carried in a hand-off (`00-overview.md` 3.2 row 24, 4.1).
pub const MODEL_TEXT_SUFFIX: &str = "(model)";

pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_STATEMENT_CHARS: usize = 16_000;
pub const MAX_CONTEXT_ITEMS: usize = 128;
pub const MAX_CONTEXT_TEXT_CHARS: usize = 8_000;
pub const MAX_CONVENTIONS_CHARS: usize = 2_000;
pub const MAX_MODEL_TEXT_ITEMS: usize = 64;
pub const MAX_MODEL_TEXT_LABEL_CHARS: usize = 120;
pub const MAX_MODEL_TEXT_CHARS: usize = 4_000;
pub const MAX_USER_NOTES: usize = 64;
pub const MAX_USER_NOTE_CHARS: usize = 4_000;
pub const MAX_PAPERS: usize = 64;
pub const MAX_ASK_ITEMS: usize = 64;

/// Role of one contextual item carried in `handoff.packet.context` (`00-overview.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRole {
    Definition,
    Assumption,
    Uses,
    KnownResult,
    Convention,
}

/// Verbatim statement text and its quote pointers (`00-overview.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffStatement {
    pub text: String,
    #[serde(default)]
    pub quotes: Vec<QuoteRef>,
}

/// One supporting definition, assumption, used equation, known result, or convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffContextItem {
    pub role: ContextRole,
    pub from: NodeRef,
    pub text: String,
    #[serde(default)]
    pub quotes: Vec<QuoteRef>,
}

/// Model-authored prose carried as untrusted data (gists, summaries, direction prose).
/// Always displayed with the `"(model)"` suffix (`00-overview.md` 3.2 row 24, 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelTextEntry {
    pub label: String,
    pub text: String,
}

impl ModelTextEntry {
    /// Format `label` ensuring it carries the `"(model)"` marker once.
    pub fn display_label(&self) -> String {
        let trimmed = self.label.trim();
        if trimmed.ends_with(MODEL_TEXT_SUFFIX) {
            trimmed.to_string()
        } else {
            format!("{trimmed} {MODEL_TEXT_SUFFIX}")
        }
    }
}

/// Target-specific Solve hand-off payload using Solve's exact field names (`00-overview.md` 3.1 row 9, 4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveHandoffAsk {
    pub task_kind: TaskKind,
    pub target: SolveTarget,
    #[serde(default)]
    pub symbols: Vec<SymbolSpec>,
    #[serde(default)]
    pub suggested_done: Vec<DoneCriterion>,
    #[serde(default)]
    pub check_first: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub stop_if: Vec<String>,
}

/// Which question Explore is being asked (`00-overview.md` 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExploreQuestionKind {
    /// "Has this been extended?"
    Extended,
    /// "Search wider in Explore" (is this result earlier? — replaces deferred V14).
    PriorWork,
}

/// Target-specific Explore hand-off payload (`00-overview.md` 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExploreHandoffAsk {
    pub idea_text: String,
    pub question_kind: ExploreQuestionKind,
    /// Origin paper to exclude from `"found"` verdicts in Explore (`00-overview.md` 4.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_paper: Option<PaperRef>,
    /// Bibliography entries from the origin paper, marked `"cited by the origin"` in Explore.
    #[serde(default)]
    pub seed_papers: Vec<PaperRef>,
    #[serde(default)]
    pub focus_claims: Vec<String>,
}

/// Target-specific Investigate hand-off payload (`00-overview.md` 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigateHandoffAsk {
    pub paper: PaperRef,
    #[serde(default)]
    pub focus_claims: Vec<String>,
    /// Quotes matched in the origin session; re-matched in the child against its own receipt.
    #[serde(default)]
    pub earlier_quotes: Vec<QuoteRef>,
}

/// Target-specific `ask: { kind, fields }` union (`00-overview.md` 4.1, 4.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "fields", rename_all = "snake_case")]
pub enum HandoffAsk {
    Solve(SolveHandoffAsk),
    Explore(ExploreHandoffAsk),
    Investigate(InvestigateHandoffAsk),
}

impl HandoffAsk {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Solve(_) => "solve",
            Self::Explore(_) => "explore",
            Self::Investigate(_) => "investigate",
        }
    }

    pub fn expected_target_mode(&self) -> Mode {
        match self {
            Self::Solve(_) => Mode::Solve,
            Self::Explore(_) => Mode::Explore,
            Self::Investigate(_) => Mode::Investigate,
        }
    }
}

/// Shared `handoff.packet` v1 payload (`00-overview.md` 4.1).
///
/// Badges, tiers, and verdicts have no fields here; `deny_unknown_fields` rejects any attempt
/// to smuggle them across a hand-over (Rule 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffPacket {
    pub from: NodeRef,
    pub target_mode: Mode,
    pub title: String,
    pub statement: HandoffStatement,
    #[serde(default)]
    pub context: Vec<HandoffContextItem>,
    #[serde(default)]
    pub papers: Vec<PaperRef>,
    #[serde(default)]
    pub conventions: String,
    #[serde(default)]
    pub model_text: Vec<ModelTextEntry>,
    #[serde(default)]
    pub user_notes: Vec<String>,
    pub ask: HandoffAsk,
}

impl HandoffPacket {
    /// Target mode's host-written ledger marker (`00-overview.md` 4.1).
    pub fn target_marker(&self) -> Result<&'static str, String> {
        target_marker_for_mode(self.target_mode)
    }

    /// Validate structural invariants, bounds, and `target_mode` <-> `ask` agreement.
    pub fn validate(&self) -> Result<(), String> {
        let _ = self.target_marker()?;
        if self.target_mode != self.ask.expected_target_mode() {
            return Err(format!(
                "handoff target_mode `{}` does not match ask.kind `{}`",
                self.target_mode.as_str(),
                self.ask.kind_name()
            ));
        }
        validate_session_id(&self.from.session).map_err(|e| format!("invalid from.session: {e}"))?;
        if self.from.id.trim().is_empty() {
            return Err("handoff from.id must not be empty".into());
        }
        check_non_empty_bound(&self.title, MAX_TITLE_CHARS, "title")?;
        check_bound(&self.statement.text, MAX_STATEMENT_CHARS, "statement.text")?;
        check_bound(&self.conventions, MAX_CONVENTIONS_CHARS, "conventions")?;
        if self.context.len() > MAX_CONTEXT_ITEMS {
            return Err(format!("handoff context exceeds {MAX_CONTEXT_ITEMS} items"));
        }
        for (i, item) in self.context.iter().enumerate() {
            validate_session_id(&item.from.session)
                .map_err(|e| format!("invalid context[{i}].from.session: {e}"))?;
            if item.from.id.trim().is_empty() {
                return Err(format!("context[{i}].from.id must not be empty"));
            }
            check_bound(&item.text, MAX_CONTEXT_TEXT_CHARS, &format!("context[{i}].text"))?;
        }
        if self.papers.len() > MAX_PAPERS {
            return Err(format!("handoff papers exceeds {MAX_PAPERS} entries"));
        }
        if self.model_text.len() > MAX_MODEL_TEXT_ITEMS {
            return Err(format!("handoff model_text exceeds {MAX_MODEL_TEXT_ITEMS} items"));
        }
        for (i, mt) in self.model_text.iter().enumerate() {
            check_non_empty_bound(&mt.label, MAX_MODEL_TEXT_LABEL_CHARS, &format!("model_text[{i}].label"))?;
            check_bound(&mt.text, MAX_MODEL_TEXT_CHARS, &format!("model_text[{i}].text"))?;
        }
        if self.user_notes.len() > MAX_USER_NOTES {
            return Err(format!("handoff user_notes exceeds {MAX_USER_NOTES} items"));
        }
        for (i, note) in self.user_notes.iter().enumerate() {
            check_bound(note, MAX_USER_NOTE_CHARS, &format!("user_notes[{i}]"))?;
        }
        match &self.ask {
            HandoffAsk::Solve(s) => {
                check_non_empty_bound(&s.target.description, MAX_STATEMENT_CHARS, "ask.target.description")?;
                if s.symbols.len() > MAX_ASK_ITEMS
                    || s.suggested_done.len() > MAX_ASK_ITEMS
                    || s.check_first.len() > MAX_ASK_ITEMS
                    || s.risks.len() > MAX_ASK_ITEMS
                    || s.stop_if.len() > MAX_ASK_ITEMS
                {
                    return Err(format!("solve handoff ask list exceeds {MAX_ASK_ITEMS} items"));
                }
            }
            HandoffAsk::Explore(e) => {
                check_non_empty_bound(&e.idea_text, MAX_STATEMENT_CHARS, "ask.idea_text")?;
                if e.seed_papers.len() > MAX_PAPERS || e.focus_claims.len() > MAX_ASK_ITEMS {
                    return Err("explore handoff ask exceeds item limit".into());
                }
            }
            HandoffAsk::Investigate(inv) => {
                if inv.focus_claims.len() > MAX_ASK_ITEMS || inv.earlier_quotes.len() > MAX_CONTEXT_ITEMS {
                    return Err("investigate handoff ask exceeds item limit".into());
                }
            }
        }
        Ok(())
    }
}

/// Host marker written after `handoff.packet` for a valid hand-over target mode.
pub fn target_marker_for_mode(mode: Mode) -> Result<&'static str, String> {
    match mode {
        Mode::Investigate => Ok(MARKER_INVESTIGATION_STARTED),
        Mode::Explore => Ok(MARKER_IDEA_STARTED),
        Mode::Solve => Ok(MARKER_SOLVE_STARTED),
        Mode::Research => Err("target_mode must be `investigate`, `explore`, or `solve` (not `research`)".into()),
    }
}

/// Input to [`crate::handoff::execute_handoff`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    pub packet: HandoffPacket,
    /// Optional verbatim quote texts keyed by `quote_sha256` (used for child re-verification
    /// when a quote is not already stored verbatim in an origin `evidence.quote` event).
    #[serde(default)]
    pub quote_texts: BTreeMap<String, String>,
    /// Optional additional `source_id`s from the origin session to copy into the child
    /// (for example the primary paper of an Investigate hand-over even before quotes exist).
    #[serde(default)]
    pub extra_source_ids: Vec<String>,
}

/// Record of a `QuoteRef` dropped during child-side re-verification (Rule 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedQuote {
    pub location: String,
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub reason: String,
}

/// Result returned by [`crate::handoff::execute_handoff`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffReceipt {
    pub child_session: String,
    pub parent_session: String,
    pub target_mode: Mode,
    pub target_marker: String,
    pub packet_seq: u64,
    pub packet_hash: String,
    pub copied_sources: usize,
    pub quotes_matched: usize,
    pub quotes_dropped: usize,
    pub dropped_quotes: Vec<DroppedQuote>,
    pub packet: HandoffPacket,
}

fn check_non_empty_bound(s: &str, max_chars: usize, label: &str) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err(format!("handoff {label} must not be empty"));
    }
    check_bound(s, max_chars, label)
}

fn check_bound(s: &str, max_chars: usize, label: &str) -> Result<(), String> {
    let len = s.chars().count();
    if len > max_chars {
        return Err(format!("handoff {label} exceeds {max_chars} characters (got {len})"));
    }
    Ok(())
}
