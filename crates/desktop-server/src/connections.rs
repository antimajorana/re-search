//! `GET /api/connections`: the ways re:Search can reach a model, and whether each is ready.
//!
//! One entry per provider, in a shape the UI renders without knowing the provider: a
//! stable `id` (also the `provider` value a message is sent with), a label, a `kind`
//! (`"subscription"` or `"api-key"`), `connected`, a one-line `status`, a one-sentence
//! `how_to` when not connected, and what the Connect panel should offer (`accepts_key`,
//! `base_url`). Keys are never returned: only whether one is saved and where the key in
//! use comes from (`"saved key"` or `"environment"`, with the variable's name).
//!
//! The Antigravity CLI is located the same way and asked only `agy --version`
//! ([`crate::antigravity`]).
//!
//! Nothing here sends a prompt anywhere. The Claude CLI is only located and asked for its
//! version (`claude --version`, with a timeout; kept once it succeeded); Ollama is
//! connected when its model list answers (a local `GET /api/tags`, cached like the lists,
//! sent with no key but Ollama's own). Which key a list call may use: [`ListAccess`].

use crate::routes_session::ApiResult;
use crate::state::{DesktopState, CLAUDE_CLI_ROUTE};
use agent::transport::{GENERIC_KEY_ENV_VARS, PROVIDER_ENV_VARS};
use agent::{ProviderKeyStore, ProviderKind, SecretKey};
use cli_route::claude::{parse_cli_version, ClaudeCode, CLAUDE_ENV_AUTH_VARS};
use cli_route::env_auth::{self, ALLOW_ENV_AUTH_VAR};
use cli_route::VendorCli;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

/// Label of the Claude subscription route.
pub const CLAUDE_CLI_LABEL: &str = "Claude (your subscription, via Claude Code)";
/// Plain-text note the UI shows beside the model of a session on a subscription CLI route.
pub const SUBSCRIPTION_NOTE: &str = "runs Claude Code (your login)";
const CLAUDE_HOW_TO: &str = "Install Claude Code and sign in once by running claude in a terminal.";
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// API-key providers, in the order the UI lists them.
pub const API_PROVIDERS: [ProviderKind; 4] =
    [ProviderKind::Anthropic, ProviderKind::OpenAi, ProviderKind::Gemini, ProviderKind::Ollama];

pub fn api_label(p: ProviderKind) -> &'static str {
    match p {
        ProviderKind::Anthropic => "Anthropic API",
        ProviderKind::OpenAi => "OpenAI API",
        ProviderKind::Gemini => "Google Gemini API",
        ProviderKind::Ollama => "Ollama (models on this computer)",
        ProviderKind::ChatGpt => crate::chatgpt::CHATGPT_LABEL,
    }
}

fn api_how_to(p: ProviderKind) -> &'static str {
    match p {
        ProviderKind::Anthropic => "Create an API key at console.anthropic.com, paste it here and press Save.",
        ProviderKind::OpenAi => "Create an API key at platform.openai.com, paste it here and press Save.",
        ProviderKind::Gemini => "Create an API key in Google AI Studio (aistudio.google.com), paste it here and press Save.",
        ProviderKind::Ollama => {
            "Install Ollama and start it; re:Search looks for it at http://127.0.0.1:11434 (change the address under Advanced)."
        }
        ProviderKind::ChatGpt => crate::chatgpt::CHATGPT_HOW_TO,
    }
}

impl DesktopState {
    /// One environment variable as connection status and model lists see it
    /// ([`crate::DesktopConfig::connection_env`], else the process environment).
    pub fn env_var(&self, name: &str) -> Option<OsString> {
        match &self.cfg.connection_env {
            Some(map) => map.get(name).map(OsString::from),
            None => std::env::var_os(name),
        }
    }

    /// The base URL a provider's calls (and its key) go to, and where it was set:
    /// `"launch"` (`--api-base`), `"saved"` (providers.json) or `"default"`. Never a
    /// request body.
    pub fn base_url_for(&self, provider: ProviderKind, store: &ProviderKeyStore) -> (String, &'static str) {
        // The ChatGPT token goes only to OpenAI's API host: no launch override, no saved URL.
        if provider == ProviderKind::ChatGpt {
            return (self.chatgpt.api_base(), "fixed");
        }
        if let Some(b) = &self.cfg.api_base_override {
            return (b.clone(), "launch");
        }
        match store.providers.get(provider.cli_name()).and_then(|e| e.base_url.clone()) {
            Some(b) => (b, "saved"),
            None => (provider.default_base_url().to_string(), "default"),
        }
    }
}

/// The key a provider's calls would use: the environment first (as the transports read
/// it), then the saved key.
pub struct Credential {
    pub key: Option<SecretKey>,
    /// `"environment"` or `"saved key"`; `None` without a key.
    pub source: Option<&'static str>,
    /// Name of the environment variable the key came from.
    pub env_name: Option<&'static str>,
}

pub fn credential(state: &DesktopState, p: ProviderKind, store: &ProviderKeyStore) -> Result<Credential, String> {
    for &name in p.key_env_vars() {
        if let Some(val) = state.env_var(name).and_then(|v| v.into_string().ok()) {
            if !val.trim().is_empty() {
                let key = SecretKey::new(val).map_err(|e| format!("the key in {name} cannot be used: {}", e.message))?;
                return Ok(Credential { key: Some(key), source: Some("environment"), env_name: Some(name) });
            }
        }
    }
    match store.key_for(p) {
        Some(raw) => {
            let key = SecretKey::new(raw.to_string()).map_err(|e| format!("the saved key cannot be used: {}", e.message))?;
            Ok(Credential { key: Some(key), source: Some("saved key"), env_name: None })
        }
        None => Ok(Credential { key: None, source: None, env_name: None }),
    }
}

/// The first non-empty generic key variable ([`GENERIC_KEY_ENV_VARS`]), by name only.
fn generic_key_name(state: &DesktopState) -> Option<&'static str> {
    GENERIC_KEY_ENV_VARS
        .into_iter()
        .find(|n| state.env_var(n).and_then(|v| v.into_string().ok()).is_some_and(|v| !v.trim().is_empty()))
}

/// The provider the generic key belongs to, when one is named explicitly
/// (`RE_SEARCH_PROVIDER` / `RESEARCH_CORE_PROVIDER`, as for the console).
fn generic_key_owner(state: &DesktopState) -> Option<ProviderKind> {
    PROVIDER_ENV_VARS
        .into_iter()
        .find_map(|n| state.env_var(n).and_then(|v| v.into_string().ok()).filter(|v| !v.trim().is_empty()))
        .and_then(|v| ProviderKind::parse(&v).ok())
        .filter(|&p| p != ProviderKind::ChatGpt)
}

/// What a model-list call for one provider may do. List calls are made by the desktop on
/// its own (at start, on refresh), not because the person chose that provider, so they use
/// only that provider's own credential: a saved key or its own variable
/// ([`ProviderKind::own_key_env_vars`]); a generic key only when `RE_SEARCH_PROVIDER` names
/// this provider. With a launch `--api-base` every provider's calls would go to that one
/// address, so no list call is made at all.
pub enum ListAccess {
    /// Call the list endpoint with this key (`None`: keyless, Ollama only).
    Call(Option<SecretKey>),
    /// Connected for turns, but no list call is made; the reason, for status and errors.
    NoCall(String),
    /// Not connected: nothing this provider's own credential could be.
    Off,
}

/// Why no key is sent while a launch override is set.
fn override_reason(state: &DesktopState) -> String {
    let base = state.cfg.api_base_override.as_deref().unwrap_or("");
    format!(
        "model list not fetched: re:Search was started with --api-base {base}, which every provider's calls would go to, so no key is sent there to list models"
    )
}

/// This provider's own key (see [`ListAccess`]); `Ok(None)` when it has none.
pub(crate) fn own_key(state: &DesktopState, p: ProviderKind, store: &ProviderKeyStore) -> Result<Option<SecretKey>, String> {
    if p == ProviderKind::ChatGpt {
        return Ok(None);
    }
    let owned_generic: &[&str] = if generic_key_owner(state) == Some(p) { &GENERIC_KEY_ENV_VARS } else { &[] };
    for &name in owned_generic.iter().chain(p.own_key_env_vars()) {
        if let Some(val) = state.env_var(name).and_then(|v| v.into_string().ok()) {
            if !val.trim().is_empty() {
                return SecretKey::new(val).map(Some).map_err(|e| format!("the key in {name} cannot be used: {}", e.message));
            }
        }
    }
    match store.key_for(p) {
        Some(raw) => SecretKey::new(raw.to_string()).map(Some).map_err(|e| format!("the saved key cannot be used: {}", e.message)),
        None => Ok(None),
    }
}

pub fn list_access(state: &DesktopState, p: ProviderKind, store: &ProviderKeyStore) -> Result<ListAccess, String> {
    let key = own_key(state, p, store)?;
    if p == ProviderKind::Ollama {
        return Ok(match state.cfg.api_base_override {
            Some(_) => ListAccess::NoCall(override_reason(state)),
            None => ListAccess::Call(key),
        });
    }
    Ok(match (key, state.cfg.api_base_override.is_some()) {
        (None, _) => ListAccess::Off,
        (Some(_), true) => ListAccess::NoCall(override_reason(state)),
        (Some(k), false) => ListAccess::Call(Some(k)),
    })
}

/// Whether the Claude subscription route can start a turn now, and why not.
pub struct ClaudeStatus {
    pub connected: bool,
    pub status: String,
    pub how_to: Option<String>,
    pub version: Option<String>,
}

fn names_verb(names: &[&str]) -> (String, &'static str) {
    (names.join(", "), if names.len() == 1 { "is" } else { "are" })
}

pub fn claude_status(state: &DesktopState) -> ClaudeStatus {
    let env = |n: &str| state.env_var(n);
    let exe = ClaudeCode.locate(state.cfg.claude_exe.as_deref(), &env).ok().filter(|p| p.is_file());
    let Some(exe) = exe else {
        return ClaudeStatus {
            connected: false,
            status: "Claude Code was not found on this computer.".to_string(),
            how_to: Some(CLAUDE_HOW_TO.to_string()),
            version: None,
        };
    };
    // Billing safety, as for a turn: never the variables' values, only their names.
    let allowed = match env_auth::check(CLAUDE_ENV_AUTH_VARS, &env) {
        Ok(names) => names,
        Err(_) => {
            let (list, verb) = names_verb(&env_auth::detect(CLAUDE_ENV_AUTH_VARS, &env));
            return ClaudeStatus {
                connected: false,
                status: format!(
                    "Blocked: {list} {verb} set in re:Search's environment, and Claude Code would use it instead of your subscription."
                ),
                how_to: Some(format!(
                    "Unset {list} before starting re:Search, or set {ALLOW_ENV_AUTH_VAR}=1 to use it anyway."
                )),
                version: None,
            };
        }
    };
    // A successful probe is kept for the launch; a failed one is re-run by the next check.
    let version = match state.claude_version.get() {
        Some(v) => Ok(v.clone()),
        None => run_version(&exe, VERSION_TIMEOUT).inspect(|v| {
            let _ = state.claude_version.set(v.clone());
        }),
    };
    let mut status = match &version {
        Ok(v) => format!("Claude Code {v} found; it uses the account you signed into by running claude."),
        Err(e) => format!("Claude Code found, but its version check failed ({e})."),
    };
    if !allowed.is_empty() {
        let (list, verb) = names_verb(&allowed);
        status.push_str(&format!(" {list} {verb} set and allowed by {ALLOW_ENV_AUTH_VAR}: it may be billed instead of your subscription."));
    }
    if env("CLAUDE_CODE_SIMPLE").is_some_and(|v| !v.is_empty() && v != "0") {
        status.push_str(" CLAUDE_CODE_SIMPLE is set, which makes Claude Code ignore your subscription login.");
    }
    ClaudeStatus { connected: true, status, how_to: None, version: version.ok() }
}

/// `claude --version` with a timeout (the same bounded probe as a turn's:
/// [`cli_route::claude::probe_version`]); sends nothing to a model. Only the leading
/// version token is kept, limited to version characters.
fn run_version(exe: &Path, timeout: Duration) -> Result<String, String> {
    let raw = cli_route::claude::probe_version(exe, timeout)?;
    let v: String = parse_cli_version(&raw)
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

fn claude_entry(state: &DesktopState) -> Value {
    let st = claude_status(state);
    json!({
        "id": CLAUDE_CLI_ROUTE,
        "label": CLAUDE_CLI_LABEL,
        "kind": "subscription",
        "route": CLAUDE_CLI_ROUTE,
        "connected": st.connected,
        "status": st.status,
        "how_to": st.how_to,
        "version": st.version,
        "accepts_key": false,
        "key_required": false,
        "has_saved_key": false,
        "key_source": null,
        "key_env": null,
        "base_url": null,
        "session_note": SUBSCRIPTION_NOTE,
    })
}

fn api_entry(state: &DesktopState, store: &ProviderKeyStore, p: ProviderKind, refresh: bool, since: u64) -> Value {
    let id = p.cli_name();
    let (base_url, base_from) = state.base_url_for(p, store);
    // A saved base URL is shown back only if it passes the settings check (as in the
    // redacted settings view); the launch override and the default are configuration.
    let shown_base = match base_from {
        "saved" => store.redacted_view().providers.get(id).and_then(|e| e.base_url.clone()),
        _ => Some(base_url.clone()),
    };
    let cred = credential(state, p, store);
    let (key_source, key_env) = match &cred {
        Ok(c) => (c.source, c.env_name),
        Err(_) => (None, None),
    };
    // "Connected" means this provider's own credential is there (a saved key, its own
    // variable, or the generic key tied to it by RE_SEARCH_PROVIDER); for Ollama, that it
    // answered. `key_source` says what a turn would use.
    let access = list_access(state, p, store);
    let (connected, status) = match (&access, &cred) {
        (Err(e), _) | (_, Err(e)) => (false, format!("The key cannot be used: {e}.")),
        (Ok(ListAccess::NoCall(reason)), _) if p == ProviderKind::Ollama => (false, format!("Not checked: {reason}.")),
        (Ok(ListAccess::Call(key)), _) if p == ProviderKind::Ollama => {
            let f = state.catalog.fetch(state, store, p, key.as_ref(), refresh, since);
            match (f.ever_ok, &f.error) {
                (true, None) => (true, format!("Running at {}; {} models.", shown_base.as_deref().unwrap_or("its address"), f.models.len())),
                (true, Some(e)) => (true, format!("Not answering now ({e}); showing the last list.")),
                (false, Some(e)) => (false, format!("Not answering at {} ({e}).", shown_base.as_deref().unwrap_or("its address"))),
                (false, None) => (false, "Not answering.".to_string()),
            }
        }
        (Ok(ListAccess::Off), _) => match generic_key_name(state) {
            Some(name) => (
                false,
                format!(
                    "{name} is set, but it is not tied to one provider, so it is not counted as this provider's key and is not sent to list models (a turn on this provider would still use it). Save a key here, set {}, or set RE_SEARCH_PROVIDER={id}.",
                    p.own_key_env_vars()[0]
                ),
            ),
            None => (false, "No API key yet.".to_string()),
        },
        (Ok(access), Ok(c)) => {
            let from = match (c.source, c.env_name) {
                (Some("environment"), Some(name)) => format!("key from the environment ({name})"),
                _ => "saved key".to_string(),
            };
            match (access, state.catalog.last_error(id)) {
                (ListAccess::NoCall(reason), _) => (true, format!("Connected with a {from}; {reason}.")),
                (_, Some(err)) => (true, format!("Connected with a {from}, but the model list failed: {err}")),
                _ => (true, format!("Connected with a {from}.")),
            }
        }
    };
    json!({
        "id": id,
        "label": api_label(p),
        "kind": "api-key",
        "route": "api",
        "connected": connected,
        "status": status,
        "how_to": (!connected).then(|| api_how_to(p)),
        "version": null,
        "accepts_key": true,
        "key_required": p != ProviderKind::Ollama,
        "has_saved_key": store.key_for(p).is_some(),
        "key_source": key_source,
        "key_env": key_env,
        "base_url": shown_base,
        "base_url_source": base_from,
        "default_base_url": p.default_base_url(),
        "session_note": null,
    })
}

/// `GET /api/connections[?refresh=1]` (token required). `refresh=1` re-asks Ollama.
pub fn handle_connections(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let refresh = query.get("refresh").is_some_and(|v| v == "1" || v == "true");
    let since = state.catalog.generation();
    let store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).map_err(|e| (500, e))?;
    let mut list = vec![claude_entry(state), crate::antigravity::connection_entry(state), crate::chatgpt::connection_entry(state)];
    for p in API_PROVIDERS {
        list.push(api_entry(state, &store, p, refresh, since));
    }
    let sympy_worker = if !cfg!(windows) {
        "unsupported_platform"
    } else if state.cfg.python.is_some() && state.cfg.worker_script.is_some() {
        "configured"
    } else {
        "unconfigured"
    };
    Ok((200, json!({
        "connections": list,
        "capabilities": { "sympy_worker": sympy_worker },
    })))
}
