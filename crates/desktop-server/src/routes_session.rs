//! Session, SSE, approval, fork, diff, and provider-settings REST handlers.

use crate::sse::stream_session_sse;
use crate::state::{now_ms, update_provider_roles_from_ui, DesktopState, ForkInfo, SessionMeta};
use crate::turn_runner::{reconstruct_session_view, spawn_turn_thread};
use agent::{ProviderKeyStore, ProviderKind, RoleAssignment};
use export::{compute_branch_diff, fork_session, ForkOptions};
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::BTreeSet;
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

pub fn handle_list_sessions(state: &DesktopState) -> ApiResult {
    let ledger = state.open_ledger().map_err(internal)?;
    let ledger_ids = ledger.sessions().map_err(|e| internal(e.to_string()))?;
    let meta_ids = state.list_meta_ids();
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
        let meta = state.load_meta(&id);
        let events = ledger.read_session(&id).unwrap_or_default();
        let prompt_title = events
            .iter()
            .find(|e| e.kind == "session.start")
            .and_then(|e| e.payload.get("prompt").and_then(Value::as_str))
            .map(|p| clip_title(p, 52));
        let fork_info = meta.as_ref().and_then(|m| m.fork_info.as_ref());
        let parent_session = fork_info.map(|f| f.parent_session.clone()).or_else(|| {
            events
                .iter()
                .find(|e| e.kind == "session.forked")
                .and_then(|e| {
                    e.payload
                        .get("parent_session")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        });
        let title = meta
            .as_ref()
            .map(|m| m.title.clone())
            .filter(|t| !t.trim().is_empty())
            .or(prompt_title)
            .unwrap_or_else(|| id.clone());
        let workspace_root = meta
            .as_ref()
            .and_then(|m| m.workspace_root.clone())
            .or_else(|| state.cfg.workspace_root.as_ref().map(|p| p.display().to_string()));
        let created_ms = meta
            .as_ref()
            .map(|m| m.created_ms)
            .filter(|&t| t > 0)
            .or_else(|| events.first().map(|e| e.ts_ms))
            .unwrap_or(0);
        let updated_ms = events.last().map(|e| e.ts_ms).unwrap_or(created_ms);
        let (route, _) = crate::cli_turn::session_route(meta.as_ref(), &events);
        let is_agy = route.as_str() == Some(crate::state::ANTIGRAVITY_ROUTE);
        sessions.push(json!({
            "id": id,
            "title": title,
            "route": route,
            "confinement": is_agy.then_some(cli_route::antigravity::CONFINEMENT),
            "route_label": is_agy.then_some(crate::antigravity::AGY_LABEL),
            "workspace_root": workspace_root,
            "parent_session": parent_session,
            "event_count": events.len(),
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
    let id = match val.get("id").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => {
            let trimmed = raw.trim();
            validate_session_id(trimmed).map_err(|e| bad(e.to_string()))?;
            trimmed.to_string()
        }
        _ => format!("s-{}", random_hex(8)),
    };
    if state.session_exists(&id).map_err(internal)? {
        return Err(conflict(format!("session `{id}` already exists")));
    }

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
    let title = val
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            val.get("prompt")
                .and_then(Value::as_str)
                .map(|p| clip_title(p, 52))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "Research Session".to_string());

    let provider = val.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let meta = SessionMeta {
        route: provider.and_then(crate::cli_turn::CliRouteKind::from_provider).map(|k| k.provider_id().to_string()),
        id: id.clone(),
        title: title.clone(),
        workspace_root: workspace_root.clone(),
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
        created_ms: now_ms(),
        fork_info: None,
    };
    state.save_meta(&meta).map_err(internal)?;

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
            "route": meta.route,
        }),
    ))
}

pub fn handle_get_session(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    crate::cli_turn::sweep_session_tool_config(state, session_id);
    let meta = state.load_meta(session_id);
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let (messages, traces, sources) = reconstruct_session_view(&events);
    let title = meta
        .as_ref()
        .map(|m| m.title.clone())
        .filter(|t| !t.trim().is_empty())
        .or_else(|| {
            messages
                .iter()
                .find(|m| m["role"] == "user")
                .and_then(|m| m["text"].as_str())
                .map(|p| clip_title(p, 52))
        })
        .unwrap_or_else(|| session_id.to_string());
    let workspace_root = meta
        .as_ref()
        .and_then(|m| m.workspace_root.clone())
        .or_else(|| state.cfg.workspace_root.as_ref().map(|p| p.display().to_string()));
    let pending = state.pending_for_session(session_id);
    let (route, provenance) = crate::cli_turn::session_route(meta.as_ref(), &events);
    let is_agy = route.as_str() == Some(crate::state::ANTIGRAVITY_ROUTE);
    let fork_info = meta.as_ref().and_then(|m| m.fork_info.clone()).or_else(|| {
        events.iter().find(|e| e.kind == "session.forked").map(|e| ForkInfo {
            parent_session: e.payload["parent_session"].as_str().unwrap_or("").to_string(),
            forked_at_seq: e.payload["parent_seq"].as_u64().unwrap_or(0),
            modified_assumption: e.payload["modified_assumption"].as_str().map(str::to_string),
        })
    });

    let events_json: Vec<Value> = events
        .iter()
        .map(|ev| {
            json!({
                "seq": ev.seq,
                "ts_ms": ev.ts_ms,
                "kind": ev.kind,
                "schema_version": ev.schema_version,
                "payload": ev.payload,
                "prev_hash": ev.prev_hash,
                "event_hash": ev.event_hash,
            })
        })
        .collect();

    Ok((
        200,
        json!({
            "id": session_id,
            "title": title,
            "workspace_root": workspace_root,
            "route": route,
            "confinement": is_agy.then_some(cli_route::antigravity::CONFINEMENT),
            "route_label": is_agy.then_some(crate::antigravity::AGY_LABEL),
            "provenance": provenance,
            "meta": meta,
            "fork_info": fork_info,
            "events": events_json,
            "messages": messages,
            "traces": traces,
            "sources": sources,
            "pending_approvals": pending,
        }),
    ))
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
        title: format!(
            "Fork of {}",
            parent_meta
                .as_ref()
                .map(|m| m.title.as_str())
                .unwrap_or(session_id)
        ),
        workspace_root: parent_meta.as_ref().and_then(|m| m.workspace_root.clone()),
        pinned_conventions: parent_meta.as_ref().and_then(|m| m.pinned_conventions.clone()),
        verification_mode: parent_meta.as_ref().is_some_and(|m| m.verification_mode),
        calculation_mode: parent_meta.as_ref().is_some_and(|m| m.calculation_mode),
        model_preset: parent_meta.as_ref().and_then(|m| m.model_preset.clone()),
        default_provider: parent_meta.as_ref().and_then(|m| m.default_provider.clone()),
        default_model: parent_meta.as_ref().and_then(|m| m.default_model.clone()),
        route: parent_meta.as_ref().and_then(|m| m.route.clone()),
        created_ms: now_ms(),
        fork_info: Some(ForkInfo {
            parent_session: session_id.to_string(),
            forked_at_seq: at_seq,
            modified_assumption,
        }),
    };
    let _ = state.save_meta(&child_meta);
    Ok((200, serde_json::to_value(receipt).map_err(|e| internal(e.to_string()))?))
}

pub fn handle_diff(state: &DesktopState, base_id: &str, other_id: &str) -> ApiResult {
    validate_session_id(base_id).map_err(|e| bad(e.to_string()))?;
    validate_session_id(other_id).map_err(|e| bad(e.to_string()))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let report = compute_branch_diff(&ledger, base_id, other_id).map_err(bad)?;
    Ok((200, serde_json::to_value(report).map_err(|e| internal(e.to_string()))?))
}

pub fn handle_get_settings(state: &DesktopState) -> ApiResult {
    Ok((200, state.settings_json().map_err(internal)?))
}

pub fn handle_put_settings(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let mut store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).map_err(internal)?;

    let next_provider = match val.get("default_provider").and_then(Value::as_str) {
        Some(dp) => Some(ProviderKind::parse(dp).map_err(bad)?.cli_name().to_string()),
        None => None,
    };
    let next_model = val.get("default_model").and_then(Value::as_str).map(|dm| {
        let trimmed = dm.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    let next_conv = match val.get("pinned_conventions").and_then(Value::as_str) {
        Some(conv) => {
            agent::validate_pinned_conventions(conv).map_err(bad)?;
            Some(conv.to_string())
        }
        None => None,
    };
    let next_role_presets = val.get("role_presets").cloned();

    if let Some(roles_val) = val.get("roles") {
        let parsed: RoleAssignment =
            serde_json::from_value(roles_val.clone()).map_err(|e| bad(format!("invalid `roles`: {e}")))?;
        agent::validate_roles(&parsed).map_err(bad)?;
        store.roles = parsed;
    } else {
        let mut preview_ui = state.load_ui_settings();
        if let Some(dp) = &next_provider {
            preview_ui.default_provider = Some(dp.clone());
        }
        if let Some(dm) = &next_model {
            preview_ui.default_model = dm.clone();
        }
        update_provider_roles_from_ui(&mut store.roles, &preview_ui);
        agent::validate_roles(&store.roles).map_err(bad)?;
    }

    if let Some(keys) = val.get("keys").and_then(Value::as_object) {
        for (k, v) in keys {
            let pk = ProviderKind::parse(k).map_err(bad)?;
            if v.is_null() {
                store.set_key(pk, None).map_err(bad)?;
            } else if let Some(s) = v.as_str() {
                store.set_key(pk, Some(s.to_string())).map_err(bad)?;
            } else {
                return Err(bad(format!("`keys.{k}` must be a string or null")));
            }
        }
    }
    if let Some(bases) = val.get("base_urls").and_then(Value::as_object) {
        for (k, v) in bases {
            let pk = ProviderKind::parse(k).map_err(bad)?;
            let url = if v.is_null() {
                None
            } else if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                return Err(bad(format!("`base_urls.{k}` must be a string or null")));
            };
            store.set_base_url(pk, url).map_err(bad)?;
        }
    }

    store.save_to_data_dir(&state.cfg.data_dir).map_err(internal)?;
    state
        .update_ui_settings(|ui| {
            if let Some(dp) = next_provider {
                ui.default_provider = Some(dp);
            }
            if let Some(dm) = next_model {
                ui.default_model = dm;
            }
            if let Some(conv) = next_conv {
                ui.pinned_conventions = Some(conv);
            }
            if let Some(rp) = next_role_presets {
                ui.role_presets = Some(rp);
            }
        })
        .map_err(internal)?;
    if val.get("keys").is_some() || val.get("base_urls").is_some() {
        // Lists fetched with the old key or from the old host are no longer valid.
        state.catalog.clear();
    }
    Ok((200, state.settings_json().map_err(internal)?))
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
