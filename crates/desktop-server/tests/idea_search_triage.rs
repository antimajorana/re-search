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

fn spawn_mock_lit_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            hits_clone.fetch_add(1, Ordering::SeqCst);
            let (ct, body) = if req.contains("/api/query") {
                (
                    "application/atom+xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2501.01234v1</id>
    <title>Synthetic Krylov Complexity in Monitored Majorana Chains</title>
    <summary>We show that b_n grows linearly in the volume-law phase and saturates in the area-law phase.</summary>
    <published>2025-01-10T00:00:00Z</published>
    <author><name>A. Author</name></author>
  </entry>
</feed>"#
                        .to_string(),
                )
            } else {
                (
                    "application/json",
                    json!({
                        "hits": {
                            "total": 1,
                            "hits": [{
                                "id": "9900001",
                                "metadata": {
                                    "titles": [{ "title": "Synthetic Spread Complexity and Entanglement" }],
                                    "authors": [{ "full_name": "B. Author" }],
                                    "earliest_date": "2024-06-01",
                                    "arxiv_eprints": [{ "value": "2406.05432" }],
                                    "abstracts": [{ "value": "Spread complexity diagnoses measurement-induced transitions in free-fermion chains." }],
                                    "citation_count": 12
                                }
                            }]
                        }
                    })
                    .to_string(),
                )
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (format!("http://127.0.0.1:{}", addr.port()), hits)
}

fn wait_for_pending_approval(port: u16, token: &str, sid: &str) -> (String, String) {
    let auth = [("X-ReSearch-Token", token)];
    for _ in 0..100 {
        let (st, _, raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/approvals"), &auth, b"");
        if st == 200 {
            let v: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
            if let Some(first) = v["pending"].as_array().and_then(|a| a.first()) {
                return (
                    first["call_id"].as_str().unwrap().to_string(),
                    first["digest"].as_str().unwrap().to_string(),
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for UiApprover pending approval on {sid}");
}

#[test]
fn x7_plan_search_enforces_router_approval_denial_session_grant_rule_p5_p7_and_f12_coverage() {
    use std::sync::atomic::Ordering;
    let (lit_base, lit_hits) = spawn_mock_lit_server();
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, Some(&lit_base))).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let auth = [
        ("X-ReSearch-Token", token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // Disable desktop auto-approve so real interactive UiApprover prompts + denial + session grant run.
    let settings_body = serde_json::to_vec(&json!({ "auto_approve_tools": false })).unwrap();
    let (st, _, raw) = raw_http(port, "PUT", "/api/settings/providers", &auth, &settings_body);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));

    // 1. Preview & create online Explore session with a known seed paper (for F12 recall).
    let prev_body = serde_json::to_vec(&json!({
        "text": IDEA_TEXT,
        "seeds": ["arxiv:2501.01234"]
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/idea/preview", &auth, &prev_body);
    assert_eq!(st, 200);
    let preview: Value = serde_json::from_slice(&raw).unwrap();
    let first_q = &preview["first_queries"].as_array().unwrap()[0];
    let create_body = serde_json::to_vec(&json!({
        "id": "s-x7-flow",
        "mode": "explore",
        "text": IDEA_TEXT,
        "seeds": ["arxiv:2501.01234"],
        "preset": "standard",
        "offline": false,
        "queries": [{ "provider": first_q["provider"], "q": first_q["q"] }]
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &create_body);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));

    // 2. Run Sharpen, then call `/idea/plan` -> SearchPlan + plan_approval grant descriptor.
    let ledger = Ledger::open(&data_dir).unwrap();
    let t = ScriptedTransport::new(vec![tool_resp("tu-1", "record_idea_brief", valid_sharpen_input())]);
    run_sharpen_with_transport(&ledger, "s-x7-flow", &t, "m", None, |_, _| {}).unwrap();

    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-x7-flow/idea/plan", &auth, b"{}");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));
    let plan_resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(plan_resp["idea"]["plan_approval"]["grants"][0]["kind"], "session_capability");
    assert_eq!(plan_resp["idea"]["plan_approval"]["grants"][0]["effect"], "network-read");

    // 3. Rule P5: `/idea/search` blocks on UiApprover; DENYING executes zero literature hits.
    let hits_before_deny = lit_hits.load(Ordering::SeqCst);
    let tok_bg = token.clone();
    let bg_deny = std::thread::spawn(move || {
        let a = [("X-ReSearch-Token", tok_bg.as_str()), ("Content-Type", "application/json")];
        raw_http(port, "POST", "/api/sessions/s-x7-flow/idea/search", &a, b"{}")
    });
    let (call_id, digest) = wait_for_pending_approval(port, &token, "s-x7-flow");
    let deny_body = serde_json::to_vec(&json!({ "digest": digest, "approved": false })).unwrap();
    let (st, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/s-x7-flow/approvals/{call_id}"),
        &auth,
        &deny_body,
    );
    assert_eq!(st, 200);
    let (st_deny, _, raw_deny) = bg_deny.join().unwrap();
    assert_eq!(st_deny, 200, "{}", String::from_utf8_lossy(&raw_deny));
    let deny_json: Value = serde_json::from_slice(&raw_deny).unwrap();
    assert_eq!(deny_json["ok"], false);
    assert_eq!(deny_json["denied"], true);
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_before_deny);

    // 4. APPROVING with `approve_for_session: true` runs `idea_search` through Router::try_call,
    // updates F12 coverage + known_paper_recall, and binds session network-read grant.
    let tok_bg2 = token.clone();
    let bg_ok = std::thread::spawn(move || {
        let a = [("X-ReSearch-Token", tok_bg2.as_str()), ("Content-Type", "application/json")];
        raw_http(port, "POST", "/api/sessions/s-x7-flow/idea/search", &a, b"{}")
    });
    let (call_id2, digest2) = wait_for_pending_approval(port, &token, "s-x7-flow");
    let ok_body = serde_json::to_vec(&json!({
        "digest": digest2,
        "approved": true,
        "approve_for_session": true
    }))
    .unwrap();
    let (st, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/s-x7-flow/approvals/{call_id2}"),
        &auth,
        &ok_body,
    );
    assert_eq!(st, 200);
    let (st_ok, _, raw_ok) = bg_ok.join().unwrap();
    assert_eq!(st_ok, 200, "{}", String::from_utf8_lossy(&raw_ok));
    let ok_json: Value = serde_json::from_slice(&raw_ok).unwrap();
    assert_eq!(ok_json["ok"], true);
    assert!(ok_json["idea"]["candidates"].as_array().unwrap().len() >= 2);
    assert!(ok_json["idea"]["coverage"]["gate"]["answered_external_services"].as_u64().unwrap() >= 2);
    assert_eq!(ok_json["idea"]["where_i_looked"]["known_paper_recall"]["given"], json!(["arxiv:2501.01234"]));
    assert_eq!(ok_json["idea"]["where_i_looked"]["known_paper_recall"]["found_by_search"], json!(["arxiv:2501.01234"]));
    assert_eq!(ok_json["idea"]["where_i_looked"]["known_paper_recall"]["missed"], json!([]));

    // 5. Session network-read grant is active: retrying one provider executes immediately without
    // prompting, preserves original qids, keeps INSPIRE coverage intact, and never corrupts abstract_ref.
    let retry_body = serde_json::to_vec(&json!({ "provider": "arxiv" })).unwrap();
    let (st_retry, _, raw_retry) =
        raw_http(port, "POST", "/api/sessions/s-x7-flow/idea/search", &auth, &retry_body);
    assert_eq!(st_retry, 200, "{}", String::from_utf8_lossy(&raw_retry));
    let retry_json: Value = serde_json::from_slice(&raw_retry).unwrap();
    assert_eq!(retry_json["ok"], true);
    let inspire_prov = retry_json["idea"]["where_i_looked"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "inspire")
        .unwrap();
    assert_eq!(inspire_prov["state"], "ok");
    assert!(inspire_prov["queries_ok"].as_u64().unwrap() >= 1);
    for c in retry_json["idea"]["candidates"].as_array().unwrap() {
        if let Some(aref) = c.get("abstract_ref").and_then(Value::as_object) {
            assert_ne!(aref.get("provider").and_then(Value::as_str), Some("prior"));
        }
    }

    // 5b. X8 Triage runs cleanly after Search, emitting verbatim `evidence.abstract` (`xa-<seq>`).
    let triage_items: Vec<Value> = retry_json["idea"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let k = c["key"].as_str().unwrap();
            if k == "arxiv:2501.01234" {
                json!({
                    "key": k, "relevance": "core", "subclaim_ids": ["sc-1"],
                    "why": "Directly tests Lanczos growth in monitored Majorana chains.",
                    "abstract_quote": "b_n grows linearly in the volume-law phase"
                })
            } else {
                json!({
                    "key": k, "relevance": "background", "subclaim_ids": [],
                    "why": "Secondary search hit.", "abstract_quote": null
                })
            }
        })
        .collect();
    let t_triage = ScriptedTransport::new(vec![tool_resp(
        "tu-triage",
        "record_idea_triage",
        json!({ "items": triage_items, "reading_list": ["arxiv:2501.01234"] }),
    )]);
    let triage_res = desktop_server::idea_exchanges_triage::run_triage_with_transport(
        &data_dir, &ledger, "s-x7-flow", &t_triage, "m", None, |_, _| {},
    )
    .expect("X8 triage should succeed after X7 search");
    match triage_res {
        desktop_server::idea_exchanges_triage::TriageRunResult::Finished { outcome, .. } => {
            assert_eq!(outcome.abstract_evidence_ids.len(), 1);
            assert!(outcome.abstract_evidence_ids[0].starts_with("xa-"));
        }
        other => panic!("expected Finished triage, got {other:?}"),
    }

    // 6. Calling `/idea/stop` while a fresh session's `/idea/search` is waiting on UiApprover
    // immediately unblocks UiApprover (Carbon Finding 4).
    let stop_create = serde_json::to_vec(&json!({
        "id": "s-x7-stop",
        "mode": "explore",
        "text": IDEA_TEXT,
        "offline": false,
        "queries": [{ "provider": first_q["provider"], "q": first_q["q"] }]
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions", &auth, &stop_create).0, 200);
    let plan_q = serde_json::to_vec(&json!({
        "queries": [{ "qid": "q-stop-1", "provider": "arxiv", "q": "Krylov Majorana" }]
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions/s-x7-stop/idea/plan", &auth, &plan_q).0, 200);
    let tok_stop = token.clone();
    let bg_stop = std::thread::spawn(move || {
        let a = [("X-ReSearch-Token", tok_stop.as_str()), ("Content-Type", "application/json")];
        raw_http(port, "POST", "/api/sessions/s-x7-stop/idea/search", &a, b"{}")
    });
    let _ = wait_for_pending_approval(port, &token, "s-x7-stop");
    let (st_stop, _, _) = raw_http(port, "POST", "/api/sessions/s-x7-stop/idea/stop", &auth, b"{}");
    assert_eq!(st_stop, 200);
    let (st_bg, _, raw_bg) = bg_stop.join().unwrap();
    assert_eq!(st_bg, 200, "{}", String::from_utf8_lossy(&raw_bg));
    let bg_json: Value = serde_json::from_slice(&raw_bg).unwrap();
    assert_eq!(bg_json["ok"], false);

    srv.shutdown();
}


