//! Desktop turns on the Antigravity route (`provider: "antigravity"`), driven by a scripted
//! fake `agy` (no real CLI, no model, no network): the person's acknowledgement gate, the
//! handshake ("core tools reachable + settings applied"), a core tool call approved on a
//! UI card, built-in tool use recorded as `vendor.builtin_tool`, the workspace
//! `mcp_config.json` lifetime, stop, the model list, and the labels.

mod cli_support;
mod common;

use cli_support::*;
use ledger::Ledger;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

const TURN: Duration = Duration::from_secs(60);
const AGY_LABEL: &str = "Antigravity (Google) · vendor-sandboxed";

fn kinds(data: &std::path::Path, sid: &str) -> Vec<String> {
    Ledger::open(data).unwrap().read_session(sid).unwrap().into_iter().map(|e| e.kind).collect()
}

fn acknowledge(h: &CliHarness) {
    let (st, v) = h.post("/api/connections/antigravity/acknowledge", &json!({}));
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["acknowledged"], true);
}

fn create_agy_session(h: &CliHarness) -> String {
    let (st, v) = h.post("/api/sessions", &json!({ "title": "agy", "provider": "antigravity", "model": "fake-model-a" }));
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["route"], "antigravity");
    v["id"].as_str().unwrap().to_string()
}

fn send_agy(h: &CliHarness, sid: &str, prompt: &str) {
    let (st, v) = h.post(&format!("/api/sessions/{sid}/messages"), &json!({ "prompt": prompt, "provider": "antigravity" }));
    assert_eq!(st, 202, "{v}");
}

#[cfg(windows)]
fn builtins_and_core(h: &CliHarness) -> Vec<String> {
    let mut tools = vec!["run_command".to_string(), "view_file".to_string(), "call_mcp_tool".to_string()];
    tools.extend(agy_expected_tools(&h.data));
    tools
}

fn find<'a>(list: &'a Value, key: &str, id: &str) -> Option<&'a Value> {
    list[key].as_array().unwrap().iter().find(|c| c["id"] == id)
}

#[test]
fn route_requires_current_acknowledgement_before_listing_models_or_starting() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy();
    let (st, created) = h.post("/api/sessions", &json!({ "title": "unbound" }));
    assert_eq!(st, 200);
    let sid = created["id"].as_str().unwrap();
    for settings in ["{}", r#"{"antigravity_ack":{"acknowledged_ms":1,"notice_version":1}}"#] {
        std::fs::write(h.data.join("ui_settings.json"), settings).unwrap();
        let (_, conns) = h.get("/api/connections");
        let c = find(&conns, "connections", "antigravity").unwrap();
        assert_eq!(c["connected"], false);
        assert_eq!(c["acknowledged"], false);
        assert_eq!(c["found"], true);
        assert!(c["version"].is_null());
        assert_eq!(c["label"], AGY_LABEL);
        assert_eq!(c["session_note"], "Antigravity CLI · vendor-sandboxed");
        assert_eq!(c["confinement"], "vendor-sandboxed");
        let warning = c["warning"].as_str().unwrap();
        assert!(warning.contains("terms") && warning.contains("built-in tools"), "{warning}");
        assert_eq!(c["acknowledge_action"], "acknowledge");
        assert_eq!(c["withdraw_action"], "withdraw");

        let (_, models) = h.get("/api/models");
        assert!(find(&models, "providers", "antigravity").is_none(), "{models}");
        let (st, err) = h.post(
            &format!("/api/sessions/{sid}/messages"),
            &json!({ "prompt": "hello", "provider": "antigravity" }),
        );
        assert_eq!(st, 403, "{err}");
        assert!(kinds(&h.data, sid).is_empty());
        let (_, session) = h.get(&format!("/api/sessions/{sid}"));
        assert!(session["meta"]["route"].is_null());
        assert!(h.fake.read("invocations.txt").is_none());
    }

    for action in ["acknowledge", "withdraw"] {
        let path = format!("/api/connections/antigravity/{action}");
        let (st, _, _) = common::raw_http(h.port, "POST", &path, &[], b"{}");
        assert_eq!(st, 401);
    }
    acknowledge(&h);
    let (_, conns) = h.get("/api/connections");
    let c = find(&conns, "connections", "antigravity").unwrap();
    assert_eq!(c["connected"], true);
    assert_eq!(c["acknowledged"], true);
    h.srv.shutdown();
}

#[test]
fn model_list_comes_from_agy_models_and_carries_the_label() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy();
    acknowledge(&h);
    h.fake.write("models.txt", include_str!("../../cli-route/fixtures/agy_models_1_5_13.txt"));
    let (st, models) = h.get("/api/models");
    assert_eq!(st, 200);
    let g = find(&models, "providers", "antigravity").expect("antigravity group").clone();
    assert_eq!(g["label"], AGY_LABEL);
    assert_eq!(g["session_note"], "Antigravity CLI · vendor-sandboxed");
    assert_eq!((g["route"].as_str(), g["source"].as_str(), g["stale"].as_bool()), (Some("antigravity"), Some("live"), Some(false)));
    let list = g["models"].as_array().unwrap();
    assert_eq!(list.len(), 27);
    let default: Vec<&Value> = list.iter().filter(|m| m["default"] == true).collect();
    assert_eq!(default.len(), 1);
    assert_eq!(default[0]["id"], "gemini-3.8-flash-medium");
    assert_eq!(list[0]["label"], "Gemini 4 Neon (High)");
    let (_, again) = h.get("/api/models");
    assert_eq!(find(&again, "providers", "antigravity").unwrap()["source"], "cache");
    h.srv.shutdown();
}

#[cfg(windows)]
#[test]
fn turn_runs_core_tool_with_a_persons_approval_and_cleans_up() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy();
    common::disable_auto_approve(h.port, &h.tok);
    acknowledge(&h);
    let sid = create_agy_session(&h);
    let digest = "cd".repeat(32);
    let request = json!({
        "schema_version": tools::APPROVAL_SCHEMA_VERSION, "session": sid, "call_id": "c-1", "digest": digest,
        "caller": format!("antigravity:{sid}"), "tool_id": "fs_write", "scope": "once", "effect": "write-workspace",
        "canonical_args": "{\"content\":\"new\\n\",\"path\":\"notes.tex\"}", "policy_reason": "ask",
    });
    let msg = json!({ "v": tools::APPROVAL_CHANNEL_VERSION, "token": "@TOKEN@", "request": request, "containment": "none", "contained": false });
    h.fake.write("approval_message.json", &msg.to_string());
    h.fake.write("ack.json", &json!({ "v": tools::APPROVAL_CHANNEL_VERSION, "ack": true }).to_string());
    let params = json!({ "path": "notes.tex", "content": "new\n" });
    h.fake.script(&[
        agy_init(&builtins_and_core(&h), "request-review"),
        agy_step(0, "DONE", "user_input", json!({})),
        agy_tool(1, "ACTIVE", "mcp_core_fs_write", params.clone(), None, None),
        "@approval".into(),
        agy_tool(1, "DONE", "mcp_core_fs_write", params, Some("written"), None),
        agy_step(2, "ACTIVE", "agent_response", json!({ "text_delta": "Done: " })),
        agy_step(2, "DONE", "agent_response", json!({ "text_delta": "notes updated." })),
        agy_result("SUCCESS", "Done: notes updated."),
    ]);
    let mut sse = h.sse(&sid);
    let prompt = "AGY-PROMPT-MARKER edit the notes";
    send_agy(&h, &sid, prompt);

    let card = sse.wait_for("approval_requested", TURN);
    assert_eq!(card["digest"], digest.as_str());
    let call_id = card["call_id"].as_str().unwrap().to_string();
    let (st, v) = h.post(&format!("/api/sessions/{sid}/approvals/{call_id}"), &json!({ "digest": digest, "approved": true }));
    assert_eq!(st, 200, "{v}");
    let done = sse.wait_turn_end(TURN).expect("turn_complete");
    assert_eq!(done["text"], "Done: notes updated.");
    let reply: Value = serde_json::from_str(h.fake.read("approval_reply.txt").unwrap().trim()).unwrap();
    assert_eq!(reply["approved"], true);
    let traces: Vec<&Value> = sse.seen.iter().filter(|(k, _)| k == "tool_trace").map(|(_, v)| v).collect();
    assert_eq!(traces.len(), 2);
    assert_eq!(traces[1]["status"], "ok");

    let ledger = Ledger::open(&h.data).unwrap();
    ledger.verify_session(&sid).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let start = &events[0];
    assert_eq!((start.kind.as_str(), start.payload["route"].as_str()), ("session.start", Some("antigravity")));
    assert_eq!(start.payload["confinement"], "vendor-sandboxed");
    assert_eq!(start.payload["route_label"], AGY_LABEL);
    assert_eq!(start.payload["provider"], "antigravity");
    let flags: Vec<String> = start.payload["cli_flags"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().to_string()).collect();
    let settings_at = flags.iter().position(|f| f == "--settings").unwrap();
    let settings: Value = serde_json::from_str(&flags[settings_at + 1]).unwrap();
    assert_eq!(settings["permissions"]["allow"], json!(["mcp(core/*)"]));
    assert_eq!(settings["permissions"]["deny"].as_array().unwrap().len(), 5);
    let init = events.iter().find(|e| e.kind == "cli.init").unwrap();
    assert_eq!(init.payload["permission_mode"], "request-review");
    assert_eq!(init.payload["builtin_tools"], json!(["run_command", "view_file", "call_mcp_tool"]));
    assert!(events.iter().any(|e| e.kind == "approval.answered" && e.payload["approved"] == true));
    let k: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    for want in ["model.tool_use", "model.tool_result", "model.text", "cli.result", "session.end"] {
        assert!(k.contains(&want), "{want} missing: {k:?}");
    }
    assert!(!k.contains(&"vendor.builtin_tool"));

    // Prompt on stdin, not argv; flags as pinned; the CLI ran in the per-session `agy` folder.
    let argv = h.fake.argv().unwrap();
    assert!(!argv.iter().any(|a| a.contains("AGY-PROMPT-MARKER")), "{argv:?}");
    // One stream-json `user` event carrying the prompt, and the ledger keeps those exact bytes.
    let stdin = h.fake.stdin().unwrap();
    let user: Value = serde_json::from_str(stdin.trim_end()).unwrap();
    assert_eq!(user, json!({ "event": "user", "message": { "content": prompt } }));
    assert_eq!(start.payload["prompt_via"], "stdin (stream-json user event)");
    let blob = ledger.get_blob(start.payload["raw_blob"].as_str().unwrap()).unwrap();
    assert_eq!(blob, stdin.as_bytes());
    assert_eq!(argv, flags);
    let cwd = h.fake.read("cwd.txt").unwrap();
    assert!(cwd.ends_with(&format!("{sid}\\agy")) || cwd.ends_with(&format!("{sid}/agy")), "{cwd}");
    // The workspace mcp_config.json named only core, carried the token, and is gone now.
    let seen: Value = serde_json::from_str(&h.fake.read("mcp_copy.json").unwrap()).unwrap();
    assert_eq!(seen["mcpServers"].as_object().unwrap().len(), 1);
    assert_eq!(seen["mcpServers"]["core"]["env"]["CORE_SESSION_TOKEN"].as_str().map(str::len), Some(64));
    assert_eq!(seen["mcpServers"]["core"]["tools"]["fs_write"]["eager"], true);
    assert_eq!(seen["mcpServers"]["core"]["env"]["CORE_CALLER_ROUTE"], "antigravity");
    assert!(seen["mcpServers"]["core"]["enabledTools"].as_array().unwrap().iter().any(|t| t == "fs_write"));
    let mcp_path = h.fake.read("mcp_path.txt").unwrap();
    assert!(!std::path::Path::new(&mcp_path).exists(), "mcp_config.json must be deleted after the turn");

    let (_, v) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(v["route"], "antigravity");
    // No mixing routes in one session.
    let (st, v) = h.post(&format!("/api/sessions/{sid}/messages"), &json!({ "prompt": "x", "provider": "claude-cli" }));
    assert_eq!(st, 409, "{v}");
    h.srv.shutdown();
}

#[cfg(windows)]
#[test]
fn handshake_without_core_tools_or_settings_aborts() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy();
    acknowledge(&h);
    let only_builtins = vec!["run_command".to_string(), "call_mcp_tool".to_string()];
    let cases = [
        (agy_init(&only_builtins, "request-review"), "core tools not reachable"),
        (agy_init(&builtins_and_core(&h), "always-proceed"), "settings not applied"),
    ];
    for (init, why) in cases {
        let sid = create_agy_session(&h);
        h.fake.script(&[
            init,
            "@sleep 200".into(),
            agy_step(1, "DONE", "agent_response", json!({ "text_delta": "MUST NOT APPEAR" })),
            agy_result("SUCCESS", "x"),
        ]);
        let mut sse = h.sse(&sid);
        send_agy(&h, &sid, "go");
        let err = sse.wait_turn_end(TURN).expect_err("turn_error");
        assert!(err["error"].as_str().unwrap().contains(why), "{err}");
        let k = kinds(&h.data, &sid);
        assert!(k.contains(&"cli.init".to_string()) && k.contains(&"session.abort".to_string()), "{k:?}");
        assert!(!k.iter().any(|x| x == "model.text" || x == "cli.result" || x == "session.end"), "{k:?}");
        assert!(!std::path::Path::new(&h.fake.read("mcp_path.txt").unwrap()).exists());
        Ledger::open(&h.data).unwrap().verify_session(&sid).unwrap();
    }
    h.srv.shutdown();
}

#[cfg(windows)]
#[test]
fn builtin_tool_use_is_recorded_and_shown_as_a_warning() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy();
    acknowledge(&h);
    let sid = create_agy_session(&h);
    let params = json!({ "CommandLine": "dir C:\\" });
    let denied = json!({ "type": "PERMISSION_DENIED", "message": "denied by rule command(*)" });
    h.fake.script(&[
        agy_init(&builtins_and_core(&h), "request-review"),
        agy_tool(1, "ACTIVE", "run_command", params.clone(), None, None),
        agy_tool(1, "ACTIVE", "run_command", params.clone(), None, None),
        agy_tool(1, "DONE", "run_command", params, None, Some(denied)),
        agy_step(2, "DONE", "agent_response", json!({ "text_delta": "I could not run it." })),
        agy_result("SUCCESS", "I could not run it."),
    ]);
    let mut sse = h.sse(&sid);
    send_agy(&h, &sid, "list C:");
    sse.wait_turn_end(TURN).expect("turn_complete");
    let events = Ledger::open(&h.data).unwrap().read_session(&sid).unwrap();
    let builtin: Vec<&ledger::Event> = events.iter().filter(|e| e.kind == "vendor.builtin_tool").collect();
    assert_eq!(builtin.len(), 2, "first sighting and the end, not every repeat");
    assert_eq!(builtin[0].payload["tool_name"], "run_command");
    assert_eq!(builtin[1].payload["denied_or_failed"], true);
    assert!(!events.iter().any(|e| e.kind == "model.tool_use"), "not shown as a re:Search tool call");
    assert!(sse.session_kinds().iter().filter(|k| *k == "vendor.builtin_tool").count() == 2);
    let (_, v) = h.get(&format!("/api/sessions/{sid}"));
    let warnings: Vec<&Value> = v["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "warning").collect();
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0]["text"].as_str().unwrap().contains("run_command") && warnings[0]["text"].as_str().unwrap().contains("Antigravity built-in tool"), "{warnings:?}");
    h.srv.shutdown();
}

#[cfg(windows)]
#[test]
fn withdrawing_acknowledgement_kills_the_turn_and_deletes_the_config() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy();
    acknowledge(&h);
    let sid = create_agy_session(&h);
    h.fake.script(&[agy_init(&builtins_and_core(&h), "request-review"), "@sleep 60000".into(), agy_result("SUCCESS", "late")]);
    let mut sse = h.sse(&sid);
    send_agy(&h, &sid, "long job");
    while !sse.session_kinds().contains(&"cli.init".to_string()) {
        sse.wait_for("session_event", TURN);
    }
    let mcp_path = h.fake.read("mcp_path.txt").unwrap();
    assert!(std::path::Path::new(&mcp_path).exists(), "present while the turn runs");
    let t0 = Instant::now();
    let (st, v) = h.post("/api/connections/antigravity/withdraw", &json!({}));
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["acknowledged"], false);
    let err = sse.wait_turn_end(TURN).expect_err("turn_error");
    assert!(err["error"].as_str().unwrap().contains("stopped"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(20));
    let events = Ledger::open(&h.data).unwrap().read_session(&sid).unwrap();
    assert_eq!(events.iter().find(|e| e.kind == "session.abort").unwrap().payload["stopped"], true);
    assert!(!std::path::Path::new(&mcp_path).exists(), "mcp_config.json must be deleted on stop");
    h.srv.shutdown();
}

#[test]
fn the_fake_rejects_unknown_flags_and_a_prompt_argument() {
    let tmp = tempfile::TempDir::new().unwrap();
    let fake = cli_route::testkit::FakeCli::new_agy(tmp.path(), "agy");
    let run = |args: &[&str]| std::process::Command::new(&fake.exe).args(args).output().unwrap();
    let out = run(&["--output-format", "stream-json", "--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not defined"));
    assert_eq!(run(&["-p", "a prompt in argv"]).status.code(), Some(2));
    assert_eq!(run(&["--input-format", "stream-json", "a prompt in argv"]).status.code(), Some(2));
    assert_eq!(run(&["--settings"]).status.code(), Some(2));
    assert!(fake.argv().is_none());
    assert_eq!(String::from_utf8_lossy(&run(&["--version"]).stdout).trim(), "1.5.14");
}

#[test]
fn version_probe_runs_after_acknowledgement_and_caches() {
    let h = CliHarness::start_agy();
    acknowledge(&h);
    let (_, conns) = h.get("/api/connections");
    let c = find(&conns, "connections", "antigravity").unwrap();
    assert_eq!((c["found"].as_bool(), c["connected"].as_bool()), (Some(true), Some(true)));
    assert_eq!(c["version"], "1.5.14");
    let (_, conns) = h.get("/api/connections");
    assert_eq!(find(&conns, "connections", "antigravity").unwrap()["version"], "1.5.14");
    let log = h.fake.read("invocations.txt").unwrap_or_default();
    assert_eq!(log.lines().filter(|l| *l == "--version").count(), 1, "{log:?}");
    h.srv.shutdown();
}

#[test]
fn env_auth_refusal_blocks_route_unless_explicitly_allowed() {
    for var in ["GEMINI_API_KEY", "AGY_ADC_AUTH"] {
        let h = CliHarness::start_agy_with(|cfg| {
            cfg.connection_env = Some(BTreeMap::from([(var.to_string(), "dummy-agy-secret".to_string())]));
        });
        acknowledge(&h);
        let (_, conns) = h.get("/api/connections");
        let c = find(&conns, "connections", "antigravity").unwrap();
        assert_eq!(c["connected"], false, "{var}: {c}");
        let status = c["status"].as_str().unwrap();
        assert!(status.contains("Blocked:") && status.contains(var) && !status.contains("dummy-agy-secret"), "{status}");
        let (_, models) = h.get("/api/models");
        assert!(find(&models, "providers", "antigravity").is_none(), "{var}: {models}");

        let sid = create_agy_session(&h);
        let mut sse = h.sse(&sid);
        send_agy(&h, &sid, "blocked turn");
        let err = sse.wait_turn_end(TURN).expect_err("turn_error");
        let text = err["error"].as_str().unwrap();
        assert!(text.contains(var) && !text.contains("dummy-agy-secret"), "{text}");
        assert!(h.fake.argv().is_none(), "agy must not start when {var} is set");
        assert!(kinds(&h.data, &sid).is_empty());
        h.srv.shutdown();
    }

    let h = CliHarness::start_agy_with(|cfg| {
        cfg.connection_env = Some(BTreeMap::from([
            ("GEMINI_API_KEY".to_string(), "dummy-agy-secret".to_string()),
            ("RE_SEARCH_ALLOW_AGY_ENV_AUTH".to_string(), "1".to_string()),
        ]));
    });
    acknowledge(&h);
    let (_, conns) = h.get("/api/connections");
    let c = find(&conns, "connections", "antigravity").unwrap();
    assert_eq!(c["connected"], true, "{c}");
    assert!(c["status"].as_str().unwrap().contains("RE_SEARCH_ALLOW_AGY_ENV_AUTH"), "{c}");
    let (_, models) = h.get("/api/models");
    assert!(find(&models, "providers", "antigravity").is_some(), "{models}");
    h.srv.shutdown();
}

#[test]
fn connection_env_cannot_supply_env_auth_opt_in_bypass_for_turns() {
    clear_agy_env_auth();
    let h = CliHarness::start_agy_with(|cfg| {
        cfg.connection_env = Some(BTreeMap::from([
            ("GEMINI_API_KEY".to_string(), "dummy-agy-secret".to_string()),
            ("RE_SEARCH_ALLOW_AGY_ENV_AUTH".to_string(), "1".to_string()),
        ]));
    });
    acknowledge(&h);
    let sid = create_agy_session(&h);
    let mut sse = h.sse(&sid);
    send_agy(&h, &sid, "bypass attempt");
    let err = sse.wait_turn_end(TURN).expect_err("turn_error");
    let text = err["error"].as_str().unwrap();
    assert!(text.contains("GEMINI_API_KEY") && !text.contains("dummy-agy-secret"), "{text}");
    assert!(h.fake.argv().is_none(), "agy must not start when opt-in is only in connection_env");
    assert!(kinds(&h.data, &sid).is_empty(), "nothing recorded");
    h.srv.shutdown();
}

#[test]
fn withdrawal_disables_cached_models_and_turns_but_preserves_session_label() {
    let h = CliHarness::start_agy();
    acknowledge(&h);
    let sid = create_agy_session(&h);
    Ledger::open(&h.data)
        .unwrap()
        .append(&sid, "session.start", 1, &json!({ "route": "antigravity", "prompt": "hi" }))
        .unwrap();

    let (_, models) = h.get("/api/models");
    assert!(find(&models, "providers", "antigravity").is_some());
    let (st, v) = h.post("/api/connections/antigravity/withdraw", &json!({}));
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["acknowledged"], false);
    let (_, conns) = h.get("/api/connections");
    let c = find(&conns, "connections", "antigravity").unwrap();
    assert_eq!(c["connected"], false);
    assert_eq!(c["acknowledged"], false);
    let (_, models) = h.get("/api/models");
    assert!(find(&models, "providers", "antigravity").is_none());

    let (st, sess) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(st, 200);
    assert_eq!(sess["route"], "antigravity");
    assert_eq!(sess["confinement"], "vendor-sandboxed");
    assert_eq!(sess["route_label"], AGY_LABEL);

    let (st, list) = h.get("/api/sessions");
    assert_eq!(st, 200);
    let row = find(&list, "sessions", &sid).expect("session in list");
    assert_eq!(row["confinement"], "vendor-sandboxed");
    assert_eq!(row["route_label"], AGY_LABEL);
    let before = kinds(&h.data, &sid);
    for request in [
        json!({ "prompt": "again" }),
        json!({ "prompt": "again", "provider": "antigravity" }),
    ] {
        let (st, err) = h.post(&format!("/api/sessions/{sid}/messages"), &request);
        assert_eq!(st, 403, "{err}");
    }
    assert_eq!(kinds(&h.data, &sid), before);
    assert!(h.fake.argv().is_none());

    acknowledge(&h);
    let (_, models) = h.get("/api/models");
    assert_eq!(find(&models, "providers", "antigravity").unwrap()["source"], "live");
    h.srv.shutdown();
}

#[test]
fn route_mixing_is_refused_even_after_pre_turn_one_upload() {
    let h = CliHarness::start_agy();
    acknowledge(&h);
    let sid = create_agy_session(&h);
    let (st, v) = h.post(
        &format!("/api/sessions/{sid}/upload"),
        &json!({ "files": [{ "name": "note.txt", "bytes_base64": "aGVsbG8K" }] }),
    );
    assert_eq!(st, 200, "{v}");
    // Simulate a Turn 1 recorded as session.resume (after source.ingested) carrying route.
    Ledger::open(&h.data)
        .unwrap()
        .append(&sid, "session.resume", 1, &json!({ "route": "antigravity", "prompt": "turn 1" }))
        .unwrap();
    for other in ["claude-cli", "anthropic"] {
        let (st, err) = h.post(&format!("/api/sessions/{sid}/messages"), &json!({ "prompt": "mix", "provider": other }));
        assert_eq!(st, 409, "switching to {other} must be 409: {err}");
    }

    // Also verify an API session with pre-Turn-1 upload + session.resume refuses CLI routes.
    let (st, created) = h.post("/api/sessions", &json!({ "title": "api-pre-upload" }));
    assert_eq!(st, 200);
    let sid2 = created["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.post(&format!("/api/sessions/{sid2}/upload"), &json!({ "files": [{ "name": "a.txt", "bytes_base64": "YQ==" }] })).0,
        200
    );
    Ledger::open(&h.data)
        .unwrap()
        .append(&sid2, "session.resume", 1, &json!({ "route": "api", "prompt": "t1" }))
        .unwrap();
    for other in ["antigravity", "claude-cli"] {
        let (st, err) = h.post(&format!("/api/sessions/{sid2}/messages"), &json!({ "prompt": "mix", "provider": other }));
        assert_eq!(st, 409, "switching API session to {other} must be 409: {err}");
    }
    h.srv.shutdown();
}

#[test]
fn settings_validation_and_chatgpt_generic_key_isolation() {
    let h = CliHarness::start_agy();
    let hdrs = [("X-ReSearch-Token", h.tok.as_str()), ("Content-Type", "application/json")];
    let put = |body: Value| {
        let (st, _, b) = common::raw_http(h.port, "PUT", "/api/settings/providers", &hdrs, &serde_json::to_vec(&body).unwrap());
        (st, String::from_utf8_lossy(&b).into_owned())
    };
    let (st, body) = put(json!({ "keys": { "openai": "sk-openai-valid-0001" } }));
    assert_eq!(st, 200, "{body}");
    let providers_path = h.data.join("providers.json");
    let before = std::fs::read(&providers_path).unwrap();

    for bad_req in [
        json!({ "roles": {
            "solver": { "provider": "chatgpt", "model": "gpt-5", "base_url": "https://evil.example" },
            "step_verifier": { "provider": "openai", "model": "gpt-4.1-mini" },
            "strategist": { "provider": "openai", "model": "gpt-4.1-mini" },
            "narrator": { "provider": "openai", "model": "gpt-4.1-mini" }
        }}),
        json!({ "roles": {
            "solver": { "provider": "openai", "model": "gpt-4.1", "base_url": "https://user:pass@example.org/v1" },
            "step_verifier": { "provider": "openai", "model": "gpt-4.1-mini" },
            "strategist": { "provider": "openai", "model": "gpt-4.1-mini" },
            "narrator": { "provider": "openai", "model": "gpt-4.1-mini" }
        }}),
        json!({ "keys": { "chatgpt": 123 } }),
        json!({ "base_urls": { "chatgpt": true } }),
    ] {
        let (st, body) = put(bad_req);
        assert_eq!(st, 400, "must reject invalid settings: {body}");
        assert_eq!(std::fs::read(&providers_path).unwrap(), before, "providers.json must stay untouched on 400");
    }

    // RE_SEARCH_API_KEY + RE_SEARCH_PROVIDER=chatgpt must never leak to ProviderKind::ChatGpt.
    let mut cfg = common::test_cfg(&h.data, None, None, None);
    cfg.connection_env = Some(BTreeMap::from([
        ("RE_SEARCH_PROVIDER".to_string(), "chatgpt".to_string()),
        ("RE_SEARCH_API_KEY".to_string(), "sk-generic-secret-9999".to_string()),
    ]));
    let chatgpt = desktop_server::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let auth = desktop_server::auth::AuthGuard::new(1).unwrap();
    let sse = std::sync::Arc::new(desktop_server::sse::SseHub::new());
    let state = desktop_server::state::DesktopState::new(cfg, auth, sse, chatgpt);
    let store = agent::ProviderKeyStore::default();
    assert!(matches!(
        desktop_server::connections::list_access(&state, agent::ProviderKind::ChatGpt, &store),
        Ok(desktop_server::connections::ListAccess::Off)
    ));
    h.srv.shutdown();
}

