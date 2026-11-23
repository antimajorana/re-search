//! `GET /api/models`: the models the person can pick right now, grouped by connected
//! provider, for the composer's single model dropdown.
//!
//! API providers' lists are fetched live by the server with that provider's own
//! credential only (a saved key or its own variable, never a generic key unless
//! `RE_SEARCH_PROVIDER` names the provider, and no list call at all under a launch
//! `--api-base`: [`crate::connections::ListAccess`]), through `agent::model_list`
//! (configured base URL only, https off loopback, no redirects, bounded size, key in a
//! header, errors reduced to a status class), and kept in memory for [`CACHE_TTL`];
//! `?refresh=1` bypasses the cache, and a fetch begun before [`Catalog::clear`] does not
//! store its result. A failing provider never breaks the
//! others: it is returned with an `error`, its last good list, or else a small built-in
//! list, marked `stale`. The Claude subscription route has no list endpoint that can be
//! called without a prompt, so it offers the CLI's own aliases, which always resolve to
//! the latest models. The ChatGPT plan (Sign in with ChatGPT) is listed once signed in
//! with plan use granted: `GET https://api.openai.com/v1/models` with the access token,
//! keeping entries with `visibility == "list"`, through the same cache and class-only
//! errors. The Antigravity route (once acknowledged) lists `agy models` (a list call with
//! the CLI's own login, no prompt), cached the same way, else a small built-in list marked
//! as such. Model ids and labels from providers are untrusted text.

use crate::connections::{
    api_label, claude_status, list_access, ListAccess, API_PROVIDERS, CLAUDE_CLI_LABEL, SUBSCRIPTION_NOTE,
};
use crate::routes_session::ApiResult;
use crate::state::{now_ms, DesktopState, UiSettings, CLAUDE_CLI_ROUTE};
use agent::model_list::{describe_list_error, list_models, ListedModel};
use agent::{ProviderKeyStore, ProviderKind, SecretKey};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a fetched list is reused.
pub const CACHE_TTL: Duration = Duration::from_secs(600);
/// How long a failed fetch is remembered before the next request tries again.
const ERROR_TTL: Duration = Duration::from_secs(30);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
/// Ollama runs locally: a server that does not answer quickly is treated as not running.
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_ERROR_CHARS: usize = 300;

/// `--model` aliases of the Claude CLI. `claude --help` (2.1.318) documents "an alias for
/// the latest model (e.g. 'fable', 'opus', or 'sonnet')"; `haiku` is the alias the
/// console route has always used. Default: [`crate::cli_turn::DEFAULT_CLI_MODEL`].
pub const CLAUDE_ALIASES: [(&str, &str); 4] = [
    ("fable", "Claude Fable (latest)"),
    ("opus", "Claude Opus (latest)"),
    ("sonnet", "Claude Sonnet (latest)"),
    ("haiku", "Claude Haiku (latest)"),
];

#[derive(Clone)]
struct Entry {
    /// Last good list (empty if none yet) and when it was fetched.
    models: Vec<ListedModel>,
    ok_ms: Option<i64>,
    /// Result of the last attempt and when it was made.
    last_error: Option<String>,
    checked: Instant,
}

#[derive(Default)]
struct Entries {
    /// Bumped by [`Catalog::clear`]: a fetch that began under an older generation (with an
    /// old key or base URL) does not store its result.
    generation: u64,
    map: HashMap<&'static str, Entry>,
}

/// In-memory model lists, by provider id.
#[derive(Default)]
pub struct Catalog {
    entries: Mutex<Entries>,
}

/// One provider's list as served.
pub struct Fetched {
    pub models: Vec<ListedModel>,
    pub error: Option<String>,
    /// The list is not from a successful call just now or within the cache time.
    pub stale: bool,
    /// `"live"`, `"cache"` or `"fallback"`.
    pub source: &'static str,
    pub fetched_ms: Option<i64>,
    /// A list call has succeeded at least once in this launch.
    pub ever_ok: bool,
}

fn clip(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    flat.chars().take(MAX_ERROR_CHARS).collect()
}

impl Catalog {
    /// Forget every list (after keys or base URLs change).
    pub fn clear(&self) {
        if let Ok(mut e) = self.entries.lock() {
            e.generation = e.generation.wrapping_add(1);
            e.map.clear();
        }
    }

    /// Error of the last list attempt for `id`, if it failed.
    pub fn last_error(&self, id: &str) -> Option<String> {
        self.entries.lock().ok()?.map.get(id)?.last_error.clone()
    }

    /// True if a live list fetch for `id` has succeeded and has not subsequently failed.
    pub fn has_ok_entry(&self, id: &str) -> bool {
        self.entries
            .lock()
            .ok()
            .and_then(|e| {
                e.map
                    .get(id)
                    .map(|ent| ent.ok_ms.is_some() && ent.last_error.is_none())
            })
            .unwrap_or(false)
    }

    /// The current generation. Read it BEFORE loading keys and base URLs, and pass it to
    /// [`Catalog::fetch`], so a result fetched with settings older than a `clear` is dropped.
    pub fn generation(&self) -> u64 {
        self.entries.lock().map(|e| e.generation).unwrap_or(u64::MAX)
    }

    /// The cached entry for `id` and the current generation.
    fn begin(&self, id: &str) -> (Option<Entry>, u64) {
        match self.entries.lock() {
            Ok(e) => (e.map.get(id).cloned(), e.generation),
            Err(_) => (None, u64::MAX),
        }
    }

    /// Store `entry` unless [`Catalog::clear`] ran since `generation` was read.
    fn commit(&self, generation: u64, id: &'static str, entry: Entry) -> bool {
        match self.entries.lock() {
            Ok(mut e) if e.generation == generation => {
                e.map.insert(id, entry);
                true
            }
            _ => false,
        }
    }

    /// Return the cached entry for `p` if it is fresh and matches `since` (without fetching a credential).
    pub fn cached_fresh(
        &self,
        state: &DesktopState,
        store: &ProviderKeyStore,
        p: ProviderKind,
        force: bool,
        since: u64,
    ) -> Option<Fetched> {
        if force {
            return None;
        }
        let (cached, now) = self.begin(p.cli_name());
        let e = cached.filter(|_| now == since)?;
        let ttl = if e.last_error.is_some() { ERROR_TTL } else { CACHE_TTL };
        (e.checked.elapsed() < ttl).then(|| served(p, store, state, &e, "cache"))
    }

    /// The provider's list: from the cache while fresh (unless `force`), else fetched with
    /// `key` (the provider's own credential: [`crate::connections::list_access`]).
    pub fn fetch(
        &self,
        state: &DesktopState,
        store: &ProviderKeyStore,
        p: ProviderKind,
        key: Option<&SecretKey>,
        force: bool,
        since: u64,
    ) -> Fetched {
        let id = p.cli_name();
        let (cached, now) = self.begin(id);
        // `since` predates the caller's key/base-URL load; if a clear happened in between,
        // commit() below refuses (generation mismatch) and the cache entry is not trusted.
        let generation = since;
        let cached = cached.filter(|_| now == since);
        if let Some(e) = cached.as_ref().filter(|_| !force) {
            let ttl = if e.last_error.is_some() { ERROR_TTL } else { CACHE_TTL };
            if e.checked.elapsed() < ttl {
                return served(p, store, state, e, "cache");
            }
        }
        let (base, _) = state.base_url_for(p, store);
        let timeout = if p == ProviderKind::Ollama { OLLAMA_TIMEOUT } else { LIST_TIMEOUT };
        let result = list_models(p, &base, key, timeout).map_err(|e| describe_list_error(&e));
        let mut entry = cached.unwrap_or(Entry { models: Vec::new(), ok_ms: None, last_error: None, checked: Instant::now() });
        entry.checked = Instant::now();
        match result {
            Ok(models) => {
                entry.models = models;
                entry.ok_ms = Some(now_ms());
                entry.last_error = None;
            }
            Err(e) => entry.last_error = Some(clip(&e)),
        }
        self.commit(generation, id, entry.clone());
        served(p, store, state, &entry, "live")
    }
}

fn served(p: ProviderKind, store: &ProviderKeyStore, state: &DesktopState, e: &Entry, how: &'static str) -> Fetched {
    let ever_ok = e.ok_ms.is_some();
    match (&e.last_error, ever_ok) {
        (None, _) => Fetched { models: e.models.clone(), error: None, stale: false, source: how, fetched_ms: e.ok_ms, ever_ok },
        (Some(err), true) => Fetched {
            models: e.models.clone(),
            error: Some(err.clone()),
            stale: true,
            source: "cache",
            fetched_ms: e.ok_ms,
            ever_ok,
        },
        (Some(err), false) => Fetched {
            models: fallback(p, store, &state.load_ui_settings()),
            error: Some(err.clone()),
            stale: true,
            source: "fallback",
            fetched_ms: None,
            ever_ok,
        },
    }
}

fn saved_default(p: ProviderKind, store: &ProviderKeyStore, ui: &UiSettings) -> Option<String> {
    let from_ui = ui
        .default_provider
        .as_deref()
        .filter(|d| ProviderKind::parse(d).ok() == Some(p))
        .and(ui.default_model.clone());
    store.providers.get(p.cli_name()).and_then(|e| e.default_model.clone()).or(from_ui)
}

/// The built-in list when a provider has never answered: the saved default model, if
/// any, and re:Search's default for that provider.
fn fallback(p: ProviderKind, store: &ProviderKeyStore, ui: &UiSettings) -> Vec<ListedModel> {
    let mut out: Vec<ListedModel> = Vec::new();
    for id in saved_default(p, store, ui).into_iter().chain([p.default_model().to_string()]) {
        if !id.trim().is_empty() && !out.iter().any(|m| m.id == id) {
            out.push(ListedModel { label: id.clone(), id, created_s: None });
        }
    }
    out
}

fn default_id(p: ProviderKind, models: &[ListedModel], store: &ProviderKeyStore, ui: &UiSettings) -> Option<String> {
    let has = |id: &str| models.iter().any(|m| m.id == id);
    saved_default(p, store, ui)
        .filter(|id| has(id))
        .or_else(|| Some(p.default_model().to_string()).filter(|id| has(id)))
        .or_else(|| models.first().map(|m| m.id.clone()))
}

fn model_json(provider: &str, id: &str, label: &str, is_default: bool) -> Value {
    json!({ "id": id, "label": label, "provider": provider, "default": is_default })
}

fn claude_group() -> Value {
    let default = crate::cli_turn::DEFAULT_CLI_MODEL;
    let models: Vec<Value> =
        CLAUDE_ALIASES.iter().map(|(id, label)| model_json(CLAUDE_CLI_ROUTE, id, label, *id == default)).collect();
    json!({
        "id": CLAUDE_CLI_ROUTE,
        "label": CLAUDE_CLI_LABEL,
        "kind": "subscription",
        "route": CLAUDE_CLI_ROUTE,
        "session_note": SUBSCRIPTION_NOTE,
        "models": models,
        "source": "cli-aliases",
        "stale": false,
        "error": null,
        "fetched_ms": null,
    })
}

/// The ChatGPT plan's group, when signed in with plan use granted.
fn chatgpt_group(state: &DesktopState, store: &ProviderKeyStore, ui: &UiSettings, refresh: bool, since: u64) -> Option<Value> {
    use crate::chatgpt::{CHATGPT_ID, CHATGPT_LABEL, CHATGPT_NOTE};
    use chatgpt_auth::TokenError;
    if !state.chatgpt.ready() {
        return None;
    }
    let p = ProviderKind::ChatGpt;
    let f = if let Some(cached) = state.catalog.cached_fresh(state, store, p, refresh, since) {
        cached
    } else {
        match state.chatgpt.auth.access_token() {
            Ok(t) => match SecretKey::new(t.as_str().to_string()) {
                Ok(key) => state.catalog.fetch(state, store, p, Some(&key), refresh, since),
                Err(e) => Fetched {
                    models: fallback(p, store, ui),
                    error: Some(clip(&e.message)),
                    stale: true,
                    source: "fallback",
                    fetched_ms: None,
                    ever_ok: false,
                },
            },
            Err(TokenError::Disconnected(_) | TokenError::NotSignedIn | TokenError::PlanNotEnabled) => return None,
            Err(e) => Fetched {
                models: fallback(p, store, ui),
                error: Some(clip(&e.to_string())),
                stale: true,
                source: "fallback",
                fetched_ms: None,
                ever_ok: false,
            },
        }
    };
    let default = default_id(p, &f.models, store, ui);
    let models: Vec<Value> =
        f.models.iter().map(|m| model_json(CHATGPT_ID, &m.id, &m.label, default.as_deref() == Some(m.id.as_str()))).collect();
    Some(json!({
        "id": CHATGPT_ID,
        "label": CHATGPT_LABEL,
        "kind": "subscription",
        "route": "api",
        "session_note": CHATGPT_NOTE,
        "models": models,
        "source": f.source,
        "stale": f.stale,
        "error": f.error,
        "fetched_ms": f.fetched_ms,
    }))
}

/// `GET /api/models[?refresh=1]` (token required).
pub fn handle_models(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    state.chatgpt.sync_catalog(&state.catalog);
    let refresh = query.get("refresh").is_some_and(|v| v == "1" || v == "true");
    let since = state.catalog.generation();
    let store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).map_err(|e| (500, e))?;
    let ui = state.load_ui_settings();
    let mut groups = Vec::new();
    if claude_status(state).connected {
        groups.push(claude_group());
    }
    // Only once found, with a clean environment, and acknowledged by the person.
    if let Some(g) = crate::antigravity::model_group(state, refresh) {
        groups.push(g);
    }
    // One thread per provider (including ChatGPT), so a slow provider does not hold up the others.
    let (chatgpt, fetched): (Option<Value>, Vec<(ProviderKind, Option<Fetched>)>) = std::thread::scope(|s| {
        let store = &store;
        let ui = &ui;
        let cg_handle = s.spawn(move || chatgpt_group(state, store, ui, refresh, since));
        let handles: Vec<_> = API_PROVIDERS
            .iter()
            .map(|&p| {
                s.spawn(move || {
                    let fetched = match list_access(state, p, store) {
                        Ok(ListAccess::Call(key)) => Some(state.catalog.fetch(state, store, p, key.as_ref(), refresh, since)),
                        // Connected, but no list call may go out (a launch --api-base):
                        // the built-in names, with the reason as the error.
                        Ok(ListAccess::NoCall(reason)) if p != ProviderKind::Ollama => Some(Fetched {
                            models: fallback(p, store, &state.load_ui_settings()),
                            error: Some(reason),
                            stale: true,
                            source: "fallback",
                            fetched_ms: None,
                            ever_ok: false,
                        }),
                        // A key that cannot be used is reported by /api/connections.
                        Ok(_) | Err(_) => None,
                    };
                    (p, fetched)
                })
            })
            .collect();
        let cg = cg_handle.join().ok().flatten();
        let api = handles.into_iter().filter_map(|h| h.join().ok()).collect();
        (cg, api)
    });
    if let Some(g) = chatgpt {
        groups.push(g);
    }
    for (p, f) in fetched {
        let Some(f) = f else { continue };
        // Ollama counts as connected only once it has answered.
        if p == ProviderKind::Ollama && !f.ever_ok {
            continue;
        }
        let id = p.cli_name();
        let default = default_id(p, &f.models, &store, &ui);
        let models: Vec<Value> = f
            .models
            .iter()
            .map(|m| model_json(id, &m.id, &m.label, default.as_deref() == Some(m.id.as_str())))
            .collect();
        groups.push(json!({
            "id": id,
            "label": api_label(p),
            "kind": "api-key",
            "route": "api",
            "session_note": null,
            "models": models,
            "source": f.source,
            "stale": f.stale,
            "error": f.error,
            "fetched_ms": f.fetched_ms,
        }));
    }
    Ok((200, json!({ "providers": groups, "cache_ttl_s": CACHE_TTL.as_secs() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            models: vec![ListedModel { id: id.to_string(), label: id.to_string(), created_s: None }],
            ok_ms: Some(1),
            last_error: None,
            checked: Instant::now(),
        }
    }

    #[test]
    fn a_fetch_begun_before_clear_does_not_repopulate_the_cache() {
        let c = Catalog::default();
        let (_, before) = c.begin("openai");
        // A key or base URL changes while the old list call is still running.
        c.clear();
        assert!(!c.commit(before, "openai", entry("old-key-model")), "stale result refused");
        assert!(c.begin("openai").0.is_none(), "nothing cached from the old call");
        // A fetch begun after the change stores its result as usual.
        let (_, now) = c.begin("openai");
        assert!(c.commit(now, "openai", entry("new-key-model")));
        assert_eq!(c.begin("openai").0.unwrap().models[0].id, "new-key-model");
    }
}
