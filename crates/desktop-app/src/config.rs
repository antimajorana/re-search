//! Launch settings for `re-search-app`, which has no console and, when started from the
//! Start menu, no useful environment.
//!
//! Resolution order, per setting: command-line flag, then environment variable, then the
//! optional `re-search-app.json` next to the executable, then the built-in default.
//! Everything here is pure (the environment is passed in) so it is unit-tested.

use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = "re-search-app.json";

pub const USAGE: &str = "usage: re-search-app [--data-dir DIR] [--workspace DIR] [--python EXE] \
[--worker-script PY] [--port PORT] [--core-exe EXE] [--claude EXE] [--agy EXE]";

/// The flags the app understands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Flags {
    pub data_dir: Option<String>,
    pub workspace: Option<String>,
    pub python: Option<String>,
    pub worker_script: Option<String>,
    pub port: Option<u16>,
    /// `core.exe` for the Claude subscription route.
    pub core_exe: Option<String>,
    /// `claude.exe` for the Claude subscription route.
    pub claude: Option<String>,
    /// `agy.exe` for the Antigravity route.
    pub agy: Option<String>,
}

/// Keys accepted in `re-search-app.json`. Relative paths are taken relative to the
/// directory that holds the file.
#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub python: Option<String>,
    pub worker_script: Option<String>,
    pub data_dir: Option<String>,
    pub workspace: Option<String>,
    pub core_exe: Option<String>,
    pub claude: Option<String>,
    pub agy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub workspace: Option<PathBuf>,
    pub python: Option<PathBuf>,
    pub worker_script: Option<PathBuf>,
    pub port: u16,
    /// `None`: the desktop server looks beside its executable (`core.exe`).
    pub core_exe: Option<PathBuf>,
    /// `None`: the desktop server looks on `PATH` and in the npm install location.
    pub claude: Option<PathBuf>,
    /// `None`: the desktop server looks on `PATH` and in WinGet's folders.
    pub agy: Option<PathBuf>,
}

/// Parses `--name value` and `--name=value`. Unknown flags and positional arguments are errors.
pub fn parse_flags<I: IntoIterator<Item = String>>(args: I) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let Some(body) = arg.strip_prefix("--") else {
            return Err(format!("unexpected argument `{arg}`\n{USAGE}"));
        };
        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n.to_string(), v.to_string()),
            None => {
                let v = it.next().ok_or_else(|| format!("--{body} needs a value\n{USAGE}"))?;
                (body.to_string(), v)
            }
        };
        let slot = match name.as_str() {
            "data-dir" => &mut flags.data_dir,
            "workspace" => &mut flags.workspace,
            "python" => &mut flags.python,
            "worker-script" => &mut flags.worker_script,
            "core-exe" => &mut flags.core_exe,
            "claude" => &mut flags.claude,
            "agy" => &mut flags.agy,
            "port" => {
                let port = value
                    .trim()
                    .parse::<u16>()
                    .map_err(|_| format!("--port must be a TCP port (0..=65535), got `{value}`"))?;
                flags.port = Some(port);
                continue;
            }
            other => return Err(format!("unknown flag `--{other}`\n{USAGE}")),
        };
        if value.trim().is_empty() {
            return Err(format!("--{name} needs a non-empty value"));
        }
        *slot = Some(value);
    }
    Ok(flags)
}

/// Reads `re-search-app.json` from `dir`. A missing file is `Ok(None)`; an unreadable or
/// malformed file is an error (it is better to say so than to silently ignore settings).
pub fn load_file_config(dir: &Path) -> Result<Option<FileConfig>, String> {
    let path = dir.join(CONFIG_FILE_NAME);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| format!("{} is not valid: {e}", path.display()))
}

fn env_any(env: &dyn Fn(&str) -> Option<OsString>, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .find_map(|n| env(n).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

fn absolute(p: PathBuf) -> Result<PathBuf, String> {
    std::path::absolute(&p).map_err(|e| format!("cannot resolve `{}`: {e}", p.display()))
}

fn file_path(base: &Path, raw: &Option<String>) -> Option<PathBuf> {
    raw.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| base.join(s))
}

/// `python/worker.py` (or `worker.py`) next to the executable or in the directories above it,
/// as `re-search` looks for it (the development layout is `target/debug/`).
fn find_worker_script(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir
        .ancestors()
        .take(4)
        .flat_map(|d| [d.join("worker.py"), d.join("python").join("worker.py")])
        .find(|p| p.is_file())
}

/// Combines flags, environment and file settings (in that order of precedence).
pub fn resolve(
    flags: &Flags,
    env: &dyn Fn(&str) -> Option<OsString>,
    file: Option<&FileConfig>,
    exe_dir: &Path,
) -> Result<AppConfig, String> {
    let empty = FileConfig::default();
    let file = file.unwrap_or(&empty);

    let data_dir = match flags.data_dir.as_deref().map(PathBuf::from) {
        Some(d) => d,
        None => match env_any(env, &["RE_SEARCH_DATA", "RESEARCH_CORE_DATA"]) {
            Some(d) => d,
            None => match file_path(exe_dir, &file.data_dir) {
                Some(d) => d,
                None => env_any(env, &["LOCALAPPDATA"])
                    .map(|l| l.join("re-search"))
                    .ok_or("no data directory: pass --data-dir or set RE_SEARCH_DATA")?,
            },
        },
    };

    let workspace = flags
        .workspace
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| file_path(exe_dir, &file.workspace));

    let python = flags
        .python
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| env_any(env, &["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"]))
        .or_else(|| file_path(exe_dir, &file.python))
        .or_else(|| {
            let bundled = exe_dir.join("runtime").join("python.exe");
            bundled.is_file().then_some(bundled)
        });

    let worker_script = match &python {
        None => None,
        Some(_) => Some(
            flags
                .worker_script
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| env_any(env, &["RE_SEARCH_WORKER_SCRIPT", "RESEARCH_CORE_WORKER_SCRIPT"]))
                .or_else(|| file_path(exe_dir, &file.worker_script))
                .or_else(|| find_worker_script(exe_dir))
                .ok_or("python/worker.py not found next to re-search-app: pass --worker-script")?,
        ),
    };

    let exe_setting = |flag: &Option<String>, vars: &[&str], from_file: &Option<String>| {
        flag.as_deref().map(PathBuf::from).or_else(|| env_any(env, vars)).or_else(|| file_path(exe_dir, from_file))
    };
    let core_exe = exe_setting(&flags.core_exe, &["RE_SEARCH_CORE_EXE"], &file.core_exe);
    let claude = exe_setting(&flags.claude, &["RE_SEARCH_CLAUDE", "RESEARCH_CORE_CLAUDE"], &file.claude);
    let agy = exe_setting(&flags.agy, &["RE_SEARCH_AGY"], &file.agy);

    Ok(AppConfig {
        data_dir: absolute(data_dir)?,
        workspace: workspace.map(absolute).transpose()?,
        python: python.map(absolute).transpose()?,
        worker_script: worker_script.map(absolute).transpose()?,
        port: flags.port.unwrap_or(0),
        core_exe: core_exe.map(absolute).transpose()?,
        claude: claude.map(absolute).transpose()?,
        agy: agy.map(absolute).transpose()?,
    })
}

/// Where WebView2 keeps its user data folder: `%LOCALAPPDATA%\re-search\webview`.
pub fn webview_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    env_any(env, &["LOCALAPPDATA"])
        .map(|l| l.join("re-search").join("webview"))
        .ok_or_else(|| "LOCALAPPDATA is not set; cannot place the WebView2 profile".to_string())
}

/// Refuses a WebView2 folder inside the data directory's `sessions/` or inside the workspace.
/// Compares canonical paths (deepest existing ancestor resolved, so it also works before the
/// folders exist) and ignores ASCII case, as Windows paths do.
pub fn check_webview_dir(webview: &Path, cfg: &AppConfig) -> Result<(), String> {
    let wv = canon_lenient(webview);
    if is_inside(&wv, &canon_lenient(&cfg.data_dir.join("sessions"))) {
        return Err(format!("WebView2 folder `{}` lies inside the sessions directory", webview.display()));
    }
    if let Some(ws) = &cfg.workspace {
        if is_inside(&wv, &canon_lenient(ws)) {
            return Err(format!(
                "WebView2 folder `{}` lies inside the workspace `{}`; choose another workspace",
                webview.display(),
                ws.display()
            ));
        }
    }
    Ok(())
}

fn canon_lenient(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut tail = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(c) = std::fs::canonicalize(cur) {
            return tail.iter().rev().fold(c, |acc, seg| acc.join(seg));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            _ => return abs,
        }
    }
}

fn is_inside(inner: &Path, outer: &Path) -> bool {
    let key = |p: &Path| -> Vec<String> {
        let s = p.to_string_lossy().to_ascii_lowercase();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('/', "\\");
        s.split('\\').filter(|c| !c.is_empty()).map(str::to_string).collect()
    };
    let (i, o) = (key(inner), key(outer));
    i.len() >= o.len() && i[..o.len()] == o[..]
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
