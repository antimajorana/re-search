//! Mapping from the Claude Code CLI's `stream-json` lines to the core's own
//! versioned events, and the check of the `system/init` event.
//!
//! Both functions are pure. The CLI's format is not ours and may change
//! between CLI versions, so every raw line is also kept as a blob and each
//! core event carries the hash of the line it came from.

use serde_json::{json, Value};

/// Version of the NDJSON events this module produces.
pub const EVENT_VERSION: u32 = 1;

/// Claude Code CLI version this supervisor was tested against.
pub const TESTED_CLI_VERSION: &str = "2.1.318";

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

/// Check the CLI's `system/init` line: the model must be offered exactly the
/// expected tools (no built-ins, nothing extra, nothing missing) and the
/// `core` MCP server must be connected. Any difference is an error; the
/// supervisor aborts the session on it.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> Vec<String> {
        vec!["mcp__core__sympy_eval".to_string(), "mcp__core__ledger_note".to_string()]
    }

    fn init(tools: Value, servers: Value) -> Value {
        json!({ "type": "system", "subtype": "init", "tools": tools, "mcp_servers": servers, "model": "m" })
    }

    #[test]
    fn init_with_exactly_the_core_tools_passes() {
        let ok = init(
            json!(["mcp__core__ledger_note", "mcp__core__sympy_eval"]),
            json!([{ "name": "core", "status": "connected" }]),
        );
        assert!(check_init(&ok, &expected()).is_ok());
    }

    #[test]
    fn any_difference_fails() {
        let servers = json!([{ "name": "core", "status": "connected" }]);
        let cases = [
            init(json!(["mcp__core__ledger_note", "mcp__core__sympy_eval", "Bash"]), servers.clone()),
            init(json!(["mcp__core__sympy_eval"]), servers.clone()),
            init(json!([]), servers.clone()),
            init(json!(["mcp__core__ledger_note", "mcp__core__sympy_eval"]), json!([{ "name": "core", "status": "failed" }])),
            init(
                json!(["mcp__core__ledger_note", "mcp__core__sympy_eval"]),
                json!([{ "name": "core", "status": "connected" }, { "name": "x", "status": "connected" }]),
            ),
            json!({ "type": "system", "subtype": "init" }),
            json!({ "type": "assistant" }),
        ];
        for case in cases {
            assert!(check_init(&case, &expected()).is_err(), "{case}");
        }
    }

    #[test]
    fn assistant_and_user_lines_map_to_model_events() {
        let line = json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "I will compute it." },
            { "type": "tool_use", "id": "t1", "name": "mcp__core__sympy_eval", "input": { "code": "1+1" } }
        ]}});
        let events = map_line(&line);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "model.text");
        assert_eq!(events[1].1["input"]["code"], "1+1");

        let line = json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "t1", "content": [{ "type": "text", "text": "2" }] }
        ]}});
        let events = map_line(&line);
        assert_eq!(events[0].0, "model.tool_result");
        assert_eq!(events[0].1["text"], "2");
        assert_eq!(events[0].1["is_error"], false);
    }

    /// Real output of Claude Code 2.1.318 (one denied tool call), with ids
    /// and local paths replaced. If a newer CLI changes the format, record a
    /// new fixture next to this one.
    const FIXTURE: &str = include_str!("fixtures/stream_cli_2_1_318.ndjson");

    #[test]
    fn real_cli_stream_maps_as_expected() {
        let lines: Vec<Value> = FIXTURE.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert!(check_init(&lines[0], &expected()).is_ok());
        let kinds: Vec<String> = lines.iter().flat_map(map_line).map(|(k, _)| k).collect();
        assert_eq!(
            kinds,
            ["cli.init", "model.tool_use", "cli.other", "model.tool_result", "model.text", "cli.result"]
        );
        let init = &map_line(&lines[0])[0].1;
        assert_eq!(init["cli_version"], "2.1.318");
        assert_eq!(init["api_key_source"], "none");
        let call = &map_line(&lines[1])[0].1;
        assert_eq!(call["name"], "mcp__core__sympy_eval");
        let result = &map_line(&lines[3])[0].1;
        assert_eq!((result["is_error"].as_bool(), result["text"].as_str()), (Some(true), Some("DENIED: approval: declined")));
    }

    #[test]
    fn real_init_with_one_extra_tool_is_refused() {
        let mut init: Value = serde_json::from_str(FIXTURE.lines().next().unwrap()).unwrap();
        init["tools"].as_array_mut().unwrap().push(json!("Bash"));
        assert!(check_init(&init, &expected()).is_err());
    }

    #[test]
    fn unknown_lines_are_kept_as_other() {
        let events = map_line(&json!({ "type": "tool_progress", "elapsed": 3 }));
        assert_eq!(events, vec![("cli.other".to_string(), json!({ "type": "tool_progress", "subtype": "" }))]);
    }

    #[test]
    fn parses_leading_cli_version_token() {
        assert_eq!(parse_cli_version("2.1.318 (Claude Code)"), "2.1.318");
        assert_eq!(parse_cli_version("2.1.318"), "2.1.318");
        assert_eq!(parse_cli_version("  2.2.0 (Claude Code)\n"), "2.2.0");
        assert_eq!(parse_cli_version(""), "");
    }
}
