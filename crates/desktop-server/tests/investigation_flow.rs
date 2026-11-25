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

fn start_openai_capture_mock() -> (String, std::sync::Arc<Mutex<Vec<Value>>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
    let seen_cl = std::sync::Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen_inner = std::sync::Arc::clone(&seen_cl);
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(stream);
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut buf = vec![0u8; len];
                let _ = reader.read_exact(&mut buf);
                let body: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                seen_inner.lock().unwrap().push(body);
                let reply = serde_json::to_vec(&json!({
                    "id": "chatcmpl_1", "object": "chat.completion", "model": "gpt-4.1",
                    "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "Done."}}],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5}
                })).unwrap();
                let mut s = reader.into_inner();
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.len()
                );
                let _ = s.write_all(hdr.as_bytes());
                let _ = s.write_all(&reply);
                let _ = s.flush();
            });
        }
    });
    (format!("http://{addr}"), seen)
}

fn wait_idle(server: &RunningDesktopServer, sid: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let view = call(server, "GET", &format!("/api/sessions/{sid}"), Value::Null, 200);
        if view["running"] != true {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    panic!("session {sid} stayed running");
}

fn tool_names(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["function"]["name"].as_str().map(str::to_string))
        .collect()
}

fn setup_worker_server(temp: &std::path::Path, mock_url: &str) -> RunningDesktopServer {
    std::fs::write(
        temp.join("providers.json"),
        serde_json::to_vec(&json!({"keys": {"openai": "sk-test-key"}})).unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.join("subagents.json"),
        serde_json::to_vec(&json!([
            {
                "id": "tensor-helper",
                "name": "Tensor Helper",
                "description": "Checks tensor algebra.",
                "tools_profile": "read_only",
                "system_prompt": "Check tensor algebra carefully."
            }
        ]))
        .unwrap(),
    )
    .unwrap();
    let mut cfg = test_cfg(temp, None, None, None);
    cfg.api_base_override = Some(mock_url.to_string());
    start_desktop_server(cfg).unwrap()
}

#[test]
fn mapping_turn_offers_no_compute_lean_explore_subagent() {
    let (mock_url, seen) = start_openai_capture_mock();
    let temp = tempfile::tempdir().unwrap();
    let server = setup_worker_server(temp.path(), &mock_url);
    let sid = "s-map-gate";
    call(&server, "POST", "/api/sessions", json!({"id": sid, "mode": "investigation"}), 200);
    let env = ResearchEnv::new(temp.path().to_path_buf());
    let db = Mutex::new(Ledger::open(temp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"We assume positive mass.\nEquation (1) holds.\n",
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    drop(db);
    call(
        &server,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        json!({"prompt": "Map the paper.", "provider": "openai", "model": "gpt-4.1", "investigation_source_id": src.source_id}),
        202,
    );
    wait_idle(&server, sid);
    let reqs = seen.lock().unwrap().clone();
    assert!(!reqs.is_empty());
    let names = tool_names(&reqs[0]);
    for forbidden in ["delegate_compute", "delegate_lean", "delegate_explore", "delegate_subagent"] {
        assert!(!names.iter().any(|n| n == forbidden), "unexpected {forbidden} in {names:?}");
    }
    assert!(names.iter().any(|n| n == "read_paper_lines"), "missing read_paper_lines in {names:?}");
    let body_str = reqs[0].to_string();
    assert!(body_str.contains("Other worker delegation is unavailable during paper reading."));
    assert!(!body_str.contains("delegate_read_paper"));
    server.shutdown();
}

#[test]
fn check_action_keeps_workers() {
    let (mock_url, seen) = start_openai_capture_mock();
    let temp = tempfile::tempdir().unwrap();
    let server = setup_worker_server(temp.path(), &mock_url);
    let sid = "s-check-workers";
    call(&server, "POST", "/api/sessions", json!({"id": sid, "mode": "investigation"}), 200);
    let env = ResearchEnv::new(temp.path().to_path_buf());
    let db = Mutex::new(Ledger::open(temp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"We assume positive mass.\nThe boundary term vanishes.\n",
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger.append(sid, "investigation.paper", 1, &json!({"source_id": src.source_id, "question": "Check?"})).unwrap();
    ledger.append(sid, "investigation.mapping_requested", 1, &json!({"source_id": src.source_id})).unwrap();
    let anchor = json!({"source_id": src.source_id, "start_line": 2, "end_line": 2, "quote": "boundary term vanishes", "kind": "passage"});
    let map_res = record(&env, &ledger, sid, &json!({
        "source_id": src.source_id, "title": "Boundary", "coverage": "paper", "limitations": [],
        "nodes": [{"id": "b", "kind": "result", "main": true, "sentence": "Boundary term vanishes.", "anchors": [anchor]}],
        "edges": []
    })).unwrap();
    let map_seq = map_res["map_seq"].as_u64().unwrap();
    call(
        &server,
        "POST",
        &format!("/api/sessions/{sid}/investigation/rechecks"),
        json!({"map_seq": map_seq, "node_id": "b", "action": "check", "provider": "openai", "model": "gpt-4.1"}),
        202,
    );
    wait_idle(&server, sid);
    let reqs = seen.lock().unwrap().clone();
    assert!(!reqs.is_empty());
    let names = tool_names(&reqs[0]);
    for expected in ["delegate_compute", "delegate_lean", "delegate_explore", "delegate_subagent"] {
        assert!(names.iter().any(|n| n == expected), "missing {expected} in {names:?}");
    }
    server.shutdown();
}
