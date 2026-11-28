//! Pure prompt framing, pinned CLI flags, handshake checks, JSON-block extraction, and
//! NDJSON stream parsing for Explore CLI exchanges (package X18 Checkpoint 1).

use crate::cli_turn_setup::CliRouteKind;
use agent::{ContentBlock, Message, ModelRequest, Role, TransportError, TransportErrorKind};
use cli_route::antigravity::{
    body as agy_body, check_init as agy_check_init, Antigravity,
    DENIED_KINDS as AGY_DENIED_KINDS, EXPECTED_PERMISSION_MODE as AGY_EXPECTED_PERM_MODE,
};
use cli_route::claude::{check_auth_source as claude_check_auth_source, ClaudeCode};
use cli_route::vendor::{push_effort_flag, VendorCli};
use serde_json::{json, Value};
use std::path::Path;

/// Exchange-only Antigravity `--settings` object: Default agent mode, `request-review`,
/// terminal sandbox on, no non-workspace access, every built-in kind denied, and `allow: []`
/// (plain JSON-block exchanges never expose any MCP server).
pub fn exchange_agy_settings() -> Value {
    json!({
        "agentMode": "",
        "toolPermission": AGY_EXPECTED_PERM_MODE,
        "enableTerminalSandbox": true,
        "allowNonWorkspaceAccess": false,
        "permissions": {
            "allow": [],
            "ask": [],
            "deny": AGY_DENIED_KINDS.iter().map(|k| format!("{k}(*)")).collect::<Vec<_>>(),
        },
    })
}

/// Pinned Claude Code CLI flags for plain JSON-block Explore exchanges (zero tools, zero MCP
/// servers, prompt strictly on stdin).
pub fn exchange_claude_args(model: &str, mcp_config: &Path, effort: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--model",
        model,
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        &mcp_config.to_string_lossy(),
        "--setting-sources",
        "",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--permission-prompts",
        "none",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    push_effort_flag(&mut args, effort);
    args
}

/// Pinned Antigravity CLI flags for plain JSON-block Explore exchanges (prompt strictly on stdin
/// as a single `stream-json` `user` event).
pub fn exchange_agy_args(model: &str, effort: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--model".into(),
        model.into(),
        "--sandbox".into(),
        "--disable-slash-commands".into(),
        "--settings".into(),
        exchange_agy_settings().to_string(),
    ];
    push_effort_flag(&mut args, effort);
    args
}

/// Fail-closed handshake check for a Claude Code JSON-block exchange: zero tools offered,
/// zero MCP servers attached, and subscription billing safety (`apiKeySource == "none"` unless
/// opted in).
pub fn check_claude_exchange_init(init: &Value, allow_env_auth: bool) -> Result<(), String> {
    if init["type"] != "system" || init["subtype"] != "init" {
        return Err("not a system/init event".into());
    }
    let tools = init["tools"]
        .as_array()
        .ok_or("init event has no `tools` list")?;
    if !tools.is_empty() {
        return Err(format!(
            "exchange tool list mismatch: expected zero tools, CLI offered {tools:?}"
        ));
    }
    let servers = init["mcp_servers"]
        .as_array()
        .ok_or("init event has no `mcp_servers` list")?;
    if !servers.is_empty() {
        return Err(format!(
            "exchange MCP servers mismatch: expected none, CLI attached {servers:?}"
        ));
    }
    claude_check_auth_source(init, allow_env_auth)
}

/// Extract a single top-level JSON object (`Value::Object`) from CLI assistant prose.
///
/// Prefers fenced ```` ```json ... ``` ```` blocks (then generic ```` ``` ... ``` ```` blocks),
/// falling back to a standalone JSON object or balanced `{...}` object in short prose.
pub fn extract_json_object(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("CLI returned an empty response with no JSON block".into());
    }

    let mut json_fenced = Vec::new();
    let mut other_fenced = Vec::new();
    collect_fenced_blocks(trimmed, &mut json_fenced, &mut other_fenced);

    let mut last_err: Option<String> = None;
    for block in json_fenced.iter().rev() {
        match parse_json_object(block) {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
    }
    for block in other_fenced.iter().rev() {
        match parse_json_object(block) {
            Ok(v) => return Ok(v),
            Err(e) if last_err.is_none() => last_err = Some(e),
            Err(_) => {}
        }
    }
    if let Ok(v) = parse_json_object(trimmed) {
        return Ok(v);
    }
    for slice in find_balanced_json_objects(trimmed).into_iter().rev() {
        match parse_json_object(slice) {
            Ok(v) => return Ok(v),
            Err(e) if last_err.is_none() => last_err = Some(e),
            Err(_) => {}
        }
    }
    Err(last_err.unwrap_or_else(|| {
        "response did not contain a valid JSON object inside a ```json ... ``` block".into()
    }))
}

fn parse_json_object(raw: &str) -> Result<Value, String> {
    let val: Value = serde_json::from_str(raw.trim()).map_err(|e| format!("invalid JSON: {e}"))?;
    if val.is_object() {
        Ok(val)
    } else {
        Err("expected a top-level JSON object `{...}`, not an array or scalar".into())
    }
}

fn collect_fenced_blocks(text: &str, json_out: &mut Vec<String>, other_out: &mut Vec<String>) {
    let mut in_fence = false;
    let mut is_json = false;
    let mut buf = String::new();
    for line in text.lines() {
        let t = line.trim_start();
        if !in_fence {
            if let Some(rest) = t.strip_prefix("```") {
                let tag = rest.split_whitespace().next().unwrap_or("");
                in_fence = true;
                is_json = tag.eq_ignore_ascii_case("json");
                buf.clear();
            }
        } else if t.starts_with("```") {
            let block = buf.trim().to_string();
            if !block.is_empty() {
                if is_json {
                    json_out.push(block);
                } else {
                    other_out.push(block);
                }
            }
            in_fence = false;
            is_json = false;
            buf.clear();
        } else {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        }
    }
}

fn find_balanced_json_objects(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let (mut depth, mut in_str, mut escaped) = (0usize, false, false);
            let mut j = i;
            while j < bytes.len() {
                let b = bytes[j];
                if in_str {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        in_str = false;
                    }
                } else {
                    match b {
                        b'"' => in_str = true,
                        b'{' => depth += 1,
                        b'}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                out.push(&text[start..=j]);
                                i = j;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                j += 1;
            }
        }
        i += 1;
    }
    out
}

/// Render a single-recorder `ModelRequest` (including repair history) into a plain stdin prompt.
pub(crate) fn render_plain_exchange_prompt(
    req: &ModelRequest,
) -> Result<(String, String), TransportError> {
    if req.tools.len() != 1 {
        return Err(TransportError {
            kind: TransportErrorKind::Config,
            status: None,
            message: format!(
                "Explore CLI exchange requires exactly 1 recorder schema, got {}",
                req.tools.len()
            ),
            raw_body: None,
        });
    }
    let tool = &req.tools[0];
    let schema_json = serde_json::to_string_pretty(&tool.input_schema).map_err(|e| {
        TransportError {
            kind: TransportErrorKind::Config,
            status: None,
            message: format!("failed to serialize recorder schema: {e}"),
            raw_body: None,
        }
    })?;

    let mut out = String::new();
    if let Some(sys) = req.system.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(sys);
        out.push_str("\n\n");
    }
    out.push_str(&format!(
        "OUTPUT FORMAT (CLI JSON-block exchange):\n\
         - Do NOT invoke any tools. You have no tools in this exchange.\n\
         - Respond with ONLY a single valid JSON object inside a ```json ... ``` fenced code block \
           matching the `{}` schema below.\n\
         - Description: {}\n\
         - JSON Schema:\n```json\n{schema_json}\n```\n\n",
        tool.name, tool.description
    ));
    for (idx, msg) in req.messages.iter().enumerate() {
        render_message(&mut out, msg, idx, &tool.name);
    }
    Ok((tool.name.clone(), out))
}

fn render_message(out: &mut String, msg: &Message, idx: usize, recorder: &str) {
    match msg.role {
        Role::User => {
            if idx > 0 {
                out.push_str("\n--- Repair feedback from validator ---\n");
            }
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        out.push_str(text);
                        out.push('\n');
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        out.push_str(content);
                        out.push_str(&format!(
                            "\nRespond with ONLY the corrected ```json ... ``` object for `{recorder}`.\n"
                        ));
                    }
                    _ => {}
                }
            }
        }
        Role::Assistant => {
            out.push_str("\n--- Previous assistant output ---\n");
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        out.push_str(text);
                        out.push('\n');
                    }
                    ContentBlock::ToolUse { input, .. } => {
                        let pretty = serde_json::to_string_pretty(input).unwrap_or_default();
                        out.push_str("```json\n");
                        out.push_str(&pretty);
                        out.push_str("\n```\n");
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Incremental NDJSON stream state for one CLI exchange attempt.
pub(crate) struct ExchangeStreamState {
    pub init_ok: bool,
    pub result_received: bool,
    pub abort: Option<String>,
    pub observed_model: String,
    pub usage: Value,
    pub last_text: String,
    pub result_text: String,
    pub unexpected_tool: Option<(String, String, Value)>,
    pub raw_lines: Vec<String>,
}

impl ExchangeStreamState {
    pub fn new(requested_model: &str) -> Self {
        Self {
            init_ok: false,
            result_received: false,
            abort: None,
            observed_model: requested_model.to_string(),
            usage: Value::Null,
            last_text: String::new(),
            result_text: String::new(),
            unexpected_tool: None,
            raw_lines: Vec::new(),
        }
    }

    pub fn final_text(&self) -> &str {
        if self.last_text.is_empty() {
            &self.result_text
        } else {
            &self.last_text
        }
    }

    pub fn feed_line(
        &mut self,
        kind: CliRouteKind,
        agy_vendor: &Antigravity,
        allow_env_auth: bool,
        trimmed: String,
    ) {
        self.raw_lines.push(trimmed.clone());
        let Ok(val) = serde_json::from_str::<Value>(&trimmed) else {
            return;
        };
        match kind {
            CliRouteKind::Claude => self.feed_claude(&val, allow_env_auth),
            CliRouteKind::Antigravity => self.feed_agy(agy_vendor, &val),
        }
    }

    fn feed_claude(&mut self, val: &Value, allow_env_auth: bool) {
        if ClaudeCode.is_handshake(val) {
            match check_claude_exchange_init(val, allow_env_auth) {
                Ok(()) => {
                    self.init_ok = true;
                    if let Some(m) = val["model"].as_str().filter(|s| !s.is_empty()) {
                        self.observed_model = m.to_string();
                    }
                }
                Err(e) => {
                    self.abort = Some(e);
                    return;
                }
            }
        } else if !self.init_ok && ClaudeCode.is_model_activity(val) {
            self.abort =
                Some("Claude Code emitted model activity before verified system/init".into());
            return;
        }
        for (ev_kind, data) in ClaudeCode.map_line(val) {
            match ev_kind.as_str() {
                "model.text" => {
                    if let Some(t) = data["text"].as_str() {
                        self.last_text = t.to_string();
                    }
                }
                "model.tool_use" if self.unexpected_tool.is_none() => {
                    self.unexpected_tool = Some((
                        data["id"].as_str().unwrap_or("cli-tool").to_string(),
                        data["name"].as_str().unwrap_or("unknown").to_string(),
                        data["input"].clone(),
                    ));
                }
                "cli.result" => {
                    self.result_received = true;
                    if let Some(t) = data["result"].as_str() {
                        self.result_text = t.to_string();
                    }
                    if !data["usage"].is_null() {
                        self.usage = data["usage"].clone();
                    }
                    if data["is_error"].as_bool() == Some(true) {
                        self.abort = Some(
                            data["result"]
                                .as_str()
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or("Claude Code reported a failed exchange")
                                .to_string(),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn feed_agy(&mut self, agy_vendor: &Antigravity, val: &Value) {
        if agy_vendor.is_handshake(val) {
            match agy_check_init(val, &[]) {
                Ok(()) => {
                    self.init_ok = true;
                    let init = agy_body(val, "init");
                    if let Some(m) = init["model"].as_str().filter(|s| !s.is_empty()) {
                        self.observed_model = m.to_string();
                    }
                }
                Err(e) => {
                    self.abort = Some(e);
                    return;
                }
            }
        } else if !self.init_ok && agy_vendor.is_model_activity(val) {
            self.abort = Some("Antigravity CLI emitted model activity before verified init".into());
            return;
        }
        for (ev_kind, data) in agy_vendor.map_line(val) {
            match ev_kind.as_str() {
                "model.text" => {
                    if let Some(t) = data["text"].as_str() {
                        self.last_text = t.to_string();
                    }
                }
                "model.tool_use" if self.unexpected_tool.is_none() => {
                    self.unexpected_tool = Some((
                        data["id"].as_str().unwrap_or("agy-tool").to_string(),
                        data["name"].as_str().unwrap_or("unknown").to_string(),
                        data["input"].clone(),
                    ));
                }
                "vendor.builtin_tool" if self.unexpected_tool.is_none() => {
                    let st = data["state"].as_str().unwrap_or("");
                    let completed =
                        matches!(st.to_ascii_uppercase().as_str(), "DONE" | "COMPLETED");
                    if completed && data["denied_or_failed"].as_bool() == Some(false) {
                        self.unexpected_tool = Some((
                            format!("agy-builtin-{}", data["step_index"].as_u64().unwrap_or(0)),
                            data["tool_name"]
                                .as_str()
                                .unwrap_or("vendor_builtin")
                                .to_string(),
                            json!({ "parameters": data["parameters"] }),
                        ));
                    }
                }
                "cli.result" => {
                    self.result_received = true;
                    if let Some(t) = data["result"].as_str() {
                        self.result_text = t.to_string();
                    }
                    if !data["usage"].is_null() {
                        self.usage = data["usage"].clone();
                    }
                    if data["is_error"].as_bool() == Some(true) {
                        self.abort = Some(
                            data["error"]
                                .as_str()
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or("Antigravity CLI reported a failed exchange")
                                .to_string(),
                        );
                    }
                }
                _ => {}
            }
        }
    }
}
