//! Regression tests for X16 audit findings (C1, C2, H4, M2, M4, L3).
//!
//! Loopback mocks only; zero real network calls, zero vendor CLIs, zero third-party paper text.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::idea::{
    append_explore_session_start, budget_snapshot, build_idea_statement, IdeaPreset,
    EVENT_IDEA_CANDIDATES, EVENT_IDEA_CITATION_NET, EVENT_IDEA_MAP, EVENT_IDEA_SEARCH_FINISHED,
    EVENT_IDEA_SEARCH_PLANNED, EVENT_IDEA_STAGE, EVENT_IDEA_TRIAGE,
};
use desktop_server::idea_round2::{
    reader_calls_by_paper, run_round2_with_router, Round2Request, Round2RunResult,
};
use desktop_server::routes_idea_net::{
    run_cited_since_with_router, CitedSinceRequest, NetBatchOutcome,
};
use desktop_server::{idea_view, start_desktop_server};
use ledger::Ledger;
use research_tools::idea_tools::{idea_search_def, idea_search_handler_with_gate_and_config};
use research_tools::lit_providers::LitProvider;
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::policy_preset::research_policy;
use research_tools::lines_sent::CONTEXT_SUPPLIED_EVENT;
use research_tools::{LiteratureEndpoints, ResearchEnv};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tools::{ApprovalReply, Approver, Registry, Router};

const IDEA_TEXT: &str =
    "Use Krylov complexity growth b_n ~ alpha*n in the volume-law phase of a monitored Majorana chain.";

struct AlwaysAllowApprover;

impl Approver for AlwaysAllowApprover {
    fn decide(&self, req: &tools::ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "audit-test".into(),
        }
    }
}

fn spawn_mock_lit(status_500: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_c = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            hits_c.fetch_add(1, Ordering::SeqCst);
            let resp = if status_500 {
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 5\r\nConnection: close\r\n\r\nerror"
                    .to_string()
            } else {
                let body = json!({
                    "hits": {
                        "hits": [{
                            "id": "9001001",
                            "metadata": {
                                "titles": [{ "title": "Synthetic Post-Map Citation" }],
                                "authors": [{ "full_name": "Example, A." }],
                                "arxiv_eprints": [{ "value": "2505.00011" }],
                                "preprint_date": "2025-05-01",
                                "citation_count": 3
                            }
                        }]
                    }
                })
                .to_string();
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
            };
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), hits)
}

fn make_router(data_dir: &std::path::Path, lit_base: &str, sid: &str) -> Router {
    let mut env = ResearchEnv::new(data_dir);
    env.network_endpoints = LiteratureEndpoints::loopback_mock(lit_base).unwrap();
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let mut reg = Registry::new();
    reg.register(
        idea_search_def(),
        idea_search_handler_with_gate_and_config(Arc::new(env), Some(gate), None),
    );
    Router::new(
        reg,
        research_policy(),
        Ledger::open(data_dir).unwrap(),
        sid,
        Arc::new(AlwaysAllowApprover),
    )
}

#[test]
fn audit_c1_post_map_cited_since_and_round2_do_not_corrupt_round1_stage_or_next_stage() {
    let (lit_base, _) = spawn_mock_lit(false);
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-audit-c1";
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &["arxiv:2401.00001".into()]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Thorough, false, &stmt).unwrap();

    // Seed a completed Round-1 Search -> Triage -> Map workflow.
    ledger.append(sid, EVENT_IDEA_SEARCH_PLANNED, 1, &json!({
        "round": 1,
        "origin": "round1",
        "queries": [{ "qid": "q-r1", "provider": "inspire", "query": "Krylov Majorana" }]
    })).unwrap();
    ledger.append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": "search", "status": "done", "note": "r1 done" })).unwrap();
    ledger.append(sid, EVENT_IDEA_CANDIDATES, 1, &json!({
        "round": 1,
        "candidates": [{ "key": "arxiv:2401.00001", "title": "Seed Paper", "year": 2024 }]
    })).unwrap();
    ledger.append(sid, EVENT_IDEA_TRIAGE, 1, &json!({
        "items": [{ "key": "arxiv:2401.00001", "relevance": "core" }]
    })).unwrap();
    ledger.append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": "triage", "status": "done" })).unwrap();
    ledger.append(sid, EVENT_IDEA_MAP, 1, &json!({
        "idea_rev": 1,
        "nodes": [{
            "id": "q-1", "kind": "open_question", "label": "Open Q", "statement": "Still open.",
            "evidence": ["xe-1"], "papers": ["arxiv:2401.00001"], "stated_year": 2024,
            "later_work_checked": false, "status": "active"
        }],
        "edges": [], "overlaps": [], "dropped": []
    })).unwrap();

    let before = idea_view::view(&ledger.read_session(sid).unwrap(), false);
    let next_before = before["next_stage"].clone();
    let plan_before = before["search_plan"].clone();
    let stage_search_before = before["stages"]["search"].clone();
    assert_ne!(next_before, "triage", "post-triage session should not want triage before cited-since");

    let router = make_router(&data_dir, &lit_base, sid);
    let req = CitedSinceRequest {
        target_key: PaperKey::parse("arxiv:2401.00001").unwrap(),
        node_id: Some("q-1".into()),
        providers: vec![LitProvider::Inspire],
        since_year: Some(2024),
        is_open_question: true,
        max_results: 5,
    };
    let out = run_cited_since_with_router(&data_dir, &ledger, &router, sid, &req, None, |_, _| {}).unwrap();
    assert!(matches!(out, NetBatchOutcome::Finished { executed_queries: 1, .. }), "{out:?}");

    let r2 = run_round2_with_router(
        &data_dir,
        &ledger,
        &router,
        sid,
        &Round2Request { providers: vec![LitProvider::Inspire], max_results: 5, allow_non_thorough: false },
        None,
        |_, _| {},
    )
    .unwrap();
    assert!(matches!(r2, Round2RunResult::Finished { .. }), "{r2:?}");

    let after = idea_view::view(&ledger.read_session(sid).unwrap(), false);
    assert_eq!(after["next_stage"], next_before, "C1: next_stage must not regress to triage");
    assert_eq!(after["search_plan"], plan_before, "C1: Round-1 search_plan must not be overwritten");
    assert_eq!(after["stages"]["search"], stage_search_before, "C1: Round-1 search stage must not flip");
}

#[test]
fn audit_c2_and_l3_round2_enforces_calls_per_paper_via_f4_context_supplied_and_screened_80() {
    let (lit_base, lit_hits) = spawn_mock_lit(false);
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &["arxiv:2401.00001".into()]).unwrap();
    let req = Round2Request { providers: vec![LitProvider::Inspire], max_results: 5, allow_non_thorough: false };

    // 1. C2: 4 F4 `context.supplied` events with X10 caller `reader:<sid>:<paper_key>` -> calls_per_paper cap!
    let sid_cpp = "s-audit-cpp";
    append_explore_session_start(&ledger, sid_cpp, IdeaPreset::Thorough, false, &stmt).unwrap();
    for i in 1..=4 {
        ledger.append(sid_cpp, CONTEXT_SUPPLIED_EVENT, 1, &json!({
            "caller": format!("reader:{sid_cpp}:arxiv:2401.00001"),
            "chunk_id": format!("c-{i}")
        })).unwrap();
    }
    let evs = ledger.read_session(sid_cpp).unwrap();
    assert_eq!(reader_calls_by_paper(&evs).get("arxiv:2401.00001"), Some(&4));

    let hits_before = lit_hits.load(Ordering::SeqCst);
    let r_cpp = make_router(&data_dir, &lit_base, sid_cpp);
    let out_cpp = run_round2_with_router(&data_dir, &ledger, &r_cpp, sid_cpp, &req, None, |_, _| {}).unwrap();
    assert!(matches!(out_cpp, Round2RunResult::StoppedAtCap { cap: "calls_per_paper", .. }), "{out_cpp:?}");
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_before);
    let post_events = ledger.read_session(sid_cpp).unwrap();
    assert!(post_events.iter().any(|e| e.kind == EVENT_IDEA_CITATION_NET && e.payload["kind"] == "round2_stopped_at_cap"));

    // 2. L3: 80 screened items in `idea.triage` -> screened cap!
    let sid_scr = "s-audit-scr";
    append_explore_session_start(&ledger, sid_scr, IdeaPreset::Thorough, false, &stmt).unwrap();
    let items: Vec<Value> = (1..=80)
        .map(|i| json!({ "key": format!("arxiv:2401.{i:05}"), "relevance": "background" }))
        .collect();
    ledger.append(sid_scr, EVENT_IDEA_TRIAGE, 1, &json!({ "items": items })).unwrap();
    let r_scr = make_router(&data_dir, &lit_base, sid_scr);
    let out_scr = run_round2_with_router(&data_dir, &ledger, &r_scr, sid_scr, &req, None, |_, _| {}).unwrap();
    assert!(matches!(out_scr, Round2RunResult::StoppedAtCap { cap: "screened", .. }), "{out_scr:?}");
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_before);
}

#[test]
fn audit_h4_m2_m4_provider_validation_unsupported_budget_zero_ok_seeds_and_strict_http() {
    let (lit_base_500, hits_500) = spawn_mock_lit(true);
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &["arxiv:2401.00001".into()]).unwrap();

    // H4: `run_cited_since_with_router` rejects non-citation provider (arXiv) before network/budget.
    let sid = "s-audit-h4";
    append_explore_session_start(&ledger, sid, IdeaPreset::Quick, false, &stmt).unwrap();
    let router = make_router(&data_dir, &lit_base_500, sid);
    let bad_req = CitedSinceRequest {
        target_key: PaperKey::parse("arxiv:2401.00001").unwrap(),
        node_id: None,
        providers: vec![LitProvider::Arxiv],
        since_year: Some(2024),
        is_open_question: false,
        max_results: 5,
    };
    assert!(run_cited_since_with_router(&data_dir, &ledger, &router, sid, &bad_req, None, |_, _| {}).is_err());
    assert_eq!(hits_500.load(Ordering::SeqCst), 0);

    // H4: `"unsupported"` search result rows must NOT count against `queries_used`.
    ledger.append(sid, EVENT_IDEA_SEARCH_FINISHED, 1, &json!({
        "round": 1,
        "results": [
            { "qid": "q-1", "provider": "arxiv", "status": "unsupported", "count": 0 },
            { "qid": "q-2", "provider": "inspire", "status": "ok", "count": 2 }
        ]
    })).unwrap();
    let snap = budget_snapshot(&ledger.read_session(sid).unwrap(), false);
    assert_eq!(snap.queries_used, 1, "unsupported must not consume query budget");

    // M2: When citation lookup fails (HTTP 500 -> ok_count == 0), seeds & hops must stay 0!
    let fail_req = CitedSinceRequest {
        target_key: PaperKey::parse("arxiv:2401.00001").unwrap(),
        node_id: None,
        providers: vec![LitProvider::Inspire],
        since_year: Some(2024),
        is_open_question: false,
        max_results: 5,
    };
    let res_fail = run_cited_since_with_router(&data_dir, &ledger, &router, sid, &fail_req, None, |_, _| {}).unwrap();
    assert!(matches!(res_fail, NetBatchOutcome::Finished { executed_queries: 1, .. }), "{res_fail:?}");
    let view = idea_view::view(&ledger.read_session(sid).unwrap(), false);
    assert_eq!(view["coverage"]["citation_expansion"]["seeds"], 0, "M2: failed lookup must not inflate seeds");
    assert_eq!(view["coverage"]["citation_expansion"]["hops"], 0, "M2: failed lookup must not inflate hops");

    // M4: HTTP route rejects non-allowlisted provider (`theoremsearch`) and ambiguous `{}` on `/idea/more`.
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, Some(&lit_base_500))).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", token.as_str()), ("Content-Type", "application/json")];

    let (st_prov, _, raw_prov) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/idea/cited-since"),
        &auth,
        br#"{"paper_key":"arxiv:2401.00001","providers":["theoremsearch"]}"#,
    );
    assert_eq!(st_prov, 400, "M4: non-allowlisted provider must return 400: {}", String::from_utf8_lossy(&raw_prov));

    let (st_more, _, raw_more) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/idea/more"),
        &auth,
        b"{}",
    );
    assert_eq!(st_more, 400, "M4: ambiguous {{}} on non-Thorough pre-map /idea/more must return 400: {}", String::from_utf8_lossy(&raw_more));

    srv.shutdown();
}
