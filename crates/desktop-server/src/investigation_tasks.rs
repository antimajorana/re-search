//! Host task lifecycle and HTTP handler for Investigate v2 tasks
//! (`POST /api/sessions/{id}/investigation/tasks`, package V6).
//!
//! Enforces deterministic-first gating (rule 6.1), task call caps and F9 model-call
//! accounting (rule 6.15), F4 `context.supplied` recording (rule 6.17), and explicit
//! `investigation.task_requested` / `investigation.task_finished` lifecycle markers
//! (`investigate.md` §4.4, §5.2–5.3).

use crate::investigation::{is_investigation, latest};
use crate::investigation_skeleton::{EVENT_INVESTIGATION_PAPER, EVENT_INVESTIGATION_SKELETON};
use crate::investigation_task_context::{
    build_task_context, record_context_supplied, DEFAULT_CONTEXT_CHAR_BUDGET,
};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use ledger::{Event, Ledger};
use research_tools::investigation_markers::{
    InvestigationTaskKind, EVENT_INVESTIGATION_READING, MARKER_TASK_FINISHED,
    MARKER_TASK_REQUESTED,
};
use research_tools::lines_sent::SuppliedRange;
use research_tools::run_budget::{
    count_model_calls, investigate_task_run_state, InvestigateTaskState, RunScope,
};
use research_tools::skeleton::PaperSkeleton;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Internal request field carrying the validated Investigate v2 task spec from `handle`
/// to `prepare_task` (`investigate.md` §5.2). Stripped from untrusted HTTP callers.
pub const TASK_SPEC_FIELD: &str = "_investigation_task_spec";
/// Internal request field storing the `investigation.task_requested` ledger sequence number
/// (`investigate.md` §4.4).
pub const TASK_REQUEST_SEQ_FIELD: &str = "_investigation_task_request";
/// Internal request field storing the task's model-call cap (`investigate.md` §5.3, rule 6.15).
pub const TASK_CAP_FIELD: &str = "_investigation_task_cap";
/// Internal request field carrying serialized [`SuppliedRange`] entries injected into the
/// turn prompt (`investigate.md` §5.1, rule 6.17).
pub const TASK_SUPPLIED_RANGES_FIELD: &str = "_investigation_task_supplied_ranges";

const EVENT_SOURCE_INGESTED: &str = "source.ingested";
const MAX_DEPTH_CHARS: usize = 64;
const MAX_FOCUS_CHARS: usize = 2_000;
const MAX_NEARBY_TAGS: usize = 8;

/// Returns `true` when `req` represents an Investigate v2 task turn.
pub fn is_task_turn(req: &Value) -> bool {
    req.get(TASK_SPEC_FIELD).is_some() || req.get(TASK_REQUEST_SEQ_FIELD).is_some()
}

/// Default model-call cap for `task` (`investigate.md` §5.3).
pub fn task_cap_for(task: InvestigationTaskKind) -> u32 {
    task.default_call_cap()
}

/// HTTP handler for `POST /api/sessions/{id}/investigation/tasks` (`investigate.md` §5.2).
pub fn handle(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(storage)? {
        return Err(not_found("Session missing."));
    }
    let mut req_json = parse_json_body(body)?;
    if !req_json.is_object() {
        return Err(bad("Task request body must be a JSON object."));
    }

    let task = parse_task_kind(req_json.get("task").and_then(Value::as_str))?;
    let depth = parse_optional_text(req_json.get("depth"), "depth", MAX_DEPTH_CHARS)?;
    let focus = parse_optional_text(req_json.get("focus"), "focus", MAX_FOCUS_CHARS)?;

    let ledger = state.open_ledger().map_err(storage)?;
    let events = ledger.read_session(session_id).map_err(storage)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before starting an Investigate task."));
    }
    require_map_tools_profile(&events)?;

    // Rule 6.1 (deterministic first): refuse any task until a pinned skeleton exists
    // (auto-building from hash-verified ingested source on older pinned Investigation sessions per §8.4).
    let (source_id, skeleton_seq, skeleton) =
        load_pinned_skeleton(state, session_id, &ledger, &events, None)?;
    let tags = parse_and_validate_tags(req_json.get("tags"), task, &skeleton)?;
    let cap = clamp_cap(req_json.get("max_turns").and_then(Value::as_u64), task_cap_for(task));
    let prev_last_seq = events.last().map_or(0, |e| e.seq).max(skeleton_seq);

    strip_internal_task_fields(&mut req_json);
    req_json[TASK_SPEC_FIELD] = json!({
        "task": task.as_str(), "tags": tags, "depth": depth, "focus": focus,
        "source_id": source_id, "skeleton_seq": skeleton_seq, "cap": cap,
    });
    req_json["max_turns"] = json!(cap);

    let prompt = build_task_prompt(task, &tags, focus.as_deref());
    drop(ledger);
    crate::turn_runner::spawn_task_turn_thread(Arc::clone(state), session_id.to_owned(), req_json, prompt)?;

    let req_seq = state.open_ledger().ok()
        .and_then(|l| l.read_session(session_id).ok())
        .and_then(|evs| {
            evs.iter().rev()
                .find(|e| e.seq > prev_last_seq && e.kind == MARKER_TASK_REQUESTED)
                .or_else(|| evs.iter().rev().find(|e| e.kind == MARKER_TASK_REQUESTED))
                .map(|e| e.seq)
        });

    Ok((202, json!({
        "session": session_id, "status": "running", "task": task.as_str(),
        "request_seq": req_seq, "source_id": source_id, "skeleton_seq": skeleton_seq, "cap": cap,
    })))
}

/// Host prepare hook called before starting a turn thread for an Investigate v2 task.
///
/// Re-verifies the pinned paper, skeleton blob hash, and source receipt, appends
/// `investigation.task_requested`, builds quoted source context, and populates internal
/// turn fields on `req`. Returns `Ok(false)` if `req` is not an Investigate v2 task.
pub(crate) fn prepare_task(
    state: &DesktopState,
    session_id: &str,
    req: &mut Value,
) -> Result<bool, (u16, String)> {
    let Some(spec) = req.get(TASK_SPEC_FIELD).cloned() else {
        strip_internal_task_fields(req);
        return Ok(false);
    };

    let task = parse_task_kind(spec.get("task").and_then(Value::as_str))?;
    let expected_source = spec.get("source_id").and_then(Value::as_str)
        .map(str::trim).filter(|s| !s.is_empty())
        .ok_or_else(|| bad("Investigate task spec missing `source_id`."))?.to_string();
    let expected_skel_seq = spec.get("skeleton_seq").and_then(Value::as_u64)
        .ok_or_else(|| bad("Investigate task spec missing `skeleton_seq`."))?;
    let depth = parse_optional_text(spec.get("depth"), "depth", MAX_DEPTH_CHARS)?;
    let focus = parse_optional_text(spec.get("focus"), "focus", MAX_FOCUS_CHARS)?;
    let cap = clamp_cap(spec.get("cap").and_then(Value::as_u64), task_cap_for(task));

    let ledger = state.open_ledger().map_err(storage)?;
    let events = ledger.read_session(session_id).map_err(storage)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before starting an Investigate task."));
    }
    require_map_tools_profile(&events)?;
    let (source_id, skeleton_seq, skeleton) =
        load_pinned_skeleton(state, session_id, &ledger, &events, Some(expected_skel_seq))?;
    if source_id != expected_source {
        return Err(conflict("The pinned paper changed before the Investigate task started."));
    }
    let tags = parse_and_validate_tags(spec.get("tags"), task, &skeleton)?;
    let receipt = find_source_receipt(&events, &source_id, Some(skeleton.source_seq))
        .or_else(|| find_source_receipt(&events, &source_id, None))
        .ok_or_else(|| not_found(format!("source receipt `{source_id}` not found")))?;
    let env = state.research_env_for_session(session_id).map_err(internal)?;

    let marker_payload = json!({
        "task": task.as_str(), "tags": tags, "depth": depth, "focus": focus,
        "source_id": source_id, "skeleton_seq": skeleton_seq, "cap": cap,
    });
    let request_seq = {
        let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
        ledger.append(session_id, MARKER_TASK_REQUESTED, 1, &marker_payload).map_err(storage)?
    };

    let built = match build_task_context(
        &env.sandbox(), session_id, receipt, &skeleton, skeleton_seq, request_seq,
        task, &tags, depth.as_deref(), focus.as_deref(), DEFAULT_CONTEXT_CHAR_BUDGET,
    ) {
        Ok(ctx) => ctx,
        Err(err) => {
            // Close the just-opened marker so no dangling active task remains.
            let fail_payload = json!({
                "request_seq": request_seq,
                "status": InvestigateTaskState::Failed.as_str(),
                "run_state": investigate_task_run_state(InvestigateTaskState::Failed),
                "counts": { "items": 0, "edges": 0, "rejected_items": 0, "rejected_edges": 0, "model_calls": 0, "cap": cap },
                "error": err,
            });
            if let Ok(_lock) = state.ledger_write_lock.lock() {
                let _ = ledger.append(session_id, MARKER_TASK_FINISHED, 1, &fail_payload);
            }
            return Err(internal(err));
        }
    };

    let ranges_json = serde_json::to_value(&built.supplied_ranges).map_err(storage)?;
    strip_internal_task_fields(req);
    req[TASK_REQUEST_SEQ_FIELD] = json!(request_seq);
    req[TASK_CAP_FIELD] = json!(cap);
    req[TASK_SUPPLIED_RANGES_FIELD] = ranges_json;
    req["_paper_context"] = json!(built.instructions);
    req["max_turns"] = json!(cap);
    req["verification_mode"] = json!(false);
    req["calculation_mode"] = json!(false);
    req["use_strategist"] = json!(false);
    Ok(true)
}

/// Closes an opened `investigation.task_requested` marker as `failed` if spawning the
/// background solver thread fails before the worker starts.
pub(crate) fn fail_task_on_spawn_error(
    state: &DesktopState,
    session_id: &str,
    req: &Value,
    err: &str,
) {
    let Some(request_seq) = req.get(TASK_REQUEST_SEQ_FIELD).and_then(Value::as_u64) else {
        return;
    };
    let cap = req.get(TASK_CAP_FIELD).and_then(Value::as_u64).unwrap_or(8);
    let payload = json!({
        "request_seq": request_seq,
        "status": InvestigateTaskState::Failed.as_str(),
        "run_state": investigate_task_run_state(InvestigateTaskState::Failed),
        "counts": {
            "items": 0, "edges": 0, "rejected_items": 0, "rejected_edges": 0,
            "model_calls": 0, "cap": cap,
        },
        "error": err,
    });
    if let (Ok(ledger), Ok(_lock)) = (state.open_ledger(), state.ledger_write_lock.lock()) {
        let _ = ledger.append(session_id, MARKER_TASK_FINISHED, 1, &payload);
    }
}

/// Records the F4 `context.supplied` ledger event crediting injected lines to `caller`.
pub(crate) fn record_supplied_for_caller(
    state: &DesktopState,
    session_id: &str,
    req: &Value,
    caller: &str,
) -> Result<(), String> {
    let Some(request_seq) = req.get(TASK_REQUEST_SEQ_FIELD).and_then(Value::as_u64) else {
        return Ok(());
    };
    let Some(ranges_val) = req.get(TASK_SUPPLIED_RANGES_FIELD) else {
        return Ok(());
    };
    let ranges: Vec<SuppliedRange> = serde_json::from_value(ranges_val.clone())
        .map_err(|e| format!("invalid supplied ranges in task request: {e}"))?;
    if ranges.is_empty() {
        return Ok(());
    }
    let ledger = state.open_ledger()?;
    let _lock = state.ledger_write_lock.lock().map_err(|_| "ledger lock poisoned".to_string())?;
    let _ = record_context_supplied(&ledger, session_id, request_seq, caller, &ranges)?;
    Ok(())
}

/// Host finish hook recording `investigation.task_finished` with F9 model-call counts,
/// saved reading counts, and rule 6.15 / overview 3.3 state transitions.
pub(crate) fn finish_task(
    state: &DesktopState,
    session_id: &str,
    req: &Value,
    outcome: Result<String, String>,
    cancelled: bool,
) -> Result<String, String> {
    let Some(request_seq) = req.get(TASK_REQUEST_SEQ_FIELD).and_then(Value::as_u64) else {
        return outcome;
    };
    let cap = req.get(TASK_CAP_FIELD).and_then(Value::as_u64).unwrap_or(8);

    let ledger = state.open_ledger()?;
    let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let model_calls = count_model_calls(&events, RunScope::AfterSeq(request_seq));

    let (mut saved_items, mut saved_edges, mut saved_limitations) = (0u64, 0u64, 0u64);
    let (mut rejected_items, mut rejected_edges, mut any_done) = (0u64, 0u64, false);
    for ev in events.iter().filter(|e| {
        e.kind == EVENT_INVESTIGATION_READING
            && e.seq > request_seq
            && e.payload["request_seq"].as_u64() == Some(request_seq)
    }) {
        saved_items += ev.payload["items"].as_array().map_or(0, Vec::len) as u64;
        saved_edges += ev.payload["edges"].as_array().map_or(0, Vec::len) as u64;
        saved_limitations += ev.payload["limitations"]
            .as_array()
            .map_or(0, |a| a.iter().filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())).count())
            as u64;
        rejected_items += ev.payload["rejected"]["items"].as_array().map_or(0, Vec::len) as u64;
        rejected_edges += ev.payload["rejected"]["edges"].as_array().map_or(0, Vec::len) as u64;
        if ev.payload["done"].as_bool() == Some(true) {
            any_done = true;
        }
    }

    let has_saved = saved_items > 0 || saved_edges > 0;
    let has_documented_done = any_done && (has_saved || saved_limitations > 0);
    let stopped_by_user = cancelled
        || outcome.as_ref().err().is_some_and(|e| {
            let low = e.to_ascii_lowercase();
            low.contains("stopped by user") || low.contains("stopped by the user")
        });

    let (status, final_outcome) = if stopped_by_user {
        (InvestigateTaskState::Stopped, outcome)
    } else {
        match outcome {
            Ok(reply) if has_saved || has_documented_done => (InvestigateTaskState::Done, Ok(reply)),
            Ok(_) => (
                InvestigateTaskState::Failed,
                Err("The model finished without saving any source-anchored reading records.".into()),
            ),
            Err(err) if has_saved => (InvestigateTaskState::Partial, Err(err)),
            Err(err) if is_interruption_error(&err) => (InvestigateTaskState::Interrupted, Err(err)),
            Err(err) => (InvestigateTaskState::Failed, Err(err)),
        }
    };

    let run_state = investigate_task_run_state(status);
    let payload = json!({
        "request_seq": request_seq,
        "status": status.as_str(),
        "run_state": run_state,
        "counts": {
            "items": saved_items, "edges": saved_edges,
            "rejected_items": rejected_items, "rejected_edges": rejected_edges,
            "model_calls": model_calls, "cap": cap,
        },
        "error": final_outcome.as_ref().err(),
    });
    {
        let _lock = state.ledger_write_lock.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        ledger.append(session_id, MARKER_TASK_FINISHED, 1, &payload).map_err(|e| e.to_string())?;
    }
    final_outcome
}

fn require_map_tools_profile(events: &[Event]) -> Result<(), (u16, String)> {
    let profile = crate::session_mode::profile_for_session(events).map_err(bad)?;
    if profile.def_for(research_tools::map_defs::READ_MAP_NODE).is_none()
        || profile.def_for(research_tools::map_defs::RECORD_MAP_READING).is_none()
    {
        return Err(bad(format!(
            "Session profile `{}` does not include `read_map_node` and `record_map_reading`; open a new Investigate session (`investigation/2`) to run v2 tasks.",
            profile.name
        )));
    }
    Ok(())
}

fn storage(err: impl ToString) -> (u16, String) {
    internal(err.to_string())
}

fn parse_task_kind(raw: Option<&str>) -> Result<InvestigationTaskKind, (u16, String)> {
    let name = raw.map(str::trim).filter(|s| !s.is_empty())
        .ok_or_else(|| bad("Provide `task` (`orient`, `open`, or `context`)."))?;
    if name == "prior_work" {
        return Err(bad("Prior-work tasks inside Investigate are deferred (V14); use Explore instead."));
    }
    match InvestigationTaskKind::from_str_exact(name) {
        Some(k @ (InvestigationTaskKind::Orient | InvestigationTaskKind::Open | InvestigationTaskKind::Context)) => Ok(k),
        _ => Err(bad(format!("Unknown Investigate task `{name}`; expected `orient`, `open`, or `context`."))),
    }
}

fn parse_optional_text(val: Option<&Value>, field: &str, limit: usize) -> Result<Option<String>, (u16, String)> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() { return Ok(None); }
            if trimmed.chars().count() > limit {
                return Err(bad(format!("`{field}` must be at most {limit} characters.")));
            }
            Ok(Some(trimmed.to_string()))
        }
        Some(_) => Err(bad(format!("`{field}` must be a string."))),
    }
}

fn parse_and_validate_tags(
    val: Option<&Value>, task: InvestigationTaskKind, skeleton: &PaperSkeleton,
) -> Result<Vec<String>, (u16, String)> {
    let mut tags = Vec::new();
    let mut seen = HashSet::new();
    match val {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for item in items {
                let raw = item.as_str().map(str::trim).filter(|s| !s.is_empty())
                    .ok_or_else(|| bad("`tags` must be an array of non-empty tag strings."))?;
                if seen.insert(raw.to_string()) {
                    tags.push(raw.to_string());
                }
            }
        }
        Some(_) => return Err(bad("`tags` must be an array of tag strings.")),
    }
    if matches!(task, InvestigationTaskKind::Open | InvestigationTaskKind::Context) && tags.is_empty() {
        return Err(bad(format!("Task `{}` requires at least one skeleton tag in `tags`.", task.as_str())));
    }

    let valid_tags: Vec<&str> = skeleton.nodes.iter().filter_map(|n| n.tag.as_deref()).collect();
    let valid_set: HashSet<&str> = valid_tags.iter().copied().collect();
    for tag in &tags {
        if !valid_set.contains(tag.as_str()) {
            let nearby = suggest_nearby_tags(tag, &valid_tags);
            let hint = if nearby.is_empty() { String::new() } else { format!(" Valid tags include: {}.", nearby.join(", ")) };
            return Err(bad(format!("Unknown skeleton tag `{tag}` for this pinned paper.{hint}")));
        }
    }
    Ok(tags)
}

fn suggest_nearby_tags<'a>(needle: &str, valid: &[&'a str]) -> Vec<&'a str> {
    let low = needle.to_ascii_lowercase();
    let mut out: Vec<&str> = valid.iter().copied()
        .filter(|t| { let tl = t.to_ascii_lowercase(); tl.contains(&low) || low.contains(&tl) })
        .take(MAX_NEARBY_TAGS).collect();
    for &t in valid {
        if out.len() >= MAX_NEARBY_TAGS { break; }
        if !out.contains(&t) { out.push(t); }
    }
    out
}

fn load_pinned_skeleton(
    state: &DesktopState,
    session_id: &str,
    ledger: &Ledger,
    events: &[Event],
    required_seq: Option<u64>,
) -> Result<(String, u64, PaperSkeleton), (u16, String)> {
    const NEED_SKEL: &str = "Build or pin the deterministic paper skeleton before starting an Investigate task.";
    let source_id = latest(events, EVENT_INVESTIGATION_PAPER)
        .and_then(|e| e.payload["source_id"].as_str()).map(str::trim).filter(|s| !s.is_empty())
        .ok_or_else(|| bad(NEED_SKEL))?.to_string();

    let skel_ev = match required_seq {
        Some(seq) => events.iter().find(|e| {
            e.seq == seq && e.kind == EVENT_INVESTIGATION_SKELETON && e.payload["source_id"].as_str() == Some(source_id.as_str())
        }).ok_or_else(|| conflict(format!("Pinned skeleton seq {seq} is no longer available.")))?,
        None => {
            if let Some(ev) = events.iter().rev().find(|e| {
                e.kind == EVENT_INVESTIGATION_SKELETON && e.payload["source_id"].as_str() == Some(source_id.as_str())
            }) {
                ev
            } else {
                // Older Investigation sessions (`investigate.md` §8.4) may have `investigation.paper`
                // and hash-verified `source.ingested` files without an `investigation.skeleton` event yet.
                let built = crate::investigation_skeleton::ensure_skeleton_on_open(state, session_id)?
                    .filter(|rec| rec.skeleton.source_id == source_id)
                    .ok_or_else(|| bad(NEED_SKEL))?;
                return Ok((source_id, built.event_seq, built.skeleton));
            }
        }
    };

    let blob_hash = skel_ev.payload["blob"].as_str()
        .ok_or_else(|| internal("stored investigation.skeleton missing `blob` hash"))?;
    let bytes = ledger.get_blob(blob_hash)
        .map_err(|e| internal(format!("failed to read skeleton blob `{blob_hash}`: {e}")))?;
    if ledger::sha256_hex(&bytes) != blob_hash {
        return Err(internal(format!("skeleton blob hash mismatch for `{blob_hash}`")));
    }
    let skeleton: PaperSkeleton = serde_json::from_slice(&bytes)
        .map_err(|e| internal(format!("failed to decode skeleton blob: {e}")))?;
    if skeleton.source_id != source_id {
        return Err(internal("skeleton blob source_id does not match pinned paper"));
    }
    Ok((source_id, skel_ev.seq, skeleton))
}

fn find_source_receipt<'a>(events: &'a [Event], source_id: &str, seq: Option<u64>) -> Option<&'a Event> {
    events.iter().rev().find(|e| {
        e.kind == EVENT_SOURCE_INGESTED
            && e.payload["source_id"].as_str() == Some(source_id)
            && seq.is_none_or(|s| e.seq == s)
    })
}

fn clamp_cap(requested: Option<u64>, default_cap: u32) -> u32 {
    match requested {
        Some(n) if n >= 1 && n <= u64::from(default_cap) => n as u32,
        _ => default_cap,
    }
}

pub(crate) fn strip_internal_task_fields(req: &mut Value) {
    if let Some(obj) = req.as_object_mut() {
        obj.retain(|k, _| {
            k != TASK_SPEC_FIELD
                && k != TASK_REQUEST_SEQ_FIELD
                && k != TASK_CAP_FIELD
                && k != TASK_SUPPLIED_RANGES_FIELD
                && !k.starts_with("_investigation_task_")
        });
    }
}

fn build_task_prompt(task: InvestigationTaskKind, tags: &[String], focus: Option<&str>) -> String {
    let mut prompt = match task {
        InvestigationTaskKind::Orient => "Run Investigate orient task on pinned paper.".to_string(),
        InvestigationTaskKind::Open => format!("Open Investigate node(s): {}.", tags.join(", ")),
        InvestigationTaskKind::Context => format!("Gather context for Investigate node(s): {}.", tags.join(", ")),
        InvestigationTaskKind::PriorWork => "Check prior work on pinned paper.".to_string(),
    };
    if let Some(f) = focus {
        prompt.push_str("\nFocus: ");
        prompt.push_str(f);
    }
    prompt
}

fn is_interruption_error(err: &str) -> bool {
    let low = err.to_ascii_lowercase();
    low.contains("transport")
        || low.contains("interrupted")
        || low.contains("connection")
        || low.contains("timed out")
        || low.contains("timeout")
}
