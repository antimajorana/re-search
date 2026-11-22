//! The Claude Code CLI as a [`VendorCli`]: how it is found, started and read.
//!
//! re:Search only STARTS the unmodified `claude.exe` the user signed into. It never reads
//! or passes the CLI's login token, never passes `--bare`, and does not change how the CLI
//! authenticates (see [`crate::env_auth`] for the refusal when environment credentials
//! would take precedence over the subscription login).
//!
//! The mapping and the `system/init` check are pure. The CLI's format is not ours and may
//! change between versions, so every raw line is also kept as a blob and each event
//! carries the hash of the line it came from.

use crate::vendor::{CoreLaunch, VendorCli, VersionInfo};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Claude Code CLI version this route was tested against.
pub const TESTED_CLI_VERSION: &str = "2.1.318";

/// Route label recorded in the ledger.
pub const ROUTE: &str = "claude-code-cli";

/// Environment variables the Claude CLI prefers over the subscription login in
/// non-interactive mode (API key, bearer/OAuth tokens, third-party cloud providers).
pub const CLAUDE_ENV_AUTH_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// Extract the leading version token from `claude --version` output
/// (for example `"2.1.318 (Claude Code)"` -> `"2.1.318"`).
pub fn parse_cli_version(raw: &str) -> &str {
    raw.split_whitespace().next().unwrap_or("")
}

/// Longest tool-result text copied into an event (the raw line has it all).
const RESULT_TEXT_LIMIT: usize = 4000;

fn clip(text: &str) -> (String, bool) {
    if text.chars().count() <= RESULT_TEXT_LIMIT {
        (text.to_string(), false)
    } else {
        (text.chars().take(RESULT_TEXT_LIMIT).collect(), true)
    }
}

/// Text of a tool_result `content`, which is a string or a list of blocks.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Map one parsed stream-json line to zero or more `(kind, data)` events.
pub fn map_line(line: &Value) -> Vec<(String, Value)> {
    let ty = line["type"].as_str().unwrap_or("");
    let subtype = line["subtype"].as_str().unwrap_or("");
    let mut out = Vec::new();
    match (ty, subtype) {
        ("system", "init") => out.push((
            "cli.init".to_string(),
            json!({
                "model": line["model"],
                "tools": line["tools"],
                "mcp_servers": line["mcp_servers"],
                "cli_version": line["claude_code_version"],
                "api_key_source": line["apiKeySource"],
                "permission_mode": line["permissionMode"],
                "cli_session_id": line["session_id"],
            }),
        )),
        ("assistant", _) => {
            for block in line["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or("") {
                    "text" => out.push(("model.text".to_string(), json!({ "text": block["text"] }))),
                    "tool_use" => out.push((
                        "model.tool_use".to_string(),
                        json!({ "id": block["id"], "name": block["name"], "input": block["input"] }),
                    )),
                    "thinking" => out.push((
                        "model.thinking".to_string(),
                        json!({ "chars": block["thinking"].as_str().map_or(0, |s| s.chars().count()) }),
                    )),
                    other => out.push(("model.other".to_string(), json!({ "block_type": other }))),
                }
            }
        }
        ("user", _) => {
            for block in line["message"]["content"].as_array().into_iter().flatten() {
                if block["type"] == "tool_result" {
                    let (text, truncated) = clip(&result_text(&block["content"]));
                    out.push((
                        "model.tool_result".to_string(),
                        json!({
                            "tool_use_id": block["tool_use_id"],
                            "is_error": block["is_error"].as_bool().unwrap_or(false),
                            "text": text,
                            "truncated": truncated,
                        }),
                    ));
                }
            }
        }
        ("result", _) => out.push((
            "cli.result".to_string(),
            json!({
                "subtype": subtype,
                "is_error": line["is_error"],
                "result": line["result"],
                "num_turns": line["num_turns"],
                "duration_ms": line["duration_ms"],
                "total_cost_usd": line["total_cost_usd"],
                "usage": line["usage"],
            }),
        )),
        _ => out.push(("cli.other".to_string(), json!({ "type": ty, "subtype": subtype }))),
    }
    out
}

/// Check the CLI's `system/init` line: the model must be offered exactly the expected
/// tools (no built-ins, nothing extra, nothing missing) and the `core` MCP server must be
/// the only server and connected. Any difference is an error; the turn is aborted on it.
pub fn check_init(init: &Value, expected_tools: &[String]) -> Result<(), String> {
    if init["type"] != "system" || init["subtype"] != "init" {
        return Err("not a system/init event".into());
    }
    let tools = init["tools"].as_array().ok_or("init event has no `tools` list")?;
    let mut seen: Vec<String> = tools.iter().map(|t| t.as_str().unwrap_or("<non-string>").to_string()).collect();
    seen.sort();
    let mut expected = expected_tools.to_vec();
    expected.sort();
    if seen != expected {
        return Err(format!("tool list mismatch: the CLI offers {seen:?}, expected exactly {expected:?}"));
    }
    let servers = init["mcp_servers"].as_array().ok_or("init event has no `mcp_servers` list")?;
    if servers.len() != 1 || servers[0]["name"] != "core" || servers[0]["status"] != "connected" {
        return Err(format!("MCP servers mismatch: {}, expected only `core`, connected", init["mcp_servers"]));
    }
    Ok(())
}

/// The flags of the pilot (Claude Code 2.1.318): print mode, stream-json output, no
/// built-in tools, only our MCP config, no user/project settings, no slash commands, no
/// session files, only `mcp__core__*` pre-allowed, and nobody to answer a permission
/// prompt (anything that would prompt is denied). The prompt is NOT an argument: it is
/// written to the CLI's stdin, which `-p` reads when no prompt argument is given.
pub fn claude_args(model: &str, mcp_config: &str) -> Vec<String> {
    [
        "-p",
        "--model", model,
        "--output-format", "stream-json",
        "--verbose",
        "--tools", "",
        "--strict-mcp-config",
        "--mcp-config", mcp_config,
        "--setting-sources", "",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--permission-prompts", "none",
        "--allowedTools", "mcp__core__*",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Billing check on the CLI's own report: `system/init` carries `apiKeySource`, which is
/// `"none"` when the CLI uses the subscription login (the live 2.1.318 recording shows
/// `"apiKeySource":"none"`). Any other value (for example `"ANTHROPIC_API_KEY"` or
/// `"apiKeyHelper"`) means an API key would be billed; that is refused unless the person
/// opted in with [`crate::env_auth::ALLOW_ENV_AUTH_VAR`]. A missing field is accepted (older
/// CLIs did not report it); the environment refusal still applies before the start.
pub fn check_auth_source(init: &Value, allow_env_auth: bool) -> Result<(), String> {
    match init.get("apiKeySource") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(s)) if s == "none" => Ok(()),
        Some(_) if allow_env_auth => Ok(()),
        Some(other) => {
            let shown: String = match other {
                Value::String(s) => s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).take(60).collect(),
                _ => "<not a string>".to_string(),
            };
            Err(format!(
                "Claude Code reports apiKeySource `{shown}` (not \"none\"): it would bill an API key instead of your \
                 Claude subscription. Remove that credential (or apiKeyHelper) from Claude Code's configuration, use \
                 the API-key route, or set {}=1 to proceed anyway.",
                crate::env_auth::ALLOW_ENV_AUTH_VAR
            ))
        }
    }
}

/// Longest wait for `claude --version`.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Run `<exe> --version` with a deadline: no console window, stdin closed, stdout read on a
/// separate thread until its first line (so a descendant that keeps the pipe open cannot
/// block), the process killed when it does not answer in time. Sends nothing to a model.
/// Returns the trimmed first output line.
pub fn probe_version(exe: &Path, timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    ClaudeCode.prepare(&mut cmd);
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
        let mut chunk = [0u8; 512];
        while buf.len() < 4096 && !buf.contains(&b'\n') {
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
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer within {} s", timeout.as_secs()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    };
    let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(500));
    let buf = rx.recv_timeout(left).map_err(|_| "its output did not arrive in time".to_string())?;
    if !status.success() {
        return Err(format!("exited with {status}"));
    }
    let text = String::from_utf8_lossy(&buf);
    Ok(text.lines().next().unwrap_or("").trim().to_string())
}

fn env_path(env: &dyn Fn(&str) -> Option<OsString>, name: &str) -> Option<PathBuf> {
    env(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The Claude Code CLI.
pub struct ClaudeCode;

impl VendorCli for ClaudeCode {
    fn route(&self) -> &'static str {
        ROUTE
    }

    fn caller(&self, session: &str) -> String {
        format!("{ROUTE}:{session}")
    }

    /// The real `claude.exe`. The npm `claude.cmd`/`claude.ps1` shims are not used: they
    /// cannot be started directly and mangle empty arguments.
    fn locate(&self, explicit: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
        if let Some(p) = explicit {
            return Ok(p.to_path_buf());
        }
        if let Some(p) = env_path(env, "RE_SEARCH_CLAUDE").or_else(|| env_path(env, "RESEARCH_CORE_CLAUDE")) {
            return Ok(p);
        }
        let on_path = env("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|dir| dir.join("claude.exe"));
        let npm = env_path(env, "APPDATA").map(|p| p.join("npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe"));
        on_path
            .chain(npm)
            .find(|p| p.is_file())
            .ok_or_else(|| "claude.exe not found: pass --claude or set RE_SEARCH_CLAUDE".to_string())
    }

    fn version(&self, exe: &Path) -> Result<VersionInfo, String> {
        let raw = probe_version(exe, VERSION_TIMEOUT).map_err(|e| format!("`{} --version`: {e}", exe.display()))?;
        let detected = parse_cli_version(&raw).to_string();
        Ok(VersionInfo { raw, detected, tested: TESTED_CLI_VERSION.to_string() })
    }

    fn check_auth(&self, line: &Value, allow_env_auth: bool) -> Result<(), String> {
        check_auth_source(line, allow_env_auth)
    }

    fn env_auth_vars(&self) -> &'static [&'static str] {
        CLAUDE_ENV_AUTH_VARS
    }

    fn tool_name(&self, tool_id: &str) -> String {
        format!("mcp__core__{tool_id}")
    }

    fn write_tool_config(&self, path: &Path, core: &CoreLaunch) -> Result<(), String> {
        let config = json!({ "mcpServers": { "core": {
            "command": core.exe.to_string_lossy(),
            "args": [],
            "env": core.env,
        }}});
        let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
        crate::recorder::write_private_file(path, &bytes)
    }

    fn args(&self, model: &str, tool_config: &Path) -> Vec<String> {
        claude_args(model, &tool_config.to_string_lossy())
    }

    fn args_with_effort(&self, model: &str, tool_config: &Path, effort: Option<&str>) -> Vec<String> {
        let mut out = self.args(model, tool_config);
        crate::vendor::push_effort_flag(&mut out, effort);
        out
    }

    fn prepare(&self, cmd: &mut Command) {
        // Not a nested session of whatever Claude Code may have started re:Search.
        cmd.env_remove("CLAUDECODE").env_remove("CLAUDE_CODE_ENTRYPOINT");
    }

    fn is_handshake(&self, line: &Value) -> bool {
        line["type"] == "system" && line["subtype"] == "init"
    }

    fn check_handshake(&self, line: &Value, expected_tools: &[String]) -> Result<(), String> {
        check_init(line, expected_tools)
    }

    fn handshake_version(&self, line: &Value) -> Option<String> {
        line["claude_code_version"].as_str().map(str::to_string)
    }

    fn is_model_activity(&self, line: &Value) -> bool {
        matches!(line["type"].as_str().unwrap_or(""), "assistant" | "user" | "result")
    }

    fn map_line(&self, line: &Value) -> Vec<(String, Value)> {
        map_line(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn prompt_is_never_an_argument_and_flags_are_pinned() {
        let args = ClaudeCode.args("sonnet", Path::new("C:/d/sessions/s-1/mcp.json"));
        assert_eq!(args[0], "-p");
        assert_eq!(args.iter().filter(|a| *a == "-p").count(), 1);
        let pair = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
        assert_eq!(pair("--model"), Some("sonnet"));
        assert_eq!(pair("--tools"), Some(""));
        assert_eq!(pair("--setting-sources"), Some(""));
        assert_eq!(pair("--permission-prompts"), Some("none"));
        assert_eq!(pair("--allowedTools"), Some("mcp__core__*"));
        assert_eq!(pair("--mcp-config"), Some("C:/d/sessions/s-1/mcp.json"));
        for required in ["--strict-mcp-config", "--disable-slash-commands", "--no-session-persistence", "--verbose"] {
            assert!(args.iter().any(|a| a == required), "{required}");
        }
        assert!(!args.iter().any(|a| a == "--bare" || a.contains("dangerously")), "{args:?}");
        // Every argument is either a known flag or the value right after one.
        assert_eq!(args.len(), 19);
    }

    /// Option name -> (takes a value, the option's help text), from the captured
    /// `claude --help` excerpt of the tested version.
    fn help_options() -> HashMap<String, (bool, String)> {
        let help = include_str!("../fixtures/claude_help_2_1_318.txt");
        let mut out: HashMap<String, (bool, String)> = HashMap::new();
        let mut current: Vec<String> = Vec::new();
        for line in help.lines() {
            if line.starts_with("  -") {
                let body = line.trim_start();
                let (spec, desc) = match body.find("  ") {
                    Some(i) => (&body[..i], body[i..].trim()),
                    None => (body, ""),
                };
                let takes_value = spec.contains('<');
                current = spec
                    .split(", ")
                    .filter_map(|part| part.split_whitespace().next())
                    .map(str::to_string)
                    .collect();
                for name in &current {
                    out.insert(name.clone(), (takes_value, desc.to_string()));
                }
            } else if line.starts_with("   ") {
                for name in &current {
                    if let Some(entry) = out.get_mut(name) {
                        entry.1.push(' ');
                        entry.1.push_str(line.trim());
                    }
                }
            } else {
                current.clear();
            }
        }
        out
    }

    #[test]
    fn every_flag_we_pass_is_in_the_captured_help_of_the_tested_cli() {
        let help = help_options();
        assert!(help.len() >= 13, "{:?}", help.keys());
        let args = ClaudeCode.args("sonnet", Path::new("C:/d/mcp.json"));
        let mut i = 0;
        while i < args.len() {
            let flag = &args[i];
            let (takes_value, desc) = help.get(flag).unwrap_or_else(|| panic!("`{flag}` is not in claude --help {TESTED_CLI_VERSION}"));
            if *takes_value {
                let value = args.get(i + 1).unwrap_or_else(|| panic!("`{flag}` needs a value"));
                assert!(!value.starts_with('-'), "`{flag}` value `{value}`");
                match flag.as_str() {
                    "--output-format" | "--permission-prompts" => {
                        assert!(desc.contains(&format!("\"{value}\"")), "`{flag} {value}` is not a documented choice: {desc}")
                    }
                    "--tools" => assert!(value.is_empty() && desc.contains("Use \"\" to disable all"), "{desc}"),
                    _ => {}
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        // The excerpt is the tested version's, and the prompt may come from stdin.
        assert!(include_str!("../fixtures/claude_help_2_1_318.txt").contains("Claude Code 2.1.318"));
        assert!(help["-p"].1.contains("useful for") && help["-p"].1.contains("pipes"));
    }

    #[test]
    fn api_key_source_other_than_none_is_refused_unless_opted_in() {
        let init = |src: Value| json!({ "type": "system", "subtype": "init", "apiKeySource": src });
        assert!(check_auth_source(&init(json!("none")), false).is_ok());
        assert!(check_auth_source(&json!({ "type": "system" }), false).is_ok(), "older CLIs omit it");
        for src in ["ANTHROPIC_API_KEY", "apiKeyHelper", "/login managed key"] {
            let err = check_auth_source(&init(json!(src)), false).unwrap_err();
            assert!(err.contains("apiKeySource") && err.contains(crate::env_auth::ALLOW_ENV_AUTH_VAR), "{err}");
            assert!(check_auth_source(&init(json!(src)), true).is_ok());
        }
        assert!(check_auth_source(&init(json!(7)), false).is_err());
    }

    #[test]
    fn locate_prefers_explicit_then_env() {
        let map: HashMap<&str, OsString> = [("RE_SEARCH_CLAUDE", OsString::from("C:/x/claude.exe"))].into();
        let env = |n: &str| map.get(n).cloned();
        assert_eq!(ClaudeCode.locate(Some(Path::new("C:/y/c.exe")), &env).unwrap(), PathBuf::from("C:/y/c.exe"));
        assert_eq!(ClaudeCode.locate(None, &env).unwrap(), PathBuf::from("C:/x/claude.exe"));
        let none = |_: &str| None;
        assert!(ClaudeCode.locate(None, &none).unwrap_err().contains("claude.exe not found"));
    }

    #[test]
    fn tool_config_names_only_core_and_carries_its_env() {
        let dir = std::env::temp_dir().join(format!("cli-route-claude-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        let core = CoreLaunch {
            exe: PathBuf::from("C:/bin/core.exe"),
            env: [("CORE_SESSION".to_string(), "s-1".to_string())].into(),
        };
        ClaudeCode.write_tool_config(&path, &core).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"].as_object().unwrap().len(), 1);
        assert_eq!(v["mcpServers"]["core"]["command"], "C:/bin/core.exe");
        assert_eq!(v["mcpServers"]["core"]["env"]["CORE_SESSION"], "s-1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handshake_hooks_follow_check_init() {
        let ok = json!({ "type": "system", "subtype": "init", "tools": ["mcp__core__ledger_note"],
            "mcp_servers": [{ "name": "core", "status": "connected" }], "claude_code_version": "2.1.318" });
        let expected = vec![ClaudeCode.tool_name("ledger_note")];
        assert!(ClaudeCode.is_handshake(&ok));
        assert!(ClaudeCode.check_handshake(&ok, &expected).is_ok());
        assert_eq!(ClaudeCode.handshake_version(&ok).as_deref(), Some("2.1.318"));
        let mut two = ok.clone();
        two["mcp_servers"] = json!([{ "name": "core", "status": "connected" }, { "name": "x", "status": "connected" }]);
        assert!(ClaudeCode.check_handshake(&two, &expected).is_err());
        assert!(ClaudeCode.is_model_activity(&json!({ "type": "assistant" })));
        assert!(!ClaudeCode.is_model_activity(&json!({ "type": "system", "subtype": "hook" })));
    }

    #[test]
    fn effort_flag_is_appended_when_set_and_matches_captured_help() {
        let none = ClaudeCode.args_with_effort("sonnet", Path::new("C:/d/mcp.json"), None);
        assert!(!none.iter().any(|a| a == "--effort"), "{none:?}");

        let def = ClaudeCode.args_with_effort("sonnet", Path::new("C:/d/mcp.json"), Some("default"));
        assert!(!def.iter().any(|a| a == "--effort"), "{def:?}");

        let high = ClaudeCode.args_with_effort("sonnet", Path::new("C:/d/mcp.json"), Some("high"));
        let pos = high.iter().position(|a| a == "--effort").unwrap();
        assert_eq!(high[pos + 1], "high");

        let help = help_options();
        let (takes_value, desc) = help.get("--effort").expect("--effort documented in captured claude --help");
        assert!(*takes_value);
        assert!(desc.contains("high") && desc.contains("max"), "{desc}");
    }
}
