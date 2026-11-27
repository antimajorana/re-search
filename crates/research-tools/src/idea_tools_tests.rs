use super::*;
use crate::lit_providers::LITERATURE_FETCHED_EVENT;
use crate::{research_policy, LiteratureEndpoints};
use ledger::Ledger;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Router};

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "idea-tools-test".into(),
        }
    }
}

struct DenyAll;
impl Approver for DenyAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: false,
            digest: req.digest.clone(),
            by: "idea-tools-test".into(),
        }
    }
}

fn spawn_one_shot_http(status: u16, content_type: &'static str, body: String) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req_text = String::from_utf8_lossy(&buf[..n]).to_string();
        let resp = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        req_text
    });
    (format!("http://127.0.0.1:{}", addr.port()), handle)
}

#[test]
fn explore_v1_defs_freeze_four_tools_with_zero_exec_or_write_rule_p7() {
    let defs = explore_v1_defs();
    let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, EXPLORE_V1_TOOL_IDS);
    for def in &defs {
        assert_ne!(def.effect, EffectClass::ExecuteCode);
        assert_ne!(def.effect, EffectClass::WriteWorkspace);
    }
    let search = defs.iter().find(|d| d.id == IDEA_SEARCH).unwrap();
    assert_eq!(search.effect, EffectClass::NetworkRead);
    assert_eq!(search.approval_scope, ApprovalScope::Once);
    let enum_vals = search.input_schema["properties"]["queries"]["items"]["properties"]["provider"]
        ["enum"]
        .as_array()
        .unwrap();
    assert_eq!(enum_vals.len(), 6);
    for prov in ALL_PROVIDER_ENUM {
        assert!(enum_vals.iter().any(|v| v.as_str() == Some(*prov)));
    }
}

#[test]
fn idea_search_disabled_services_return_disabled_and_stub_fetch_fails_closed() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let mut reg = Registry::new();
    register_explore_v1_tools(&mut reg, env, None);
    let router = Router::new(reg, research_policy(), ledger, "s-disabled", Arc::new(ApproveAll));

    let out = router
        .try_call(
            "host:idea:s-disabled",
            IDEA_SEARCH,
            &json!({
                "queries": [
                    { "qid": "q-oa", "provider": "openalex", "q": "Krylov complexity" },
                    { "qid": "q-ts", "provider": "theoremsearch", "q": "Majorana chain" }
                ]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else { panic!("expected Ok, got {out:?}") };
    assert_eq!(val["results"][0]["status"], "disabled");
    assert_eq!(val["results"][1]["status"], "disabled");
    assert_eq!(val["total_returned"], 0);

    let fetch_out = router
        .try_call(
            "host:idea:s-disabled",
            IDEA_FETCH_PAPERS,
            &json!({ "keys": ["arxiv:2501.01234"] }),
        )
        .unwrap();
    match fetch_out {
        CallOutcome::Error(msg) => assert!(msg.contains("package X9"), "{msg}"),
        other => panic!("expected fail-closed stub Error, got {other:?}"),
    }
}

#[test]
fn idea_search_runs_through_router_records_f11_blob_respects_denial_and_rule_p8_headers() {
    let s2_body = json!({
        "total": 1,
        "data": [{
            "paperId": "abc123",
            "title": "Synthetic S2 Paper on Lanczos Growth",
            "authors": [{ "name": "A. Researcher" }],
            "year": 2025,
            "abstract": "Synthetic verbatim abstract for Lanczos growth tests.",
            "externalIds": { "ArXiv": "2501.09999" },
            "citationCount": 7
        }]
    })
    .to_string();
    let (base_url, srv_handle) = spawn_one_shot_http(200, "application/json", s2_body);

    let tmp = TempDir::new().unwrap();
    let mut env = ResearchEnv::new(tmp.path());
    env.network_endpoints = LiteratureEndpoints::loopback_mock(&base_url).unwrap();
    let env = Arc::new(env);

    // 1. Denied call never hits network.
    let mut reg_deny = Registry::new();
    reg_deny.register(
        idea_search_def(),
        idea_search_handler_with_gate_and_config(
            Arc::clone(&env),
            Some(Arc::new(NetRateGate::new(Arc::new(crate::net_rate::FakeRateClock::new())))),
            Some(ProviderConfig {
                semantic_scholar_api_key: Some("secret-s2-key-rule-p8".into()),
                request_timeout: None,
            }),
        ),
    );
    let router_deny = Router::new(
        reg_deny,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        "s-deny",
        Arc::new(DenyAll),
    );
    let denied = router_deny
        .try_call(
            "host:idea:s-deny",
            IDEA_SEARCH,
            &json!({ "queries": [{ "qid": "q-1", "provider": "semantic_scholar", "q": "Lanczos" }] }),
        )
        .unwrap();
    assert!(matches!(denied, CallOutcome::Denied(_)), "{denied:?}");

    // 2. Approved call sends x-api-key header ONLY, never puts key in URL or ledger (Rule P8).
    let mut reg_ok = Registry::new();
    reg_ok.register(
        idea_search_def(),
        idea_search_handler_with_gate_and_config(
            env,
            Some(Arc::new(NetRateGate::new(Arc::new(crate::net_rate::FakeRateClock::new())))),
            Some(ProviderConfig {
                semantic_scholar_api_key: Some("secret-s2-key-rule-p8".into()),
                request_timeout: None,
            }),
        ),
    );
    let router_ok = Router::new(
        reg_ok,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        "s-ok",
        Arc::new(ApproveAll),
    );
    let out = router_ok
        .try_call(
            "host:idea:s-ok",
            IDEA_SEARCH,
            &json!({
                "run_id": "run-x7",
                "queries": [{ "qid": "q-s2", "provider": "semantic_scholar", "q": "Lanczos growth", "max_results": 5 }]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else { panic!("expected Ok, got {out:?}") };
    assert_eq!(val["results"][0]["status"], "ok");
    assert_eq!(val["results"][0]["returned"], 1);
    assert_eq!(val["results"][0]["new_papers"], 1);

    let req_text = srv_handle.join().unwrap();
    let first_line = req_text.lines().next().unwrap_or("");
    assert!(!first_line.contains("secret-s2-key-rule-p8"), "API key leaked into URL: {first_line}");
    assert!(
        req_text.to_ascii_lowercase().contains("x-api-key: secret-s2-key-rule-p8"),
        "expected x-api-key header in request: {req_text}"
    );
    assert!(
        req_text.to_ascii_lowercase().contains("user-agent:"),
        "expected honest User-Agent header: {req_text}"
    );

    let ledger = Ledger::open(tmp.path()).unwrap();
    let events = ledger.read_session("s-ok").unwrap();
    assert!(events.iter().any(|e| e.kind == LITERATURE_FETCHED_EVENT));
    let dumped = serde_json::to_string(&events).unwrap();
    assert!(!dumped.contains("secret-s2-key-rule-p8"), "API key leaked into ledger!");

    // Unknown field inside a query item must fail closed (additionalProperties: false).
    let bad = router_ok
        .try_call(
            "host:idea:s-ok",
            IDEA_SEARCH,
            &json!({ "queries": [{ "qid": "q-bad", "provider": "arxiv", "q": "test", "injected": true }] }),
        )
        .unwrap();
    assert!(matches!(bad, CallOutcome::Error(msg) if msg.contains("unknown field")));
}
