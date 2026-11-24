//! One headless session on the Claude Code CLI route.
//!
//! The CLI owns the agent loop; the core only sees what the CLI prints and
//! what arrives at its MCP server. Such sessions are recorded as
//! "external loop, partial provenance". The route itself (spawn, prompt on
//! stdin, handshake check, line loop, cleanup) is `cli_route`; this module
//! adds the console: NDJSON on stdout, a readable trace and the approval
//! prompt on the terminal.

use crate::approval;
use cli_route::claude::ClaudeCode;
use cli_route::core_tools::core_tool_set;
use cli_route::{run_turn, CoreLaunch, EventSink, StopHandle, TurnEnd, TurnSpec, VendorCli};
use ledger::{AppendedEvent, Ledger};
use serde_json::{json, Value};
use std::path::PathBuf;
use tools::canon::random_hex;

pub(crate) use cli_route::{Recorder, SessionGuard};

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

/// Exit code used when the model ends a turn with `stop_reason == "refusal"`.
pub const EXIT_REFUSAL: i32 = 5;

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

/// stdout: one NDJSON line per ledger event; stderr: a short readable trace.
struct ConsoleSink {
    session: String,
    data_dir: PathBuf,
}

impl EventSink for ConsoleSink {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String> {
        use std::io::Write;
        let line = cli_route::ndjson_line(&self.session, receipt, kind, data);
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").and_then(|_| out.flush()).map_err(|e| e.to_string())?;
        progress(&self.session, &self.data_dir, kind, data);
        Ok(())
    }
}

pub fn run(opts: &RunOpts) -> Result<i32, String> {
    let vendor = ClaudeCode;
    // Billing safety: environment credentials would take precedence over the subscription.
    let allowed_env = cli_route::env_auth::check(vendor.env_auth_vars(), &|n| std::env::var_os(n))?;

    let session = format!("s-{}", random_hex(8));
    let token = random_hex(32);
    let io = |e: std::io::Error| e.to_string();

    let _activity = research_tools::session_activity::start_turn(&opts.data_dir, &session).map_err(|e| e.to_string())?;
    let dir = opts.data_dir.join("sessions").join(&session);
    let (work, scratch, pycache) = (dir.join("work"), dir.join("scratch"), dir.join("pycache"));
    std::fs::create_dir_all(&work).map_err(io)?;
    std::fs::create_dir_all(&scratch).map_err(io)?;
    std::fs::create_dir_all(&pycache).map_err(io)?;
    let _session_guard = SessionGuard::create(&opts.data_dir, &dir, &session)?;
    let rec = Recorder { ledger: Ledger::open(&opts.data_dir).map_err(|e| e.to_string())?, session: session.clone() };

    let approval = approval::start(opts.data_dir.clone(), session.clone(), token.clone(), vendor.caller(&session)).map_err(io)?;
    let core = CoreLaunch {
        exe: opts.core_exe.clone(),
        env: [
            ("CORE_SESSION", session.clone()),
            ("CORE_SESSION_TOKEN", token),
            ("CORE_CALLER_ROUTE", cli_route::claude::ROUTE.to_string()),
            ("CORE_DATA_DIR", path_str(&opts.data_dir)),
            ("CORE_APPROVAL_ADDR", approval.addr.clone()),
            ("CORE_PYTHON", path_str(&opts.python)),
            ("CORE_WORKER_SCRIPT", path_str(&opts.worker_script)),
            ("CORE_SCRATCH", path_str(&scratch)),
            ("CORE_PYCACHE", path_str(&pycache)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    };
    let tools = core_tool_set(None);
    let mut start_fields = json!({ "prompt": opts.prompt });
    if !allowed_env.is_empty() {
        start_fields["env_auth_vars_allowed"] = json!(allowed_env);
    }
    let tool_config = dir.join("mcp.json");
    let spec = TurnSpec {
        session: &session,
        exe: &opts.claude,
        work_dir: &work,
        tool_config: &tool_config,
        core: &core,
        model: &opts.model,
        reasoning_effort: None,
        prompt: &opts.prompt,
        tool_defs: &tools.defs,
        policy_version: &tools.policy_version,
        start_kind: "session.start",
        start_fields,
        inherit_stderr: true,
        allow_env_auth: cli_route::env_auth::opted_in(&|n| std::env::var_os(n)),
    };
    let mut sink = ConsoleSink { session: session.clone(), data_dir: opts.data_dir.clone() };
    let report = run_turn(&vendor, &rec, &mut sink, &spec, &StopHandle::new())?;
    drop(approval);
    Ok(match report.end {
        TurnEnd::Aborted(_) | TurnEnd::Stopped => EXIT_ABORTED,
        TurnEnd::Completed { exit_code, init_verified } => {
            eprintln!(
                "[supervisor] session {session} ended; see it with: re-search show {session} --data-dir \"{}\"",
                opts.data_dir.display()
            );
            if init_verified {
                exit_code.unwrap_or(1)
            } else {
                EXIT_ABORTED
            }
        }
    })
}

/// A short human-readable trace on stderr (stdout carries only NDJSON).
fn progress(session: &str, data_dir: &std::path::Path, kind: &str, data: &Value) {
    match kind {
        "session.start" => eprintln!(
            "[supervisor] session {session} ({}); data in {}",
            data["cli_version"].as_str().unwrap_or(""),
            data_dir.display()
        ),
        "cli.version_warning" => eprintln!(
            "[supervisor] WARNING: Claude Code CLI version is {} ({}), tested against {}; continuing (system/init check remains active)",
            data["detected"], data["source"], data["tested"]
        ),
        "session.abort" => eprintln!("[supervisor] ABORTED: {}", data["reason"].as_str().unwrap_or("")),
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

/// Verify one session or the entire ledger, returning CLI exit codes:
/// 0 = valid, 1 = tamper/corrupt/bad-hash/unknown-session, 2 = missing DB or I/O, 4 = legacy unchained schema.
pub fn verify(data_dir: &std::path::Path, session: Option<&str>) -> Result<i32, String> {
    let ledger = match Ledger::open_existing(data_dir) {
        Ok(l) => l,
        Err(e @ (ledger::Error::Tamper(_) | ledger::Error::Sqlite(_) | ledger::Error::BadHash(_))) => {
            eprintln!("re-search verify: {e}");
            return Ok(1);
        }
        Err(e @ ledger::Error::LegacyUnchainedSchema) => {
            eprintln!("re-search verify: {e}");
            return Ok(4);
        }
        Err(e) => return Err(e.to_string()),
    };
    let res = match session {
        Some(s) => ledger.verify_session(s).map(|r| {
            println!(
                "verified session {}: {} events, {} blob refs, head {}",
                r.session, r.event_count, r.blob_refs_verified, r.head_hash
            );
        }),
        None => ledger.verify_all().map(|r| {
            println!(
                "verified ledger: {} sessions, {} events, {} blobs ({} unreferenced)",
                r.sessions.len(),
                r.total_events,
                r.blobs_verified,
                r.unreferenced_blobs
            );
        }),
    };
    match res {
        Ok(()) => Ok(0),
        Err(e @ ledger::Error::LegacyUnchainedSchema) => {
            eprintln!("re-search verify: {e}");
            Ok(4)
        }
        Err(
            e @ (ledger::Error::Tamper(_)
            | ledger::Error::BadHash(_)
            | ledger::Error::UnknownSession(_)
            | ledger::Error::Sqlite(_)
            | ledger::Error::Json(_)),
        ) => {
            eprintln!("re-search verify: {e}");
            Ok(1)
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger::GENESIS_HASH;

    #[test]
    fn recorder_emit_outputs_hash_chained_ndjson() {
        let dir = std::env::temp_dir().join(format!("cli-recorder-emit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ledger = Ledger::open(&dir).unwrap();
        let rec = Recorder { ledger, session: "s-emit".into() };
        let h_raw = rec.ledger.put_blob(b"raw line").unwrap();

        let mut buf = Vec::new();
        rec.emit_to(&mut buf, "session.start", json!({"ok": true}), None).unwrap();
        rec.emit_to(&mut buf, "model.text", json!({"text": "hi"}), Some(&h_raw)).unwrap();

        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["seq"], 1);
        assert_eq!(lines[0]["prev_hash"], GENESIS_HASH);
        assert!(lines[0]["ts_ms"].as_i64().is_some());
        assert_eq!(lines[1]["seq"], 2);
        assert_eq!(lines[1]["prev_hash"], lines[0]["event_hash"]);
        assert_eq!(lines[1]["data"]["raw_blob"], h_raw);

        let report = rec.ledger.verify_session("s-emit").unwrap();
        assert_eq!(report.event_count, 2);
        assert_eq!(report.blob_refs_verified, 1);
        assert_eq!(report.head_hash.as_str(), lines[1]["event_hash"].as_str().unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
