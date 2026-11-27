//! Lazy 0-model-call worker activity routes and `run.control` v1 host endpoint (F9b Core).
//!
//! Routes registered behind `AuthGuard::authorize_api` in `api_dispatch.rs`:
//! - `GET  /api/sessions/{id}/workers`
//! - `GET  /api/sessions/{id}/workers/{job_id}`
//! - `POST /api/sessions/{id}/run-control`

#[path = "worker_activity_fold.rs"]
mod fold;

use crate::connections::API_PROVIDERS;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use agent::role_routing::{
    resolve_run_roles, PinnedRoleRecord, RoleModelInput, RoleRoutingInputs, RoleRoutingPolicy,
    ROLE_COMPUTE_BLIND, ROLE_COMPUTE_WORKER, ROLE_EXPLORER_WORKER, ROLE_IDEA_READER,
    ROLE_LITERATURE_WORKER, ROLE_NARRATOR, ROLE_ORCHESTRATOR, ROLE_PAPER_READER, ROLE_STRATEGIST,
    ROLE_VERIFIER,
};
use agent::{ProviderKeyStore, ProviderKind};
use fold::{fold_controls, fold_worker_activity, RunControlsView, EVENT_RUN_CONTROL};
use ledger::Event;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tools::EVENT_SCHEMA_VERSION;

const MAX_NOTE_CHARS: usize = 2_000;
const MAX_REASON_CHARS: usize = 500;
const MAX_ITEM_ID_CHARS: usize = 256;
const MAX_JOB_ID_CHARS: usize = 160;
const MAX_BUDGET_DELTA: i64 = 100;

const ALLOWED_CONTROL_ROLE_KEYS: &[&str] = &[
    "orchestrator",
    "solver",
    "verifier",
    "full_verifier",
    "step_verifier",
    "compute_worker",
    "compute",
    "compute_blind",
    "idea_reader",
    "paper_reader",
    "literature_worker",
    "explorer_worker",
    "strategist",
    "narrator",
    "explain",
    "sidetalk",
    "lean_worker",
    "subagent_builder",
];

/// `GET /api/sessions/{id}/workers` — lists worker/exchange/verifier jobs, honest role/model
/// usage rollup, active `run.control` state, and resolved role availability (0 model calls).
pub fn handle_list_workers(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(internal_str)?;
    let secrets = collect_redaction_secrets(state);
    let folded = fold_worker_activity(&events, &ledger, &secrets, None);
    let roles = resolve_roles_display(state, &events, &folded.controls, &folded.pinned_roles);

    Ok((
        200,
        json!({
            "session": session_id,
            "jobs": folded.jobs,
            "usage_rollup": folded.usage_rollup,
            "roles": roles,
            "controls": folded.controls,
        }),
    ))
}

/// `GET /api/sessions/{id}/workers/{job_id}` — lazy detail loader reading request/response
/// blobs, `context.supplied` input ranges, tool receipts, and returned item IDs (`ev-*`, `xe-*`,
/// `name@v`), flagging missing artifacts explicitly with `"artifact_missing": true`.
pub fn handle_get_worker_detail(
    state: &DesktopState,
    session_id: &str,
    job_id: &str,
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let job_id = validate_job_id(job_id)?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(internal_str)?;
    let secrets = collect_redaction_secrets(state);
    let folded = fold_worker_activity(&events, &ledger, &secrets, Some(job_id));
    let Some(detail) = folded.detail else {
        return Err(not_found(format!(
            "worker job `{job_id}` not found in session `{session_id}`"
        )));
    };
    serde_json::to_value(detail)
        .map(|mut v| {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("session".into(), json!(session_id));
            }
            (200, v)
        })
        .map_err(internal_str)
}

/// `POST /api/sessions/{id}/run-control` — appends validated `run.control` v1 host events
/// (`focus_note`, `pin_worker_item`, `dismiss_worker_item`, `pause_after_stage`, `next_run_roles`,
/// `budget_delta`) without mutating completed receipts.
pub fn handle_post_run_control(
    state: &DesktopState,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let payload = validate_run_control_payload(&val)?;

    let ledger = state.open_ledger().map_err(internal)?;
    let seq = {
        let _lock = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("ledger lock poisoned"))?;
        ledger
            .append(session_id, EVENT_RUN_CONTROL, EVENT_SCHEMA_VERSION, &payload)
            .map_err(internal_str)?
    };
    state.sse.emit(
        session_id,
        "session_event",
        json!({ "session": session_id, "kind": EVENT_RUN_CONTROL, "payload": payload }),
    );
    let updated = ledger.read_session(session_id).map_err(internal_str)?;
    let controls = fold_controls(&updated);

    Ok((
        200,
        json!({
            "ok": true,
            "seq": seq,
            "event": payload,
            "controls": controls,
        }),
    ))
}

fn validate_job_id(raw: &str) -> Result<&str, (u16, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_JOB_ID_CHARS
        || trimmed.bytes().any(|b| b < 0x20 || b == 0x7f || b == b'/' || b == b'\\')
    {
        return Err(bad("invalid `job_id`"));
    }
    Ok(trimmed)
}

fn validate_run_control_payload(val: &Value) -> Result<Value, (u16, String)> {
    let obj = val
        .as_object()
        .ok_or_else(|| bad("run-control body must be a JSON object"))?;
    let action = obj
        .get("action")
        .or_else(|| obj.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`action` is required"))?;

    match action {
        "focus_note" => {
            let note = obj
                .get("note")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| bad("`note` must be a non-empty string"))?;
            if note.chars().count() > MAX_NOTE_CHARS {
                return Err(bad(format!("`note` exceeds {MAX_NOTE_CHARS} characters")));
            }
            Ok(json!({ "action": "focus_note", "note": note }))
        }
        "pin_worker_item" => {
            let item_id = validate_item_id(obj.get("item_id"))?;
            let job_id = optional_job_id(obj.get("job_id"))?;
            let mut out = json!({ "action": "pin_worker_item", "item_id": item_id });
            if let Some(jid) = job_id {
                out["job_id"] = json!(jid);
            }
            Ok(out)
        }
        "dismiss_worker_item" => {
            let item_id = validate_item_id(obj.get("item_id"))?;
            let job_id = optional_job_id(obj.get("job_id"))?;
            let reason = match obj.get("reason").filter(|v| !v.is_null()) {
                None => None,
                Some(Value::String(s)) => {
                    let t = s.trim();
                    if t.chars().count() > MAX_REASON_CHARS {
                        return Err(bad(format!("`reason` exceeds {MAX_REASON_CHARS} characters")));
                    }
                    (!t.is_empty()).then(|| t.to_string())
                }
                Some(_) => return Err(bad("`reason` must be a string")),
            };
            let mut out = json!({ "action": "dismiss_worker_item", "item_id": item_id });
            if let Some(jid) = job_id {
                out["job_id"] = json!(jid);
            }
            if let Some(r) = reason {
                out["reason"] = json!(r);
            }
            Ok(out)
        }
        "pause_after_stage" => {
            let enabled = match obj.get("enabled") {
                None => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(bad("`enabled` must be a boolean")),
            };
            Ok(json!({ "action": "pause_after_stage", "enabled": enabled }))
        }
        "next_run_roles" => {
            let roles_obj = obj
                .get("roles")
                .and_then(Value::as_object)
                .ok_or_else(|| bad("`roles` must be a JSON object"))?;
            let mut sanitized = BTreeMap::new();
            for (k, v) in roles_obj {
                if !ALLOWED_CONTROL_ROLE_KEYS.contains(&k.as_str()) {
                    return Err(bad(format!("unknown role `{k}` in `next_run_roles`")));
                }
                if v.is_null() {
                    sanitized.insert(k.clone(), Value::Null);
                    continue;
                }
                let spec = v
                    .as_object()
                    .ok_or_else(|| bad(format!("`roles.{k}` must be an object or null")))?;
                let p_raw = spec
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| bad(format!("`roles.{k}.provider` is required")))?;
                let pk = ProviderKind::parse(p_raw)
                    .map_err(|e| bad(format!("invalid provider for `roles.{k}`: {e}")))?;
                let m_raw = spec
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| bad(format!("`roles.{k}.model` is required")))?;
                if m_raw.len() > 160
                    || m_raw.starts_with('-')
                    || m_raw.chars().any(|c| c.is_whitespace() || c.is_control())
                {
                    return Err(bad(format!("invalid model `{m_raw}` for `roles.{k}`")));
                }
                let base_url = match spec.get("base_url").filter(|b| !b.is_null()) {
                    None => None,
                    Some(Value::String(u)) => {
                        let trimmed = u.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            validate_safe_role_url(pk, trimmed)?;
                            Some(trimmed.to_string())
                        }
                    }
                    Some(_) => return Err(bad(format!("`roles.{k}.base_url` must be a string"))),
                };
                sanitized.insert(
                    k.clone(),
                    json!({ "provider": pk.cli_name(), "model": m_raw, "base_url": base_url }),
                );
            }
            let allow_frontier = match obj.get("allow_frontier_fallback").filter(|v| !v.is_null()) {
                None => None,
                Some(Value::Bool(b)) => Some(*b),
                Some(_) => return Err(bad("`allow_frontier_fallback` must be a boolean")),
            };
            let mut out = json!({ "action": "next_run_roles", "roles": sanitized });
            if let Some(allow) = allow_frontier {
                out["allow_frontier_fallback"] = json!(allow);
            }
            Ok(out)
        }
        "budget_delta" => {
            let delta = obj
                .get("delta")
                .or_else(|| obj.get("extra_model_calls"))
                .and_then(Value::as_i64)
                .ok_or_else(|| bad("`delta` must be an integer"))?;
            if delta == 0 || !(-MAX_BUDGET_DELTA..=MAX_BUDGET_DELTA).contains(&delta) {
                return Err(bad(format!(
                    "`delta` must be non-zero and within -{MAX_BUDGET_DELTA}..={MAX_BUDGET_DELTA}"
                )));
            }
            Ok(json!({ "action": "budget_delta", "delta": delta }))
        }
        other => Err(bad(format!("unknown run-control action `{other}`"))),
    }
}

fn validate_item_id(val: Option<&Value>) -> Result<String, (u16, String)> {
    let raw = val
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`item_id` must be a non-empty string"))?;
    if raw.chars().count() > MAX_ITEM_ID_CHARS || raw.chars().any(char::is_control) {
        return Err(bad("invalid `item_id`"));
    }
    Ok(raw.to_string())
}

fn optional_job_id(val: Option<&Value>) -> Result<Option<String>, (u16, String)> {
    match val.filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(validate_job_id(s)?.to_string())),
        Some(_) => Err(bad("`job_id` must be a string")),
    }
}

fn validate_safe_role_url(provider: ProviderKind, url: &str) -> Result<(), (u16, String)> {
    if provider == ProviderKind::ChatGpt {
        return Err(bad("ChatGPT roles do not accept a custom `base_url`"));
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| bad("`base_url` must start with `https://` or `http://`"))?;
    if rest.split('/').next().unwrap_or("").is_empty()
        || url.contains('@')
        || url.contains('?')
        || url.contains('#')
        || url.contains('\\')
        || url.bytes().any(|b| b <= 0x20 || b == 0x7f)
    {
        return Err(bad("invalid `base_url`"));
    }
    Ok(())
}

fn resolve_roles_display(
    state: &DesktopState,
    events: &[Event],
    controls: &RunControlsView,
    pinned_from_events: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let ui = state.load_ui_settings();
    let store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).unwrap_or_default();
    let primary_provider = events
        .iter()
        .rev()
        .filter(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"))
        .find_map(|e| e.payload.get("provider").and_then(Value::as_str))
        .or(ui.default_provider.as_deref())
        .and_then(|s| ProviderKind::parse(s).ok())
        .unwrap_or(ProviderKind::Anthropic);
    let primary_model = events
        .iter()
        .rev()
        .filter(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"))
        .find_map(|e| e.payload.get("model").and_then(Value::as_str))
        .or(ui.default_model.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| primary_provider.default_model().to_string());

    let mut run_overrides = BTreeMap::new();
    for (k, v) in &controls.next_run_roles {
        if let Some(spec) = v {
            run_overrides.insert(k.clone(), spec.clone());
        }
    }
    let mut agent_models = BTreeMap::new();
    for (k, v) in &ui.agent_models {
        agent_models.insert(k.clone(), RoleModelInput::new(&v.provider, &v.model));
    }
    let mut role_presets = BTreeMap::new();
    if let Some(obj) = ui.role_presets.as_ref().and_then(Value::as_object) {
        for (k, v) in obj {
            if let (Some(p), Some(m)) = (
                v.get("provider").and_then(Value::as_str),
                v.get("model").and_then(Value::as_str),
            ) {
                role_presets.insert(k.clone(), RoleModelInput::new(p, m));
            }
        }
    }

    let since = state.catalog.generation();
    let mut catalog_models = BTreeMap::new();
    for p in API_PROVIDERS {
        if let Some(fetched) = state.catalog.cached_fresh(state, &store, p, false, since) {
            catalog_models.insert(
                p.cli_name().to_string(),
                fetched.models.into_iter().map(|m| m.id).collect(),
            );
        }
    }

    let inputs = RoleRoutingInputs {
        run_overrides,
        agent_models,
        role_presets,
        store,
        store_roles_mode: agent::role_routing::StoreRolesMode::Auto,
        catalog_models,
        env: state.cfg.connection_env.clone(),
        api_base_override: state.cfg.api_base_override.clone(),
        chatgpt_ready: state.chatgpt.ready(),
        chatgpt_api_base: Some(state.chatgpt.api_base()),
    };
    let mut policy = RoleRoutingPolicy::new(primary_provider, primary_model);
    if let Some(allow) = controls.allow_frontier_fallback {
        policy.allow_frontier_fallback = allow;
    }

    let role_list = [
        ROLE_ORCHESTRATOR,
        ROLE_PAPER_READER,
        ROLE_IDEA_READER,
        ROLE_LITERATURE_WORKER,
        ROLE_COMPUTE_WORKER,
        ROLE_COMPUTE_BLIND,
        ROLE_VERIFIER,
        ROLE_EXPLORER_WORKER,
        ROLE_STRATEGIST,
        ROLE_NARRATOR,
    ];
    let resolved = resolve_run_roles(&role_list, &policy, &inputs);
    let mut out = BTreeMap::new();
    for (k, res) in &resolved {
        let rec = PinnedRoleRecord::from_resolved(k, res);
        let mut val = serde_json::to_value(rec).unwrap_or(Value::Null);
        if let Some(pinned_val) = pinned_from_events.get(k) {
            if let Some(obj) = val.as_object_mut() {
                obj.insert("pinned_in_session".into(), pinned_val.clone());
            }
        }
        out.insert(k.clone(), val);
    }
    out
}

fn collect_redaction_secrets(state: &DesktopState) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(store) = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir) {
        for p in API_PROVIDERS {
            if let Some(k) = store.key_for(p) {
                if !k.trim().is_empty() {
                    out.push(k.to_string());
                }
            }
        }
    }
    out
}

fn internal_str(e: impl ToString) -> (u16, String) {
    internal(e.to_string())
}

#[cfg(test)]
#[path = "routes_worker_activity_tests.rs"]
mod tests;
