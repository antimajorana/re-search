//! Integration tests for session Archive / Unarchive, Delete (preserving SQLite
//! hash-chained ledger integrity and local workspace files), and human opt-in
//! per-session capability grants (`write-workspace`, contained `execute-code`,
//! `network-read`) + grant revocation.

mod common;

use common::{drain_http_request, launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use export::{fork_session, ForkOptions};
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn ledger_only_forks_expose_their_immediate_parent_and_fork_details() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let original = "s-original";
    ledger
        .append_with_hash(original, "session.start", 1, &json!({ "prompt": "Solve for x" }))
        .unwrap();
    let child = fork_session(
        &data_dir,
        &ForkOptions {
            parent_session: original.to_string(),
            at_seq: 1,
            modified_assumption: Some("x is positive".to_string()),
        },
    )
    .unwrap();
    let child_head = ledger.read_session(&child.new_session).unwrap().last().unwrap().seq;
    let grandchild = fork_session(
        &data_dir,
        &ForkOptions {
            parent_session: child.new_session.clone(),
            at_seq: child_head,
            modified_assumption: Some("x is even".to_string()),
        },
    )
    .unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let headers = [("X-ReSearch-Token", srv.info.token.as_str())];
    let (status, _, list) = raw_http(port, "GET", "/api/sessions", &headers, b"");
    assert_eq!(status, 200);
    let list: Value = serde_json::from_slice(&list).unwrap();
    let sessions = list["sessions"].as_array().unwrap();
    for fork in [&child, &grandchild] {
        let row = sessions.iter().find(|row| row["id"] == fork.new_session).unwrap();
        assert_eq!(row["parent_session"], fork.parent_session);
    }

    let path = format!("/api/sessions/{}", grandchild.new_session);
    let (status, _, detail) = raw_http(port, "GET", &path, &headers, b"");
    assert_eq!(status, 200);
    let detail: Value = serde_json::from_slice(&detail).unwrap();
    assert_eq!(
        detail["fork_info"],
        json!({
            "parent_session": child.new_session,
            "forked_at_seq": child_head,
            "modified_assumption": "x is even"
        })
    );
    srv.shutdown();
}

#[test]
fn archive_unarchive_and_delete_preserve_ledger_chain_and_workspace_files() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("paper_ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("main.tex"), "\\section{Intact}\n").unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws_dir.clone()), None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let hdr = [("X-ReSearch-Token", tok.as_str())];

    // Session 1: has ledger events + a content-addressed blob + transient scratch/sidetalk.
    let c1 = serde_json::to_vec(&json!({ "title": "Recorded Chat", "workspace_root": ws_dir })).unwrap();
    let (_, _, r1) = raw_http(port, "POST", "/api/sessions", &hdr, &c1);
    let s1 = serde_json::from_slice::<Value>(&r1).unwrap()["id"].as_str().unwrap().to_string();

    // Session 2: blank chat with zero ledger events.
    let (_, _, r2) = raw_http(port, "POST", "/api/sessions", &hdr, br#"{"title":"Blank Chat"}"#);
    let s2 = serde_json::from_slice::<Value>(&r2).unwrap()["id"].as_str().unwrap().to_string();

    {
        let ledger = Ledger::open(&data_dir).unwrap();
        let blob_hash = ledger.put_blob(b"raw model payload bytes").unwrap();
        ledger
            .append_with_hash(
                &s1,
                "model.request",
                1,
                &json!({ "turn": 1, "model": "claude-haiku-4-5", "raw_blob": blob_hash }),
            )
            .unwrap();
        let s1_dir = data_dir.join("sessions").join(&s1);
        std::fs::create_dir_all(s1_dir.join("scratch")).unwrap();
        std::fs::write(s1_dir.join("scratch").join("temp.py"), "x = 1\n").unwrap();
        std::fs::write(s1_dir.join("sidetalk.jsonl"), "{}\n").unwrap();
    }

    // 1. Archive s1 -> appears with archived: true.
    let (st_arch, _, arch_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{s1}/archive"),
        &hdr,
        br#"{"archived":true}"#,
    );
    assert_eq!(st_arch, 200);
    assert_eq!(serde_json::from_slice::<Value>(&arch_raw).unwrap()["archived"], true);

    let (_, _, list1) = raw_http(port, "GET", "/api/sessions", &hdr, b"");
    let rows1 = serde_json::from_slice::<Value>(&list1).unwrap()["sessions"].as_array().cloned().unwrap();
    assert_eq!(rows1.iter().find(|r| r["id"] == s1).unwrap()["archived"], true);
    assert_eq!(rows1.iter().find(|r| r["id"] == s2).unwrap()["archived"], false);

    // 2. Unarchive s1 -> archived: false.
    let (st_un, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{s1}/archive"),
        &hdr,
        br#"{"archived":false}"#,
    );
    assert_eq!(st_un, 200);
    let (_, _, get_s1) = raw_http(port, "GET", &format!("/api/sessions/{s1}"), &hdr, b"");
    assert_eq!(serde_json::from_slice::<Value>(&get_s1).unwrap()["archived"], false);

    // 3. Delete blank s2 -> directory removed completely, 404 thereafter.
    let (st_del2, _, _) = raw_http(port, "DELETE", &format!("/api/sessions/{s2}"), &hdr, b"");
    assert_eq!(st_del2, 200);
    assert!(!data_dir.join("sessions").join(&s2).exists());
    assert_eq!(raw_http(port, "GET", &format!("/api/sessions/{s2}"), &hdr, b"").0, 404);

    // 4. Delete recorded s1 -> scratch & sidetalk cleaned, hidden from list, 404 on GET/DELETE,
    //    local workspace `main.tex` untouched, and `ledger.verify_all()` remains 100% green!
    let (st_del1, _, _) = raw_http(port, "DELETE", &format!("/api/sessions/{s1}"), &hdr, b"");
    assert_eq!(st_del1, 200);
    assert!(!data_dir.join("sessions").join(&s1).join("scratch").exists());
    assert!(!data_dir.join("sessions").join(&s1).join("sidetalk.jsonl").exists());
    assert_eq!(raw_http(port, "GET", &format!("/api/sessions/{s1}"), &hdr, b"").0, 404);
    assert_eq!(raw_http(port, "DELETE", &format!("/api/sessions/{s1}"), &hdr, b"").0, 404);

    let (_, _, list2) = raw_http(port, "GET", "/api/sessions", &hdr, b"");
    assert!(serde_json::from_slice::<Value>(&list2).unwrap()["sessions"].as_array().unwrap().is_empty());

    assert_eq!(std::fs::read_to_string(ws_dir.join("main.tex")).unwrap(), "\\section{Intact}\n");

    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let report = ledger.verify_all().unwrap();
    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.unreferenced_blobs, 0);

    srv.shutdown();
}

#[test]
fn workspace_write_session_grant_reuses_across_turns_and_can_be_revoked() {
    let model_srv = TcpListener::bind("127.0.0.1:0").unwrap();
    let model_addr = model_srv.local_addr().unwrap();
    std::thread::spawn(move || {
        let replies = [
            json!({
                "id": "w1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "tool_use",
                "content": [{ "type": "tool_use", "id": "tu_w1", "name": "fs_write", "input": { "path": "step1.tex", "content": "A\n" } }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
            json!({
                "id": "w2", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "end_turn",
                "content": [{ "type": "text", "text": "Wrote step1." }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
            json!({
                "id": "w3", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "tool_use",
                "content": [{ "type": "tool_use", "id": "tu_w2", "name": "fs_write", "input": { "path": "step2.tex", "content": "B\n" } }],
                "usage": { "input_tokens": 5, "output_tokens": 5 }
            }),
            json!({
                "id": "w4", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                "stop_reason": "end_turn",
                "content": [{ "type": "text", "text": "Wrote step2 without re-prompting!" }],
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
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        data_dir.join("providers.json"),
        serde_json::to_vec(&json!({ "providers": { "anthropic": { "api_key": "sk-ant-ws-grant" } } })).unwrap(),
    )
    .unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, Some(ws_dir.clone()), Some(format!("http://{model_addr}")), None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    common::disable_auto_approve(port, &tok);
    let cookie = launch_cookie(port, &tok);
    let hdr = [("X-ReSearch-Token", tok.as_str())];

    let (_, _, raw) = raw_http(port, "POST", "/api/sessions", &hdr, b"{}");
    let sid = serde_json::from_slice::<Value>(&raw).unwrap()["id"].as_str().unwrap().to_string();

    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(sse, "GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n").unwrap();
    let mut reader = BufReader::new(sse);

    // Turn 1: approves `fs_write` with `approve_for_session: true`.
    let m1 = serde_json::to_vec(&json!({ "prompt": "Step 1", "provider": "anthropic", "model": "claude-haiku-4-5" })).unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &m1).0, 202);

    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if let Some(rest) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(rest.trim()) {
                if v.get("allow_session_approve") == Some(&json!(true)) {
                    let cid = v["call_id"].as_str().unwrap();
                    let dig = v["digest"].as_str().unwrap();
                    let body = serde_json::to_vec(&json!({
                        "digest": dig, "approved": true, "approve_for_session": true
                    }))
                    .unwrap();
                    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/approvals/{cid}"), &hdr, &body).0, 200);
                }
                if v.get("text") == Some(&json!("Wrote step1.")) {
                    break;
                }
            }
        }
    }

    let (_, _, s_after_1) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &hdr, b"");
    assert_eq!(
        serde_json::from_slice::<Value>(&s_after_1).unwrap()["session_grants"],
        json!(["write-workspace"])
    );

    // Turn 2: writes `step2.tex` with ZERO new approval prompt!
    let m2 = serde_json::to_vec(&json!({ "prompt": "Step 2", "provider": "anthropic", "model": "claude-haiku-4-5" })).unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &m2).0, 202);

    let mut turn2_done = false;
    for line in reader.lines().map_while(Result::ok) {
        assert!(!line.contains("event: approval_requested"), "unexpected prompt on Turn 2: {line}");
        if line.contains("Wrote step2 without re-prompting!") {
            turn2_done = true;
            break;
        }
    }
    assert!(turn2_done);
    assert_eq!(std::fs::read_to_string(ws_dir.join("step2.tex")).unwrap(), "B\n");

    // Revoking grants empties `session_grants`.
    let (st_rev, _, rev_raw) = raw_http(port, "DELETE", &format!("/api/sessions/{sid}/grants"), &hdr, b"");
    assert_eq!(st_rev, 200);
    assert_eq!(serde_json::from_slice::<Value>(&rev_raw).unwrap()["session_grants"], json!([]));

    let ledger = Ledger::open_existing(&data_dir).unwrap();
    assert!(ledger.verify_session(&sid).is_ok());
    srv.shutdown();
}
