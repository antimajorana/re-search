//! Persisted local OCR settings sidecar (package F14b).
//!
//! Stored in `<data_dir>/ocr_settings.json` so `state.rs` is not grown.
//! Never downloads binaries or models and never mutates existing session receipts.

use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::pdf_ocr_models::{
    effective_langs, ocr_status_with_probe, OcrEnvProbe, OcrLocalConfig, OcrStatus,
    DEFAULT_OCR_LANG, OCR_INSTALL_PROMPT,
};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Sidecar filename inside `DesktopConfig::data_dir`.
pub const OCR_SETTINGS_FILE: &str = "ocr_settings.json";

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(OCR_SETTINGS_FILE)
}

/// Loads [`OcrLocalConfig`] from `<data_dir>/ocr_settings.json`, returning defaults
/// when the file does not exist yet or cannot be read.
pub fn load_ocr_settings(data_dir: &Path) -> OcrLocalConfig {
    let Ok(bytes) = fs::read(settings_path(data_dir)) else {
        return OcrLocalConfig::default();
    };
    let mut cfg: OcrLocalConfig = serde_json::from_slice(&bytes).unwrap_or_default();
    if cfg.langs.is_empty() {
        cfg.langs = vec![DEFAULT_OCR_LANG.to_string()];
    }
    cfg
}

/// Atomically persists [`OcrLocalConfig`] to `<data_dir>/ocr_settings.json`.
pub fn save_ocr_settings(data_dir: &Path, cfg: &OcrLocalConfig) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|e| format!("failed to create data directory `{}`: {e}", data_dir.display()))?;
    let bytes = serde_json::to_vec_pretty(cfg)
        .map_err(|e| format!("failed to serialize OCR settings: {e}"))?;
    let target = settings_path(data_dir);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = data_dir.join(format!("{OCR_SETTINGS_FILE}.{}.{seq}.tmp", std::process::id()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)
        .map_err(|e| format!("failed to create temp OCR settings file: {e}"))?;
    if let Err(err) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to write OCR settings: {err}"));
    }
    drop(file);
    if let Err(err) = fs::rename(&tmp, &target) {
        // On Windows, `rename` fails if the destination exists; fall back to remove + rename.
        if target.exists() {
            let _ = fs::remove_file(&target);
            if fs::rename(&tmp, &target).is_ok() { return Ok(()); }
        }
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to commit OCR settings: {err}"));
    }
    Ok(())
}

fn scan_explicit_tessdata(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new(); };
    let mut langs = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_file() { continue; }
        let name = entry.file_name();
        if let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(".traineddata")) {
            if let Ok(valid) = research_tools::pdf_ocr_models::validate_ocr_lang(stem) {
                langs.push(valid);
            }
        }
    }
    langs.sort();
    langs.dedup();
    langs
}

/// Inspects local Tesseract availability while strictly honoring explicit path overrides.
pub fn effective_ocr_status(cfg: &OcrLocalConfig) -> OcrStatus {
    let mut probe = OcrEnvProbe::from_process_env();
    if cfg.tesseract_path.is_some() {
        probe.re_search_tesseract = None;
        probe.path_dirs.clear();
        probe.extra_exe_candidates.clear();
    }
    if cfg.tessdata_dir.is_some() {
        probe.tessdata_prefix = None;
        probe.extra_tessdata_candidates.clear();
    }
    let mut status = ocr_status_with_probe(cfg, &probe);
    if let Some(exe) = &cfg.tesseract_path {
        if !exe.is_file() {
            status.tesseract_path = None;
            status.version = None;
        }
    }
    if let Some(dir) = &cfg.tessdata_dir {
        if dir.is_dir() {
            let found = scan_explicit_tessdata(dir);
            let requested = effective_langs(&cfg.langs).unwrap_or_else(|_| cfg.langs.clone());
            status.tessdata_dir = Some(dir.to_string_lossy().into_owned());
            status.missing_langs = requested.into_iter().filter(|r| found.binary_search(r).is_err()).collect();
            status.langs = found;
        } else {
            status.tessdata_dir = None;
            status.langs.clear();
            status.missing_langs = effective_langs(&cfg.langs).unwrap_or_else(|_| cfg.langs.clone());
        }
    }
    let valid_langs = effective_langs(&cfg.langs).is_ok();
    status.available = valid_langs
        && status.tesseract_path.is_some()
        && status.tessdata_dir.is_some()
        && status.missing_langs.is_empty();
    status.message = if !status.available {
        OCR_INSTALL_PROMPT.to_string()
    } else if !cfg.enabled {
        "Local Tesseract detected (OCR disabled in settings).".to_string()
    } else {
        format!("Local Tesseract ready ({} language model(s)).", status.langs.len())
    };
    status
}

fn settings_response(cfg: &OcrLocalConfig) -> Value {
    let status = effective_ocr_status(cfg);
    json!({
        "enabled": cfg.enabled,
        "tesseract_path": cfg.tesseract_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        "tessdata_dir": cfg.tessdata_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
        "langs": cfg.langs,
        "status": status,
    })
}

fn parse_optional_path(raw: &Value, field: &str) -> Result<Option<PathBuf>, (u16, String)> {
    match raw {
        Value::Null => Ok(None),
        Value::String(s) if s.contains('\0') => {
            Err(bad(format!("`{field}` must not contain NUL bytes.")))
        }
        Value::String(s) => {
            let trimmed = s.trim();
            Ok((!trimmed.is_empty()).then(|| PathBuf::from(trimmed)))
        }
        _ => Err(bad(format!("`{field}` must be a string or null."))),
    }
}

fn parse_langs(raw: &Value) -> Result<Vec<String>, (u16, String)> {
    let items = raw.as_array().ok_or_else(|| bad("`langs` must be an array of strings."))?;
    let mut langs = Vec::with_capacity(items.len());
    for item in items {
        let s = item.as_str().ok_or_else(|| bad("`langs` entries must be strings."))?;
        langs.push(s.to_string());
    }
    effective_langs(&langs).map_err(bad)
}

/// HTTP handler for `GET /api/settings/ocr`.
pub fn handle_get_ocr_settings(state: &DesktopState) -> ApiResult {
    Ok((200, settings_response(&load_ocr_settings(&state.cfg.data_dir))))
}

/// HTTP handler for `PUT /api/settings/ocr`.
pub fn handle_put_ocr_settings(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else {
        return Err(bad("OCR settings body must be a JSON object."));
    };
    for key in obj.keys() {
        if !matches!(key.as_str(), "enabled" | "tesseract_path" | "tessdata_dir" | "langs") {
            return Err(bad(format!("Unknown OCR setting `{key}`.")));
        }
    }
    let enabled = obj
        .get("enabled")
        .map(|v| v.as_bool().ok_or_else(|| bad("`enabled` must be a boolean.")))
        .transpose()?;
    let tesseract_path = obj
        .get("tesseract_path")
        .map(|v| parse_optional_path(v, "tesseract_path"))
        .transpose()?;
    let tessdata_dir = obj
        .get("tessdata_dir")
        .map(|v| parse_optional_path(v, "tessdata_dir"))
        .transpose()?;
    let langs = obj.get("langs").map(parse_langs).transpose()?;

    let updated = {
        let _lock = state.ledger_write_lock.lock().map_err(|_| internal("settings write lock poisoned"))?;
        let mut cfg = load_ocr_settings(&state.cfg.data_dir);
        if let Some(flag) = enabled { cfg.enabled = flag; }
        if let Some(path) = tesseract_path { cfg.tesseract_path = path; }
        if let Some(dir) = tessdata_dir { cfg.tessdata_dir = dir; }
        if let Some(list) = langs { cfg.langs = list; }
        save_ocr_settings(&state.cfg.data_dir, &cfg).map_err(internal)?;
        cfg
    };
    Ok((200, settings_response(&updated)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip_persist() {
        let dir = tempfile::tempdir().unwrap();
        let initial = load_ocr_settings(dir.path());
        assert!(initial.enabled);
        assert_eq!(initial.langs, vec!["eng"]);
        assert!(initial.tesseract_path.is_none());
        let updated = OcrLocalConfig {
            enabled: false,
            tesseract_path: Some(PathBuf::from("/nonexistent/tesseract")),
            tessdata_dir: Some(PathBuf::from("/nonexistent/tessdata")),
            langs: vec!["eng".into(), "deu".into()],
        };
        save_ocr_settings(dir.path(), &updated).unwrap();
        let loaded = load_ocr_settings(dir.path());
        assert_eq!(loaded, updated);
        let st = effective_ocr_status(&loaded);
        assert!(!st.available);
        assert_eq!(st.message, OCR_INSTALL_PROMPT);
    }
}
