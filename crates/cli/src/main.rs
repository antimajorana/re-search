//! `re-search`: the supervisor for re:Search.
//!
//!   re-search run --prompt "..." [--model haiku] [--python EXE]
//!                 [--claude EXE] [--core-exe EXE] [--data-dir DIR]
//!                 [--worker-script PY]
//!   re-search show <session> [--data-dir DIR]
//!   re-search sessions [--data-dir DIR]
//!
//! `run` prints the session as versioned NDJSON on stdout and a readable
//! trace on stderr. Approval prompts appear on stderr and are answered on
//! the terminal.

mod approval;
mod events;
mod session;

use std::path::PathBuf;

const USAGE: &str = "usage:
  re-search run --prompt TEXT [--model haiku] [--python EXE] [--claude EXE]
                [--core-exe EXE] [--data-dir DIR] [--worker-script PY]
  re-search show SESSION [--data-dir DIR]
  re-search sessions [--data-dir DIR]

environment (RE_SEARCH_* preferred, RESEARCH_CORE_* fallback):
  RE_SEARCH_PYTHON, RE_SEARCH_CLAUDE, RE_SEARCH_DATA,
  RE_SEARCH_WORKER_SCRIPT, RE_SEARCH_CORE_EXE";

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn env_path_any(names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| env_path(name))
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

fn real_main() -> Result<i32, String> {
    let mut args = std::env::args().skip(1);
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
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let exe_dir = exe.parent().ok_or("executable has no directory")?;
            let core_exe = find_core_exe(flags.remove("core-exe"), exe_dir)?;
            std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
            let opts = session::RunOpts {
                prompt: flags.remove("prompt").ok_or(format!("run needs --prompt\n{USAGE}"))?,
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
            session::run(&opts)
        }
        "show" => {
            let session = positional.first().ok_or(format!("show needs a session id\n{USAGE}"))?;
            session::show(&data_dir, session).map(|_| 0)
        }
        "sessions" => session::list(&data_dir).map(|_| 0),
        _ => Err(USAGE.to_string()),
    }
}

fn main() {
    match real_main() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("re-search: {e}");
            std::process::exit(2);
        }
    }
}
