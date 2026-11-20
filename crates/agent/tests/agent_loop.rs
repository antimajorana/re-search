//! End-to-end integration tests for `run_agent_loop` over
//! `AnthropicTransport` + in-process `127.0.0.1:0` mock server + real
//! `tools::Router` + real `ledger::Ledger`.

mod support;

use agent::{run_agent_loop, EventSink, NoopSink, SessionOutcome};
use ledger::{AppendedEvent, Ledger};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use support::{
    assert_secret_absent, default_cfg, make_router, make_transport, ok_msg, MockApprover, MockEpoch, MockServer,
    RecordingSink, TempDir,
};

fn independent_sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn multi_turn_ledger_note_records_exact_sequence_and_verifies() {
    let tmp = TempDir::new("note");
    let secret = "sk-agent-note-secret-999";
    let session = "s-note-1";
    let approver = Arc::new(MockApprover::new(true));
    let epoch = Arc::new(MockEpoch::new(1, false));
    let runs = Arc::new(AtomicUsize::new(0));
    let router = make_router(&tmp.path, session, approver.clone(), epoch, runs);
    let ledger = Ledger::open(&tmp.path).unwrap();

    let server = MockServer::start(vec![
        ok_msg(
            "msg_1",
            "tool_use",
            json!([
                { "type": "tool_use", "id": "tu_1", "name": "ledger_note", "input": { "text": "hello note" } }
            ]),
        ),
        ok_msg(
            "msg_2",
            "end_turn",
            json!([{ "type": "text", "text": "Done recording note." }]),
        ),
    ]);
    let transport = make_transport(server.base_url.clone(), secret);
    let mut sink = RecordingSink::new();
    let cfg = default_cfg("Take a note.", false);

    let outcome = run_agent_loop(&transport, &router, &ledger, &cfg, &mut sink).unwrap();
    assert_eq!(outcome, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
    assert_eq!(approver.asked(), 0);

    let wire_reqs = server.requests_received();
    assert_eq!(wire_reqs.len(), 2);
    server.finish();

    let events = ledger.read_session(session).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "session.start",
            "model.request",
            "model.response",
            "model.tool_use",
            "tool.request",
            "policy.decision",
            "note",
            "tool.result",
            "model.tool_result",
            "model.request",
            "model.response",
            "model.text",
            "session.end",
        ]
    );

    assert_eq!(events[0].payload["provenance"], "in-core loop, full provenance");
    assert_eq!(events[0].payload["api_key_env"], "RE_SEARCH_API_KEY");
    assert_eq!(events[1].payload["raw_blob"], independent_sha256_hex(&wire_reqs[0]));
    assert_eq!(events[9].payload["raw_blob"], independent_sha256_hex(&wire_reqs[1]));
    assert_eq!(events[8].payload["tool_use_id"], "tu_1");
    assert_eq!(events[8].payload["is_error"], false);
    assert_eq!(events[12].payload["outcome"], "completed");

    let s_rep = ledger.verify_session(session).unwrap();
    assert_eq!(s_rep.event_count, events.len() as u64);
    let all_rep = ledger.verify_all().unwrap();
    assert_eq!(all_rep.unreferenced_blobs, 0);
    assert_secret_absent(&tmp.path, session, &sink, secret);
}

#[test]
fn contained_sympy_eval_reuses_approval_per_epoch_and_reprompts_on_new_epoch() {
    let tmp = TempDir::new("contained");
    let secret = "sk-agent-contained-secret";
    let session = "s-contained";
    let approver = Arc::new(MockApprover::new(true));
    let epoch = Arc::new(MockEpoch::new(1, true));
    let runs = Arc::new(AtomicUsize::new(0));
    let router = make_router(&tmp.path, session, approver.clone(), epoch.clone(), runs.clone());
    let ledger = Ledger::open(&tmp.path).unwrap();

    // Two sympy_eval calls on live epoch 1 -> asks once, reuses once.
    let server1 = MockServer::start(vec![
        ok_msg(
            "m1",
            "tool_use",
            json!([{ "type": "tool_use", "id": "t1", "name": "sympy_eval", "input": { "code": "1+1" } }]),
        ),
        ok_msg(
            "m2",
            "tool_use",
            json!([{ "type": "tool_use", "id": "t2", "name": "sympy_eval", "input": { "code": "2+2" } }]),
        ),
        ok_msg("m3", "end_turn", json!([{ "type": "text", "text": "4" }])),
    ]);
    let transport1 = make_transport(server1.base_url.clone(), secret);
    let mut sink = RecordingSink::new();
    let outcome = run_agent_loop(&transport1, &router, &ledger, &default_cfg("eval", true), &mut sink).unwrap();
    assert_eq!(outcome, SessionOutcome::Completed { turns: 3, stop_reason: "end_turn".into() });
    server1.finish();

    assert_eq!(approver.asked(), 1);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    let events = ledger.read_session(session).unwrap();
    assert!(events.iter().any(|e| e.kind == "approval.session_bound"));
    assert!(events.iter().any(|e| e.kind == "approval.session_reused"));

    // Epoch 1 dies and epoch 2 starts -> next sympy_eval call through same Router must re-prompt.
    epoch.set_epoch(0);
    epoch.set_epoch(2);
    let server2 = MockServer::start(vec![
        ok_msg(
            "m4",
            "tool_use",
            json!([{ "type": "tool_use", "id": "t3", "name": "sympy_eval", "input": { "code": "3+3" } }]),
        ),
        ok_msg("m5", "end_turn", json!([{ "type": "text", "text": "6" }])),
    ]);
    let transport2 = make_transport(server2.base_url.clone(), secret);
    let outcome2 = run_agent_loop(&transport2, &router, &ledger, &default_cfg("eval2", true), &mut NoopSink).unwrap();
    assert_eq!(outcome2, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
    server2.finish();
    assert_eq!(approver.asked(), 2);
    assert_eq!(runs.load(Ordering::SeqCst), 3);
}

#[test]
fn uncontained_sympy_eval_asks_every_call_and_declined_never_runs() {
    // Part A: uncontained asks twice for two calls, zero session binding.
    {
        let tmp = TempDir::new("uncontained");
        let session = "s-uncontained";
        let approver = Arc::new(MockApprover::new(true));
        let epoch = Arc::new(MockEpoch::new(1, false));
        let runs = Arc::new(AtomicUsize::new(0));
        let router = make_router(&tmp.path, session, approver.clone(), epoch, runs.clone());
        let ledger = Ledger::open(&tmp.path).unwrap();

        let server = MockServer::start(vec![
            ok_msg(
                "m1",
                "tool_use",
                json!([{ "type": "tool_use", "id": "t1", "name": "sympy_eval", "input": { "code": "1+1" } }]),
            ),
            ok_msg(
                "m2",
                "tool_use",
                json!([{ "type": "tool_use", "id": "t2", "name": "sympy_eval", "input": { "code": "2+2" } }]),
            ),
            ok_msg("m3", "end_turn", json!([{ "type": "text", "text": "done" }])),
        ]);
        let transport = make_transport(server.base_url.clone(), "sk-uncontained");
        let out = run_agent_loop(&transport, &router, &ledger, &default_cfg("eval", false), &mut NoopSink).unwrap();
        assert_eq!(out, SessionOutcome::Completed { turns: 3, stop_reason: "end_turn".into() });
        server.finish();
        assert_eq!(approver.asked(), 2);
        assert_eq!(runs.load(Ordering::SeqCst), 2);

        let events = ledger.read_session(session).unwrap();
        assert!(!events.iter().any(|e| e.kind == "approval.session_bound"));
        assert!(!events.iter().any(|e| e.kind == "approval.session_reused"));
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }

    // Part B: declined approval -> handler never runs, model receives DENIED tool_result.
    {
        let tmp = TempDir::new("declined");
        let session = "s-declined";
        let approver = Arc::new(MockApprover::new(false));
        let epoch = Arc::new(MockEpoch::new(1, true));
        let runs = Arc::new(AtomicUsize::new(0));
        let router = make_router(&tmp.path, session, approver, epoch, runs.clone());
        let ledger = Ledger::open(&tmp.path).unwrap();

        let server = MockServer::start(vec![
            ok_msg(
                "m1",
                "tool_use",
                json!([{ "type": "tool_use", "id": "t1", "name": "sympy_eval", "input": { "code": "1+1" } }]),
            ),
            ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "ok, skipped" }])),
        ]);
        let transport = make_transport(server.base_url.clone(), "sk-declined");
        let out = run_agent_loop(&transport, &router, &ledger, &default_cfg("eval", true), &mut NoopSink).unwrap();
        assert_eq!(out, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
        let reqs = server.requests_received();
        server.finish();

        assert_eq!(runs.load(Ordering::SeqCst), 0);
        let events = ledger.read_session(session).unwrap();
        assert!(events.iter().any(|e| e.kind == "approval.denied"));
        assert!(events.iter().any(|e| e.kind == "tool.denied"));

        let turn2_body: Value = serde_json::from_slice(&reqs[1]).unwrap();
        let tr = &turn2_body["messages"][2]["content"][0];
        assert_eq!(tr["type"], "tool_result");
        assert_eq!(tr["tool_use_id"], "t1");
        assert_eq!(tr["is_error"], true);
        assert!(tr["content"].as_str().unwrap().starts_with("DENIED: "));
    }
}

#[test]
fn multi_tool_turn_routes_both_and_returns_matching_tool_results() {
    let tmp = TempDir::new("multi-tool");
    let session = "s-multi";
    let approver = Arc::new(MockApprover::new(false));
    let epoch = Arc::new(MockEpoch::new(1, false));
    let runs = Arc::new(AtomicUsize::new(0));
    let router = make_router(&tmp.path, session, approver, epoch, runs.clone());
    let ledger = Ledger::open(&tmp.path).unwrap();

    let server = MockServer::start(vec![
        ok_msg(
            "m1",
            "tool_use",
            json!([
                { "type": "tool_use", "id": "tu_a", "name": "sympy_eval", "input": { "code": "9*9" } },
                { "type": "tool_use", "id": "tu_b", "name": "ledger_note", "input": { "text": "recorded b" } }
            ]),
        ),
        ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "all done" }])),
    ]);
    let transport = make_transport(server.base_url.clone(), "sk-multi");
    let out = run_agent_loop(&transport, &router, &ledger, &default_cfg("both", false), &mut NoopSink).unwrap();
    assert_eq!(out, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });
    let reqs = server.requests_received();
    server.finish();

    assert_eq!(runs.load(Ordering::SeqCst), 0);
    let turn2: Value = serde_json::from_slice(&reqs[1]).unwrap();
    let results = turn2["messages"][2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["tool_use_id"], "tu_a");
    assert_eq!(results[0]["is_error"], true);
    assert!(results[0]["content"].as_str().unwrap().starts_with("DENIED: "));
    assert_eq!(results[1]["tool_use_id"], "tu_b");
    assert_eq!(results[1]["is_error"], false);
    assert!(ledger.read_session(session).unwrap().iter().any(|e| e.kind == "note"));
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn model_refusal_dispatches_zero_tools_and_records_refusal() {
    let tmp = TempDir::new("refusal");
    let session = "s-refusal";
    let approver = Arc::new(MockApprover::new(true));
    let epoch = Arc::new(MockEpoch::new(1, false));
    let runs = Arc::new(AtomicUsize::new(0));
    let router = make_router(&tmp.path, session, approver, epoch, runs.clone());
    let ledger = Ledger::open(&tmp.path).unwrap();

    let server = MockServer::start(vec![ok_msg(
        "m1",
        "refusal",
        json!([
            { "type": "text", "text": "Cannot assist with that." },
            { "type": "tool_use", "id": "tu_x", "name": "ledger_note", "input": { "text": "must not run" } }
        ]),
    )]);
    let transport = make_transport(server.base_url.clone(), "sk-refusal");
    let out = run_agent_loop(&transport, &router, &ledger, &default_cfg("refuse", false), &mut NoopSink).unwrap();
    assert_eq!(out, SessionOutcome::Refusal { turns: 1, text: "Cannot assist with that.".into() });
    server.finish();
    assert_eq!(runs.load(Ordering::SeqCst), 0);

    let events = ledger.read_session(session).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert!(!kinds.contains(&"tool.request"));
    assert!(kinds.contains(&"model.response"));
    assert!(kinds.contains(&"model.refusal"));
    let end = events.last().unwrap();
    assert_eq!(end.kind, "session.end");
    assert_eq!(end.payload["outcome"], "refusal");
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn truncation_protocol_violations_http_error_and_max_turns_abort_cleanly() {
    type AbortCase<'a> = (&'a str, Vec<(&'static str, Vec<u8>)>, u32, bool);
    let cases: Vec<AbortCase<'_>> = vec![
        (
            "max-tokens",
            vec![ok_msg(
                "m1",
                "max_tokens",
                json!([{ "type": "tool_use", "id": "t1", "name": "ledger_note", "input": { "text": "no" } }]),
            )],
            4,
            false,
        ),
        (
            "end-turn-with-tool",
            vec![ok_msg(
                "m1",
                "end_turn",
                json!([{ "type": "tool_use", "id": "t1", "name": "ledger_note", "input": { "text": "no" } }]),
            )],
            4,
            false,
        ),
        ("tool-use-empty", vec![ok_msg("m1", "tool_use", json!([{ "type": "text", "text": "oops" }]))], 4, false),
        (
            "dup-tool-id",
            vec![ok_msg(
                "m1",
                "tool_use",
                json!([
                    { "type": "tool_use", "id": "dup", "name": "ledger_note", "input": { "text": "a" } },
                    { "type": "tool_use", "id": "dup", "name": "ledger_note", "input": { "text": "b" } }
                ]),
            )],
            4,
            false,
        ),
        (
            "http-500",
            vec![(
                "500 Internal Server Error",
                br#"{"type":"error","error":{"message":"boom sk-abort-secret-leak-check"}}"#.to_vec(),
            )],
            4,
            true,
        ),
        (
            "max-turns",
            vec![ok_msg(
                "m1",
                "tool_use",
                json!([{ "type": "tool_use", "id": "t1", "name": "ledger_note", "input": { "text": "ok" } }]),
            )],
            1,
            false,
        ),
    ];

    for (tag, resps, max_turns, expect_model_error) in cases {
        let tmp = TempDir::new(tag);
        let session = format!("s-{tag}");
        let approver = Arc::new(MockApprover::new(true));
        let epoch = Arc::new(MockEpoch::new(1, false));
        let runs = Arc::new(AtomicUsize::new(0));
        let router = make_router(&tmp.path, &session, approver, epoch, runs.clone());
        let ledger = Ledger::open(&tmp.path).unwrap();

        let server = MockServer::start(resps);
        let secret = "sk-abort-secret-leak-check";
        let transport = make_transport(server.base_url.clone(), secret);
        let mut cfg = default_cfg("test", false);
        cfg.max_turns = max_turns;
        let mut sink = RecordingSink::new();
        let out = run_agent_loop(&transport, &router, &ledger, &cfg, &mut sink).unwrap();
        assert!(matches!(out, SessionOutcome::Aborted { .. }), "{tag}: expected Aborted, got {out:?}");
        server.finish();
        assert_eq!(runs.load(Ordering::SeqCst), 0);

        let events = ledger.read_session(&session).unwrap();
        assert!(!events.iter().any(|e| e.kind == "tool.request"), "{tag}: zero tools should run");
        assert_eq!(events.last().unwrap().kind, "session.abort", "{tag}");
        if expect_model_error {
            let err_ev = events.iter().find(|e| e.kind == "model.error").expect("model.error recorded");
            assert!(err_ev.payload["raw_blob"].as_str().is_some());
        }
        assert_secret_absent(&tmp.path, &session, &sink, secret);
        let all = ledger.verify_all().unwrap();
        assert_eq!(all.unreferenced_blobs, 0, "{tag}: zero orphan blobs");
    }
}

#[test]
fn fail_closed_before_http_when_blobs_unwritable_and_mid_session_router_failure() {
    // Case 1: blobs/ unwritable before turn 1 -> Err BEFORE sending a single HTTP request.
    {
        let tmp = TempDir::new("fail-pre-http");
        let session = "s-fail-pre";
        let approver = Arc::new(MockApprover::new(true));
        let epoch = Arc::new(MockEpoch::new(1, false));
        let runs = Arc::new(AtomicUsize::new(0));
        let router = make_router(&tmp.path, session, approver, epoch, runs);
        let ledger = Ledger::open(&tmp.path).unwrap();

        let blobs = tmp.path.join("blobs");
        std::fs::remove_dir_all(&blobs).unwrap();
        std::fs::write(&blobs, b"not-a-dir").unwrap();

        let server = MockServer::start(vec![]);
        let transport = make_transport(server.base_url.clone(), "sk-fail-pre");
        let res = run_agent_loop(&transport, &router, &ledger, &default_cfg("hi", false), &mut NoopSink);
        assert!(res.is_err(), "expected Err when model.request blob fails: {res:?}");
        assert!(server.requests_received().is_empty(), "must not send HTTP request when ledger fails");
    }

    // Case 2: legacy v0 unchained database -> fails closed BEFORE any HTTP request.
    {
        let tmp = TempDir::new("fail-legacy-v0");
        std::fs::write(tmp.path.join("ledger.db"), support::legacy_v0_db_bytes()).unwrap();
        let session = "s-legacy";
        let approver = Arc::new(MockApprover::new(true));
        let epoch = Arc::new(MockEpoch::new(1, false));
        let runs = Arc::new(AtomicUsize::new(0));
        let router = make_router(&tmp.path, session, approver, epoch, runs);
        let ledger = Ledger::open(&tmp.path).unwrap();

        let server = MockServer::start(vec![]);
        let transport = make_transport(server.base_url.clone(), "sk-fail-legacy");
        let res = run_agent_loop(&transport, &router, &ledger, &default_cfg("hi", false), &mut NoopSink);
        assert!(res.is_err(), "expected Err on legacy v0 unchained db: {res:?}");
        assert!(server.requests_received().is_empty(), "must not send HTTP request on legacy db");
    }

    // Case 3: blobs/ becomes unwritable mid-session before a > 4 KiB tool call -> Err without running handler.
    {
        let tmp = TempDir::new("fail-mid-tool");
        let session = "s-fail-mid";
        let approver = Arc::new(MockApprover::new(true));
        let epoch = Arc::new(MockEpoch::new(1, false));
        let runs = Arc::new(AtomicUsize::new(0));
        let router = make_router(&tmp.path, session, approver, epoch, runs.clone());
        let ledger = Ledger::open(&tmp.path).unwrap();

        let big_code = "x".repeat(5000);
        let server = MockServer::start(vec![ok_msg(
            "m1",
            "tool_use",
            json!([{ "type": "tool_use", "id": "tu_big", "name": "sympy_eval", "input": { "code": big_code } }]),
        )]);
        let transport = make_transport(server.base_url.clone(), "sk-fail-mid");

        struct CorruptBlobsAfterResponse {
            blobs_dir: PathBuf,
        }
        impl EventSink for CorruptBlobsAfterResponse {
            fn on_event(&mut self, _receipt: &AppendedEvent, kind: &str, _data: &Value) -> Result<(), String> {
                if kind == "model.tool_use" {
                    let _ = std::fs::remove_dir_all(&self.blobs_dir);
                    std::fs::write(&self.blobs_dir, b"blocked").map_err(|e| e.to_string())?;
                }
                Ok(())
            }
        }

        let mut sink = CorruptBlobsAfterResponse { blobs_dir: tmp.path.join("blobs") };
        let res = run_agent_loop(&transport, &router, &ledger, &default_cfg("big tool", false), &mut sink);
        assert!(res.is_err(), "expected Err when mid-session tool call blob write fails: {res:?}");
        assert_eq!(server.requests_received().len(), 1, "must not make another model request");
        server.finish();
        assert_eq!(runs.load(Ordering::SeqCst), 0, "handler must not run when Router::try_call fails");
    }
}
