//! Guards on the Claude subscription route (`claude-cli`), driven by the scripted fake
//! CLI (no real CLI, no model): the CLI's own `apiKeySource` report, a stop right after
//! the 202, a bounded `--version`, provisional route binding, stale `mcp.json` sweeps,
//! undecodable output lines, and the fake's own option check.
#![cfg(windows)]

mod cli_support;
mod common;

use cli_support::*;
use ledger::Ledger;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const TURN: Duration = Duration::from_secs(60);

fn kinds(data: &Path, sid: &str) -> Vec<String> {
    Ledger::open(data).unwrap().read_session(sid).unwrap().into_iter().map(|e| e.kind).collect()
}

#[test]
fn api_key_source_other_than_none_aborts_the_turn() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    let mut init: Value = serde_json::from_str(&init_line(&expected_tools(&h.data), core_only())).unwrap();
    init["apiKeySource"] = json!("apiKeyHelper");
    let tail = assistant(json!([{ "type": "text", "text": "MUST NOT APPEAR" }]));
    h.fake.script(&[init.to_string(), "@sleep 200".into(), tail, result("x")]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "go");
    let err = sse.wait_turn_end(TURN).expect_err("turn_error");
    let text = err["error"].as_str().unwrap();
    assert!(text.contains("apiKeySource") && text.contains("apiKeyHelper"), "{text}");
    assert!(text.contains("RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH"), "{text}");
    let events = Ledger::open(&h.data).unwrap().read_session(&sid).unwrap();
    let k: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert!(k.contains(&"cli.init"), "{k:?}");
    let abort = events.iter().find(|e| e.kind == "session.abort").expect("session.abort");
    assert!(abort.payload["reason"].as_str().unwrap().contains("apiKeySource"), "{abort:?}");
    for forbidden in ["model.text", "cli.result", "session.end"] {
        assert!(!k.contains(&forbidden), "{forbidden} recorded: {k:?}");
    }
    assert!(!Path::new(&h.fake.read("mcp_path.txt").unwrap()).exists());
    Ledger::open(&h.data).unwrap().verify_session(&sid).unwrap();
    h.srv.shutdown();
}

#[test]
fn a_stop_right_after_the_202_stops_the_turn_before_it_starts() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.write("version_sleep_ms.txt", "1500");
    h.fake.script(&[init_line(&expected_tools(&h.data), core_only()), "@sleep 60000".into(), result("late")]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "long job");
    // Immediately: the handle is registered before the turn's thread exists.
    let (st, v) = h.post(&format!("/api/sessions/{sid}/stop"), &json!({}));
    assert_eq!(st, 200, "{v}");
    let err = sse.wait_turn_end(TURN).expect_err("turn_error");
    assert!(err["error"].as_str().unwrap().contains("stopped"), "{err}");
    assert!(h.fake.argv().is_none(), "the CLI must not have started a turn");
    assert!(kinds(&h.data, &sid).is_empty(), "nothing started, nothing recorded");
    let (st, v) = h.post(&format!("/api/sessions/{sid}/stop"), &json!({}));
    assert_eq!(st, 409, "{v}");
    h.srv.shutdown();
}

#[test]
fn a_hanging_version_check_is_bounded() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.write("version_sleep_ms.txt", "30000");
    h.fake.script(&[init_line(&expected_tools(&h.data), core_only()), result("x")]);
    let mut sse = h.sse(&sid);
    let t0 = Instant::now();
    h.send(&sid, "hello");
    let err = sse.wait_turn_end(TURN).expect_err("turn_error");
    assert!(err["error"].as_str().unwrap().contains("no answer within 5 s"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(20), "{:?}", t0.elapsed());
    assert!(h.fake.argv().is_none());
    assert!(kinds(&h.data, &sid).is_empty());
    h.srv.shutdown();
}

#[test]
fn a_refused_first_turn_does_not_bind_the_session() {
    clear_env_auth();
    let missing_core = std::env::temp_dir().join(format!("no-such-core-{}", tools::canon::random_hex(6))).join("core.exe");
    // The API turn below may only reach a loopback port nothing listens on.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let h = CliHarness::start_with(|cfg| {
        cfg.core_exe = Some(missing_core.clone());
        cfg.api_base_override = Some(closed.clone());
    });
    let sid = h.create_cli_session();
    let mut sse = h.sse(&sid);
    h.send(&sid, "first");
    let err = sse.wait_turn_end(TURN).expect_err("refused");
    assert!(err["error"].as_str().unwrap().contains("not found"), "{err}");
    assert!(kinds(&h.data, &sid).is_empty(), "refused before session.start");

    // Same session, an API model: accepted, the provisional binding is dropped.
    let (st, v) = h.post(
        &format!("/api/sessions/{sid}/messages"),
        &json!({ "prompt": "second", "provider": "anthropic", "model": "claude-haiku-4-5" }),
    );
    assert_eq!(st, 202, "{v}");
    let _ = sse.wait_turn_end(TURN);
    let (st, v) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(st, 200);
    assert_ne!(v["route"], "claude-code-cli", "{v}");
    assert!(h.fake.argv().is_none());
    h.srv.shutdown();
}

#[test]
fn stale_mcp_json_is_removed_at_server_start_and_session_open_but_not_under_a_live_turn() {
    clear_env_auth();
    let stale_at_start = std::cell::RefCell::new(None);
    let h = CliHarness::start_with(|cfg| {
        // Left by a killed earlier run: no lock, and one whose pid is gone.
        for (id, lock) in [("s-stale0001", None), ("s-stale0002", Some(u32::MAX.to_string()))] {
            let dir = cfg.data_dir.join("sessions").join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("mcp.json"), "{\"CORE_SESSION_TOKEN\":\"old\"}").unwrap();
            if let Some(l) = lock {
                std::fs::write(dir.join(".active.lock"), l).unwrap();
            }
        }
        *stale_at_start.borrow_mut() = Some(cfg.data_dir.join("sessions"));
    });
    let sessions = stale_at_start.borrow().clone().unwrap();
    assert!(!sessions.join("s-stale0001").join("mcp.json").exists());
    assert!(!sessions.join("s-stale0002").join("mcp.json").exists());

    // Opening a session removes one left behind since.
    let sid = h.create_cli_session();
    let cfg_path = sessions.join(&sid).join("mcp.json");
    std::fs::write(&cfg_path, "{}").unwrap();
    std::fs::write(sessions.join(&sid).join(".active.lock"), std::process::id().to_string()).unwrap();
    let (st, _) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(st, 200);
    assert!(!cfg_path.exists(), "stale mcp.json removed when the session is opened");

    // A running turn's own config stays.
    h.fake.script(&[init_line(&expected_tools(&h.data), core_only()), "@sleep 60000".into(), result("late")]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "long job");
    while !sse.session_kinds().contains(&"cli.init".to_string()) {
        sse.wait_for("session_event", TURN);
    }
    let (st, _) = h.get(&format!("/api/sessions/{sid}"));
    assert_eq!(st, 200);
    assert!(cfg_path.exists(), "the live turn's mcp.json must not be removed");
    let (st, _) = h.post(&format!("/api/sessions/{sid}/stop"), &json!({}));
    assert_eq!(st, 200);
    sse.wait_turn_end(TURN).expect_err("stopped");
    assert!(!cfg_path.exists());
    h.srv.shutdown();
}

#[test]
fn an_undecodable_output_line_is_kept_as_a_blob() {
    clear_env_auth();
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.script(&[
        init_line(&expected_tools(&h.data), core_only()),
        "@bytes 7b2278223a22fffe227d".into(),
        assistant(json!([{ "type": "text", "text": "Fine." }])),
        result("Fine."),
    ]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "go");
    let done = sse.wait_turn_end(TURN).expect("turn_complete");
    assert_eq!(done["text"], "Fine.");
    let ledger = Ledger::open(&h.data).unwrap();
    ledger.verify_session(&sid).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let unparsed = events.iter().find(|e| e.kind == "cli.unparsed").expect("cli.unparsed");
    assert_eq!(unparsed.payload["reason"], "not valid UTF-8");
    let blob = ledger.get_blob(unparsed.payload["raw_blob"].as_str().unwrap()).unwrap();
    assert_eq!(blob, vec![0x7b, 0x22, 0x78, 0x22, 0x3a, 0x22, 0xff, 0xfe, 0x22, 0x7d]);
    assert_eq!(events.last().unwrap().kind, "session.end");
    h.srv.shutdown();
}

#[test]
fn the_fake_cli_rejects_unknown_options() {
    let tmp = tempfile::TempDir::new().unwrap();
    let fake = cli_route::testkit::FakeCli::new(tmp.path(), "fake");
    let out = std::process::Command::new(&fake.exe)
        .args(["-p", "--permision-prompts", "none"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option '--permision-prompts'"));
    assert!(fake.argv().is_none());
}
