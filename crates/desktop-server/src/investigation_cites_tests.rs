use super::*;

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s1".to_string(),
        seq,
        ts_ms: (seq * 1000) as i64,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: String::new(),
    }
}

#[test]
fn cites_marked_read_only_inside_received_ranges() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("open ledger");
    let path = "src-1/paper.tex";
    let blob_payload = serde_json::to_vec(&json!({
        "root": "sources",
        "path": path,
        "start_line": 21,
        "end_line": 30,
        "total_lines": 100
    }))
    .unwrap();
    let blob_hash = ledger.put_blob(&blob_payload).expect("put_blob");

    let events = vec![
        // Prior turn read (before request_seq=10) must be ignored.
        ev(5, "tool.request", json!({"call_id": "old", "tool": "fs_read"})),
        ev(
            6,
            "tool.result",
            json!({"call_id": "old", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 1, "end_line": 5, "total_lines": 100
            })).unwrap()}}),
        ),
        ev(10, "investigation.question_requested", json!({"source_id": "src-1"})),
        // Workspace-root fs_read with same relative path must be ignored.
        ev(11, "tool.request", json!({"call_id": "ws", "tool": "fs_read"})),
        ev(
            12,
            "tool.result",
            json!({"call_id": "ws", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "workspace", "path": path, "start_line": 80, "end_line": 90, "total_lines": 100
            })).unwrap()}}),
        ),
        // Offset-mode fs_read (inline) lines 10..20 + blob lines 21..30 -> merged 10..30.
        ev(13, "tool.request", json!({"call_id": "r1", "tool": "fs_read"})),
        ev(
            14,
            "tool.result",
            json!({"call_id": "r1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 10, "end_line": 20, "total_lines": 100
            })).unwrap()}}),
        ),
        ev(15, "tool.request", json!({"call_id": "r2", "tool": "fs_read"})),
        ev(
            16,
            "tool.result",
            json!({"call_id": "r2", "ok": true, "result": {"blob": blob_hash}}),
        ),
        // Grep-mode fs_read: lines 40 and 42 (gap at 41!).
        ev(17, "tool.request", json!({"call_id": "g1", "tool": "fs_read"})),
        ev(
            18,
            "tool.result",
            json!({"call_id": "g1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "matches": [
                    {"line": 40, "text": "hit a"},
                    {"line": 42, "text": "hit b"}
                ], "total_lines": 100
            })).unwrap()}}),
        ),
        // search_paper_passages: lines 60..65 on primary_path, 70..75 on another paper.
        ev(19, "tool.request", json!({"call_id": "s1", "tool": "search_paper_passages"})),
        ev(
            20,
            "tool.result",
            json!({"call_id": "s1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "passages": [
                    {"path": path, "start_line": 60, "end_line": 65},
                    {"path": "src-2/other.tex", "start_line": 70, "end_line": 75}
                ]
            })).unwrap()}}),
        ),
    ];

    let answer = concat!(
        "Offset [L12] and merged [L18–25]. ",
        "Grep [L40, L42] vs gap [L40-42]. ",
        "Search [L61-64] vs other paper [L72]. ",
        "Prior turn [L3], workspace [L85], unread [L50]. ",
        "Invalid: out of range [L101], reversed [L20-15], zero [L0]."
    );

    let cites = answer_cites(Some(&ledger), &events, 10, path, Some(100), answer);
    assert_eq!(
        cites,
        vec![
            json!({"start": 12, "end": 12, "read": true}),
            json!({"start": 18, "end": 25, "read": true}),
            json!({"start": 40, "end": 40, "read": true}),
            json!({"start": 42, "end": 42, "read": true}),
            json!({"start": 40, "end": 42, "read": false}),
            json!({"start": 61, "end": 64, "read": true}),
            json!({"start": 72, "end": 72, "read": false}),
            json!({"start": 3, "end": 3, "read": false}),
            json!({"start": 85, "end": 85, "read": false}),
            json!({"start": 50, "end": 50, "read": false}),
        ]
    );
}

#[test]
fn read_paper_lines_marks_cites_read() {
    let path = "src-1/paper.txt";
    let events = vec![
        ev(10, "investigation.question_requested", json!({"source_id": "src-1"})),
        ev(11, "tool.request", json!({"call_id": "r1", "tool": "read_paper_lines"})),
        ev(
            12,
            "tool.result",
            json!({"call_id": "r1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "a_meta": {
                    "path": path,
                    "start_line": 15,
                    "end_line": 28,
                    "total_lines": 50
                },
                "text": "15\tline"
            })).unwrap()}}),
        ),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        None,
        "Read [L16-20], unread [L30], out of bounds [L55].",
    );
    assert_eq!(
        cites,
        vec![
            json!({"start": 16, "end": 20, "read": true}),
            json!({"start": 30, "end": 30, "read": false}),
        ]
    );
}

#[test]
fn caller_denylist_and_unmatched_results() {
    let path = "src-1/paper.txt";
    let res = |cid: &str, s: u64, e: u64| {
        ev(20, "tool.result", json!({"call_id": cid, "ok": true, "result": {"inline":
            serde_json::to_string(&json!({"root":"sources","path":path,"start_line":s,"end_line":e,"total_lines":100})).unwrap()}}))
    };
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "w", "tool": "fs_read", "caller": "worker:lit"})),
        res("w", 1, 5),
        ev(12, "tool.request", json!({"call_id": "u", "tool": "fs_read", "caller": "user:canvas"})),
        res("u", 6, 10),
        ev(13, "tool.request", json!({"call_id": "e", "tool": "fs_read", "caller": ""})),
        res("e", 11, 15),
        ev(14, "tool.request", json!({"call_id": "n", "tool": "fs_read", "caller": 42})),
        res("n", 16, 20),
        res("unmatched", 21, 25),
        ev(15, "tool.request", json!({"call_id": "a", "tool": "fs_read", "caller": "agent:s1"})),
        res("a", 30, 32),
        ev(16, "tool.request", json!({"call_id": "c", "tool": "fs_read", "caller": "claude-code-cli:s1"})),
        res("c", 40, 42),
        ev(17, "tool.request", json!({"call_id": "g", "tool": "fs_read", "caller": "antigravity:s1"})),
        res("g", 50, 52),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        Some(100),
        "[L2] [L7] [L12] [L17] [L22] [L31] [L41] [L51]",
    );
    let flags: Vec<bool> = cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
    assert_eq!(flags, vec![false, false, true, false, false, true, true, true]);
}

#[test]
fn truncated_empty_and_partial_reads_excluded() {
    let path = "src-1/paper.txt";
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "rpl", "tool": "read_paper_lines", "caller": "agent:s1"})),
        ev(12, "tool.result", json!({"call_id": "rpl", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "a_meta": {"path": path, "start_line": 1, "end_line": 1, "total_lines": 30, "line_truncated": true}
        })).unwrap()}})),
        ev(13, "tool.request", json!({"call_id": "eof", "tool": "fs_read", "caller": "agent:s1"})),
        ev(14, "tool.result", json!({"call_id": "eof", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "root": "sources", "path": path, "start_line": 30, "end_line": 30, "returned_chars": 0, "content": ""
        })).unwrap()}})),
        ev(15, "tool.request", json!({"call_id": "mid", "tool": "fs_read", "caller": "agent:s1"})),
        ev(16, "tool.result", json!({"call_id": "mid", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "root": "sources", "path": path, "start_line": 5, "end_line": 8, "offset": 12, "truncated": true, "content": "tail\n6\n7\nhead"
        })).unwrap()}})),
        ev(17, "tool.request", json!({"call_id": "nl", "tool": "fs_read", "caller": "agent:s1"})),
        ev(18, "tool.result", json!({"call_id": "nl", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "root": "sources", "path": path, "start_line": 10, "end_line": 12, "offset": 0, "truncated": true, "content": "10\n11\n12\n"
        })).unwrap()}})),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        None,
        "[L1] [L30] [L5] [L6-7] [L8] [L10-12] [L31]",
    );
    let got: Vec<(u64, u64, bool)> = cites
        .iter()
        .map(|c| {
            (
                c["start"].as_u64().unwrap(),
                c["end"].as_u64().unwrap(),
                c["read"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (1, 1, false),
            (30, 30, false),
            (5, 5, true),
            (6, 7, true),
            (8, 8, true),
            (10, 12, true)
        ]
    );
}

#[test]
fn review_honesty_f10_wide_cite_stays_read_true() {
    let path = "src-1/wide.txt";
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "a", "tool": "read_paper_lines", "caller": "agent:s1"})),
        ev(12, "tool.result", json!({"call_id": "a", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "a_meta": {"path": path, "start_line": 1, "end_line": 600, "total_lines": 600}
        })).unwrap()}})),
    ];
    let cites = answer_cites(None, &events, 10, path, Some(600), "[L1-513] [L1-550]");
    assert_eq!(
        cites,
        vec![
            json!({"start": 1, "end": 513, "read": true}),
            json!({"start": 1, "end": 550, "read": true}),
        ]
    );
}

#[test]
fn review_honesty_f1_custom_main_caller_allowed() {
    let path = "src-1/paper.txt";
    let res = |seq: u64, cid: &str, s: u64, e: u64| {
        ev(
            seq,
            "tool.result",
            json!({"call_id": cid, "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": s, "end_line": e, "total_lines": 100
            })).unwrap()}}),
        )
    };
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "c1", "tool": "fs_read", "caller": "codex-cli:s1"})),
        res(12, "c1", 1, 5),
        ev(13, "tool.request", json!({"call_id": "c2", "tool": "fs_read", "caller": "future-route:s1"})),
        res(14, "c2", 10, 12),
        ev(15, "tool.request", json!({"call_id": "c3", "tool": "fs_read", "caller": ""})),
        res(16, "c3", 20, 22),
        ev(17, "tool.request", json!({"call_id": "c4", "tool": "fs_read"})),
        res(18, "c4", 30, 32),
        ev(19, "tool.request", json!({"call_id": "w1", "tool": "fs_read", "caller": "worker:reader"})),
        res(20, "w1", 40, 42),
        ev(21, "tool.request", json!({"call_id": "u1", "tool": "fs_read", "caller": "user:ui"})),
        res(22, "u1", 50, 52),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        Some(100),
        "[L3] [L11] [L21] [L31] [L41] [L51]",
    );
    let flags: Vec<bool> = cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
    assert_eq!(flags, vec![true, true, true, true, false, false]);
}

#[test]
fn review_honesty_m3_unmatched_tool_result_not_credited() {
    let path = "src-1/paper.txt";
    let events = vec![
        // Request before request_seq=10 must not match a result after request_seq.
        ev(5, "tool.request", json!({"call_id": "stale", "tool": "fs_read", "caller": "agent:s1"})),
        ev(10, "investigation.question_requested", json!({"source_id": "src-1"})),
        ev(
            11,
            "tool.result",
            json!({"call_id": "stale", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 1, "end_line": 5, "total_lines": 50
            })).unwrap()}}),
        ),
        ev(
            12,
            "tool.result",
            json!({"call_id": "missing_req_rpl", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "a_meta": {"path": path, "start_line": 10, "end_line": 15, "total_lines": 50}
            })).unwrap()}}),
        ),
        ev(
            13,
            "tool.result",
            json!({"call_id": "missing_req_spp", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "passages": [{"path": path, "start_line": 20, "end_line": 25}]
            })).unwrap()}}),
        ),
        ev(
            14,
            "tool.result",
            json!({"ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 30, "end_line": 35, "total_lines": 50
            })).unwrap()}}),
        ),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        Some(50),
        "[L2] [L12] [L22] [L32]",
    );
    let flags: Vec<bool> = cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
    assert_eq!(flags, vec![false, false, false, false]);
}

#[test]
fn review_honesty_m1_grep_truncated_line_not_credited() {
    let path = "src-1/paper.txt";
    let trunc_end = format!("{}...", "a".repeat(500));
    let trunc_start = format!("...{}", "b".repeat(500));
    let short_ellipsis = format!("{}...", "c".repeat(40));
    let long_no_ellipsis = "d".repeat(505);
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "g1", "tool": "fs_read", "caller": "agent:s1"})),
        ev(
            12,
            "tool.result",
            json!({"call_id": "g1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources",
                "path": path,
                "total_lines": 50,
                "matches": [
                    {"line": 10, "text": trunc_end},
                    {"line": 11, "text": trunc_start},
                    {"line": 12, "text": short_ellipsis},
                    {"line": 13, "text": long_no_ellipsis}
                ]
            })).unwrap()}}),
        ),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        Some(50),
        "[L10] [L11] [L12] [L13]",
    );
    let flags: Vec<bool> = cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
    assert_eq!(flags, vec![false, false, true, true]);
}

#[test]
fn review_honesty_m2_passage_cut_line_not_credited() {
    let path = "src-1/paper.txt";
    // Case 1: verified disk bytes available.
    let line1 = format!("{}END1\n", "a".repeat(100));
    let line2 = "line two complete\n";
    let line3 = "line three complete\n";
    let line4 = format!("START4{}END4\n", "b".repeat(100));
    let full = format!("{line1}{line2}{line3}{line4}");
    let l1_mid = 50usize;
    let l2_end = line1.len() + line2.len();
    let l3_start = l2_end;
    let l4_mid = l3_start + line3.len() + 30;

    let disk_events = vec![
        ev(11, "tool.request", json!({"call_id": "p1", "tool": "search_paper_passages", "caller": "agent:s1"})),
        ev(
            12,
            "tool.result",
            json!({"call_id": "p1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "passages": [
                    {
                        "path": path,
                        "start_line": 1,
                        "end_line": 2,
                        "start_byte": l1_mid,
                        "end_byte": l2_end,
                        "excerpt": &full[l1_mid..l2_end]
                    },
                    {
                        "path": path,
                        "start_line": 3,
                        "end_line": 4,
                        "start_byte": l3_start,
                        "end_byte": l4_mid,
                        "excerpt": &full[l3_start..l4_mid]
                    }
                ]
            })).unwrap()}}),
        ),
    ];
    let disk_cites = answer_cites_with_bytes(
        None,
        &disk_events,
        10,
        path,
        Some(4),
        Some(full.as_bytes()),
        "[L1] [L2] [L3] [L4]",
    );
    let disk_flags: Vec<bool> = disk_cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
    assert_eq!(disk_flags, vec![false, true, true, false]);

    // Case 2: without disk bytes (ledger-only fixture).
    let long_cut = "x".repeat(950);
    let mem_events = vec![
        ev(11, "tool.request", json!({"call_id": "p2", "tool": "search_paper_passages", "caller": "agent:s1"})),
        ev(
            12,
            "tool.result",
            json!({"call_id": "p2", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "passages": [
                    {
                        "path": path,
                        "start_line": 1,
                        "end_line": 2,
                        "start_byte": 40,
                        "end_byte": 90,
                        "excerpt": "tail1\nline2\n"
                    },
                    {
                        "path": path,
                        "start_line": 10,
                        "end_line": 12,
                        "excerpt": "line10\nline11\npartial12"
                    },
                    {
                        "path": path,
                        "start_line": 20,
                        "end_line": 20,
                        "excerpt": long_cut
                    }
                ]
            })).unwrap()}}),
        ),
    ];
    let mem_cites = answer_cites(
        None,
        &mem_events,
        10,
        path,
        Some(30),
        "[L1] [L2] [L10-11] [L12] [L20]",
    );
    let mem_flags: Vec<bool> = mem_cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
    assert_eq!(mem_flags, vec![false, true, true, false, false]);
}

fn test_desktop_state(data_dir: &std::path::Path) -> DesktopState {
    let cfg = crate::DesktopConfig {
        port: 0,
        data_dir: data_dir.to_path_buf(),
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: research_tools::LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(Default::default()),
        chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
    };
    let auth = crate::auth::AuthGuard::new(0).unwrap();
    let sse = std::sync::Arc::new(crate::sse::SseHub::new());
    let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
    DesktopState::new(cfg, auth, sse, chatgpt)
}

#[test]
fn review_honesty_late_m2_finish_question_append_error() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_desktop_state(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    ledger
        .append("s1", "investigation.started", 1, &json!({}))
        .unwrap();
    let seq = ledger
        .append(
            "s1",
            "investigation.question_requested",
            1,
            &json!({"source_id": "src-1", "question": "Q"}),
        )
        .unwrap();
    drop(ledger);

    let db_path = tmp.path().join("ledger.db");
    let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&db_path, perms).unwrap();

    let ro_ledger = Ledger::open_existing(tmp.path()).unwrap();
    let ok_res = crate::investigation_question::finish_question(
        &state,
        &ro_ledger,
        "s1",
        seq,
        Ok("Answer text".into()),
    );
    let err_res = crate::investigation_question::finish_question(
        &state,
        &ro_ledger,
        "s1",
        seq,
        Err("provider failed".into()),
    );
    drop(ro_ledger);

    let mut restore = std::fs::metadata(&db_path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    restore.set_readonly(false);
    let _ = std::fs::set_permissions(&db_path, restore);

    let err_msg = ok_res.expect_err("append failure must not return Ok");
    assert!(
        err_msg.contains("Could not save investigation answer:"),
        "unexpected error: {err_msg}"
    );
    assert_eq!(err_res, Err("provider failed".into()));
}

#[test]
fn review_flow_f9_early_stop_question_is_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_desktop_state(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    ledger
        .append("s1", "investigation.started", 1, &json!({}))
        .unwrap();
    let seq = ledger
        .append(
            "s1",
            "investigation.question_requested",
            1,
            &json!({"source_id": "src-1", "question": "Q"}),
        )
        .unwrap();

    let out = crate::investigation_question::finish_question(
        &state,
        &ledger,
        "s1",
        seq,
        Err("stopped by user".into()),
    );
    assert_eq!(out, Err("stopped by user".into()));

    let events = ledger.read_session("s1").unwrap();
    assert!(
        !events.iter().any(|e| e.kind == "session.abort"),
        "early stop must not invent a session.abort event"
    );
    let answer = events
        .iter()
        .find(|e| e.kind == "investigation.answer")
        .expect("investigation.answer recorded");
    assert_eq!(answer.payload["request_seq"], seq);
    assert_eq!(answer.payload["status"], "stopped");
    assert_eq!(answer.payload["error"], "stopped by user");
}

#[test]
fn review3_flow_f9_cli_early_stop_question_is_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_desktop_state(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    ledger
        .append("s1", "investigation.started", 1, &json!({}))
        .unwrap();
    let seq = ledger
        .append(
            "s1",
            "investigation.question_requested",
            1,
            &json!({"source_id": "src-1", "question": "Q"}),
        )
        .unwrap();

    let out = crate::investigation_question::finish_question(
        &state,
        &ledger,
        "s1",
        seq,
        Err("turn stopped by the user".into()),
    );
    assert_eq!(out, Err("turn stopped by the user".into()));

    let events = ledger.read_session("s1").unwrap();
    assert!(
        !events.iter().any(|e| e.kind == "session.abort"),
        "CLI early stop must not invent a session.abort event"
    );
    let answer = events
        .iter()
        .find(|e| e.kind == "investigation.answer")
        .expect("investigation.answer recorded");
    assert_eq!(answer.payload["request_seq"], seq);
    assert_eq!(answer.payload["status"], "stopped");
    assert_eq!(answer.payload["error"], "turn stopped by the user");
}

#[test]
fn review3_honesty_m2_no_bytes_start_byte_fails_closed() {
    let path = "src-1/paper.txt";
    let events = vec![
        ev(
            11,
            "tool.request",
            json!({"call_id": "p1", "tool": "search_paper_passages", "caller": "agent:s1"}),
        ),
        ev(
            12,
            "tool.result",
            json!({"call_id": "p1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "passages": [
                    {
                        "path": path,
                        "start_line": 3,
                        "end_line": 4,
                        "start_byte": 120,
                        "end_byte": 170,
                        "excerpt": "tail of line 3\nfull line 4\n"
                    },
                    {
                        "path": path,
                        "start_line": 9,
                        "end_line": 10,
                        "start_byte": 300,
                        "end_byte": 335,
                        "excerpt": "tail of line 9\nshort eof line 10"
                    }
                ]
            })).unwrap()}}),
        ),
    ];
    let cites = answer_cites(
        None,
        &events,
        10,
        path,
        Some(10),
        "[L3] [L4] [L9] [L10]",
    );
    assert_eq!(
        cites,
        vec![
            json!({"start": 3, "end": 3, "read": false}),
            json!({"start": 4, "end": 4, "read": true}),
            json!({"start": 9, "end": 9, "read": false}),
            json!({"start": 10, "end": 10, "read": true}),
        ]
    );

    // When total_lines is unknown, an oversized cut excerpt without trailing newline fails closed.
    let unknown_total_events = vec![
        ev(
            11,
            "tool.request",
            json!({"call_id": "p2", "tool": "search_paper_passages", "caller": "agent:s1"}),
        ),
        ev(
            12,
            "tool.result",
            json!({"call_id": "p2", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "passages": [
                    {
                        "path": path,
                        "start_line": 2,
                        "end_line": 2,
                        "excerpt": "x".repeat(950)
                    }
                ]
            })).unwrap()}}),
        ),
    ];
    let unknown_cites = answer_cites(None, &unknown_total_events, 10, path, None, "[L2]");
    assert_eq!(unknown_cites, vec![json!({"start": 2, "end": 2, "read": false})]);
}

#[test]
fn review3_honesty_f17_unselected_page_lines_stay_read() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_desktop_state(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let path = "src-pdf/paper.txt";
    let content = concat!(
        "p1 line 1\np1 line 2\np1 line 3\n",
        "p2 line 4\np2 line 5\np2 line 6\n",
        "p3 line 7\np3 line 8\np3 line 9\n"
    );
    let file_path = tmp.path().join("sessions/s1/sources").join(path);
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, content).unwrap();
    let sha = ledger::sha256_hex(content.as_bytes());

    ledger
        .append(
            "s1",
            "source.ingested",
            1,
            &json!({
                "source_id": "src-pdf",
                "format": "pdf",
                "primary_path": path,
                "files_sha256": { path: sha },
                "page_lines": [[1, 1, 3], [2, 4, 6], [3, 7, 9]]
            }),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "investigation.paper",
            1,
            &json!({
                "source_id": "src-pdf",
                "question": "Q",
                "page_start": 2,
                "page_end": 3
            }),
        )
        .unwrap();
    let req_seq = ledger
        .append(
            "s1",
            "investigation.question_requested",
            1,
            &json!({
                "source_id": "src-pdf",
                "question": "Q",
                "page_start": 2,
                "page_end": 3
            }),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "tool.request",
            1,
            &json!({"call_id": "rpl1", "tool": "read_paper_lines", "caller": "agent:s1"}),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "tool.result",
            1,
            &json!({"call_id": "rpl1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "a_meta": {"path": path, "start_line": 1, "end_line": 3, "total_lines": 9},
                "text": "1\tp1 line 1\n2\tp1 line 2\n3\tp1 line 3"
            })).unwrap()}}),
        )
        .unwrap();

    let events = ledger.read_session("s1").unwrap();
    let cites = answer_cites_for_session(
        &state,
        &ledger,
        "s1",
        &events,
        req_seq,
        "Page 1 [L1-3] was read, page 2 [L5] was not.",
    );
    assert_eq!(
        cites,
        vec![
            json!({"start": 1, "end": 3, "read": true}),
            json!({"start": 5, "end": 5, "read": false}),
        ]
    );
}

#[test]
fn review3_honesty_f16_no_files_sha256_entry_still_read() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_desktop_state(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let path = "src-legacy/paper.tex";

    ledger
        .append(
            "s1",
            "source.ingested",
            1,
            &json!({
                "source_id": "src-legacy",
                "format": "tex",
                "primary_path": path
            }),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "investigation.paper",
            1,
            &json!({"source_id": "src-legacy", "question": "Q"}),
        )
        .unwrap();
    let req_seq = ledger
        .append(
            "s1",
            "investigation.question_requested",
            1,
            &json!({"source_id": "src-legacy", "question": "Q"}),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "tool.request",
            1,
            &json!({"call_id": "rpl1", "tool": "read_paper_lines", "caller": "agent:s1"}),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "tool.result",
            1,
            &json!({"call_id": "rpl1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "a_meta": {"path": path, "start_line": 4, "end_line": 8, "total_lines": 20},
                "text": "4\tline 4\n5\tline 5"
            })).unwrap()}}),
        )
        .unwrap();

    let events = ledger.read_session("s1").unwrap();
    let cites = answer_cites_for_session(
        &state,
        &ledger,
        "s1",
        &events,
        req_seq,
        "Received [L5-7], unread [L12], out-of-bounds [L25].",
    );
    assert_eq!(
        cites,
        vec![
            json!({"start": 5, "end": 7, "read": true}),
            json!({"start": 12, "end": 12, "read": false}),
        ]
    );
}

