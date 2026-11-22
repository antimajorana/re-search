use super::*;
use std::collections::BTreeMap;

fn expected() -> Vec<String> {
    vec![eager_name("ledger_note"), eager_name("sympy_eval")]
}

fn init(tools: &[&str], mode: &str) -> Value {
    json!({ "event": "init", "conversation_id": "c1", "init": { "cwd": "C:/d/sessions/s-1/agy", "tools": tools, "permission_mode": mode } })
}

#[test]
fn flags_are_pinned_and_the_settings_deny_every_builtin_kind() {
    let args = Antigravity::new().args("gemini-4-neon-high", Path::new("C:/d/.agents/mcp_config.json"));
    let pair = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
    assert_eq!(pair("--input-format"), Some("stream-json"));
    assert!(!args.iter().any(|a| a == "-p" || a == "--print" || a == "--prompt"), "the prompt never goes in argv");
    assert_eq!(pair("--output-format"), Some("stream-json"));
    assert_eq!(pair("--model"), Some("gemini-4-neon-high"));
    assert!(args.iter().any(|a| a == "--sandbox") && args.iter().any(|a| a == "--disable-slash-commands"));
    assert!(!args.iter().any(|a| a.contains("dangerously") || a == "--mode" || a == "--continue" || a == "-c"), "{args:?}");
    assert_eq!(args.len(), 10);
    let settings: Value = serde_json::from_str(pair("--settings").unwrap()).unwrap();
    assert_eq!(settings["agentMode"], "default");
    assert_eq!(settings["toolPermission"], "request-review");
    assert_eq!(settings["enableTerminalSandbox"], true);
    assert_eq!(settings["permissions"]["allow"], json!(["mcp(core/*)"]));
    let deny: Vec<&str> = settings["permissions"]["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(deny, ["command(*)", "read_file(*)", "write_file(*)", "read_url(*)", "execute_url(*)"]);
    assert!(!deny.iter().any(|d| d.starts_with("mcp(")), "a deny on mcp would beat the core allow");
}

/// Option names in the captured `agy --help` (Go flag style: `  --name   description`).
fn help_options() -> HashMap<String, String> {
    include_str!("../fixtures/agy_help_1_5_14.txt")
        .lines()
        .filter(|l| l.starts_with("  -"))
        .filter_map(|l| {
            let body = l.trim_start();
            let (name, desc) = body.split_once(char::is_whitespace)?;
            Some((name.to_string(), desc.trim().to_string()))
        })
        .collect()
}

#[test]
fn every_flag_we_pass_is_in_the_captured_help_of_the_tested_cli() {
    let help = help_options();
    assert!(help.len() >= 20, "{:?}", help.keys());
    for a in agy_args("m").iter().filter(|a| a.starts_with('-')) {
        assert!(help.contains_key(a.as_str()), "`{a}` is not in agy --help {TESTED_CLI_VERSION}");
    }
    assert!(help["--output-format"].contains("stream-json"));
    assert!(help["--settings"].contains("inline"));
    assert!(help["--sandbox"].contains("sandbox"));
    assert!(help["--effort"].contains("xhigh"));
    assert!(include_str!("../fixtures/agy_help_1_5_14.txt").contains("agy --version: 1.5.14"));

    let no_eff = Antigravity::new().args_with_effort("m", Path::new("C:/d/mcp.json"), None);
    assert!(!no_eff.iter().any(|a| a == "--effort"));
    let xhigh = Antigravity::new().args_with_effort("m", Path::new("C:/d/mcp.json"), Some("xhigh"));
    let pos = xhigh.iter().position(|a| a == "--effort").unwrap();
    assert_eq!(xhigh[pos + 1], "xhigh");
}

#[test]
fn init_check_requires_core_tools_and_request_review() {
    let ok = init(&["view_file", "run_command", "mcp_core_ledger_note", "mcp_core_sympy_eval"], "request-review");
    assert!(check_init(&ok, &expected()).is_ok());
    assert_eq!(builtin_tools(&ok), ["view_file", "run_command"]);

    let skip = init(&["mcp_core_ledger_note", "mcp_core_sympy_eval"], "always-proceed");
    assert!(check_init(&skip, &expected()).unwrap_err().contains("settings not applied"));
    let mut missing_mode = ok.clone();
    missing_mode["init"].as_object_mut().unwrap().remove("permission_mode");
    assert!(check_init(&missing_mode, &expected()).unwrap_err().contains("(missing)"));

    let lazy = init(&["call_mcp_tool", "view_file"], "request-review");
    assert!(check_init(&lazy, &expected()).unwrap_err().contains("core tools not reachable"));
    let extra = init(&["mcp_core_ledger_note", "mcp_core_sympy_eval", "mcp_core_shell"], "request-review");
    assert!(check_init(&extra, &expected()).unwrap_err().contains("unexpected core tools"));
    assert!(check_init(&json!({ "event": "init", "init": { "permission_mode": "request-review" } }), &expected()).is_err());
    // Fields at the top level (not under `init`) are not an init event of the documented shape.
    let flat = json!({ "event": "init", "tools": expected(), "permission_mode": "request-review" });
    assert!(check_init(&flat, &expected()).unwrap_err().contains("`init` object"));
    assert!(check_init(&json!({ "event": "result" }), &expected()).is_err());
}

fn step(idx: u64, state: &str, step_type: &str, extra: Value) -> Value {
    let mut v = json!({ "conversation_id": "c1", "step_index": idx, "state": state, "step_type": step_type });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    json!({ "event": "step_update", "step_update": v })
}

#[test]
fn text_deltas_join_into_one_model_text() {
    let a = Antigravity::new();
    let k1 = a.map_line(&step(2, "ACTIVE", "agent_response", json!({ "text_delta": "Hel" })));
    assert_eq!(k1[0].0, "cli.delta");
    a.map_line(&step(2, "ACTIVE", "agent_response", json!({ "text_delta": "lo " })));
    let done = a.map_line(&step(2, "DONE", "agent_response", json!({ "text_delta": "world" })));
    assert_eq!(done, vec![("model.text".to_string(), json!({ "text": "Hello world", "step_index": 2 }))]);
    // A final frame that repeats the whole text is not doubled.
    a.map_line(&step(3, "ACTIVE", "agent_response", json!({ "text_delta": "Ab" })));
    let done = a.map_line(&step(3, "DONE", "agent_response", json!({ "text_delta": "Abc" })));
    assert_eq!(done[0].1["text"], "Abc");
}

#[test]
fn core_tool_steps_become_tool_use_and_result() {
    let a = Antigravity::new();
    let info = json!({ "name": "mcp_core_ledger_note", "parameters": { "text": "n" } });
    let start = a.map_line(&step(4, "ACTIVE", "tool", json!({ "tool_name": "mcp_core_ledger_note", "tool_info": info })));
    assert_eq!(start.len(), 1);
    assert_eq!(start[0].0, "model.tool_use");
    assert_eq!(start[0].1["id"], "step-4");
    assert!(a.map_line(&step(4, "ACTIVE", "tool", json!({ "tool_name": "mcp_core_ledger_note", "tool_info": info }))).is_empty());
    let mut done_info = info.clone();
    done_info["output"] = json!("{\"recorded\":true}");
    let done = a.map_line(&step(4, "DONE", "tool", json!({ "tool_name": "mcp_core_ledger_note", "tool_info": done_info })));
    assert_eq!(done[0].0, "model.tool_result");
    assert_eq!(done[0].1["tool_use_id"], "step-4");
    assert_eq!(done[0].1["is_error"], false);
    // The lazy loader aimed at core counts as core.
    assert!(is_core_tool("call_mcp_tool", &json!({ "ServerName": "core", "ToolName": "x" })));
    assert!(!is_core_tool("call_mcp_tool", &json!({ "ServerName": "github" })));
}

#[test]
fn builtin_tool_steps_are_recorded_as_vendor_builtin_tool() {
    let a = Antigravity::new();
    let info = json!({ "name": "run_command", "parameters": { "CommandLine": "dir" } });
    let start = a.map_line(&step(5, "ACTIVE", "tool", json!({ "tool_name": "run_command", "tool_info": info })));
    assert_eq!(start.len(), 1);
    assert_eq!(start[0].0, "vendor.builtin_tool");
    assert_eq!(start[0].1["tool_name"], "run_command");
    assert_eq!(start[0].1["state"], "ACTIVE");
    assert!(start[0].1["parameters"].as_str().unwrap().contains("dir"));
    let mut denied = info.clone();
    denied["error"] = json!({ "type": "PERMISSION_DENIED", "message": "denied by rule" });
    let done = a.map_line(&step(5, "DONE", "tool", json!({ "tool_name": "run_command", "tool_info": denied })));
    assert_eq!(done[0].0, "vendor.builtin_tool");
    assert_eq!(done[0].1["denied_or_failed"], true);
    assert!(!done.iter().any(|(k, _)| k.starts_with("model.tool")));
}

#[test]
fn result_and_other_events_are_mapped() {
    let a = Antigravity::new();
    let r = a.map_line(&json!({ "event": "result", "result": { "status": "SUCCESS", "response": "42", "num_turns": 1 } }));
    assert_eq!(r[0].0, "cli.result");
    assert_eq!((r[0].1["result"].as_str(), r[0].1["is_error"].as_bool()), (Some("42"), Some(false)));
    let r = a.map_line(&json!({ "event": "result", "result": { "status": "ERROR" } }));
    assert_eq!(r[0].1["is_error"], true);
    assert_eq!(a.map_line(&json!({ "event": "x" }))[0].0, "cli.other");
    let s = a.map_line(&step(0, "DONE", "user_input", json!({})));
    assert_eq!(s[0].0, "cli.step");
    assert!(a.is_model_activity(&step(0, "DONE", "user_input", json!({}))));
    assert!(!a.is_model_activity(&json!({ "event": "init" })));
}

#[test]
fn model_list_parses_the_captured_output() {
    let models = parse_models(include_str!("../fixtures/agy_models_1_5_13.txt"));
    assert_eq!(models.len(), 27);
    assert_eq!(models[0], AgyModel { id: "gemini-4-neon-high".into(), label: "Gemini 4 Neon (High)".into() });
    assert!(models.iter().any(|m| m.id == "claude-sonnet-5-5-medium"));
    let odd = parse_models("--model\tbad\nok-1\t\u{1b}[31mRed\u{7}\nok-1\tdup\n\nno-label\n");
    assert_eq!(odd, vec![
        AgyModel { id: "ok-1".into(), label: "[31mRed".into() },
        AgyModel { id: "no-label".into(), label: "no-label".into() },
    ]);
}

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(*v))).collect();
    move |n: &str| map.get(n).cloned()
}

#[test]
fn env_credentials_are_refused_by_name_unless_opted_in() {
    assert_eq!(check_env_auth(&env_of(&[("PATH", "x")])), Ok(vec![]));
    for name in AGY_ENV_AUTH_VARS {
        let err = check_env_auth(&env_of(&[(name, "secret-value-123")])).unwrap_err();
        assert!(err.contains(name) && err.contains(ALLOW_ENV_AUTH_VAR) && !err.contains("secret-value-123"), "{err}");
        assert_eq!(check_env_auth(&env_of(&[(name, "v"), (ALLOW_ENV_AUTH_VAR, "1")])), Ok(vec![*name]));
    }
}

#[test]
fn locate_prefers_explicit_then_env_then_winget_packages() {
    let tmp = std::env::temp_dir().join(format!("cli-route-agy-locate-{}", std::process::id()));
    let pkg = tmp.join("Microsoft/WinGet/Packages/Google.AntigravityCLI_Microsoft.Winget.Source_x");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("agy.exe"), b"").unwrap();
    let a = Antigravity::new();
    let local = tmp.to_string_lossy().into_owned();
    assert_eq!(a.locate(Some(Path::new("C:/y/agy.exe")), &env_of(&[])).unwrap(), PathBuf::from("C:/y/agy.exe"));
    assert_eq!(a.locate(None, &env_of(&[("RE_SEARCH_AGY", "C:/x/agy.exe")])).unwrap(), PathBuf::from("C:/x/agy.exe"));
    assert_eq!(a.locate(None, &env_of(&[("LOCALAPPDATA", &local)])).unwrap(), pkg.join("agy.exe"));
    assert!(a.locate(None, &env_of(&[])).unwrap_err().contains("agy.exe not found"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn tool_config_names_only_core_with_eager_enabled_tools() {
    let dir = std::env::temp_dir().join(format!("cli-route-agy-cfg-{}", std::process::id()));
    let path = dir.join(TOOL_CONFIG_REL);
    let env: BTreeMap<String, String> = [("CORE_SESSION".to_string(), "s-1".to_string())].into();
    Antigravity::new().write_tool_config(&path, &CoreLaunch { exe: PathBuf::from("C:/bin/core.exe"), env }).unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let servers = v["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let core = &servers["core"];
    assert_eq!(core["command"], "C:/bin/core.exe");
    assert_eq!(core["env"]["CORE_SESSION"], "s-1");
    assert_eq!(core["enabledTools"], json!(["ledger_note", "sympy_eval"]));
    assert_eq!(core["tools"]["sympy_eval"]["eager"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompt_is_one_stream_json_user_event_on_stdin() {
    let a = Antigravity::new();
    let prompt = "line one\n\"quoted\" /not-a-command";
    let bytes = a.stdin_payload(prompt);
    assert_eq!(bytes.last(), Some(&b'\n'));
    assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1, "one NDJSON line");
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v, json!({ "event": "user", "message": { "content": prompt } }));
    assert_eq!(a.prompt_via(), "stdin (stream-json user event)");
    assert_eq!(crate::claude::ClaudeCode.stdin_payload("x"), b"x", "the Claude route is unchanged");
}

#[test]
fn settings_forbid_non_workspace_access_and_have_empty_ask_list() {
    let settings = session_settings();
    assert_eq!(settings["allowNonWorkspaceAccess"], false);
    assert_eq!(settings["permissions"]["ask"], json!([]));
}

#[test]
fn spoofed_mcp_core_prefix_after_handshake_is_recorded_as_builtin_tool() {
    let a = Antigravity::new();
    // Before handshake, falls back to prefix / ServerName check.
    assert!(a.is_expected_core_tool("mcp_core_shell_run", &Value::Null));
    let ok = init(&["view_file", "mcp_core_ledger_note", "mcp_core_sympy_eval"], "request-review");
    a.check_handshake(&ok, &expected()).unwrap();
    // After handshake, requires exact verified membership.
    assert!(a.is_expected_core_tool("mcp_core_ledger_note", &Value::Null));
    assert!(!a.is_expected_core_tool("mcp_core_shell_run", &Value::Null));
    assert!(a.is_expected_core_tool("call_mcp_tool", &json!({ "ServerName": "core", "ToolName": "ledger_note" })));
    assert!(!a.is_expected_core_tool("call_mcp_tool", &json!({ "ServerName": "core", "ToolName": "shell_run" })));

    // Unknown tool spoofing the `mcp_core_` prefix must NOT be classified as model.tool_use.
    let spoof_info = json!({ "name": "mcp_core_shell_run", "parameters": { "cmd": "id" } });
    let evs = a.map_line(&step(10, "ACTIVE", "tool", json!({ "tool_name": "mcp_core_shell_run", "tool_info": spoof_info })));
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].0, "vendor.builtin_tool");
    assert_eq!(evs[0].1["tool_name"], "mcp_core_shell_run");

    // `call_mcp_tool` targeting server `core` with an unknown tool name is also non-core.
    let unknown_lazy = json!({ "name": "call_mcp_tool", "parameters": { "ServerName": "core", "ToolName": "shell_run" } });
    let evs = a.map_line(&step(11, "ACTIVE", "tool", json!({ "tool_name": "call_mcp_tool", "tool_info": unknown_lazy })));
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].0, "vendor.builtin_tool");
    assert_eq!(evs[0].1["tool_name"], "call_mcp_tool");
}

#[test]
fn lazy_core_tool_omitting_parameters_on_done_stays_classified_as_core() {
    let a = Antigravity::new();
    let ok = init(&["mcp_core_ledger_note", "mcp_core_sympy_eval"], "request-review");
    a.check_handshake(&ok, &expected()).unwrap();

    let start_info = json!({
        "name": "call_mcp_tool",
        "parameters": { "ServerName": "core", "ToolName": "ledger_note", "Arguments": { "text": "hi" } }
    });
    let start = a.map_line(&step(12, "ACTIVE", "tool", json!({ "tool_name": "call_mcp_tool", "tool_info": start_info })));
    assert_eq!(start.len(), 1);
    assert_eq!(start[0].0, "model.tool_use");

    // DONE frame omits `parameters` entirely: step 12 must still map to model.tool_result.
    let done_info = json!({ "name": "call_mcp_tool", "output": "{\"recorded\":true}" });
    let done = a.map_line(&step(12, "DONE", "tool", json!({ "tool_name": "call_mcp_tool", "tool_info": done_info })));
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].0, "model.tool_result");
    assert_eq!(done[0].1["tool_use_id"], "step-12");
    assert_eq!(done[0].1["is_error"], false);
}

#[test]
fn non_done_terminal_builtin_states_emit_final_vendor_builtin_tool() {
    let a = Antigravity::new();
    for (idx, final_state) in [(20u64, "FAILED"), (21, "ERROR"), (22, "DENIED"), (23, "cancelled")] {
        let info = json!({ "name": "run_command", "parameters": { "CommandLine": "rm -rf /" } });
        let start = a.map_line(&step(idx, "ACTIVE", "tool", json!({ "tool_name": "run_command", "tool_info": info })));
        assert_eq!(start.len(), 1);
        assert_eq!(start[0].0, "vendor.builtin_tool");

        let mut end_info = info.clone();
        end_info["error"] = json!({ "message": "blocked" });
        let end = a.map_line(&step(idx, final_state, "tool", json!({ "tool_name": "run_command", "tool_info": end_info })));
        assert_eq!(end.len(), 1, "final frame for state {final_state} must emit");
        assert_eq!(end[0].0, "vendor.builtin_tool");
        assert_eq!(end[0].1["state"], final_state);
        assert_eq!(end[0].1["denied_or_failed"], true);
    }
}

#[test]
fn missing_step_index_emits_builtin_tool_for_every_step() {
    let a = Antigravity::new();
    let unindexed = |tool: &str, cmd: &str| {
        json!({
            "event": "step_update",
            "step_update": {
                "conversation_id": "c1",
                "state": "ACTIVE",
                "step_type": "tool",
                "tool_name": tool,
                "tool_info": { "name": tool, "parameters": { "cmd": cmd } }
            }
        })
    };
    let e1 = a.map_line(&unindexed("run_command", "ls"));
    let e2 = a.map_line(&unindexed("write_file", "evil.txt"));
    assert_eq!(e1.len(), 1);
    assert_eq!(e1[0].0, "vendor.builtin_tool");
    assert_eq!(e1[0].1["tool_name"], "run_command");
    assert_eq!(e2.len(), 1, "second unindexed built-in step must not be swallowed");
    assert_eq!(e2[0].0, "vendor.builtin_tool");
    assert_eq!(e2[0].1["tool_name"], "write_file");
}

#[test]
fn non_tool_step_type_with_tool_fields_routes_to_builtin_tool() {
    let a = Antigravity::new();
    let by_name = a.map_line(&step(30, "ACTIVE", "custom_action", json!({ "tool_name": "run_command" })));
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].0, "vendor.builtin_tool");
    assert_eq!(by_name[0].1["tool_name"], "run_command");

    let by_info = a.map_line(&step(
        31,
        "DONE",
        "unexpected_step",
        json!({ "tool_info": { "name": "read_file", "parameters": { "path": "/etc/passwd" } } }),
    ));
    assert_eq!(by_info.len(), 1);
    assert_eq!(by_info[0].0, "vendor.builtin_tool");
    assert_eq!(by_info[0].1["tool_name"], "read_file");
}

#[test]
fn check_init_rejects_non_string_tool_entries() {
    let bad = json!({
        "event": "init",
        "init": {
            "permission_mode": "request-review",
            "tools": ["mcp_core_ledger_note", "mcp_core_sympy_eval", 42]
        }
    });
    let err = check_init(&bad, &expected()).unwrap_err();
    assert!(err.contains("non-string"), "{err}");
}

#[test]
fn valid_model_id_rejects_dot_and_dotdot() {
    assert!(!valid_model_id("."));
    assert!(!valid_model_id(".."));
    assert!(!valid_model_id("___"));
    assert!(valid_model_id("gemini-2.5-pro"));
}

#[test]
fn core_tool_done_with_error_and_single_frame_done_builtin() {
    let a = Antigravity::new();
    let err_info = json!({
        "name": "mcp_core_sympy_eval",
        "parameters": { "code": "1/0" },
        "error": { "message": "division by zero" }
    });
    let evs = a.map_line(&step(40, "DONE", "tool", json!({ "tool_name": "mcp_core_sympy_eval", "tool_info": err_info })));
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0].0, "model.tool_use");
    assert_eq!(evs[1].0, "model.tool_result");
    assert_eq!(evs[1].1["is_error"], true);
    assert_eq!(evs[1].1["text"], "division by zero");

    let builtin_info = json!({ "name": "view_file", "parameters": { "path": "a.txt" } });
    let evs = a.map_line(&step(41, "DONE", "tool", json!({ "tool_name": "view_file", "tool_info": builtin_info })));
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].0, "vendor.builtin_tool");
    assert_eq!(evs[0].1["denied_or_failed"], false);
}

#[cfg(unix)]
#[test]
fn tool_config_file_permissions_are_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("cli-route-agy-perm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join(TOOL_CONFIG_REL);
    let env: BTreeMap<String, String> = [("CORE_SESSION_TOKEN".to_string(), "secret".to_string())].into();
    Antigravity::new().write_tool_config(&path, &CoreLaunch { exe: PathBuf::from("/bin/core"), env }).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
    let _ = std::fs::remove_dir_all(&dir);
}

