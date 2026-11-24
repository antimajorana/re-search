//! Exercise the validation plan barrier over the real HTTP and ledger paths.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use serde_json::{json, Value};
use tempfile::TempDir;

const SESSION: &str = "s-validation";
const REFERENCE: &str = "123.456789012345678901234567890";
const KNOWN_PASS: &str = "123.456789012345678901234567891";
const PLANTED_FAIL: &str = "123.456789012345678901234567892";

fn req(server: &RunningDesktopServer, method: &str, path: &str, body: Value) -> (u16, Value) {
    let headers = [("X-ReSearch-Token", server.info.token.as_str())];
    let (status, _, bytes) = raw_http(
        server.addr.port(),
        method,
        path,
        &headers,
        &serde_json::to_vec(&body).unwrap(),
    );
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn prepare(server: &RunningDesktopServer) -> String {
    let (status, project) = req(
        server,
        "POST",
        "/api/projects",
        json!({ "title": "Spectral validation" }),
    );
    assert_eq!(status, 201, "{project}");
    let pid = project["id"].as_str().unwrap();
    assert_eq!(
        req(
            server,
            "POST",
            "/api/sessions",
            json!({ "id": SESSION, "title": "Spectrum" })
        )
        .0,
        200
    );
    let project_path = format!("/api/projects/{pid}");
    assert_eq!(
        req(
            server,
            "PUT",
            &format!("{project_path}/sessions/{SESSION}"),
            json!({})
        )
        .0,
        200
    );
    format!("{project_path}/validation-plans")
}

fn input() -> Value {
    json!({
        "session_id": SESSION,
        "claim": "Refinement converges to the measured eigenvalue",
        "candidate_method": "Run the Galerkin approximation on three basis sizes",
        "reference_method": "Compute the independent closed-form root with interval arithmetic",
        "axis": "basis",
        "atol": "1e-27",
        "rtol": "0",
        "reference": REFERENCE,
        "known_pass": KNOWN_PASS,
        "planted_fail": PLANTED_FAIL,
    })
}

fn tool_request(ledger: &Ledger, id: &str) {
    ledger
        .append(
            SESSION,
            "tool.request",
            1,
            &json!({
                "call_id": id, "tool": "sympy_eval", "args": {"inline": "{}"},
            }),
        )
        .unwrap();
}

fn record_candidate(
    ledger: &Ledger,
    call: &str,
    entry: &str,
    value: &str,
    axis: &str,
    stable: bool,
    atol: &str,
) -> u64 {
    let report = json!({
        "value": "typed report",
        "validation": {
            "schema_version": 1, "kind": "numerical_convergence", "axis": axis,
            "status": if stable { "within_tolerance" } else { "not_stable" },
            "stable": stable, "tail": 1, "atol": atol, "rtol": "0",
            "samples": [
                {"level":"8", "value": value, "error":null},
                {"level":"16", "value": value, "passes":stable, "error":null},
            ],
        }
    });
    let result_seq = ledger
        .append(
            SESSION,
            "tool.result",
            1,
            &json!({
                "call_id": call, "ok": true, "result": { "inline": report.to_string() },
            }),
        )
        .unwrap();
    ledger.append(SESSION, "proof.entry", 1, &json!({
        "entry_id": entry, "entry_type": "numerical_computation", "summary":"Recorded convergence",
        "tool_call_ref": format!("{call}@seq:{result_seq}"),
    })).unwrap();
    result_seq
}

fn hide_private(body: &Value) {
    let text = body.to_string();
    for value in [REFERENCE, KNOWN_PASS, PLANTED_FAIL] {
        assert!(!text.contains(value), "{text}");
    }
    assert!(!text.contains("\"salt\""), "{text}");
}

#[test]
fn commits_before_requests_and_evaluates_one_exact_linked_candidate() {
    let temp = TempDir::new().unwrap();
    let cfg = test_cfg(temp.path(), None, None, None);
    let server = start_desktop_server(cfg.clone()).unwrap();
    let path = prepare(&server);
    let ledger = Ledger::open(temp.path()).unwrap();
    tool_request(&ledger, "old-request");
    let (status, public) = req(&server, "POST", &path, input());
    assert_eq!(status, 201, "{public}");
    assert_eq!(public["status"], "open");
    assert!(public["evaluation"].is_null());
    hide_private(&public);
    let plan = format!("{path}/{}", public["id"].as_str().unwrap());
    let candidates_url = format!("{plan}/candidates");
    let evaluate = format!("{plan}/evaluate");
    let commit_seq = public["commit_seq"].as_u64().unwrap();
    let commit_hash = public["commit_hash"].as_str().unwrap();

    let old_result = record_candidate(
        &ledger,
        "old-request",
        "e-1",
        "123.4",
        "basis",
        true,
        "1e-27",
    );
    tool_request(&ledger, "wrong-axis");
    record_candidate(
        &ledger,
        "wrong-axis",
        "e-2",
        "123.4",
        "precision",
        true,
        "1e-27",
    );
    tool_request(&ledger, "unstable");
    record_candidate(&ledger, "unstable", "e-3", "123.4", "basis", false, "1e-27");
    tool_request(&ledger, "retired");
    record_candidate(&ledger, "retired", "e-4", "123.4", "basis", true, "1e-27");
    ledger
        .append(SESSION, "proof.supersede", 1, &json!({ "entry_id": "e-4" }))
        .unwrap();
    tool_request(&ledger, "candidate");
    let result_seq = record_candidate(
        &ledger,
        "candidate",
        "e-5",
        "123.456789012345678901234567889",
        "basis",
        true,
        "1.0e-27",
    );
    tool_request(&ledger, "loose-tolerance");
    record_candidate(
        &ledger,
        "loose-tolerance",
        "e-6",
        "123.4",
        "basis",
        true,
        "1e-26",
    );
    tool_request(&ledger, "candidate");
    record_candidate(&ledger, "candidate", "e-7", "123.4", "basis", true, "1e-26");
    ledger
        .append(
            SESSION,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-8",
                "entry_type": "numerical_computation",
                "tool_call_ref": "candidate",
            }),
        )
        .unwrap();
    let (status, options) = req(&server, "GET", &candidates_url, Value::Null);
    assert_eq!(status, 200, "{options}");
    let candidates = options["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{options}");
    assert_eq!(candidates[0]["entry_id"], "e-5");
    assert!(candidates[0]["request_seq"].as_u64().unwrap() > commit_seq);
    assert_eq!(candidates[0]["result_seq"], result_seq);
    assert_eq!(
        req(
            &server,
            "POST",
            &evaluate,
            json!({ "entry_id": "e-1", "result_seq": old_result })
        )
        .0,
        400
    );
    assert_eq!(
        req(
            &server,
            "POST",
            &evaluate,
            json!({ "entry_id": "e-5", "result_seq": old_result })
        )
        .0,
        400
    );

    let chosen = json!({ "entry_id": "e-5", "result_seq": result_seq });
    let (status, evaluated) = req(&server, "POST", &evaluate, chosen.clone());
    assert_eq!(status, 200, "{evaluated}");
    assert_eq!(evaluated["status"], "accepted");
    assert_eq!(evaluated["evaluation"]["known_pass_passed"], true);
    assert_eq!(evaluated["evaluation"]["planted_fail_rejected"], true);
    hide_private(&evaluated);
    assert_eq!(req(&server, "POST", &evaluate, chosen).1, evaluated);
    assert_eq!(
        req(
            &server,
            "POST",
            &evaluate,
            json!({ "entry_id": "e-1", "result_seq": old_result })
        )
        .0,
        409
    );
    assert_eq!(raw_http(server.addr.port(), "GET", &path, &[], b"").0, 401);
    server.shutdown();

    let server = start_desktop_server(cfg).unwrap();
    let (status, listed) = req(&server, "GET", &path, Value::Null);
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["plans"][0], evaluated);
    let (_, public_session) = req(
        &server,
        "GET",
        &format!("/api/sessions/{SESSION}"),
        Value::Null,
    );
    hide_private(&public_session);
    let (_, events) = ledger.read_verified_session(SESSION).unwrap();
    let committed = events.iter().find(|event| event.seq == commit_seq).unwrap();
    assert_eq!(
        (committed.kind.as_str(), committed.event_hash.as_str()),
        ("validation.plan.committed", commit_hash)
    );
    assert_eq!(events.last().unwrap().kind, "validation.plan.evaluated");
    assert_eq!(
        evaluated["evaluation"]["evaluation_hash"],
        events.last().unwrap().event_hash
    );
    let selected = events.iter().find(|event| event.seq == result_seq).unwrap();
    assert_eq!(evaluated["evaluation"]["result_hash"], selected.event_hash);
    server.shutdown();
}

#[test]
fn control_points_must_straddle_tolerance_and_failed_candidate_is_terminal() {
    let temp = TempDir::new().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let path = prepare(&server);
    let mut invalid = input();
    invalid["planted_fail"] = json!(REFERENCE);
    assert_eq!(req(&server, "POST", &path, invalid).0, 400);
    assert_eq!(
        req(&server, "GET", &path, Value::Null).1["plans"],
        json!([])
    );

    let (status, plan) = req(&server, "POST", &path, input());
    assert_eq!(status, 201, "{plan}");
    let ledger = Ledger::open(temp.path()).unwrap();
    tool_request(&ledger, "fails-holdout");
    let result_seq = record_candidate(
        &ledger,
        "fails-holdout",
        "e-1",
        "123.456789012345678901234567888",
        "basis",
        true,
        "1e-27",
    );
    let evaluate = format!("{path}/{}/evaluate", plan["id"].as_str().unwrap());
    let (status, failed) = req(
        &server,
        "POST",
        &evaluate,
        json!({ "entry_id": "e-1", "result_seq": result_seq }),
    );
    assert_eq!(status, 200, "{failed}");
    assert_eq!(failed["status"], "rejected");
    assert_eq!(failed["evaluation"]["known_pass_passed"], true);
    assert_eq!(failed["evaluation"]["planted_fail_rejected"], true);
    hide_private(&failed);
    server.shutdown();
}
