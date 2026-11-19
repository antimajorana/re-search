//! One headless session on the Claude Code CLI route.
//!
//! The CLI owns the agent loop; the core only sees what the CLI prints and
//! what arrives at its MCP server. Such sessions are recorded as
//! "external loop, partial provenance".

use crate::approval;
use crate::events::{check_init, map_line, parse_cli_version, EVENT_VERSION, TESTED_CLI_VERSION};
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tools::builtin;
use tools::canon::random_hex;
use worker::sandbox::JobObjectSandbox;

pub struct RunOpts {
    pub prompt: String,
    pub model: String,
    pub data_dir: PathBuf,
    pub python: PathBuf,
    pub claude: PathBuf,
    pub core_exe: PathBuf,
    pub worker_script: PathBuf,
}

/// Exit code used when the session is aborted by the supervisor.
pub const EXIT_ABORTED: i32 = 3;

struct Recorder {
    ledger: Ledger,
    session: String,
}

impl Recorder {
    /// Append to the ledger and print the same event as one NDJSON line.
    fn emit(&self, kind: &str, data: Value, raw: Option<&str>) -> Result<(), String> {
        let mut payload = data;
        if let (Some(hash), Some(map)) = (raw, payload.as_object_mut()) {
            map.insert("raw_blob".into(), json!(hash));
        }
        let seq = self.ledger.append(&self.session, kind, EVENT_VERSION, &payload).map_err(|e| e.to_string())?;
        let line = json!({ "v": EVENT_VERSION, "session": self.session, "seq": seq, "kind": kind, "data": payload });
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").and_then(|_| out.flush()).map_err(|e| e.to_string())
    }
}

/// The exact flags of the pilot (2026-11-18, Claude Code 2.1.318): no
/// built-in tools, only our MCP config, no user/project settings, no slash
/// commands, no session files, and only `mcp__core__*` pre-allowed.
fn cli_flags(prompt: &str, model: &str, mcp_config: &str) -> Vec<String> {
    [
        "-p", prompt,
        "--model", model,
        "--output-format", "stream-json",
        "--verbose",
        "--tools", "",
        "--strict-mcp-config",
        "--mcp-config", mcp_config,
        "--setting-sources", "",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--allowedTools", "mcp__core__*",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn run(opts: &RunOpts) -> Result<i32, String> {
    let session = format!("s-{}", random_hex(8));
    let token = random_hex(32);
    let io = |e: std::io::Error| e.to_string();

    let dir = opts.data_dir.join("sessions").join(&session);
    let (work, scratch, pycache) = (dir.join("work"), dir.join("scratch"), dir.join("pycache"));
    std::fs::create_dir_all(&work).map_err(io)?;
    std::fs::create_dir_all(&scratch).map_err(io)?;
    std::fs::create_dir_all(&pycache).map_err(io)?;
    #[cfg(windows)]
    worker::appcontainer::sweep_stale_profiles(&opts.data_dir);
    let active_lock = dir.join(".active.lock");
    std::fs::write(&active_lock, std::process::id().to_string()).map_err(io)?;
    struct SessionGuard {
        #[cfg(windows)]
        session: String,
        lock_path: PathBuf,
    }
    impl Drop for SessionGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.lock_path);
            #[cfg(windows)]
            worker::appcontainer::delete_session_profile(&self.session);
        }
    }
    let _session_guard = SessionGuard {
        #[cfg(windows)]
        session: session.clone(),
        lock_path: active_lock.clone(),
    };
    let rec = Recorder { ledger: Ledger::open(&opts.data_dir).map_err(|e| e.to_string())?, session: session.clone() };

    let approval = approval::start(opts.data_dir.clone(), session.clone(), token.clone()).map_err(io)?;
    let mcp_config = dir.join("mcp.json");
    let config = json!({ "mcpServers": { "core": {
        "command": path_str(&opts.core_exe),
        "args": [],
        "env": {
            "CORE_SESSION": session,
            "CORE_SESSION_TOKEN": token,
            "CORE_DATA_DIR": path_str(&opts.data_dir),
            "CORE_APPROVAL_ADDR": approval.addr,
            "CORE_PYTHON": path_str(&opts.python),
            "CORE_WORKER_SCRIPT": path_str(&opts.worker_script),
            "CORE_SCRATCH": path_str(&scratch),
            "CORE_PYCACHE": path_str(&pycache),
        }
    }}});
    std::fs::write(&mcp_config, serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?).map_err(io)?;

    // Check and record the CLI version: its headless defaults are due to change.
    let ver_output = Command::new(&opts.claude)
        .arg("--version")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .output()
        .map_err(|e| format!("could not run `{} --version`: {e}", opts.claude.display()))?;
    if !ver_output.status.success() {
        let err_text = String::from_utf8_lossy(&ver_output.stderr);
        return Err(format!(
            "`{} --version` exited with {}: {}",
            opts.claude.display(),
            ver_output.status,
            err_text.trim()
        ));
    }
    let version = String::from_utf8_lossy(&ver_output.stdout).trim().to_string();
    let detected_version = parse_cli_version(&version).to_string();
    let version_matches = detected_version == TESTED_CLI_VERSION;

    let defs = builtin::defs();
    let expected: Vec<String> = defs.iter().map(|d| format!("mcp__core__{}", d.id)).collect();
    let flags = cli_flags(&opts.prompt, &opts.model, &path_str(&mcp_config));
    rec.emit(
        "session.start",
        json!({
            "route": "claude-code-cli",
            "provenance": "external loop, partial provenance",
            "cli_version": version,
            "cli_version_detected": detected_version,
            "tested_cli_version": TESTED_CLI_VERSION,
            "cli_version_matches_tested": version_matches,
            "cli_flags": flags[2..],
            "core_exe": path_str(&opts.core_exe),
            "model_requested": opts.model,
            "prompt": opts.prompt,
            "policy_version": builtin::POLICY_VERSION,
            "expected_tools": expected,
            "tool_defs": defs.iter().map(|d| json!({
                "id": d.id, "effect": d.effect.as_str(), "approval_scope": d.approval_scope.as_str(), "def_hash": d.def_hash(),
            })).collect::<Vec<_>>(),
        }),
        None,
    )?;
    eprintln!("[supervisor] session {session} ({version}); data in {}", opts.data_dir.display());
    if !version_matches {
        eprintln!(
            "[supervisor] WARNING: Claude Code CLI version is {detected_version:?} (raw: {version:?}), tested against {TESTED_CLI_VERSION}; continuing (system/init check remains active)"
        );
        rec.emit(
            "cli.version_warning",
            json!({
                "source": "version-flag",
                "detected": detected_version,
                "raw": version,
                "tested": TESTED_CLI_VERSION,
            }),
            None,
        )?;
    }

    let mut command = Command::new(&opts.claude);
    command
        .args(&flags)
        .current_dir(&work)
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    // Same job-object backend for the CLI: if the supervisor dies or
    // aborts, the CLI and everything it started die with it.
    let mut proc = JobObjectSandbox.spawn_command(command).map_err(|e| format!("could not start the CLI: {e}"))?;
    let stdout = proc.child().stdout.take().ok_or("the CLI has no stdout pipe")?;

    let mut init_ok = false;
    let mut abort: Option<String> = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(io)?;
        if line.trim().is_empty() {
            continue;
        }
        let raw = rec.ledger.put_blob(line.as_bytes()).map_err(|e| e.to_string())?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            rec.emit("cli.unparsed", json!({}), Some(&raw))?;
            continue;
        };
        let ty = value["type"].as_str().unwrap_or("");
        if ty == "system" && value["subtype"] == "init" {
            if let Some(init_ver) = value["claude_code_version"].as_str() {
                if init_ver != TESTED_CLI_VERSION && init_ver != detected_version {
                    eprintln!(
                        "[supervisor] WARNING: Claude Code CLI system/init reports version {init_ver:?}, tested against {TESTED_CLI_VERSION}; continuing"
                    );
                    rec.emit(
                        "cli.version_warning",
                        json!({
                            "source": "system-init",
                            "detected": init_ver,
                            "tested": TESTED_CLI_VERSION,
                        }),
                        Some(&raw),
                    )?;
                }
            }
            match check_init(&value, &expected) {
                Ok(()) => init_ok = true,
                Err(e) => abort = Some(e),
            }
        } else if !init_ok && matches!(ty, "assistant" | "user" | "result") {
            abort = Some(format!("`{ty}` event before a verified system/init event"));
        }
        for (kind, data) in map_line(&value) {
            progress(&kind, &data);
            rec.emit(&kind, data, Some(&raw))?;
        }
        if abort.is_some() {
            break;
        }
    }

    if let Some(reason) = abort {
        proc.kill_tree();
        let _ = std::fs::remove_file(&active_lock);
        #[cfg(windows)]
        worker::appcontainer::delete_session_profile(&session);
        eprintln!("[supervisor] ABORTED: {reason}");
        rec.emit("session.abort", json!({ "reason": reason }), None)?;
        return Ok(EXIT_ABORTED);
    }
    let status = proc.child().wait().map_err(io)?;
    proc.kill_tree(); // nothing the CLI started may outlive the session
    let _ = std::fs::remove_file(&active_lock);
    #[cfg(windows)]
    worker::appcontainer::delete_session_profile(&session);
    rec.emit("session.end", json!({ "cli_exit_code": status.code(), "init_verified": init_ok }), None)?;
    eprintln!(
        "[supervisor] session {session} ended; see it with: re-search show {session} --data-dir \"{}\"",
        opts.data_dir.display()
    );
    Ok(if init_ok { status.code().unwrap_or(1) } else { EXIT_ABORTED })
}

/// A short human-readable trace on stderr (stdout carries only NDJSON).
fn progress(kind: &str, data: &Value) {
    match kind {
        "cli.init" => eprintln!("[cli] init: model {}, tools {}", data["model"], data["tools"]),
        "model.text" => eprintln!("[model] {}", data["text"].as_str().unwrap_or("")),
        "model.tool_use" => eprintln!("[model calls] {} {}", data["name"], data["input"]),
        "model.tool_result" => eprintln!("[tool result] {}", data["text"].as_str().unwrap_or("")),
        "cli.result" => eprintln!("[cli] finished: {} turns, {} ms", data["num_turns"], data["duration_ms"]),
        _ => {}
    }
}

/// Print one session's ledger events, one per line.
pub fn show(data_dir: &std::path::Path, session: &str) -> Result<(), String> {
    let ledger = Ledger::open(data_dir).map_err(|e| e.to_string())?;
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    if events.is_empty() {
        return Err(format!("no events for session `{session}`"));
    }
    let t0 = events[0].ts_ms;
    for e in events {
        let mut text = e.payload.to_string();
        if text.chars().count() > 220 {
            text = text.chars().take(220).collect::<String>() + " ...";
        }
        println!("{:>3}  +{:>7.2}s  {:<20} {}", e.seq, (e.ts_ms - t0) as f64 / 1000.0, e.kind, text);
    }
    Ok(())
}

pub fn list(data_dir: &std::path::Path) -> Result<(), String> {
    let ledger = Ledger::open(data_dir).map_err(|e| e.to_string())?;
    for (session, count) in ledger.sessions().map_err(|e| e.to_string())? {
        println!("{session}  {count} events");
    }
    Ok(())
}
