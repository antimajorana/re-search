//! Local Tesseract binary and `tessdata` detection plus runtime SHA-256 provenance
//! for scanned PDF OCR (package F14b).
//!
//! Policy invariants:
//! - Never downloads Tesseract or `tessdata` over the network.
//! - Never hardcodes `.traineddata` SHA-256 digests; hashes are computed from local files at runtime.
//! - Surfaces [`OCR_INSTALL_PROMPT`] whenever local `tesseract` or required `.traineddata` is absent.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// Owner Decision 2026-11-26: Tesseract Apache-2.0 free default OCR text layer.
pub const DEFAULT_OCR_ENABLED: bool = true;
pub const DEFAULT_OCR_LANG: &str = "eng";
// Coordinator Clarification 2026-11-26.
pub const OCR_INSTALL_PROMPT: &str = "Install Tesseract to enable OCR";
pub const MAX_TRAINEDDATA_BYTES: u64 = 256 * 1024 * 1024;
pub const ENV_RE_SEARCH_TESSERACT: &str = "RE_SEARCH_TESSERACT";
pub const ENV_TESSDATA_PREFIX: &str = "TESSDATA_PREFIX";

const HASH_CHUNK_BYTES: usize = 64 * 1024;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_VERSION_READ_BYTES: u64 = 16 * 1024;
const MAX_VERSION_CHARS: usize = 80;
const TRAINEDDATA_SUFFIX: &str = ".traineddata";

fn default_ocr_enabled() -> bool { DEFAULT_OCR_ENABLED }
fn default_ocr_langs() -> Vec<String> { vec![DEFAULT_OCR_LANG.to_string()] }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrLocalConfig {
    #[serde(default = "default_ocr_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tesseract_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tessdata_dir: Option<PathBuf>,
    #[serde(default = "default_ocr_langs")]
    pub langs: Vec<String>,
}

impl Default for OcrLocalConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_OCR_ENABLED,
            tesseract_path: None,
            tessdata_dir: None,
            langs: default_ocr_langs(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrModelProvenance {
    pub lang: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrStatus {
    pub enabled: bool,
    pub available: bool,
    pub tesseract_path: Option<String>,
    pub tessdata_dir: Option<String>,
    pub langs: Vec<String>,
    pub missing_langs: Vec<String>,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrEnvProbe {
    pub re_search_tesseract: Option<PathBuf>,
    pub tessdata_prefix: Option<PathBuf>,
    pub path_dirs: Vec<PathBuf>,
    pub local_appdata: Option<PathBuf>,
    pub extra_exe_candidates: Vec<PathBuf>,
    pub extra_tessdata_candidates: Vec<PathBuf>,
    pub probe_version: bool,
}

impl OcrEnvProbe {
    /// Captures process environment variables and standard Windows/Unix installation paths.
    pub fn from_process_env() -> Self {
        let re_search_tesseract = non_empty_env_path(ENV_RE_SEARCH_TESSERACT);
        let tessdata_prefix = non_empty_env_path(ENV_TESSDATA_PREFIX);
        let path_dirs = std::env::var_os("PATH")
            .map(|raw| std::env::split_paths(&raw).filter(|p| !p.as_os_str().is_empty()).collect())
            .unwrap_or_default();
        let local_appdata = non_empty_env_path("LOCALAPPDATA");
        let extra_exe_candidates = standard_candidates(
            local_appdata.as_deref(),
            &["Programs", "Tesseract-OCR", "tesseract.exe"],
            &[r"C:\Program Files\Tesseract-OCR\tesseract.exe", r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe"],
            &["/usr/bin/tesseract", "/usr/local/bin/tesseract", "/opt/homebrew/bin/tesseract"],
        );
        let extra_tessdata_candidates = standard_candidates(
            local_appdata.as_deref(),
            &["Programs", "Tesseract-OCR", "tessdata"],
            &[r"C:\Program Files\Tesseract-OCR\tessdata", r"C:\Program Files (x86)\Tesseract-OCR\tessdata"],
            &[
                "/usr/share/tesseract-ocr/5/tessdata",
                "/usr/share/tesseract-ocr/4.00/tessdata",
                "/usr/share/tesseract-ocr/tessdata",
                "/usr/local/share/tessdata",
                "/opt/homebrew/share/tessdata",
            ],
        );
        Self {
            re_search_tesseract,
            tessdata_prefix,
            path_dirs,
            local_appdata,
            extra_exe_candidates,
            extra_tessdata_candidates,
            probe_version: true,
        }
    }
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    let raw = std::env::var_os(key)?;
    if raw.is_empty() || raw.to_str().is_some_and(|s| s.trim().is_empty()) {
        return None;
    }
    Some(PathBuf::from(raw))
}

fn standard_candidates(
    local_appdata: Option<&Path>,
    local_rel: &[&str],
    win: &[&str],
    unix: &[&str],
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = win.iter().copied().map(PathBuf::from).collect();
    if let Some(dir) = local_appdata {
        out.push(local_rel.iter().fold(dir.to_path_buf(), |acc, seg| acc.join(seg)));
    }
    out.extend(unix.iter().copied().map(PathBuf::from));
    out
}

/// Validates a single Tesseract language identifier: 1..=24 ASCII `[a-zA-Z0-9_-]`,
/// not starting with `-` (prevents path traversal and CLI flag injection).
pub fn validate_ocr_lang(lang: &str) -> Result<String, String> {
    if lang.is_empty() || lang.len() > 24 {
        return Err(format!("OCR language `{lang}` must be 1..=24 ASCII characters"));
    }
    if lang.starts_with('-') {
        return Err(format!("OCR language `{lang}` must not start with `-`"));
    }
    if !lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err(format!(
            "OCR language `{lang}` contains invalid characters (allowed: ASCII [a-zA-Z0-9_-])"
        ));
    }
    Ok(lang.to_string())
}

/// Validates and deduplicates requested OCR languages; defaults to `["eng"]` when empty.
pub fn effective_langs(langs: &[String]) -> Result<Vec<String>, String> {
    if langs.is_empty() {
        return Ok(vec![DEFAULT_OCR_LANG.to_string()]);
    }
    let mut out = Vec::with_capacity(langs.len());
    for raw in langs {
        let valid = validate_ocr_lang(raw)?;
        if !out.contains(&valid) {
            out.push(valid);
        }
    }
    Ok(out)
}

/// Inspects local Tesseract and `tessdata` availability using the current process environment.
pub fn ocr_status(config: &OcrLocalConfig) -> OcrStatus {
    let probe = OcrEnvProbe::from_process_env();
    ocr_status_with_probe(config, &probe)
}

/// Inspects local Tesseract and `tessdata` availability using an injectable [`OcrEnvProbe`].
pub fn ocr_status_with_probe(config: &OcrLocalConfig, probe: &OcrEnvProbe) -> OcrStatus {
    let tesseract_exe = discover_tesseract_exe(config, probe);
    let version = match (&tesseract_exe, probe.probe_version) {
        (Some(exe), true) => probe_tesseract_version(exe),
        _ => None,
    };
    let (requested, invalid_requested) = match effective_langs(&config.langs) {
        Ok(valid) => (valid, false),
        Err(_) => (config.langs.clone(), true),
    };
    let (tessdata_dir, langs) =
        discover_tessdata_dir(config, probe, tesseract_exe.as_deref(), &requested);
    let missing_langs: Vec<String> = if invalid_requested {
        requested
    } else {
        requested.into_iter().filter(|req| langs.binary_search(req).is_err()).collect()
    };
    let available = !invalid_requested
        && tesseract_exe.is_some()
        && tessdata_dir.is_some()
        && missing_langs.is_empty();
    let message = if !available {
        OCR_INSTALL_PROMPT.to_string()
    } else if !config.enabled {
        "Local Tesseract detected (OCR disabled in settings).".to_string()
    } else {
        format!("Local Tesseract ready ({} language model(s)).", langs.len())
    };
    OcrStatus {
        enabled: config.enabled,
        available,
        tesseract_path: tesseract_exe.map(|p| p.to_string_lossy().into_owned()),
        tessdata_dir: tessdata_dir.map(|p| p.to_string_lossy().into_owned()),
        langs,
        missing_langs,
        version,
        message,
    }
}

fn push_unique_path(list: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidate.as_os_str().is_empty() && !list.contains(&candidate) {
        list.push(candidate);
    }
}

fn discover_tesseract_exe(config: &OcrLocalConfig, probe: &OcrEnvProbe) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = &config.tesseract_path { push_unique_path(&mut candidates, p.clone()); }
    if let Some(p) = &probe.re_search_tesseract { push_unique_path(&mut candidates, p.clone()); }
    #[cfg(windows)]
    let path_names = ["tesseract.exe", "tesseract"];
    #[cfg(not(windows))]
    let path_names = ["tesseract", "tesseract.exe"];
    for dir in probe.path_dirs.iter().filter(|d| !d.as_os_str().is_empty()) {
        for name in path_names {
            push_unique_path(&mut candidates, dir.join(name));
        }
    }
    if let Some(local_app) = &probe.local_appdata {
        push_unique_path(&mut candidates, local_app.join("Programs").join("Tesseract-OCR").join("tesseract.exe"));
    }
    for extra in &probe.extra_exe_candidates {
        push_unique_path(&mut candidates, extra.clone());
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn discover_tessdata_dir(
    config: &OcrLocalConfig,
    probe: &OcrEnvProbe,
    tesseract_exe: Option<&Path>,
    requested_langs: &[String],
) -> (Option<PathBuf>, Vec<String>) {
    let mut candidates = Vec::new();
    if let Some(dir) = &config.tessdata_dir { push_unique_path(&mut candidates, dir.clone()); }
    if let Some(prefix) = &probe.tessdata_prefix {
        push_unique_path(&mut candidates, prefix.clone());
        push_unique_path(&mut candidates, prefix.join("tessdata"));
    }
    if let Some(exe_dir) = tesseract_exe.and_then(Path::parent) {
        push_unique_path(&mut candidates, exe_dir.join("tessdata"));
        if let Some(parent) = exe_dir.parent() {
            for rel in [
                &["share", "tessdata"][..],
                &["share", "tesseract-ocr", "5", "tessdata"][..],
                &["share", "tesseract-ocr", "4.00", "tessdata"][..],
                &["share", "tesseract-ocr", "tessdata"][..],
            ] {
                push_unique_path(&mut candidates, rel.iter().fold(parent.to_path_buf(), |a, s| a.join(s)));
            }
        }
    }
    if let Some(local_app) = &probe.local_appdata {
        push_unique_path(&mut candidates, local_app.join("Programs").join("Tesseract-OCR").join("tessdata"));
    }
    for extra in &probe.extra_tessdata_candidates {
        push_unique_path(&mut candidates, extra.clone());
    }

    let mut fallback: Option<(PathBuf, Vec<String>)> = None;
    for dir in candidates {
        let found = scan_tessdata_dir(&dir);
        if found.is_empty() {
            continue;
        }
        let has_all = !requested_langs.is_empty()
            && requested_langs.iter().all(|req| found.binary_search(req).is_ok());
        if has_all {
            return (Some(dir), found);
        }
        if fallback.is_none() {
            fallback = Some((dir, found));
        }
    }
    fallback.map_or_else(|| (None, Vec::new()), |(dir, langs)| (Some(dir), langs))
}

fn scan_tessdata_dir(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new(); };
    let mut langs = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name();
        if let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(TRAINEDDATA_SUFFIX)) {
            if let Ok(valid) = validate_ocr_lang(stem) {
                langs.push(valid);
            }
        }
    }
    langs.sort();
    langs.dedup();
    langs
}

/// Runs `exe --version` with a 3-second timeout and returns the sanitized first `tesseract ...`
/// (or first non-empty) line, capped at 80 characters.
pub fn probe_tesseract_version(exe: &Path) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(VERSION_POLL_INTERVAL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let stdout_buf = read_capped(child.stdout.take());
    let stderr_buf = read_capped(child.stderr.take());
    extract_version_line(&String::from_utf8_lossy(&stdout_buf), &String::from_utf8_lossy(&stderr_buf))
}

fn read_capped<R: Read>(pipe: Option<R>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(p) = pipe {
        let _ = p.take(MAX_VERSION_READ_BYTES).read_to_end(&mut buf);
    }
    buf
}

fn starts_with_tesseract_ascii(s: &str) -> bool {
    s.as_bytes().get(..9).is_some_and(|p| p.eq_ignore_ascii_case(b"tesseract"))
}

fn extract_version_line(stdout: &str, stderr: &str) -> Option<String> {
    let mut first_non_empty: Option<&str> = None;
    for line in stdout.lines().chain(stderr.lines()).map(str::trim).filter(|l| !l.is_empty()) {
        first_non_empty.get_or_insert(line);
        if starts_with_tesseract_ascii(line) {
            return sanitize_version_line(line);
        }
    }
    first_non_empty.and_then(sanitize_version_line)
}

fn sanitize_version_line(raw: &str) -> Option<String> {
    let mut out = String::new();
    let (mut prev_space, mut written) = (false, 0usize);
    for ch in raw.chars() {
        let mapped = if ch.is_whitespace() {
            ' '
        } else if ch.is_control() {
            continue;
        } else {
            ch
        };
        if mapped == ' ' {
            if out.is_empty() || prev_space { continue; }
            prev_space = true;
        } else {
            prev_space = false;
        }
        if written >= MAX_VERSION_CHARS { break; }
        out.push(mapped);
        written += 1;
    }
    let trimmed = out.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Formats a normalized engine identifier such as `"tesseract/5.3.4"`,
/// falling back to `"tesseract/local"` when the version string cannot be parsed.
pub fn format_ocr_engine_id(version: Option<&str>) -> String {
    let Some(raw) = version else { return "tesseract/local".to_string(); };
    let line = raw.lines().next().unwrap_or("").trim();
    let mut tokens = line.split_whitespace();
    let candidate = match tokens.next() {
        Some(first) if first.eq_ignore_ascii_case("tesseract") => tokens.next(),
        first => first,
    };
    let Some(token) = candidate else { return "tesseract/local".to_string(); };
    let cleaned = token.strip_prefix(['v', 'V']).unwrap_or(token);
    let valid = !cleaned.is_empty()
        && cleaned.len() <= 40
        && cleaned.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && cleaned.contains('.')
        && cleaned.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'));
    if valid { format!("tesseract/{cleaned}") } else { "tesseract/local".to_string() }
}

/// Computes real runtime SHA-256 provenance directly from local `<tessdata_dir>/<lang>.traineddata`
/// files on disk in 64 KiB streaming chunks.
pub fn hash_local_traineddata(
    tessdata_dir: &Path,
    langs: &[String],
) -> Result<Vec<OcrModelProvenance>, String> {
    let resolved = effective_langs(langs)?;
    let mut out = Vec::with_capacity(resolved.len());
    let mut buf = vec![0u8; HASH_CHUNK_BYTES];
    for lang in resolved {
        let path = tessdata_dir.join(format!("{lang}{TRAINEDDATA_SUFFIX}"));
        let mut file = fs::File::open(&path)
            .map_err(|e| format!("failed to open traineddata `{lang}` at `{}`: {e}", path.display()))?;
        let meta = file
            .metadata()
            .map_err(|e| format!("failed to stat traineddata `{lang}` at `{}`: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("traineddata path `{}` is not a regular file", path.display()));
        }
        if meta.len() > MAX_TRAINEDDATA_BYTES {
            return Err(format!(
                "traineddata `{lang}` ({} bytes) exceeds limit ({MAX_TRAINEDDATA_BYTES} bytes)",
                meta.len()
            ));
        }
        let (mut hasher, mut total) = (Sha256::new(), 0u64);
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("failed to read traineddata `{lang}` at `{}`: {e}", path.display()))?;
            if n == 0 { break; }
            total = total.saturating_add(n as u64);
            if total > MAX_TRAINEDDATA_BYTES {
                return Err(format!("traineddata `{lang}` exceeds limit ({MAX_TRAINEDDATA_BYTES} bytes)"));
            }
            hasher.update(&buf[..n]);
        }
        if total == 0 {
            return Err(format!("traineddata `{lang}` at `{}` is empty (0 bytes)", path.display()));
        }
        out.push(OcrModelProvenance { lang, sha256: hex_lower(&hasher.finalize()), bytes: total });
    }
    Ok(out)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
#[path = "pdf_ocr_models_tests.rs"]
mod tests;
