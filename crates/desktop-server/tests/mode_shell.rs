//! Integration tests for package F5 (mode shell):
//! - `/api/library/..` route registration and header-token auth enforcement;
//! - `session_mode` single rule, unbuilt-mode refusals (Explore/Solve start nothing);
//! - `UiApprover` prompting and resolving outside a running model turn (`00-overview.md` 3.1 row 13).

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use research_tools::library::{Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::paper_key::PaperKey;
use serde_json::{json, Value};
use tempfile::TempDir;

#[test]
fn library_routes_require_header_token_and_answer_when_authenticated() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);

    // Seed one synthetic paper in `<data_dir>/library/library.db`.
    let lib = Library::open(&data_dir).unwrap();
    let key = PaperKey::parse("arxiv:2405.01234").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds {
            arxiv: Some("2405.01234".into()),
            ..Default::default()
        },
        title: "Synthetic Library Auth Fixture".into(),
        authors: vec!["Ada Example".into()],
        year: Some(2024),
        abstract_text: "Self-written fixture paper for F5 route registration.".into(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1000,
        last_seen_ms: 2000,
    })
    .unwrap();

    // Unauthenticated GET and POST are refused with 401.
    let (st, _, _) = raw_http(port, "GET", "/api/library", &[], b"");
    assert_eq!(st, 401, "unauthenticated GET /api/library must be refused");
    let (st, _, _) = raw_http(port, "GET", "/api/library", &[("Cookie", cookie.as_str())], b"");
    assert_eq!(st, 401, "stream cookie alone must not open /api/library");
    let (st, _, _) = raw_http(port, "POST", "/api/library/rebuild", &[], b"");
    assert_eq!(st, 401, "unauthenticated POST /api/library/rebuild must be refused");

    // Authenticated GET /api/library answers 200 with the seeded paper.
    let auth = [("X-ReSearch-Token", tok.as_str())];
    let (st, _, body) = raw_http(port, "GET", "/api/library", &auth, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
    let val: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["count"], 1);
    assert_eq!(val["papers"][0]["paper"]["key"], key.as_str());

    // Authenticated GET /api/library/<key> resolves detail cleanly.
    let (st, _, detail_body) = raw_http(port, "GET", "/api/library/arxiv:2405.01234", &auth, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&detail_body));
    let detail: Value = serde_json::from_slice(&detail_body).unwrap();
    assert_eq!(detail["paper"]["title"], "Synthetic Library Auth Fixture");

    // Foreign Origin on mutation is still rejected even with a valid token.
    let foreign_body = serde_json::to_vec(&json!({ "status": "key" })).unwrap();
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/library/arxiv:2405.01234/mark",
        &[("X-ReSearch-Token", tok.as_str()), ("Origin", "http://evil.invalid")],
        &foreign_body,
    );
    assert_eq!(st, 403, "foreign Origin mutation on /api/library must be refused");
}

#[test]
fn unbuilt_modes_refuse_session_creation_and_create_nothing() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];

    for (mode, label) in [("explore", "Explore"), ("solve", "Solve")] {
        let body = serde_json::to_vec(&json!({ "id": format!("s-{mode}"), "mode": mode })).unwrap();
        let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &body);
        assert_eq!(st, 400, "POST /api/sessions mode={mode} must return 400");
        let err = serde_json::from_slice::<Value>(&raw).unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            err.contains(label) && err.contains("not built yet"),
            "unexpected error for {mode}: {err}"
        );
    }

    // Nothing was created in sessions list or ledger.
    let (st, _, list_raw) = raw_http(port, "GET", "/api/sessions", &auth, b"");
    assert_eq!(st, 200);
    let list = serde_json::from_slice::<Value>(&list_raw).unwrap();
    assert_eq!(list["sessions"].as_array().unwrap().len(), 0);

    // Alias "investigate" is accepted and reports wire mode "investigation".
    let inv_body = serde_json::to_vec(&json!({ "id": "s-inv-alias", "mode": "investigate" })).unwrap();
    let (st, _, inv_raw) = raw_http(port, "POST", "/api/sessions", &auth, &inv_body);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&inv_raw));
    let inv = serde_json::from_slice::<Value>(&inv_raw).unwrap();
    assert_eq!(inv["mode"], "investigation");
}

#[test]
fn unbuilt_mode_sessions_report_mode_and_refuse_turns_starting_nothing() {
    use ledger::Ledger;
    use research_tools::tool_profile::{
        MARKER_IDEA_STARTED, MARKER_INVESTIGATION_STARTED, MARKER_SOLVE_STARTED,
    };

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger
        .append_new_session("s-explore-1", &[(MARKER_IDEA_STARTED, 1, &json!({}))])
        .unwrap();
    ledger
        .append_new_session(
            "s-solve-1",
            &[
                (MARKER_INVESTIGATION_STARTED, 1, &json!({})),
                (MARKER_SOLVE_STARTED, 1, &json!({})),
            ],
        )
        .unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];

    // GET /api/sessions reports each session's latest-marker mode accurately.
    let (st, _, list_raw) = raw_http(port, "GET", "/api/sessions", &auth, b"");
    assert_eq!(st, 200);
    let sessions = serde_json::from_slice::<Value>(&list_raw).unwrap()["sessions"]
        .as_array()
        .unwrap()
        .clone();
    let mode_of = |id: &str| {
        sessions
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()["mode"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(mode_of("s-explore-1"), "explore");
    assert_eq!(mode_of("s-solve-1"), "solve");

    // GET /api/sessions/{id} agrees with GET /api/sessions.
    let (st, _, solve_raw) = raw_http(port, "GET", "/api/sessions/s-solve-1", &auth, b"");
    assert_eq!(st, 200);
    assert_eq!(serde_json::from_slice::<Value>(&solve_raw).unwrap()["mode"], "solve");

    // POST /api/sessions/{id}/messages refuses cleanly and appends zero events.
    for (sid, expected_len, label) in [
        ("s-explore-1", 1usize, "Explore"),
        ("s-solve-1", 2usize, "Solve"),
    ] {
        let msg = serde_json::to_vec(&json!({ "prompt": "Run please" })).unwrap();
        let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &auth, &msg);
        assert_eq!(st, 400, "turn on unbuilt {sid} must be refused");
        let err = serde_json::from_slice::<Value>(&raw).unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(err.contains(label) && err.contains("not built yet"), "{err}");
        assert_eq!(
            ledger.read_session(sid).unwrap().len(),
            expected_len,
            "refused turn on {sid} must not append ledger events"
        );
    }
}

#[test]
fn ui_approver_prompts_and_resolves_outside_a_running_model_turn() {
    use desktop_server::approver::UiApprover;
    use desktop_server::auth::AuthGuard;
    use desktop_server::chatgpt::ChatGptHub;
    use desktop_server::routes_session::{handle_pending_approvals, handle_post_approval};
    use desktop_server::session_lifecycle::{is_turn_running, list_session_grants};
    use desktop_server::sse::SseHub;
    use desktop_server::state::DesktopState;
    use ledger::Ledger;
    use research_tools::{register_research_tools, research_policy, ResearchEnv};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tools::{CallOutcome, Registry, Router};

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&ws_dir).unwrap();

    let cfg = test_cfg(&data_dir, Some(ws_dir.clone()), None, None);
    let auth = AuthGuard::new(18_421).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    let state = Arc::new(DesktopState::new(cfg, auth, sse, chatgpt));
    // Turn off auto-approve so UiApprover genuinely prompts for human approval.
    state.update_ui_settings(|ui| ui.auto_approve_tools = false).unwrap();

    let sid = "s-approver-outside-turn";
    assert!(!is_turn_running(&state, sid), "no model turn may be running");

    // Trigger a write-workspace tool call via Router + UiApprover on a worker thread
    // while NO model turn is registered in `active_turns` / `api_turns` / `cli_turns`.
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
            "host:outside-turn",
            "fs_write",
            &json!({ "path": "note.txt", "content": "approved outside turn\n" }),
        )
    });

    // Wait for the pending approval card to appear in state (with still zero running turns).
    let deadline = Instant::now() + Duration::from_secs(5);
    let pending = loop {
        let items = state.pending_for_session(sid);
        if !items.is_empty() {
            break items;
        }
        assert!(Instant::now() < deadline, "timed out waiting for out-of-turn approval card");
        std::thread::sleep(Duration::from_millis(15));
    };
    assert!(!is_turn_running(&state, sid));
    assert_eq!(pending.len(), 1);
    let call_id = pending[0].call_id.clone();
    let digest = pending[0].digest.clone();

    // Ensure GET /approvals sees it and POST /approvals/{call_id} resolves it.
    let (st, list_val) = handle_pending_approvals(&state, sid).unwrap();
    assert_eq!(st, 200);
    assert_eq!(list_val["pending"].as_array().unwrap().len(), 1);

    let reply_body = serde_json::to_vec(&json!({
        "digest": digest,
        "approved": true,
        "approve_for_session": true,
    }))
    .unwrap();
    let (st, reply_val) = handle_post_approval(&state, sid, &call_id, &reply_body).unwrap();
    assert_eq!(st, 200);
    assert_eq!(reply_val["approved"], true);
    assert_eq!(reply_val["approve_for_session"], true);

    let outcome = worker.join().unwrap().unwrap();
    assert!(matches!(outcome, CallOutcome::Ok(_)), "{outcome:?}");
    assert_eq!(std::fs::read_to_string(ws_dir.join("note.txt")).unwrap(), "approved outside turn\n");

    // Ledger recorded approval.answered and session grant was bound with zero model turns.
    let ledger = Ledger::open(&data_dir).unwrap();
    let events = ledger.read_session(sid).unwrap();
    assert!(events.iter().any(|e| e.kind == "approval.answered" && e.payload["approved"] == true));
    assert!(list_session_grants(&state, sid).contains(&"write-workspace"));
}

