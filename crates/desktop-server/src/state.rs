//! Shared desktop server state, pending UI approvals, settings, and provider construction.

use crate::auth::AuthGuard;
use crate::sse::SseHub;
use crate::DesktopConfig;
use agent::{
    AnthropicConfig, AnthropicTransport, GeminiConfig, GeminiTransport, OpenAiConfig, OpenAiTransport,
    ProviderKeyStore, ProviderKind, RoleAssignment, SecretKey, Transport,
};
use ledger::Ledger;
use research_tools::{validate_session_id, EditPreview, ResearchEnv};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

const UI_SETTINGS_FILE: &str = "ui_settings.json";
pub(crate) const DEFAULT_SESSION_TITLE: &str = "Research Session";
static UI_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_model_title: Option<bool>,
    pub workspace_root: Option<String>,
    /// An explicitly detached or restored session must not inherit the launch workspace.
    #[serde(default)]
    pub workspace_detached: bool,
    pub pinned_conventions: Option<String>,
    #[serde(default)]
    pub verification_mode: bool,
    #[serde(default)]
    pub calculation_mode: bool,
    pub model_preset: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub created_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_info: Option<ForkInfo>,
    /// `Some("claude-cli")` / `Some("antigravity")` once the session is bound to that
    /// subscription-CLI route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub deleted: bool,
}

impl SessionMeta {
    pub(crate) fn allows_model_title(&self) -> bool {
        self.allow_model_title.unwrap_or(self.title == DEFAULT_SESSION_TITLE)
    }
}

/// Route value stored in [`SessionMeta::route`] (and accepted as `provider`) for the
/// Claude subscription route (the Claude Code CLI driven by re:Search).
pub const CLAUDE_CLI_ROUTE: &str = "claude-cli";

/// Route value stored in [`SessionMeta::route`] (and accepted as `provider`) for the
/// Antigravity route (the Antigravity CLI driven by re:Search; vendor-sandboxed, not
/// confined to re:Search tools). Also the route label in its ledger start events.
pub const ANTIGRAVITY_ROUTE: &str = "antigravity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkInfo {
    pub parent_session: String,
    pub forked_at_seq: u64,
    pub modified_assumption: Option<String>,
}

pub fn default_auto_approve_tools() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub pinned_conventions: Option<String>,
    pub role_presets: Option<Value>,
    /// The person's acknowledgement of the Antigravity route's warning (terms risk, built-in
    /// tools outside re:Search's control), given by clicking in the Connect panel. Without
    /// it (or for an older notice version) the route refuses to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub antigravity_ack: Option<AntigravityAck>,
    #[serde(default = "default_auto_approve_tools")]
    pub auto_approve_tools: bool,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub agent_models: std::collections::BTreeMap<String, crate::agent_models::AgentModelSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_readers_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_reader_max_parallel: Option<usize>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            default_provider: None, default_model: None, pinned_conventions: None,
            role_presets: None, antigravity_ack: None,
            auto_approve_tools: default_auto_approve_tools(),
            agent_models: std::collections::BTreeMap::new(),
            paper_readers_enabled: None, paper_reader_max_parallel: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AntigravityAck {
    pub acknowledged_ms: i64,
    /// [`crate::antigravity::NOTICE_VERSION`] of the warning that was shown.
    pub notice_version: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingApprovalView {
    pub call_id: String,
    pub caller: String,
    pub session: String,
    pub tool_id: String,
    pub effect: String,
    pub allow_session_approve: bool,
    pub canonical_args: String,
    pub digest: String,
    pub scope: String,
    pub containment: String,
    pub contained: bool,
    pub edit_preview: Option<EditPreview>,
    pub created_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    pub digest: String,
    pub approved: bool,
    pub approve_for_session: bool,
    pub responder: String,
}

pub struct PendingSlot {
    pub view: PendingApprovalView,
    pub decision: Mutex<Option<ApprovalDecision>>,
    pub cv: Condvar,
}

pub struct DesktopState {
    pub cfg: DesktopConfig,
    pub auth: AuthGuard,
    pub sse: Arc<SseHub>,
    pub active_turns: Mutex<HashSet<String>>,
    pub(crate) backup_downloads: Mutex<HashMap<String, crate::projects::backup::Download>>,
    pub pending_approvals: Mutex<HashMap<(String, String), Arc<PendingSlot>>>,
    pub ledger_write_lock: Mutex<()>,
    /// Serializes load-modify-save updates to `ui_settings.json`.
    pub ui_settings_lock: Mutex<()>,
    /// Running subscription-CLI turns (`claude-cli`, `antigravity`), by session: route kind + stop handle.
    pub cli_turns: Mutex<HashMap<String, (crate::cli_turn::CliRouteKind, cli_route::StopHandle)>>,
    /// Running in-core API / ChatGPT turns, by session: cooperative cancellation token.
    pub api_turns: Mutex<HashMap<String, agent::CancelToken>>,
    /// Session-scoped human-approved capability grants keyed by `(session_id, effect)`.
    pub session_capability_grants: Mutex<HashMap<(String, policy::EffectClass), crate::session_lifecycle::SessionCapabilityGrant>>,
    /// Model lists fetched from providers, kept for [`crate::model_catalog::CACHE_TTL`].
    pub catalog: crate::model_catalog::Catalog,
    /// `claude --version` output once a probe succeeded (a failed probe is not kept).
    pub claude_version: std::sync::OnceLock<String>,
    /// Sign in with ChatGPT: tokens, sign-in attempts and the browser opener.
    pub chatgpt: crate::chatgpt::ChatGptHub,
    /// `agy --version` output once a probe succeeded (a failed probe is not kept).
    pub agy_version: std::sync::OnceLock<String>,
    /// Generation-tagged `agy models` cache, kept for [`crate::model_catalog::CACHE_TTL`].
    pub agy_models: Mutex<(u64, Option<crate::antigravity::AgyModelCache>)>,
    /// Stage 15 Human-in-the-Loop steer queue and pending checkpoint slots.
    pub hitl: crate::hitl::HitlHub,
}

impl DesktopState {
    pub fn new(cfg: DesktopConfig, auth: AuthGuard, sse: Arc<SseHub>, chatgpt: crate::chatgpt::ChatGptHub) -> Self {
        Self {
            cfg,
            auth,
            sse,
            active_turns: Mutex::new(HashSet::new()),
            backup_downloads: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            ledger_write_lock: Mutex::new(()),
            ui_settings_lock: Mutex::new(()),
            cli_turns: Mutex::new(HashMap::new()),
            api_turns: Mutex::new(HashMap::new()),
            session_capability_grants: Mutex::new(HashMap::new()),
            catalog: crate::model_catalog::Catalog::default(),
            claude_version: std::sync::OnceLock::new(),
            chatgpt,
            agy_version: std::sync::OnceLock::new(),
            agy_models: Mutex::new((0, None)),
            hitl: crate::hitl::HitlHub::default(),
        }
    }

    /// Deny (never approve) pending approval cards of `session`, optionally only the one
    /// bound to `digest`. Used when the caller went away, the turn was stopped, or the
    /// server shuts down; `responder` says why. Returns how many cards were denied.
    pub fn deny_pending(&self, session: &str, digest: Option<&str>, responder: &str) -> usize {
        let slots: Vec<Arc<PendingSlot>> = match self.pending_approvals.lock() {
            Ok(map) => map
                .iter()
                .filter(|((sid, _), slot)| sid == session && digest.is_none_or(|d| slot.view.digest == d))
                .map(|(_, slot)| Arc::clone(slot))
                .collect(),
            Err(_) => return 0,
        };
        let mut denied = 0;
        for slot in slots {
            if let Ok(mut dec) = slot.decision.lock() {
                if dec.is_none() {
                    *dec = Some(ApprovalDecision {
                        digest: slot.view.digest.clone(),
                        approved: false,
                        approve_for_session: false,
                        responder: responder.to_string(),
                    });
                    slot.cv.notify_all();
                    denied += 1;
                }
            }
        }
        denied
    }

    /// Stop every running subscription-CLI turn of `kind` and deny its open approval cards.
    pub fn stop_cli_turns_of(&self, kind: crate::cli_turn::CliRouteKind) -> usize {
        let targets: Vec<(String, cli_route::StopHandle)> = match self.cli_turns.lock() {
            Ok(map) => map
                .iter()
                .filter(|(_, (k, _))| *k == kind)
                .map(|(sid, (_, h))| (sid.clone(), h.clone()))
                .collect(),
            Err(_) => return 0,
        };
        for (sid, h) in &targets {
            h.stop();
            self.deny_pending(sid, None, "supervisor: route acknowledgement withdrawn");
        }
        targets.len()
    }

    /// Stop every running subscription-CLI turn (kills each CLI process tree), then wait up to
    /// five seconds for those turns to record `session.abort` and delete their `mcp.json`
    /// (the process may exit right after this, e.g. when the app window closes).
    pub fn stop_all_cli_turns(&self) {
        let handles: Vec<cli_route::StopHandle> = match self.cli_turns.lock() {
            Ok(map) => map.values().map(|(_, h)| h.clone()).collect(),
            Err(_) => return,
        };
        if handles.is_empty() {
            return;
        }
        for h in handles {
            h.stop();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if self.cli_turns.lock().map(|m| m.is_empty()).unwrap_or(true) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    pub fn open_ledger(&self) -> Result<Ledger, String> {
        Ledger::open(&self.cfg.data_dir).map_err(|e| e.to_string())
    }

    pub fn session_dir(&self, session_id: &str) -> Result<PathBuf, String> {
        validate_session_id(session_id).map_err(|e| e.to_string())?;
        Ok(self.cfg.data_dir.join("sessions").join(session_id))
    }

    pub fn meta_path(&self, session_id: &str) -> Result<PathBuf, String> {
        Ok(self.session_dir(session_id)?.join("meta.json"))
    }

    pub fn load_meta(&self, session_id: &str) -> Option<SessionMeta> {
        let path = self.meta_path(session_id).ok()?;
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        let dir = self.session_dir(&meta.id)?;
        fs::create_dir_all(&dir).map_err(|e| format!("failed to create session dir: {e}"))?;
        let path = dir.join("meta.json");
        let tmp = dir.join(format!(".meta.tmp.{}", std::process::id()));
        let mut bytes = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        fs::write(&tmp, &bytes).map_err(|e| format!("failed to write meta: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("failed to install meta: {e}"))?;
        Ok(())
    }

    pub fn list_meta_ids(&self) -> Vec<String> {
        let root = self.cfg.data_dir.join("sessions");
        let Ok(rd) = fs::read_dir(root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            if entry.path().join("meta.json").is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    if validate_session_id(name).is_ok() {
                        out.push(name.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    pub(crate) fn session_restore_pending(&self, session_id: &str) -> Result<bool, String> {
        validate_session_id(session_id).map_err(|e| e.to_string())?;
        self.cfg.data_dir.join(research_tools::session_activity::pending_name(session_id))
            .try_exists().map_err(|e| e.to_string())
    }

    pub fn session_exists(&self, session_id: &str) -> Result<bool, String> {
        if self.session_restore_pending(session_id)? { return Ok(false); }
        if let Some(meta) = self.load_meta(session_id) {
            return Ok(!meta.deleted);
        }
        let ledger = self.open_ledger()?;
        Ok(!ledger.read_session(session_id).map_err(|e| e.to_string())?.is_empty())
    }

    pub(crate) fn workspace_for(&self, meta: Option<&SessionMeta>) -> Option<PathBuf> {
        match meta {
            Some(m) if m.workspace_detached => None,
            _ => meta.and_then(|m| m.workspace_root.as_ref()).map(PathBuf::from)
                .or_else(|| self.cfg.workspace_root.clone()),
        }
    }

    pub fn research_env_for_session(&self, session_id: &str) -> Result<ResearchEnv, String> {
        let ws = self.workspace_for(self.load_meta(session_id).as_ref());
        let env = match ws {
            Some(root) => ResearchEnv::with_workspace(self.cfg.data_dir.clone(), root)
                .map_err(|e| e.to_string())?,
            None => ResearchEnv::new(self.cfg.data_dir.clone()),
        };
        Ok(env.with_endpoints(self.cfg.literature_endpoints.clone()))
    }

    pub fn pending_for_session(&self, session_id: &str) -> Vec<PendingApprovalView> {
        let Ok(map) = self.pending_approvals.lock() else {
            return Vec::new();
        };
        let mut list: Vec<PendingApprovalView> = map
            .iter()
            .filter(|((sid, _), _)| sid == session_id)
            .map(|(_, slot)| slot.view.clone())
            .collect();
        list.sort_by_key(|v| v.created_ms);
        list
    }

    pub fn resolve_pending(
        &self,
        session_id: &str,
        call_id: &str,
        digest: &str,
        approved: bool,
        approve_for_session: bool,
    ) -> Result<(), (u16, String)> {
        let slot = {
            let map = self
                .pending_approvals
                .lock()
                .map_err(|_| (500, "pending approval map poisoned".to_string()))?;
            map.get(&(session_id.to_string(), call_id.to_string())).cloned()
        }
        .ok_or_else(|| (404, format!("no pending approval `{call_id}` for `{session_id}`")))?;

        if slot.view.digest != digest {
            return Err((400, "approval digest mismatch".to_string()));
        }
        let mut dec = slot
            .decision
            .lock()
            .map_err(|_| (500, "approval slot lock poisoned".to_string()))?;
        if dec.is_some() {
            return Err((409, "approval already resolved".to_string()));
        }
        *dec = Some(ApprovalDecision {
            digest: digest.to_string(),
            approved,
            approve_for_session: approved && approve_for_session && slot.view.allow_session_approve,
            responder: "desktop-ui".to_string(),
        });
        slot.cv.notify_all();
        Ok(())
    }

    pub fn load_ui_settings(&self) -> UiSettings {
        let path = self.cfg.data_dir.join(UI_SETTINGS_FILE);
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn write_ui_settings_locked(&self, settings: &UiSettings) -> Result<(), String> {
        fs::create_dir_all(&self.cfg.data_dir).map_err(|e| format!("create data_dir failed: {e}"))?;
        let path = self.cfg.data_dir.join(UI_SETTINGS_FILE);
        let seq = UI_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .cfg
            .data_dir
            .join(format!(".ui_settings.tmp.{}.{seq}", std::process::id()));
        let mut bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        write_private_file(&tmp, &path, &bytes)
    }

    pub fn save_ui_settings(&self, settings: &UiSettings) -> Result<(), String> {
        let _guard = self.ui_settings_lock.lock().map_err(|_| "ui_settings lock poisoned".to_string())?;
        self.write_ui_settings_locked(settings)
    }

    pub fn update_ui_settings<F>(&self, f: F) -> Result<UiSettings, String>
    where
        F: FnOnce(&mut UiSettings),
    {
        let _guard = self.ui_settings_lock.lock().map_err(|_| "ui_settings lock poisoned".to_string())?;
        let mut ui = self.load_ui_settings();
        f(&mut ui);
        self.write_ui_settings_locked(&ui)?;
        Ok(ui)
    }

    pub fn settings_json(&self) -> Result<Value, String> {
        let store = ProviderKeyStore::load_from_data_dir(&self.cfg.data_dir)?;
        let redacted = store.redacted_view();
        let ui = self.load_ui_settings();
        let has_key = |p: &str| {
            redacted
                .providers
                .get(p)
                .is_some_and(|e| e.configured)
        };
        Ok(json!({
            "has_anthropic_key": has_key("anthropic"),
            "has_openai_key": has_key("openai"),
            "has_gemini_key": has_key("gemini"),
            "has_ollama_key": has_key("ollama"),
            "providers": redacted.providers,
            "roles": redacted.roles,
            "default_provider": ui.default_provider.unwrap_or_else(|| redacted.roles.solver.provider.cli_name().to_string()),
            "default_model": ui.default_model.unwrap_or_else(|| redacted.roles.solver.model.clone()),
            "pinned_conventions": ui.pinned_conventions.unwrap_or_default(),
            "role_presets": ui.role_presets.unwrap_or_else(|| json!({})),
            "auto_approve_tools": ui.auto_approve_tools,
            "agent_models": ui.agent_models,
            "paper_readers_enabled": ui.paper_readers_enabled,
            "paper_reader_max_parallel": ui.paper_reader_max_parallel,
        }))
    }

    pub fn build_transport(
        &self,
        provider: ProviderKind,
        model: &str,
    ) -> Result<Box<dyn Transport>, String> {
        let store = ProviderKeyStore::load_from_data_dir(&self.cfg.data_dir).unwrap_or_default();
        // The base URL comes only from launch configuration or saved settings, never from a
        // request body: the stored provider key is sent to whatever host this names.
        let (base_url, _) = self.base_url_for(provider, &store);

        let live_https_stream = self.cfg.api_base_override.is_none() && base_url.starts_with("https://");
        match provider {
            ProviderKind::ChatGpt => crate::chatgpt::build_transport(self),
            ProviderKind::Anthropic => {
                let cfg = AnthropicConfig {
                    base_url,
                    stream: live_https_stream,
                    ..AnthropicConfig::default()
                };
                match AnthropicTransport::from_env(cfg.clone()) {
                    Ok(t) => Ok(Box::new(t)),
                    Err(env_err) => {
                        if let Some(raw) = store.key_for(ProviderKind::Anthropic) {
                            let secret = SecretKey::new(raw.to_string()).map_err(|e| e.to_string())?;
                            let t = AnthropicTransport::new(cfg, secret, "providers.json")
                                .map_err(|e| e.to_string())?;
                            Ok(Box::new(t))
                        } else {
                            Err(env_err.to_string())
                        }
                    }
                }
            }
            ProviderKind::OpenAi | ProviderKind::Ollama => {
                let cfg = OpenAiConfig {
                    provider,
                    base_url: base_url.clone(),
                    stream: live_https_stream,
                    ..OpenAiConfig::default()
                };
                match OpenAiTransport::from_env(cfg.clone()) {
                    Ok(t) => Ok(Box::new(t)),
                    Err(env_err) => {
                        if let Some(raw) = store.key_for(provider) {
                            let secret = SecretKey::new(raw.to_string()).map_err(|e| e.to_string())?;
                            let t = OpenAiTransport::new(cfg, Some(secret), "providers.json")
                                .map_err(|e| e.to_string())?;
                            Ok(Box::new(t))
                        } else {
                            Err(env_err.to_string())
                        }
                    }
                }
            }
            ProviderKind::Gemini => {
                let cfg = GeminiConfig {
                    base_url,
                    model: model.to_string(),
                    ..GeminiConfig::default()
                };
                match GeminiTransport::from_env(cfg.clone()) {
                    Ok(t) => Ok(Box::new(t)),
                    Err(env_err) => {
                        if let Some(raw) = store.key_for(ProviderKind::Gemini) {
                            let secret = SecretKey::new(raw.to_string()).map_err(|e| e.to_string())?;
                            let t = GeminiTransport::new(cfg, secret, "providers.json")
                                .map_err(|e| e.to_string())?;
                            Ok(Box::new(t))
                        } else {
                            Err(env_err.to_string())
                        }
                    }
                }
            }
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn write_private_file(tmp: &Path, dst: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(tmp).map_err(|e| format!("failed to open `{}`: {e}", tmp.display()))?;
    f.write_all(bytes)
        .map_err(|e| format!("failed to write `{}`: {e}", tmp.display()))?;
    let _ = f.sync_all();
    drop(f);
    fs::rename(tmp, dst).map_err(|e| format!("failed to install `{}`: {e}", dst.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dst, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub use crate::upload_codec::decode_base64;
pub(crate) use crate::upload_codec::{parse_multipart_files, UploadedFile};

pub fn update_provider_roles_from_ui(roles: &mut RoleAssignment, ui: &UiSettings) {
    if let Some(p_raw) = &ui.default_provider {
        if let Ok(pk) = ProviderKind::parse(p_raw) {
            roles.solver.provider = pk;
            if let Some(m) = &ui.default_model {
                if !m.trim().is_empty() {
                    roles.solver.model = pk.resolve_model(m);
                }
            }
        }
    }
}

