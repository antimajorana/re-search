//! HTTP API handlers for listing built-in agents, managing custom subagents, and running
//! the in-app Subagent Builder Agent (`POST /api/subagents/draft`).

use crate::connections::API_PROVIDERS;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use crate::subagents::{
    build_subagent_builder_prompts, builtin_agents_json, is_reserved_id, load_subagents,
    parse_subagent_draft, save_subagents, validate_new_subagent, MAX_GOAL_CHARS,
};
use agent::{ContentBlock, Message, ModelRequest, ProviderKeyStore, ProviderKind, Role};
use serde_json::{json, Value};
use std::sync::Arc;

pub fn handle_list_subagents(state: &DesktopState) -> ApiResult {
    let _guard = state
        .ui_settings_lock
        .lock()
        .map_err(|_| internal("settings lock poisoned"))?;
    let custom = load_subagents(&state.cfg.data_dir);
    Ok((
        200,
        json!({
            "builtin": builtin_agents_json(),
            "custom": custom,
        }),
    ))
}

pub fn handle_create_subagent(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let _guard = state
        .ui_settings_lock
        .lock()
        .map_err(|_| internal("settings lock poisoned"))?;
    let mut items = load_subagents(&state.cfg.data_dir);
    let def = validate_new_subagent(&val, &items)?;
    if let Some(pos) = items.iter().position(|s| s.id == def.id) {
        items[pos] = def.clone();
    } else {
        items.insert(0, def.clone());
    }
    save_subagents(&state.cfg.data_dir, &items).map_err(internal)?;
    Ok((201, json!({ "subagent": def, "custom": items })))
}

pub fn handle_delete_subagent(state: &DesktopState, raw_id: &str) -> ApiResult {
    let id = raw_id.trim();
    if id.is_empty() {
        return Err(bad("subagent id cannot be empty"));
    }
    if is_reserved_id(id) {
        return Err(bad(format!("built-in agent `{id}` cannot be deleted")));
    }
    let _guard = state
        .ui_settings_lock
        .lock()
        .map_err(|_| internal("settings lock poisoned"))?;
    let mut items = load_subagents(&state.cfg.data_dir);
    let before = items.len();
    items.retain(|s| s.id != id);
    if items.len() == before {
        return Err(not_found(format!("custom subagent `{id}` not found")));
    }
    save_subagents(&state.cfg.data_dir, &items).map_err(internal)?;
    Ok((200, json!({ "deleted": id, "custom": items })))
}

/// Invokes the in-app Subagent Builder Agent on the connected in-core provider (or returns
/// a clear error if none is configured) to draft a structured custom subagent profile.
pub fn handle_draft_subagent(state: &Arc<DesktopState>, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let goal = val
        .get("goal")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`goal` is required to draft a subagent"))?;
    if goal.chars().count() > MAX_GOAL_CHARS {
        return Err(bad(format!("`goal` exceeds {MAX_GOAL_CHARS} characters")));
    }

    let (provider, model) = pick_builder_model(state, &val)?;
    let transport = state.build_transport(provider, &model).map_err(bad)?;
    let (sys, user) = build_subagent_builder_prompts(goal);
    let req = ModelRequest {
        model: model.clone(),
        system: Some(sys),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user }],
        }],
        tools: Vec::new(),
        max_tokens: 2048,
        reasoning_effort: None,
    };
    let prepared = transport.prepare_request(&req).map_err(|e| bad(e.to_string()))?;
    let resp = transport.send_prepared(&prepared).map_err(|e| bad(e.to_string()))?;
    let raw_text: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let draft = parse_subagent_draft(&raw_text).map_err(bad)?;
    Ok((
        200,
        json!({
            "draft": draft,
            "provider": provider.cli_name(),
            "model": model,
        }),
    ))
}

fn pick_builder_model(state: &DesktopState, req: &Value) -> Result<(ProviderKind, String), (u16, String)> {
    let ui = state.load_ui_settings();
    if let Some(p_raw) = req.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(pk) = ProviderKind::parse(p_raw) {
            let m = req
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| pk.resolve_model(s))
                .unwrap_or_else(|| pk.default_model().to_string());
            return Ok((pk, m));
        }
    }

    let store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).unwrap_or_default();
    if let Some(pk) = ui
        .default_provider
        .as_deref()
        .and_then(|p| ProviderKind::parse(p).ok())
        .filter(|&p| state.build_transport(p, p.default_model()).is_ok())
    {
        let m = ui
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| pk.resolve_model(s))
            .unwrap_or_else(|| pk.default_model().to_string());
        return Ok((pk, m));
    }

    let mut candidates = Vec::with_capacity(5);
    candidates.push(ProviderKind::ChatGpt);
    candidates.extend(API_PROVIDERS);
    for pk in candidates {
        let ch_st = state.chatgpt.auth.status();
        let has_key = (pk == ProviderKind::ChatGpt && ch_st.signed_in && ch_st.plan_enabled)
            || (pk != ProviderKind::ChatGpt
                && crate::connections::credential(state, pk, &store)
                    .ok()
                    .and_then(|c| c.key)
                    .is_some())
            || (pk == ProviderKind::Ollama && state.build_transport(pk, pk.default_model()).is_ok());
        if has_key && state.build_transport(pk, pk.default_model()).is_ok() {
            return Ok((pk, pk.default_model().to_string()));
        }
    }

    Err(bad(
        "Subagent Builder requires a connected in-core provider (Anthropic API, OpenAI API, Sign in with ChatGPT, Gemini API, or Ollama). Connect one under Settings -> Providers, or fill in the Subagent form manually.",
    ))
}
