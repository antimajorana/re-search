//! Extended Stage 6-8 CLI subcommands (`ui`, `desktop`, `fork`, `diff`, `export`)
//! and research flag helpers (`--workspace`, literature base URL overrides).

use crate::cmd::{
    env_str_any, parse_eval_timeout_s, reject_unknown_flags, resolve_worker_paths, USAGE,
};
use crate::session_lock::validate_session_id;
use desktop_server::{run_blocking, DesktopConfig};
use export::{compute_branch_diff, export_session_to_path, fork_session, ExportFormat, ForkOptions};
use ledger::Ledger;
use research_tools::LiteratureEndpoints;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) fn parse_bool_flag(flag: &str, raw: Option<String>, default: bool) -> Result<bool, String> {
    let Some(v) = raw else {
        return Ok(default);
    };
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        other => Err(format!("--{flag} expects true|false, got `{other}`")),
    }
}

pub(crate) fn parse_literature_endpoints(flags: &mut HashMap<String, String>) -> Result<LiteratureEndpoints, String> {
    let mut ep = LiteratureEndpoints::default();
    if let Some(u) = flags
        .remove("arxiv-base")
        .or_else(|| env_str_any(&["RE_SEARCH_ARXIV_BASE"]))
    {
        let clean = validate_endpoint_url("arxiv-base", &u)?;
        ep.arxiv_api_base = clean.clone();
        ep.arxiv_eprint_base = clean;
    }
    if let Some(u) = flags
        .remove("inspire-base")
        .or_else(|| env_str_any(&["RE_SEARCH_INSPIRE_BASE"]))
    {
        ep.inspire_api_base = validate_endpoint_url("inspire-base", &u)?;
    }
    if let Some(u) = flags
        .remove("crossref-base")
        .or_else(|| env_str_any(&["RE_SEARCH_CROSSREF_BASE"]))
    {
        ep.crossref_api_base = validate_endpoint_url("crossref-base", &u)?;
    }
    if let Some(u) = flags
        .remove("s2-base")
        .or_else(|| env_str_any(&["RE_SEARCH_S2_BASE", "RE_SEARCH_SEMANTIC_SCHOLAR_BASE"]))
    {
        ep.semantic_scholar_api_base = validate_endpoint_url("s2-base", &u)?;
    }
    Ok(ep)
}

fn validate_endpoint_url(flag: &str, raw: &str) -> Result<String, String> {
    let clean = raw.trim().trim_end_matches('/').to_string();
    if !clean.starts_with("https://") && !clean.starts_with("http://127.0.0.1:") {
        return Err(format!(
            "--{flag} must use https:// or http://127.0.0.1:<port>, got `{clean}`"
        ));
    }
    Ok(clean)
}

/// The flags `re-search-app` understands besides `--data-dir` (which is always passed).
const APP_FLAGS: [&str; 6] = ["workspace", "python", "worker-script", "port", "core-exe", "claude"];

/// Arguments for `re-search-app`, or `None` when a flag only the in-process server
/// understands (`--no-open`, `--print-url-json`, `--api-base`, ...) was given. No token is
/// ever part of them: the app makes its own.
pub(crate) fn desktop_app_args(flags: &HashMap<String, String>, data_dir: &Path) -> Option<Vec<OsString>> {
    if flags.keys().any(|k| !APP_FLAGS.contains(&k.as_str())) {
        return None;
    }
    let mut args: Vec<OsString> = vec!["--data-dir".into(), data_dir.as_os_str().to_owned()];
    for name in APP_FLAGS {
        // An empty value means "not set", as in `re-search ui`.
        if let Some(v) = flags.get(name).filter(|v| !v.trim().is_empty()) {
            args.push(format!("--{name}").into());
            args.push(v.into());
        }
    }
    Some(args)
}

/// `re-search desktop`: starts `re-search-app` from this executable's directory and returns
/// `Some(0)`, or `None` (fall back to the browser) when it is absent or cannot take the flags.
pub(crate) fn launch_desktop_app(flags: &HashMap<String, String>, data_dir: &Path) -> Result<Option<i32>, String> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let Some(dir) = exe.parent() else {
        return Ok(None);
    };
    let app = dir.join(format!("re-search-app{}", std::env::consts::EXE_SUFFIX));
    if !app.is_file() {
        return Ok(None);
    }
    let abs_data = std::path::absolute(data_dir).map_err(|e| e.to_string())?;
    let Some(args) = desktop_app_args(flags, &abs_data) else {
        return Ok(None);
    };
    Command::new(&app)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", app.display()))?;
    eprintln!(
        "[re-search] started {} (if re:Search was already running, its window is unchanged)",
        app.display()
    );
    Ok(Some(0))
}

pub(crate) fn ui_command(
    mut flags: HashMap<String, String>,
    data_dir: PathBuf,
) -> Result<i32, String> {
    let port: u16 = match flags.remove("port") {
        Some(raw) => raw
            .trim()
            .parse::<u16>()
            .map_err(|_| format!("--port must be a valid TCP port (0..=65535), got `{raw}`"))?,
        None => 0,
    };
    let workspace_root = flags
        .remove("workspace")
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    if let Some(ws) = &workspace_root {
        if !ws.exists() || !ws.is_dir() {
            return Err(format!("workspace `{}` does not exist or is not a directory", ws.display()));
        }
    }

    let api_base_override = flags
        .remove("api-base")
        .or_else(|| env_str_any(&["RE_SEARCH_API_BASE", "RESEARCH_CORE_API_BASE"]));
    let literature_endpoints = parse_literature_endpoints(&mut flags)?;
    let no_open = parse_bool_flag("no-open", flags.remove("no-open"), false)?;
    let print_url_json = parse_bool_flag("print-url-json", flags.remove("print-url-json"), false)?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("executable has no directory")?;
    let (python, worker_script) = resolve_worker_paths(&mut flags, exe_dir)?;
    // Claude subscription route: `--core-exe` / `--claude`, else the environment; when both
    // are unset the server looks beside its executable (core.exe) and on PATH / npm (claude).
    let exe_flag = |flags: &mut HashMap<String, String>, flag: &str, vars: &[&str]| {
        flags
            .remove(flag)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| env_str_any(vars))
            .map(|p| std::path::absolute(PathBuf::from(p)).map_err(|e| e.to_string()))
            .transpose()
    };
    let core_exe = exe_flag(&mut flags, "core-exe", &["RE_SEARCH_CORE_EXE"])?;
    let claude_exe = exe_flag(&mut flags, "claude", &["RE_SEARCH_CLAUDE", "RESEARCH_CORE_CLAUDE"])?;
    reject_unknown_flags(&flags)?;

    let abs_data = std::path::absolute(&data_dir).map_err(|e| e.to_string())?;
    let abs_ws = workspace_root
        .map(|ws| std::path::absolute(&ws))
        .transpose()
        .map_err(|e| e.to_string())?;

    run_blocking(DesktopConfig {
        port,
        data_dir: abs_data,
        workspace_root: abs_ws,
        python,
        worker_script,
        api_base_override,
        literature_endpoints,
        open_window: !no_open,
        print_url_json,
        eval_timeout_s: parse_eval_timeout_s(),
        core_exe,
        claude_exe,
        connection_env: None,
    })
}

pub(crate) fn fork_command(
    positional: Vec<String>,
    mut flags: HashMap<String, String>,
    data_dir: PathBuf,
) -> Result<i32, String> {
    if positional.len() != 1 {
        return Err(format!("fork expects exactly one SESSION id\n{USAGE}"));
    }
    let parent_session = positional.into_iter().next().unwrap();
    validate_session_id(&parent_session)?;
    let at_seq: u64 = flags
        .remove("at-seq")
        .ok_or_else(|| format!("fork requires --at-seq N\n{USAGE}"))?
        .trim()
        .parse()
        .map_err(|_| "--at-seq must be a positive integer".to_string())?;
    if at_seq == 0 {
        return Err("--at-seq must be >= 1".to_string());
    }
    let modified_assumption = flags
        .remove("assumption")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    reject_unknown_flags(&flags)?;

    let abs_data = std::path::absolute(&data_dir).map_err(|e| e.to_string())?;
    let receipt = fork_session(
        &abs_data,
        &ForkOptions {
            parent_session,
            at_seq,
            modified_assumption,
        },
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?
    );
    Ok(0)
}

pub(crate) fn diff_command(
    positional: Vec<String>,
    flags: HashMap<String, String>,
    data_dir: PathBuf,
) -> Result<i32, String> {
    reject_unknown_flags(&flags)?;
    if positional.len() != 2 {
        return Err(format!("diff expects BASE_SESSION TARGET_SESSION\n{USAGE}"));
    }
    let base = &positional[0];
    let target = &positional[1];
    validate_session_id(base)?;
    validate_session_id(target)?;

    let abs_data = std::path::absolute(&data_dir).map_err(|e| e.to_string())?;
    let ledger = Ledger::open_existing(&abs_data).map_err(|e| e.to_string())?;
    let report = compute_branch_diff(&ledger, base, target)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    Ok(0)
}

pub(crate) fn export_command(
    positional: Vec<String>,
    mut flags: HashMap<String, String>,
    data_dir: PathBuf,
) -> Result<i32, String> {
    if positional.len() != 1 {
        return Err(format!("export expects exactly one SESSION id\n{USAGE}"));
    }
    let session = positional.into_iter().next().unwrap();
    validate_session_id(&session)?;
    let fmt_raw = flags.remove("format").unwrap_or_else(|| "tex".to_string());
    let format = ExportFormat::parse(&fmt_raw)?;
    let out = flags
        .remove("out")
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("export requires --out PATH\n{USAGE}"))?;
    reject_unknown_flags(&flags)?;

    let abs_data = std::path::absolute(&data_dir).map_err(|e| e.to_string())?;
    let bytes = export_session_to_path(&abs_data, &session, format, &out)?;
    eprintln!(
        "[re-search] exported session {session} ({fmt_raw}, {} bytes) -> {}",
        bytes.len(),
        out.display()
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn desktop_app_gets_only_its_five_flags_and_never_a_token() {
        let data = Path::new("D:/data");
        let args = desktop_app_args(
            &flags(&[("workspace", "W"), ("python", "P"), ("worker-script", "S"), ("port", "8123")]),
            data,
        )
        .unwrap();
        let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            ["--data-dir", "D:/data", "--workspace", "W", "--python", "P", "--worker-script", "S", "--port", "8123"]
        );
        assert!(!args.iter().any(|a| a.contains("token")));

        let only_data = desktop_app_args(&flags(&[]), data).unwrap();
        assert_eq!(only_data.len(), 2);
    }

    #[test]
    fn server_only_flags_fall_back_to_the_in_process_server() {
        let data = Path::new("D:/data");
        for extra in ["no-open", "print-url-json", "api-base", "arxiv-base", "token"] {
            assert!(desktop_app_args(&flags(&[("port", "0"), (extra, "x")]), data).is_none(), "{extra}");
        }
    }
}
