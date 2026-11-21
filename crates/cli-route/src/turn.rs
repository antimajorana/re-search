//! One turn on the vendor-CLI route.
//!
//! Write the tool config, check and record the CLI version, record the start event, start
//! the CLI in a kill-on-close job object with the prompt on stdin, verify the handshake
//! before the model may act, map every output line to ledger events, and end with
//! `session.end` or `session.abort`. The tool config is deleted and the process tree is
//! killed on every exit path.

use crate::recorder::{EventSink, Recorder, ToolConfigGuard};
use crate::vendor::{CoreLaunch, VendorCli};
use crate::PROVENANCE;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::ToolDef;
use worker::sandbox::JobObjectSandbox;

#[cfg(windows)]
type JobProc = worker::win::JobChild;
#[cfg(not(windows))]
type JobProc = worker::sandbox::StubJobChild;

/// Lets another thread stop a running turn: sets the flag and kills the CLI's process tree.
#[derive(Clone, Default)]
pub struct StopHandle(Arc<StopInner>);

#[derive(Default)]
struct StopInner {
    stopped: AtomicBool,
    proc: Mutex<Option<Arc<Mutex<JobProc>>>>,
}

impl StopHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request a stop; kills the CLI tree now if it is running.
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::Release);
        let proc = self.0.proc.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(p) = proc {
            p.lock().unwrap_or_else(|e| e.into_inner()).kill_tree();
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.0.stopped.load(Ordering::Acquire)
    }

    fn attach(&self, proc: Arc<Mutex<JobProc>>) {
        *self.0.proc.lock().unwrap_or_else(|e| e.into_inner()) = Some(proc);
    }

    fn detach(&self) {
        *self.0.proc.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Kills the tree and forgets it, whichever way the turn ends.
struct ProcGuard {
    proc: Arc<Mutex<JobProc>>,
    stop: StopHandle,
}

impl Drop for ProcGuard {
    fn drop(&mut self) {
        self.stop.detach();
        self.proc.lock().unwrap_or_else(|e| e.into_inner()).kill_tree();
    }
}

/// What one turn needs.
pub struct TurnSpec<'a> {
    pub session: &'a str,
    /// The vendor CLI executable.
    pub exe: &'a Path,
    /// Working directory of the CLI (an empty per-session folder).
    pub work_dir: &'a Path,
    /// Where the per-turn tool config is written; deleted when the turn ends.
    pub tool_config: &'a Path,
    pub core: &'a CoreLaunch,
    pub model: &'a str,
    /// Written to the CLI's stdin (framed by [`VendorCli::stdin_payload`]), then stdin is
    /// closed. Never put in argv.
    pub prompt: &'a str,
    /// The tools `core.exe` serves (see [`crate::core_tools::core_tool_set`]).
    pub tool_defs: &'a [ToolDef],
    pub policy_version: &'a str,
    /// `session.start` for a session's first turn, `session.resume` afterwards.
    pub start_kind: &'a str,
    /// Extra fields for the start event (for example `prompt`, history replay counts).
    pub start_fields: Value,
    /// Pass the CLI's stderr through (console) or discard it (desktop).
    pub inherit_stderr: bool,
    /// The person opted in to non-subscription credentials
    /// ([`crate::env_auth::opted_in`]); otherwise a handshake that reports one aborts.
    pub allow_env_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    /// The CLI exited on its own after the line stream ended.
    Completed { exit_code: Option<i32>, init_verified: bool },
    /// The supervisor aborted the turn (handshake mismatch, model activity before it).
    Aborted(String),
    /// Stopped through a [`StopHandle`].
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    pub end: TurnEnd,
    /// The last `model.text`, or the CLI's final result text when there was none.
    pub final_text: String,
}

fn spawn_err(e: std::io::Error) -> String {
    format!("could not start the CLI: {e}")
}

/// Run one turn. Errors are returned for failures before the CLI starts and for ledger
/// failures; everything after the start event ends in `session.end` or `session.abort`
/// unless the ledger itself fails. A stop that arrives before the start event ends the
/// turn as [`TurnEnd::Stopped`] with nothing recorded and nothing started.
pub fn run_turn(
    vendor: &dyn VendorCli,
    rec: &Recorder,
    sink: &mut dyn EventSink,
    spec: &TurnSpec<'_>,
    stop: &StopHandle,
) -> Result<TurnReport, String> {
    // Bounded (see `ClaudeCode::version`), so a stop during it takes effect right after.
    let version = vendor.version(spec.exe)?;
    if stop.is_stopped() {
        // Nothing was recorded or started yet.
        return Ok(TurnReport { end: TurnEnd::Stopped, final_text: String::new() });
    }

    let expected: Vec<String> = spec.tool_defs.iter().map(|d| vendor.tool_name(&d.id)).collect();
    let args = vendor.args(spec.model, spec.tool_config);
    let mut start = json!({
        "route": vendor.route(),
        "provenance": PROVENANCE,
        "cli_version": version.raw,
        "cli_version_detected": version.detected,
        "tested_cli_version": version.tested,
        "cli_version_matches_tested": version.matches_tested(),
        "cli_flags": args,
        "prompt_via": vendor.prompt_via(),
        "caller": vendor.caller(spec.session),
        "core_exe": spec.core.exe.to_string_lossy(),
        "model_requested": spec.model,
        "policy_version": spec.policy_version,
        "expected_tools": expected,
        "tool_defs": spec.tool_defs.iter().map(|d| json!({
            "id": d.id, "effect": d.effect.as_str(), "approval_scope": d.approval_scope.as_str(), "def_hash": d.def_hash(),
        })).collect::<Vec<_>>(),
    });
    if let (Some(map), Some(extra)) = (start.as_object_mut(), spec.start_fields.as_object()) {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    // The start event's `raw_blob` is the exact text written to the CLI's stdin.
    start["stdin_prompt_chars"] = json!(spec.prompt.chars().count());
    let stdin_bytes = vendor.stdin_payload(spec.prompt);
    let prompt_blob = rec.ledger.put_blob(&stdin_bytes).map_err(|e| e.to_string())?;
    rec.emit_into(sink, spec.start_kind, start, Some(&prompt_blob))?;
    if !version.matches_tested() {
        let warn = json!({ "source": "version-flag", "detected": version.detected, "raw": version.raw, "tested": version.tested });
        rec.emit_into(sink, "cli.version_warning", warn, None)?;
    }
    if stop.is_stopped() {
        rec.emit_into(sink, "session.abort", json!({ "reason": "stopped by the user", "stopped": true }), None)?;
        return Ok(TurnReport { end: TurnEnd::Stopped, final_text: String::new() });
    }

    // Write the per-turn tool config (carrying `CORE_SESSION_TOKEN`) only right before
    // spawning the CLI, after `version()` and stop checks have passed. Declared before
    // `ProcGuard` so the process tree is killed before the config file is deleted on drop.
    let _config_guard = ToolConfigGuard::new(spec.tool_config);
    vendor.write_tool_config(spec.tool_config, spec.core)?;

    let mut command = Command::new(spec.exe);
    command
        .args(&args)
        .current_dir(spec.work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if spec.inherit_stderr { Stdio::inherit() } else { Stdio::null() });
    vendor.prepare(&mut command);
    // Job object: if the supervisor dies, aborts or is stopped, the CLI and everything it
    // started (core.exe, the worker) die with it.
    let mut proc = JobObjectSandbox.spawn_command(command).map_err(spawn_err)?;
    let stdin = proc.child().stdin.take();
    let stdout = proc.child().stdout.take();
    let proc = Arc::new(Mutex::new(proc));
    stop.attach(Arc::clone(&proc));
    let guard = ProcGuard { proc: Arc::clone(&proc), stop: stop.clone() };
    if stop.is_stopped() {
        guard.proc.lock().unwrap_or_else(|e| e.into_inner()).kill_tree();
    }
    let stdout = stdout.ok_or("the CLI has no stdout pipe")?;

    // The prompt goes on stdin (not visible in the process list, no argv length limit).
    // A separate thread, so a CLI that does not read stdin cannot block the line loop.
    let writer = stdin.map(|mut pipe| {
        std::thread::spawn(move || {
            let _ = pipe.write_all(&stdin_bytes).and_then(|_| pipe.flush());
            // Dropping `pipe` closes stdin: end of prompt.
        })
    });

    let mut reader = BufReader::new(stdout);
    let mut child = LiveTurnProcess { guard: Some(guard), writer };
    drive_turn_stream(
        vendor,
        rec,
        sink,
        &version,
        &expected,
        spec.allow_env_auth,
        stop,
        &mut reader,
        &mut child,
    )
}

trait TurnProcess {
    fn kill(&mut self);
    fn wait(&mut self) -> Result<Option<i32>, String>;
}

struct LiveTurnProcess {
    guard: Option<ProcGuard>,
    writer: Option<std::thread::JoinHandle<()>>,
}

impl TurnProcess for LiveTurnProcess {
    fn kill(&mut self) {
        drop(self.guard.take());
        join(self.writer.take());
    }

    fn wait(&mut self) -> Result<Option<i32>, String> {
        let status = loop {
            let polled = self
                .guard
                .as_ref()
                .expect("proc guard active")
                .proc
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .child()
                .try_wait()
                .map_err(|e| e.to_string())?;
            match polled {
                Some(status) => break status,
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        self.kill(); // nothing the CLI started may outlive the turn
        Ok(status.code())
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_turn_stream<R: BufRead>(
    vendor: &dyn VendorCli,
    rec: &Recorder,
    sink: &mut dyn EventSink,
    version: &crate::vendor::VersionInfo,
    expected: &[String],
    allow_env_auth: bool,
    stop: &StopHandle,
    reader: &mut R,
    proc: &mut dyn TurnProcess,
) -> Result<TurnReport, String> {
    let mut init_ok = false;
    let mut abort: Option<String> = None;
    let (mut last_text, mut result_text) = (String::new(), String::new());
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                // A killed tree (stop) closes the pipe; anything else is a gap in the record.
                if !stop.is_stopped() {
                    abort = Some(format!("could not read the CLI's output: {e}"));
                }
                break;
            }
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        // The exact bytes the CLI printed are kept, decodable or not.
        let raw = rec.ledger.put_blob(&bytes).map_err(|e| e.to_string())?;
        let Ok(line) = std::str::from_utf8(&bytes) else {
            rec.emit_into(sink, "cli.unparsed", json!({ "reason": "not valid UTF-8", "bytes": bytes.len() }), Some(&raw))?;
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            rec.emit_into(sink, "cli.unparsed", json!({}), Some(&raw))?;
            continue;
        };
        if vendor.is_handshake(&value) {
            if let Some(init_ver) = vendor.handshake_version(&value) {
                if init_ver != version.tested && init_ver != version.detected {
                    let warn = json!({ "source": "system-init", "detected": init_ver, "tested": version.tested });
                    rec.emit_into(sink, "cli.version_warning", warn, Some(&raw))?;
                }
            }
            match vendor.check_handshake(&value, expected).and_then(|()| vendor.check_auth(&value, allow_env_auth)) {
                Ok(()) => init_ok = true,
                Err(e) => abort = Some(e),
            }
        } else if !init_ok && vendor.is_model_activity(&value) {
            let kind = value["type"].as_str().or_else(|| value["event"].as_str()).unwrap_or("");
            abort = Some(format!("`{kind}` event before a verified init handshake"));
        }
        // The offending line itself is still recorded (it is what the CLI printed).
        for (kind, data) in vendor.map_line(&value) {
            match kind.as_str() {
                "model.text" => last_text = data["text"].as_str().unwrap_or("").to_string(),
                "cli.result" => result_text = data["result"].as_str().unwrap_or("").to_string(),
                _ => {}
            }
            rec.emit_into(sink, &kind, data, Some(&raw))?;
        }
        if abort.is_some() {
            break;
        }
    }
    let final_text = if last_text.is_empty() { result_text } else { last_text };

    if let Some(reason) = abort {
        proc.kill();
        rec.emit_into(sink, "session.abort", json!({ "reason": reason }), None)?;
        return Ok(TurnReport { end: TurnEnd::Aborted(reason), final_text });
    }
    if stop.is_stopped() {
        proc.kill();
        rec.emit_into(sink, "session.abort", json!({ "reason": "stopped by the user", "stopped": true }), None)?;
        return Ok(TurnReport { end: TurnEnd::Stopped, final_text });
    }
    let exit_code = proc.wait()?;
    if stop.is_stopped() && exit_code != Some(0) {
        rec.emit_into(sink, "session.abort", json!({ "reason": "stopped by the user", "stopped": true }), None)?;
        return Ok(TurnReport { end: TurnEnd::Stopped, final_text });
    }
    if !init_ok {
        let reason = "CLI exited without a verified init handshake".to_string();
        rec.emit_into(sink, "session.abort", json!({ "reason": reason, "cli_exit_code": exit_code }), None)?;
        return Ok(TurnReport { end: TurnEnd::Aborted(reason), final_text });
    }
    rec.emit_into(sink, "session.end", json!({ "cli_exit_code": exit_code, "init_verified": true }), None)?;
    Ok(TurnReport { end: TurnEnd::Completed { exit_code, init_verified: true }, final_text })
}

fn join(writer: Option<std::thread::JoinHandle<()>>) {
    if let Some(w) = writer {
        let _ = w.join();
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
