//! Integration tests for `GET /api/sessions/{id}/export?format=notebook|ipynb|zip` (S19c).

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::solve_model::{
    compute_math_sha256, event_kinds, RoleModel, SolveRoles, SolveSetup, SolveTarget, TargetForm,
    TaskKind,
};
use serde_json::{json, Value};
use std::io::Cursor;
use tempfile::TempDir;

fn header_val<'a>(hdrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hdrs.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[test]
fn solve_export_route_serves_notebook_and_zip_behind_auth_guard() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // Create a Solve session and a Research session via API.
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &auth,
        &serde_json::to_vec(&json!({ "id": "s-route-solve", "mode": "solve" })).unwrap(),
    );
    assert_eq!(st, 200);
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &auth,
        &serde_json::to_vec(&json!({ "id": "s-route-research", "mode": "research" })).unwrap(),
    );
    assert_eq!(st, 200);

    // Seed Solve events directly into the ledger.
    let ledger = Ledger::open(&data_dir).unwrap();
    let setup = SolveSetup {
        revision: 1,
        statement: "Compute 2 + 2".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "Exact integer 4".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: vec![],
        conventions: "".into(),
        symbols: vec![],
        dimensions: Default::default(),
        assumptions: vec![],
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "openai".into(), model: "m-s".into() },
            verifier: RoleModel { provider: "anthropic".into(), model: "m-v".into() },
            compute: None,
        },
        budgets: Default::default(),
        review: Default::default(),
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: compute_math_sha256(&[], &Default::default(), &[], ""),
    };
    ledger
        .append_with_hash("s-route-solve", event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(setup).unwrap())
        .unwrap();
    ledger
        .append_with_hash("s-route-solve", event_kinds::PROOF_ENTRY, 1, &json!({
            "entry_id": "e-1",
            "entry_type": "final_answer",
            "summary": "Result is 4",
            "details": "",
            "origin": "solver"
        }))
        .unwrap();
    ledger
        .append_with_hash("s-route-solve", event_kinds::PROOF_ENTRY, 1, &json!({
            "entry_id": "e-discarded",
            "entry_type": "derivation",
            "summary": "Off-branch scratch step",
            "details": "",
            "origin": "solver"
        }))
        .unwrap();
    ledger
        .append_with_hash("s-route-solve", event_kinds::SOLVE_DONE, 1, &json!({
            "final_entry_id": "e-1",
            "own_tier": "unchecked",
            "effective_tier": "unchecked",
            "criteria": [],
            "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
            "status": "finished",
            "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
        }))
        .unwrap();

    // 1. Unauthenticated request blocked by AuthGuard (401).
    let (st_unauth, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-route-solve/export?format=notebook",
        &[],
        b"",
    );
    assert_eq!(st_unauth, 401);

    // 2. Same-origin stream cookie permitted on GET /export?format=notebook (default closure D18).
    let (st_cookie, hdrs, body) = raw_http(
        port,
        "GET",
        "/api/sessions/s-route-solve/export?format=notebook",
        &[("Cookie", cookie.as_str()), ("Sec-Fetch-Site", "same-origin")],
        b"",
    );
    assert_eq!(st_cookie, 200, "body={}", String::from_utf8_lossy(&body));
    assert_eq!(
        header_val(&hdrs, "content-type"),
        Some("application/x-ipynb+json; charset=utf-8")
    );
    let nb: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(nb["nbformat"], 4);
    assert_eq!(nb["nbformat_minor"], 5);
    let md = String::from_utf8_lossy(&body);
    assert!(md.contains("Result is 4"));
    assert!(!md.contains("Off-branch scratch step"));

    // 3. `format=ipynb&include_all_runs=true` includes off-branch e-discarded.
    let (st_all, _, all_body) = raw_http(
        port,
        "GET",
        "/api/sessions/s-route-solve/export?format=ipynb&include_all_runs=true",
        &auth,
        b"",
    );
    assert_eq!(st_all, 200);
    assert!(String::from_utf8_lossy(&all_body).contains("Off-branch scratch step"));

    // 4. `format=zip` serves Solve bundle with notebook.ipynb and `_re_search_runtime/checks/srepr_safe.py`.
    let (st_zip, zip_hdrs, zip_body) = raw_http(
        port,
        "GET",
        "/api/sessions/s-route-solve/export?format=zip",
        &auth,
        b"",
    );
    assert_eq!(st_zip, 200);
    assert_eq!(header_val(&zip_hdrs, "content-type"), Some("application/zip"));
    let archive = zip::ZipArchive::new(Cursor::new(zip_body)).unwrap();
    let names: Vec<String> = archive.file_names().map(|r| r.unwrap().into_owned()).collect();
    assert!(names.contains(&"notebook.ipynb".to_string()));
    assert!(names.contains(&"ledger.json".to_string()));
    assert!(names.contains(&"_re_search_runtime/checks/srepr_safe.py".to_string()));

    // 5. Research session rejects `format=notebook` cleanly with 400.
    let (st_bad, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-route-research/export?format=notebook",
        &auth,
        b"",
    );
    assert_eq!(st_bad, 400);

    srv.shutdown();
}
