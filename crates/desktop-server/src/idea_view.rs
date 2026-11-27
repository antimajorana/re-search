//! Pure Explore session view projection (package X3).
//!
//! Reads ledger events strictly through `serde_json::Value` so older or future
//! events with missing/unknown fields stay readable (`explore.md` 4, 8).
//! Maps every stage state through F9's `explore_stage_run_state` (`00-overview.md` 3.3),
//! returns the cost-line payload for F6's `run_bar.js`, candidate list, `"Where I looked"`
//! skeleton, and a `GraphView`-shaped map placeholder (`00-overview.md` 2.6).
//!
//! Rule S8: zero `"verified"`, `"new"`, or `"novel"` strings appear in any view label.

use crate::idea::{
    budget_snapshot, next_stage, resolve_stages, IdeaBudgetSnapshot, IdeaPreset, IdeaStage,
    StageRecord, EVENT_IDEA_BRIEF, EVENT_IDEA_CANDIDATES, EVENT_IDEA_DIRECTIONS, EVENT_IDEA_MAP,
    EVENT_IDEA_NOTE, EVENT_IDEA_PAPER, EVENT_IDEA_SEARCH_FINISHED, EVENT_IDEA_SEARCH_PLANNED,
    EVENT_IDEA_STARTED, EVENT_IDEA_STATED, EVENT_IDEA_TRIAGE, EVENT_IDEA_VERDICT, PIPELINE_VERSION,
};
use crate::idea_view_coverage::{build_map_graph_view, build_where_i_looked, fold_answers};
pub use crate::idea_view_coverage::NOT_SEARCHED_ITEMS;
use ledger::Event;
use research_tools::run_budget::{
    ExploreStageState, RunState, RunStateAction, RUN_STATE_LABEL_DONE, RUN_STATE_LABEL_RUNNING,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

// Named constants for owner-accepted on-screen strings (`explore.md` 3, `00-overview.md` 3.3 & 8.2).
pub const NO_MODEL_NOTICE: &str =
    "Search and library work without a model. Sharpening, reading and mapping need one.";
pub const EMPTY_OFFLINE_MESSAGE: &str =
    "Offline: showing library matches only; nothing was searched";
pub const EMPTY_ALL_FAILED_MESSAGE: &str = "All services failed: no result is shown";
pub const EMPTY_ZERO_CANDIDATES_MESSAGE: &str =
    "Zero candidates: try fewer or broader terms";
pub const BADGE_NOT_READ: &str = "not read";
pub const BADGE_IN_LIBRARY: &str = "in your library";
pub const STAGE_IDLE_LABEL: &str = "Idle";

/// Pure projection of an Explore session's ledger events into the JSON payload attached
/// under `"idea"` on `GET /api/sessions/{id}` (`explore.md` 5.4).
pub fn view(events: &[Event], running: bool) -> Value {
    let mut pipeline_version = u64::from(PIPELINE_VERSION);
    let mut preset = IdeaPreset::Standard;
    let mut offline = false;

    let mut latest_statement: Option<Value> = None;
    let mut statement_revisions = 0u64;
    let mut latest_statement_seq = 0u64;

    let mut latest_plan: Option<Value> = None;
    let mut latest_search_finished: Option<Value> = None;
    let mut latest_candidates: Vec<Value> = Vec::new();
    let mut candidates_round = 0u64;

    let mut latest_brief: Option<Value> = None;
    let mut latest_brief_seq = 0u64;
    let mut latest_triage: Option<Value> = None;
    let mut latest_map: Option<Value> = None;
    let mut latest_map_seq = 0u64;
    let mut latest_verdicts: Option<Value> = None;
    let mut latest_directions: Vec<Value> = Vec::new();

    let mut paper_statuses: BTreeMap<String, String> = BTreeMap::new();
    let mut notes: Vec<Value> = Vec::new();

    for ev in events {
        match ev.kind.as_str() {
            EVENT_IDEA_STARTED => {
                if let Some(v) = ev.payload.get("pipeline_version").and_then(Value::as_u64) {
                    pipeline_version = v;
                }
                if let Some(p) = ev.payload.get("preset").and_then(Value::as_str) {
                    if let Ok(parsed) = IdeaPreset::parse(p) {
                        preset = parsed;
                    }
                }
                if let Some(off) = ev.payload.get("offline").and_then(Value::as_bool) {
                    offline = off;
                }
            }
            EVENT_IDEA_STATED => {
                statement_revisions += 1;
                latest_statement_seq = ev.seq;
                latest_brief = None;
                latest_brief_seq = 0;
                latest_statement = Some(json!({
                    "seq": ev.seq,
                    "rev": ev.payload.get("rev").and_then(Value::as_u64).unwrap_or(statement_revisions),
                    "text": ev.payload.get("text").and_then(Value::as_str).unwrap_or(""),
                    "text_sha256": ev.payload.get("text_sha256").and_then(Value::as_str).unwrap_or(""),
                    "fields": ev.payload.get("fields").and_then(Value::as_array).cloned().unwrap_or_default(),
                    "seeds": ev.payload.get("seeds").and_then(Value::as_array).cloned().unwrap_or_default(),
                    "author": ev.payload.get("author").and_then(Value::as_str).unwrap_or("user"),
                }));
            }
            EVENT_IDEA_SEARCH_PLANNED => {
                if let Some(off) = ev.payload.get("offline").and_then(Value::as_bool) {
                    offline = off;
                }
                latest_plan = Some(ev.payload.clone());
            }
            EVENT_IDEA_SEARCH_FINISHED => {
                if let Some(off) = ev.payload.get("offline").and_then(Value::as_bool) {
                    offline = off;
                }
                latest_search_finished = Some(ev.payload.clone());
            }
            EVENT_IDEA_CANDIDATES => {
                candidates_round = ev.payload.get("round").and_then(Value::as_u64).unwrap_or(1);
                latest_candidates = ev
                    .payload
                    .get("candidates")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
            }
            EVENT_IDEA_PAPER => {
                if let (Some(k), Some(st)) = (
                    ev.payload.get("key").and_then(Value::as_str),
                    ev.payload.get("status").and_then(Value::as_str),
                ) {
                    paper_statuses.insert(k.to_string(), st.to_string());
                }
            }
            EVENT_IDEA_BRIEF => {
                latest_brief_seq = ev.seq;
                latest_brief = Some(ev.payload.clone());
            }
            EVENT_IDEA_TRIAGE => latest_triage = Some(ev.payload.clone()),
            EVENT_IDEA_MAP => {
                latest_map_seq = ev.seq;
                latest_map = Some(ev.payload.clone());
            }
            EVENT_IDEA_VERDICT => latest_verdicts = Some(ev.payload.clone()),
            EVENT_IDEA_DIRECTIONS => {
                latest_directions = ev
                    .payload
                    .get("directions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
            }
            EVENT_IDEA_NOTE => {
                notes.push(json!({
                    "seq": ev.seq,
                    "ts_ms": ev.ts_ms,
                    "target_kind": ev.payload.get("target_kind").and_then(Value::as_str).unwrap_or(""),
                    "target_id": ev.payload.get("target_id").and_then(Value::as_str).unwrap_or(""),
                    "action": ev.payload.get("action").and_then(Value::as_str).unwrap_or(""),
                    "comment": ev.payload.get("comment").and_then(Value::as_str).unwrap_or(""),
                }));
            }
            _ => {}
        }
    }

    let stage_records = resolve_stages(events, running);
    let budget = budget_snapshot(events, running);
    let next = next_stage(events);

    let applied = fold_answers(
        events,
        latest_statement_seq,
        latest_brief.as_ref(),
        latest_brief_seq,
    );

    let candidates = annotate_candidate_views(latest_candidates, &paper_statuses);
    let library_matches: Vec<Value> = candidates
        .iter()
        .filter(|c| c.get("in_library").and_then(Value::as_bool).unwrap_or(false))
        .cloned()
        .collect();

    let (where_i_looked, coverage) = build_where_i_looked(
        events,
        offline,
        latest_plan.as_ref(),
        latest_search_finished.as_ref(),
        &candidates,
    );

    let empty_state = compute_empty_state(
        offline,
        &stage_records,
        latest_search_finished.as_ref(),
        &candidates,
    );

    let run_bar = build_run_bar(&stage_records, &budget, running);
    let stages_json: Vec<Value> = stage_records.iter().map(stage_record_to_json).collect();
    let search_awaiting_approval = stage_records.iter().any(|r| {
        r.stage == IdeaStage::Search && r.stored_state == Some(ExploreStageState::PlanApproval)
    });
    let plan_approval =
        crate::idea_view_coverage::build_plan_approval(latest_plan.as_ref(), search_awaiting_approval);
    let map_view = build_map_graph_view(latest_map.as_ref());
    let map_stale = latest_map_seq > 0 && latest_statement_seq > latest_map_seq;

    json!({
        "pipeline_version": pipeline_version,
        "preset": preset.as_str(),
        "preset_label": preset.label(),
        "offline": offline,
        "no_model_notice": NO_MODEL_NOTICE,
        "statement": latest_statement,
        "statement_revisions": statement_revisions,
        "next_stage": next.map(IdeaStage::as_str),
        "stages": stages_json,
        "run_bar": run_bar,
        "budget": budget,
        "candidates_round": candidates_round,
        "candidates": candidates,
        "library_matches": library_matches,
        "where_i_looked": where_i_looked,
        "coverage": coverage,
        "empty_state": empty_state,
        "brief": applied.sharpened_idea,
        "sharpened_idea": applied.sharpened_idea,
        "question_answers": applied.question_answers,
        "reading_list_override": applied.reading_list_override,
        "pause_before_reading": applied.pause_before_reading,
        "search_plan": latest_plan,
        "plan_approval": plan_approval,
        "triage": latest_triage,
        "verdicts": latest_verdicts,
        "map": map_view,
        "map_stale": map_stale,
        "directions": latest_directions,
        "notes": notes,
    })
}

fn stage_record_to_json(rec: &StageRecord) -> Value {
    let run_state_str = rec.run_state.map(run_state_wire_key);
    let label = rec
        .run_state
        .map(RunState::label)
        .unwrap_or(STAGE_IDLE_LABEL);
    let action = rec.run_state.and_then(RunState::action).map(|a| match a {
        RunStateAction::Continue => "continue",
        RunStateAction::Retry => "retry",
    });
    json!({
        "id": rec.stage.as_str(),
        "label": rec.stage.label(),
        "stored_state": rec.stored_state.map_or("idle", ExploreStageState::as_str),
        "run_state": run_state_str,
        "run_state_label": label,
        "action": action,
        "reason": rec.reason,
        "request_id": rec.request_id,
        "updated_seq": rec.updated_seq,
    })
}

fn run_state_wire_key(state: RunState) -> &'static str {
    match state {
        RunState::Running => "running",
        RunState::WaitingForYou => "waiting",
        RunState::Paused => "paused",
        RunState::Interrupted => "interrupted",
        RunState::Failed => "failed",
        RunState::Done => "done",
        RunState::DoneWithOpenItems => "done_open_items",
    }
}

fn build_run_bar(
    stages: &[StageRecord],
    budget: &IdeaBudgetSnapshot,
    running: bool,
) -> Value {
    // Priority order matches overview 3.3: active running/waiting > interrupted > failed > paused > done.
    let active_rec = stages
        .iter()
        .find(|r| r.run_state == Some(RunState::Running))
        .or_else(|| stages.iter().find(|r| r.run_state == Some(RunState::WaitingForYou)))
        .or_else(|| stages.iter().find(|r| r.run_state == Some(RunState::Interrupted)))
        .or_else(|| stages.iter().find(|r| r.run_state == Some(RunState::Failed)))
        .or_else(|| stages.iter().find(|r| r.run_state == Some(RunState::Paused)));

    let (state_key, state_label, reason, action, stage_id) = if let Some(rec) = active_rec {
        let rs = rec.run_state.unwrap_or(RunState::Done);
        let act = rs.action().map(|a| match a {
            RunStateAction::Continue => "continue",
            RunStateAction::Retry => "retry",
        });
        (
            run_state_wire_key(rs),
            rs.label(),
            rec.reason.clone(),
            act,
            Some(rec.stage.as_str()),
        )
    } else if running {
        ("running", RUN_STATE_LABEL_RUNNING, None, None, None)
    } else if budget.state == "stopped_at_cap" {
        (
            "paused",
            RunState::Paused.label(),
            Some("Budget cap reached".to_string()),
            Some("continue"),
            None,
        )
    } else {
        ("done", RUN_STATE_LABEL_DONE, None, None, None)
    };

    let queries_part = format!("{} of {} queries", budget.queries_used, budget.max_queries);
    let papers_part = format!("{} of {} papers", budget.papers_fetched, budget.max_papers);
    let calls_part = format!(
        "{} of at most {} model calls",
        budget.model_calls.used, budget.model_calls.cap
    );
    let cost_line = format!(
        "{} - {} - {} - {}",
        budget.preset_label, queries_part, papers_part, calls_part
    );

    json!({
        "state": state_key,
        "label": state_label,
        "reason": reason,
        "action": action,
        "stage": stage_id,
        "preset_label": budget.preset_label,
        "queries_used": budget.queries_used,
        "queries_max": budget.max_queries,
        "papers_read": budget.papers_fetched,
        "papers_max": budget.max_papers,
        "model_calls_used": budget.model_calls.used,
        "model_calls_max": budget.model_calls.cap,
        // F6 run_bar.js camelCase fields so `idea.run_bar` can be passed straight to `renderRunBar`.
        "modelCallsUsed": budget.model_calls.used,
        "modelCallsMax": budget.model_calls.cap,
        "extras": [budget.preset_label, queries_part, papers_part],
        "cost_line": cost_line,
    })
}

fn annotate_candidate_views(
    raw_candidates: Vec<Value>,
    paper_statuses: &BTreeMap<String, String>,
) -> Vec<Value> {
    raw_candidates
        .into_iter()
        .map(|item| {
            let mut obj = match item {
                Value::Object(m) => m,
                other => {
                    let mut m = Map::new();
                    m.insert("raw".into(), other);
                    m
                }
            };
            let key = obj.get("key").and_then(Value::as_str).unwrap_or("").to_string();
            let read_status = paper_statuses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "not_read".to_string());
            let read_badge = if read_status == "not_read" {
                BADGE_NOT_READ.to_string()
            } else {
                read_status.replace('_', " ")
            };
            obj.insert("read_status".into(), Value::String(read_status));
            obj.insert("read_badge".into(), Value::String(read_badge));
            if obj.get("in_library").and_then(Value::as_bool).unwrap_or(false) {
                obj.insert("library_badge".into(), Value::String(BADGE_IN_LIBRARY.into()));
            }
            Value::Object(obj)
        })
        .collect()
}

fn compute_empty_state(
    offline: bool,
    stages: &[StageRecord],
    finished: Option<&Value>,
    candidates: &[Value],
) -> Value {
    for rec in stages {
        if rec.stage != IdeaStage::Idea
            && rec.stage != IdeaStage::Search
            && rec.run_state == Some(RunState::Failed)
        {
            let msg = if candidates.is_empty() {
                format!("Model call failed at {}: Retry", rec.stage.label())
            } else {
                format!(
                    "Model call failed at {}: candidates are still listed; Retry",
                    rec.stage.label()
                )
            };
            return json!({
                "code": "stage_failed",
                "stage": rec.stage.as_str(),
                "message": msg,
            });
        }
    }

    if offline {
        return json!({
            "code": "offline_library_only",
            "message": EMPTY_OFFLINE_MESSAGE,
        });
    }

    let results = finished
        .and_then(|f| f.get("results").or_else(|| f.get("queries")))
        .and_then(Value::as_array);
    if let Some(arr) = results {
        let attempted = arr
            .iter()
            .filter(|r| {
                !matches!(
                    r.get("status").and_then(Value::as_str),
                    Some("skipped_budget" | "not_configured" | "disabled")
                )
            })
            .count();
        let ok = arr
            .iter()
            .filter(|r| r.get("status").and_then(Value::as_str) == Some("ok"))
            .count();
        if attempted > 0 && ok == 0 {
            return json!({
                "code": "all_services_failed",
                "message": EMPTY_ALL_FAILED_MESSAGE,
            });
        }
        if ok > 0 && candidates.is_empty() {
            return json!({
                "code": "zero_candidates",
                "message": EMPTY_ZERO_CANDIDATES_MESSAGE,
            });
        }
    }

    Value::Null
}
