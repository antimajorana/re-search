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
/// Default model-call cap for Orient tasks (`investigate.md` §5.3).
pub const DEFAULT_ORIENT_CALL_CAP: u32 = 8;
/// Minimum allowed Orient model-call cap.
pub const MIN_ORIENT_CALL_CAP: u32 = 4;
/// Maximum allowed Orient model-call cap.
pub const MAX_ORIENT_CALL_CAP: u32 = 24;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn default_orient_auto_start() -> bool {
    DEFAULT_ORIENT_AUTO_START
}

fn default_orient_call_cap() -> u32 {
    DEFAULT_ORIENT_CALL_CAP
}

/// Persisted user preferences for Investigate mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigateSettings {
    #[serde(default = "default_orient_auto_start")]
    pub orient_auto_start: bool,
    #[serde(default = "default_orient_call_cap")]
    pub orient_call_cap: u32,
}

impl Default for InvestigateSettings {
    fn default() -> Self {
        Self {
            orient_auto_start: DEFAULT_ORIENT_AUTO_START,
            orient_call_cap: DEFAULT_ORIENT_CALL_CAP,
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
    let mut settings: InvestigateSettings = serde_json::from_slice(&bytes).unwrap_or_default();
    settings.orient_call_cap = settings
        .orient_call_cap
        .clamp(MIN_ORIENT_CALL_CAP, MAX_ORIENT_CALL_CAP);
    settings
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

fn project_settings(settings: InvestigateSettings) -> Value {
    json!({
        "orient_auto_start": settings.orient_auto_start,
        "orient_call_cap": settings.orient_call_cap,
    })
}

/// HTTP handler for `GET /api/settings/investigate`.
pub fn handle_get(state: &DesktopState) -> ApiResult {
    Ok((200, project_settings(load_settings(&state.cfg.data_dir))))
}

/// HTTP handler for `PUT /api/settings/investigate`.
pub fn handle_put(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else {
        return Err(bad("Investigate settings body must be a JSON object."));
    };
    for key in obj.keys() {
        if key != "orient_auto_start" && key != "orient_call_cap" {
            return Err(bad(format!("Unknown Investigate setting `{key}`.")));
        }
    }
    let mut settings = load_settings(&state.cfg.data_dir);
    let mut updated = false;
    if let Some(raw) = obj.get("orient_auto_start") {
        settings.orient_auto_start = raw
            .as_bool()
            .ok_or_else(|| bad("`orient_auto_start` must be a boolean."))?;
        updated = true;
    }
    if let Some(raw) = obj.get("orient_call_cap") {
        let cap = raw
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|&n| (MIN_ORIENT_CALL_CAP..=MAX_ORIENT_CALL_CAP).contains(&n))
            .ok_or_else(|| {
                bad(format!(
                    "`orient_call_cap` must be an integer between {MIN_ORIENT_CALL_CAP} and {MAX_ORIENT_CALL_CAP}."
                ))
            })?;
        settings.orient_call_cap = cap;
        updated = true;
    }
    if !updated {
        return Err(bad(
            "Provide `orient_auto_start` (boolean) and/or `orient_call_cap` (integer 4..=24).",
        ));
    }
    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("settings write lock poisoned"))?;
    save_settings(&state.cfg.data_dir, &settings).map_err(internal)?;
    Ok((200, project_settings(settings)))
}

/// Convenience helper for reading `orient_auto_start` from `DesktopState`.
pub fn orient_auto_start_enabled(state: &DesktopState) -> bool {
    load_settings(&state.cfg.data_dir).orient_auto_start
}

/// Convenience helper for reading `orient_call_cap` from `DesktopState`.
pub fn orient_call_cap(state: &DesktopState) -> u32 {
    load_settings(&state.cfg.data_dir).orient_call_cap
}

/// Convenience JSON projection embedded in `mapview`.
pub fn settings_json(state: &DesktopState) -> Value {
    project_settings(load_settings(&state.cfg.data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthGuard;
    use crate::chatgpt::{offline_test_hooks, ChatGptHub};
    use crate::sse::SseHub;
    use crate::DesktopConfig;
    use research_tools::LiteratureEndpoints;
    use std::sync::Arc;

    fn mk_state(dir: &Path) -> DesktopState {
        let cfg = DesktopConfig {
            port: 0,
            data_dir: dir.to_path_buf(),
            workspace_root: None,
            python: None,
            worker_script: None,
            api_base_override: None,
            literature_endpoints: LiteratureEndpoints::default(),
            open_window: false,
            print_url_json: false,
            eval_timeout_s: 30,
            core_exe: None,
            claude_exe: None,
            agy_exe: None,
            connection_env: Some(Default::default()),
            chatgpt_test: Some(offline_test_hooks()),
        };
        let chatgpt = ChatGptHub::new(&cfg).unwrap();
        DesktopState::new(cfg, AuthGuard::new(0).unwrap(), Arc::new(SseHub::new()), chatgpt)
    }

    #[test]
    fn defaults_to_true_and_eight_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let initial = load_settings(dir.path());
        assert!(initial.orient_auto_start);
        assert_eq!(initial.orient_call_cap, 8);

        save_settings(
            dir.path(),
            &InvestigateSettings {
                orient_auto_start: false,
                orient_call_cap: 16,
            },
        )
        .unwrap();
        let loaded = load_settings(dir.path());
        assert!(!loaded.orient_auto_start);
        assert_eq!(loaded.orient_call_cap, 16);

        // Legacy file without `orient_call_cap` defaults to 8.
        fs::write(settings_path(dir.path()), br#"{"orient_auto_start":true}"#).unwrap();
        let legacy = load_settings(dir.path());
        assert!(legacy.orient_auto_start);
        assert_eq!(legacy.orient_call_cap, 8);
    }

    #[test]
    fn http_handlers_validate_orient_call_cap_bounds_and_types() {
        let dir = tempfile::tempdir().unwrap();
        let state = mk_state(dir.path());

        let (st, body) = handle_get(&state).unwrap();
        assert_eq!(st, 200);
        assert_eq!(body, json!({ "orient_auto_start": true, "orient_call_cap": 8 }));

        // Valid lower & upper bounds (4 and 24) and partial updates.
        let (st, body) = handle_put(&state, br#"{"orient_call_cap":4}"#).unwrap();
        assert_eq!(st, 200);
        assert_eq!(body, json!({ "orient_auto_start": true, "orient_call_cap": 4 }));
        assert_eq!(orient_call_cap(&state), 4);

        let (st, body) = handle_put(&state, br#"{"orient_call_cap":24,"orient_auto_start":false}"#).unwrap();
        assert_eq!(st, 200);
        assert_eq!(body, json!({ "orient_auto_start": false, "orient_call_cap": 24 }));
        assert_eq!(settings_json(&state), json!({ "orient_auto_start": false, "orient_call_cap": 24 }));

        // Reject out-of-range, non-integer, empty, and unknown keys with 400.
        for bad_body in [
            br#"{"orient_call_cap":3}"#.as_slice(),
            br#"{"orient_call_cap":25}"#.as_slice(),
            br#"{"orient_call_cap":"12"}"#.as_slice(),
            br#"{"orient_call_cap":8.5}"#.as_slice(),
            br#"{}"#.as_slice(),
            br#"{"orient_call_cap":12,"extra":true}"#.as_slice(),
        ] {
            assert_eq!(handle_put(&state, bad_body).unwrap_err().0, 400);
        }
    }
}
