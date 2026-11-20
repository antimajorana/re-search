//! Mode-`0600` outside-git external provider key & role configuration store (F11).
//!
//! Stores settings at `<data_dir>/providers.json` with strict `0600` Unix
//! permissions, redacts secrets in `Debug` and UI serialization (`redacted_view`),
//! and zeroizes stored API keys on drop.

use crate::presets::RoleAssignment;
use crate::transport::ProviderKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

pub const PROVIDERS_FILE: &str = "providers.json";

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

impl fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("configured", &self.api_key.as_ref().is_some_and(|k| !k.is_empty()))
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl Drop for ProviderEntry {
    fn drop(&mut self) {
        if let Some(k) = &mut self.api_key {
            k.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedProviderEntry {
    pub configured: bool,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedProviderConfigView {
    pub providers: BTreeMap<String, RedactedProviderEntry>,
    pub roles: RoleAssignment,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderKeyStore {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderEntry>,
    #[serde(default)]
    pub roles: RoleAssignment,
}

impl ProviderKeyStore {
    pub fn config_path(data_dir: &Path) -> PathBuf {
        data_dir.join(PROVIDERS_FILE)
    }

    pub fn load_from_data_dir(data_dir: &Path) -> Result<Self, String> {
        let path = Self::config_path(data_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(&path).map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| format!("failed to parse `{}`: {e}", path.display()))
    }

    pub fn save_to_data_dir(&self, data_dir: &Path) -> Result<PathBuf, String> {
        fs::create_dir_all(data_dir).map_err(|e| format!("failed to create data_dir: {e}"))?;
        let path = Self::config_path(data_dir);
        let tmp = data_dir.join(format!(".providers.tmp.{}", std::process::id()));
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');

        let write_res = (|| -> Result<(), String> {
            let mut opts = OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp).map_err(|e| format!("failed to create temp config: {e}"))?;
            f.write_all(&bytes).map_err(|e| format!("failed to write config: {e}"))?;
            f.sync_all().map_err(|e| format!("failed to sync config: {e}"))?;
            drop(f);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("failed to chmod 0600 `{}`: {e}", tmp.display()))?;
            }
            fs::rename(&tmp, &path).map_err(|e| format!("failed to install `{}`: {e}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("failed to enforce 0600 on `{}`: {e}", path.display()))?;
            }
            Ok(())
        })();

        bytes.zeroize();
        if write_res.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        write_res?;
        Ok(path)
    }

    pub fn set_key(&mut self, provider: ProviderKind, key: Option<String>) -> Result<(), String> {
        let cleaned = match key {
            Some(mut raw) => {
                let t = raw.trim().to_string();
                raw.zeroize();
                if t.is_empty() {
                    None
                } else if t.bytes().any(|b| b < 0x20 || b == 0x7f) {
                    return Err("API key contains control characters".into());
                } else {
                    Some(t)
                }
            }
            None => None,
        };
        let entry = self
            .providers
            .entry(provider.cli_name().to_string())
            .or_default();
        if let Some(old) = &mut entry.api_key {
            old.zeroize();
        }
        entry.api_key = cleaned;
        Ok(())
    }

    pub fn set_base_url(&mut self, provider: ProviderKind, url: Option<String>) -> Result<(), String> {
        let cleaned = url.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(u) = &cleaned {
            base_url_is_safe(u)?;
        }
        let entry = self
            .providers
            .entry(provider.cli_name().to_string())
            .or_default();
        entry.base_url = cleaned;
        Ok(())
    }

    pub fn key_for(&self, provider: ProviderKind) -> Option<&str> {
        self.providers
            .get(provider.cli_name())
            .and_then(|e| e.api_key.as_deref())
            .filter(|s| !s.is_empty())
    }

    pub fn redacted_view(&self) -> RedactedProviderConfigView {
        let mut providers = BTreeMap::new();
        for p in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::Ollama,
            ProviderKind::Gemini,
        ] {
            let name = p.cli_name();
            let entry = self.providers.get(name);
            providers.insert(
                name.to_string(),
                RedactedProviderEntry {
                    configured: entry.and_then(|e| e.api_key.as_deref()).is_some_and(|k| !k.is_empty()),
                    // A value that predates the check (or was edited by hand) is not echoed.
                    base_url: entry.and_then(|e| e.base_url.clone()).filter(|u| base_url_is_safe(u).is_ok()),
                    default_model: entry.and_then(|e| e.default_model.clone()),
                },
            );
        }
        RedactedProviderConfigView {
            providers,
            roles: self.roles.clone(),
        }
    }
}

/// The base URL is shown back unredacted by the settings API, so it must not be able to carry
/// a secret: an explicit `http://` or `https://` scheme, no `user:pass@` part, no query string,
/// no fragment, no backslashes, whitespace or control characters. The message never echoes it.
fn base_url_is_safe(u: &str) -> Result<(), String> {
    let rest = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"))
        .ok_or("base URL must start with `https://` or `http://`")?;
    if rest.split('/').next().unwrap_or("").is_empty() {
        return Err("base URL has no host".into());
    }
    if u.contains('@') || u.contains('?') || u.contains('#') || u.contains('\\') {
        return Err(
            "base URL must not contain credentials (`user@`), a query string, a fragment or backslashes; put the key in the API key field"
                .into(),
        );
    }
    if u.bytes().any(|b| b <= 0x20 || b == 0x7f) {
        return Err("base URL contains whitespace or control characters".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_must_not_carry_secrets_on_write_or_read() {
        let mut store = ProviderKeyStore::default();
        for bad in [
            "https://user:secret@example.org/v1",
            "https:/user:secret@example.org/v1",
            "//user:secret@example.org/v1",
            "https:\\/user:secret@example.org",
            "https://example.org/v1?key=secret",
            "https://example.org/v1#secret",
            "https://example.org/a@b",
            "example.org/v1",
            "https:///v1",
            "https://example.org/v 1",
        ] {
            let err = store.set_base_url(ProviderKind::OpenAi, Some(bad.to_string())).unwrap_err();
            assert!(!err.contains("secret"), "error must not echo the URL: {err}");
        }
        for good in ["https://example.org/v1", "http://127.0.0.1:11434/v1", "https://[::1]:8443"] {
            store.set_base_url(ProviderKind::OpenAi, Some(good.to_string())).unwrap();
        }
        store.set_base_url(ProviderKind::OpenAi, None).unwrap();

        // A bad value already on disk is not echoed by the redacted view.
        let stored: ProviderKeyStore = serde_json::from_str(
            r#"{"providers":{"openai":{"base_url":"https://user:secret@example.org/v1"}}}"#,
        )
        .unwrap();
        let view = serde_json::to_string(&stored.redacted_view()).unwrap();
        assert!(!view.contains("secret"), "{view}");
    }
}
