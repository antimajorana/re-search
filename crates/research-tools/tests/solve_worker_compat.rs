//! Compatibility test for Worker Protocol 2 additive `sympy_eval` result keys (`solve.md` 8.3).
//!
//! Confirms that `proof_dag::read_proof_dag` (via `successful_sympy_result`) and
//! `solve_state::SolveState` continue to accept `sympy_eval` results carrying protocol 2
//! additive keys (`exprs_out`, `code_sha256`, `wall_ms`, `process`, `env`).

use ledger::Ledger;
use research_tools::read_proof_dag;
use research_tools::solve_model::ReceiptProcess;
use research_tools::solve_state::SolveState;
use serde_json::json;
use std::fs;

#[test]
fn proof_dag_and_solve_state_accept_protocol2_sympy_eval_extra_keys() {
    let data_dir = std::env::temp_dir().join(format!(
        "research-s6-worker-compat-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&data_dir);
    let ledger = Ledger::open(&data_dir).unwrap();
    let session = "s6-compat-session";

    // 1. Successful protocol 2 sympy_eval result with additive keys (inline JSON)
    let ok_payload = json!({
        "value": "x**2 + 1",
        "stdout": "",
        "stderr": "",
        "truncated": false,
        "seed_events": [],
        "latex": "x^{2} + 1",
        "exprs_out": { "e1": "Add(Pow(Symbol('x'), Integer(2)), Integer(1))" },
        "code_sha256": "0123456789abcdef",
        "wall_ms": 14,
        "process": "solver",
        "env": { "protocol": 2, "role": "solver", "python": "3.12.16", "sympy": "1.14.0" }
    });
    ledger
        .append(
            session,
            "tool.request",
            1,
            &json!({ "call_id": "c-ok", "tool": "sympy_eval", "args": { "inline": "{\"code\":\"save_expr('e1', x**2+1); x**2+1\"}" } }),
        )
        .unwrap();
    let ok_seq = ledger
        .append(
            session,
            "tool.result",
            1,
            &json!({ "call_id": "c-ok", "ok": true, "result": { "inline": ok_payload.to_string() } }),
        )
        .unwrap();

    // 2. Failed protocol 2 sympy_eval result (error non-null) must NOT count as successful
    let err_payload = json!({
        "value": null,
        "stdout": "",
        "stderr": "",
        "truncated": false,
        "seed_events": [],
        "error": { "type": "ZeroDivisionError", "message": "division by zero" },
        "exprs_out": {},
        "code_sha256": "fedcba9876543210",
        "wall_ms": 3,
        "process": "solver",
        "env": { "protocol": 2, "role": "solver" }
    });
    ledger
        .append(
            session,
            "tool.request",
            1,
            &json!({ "call_id": "c-err", "tool": "sympy_eval", "args": { "inline": "{\"code\":\"1/0\"}" } }),
        )
        .unwrap();
    ledger
        .append(
            session,
            "tool.result",
            1,
            &json!({ "call_id": "c-err", "ok": true, "result": { "inline": err_payload.to_string() } }),
        )
        .unwrap();

    // Proof DAG reader (successful_sympy_result) sees only c-ok, ignoring protocol 2 extra keys
    let dag = read_proof_dag(&ledger, session).unwrap();
    assert_eq!(dag.unverified_computations, vec![ok_seq]);

    // SolveState fold extracts protocol 2 receipt metadata cleanly and exempts c-err from pending_runs
    let events = ledger.read_session(session).unwrap();
    let state = SolveState::from_events(&events);
    assert_eq!(state.pending_runs, vec!["c-ok".to_string()]);
    let receipt = state.receipts.get("c-ok").expect("c-ok receipt recorded");
    assert_eq!(receipt.code_sha256.as_deref(), Some("0123456789abcdef"));
    assert_eq!(receipt.wall_ms, Some(14));
    assert_eq!(receipt.process, Some(ReceiptProcess::Solver));
    assert_eq!(receipt.env.as_ref().and_then(|e| e["protocol"].as_u64()), Some(2));

    let _ = fs::remove_dir_all(&data_dir);
}
