//! The Google Antigravity CLI (`agy`) as a [`VendorCli`]: "vendor-sandboxed, not confined
//! to re:Search tools".
//!
//! Unlike the Claude Code route, the CLI's built-in tools (terminal, file, web, browser)
//! cannot be removed from the model's tool list. re:Search therefore only ASKS the CLI,
//! through a per-session `--settings` object, to deny every built-in permission kind and to
//! allow only `mcp(core/*)`; whether that holds is up to the CLI. The handshake check is
//! "core tools reachable + settings applied" (`permission_mode` is `request-review`), and
//! any built-in tool the CLI reports running is recorded as `vendor.builtin_tool`.
//!
//! Sources for every flag and field used here (Antigravity CLI 1.5.14, read 2026-11-21):
//! `agy --help` (captured in `fixtures/agy_help_1_5_14.txt`), `agy changelog`, and Google's
//! documentation pages "Headless mode" (`/docs/cli/headless`), "Settings" (`/docs/settings`),
//! "Permissions" (`/docs/permissions`, CLI tab) and "MCP" (`/docs/mcp`). Mapping and checks
//! are pure; the raw line is kept as a blob, as on the Claude route.
//!
//! Print mode, not `agy acp`: core tools are allowed by rule, so the CLI never has to ask
//! about them (`core.exe` asks the person through re:Search's relay), and everything else is
//! denied, so an approval channel to the CLI would only ever be asked about tools re:Search
//! does not want run. Print mode soft-denies whatever needs an approval (Headless page),
//! which fails closed. The prompt goes on stdin as one stream-json `user` event
//! (`--input-format stream-json`; a `-p` prompt would be argv, and is dropped in that mode).
//! Every event's fields sit under a key named after the event (`{"event":"init","init":{..}}`).

use crate::vendor::{CoreLaunch, VendorCli, VersionInfo};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

/// Antigravity CLI version this route was written against (the CLI updates itself).
pub const TESTED_CLI_VERSION: &str = "1.5.14";
/// Route label recorded in the ledger.
pub const ROUTE: &str = "antigravity";
/// Confinement recorded in the start event and shown in the UI.
pub const CONFINEMENT: &str = "vendor-sandboxed";
/// The label every surface of this route carries.
pub const LABEL_SUFFIX: &str = "vendor-sandboxed, not confined to re:Search tools";
/// MCP server name of `core.exe` in the workspace `mcp_config.json`.
pub const CORE_SERVER: &str = "core";
/// The per-turn tool config, relative to the CLI's working directory.
pub const TOOL_CONFIG_REL: &str = ".agents/mcp_config.json";
/// `permission_mode` the headless `init` event reports when nothing auto-approves.
pub const EXPECTED_PERMISSION_MODE: &str = "request-review";

/// Environment variables with which the CLI would use something other than the cached
/// interactive Google login: a Gemini API key (used with `modelProvider: "gemini"`), a
/// custom Gemini endpoint, or Application Default Credentials (from `agy changelog` and
/// the Settings page).
pub const AGY_ENV_AUTH_VARS: &[&str] = &["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "AGY_ADC_AUTH"];
/// Opt-in: `1`, `true` or `yes` lets a turn start although one of [`AGY_ENV_AUTH_VARS`] is set.
pub const ALLOW_ENV_AUTH_VAR: &str = "RE_SEARCH_ALLOW_AGY_ENV_AUTH";

/// Built-in permission kinds denied for the whole session (the "Permissions" page lists
/// `read_file`, `write_file`, `read_url`, `execute_url`, `command`, `unsandboxed`, `mcp`;
/// `unsandboxed` is honoured for business accounts only and `command` covers both sides of
/// the terminal sandbox, so it is not repeated).
pub const DENIED_KINDS: [&str; 5] = ["command", "read_file", "write_file", "read_url", "execute_url"];

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

/// The `--settings` object for one session (never saved by the CLI): Default agent mode
/// (nothing auto-approved; `auto` would approve edits and safe commands), `request-review`
/// tool permission, terminal sandbox on, no access outside the workspace, every built-in
/// kind denied and only `core`'s MCP tools allowed. Deny > ask > allow, so a user-level
/// allow rule for a built-in kind cannot win. Other MCP servers (global or plugin ones,
/// which still load) cannot be denied by rule without also denying core (`mcp(*)` would
/// beat `mcp(core/*)`, and their names would have to be read from the user's Antigravity
/// folder, which re:Search does not read): they have no rule here and fall back to Ask
/// (Permissions page), which print mode soft-denies, unless a user-level allow rule of
/// the person's own covers them. `strict` is not used: it asks even when an allow rule
/// matches, so core tools could never run in print mode.
pub fn session_settings() -> Value {
    json!({
        "agentMode": "default",
        "toolPermission": EXPECTED_PERMISSION_MODE,
        "enableTerminalSandbox": true,
        "allowNonWorkspaceAccess": false,
        "permissions": {
            "allow": [format!("mcp({CORE_SERVER}/*)")],
            "ask": [],
            "deny": DENIED_KINDS.iter().map(|k| format!("{k}(*)")).collect::<Vec<_>>(),
        },
    })
}

/// The flags (all from `agy --help` 1.5.14): print mode with NDJSON prompts on stdin and
/// NDJSON events out, the model, terminal sandbox, no slash-command expansion, and the
/// session settings inline. `--mode` is not passed: its values are `accept-edits`, `auto`
/// and `plan`, each less restrictive than (or, for `plan`, a prompt rewrite on top of) the
/// Default mode, which is selected through `agentMode` in `--settings`. No `-p`: the prompt
/// is the stdin `user` event ([`Antigravity::stdin_payload`]).
pub fn agy_args(model: &str) -> Vec<String> {
    vec![
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--model".into(),
        model.into(),
        "--sandbox".into(),
        "--disable-slash-commands".into(),
        "--settings".into(),
        session_settings().to_string(),
    ]
}

/// The stdin line for one prompt: a stream-json `user` event (Headless page), then EOF,
/// after which the CLI finishes the turn and exits.
pub fn user_event(prompt: &str) -> Vec<u8> {
    let mut line = json!({ "event": "user", "message": { "content": prompt } }).to_string().into_bytes();
    line.push(b'\n');
    line
}

/// An event's fields: the object under the key named after the event, or `Null`.
pub fn body<'a>(line: &'a Value, event: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    match line.get(event) {
        Some(v) if line["event"] == event && v.is_object() => v,
        _ => &NULL,
    }
}

/// Name under which an eagerly loaded MCP tool is offered (`mcp_<server>_<tool>`, MCP page).
pub fn eager_name(tool_id: &str) -> String {
    format!("mcp_{CORE_SERVER}_{tool_id}")
}

/// Leading version token of `agy --version` (it prints e.g. `1.5.14`).
pub fn parse_cli_version(raw: &str) -> &str {
    raw.split_whitespace().next().unwrap_or("")
}

const TEXT_LIMIT: usize = 4000;

fn clip(text: &str) -> (String, bool) {
    if text.chars().count() <= TEXT_LIMIT {
        (text.to_string(), false)
    } else {
        (text.chars().take(TEXT_LIMIT).collect(), true)
    }
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Whether a tool step is one of `core`'s: an eager `mcp_core_*` tool, or the lazy
/// `call_mcp_tool` aimed at server `core`.
pub fn is_core_tool(name: &str, params: &Value) -> bool {
    name.starts_with(&format!("mcp_{CORE_SERVER}_"))
        || (name == "call_mcp_tool" && params.get("ServerName").and_then(Value::as_str) == Some(CORE_SERVER))
}

/// Check the `init` event: the CLI reports `request-review` (nothing auto-approved, i.e.
/// the session settings were applied and `--dangerously-skip-permissions` is not in
/// effect), and offers every expected core tool by its eager name and no other `mcp_core_*`
/// tool. Built-in tools cannot be removed, so they are allowed here (and recorded).
pub fn check_init(line: &Value, expected_tools: &[String]) -> Result<(), String> {
    let init = body(line, "init");
    if !init.is_object() {
        return Err("not an init event with an `init` object".into());
    }
    match init.get("permission_mode").and_then(Value::as_str) {
        Some(EXPECTED_PERMISSION_MODE) => {}
        other => {
            return Err(format!(
                "settings not applied: the CLI reports permission_mode {}, expected `{EXPECTED_PERMISSION_MODE}`",
                other.map(|s| format!("`{}`", s.chars().take(40).collect::<String>())).unwrap_or_else(|| "(missing)".into())
            ))
        }
    }
    let tools: Vec<String> = init["tools"]
        .as_array()
        .ok_or("init event has no `tools` list")?
        .iter()
        .map(|t| t.as_str().unwrap_or("<non-string>").to_string())
        .collect();
    let missing: Vec<&String> = expected_tools.iter().filter(|t| !tools.contains(t)).collect();
    if !missing.is_empty() {
        return Err(format!("core tools not reachable: the CLI does not offer {missing:?}"));
    }
    let prefix = format!("mcp_{CORE_SERVER}_");
    let extra: Vec<&String> = tools.iter().filter(|t| t.starts_with(&prefix) && !expected_tools.contains(t)).collect();
    if !extra.is_empty() {
        return Err(format!("unexpected core tools offered: {extra:?}"));
    }
    Ok(())
}

/// Built-in (non-core) tools an `init` event lists.
pub fn builtin_tools(line: &Value) -> Vec<String> {
    let prefix = format!("mcp_{CORE_SERVER}_");
    body(line, "init")["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|t| !t.starts_with(&prefix))
        .map(str::to_string)
        .collect()
}

/// Per-turn state for joining streamed steps (text deltas, first sighting of a tool step).
#[derive(Default)]
struct Steps {
    text: HashMap<u64, String>,
    tool_started: HashSet<u64>,
}

/// The Antigravity CLI. One value per turn (it joins streamed steps).
#[derive(Default)]
pub struct Antigravity {
    steps: Mutex<Steps>,
}

impl Antigravity {
    pub fn new() -> Self {
        Self::default()
    }

    fn map_step(&self, line: &Value) -> Vec<(String, Value)> {
        let line = body(line, "step_update");
        let idx = line["step_index"].as_u64().unwrap_or(u64::MAX);
        let state = line["state"].as_str().unwrap_or("");
        let done = state == "DONE";
        let step_type = line["step_type"].as_str().unwrap_or("");
        let mut steps = self.steps.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        match step_type {
            "agent_response" => {
                let delta = line["text_delta"].as_str().unwrap_or("");
                let acc = steps.text.entry(idx).or_default();
                // A final frame that repeats the whole text replaces the joined deltas.
                if done && !acc.is_empty() && delta.starts_with(acc.as_str()) {
                    *acc = delta.to_string();
                } else {
                    acc.push_str(delta);
                }
                if done {
                    let text = steps.text.remove(&idx).unwrap_or_default();
                    if !text.is_empty() {
                        out.push(("model.text".to_string(), json!({ "text": text, "step_index": idx })));
                    }
                } else {
                    out.push(("cli.delta".to_string(), json!({ "step_index": idx, "chars": delta.chars().count() })));
                }
            }
            "tool" => {
                let info = &line["tool_info"];
                let name = line["tool_name"].as_str().or_else(|| info["name"].as_str()).unwrap_or("").to_string();
                let params = info.get("parameters").cloned().unwrap_or(Value::Null);
                let error = info.get("error").filter(|e| !e.is_null()).cloned();
                let id = format!("step-{idx}");
                let first = steps.tool_started.insert(idx);
                if is_core_tool(&name, &params) {
                    if first {
                        out.push(("model.tool_use".to_string(), json!({ "id": id, "name": name, "input": params })));
                    }
                    if done {
                        let raw = match &error {
                            Some(e) => as_text(e.get("message").unwrap_or(e)),
                            None => as_text(&info["output"]),
                        };
                        let (text, truncated) = clip(&raw);
                        out.push((
                            "model.tool_result".to_string(),
                            json!({ "tool_use_id": id, "is_error": error.is_some(), "text": text, "truncated": truncated }),
                        ));
                    }
                } else if first || done {
                    // Outside re:Search's router: never hidden, recorded as its own kind.
                    let (shown, truncated) = clip(&as_text(&params));
                    out.push((
                        "vendor.builtin_tool".to_string(),
                        json!({
                            "step_index": idx, "state": state, "tool_name": name, "parameters": shown,
                            "parameters_truncated": truncated, "error": error, "denied_or_failed": error.is_some(),
                            "confinement": CONFINEMENT,
                        }),
                    ));
                }
            }
            other => out.push(("cli.step".to_string(), json!({ "step_type": other, "state": state, "step_index": idx }))),
        }
        out
    }
}

/// Map one `result` event.
fn map_result(line: &Value) -> (String, Value) {
    let line = body(line, "result");
    let status = line["status"].as_str().unwrap_or("");
    (
        "cli.result".to_string(),
        json!({
            "status": status,
            "is_error": status != "SUCCESS",
            "result": line["response"],
            "error": line["error"],
            "finished_reason": line["finished_reason"],
            "num_turns": line["num_turns"],
            "duration_seconds": line["duration_seconds"],
            "usage": line["usage"],
            "conversation_id": line["conversation_id"],
        }),
    )
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

/// A slug the CLI could never read as a flag.
pub fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id.len() <= 100
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

impl VendorCli for Antigravity {
    fn route(&self) -> &'static str {
        ROUTE
    }

    fn caller(&self, session: &str) -> String {
        format!("{ROUTE}:{session}")
    }

    /// `explicit`, then `RE_SEARCH_AGY`, `agy.exe` on `PATH`, WinGet's link and package folders.
    fn locate(&self, explicit: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
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

    fn version(&self, exe: &Path) -> Result<VersionInfo, String> {
        let raw = run_bounded(exe, &["--version"], VERSION_TIMEOUT)
            .map(|t| t.lines().next().unwrap_or("").trim().to_string())
            .map_err(|e| format!("`{} --version`: {e}", exe.display()))?;
        let detected = parse_cli_version(&raw).to_string();
        Ok(VersionInfo { raw, detected, tested: TESTED_CLI_VERSION.to_string() })
    }

    fn env_auth_vars(&self) -> &'static [&'static str] {
        AGY_ENV_AUTH_VARS
    }

    fn tool_name(&self, tool_id: &str) -> String {
        eager_name(tool_id)
    }

    /// `<work>/.agents/mcp_config.json`: only `core`, only re:Search's tools (`enabledTools`),
    /// each loaded eagerly so the model sees it by name and `init` lists it.
    fn write_tool_config(&self, path: &Path, core: &CoreLaunch) -> Result<(), String> {
        let ids = crate::core_tools::core_tool_ids_for(&core.env);
        let eager: serde_json::Map<String, Value> = ids.iter().map(|id| (id.clone(), json!({ "eager": true }))).collect();
        let config = json!({ "mcpServers": { CORE_SERVER: {
            "command": core.exe.to_string_lossy(),
            "args": [],
            "env": core.env,
            "enabledTools": ids,
            "tools": eager,
        }}});
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))
    }

    fn args(&self, model: &str, _tool_config: &Path) -> Vec<String> {
        agy_args(model)
    }

    fn stdin_payload(&self, prompt: &str) -> Vec<u8> {
        user_event(prompt)
    }

    fn prompt_via(&self) -> &'static str {
        "stdin (stream-json user event)"
    }

    fn prepare(&self, _cmd: &mut Command) {}

    fn is_handshake(&self, line: &Value) -> bool {
        line["event"] == "init"
    }

    fn check_handshake(&self, line: &Value, expected_tools: &[String]) -> Result<(), String> {
        check_init(line, expected_tools)
    }

    /// The `init` event carries no credential source; the environment refusal
    /// ([`check_env_auth`]) runs before the start.
    fn check_auth(&self, _line: &Value, _allow_env_auth: bool) -> Result<(), String> {
        Ok(())
    }

    fn handshake_version(&self, _line: &Value) -> Option<String> {
        None
    }

    fn is_model_activity(&self, line: &Value) -> bool {
        matches!(line["event"].as_str().unwrap_or(""), "step_update" | "result")
    }

    fn map_line(&self, line: &Value) -> Vec<(String, Value)> {
        match line["event"].as_str().unwrap_or("") {
            "init" => {
                let init = body(line, "init");
                vec![(
                    "cli.init".to_string(),
                    json!({
                        "tools": init["tools"],
                        "builtin_tools": builtin_tools(line),
                        "permission_mode": init["permission_mode"],
                        "cwd": init["cwd"],
                        "model": init["model"],
                        "agent": init["agent"],
                        "conversation_id": line["conversation_id"],
                        "confinement": CONFINEMENT,
                    }),
                )]
            }
            "step_update" => self.map_step(line),
            "result" => vec![map_result(line)],
            other => vec![("cli.other".to_string(), json!({ "event": other }))],
        }
    }
}

#[cfg(test)]
#[path = "antigravity_tests.rs"]
mod tests;
