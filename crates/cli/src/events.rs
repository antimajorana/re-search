//! Mapping from the Claude Code CLI's `stream-json` lines to the core's own
//! versioned events, and the check of the `system/init` event.
//!
//! The implementation moved to `cli_route::claude` so the desktop server uses the same
//! code; this module re-exports it and keeps the fixture-based tests of the real CLI
//! stream next to the console supervisor.

#[cfg(test)]
pub use cli_route::claude::{check_init, map_line, parse_cli_version};
pub use cli_route::EVENT_VERSION;
#[cfg(test)]
use serde_json::{json, Value};
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
