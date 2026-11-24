//! The UI search and tool route return the same verified passage for proof evidence.

mod common;

use common::{raw_http, test_cfg};
use ledger::Ledger;
use research_tools::paper_passages::SEARCH_PAPER_PASSAGES;
use research_tools::{register_research_tools, research_policy, ResearchEnv, LEDGER_APPEND};
use serde_json::{json, Value};
use std::sync::Arc;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router};

struct NoApprovalExpected;

impl Approver for NoApprovalExpected {
    fn decide(&self, _: &ApprovalRequest) -> ApprovalReply {
        panic!("local passage search and proof bookkeeping should use their normal allow rules")
    }
}

#[test]
fn authenticated_session_search_and_recorded_proof_evidence_agree() {
    let tmp = tempfile::tempdir().unwrap();
    let server =
        desktop_server::start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = server.addr.port();
    let auth = [("X-ReSearch-Token", server.info.token.as_str())];
    let call = |method: &str, path: &str, payload: Value| {
        let payload = serde_json::to_vec(&payload).unwrap();
        let (status, _, response) = raw_http(port, method, path, &auth, &payload);
        (status, serde_json::from_slice::<Value>(&response).unwrap())
    };

    let (status, session) = call("POST", "/api/sessions", json!({ "title": "Spectrum" }));
    assert_eq!(status, 200, "{session}");
    let sid = session["id"].as_str().unwrap();
    let raw = "\\title{Spectral Notes}\n\nThe spectral entropy result uses α = 2.\n";
    let upload = format!("--passage-test\r\nContent-Disposition: form-data; name=\"files\"; filename=\"notes.tex\"\r\nContent-Type: text/plain\r\n\r\n{raw}\r\n--passage-test--\r\n");
    let (status, _, upload_result) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/upload"),
        &[
            auth[0],
            ("Content-Type", "multipart/form-data; boundary=passage-test"),
        ],
        upload.as_bytes(),
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&upload_result));
    let upload_result: Value = serde_json::from_slice(&upload_result).unwrap();
    let source = &upload_result["ingested"][0];

    let path = format!("/api/sessions/{sid}/paper-passages");
    let query = json!({ "query": "spectral entropy α", "limit": 1 });
    assert_eq!(
        raw_http(
            port,
            "POST",
            &path,
            &[],
            &serde_json::to_vec(&query).unwrap()
        )
        .0,
        401
    );
    let (status, searched) = call("POST", &path, query.clone());
    assert_eq!(status, 200, "{searched}");
    assert_eq!(searched["status"], "matches");
    let hit = &searched["passages"][0];
    assert_eq!(hit["source_id"], source["source_id"]);
    assert_eq!(hit["path"], source["primary_path"]);
    assert_eq!(hit["title"], "Spectral Notes");
    assert_eq!(hit["excerpt"], "The spectral entropy result uses α = 2.\n");
    assert_eq!(hit["start_line"], 3);
    assert_eq!(hit["end_line"], 3);
    assert_eq!(hit["file_sha256"], ledger::sha256_hex(raw.as_bytes()));

    let mut registry = Registry::new();
    register_research_tools(&mut registry, Arc::new(ResearchEnv::new(tmp.path())));
    let router = Router::new(
        registry,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        sid,
        Arc::new(NoApprovalExpected),
    );
    let CallOutcome::Ok(mut tool_search) = router
        .try_call("test", SEARCH_PAPER_PASSAGES, &query)
        .unwrap()
    else {
        panic!("the local passage search tool failed");
    };
    let call_ref = tool_search
        .as_object_mut()
        .unwrap()
        .remove("tool_call_ref")
        .unwrap();
    assert_eq!(tool_search, searched);
    let ledger = Ledger::open(tmp.path()).unwrap();
    let (_, records) = ledger.read_verified_session(sid).unwrap();
    let result = records.last().unwrap();
    assert_eq!(result.kind, "tool.result");
    assert_eq!(result.payload["ok"], true);
    assert_eq!(call_ref, result.payload["call_id"]);
    let CallOutcome::Ok(entry) = router
        .try_call(
            "test",
            LEDGER_APPEND,
            &json!({
                "entry_type": "reference_lookup",
                "summary": "Spectral entropy parameter",
                "details": "The attached paper states its choice of α.",
                "tool_call_ref": call_ref,
            }),
        )
        .unwrap()
    else {
        panic!("could not attach the recorded lookup to a proof entry");
    };
    let entry_id = entry["entry_id"].as_str().unwrap();
    let (status, evidence) = call(
        "GET",
        &format!("/api/sessions/{sid}/evidence?entry_id={entry_id}"),
        Value::Null,
    );
    assert_eq!(status, 200, "{evidence}");
    assert_eq!(evidence["ledger_verified"], true);
    let support = &evidence["entries"][0]["support"][0];
    assert_eq!(support["kind"], "paper_passages");
    assert_eq!(support["status"], "matches");
    assert_eq!(support["via_entry_id"], entry_id);
    assert_eq!(support["result_seq"], result.seq);
    assert_eq!(
        support["passages"][0],
        json!({
            "title": hit["title"],
            "path": hit["path"],
            "file_sha256": hit["file_sha256"],
            "start_line": hit["start_line"],
            "end_line": hit["end_line"],
            "excerpt": hit["excerpt"],
            "reader": {
                "source_id": hit["source_id"], "path": hit["path"],
                "file_sha256": hit["file_sha256"], "start_line": hit["start_line"], "end_line": hit["end_line"],
            },
        })
    );
    server.shutdown();
}
