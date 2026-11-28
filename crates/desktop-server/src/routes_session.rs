//! Session, SSE, approval, fork, diff, and provider-settings REST handlers.

use crate::projects::{attach_session, list_projects, project_for_session, save_new_session};
pub use crate::routes_settings::{handle_get_settings, handle_put_settings};
use crate::session_lifecycle::{is_turn_running, list_session_grants};
use crate::session_view::summarize_session_events;
use crate::sse::stream_session_sse;
use crate::state::{now_ms, DesktopState, ForkInfo, SessionMeta, DEFAULT_SESSION_TITLE};
use crate::turn_runner::{reconstruct_session_view, spawn_turn_thread};
use export::{compute_branch_diff, fork_session, ForkOptions};
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tools::canon::random_hex;

pub type ApiResult = Result<(u16, Value), (u16, String)>;

pub(crate) fn bad(msg: impl Into<String>) -> (u16, String) {
    (400, msg.into())
}

pub(crate) fn not_found(msg: impl Into<String>) -> (u16, String) {
    (404, msg.into())
}

pub(crate) fn conflict(msg: impl Into<String>) -> (u16, String) {
    (409, msg.into())
}

pub(crate) fn internal(msg: impl Into<String>) -> (u16, String) {
    (500, msg.into())
}

pub(crate) fn parse_json_body(body: &[u8]) -> Result<Value, (u16, String)> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(body).map_err(|e| bad(format!("invalid JSON body: {e}")))
}

pub fn handle_health() -> ApiResult {
    Ok((
        200,
        json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    ))
}

fn session_fork_info(meta: Option<&SessionMeta>, events: &[ledger::Event]) -> Option<ForkInfo> {
    meta.and_then(|m| m.fork_info.clone()).or_else(|| {
        // Nested forks inherit earlier markers from their parent's event prefix.
        events
            .iter()
            .rev()
            .filter(|event| matches!(event.kind.as_str(), "session.fork" | "session.forked"))
            .find_map(|event| {
                Some(ForkInfo {
                    parent_session: event.payload["parent_session"].as_str()?.to_string(),
                    forked_at_seq: event.payload["parent_seq"].as_u64().unwrap_or_default(),
                    modified_assumption: event.payload["modified_assumption"].as_str().map(str::to_string),
                })
            })
    })
}

pub(crate) fn model_session_title(events: &[ledger::Event]) -> Option<&str> {
    events.iter()
        .filter(|event| event.kind == agent::runner::SESSION_TITLE_EVENT)
        .filter_map(|event| event.payload["title"].as_str())
        .find(|title| !title.trim().is_empty())
}

pub(crate) fn session_title(meta: Option<&SessionMeta>, events: &[ledger::Event], id: &str) -> String {
    if meta.is_some_and(SessionMeta::allows_model_title) {
        if let Some(title) = crate::investigation_title::reading_title(events)
            .or_else(|| model_session_title(events).map(str::to_string))
            .or_else(|| crate::investigation_title::paper_title(events))
        {
            return title;
        }
    }
    meta.map(|meta| meta.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            events.iter()
                .filter(|event| matches!(event.kind.as_str(), "session.start" | "session.resume"))
                .find_map(|event| event.payload["prompt"].as_str())
                .map(|prompt| clip_title(prompt, 52))
        })
        .unwrap_or_else(|| id.to_string())
}

pub fn handle_list_sessions(state: &DesktopState) -> ApiResult {
    let ledger = state.open_ledger().map_err(internal)?;
    let ledger_ids = ledger.sessions().map_err(|e| internal(e.to_string()))?;
    let meta_ids = state.list_meta_ids();
    let projects = list_projects(state)?;
    let membership: HashMap<_, _> = projects
        .iter()
        .flat_map(|p| p.session_ids.iter().map(|s| (s.as_str(), p.id.as_str())))
        .collect();
    let mut ids = BTreeSet::new();
    for (id, _) in ledger_ids {
        if validate_session_id(&id).is_ok() {
            ids.insert(id);
        }
    }
    for id in meta_ids {
        ids.insert(id);
    }

    let mut sessions = Vec::new();
    for id in ids {
        if state.session_restore_pending(&id).map_err(internal)? { continue; }
        let meta = state.load_meta(&id);
        if meta.as_ref().is_some_and(|m| m.deleted) {
            continue;
        }
        let archived = meta.as_ref().is_some_and(|m| m.archived);
        let events = ledger.read_session(&id).unwrap_or_default();
        let parent_session = session_fork_info(meta.as_ref(), &events).map(|fork| fork.parent_session);
        let title = session_title(meta.as_ref(), &events, &id);
        let workspace_root = state.workspace_for(meta.as_ref());
        let created_ms = meta
            .as_ref()
            .map(|m| m.created_ms)
            .filter(|&t| t > 0)
            .or_else(|| events.first().map(|e| e.ts_ms))
            .unwrap_or(0);
        let updated_ms = events.last().map(|e| e.ts_ms).unwrap_or(created_ms);
        let (route, _) = crate::cli_turn::session_route(meta.as_ref(), &events);
        let is_agy = route.as_str() == Some(crate::state::ANTIGRAVITY_ROUTE);
        let stats = summarize_session_events(&events);
        let mode = crate::session_mode::session_mode(&events);
        sessions.push(json!({
            "id": id,
            "mode": crate::session_mode::api_mode_name(mode),
            "title": title,
            "route": route,
            "route_label": is_agy.then_some(crate::antigravity::AGY_LABEL),
            "workspace_root": workspace_root,
            "parent_session": parent_session,
            "project_id": membership.get(id.as_str()),
            "event_count": events.len(),
            "archived": archived,
            "running": is_turn_running(state, &id),
            "stats": stats,
            "created_ms": created_ms,
            "updated_ms": updated_ms,
        }));
    }
    sessions.sort_by(|a, b| {
        let tb = b["updated_ms"].as_i64().unwrap_or(0);
        let ta = a["updated_ms"].as_i64().unwrap_or(0);
        tb.cmp(&ta)
    });
    Ok((200, json!({ "sessions": sessions })))
}

pub fn handle_create_session(state: &Arc<DesktopState>, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let raw_mode = val.get("mode").map(|v| v.as_str().unwrap_or("")).unwrap_or(crate::session_mode::MODE_API_RESEARCH);
    let mode = crate::session_mode::parse_requested_mode(raw_mode).map_err(bad)?;
    if mode == research_tools::paper_key::Mode::Explore {
        if val.get("text").is_some() {
            return crate::routes_idea::create_explore_session(state, &val);
        }
        return Err(bad(
            "Explore sessions require an idea `text` at creation; bare Explore sessions (tool profile `explore/1`) are not built yet.",
        ));
    }
    if mode != research_tools::paper_key::Mode::Solve {
        crate::session_mode::profile_for_mode(mode).map_err(bad)?;
    } else if val.get("prompt").and_then(Value::as_str).is_some_and(|p| !p.trim().is_empty()) {
        return Err(bad("Create the Solve session and confirm its setup before starting a run."));
    }
    if mode == research_tools::paper_key::Mode::Investigate
        && val.get("prompt").and_then(Value::as_str).is_some_and(|p| !p.trim().is_empty())
    {
        return Err(bad("Create the Investigation and upload its primary paper before starting a turn."));
    }
    let id = match val.get("id").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => {
            let trimmed = raw.trim();
            validate_session_id(trimmed).map_err(|e| bad(e.to_string()))?;
            trimmed.to_string()
        }
        _ => format!("s-{}", random_hex(8)),
    };
    let project_id = match val.get("project_id").filter(|value| !value.is_null()) {
        Some(value) => Some(value.as_str().ok_or_else(|| bad("project_id must be a string"))?),
        None => None,
    };

    let workspace_root = val
        .get("workspace_root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| state.cfg.workspace_root.as_ref().map(|p| p.display().to_string()));
    if let Some(ws) = &workspace_root {
        let p = PathBuf::from(ws);
        if !p.exists() || !p.is_dir() {
            return Err(bad(format!(
                "workspace_root `{ws}` does not exist or is not a directory"
            )));
        }
        // Refuse a workspace that overlaps the data directory (keys, ledger, blobs) up front.
        research_tools::ResearchEnv::with_workspace(state.cfg.data_dir.clone(), &p)
            .map_err(|e| bad(e.to_string()))?;
    }

    let ui = state.load_ui_settings();
    let explicit_title = val
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let title = explicit_title.map(str::to_string)
        .or_else(|| {
            val.get("prompt")
                .and_then(Value::as_str)
                .map(|p| clip_title(p, 52))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string());

    let provider = val.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let meta = SessionMeta {
        route: provider.and_then(crate::cli_turn::CliRouteKind::from_provider).map(|k| k.provider_id().to_string()),
        id: id.clone(),
        title: title.clone(),
        allow_model_title: Some(explicit_title.is_none()),
        workspace_root: workspace_root.clone(),
        workspace_detached: false,
        pinned_conventions: val
            .get("pinned_conventions")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(ui.pinned_conventions),
        verification_mode: val
            .get("verification_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        calculation_mode: val
            .get("calculation_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        model_preset: val
            .get("model_preset")
            .and_then(Value::as_str)
            .map(str::to_string),
        default_provider: provider.map(str::to_string).or(ui.default_provider),
        default_model: val
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(ui.default_model),
        reasoning_effort: agent::normalize_reasoning_effort(val.get("reasoning_effort").and_then(Value::as_str)),
        created_ms: now_ms(),
        fork_info: None,
        archived: false,
        deleted: false,
    };
    save_new_session(state, &meta, project_id)?;
    if let Some(marker) = crate::session_mode::start_marker_for_mode(mode) {
        state
            .open_ledger()
            .map_err(internal)?
            .append_new_session(&id, &[(marker, 1, &json!({}))])
            .map_err(|e| internal(e.to_string()))?;
    }

    if let Some(prompt) = val
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        spawn_turn_thread(Arc::clone(state), id.clone(), val.clone(), prompt.to_string())?;
    }

    Ok((
        200,
        json!({
            "id": id,
            "title": title,
            "workspace_root": workspace_root,
            "created_ms": meta.created_ms,
            "mode": crate::session_mode::api_mode_name(mode),
            "route": meta.route,
            "project_id": project_id,
        }),
    ))
}

pub fn handle_pending_approvals(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found("Session not found."));
    }
    Ok((200, json!({ "pending": state.pending_for_session(session_id) })))
}

pub fn handle_get_session(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    crate::cli_turn::sweep_session_tool_config(state, session_id);
    let meta = state.load_meta(session_id);
    let running = is_turn_running(state, session_id);
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let (messages, traces, sources) = reconstruct_session_view(&events);
    let linked_sources = crate::evidence::linked_session_sources(state, &ledger, &events);
    let stats = summarize_session_events(&events);
    let title = session_title(meta.as_ref(), &events, session_id);
    let workspace_root = state.workspace_for(meta.as_ref());
    let pending = state.pending_for_session(session_id);
    let (route, provenance) = crate::cli_turn::session_route(meta.as_ref(), &events);
    let is_agy = route.as_str() == Some(crate::state::ANTIGRAVITY_ROUTE);
    let fork_info = session_fork_info(meta.as_ref(), &events);
    let project = project_for_session(state, session_id)?;

    let events_json: Vec<Value> = events
        .iter()
        .map(|ev| {
            let mut view = json!({
                "seq": ev.seq,
                "ts_ms": ev.ts_ms,
                "kind": ev.kind,
                "schema_version": ev.schema_version,
                "payload": crate::paper_reader::poll_event_payload(&ev.kind, &ev.payload),
                "prev_hash": ev.prev_hash,
                "event_hash": ev.event_hash,
            });
            if ev.kind == "proof.verification" {
                view["effective_verdict"] = json!(research_tools::effective_verification_verdict(&ev.payload));
            }
            view
        })
        .collect();

    let mode = crate::session_mode::session_mode(&events);
    let mut body = json!({
        "id": session_id,
        "title": title,
        "workspace_root": workspace_root,
        "route": route,
        "route_label": is_agy.then_some(crate::antigravity::AGY_LABEL),
        "provenance": provenance,
        "meta": meta,
        "fork_info": fork_info,
        "project_id": project.map(|p| p.id),
        "events": events_json,
        "mode": crate::session_mode::api_mode_name(mode),
        "investigation": crate::investigation::view(&events, running),
        "solve": (mode == research_tools::paper_key::Mode::Solve)
            .then(|| crate::solve::session_solve_view(state, session_id, &events, running)),
        "messages": messages,
        "traces": traces,
        "sources": sources,
        "linked_sources": linked_sources,
        "stats": stats,
        "archived": meta.as_ref().is_some_and(|m| m.archived),
        "running": running,
        "session_grants": list_session_grants(state, session_id),
        "pending_approvals": pending,
        "pending_checkpoints": state.hitl.list_pending(session_id),
    });
    if mode == research_tools::paper_key::Mode::Explore {
        body["idea"] = crate::idea_view::view(&events, running);
    }
    Ok((200, body))
}

pub fn handle_sse_stream(
    stream: TcpStream,
    state: &DesktopState,
    session_id: &str,
    shutdown: &Arc<AtomicBool>,
) -> Result<(), (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    stream_session_sse(stream, &state.sse, session_id, shutdown);
    Ok(())
}

pub fn handle_post_message(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let prompt = val
        .get("prompt")
        .or_else(|| val.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`prompt` must be a non-empty string"))?
        .to_string();
    spawn_turn_thread(Arc::clone(state), session_id.to_string(), val, prompt)?;
    Ok((202, json!({ "ok": true, "session": session_id, "status": "running" })))
}

pub fn handle_post_approval(
    state: &DesktopState,
    session_id: &str,
    call_id: &str,
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let val = parse_json_body(body)?;
    let digest = val
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`digest` is required"))?;
    let approved = val
        .get("approved")
        .and_then(Value::as_bool)
        .ok_or_else(|| bad("`approved` boolean is required"))?;
    let approve_for_session = val
        .get("approve_for_session")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    state.resolve_pending(session_id, call_id, digest, approved, approve_for_session)?;
    Ok((
        200,
        json!({
            "ok": true,
            "call_id": call_id,
            "approved": approved,
            "approve_for_session": approve_for_session,
        }),
    ))
}

pub fn handle_fork(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let parent_project = project_for_session(state, session_id)?;
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let default_seq = events.last().map(|e| e.seq).unwrap_or(1);
    let at_seq = val.get("at_seq").and_then(Value::as_u64).unwrap_or(default_seq);
    let modified_assumption = val
        .get("modified_assumption")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let receipt = fork_session(
        &state.cfg.data_dir,
        &ForkOptions {
            parent_session: session_id.to_string(),
            at_seq,
            modified_assumption: modified_assumption.clone(),
        },
    )
    .map_err(bad)?;

    let parent_meta = state.load_meta(session_id);
    let child_meta = SessionMeta {
        id: receipt.new_session.clone(),
        title: format!("Fork of {}", session_title(parent_meta.as_ref(), &events, session_id)),
        allow_model_title: Some(false),
        workspace_root: parent_meta.as_ref().and_then(|m| m.workspace_root.clone()),
        workspace_detached: parent_meta.as_ref().is_some_and(|m| m.workspace_detached),
        pinned_conventions: parent_meta.as_ref().and_then(|m| m.pinned_conventions.clone()),
        verification_mode: parent_meta.as_ref().is_some_and(|m| m.verification_mode),
        calculation_mode: parent_meta.as_ref().is_some_and(|m| m.calculation_mode),
        model_preset: parent_meta.as_ref().and_then(|m| m.model_preset.clone()),
        default_provider: parent_meta.as_ref().and_then(|m| m.default_provider.clone()),
        default_model: parent_meta.as_ref().and_then(|m| m.default_model.clone()),
        reasoning_effort: parent_meta.as_ref().and_then(|m| m.reasoning_effort.clone()),
        route: parent_meta.as_ref().and_then(|m| m.route.clone()),
        created_ms: now_ms(),
        fork_info: Some(ForkInfo {
            parent_session: session_id.to_string(),
            forked_at_seq: at_seq,
            modified_assumption,
        }),
        archived: false,
        deleted: false,
    };
    state.save_meta(&child_meta).map_err(internal)?;
    let mut response = serde_json::to_value(&receipt).map_err(|e| internal(e.to_string()))?;
    if let Some(project) = parent_project {
        match attach_session(state, &project.id, &receipt.new_session) {
            Ok(_) => response["project_id"] = json!(project.id),
            Err((_, error)) => response["warning"] = json!(format!("Could not link fork to project: {error}")),
        }
    }
    Ok((200, response))
}

pub fn handle_diff(state: &DesktopState, base_id: &str, other_id: &str) -> ApiResult {
    validate_session_id(base_id).map_err(|e| bad(e.to_string()))?;
    validate_session_id(other_id).map_err(|e| bad(e.to_string()))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let report = compute_branch_diff(&ledger, base_id, other_id).map_err(bad)?;
    Ok((200, serde_json::to_value(report).map_err(|e| internal(e.to_string()))?))
}

fn clip_title(s: &str, max_chars: usize) -> String {
    let trimmed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}
