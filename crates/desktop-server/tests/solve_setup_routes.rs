//! Integration tests for Solve S4 HTTP routes, view projection, R22 wording, D15 revisions,
//! and pre-turn `UiApprover` verification on Solve sessions.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
#[cfg(not(windows))]
use desktop_server::solve_view::LINUX_TOOLBOX_LINE;
use desktop_server::solve_view::WORKER_RESTART_NOTE;
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, event_kinds, CheckOutcome, CheckRequester, Claim,
    ClaimForm, ClaimRelation, HandleRef, ReceiptRef, SolveCheck, SolveExpr, StepEntryType,
    StepOrigin, MAX_CONVENTIONS_CHARS, MAX_STATEMENT_CHARS,
};
use research_tools::solve_setup::{PROVE_PROSE_NOTICE, SAME_MODEL_WARNING};
use research_tools::solve_status::wording;
use serde_json::{json, Value};
use std::time::Instant;
use tempfile::TempDir;

const WORKED_STMT: &str = "Compute the one-loop effective potential V_1(phi) of massless \
    lambda phi^4 theory, V_tree = lambda phi^4/4!, in d = 4 - 2 eps with MS-bar, \
    and show mu dV/dmu = 0 at order lambda^2.";

fn create_solve_session(port: u16, tok: &str, sid: &str) -> Value {
    let auth = [("X-ReSearch-Token", tok)];
    let body = serde_json::to_vec(&json!({ "id": sid, "mode": "solve" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &body);
    assert_eq!(st, 200, "create solve session failed: {}", String::from_utf8_lossy(&raw));
    serde_json::from_slice(&raw).unwrap()
}

fn sample_setup_body(conventions: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "statement": WORKED_STMT,
        "task_kind": "calculate",
        "target": { "description": "One-loop effective potential", "form": "series" },
        "done": [
            { "id": "final_result_tier", "kind": "final_tier_at_least", "params": { "tier": "verified" } },
            { "id": "closure_model_agreed", "kind": "closure_tier_at_least", "params": { "tier": "model_agreed" } },
            { "id": "no_open_flags", "kind": "no_open_flags", "params": {} }
        ],
        "conventions": conventions,
        "symbols": [
            { "name": "lambda", "assumptions": ["real", "positive"] },
            { "name": "phi", "assumptions": ["real"] },
            { "name": "mu", "assumptions": ["real", "positive"] }
        ],
        "dimensions": { "phi": { "mass": 1 }, "mu": { "mass": 1 } },
        "assumptions": [
            { "id": "a-1", "text": "Massless scalar field in Euclidean space" }
        ],
        "roles": {
            "solver": { "provider": "", "model": "" },
            "verifier": { "provider": "", "model": "" }
        },
        "ask_before_running": false
    }))
    .unwrap()
}

#[test]
fn solve_routes_enforce_auth_session_mode_and_marker_r2() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // Creating a Solve session with a prompt is refused (must confirm setup first).
    let with_prompt = serde_json::to_vec(&json!({ "id": "s-bad-prompt", "mode": "solve", "prompt": "go" })).unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions", &auth, &with_prompt);
    assert_eq!(st, 400);

    let created = create_solve_session(port, &tok, "s-solve-auth");
    assert_eq!(created["mode"], "solve");

    // Host-only `solve.started` was written as seq 1.
    let ledger = Ledger::open(&data_dir).unwrap();
    let evs = ledger.read_session("s-solve-auth").unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, event_kinds::SOLVE_STARTED);

    // AuthGuard enforcement: 401 without token or with cookie-only; 403 on foreign Origin.
    for (method, path) in [
        ("GET", "/api/sessions/s-solve-auth/solve/view"),
        ("POST", "/api/sessions/s-solve-auth/solve/prefill"),
        ("POST", "/api/sessions/s-solve-auth/solve/setup"),
    ] {
        let (st, _, _) = raw_http(port, method, path, &[], b"{}");
        assert_eq!(st, 401, "{method} {path} must reject unauthenticated");
        let (st, _, _) = raw_http(port, method, path, &[("Cookie", cookie.as_str())], b"{}");
        assert_eq!(st, 401, "{method} {path} must reject cookie-only");
    }
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-solve-auth/solve/setup",
        &[("X-ReSearch-Token", tok.as_str()), ("Origin", "http://evil.invalid")],
        &sample_setup_body("Euclidean"),
    );
    assert_eq!(st, 403, "foreign Origin mutation on /solve/setup must be refused");

    // Non-Solve session refuses /solve/* endpoints.
    let res_body = serde_json::to_vec(&json!({ "id": "s-research-1", "mode": "research" })).unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions", &auth, &res_body);
    assert_eq!(st, 200);
    let (st, _, _) = raw_http(port, "GET", "/api/sessions/s-research-1/solve/view", &auth, b"");
    assert_eq!(st, 400);
}

#[test]
fn solve_prefill_is_deterministic_fast_zero_model_and_has_linux_toolbox_fallback() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];
    create_solve_session(port, &tok, "s-solve-pre");

    let req_body = serde_json::to_vec(&json!({
        "statement": WORKED_STMT,
        "conventions": "Euclidean signature, hbar = c = 1",
        "solver": { "provider": "openai", "model": "gpt-5" },
        "verifier": { "provider": "openai", "model": "gpt-5" }
    }))
    .unwrap();

    let t0 = Instant::now();
    let (st1, _, raw1) = raw_http(port, "POST", "/api/sessions/s-solve-pre/solve/prefill", &auth, &req_body);
    let (st2, _, raw2) = raw_http(port, "POST", "/api/sessions/s-solve-pre/solve/prefill", &auth, &req_body);
    assert!(t0.elapsed().as_millis() < 1_000, "prefill HTTP round-trip must be < 1 s");
    assert_eq!(st1, 200, "{}", String::from_utf8_lossy(&raw1));
    assert_eq!(st2, 200);
    assert_eq!(raw1, raw2, "prefill must be deterministic");

    let v: Value = serde_json::from_slice(&raw1).unwrap();
    assert_eq!(v["prefill"]["task_kind"], "calculate");
    assert_eq!(v["prefill"]["same_model"], true);
    assert_eq!(v["prefill"]["same_model_warning"], SAME_MODEL_WARNING);
    assert_eq!(v["model_connected"], false);

    // Toolbox fallback on Linux never spawns Windows AppContainerSandbox.
    #[cfg(not(windows))]
    {
        assert_eq!(v["toolbox"]["available"], false);
        assert_eq!(v["toolbox"]["line"], LINUX_TOOLBOX_LINE);
    }
    #[cfg(windows)]
    {
        assert_eq!(v["toolbox"]["sandbox"], "appcontainer");
        assert_eq!(v["toolbox"]["available"], false);
    }

    // Approval grants include both `worker_session` and `session_capability` kinds.
    let grants = v["approval_grants"]["grants"].as_array().unwrap();
    assert!(grants.iter().any(|g| g["kind"] == "worker_session" && g["caller"] == "solver:solve"));
    assert!(grants.iter().any(|g| g["kind"] == "worker_session" && g["caller"] == "host:check"));
    assert!(grants.iter().any(|g| g["kind"] == "worker_session" && g["caller"] == "verifier:solve"));
    assert!(grants.iter().any(|g| g["kind"] == "session_capability" && g["effect"] == "network-read"));
    assert_eq!(v["approval_grants"]["worker_restart_note"], WORKER_RESTART_NOTE);
    assert_eq!(v["approval_grants"]["pre_turn_session_capability_supported"], true);
    assert_eq!(v["approval_grants"]["pre_turn_worker_session_supported"], false);
    assert_eq!(v["approval_grants"]["pre_turn_approver_supported"], false);

    // Oversized prefill statement is rejected with 400.
    let huge_req = serde_json::to_vec(&json!({ "statement": "x".repeat(MAX_STATEMENT_CHARS + 1) })).unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-solve-pre/solve/prefill", &auth, &huge_req);
    assert_eq!(st, 400);

    // Prove task prefill includes explicit D21 notice.
    let prove_req = serde_json::to_vec(&json!({
        "statement": "Prove that every finite integral domain is a field."
    }))
    .unwrap();
    let (st, _, prove_raw) = raw_http(port, "POST", "/api/sessions/s-solve-pre/solve/prefill", &auth, &prove_req);
    assert_eq!(st, 200);
    let pv: Value = serde_json::from_slice(&prove_raw).unwrap();
    assert_eq!(pv["prefill"]["task_kind"], "prove");
    assert_eq!(pv["prefill"]["prove_notice"], PROVE_PROSE_NOTICE);
}

#[test]
fn solve_setup_validation_given_steps_graph_view_and_d15_revision_staleness() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];
    create_solve_session(port, &tok, "s-solve-rev");

    // Validation rejections return 400.
    for bad_body in [
        json!({
            "statement": "x".repeat(MAX_STATEMENT_CHARS + 1),
            "task_kind": "calculate",
            "target": { "description": "d", "form": "closed_form" }
        }),
        json!({
            "statement": "Compute x + 1",
            "task_kind": "calculate",
            "target": { "description": "d", "form": "closed_form" },
            "conventions": "c".repeat(MAX_CONVENTIONS_CHARS + 1)
        }),
        json!({
            "statement": "Compute x + 1",
            "task_kind": "calculate",
            "target": { "description": "d", "form": "closed_form" },
            "symbols": [{ "name": "9invalid", "assumptions": ["real"] }]
        }),
        json!({
            "statement": "Compute x + 1",
            "task_kind": "calculate",
            "target": { "description": "d", "form": "closed_form" },
            "dimensions": { "x": { "mass": [1, 2] } }
        }),
        json!({
            "statement": "Compute x + 1",
            "task_kind": "calculate",
            "target": { "description": "d", "form": "closed_form" },
            "budgets": { "max_model_calls": 0, "max_wall_minutes": 30, "max_model_calls_per_leg": 10, "max_review_loops": 6, "max_gate_bounces": 3 }
        }),
    ] {
        let bytes = serde_json::to_vec(&bad_body).unwrap();
        let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-solve-rev/solve/setup", &auth, &bytes);
        assert_eq!(st, 400, "invalid setup must be rejected with 400: {bad_body}");
    }

    // Valid setup revision 1 succeeds, writes `solve.setup` + `Given` `proof.entry` steps.
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions/s-solve-rev/solve/setup",
        &auth,
        &sample_setup_body("Euclidean signature, hbar = c = 1"),
    );
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));
    let res: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(res["setup"]["revision"], 1);
    let rev1_math_sha = res["setup"]["math_sha256"].as_str().unwrap().to_string();
    let given_ids = res["given_entry_ids"].as_array().unwrap();
    assert_eq!(given_ids.len(), 3, "conventions + symbols + 1 assumption -> 3 Given steps");

    // GET /api/sessions/{id} includes "solve" payload matching GET /solve/view.
    let (st, _, sess_raw) = raw_http(port, "GET", "/api/sessions/s-solve-rev", &auth, b"");
    assert_eq!(st, 200);
    let sess: Value = serde_json::from_slice(&sess_raw).unwrap();
    assert_eq!(sess["mode"], "solve");
    assert_eq!(sess["solve"]["setup"]["revision"], 1);

    let (st, _, view_raw) = raw_http(port, "GET", "/api/sessions/s-solve-rev/solve/view", &auth, b"");
    assert_eq!(st, 200);
    let view: Value = serde_json::from_slice(&view_raw).unwrap();
    for key in [
        "setup", "run_state", "steps", "graph", "handles", "receipts", "checks", "artifacts",
        "done", "toolbox", "approval_grants",
    ] {
        assert!(view.get(key).is_some(), "solve/view missing required key `{key}`");
    }
    let steps = view["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    for step in steps {
        assert_eq!(step["badge"]["label"], wording::BADGE_GIVEN);
        assert_eq!(step["badge"]["grade"], "user_statement");
        assert_eq!(step["badge"]["provenance_tag"], wording::TAG_YOU);
    }
    let graph_nodes = view["graph"]["nodes"].as_array().unwrap();
    assert_eq!(graph_nodes.len(), 3);
    assert_eq!(graph_nodes[0]["grade"], "user");
    assert_eq!(graph_nodes[0]["border"], "plain");

    // Append a checked step under rev1_math_sha -> Verified, then revise setup -> Stale (D15).
    let ledger = Ledger::open(&data_dir).unwrap();
    let h_lhs = HandleRef::parse("lhs@1").unwrap();
    let h_rhs = HandleRef::parse("rhs@1").unwrap();
    let expr_lhs = SolveExpr {
        name: "lhs".into(), version: 1, srepr_blob: "b1".into(), srepr_sha256: "1".repeat(64),
        latex: "x+x".into(), str_preview: "x + x".into(), free_symbols: vec![],
        produced_by: ReceiptRef { call_id: "c1".into(), result_seq: 1 },
        owner_entry: Some("e-10".into()), portable: true,
    };
    let expr_rhs = SolveExpr {
        name: "rhs".into(), version: 1, srepr_blob: "b2".into(), srepr_sha256: "2".repeat(64),
        latex: "2x".into(), str_preview: "2*x".into(), free_symbols: vec![],
        produced_by: ReceiptRef { call_id: "c1".into(), result_seq: 1 },
        owner_entry: Some("e-10".into()), portable: true,
    };
    let claim = Claim {
        form: ClaimForm::Equation, lhs: h_lhs, rhs: h_rhs, relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    let claim_sha = compute_claim_sha256(std::slice::from_ref(&claim), |h| match h.name.as_str() {
        "lhs" => Some(expr_lhs.srepr_sha256.as_str()),
        "rhs" => Some(expr_rhs.srepr_sha256.as_str()),
        _ => None,
    })
    .unwrap();
    ledger.append_with_hash("s-solve-rev", event_kinds::SOLVE_EXPR, 1, &serde_json::to_value(&expr_lhs).unwrap()).unwrap();
    ledger.append_with_hash("s-solve-rev", event_kinds::SOLVE_EXPR, 1, &serde_json::to_value(&expr_rhs).unwrap()).unwrap();
    ledger.append_with_hash("s-solve-rev", event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-10",
        "entry_type": StepEntryType::Derivation.as_str(),
        "summary": "Combine like terms",
        "details": "x + x = 2x",
        "depends_on": [given_ids[0].as_str().unwrap()],
        "claims": [claim],
        "claim_sha256": claim_sha,
        "entry_sha256": compute_entry_sha256(Some(&claim_sha), "Combine like terms", "x + x = 2x"),
        "origin": StepOrigin::Solver,
    })).unwrap();
    let chk = SolveCheck {
        check_id: "chk-1".into(), claim_sha256: claim_sha, claim_index: 0,
        entry_id: Some("e-10".into()), math_sha256: rev1_math_sha.clone(),
        name: "identity_symbolic".into(), registry_sha256: "3".repeat(64),
        params: json!({}), outcome: CheckOutcome::Pass, asked_by: CheckRequester::Host,
        receipt: ReceiptRef { call_id: "chk-call".into(), result_seq: 9 },
        evidence: json!({ "exact": true }), evidence_sha256: "4".repeat(64),
    };
    ledger.append_with_hash("s-solve-rev", event_kinds::SOLVE_CHECK, 1, &serde_json::to_value(&chk).unwrap()).unwrap();

    let (st, _, v1_raw) = raw_http(port, "GET", "/api/sessions/s-solve-rev/solve/view", &auth, b"");
    assert_eq!(st, 200);
    let v1: Value = serde_json::from_slice(&v1_raw).unwrap();
    let e10_v1 = v1["steps"].as_array().unwrap().iter().find(|s| s["entry_id"] == "e-10").unwrap();
    assert_eq!(e10_v1["badge"]["label"], wording::BADGE_VERIFIED);
    assert_eq!(e10_v1["badge"]["stale"], false);

    // Submit revision 2 changing conventions -> math_sha256 changes -> e-10 becomes Stale (D15).
    let (st, _, rev2_raw) = raw_http(
        port,
        "POST",
        "/api/sessions/s-solve-rev/solve/setup",
        &auth,
        &sample_setup_body("Lorentzian signature (-,+,+,+)"),
    );
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&rev2_raw));
    let rev2: Value = serde_json::from_slice(&rev2_raw).unwrap();
    assert_eq!(rev2["setup"]["revision"], 2);
    assert_ne!(rev2["setup"]["math_sha256"].as_str().unwrap(), rev1_math_sha);

    let e10_v2 = rev2["view"]["steps"].as_array().unwrap().iter().find(|s| s["entry_id"] == "e-10").unwrap();
    assert_eq!(e10_v2["badge"]["label"], wording::BADGE_STALE);
    assert_eq!(e10_v2["badge"]["stale"], true);

    // Rule R22 wording audit on revision-2 view (zero verified steps now):
    // no step badge, graph node, or legend may claim "Verified", "Computed", or "proved".
    let graph_str = serde_json::to_string(&rev2["view"]["graph"]).unwrap();
    assert!(!graph_str.to_ascii_lowercase().contains("verified"), "stale graph leaked verified: {graph_str}");
    assert!(!graph_str.contains("Computed"), "retired Computed leaked: {graph_str}");
    assert!(!graph_str.contains("Checked numerically"), "pre-2026-11-26 spelling leaked: {graph_str}");
    assert!(!graph_str.to_ascii_lowercase().contains("proved"), "proved leaked on graph: {graph_str}");
}

#[test]
fn solve_ui_approver_prompts_and_records_session_grant_outside_turn() {
    use desktop_server::approver::UiApprover;
    use desktop_server::auth::AuthGuard;
    use desktop_server::chatgpt::ChatGptHub;
    use desktop_server::routes_session::{handle_pending_approvals, handle_post_approval};
    use desktop_server::session_lifecycle::{is_turn_running, list_session_grants};
    use desktop_server::solve::session_solve_view;
    use desktop_server::sse::SseHub;
    use desktop_server::state::DesktopState;
    use research_tools::{register_research_tools, research_policy, ResearchEnv};
    use std::sync::Arc;
    use std::time::Duration;
    use tools::{CallOutcome, Registry, Router};

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&ws_dir).unwrap();

    let cfg = test_cfg(&data_dir, Some(ws_dir.clone()), None, None);
    let auth = AuthGuard::new(19_101).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    let state = Arc::new(DesktopState::new(cfg, auth, sse, chatgpt));
    state.update_ui_settings(|ui| ui.auto_approve_tools = false).unwrap();

    let sid = "s-solve-preturn-approve";
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append_new_session(sid, &[(event_kinds::SOLVE_STARTED, 1, &json!({}))]).unwrap();
    assert!(!is_turn_running(&state, sid), "no model turn may be running");

    let env = Arc::new(ResearchEnv::with_workspace(data_dir.clone(), &ws_dir).unwrap());
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env);
    let approver = Arc::new(
        UiApprover::new(Arc::clone(&state), "desktop host".into(), false)
            .with_session_network_grant(true),
    );
    let router = Router::new(reg, research_policy(), Ledger::open(&data_dir).unwrap(), sid, approver);

    let worker = std::thread::spawn(move || {
        router.try_call(
            "host:solve-setup",
            "fs_write",
            &json!({ "path": "preturn.txt", "content": "ok\n" }),
        )
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let pending = loop {
        let items = state.pending_for_session(sid);
        if !items.is_empty() {
            break items;
        }
        assert!(Instant::now() < deadline, "timed out waiting for pre-turn approval");
        std::thread::sleep(Duration::from_millis(15));
    };
    assert!(!is_turn_running(&state, sid));
    // While approval is pending, solve_view reports `Waiting for you` (`waiting`).
    let events_while_pending = ledger.read_session(sid).unwrap();
    let pending_view = session_solve_view(&state, sid, &events_while_pending, false);
    assert_eq!(pending_view["run_state"]["state"], "waiting");
    assert_eq!(pending_view["run_state"]["label"], "Waiting for you");

    let (st, list) = handle_pending_approvals(&state, sid).unwrap();
    assert_eq!(st, 200);
    assert_eq!(list["pending"].as_array().unwrap().len(), 1);

    let reply = serde_json::to_vec(&json!({
        "digest": pending[0].digest,
        "approved": true,
        "approve_for_session": true,
    }))
    .unwrap();
    let (st, _) = handle_post_approval(&state, sid, &pending[0].call_id, &reply).unwrap();
    assert_eq!(st, 200);
    let outcome = worker.join().unwrap().unwrap();
    assert!(matches!(outcome, CallOutcome::Ok(_)));
    assert!(list_session_grants(&state, sid).contains(&"write-workspace"));

    let events = ledger.read_session(sid).unwrap();
    let view = session_solve_view(&state, sid, &events, false);
    assert_eq!(view["approval_grants"]["pre_turn_session_capability_supported"], true);
    assert_eq!(view["approval_grants"]["pre_turn_worker_session_supported"], false);
    assert_eq!(view["approval_grants"]["pre_turn_approver_supported"], false);
    assert!(view["approval_grants"]["active_session_grants"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g == "write-workspace"));
}

#[test]
fn solve_view_handles_interrupted_done_open_items_and_stale_graph_legend() {
    use desktop_server::solve_view::build_solve_view;
    use research_tools::solve_model::{
        compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, Claim,
        ClaimForm, ClaimRelation, HandleRef,
    };

    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-solve-audit-view";
    let setup_hash = compute_math_sha256(&[], &Default::default(), &[], "real x");
    let h1 = HandleRef::new("expr_a", 1).unwrap();
    let h2 = HandleRef::new("expr_b", 1).unwrap();
    let claim1 = Claim { form: ClaimForm::Equation, lhs: h1.clone(), rhs: h1.clone(), relation: ClaimRelation::Eq, order: None, domain: None, how: None };
    let claim2 = Claim { form: ClaimForm::Equation, lhs: h2.clone(), rhs: h2.clone(), relation: ClaimRelation::Eq, order: None, domain: None, how: None };
    let c1_sha = compute_claim_sha256(std::slice::from_ref(&claim1), |_| Some("sha-a")).unwrap();
    let c2_sha = compute_claim_sha256(std::slice::from_ref(&claim2), |_| Some("sha-b")).unwrap();
    let e1_sha = compute_entry_sha256(Some(&c1_sha), "Step 1", "Compute x^2");
    let e2_sha = compute_entry_sha256(Some(&c2_sha), "Step 2", "Compute x^3");

    let mk_expr = |name: &str, sha: &str| {
        json!({
            "name": name, "version": 1, "srepr_blob": "b1", "srepr_sha256": sha,
            "latex": "x", "str_preview": "x", "free_symbols": [],
            "produced_by": { "call_id": "call-1", "result_seq": 4 }, "portable": true
        })
    };
    let mk_chk = |cid: &str, eid: &str, csha: &str, msha: &str| {
        json!({
            "check_id": cid, "claim_sha256": csha, "claim_index": 0, "entry_id": eid,
            "math_sha256": msha, "name": "equal_zero", "registry_sha256": "reg1",
            "params": {}, "outcome": "pass", "asked_by": "host",
            "receipt": { "call_id": "call-1", "result_seq": 4 },
            "evidence": { "symbolic_zero": true }, "evidence_sha256": "ev1"
        })
    };

    ledger
        .append_new_session(
            sid,
            &[
                (event_kinds::SOLVE_STARTED, 1, &json!({})),
                (
                    event_kinds::SOLVE_SETUP,
                    1,
                    &json!({
                        "revision": 1, "statement": "Compute $x^2$", "task_kind": "calculate",
                        "target": { "description": "expr", "form": "closed_form" },
                        "done": [{ "id": "final_result_tier", "kind": "final_tier_at_least", "params": { "tier": "verified" } }],
                        "conventions": "real x", "symbols": [], "dimensions": {}, "assumptions": [],
                        "roles": { "solver": { "provider": "", "model": "" }, "verifier": { "provider": "", "model": "" } },
                        "budgets": { "max_model_calls": 60, "max_wall_minutes": 30, "max_model_calls_per_leg": 48, "max_review_loops": 6, "max_gate_bounces": 3 },
                        "review": "prose_only", "tool_profile": "solve/1", "math_sha256": setup_hash
                    }),
                ),
                (event_kinds::SESSION_RESUME, 1, &json!({})),
                (event_kinds::TOOL_RESULT, 1, &json!({ "call_id": "call-1", "tool": "check_run", "ok": true, "process": "check" })),
                (event_kinds::SOLVE_EXPR, 1, &mk_expr("expr_a", "sha-a")),
                (event_kinds::SOLVE_EXPR, 1, &mk_expr("expr_b", "sha-b")),
                (
                    event_kinds::PROOF_ENTRY,
                    1,
                    &json!({
                        "entry_id": "e-1", "entry_type": "step", "summary": "Step 1", "details": "Compute x^2",
                        "depends_on": [], "claims": [claim1], "claim_sha256": c1_sha, "entry_sha256": e1_sha
                    }),
                ),
                (event_kinds::SOLVE_CHECK, 1, &mk_chk("c-1", "e-1", &c1_sha, &setup_hash)),
                (
                    event_kinds::PROOF_ENTRY,
                    1,
                    &json!({
                        "entry_id": "e-2", "entry_type": "step", "summary": "Step 2", "details": "Compute x^3",
                        "depends_on": [], "claims": [claim2], "claim_sha256": c2_sha, "entry_sha256": e2_sha
                    }),
                ),
                (event_kinds::SOLVE_CHECK, 1, &mk_chk("c-2", "e-2", &c2_sha, "outdated-setup-hash")),
                (event_kinds::PROOF_SUPERSEDE, 1, &json!({ "entry_id": "e-1", "replaced_by": "e-2", "reason": "revised" })),
            ],
        )
        .unwrap();

    let events = ledger.read_session(sid).unwrap();
    let view = build_solve_view(&events, false, json!({}), json!({}), false);
    assert_eq!(view["run_state"]["state"], "interrupted");

    // Superseded e-1 is excluded from Verified legend, and stale e-2 emits "Needs re-check" without Verified legend.
    let legend = view["graph"]["legend"].as_array().unwrap();
    assert!(!legend.iter().any(|item| item["label"] == "Verified"), "unexpected Verified legend: {legend:?}");
    let nodes = view["graph"]["nodes"].as_array().unwrap();
    let e2 = nodes.iter().find(|n| n["id"] == "e-2").unwrap();
    assert_eq!(e2["stale"], true);
    assert_eq!(e2["badges"][0]["text"], "Needs re-check");

    // Append solve.done (finished_open_items) and verify run_state surfaces open_items from state.done.
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_DONE,
            1,
            &json!({
                "final_entry_id": "e-2", "own_tier": "stale", "effective_tier": "stale",
                "criteria": [{ "id": "final_result_tier", "met": false, "evidence": {} }],
                "rests_on": { "given": [], "cited": [], "assumed_by_model": ["e-2"], "weakest": [] },
                "status": "finished_open_items",
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
            }),
        )
        .unwrap();
    let done_events = ledger.read_session(sid).unwrap();
    let done_view = build_solve_view(&done_events, false, json!({}), json!({}), false);
    assert_eq!(done_view["run_state"]["state"], "done_open_items");
    assert_eq!(done_view["run_state"]["open_items"].as_array().unwrap().len(), 2);
}
