use super::*;
use crate::ingest::{ingest_bytes, IngestRequest};
use crate::quote_core::{clip_quote, verify_quote, EvidenceModeMeta, EvidenceQuotePayload};
use crate::solve_model::{CheckOutcome, CheckRequester, ReceiptRef, SolveCheck};
use ledger::sha256_hex;
use serde_json::json;
use std::sync::Mutex;
use tempfile::TempDir;

fn setup_env() -> (TempDir, ResearchEnv, Ledger, String) {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    let ledger = Ledger::open(&data).unwrap();
    let env = ResearchEnv::with_workspace(data, ws).unwrap();
    let sid = "s-steps-1".to_string();
    (tmp, env, ledger, sid)
}

fn seed_sympy_and_expr(
    ledger: &Ledger,
    sid: &str,
    call_id: &str,
    ok: bool,
    exprs: &[(&str, u32, &str, &str, bool)],
) -> u64 {
    ledger
        .append(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({ "call_id": call_id, "tool": "sympy_eval", "caller": "solver" }),
        )
        .unwrap();
    let inline = if ok {
        serde_json::to_string(&json!({ "value": "ok", "error": null, "process": "solver" })).unwrap()
    } else {
        serde_json::to_string(&json!({ "value": null, "error": "ZeroDivisionError", "process": "solver" })).unwrap()
    };
    let rseq = ledger
        .append(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({ "call_id": call_id, "ok": ok, "result": { "inline": inline } }),
        )
        .unwrap();
    for &(name, version, latex, srepr, portable) in exprs {
        let sha = sha256_hex(srepr.as_bytes());
        ledger.put_blob(srepr.as_bytes()).unwrap();
        ledger
            .append(
                sid,
                event_kinds::SOLVE_EXPR,
                1,
                &json!({
                    "name": name,
                    "version": version,
                    "srepr_blob": if portable { sha.clone() } else { String::new() },
                    "srepr_sha256": if portable { sha } else { String::new() },
                    "latex": latex,
                    "str_preview": srepr,
                    "free_symbols": [],
                    "produced_by": { "call_id": call_id, "result_seq": rseq },
                    "portable": portable
                }),
            )
            .unwrap();
    }
    rseq
}

#[test]
fn step_record_enforces_r5_r9_r10_handles_and_derived_deps() {
    let (_tmp, env, ledger, sid) = setup_env();
    let math_sha = "0".repeat(64);
    ledger
        .append(
            &sid,
            event_kinds::SOLVE_SETUP,
            1,
            &json!({
                "revision": 1,
                "statement": "Test problem",
                "task_kind": "calculate",
                "target": { "description": "x", "form": "closed_form" },
                "roles": {
                    "solver": { "provider": "anthropic", "model": "claude-sonnet-4-6" },
                    "verifier": { "provider": "openai", "model": "gpt-5" }
                },
                "tool_profile": "solve/1",
                "math_sha256": math_sha,
            }),
        )
        .unwrap();
    // Seed an initial Given assumption e-1.
    ledger
        .append(
            &sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": "assumption",
                "summary": "V_tree definition",
                "details": "lambda * phi**4 / 24",
                "depends_on": [],
                "origin": "setup",
                "entry_sha256": compute_entry_sha256(None, "V_tree definition", "lambda * phi**4 / 24"),
            }),
        )
        .unwrap();

    // R5: rejects `latex` field.
    let err = run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "assumption",
            "summary": "bad",
            "details": "bad",
            "latex": "x=1"
        }),
    )
    .unwrap_err();
    assert!(err.contains("latex"));

    // R9: derivation/result/final_answer require non-empty `depends_on`.
    for ty in ["derivation", "result", "final_answer"] {
        let err = run_step_record(
            &env,
            &ledger,
            &sid,
            "solver",
            &json!({
                "entry_type": ty,
                "summary": "needs deps",
                "details": "body",
                "depends_on": []
            }),
        )
        .unwrap_err();
        assert!(err.contains("R9"), "expected R9 refusal for {ty}: {err}");
    }

    // R10: symbolic_computation requires successful receipt.
    let fail_call = "call-fail";
    seed_sympy_and_expr(&ledger, &sid, fail_call, false, &[]);
    let err = run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "failed run",
            "details": "details",
            "tool_call_refs": [fail_call]
        }),
    )
    .unwrap_err();
    assert!(err.contains("R10"), "unexpected error: {err}");

    // Unknown, non-portable, and superseded handles refused.
    seed_sympy_and_expr(
        &ledger,
        &sid,
        "call-1",
        true,
        &[
            ("lhs", 1, "x^2", "Pow(Symbol('x'), Integer(2))", true),
            ("rhs", 1, "x \\cdot x", "Mul(Symbol('x'), Symbol('x'))", true),
            ("bad_h", 1, "", "", false),
        ],
    );
    seed_sympy_and_expr(
        &ledger,
        &sid,
        "call-2",
        true,
        &[
            ("lhs", 2, "x^3", "Pow(Symbol('x'), Integer(3))", true),
            ("rhs_alt", 1, "x \\cdot x^2", "Mul(Symbol('x'), Pow(Symbol('x'), Integer(2)))", true),
        ],
    );

    assert!(run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "unknown handle",
            "details": "d",
            "tool_call_refs": ["call-1"],
            "claims": [{ "form": "equation", "lhs": "nope@1", "rhs": "rhs@1", "relation": "eq" }]
        }),
    )
    .unwrap_err()
    .contains("unknown handle"));

    assert!(run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "non-portable handle",
            "details": "d",
            "tool_call_refs": ["call-1"],
            "claims": [{ "form": "equation", "lhs": "bad_h@1", "rhs": "rhs@1", "relation": "eq" }]
        }),
    )
    .unwrap_err()
    .contains("non-portable"));

    assert!(run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "superseded handle",
            "details": "d",
            "tool_call_refs": ["call-1"],
            "claims": [{ "form": "equation", "lhs": "lhs@1", "rhs": "rhs@1", "relation": "eq" }]
        }),
    )
    .unwrap_err()
    .contains("superseded"));

    // Valid e-2 with optional `plan` and `lhs@2 = rhs@1`.
    let res2 = run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "Evaluate integral",
            "details": "Computed via sympy_eval",
            "depends_on": ["e-1"],
            "tool_call_refs": ["call-2"],
            "claims": [{ "form": "equation", "lhs": "lhs@2", "rhs": "rhs@1", "relation": "eq" }],
            "plan": [{ "title": "Set up integral" }, { "title": "Evaluate" }]
        }),
    )
    .unwrap();
    assert_eq!(res2["entry_id"], "e-2");
    assert_eq!(res2["latex"], "x^3 = x \\cdot x");

    // Step e-3 uses handle `lhs@2` produced by `call-2` (owned by e-2) -> host derives `derived_depends_on: ["e-2"]`.
    let res3 = run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "derivation",
            "summary": "Use computed term",
            "details": "Algebra step",
            "depends_on": ["e-1"],
            "claims": [{ "form": "equation", "lhs": "lhs@2", "rhs": "rhs_alt@1", "relation": "eq" }]
        }),
    )
    .unwrap();
    assert_eq!(res3["entry_id"], "e-3");
    assert_eq!(res3["derived_depends_on"], json!(["e-2"]));

    // Failed closure blocks `final_answer`.
    let failed_check = SolveCheck {
        check_id: "chk-fail".into(),
        claim_sha256: res2["claim_sha256"].as_str().unwrap().into(),
        claim_index: 0,
        entry_id: Some("e-2".into()),
        math_sha256: "0".repeat(64),
        name: "identity_numeric".into(),
        registry_sha256: "0".repeat(64),
        params: json!({}),
        outcome: CheckOutcome::Fail,
        asked_by: CheckRequester::Host,
        receipt: ReceiptRef { call_id: "chk-call".into(), result_seq: 99 },
        evidence: json!({}),
        evidence_sha256: "0".repeat(64),
    };
    ledger
        .append(
            &sid,
            event_kinds::SOLVE_CHECK,
            1,
            &serde_json::to_value(failed_check).unwrap(),
        )
        .unwrap();
    let err_final = run_step_record(
        &env,
        &ledger,
        &sid,
        "solver",
        &json!({
            "entry_type": "final_answer",
            "summary": "Final",
            "details": "Depends on e-3 which depends on failed e-2",
            "depends_on": ["e-3"]
        }),
    )
    .unwrap_err();
    assert!(err_final.contains("Failed") && err_final.contains("e-2"));
}

#[test]
fn step_record_reference_lookup_enforces_f4_verify_and_lines_sent_r17() {
    let (_tmp, env, ledger, sid) = setup_env();
    let bytes = b"Line 1 intro\nThe coupling satisfies g^2 > 0 in d=4.\nLine 3 end\n";
    let ledger_mutex = Mutex::new(ledger);
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id: &sid,
            file_name: "ref.txt",
            origin: "workspace:ref.txt",
            raw_bytes: bytes,
            source_id_hint: Some("ref"),
            section_filter: None,
            title: Some("Ref"),
        },
    )
    .unwrap();
    let path = ingested.primary_path.unwrap();
    let quote = "The coupling satisfies g^2 > 0 in d=4.";
    let evs = ledger_mutex.lock().unwrap().read_session(&sid).unwrap();
    let verified =
        verify_quote(&env.sandbox(), &sid, &evs, &ingested.source_id, &path, 2, 2, quote).unwrap();
    let mut eq_payload = EvidenceQuotePayload::from_verified(
        &verified,
        quote,
        "user",
        EvidenceModeMeta::default(),
    )
    .unwrap();
    eq_payload.excerpt = clip_quote(&eq_payload.excerpt).to_string();
    let q_seq = ledger_mutex
        .lock()
        .unwrap()
        .append(
            &sid,
            event_kinds::EVIDENCE_QUOTE,
            1,
            &serde_json::to_value(&eq_payload).unwrap(),
        )
        .unwrap();
    let qref_json = json!({
        "source_id": verified.quote_ref.source_id,
        "path": verified.quote_ref.path,
        "start_line": verified.quote_ref.start_line,
        "end_line": verified.quote_ref.end_line,
        "file_sha256": verified.quote_ref.file_sha256,
        "quote_sha256": verified.quote_ref.quote_sha256,
        "evidence_seq": q_seq
    });

    // Without lines sent to caller `solver`, R17 refuses.
    let ledger_guard = ledger_mutex.lock().unwrap();
    let err = run_step_record(
        &env,
        &ledger_guard,
        &sid,
        "solver",
        &json!({
            "entry_type": "reference_lookup",
            "summary": "Cite coupling sign",
            "details": "From Ref line 2",
            "source_ref": qref_json
        }),
    )
    .unwrap_err();
    assert!(err.contains("R17"), "expected R17 refusal: {err}");

    // Supply line 2 via `context.supplied` to `solver`.
    ledger_guard
        .append(
            &sid,
            event_kinds::CONTEXT_SUPPLIED,
            1,
            &json!({
                "run_id": "run-1",
                "caller": "solver",
                "ranges": [{
                    "source_id": verified.quote_ref.source_id,
                    "path": verified.quote_ref.path,
                    "file_sha256": verified.quote_ref.file_sha256,
                    "start_line": 2,
                    "end_line": 2
                }]
            }),
        )
        .unwrap();

    let ok_res = run_step_record(
        &env,
        &ledger_guard,
        &sid,
        "solver",
        &json!({
            "entry_type": "reference_lookup",
            "summary": "Cite coupling sign",
            "details": "From Ref line 2",
            "source_ref": qref_json
        }),
    )
    .unwrap();
    assert_eq!(ok_res["entry_id"], "e-1");
    assert_eq!(ok_res["own_tier"], json!("cited"));
}
