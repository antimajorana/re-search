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
use std::sync::{Arc, Condvar, Mutex};

const UI_SETTINGS_FILE: &str = "ui_settings.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub workspace_root: Option<String>,
    pub pinned_conventions: Option<String>,
    #[serde(default)]
    pub verification_mode: bool,
    #[serde(default)]
    pub calculation_mode: bool,
    pub model_preset: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub created_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_info: Option<ForkInfo>,
    /// `Some("claude-cli")` once the session is bound to the Claude subscription route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// Route value stored in [`SessionMeta::route`] (and accepted as `provider`) for the
/// Claude subscription route (the Claude Code CLI driven by re:Search).
pub const CLAUDE_CLI_ROUTE: &str = "claude-cli";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkInfo {
    pub parent_session: String,
    pub forked_at_seq: u64,
    pub modified_assumption: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiSettings {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub pinned_conventions: Option<String>,
    pub role_presets: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingApprovalView {
    pub call_id: String,
    pub session: String,
    pub tool_id: String,
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
    pub pending_approvals: Mutex<HashMap<(String, String), Arc<PendingSlot>>>,
    pub ledger_write_lock: Mutex<()>,
    /// Running Claude subscription (`claude-cli`) turns, by session: their stop handles.
    pub cli_turns: Mutex<HashMap<String, cli_route::StopHandle>>,
    /// Model lists fetched from providers, kept for [`crate::model_catalog::CACHE_TTL`].
    pub catalog: crate::model_catalog::Catalog,
    /// `claude --version` output once a probe succeeded (a failed probe is not kept).
    pub claude_version: std::sync::OnceLock<String>,
}

impl DesktopState {
    pub fn new(cfg: DesktopConfig, auth: AuthGuard, sse: Arc<SseHub>) -> Self {
        Self {
            cfg,
            auth,
            sse,
            active_turns: Mutex::new(HashSet::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            ledger_write_lock: Mutex::new(()),
            cli_turns: Mutex::new(HashMap::new()),
            catalog: crate::model_catalog::Catalog::default(),
            claude_version: std::sync::OnceLock::new(),
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
                        responder: responder.to_string(),
                    });
                    slot.cv.notify_all();
                    denied += 1;
                }
            }
        }
        denied
    }

    /// Stop every running `claude-cli` turn (kills each CLI process tree), then wait up to
    /// five seconds for those turns to record `session.abort` and delete their `mcp.json`
    /// (the process may exit right after this, e.g. when the app window closes).
    pub fn stop_all_cli_turns(&self) {
        let handles: Vec<cli_route::StopHandle> = match self.cli_turns.lock() {
            Ok(map) => map.values().cloned().collect(),
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

    pub fn session_exists(&self, session_id: &str) -> Result<bool, String> {
        validate_session_id(session_id).map_err(|e| e.to_string())?;
        if self.load_meta(session_id).is_some() {
            return Ok(true);
        }
        let ledger = self.open_ledger()?;
        Ok(!ledger.read_session(session_id).map_err(|e| e.to_string())?.is_empty())
    }

    pub fn research_env_for_session(&self, session_id: &str) -> Result<ResearchEnv, String> {
        let meta = self.load_meta(session_id);
        let ws = meta
            .and_then(|m| m.workspace_root)
            .map(PathBuf::from)
            .or_else(|| self.cfg.workspace_root.clone());
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

    pub fn save_ui_settings(&self, settings: &UiSettings) -> Result<(), String> {
        fs::create_dir_all(&self.cfg.data_dir).map_err(|e| format!("create data_dir failed: {e}"))?;
        let path = self.cfg.data_dir.join(UI_SETTINGS_FILE);
        let tmp = self
            .cfg
            .data_dir
            .join(format!(".ui_settings.tmp.{}", std::process::id()));
        let mut bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        write_private_file(&tmp, &path, &bytes)
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

        match provider {
            ProviderKind::Anthropic => {
                let cfg = AnthropicConfig {
                    base_url,
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

/// Decodes standard or URL-safe base64 strings (whitespace ignored, optional `=` padding).
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    let mut ended = false;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            ended = true;
            continue;
        }
        if ended {
            return Err("unexpected data after base64 padding".to_string());
        }
        let val: u32 = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("invalid base64 byte: 0x{b:02x}")),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

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

pub(crate) type UploadedFile = (String, Vec<u8>);

pub(crate) fn parse_multipart_files(
    content_type: &str,
    body: &[u8],
) -> Result<Vec<UploadedFile>, (u16, String)> {
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
        .map(|b| b.trim().trim_matches('"').to_string())
        .filter(|b| !b.is_empty())
        .ok_or_else(|| (400, "multipart/form-data missing boundary".to_string()))?;
    let sep = format!("--{boundary}").into_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = find_bytes(&body[pos..], &sep) {
        let start = pos + rel + sep.len();
        if start + 2 <= body.len() && &body[start..start + 2] == b"--" {
            break;
        }
        let part_start = if start + 2 <= body.len() && &body[start..start + 2] == b"\r\n" {
            start + 2
        } else if start < body.len() && body[start] == b'\n' {
            start + 1
        } else {
            start
        };
        let next = match find_bytes(&body[part_start..], &sep) {
            Some(n) => part_start + n,
            None => break,
        };
        let mut part = &body[part_start..next];
        if part.ends_with(b"\r\n") {
            part = &part[..part.len() - 2];
        } else if part.ends_with(b"\n") {
            part = &part[..part.len() - 1];
        }
        if let Some(hdr_end) = find_bytes(part, b"\r\n\r\n") {
            let hdrs = String::from_utf8_lossy(&part[..hdr_end]);
            let data = &part[hdr_end + 4..];
            for line in hdrs.lines() {
                if line.to_ascii_lowercase().starts_with("content-disposition:") {
                    for seg in line.split(';') {
                        if let Some(val) = seg.trim().strip_prefix("filename=") {
                            out.push((val.trim().trim_matches('"').to_string(), data.to_vec()));
                            break;
                        }
                    }
                }
            }
        }
        pos = next;
    }
    Ok(out)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
