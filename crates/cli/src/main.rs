//! `re-search`: the supervisor for re:Search.
//!
//!   re-search run --prompt "..." [--route cli|api] [--model haiku]
//!                 [--python EXE] [--claude EXE] [--core-exe EXE]
//!                 [--data-dir DIR] [--worker-script PY]
//!                 [--api-base URL] [--max-turns N] [--max-tokens N]
//!   re-search show <session> [--data-dir DIR]
//!   re-search sessions [--data-dir DIR]
//!   re-search verify [SESSION] [--data-dir DIR]
//!
//! `run` prints the session as versioned NDJSON on stdout and a readable
//! trace on stderr. Approval prompts appear on stderr and are answered on
//! the terminal.

mod api_session;
mod approval;
mod events;
mod runtime;
mod session;

use agent::{DEFAULT_API_BASE, DEFAULT_API_MODEL, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS};
use std::path::PathBuf;

const USAGE: &str = "usage:
  re-search run --prompt TEXT [--route cli|api] [--model MODEL] [--python EXE]
                [--claude EXE] [--core-exe EXE] [--data-dir DIR]
                [--worker-script PY] [--api-base URL] [--max-turns N]
                [--max-tokens N]
  re-search show SESSION [--data-dir DIR]
  re-search sessions [--data-dir DIR]
  re-search verify [SESSION] [--data-dir DIR]

environment (RE_SEARCH_* preferred, RESEARCH_CORE_* fallback):
  RE_SEARCH_ROUTE, RE_SEARCH_PYTHON, RE_SEARCH_CLAUDE, RE_SEARCH_DATA,
  RE_SEARCH_WORKER_SCRIPT, RE_SEARCH_CORE_EXE,
  RE_SEARCH_API_KEY, RE_SEARCH_API_BASE, ANTHROPIC_API_KEY";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Cli,
    Api,
}

fn env_str_any(names: &[&str]) -> Option<String> {
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
fn find_worker_script(exe_dir: &std::path::Path) -> Result<PathBuf, String> {
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

fn find_core_exe(flag: Option<String>, exe_dir: &std::path::Path) -> Result<PathBuf, String> {
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

fn run_command(mut flags: std::collections::HashMap<String, String>, data_dir: PathBuf) -> Result<i32, String> {
    let route = parse_route(flags.remove("route"))?;
    let prompt = flags.remove("prompt").ok_or_else(|| format!("run needs --prompt\n{USAGE}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("executable has no directory")?;

    match route {
        Route::Cli => {
            for disallowed in ["api-base", "max-turns", "max-tokens"] {
                if flags.contains_key(disallowed) {
                    return Err(format!("--{disallowed} is only supported with --route api\n{USAGE}"));
                }
            }
            let core_exe = find_core_exe(flags.remove("core-exe"), exe_dir)?;
            let opts = session::RunOpts {
                prompt,
                model: flags.remove("model").unwrap_or_else(|| "haiku".to_string()),
                // `absolute`, not `canonicalize`: no verbatim `\\?\` prefix, which Python and
                // the CLI handle badly in working directories.
                data_dir: std::path::absolute(&data_dir).map_err(|e| e.to_string())?,
                python: flags
                    .remove("python")
                    .map(PathBuf::from)
                    .or_else(|| env_path_any(&["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"]))
                    .ok_or("no Python: pass --python or set RE_SEARCH_PYTHON")?,
                claude: match flags.remove("claude") {
                    Some(p) => PathBuf::from(p),
                    None => find_claude()?,
                },
                core_exe,
                worker_script: match flags.remove("worker-script") {
                    Some(p) => PathBuf::from(p),
                    None => find_worker_script(exe_dir)?,
                },
            };
            if let Some(unknown) = flags.keys().next() {
                return Err(format!("unknown flag --{unknown}\n{USAGE}"));
            }
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            session::run(&opts)
        }
        Route::Api => {
            for disallowed in ["claude", "core-exe"] {
                if flags.contains_key(disallowed) {
                    return Err(format!("--{disallowed} cannot be used with --route api\n{USAGE}"));
                }
            }
            let model_requested = flags.remove("model").unwrap_or_else(|| DEFAULT_API_MODEL.to_string());
            let api_base = flags
                .remove("api-base")
                .or_else(|| env_str_any(&["RE_SEARCH_API_BASE", "RESEARCH_CORE_API_BASE"]))
                .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
            let max_turns = match flags.remove("max-turns") {
                Some(v) => parse_positive_u32("max-turns", v)?,
                None => DEFAULT_MAX_TURNS,
            };
            let max_tokens = match flags.remove("max-tokens") {
                Some(v) => parse_positive_u32("max-tokens", v)?,
                None => DEFAULT_MAX_TOKENS,
            };
            let python = flags
                .remove("python")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .or_else(|| env_path_any(&["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"]));
            let worker_script_flag = flags.remove("worker-script").map(PathBuf::from);
            if let Some(unknown) = flags.keys().next() {
                return Err(format!("unknown flag --{unknown}\n{USAGE}"));
            }
            let worker_script = match (&python, worker_script_flag) {
                (Some(_), Some(script)) => Some(script),
                (Some(_), None) => Some(find_worker_script(exe_dir)?),
                (None, _) => None,
            };
            let eval_timeout_s = env_str_any(&[
                "RE_SEARCH_EVAL_TIMEOUT_S",
                "RESEARCH_CORE_EVAL_TIMEOUT_S",
                "CORE_EVAL_TIMEOUT_S",
            ])
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(120);
            let opts = api_session::ApiRunOpts {
                prompt,
                model_requested,
                data_dir: std::path::absolute(&data_dir).map_err(|e| e.to_string())?,
                python,
                worker_script,
                api_base,
                max_turns,
                max_tokens,
                eval_timeout_s,
            };
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            api_session::run(&opts)
        }
    }
}

fn run_args<I>(mut args: I) -> Result<i32, String>
where
    I: Iterator<Item = String>,
{
    let command = args.next().ok_or(USAGE)?;
    let mut flags = std::collections::HashMap::new();
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        match arg.strip_prefix("--") {
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
        "show" => {
            if let Some(unknown) = flags.keys().next() {
                return Err(format!("unknown flag --{unknown}\n{USAGE}"));
            }
            let session = positional.first().ok_or(format!("show needs a session id\n{USAGE}"))?;
            session::show(&data_dir, session).map(|_| 0)
        }
        "sessions" => {
            if let Some(unknown) = flags.keys().next() {
                return Err(format!("unknown flag --{unknown}\n{USAGE}"));
            }
            session::list(&data_dir).map(|_| 0)
        }
        "verify" => {
            if let Some(unknown) = flags.keys().next() {
                return Err(format!("unknown flag --{unknown}\n{USAGE}"));
            }
            if positional.len() > 1 {
                return Err(format!("verify accepts at most one session id\n{USAGE}"));
            }
            session::verify(&data_dir, positional.first().map(String::as_str))
        }
        _ => Err(USAGE.to_string()),
    }
}

fn exit_code_for_args<I>(args: I) -> i32
where
    I: Iterator<Item = String>,
{
    match run_args(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("re-search: {e}");
            2
        }
    }
}

fn main() {
    std::process::exit(exit_code_for_args(std::env::args().skip(1)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger::Ledger;
    use serde_json::json;

    fn cli_exit(args: &[&str]) -> i32 {
        exit_code_for_args(args.iter().map(|s| (*s).to_string()))
    }

    fn write_minimal_legacy_v0_db(path: &std::path::Path) {
        let mut buf = vec![0u8; 1536];
        let patches: &[(usize, &[u8])] = &[
            (0, b"SQLite format 3\x00\x02\x00\x01\x01\x00@  \x00\x00\x00\x02\x00\x00\x00\x03"),
            (43, b"\x01\x00\x00\x00\x04"),
            (59, b"\x01"),
            (95, b"\x02\x00.\x95\xc9\r\x01\xf8\x00\x02\x00\xf3\x00\x00\xf3\x01\xcd"),
            (243, b"\x81W\x01\x07\x17\x17\x17\x01\x83\rtableeventevent\x02CREATE TABLE event (session TEXT NOT NULL, seq INTEGER NOT NULL, ts_ms INTEGER NOT NULL, kind TEXT NOT NULL, schema_version INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (session, seq)))\x02\x06\x17=\x17\x01\x00indexsqlite_autoindex_event_1event\x03\x00\x00\x00\x08"),
            (512, b"\r\x00\x00\x00\x01\x01\xef\x00\x01\xef"),
            (1007, b"\x0f\x01\x07\x11\t\x08\x15\t\x11s0note{}\n\x00\x00\x00\x01\x01\xf9\x00\x01\xf9"),
            (1529, b"\x06\x04\x11\t\ts0"),
        ];
        for (off, bytes) in patches {
            buf[*off..*off + bytes.len()].copy_from_slice(bytes);
        }
        std::fs::write(path, buf).unwrap();
    }

    #[test]
    fn verify_cli_exit_codes() {
        let base = std::env::temp_dir().join(format!("cli-verify-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let missing = base.join("missing-dir");
        let missing_s = missing.to_string_lossy().into_owned();
        assert_eq!(cli_exit(&["verify", "--data-dir", &missing_s]), 2);
        assert!(!missing.exists());

        let valid = base.join("valid");
        let valid_s = valid.to_string_lossy().into_owned();
        let ledger = Ledger::open(&valid).unwrap();
        let h = ledger.put_blob(b"raw").unwrap();
        ledger.append("s1", "model.text", 1, &json!({ "text": "ok", "raw_blob": h })).unwrap();
        assert_eq!(cli_exit(&["verify", "--data-dir", &valid_s]), 0);
        assert_eq!(cli_exit(&["verify", "s1", "--data-dir", &valid_s]), 0);
        assert_eq!(cli_exit(&["verify", "unknown", "--data-dir", &valid_s]), 1);
        assert_eq!(cli_exit(&["verify", "--bogus", "1", "--data-dir", &valid_s]), 2);
        assert_eq!(cli_exit(&["verify", "s1", "s2", "--data-dir", &valid_s]), 2);

        std::fs::write(valid.join("blobs").join(&h[..2]).join(&h), b"tampered").unwrap();
        assert_eq!(cli_exit(&["verify", "--data-dir", &valid_s]), 1);
        assert_eq!(cli_exit(&["verify", "s1", "--data-dir", &valid_s]), 1);

        let legacy = base.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        write_minimal_legacy_v0_db(&legacy.join("ledger.db"));
        let legacy_s = legacy.to_string_lossy().into_owned();
        assert_eq!(cli_exit(&["verify", "--data-dir", &legacy_s]), 4);
        assert_eq!(cli_exit(&["verify", "s0", "--data-dir", &legacy_s]), 4);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn route_flags_validate_and_avoid_creating_data_dir_on_errors() {
        let base = std::env::temp_dir().join(format!("cli-route-flags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let unused = base.join("should-not-exist");
        let unused_s = unused.to_string_lossy().into_owned();

        // Unknown --route -> exit 2 and no data_dir created.
        assert_eq!(cli_exit(&["run", "--route", "bogus", "--prompt", "hi", "--data-dir", &unused_s]), 2);
        assert!(!unused.exists());

        // --route api rejects --claude / --core-exe -> exit 2 and no data_dir created.
        assert_eq!(
            cli_exit(&["run", "--route", "api", "--claude", "x", "--prompt", "hi", "--data-dir", &unused_s]),
            2
        );
        assert!(!unused.exists());
        assert_eq!(
            cli_exit(&["run", "--route", "api", "--core-exe", "x", "--prompt", "hi", "--data-dir", &unused_s]),
            2
        );
        assert!(!unused.exists());

        // --route cli rejects api-only flags -> exit 2 and no data_dir created.
        for (flag, val) in [
            ("--api-base", "http://127.0.0.1:1"),
            ("--max-turns", "4"),
            ("--max-tokens", "100"),
        ] {
            assert_eq!(
                cli_exit(&["run", "--route", "cli", flag, val, "--prompt", "hi", "--data-dir", &unused_s]),
                2,
                "flag {flag}"
            );
            assert!(!unused.exists(), "data_dir created for {flag}");
        }

        // Zero limits rejected -> exit 2.
        assert_eq!(
            cli_exit(&["run", "--route", "api", "--max-turns", "0", "--prompt", "hi", "--data-dir", &unused_s]),
            2
        );
        assert_eq!(
            cli_exit(&["run", "--route", "api", "--max-tokens", "0", "--prompt", "hi", "--data-dir", &unused_s]),
            2
        );
        assert!(!unused.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn route_api_rejects_missing_key_and_non_windows_python() {
        let base = std::env::temp_dir().join(format!("cli-route-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // Missing API key -> exit 2 (skip only if parent test env explicitly sets one).
        if env_str_any(&["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY"]).is_none() {
            let d = base.join("no-key");
            let d_s = d.to_string_lossy().into_owned();
            assert_eq!(cli_exit(&["run", "--route", "api", "--prompt", "hi", "--data-dir", &d_s]), 2);
        }

        #[cfg(not(windows))]
        {
            // Non-Windows `--route api` with `--python` fails closed with exit 2.
            let d = base.join("linux-py");
            let d_s = d.to_string_lossy().into_owned();
            let worker_script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python/worker.py");
            let script_s = worker_script.to_string_lossy().into_owned();
            assert_eq!(
                cli_exit(&[
                    "run",
                    "--route",
                    "api",
                    "--python",
                    "/usr/bin/python3",
                    "--worker-script",
                    &script_s,
                    "--prompt",
                    "hi",
                    "--data-dir",
                    &d_s,
                ]),
                2
            );
            let err = api_session::run(&api_session::ApiRunOpts {
                prompt: "hi".into(),
                model_requested: "haiku".into(),
                data_dir: d,
                python: Some(PathBuf::from("/usr/bin/python3")),
                worker_script: Some(worker_script),
                api_base: "http://127.0.0.1:1".into(),
                max_turns: 4,
                max_tokens: 100,
                eval_timeout_s: 30,
            });
            if env_str_any(&["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY"]).is_some() {
                assert_eq!(err.unwrap_err(), "AppContainer sandbox requires Windows");
            } else {
                assert!(err.is_err());
            }
        }

        let _ = std::fs::remove_dir_all(&base);
    }
}
