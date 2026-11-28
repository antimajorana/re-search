//! Integration tests for S15 CP2: Solve subscription-CLI turn dispatch (gated on
//! `solve.setup`) and automatic post-CLI-turn API `verifier_loop` review.

mod cli_support;
mod common;

use cli_route::core_tools::core_tool_set_for_profile;
use cli_route::testkit::FakeCli;
use cli_support::clear_env_auth;
use common::{raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::solve_model::{compute_math_sha256, event_kinds, StepEntryType, StepOrigin};
use research_tools::tool_profile::PROFILE_SOLVE_V1;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[test]
fn solve_cli_turn_gated_by_setup_and_runs_post_turn_api_verifier_loop() {
    clear_env_auth();

    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("providers.json"),
        serde_json::to_vec(&json!({
            "providers": { "openai": { "api_key": "sk-test-cli-verifier" } }
        }))
        .unwrap(),
    )
    .unwrap();

    let fake = FakeCli::new(tmp.path(), "fake-cli");
    let mock = MockHttp::start(vec![MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&json!({
            "id": "chatcmpl_cli_cp2",
            "object": "chat.completion",
            "model": "gpt-4.1",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_cp2_v1",
                        "type": "function",
                        "function": {
                            "name": "report_verdict",
                            "arguments": json!({
                                "verdicts": [{
                                    "entry_id": "e-1",
                                    "verdict": "accepted",
                                    "summary": "Prose step e-1 verified via CP2 API loop.",
                                    "checks_cited": []
                                }]
                            }).to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 30, "completion_tokens": 18, "total_tokens": 48 }
        }))
        .unwrap(),
    }]);

    let mut cfg = test_cfg(&data, None, Some(mock.base_url.clone()), None);
    cfg.claude_exe = Some(fake.exe.clone());
    cfg.core_exe = Some(fake.exe.clone());
    cfg.connection_env = Some(BTreeMap::new());
    let srv = start_desktop_server(cfg).unwrap();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    let hdrs = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];

    let sid = "s-cli-cp2";
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &hdrs,
        serde_json::to_vec(&json!({ "id": sid, "mode": "solve" })).unwrap().as_slice(),
    );
    assert_eq!(st, 200, "create solve session failed: {}", String::from_utf8_lossy(&raw));

    // 1. Before `solve.setup`, `POST .../messages` on CLI route still returns 400.
    let msg = serde_json::to_vec(&json!({
        "prompt": "Run Solve on CLI",
        "provider": "claude-cli",
        "model": "sonnet"
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdrs, &msg);
    assert_eq!(st, 400, "expected 400 before setup: {}", String::from_utf8_lossy(&raw));
    assert!(String::from_utf8_lossy(&raw).contains("setup"));
    assert!(fake.argv().is_none(), "fake CLI must not run before solve.setup");

    // 2. Seed `solve.setup` + prose step `e-1` + `solve.done`, then run CLI turn.
    let ledger = Ledger::open(&data).unwrap();
    let msha = compute_math_sha256(&[], &Default::default(), &[], "");
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_SETUP,
            1,
            &json!({
                "revision": 1,
                "statement": "Compute sum on CLI",
                "task_kind": "calculate",
                "target": { "description": "2", "form": "closed_form" },
                "done": [],
                "conventions": "",
                "symbols": [],
                "dimensions": {},
                "assumptions": [],
                "sources": [],
                "roles": {
                    "solver": { "provider": "claude-cli", "model": "sonnet" },
                    "verifier": { "provider": "openai", "model": "gpt-4.1" }
                },
                "budgets": {
                    "max_model_calls": 60, "max_wall_minutes": 30,
                    "max_model_calls_per_leg": 48, "max_review_loops": 6, "max_gate_bounces": 3
                },
                "review": "prose_only",
                "tool_profile": "solve/1",
                "math_sha256": msha
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": StepEntryType::FinalAnswer.as_str(),
                "summary": "Sum equals two",
                "details": "Prose proof ready for CP2 API verifier review.",
                "depends_on": [],
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_DONE,
            1,
            &json!({
                "final_entry_id": "e-1",
                "own_tier": "unchecked",
                "effective_tier": "unchecked",
                "criteria": [],
                "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
                "status": "finished_open_items",
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
            }),
        )
        .unwrap();

    let tools: Vec<String> = core_tool_set_for_profile(
        Arc::new(ResearchEnv::new(&data)),
        PROFILE_SOLVE_V1,
    )
    .unwrap()
    .defs
    .iter()
    .map(|d| format!("mcp__core__{}", d.id))
    .collect();
    let script = vec![
        json!({
            "type": "system", "subtype": "init", "tools": tools,
            "mcp_servers": [{ "name": "core", "status": "connected" }],
            "model": "claude-sonnet", "claude_code_version": "2.1.318",
            "apiKeySource": "none", "session_id": "cli-cp2"
        }).to_string(),
        json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": "CLI solve turn done." }] }
        }).to_string(),
        json!({
            "type": "result", "subtype": "success", "is_error": false,
            "result": "CLI solve turn done.", "num_turns": 1, "duration_ms": 5
        }).to_string(),
    ];
    fake.script(&script);

    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdrs, &msg);
    assert_eq!(st, 202, "CLI Solve turn after setup failed: {}", String::from_utf8_lossy(&raw));

    // Wait for post-CLI CP2 verifier review to persist `proof.verification` (written after CLI `session.end`).
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let evs = ledger.read_session(sid).unwrap();
        if evs.iter().any(|e| {
            e.kind == event_kinds::PROOF_VERIFICATION
                || e.kind == "verifier.incomplete"
                || e.kind == "session.abort"
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let evs = ledger.read_session(sid).unwrap();
    assert!(evs.iter().any(|e| e.kind == "session.end"), "expected session.end in {evs:#?}");
    assert!(!evs.iter().any(|e| e.kind == "session.abort"), "unexpected session.abort in {evs:#?}");
    assert!(
        evs.iter().any(|e| e.kind == "verifier.request" && e.payload["caller"] == "verifier:solve" && e.payload["provider"] == "openai"),
        "expected CP2 verifier.request in {evs:#?}"
    );
    assert!(
        evs.iter().any(|e| e.kind == event_kinds::PROOF_VERIFICATION && e.payload["target_entry_id"] == "e-1" && e.payload["verdict"] == "accepted"),
        "expected CP2 PROOF_VERIFICATION accepted in {evs:#?}"
    );
    assert!(fake.argv().is_some(), "expected fake CLI to run");

    let (st, _, view_raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/solve/view"), &hdrs, b"");
    assert_eq!(st, 200);
    let view: Value = serde_json::from_slice(&view_raw).unwrap();
    assert_eq!(view["cli_review"]["automatic_review_available"], true);
    assert_eq!(view["cli_review"]["s14_verifier_loop_ready"], true);
    assert_eq!(view["cli_review"]["notice"], Value::Null);
    assert_eq!(view["result_sheet"]["badge_label"], "Model agreed");

    srv.shutdown();
    mock.finish();
}
