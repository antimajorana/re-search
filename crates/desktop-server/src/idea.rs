//! Explore mode core session model, deterministic stage machine, budget ceilings,
//! and host-only start/statement writers (package X3).
//!
//! Governance: `00-overview.md` 2.3/2.4/2.7/2.8/3.1/3.2/3.3/5.2/8.1-8.3,
//! `explore.md` 3/4.1/4.2/4.4/5.3/6 (P1, P3, P4, P6, S3, S7, S8).

#[path = "idea_search.rs"]
pub(crate) mod search;
pub use search::*;

use ledger::{sha256_hex, Event, Ledger};
use research_tools::idea_terms::MAX_IDEA_TEXT_CHARS;
use research_tools::paper_fetch::{check_fetch_budget, fetched_sources, parse_paper_input};
use research_tools::paper_key::PaperKey;
use research_tools::run_budget::{
    count_model_calls, explore_stage_run_state, ExploreStageState, ModelCallBudget, RunScope,
    RunState, IDEA_EXCHANGE_EVENT,
};
use research_tools::tool_profile::MARKER_IDEA_STARTED;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use tools::EVENT_SCHEMA_VERSION;

/// Current Explore pipeline version written into `idea.started` (`explore.md` 4.2, 8).
pub const PIPELINE_VERSION: u32 = 1;

// Named event-kind constants tied to accepted decisions (`00-overview.md` 5.2, 8.2 decisions 10 & 11;
// `explore.md` 4.2 & decision 16): Explore-only events use the `idea.` prefix, and "leg" is "exchange".
pub const EVENT_IDEA_STARTED: &str = MARKER_IDEA_STARTED;
pub const EVENT_IDEA_STATED: &str = "idea.stated";
pub const EVENT_IDEA_BRIEF: &str = "idea.brief";
pub const EVENT_IDEA_ANSWER: &str = "idea.answer";
pub const EVENT_IDEA_SEARCH_PLANNED: &str = "idea.search_planned";
pub const EVENT_IDEA_SEARCH_FINISHED: &str = "idea.search_finished";
pub const EVENT_IDEA_CANDIDATES: &str = "idea.candidates";
pub const EVENT_IDEA_TRIAGE: &str = "idea.triage";
pub const EVENT_IDEA_PAPER: &str = "idea.paper";
pub const EVENT_IDEA_MAP: &str = "idea.map";
pub const EVENT_IDEA_VERDICT: &str = "idea.verdict";
pub const EVENT_IDEA_DIRECTIONS: &str = "idea.directions";
pub const EVENT_IDEA_NOTE: &str = "idea.note";
pub const EVENT_IDEA_STAGE: &str = "idea.stage";
pub const EVENT_IDEA_EXCHANGE: &str = IDEA_EXCHANGE_EVENT;
/// Recorded when a user explicitly clicks to raise a budget ceiling (`explore.md` Rule P3).
pub const EVENT_IDEA_BUDGET_RAISED: &str = "idea.budget_raised";

/// Author tag for user-authored `idea.stated` events (`explore.md` 4.1).
pub const IDEA_AUTHOR_USER: &str = "user";
/// Default results requested per user-shown first query (within F11 `MAX_RESULTS_CAP = 25`).
pub const FIRST_SEARCH_MAX_RESULTS: usize = 15;
pub const MAX_SEEDS: usize = 32;
pub const MAX_FIELDS: usize = 16;
pub const MAX_PREVIEW_LIBRARY_MATCHES: usize = 20;
pub const MAX_NOTE_COMMENT_CHARS: usize = 2_000;

pub const NOTE_ACTIONS: &[&str] = &[
    "wrong", "not_relevant", "pin", "key_paper", "confirm_same", "reject_same", "comment",
];
pub const NOTE_TARGET_KINDS: &[&str] = &["node", "direction", "candidate", "verdict", "paper"];

/// Explore presets with hard ceilings enforced in code (`explore.md` 3 & decision 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeaPreset {
    Quick,
    #[default]
    Standard,
    Thorough,
}

impl IdeaPreset {
    pub const ALL: [IdeaPreset; 3] = [Self::Quick, Self::Standard, Self::Thorough];

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "quick" => Ok(Self::Quick),
            "standard" => Ok(Self::Standard),
            "thorough" => Ok(Self::Thorough),
            other => Err(format!("invalid preset `{other}` (expected `quick`, `standard`, or `thorough`)")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self { Self::Quick => "quick", Self::Standard => "standard", Self::Thorough => "thorough" }
    }

    pub fn label(self) -> &'static str {
        match self { Self::Quick => "Quick", Self::Standard => "Standard", Self::Thorough => "Thorough" }
    }

    pub fn default_budget(self) -> IdeaBudget {
        let (max_queries, max_screened, max_papers, calls_per_paper, typical_model_calls, max_model_calls) =
            match self {
                Self::Quick => (8, 25, 0, 0, 3, 5),
                Self::Standard => (18, 40, 6, 3, 15, 24),
                Self::Thorough => (36, 80, 12, 4, 35, 55),
            };
        IdeaBudget {
            preset: self, max_queries, max_screened, max_papers,
            calls_per_paper, typical_model_calls, max_model_calls,
        }
    }
}

/// Hard budget ceilings written in `idea.started` (`explore.md` 3 & 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaBudget {
    pub preset: IdeaPreset,
    pub max_queries: u64,
    pub max_screened: u64,
    pub max_papers: usize,
    pub calls_per_paper: u64,
    pub typical_model_calls: u64,
    pub max_model_calls: u64,
}

/// Live budget accounting derived strictly from ledger events via F9 and F3 (`explore.md` Rule P3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaBudgetSnapshot {
    pub preset: String,
    pub preset_label: String,
    pub max_queries: u64,
    pub queries_used: u64,
    pub max_screened: u64,
    pub screened_used: u64,
    pub max_papers: usize,
    pub papers_fetched: usize,
    pub calls_per_paper: u64,
    pub model_calls: ModelCallBudget,
    pub used_by_stage: BTreeMap<String, u64>,
    pub state: String,
}

impl IdeaBudgetSnapshot {
    pub fn can_run_query(&self) -> bool { self.queries_used < self.max_queries }
    pub fn can_fetch_paper(&self) -> bool { self.papers_fetched < self.max_papers }
    pub fn can_call_model(&self) -> bool { self.model_calls.can_spend(1) }
}

/// Seven stages of the Explore stage rail (`explore.md` 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeaStage { Idea, Sharpen, Search, Triage, Read, Map, Directions }

impl IdeaStage {
    pub const ALL: [IdeaStage; 7] = [
        Self::Idea, Self::Sharpen, Self::Search, Self::Triage, Self::Read, Self::Map, Self::Directions,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idea => "idea", Self::Sharpen => "sharpen", Self::Search => "search",
            Self::Triage => "triage", Self::Read => "read", Self::Map => "map", Self::Directions => "directions",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idea => "Idea", Self::Sharpen => "Sharpen", Self::Search => "Search",
            Self::Triage => "Triage", Self::Read => "Read", Self::Map => "Map", Self::Directions => "Directions",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "idea" => Some(Self::Idea), "sharpen" => Some(Self::Sharpen), "search" => Some(Self::Search),
            "triage" => Some(Self::Triage), "read" => Some(Self::Read), "map" => Some(Self::Map),
            "directions" => Some(Self::Directions), _ => None,
        }
    }
}

/// Exchange recorders guarded by the stage transition table (`explore.md` 5.1, Rule P1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeaRecorder {
    RecordIdeaBrief, RecordIdeaTriage, RecordPaperEvidence, RecordIdeaMap, RecordIdeaDirections,
}

impl IdeaRecorder {
    pub fn required_stage(self) -> IdeaStage {
        match self {
            Self::RecordIdeaBrief => IdeaStage::Sharpen, Self::RecordIdeaTriage => IdeaStage::Triage,
            Self::RecordPaperEvidence => IdeaStage::Read, Self::RecordIdeaMap => IdeaStage::Map,
            Self::RecordIdeaDirections => IdeaStage::Directions,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RecordIdeaBrief => "record_idea_brief",
            Self::RecordIdeaTriage => "record_idea_triage",
            Self::RecordPaperEvidence => "record_paper_evidence",
            Self::RecordIdeaMap => "record_idea_map",
            Self::RecordIdeaDirections => "record_idea_directions",
        }
    }
}

/// User-submitted verbatim idea statement (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaStatement {
    pub rev: u32,
    pub text: String,
    pub text_sha256: String,
    pub fields: Vec<String>,
    pub seeds: Vec<PaperKey>,
    pub author: String,
}

/// Resolved effective state of one stage for views and transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRecord {
    pub stage: IdeaStage,
    pub stored_state: Option<ExploreStageState>,
    pub run_state: Option<RunState>,
    pub reason: Option<String>,
    pub request_id: Option<String>,
    pub updated_seq: Option<u64>,
}

/// Shared normalizer for arXiv category `fields` used identically by preview and create (Rule P4).
pub fn normalize_fields(raw_fields: &[String]) -> Result<Vec<String>, String> {
    if raw_fields.len() > MAX_FIELDS {
        return Err(format!("at most {MAX_FIELDS} arXiv categories may be supplied"));
    }
    let (mut fields, mut seen) = (Vec::new(), BTreeSet::new());
    for f in raw_fields {
        let stripped = f.trim();
        let c = stripped
            .strip_prefix("cat:")
            .or_else(|| stripped.strip_prefix("CAT:"))
            .unwrap_or(stripped)
            .trim();
        if c.is_empty() { continue; }
        if c.len() > 64 || !c.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')) {
            return Err(format!("invalid field/category `{c}`"));
        }
        if seen.insert(c.to_string()) { fields.push(c.to_string()); }
    }
    Ok(fields)
}

/// Extract configured budget from `idea.started` + any `idea.budget_raised` events.
pub fn effective_budget(events: &[Event]) -> IdeaBudget {
    let mut budget = IdeaPreset::Standard.default_budget();
    for ev in events {
        if ev.kind == EVENT_IDEA_STARTED {
            if let Some(p) = ev.payload.get("preset").and_then(Value::as_str).and_then(|s| IdeaPreset::parse(s).ok()) {
                budget = p.default_budget();
            }
            if let Some(b) = ev.payload.get("budget").and_then(Value::as_object) {
                if let Some(v) = b.get("max_queries").and_then(Value::as_u64) { budget.max_queries = v; }
                if let Some(v) = b.get("max_screened").and_then(Value::as_u64) { budget.max_screened = v; }
                if let Some(v) = b.get("max_papers").and_then(Value::as_u64) { budget.max_papers = v as usize; }
                if let Some(v) = b.get("calls_per_paper").and_then(Value::as_u64) { budget.calls_per_paper = v; }
                if let Some(v) = b.get("max_model_calls").and_then(Value::as_u64) { budget.max_model_calls = v; }
            }
        } else if ev.kind == EVENT_IDEA_BUDGET_RAISED {
            budget.max_queries = budget.max_queries.saturating_add(ev.payload["extra_queries"].as_u64().unwrap_or(0));
            budget.max_papers = budget.max_papers.saturating_add(ev.payload["extra_papers"].as_u64().unwrap_or(0) as usize);
            budget.max_model_calls = budget.max_model_calls.saturating_add(ev.payload["extra_model_calls"].as_u64().unwrap_or(0));
        }
    }
    budget
}

/// Compute live budget usage strictly from ledger events using F9 + F3 (`explore.md` Rule P3).
pub fn budget_snapshot(events: &[Event], running: bool) -> IdeaBudgetSnapshot {
    let cfg = effective_budget(events);
    let mut queries_used = 0u64;
    let mut screened_used = 0u64;
    let mut used_by_stage: BTreeMap<String, u64> = BTreeMap::new();
    let mut last_budget_raise_seq = 0u64;
    let mut last_cap_stop_seq = 0u64;

    for ev in events {
        match ev.kind.as_str() {
            EVENT_IDEA_SEARCH_FINISHED => {
                if let Some(list) = ev.payload.get("results").and_then(Value::as_array) {
                    let attempted = list
                        .iter()
                        .filter(|r| {
                            !matches!(
                                r.get("status").and_then(Value::as_str),
                                Some("skipped_budget" | "disabled" | "not_configured")
                            )
                        })
                        .count() as u64;
                    queries_used = queries_used.saturating_add(attempted);
                }
            }
            EVENT_IDEA_TRIAGE => {
                let n = ev.payload.get("items").and_then(Value::as_array).map_or(0, Vec::len) as u64;
                screened_used = screened_used.max(n);
            }
            EVENT_IDEA_EXCHANGE => {
                let key = ev.payload.get("stage").or_else(|| ev.payload.get("exchange"))
                    .and_then(Value::as_str).unwrap_or("exchange").to_string();
                *used_by_stage.entry(key).or_insert(0) += 1;
            }
            EVENT_IDEA_BUDGET_RAISED => last_budget_raise_seq = ev.seq,
            EVENT_IDEA_STAGE
                if ev.payload.get("state").and_then(Value::as_str) == Some("stopped_at_cap") =>
            {
                last_cap_stop_seq = ev.seq;
            }
            _ => {}
        }
    }

    let stages = resolve_stages(events, running);
    let directions_done = stages
        .iter()
        .find(|r| r.stage == IdeaStage::Directions)
        .is_some_and(|r| r.stored_state == Some(ExploreStageState::Finished));
    let model_calls = ModelCallBudget::new(cfg.max_model_calls, count_model_calls(events, RunScope::All));
    let papers_fetched = fetched_sources(events);
    let fetch_capped = cfg.max_papers > 0 && check_fetch_budget(events, cfg.max_papers).is_err();
    let active_cap_stop = last_cap_stop_seq > last_budget_raise_seq
        && stages.iter().any(|r| r.stored_state == Some(ExploreStageState::StoppedAtCap));

    let state = if directions_done {
        "done"
    } else if running {
        "running"
    } else if active_cap_stop || model_calls.is_exhausted() || fetch_capped {
        "stopped_at_cap"
    } else {
        "idle"
    };

    IdeaBudgetSnapshot {
        preset: cfg.preset.as_str().to_string(),
        preset_label: cfg.preset.label().to_string(),
        max_queries: cfg.max_queries,
        queries_used,
        max_screened: cfg.max_screened,
        screened_used,
        max_papers: cfg.max_papers,
        papers_fetched,
        calls_per_paper: cfg.calls_per_paper,
        model_calls,
        used_by_stage,
        state: state.to_string(),
    }
}

/// Sequence number of the latest `idea.stated` revision (0 if none).
pub fn latest_statement_seq(events: &[Event]) -> u64 {
    events.iter().rev().find(|e| e.kind == EVENT_IDEA_STATED).map_or(0, |e| e.seq)
}

/// Resolve every stage's latest state for the active idea revision, converting any `requested`
/// stage left open when `running == false` into [`ExploreStageState::Interrupted`] (Rule S3).
pub fn resolve_stages(events: &[Event], running: bool) -> Vec<StageRecord> {
    let rev_seq = latest_statement_seq(events);
    let mut latest: BTreeMap<IdeaStage, (ExploreStageState, Option<String>, Option<String>, u64)> = BTreeMap::new();
    for ev in events {
        if ev.kind != EVENT_IDEA_STAGE || ev.seq < rev_seq { continue; }
        let Some(stage) = ev.payload.get("stage").and_then(Value::as_str).and_then(IdeaStage::parse) else { continue; };
        let Some(state) = ev.payload.get("state").and_then(Value::as_str).and_then(ExploreStageState::from_stored) else { continue; };
        let reason = ev.payload.get("reason").and_then(Value::as_str).map(str::to_string);
        let req_id = ev.payload.get("request_id").and_then(Value::as_str).map(str::to_string);
        latest.insert(stage, (state, reason, req_id, ev.seq));
    }

    IdeaStage::ALL.iter().map(|&stage| {
        if let Some((raw_state, reason, request_id, seq)) = latest.get(&stage) {
            let effective = if *raw_state == ExploreStageState::Requested && !running {
                ExploreStageState::Interrupted
            } else {
                *raw_state
            };
            let effective_reason = if effective == ExploreStageState::Interrupted && reason.is_none() {
                Some("Interrupted before completion; click Retry to run again".to_string())
            } else {
                reason.clone()
            };
            StageRecord {
                stage,
                stored_state: Some(effective),
                run_state: Some(explore_stage_run_state(effective)),
                reason: effective_reason,
                request_id: request_id.clone(),
                updated_seq: Some(*seq),
            }
        } else if stage == IdeaStage::Idea && rev_seq > 0 {
            StageRecord {
                stage,
                stored_state: Some(ExploreStageState::Finished),
                run_state: Some(explore_stage_run_state(ExploreStageState::Finished)),
                reason: None,
                request_id: None,
                updated_seq: Some(rev_seq),
            }
        } else {
            StageRecord { stage, stored_state: None, run_state: None, reason: None, request_id: None, updated_seq: None }
        }
    }).collect()
}

fn is_offline_session(events: &[Event]) -> bool {
    events.iter().rev().find_map(|e| {
        matches!(e.kind.as_str(), EVENT_IDEA_STARTED | EVENT_IDEA_SEARCH_PLANNED | EVENT_IDEA_SEARCH_FINISHED)
            .then(|| e.payload.get("offline").and_then(Value::as_bool)).flatten()
    }).unwrap_or(false)
}

/// Deterministic single transition table (`explore.md` Rule P1).
///
/// Scoped to the active `idea.stated` revision; preserves post-Sharpen `Search` when only
/// the initial pre-Sharpen user search has run, and skips `Read` when `max_papers == 0`.
pub fn next_stage(events: &[Event]) -> Option<IdeaStage> {
    let rev_seq = latest_statement_seq(events);
    if rev_seq == 0 { return Some(IdeaStage::Idea); }
    let stages = resolve_stages(events, false);
    let budget = effective_budget(events);
    if let Some(rec) = stages.iter().find(|r| r.stored_state.is_some_and(|st| st != ExploreStageState::Finished)) {
        return Some(rec.stage);
    }

    let stage_of = |t: IdeaStage| stages.iter().find(|r| r.stage == t);
    let latest_kind_seq = |k: &str| events.iter().filter(|e| e.seq > rev_seq && e.kind == k).map(|e| e.seq).max();

    let brief_seq = latest_kind_seq(EVENT_IDEA_BRIEF);
    if stage_of(IdeaStage::Sharpen).and_then(|r| r.stored_state) != Some(ExploreStageState::Finished) || brief_seq.is_none() {
        return Some(IdeaStage::Sharpen);
    }
    let search_rec = stage_of(IdeaStage::Search);
    let search_ready = search_rec.is_some_and(|r| {
        r.stored_state == Some(ExploreStageState::Finished)
            && (is_offline_session(events) || r.updated_seq.unwrap_or(0) > brief_seq.unwrap_or(0))
    });
    let latest_cands_seq = latest_kind_seq(EVENT_IDEA_CANDIDATES).unwrap_or(0);
    let triage_done = stage_of(IdeaStage::Triage).and_then(|r| r.stored_state) == Some(ExploreStageState::Finished)
        && latest_kind_seq(EVENT_IDEA_TRIAGE).is_some_and(|ts| ts > latest_cands_seq);
    if !search_ready && !triage_done { return Some(IdeaStage::Search); }
    if !triage_done { return Some(IdeaStage::Triage); }
    if budget.max_papers > 0 && stage_of(IdeaStage::Read).and_then(|r| r.stored_state) != Some(ExploreStageState::Finished) {
        return Some(IdeaStage::Read);
    }
    if stage_of(IdeaStage::Map).and_then(|r| r.stored_state) != Some(ExploreStageState::Finished) || latest_kind_seq(EVENT_IDEA_MAP).is_none() {
        return Some(IdeaStage::Map);
    }
    if stage_of(IdeaStage::Directions).and_then(|r| r.stored_state) != Some(ExploreStageState::Finished) || latest_kind_seq(EVENT_IDEA_DIRECTIONS).is_none() {
        return Some(IdeaStage::Directions);
    }
    None
}

/// Enforce Rule P1: a recorder is accepted ONLY while its stage is active (`requested`)
/// in the current revision and every prerequisite stage has actually finished with its artifact.
pub fn validate_recorder_stage(events: &[Event], recorder: IdeaRecorder) -> Result<(), String> {
    let rev_seq = latest_statement_seq(events);
    let required = recorder.required_stage();
    let stages = resolve_stages(events, true);
    let state_of = |t: IdeaStage| stages.iter().find(|r| r.stage == t).and_then(|r| r.stored_state);

    if state_of(required) != Some(ExploreStageState::Requested) {
        return Err(format!(
            "recorder `{}` refused outside stage `{}` (current stage state: {})",
            recorder.as_str(), required.as_str(),
            state_of(required).map_or("idle", ExploreStageState::as_str),
        ));
    }

    let has_in_rev = |k: &str| events.iter().any(|e| e.seq > rev_seq && e.kind == k);
    let budget = effective_budget(events);
    let prereq_ok = match recorder {
        IdeaRecorder::RecordIdeaBrief => rev_seq > 0,
        IdeaRecorder::RecordIdeaTriage => {
            let brief_seq = events.iter().filter(|e| e.seq > rev_seq && e.kind == EVENT_IDEA_BRIEF)
                .map(|e| e.seq).max().unwrap_or(0);
            let search_ok = stages.iter().find(|r| r.stage == IdeaStage::Search).is_some_and(|r| {
                r.stored_state == Some(ExploreStageState::Finished)
                    && (is_offline_session(events) || r.updated_seq.unwrap_or(0) > brief_seq)
            });
            state_of(IdeaStage::Sharpen) == Some(ExploreStageState::Finished)
                && brief_seq > 0 && search_ok && has_in_rev(EVENT_IDEA_CANDIDATES)
        }
        IdeaRecorder::RecordPaperEvidence => {
            budget.max_papers > 0 && state_of(IdeaStage::Triage) == Some(ExploreStageState::Finished) && has_in_rev(EVENT_IDEA_TRIAGE)
        }
        IdeaRecorder::RecordIdeaMap => {
            state_of(IdeaStage::Triage) == Some(ExploreStageState::Finished) && has_in_rev(EVENT_IDEA_TRIAGE)
                && (budget.max_papers == 0 || state_of(IdeaStage::Read) == Some(ExploreStageState::Finished))
        }
        IdeaRecorder::RecordIdeaDirections => {
            state_of(IdeaStage::Map) == Some(ExploreStageState::Finished) && has_in_rev(EVENT_IDEA_MAP)
        }
    };
    if !prereq_ok {
        return Err(format!(
            "recorder `{}` refused: prerequisite stage not finished for `{}`",
            recorder.as_str(), required.as_str()
        ));
    }
    Ok(())
}

/// Validate and normalize user idea statement (`explore.md` 4.1).
pub fn build_idea_statement(
    rev: u32,
    raw_text: &str,
    raw_fields: &[String],
    raw_seeds: &[String],
) -> Result<IdeaStatement, String> {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return Err("`text` must be a non-empty string".to_string());
    }
    let char_count = trimmed.chars().count();
    if char_count > MAX_IDEA_TEXT_CHARS {
        return Err(format!(
            "`text` exceeds the {MAX_IDEA_TEXT_CHARS}-character limit ({char_count} characters)"
        ));
    }
    let fields = normalize_fields(raw_fields)?;
    if raw_seeds.len() > MAX_SEEDS {
        return Err(format!("at most {MAX_SEEDS} seed papers may be supplied"));
    }
    let (mut seeds, mut seen_seeds) = (Vec::new(), BTreeSet::new());
    for s in raw_seeds {
        let c = s.trim();
        if c.is_empty() { continue; }
        let key = PaperKey::parse(c)
            .or_else(|_| parse_paper_input(c).map(|r| r.key))
            .map_err(|_| format!("invalid seed paper identifier `{c}`"))?;
        if seen_seeds.insert(key.clone()) { seeds.push(key); }
    }
    Ok(IdeaStatement {
        rev,
        text: trimmed.to_string(),
        text_sha256: sha256_hex(trimmed.as_bytes()),
        fields,
        seeds,
        author: IDEA_AUTHOR_USER.to_string(),
    })
}

/// Host-only writer: atomically appends `idea.started`, `idea.stated`, and `idea.stage(idea, finished)`.
pub fn append_explore_session_start(
    ledger: &Ledger,
    session_id: &str,
    preset: IdeaPreset,
    offline: bool,
    stmt: &IdeaStatement,
) -> Result<(), String> {
    let started = json!({
        "preset": preset.as_str(),
        "budget": preset.default_budget(),
        "pipeline_version": PIPELINE_VERSION,
        "offline": offline,
    });
    let stated = serde_json::to_value(stmt).map_err(|e| e.to_string())?;
    let idea_done = json!({
        "stage": IdeaStage::Idea.as_str(),
        "state": ExploreStageState::Finished.as_str(),
        "request_id": "idea-1",
    });
    ledger
        .append_new_session(
            session_id,
            &[
                (EVENT_IDEA_STARTED, EVENT_SCHEMA_VERSION, &started),
                (EVENT_IDEA_STATED, EVENT_SCHEMA_VERSION, &stated),
                (EVENT_IDEA_STAGE, EVENT_SCHEMA_VERSION, &idea_done),
            ],
        )
        .map_err(|e| format!("failed to write Explore start events: {e}"))?;
    Ok(())
}
