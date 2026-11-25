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
fn caller_allowlist_and_unmatched_results() {
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
    assert_eq!(flags, vec![false, false, false, false, false, true, true, true]);
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
            (5, 5, false),
            (6, 7, true),
            (8, 8, false),
            (10, 12, true)
        ]
    );
}

#[test]
fn wide_citation_above_513_lines_stays_unread() {
    let path = "src-1/wide.txt";
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "a", "tool": "read_paper_lines", "caller": "agent:s1"})),
        ev(12, "tool.result", json!({"call_id": "a", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "a_meta": {"path": path, "start_line": 1, "end_line": 600, "total_lines": 600}
        })).unwrap()}})),
    ];
    let cites = answer_cites(None, &events, 10, path, Some(600), "[L1-513] [L1-514]");
    assert_eq!(
        cites,
        vec![
            json!({"start": 1, "end": 513, "read": true}),
            json!({"start": 1, "end": 514, "read": false})
        ]
    );
}

#[test]
fn expected_sha_filters_mismatched_reads_and_total_lines() {
    let path = "src-1/paper.txt";
    let good = "good_sha_hex";
    let bad = "tampered_sha_hex";
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "f_bad", "tool": "fs_read", "caller": "agent:s1"})),
        ev(12, "tool.result", json!({"call_id": "f_bad", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "root": "sources", "path": path, "file_sha256": bad, "start_line": 1, "end_line": 50, "total_lines": 50
        })).unwrap()}})),
        ev(13, "tool.request", json!({"call_id": "r_bad", "tool": "read_paper_lines", "caller": "agent:s1"})),
        ev(14, "tool.result", json!({"call_id": "r_bad", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "a_meta": {"path": path, "file_sha256": bad, "start_line": 10, "end_line": 20, "total_lines": 99}
        })).unwrap()}})),
        ev(15, "tool.request", json!({"call_id": "s_mix", "tool": "search_paper_passages", "caller": "agent:s1"})),
        ev(16, "tool.result", json!({"call_id": "s_mix", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "passages": [
                {"path": path, "file_sha256": bad, "start_line": 30, "end_line": 35},
                {"path": path, "file_sha256": good, "start_line": 3, "end_line": 5}
            ]
        })).unwrap()}})),
        ev(17, "tool.request", json!({"call_id": "r_good", "tool": "read_paper_lines", "caller": "agent:s1"})),
        ev(18, "tool.result", json!({"call_id": "r_good", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "a_meta": {"path": path, "file_sha256": good, "start_line": 1, "end_line": 2, "total_lines": 10}
        })).unwrap()}})),
    ];
    let (ranges, observed_total) =
        collect_received_ranges(None, &events, 10, path, Some(good));
    assert_eq!(ranges, vec![(1, 5)]);
    assert_eq!(observed_total, Some(10));
}

#[test]
fn pinned_pdf_page_bounds_gate_read_flag_and_fail_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = Ledger::open(dir.path()).expect("open ledger");
    let receipt = json!({
        "format": "pdf",
        "page_count": 2,
        "pdf_page_lines": [[1, 1, 10], [2, 11, 20]]
    });
    assert_eq!(
        selected_page_line_bounds(&ledger, &json!({}), &receipt),
        PageLineBounds::Unrestricted
    );
    assert_eq!(
        selected_page_line_bounds(&ledger, &json!({"page_start": 2, "page_end": 2}), &receipt),
        PageLineBounds::Allowed(11, 20)
    );
    let broken_receipt = json!({
        "format": "pdf",
        "page_count": 2,
        "raw_blob": "missing-blob"
    });
    assert_eq!(
        selected_page_line_bounds(
            &ledger,
            &json!({"page_start": 2, "page_end": 2}),
            &broken_receipt
        ),
        PageLineBounds::Denied
    );

    let path = "pdf1/extracted.txt";
    let events = vec![
        ev(11, "tool.request", json!({"call_id": "r", "tool": "read_paper_lines", "caller": "agent:s1"})),
        ev(12, "tool.result", json!({"call_id": "r", "ok": true, "result": {"inline": serde_json::to_string(&json!({
            "a_meta": {"path": path, "start_line": 1, "end_line": 20, "total_lines": 20}
        })).unwrap()}})),
    ];
    let cites = answer_cites_checked(
        None,
        &events,
        10,
        path,
        None,
        Some(20),
        PageLineBounds::Allowed(11, 20),
        "[L3-4] [L10-12] [L12-15]",
    );
    assert_eq!(
        cites,
        vec![
            json!({"start": 3, "end": 4, "read": false}),
            json!({"start": 10, "end": 12, "read": false}),
            json!({"start": 12, "end": 15, "read": true}),
        ]
    );
    let denied = answer_cites_checked(
        None,
        &events,
        10,
        path,
        None,
        Some(20),
        PageLineBounds::Denied,
        "[L12-15]",
    );
    assert_eq!(denied, vec![json!({"start": 12, "end": 15, "read": false})]);
}
