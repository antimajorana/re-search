//! Persisted Investigate-mode user settings sidecar (package V7, `investigate.md` §13 decision 1).
//!
//! Stored in `<data_dir>/investigation_settings.json` so `state.rs` (599 lines) is not grown.
//! Default `orient_auto_start` is `true` (starts Orient automatically when a paper opens and
//! a model is connected, while keeping the deterministic skeleton on screen first).

use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::DesktopState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Sidecar filename inside `DesktopConfig::data_dir` (`investigate.md` §13 decision 1).
pub const INVESTIGATION_SETTINGS_FILE: &str = "investigation_settings.json";
/// Default value for Orient auto-start when a model is connected (`investigate.md` §13 decision 1).
pub const DEFAULT_ORIENT_AUTO_START: bool = true;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn default_orient_auto_start() -> bool {
    DEFAULT_ORIENT_AUTO_START
}

/// Persisted user preferences for Investigate mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigateSettings {
    #[serde(default = "default_orient_auto_start")]
    pub orient_auto_start: bool,
}

impl Default for InvestigateSettings {
    fn default() -> Self {
        Self {
            orient_auto_start: DEFAULT_ORIENT_AUTO_START,
        }
    }
}

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(INVESTIGATION_SETTINGS_FILE)
}

/// Loads [`InvestigateSettings`] from `<data_dir>/investigation_settings.json`, returning defaults
/// when the file does not exist yet or cannot be read.
pub fn load_settings(data_dir: &Path) -> InvestigateSettings {
    let path = settings_path(data_dir);
    let Ok(bytes) = fs::read(path) else {
        return InvestigateSettings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Atomically persists [`InvestigateSettings`] to `<data_dir>/investigation_settings.json`.
pub fn save_settings(data_dir: &Path, settings: &InvestigateSettings) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|e| format!("failed to create data directory `{}`: {e}", data_dir.display()))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("failed to serialize investigate settings: {e}"))?;
    let target = settings_path(data_dir);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = data_dir.join(format!(
        "{INVESTIGATION_SETTINGS_FILE}.{}.{}.tmp",
        std::process::id(),
        seq
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("failed to create temp settings file: {e}"))?;
    if let Err(err) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to write investigate settings: {err}"));
    }
    drop(file);
    if let Err(err) = fs::rename(&tmp, &target) {
        // On Windows, `rename` fails if the destination exists; fall back to remove + rename.
        if target.exists() {
            let _ = fs::remove_file(&target);
            if fs::rename(&tmp, &target).is_ok() {
                return Ok(());
            }
        }
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to commit investigate settings: {err}"));
    }
    Ok(())
}

/// HTTP handler for `GET /api/settings/investigate`.
pub fn handle_get(state: &DesktopState) -> ApiResult {
    let settings = load_settings(&state.cfg.data_dir);
    Ok((200, json!({ "orient_auto_start": settings.orient_auto_start })))
}

/// HTTP handler for `PUT /api/settings/investigate`.
pub fn handle_put(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else {
        return Err(bad("Investigate settings body must be a JSON object."));
    };
    let mut settings = load_settings(&state.cfg.data_dir);
    if let Some(raw) = obj.get("orient_auto_start") {
        let flag = raw
            .as_bool()
            .ok_or_else(|| bad("`orient_auto_start` must be a boolean."))?;
        settings.orient_auto_start = flag;
    } else {
        return Err(bad("Provide `orient_auto_start` (boolean)."));
    }
    for key in obj.keys() {
        if key != "orient_auto_start" {
            return Err(bad(format!("Unknown Investigate setting `{key}`.")));
        }
    }
    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("settings write lock poisoned"))?;
    save_settings(&state.cfg.data_dir, &settings).map_err(internal)?;
    Ok((200, json!({ "orient_auto_start": settings.orient_auto_start })))
}

/// Convenience helper for reading `orient_auto_start` from `DesktopState`.
pub fn orient_auto_start_enabled(state: &DesktopState) -> bool {
    load_settings(&state.cfg.data_dir).orient_auto_start
}

/// Convenience JSON projection embedded in `mapview`.
pub fn settings_json(state: &DesktopState) -> Value {
    let s = load_settings(&state.cfg.data_dir);
    json!({ "orient_auto_start": s.orient_auto_start })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_true_and_roundtrips_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let initial = load_settings(dir.path());
        assert!(initial.orient_auto_start);

        save_settings(dir.path(), &InvestigateSettings { orient_auto_start: false }).unwrap();
        assert!(!load_settings(dir.path()).orient_auto_start);

        save_settings(dir.path(), &InvestigateSettings { orient_auto_start: true }).unwrap();
        assert!(load_settings(dir.path()).orient_auto_start);
    }
}
