//! The Antigravity route in the desktop UI: its Connect entry and model group (from `agy models`).

use crate::model_catalog::CACHE_TTL;
use crate::routes_session::ApiResult;
use crate::state::{now_ms, DesktopState, ANTIGRAVITY_ROUTE};
use cli_route::antigravity::{self as agy, Antigravity, AgyModel, ALLOW_ENV_AUTH_VAR};
use cli_route::VendorCli;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Label of the route in the Connect panel and the model dropdown.
pub const AGY_LABEL: &str = "Antigravity (Google)";
/// Plain-text note shown beside the model of a session on this route.
pub const AGY_NOTE: &str = "Antigravity CLI";
const HOW_TO: &str = "Install the Antigravity CLI (`agy`) and sign in once by running `agy` in a terminal.";
/// Model used when neither the request nor the session names one and the list has it.
pub const DEFAULT_AGY_MODEL: &str = "gemini-3.8-flash-medium";
/// Shown when `agy models` has never answered (from the 1.5.13 list captured by the builder).
pub const FALLBACK_MODELS: [(&str, &str); 3] = [
    ("gemini-3.8-flash-medium", "Gemini 3.8 Flash (Medium)"),
    ("gemini-4-neon-medium", "Gemini 4 Neon (Medium)"),
    ("claude-sonnet-5-5-medium", "Claude Sonnet 5.5 (Medium)"),
];
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const ERROR_TTL: Duration = Duration::from_secs(30);

/// Last `agy models` result.
#[derive(Clone)]
pub struct AgyModelCache {
    pub models: Vec<AgyModel>,
    pub ok_ms: Option<i64>,
    pub error: Option<String>,
    pub checked: Instant,
}

/// Compatibility constant referenced by `state::AntigravityAck`.
pub const NOTICE_VERSION: u32 = 1;

/// Always ready (no warning click-through gate required).
pub fn acknowledged(_state: &DesktopState) -> bool {
    true
}

/// Route availability no longer requires a click-through warning gate.
pub fn require_acknowledged(_state: &DesktopState) -> Result<(), String> {
    Ok(())
}

/// `POST /api/connections/antigravity/acknowledge` (token required): kept for API compatibility.
pub fn handle_acknowledge(_state: &DesktopState) -> ApiResult {
    Ok((200, json!({ "ok": true, "acknowledged": true, "notice_version": NOTICE_VERSION })))
}

/// `POST /api/connections/antigravity/withdraw` (token required): stops active CLI turns and clears model cache.
pub fn handle_withdraw(state: &DesktopState) -> ApiResult {
    if let Ok(mut cache) = state.agy_models.lock() {
        cache.0 = cache.0.wrapping_add(1);
        cache.1 = None;
    }
    state.stop_cli_turns_of(crate::cli_turn::CliRouteKind::Antigravity);
    Ok((200, json!({ "ok": true, "acknowledged": true })))
}

/// Whether the route can start a turn now, and why not.
pub struct AgyStatus {
    pub connected: bool,
    pub found: bool,
    pub status: String,
    pub how_to: Option<String>,
    pub version: Option<String>,
    pub exe: Option<PathBuf>,
}

pub fn agy_status(state: &DesktopState) -> AgyStatus {
    let env = |n: &str| state.env_var(n);
    let exe = Antigravity::new().locate(state.cfg.agy_exe.as_deref(), &env).ok().filter(|p| p.is_file());
    let Some(exe) = exe else {
        return AgyStatus {
            connected: false,
            found: false,
            status: "The Antigravity CLI (agy) was not found on this computer.".to_string(),
            how_to: Some(HOW_TO.to_string()),
            version: None,
            exe: None,
        };
    };
    let allowed = match agy::check_env_auth(&env) {
        Ok(names) => names,
        Err(_) => {
            let names = cli_route::env_auth::detect(agy::AGY_ENV_AUTH_VARS, &env).join(", ");
            return AgyStatus {
                connected: false,
                found: true,
                status: format!(
                    "Blocked: {names} set in re:Search's environment; with it the Antigravity CLI may not use your Google plan login."
                ),
                how_to: Some(format!("Unset {names} before starting re:Search, or set {ALLOW_ENV_AUTH_VAR}=1 to use it anyway.")),
                version: None,
                exe: Some(exe),
            };
        }
    };
    let version = match state.agy_version.get() {
        Some(v) => Ok(v.clone()),
        None => run_version(&exe).inspect(|v| {
            let _ = state.agy_version.set(v.clone());
        }),
    };
    let mut status = match &version {
        Ok(v) => format!("Antigravity CLI {v} ready."),
        Err(e) => format!("Antigravity CLI detected (version check failed: {e})."),
    };
    if !allowed.is_empty() {
        status.push_str(&format!(
            " Using {} via {ALLOW_ENV_AUTH_VAR}.",
            allowed.join(", ")
        ));
    }
    AgyStatus {
        connected: true,
        found: true,
        status,
        how_to: None,
        version: version.ok(),
        exe: Some(exe),
    }
}

fn run_version(exe: &std::path::Path) -> Result<String, String> {
    let raw = agy::run_bounded(exe, &["--version"], VERSION_TIMEOUT)?;
    let v: String = agy::parse_cli_version(raw.lines().next().unwrap_or(""))
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .take(40)
        .collect();
    if v.is_empty() {
        Err("no version printed".to_string())
    } else {
        Ok(v)
    }
}

/// The Connect panel entry.
pub fn connection_entry(state: &DesktopState) -> Value {
    let st = agy_status(state);
    json!({
        "id": ANTIGRAVITY_ROUTE,
        "label": AGY_LABEL,
        "kind": "subscription",
        "route": ANTIGRAVITY_ROUTE,
        "connected": st.connected,
        "found": st.found,
        "status": st.status,
        "how_to": st.how_to,
        "version": st.version,
        "accepts_key": false,
        "key_required": false,
        "has_saved_key": false,
        "key_source": null,
        "key_env": null,
        "base_url": null,
        "session_note": AGY_NOTE,
        "confinement": agy::CONFINEMENT,
    })
}

fn model_json(id: &str, label: &str, is_default: bool) -> Value {
    json!({ "id": id, "label": label, "provider": ANTIGRAVITY_ROUTE, "default": is_default })
}

/// The model group, when the route is usable (found and clean environment).
/// `agy models` makes no model call; it is run at most once per [`CACHE_TTL`] (30 s after
/// a failure), never while holding the cache lock.
pub fn model_group(state: &DesktopState, refresh: bool) -> Option<Value> {
    let st = agy_status(state);
    let exe = st.exe.filter(|_| st.connected)?;
    let (generation, cached) = state.agy_models.lock().ok().map(|c| (c.0, c.1.clone()))?;
    let fresh = cached.as_ref().filter(|c| {
        let ttl = if c.error.is_some() { ERROR_TTL } else { CACHE_TTL };
        !refresh && c.checked.elapsed() < ttl
    });
    let (entry, how) = match fresh {
        Some(c) => (c.clone(), "cache"),
        None => {
            let mut e = cached.unwrap_or(AgyModelCache { models: Vec::new(), ok_ms: None, error: None, checked: Instant::now() });
            e.checked = Instant::now();
            match agy::list_models(&exe, agy::MODELS_TIMEOUT) {
                Ok(models) => {
                    e.models = models;
                    e.ok_ms = Some(now_ms());
                    e.error = None;
                }
                Err(err) => e.error = Some(err.chars().filter(|c| !c.is_control()).take(300).collect()),
            }
            let committed = if let Ok(mut c) = state.agy_models.lock() {
                if c.0 == generation {
                    c.1 = Some(e.clone());
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !committed {
                return None;
            }
            (e, "live")
        }
    };
    let (models, source, stale): (Vec<AgyModel>, &str, bool) = match (&entry.error, entry.models.is_empty()) {
        (None, false) => (entry.models.clone(), how, false),
        (Some(_), false) => (entry.models.clone(), "cache", true),
        (_, true) => (
            FALLBACK_MODELS.iter().map(|(id, label)| AgyModel { id: id.to_string(), label: format!("{label} (built-in list)") }).collect(),
            "fallback",
            true,
        ),
    };
    let ui = state.load_ui_settings();
    let saved = ui.default_provider.as_deref().filter(|p| *p == ANTIGRAVITY_ROUTE).and(ui.default_model.clone());
    let has = |id: &str| models.iter().any(|m| m.id == id);
    let default = saved
        .filter(|id| has(id))
        .or_else(|| has(DEFAULT_AGY_MODEL).then(|| DEFAULT_AGY_MODEL.to_string()))
        .or_else(|| models.first().map(|m| m.id.clone()));
    let list: Vec<Value> = models.iter().map(|m| model_json(&m.id, &m.label, default.as_deref() == Some(m.id.as_str()))).collect();
    Some(json!({
        "id": ANTIGRAVITY_ROUTE,
        "label": AGY_LABEL,
        "kind": "subscription",
        "route": ANTIGRAVITY_ROUTE,
        "confinement": agy::CONFINEMENT,
        "session_note": AGY_NOTE,
        "models": list,
        "source": source,
        "stale": stale,
        "error": entry.error,
        "fetched_ms": entry.ok_ms,
    }))
}
