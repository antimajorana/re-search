//! Desktop turns on the Claude subscription route (`provider: "claude-cli"`), driven by a
//! scripted fake CLI (no real CLI, no model): SSE + ledger events, handshake aborts,
//! approval cards over the relay, tool-config lifetime, prompt on stdin, history replay,
//! and stop.
#![cfg(windows)]

mod cli_support;
mod common;

use cli_support::*;
use ledger::Ledger;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const TURN: Duration = Duration::from_secs(60);

fn kinds(data: &std::path::Path, sid: &str) -> Vec<String> {
    Ledger::open(data).unwrap().read_session(sid).unwrap().into_iter().map(|e| e.kind).collect()
}

fn happy_script(h: &CliHarness, final_text: &str) -> Vec<String> {
    vec![
        init_line(&expected_tools(&h.data), core_only()),
        assistant(json!([
            { "type": "thinking", "thinking": "plan" },
            { "type": "text", "text": "I will note it." },
            { "type": "tool_use", "id": "tu1", "name": "mcp__core__ledger_note", "input": { "text": "n" } }
        ])),
        tool_result("tu1", "{\"recorded\":true}"),
        assistant(json!([{ "type": "text", "text": final_text }])),
        result(final_text),
    ]
}

#[test]
fn cli_turn_streams_sse_and_ledger_events_and_cleans_up() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.script(&happy_script(&h, "Final answer: 42."));
    let mut sse = h.sse(&sid);
    let prompt = "SECRET-PROMPT-MARKER compute the answer";
    h.send(&sid, prompt);

    // (a) SSE events of the API route, ending in turn_complete with the final text.
    let done = sse.wait_turn_end(TURN).expect("turn_complete");
    assert_eq!(done["text"], "Final answer: 42.");
    let sk = sse.session_kinds();
    for k in ["session.start", "cli.init", "model.text", "cli.result", "session.end"] {
        assert!(sk.contains(&k.to_string()), "{k} missing from {sk:?}");
    }
    let names: Vec<&str> = sse.seen.iter().map(|(k, _)| k.as_str()).collect();
    assert!(names.contains(&"thinking_delta"), "{names:?}");
    let traces: Vec<&Value> = sse.seen.iter().filter(|(k, _)| k == "tool_trace").map(|(_, v)| v).collect();
    assert_eq!(traces.len(), 2);
    assert_eq!((traces[0]["status"].as_str(), traces[1]["status"].as_str()), (Some("running"), Some("ok")));

    // Ledger: hash chain verifies; route and provenance recorded; prompt blob is the stdin text.
    let ledger = Ledger::open(&h.data).unwrap();
    let report = ledger.verify_session(&sid).unwrap();
    assert!(report.event_count >= 8);
    let events = ledger.read_session(&sid).unwrap();
    let start = &events[0];
    assert_eq!(start.kind, "session.start");
    assert_eq!(start.payload["route"], "claude-code-cli");
    assert_eq!(start.payload["provenance"], "external loop, partial provenance");
    assert_eq!(start.payload["prompt"], prompt);
    assert_eq!(start.payload["history_replayed"], false);
    assert_eq!(start.payload["policy_version"], research_tools::RESEARCH_POLICY_VERSION);
    let blob = ledger.get_blob(start.payload["raw_blob"].as_str().unwrap()).unwrap();
    assert_eq!(String::from_utf8(blob).unwrap(), prompt);
    assert_eq!(events.last().unwrap().kind, "session.end");
    assert_eq!(events.last().unwrap().payload["init_verified"], true);

    // (f) The prompt is not in argv; it arrived on stdin. Flags as pinned.
    let argv = h.fake.argv().expect("fake CLI ran");
    assert!(!argv.iter().any(|a| a.contains("SECRET-PROMPT-MARKER")), "{argv:?}");
    assert_eq!(h.fake.stdin().as_deref(), Some(prompt));
    let pos = argv.iter().position(|a| a == "--permission-prompts").unwrap();
    assert_eq!(argv[pos + 1], "none");
    assert!(argv.iter().any(|a| a == "--no-session-persistence"));
    assert!(!argv.iter().any(|a| a == "--bare"));

    // (e) The per-session mcp.json (it holds CORE_SESSION_TOKEN) existed during the turn,
    // pointed core.exe at the workspace with research tools, and is gone now.
    let mcp_path = h.fake.read("mcp_path.txt").unwrap();
    assert!(mcp_path.ends_with(&format!("{sid}\\mcp.json")) || mcp_path.ends_with(&format!("{sid}/mcp.json")), "{mcp_path}");
    let seen: Value = serde_json::from_str(&h.fake.read("mcp_copy.json").unwrap()).unwrap();
    let env = &seen["mcpServers"]["core"]["env"];
    assert_eq!(env["CORE_SESSION"], sid.as_str());
    assert_eq!(env["CORE_ENABLE_RESEARCH_TOOLS"], "1");
    assert!(env["CORE_WORKSPACE"].as_str().is_some_and(|w| w.ends_with("ws")), "{env}");
    assert_eq!(env["CORE_SESSION_TOKEN"].as_str().map(str::len), Some(64));
    assert!(!std::path::Path::new(&mcp_path).exists(), "mcp.json must be deleted after the turn");
    assert!(!h.data.join("sessions").join(&sid).join(".active.lock").exists());

    // GET exposes the route so the UI can label it, and the messages for the transcript.
    let (st, v) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(st, 200);
    assert_eq!((v["route"].as_str(), v["provenance"].as_str()), (Some("claude-code-cli"), Some("external loop, partial provenance")));
    assert!(v["messages"].as_array().unwrap().iter().any(|m| m["role"] == "user" && m["text"] == prompt));

    // A session on this route refuses an API provider rather than mixing routes.
    let (st, v) = h.post(&format!("/api/sessions/{sid}/messages"), &json!({ "prompt": "x", "provider": "anthropic" }));
    assert_eq!(st, 409, "{v}");
    h.srv.shutdown();
}

#[test]
fn handshake_mismatch_aborts_before_any_assistant_event() {
    clear_env_auth();
    let h = CliHarness::start();
    let mut extra = expected_tools(&h.data);
    extra.push("Bash".into());
    let cases = [
        (init_line(&extra, core_only()), "tool list mismatch"),
        (
            init_line(&expected_tools(&h.data), json!([{ "name": "core", "status": "connected" }, { "name": "x", "status": "connected" }])),
            "MCP servers mismatch",
        ),
    ];
    for (init, why) in cases {
        let sid = h.create_cli_session();
        let tail = assistant(json!([{ "type": "text", "text": "MUST NOT APPEAR" }]));
        h.fake.script(&[init, "@sleep 200".into(), tail, result("x")]);
        let mut sse = h.sse(&sid);
        h.send(&sid, "go");
        let err = sse.wait_turn_end(TURN).expect_err("turn_error");
        assert!(err["error"].as_str().unwrap().contains(why), "{err}");
        let k = kinds(&h.data, &sid);
        assert!(k.contains(&"cli.init".to_string()) && k.contains(&"session.abort".to_string()), "{k:?}");
        for forbidden in ["model.text", "model.tool_use", "cli.result", "session.end"] {
            assert!(!k.contains(&forbidden.to_string()), "{forbidden} recorded: {k:?}");
        }
        assert!(!sse.session_kinds().contains(&"model.text".to_string()));
        let mcp_path = h.fake.read("mcp_path.txt").unwrap();
        assert!(!std::path::Path::new(&mcp_path).exists());
        Ledger::open(&h.data).unwrap().verify_session(&sid).unwrap();
    }
    h.srv.shutdown();
}

#[test]
fn relay_approval_is_a_card_answered_with_the_digest() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    let digest = "ab".repeat(32);
    let request = json!({
        "schema_version": tools::APPROVAL_SCHEMA_VERSION, "session": sid, "call_id": "c-1", "digest": digest,
        "caller": format!("claude-code-cli:{sid}"), "tool_id": "fs_write", "scope": "once", "effect": "write-workspace",
        "canonical_args": "{\"content\":\"new\\n\",\"path\":\"notes.tex\"}", "policy_reason": "ask",
    });
    let msg = json!({ "v": tools::APPROVAL_CHANNEL_VERSION, "token": "@TOKEN@", "request": request, "containment": "none", "contained": false });
    h.fake.write("approval_message.json", &msg.to_string());
    h.fake.write("ack.json", &json!({ "v": tools::APPROVAL_CHANNEL_VERSION, "ack": true }).to_string());
    h.fake.script(&[
        init_line(&expected_tools(&h.data), core_only()),
        assistant(json!([{ "type": "tool_use", "id": "tu1", "name": "mcp__core__fs_write", "input": {} }])),
        "@approval".into(),
        tool_result("tu1", "written"),
        assistant(json!([{ "type": "text", "text": "Done." }])),
        result("Done."),
    ]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "edit notes");

    // (c) The relay's request shows up as the usual approval card.
    let card = sse.wait_for("approval_requested", TURN);
    assert_eq!(card["digest"], digest.as_str());
    assert_eq!(card["tool_id"], "fs_write");
    assert!(card["edit_preview"]["unified_diff"].as_str().unwrap_or("").contains("notes.tex"), "{card}");
    let call_id = card["call_id"].as_str().unwrap().to_string();
    let (_, pending) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(pending["pending_approvals"].as_array().unwrap().len(), 1);

    // A wrong digest is refused; the card stays open. Then the person approves.
    let path = format!("/api/sessions/{sid}/approvals/{call_id}");
    let (st, _) = h.post(&path, &json!({ "digest": "00".repeat(32), "approved": true }));
    assert_eq!(st, 400);
    let (st, v) = h.post(&path, &json!({ "digest": digest, "approved": true }));
    assert_eq!(st, 200, "{v}");

    let done = sse.wait_turn_end(TURN).expect("turn_complete");
    assert_eq!(done["text"], "Done.");
    let reply: Value = serde_json::from_str(h.fake.read("approval_reply.txt").unwrap().trim()).unwrap();
    assert_eq!((reply["approved"].as_bool(), reply["digest"].as_str()), (Some(true), Some(digest.as_str())));

    let ledger = Ledger::open(&h.data).unwrap();
    ledger.verify_session(&sid).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let answered: Vec<&ledger::Event> = events.iter().filter(|e| e.kind == "approval.answered").collect();
    assert_eq!(answered.len(), 1, "recorded once, by the approval card");
    assert_eq!(answered[0].payload["approved"], true);
    assert_eq!(answered[0].payload["digest"], digest.as_str());
    assert!(events.iter().any(|e| e.kind == "approval.delivered"));
    h.srv.shutdown();
}

#[test]
fn later_turns_replay_bounded_history_on_stdin() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.script(&happy_script(&h, "First reply FR-1."));
    let mut sse = h.sse(&sid);
    h.send(&sid, "first question FQ-1");
    sse.wait_turn_end(TURN).expect("first turn");

    // (g) The second turn's stdin carries the transcript of this session, then the message.
    h.fake.script(&happy_script(&h, "Second reply."));
    h.send(&sid, "second question SQ-2");
    sse.wait_turn_end(TURN).expect("second turn");
    let stdin = h.fake.stdin().unwrap();
    // The tags carry a random per-prompt marker (history.rs); find it from the opening tag.
    let marker = stdin.split("<transcript-").nth(1).and_then(|r| r.split('>').next()).unwrap_or_else(|| panic!("{stdin}"));
    assert!(stdin.contains(&format!("</transcript-{marker}>")), "{stdin}");
    assert!(stdin.contains("first question FQ-1") && stdin.contains("First reply FR-1."), "{stdin}");
    assert!(stdin.ends_with(&format!("[New message from the user ({marker})]\nsecond question SQ-2")), "{stdin}");
    assert!(!h.fake.argv().unwrap().iter().any(|a| a.contains("SQ-2") || a.contains("FQ-1")));

    let ledger = Ledger::open(&h.data).unwrap();
    ledger.verify_session(&sid).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let resume = events.iter().find(|e| e.kind == "session.resume").expect("second turn start");
    assert_eq!(resume.payload["history_replayed"], true);
    assert_eq!(resume.payload["prompt"], "second question SQ-2");
    let chars = resume.payload["history_chars"].as_u64().unwrap() as usize;
    assert_eq!(chars, stdin.chars().count() - "second question SQ-2".len());
    assert_eq!(resume.payload["history_turns"], 1);
    assert_eq!(resume.payload["history_turns_dropped"], 0);
    h.srv.shutdown();
}

#[test]
fn stop_kills_the_turn_and_records_abort() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.script(&[init_line(&expected_tools(&h.data), core_only()), "@sleep 60000".into(), result("late")]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "long job");
    // The CLI is running once its handshake has been recorded.
    while !sse.session_kinds().contains(&"cli.init".to_string()) {
        sse.wait_for("session_event", TURN);
    }

    // Stopping is a mutation: the token is required.
    let (st, _, _) = common::raw_http(h.port, "POST", &format!("/api/sessions/{sid}/stop"), &[], b"{}");
    assert_eq!(st, 401);
    let t0 = Instant::now();
    let (st, v) = h.post(&format!("/api/sessions/{sid}/stop"), &json!({}));
    assert_eq!(st, 200, "{v}");
    let err = sse.wait_turn_end(TURN).expect_err("turn_error");
    assert!(err["error"].as_str().unwrap().contains("stopped"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(20), "stop took {:?}", t0.elapsed());

    let events = Ledger::open(&h.data).unwrap().read_session(&sid).unwrap();
    let abort = events.iter().find(|e| e.kind == "session.abort").expect("session.abort");
    assert_eq!(abort.payload["stopped"], true);
    assert!(!events.iter().any(|e| e.kind == "cli.result"));
    let mcp_path = h.fake.read("mcp_path.txt").unwrap();
    assert!(!std::path::Path::new(&mcp_path).exists());
    let (st, _) = h.post(&format!("/api/sessions/{sid}/stop"), &json!({}));
    assert_eq!(st, 409, "nothing left to stop");
    h.srv.shutdown();
}

#[test]
fn server_shutdown_kills_a_running_cli_turn() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.script(&[init_line(&expected_tools(&h.data), core_only()), "@sleep 60000".into(), result("late")]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "long job");
    while !sse.session_kinds().contains(&"cli.init".to_string()) {
        sse.wait_for("session_event", TURN);
    }
    // What closing the app window does: shut the server down.
    h.srv.shutdown();
    let t0 = Instant::now();
    let aborted = loop {
        let k = kinds(&h.data, &sid);
        if k.contains(&"session.abort".to_string()) || t0.elapsed() > Duration::from_secs(20) {
            break k;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(aborted.contains(&"session.abort".to_string()), "{aborted:?}");
    let mcp_path = h.fake.read("mcp_path.txt").unwrap();
    let t1 = Instant::now();
    while std::path::Path::new(&mcp_path).exists() && t1.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!std::path::Path::new(&mcp_path).exists());
}
