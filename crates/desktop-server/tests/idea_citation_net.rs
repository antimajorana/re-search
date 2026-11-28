//! Integration tests for Explore citation expansion, `"Who cited this since"`,
//! `"Search more about this node"`, and Thorough Round 2 (package X16).
//!
//! Loopback mocks only; zero real network calls, zero vendor CLIs, zero third-party paper text.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_BRIEF,
    EVENT_IDEA_CITATION_NET, EVENT_IDEA_EXCHANGE, EVENT_IDEA_MAP, EVENT_IDEA_SEARCH_FINISHED,
    EVENT_IDEA_SEARCH_PLANNED, EVENT_IDEA_VERDICT,
};
use desktop_server::idea_round2::{
    find_thin_statements, run_round2_with_router, Round2Request, Round2RunResult,
};
use desktop_server::routes_idea_net::{
    run_cited_since_with_router, CitedSinceRequest, NetBatchOutcome, CITED_SINCE_NOT_ANSWERED_NOTE,
};
use desktop_server::{idea_view, start_desktop_server};
use ledger::Ledger;
use research_tools::idea_tools::{idea_search_def, idea_search_handler_with_gate_and_config};
use research_tools::lit_providers::LitProvider;
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::policy_preset::research_policy;
use research_tools::prior_work_wording::contains_forbidden_prior_work_word;
use research_tools::run_budget::{count_model_calls, RunScope};
use research_tools::{LiteratureEndpoints, ResearchEnv};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
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
            by: "test-approver".into(),
        }
    }
}

fn spawn_citation_mock_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_c = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            hits_c.fetch_add(1, Ordering::SeqCst);

            let (ct, body) = if req.contains("/api/query") {
                (
                    "application/atom+xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2401.00001v1</id>
    <title>Synthetic Krylov Complexity Seed Paper</title>
    <summary>We study Lanczos growth in a monitored Majorana chain.</summary>
    <published>2024-01-15T00:00:00Z</published>
    <author><name>Ada Example</name></author>
  </entry>
</feed>"#
                        .to_string(),
                )
            } else if req.contains("/graph/v1/paper/") && req.contains("/citations") {
                (
                    "application/json",
                    json!({
                        "data": [
                            {
                                "citingPaper": {
                                    "paperId": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                    "title": "Synthetic S2 Follow-up on Krylov Saturation",
                                    "authors": [{ "name": "Grace Example" }],
                                    "year": 2025,
                                    "abstract": "Follow-up analysis of monitored Krylov growth.",
                                    "citationCount": 7,
                                    "externalIds": { "ArXiv": "2502.00077" }
                                }
                            }
                        ]
                    })
                    .to_string(),
                )
            } else if req.contains("refersto") {
                (
                    "application/json",
                    json!({
                        "hits": {
                            "total": 2,
                            "hits": [
                                {
                                    "id": "9910001",
                                    "metadata": {
                                        "titles": [{ "title": "Synthetic 2025 Follow-up on Monitored Krylov" }],
                                        "authors": [{ "full_name": "B. Followup" }],
                                        "preprint_date": "2025-05-01",
                                        "arxiv_eprints": [{ "value": "2505.00011" }],
                                        "abstracts": [{ "value": "Extends monitored Krylov complexity to interacting chains." }],
                                        "citation_count": 9
                                    }
                                },
                                {
                                    "id": "9910002",
                                    "metadata": {
                                        "titles": [{ "title": "Synthetic 2023 Earlier Note" }],
                                        "authors": [{ "full_name": "C. Earlier" }],
                                        "preprint_date": "2023-04-01",
                                        "arxiv_eprints": [{ "value": "2304.00022" }],
                                        "abstracts": [{ "value": "Earlier comment on Lanczos coefficients." }],
                                        "citation_count": 5
                                    }
                                }
                            ]
                        }
                    })
                    .to_string(),
                )
            } else {
                // INSPIRE `citedby:` (outgoing references) or general search.
                (
                    "application/json",
                    json!({
                        "hits": {
                            "total": 1,
                            "hits": [{
                                "id": "9910099",
                                "metadata": {
                                    "titles": [{ "title": "Synthetic Foundational Reference Paper" }],
                                    "authors": [{ "full_name": "D. Reference" }],
                                    "preprint_date": "2021-03-01",
                                    "arxiv_eprints": [{ "value": "2103.00099" }],
                                    "abstracts": [{ "value": "Foundational operator growth formalism." }],
                                    "citation_count": 42
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
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for UiApprover pending approval on {sid}");
}

#[test]
fn x16_cited_since_enforces_auth_router_approval_zero_model_calls_and_node_more() {
    let (lit_base, lit_hits) = spawn_citation_mock_server();
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, Some(&lit_base))).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let auth = [
        ("X-ReSearch-Token", token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // Disable auto-approve so interactive UiApprover gates `/idea/cited-since` (Rule P5).
    let (st, _, raw) = raw_http(
        port,
        "PUT",
        "/api/settings/providers",
        &auth,
        br#"{"auto_approve_tools":false}"#,
    );
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));

    // Create online Explore session via `/api/idea/preview` + one Rule-P4 user-shown query.
    let prev_body = serde_json::to_vec(&json!({
        "text": IDEA_TEXT,
        "seeds": ["arxiv:2401.00001"]
    }))
    .unwrap();
    let (st_p, _, raw_p) = raw_http(port, "POST", "/api/idea/preview", &auth, &prev_body);
    assert_eq!(st_p, 200, "{}", String::from_utf8_lossy(&raw_p));
    let preview: Value = serde_json::from_slice(&raw_p).unwrap();
    let first_q = &preview["first_queries"].as_array().unwrap()[0];
    let create_body = serde_json::to_vec(&json!({
        "id": "s-x16-http",
        "mode": "explore",
        "text": IDEA_TEXT,
        "seeds": ["arxiv:2401.00001"],
        "preset": "thorough",
        "offline": false,
        "queries": [{ "provider": first_q["provider"], "q": first_q["q"] }]
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &create_body);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));

    // 1. AuthGuard must reject unauthenticated calls to `/idea/cited-since` and `/idea/more`.
    let (unauth_st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-x16-http/idea/cited-since",
        &[("Content-Type", "application/json")],
        br#"{"paper_key":"arxiv:2401.00001"}"#,
    );
    assert_eq!(unauth_st, 401);
    let (unauth_more, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-x16-http/idea/more",
        &[("Content-Type", "application/json")],
        br#"{"seeds":["arxiv:2401.00001"]}"#,
    );
    assert_eq!(unauth_more, 401);

    // 2. Denying the Router/UiApprover prompt executes ZERO network requests and ZERO model calls.
    let hits_before = lit_hits.load(Ordering::SeqCst);
    let tok_bg = token.clone();
    let bg_deny = std::thread::spawn(move || {
        let h = [("X-ReSearch-Token", tok_bg.as_str()), ("Content-Type", "application/json")];
        raw_http(
            port,
            "POST",
            "/api/sessions/s-x16-http/idea/cited-since",
            &h,
            br#"{"paper_key":"arxiv:2401.00001","provider":"inspire"}"#,
        )
    });
    let (call_id, digest) = wait_for_pending_approval(port, &token, "s-x16-http");
    let deny_body = serde_json::to_vec(&json!({ "digest": digest, "approved": false })).unwrap();
    let (st, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/s-x16-http/approvals/{call_id}"),
        &auth,
        &deny_body,
    );
    assert_eq!(st, 200);
    let (st_d, _, raw_d) = bg_deny.join().unwrap();
    assert_eq!(st_d, 200, "{}", String::from_utf8_lossy(&raw_d));
    let deny_json: Value = serde_json::from_slice(&raw_d).unwrap();
    assert_eq!(deny_json["ok"], false);
    assert_eq!(deny_json["denied"], true);
    assert_eq!(deny_json["model_calls"], 0);
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_before);

    // 3. Approving for session executes `idea_search` (`relation: cited_by`), records `idea.citation_net`,
    //    and uses ZERO model calls.
    let ledger = Ledger::open(&data_dir).unwrap();
    // Seed an active map with an `open_question` node so we also verify the `"cited since" != "answered"` note!
    ledger
        .append(
            "s-x16-http",
            EVENT_IDEA_MAP,
            1,
            &json!({
                "idea_rev": 1,
                "nodes": [{
                    "id": "q-1",
                    "kind": "open_question",
                    "rev": 1,
                    "label": "Krylov transition in interacting Majorana chains",
                    "statement": "Whether b_n remains a sharp order parameter under interactions is open.",
                    "evidence": ["xe-99"],
                    "relation_to_idea": "same",
                    "subclaim_ids": ["sc-1"],
                    "papers": ["arxiv:2401.00001"],
                    "stated_year": 2024,
                    "later_work_checked": false,
                    "status": "active"
                }],
                "edges": [],
                "overlaps": [],
                "dropped": []
            }),
        )
        .unwrap();

    let tok_bg2 = token.clone();
    let bg_allow = std::thread::spawn(move || {
        let h = [("X-ReSearch-Token", tok_bg2.as_str()), ("Content-Type", "application/json")];
        raw_http(
            port,
            "POST",
            "/api/sessions/s-x16-http/idea/cited-since",
            &h,
            br#"{"node_id":"q-1","provider":"inspire"}"#,
        )
    });
    let (call_id2, digest2) = wait_for_pending_approval(port, &token, "s-x16-http");
    let allow_body = serde_json::to_vec(&json!({
        "digest": digest2,
        "approved": true,
        "approve_for_session": true
    }))
    .unwrap();
    let (st, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/s-x16-http/approvals/{call_id2}"),
        &auth,
        &allow_body,
    );
    assert_eq!(st, 200);
    let (st_ok, _, raw_ok) = bg_allow.join().unwrap();
    assert_eq!(st_ok, 200, "{}", String::from_utf8_lossy(&raw_ok));
    let ok_json: Value = serde_json::from_slice(&raw_ok).unwrap();
    assert_eq!(ok_json["ok"], true);
    assert_eq!(ok_json["model_calls"], 0);
    assert_eq!(ok_json["target_key"], "arxiv:2401.00001");
    assert_eq!(ok_json["is_open_question"], true);
    assert_eq!(ok_json["note"], CITED_SINCE_NOT_ANSWERED_NOTE);
    assert_eq!(ok_json["citing_papers"].as_array().unwrap().len(), 1);
    assert_eq!(ok_json["citing_papers"][0]["year"], 2025);
    assert_eq!(ok_json["citation_net"]["edges"].as_array().unwrap().len(), 2);
    assert!(lit_hits.load(Ordering::SeqCst) > hits_before);

    // Verify ledger invariants: origin == "app", tool.request went through host:idea:<session>, 0 model calls,
    // and Round-1 search_planned / stage events are NOT overwritten (Audit C1).
    let events = ledger.read_session("s-x16-http").unwrap();
    assert_eq!(count_model_calls(&events, RunScope::All), 0);
    assert!(events.iter().any(|e| {
        e.kind == EVENT_IDEA_SEARCH_FINISHED && e.payload["origin"] == "app"
    }));
    assert!(!events.iter().any(|e| {
        e.kind == EVENT_IDEA_SEARCH_PLANNED && e.payload["origin"] == "app"
    }));
    assert!(events.iter().any(|e| {
        e.kind == "tool.request"
            && e.payload["tool"] == "idea_search"
            && e.payload["caller"] == "host:idea:s-x16-http"
    }));
    assert!(events.iter().any(|e| {
        e.kind == EVENT_IDEA_CITATION_NET && e.payload["origin"] == "app"
    }));
    assert_eq!(ok_json["idea"]["coverage"]["citation_expansion"]["seeds"], 1);
    assert_eq!(ok_json["idea"]["coverage"]["citation_expansion"]["hops"], 1);

    // 4. Session network grant now covers `POST /api/sessions/s-x16-http/idea/more` for node `q-1`.
    let (st_more, _, raw_more) = raw_http(
        port,
        "POST",
        "/api/sessions/s-x16-http/idea/more",
        &auth,
        br#"{"node_id":"q-1","provider":"inspire"}"#,
    );
    assert_eq!(st_more, 200, "{}", String::from_utf8_lossy(&raw_more));
    let more_json: Value = serde_json::from_slice(&raw_more).unwrap();
    assert_eq!(more_json["ok"], true);
    assert_eq!(more_json["model_calls"], 0);
    assert!(more_json["executed_queries"].as_u64().unwrap() >= 2);

    srv.shutdown();
}

#[test]
fn x16_cited_since_passes_f3_net_rate_gate_and_stops_at_query_cap() {
    let (lit_base, lit_hits) = spawn_citation_mock_server();
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    let sid = "s-x16-gate";
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &["arxiv:2401.00001".into()]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Quick, false, &stmt).unwrap();

    let mut env = ResearchEnv::new(&data_dir);
    env.network_endpoints = LiteratureEndpoints::loopback_mock(&lit_base).unwrap();
    let clock = Arc::new(FakeRateClock::new());
    let gate = Arc::new(NetRateGate::new(Arc::clone(&clock) as Arc<dyn research_tools::net_rate::RateClock>));

    let mut registry = Registry::new();
    registry.register(
        idea_search_def(),
        idea_search_handler_with_gate_and_config(Arc::new(env), Some(Arc::clone(&gate)), None),
    );
    let router = Router::new(
        registry,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(AlwaysAllowApprover),
    );

    // Fire 2 consecutive Semantic Scholar `cited_by` lookups (1 req/1 s limit) -> fake clock sleeps >= 1 s!
    let req = CitedSinceRequest {
        target_key: PaperKey::parse("arxiv:2401.00001").unwrap(),
        node_id: None,
        providers: vec![LitProvider::SemanticScholar],
        since_year: Some(2024),
        is_open_question: false,
        max_results: 5,
    };
    let r1 = run_cited_since_with_router(&data_dir, &ledger, &router, sid, &req, None, |_, _| {}).unwrap();
    assert!(matches!(r1, NetBatchOutcome::Finished { executed_queries: 1, .. }), "{r1:?}");
    let r2 = run_cited_since_with_router(&data_dir, &ledger, &router, sid, &req, None, |_, _| {}).unwrap();
    assert!(matches!(r2, NetBatchOutcome::Finished { executed_queries: 1, .. }), "{r2:?}");
    assert!(
        clock.total_slept() >= Duration::from_secs(1),
        "expected F3 NetRateGate to space Semantic Scholar requests by >= 1s, got {:?}",
        clock.total_slept()
    );

    // Exhaust remaining Quick query budget (8 total; 2 used so far -> run 6 more).
    for _ in 0..6 {
        let r = run_cited_since_with_router(&data_dir, &ledger, &router, sid, &req, None, |_, _| {}).unwrap();
        assert!(matches!(r, NetBatchOutcome::Finished { .. }), "{r:?}");
    }
    let hits_at_cap = lit_hits.load(Ordering::SeqCst);
    let capped = run_cited_since_with_router(&data_dir, &ledger, &router, sid, &req, None, |_, _| {}).unwrap();
    assert!(matches!(capped, NetBatchOutcome::StoppedAtCap { .. }), "{capped:?}");
    assert_eq!(
        lit_hits.load(Ordering::SeqCst),
        hits_at_cap,
        "Rule P3: stopped_at_cap must prevent any further network call"
    );
}

fn make_test_router(data_dir: &std::path::Path, lit_base: &str, sid: &str) -> Router {
    let mut env = ResearchEnv::new(data_dir);
    env.network_endpoints = LiteratureEndpoints::loopback_mock(lit_base).unwrap();
    let clock = Arc::new(FakeRateClock::new());
    let gate = Arc::new(NetRateGate::new(clock));
    let mut registry = Registry::new();
    registry.register(idea_search_def(), idea_search_handler_with_gate_and_config(Arc::new(env), Some(gate), None));
    Router::new(registry, research_policy(), Ledger::open(data_dir).unwrap(), sid, Arc::new(AlwaysAllowApprover))
}

#[test]
fn x16_round2_finds_thin_statements_updates_f12_and_passes_rule_s8() {
    let (lit_base, _) = spawn_citation_mock_server();
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-x16-r2";
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &["arxiv:2401.00001".into()]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Thorough, false, &stmt).unwrap();
    ledger.append(sid, EVENT_IDEA_BRIEF, 1, &json!({
        "subclaims": [
            { "id": "sc-1", "text": "b_n grows linearly in volume-law phase" },
            { "id": "sc-2", "text": "growth rate changes sharply at measurement transition" }
        ]
    })).unwrap();
    ledger.append(sid, EVENT_IDEA_MAP, 1, &json!({
        "idea_rev": 1,
        "nodes": [
            { "id": "q-1", "kind": "open_question", "label": "Interacting Majorana Krylov transition", "statement": "Remains open under interactions.", "evidence": ["xe-1"], "papers": ["arxiv:2401.00001"], "later_work_checked": false, "status": "active" },
            { "id": "w-1", "kind": "related_work", "label": "Abstract-only spread complexity study", "statement": "Mentions spread complexity in abstract.", "evidence": ["xa-2"], "papers": ["arxiv:2402.00002"], "status": "active" },
            { "id": "k-1", "kind": "known_result", "label": "Free-fermion Lanczos growth", "statement": "Linear growth shown in one passage.", "evidence": ["xe-3"], "papers": ["arxiv:2401.00001"], "status": "active" }
        ],
        "edges": [], "overlaps": [], "dropped": []
    })).unwrap();
    ledger.append(sid, EVENT_IDEA_VERDICT, 1, &json!({
        "verdicts": [
            { "subclaim_id": "sc-1", "status": "judged_partial" },
            { "subclaim_id": "sc-2", "status": "none_in_scope" }
        ]
    })).unwrap();

    let events_pre = ledger.read_session(sid).unwrap();
    let thin = find_thin_statements(&events_pre);
    assert_eq!(thin.len(), 4, "expected 3 thin nodes + 1 uncovered subclaim: {thin:?}");
    for t in &thin {
        assert!(contains_forbidden_prior_work_word(&t.summary).is_none(), "Rule S8: {}", t.summary);
    }

    let router = make_test_router(&data_dir, &lit_base, sid);
    let req = Round2Request { providers: vec![LitProvider::Inspire], max_results: 5, allow_non_thorough: false };
    let res = run_round2_with_router(&data_dir, &ledger, &router, sid, &req, None, |_, _| {}).unwrap();
    let Round2RunResult::Finished { executed_queries, stopped_at_cap, citation_net, .. } = res else {
        panic!("expected Round2RunResult::Finished, got {res:?}");
    };
    assert!(executed_queries >= 3 && !stopped_at_cap);
    assert!(!citation_net.unread_core_citations.is_empty());

    let updated = ledger.read_session(sid).unwrap();
    let view = idea_view::view(&updated, false);
    assert_eq!(view["coverage"]["citation_expansion"]["hops"], 1);
    assert!(view["coverage"]["citation_expansion"]["seeds"].as_u64().unwrap() >= 1);
    assert_eq!(view["where_i_looked"]["round2"]["status"], "done");
    assert_eq!(view["where_i_looked"]["round2"]["ceilings"]["max_queries"], 36);
    assert_eq!(view["where_i_looked"]["round2"]["ceilings"]["max_papers"], 12);
    assert_eq!(view["where_i_looked"]["round2"]["ceilings"]["max_model_calls"], 55);
    assert!(!view["where_i_looked"]["citation_net"]["unread_core_citations"].as_array().unwrap().is_empty());
}

#[test]
fn x16_round2_enforces_thorough_caps_36_12_55_stopped_at_cap() {
    let (lit_base, lit_hits) = spawn_citation_mock_server();
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let req = Round2Request { providers: vec![LitProvider::Inspire], max_results: 5, allow_non_thorough: false };

    // 1. Standard preset rejected unless `thorough`.
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &["arxiv:2401.00001".into()]).unwrap();
    append_explore_session_start(&ledger, "s-std", IdeaPreset::Standard, false, &stmt).unwrap();
    let r_std = make_test_router(&data_dir, &lit_base, "s-std");
    assert!(run_round2_with_router(&data_dir, &ledger, &r_std, "s-std", &req, None, |_, _| {}).is_err());

    // 2. Hard cap: 55 model calls -> stopped_at_cap with 0 network hits.
    append_explore_session_start(&ledger, "s-cap-55", IdeaPreset::Thorough, false, &stmt).unwrap();
    for i in 1..=55 {
        ledger.append("s-cap-55", EVENT_IDEA_EXCHANGE, 1, &json!({ "exchange": "read", "attempt": i })).unwrap();
    }
    let hits_before = lit_hits.load(Ordering::SeqCst);
    let r55 = make_test_router(&data_dir, &lit_base, "s-cap-55");
    let out55 = run_round2_with_router(&data_dir, &ledger, &r55, "s-cap-55", &req, None, |_, _| {}).unwrap();
    assert!(matches!(out55, Round2RunResult::StoppedAtCap { cap: "model_calls", .. }), "{out55:?}");
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_before);

    // 3. Hard cap: 12 fetched papers -> stopped_at_cap with 0 network hits.
    append_explore_session_start(&ledger, "s-cap-12", IdeaPreset::Thorough, false, &stmt).unwrap();
    for i in 1..=12 {
        ledger.append("s-cap-12", "source.ingested", 1, &json!({ "source_id": format!("src-{i}"), "origin": format!("https://arxiv.org/e-print/2401.{i:05}"), "sha256": format!("{i:064x}") })).unwrap();
    }
    let r12 = make_test_router(&data_dir, &lit_base, "s-cap-12");
    let out12 = run_round2_with_router(&data_dir, &ledger, &r12, "s-cap-12", &req, None, |_, _| {}).unwrap();
    assert!(matches!(out12, Round2RunResult::StoppedAtCap { cap: "papers", .. }), "{out12:?}");
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_before);

    // 4. Hard cap: 36 queries (35 already used -> runs 1 query, skips rest with stopped_at_cap = true;
    //    next call at 36/36 stops immediately with 0 network hits).
    append_explore_session_start(&ledger, "s-cap-36", IdeaPreset::Thorough, false, &stmt).unwrap();
    let results_35: Vec<Value> = (1..=35).map(|i| json!({ "qid": format!("q-{i}"), "provider": "inspire", "status": "ok", "count": 1 })).collect();
    ledger.append("s-cap-36", EVENT_IDEA_SEARCH_FINISHED, 1, &json!({ "round": 1, "results": results_35, "queries": results_35 })).unwrap();
    let r36 = make_test_router(&data_dir, &lit_base, "s-cap-36");
    let partial = run_round2_with_router(&data_dir, &ledger, &r36, "s-cap-36", &req, None, |_, _| {}).unwrap();
    assert!(matches!(partial, Round2RunResult::Finished { executed_queries: 1, skipped_budget, stopped_at_cap: true, .. } if skipped_budget >= 1), "{partial:?}");
    let hits_after_36 = lit_hits.load(Ordering::SeqCst);
    let full36 = run_round2_with_router(&data_dir, &ledger, &r36, "s-cap-36", &req, None, |_, _| {}).unwrap();
    assert!(matches!(full36, Round2RunResult::StoppedAtCap { cap: "queries", .. }), "{full36:?}");
    assert_eq!(lit_hits.load(Ordering::SeqCst), hits_after_36);
}
