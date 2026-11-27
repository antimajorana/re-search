//! Persisted literature service settings sidecar (package X19 Part 1).
//!
//! Stored in `<data_dir>/lit_settings.json` with `0600` Unix permissions so `state.rs`
//! (598 lines) is never grown. Holds the optional free per-user OpenAlex API key (with
//! `OPENALEX_API_KEY` environment-variable fallback) and optional polite-pool opt-in.
//!
//! Security & privacy invariants (`explore.md` Rule P8, `RULES.md`):
//! - Raw API keys are NEVER returned by `GET` or `PUT` (only masked form + provenance).
//! - Raw API keys are NEVER printed in `Debug` output or error messages, NEVER placed in
//!   URLs, and NEVER written to any ledger event.
//! - Stored key bytes are zero-scrubbed on drop and after serialization.
//! - Polite-pool contact info (`openalex_polite_pool`, `openalex_mailto`) defaults off/empty
//!   unless explicitly opted in by the user.

use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::lit_providers::ProviderConfig;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Sidecar filename inside `DesktopConfig::data_dir`.
pub const LIT_SETTINGS_FILE: &str = "lit_settings.json";
/// Environment-variable fallback for the optional free OpenAlex API key.
pub const OPENALEX_API_KEY_ENV: &str = "OPENALEX_API_KEY";

const MAX_API_KEY_LEN: usize = 512;
const MAX_MAILTO_LEN: usize = 254;
const ALLOWED_PATCH_KEYS: &[&str] = &[
    "openalex_api_key",
    "openalex_polite_pool",
    "openalex_mailto",
];

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn scrub_string(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    bytes.fill(0);
}

/// Persisted optional literature-service settings (`<data_dir>/lit_settings.json`).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LitSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openalex_api_key: Option<String>,
    #[serde(default)]
    pub openalex_polite_pool: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openalex_mailto: Option<String>,
}

impl fmt::Debug for LitSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = self.openalex_api_key.as_ref().map(|_| "<redacted>");
        f.debug_struct("LitSettings")
            .field("openalex_api_key", &key)
            .field("openalex_polite_pool", &self.openalex_polite_pool)
            .field("openalex_mailto", &self.openalex_mailto)
            .finish()
    }
}

impl Drop for LitSettings {
    fn drop(&mut self) {
        if let Some(key) = &mut self.openalex_api_key {
            scrub_string(key);
        }
    }
}

/// Path to `<data_dir>/lit_settings.json`.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LIT_SETTINGS_FILE)
}

/// Mask an API key for safe display on `GET /api/settings/literature`.
///
/// Keys of at least 8 Unicode scalars show `****` plus the final 4 scalars; shorter keys
/// return `"****"` so no full or majority substring of the secret is ever echoed.
pub fn mask_api_key(raw: &str) -> String {
    let trimmed = raw.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() >= 8 {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("****{tail}")
    } else {
        "****".to_string()
    }
}

fn validate_api_key(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_API_KEY_LEN {
        return Err(format!(
            "`openalex_api_key` exceeds maximum length ({MAX_API_KEY_LEN} bytes)."
        ));
    }
    if trimmed.bytes().any(|b| b <= 0x20 || b == 0x7f) {
        return Err(
            "`openalex_api_key` must not contain whitespace or control characters.".into(),
        );
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_mailto(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_MAILTO_LEN {
        return Err(format!(
            "`openalex_mailto` exceeds maximum length ({MAX_MAILTO_LEN} bytes)."
        ));
    }
    if trimmed.bytes().any(|b| b <= 0x20 || b == 0x7f)
        || trimmed.contains(['?', '&', '#', '\\', '<', '>', '"', '\''])
    {
        return Err("`openalex_mailto` contains invalid characters.".into());
    }
    let mut parts = trimmed.split('@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    if parts.next().is_some()
        || local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return Err("`openalex_mailto` must be a valid email address.".into());
    }
    Ok(Some(trimmed.to_string()))
}

/// Load [`LitSettings`] from `<data_dir>/lit_settings.json`, distinguishing missing files
/// (returns `Ok(LitSettings::default())`) from unreadable/corrupt files.
pub fn try_load_settings(data_dir: &Path) -> Result<LitSettings, String> {
    let path = settings_path(data_dir);
    if !path.exists() {
        return Ok(LitSettings::default());
    }
    let mut bytes = fs::read(&path)
        .map_err(|e| format!("failed to read `{LIT_SETTINGS_FILE}`: {e}"))?;
    let parsed = serde_json::from_slice::<LitSettings>(&bytes)
        .map_err(|e| format!("failed to parse `{LIT_SETTINGS_FILE}`: {e}"));
    bytes.fill(0);
    let mut settings = parsed?;
    // Sanitize any hand-edited invalid values so bad headers never reach the HTTP layer.
    if let Some(raw) = settings.openalex_api_key.take() {
        let mut raw_owned = raw;
        if let Ok(Some(valid)) = validate_api_key(&raw_owned) {
            settings.openalex_api_key = Some(valid);
        }
        scrub_string(&mut raw_owned);
    }
    if let Some(raw) = settings.openalex_mailto.take() {
        if let Ok(Some(valid)) = validate_mailto(&raw) {
            settings.openalex_mailto = Some(valid);
        }
    }
    Ok(settings)
}

/// Load [`LitSettings`] from `<data_dir>/lit_settings.json`, falling back to defaults if
/// the file does not exist or cannot be parsed.
pub fn load_settings(data_dir: &Path) -> LitSettings {
    try_load_settings(data_dir).unwrap_or_default()
}

/// Atomically persist [`LitSettings`] to `<data_dir>/lit_settings.json` with `0600` Unix
/// permissions and scrubbed temporary serialization buffers.
pub fn save_settings(data_dir: &Path, settings: &LitSettings) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| {
        format!(
            "failed to create data directory `{}`: {e}",
            data_dir.display()
        )
    })?;
    let mut bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("failed to serialize literature settings: {e}"))?;
    bytes.push(b'\n');

    let target = settings_path(data_dir);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = data_dir.join(format!(
        ".{LIT_SETTINGS_FILE}.{}.{}.tmp",
        std::process::id(),
        seq
    ));

    let write_res = (|| -> Result<(), String> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts
            .open(&tmp)
            .map_err(|e| format!("failed to create temp literature settings file: {e}"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("failed to write literature settings: {e}"))?;
        drop(file);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("failed to set 0600 permissions on temp settings: {e}"))?;
        }

        if let Err(err) = fs::rename(&tmp, &target) {
            // On Windows, `rename` fails if the destination exists; fall back to remove + rename.
            if target.exists() {
                let _ = fs::remove_file(&target);
                fs::rename(&tmp, &target)
                    .map_err(|e| format!("failed to commit literature settings: {e}"))?;
            } else {
                return Err(format!("failed to commit literature settings: {err}"));
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("failed to enforce 0600 permissions on settings: {e}"))?;
        }
        Ok(())
    })();

    bytes.fill(0);
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res
}

/// Resolve the effective OpenAlex API key and its provenance (`"saved"`, `"env"`, or `"none"`).
///
/// Saved settings in `<data_dir>/lit_settings.json` take precedence over `OPENALEX_API_KEY`.
pub fn resolve_openalex_key(
    settings: &LitSettings,
    env_value: Option<&str>,
) -> (Option<String>, &'static str) {
    if let Some(saved) = settings.openalex_api_key.as_deref() {
        if let Ok(Some(valid)) = validate_api_key(saved) {
            return (Some(valid), "saved");
        }
    }
    if let Some(env_raw) = env_value {
        if let Ok(Some(valid)) = validate_api_key(env_raw) {
            return (Some(valid), "env");
        }
    }
    (None, "none")
}

fn env_openalex_key(state: &DesktopState) -> Option<String> {
    state
        .env_var(OPENALEX_API_KEY_ENV)
        .and_then(|v| v.into_string().ok())
}

/// Build an F11/X19 [`ProviderConfig`] populated with the effective OpenAlex API key
/// (saved sidecar first, then `OPENALEX_API_KEY` env fallback).
pub fn resolve_provider_config(state: &DesktopState) -> ProviderConfig {
    let settings = load_settings(&state.cfg.data_dir);
    let mut env_val = env_openalex_key(state);
    let (openalex_api_key, _) = resolve_openalex_key(&settings, env_val.as_deref());
    if let Some(s) = &mut env_val {
        scrub_string(s);
    }
    ProviderConfig {
        openalex_api_key,
        ..ProviderConfig::default()
    }
}

/// Redacted JSON projection returned by `GET` and `PUT /api/settings/literature`.
pub fn redacted_json(settings: &LitSettings, env_value: Option<&str>) -> Value {
    let (mut effective_key, source) = resolve_openalex_key(settings, env_value);
    let masked = effective_key.as_deref().map(mask_api_key);
    let configured = effective_key.is_some();
    if let Some(k) = &mut effective_key {
        scrub_string(k);
    }
    json!({
        "openalex_api_key": masked,
        "openalex_api_key_configured": configured,
        "openalex_api_key_source": source,
        "openalex_polite_pool": settings.openalex_polite_pool,
        "openalex_mailto": settings.openalex_mailto,
    })
}

/// Dispatcher for `GET` and `PUT /api/settings/literature`.
pub fn handle(state: &DesktopState, method: &str, body: &[u8]) -> ApiResult {
    match method {
        "GET" => handle_get(state),
        "PUT" => handle_put(state, body),
        _ => Err((405, "Method not allowed.".into())),
    }
}

/// HTTP handler for `GET /api/settings/literature`.
pub fn handle_get(state: &DesktopState) -> ApiResult {
    let settings = load_settings(&state.cfg.data_dir);
    let mut env_val = env_openalex_key(state);
    let view = redacted_json(&settings, env_val.as_deref());
    if let Some(s) = &mut env_val {
        scrub_string(s);
    }
    Ok((200, view))
}

/// HTTP handler for `PUT /api/settings/literature`.
pub fn handle_put(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else {
        return Err(bad("Literature settings body must be a JSON object."));
    };
    if obj.is_empty() {
        return Err(bad(
            "Provide at least one literature setting (`openalex_api_key`, `openalex_polite_pool`, or `openalex_mailto`).",
        ));
    }
    for key in obj.keys() {
        if !ALLOWED_PATCH_KEYS.contains(&key.as_str()) {
            return Err(bad(format!("Unknown literature setting `{key}`.")));
        }
    }

    let next_key: Option<Option<String>> = match obj.get("openalex_api_key") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(validate_api_key(s).map_err(bad)?),
        Some(_) => return Err(bad("`openalex_api_key` must be a string or null.")),
    };
    let next_polite: Option<bool> = match obj.get("openalex_polite_pool") {
        None => None,
        Some(v) => Some(
            v.as_bool()
                .ok_or_else(|| bad("`openalex_polite_pool` must be a boolean."))?,
        ),
    };
    let next_mailto: Option<Option<String>> = match obj.get("openalex_mailto") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(validate_mailto(s).map_err(bad)?),
        Some(_) => return Err(bad("`openalex_mailto` must be a string or null.")),
    };

    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("settings write lock poisoned"))?;
    let mut settings = load_settings(&state.cfg.data_dir);
    if let Some(k) = next_key {
        if let Some(old) = &mut settings.openalex_api_key {
            scrub_string(old);
        }
        settings.openalex_api_key = k;
    }
    if let Some(p) = next_polite {
        settings.openalex_polite_pool = p;
    }
    if let Some(m) = next_mailto {
        settings.openalex_mailto = m;
    }
    save_settings(&state.cfg.data_dir, &settings).map_err(internal)?;

    let mut env_val = env_openalex_key(state);
    let view = redacted_json(&settings, env_val.as_deref());
    if let Some(s) = &mut env_val {
        scrub_string(s);
    }
    Ok((200, view))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_mask_never_expose_raw_secret() {
        let secret = "fake-oa-secret-token-9876";
        let settings = LitSettings {
            openalex_api_key: Some(secret.into()),
            openalex_polite_pool: true,
            openalex_mailto: Some("researcher@example.org".into()),
        };
        let dbg = format!("{settings:?}");
        assert!(!dbg.contains(secret), "Debug leaked secret: {dbg}");
        assert!(dbg.contains("<redacted>"));

        assert_eq!(mask_api_key(secret), "****9876");
        assert_eq!(mask_api_key("short"), "****");
        assert!(!mask_api_key("1234567").contains("4567"));
        assert_eq!(mask_api_key("12345678"), "****5678");
    }

    #[test]
    fn roundtrip_and_env_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let initial = load_settings(dir.path());
        assert_eq!(initial, LitSettings::default());
        assert_eq!(
            resolve_openalex_key(&initial, Some("fake-env-key-1111")),
            (Some("fake-env-key-1111".into()), "env")
        );

        let saved = LitSettings {
            openalex_api_key: Some("fake-saved-key-2222".into()),
            openalex_polite_pool: true,
            openalex_mailto: Some("alice@example.org".into()),
        };
        save_settings(dir.path(), &saved).unwrap();
        let loaded = try_load_settings(dir.path()).unwrap();
        assert_eq!(loaded, saved);
        assert_eq!(
            resolve_openalex_key(&loaded, Some("fake-env-key-1111")),
            (Some("fake-saved-key-2222".into()), "saved")
        );
    }
}
