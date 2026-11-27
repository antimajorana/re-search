//! Integration tests for Solve CLI parity Checkpoint S15d:
//! - Ledger-driven result-sheet honesty (`Not done: N steps unchecked` despite CLI final prose
//!   claiming "Final answer is verified!", R15 & R22).
//! - D16 CLI-route verifier status & setup-card notice (never hides Solve from CLI users,
//!   rejects CLI helper roles per `agent_models.rs`, and resolves configured API verifier transport).
//! - Antigravity acknowledgement gate stays enforced on Solve CLI sessions.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::cli_turn_solve::{
    resolve_api_verifier_transport_for_cli, D16_CLI_REVIEW_UNAVAILABLE_NOTICE,
    D16_CLI_VERIFIER_REJECTED_NOTICE, D16_CP2_PENDING_S14_NOTICE,
};
use desktop_server::start_desktop_server;
use desktop_server::state::DesktopState;
use ledger::Ledger;
use research_tools::solve_model::{event_kinds, RoleModel, StepEntryType, StepOrigin};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::TempDir;

fn create_solve_session(port: u16, tok: &str, sid: &str) -> Value {
    let hdrs = [("X-ReSearch-Token", tok)];
    let body = serde_json::to_vec(&json!({ "id": sid, "mode": "solve" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &hdrs, &body);
    assert_eq!(st, 200, "create solve session failed: {}", String::from_utf8_lossy(&raw));
    serde_json::from_slice(&raw).unwrap()
}

#[test]
fn cli_unchecked_final_prose_still_shows_not_done_and_unchecked_from_ledger() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data, None, None, None)).unwrap();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    let hdrs = [("X-ReSearch-Token", tok.as_str())];

    let sid = "s-cli-honest";
    create_solve_session(port, &tok, sid);

    let ledger = Ledger::open(&data).unwrap();
    // Simulate a CLI turn that recorded 3 unchecked steps and then printed bogus final prose
    // claiming "Final answer is verified and proved!" without calling `solve_finish`.
    ledger
        .append_with_hash(
            sid,
            "session.start",
            1,
            &json!({
                "route": "claude-code-cli",
                "provenance": "external loop, partial provenance",
                "tool_profile": "solve/1",
            }),
        )
        .unwrap();
    for i in 1..=3 {
        ledger
            .append_with_hash(
                sid,
                event_kinds::PROOF_ENTRY,
                1,
                &json!({
                    "entry_id": format!("e-{i}"),
                    "entry_type": StepEntryType::Derivation.as_str(),
                    "summary": format!("Unchecked step {i}"),
                    "details": "Prose derivation step without checks",
                    "depends_on": [],
                    "origin": StepOrigin::Solver,
                }),
            )
            .unwrap();
    }
    ledger
        .append_with_hash(
            sid,
            "model.text",
            1,
            &json!({ "text": "Final answer is verified and proved!" }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "cli.result",
            1,
            &json!({ "result": "Final answer is verified and proved!", "is_error": false }),
        )
        .unwrap();

    let (st, _, raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/solve/view"), &hdrs, b"");
    assert_eq!(st, 200);
    let view: Value = serde_json::from_slice(&raw).unwrap();
    assert!(view["done"].is_null());
    let sheet = &view["result_sheet"];
    assert_eq!(sheet["done"], false);
    assert_eq!(sheet["status"], "not_done");
    assert_eq!(sheet["unchecked_steps"], 3);
    assert_eq!(sheet["status_line"], "Not done: 3 steps unchecked");

    // R22 honesty: no forbidden words in result_sheet or graph while steps are unchecked.
    let sheet_str = sheet.to_string();
    assert!(!sheet_str.to_ascii_lowercase().contains("verified"), "leaked verified: {sheet_str}");
    assert!(!sheet_str.to_ascii_lowercase().contains("proved"), "leaked proved: {sheet_str}");
    assert!(!sheet_str.contains("Computed"), "leaked Computed: {sheet_str}");

    // Seed `solve.done` with `finished_open_items` -> R15 status line must NOT claim bare "Done".
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_DONE,
            1,
            &json!({
                "final_entry_id": "e-3",
                "own_tier": "unchecked",
                "effective_tier": "unchecked",
                "criteria": [{ "id": "final_result_tier", "met": false, "evidence": {} }],
                "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
                "status": "finished_open_items",
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
            }),
        )
        .unwrap();
    let (st, _, raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/solve/view"), &hdrs, b"");
    assert_eq!(st, 200);
    let view2: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(view2["result_sheet"]["done"], true);
    assert_eq!(view2["result_sheet"]["status_line"], "Done with open items");
    assert_eq!(view2["result_sheet"]["badge_label"], "Unchecked");
    srv.shutdown();
}

#[test]
fn d16_cli_review_notices_helper_role_guard_and_api_verifier_resolution() {
    let tmp = TempDir::new().unwrap();
    let data_no_api = tmp.path().join("data-no-api");
    let mut cfg = test_cfg(&data_no_api, None, None, None);
    cfg.connection_env = Some(BTreeMap::new());
    let srv = start_desktop_server(cfg).unwrap();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    let hdrs = [("X-ReSearch-Token", tok.as_str())];

    let sid = "s-d16-cli";
    create_solve_session(port, &tok, sid);

    // 1. CLI-only user with no API keys still gets HTTP 200 prefill + plain D16 notice.
    let body = serde_json::to_vec(&json!({
        "statement": "Compute derivative of x^3",
        "solver": { "provider": "claude-cli", "model": "sonnet" },
        "verifier": { "provider": "", "model": "" }
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/solve/prefill"), &hdrs, &body);
    assert_eq!(st, 200);
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    let rev = &resp["cli_review"];
    assert_eq!(rev["on_cli_route"], true);
    assert_eq!(rev["cli_provider"], "claude-cli");
    assert_eq!(rev["api_verifier_configured"], false);
    assert_eq!(rev["automatic_review_available"], false);
    assert_eq!(rev["cli_helper_rejected"], false);
    assert_eq!(rev["notice"], D16_CLI_REVIEW_UNAVAILABLE_NOTICE);
    let notice = rev["notice"].as_str().unwrap();
    assert!(!notice.to_ascii_lowercase().contains("verified"), "{notice}");
    assert!(!notice.to_ascii_lowercase().contains("proved"), "{notice}");

    // 2. Assigning a CLI provider to the verifier role is rejected per agent_models.rs helper-role rule.
    let cli_verifier_body = serde_json::to_vec(&json!({
        "statement": "Compute derivative of x^3",
        "solver": { "provider": "claude-cli", "model": "sonnet" },
        "verifier": { "provider": "antigravity", "model": "gemini-3.8-flash-medium" }
    }))
    .unwrap();
    let (st, _, raw) =
        raw_http(port, "POST", &format!("/api/sessions/{sid}/solve/prefill"), &hdrs, &cli_verifier_body);
    assert_eq!(st, 200);
    let resp2: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp2["cli_review"]["cli_helper_rejected"], true);
    assert_eq!(resp2["cli_review"]["api_verifier_configured"], false);
    assert_eq!(resp2["cli_review"]["notice"], D16_CLI_VERIFIER_REJECTED_NOTICE);
    srv.shutdown();

    // 3. Configured in-core API verifier resolves a real API Transport (CP2 seam for S14).
    let data_api = tmp.path().join("data-api");
    std::fs::create_dir_all(&data_api).unwrap();
    std::fs::write(
        data_api.join("providers.json"),
        br#"{"providers":{"anthropic":{"api_key":"sk-ant-test-dummy"}}}"#,
    )
    .unwrap();
    let cfg_api = test_cfg(
        &data_api,
        None,
        Some("http://127.0.0.1:9".into()),
        None,
    );
    let srv_api = start_desktop_server(cfg_api.clone()).unwrap();
    let (port_a, tok_a) = (srv_api.addr.port(), srv_api.info.token.clone());
    let hdrs_a = [("X-ReSearch-Token", tok_a.as_str())];
    let sid_a = "s-d16-api";
    create_solve_session(port_a, &tok_a, sid_a);

    let api_body = serde_json::to_vec(&json!({
        "statement": "Compute derivative of x^3",
        "solver": { "provider": "claude-cli", "model": "sonnet" },
        "verifier": { "provider": "anthropic", "model": "claude-haiku-4-5" }
    }))
    .unwrap();
    let (st, _, raw) =
        raw_http(port_a, "POST", &format!("/api/sessions/{sid_a}/solve/prefill"), &hdrs_a, &api_body);
    assert_eq!(st, 200);
    let resp3: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp3["cli_review"]["on_cli_route"], true);
    assert_eq!(resp3["cli_review"]["api_verifier_configured"], true);
    assert_eq!(resp3["cli_review"]["verifier_transport"], "in_core_api");
    assert_eq!(resp3["cli_review"]["verifier"]["provider"], "anthropic");
    assert_eq!(resp3["cli_review"]["notice"], D16_CP2_PENDING_S14_NOTICE);
    srv_api.shutdown();

    // Direct transport resolution helper also builds the API Transport and rejects CLI helpers.
    let chatgpt = desktop_server::chatgpt::ChatGptHub::new(&cfg_api).unwrap();
    let state = DesktopState::new(
        cfg_api,
        desktop_server::auth::AuthGuard::new(1).unwrap(),
        Arc::new(desktop_server::sse::SseHub::new()),
        chatgpt,
    );
    let resolved = resolve_api_verifier_transport_for_cli(
        &state,
        Some(&RoleModel { provider: "anthropic".into(), model: "claude-haiku-4-5".into() }),
    )
    .unwrap();
    assert!(resolved.is_some());
    let (_, prov, _) = resolved.unwrap();
    assert_eq!(prov, "anthropic");

    let rejected = resolve_api_verifier_transport_for_cli(
        &state,
        Some(&RoleModel { provider: "claude-cli".into(), model: "sonnet".into() }),
    );
    assert!(rejected.is_err());
}

#[test]
fn antigravity_acknowledgement_remains_enforced_for_solve_cli_route() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let cfg = test_cfg(&data, None, None, None);
    let chatgpt = desktop_server::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(
        cfg,
        desktop_server::auth::AuthGuard::new(1).unwrap(),
        Arc::new(desktop_server::sse::SseHub::new()),
        chatgpt,
    );
    let ledger = Ledger::open(&data).unwrap();
    ledger.append_with_hash("s-agy-solve", event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    state
        .save_meta(&desktop_server::state::SessionMeta {
            id: "s-agy-solve".into(),
            title: "Solve on Antigravity".into(),
            ..Default::default()
        })
        .unwrap();

    // Unacknowledged Antigravity must remain refused (403).
    let err = desktop_server::antigravity::require_acknowledged(&state).unwrap_err();
    assert!(err.contains("Antigravity") || err.contains("acknowledge"), "{err}");
    desktop_server::antigravity::handle_acknowledge(&state).unwrap();
    assert!(desktop_server::antigravity::require_acknowledged(&state).is_ok());
}
