//! Process execution, model listing, and executable discovery for the Antigravity CLI (`agy`).

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Environment variables with which the CLI would use something other than the cached
/// interactive Google login: a Gemini API key (used with `modelProvider: "gemini"`), a
/// custom Gemini endpoint, or Application Default Credentials (from `agy changelog` and
/// the Settings page).
pub const AGY_ENV_AUTH_VARS: &[&str] = &["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "AGY_ADC_AUTH"];
/// Opt-in: `1`, `true` or `yes` lets a turn start although one of [`AGY_ENV_AUTH_VARS`] is set.
pub const ALLOW_ENV_AUTH_VAR: &str = "RE_SEARCH_ALLOW_AGY_ENV_AUTH";

/// `{list} {is|are}` for an error message.
fn names_verb(names: &[&str]) -> (String, &'static str) {
    (names.join(", "), if names.len() == 1 { "is" } else { "are" })
}

/// Refuse to start while a variable from [`AGY_ENV_AUTH_VARS`] is set (names only, never
/// values), unless [`ALLOW_ENV_AUTH_VAR`] is set. `Ok(names)` lists what was allowed.
pub fn check_env_auth(env: &dyn Fn(&str) -> Option<OsString>) -> Result<Vec<&'static str>, String> {
    let found = crate::env_auth::detect(AGY_ENV_AUTH_VARS, env);
    if found.is_empty() || crate::env_auth::flag_set(ALLOW_ENV_AUTH_VAR, env) {
        return Ok(found);
    }
    let (list, verb) = names_verb(&found);
    Err(format!(
        "refusing to start the Antigravity CLI: {list} {verb} set in re:Search's environment, and with it the CLI may \
         use an API key, another endpoint or Application Default Credentials instead of your Google plan login. \
         Unset it, or set {ALLOW_ENV_AUTH_VAR}=1 to proceed anyway."
    ))
}

/// Leading version token of `agy --version` (it prints e.g. `1.5.14`).
pub fn parse_cli_version(raw: &str) -> &str {
    raw.split_whitespace().next().unwrap_or("")
}

/// Longest wait for `agy --version`.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest wait for `agy models` (it asks Google's servers with the cached login).
pub const MODELS_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT: usize = 256 * 1024;

/// Run `<exe> <args>` with no console window, stdin closed, stderr discarded, stdout read on
/// a thread up to [`MAX_OUTPUT`] bytes, and the process killed after `timeout`. Sends no
/// prompt anywhere (callers pass only `--version` or `models`).
pub fn run_bounded(exe: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not run it: {e}"))?;
    let mut out = child.stdout.take().ok_or("no output pipe")?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        while buf.len() < MAX_OUTPUT {
            match out.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match other {
                    Err(e) => e.to_string(),
                    _ => format!("no answer within {} s", timeout.as_secs()),
                });
            }
        }
    };
    let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(500));
    let buf = rx.recv_timeout(left).map_err(|_| "its output did not arrive in time".to_string())?;
    if !status.success() {
        return Err(format!("exited with {status}"));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// One model of `agy models`: slug and display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgyModel {
    pub id: String,
    pub label: String,
}

/// A slug the CLI could never read as a flag or relative path segment.
pub fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id.len() <= 100
        && id.chars().any(|c| c.is_ascii_alphanumeric())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
}

/// Parse `agy models` output: one `slug<TAB>display name` per line (captured from 1.5.13 in
/// `fixtures/agy_models_1_5_13.txt`). Lines with an invalid slug are skipped; labels lose
/// control characters and are clipped; at most 200 models.
pub fn parse_models(text: &str) -> Vec<AgyModel> {
    let mut out: Vec<AgyModel> = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(2, '\t');
        let id = parts.next().unwrap_or("").trim();
        if !valid_model_id(id) || out.iter().any(|m| m.id == id) {
            continue;
        }
        let label: String =
            parts.next().unwrap_or(id).chars().filter(|c| !c.is_control()).take(100).collect::<String>().trim().to_string();
        out.push(AgyModel { id: id.to_string(), label: if label.is_empty() { id.to_string() } else { label } });
        if out.len() >= 200 {
            break;
        }
    }
    out
}

/// `agy models` (no prompt; a list call with the CLI's own login).
pub fn list_models(exe: &Path, timeout: Duration) -> Result<Vec<AgyModel>, String> {
    let text = run_bounded(exe, &["models"], timeout)?;
    let models = parse_models(&text);
    if models.is_empty() {
        Err("the CLI printed no model".to_string())
    } else {
        Ok(models)
    }
}

fn env_path(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> Option<PathBuf> {
    env(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// WinGet's package folders for the CLI (`Google.AntigravityCLI_<source id>`).
fn winget_packages(local: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(local.join("Microsoft").join("WinGet").join("Packages")) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("Google.AntigravityCLI_"))
        .map(|e| e.path().join("agy.exe"))
        .collect();
    found.sort();
    found
}

/// `explicit`, then `RE_SEARCH_AGY`, `agy.exe` on `PATH`, WinGet's link and package folders.
pub(crate) fn locate_agy(explicit: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = env_path(env, "RE_SEARCH_AGY") {
        return Ok(p);
    }
    let on_path: Vec<PathBuf> = env("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("agy.exe"))
        .collect();
    let local = env_path(env, "LOCALAPPDATA");
    let link = local.as_ref().map(|l| l.join("Microsoft").join("WinGet").join("Links").join("agy.exe"));
    let packages = local.as_deref().map(winget_packages).unwrap_or_default();
    on_path
        .into_iter()
        .chain(link)
        .chain(packages)
        .find(|p| p.is_file())
        .ok_or_else(|| "agy.exe not found: pass --agy or set RE_SEARCH_AGY".to_string())
}
