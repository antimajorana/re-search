//! Integration tests for Solve CLI parity Checkpoint S15d:
//! - Ledger-driven result-sheet honesty (`Not done: N steps unchecked` despite CLI final prose
//!   claiming "Final answer is verified!", R15 & R22).
//! - D16 CLI-route verifier status & setup-card notice (never hides Solve from CLI users,
//!   rejects CLI helper roles per `agent_models.rs`, and resolves configured API verifier transport).
//! - Antigravity route is available directly for Solve CLI sessions when the CLI is found.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::cli_turn_solve::{
    resolve_api_verifier_transport_for_cli, D16_CLI_REVIEW_UNAVAILABLE_NOTICE,
    D16_CLI_VERIFIER_REJECTED_NOTICE,
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
    assert_eq!(resp3["cli_review"]["automatic_review_available"], true);
    assert_eq!(resp3["cli_review"]["s14_verifier_loop_ready"], true);
    assert_eq!(resp3["cli_review"]["verifier_transport"], "in_core_api");
    assert_eq!(resp3["cli_review"]["verifier"]["provider"], "anthropic");
    assert_eq!(resp3["cli_review"]["notice"], Value::Null);
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
fn antigravity_route_is_available_directly_for_solve_cli_route() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let fake = cli_route::testkit::FakeCli::new_agy(tmp.path(), "agy");

    // Without agy CLI configured, Antigravity is unavailable.
    let cfg_missing = test_cfg(&data, None, None, None);
    let chatgpt_m = desktop_server::chatgpt::ChatGptHub::new(&cfg_missing).unwrap();
    let state_missing = DesktopState::new(
        cfg_missing,
        desktop_server::auth::AuthGuard::new(1).unwrap(),
        Arc::new(desktop_server::sse::SseHub::new()),
        chatgpt_m,
    );
    assert!(!desktop_server::antigravity::is_available(&state_missing));

    // Once the CLI is found, Antigravity is enabled directly with no gate.
    let mut cfg = test_cfg(&data, None, None, None);
    cfg.agy_exe = Some(fake.exe.clone());
    let chatgpt = desktop_server::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(
        cfg,
        desktop_server::auth::AuthGuard::new(1).unwrap(),
        Arc::new(desktop_server::sse::SseHub::new()),
        chatgpt,
    );
    assert!(desktop_server::antigravity::is_available(&state));
    let conn = desktop_server::antigravity::connection_entry(&state);
    assert_eq!(conn["connected"], true);
    assert_eq!(conn["label"], "Antigravity (Google)");
    assert!(conn.get("warning").is_none());
    assert!(conn.get("confinement").is_none());

    let ledger = Ledger::open(&data).unwrap();
    ledger.append_with_hash("s-agy-solve", event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    let view = desktop_server::solve::session_solve_view(&state, "s-agy-solve", &ledger.read_session("s-agy-solve").unwrap(), false);
    assert_eq!(view["model_connected"], true);
}

#[test]
fn post_done_invalidation_and_api_override_and_blank_verifier_stay_honest() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    // Unrelated API key in providers.json must NOT auto-fabricate a verifier when Verifier is blank.
    std::fs::write(
        data.join("providers.json"),
        br#"{"providers":{"anthropic":{"api_key":"sk-ant-unrelated-key"}}}"#,
    )
    .unwrap();
    let mut cfg = test_cfg(&data, None, Some("http://127.0.0.1:9".into()), None);
    cfg.connection_env = Some(BTreeMap::new());
    let srv = start_desktop_server(cfg).unwrap();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    let hdrs = [("X-ReSearch-Token", tok.as_str())];

    let sid = "s-audit-honest";
    create_solve_session(port, &tok, sid);

    // 1. Blank verifier with unrelated key in providers.json must NOT report api_verifier_configured.
    let blank_body = serde_json::to_vec(&json!({
        "statement": "Compute derivative of x^4",
        "solver": { "provider": "claude-cli", "model": "sonnet" },
        "verifier": { "provider": "", "model": "" }
    }))
    .unwrap();
    let (st, _, raw) =
        raw_http(port, "POST", &format!("/api/sessions/{sid}/solve/prefill"), &hdrs, &blank_body);
    assert_eq!(st, 200);
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["cli_review"]["on_cli_route"], true);
    assert_eq!(resp["cli_review"]["api_verifier_configured"], false);
    assert_eq!(resp["cli_review"]["notice"], D16_CLI_REVIEW_UNAVAILABLE_NOTICE);

    // 2. Explicit API solver override must NOT fall through to session/orchestrator CLI route.
    let api_solver_body = serde_json::to_vec(&json!({
        "statement": "Compute derivative of x^4",
        "solver": { "provider": "anthropic", "model": "claude-sonnet-4-6" },
        "verifier": { "provider": "", "model": "" }
    }))
    .unwrap();
    let (st, _, raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/solve/prefill"),
        &hdrs,
        &api_solver_body,
    );
    assert_eq!(st, 200);
    let resp_api: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp_api["cli_review"]["on_cli_route"], false);

    // 3. Post-`solve.done` invalidation: seed setup rev 1, checked step, `solve.done`, then
    //    D15 setup rev 2 (different math_sha256 -> step becomes Stale) and verify result_sheet
    //    reverts to `done: false` / `"Not done: ..."` with zero `"verified"` leak.
    let ledger = Ledger::open(&data).unwrap();
    let setup_payload = |rev: u32, math_sha: &str| {
        json!({
            "revision": rev,
            "statement": format!("Rev {rev} problem"),
            "task_kind": "calculate",
            "target": { "description": "closed form", "form": "closed_form" },
            "done": [],
            "conventions": "real x",
            "symbols": [{ "name": "x", "assumptions": ["real"] }],
            "dimensions": {},
            "assumptions": [],
            "sources": [],
            "roles": {
                "solver": { "provider": "claude-cli", "model": "sonnet" },
                "verifier": { "provider": "", "model": "" }
            },
            "budgets": {
                "max_model_calls": 60, "max_wall_minutes": 30,
                "max_model_calls_per_leg": 48, "max_review_loops": 6, "max_gate_bounces": 3
            },
            "review": "prose_only",
            "tool_profile": "solve/1",
            "math_sha256": math_sha
        })
    };
    let math_v1 = "1".repeat(64);
    let math_v2 = "2".repeat(64);
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &setup_payload(1, &math_v1)).unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": StepEntryType::FinalAnswer.as_str(),
                "summary": "Initial answer",
                "details": "Computed under rev 1",
                "depends_on": [],
                "origin": StepOrigin::Solver,
                "claim_sha256": "a".repeat(64),
                "math_sha256": math_v1,
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
                "own_tier": "verified",
                "effective_tier": "verified",
                "criteria": [],
                "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
                "status": "finished",
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
            }),
        )
        .unwrap();

    // Even before setup rev 2, live tier for e-1 is Unchecked -> must NEVER parrot stale "verified".
    let (st, _, raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/solve/view"), &hdrs, b"");
    assert_eq!(st, 200);
    let view_pre: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(view_pre["result_sheet"]["status_line"], "Done with open items");
    assert_eq!(view_pre["result_sheet"]["badge_label"], "Unchecked");
    assert!(!view_pre["result_sheet"].to_string().to_ascii_lowercase().contains("verified"));

    // Record a math_v1 check on e-1, then append D15 setup revision 2 (math_v2) -> e-1 becomes Stale
    // and result_sheet MUST revert to `done: false` / `"Not done: 1 step stale"`.
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": "chk-v1",
                "claim_sha256": "a".repeat(64),
                "claim_index": 0,
                "entry_id": "e-1",
                "math_sha256": math_v1,
                "name": "identity_symbolic",
                "registry_sha256": "abc",
                "params": {},
                "outcome": "pass",
                "asked_by": "host",
                "receipt": { "call_id": "c-chk-1", "result_seq": 1 },
                "evidence": {},
                "evidence_sha256": "0".repeat(64)
            }),
        )
        .unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &setup_payload(2, &math_v2)).unwrap();
    let (st, _, raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/solve/view"), &hdrs, b"");
    assert_eq!(st, 200);
    let view_stale: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(view_stale["result_sheet"]["done"], false);
    assert_eq!(view_stale["result_sheet"]["status"], "not_done");
    assert_eq!(view_stale["result_sheet"]["stale_steps"], 1);
    assert_eq!(view_stale["result_sheet"]["status_line"], "Not done: 1 step stale");
    assert!(!view_stale["result_sheet"].to_string().to_ascii_lowercase().contains("verified"));

    srv.shutdown();
}
