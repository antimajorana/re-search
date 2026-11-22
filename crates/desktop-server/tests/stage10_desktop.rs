//! Integration tests for Stage 10 desktop-server features:
//! - `POST /api/sessions/{id}/stop` cancelling in-core API turns and denying open approval cards
//! - `PUT /api/sessions/{id}/workspace` in-place workspace attach + `data_dir` overlap rejection
//! - `POST /api/workspace/pick` `cfg(test)` modal-safety guard
//! - Session-scoped `approve_for_session: true` literature approval across turns of one session

mod common;

use common::{drain_http_request, launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::LiteratureEndpoints;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[test]
fn put_session_workspace_attaches_in_place_and_workspace_pick_is_guarded_in_test() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws1 = tmp.path().join("ws1");
    let ws2 = tmp.path().join("ws2");
    std::fs::create_dir_all(&ws1).unwrap();
    std::fs::create_dir_all(&ws2).unwrap();
    std::fs::write(ws2.join("paper.tex"), "\\section{Spin Chain}\n").unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws1.clone()), None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let hdr = [("X-ReSearch-Token", tok.as_str())];

    // 1. Native OS folder picker is deterministically refused in test mode (never opens GUI).
    let (st_pick, _, pick_body) = raw_http(port, "POST", "/api/workspace/pick", &hdr, b"{}");
    assert_eq!(st_pick, 400);
    assert!(String::from_utf8_lossy(&pick_body).contains("disabled in test mode"));

    // 2. Create session, seed an event in ledger, then attach `ws2` in-place without losing history!
    let create_body = serde_json::to_vec(&json!({ "title": "Keep My History" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &hdr, &create_body);
    assert_eq!(st, 200);
    let sid = serde_json::from_slice::<Value>(&raw).unwrap()["id"].as_str().unwrap().to_string();

    {
        let ledger = Ledger::open(&data_dir).unwrap();
        ledger
            .append_with_hash(&sid, "session.start", 1, &json!({ "prompt": "Preserve me" }))
            .unwrap();
    }

    // Reject overlapping `data_dir`!
    let bad_ws = serde_json::to_vec(&json!({ "workspace_root": data_dir })).unwrap();
    let (st_bad, _, _) = raw_http(port, "PUT", &format!("/api/sessions/{sid}/workspace"), &hdr, &bad_ws);
    assert_eq!(st_bad, 400);

    // Valid attach in-place!
    let good_ws = serde_json::to_vec(&json!({ "workspace_root": ws2 })).unwrap();
    let (st_ok, _, updated_raw) = raw_http(port, "PUT", &format!("/api/sessions/{sid}/workspace"), &hdr, &good_ws);
    assert_eq!(st_ok, 200);
    let updated: Value = serde_json::from_slice(&updated_raw).unwrap();
    assert!(!updated["workspace_root"].as_str().unwrap().starts_with(r"\\?\"), "{updated}");
    assert_eq!(updated["title"], "Keep My History");
    assert_eq!(updated["messages"].as_array().unwrap().len(), 1);
    assert_eq!(updated["messages"][0]["text"], "Preserve me");

    // Workspace file listing now sees `paper.tex` from `ws2`!
    let (st_list, _, list_raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}/workspace"), &hdr, b"");
    assert_eq!(st_list, 200);
    let list_val: Value = serde_json::from_slice(&list_raw).unwrap();
    assert!(list_val["entries"].as_array().unwrap().iter().any(|e| e["path"] == "paper.tex"));

    srv.shutdown();
}

#[test]
fn stop_endpoint_cancels_in_core_api_turn_promptly() {
    // Slow mock Anthropic server that accepts headers and holds the response open.
    let slow_api = TcpListener::bind("127.0.0.1:0").unwrap();
    let slow_addr = slow_api.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = slow_api.accept() {
            drain_http_request(&mut s);
            std::thread::sleep(Duration::from_secs(15));
        }
    });

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("providers.json"),
        serde_json::to_vec(&json!({
            "providers": { "anthropic": { "api_key": "sk-ant-stop-test-key" } }
        }))
        .unwrap(),
    )
    .unwrap();

    let srv = start_desktop_server(test_cfg(
        &data_dir,
        None,
        Some(format!("http://{slow_addr}")),
        None,
    ))
    .unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let hdr = [("X-ReSearch-Token", tok.as_str())];

    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &hdr, b"{}");
    assert_eq!(st, 200);
    let sid = serde_json::from_slice::<Value>(&raw).unwrap()["id"].as_str().unwrap().to_string();

    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        sse,
        "GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n"
    )
    .unwrap();

    let msg_body = serde_json::to_vec(&json!({
        "prompt": "Derive long result",
        "provider": "anthropic",
        "model": "claude-haiku-4-5"
    }))
    .unwrap();
    let (st_msg, _, _) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &msg_body);
    assert_eq!(st_msg, 202);

    std::thread::sleep(Duration::from_millis(120));
    let t0 = Instant::now();
    let (st_stop, _, stop_raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/stop"), &hdr, b"{}");
    assert_eq!(st_stop, 200, "{}", String::from_utf8_lossy(&stop_raw));

    let mut saw_stopped = false;
    for line in BufReader::new(sse).lines().map_while(Result::ok) {
        if line.contains("stopped by user") {
            saw_stopped = true;
            break;
        }
    }
    assert!(saw_stopped, "SSE must deliver stopped turn_error");
    assert!(t0.elapsed() < Duration::from_secs(3), "stop took {:?}", t0.elapsed());
    srv.shutdown();
}

#[test]
fn approve_for_session_persists_across_turns_of_same_session_only() {
    // Mock literature endpoint + mock Anthropic endpoint.
    let lit_srv = TcpListener::bind("127.0.0.1:0").unwrap();
    let lit_addr = lit_srv.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut s in lit_srv.incoming().flatten() {
            drain_http_request(&mut s);
            let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"></feed>";
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/atom+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                xml.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(xml);
        }
    });

    let model_srv = TcpListener::bind("127.0.0.1:0").unwrap();
    let model_addr = model_srv.local_addr().unwrap();
    std::thread::spawn(move || {
        let replies = [
            json!({
                "id": "m1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "tool_use",
                "content": [{ "type": "tool_use", "id": "tu_1", "name": "arxiv_search", "input": { "query": "au:Matsui" } }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
            json!({
                "id": "m2", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "end_turn",
                "content": [{ "type": "text", "text": "Turn 1 done." }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
            json!({
                "id": "m3", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "tool_use",
                "content": [{ "type": "tool_use", "id": "tu_2", "name": "arxiv_search", "input": { "query": "ti:split" } }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
            json!({
                "id": "m4", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "end_turn",
                "content": [{ "type": "text", "text": "Turn 2 done without second prompt!" }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
        ];
        for r in replies {
            let (mut s, _) = model_srv.accept().unwrap();
            drain_http_request(&mut s);
            let b = serde_json::to_vec(&r).unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                b.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&b);
        }
    });

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("providers.json"),
        serde_json::to_vec(&json!({
            "providers": { "anthropic": { "api_key": "sk-ant-lit-session-key" } }
        }))
        .unwrap(),
    )
    .unwrap();

    let mut cfg = test_cfg(&data_dir, None, Some(format!("http://{model_addr}")), None);
    cfg.literature_endpoints = LiteratureEndpoints::loopback_mock(&format!("http://{lit_addr}")).unwrap();
    let srv = start_desktop_server(cfg).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let hdr = [("X-ReSearch-Token", tok.as_str())];

    let (_, _, raw) = raw_http(port, "POST", "/api/sessions", &hdr, b"{}");
    let sid = serde_json::from_slice::<Value>(&raw).unwrap()["id"].as_str().unwrap().to_string();

    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        sse,
        "GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n"
    )
    .unwrap();
    let mut reader = BufReader::new(sse);

    // Turn 1: prompts once; human answers with `approve_for_session: true`.
    let m1 = serde_json::to_vec(&json!({ "prompt": "Lookup 1", "provider": "anthropic", "model": "claude-haiku-4-5" })).unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &m1).0, 202);

    let mut approved_once = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if let Some(rest) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(rest.trim()) {
                if v.get("allow_session_approve") == Some(&json!(true)) {
                    let cid = v["call_id"].as_str().unwrap();
                    let dig = v["digest"].as_str().unwrap();
                    let body = serde_json::to_vec(&json!({
                        "digest": dig,
                        "approved": true,
                        "approve_for_session": true
                    }))
                    .unwrap();
                    let (st_ap, _, _) = raw_http(port, "POST", &format!("/api/sessions/{sid}/approvals/{cid}"), &hdr, &body);
                    assert_eq!(st_ap, 200);
                    approved_once = true;
                }
                if v.get("text") == Some(&json!("Turn 1 done.")) {
                    break;
                }
            }
        }
    }
    assert!(approved_once);

    // Turn 2 in the SAME session: calls `arxiv_search` again and completes WITHOUT any new approval card!
    let m2 = serde_json::to_vec(&json!({ "prompt": "Lookup 2", "provider": "anthropic", "model": "claude-haiku-4-5" })).unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &m2).0, 202);

    let mut turn2_done = false;
    for line in reader.lines().map_while(Result::ok) {
        assert!(
            !line.contains("event: approval_requested"),
            "Turn 2 must reuse session literature grant without prompting: {line}"
        );
        if line.contains("Turn 2 done without second prompt!") {
            turn2_done = true;
            break;
        }
    }
    assert!(turn2_done);
    srv.shutdown();
}

#[test]
fn workbench_assets_and_session_verification_telemetry_served_accurately() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let hdr = [("X-ReSearch-Token", tok.as_str())];

    for (asset_path, expected_ct) in [
        ("/assets/workbench.css", "text/css"),
        ("/assets/trace_cards.js", "application/javascript"),
        ("/assets/workbench.js", "application/javascript"),
    ] {
        let (st, hdrs, bytes) = raw_http(port, "GET", asset_path, &[], b"");
        assert_eq!(st, 200, "expected 200 for {asset_path}");
        assert!(!bytes.is_empty(), "{asset_path} must not be empty");
        assert!(
            hdrs.iter().any(|(k, v)| k == "content-type" && v.contains(expected_ct)),
            "wrong content-type for {asset_path}: {hdrs:?}"
        );
    }

    let (_, _, raw) = raw_http(port, "POST", "/api/sessions", &hdr, br#"{"title":"Verified Session"}"#);
    let sid = serde_json::from_slice::<Value>(&raw).unwrap()["id"].as_str().unwrap().to_string();

    {
        let ledger = Ledger::open(&data_dir).unwrap();
        ledger
            .append_with_hash(&sid, "session.start", 1, &json!({ "prompt": "Derive gap", "model": "claude-opus-4-6" }))
            .unwrap();
        ledger
            .append_with_hash(
                &sid,
                "model.response",
                1,
                &json!({ "model": "claude-opus-4-6", "usage": { "input_tokens": 120, "output_tokens": 80 } }),
            )
            .unwrap();
        ledger
            .append_with_hash(&sid, "model.text", 1, &json!({ "text": "Gap is $2|\\Delta|$." }))
            .unwrap();
        ledger
            .append_with_hash(
                &sid,
                "proof.entry",
                1,
                &json!({ "entry_id": "e-1", "entry_type": "final_answer", "status": "accepted", "summary": "Gap 2|Delta|" }),
            )
            .unwrap();
        ledger
            .append_with_hash(
                &sid,
                "proof.verification",
                1,
                &json!({ "target_entry_id": "e-1", "verdict": "accepted", "summary": "Checked algebraically." }),
            )
            .unwrap();
        ledger
            .append_with_hash(
                &sid,
                "session.end",
                1,
                &json!({ "outcome": "completed", "verification_rounds": 1, "verified_entry_id": "e-1" }),
            )
            .unwrap();
    }

    let (st_get, _, get_raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &hdr, b"");
    assert_eq!(st_get, 200);
    let sess: Value = serde_json::from_slice(&get_raw).unwrap();
    assert_eq!(sess["stats"]["proof_entries"], 1);
    assert_eq!(sess["stats"]["verified_status"], "verified");
    assert_eq!(sess["stats"]["verification_rounds"], 1);
    assert_eq!(sess["stats"]["input_tokens"], 120);
    assert_eq!(sess["stats"]["output_tokens"], 80);
    assert_eq!(sess["messages"][1]["verified_status"], "verified");
    assert_eq!(sess["messages"][1]["verified_entry_id"], "e-1");
    assert_eq!(sess["messages"][1]["verifications"][0]["verdict"], "accepted");

    let (st_list, _, list_raw) = raw_http(port, "GET", "/api/sessions", &hdr, b"");
    assert_eq!(st_list, 200);
    let list_val: Value = serde_json::from_slice(&list_raw).unwrap();
    let row = list_val["sessions"].as_array().unwrap().iter().find(|s| s["id"] == sid).unwrap();
    assert_eq!(row["stats"]["verified_status"], "verified");
    assert_eq!(row["stats"]["proof_entries"], 1);

    srv.shutdown();
}
