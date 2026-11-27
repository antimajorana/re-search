//! HTTP route handlers for Explore mode (`/api/idea/preview`, `POST /api/sessions` with
//! `mode: "explore"`, and `POST /api/sessions/{id}/idea/{op}`, package X3).
//!
//! Every route here sits behind `AuthGuard::authorize_api` in `http_srv.rs` / `api_dispatch.rs`.
//! Zero model calls occur in X3/X4 (`explore/1` stays `NotBuiltYet` until X7).

use crate::idea::{
    append_explore_session_start, build_idea_statement, is_single_paper_identifier,
    normalize_fields, preview_first_queries, resolve_stages, run_first_search,
    search_library_matches, validate_first_queries, IdeaPreset, SubmittedFirstQuery,
    EVENT_IDEA_BUDGET_RAISED, EVENT_IDEA_NOTE, EVENT_IDEA_STAGE, EVENT_IDEA_STATED,
    MAX_NOTE_COMMENT_CHARS, MAX_PREVIEW_LIBRARY_MATCHES, NOTE_ACTIONS, NOTE_TARGET_KINDS,
};
use crate::idea_view;
use crate::projects::save_new_session;
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::session_mode::{api_mode_name, session_mode};
use crate::state::{now_ms, DesktopState, SessionMeta, DEFAULT_SESSION_TITLE};
use agent::CancelToken;
use research_tools::idea_terms::{extract as extract_idea_terms, MAX_IDEA_TEXT_CHARS};
use research_tools::paper_fetch::parse_paper_input;
use research_tools::paper_key::{Mode, PaperKey};
use research_tools::run_budget::ExploreStageState;
use research_tools::{validate_session_id, ResearchEnv};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tools::canon::random_hex;
use tools::EVENT_SCHEMA_VERSION;

const MAX_BUDGET_BUMP_QUERIES: u64 = 36;
const MAX_BUDGET_BUMP_PAPERS: u64 = 12;
const MAX_BUDGET_BUMP_CALLS: u64 = 60;

fn not_implemented(msg: impl Into<String>) -> (u16, String) {
    (501, msg.into())
}

/// RAII guard registering an active Explore search in `active_turns` + `api_turns` so
/// `GET /api/sessions/{id}` sees `running: true` (preventing false Rule S3 `"Interrupted"`)
/// and `/idea/stop` can cooperatively cancel in-flight queries.
pub(crate) struct ActiveExploreGuard {
    state: Arc<DesktopState>,
    session_id: String,
    pub(crate) cancel: CancelToken,
}

impl ActiveExploreGuard {
    pub(crate) fn acquire(state: &Arc<DesktopState>, session_id: &str) -> Result<Self, (u16, String)> {
        let cancel = CancelToken::new();
        {
            let mut active = state
                .active_turns
                .lock()
                .map_err(|_| internal("active_turns lock poisoned"))?;
            if !active.insert(session_id.to_string()) {
                return Err(conflict(format!("session `{session_id}` already has an active run")));
            }
        }
        if let Ok(mut api) = state.api_turns.lock() {
            api.insert(session_id.to_string(), cancel.clone());
        }
        Ok(Self { state: Arc::clone(state), session_id: session_id.to_string(), cancel })
    }
}

impl Drop for ActiveExploreGuard {
    fn drop(&mut self) {
        if let Ok(mut api) = self.state.api_turns.lock() {
            api.remove(&self.session_id);
        }
        if let Ok(mut active) = self.state.active_turns.lock() {
            active.remove(&self.session_id);
        }
    }
}

/// `POST /api/idea/preview` — deterministic local preview with zero network and zero sessions.
pub fn handle_idea_preview(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let raw_text = match val.get("text") {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(bad("`text` must be a string")),
    };
    let trimmed = raw_text.trim();
    let raw_fields = parse_string_list(val.get("fields"), "fields")?;
    let fields = normalize_fields(&raw_fields).map_err(bad)?;
    let raw_seeds = parse_string_list(val.get("seeds"), "seeds")?;
    let limit = val
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| (n as usize).clamp(1, MAX_PREVIEW_LIBRARY_MATCHES))
        .unwrap_or(MAX_PREVIEW_LIBRARY_MATCHES);

    let (mut seeds, mut invalid_seeds) = (Vec::new(), Vec::new());
    for s in &raw_seeds {
        let t = s.trim();
        if t.is_empty() { continue; }
        match PaperKey::parse(t).or_else(|_| parse_paper_input(t).map(|r| r.key)) {
            Ok(k) => seeds.push(k),
            Err(_) => invalid_seeds.push(t.to_string()),
        }
    }

    let char_count = trimmed.chars().count();
    let truncated = char_count > MAX_IDEA_TEXT_CHARS;
    let extracted = extract_idea_terms(trimmed);
    let focus_investigate = is_single_paper_identifier(trimmed);
    let first_queries = preview_first_queries(&extracted, &fields);
    let library_matches = search_library_matches(&state.cfg.data_dir, &extracted, &seeds, limit);

    let presets: Vec<Value> = IdeaPreset::ALL
        .iter()
        .map(|&p| {
            let b = p.default_budget();
            json!({
                "id": p.as_str(), "label": p.label(),
                "max_queries": b.max_queries, "max_screened": b.max_screened,
                "max_papers": b.max_papers, "calls_per_paper": b.calls_per_paper,
                "typical_model_calls": b.typical_model_calls, "max_model_calls": b.max_model_calls,
            })
        })
        .collect();

    Ok((
        200,
        json!({
            "char_count": char_count,
            "max_chars": MAX_IDEA_TEXT_CHARS,
            "truncated": truncated,
            "focus_investigate": focus_investigate,
            "terms": extracted.terms,
            "paper_refs": extracted.paper_refs,
            "quoted_phrases": extracted.quoted_phrases,
            "proper_names": extracted.proper_names,
            "math_tokens": extracted.math_tokens,
            "keywords": extracted.keywords,
            "fields": fields,
            "seeds": seeds,
            "invalid_seeds": invalid_seeds,
            "library_matches": library_matches,
            "first_queries": first_queries,
            "presets": presets,
        }),
    ))
}

/// `POST /api/sessions` with `mode: "explore"` — validates Rule P4 first queries, appends
/// `idea.started` + `idea.stated`, executes permitted user-shown queries (or offline library pass),
/// streams SSE progress, and returns the new session plus initial `idea_view`.
pub fn create_explore_session(state: &Arc<DesktopState>, val: &Value) -> ApiResult {
    let raw_text = val
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`text` must be a non-empty string"))?;
    let raw_fields = parse_string_list(val.get("fields"), "fields")?;
    let raw_seeds = parse_string_list(val.get("seeds"), "seeds")?;
    let stmt = build_idea_statement(1, raw_text, &raw_fields, &raw_seeds).map_err(bad)?;

    let preset = match val.get("preset").filter(|v| !v.is_null()) {
        None => IdeaPreset::Standard,
        Some(Value::String(s)) => IdeaPreset::parse(s).map_err(bad)?,
        Some(_) => return Err(bad("`preset` must be a string")),
    };
    let offline = match val.get("offline").filter(|v| !v.is_null()) {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad("`offline` must be a boolean")),
    };

    let submitted_queries = parse_submitted_queries(val.get("queries"))?;
    let extracted = extract_idea_terms(&stmt.text);
    // Rule P4: reject omitted/mutated/extra/disallowed queries BEFORE creating any session.
    let planned = validate_first_queries(
        &extracted,
        &stmt.fields,
        submitted_queries.as_deref(),
        offline,
    )
    .map_err(bad)?;

    let id = match opt_string(val.get("id"), "id")? {
        Some(raw) if !raw.trim().is_empty() => {
            let trimmed = raw.trim();
            validate_session_id(trimmed).map_err(|e| bad(e.to_string()))?;
            trimmed.to_string()
        }
        _ => format!("s-{}", random_hex(8)),
    };
    let ledger = state.open_ledger().map_err(internal)?;
    let has_ledger_events = !ledger.read_session(&id).map_err(|e| internal(e.to_string()))?.is_empty();
    if state.session_exists(&id).map_err(internal)? || has_ledger_events {
        return Err(conflict(format!("session `{id}` already exists")));
    }

    let project_id = opt_string(val.get("project_id"), "project_id")?;
    let ws_opt = opt_string(val.get("workspace_root"), "workspace_root")?;
    let workspace_root = ws_opt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| state.cfg.workspace_root.as_ref().map(|p| p.display().to_string()));
    if let Some(ws) = &workspace_root {
        let p = PathBuf::from(ws);
        if !p.exists() || !p.is_dir() {
            return Err(bad(format!("workspace_root `{ws}` does not exist or is not a directory")));
        }
        ResearchEnv::with_workspace(state.cfg.data_dir.clone(), &p).map_err(|e| bad(e.to_string()))?;
    }

    let ui = state.load_ui_settings();
    let explicit_title = opt_string(val.get("title"), "title")?
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let title = explicit_title
        .map(str::to_string)
        .or_else(|| {
            let c = clip_title(&stmt.text, 52);
            (!c.is_empty()).then_some(c)
        })
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string());

    let provider = opt_string(val.get("provider"), "provider")?.map(str::trim).filter(|s| !s.is_empty());
    let model_req = opt_string(val.get("model"), "model")?.map(str::trim).filter(|s| !s.is_empty());
    let route = provider
        .and_then(|p| crate::cli_turn::CliRouteKind::from_provider(&p.to_ascii_lowercase()))
        .map(|k| k.provider_id().to_string());
    let meta = SessionMeta {
        id: id.clone(),
        title: title.clone(),
        allow_model_title: Some(false),
        workspace_root: workspace_root.clone(),
        workspace_detached: false,
        pinned_conventions: ui.pinned_conventions,
        verification_mode: false,
        calculation_mode: false,
        model_preset: None,
        default_provider: provider.map(str::to_string).or(ui.default_provider),
        default_model: model_req.map(str::to_string).or(ui.default_model),
        reasoning_effort: None,
        created_ms: now_ms(),
        fork_info: None,
        route,
        archived: false,
        deleted: false,
    };
    save_new_session(state, &meta, project_id)?;

    if let Err(err) = append_explore_session_start(&ledger, &id, preset, offline, &stmt) {
        if let Ok(path) = state.meta_path(&id) {
            let _ = std::fs::remove_file(path);
        }
        return Err(internal(err));
    }
    state.sse.emit(&id, "session_event", json!({ "session": id, "kind": "idea.started" }));

    let guard = ActiveExploreGuard::acquire(state, &id)?;
    let sse = Arc::clone(&state.sse);
    let sid_for_sse = id.clone();
    let search_res = run_first_search(
        &state.cfg.data_dir,
        &state.cfg.literature_endpoints,
        &ledger,
        &id,
        &stmt,
        &planned,
        offline,
        Some(&guard.cancel),
        move |kind, payload| {
            sse.emit(
                &sid_for_sse,
                "session_event",
                json!({ "session": sid_for_sse, "kind": kind, "payload": payload }),
            );
        },
    );
    drop(guard);
    search_res.map_err(internal)?;

    let events = ledger.read_session(&id).map_err(|e| internal(e.to_string()))?;
    let view = idea_view::view(&events, false);
    Ok((
        200,
        json!({
            "id": id,
            "title": title,
            "workspace_root": workspace_root,
            "created_ms": meta.created_ms,
            "mode": api_mode_name(Mode::Explore),
            "route": meta.route,
            "project_id": project_id,
            "idea": view,
        }),
    ))
}

/// Single dispatcher for `POST /api/sessions/{id}/idea/{op}` (`explore.md` 5.4).
pub fn handle_idea_op(
    state: &Arc<DesktopState>,
    session_id: &str,
    op: &str,
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    if session_mode(&events) != Mode::Explore {
        return Err(bad(format!("session `{session_id}` is not an Explore session")));
    }

    match op {
        "revise" => handle_revise(state, &ledger, session_id, body),
        "note" => handle_note(state, &ledger, session_id, body),
        "stop" => handle_stop(state, &ledger, session_id),
        "continue" => handle_continue(state, &ledger, session_id, body),
        "sharpen" => crate::idea_exchanges::handle_sharpen_op(state, &ledger, session_id, body),
        "answer" => crate::idea_answer::handle_answer_op(state, &ledger, session_id, body),
        "plan" => crate::idea_plan_search::handle_plan_op(state, &ledger, session_id, body),
        "search" => crate::idea_plan_search::handle_search_op(state, &ledger, session_id, body),
        "triage" => Err(not_implemented("Explore `triage` is not built yet (owned by package X8).")),
        "read" => Err(not_implemented("Explore `read` is not built yet (owned by packages X9–X10).")),
        "map" => Err(not_implemented("Explore `map` is not built yet (owned by package X11).")),
        "directions" => Err(not_implemented("Explore `directions` is not built yet (owned by package X13).")),
        "handoff" => Err(not_implemented(
            "Explore `handoff` builder is not built yet (owned by package X13; use POST /api/sessions/{id}/handoff for F7 packets).",
        )),
        "more" => Err(not_implemented(
            "Explore `more` (citation expansion / second round) is not built yet (owned by package X16).",
        )),
        other => Err(not_found(format!("unknown Explore operation `{other}`"))),
    }
}

fn handle_revise(
    state: &DesktopState,
    ledger: &ledger::Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    if is_turn_running(state, session_id) {
        return Err(conflict("cannot revise the idea while an Explore run is active"));
    }
    let val = parse_json_body(body)?;
    let raw_text = val
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`text` must be a non-empty string"))?;
    let raw_fields = parse_string_list(val.get("fields"), "fields")?;
    let raw_seeds = parse_string_list(val.get("seeds"), "seeds")?;

    let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let prev_rev = events
        .iter()
        .filter(|e| e.kind == EVENT_IDEA_STATED)
        .filter_map(|e| e.payload.get("rev").and_then(Value::as_u64))
        .max()
        .unwrap_or(0);
    let next_rev = u32::try_from(prev_rev.saturating_add(1)).unwrap_or(u32::MAX);
    let stmt = build_idea_statement(next_rev, raw_text, &raw_fields, &raw_seeds).map_err(bad)?;
    let payload = serde_json::to_value(&stmt).map_err(|e| internal(e.to_string()))?;
    ledger
        .append(session_id, EVENT_IDEA_STATED, EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "session_event",
        json!({ "session": session_id, "kind": EVENT_IDEA_STATED, "payload": payload }),
    );
    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    Ok((200, json!({ "ok": true, "idea": idea_view::view(&updated, false) })))
}

fn handle_note(
    state: &DesktopState,
    ledger: &ledger::Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let val = parse_json_body(body)?;
    let target_kind = val
        .get("target_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| NOTE_TARGET_KINDS.contains(s))
        .ok_or_else(|| bad(format!("`target_kind` must be one of: {}", NOTE_TARGET_KINDS.join(", "))))?;
    let target_id = val
        .get("target_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().count() <= 256)
        .ok_or_else(|| bad("`target_id` must be a non-empty string (<= 256 chars)"))?;
    let action = val
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| NOTE_ACTIONS.contains(s))
        .ok_or_else(|| bad(format!("`action` must be one of: {}", NOTE_ACTIONS.join(", "))))?;
    let comment = match val.get("comment").filter(|v| !v.is_null()) {
        None => String::new(),
        Some(Value::String(s)) => {
            if s.chars().count() > MAX_NOTE_COMMENT_CHARS {
                return Err(bad(format!("`comment` exceeds {MAX_NOTE_COMMENT_CHARS} characters")));
            }
            s.trim().to_string()
        }
        Some(_) => return Err(bad("`comment` must be a string")),
    };
    if action == "comment" && comment.is_empty() {
        return Err(bad("`comment` is required when `action` is `comment`"));
    }

    let payload = json!({
        "target_kind": target_kind, "target_id": target_id, "action": action, "comment": comment,
    });
    ledger
        .append(session_id, EVENT_IDEA_NOTE, EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "session_event",
        json!({ "session": session_id, "kind": EVENT_IDEA_NOTE, "payload": payload }),
    );
    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let running = is_turn_running(state, session_id);
    Ok((200, json!({ "ok": true, "idea": idea_view::view(&updated, running) })))
}

fn handle_stop(
    state: &DesktopState,
    ledger: &ledger::Ledger,
    session_id: &str,
) -> ApiResult {
    if let Ok(api) = state.api_turns.lock() {
        if let Some(tok) = api.get(session_id) {
            tok.cancel();
        }
    }
    state.deny_pending(session_id, None, "supervisor: turn stopped by the user");
    let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    // Pass `running = true` so `Requested` stages are inspected raw rather than masked as Interrupted.
    let stages = resolve_stages(&events, true);
    let mut cancelled_stages = Vec::new();
    for rec in stages {
        if matches!(
            rec.stored_state,
            Some(ExploreStageState::Requested | ExploreStageState::PlanApproval)
        ) {
            let payload = json!({
                "stage": rec.stage.as_str(),
                "state": ExploreStageState::Cancelled.as_str(),
                "request_id": rec.request_id,
                "reason": "Stopped by user",
            });
            ledger
                .append(session_id, EVENT_IDEA_STAGE, EVENT_SCHEMA_VERSION, &payload)
                .map_err(|e| internal(e.to_string()))?;
            state.sse.emit(
                session_id,
                "session_event",
                json!({ "session": session_id, "kind": EVENT_IDEA_STAGE, "payload": payload }),
            );
            cancelled_stages.push(rec.stage.as_str().to_string());
        }
    }
    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    Ok((
        200,
        json!({
            "ok": true,
            "cancelled": cancelled_stages,
            "idea": idea_view::view(&updated, is_turn_running(state, session_id)),
        }),
    ))
}

fn handle_continue(
    state: &Arc<DesktopState>,
    ledger: &ledger::Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let val = if body.is_empty() { json!({}) } else { parse_json_body(body)? };
    let extra_queries = parse_opt_u64(val.get("extra_queries"), "extra_queries")?;
    let extra_papers = parse_opt_u64(val.get("extra_papers"), "extra_papers")?;
    let extra_calls = parse_opt_u64(val.get("extra_model_calls"), "extra_model_calls")?;

    if extra_queries > MAX_BUDGET_BUMP_QUERIES
        || extra_papers > MAX_BUDGET_BUMP_PAPERS
        || extra_calls > MAX_BUDGET_BUMP_CALLS
    {
        return Err(bad("requested budget increase exceeds per-click ceiling"));
    }

    if extra_queries > 0 || extra_papers > 0 || extra_calls > 0 {
        let payload = json!({
            "extra_queries": extra_queries,
            "extra_papers": extra_papers,
            "extra_model_calls": extra_calls,
            "author": "user",
        });
        ledger
            .append(session_id, EVENT_IDEA_BUDGET_RAISED, EVENT_SCHEMA_VERSION, &payload)
            .map_err(|e| internal(e.to_string()))?;
        state.sse.emit(
            session_id,
            "session_event",
            json!({ "session": session_id, "kind": EVENT_IDEA_BUDGET_RAISED, "payload": payload }),
        );
        let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
        return Ok((
            200,
            json!({
                "ok": true,
                "budget_raised": true,
                "idea": idea_view::view(&updated, is_turn_running(state, session_id)),
            }),
        ));
    }

    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    match crate::idea::next_stage(&events) {
        Some(crate::idea::IdeaStage::Sharpen) => {
            crate::idea_exchanges::handle_sharpen_op(state, ledger, session_id, body)
        }
        Some(crate::idea::IdeaStage::Search) => {
            crate::idea_plan_search::handle_search_op(state, ledger, session_id, body)
        }
        other => Err(not_implemented(format!(
            "Explore `continue` for stage `{}` is not built yet (owned by packages X8–X13); pass `extra_queries`/`extra_papers`/`extra_model_calls` to record a budget increase.",
            other.map_or("done", crate::idea::IdeaStage::as_str),
        ))),
    }
}

fn opt_string<'a>(val: Option<&'a Value>, field: &str) -> Result<Option<&'a str>, (u16, String)> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(bad(format!("`{field}` must be a string"))),
    }
}

fn parse_opt_u64(val: Option<&Value>, field: &str) -> Result<u64, (u16, String)> {
    match val {
        None | Some(Value::Null) => Ok(0),
        Some(v) => v.as_u64().ok_or_else(|| bad(format!("`{field}` must be a non-negative integer"))),
    }
}

fn parse_string_list(val: Option<&Value>, field: &str) -> Result<Vec<String>, (u16, String)> {
    match val {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|i| i.as_str().map(str::to_string).ok_or_else(|| bad(format!("`{field}` must be an array of strings"))))
            .collect(),
        Some(_) => Err(bad(format!("`{field}` must be an array of strings"))),
    }
}

fn parse_submitted_queries(val: Option<&Value>) -> Result<Option<Vec<SubmittedFirstQuery>>, (u16, String)> {
    let Some(v) = val.filter(|v| !v.is_null()) else { return Ok(None); };
    let arr = v.as_array().ok_or_else(|| bad("`queries` must be an array of `{provider, q}` objects"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item.as_object().ok_or_else(|| bad("each entry in `queries` must be an object `{provider, q}`"))?;
        let provider = obj.get("provider").and_then(Value::as_str).ok_or_else(|| bad("each query must include a string `provider`"))?;
        let q = obj.get("q").and_then(Value::as_str).ok_or_else(|| bad("each query must include a string `q`"))?;
        out.push(SubmittedFirstQuery { provider: provider.to_string(), q: q.to_string() });
    }
    Ok(Some(out))
}

fn clip_title(s: &str, max_chars: usize) -> String {
    let trimmed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars { out.push_str("..."); break; }
        out.push(ch);
    }
    out
}
