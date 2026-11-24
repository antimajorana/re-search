//! Integration through the same HTTP endpoints used by the desktop, including a fresh linked Research history.
mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::{
    ingest_bytes,
    investigation::{record, record_branch, record_conventions},
    IngestRequest, ResearchEnv,
};
use serde_json::{json, Value};
use std::sync::Mutex;

fn call(
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

#[test]
fn corrected_paper_claim_opens_fresh_research_with_its_evidence_and_assumptions() {
    let temp = tempfile::tempdir().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let sid = "s-study";
    let path = format!("/api/sessions/{sid}");
    call(
        &server,
        "POST",
        "/api/sessions",
        json!({"id":sid,"mode":"investigation"}),
        200,
    );
    assert_eq!(
        call(&server, "GET", &path, Value::Null, 200)["mode"],
        "investigation"
    );
    call(
        &server,
        "POST",
        &format!("{path}/messages"),
        json!({"prompt":"Which result follows?"}),
        400,
    );
    let env = ResearchEnv::new(temp.path().to_path_buf());
    let ledger = Mutex::new(Ledger::open(temp.path()).unwrap());
    let lines = [
        "Assume a fixed boundary.",
        "The boundary term vanishes.",
        "The endpoint variation equals zero.",
        "Throughout, d denotes the exterior derivative.",
    ];
    let mut paper_text = lines.join("\n") + "\n";
    for _ in 1..3000 {
        paper_text
            .push_str("The appendix records each of the boundary terms used in the calculation.\n");
    }
    paper_text.push_str("Appendix conclusion: the boundary term is zero.");
    let src = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "boundary.txt",
            origin: "user upload",
            raw_bytes: paper_text.as_bytes(),
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let passage = call(
        &server,
        "GET",
        &format!(
            "{path}/workspace/file?root=sources&path={}&start_line=3004&end_line=3004",
            src.primary_path.as_ref().unwrap()
        ),
        Value::Null,
        200,
    );
    assert_eq!(passage["first_line"], 2996);
    assert_eq!(
        passage["file_sha256"],
        ledger::sha256_hex(paper_text.as_bytes())
    );
    assert!(passage["content"]
        .as_str()
        .unwrap()
        .ends_with("Appendix conclusion: the boundary term is zero."));
    let ledger = ledger.into_inner().unwrap();
    ledger
        .append(
            sid,
            "investigation.paper",
            1,
            &json!({"source_id":src.source_id,"question":"What follows?"}),
        )
        .unwrap();
    let reading = ledger
        .append(
            sid,
            "investigation.mapping_requested",
            1,
            &json!({"source_id":src.source_id}),
        )
        .unwrap();
    let anchor = |line: usize| json!({"source_id":src.source_id,"path":src.primary_path,"start_line":line,"end_line":line,"excerpt":lines[line-1],"kind":"passage"});
    let map = json!({"source_id":src.source_id,"title":"Boundary terms","coverage":"paper","limitations":[],
        "nodes":[{"id":"a","kind":"assumption","main":true,"sentence":lines[0],"anchors":[anchor(1)]},
                 {"id":"b","kind":"result","main":true,"sentence":lines[1],"anchors":[anchor(2)]}],
        "edges":[{"from":"a","to":"b","kind":"depends_on","basis":"inferred","reason":"The argument uses the fixed endpoints.","anchor":anchor(2)}]});
    record(&env, &ledger, sid, &map).unwrap();
    record_conventions(&env,&ledger,sid,&json!({"request_seq":reading,"source_id":src.source_id,"batch_index":1,"done":true,"coverage":"paper","limitations":[],
        "items":[{"id":"exterior-d","category":"notation","label":"d","meaning":"Exterior derivative","scope":"Throughout the paper","basis":"explicit","anchors":[anchor(4)]}]})).unwrap();
    let view = call(&server, "GET", &path, Value::Null, 200);
    assert_eq!(view["title"], "Boundary terms");
    let seq = view["investigation"]["map_seq"].as_u64().unwrap();
    assert_eq!(
        view["investigation"]["conventions"]["items"][0]["label"],
        "d"
    );
    assert_eq!(view["investigation"]["conventions_is_current"], true);
    let before_review = ledger.read_session(sid).unwrap().len();
    assert_eq!(
        call(
            &server,
            "POST",
            &format!("{path}/investigation/conventions"),
            json!({"map_seq":seq}),
            200
        )["status"],
        "complete"
    );
    assert_eq!(ledger.read_session(sid).unwrap().len(), before_review);
    assert!(view["investigation"]["branches"]
        .as_array()
        .unwrap()
        .is_empty());
    let branch = json!({"map_seq":seq,"source_id":src.source_id,"node_id":"b","question":"What is varied at the endpoints?"});
    let request = ledger
        .append(sid, "investigation.branch_requested", 1, &branch)
        .unwrap();
    record_branch(&env, &ledger, sid, &json!({"request_seq":request,"source_id":src.source_id,
        "nodes":[{"id":"variation","kind":"result","main":false,"sentence":lines[2],"anchors":[anchor(3)]}],
        "edges":[{"from":"variation","to":"b","kind":"depends_on","basis":"inferred","reason":"The endpoint contribution is zero when the variation vanishes.","anchor":anchor(3)}],
        "limitations":[]})).unwrap();
    let count = ledger.read_session(sid).unwrap().len();
    let saved = call(
        &server,
        "POST",
        &format!("{path}/investigation/branches"),
        branch,
        200,
    );
    assert_eq!(saved["status"], "built");
    assert_eq!(ledger.read_session(sid).unwrap().len(), count);
    let developed = call(&server, "GET", &path, Value::Null, 200);
    assert_eq!(
        developed["investigation"]["map"]["nodes"][2]["id"],
        "variation"
    );
    assert_eq!(developed["investigation"]["branches"][0]["status"], "built");
    assert_eq!(developed["investigation"]["map_seq"], seq);
    call(
        &server,
        "POST",
        &format!("{path}/investigation/corrections"),
        json!({"map_seq":seq-1,"node_id":"b","text":"stale"}),
        409,
    );
    call(
        &server,
        "POST",
        &format!("{path}/investigation/corrections"),
        json!({"map_seq":seq,"node_id":"b","edge_from":"a","text":"The boundary condition applies to variations."}),
        201,
    );
    let corrected = call(&server, "GET", &path, Value::Null, 200);
    assert_eq!(
        corrected["investigation"]["map"]["edges"][0]["basis"],
        "inferred"
    );
    assert_eq!(
        corrected["investigation"]["corrections"][0]["edge_from"],
        "a"
    );
    let response = call(
        &server,
        "POST",
        &format!("{path}/investigation/research"),
        json!({"map_seq":seq,"node_id":"b","question":"What if the endpoints move?"}),
        201,
    );
    let child = response["session_id"].as_str().unwrap();
    let research = call(
        &server,
        "GET",
        &format!("/api/sessions/{child}"),
        Value::Null,
        200,
    );
    assert_eq!(research["mode"], "research");
    let origin = &research["investigation"]["origin"];
    assert_eq!(origin["assumptions"], json!([lines[0]]));
    assert_eq!(
        origin["corrections"][0]["text"],
        "The boundary condition applies to variations."
    );
    assert_eq!(origin["parent_session"], sid);
    assert_eq!(
        origin["paper_conventions"][0]["meaning"],
        "Exterior derivative"
    );
    assert_eq!(origin["sources"].as_array().unwrap().len(), 3);
    assert_eq!(research["messages"], json!([]));
    assert_eq!(research["sources"][0]["source_id"], src.source_id);
    assert!(temp
        .path()
        .join("sessions")
        .join(child)
        .join("sources")
        .join(src.primary_path.as_ref().unwrap())
        .is_file());
    assert_eq!(
        call(&server, "GET", &path, Value::Null, 200)["mode"],
        "investigation"
    );
}
