//! HTTP integration tests for S12d Solve continue & interrupted reconciliation routes.

mod common;

use common::{raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::solve_model::{compute_math_sha256, event_kinds};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn setup_payload() -> Value {
    let msha = compute_math_sha256(&[], &Default::default(), &[], "");
    json!({
        "revision": 1,
        "statement": "Compute sum",
        "task_kind": "calculate",
        "target": { "description": "2", "form": "closed_form" },
        "done": [{ "id": "d1", "kind": "no_open_flags", "params": {} }],
        "conventions": "",
        "symbols": [],
        "dimensions": {},
        "assumptions": [],
        "sources": [],
        "roles": {
            "solver": { "provider": "anthropic", "model": "claude-sonnet-4-5" },
            "verifier": { "provider": "openai", "model": "gpt-4.1" }
        },
        "budgets": {
            "max_model_calls": 60,
            "max_wall_minutes": 30,
            "max_model_calls_per_leg": 48,
            "max_review_loops": 6,
            "max_gate_bounces": 3
        },
        "review": { "mode": "prose_only" },
        "tool_profile": "solve/1",
        "math_sha256": msha
    })
}

fn anthropic_json_reply(id: &str, text: &str) -> MockReply {
    MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&json!({
            "id": id,
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": text }],
            "usage": { "input_tokens": 25, "output_tokens": 12 }
        }))
        .unwrap(),
    }
}

#[test]
fn solve_continue_runs_background_leg_and_reconcile_handles_interrupted_sessions_without_get_mutation() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    // Seed an interrupted Solve session (dangling open leg without solve.paused/done).
    ledger.append_with_hash("s-solve-int", event_kinds::SOLVE_STARTED, 1, &json!({ "origin": "new" })).unwrap();
    ledger.append_with_hash("s-solve-int", event_kinds::SOLVE_SETUP, 1, &setup_payload()).unwrap();
    ledger.append_with_hash("s-solve-int", "session.start", 1, &json!({
        "route": "api",
        "provider": "anthropic-messages",
        "model": "claude-sonnet-4-5",
        "prompt": "Solve it",
        "tool_profile": "solve/1",
        "leg": 1
    })).unwrap();
    ledger.append_with_hash("s-solve-int", "model.request", 1, &json!({ "turn": 1, "caller": "solver" })).unwrap();

    // Seed a paused Solve session ready for `POST /solve/continue`.
    ledger.append_with_hash("s-solve-cont", event_kinds::SOLVE_STARTED, 1, &json!({ "origin": "new" })).unwrap();
    ledger.append_with_hash("s-solve-cont", event_kinds::SOLVE_SETUP, 1, &setup_payload()).unwrap();
    ledger.append_with_hash("s-solve-cont", "session.start", 1, &json!({
        "route": "api",
        "provider": "anthropic-messages",
        "model": "claude-sonnet-4-5",
        "prompt": "Solve it",
        "tool_profile": "solve/1",
        "leg": 1
    })).unwrap();
    ledger.append_with_hash("s-solve-cont", event_kinds::SOLVE_PAUSED, 1, &json!({
        "reason": "leg_cap", "leg": 1, "turn": 1, "model_calls_used": 1, "open_items": []
    })).unwrap();

    // Configure provider key for loopback MockHttp.
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("providers.json"),
        serde_json::to_vec(&json!({ "anthropic": "sk-test-solve-123" })).unwrap(),
    )
    .unwrap();
    let mock = MockHttp::start(vec![anthropic_json_reply("msg_c1", "Continued leg finished cleanly.")]);
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, Some(mock.base_url.clone()))).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];

    // Startup reconciliation recorded `solve.paused { reason: "interrupted" }` on `s-solve-int`!
    let evs_int = ledger.read_session("s-solve-int").unwrap();
    assert_eq!(evs_int.last().unwrap().kind, event_kinds::SOLVE_PAUSED);
    assert_eq!(evs_int.last().unwrap().payload["reason"], "interrupted");
    let count_before_get = evs_int.len();

    // `GET /solve/view` never mutates ledger (THREAT-MODEL.md:333).
    let (st, _, view_raw) = raw_http(port, "GET", "/api/sessions/s-solve-int/solve/view", &auth, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&view_raw));
    assert_eq!(ledger.read_session("s-solve-int").unwrap().len(), count_before_get);

    // Explicit `POST /solve/reconcile` returns 200 `{ reconciled: false }` when already paused,
    // and `{ reconciled: true }` when a dangling leg is introduced after boot.
    let (st, _, rec1) = raw_http(port, "POST", "/api/sessions/s-solve-int/solve/reconcile", &auth, b"{}");
    assert_eq!(st, 200);
    assert_eq!(serde_json::from_slice::<Value>(&rec1).unwrap()["reconciled"], false);

    ledger.append_with_hash("s-solve-int", "session.resume", 1, &json!({ "leg": 2 })).unwrap();
    let (st, _, view2_raw) = raw_http(port, "GET", "/api/sessions/s-solve-int/solve/view", &auth, b"");
    assert_eq!(st, 200);
    let view2: Value = serde_json::from_slice(&view2_raw).unwrap();
    assert_eq!(view2["solve"]["run_state"]["state"], "interrupted");
    let seq_before_rec2 = ledger.read_session("s-solve-int").unwrap().len();
    // Double check GET appended zero events.
    let _ = raw_http(port, "GET", "/api/sessions/s-solve-int/solve/view", &auth, b"");
    assert_eq!(ledger.read_session("s-solve-int").unwrap().len(), seq_before_rec2);

    let (st, _, rec2) = raw_http(port, "POST", "/api/sessions/s-solve-int/solve/reconcile", &auth, b"{}");
    assert_eq!(st, 200);
    assert_eq!(serde_json::from_slice::<Value>(&rec2).unwrap()["reconciled"], true);

    // `POST /solve/continue` accepts 202 and completes leg 2 cleanly in background.
    let cont_body = serde_json::to_vec(&json!({ "note": "Keep going", "provider": "anthropic", "model": "claude-sonnet-4-5" })).unwrap();
    let (st, _, cont_raw) = raw_http(port, "POST", "/api/sessions/s-solve-cont/solve/continue", &auth, &cont_body);
    assert_eq!(st, 202, "{}", String::from_utf8_lossy(&cont_raw));

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let evs = ledger.read_session("s-solve-cont").unwrap();
        if evs.iter().any(|e| e.kind == "session.end") {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let evs_cont = ledger.read_session("s-solve-cont").unwrap();
    assert!(evs_cont.iter().any(|e| e.kind == event_kinds::SOLVE_CHECKPOINT && e.payload["reason"] == "continue"));
    assert!(evs_cont.iter().any(|e| e.kind == "session.resume" && e.payload["resume_mode"] == "brief"));
    assert!(evs_cont.iter().any(|e| e.kind == "session.end"));
    assert!(!evs_cont.iter().any(|e| e.kind == "session.abort"));
    srv.shutdown();
    mock.finish();
}
