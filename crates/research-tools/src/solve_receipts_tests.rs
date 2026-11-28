use super::*;
use ledger::Ledger;
use serde_json::json;
use tempfile::TempDir;

#[test]
fn build_receipts_handles_object_and_array_exprs_out_and_harvested_handles() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-rcpt-1";

    ledger
        .append(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({ "call_id": "c-obj", "tool": "sympy_eval", "caller": "solver" }),
        )
        .unwrap();
    let inline_obj = serde_json::to_string(&json!({
        "value": "42",
        "error": null,
        "code_sha256": "aaa111",
        "wall_ms": 17,
        "process": "solver",
        "exprs_out": { "V1": "Integer(42)", "V2": "Symbol('x')" },
        "env": { "sympy": "1.14.0" }
    }))
    .unwrap();
    let rseq1 = ledger
        .append(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": "c-obj", "ok": true, "result": { "inline": inline_obj } }),
        )
        .unwrap();

    ledger
        .append(
            sid,
            event_kinds::SOLVE_EXPR,
            1,
            &json!({
                "name": "V1",
                "version": 1,
                "srepr_blob": "b1",
                "srepr_sha256": "b1",
                "latex": "42",
                "str_preview": "42",
                "free_symbols": [],
                "produced_by": { "call_id": "c-obj", "result_seq": rseq1 },
                "portable": true
            }),
        )
        .unwrap();

    // Blob-backed result (with array `exprs_out`) decoded cleanly through Ledger.
    ledger
        .append(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({ "call_id": "c-blob", "tool": "sympy_eval", "caller": "solver" }),
        )
        .unwrap();
    let blob_bytes = serde_json::to_vec(&json!({
        "value": "x**2",
        "error": null,
        "code_sha256": "bbb222",
        "wall_ms": 9,
        "process": "solver",
        "exprs_out": ["I_d@1"]
    }))
    .unwrap();
    let blob_hash = ledger.put_blob(&blob_bytes).unwrap();
    let rseq2 = ledger
        .append(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": "c-blob", "ok": true, "result": { "blob": blob_hash } }),
        )
        .unwrap();

    // Missing blob fails closed on `sympy_succeeded` without erroring (`00-overview.md` 3.4).
    ledger
        .append(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({ "call_id": "c-miss", "tool": "sympy_eval", "caller": "solver" }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": "c-miss", "ok": true, "result": { "blob": "0".repeat(64) } }),
        )
        .unwrap();

    let events = ledger.read_session(sid).unwrap();
    let receipts = build_receipts(&events, Some(&ledger));
    let r1 = receipts.get("c-obj").unwrap();
    assert_eq!(r1.tool, "sympy_eval");
    assert_eq!(r1.caller.as_deref(), Some("solver"));
    assert!(r1.sympy_succeeded);
    assert_eq!(r1.exprs_out, vec!["V1".to_string(), "V2".to_string()]);
    assert_eq!(r1.harvested, vec![HandleRef::parse("V1@1").unwrap()]);
    assert_eq!(r1.code_sha256.as_deref(), Some("aaa111"));
    assert_eq!(r1.wall_ms, Some(17));
    assert_eq!(r1.process, Some(ReceiptProcess::Solver));

    let r2 = receipts.get("c-blob").unwrap();
    assert!(r2.sympy_succeeded);
    assert!(!r2.result_missing);
    assert_eq!(r2.exprs_out, vec!["I_d@1".to_string()]);
    assert_eq!(
        find_receipt(&receipts, &format!("seq:{rseq2}")).map(|r| r.call_id.as_str()),
        Some("c-blob")
    );
    assert_eq!(
        find_receipt(&receipts, &format!("c-blob@seq:{rseq2}")).map(|r| r.call_id.as_str()),
        Some("c-blob")
    );

    let r3 = receipts.get("c-miss").unwrap();
    assert!(r3.result_missing);
    assert!(!r3.sympy_succeeded);

    let cited = resolve_cited_receipts(&receipts, &["c-obj".into()]).unwrap();
    assert!(has_successful_sympy_receipt(&cited));
    let cited_miss = resolve_cited_receipts(&receipts, &["c-miss".into()]).unwrap();
    assert!(!has_successful_sympy_receipt(&cited_miss));
    assert!(resolve_cited_receipts(&receipts, &["no-such-call".into()]).is_err());
}
