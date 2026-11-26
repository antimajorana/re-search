use super::*;
use serde_json::json;

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-lines-sent".into(),
        seq,
        ts_ms: 1_700_000_000_000 + seq as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: ledger::GENESIS_HASH.into(),
        event_hash: ledger::GENESIS_HASH.into(),
    }
}

fn supplied_event(seq: u64, run_id: Value, caller: &str, ranges: Value) -> Event {
    ev(
        seq,
        CONTEXT_SUPPLIED_EVENT,
        json!({ "run_id": run_id, "caller": caller, "ranges": ranges }),
    )
}

#[test]
fn merge_ranges_combines_overlapping_and_adjacent_spans() {
    let merged = SentLines::merge_ranges(vec![
        LineRange { start_line: 6, end_line: 10 },
        LineRange { start_line: 1, end_line: 5 },
        LineRange { start_line: 4, end_line: 8 },
        LineRange { start_line: 15, end_line: 18 },
        LineRange { start_line: 0, end_line: 3 },  // invalid, dropped
        LineRange { start_line: 12, end_line: 11 }, // inverted, dropped
    ]);
    assert_eq!(
        merged,
        vec![
            LineRange { start_line: 1, end_line: 10 },
            LineRange { start_line: 15, end_line: 18 },
        ]
    );

    let events = vec![supplied_event(
        1,
        json!("run-merge"),
        "reader-1",
        json!([
            { "source_id": "src1", "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 1, "end_line": 5 },
            { "source_id": "src1", "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 6, "end_line": 10 },
            { "source_id": "src1", "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 4, "end_line": 8 }
        ]),
    )];
    let sent = lines_sent(&events, Some("run-merge"));
    assert_eq!(
        sent.ranges_for("reader-1", "src1", "src1/paper.txt", HASH_A),
        &[LineRange { start_line: 1, end_line: 10 }]
    );
    assert!(sent.covers("reader-1", "src1", "src1/paper.txt", HASH_A, 5, 9));
    assert!(!sent.covers("reader-1", "src1", "src1/paper.txt", HASH_A, 9, 11));
    assert!(!sent.covers("reader-1", "src1", "src1/paper.txt", HASH_A, 0, 2));
}

#[test]
fn context_supplied_isolates_caller_hash_and_run_id() {
    let events = vec![
        supplied_event(
            1,
            json!("run-1"),
            "caller-a",
            json!([
                { "source_id": "src1", "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 10, "end_line": 20 }
            ]),
        ),
        supplied_event(
            2,
            json!(42u64),
            "caller-a",
            json!([
                { "source_id": "src1", "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 30, "end_line": 35 }
            ]),
        ),
    ];

    let for_run1 = lines_sent(&events, Some("run-1"));
    assert!(for_run1.covers("caller-a", "src1", "src1/paper.txt", HASH_A, 12, 18));
    // Caller B cannot cite lines sent only to caller A.
    assert!(!for_run1.covers("caller-b", "src1", "src1/paper.txt", HASH_A, 12, 18));
    // Wrong file_sha256 does not cover the real hash.
    assert!(!for_run1.covers("caller-a", "src1", "src1/paper.txt", HASH_B, 12, 18));
    // Event from run 42 is not credited under run-1.
    assert!(!for_run1.covers("caller-a", "src1", "src1/paper.txt", HASH_A, 31, 34));

    // Numeric run_id 42 matches string "42".
    let for_run42 = lines_sent(&events, Some("42"));
    assert!(for_run42.covers("caller-a", "src1", "src1/paper.txt", HASH_A, 31, 34));
    assert!(!for_run42.covers("caller-a", "src1", "src1/paper.txt", HASH_A, 12, 18));

    // Wrong run_id credits nothing.
    assert!(lines_sent(&events, Some("run-other")).is_empty());

    // None accepts all runs.
    let all = lines_sent(&events, None);
    assert!(all.covers("caller-a", "src1", "src1/paper.txt", HASH_A, 10, 20));
    assert!(all.covers("caller-a", "src1", "src1/paper.txt", HASH_A, 30, 35));
    assert_eq!(all.all_entries().len(), 1);
}

#[test]
fn read_paper_lines_credits_inline_blob_and_handles_line_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();

    let inline_result = json!({
        "a_meta": {
            "path": "src1/paper.txt",
            "file_sha256": HASH_A,
            "start_line": 1,
            "end_line": 4
        },
        "text": "1\ta\n2\tb\n3\tc\n4\td\n"
    });
    let blob_result = json!({
        "a_meta": {
            "path": "src1/paper.txt",
            "file_sha256": HASH_A,
            "start_line": 5,
            "end_line": 9
        },
        "text": "5\te\n6\tf\n7\tg\n8\th\n9\ti\n"
    });
    let blob_hash = ledger.put_blob(blob_result.to_string().as_bytes()).unwrap();

    let truncated_single = json!({
        "a_meta": {
            "path": "src1/paper.txt",
            "file_sha256": HASH_A,
            "start_line": 20,
            "end_line": 20,
            "line_truncated": true
        },
        "text": "20\tpartial..."
    });
    let truncated_multi = json!({
        "a_meta": {
            "path": "src1/paper.txt",
            "file_sha256": HASH_A,
            "start_line": 30,
            "end_line": 33,
            "line_truncated": true
        },
        "text": "30\ta\n31\tb\n32\tc\n33\tpartial"
    });

    let events = vec![
        ev(
            1,
            "tool.request",
            json!({
                "call_id": "c1",
                "caller": "worker-a",
                "tool": "read_paper_lines",
                "args": { "inline": json!({"source_id": "src1", "start_line": 1, "end_line": 4}).to_string() }
            }),
        ),
        ev(
            2,
            "tool.result",
            json!({ "call_id": "c1", "ok": true, "result": { "inline": inline_result.to_string() } }),
        ),
        ev(
            3,
            "tool.request",
            json!({ "call_id": "c2", "caller": "worker-a", "tool": "read_paper_lines" }),
        ),
        ev(
            4,
            "tool.result",
            json!({ "call_id": "c2", "ok": true, "result": { "blob": blob_hash } }),
        ),
        ev(
            5,
            "tool.request",
            json!({ "call_id": "c3", "caller": "worker-a", "tool": "read_paper_lines" }),
        ),
        ev(
            6,
            "tool.result",
            json!({ "call_id": "c3", "ok": true, "result": { "inline": truncated_single.to_string() } }),
        ),
        ev(
            7,
            "tool.request",
            json!({ "call_id": "c4", "caller": "worker-a", "tool": "read_paper_lines" }),
        ),
        ev(
            8,
            "tool.result",
            json!({ "call_id": "c4", "ok": true, "result": { "inline": truncated_multi.to_string() } }),
        ),
        ev(
            9,
            "tool.request",
            json!({ "call_id": "c5", "caller": "worker-a", "tool": "read_paper_lines" }),
        ),
        ev(
            10,
            "tool.result",
            json!({
                "call_id": "c5",
                "ok": false,
                "result": {
                    "inline": json!({
                        "a_meta": { "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 50, "end_line": 55 },
                        "text": "50\tfailed\n"
                    }).to_string()
                }
            }),
        ),
        ev(
            11,
            "tool.request",
            json!({ "call_id": "c6", "caller": "worker-a", "tool": "read_paper_lines", "run_id": "run-other" }),
        ),
        ev(
            12,
            "tool.result",
            json!({
                "call_id": "c6",
                "ok": true,
                "result": {
                    "inline": json!({
                        "a_meta": { "path": "src1/paper.txt", "file_sha256": HASH_A, "start_line": 60, "end_line": 62 },
                        "text": "60\tother\n"
                    }).to_string()
                }
            }),
        ),
    ];

    // Without ledger, only inline c1 (1..=4) and c4 (30..=32) are credited; blob c2 is skipped.
    let without_ledger = lines_sent(&events, Some("run-any"));
    assert!(without_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 1, 4));
    assert!(!without_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 5, 9));

    // With ledger, c1 (1..=4) and blob c2 (5..=9) merge into 1..=9.
    let with_ledger = lines_sent_with_ledger(&events, Some(&ledger), Some("run-any"));
    assert!(with_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 1, 9));
    // Single truncated line 20 is NOT credited.
    assert!(!with_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 20, 20));
    // Multi-line truncated result credits 30..=32, dropping line 33.
    assert!(with_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 30, 32));
    assert!(!with_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 33, 33));
    // Failed tool.result (ok: false) is NOT credited.
    assert!(!with_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 50, 55));
    // tool.request carrying a different run_id is NOT credited for "run-any", but IS for "run-other".
    assert!(!with_ledger.covers("worker-a", "src1", "src1/paper.txt", HASH_A, 60, 62));
    assert!(lines_sent(&events, Some("run-other")).covers("worker-a", "src1", "src1/paper.txt", HASH_A, 60, 62));
    // Another caller cannot use worker-a's tool results.
    assert!(!with_ledger.covers("worker-b", "src1", "src1/paper.txt", HASH_A, 1, 4));
}

#[test]
fn search_paper_passages_and_fs_read_crediting_rules() {
    let long_unterminated = "x".repeat(950);
    let passages_res = json!({
        "passages": [
            {
                "source_id": "src1",
                "path": "src1/notes.txt",
                "file_sha256": HASH_A,
                "start_line": 4,
                "end_line": 7,
                "start_byte": 50,
                "excerpt": "Complete passage lines.\n"
            },
            {
                "source_id": "src1",
                "path": "src1/notes.txt",
                "file_sha256": HASH_A,
                "start_line": 1,
                "end_line": 3,
                "start_byte": 120,
                "excerpt": "tail of line 1\nline 2\nline 3\n"
            },
            {
                "source_id": "src1",
                "path": "src1/notes.txt",
                "file_sha256": HASH_A,
                "start_line": 10,
                "end_line": 12,
                "start_byte": 400,
                "excerpt": long_unterminated
            }
        ]
    });

    let clipped_line = format!("{}...", "a".repeat(500));
    let fs_query_res = json!({
        "root": "sources",
        "path": "src2/spec.txt",
        "file_sha256": HASH_B,
        "matches": [
            { "line": 3, "text": "Short complete matching line." },
            { "line": 4, "text": clipped_line }
        ],
        "content": "3: Short complete matching line.\n"
    });
    let fs_slice_res = json!({
        "root": "sources",
        "path": "src2/spec.txt",
        "file_sha256": HASH_B,
        "content": "alpha\nbeta\ngamma\n",
        "returned_chars": 17,
        "start_line": 10,
        "end_line": 12
    });
    let fs_workspace_res = json!({
        "root": "workspace",
        "path": "src2/spec.txt",
        "file_sha256": HASH_B,
        "content": "workspace content\n",
        "returned_chars": 18,
        "start_line": 20,
        "end_line": 22
    });
    let fs_empty_res = json!({
        "root": "sources",
        "path": "src2/spec.txt",
        "file_sha256": HASH_B,
        "content": "",
        "returned_chars": 0,
        "start_line": 30,
        "end_line": 30
    });

    let events = vec![
        ev(1, "tool.request", json!({ "call_id": "p1", "caller": "agent", "tool": "search_paper_passages" })),
        ev(2, "tool.result", json!({ "call_id": "p1", "ok": true, "result": passages_res })),
        ev(3, "tool.request", json!({ "call_id": "f1", "caller": "agent", "tool": "fs_read" })),
        ev(4, "tool.result", json!({ "call_id": "f1", "ok": true, "result": fs_query_res })),
        ev(5, "tool.request", json!({ "call_id": "f2", "caller": "agent", "tool": "fs_read" })),
        ev(6, "tool.result", json!({ "call_id": "f2", "ok": true, "result": fs_slice_res })),
        ev(7, "tool.request", json!({ "call_id": "f3", "caller": "agent", "tool": "fs_read" })),
        ev(8, "tool.result", json!({ "call_id": "f3", "ok": true, "result": fs_workspace_res })),
        ev(9, "tool.request", json!({ "call_id": "f4", "caller": "agent", "tool": "fs_read" })),
        ev(10, "tool.result", json!({ "call_id": "f4", "ok": true, "result": fs_empty_res })),
        ev(11, "tool.request", json!({ "call_id": "m1", "caller": "agent", "tool": READ_MAP_NODE_TOOL })),
        ev(12, "tool.result", json!({ "call_id": "m1", "ok": true, "result": { "node": "n-demo" } })),
    ];

    let sent = lines_sent(&events, None);
    // Passage 1: 4..=7; Passage 2: start_line 1 with start_byte>0 advances to 2..=3 -> merges to 2..=7!
    assert!(!sent.covers("agent", "src1", "src1/notes.txt", HASH_A, 1, 1));
    assert!(sent.covers("agent", "src1", "src1/notes.txt", HASH_A, 2, 7));
    // Passage 3: long unterminated excerpt decrements end_line from 12 to 11.
    assert!(sent.covers("agent", "src1", "src1/notes.txt", HASH_A, 10, 11));
    assert!(!sent.covers("agent", "src1", "src1/notes.txt", HASH_A, 12, 12));

    // fs_read query mode credits non-clipped line 3, skips clipped line 4.
    assert!(sent.covers("agent", "src2", "src2/spec.txt", HASH_B, 3, 3));
    assert!(!sent.covers("agent", "src2", "src2/spec.txt", HASH_B, 4, 4));
    // fs_read slice mode credits sources root 10..=12, ignores workspace root 20..=22 and empty 30.
    assert!(sent.covers("agent", "src2", "src2/spec.txt", HASH_B, 10, 12));
    assert!(!sent.covers("agent", "src2", "src2/spec.txt", HASH_B, 20, 22));
    assert!(!sent.covers("agent", "src2", "src2/spec.txt", HASH_B, 30, 30));
}

#[test]
fn context_supplied_payload_validates_and_round_trips() {
    let payload = ContextSuppliedPayload::new(
        "run-7",
        "worker:reader:1",
        vec![SuppliedRange {
            source_id: "src1".into(),
            path: "src1/paper.txt".into(),
            file_sha256: HASH_A.into(),
            start_line: 2,
            end_line: 8,
        }],
    )
    .unwrap();
    let encoded = serde_json::to_string(&payload).unwrap();
    let decoded: ContextSuppliedPayload = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, payload);
    assert!(decoded.validate().is_ok());

    // Reject unknown fields.
    assert!(serde_json::from_value::<ContextSuppliedPayload>(json!({
        "run_id": "run-7",
        "caller": "worker:reader:1",
        "ranges": [],
        "extra": true
    }))
    .is_err());

    // Validation failures.
    assert!(ContextSuppliedPayload::new(" ", "caller", vec![]).is_err());
    assert!(ContextSuppliedPayload::new("run-1", "", vec![]).is_err());
    assert!(ContextSuppliedPayload::new(
        "run-1",
        "caller",
        vec![SuppliedRange {
            source_id: "src1".into(),
            path: "src1/paper.txt".into(),
            file_sha256: "not-a-sha256".into(),
            start_line: 1,
            end_line: 2,
        }],
    )
    .is_err());
    assert!(ContextSuppliedPayload::new(
        "run-1",
        "caller",
        vec![SuppliedRange {
            source_id: "src1".into(),
            path: "src1/paper.txt".into(),
            file_sha256: HASH_A.into(),
            start_line: 5,
            end_line: 4,
        }],
    )
    .is_err());
}

#[test]
fn read_map_node_credits_hash_pinned_delivered_lines_only() {
    let result_ok = json!({
        "source_id": "src1",
        "tag": "n-abc1234567",
        "files": [
            {
                "section": "statement",
                "source_id": "src1",
                "path": "src1/main.tex",
                "file_sha256": HASH_A,
                "start_line": 10,
                "end_line": 14,
                "line_truncated": false,
                "text": "10\tstatement\n"
            },
            {
                "section": "proof",
                "source_id": "src1",
                "path": "src1/main.tex",
                "file_sha256": HASH_A,
                "start_line": 20,
                "end_line": 23,
                "line_truncated": true,
                "text": "20\tproof\n"
            },
            {
                "section": "statement",
                "source_id": "src1",
                "path": "src1/bad.tex",
                "file_sha256": "not-a-hash",
                "start_line": 1,
                "end_line": 5,
                "line_truncated": false,
                "text": "1\tbad\n"
            }
        ],
        "truncated": true
    });
    let events = [
        ev(1, "tool.request", json!({ "call_id": "rmn-1", "caller": "agent:anthropic:s1", "tool": READ_MAP_NODE_TOOL })),
        ev(2, "tool.result", json!({ "call_id": "rmn-1", "ok": true, "result": result_ok })),
        ev(3, "tool.request", json!({ "call_id": "rmn-2", "caller": "agent:anthropic:s1", "tool": READ_MAP_NODE_TOOL })),
        ev(4, "tool.result", json!({ "call_id": "rmn-2", "ok": false, "result": result_ok })),
    ];
    let sent = lines_sent(&events, None);
    assert!(sent.covers("agent:anthropic:s1", "src1", "src1/main.tex", HASH_A, 10, 14));
    // line_truncated=true drops line 23 while keeping 20..=22.
    assert!(sent.covers("agent:anthropic:s1", "src1", "src1/main.tex", HASH_A, 20, 22));
    assert!(!sent.covers("agent:anthropic:s1", "src1", "src1/main.tex", HASH_A, 23, 23));
    // Wrong hash or different caller is rejected.
    assert!(!sent.covers("agent:anthropic:s1", "src1", "src1/main.tex", HASH_B, 10, 14));
    assert!(!sent.covers("worker:other", "src1", "src1/main.tex", HASH_A, 10, 14));
    assert!(!sent.covers("agent:anthropic:s1", "src1", "src1/bad.tex", "not-a-hash", 1, 5));
}
