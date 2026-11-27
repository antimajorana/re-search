//! Integration tests for Explore Sharpen (X6), Search plan & approval (X7), and Triage (X8).
//!
//! Loopback mocks and scripted transports only; zero real network calls, zero vendor CLIs,
//! zero third-party paper text.

mod common;

use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, Transport,
    TransportError, TransportInfo,
};
use common::{raw_http, test_cfg};
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_ANSWER,
    EVENT_IDEA_BRIEF, EVENT_IDEA_EXCHANGE, EVENT_IDEA_STAGE,
};
use desktop_server::idea_exchanges::{
    run_sharpen_with_transport, SharpenRunResult, CLI_ROUTE_UNSUPPORTED_MSG,
};
use desktop_server::{idea_view, start_desktop_server};
use ledger::Ledger;
use research_tools::run_budget::{count_model_calls, RunScope};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Mutex;
use tempfile::TempDir;

const IDEA_TEXT: &str =
    "Use Krylov complexity growth b_n ~ alpha*n in the volume-law phase and saturation in the area-law phase of a monitored Majorana chain.";

struct ScriptedTransport {
    responses: Mutex<VecDeque<ModelResponse>>,
}

impl ScriptedTransport {
    fn new(responses: Vec<ModelResponse>) -> Self {
        Self { responses: Mutex::new(responses.into()) }
    }
}

impl Transport for ScriptedTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted-x6",
            endpoint: "http://127.0.0.1:0/v1/messages".into(),
            api_key_env: "none".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        Ok(self.responses.lock().unwrap().pop_front().expect("out of scripted responses"))
    }
}

fn tool_resp(id: &str, name: &str, input: Value) -> ModelResponse {
    ModelResponse {
        raw_body: serde_json::to_vec(&json!({ "id": id, "name": name, "input": input })).unwrap(),
        id: Some(format!("msg-{id}")),
        model: "scripted-explore-model".into(),
        stop_reason: StopReason::ToolUse,
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        usage: json!({ "input_tokens": 120, "output_tokens": 45 }),
    }
}

fn valid_sharpen_input() -> Value {
    json!({
        "restatement": "Check whether Lanczos growth rate detects the monitored Majorana chain transition.",
        "assumptions": ["Free-fermion dynamics"],
        "subclaims": [
            {
                "text": "b_n grows linearly in volume-law phase",
                "idea_quote": "b_n ~ alpha*n in the volume-law phase"
            },
            {
                "text": "b_n saturates in area-law phase",
                "idea_quote": "saturation in the area-law phase"
            }
        ],
        "terms": [{ "term": "Krylov complexity", "aliases": ["spread complexity"] }],
        "perspectives": [
            {
                "kind": "exact",
                "question": "Direct prior work on Krylov complexity in monitored Majorana chains",
                "queries": [{ "provider": "arxiv", "q": "Krylov complexity Majorana monitored" }]
            },
            {
                "kind": "other_names",
                "question": "Alternative terminology under spread complexity",
                "queries": [{ "provider": "inspire", "q": "spread complexity entanglement transition" }]
            }
        ],
        "questions": [
            {
                "question": "Projective or continuous monitoring?",
                "options": ["projective", "continuous", "both"],
                "default": "both"
            }
        ],
        "exclusions": []
    })
}

#[test]
fn x6_sharpen_repairs_bad_quote_writes_brief_and_applies_answers_and_cli_guard() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // 1. Create offline Explore session (deterministic path works with zero model/network).
    let create_body = serde_json::to_vec(&json!({
        "id": "s-x6-flow",
        "mode": "explore",
        "text": IDEA_TEXT,
        "preset": "standard",
        "offline": true,
        "queries": []
    }))
    .unwrap();
    let (status, _, raw_create) = raw_http(port, "POST", "/api/sessions", &auth, &create_body);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&raw_create));

    // 2. Calling Sharpen on CLI-subscription routes (`claude-cli`, `antigravity`) returns clear X18 message.
    for cli_prov in ["claude-cli", "antigravity"] {
        let req_body = serde_json::to_vec(&json!({ "provider": cli_prov })).unwrap();
        let (cli_status, _, cli_body) =
            raw_http(port, "POST", "/api/sessions/s-x6-flow/idea/sharpen", &auth, &req_body);
        assert_eq!(cli_status, 400);
        let cli_txt = String::from_utf8_lossy(&cli_body);
        assert!(cli_txt.contains(CLI_ROUTE_UNSUPPORTED_MSG), "{cli_txt}");
    }

    // 3. Run Sharpen directly via scripted transport: attempt 1 has bad substring quote, attempt 2 succeeds.
    let ledger = Ledger::open(&data_dir).unwrap();
    let mut bad_input = valid_sharpen_input();
    bad_input["subclaims"][0]["idea_quote"] = json!("fabricated quote not in idea");
    let transport = ScriptedTransport::new(vec![
        tool_resp("tu-1", "record_idea_brief", bad_input),
        tool_resp("tu-2", "record_idea_brief", valid_sharpen_input()),
    ]);
    let mut progress_kinds = Vec::new();
    let res = run_sharpen_with_transport(
        &ledger,
        "s-x6-flow",
        &transport,
        "scripted-explore-model",
        None,
        |kind, _| progress_kinds.push(kind.to_string()),
    )
    .expect("sharpen should succeed on repair attempt 2");

    match res {
        SharpenRunResult::Finished { brief, attempts, .. } => {
            assert_eq!(attempts, 2);
            assert_eq!(brief.subclaims[0].id, "sc-1");
            assert_eq!(brief.perspectives[1].id, "p-2");
        }
        other => panic!("expected Finished, got {other:?}"),
    }
    assert!(progress_kinds.iter().any(|k| k == EVENT_IDEA_BRIEF));
    assert!(progress_kinds.iter().any(|k| k == EVENT_IDEA_STAGE));

    let events = ledger.read_session("s-x6-flow").unwrap();
    assert_eq!(count_model_calls(&events, RunScope::All), 2);
    assert_eq!(
        events.iter().filter(|e| e.kind == EVENT_IDEA_EXCHANGE).count(),
        2
    );

    // 4. Submit user answer + edited restatement via HTTP `POST /api/sessions/{id}/idea/answer`.
    let ans_body = serde_json::to_vec(&json!({
        "answers": { "q-1": "projective" },
        "restatement": "Edited sharpened restatement for projective measurements."
    }))
    .unwrap();
    let (ans_status, _, ans_raw) =
        raw_http(port, "POST", "/api/sessions/s-x6-flow/idea/answer", &auth, &ans_body);
    assert_eq!(ans_status, 200, "{}", String::from_utf8_lossy(&ans_raw));
    let ans_json: Value = serde_json::from_slice(&ans_raw).unwrap();
    assert_eq!(ans_json["idea"]["sharpened_idea"]["heading"], "Sharpened idea");
    assert_eq!(
        ans_json["idea"]["sharpened_idea"]["restatement"],
        "Edited sharpened restatement for projective measurements."
    );
    assert_eq!(ans_json["idea"]["question_answers"]["q-1"], "projective");

    let final_events = ledger.read_session("s-x6-flow").unwrap();
    assert!(final_events.iter().any(|e| e.kind == EVENT_IDEA_ANSWER));
    let view = idea_view::view(&final_events, false);
    assert_eq!(view["question_answers"]["q-1"], "projective");

    srv.shutdown();
}

#[test]
fn x6_sharpen_respects_f9_budget_cap_rule_p3() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-x6-cap";
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &[]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Quick, true, &stmt).unwrap();

    // Quick preset hard cap is 5 model calls: seed 5 prior idea.exchange events so Sharpen stops at cap.
    for i in 1..=5 {
        ledger
            .append_with_hash(
                sid,
                EVENT_IDEA_EXCHANGE,
                1,
                &json!({ "exchange": "prior", "attempt": i, "accepted": true, "faults": [] }),
            )
            .unwrap();
    }
    let transport = ScriptedTransport::new(vec![]);
    let res = run_sharpen_with_transport(&ledger, sid, &transport, "m", None, |_, _| {}).unwrap();
    assert!(matches!(res, SharpenRunResult::StoppedAtCap { .. }), "{res:?}");
}

#[test]
fn x6_rule_p1_stage_gate_revise_reset_and_cli_session_route_consistency() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // 1. CLI-bound Explore session must persist `route: "claude-cli"` and refuse Sharpen even if
    // the caller passes an API provider override in the request body.
    let cli_create = serde_json::to_vec(&json!({
        "id": "s-x6-cli-bound",
        "mode": "explore",
        "text": IDEA_TEXT,
        "provider": "claude-cli",
        "offline": true,
        "queries": []
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &cli_create);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));
    let created: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(created["route"], "claude-cli");

    let override_req = serde_json::to_vec(&json!({ "provider": "anthropic" })).unwrap();
    let (st, _, raw) =
        raw_http(port, "POST", "/api/sessions/s-x6-cli-bound/idea/sharpen", &auth, &override_req);
    assert_eq!(st, 400);
    assert!(String::from_utf8_lossy(&raw).contains(CLI_ROUTE_UNSUPPORTED_MSG));

    // 2. Normal Explore session: run Sharpen once, then verify Rule P1 rejects a second Sharpen
    // BEFORE appending any `idea.stage(sharpen, requested)` or calling the provider.
    let ok_create = serde_json::to_vec(&json!({
        "id": "s-x6-p1-gate",
        "mode": "explore",
        "text": IDEA_TEXT,
        "offline": true,
        "queries": []
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions", &auth, &ok_create);
    assert_eq!(st, 200);

    let ledger = Ledger::open(&data_dir).unwrap();
    let t1 = ScriptedTransport::new(vec![tool_resp("tu-1", "record_idea_brief", valid_sharpen_input())]);
    run_sharpen_with_transport(&ledger, "s-x6-p1-gate", &t1, "m", None, |_, _| {}).unwrap();
    let events_after_first = ledger.read_session("s-x6-p1-gate").unwrap().len();

    let t_empty = ScriptedTransport::new(vec![]);
    let err = run_sharpen_with_transport(&ledger, "s-x6-p1-gate", &t_empty, "m", None, |_, _| {})
        .unwrap_err();
    assert!(err.contains("outside stage"), "{err}");
    assert_eq!(ledger.read_session("s-x6-p1-gate").unwrap().len(), events_after_first);

    // 3. Edited perspectives missing `other_names` must be rejected by `/idea/answer`.
    let bad_ans = serde_json::to_vec(&json!({
        "perspectives": [
            { "id": "p-1", "kind": "exact", "question": "Q1", "queries": [{ "provider": "arxiv", "q": "Krylov" }] },
            { "id": "p-2", "kind": "method", "question": "Q2", "queries": [{ "provider": "inspire", "q": "Lanczos" }] }
        ]
    }))
    .unwrap();
    let (st, _, raw) =
        raw_http(port, "POST", "/api/sessions/s-x6-p1-gate/idea/answer", &auth, &bad_ans);
    assert_eq!(st, 400, "{}", String::from_utf8_lossy(&raw));

    // 4. Revising the idea resets stale `sharpened_idea` to null until Sharpen runs on rev 2.
    let rev_body = serde_json::to_vec(&json!({
        "text": format!("{IDEA_TEXT} Focus on projective measurements.")
    }))
    .unwrap();
    let (st, _, raw) =
        raw_http(port, "POST", "/api/sessions/s-x6-p1-gate/idea/revise", &auth, &rev_body);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));
    let rev_json: Value = serde_json::from_slice(&raw).unwrap();
    assert!(rev_json["idea"]["sharpened_idea"].is_null());

    srv.shutdown();
}

