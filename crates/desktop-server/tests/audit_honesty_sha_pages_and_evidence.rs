//! Honesty-lens audit proof tests for:
//! - F20 (`major`): `read_paper_lines` Proof-DAG `tool_call_ref` downgraded to
//!   `"unsupported_reference"` in `evidence.rs` and omitted from `linked_sources`.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::{
    ingest_bytes, register_research_tools, research_policy, IngestRequest, ResearchEnv,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router};

fn call_api(
    server: &RunningDesktopServer,
    method: &str,
    path: &str,
    body: Value,
    expected: u16,
) -> Value {
    let (status, _, bytes) = raw_http(
        server.addr.port(),
        method,
        path,
        &[("X-ReSearch-Token", &server.info.token)],
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(
        status,
        expected,
        "{path}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "test".into(),
        }
    }
}

/// Finding F20 (major): `crates/desktop-server/src/evidence.rs` (`describe_tool` and
/// `linked_session_sources`) has no arm for `"read_paper_lines"`, downgrading Proof-DAG
/// steps backed by `read_paper_lines` to `"status": "unsupported_reference"` and omitting
/// the paper from `linked_sources` (`Sources Used`).
#[test]
fn audit_honesty_proof_evidence_and_linked_sources_must_support_read_paper_lines() {
    let temp = tempfile::tempdir().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let res = call_api(
        &server,
        "POST",
        "/api/sessions",
        json!({"title": "RPL Evidence"}),
        200,
    );
    let sid = res["id"].as_str().unwrap().to_string();

    let env = Arc::new(ResearchEnv::new(temp.path().to_path_buf()));
    let db = Mutex::new(Ledger::open(temp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: &sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"Theorem 1: E = mc^2 holds in flat spacetime.\n",
            source_id_hint: Some("paper"),
            section_filter: None,
            title: Some("Relativity Note"),
        },
    )
    .unwrap();
    {
        let l = db.lock().unwrap();
        l.append(&sid, "session.start", 1, &json!({"prompt": "Check Theorem 1"}))
            .unwrap();
    }

    let mut reg = Registry::new();
    register_research_tools(&mut reg, Arc::clone(&env));
    let router = Router::new(
        reg,
        research_policy(),
        db.into_inner().unwrap(),
        &sid,
        Arc::new(AllowAll),
    );
    let rpl_out = router
        .try_call(
            "agent:ollama:test",
            "read_paper_lines",
            &json!({"source_id": src.source_id, "start_line": 1, "end_line": 1}),
        )
        .unwrap();
    assert!(matches!(rpl_out, CallOutcome::Ok(_)));

    let ledger = Ledger::open(temp.path()).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let call_id = events
        .iter()
        .rfind(|e| e.kind == "tool.request" && e.payload["tool"] == "read_paper_lines")
        .and_then(|e| e.payload["call_id"].as_str())
        .unwrap()
        .to_string();
    drop(events);
    let dag_out = router
        .try_call(
            "agent:ollama:test",
            "ledger_append",
            &json!({
                "entry_type": "reference_lookup",
                "summary": "Theorem 1 states E = mc^2.",
                "details": "Read directly via read_paper_lines.",
                "tool_call_ref": call_id,
                "depends_on": []
            }),
        )
        .unwrap();
    assert!(matches!(dag_out, CallOutcome::Ok(_)), "{dag_out:?}");
    ledger
        .append(&sid, "session.end", 1, &json!({"outcome": "completed"}))
        .unwrap();

    let ev_resp = call_api(
        &server,
        "GET",
        &format!("/api/sessions/{sid}/evidence"),
        Value::Null,
        200,
    );
    let session_view = call_api(&server, "GET", &format!("/api/sessions/{sid}"), Value::Null, 200);
    server.shutdown();

    let support = &ev_resp["entries"][0]["support"];
    assert!(
        support[0]["status"] != "unsupported_reference"
            && !session_view["linked_sources"].as_array().unwrap().is_empty(),
        "read_paper_lines Proof-DAG reference rejected: support={support} linked_sources={}",
        session_view["linked_sources"]
    );
}
