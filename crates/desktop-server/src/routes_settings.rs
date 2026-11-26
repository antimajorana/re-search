//! Provider and UI settings REST handlers extracted from `routes_session.rs`.

use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::{update_provider_roles_from_ui, DesktopState};
use agent::{ProviderKeyStore, ProviderKind, RoleAssignment};
use serde_json::Value;

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
    let next_auto_approve = match val.get("auto_approve_tools") {
        Some(v) => Some(v.as_bool().ok_or_else(|| bad("`auto_approve_tools` must be a boolean"))?),
        None => None,
    };
    let next_agent_models = match val.get("agent_models") {
        Some(am_val) => {
            let mut map = state.load_ui_settings().agent_models;
            crate::agent_models::parse_agent_models_patch(am_val, &mut map)?;
            Some(map)
        }
        None => None,
    };
    let reader_patch = crate::paper_reader::parse_reader_settings_patch(&val)?;

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
            if let Some(dp) = next_provider { ui.default_provider = Some(dp); }
            if let Some(dm) = next_model { ui.default_model = dm; }
            if let Some(conv) = next_conv { ui.pinned_conventions = Some(conv); }
            if let Some(rp) = next_role_presets { ui.role_presets = Some(rp); }
            if let Some(auto) = next_auto_approve { ui.auto_approve_tools = auto; }
            if let Some(am) = next_agent_models { ui.agent_models = am; }
            if let Some(en) = reader_patch.enabled { ui.paper_readers_enabled = en; }
            if let Some(mp) = reader_patch.max_parallel { ui.paper_reader_max_parallel = mp; }
        })
        .map_err(internal)?;
    if val.get("keys").is_some() || val.get("base_urls").is_some() {
        // Lists fetched with the old key or from the old host are no longer valid.
        state.catalog.clear();
    }
    Ok((200, state.settings_json().map_err(internal)?))
}
