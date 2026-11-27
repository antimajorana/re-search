//! CLI argument parsing, usage text, and subcommand dispatch helpers.

#[path = "cmd_writeup.rs"]
mod cmd_writeup;

use crate::api_session;
use crate::cmd_ext;
use crate::session;
use crate::session_lock::validate_session_id;
use agent::{ProviderKind, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage:
  re-search run --prompt TEXT [--route cli|api] [--provider anthropic|openai|ollama|gemini] [--model MODEL]
                [--python EXE] [--claude EXE] [--core-exe EXE] [--data-dir DIR]
                [--worker-script PY] [--api-base URL] [--max-turns N] [--max-tokens N]
                [--workspace DIR] [--enable-research-tools true|false]
                [--verification-mode true|false] [--calculation-mode true|false]
                [--conventions TEXT] [--arxiv-base URL] [--inspire-base URL] [--crossref-base URL]
  re-search resume SESSION --prompt TEXT [--provider anthropic|openai|ollama|gemini] [--model MODEL]
                [--python EXE] [--data-dir DIR] [--worker-script PY] [--api-base URL]
                [--max-turns N] [--max-tokens N] [--workspace DIR] [--enable-research-tools true|false]
                [--arxiv-base URL] [--inspire-base URL] [--crossref-base URL]
  re-search ui [--port PORT] [--workspace DIR] [--data-dir DIR] [--python EXE]
               [--core-exe EXE] [--claude EXE] [--agy EXE]
               [--api-base URL] [--arxiv-base URL] [--inspire-base URL] [--crossref-base URL]
               [--no-open true|false] [--print-url-json true|false]
  re-search desktop [--port PORT] [--workspace DIR] [--data-dir DIR] [--python EXE] [--worker-script PY]
               [--core-exe EXE] [--claude EXE] [--agy EXE]
               (opens re-search-app.exe from this directory when present; with any other
                `ui` flag, or without re-search-app.exe, behaves like `re-search ui`)
  re-search fork SESSION --at-seq N [--assumption TEXT] [--data-dir DIR]
  re-search diff BASE_SESSION TARGET_SESSION [--data-dir DIR]
  re-search export SESSION --format tex|html|zip --out PATH [--data-dir DIR]
  re-search writeup SESSION --format tex|md|bib --out PATH [--no-inline-marks] [--data-dir DIR]
  re-search show SESSION [--data-dir DIR]
  re-search sessions [--data-dir DIR]
  re-search verify [SESSION] [--data-dir DIR]

environment (RE_SEARCH_* preferred, RESEARCH_CORE_* fallback):
  RE_SEARCH_ROUTE, RE_SEARCH_PROVIDER, RE_SEARCH_API_KEY, RE_SEARCH_API_BASE,
  RE_SEARCH_PYTHON, RE_SEARCH_CLAUDE, RE_SEARCH_AGY, RE_SEARCH_DATA, RE_SEARCH_WORKER_SCRIPT,
  RE_SEARCH_CORE_EXE, RE_SEARCH_ARXIV_BASE, RE_SEARCH_INSPIRE_BASE, RE_SEARCH_CROSSREF_BASE,
  ANTHROPIC_API_KEY, OPENAI_API_KEY, OLLAMA_API_KEY, GEMINI_API_KEY, GOOGLE_API_KEY
  RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1  start the Claude CLI route although ANTHROPIC_API_KEY,
      ANTHROPIC_AUTH_TOKEN, CLAUDE_CODE_OAUTH_TOKEN or CLAUDE_CODE_USE_{BEDROCK,VERTEX,FOUNDRY}
      is set (otherwise refused: the CLI would bill that instead of your Claude subscription)
  RE_SEARCH_ALLOW_AGY_ENV_AUTH=1  start the Antigravity CLI route although GEMINI_API_KEY,
      GOOGLE_GEMINI_BASE_URL or AGY_ADC_AUTH is set (otherwise refused: the CLI may use an API
      key, custom endpoint or ADC instead of your Google plan login)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Cli,
    Api,
}

pub(crate) fn env_str_any(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn env_path_any(names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| env_path(name))
}

fn parse_route(flag: Option<String>) -> Result<Route, String> {
    let raw = flag
        .or_else(|| env_str_any(&["RE_SEARCH_ROUTE", "RESEARCH_CORE_ROUTE"]))
        .unwrap_or_else(|| "cli".to_string());
    match raw.trim() {
        "cli" => Ok(Route::Cli),
        "api" => Ok(Route::Api),
        other => Err(format!("unknown --route `{other}` (expected `cli` or `api`)\n{USAGE}")),
    }
}

fn parse_positive_u32(flag: &str, raw: String) -> Result<u32, String> {
    let val: u32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("--{flag} must be a positive integer, got `{raw}`"))?;
    if val == 0 {
        return Err(format!("--{flag} must be at least 1"));
    }
    Ok(val)
}

pub(crate) fn parse_eval_timeout_s() -> u64 {
    env_str_any(&[
        "RE_SEARCH_EVAL_TIMEOUT_S",
        "RESEARCH_CORE_EVAL_TIMEOUT_S",
        "CORE_EVAL_TIMEOUT_S",
    ])
    .and_then(|s| s.parse::<u64>().ok())
    .filter(|&n| n >= 1)
    .unwrap_or(120)
}

fn default_data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = env_path_any(&["RE_SEARCH_DATA", "RESEARCH_CORE_DATA"]) {
        return Ok(dir);
    }
    let local = env_path("LOCALAPPDATA").ok_or_else(|| "no data directory: pass --data-dir".to_string())?;
    let dir = local.join("re-search");
    let legacy = local.join("research-core");
    if !dir.join("ledger.db").is_file() && legacy.join("ledger.db").is_file() {
        eprintln!(
            "[re-search] note: default data directory is now {} (pass --data-dir \"{}\" to inspect earlier sessions)",
            dir.display(),
            legacy.display()
        );
    }
    Ok(dir)
}

/// The real `claude.exe`. The npm `claude.cmd`/`claude.ps1` shims are not
/// used: they cannot be started directly and mangle empty arguments.
fn find_claude() -> Result<PathBuf, String> {
    if let Some(p) = env_path_any(&["RE_SEARCH_CLAUDE", "RESEARCH_CORE_CLAUDE"]) {
        return Ok(p);
    }
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("claude.exe"));
    let npm = env_path("APPDATA").map(|p| p.join("npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe"));
    on_path
        .chain(npm)
        .find(|p| p.is_file())
        .ok_or_else(|| "claude.exe not found: pass --claude or set RE_SEARCH_CLAUDE".to_string())
}

/// `python/worker.py`, looked for next to the executable and in the
/// directories above it (the development layout is target/debug/).
fn find_worker_script(exe_dir: &Path) -> Result<PathBuf, String> {
    if let Some(p) = env_path_any(&["RE_SEARCH_WORKER_SCRIPT", "RESEARCH_CORE_WORKER_SCRIPT"]) {
        return Ok(p);
    }
    exe_dir
        .ancestors()
        .take(4)
        .flat_map(|d| [d.join("worker.py"), d.join("python").join("worker.py")])
        .find(|p| p.is_file())
        .ok_or_else(|| "python/worker.py not found: pass --worker-script".to_string())
}

fn find_core_exe(flag: Option<String>, exe_dir: &Path) -> Result<PathBuf, String> {
    if let Some(p) = flag {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("{} not found", path.display()));
    }
    if let Some(p) = env_path("RE_SEARCH_CORE_EXE") {
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("{} not found (from RE_SEARCH_CORE_EXE)", p.display()));
    }
    let core_exe = exe_dir.join("core.exe");
    if !core_exe.is_file() {
        return Err(format!("{} not found (build the `mcp-server` crate)", core_exe.display()));
    }
    Ok(core_exe)
}

fn resolve_python(flag: Option<String>, exe_dir: &Path) -> Option<PathBuf> {
    flag.filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| env_path_any(&["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"]))
        .or_else(|| {
            let bundled = exe_dir.join("runtime").join("python.exe");
            (cfg!(windows) && bundled.is_file()).then_some(bundled)
        })
}

pub(crate) fn resolve_worker_paths(
    flags: &mut HashMap<String, String>,
    exe_dir: &Path,
) -> Result<(Option<PathBuf>, Option<PathBuf>), String> {
    let python = resolve_python(flags.remove("python"), exe_dir);
    let worker_script_flag = flags.remove("worker-script").map(PathBuf::from);
    let worker_script = match (&python, worker_script_flag) {
        (Some(_), Some(script)) => Some(script),
        (Some(_), None) => Some(find_worker_script(exe_dir)?),
        (None, _) => None,
    };
    Ok((python, worker_script))
}

fn resolve_provider(flag: Option<String>) -> Result<ProviderKind, String> {
    let raw = flag
        .or_else(|| env_str_any(&["RE_SEARCH_PROVIDER", "RESEARCH_CORE_PROVIDER"]))
        .unwrap_or_else(|| "anthropic".to_string());
    ProviderKind::parse(&raw)
}

fn resolve_optional_provider(flag: Option<String>) -> Result<Option<ProviderKind>, String> {
    flag.or_else(|| env_str_any(&["RE_SEARCH_PROVIDER", "RESEARCH_CORE_PROVIDER"]))
        .map(|raw| ProviderKind::parse(&raw))
        .transpose()
}

pub(crate) fn reject_unknown_flags(flags: &HashMap<String, String>) -> Result<(), String> {
    if let Some(unknown) = flags.keys().next() {
        return Err(format!("unknown flag --{unknown}\n{USAGE}"));
    }
    Ok(())
}

fn take_research_opts(flags: &mut HashMap<String, String>) -> Result<api_session::ResearchOpts, String> {
    let workspace_root = flags
        .remove("workspace")
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    let enable_research_tools =
        cmd_ext::parse_bool_flag("enable-research-tools", flags.remove("enable-research-tools"), false)?;
    let verification_mode =
        cmd_ext::parse_bool_flag("verification-mode", flags.remove("verification-mode"), false)?;
    let calculation_mode =
        cmd_ext::parse_bool_flag("calculation-mode", flags.remove("calculation-mode"), false)?;
    let pinned_conventions = flags.remove("conventions").filter(|s| !s.trim().is_empty());
    let literature_endpoints = cmd_ext::parse_literature_endpoints(flags)?;
    Ok(api_session::ResearchOpts {
        workspace_root,
        enable_research_tools,
        literature_endpoints,
        verification_mode,
        calculation_mode,
        pinned_conventions,
    })
}

fn run_command(mut flags: HashMap<String, String>, data_dir: PathBuf) -> Result<i32, String> {
    let route = parse_route(flags.remove("route"))?;
    let prompt = flags
        .remove("prompt")
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("run needs non-empty --prompt\n{USAGE}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("executable has no directory")?;

    match route {
        Route::Cli => {
            for disallowed in [
                "provider",
                "api-base",
                "max-turns",
                "max-tokens",
                "workspace",
                "enable-research-tools",
                "verification-mode",
                "calculation-mode",
                "conventions",
                "arxiv-base",
                "inspire-base",
                "crossref-base",
            ] {
                if flags.contains_key(disallowed) {
                    return Err(format!("--{disallowed} is only supported with --route api\n{USAGE}"));
                }
            }
            let model = flags.remove("model").unwrap_or_else(|| "haiku".to_string());
            let python_flag = flags.remove("python");
            let claude_flag = flags.remove("claude");
            let core_exe_flag = flags.remove("core-exe");
            let worker_script_flag = flags.remove("worker-script");
            reject_unknown_flags(&flags)?;

            let core_exe = find_core_exe(core_exe_flag, exe_dir)?;
            let opts = session::RunOpts {
                prompt,
                model,
                // `absolute`, not `canonicalize`: no verbatim `\\?\` prefix, which Python and
                // the CLI handle badly in working directories.
                data_dir: std::path::absolute(&data_dir).map_err(|e| e.to_string())?,
                python: resolve_python(python_flag, exe_dir)
                    .ok_or("no Python: pass --python or set RE_SEARCH_PYTHON")?,
                claude: match claude_flag {
                    Some(p) => PathBuf::from(p),
                    None => find_claude()?,
                },
                core_exe,
                worker_script: match worker_script_flag {
                    Some(p) => PathBuf::from(p),
                    None => find_worker_script(exe_dir)?,
                },
            };
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            session::run(&opts)
        }
        Route::Api => {
            for disallowed in ["claude", "core-exe"] {
                if flags.contains_key(disallowed) {
                    return Err(format!("--{disallowed} cannot be used with --route api\n{USAGE}"));
                }
            }
            let provider = resolve_provider(flags.remove("provider"))?;
            let model_requested = flags
                .remove("model")
                .unwrap_or_else(|| provider.default_model().to_string());
            let api_base = flags
                .remove("api-base")
                .or_else(|| env_str_any(&["RE_SEARCH_API_BASE", "RESEARCH_CORE_API_BASE"]))
                .unwrap_or_else(|| provider.default_base_url().to_string());
            let max_turns = match flags.remove("max-turns") {
                Some(v) => parse_positive_u32("max-turns", v)?,
                None => DEFAULT_MAX_TURNS,
            };
            let max_tokens = match flags.remove("max-tokens") {
                Some(v) => parse_positive_u32("max-tokens", v)?,
                None => DEFAULT_MAX_TOKENS,
            };
            let (python, worker_script) = resolve_worker_paths(&mut flags, exe_dir)?;
            let research_opts = take_research_opts(&mut flags)?;
            reject_unknown_flags(&flags)?;

            let opts = api_session::ApiRunOpts {
                prompt,
                provider,
                model_requested,
                data_dir: std::path::absolute(&data_dir).map_err(|e| e.to_string())?,
                python,
                worker_script,
                api_base,
                max_turns,
                max_tokens,
                eval_timeout_s: parse_eval_timeout_s(),
            };
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            api_session::run_with_research(&opts, &research_opts)
        }
    }
}

fn resume_command(
    positional: Vec<String>,
    mut flags: HashMap<String, String>,
    data_dir: PathBuf,
) -> Result<i32, String> {
    if positional.is_empty() {
        return Err(format!("resume needs a session id\n{USAGE}"));
    }
    if positional.len() > 1 {
        return Err(format!("resume accepts exactly one session id\n{USAGE}"));
    }
    let session = positional.into_iter().next().unwrap();
    validate_session_id(&session)?;

    for disallowed in ["route", "claude", "core-exe"] {
        if flags.contains_key(disallowed) {
            return Err(format!("--{disallowed} cannot be used with resume\n{USAGE}"));
        }
    }
    let prompt = flags
        .remove("prompt")
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("resume needs non-empty --prompt\n{USAGE}"))?;
    let provider_override = resolve_optional_provider(flags.remove("provider"))?;
    let model_override = flags.remove("model").filter(|s| !s.trim().is_empty());
    let api_base_override = flags
        .remove("api-base")
        .or_else(|| env_str_any(&["RE_SEARCH_API_BASE", "RESEARCH_CORE_API_BASE"]));
    let max_turns = match flags.remove("max-turns") {
        Some(v) => parse_positive_u32("max-turns", v)?,
        None => DEFAULT_MAX_TURNS,
    };
    let max_tokens = match flags.remove("max-tokens") {
        Some(v) => parse_positive_u32("max-tokens", v)?,
        None => DEFAULT_MAX_TOKENS,
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("executable has no directory")?;
    let (python, worker_script) = resolve_worker_paths(&mut flags, exe_dir)?;
    let research_opts = take_research_opts(&mut flags)?;
    reject_unknown_flags(&flags)?;

    let opts = api_session::ApiResumeOpts {
        session,
        prompt,
        provider_override,
        model_override,
        data_dir: std::path::absolute(&data_dir).map_err(|e| e.to_string())?,
        python,
        worker_script,
        api_base_override,
        max_turns,
        max_tokens,
        eval_timeout_s: parse_eval_timeout_s(),
    };
    api_session::resume_with_research(&opts, &research_opts)
}

pub(crate) fn run_args<I>(mut args: I) -> Result<i32, String>
where
    I: Iterator<Item = String>,
{
    let command = args.next().ok_or(USAGE)?;
    let mut flags = HashMap::new();
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        match arg.strip_prefix("--") {
            Some("no-inline-marks") => {
                flags.insert("no-inline-marks".to_string(), "true".to_string());
            }
            Some(rest) if rest.starts_with("no-inline-marks=") => {
                let val = rest.trim_start_matches("no-inline-marks=");
                flags.insert("no-inline-marks".to_string(), val.to_string());
            }
            Some(name) => {
                let value = args.next().ok_or(format!("--{name} needs a value\n{USAGE}"))?;
                flags.insert(name.to_string(), value);
            }
            None => positional.push(arg),
        }
    }
    let data_dir = match flags.remove("data-dir") {
        Some(d) => PathBuf::from(d),
        None => default_data_dir()?,
    };
    match command.as_str() {
        "run" => {
            if !positional.is_empty() {
                return Err(format!("unexpected positional argument `{}`\n{USAGE}", positional[0]));
            }
            run_command(flags, data_dir)
        }
        "resume" => resume_command(positional, flags, data_dir),
        "ui" => {
            if !positional.is_empty() {
                return Err(format!("unexpected positional argument `{}`\n{USAGE}", positional[0]));
            }
            cmd_ext::ui_command(flags, data_dir)
        }
        "desktop" => {
            if !positional.is_empty() {
                return Err(format!("unexpected positional argument `{}`\n{USAGE}", positional[0]));
            }
            // The re:Search window (`re-search-app` next to this executable) when it exists
            // and only flags it understands were given; otherwise the default browser.
            if let Some(code) = cmd_ext::launch_desktop_app(&flags, &data_dir)? {
                return Ok(code);
            }
            cmd_ext::ui_command(flags, data_dir)
        }
        "fork" => cmd_ext::fork_command(positional, flags, data_dir),
        "diff" => cmd_ext::diff_command(positional, flags, data_dir),
        "export" => cmd_ext::export_command(positional, flags, data_dir),
        "writeup" => cmd_writeup::writeup_command(positional, flags, data_dir),
        "show" => {
            reject_unknown_flags(&flags)?;
            let session = positional.first().ok_or(format!("show needs a session id\n{USAGE}"))?;
            session::show(&data_dir, session).map(|_| 0)
        }
        "sessions" => {
            reject_unknown_flags(&flags)?;
            session::list(&data_dir).map(|_| 0)
        }
        "verify" => {
            reject_unknown_flags(&flags)?;
            if positional.len() > 1 {
                return Err(format!("verify accepts at most one session id\n{USAGE}"));
            }
            session::verify(&data_dir, positional.first().map(String::as_str))
        }
        _ => Err(USAGE.to_string()),
    }
}
