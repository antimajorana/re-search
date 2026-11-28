//! Plain prompt-in, JSON-block-out Explore exchange transport over subscription CLIs
//! (`claude-cli`, `antigravity`; package X18 Checkpoint 1).
//!
//! Governance: `explore.md` 5.1, 6 (Rules P2, P3, C7), 9 (row X18), 10; `00-overview.md` 2.2, 3.1.
//!
//! Bridges `agent::exchange::run_exchange` to unmodified vendor CLIs without registering any
//! MCP tools. Extracted JSON objects are returned as a single `ContentBlock::ToolUse` for the
//! exchange's recorder so the exact same Rust validators, `<= 2` repair rounds, F9 budget checks
//! (Rule P3), and `idea.exchange` provenance audit events run identically to API routes.

#[path = "idea_cli_exchange_parse.rs"]
mod parse;

pub use parse::{
    check_claude_exchange_init, exchange_agy_args, exchange_agy_settings, exchange_claude_args,
    extract_json_object,
};

use crate::cli_turn_setup::{pick_model, CliRouteKind};
use crate::state::DesktopState;
use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, StreamObserver,
    Transport, TransportError, TransportErrorKind, TransportInfo,
};
use cli_route::antigravity::{
    check_env_auth as agy_check_env_auth, user_event as agy_user_event, Antigravity,
    ALLOW_ENV_AUTH_VAR as AGY_ALLOW_ENV_AUTH_VAR, CONFINEMENT as AGY_CONFINEMENT,
    ROUTE as AGY_ROUTE,
};
use cli_route::claude::{ClaudeCode, ROUTE as CLAUDE_ROUTE};
use cli_route::env_auth::{
    check as claude_check_env_auth, flag_set, opted_in as claude_opted_in,
    ALLOW_ENV_AUTH_VAR as CLAUDE_ALLOW_ENV_AUTH_VAR,
};
use cli_route::vendor::VendorCli;
use parse::{render_plain_exchange_prompt, ExchangeStreamState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tools::canon::random_hex;

/// Default wall-clock ceiling for a single CLI exchange attempt.
const EXCHANGE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(600);
/// Poll interval for checking cooperative cancellation while waiting for CLI output.
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// Upper bound on captured raw stdout bytes per attempt to prevent unbounded memory growth.
const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreparedEnvelope {
    route: String,
    provenance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confinement: Option<String>,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    recorder: String,
    prompt: String,
}

/// Thread-safe [`Transport`] executing Explore structured exchanges over a subscription CLI
/// (`claude-cli` or `antigravity`). Safe for concurrent parallel passage readers (each attempt
/// gets its own isolated working directory cleaned up on drop).
pub struct CliExchangeTransport {
    kind: CliRouteKind,
    exe: PathBuf,
    work_root: PathBuf,
    allow_env_auth: bool,
    timeout: Duration,
}

impl CliExchangeTransport {
    pub fn new(
        kind: CliRouteKind,
        exe: PathBuf,
        work_root: PathBuf,
        allow_env_auth: bool,
    ) -> Self {
        Self {
            kind,
            exe,
            work_root,
            allow_env_auth,
            timeout: EXCHANGE_ATTEMPT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn route_label(&self) -> &'static str {
        self.kind.ledger_route()
    }
}

/// Validate consent, billing safety, executable path, and model slug, then construct a
/// [`CliExchangeTransport`] for Explore exchanges on `kind`.
pub fn build_cli_exchange_transport(
    state: &DesktopState,
    session_id: &str,
    kind: CliRouteKind,
    req: Option<&Value>,
) -> Result<(Box<dyn Transport>, String), (u16, String)> {
    let env_lookup = |n: &str| state.env_var(n);
    // Opt-in env-auth bypass flags come strictly from the real process environment (matching cli_turn.rs).
    let auth_env = |n: &str| {
        if n == CLAUDE_ALLOW_ENV_AUTH_VAR || n == AGY_ALLOW_ENV_AUTH_VAR {
            std::env::var_os(n)
        } else {
            std::env::var_os(n).or_else(|| state.env_var(n))
        }
    };
    let (exe, allow_env_auth) = match kind {
        CliRouteKind::Claude => {
            claude_check_env_auth(ClaudeCode.env_auth_vars(), &auth_env).map_err(|e| (400, e))?;
            let exe = ClaudeCode
                .locate(state.cfg.claude_exe.as_deref(), &env_lookup)
                .map_err(|e| (400, e))?;
            (exe, claude_opted_in(&auth_env))
        }
        CliRouteKind::Antigravity => {
            crate::antigravity::require_acknowledged(state).map_err(|e| (403, e))?;
            agy_check_env_auth(&auth_env).map_err(|e| (400, e))?;
            let exe = Antigravity::new()
                .locate(state.cfg.agy_exe.as_deref(), &env_lookup)
                .map_err(|e| (400, e))?;
            (exe, flag_set(AGY_ALLOW_ENV_AUTH_VAR, &auth_env))
        }
    };

    let meta = state.load_meta(session_id);
    let ui = state.load_ui_settings();
    let empty_req = json!({});
    let model = pick_model(req.unwrap_or(&empty_req), meta.as_ref(), &ui, kind)
        .map_err(|e| (400, e))?;

    let sdir = state.session_dir(session_id).map_err(|e| (400, e))?;
    let work_root = sdir.join(match kind {
        CliRouteKind::Claude => "work",
        CliRouteKind::Antigravity => "agy",
    });
    std::fs::create_dir_all(&work_root)
        .map_err(|e| (500, format!("could not create {}: {e}", work_root.display())))?;

    Ok((
        Box::new(CliExchangeTransport::new(kind, exe, work_root, allow_env_auth)),
        model,
    ))
}

struct AttemptDirGuard(PathBuf);
impl Drop for AttemptDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
struct ExchangeChild(worker::win::JobChild);

#[cfg(not(windows))]
struct ExchangeChild(std::process::Child);

impl ExchangeChild {
    #[cfg(windows)]
    fn spawn(cmd: Command) -> std::io::Result<Self> {
        worker::sandbox::JobObjectSandbox
            .spawn_command(cmd)
            .map(Self)
    }

    #[cfg(not(windows))]
    fn spawn(mut cmd: Command) -> std::io::Result<Self> {
        cmd.spawn().map(Self)
    }

    fn child(&mut self) -> &mut std::process::Child {
        #[cfg(windows)]
        {
            self.0.child()
        }
        #[cfg(not(windows))]
        {
            &mut self.0
        }
    }

    fn kill_tree(&mut self) {
        #[cfg(windows)]
        {
            self.0.kill_tree();
        }
        #[cfg(not(windows))]
        {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

impl Drop for ExchangeChild {
    fn drop(&mut self) {
        self.kill_tree();
    }
}

fn cfg_err(message: impl Into<String>) -> TransportError {
    TransportError {
        kind: TransportErrorKind::Config,
        status: None,
        message: message.into(),
        raw_body: None,
    }
}

fn net_err(message: impl Into<String>, raw_body: Option<Vec<u8>>) -> TransportError {
    TransportError {
        kind: TransportErrorKind::Network,
        status: None,
        message: message.into(),
        raw_body,
    }
}

fn cleanup_child(
    child: &mut ExchangeChild,
    writer: Option<std::thread::JoinHandle<()>>,
    reader: std::thread::JoinHandle<()>,
) {
    child.kill_tree();
    if let Some(w) = writer {
        let _ = w.join();
    }
    let _ = reader.join();
}

impl Transport for CliExchangeTransport {
    fn info(&self) -> TransportInfo {
        let provider = self.route_label();
        TransportInfo {
            provider,
            endpoint: format!("cli://{provider}"),
            api_key_env: "none".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        let (recorder, prompt) = render_plain_exchange_prompt(req)?;
        let envelope = PreparedEnvelope {
            route: self.route_label().to_string(),
            provenance: cli_route::PROVENANCE.to_string(),
            confinement: (self.kind == CliRouteKind::Antigravity)
                .then(|| AGY_CONFINEMENT.to_string()),
            model: req.model.clone(),
            reasoning_effort: req.reasoning_effort.clone(),
            recorder,
            prompt,
        };
        let body = serde_json::to_vec(&envelope)
            .map_err(|e| cfg_err(format!("failed to encode CLI exchange envelope: {e}")))?;
        Ok(PreparedRequest { body })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let mut noop = agent::NoopStreamObserver;
        self.send_prepared_streaming(prepared, &mut noop)
    }

    fn send_prepared_streaming(
        &self,
        prepared: &PreparedRequest,
        observer: &mut dyn StreamObserver,
    ) -> Result<ModelResponse, TransportError> {
        if observer.is_cancelled() {
            return Err(TransportError::cancelled());
        }
        let env: PreparedEnvelope = serde_json::from_slice(&prepared.body)
            .map_err(|e| cfg_err(format!("invalid prepared CLI exchange envelope: {e}")))?;

        let agy_vendor = Antigravity::new();
        let version = match self.kind {
            CliRouteKind::Claude => ClaudeCode.version(&self.exe),
            CliRouteKind::Antigravity => agy_vendor.version(&self.exe),
        }
        .map_err(|e| net_err(e, None))?;

        if observer.is_cancelled() {
            return Err(TransportError::cancelled());
        }

        let attempt_dir = self.work_root.join(format!("ex-{}", random_hex(6)));
        std::fs::create_dir_all(&attempt_dir)
            .map_err(|e| cfg_err(format!("could not create {}: {e}", attempt_dir.display())))?;
        let _dir_guard = AttemptDirGuard(attempt_dir.clone());

        let (args, stdin_bytes) = match self.kind {
            CliRouteKind::Claude => {
                let mcp_path = attempt_dir.join("mcp.json");
                write_empty_mcp_config(&mcp_path).map_err(|e| {
                    cfg_err(format!("could not write empty exchange mcp.json: {e}"))
                })?;
                (
                    exchange_claude_args(&env.model, &mcp_path, env.reasoning_effort.as_deref()),
                    env.prompt.as_bytes().to_vec(),
                )
            }
            CliRouteKind::Antigravity => (
                exchange_agy_args(&env.model, env.reasoning_effort.as_deref()),
                agy_user_event(&env.prompt),
            ),
        };

        let mut cmd = Command::new(&self.exe);
        cmd.args(&args)
            .current_dir(&attempt_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if self.kind == CliRouteKind::Claude {
            ClaudeCode.prepare(&mut cmd);
        }

        let mut child = ExchangeChild::spawn(cmd)
            .map_err(|e| net_err(format!("could not start {}: {e}", self.kind.cli_name()), None))?;
        let stdin = child.child().stdin.take();
        let stdout = child
            .child()
            .stdout
            .take()
            .ok_or_else(|| net_err("CLI process has no stdout pipe", None))?;

        let writer = stdin.map(|mut pipe| {
            std::thread::spawn(move || {
                let _ = pipe.write_all(&stdin_bytes).and_then(|_| pipe.flush());
            })
        });

        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
        let reader_thread = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(n) => {
                        total = total.saturating_add(n);
                        if total > MAX_CAPTURE_BYTES {
                            let _ = tx.send(Err("CLI stdout exceeded 4 MiB safety ceiling".into()));
                            break;
                        }
                        if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("error reading CLI stdout: {e}")));
                        break;
                    }
                }
            }
        });

        let deadline = Instant::now() + self.timeout;
        let mut stream = ExchangeStreamState::new(&env.model);

        loop {
            if observer.is_cancelled() {
                cleanup_child(&mut child, writer, reader_thread);
                return Err(TransportError::cancelled());
            }
            if Instant::now() >= deadline {
                cleanup_child(&mut child, writer, reader_thread);
                return Err(net_err(
                    format!(
                        "{} exchange timed out after {} s",
                        self.kind.cli_name(),
                        self.timeout.as_secs()
                    ),
                    Some(build_raw_audit(self.kind, &version.raw, &stream)),
                ));
            }
            let raw_bytes = match rx.recv_timeout(CANCEL_POLL) {
                Ok(Ok(b)) => b,
                Ok(Err(e)) => {
                    stream.abort = Some(e);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let trimmed = String::from_utf8_lossy(&raw_bytes)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if trimmed.trim().is_empty() {
                continue;
            }
            stream.feed_line(self.kind, &agy_vendor, self.allow_env_auth, trimmed);
            if stream.abort.is_some() {
                break;
            }
        }

        let raw_body = build_raw_audit(self.kind, &version.raw, &stream);
        if let Some(reason) = stream.abort {
            cleanup_child(&mut child, writer, reader_thread);
            return Err(net_err(reason, Some(raw_body)));
        }

        let exit_code = loop {
            if observer.is_cancelled() {
                cleanup_child(&mut child, writer, reader_thread);
                return Err(TransportError::cancelled());
            }
            match child.child().try_wait() {
                Ok(Some(st)) => break st.code(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(None) => {
                    cleanup_child(&mut child, writer, reader_thread);
                    return Err(net_err("CLI process did not exit after stream closed", Some(raw_body)));
                }
                Err(e) => {
                    cleanup_child(&mut child, writer, reader_thread);
                    return Err(net_err(e.to_string(), Some(raw_body)));
                }
            }
        };
        cleanup_child(&mut child, writer, reader_thread);

        if !stream.init_ok {
            return Err(net_err(
                format!("{} exited without a verified init handshake", self.kind.cli_name()),
                Some(raw_body),
            ));
        }
        if !stream.result_received {
            return Err(net_err(
                format!("{} exited without a terminal result event", self.kind.cli_name()),
                Some(raw_body),
            ));
        }
        if exit_code != Some(0) {
            return Err(net_err(
                format!("{} exited with code {exit_code:?}", self.kind.cli_name()),
                Some(raw_body),
            ));
        }

        // Rule P2: surface any unauthorized tool call directly so `run_exchange` records
        // `UnexpectedToolCall` in `idea.exchange` and fails closed.
        if let Some((id, name, input)) = stream.unexpected_tool {
            return Ok(ModelResponse {
                raw_body,
                id: None,
                model: stream.observed_model,
                stop_reason: StopReason::ToolUse,
                content: vec![ContentBlock::ToolUse { id, name, input }],
                usage: stream.usage,
            });
        }

        let final_text = stream.final_text();
        let (stop_reason, content) = match extract_json_object(final_text) {
            Ok(obj) => (
                StopReason::ToolUse,
                vec![ContentBlock::ToolUse {
                    id: format!("cli-json-{}", random_hex(4)),
                    name: env.recorder,
                    input: obj,
                }],
            ),
            Err(parse_err) => (
                StopReason::EndTurn,
                vec![ContentBlock::Text {
                    text: format!("{final_text}\n\n[CLI JSON extraction fault: {parse_err}]"),
                }],
            ),
        };

        Ok(ModelResponse {
            raw_body,
            id: None,
            model: stream.observed_model,
            stop_reason,
            content,
            usage: stream.usage,
        })
    }
}

fn build_raw_audit(kind: CliRouteKind, cli_version: &str, stream: &ExchangeStreamState) -> Vec<u8> {
    let route = match kind {
        CliRouteKind::Claude => CLAUDE_ROUTE,
        CliRouteKind::Antigravity => AGY_ROUTE,
    };
    let confinement = (kind == CliRouteKind::Antigravity).then_some(AGY_CONFINEMENT);
    serde_json::to_vec(&json!({
        "route": route,
        "provenance": cli_route::PROVENANCE,
        "confinement": confinement,
        "cli_version": cli_version,
        "init_verified": stream.init_ok,
        "model": stream.observed_model,
        "text": stream.final_text(),
        "raw_ndjson_lines": stream.raw_lines,
    }))
    .unwrap_or_default()
}

fn write_empty_mcp_config(path: &Path) -> Result<(), String> {
    std::fs::write(path, br#"{"mcpServers":{}}"#).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
