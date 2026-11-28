//! Integration tests for F16b `/api/lessons/notes..` & `/api/lessons/supplied/..` routes,
//! opt-in `(model)` draft summary, AuthGuard protection, and byte-identical `build_resume_brief`
//! across live `lessons.db` mutations/deletion.

mod common;

use agent::resume_brief::build_resume_brief;
use agent::transport::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, StreamObserver,
    Transport, TransportError, TransportInfo,
};
use common::test_cfg;
use desktop_server::auth::{ApiCredentials, AuthGuard};
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::routes_lessons::handle_lessons;
use desktop_server::routes_lessons_ui::draft_summary_with_transport;
use desktop_server::routes_session::handle_create_session;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use research_tools::lessons_capture::{LessonScope, LessonSourceKind, SaveLessonRequest, UserLessonsStore};
use research_tools::lessons_prompt::{supply_lessons_to_ledger, LessonMatchContext};
use research_tools::paper_key::{Mode, NodeRef};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

fn make_state(tmp: &TempDir) -> Arc<DesktopState> {
    let cfg = test_cfg(tmp.path(), None, None, None);
    let auth = AuthGuard::new(18_429).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    Arc::new(DesktopState::new(cfg, auth, sse, chatgpt))
}

fn qmap(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

struct MockDraftTransport {
    calls: AtomicUsize,
    reply: String,
}

impl Transport for MockDraftTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "mock",
            endpoint: "http://127.0.0.1/mock".to_string(),
            api_key_env: "MOCK_KEY".to_string(),
        }
    }

    fn prepare_request(&self, _req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest { body: b"{}".to_vec() })
    }

    fn send_prepared(&self, _req: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelResponse {
            raw_body: b"{}".to_vec(),
            id: Some("mock-draft-1".into()),
            model: "mock-model".into(),
            stop_reason: StopReason::EndTurn,
            content: vec![ContentBlock::Text { text: self.reply.clone() }],
            usage: Default::default(),
        })
    }

    fn send_prepared_streaming(
        &self,
        req: &PreparedRequest,
        _obs: &mut dyn StreamObserver,
    ) -> Result<ModelResponse, TransportError> {
        self.send_prepared(req)
    }
}

#[test]
fn notes_crud_300_char_cap_opt_in_backup_and_tamper_rejection() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    // 1. Prefill uses user's own words with 0 model calls and defaults to (mode, field).
    let prefill_body = serde_json::to_vec(&json!({
        "from": { "session": "s-1", "mode": "solve", "id": "e-4" },
        "user_words": "Account for the 1/2 symmetry factor on sunset diagrams.",
        "field": "QFT",
        "source_kind": "step_correction"
    }))
    .unwrap();
    let (code, prefill) =
        handle_lessons(&state, "POST", &["notes", "prefill"], &qmap(&[]), &prefill_body).unwrap();
    assert_eq!(code, 200);
    assert_eq!(
        prefill["draft"]["prefilled_text"],
        json!("Account for the 1/2 symmetry factor on sunset diagrams.")
    );
    assert_eq!(prefill["draft"]["default_scope"]["kind"], json!("mode_field"));
    assert_eq!(prefill["draft"]["model_summary"], json!(false));

    // 2. Reject > 300 Unicode chars on save with HTTP 400.
    let too_long = "κ".repeat(301);
    let bad_body = serde_json::to_vec(&json!({
        "text": too_long,
        "scope": { "kind": "global" },
        "from": { "session": "s-1", "mode": "solve", "id": "e-4" }
    }))
    .unwrap();
    let err = handle_lessons(&state, "POST", &["notes"], &qmap(&[]), &bad_body).unwrap_err();
    assert_eq!(err.0, 400);

    // 3. Explicit save creates row; edit and delete work; backup requires ?confirm=true.
    let ok_body = serde_json::to_vec(&json!({
        "text": "Keep mostly-minus metric (+,-,-,-).",
        "scope": { "kind": "mode_field", "mode": "solve", "field": "qft" },
        "from": { "session": "s-1", "mode": "solve", "id": "e-4" }
    }))
    .unwrap();
    let (code, created) =
        handle_lessons(&state, "POST", &["notes"], &qmap(&[]), &ok_body).unwrap();
    assert_eq!(code, 200);
    let lesson_id = created["lesson"]["id"].as_str().unwrap().to_string();

    let edit_body = serde_json::to_vec(&json!({
        "text": "Keep mostly-minus metric (+,-,-,-) across loops.",
        "scope": { "kind": "global" }
    }))
    .unwrap();
    let (code, edited) =
        handle_lessons(&state, "PUT", &["notes", &lesson_id], &qmap(&[]), &edit_body).unwrap();
    assert_eq!(code, 200);
    assert_eq!(
        edited["lesson"]["text"],
        json!("Keep mostly-minus metric (+,-,-,-) across loops.")
    );

    let err_no_confirm =
        handle_lessons(&state, "GET", &["notes", "backup"], &qmap(&[]), b"").unwrap_err();
    assert_eq!(err_no_confirm.0, 400);

    let (code, backup_res) = handle_lessons(
        &state,
        "GET",
        &["notes", "backup"],
        &qmap(&[("confirm", "true")]),
        b"",
    )
    .unwrap();
    assert_eq!(code, 200);

    // Tampered backup rejected cleanly.
    let mut tampered = backup_res["backup"].clone();
    tampered["lessons"][0]["text"] = json!("Tampered note");
    let tampered_bytes = serde_json::to_vec(&json!({ "backup": tampered, "replace": true })).unwrap();
    assert_eq!(
        handle_lessons(&state, "POST", &["notes", "restore"], &qmap(&[]), &tampered_bytes)
            .unwrap_err()
            .0,
        400
    );

    // Delete live lesson, then restore valid checksummed backup.
    let (code, _) =
        handle_lessons(&state, "DELETE", &["notes", &lesson_id], &qmap(&[]), b"").unwrap();
    assert_eq!(code, 200);

    let valid_restore =
        serde_json::to_vec(&json!({ "backup": backup_res["backup"], "replace": true })).unwrap();
    let (code, restored) =
        handle_lessons(&state, "POST", &["notes", "restore"], &qmap(&[]), &valid_restore).unwrap();
    assert_eq!(code, 200);
    assert_eq!(restored["restored"], json!(1));
}

#[test]
fn explicit_model_draft_summary_and_auth_guard_coverage() {
    let tr = MockDraftTransport {
        calls: AtomicUsize::new(0),
        reply: "Always include 1/2 symmetry factor on sunset loops.".into(),
    };
    let drafted = draft_summary_with_transport(
        &tr,
        "mock-model",
        "Check symmetry factor 1/2.",
        Mode::Solve,
        Some("qft"),
        Some("Compute sunset"),
    )
    .unwrap();
    assert_eq!(tr.calls.load(Ordering::SeqCst), 1);
    assert!(drafted.ends_with(" (model)"));
    assert!(drafted.chars().count() <= 300);

    let auth = AuthGuard::new(18_429).unwrap();
    for path in ["/api/lessons/notes", "/api/lessons/supplied/s-1"] {
        let unauth = ApiCredentials {
            method: "GET",
            path,
            cookie: None,
            token_hdr: None,
            auth_hdr: None,
            origin: None,
            sec_fetch_site: None,
        };
        assert_eq!(auth.authorize_api(&unauth).unwrap_err().0, 401);
        let authed = ApiCredentials {
            token_hdr: Some(auth.token()),
            ..unauth
        };
        assert!(auth.authorize_api(&authed).is_ok());
    }
}

#[test]
fn resume_brief_is_byte_identical_after_live_lessons_db_mutation_and_deletion() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    let (_, created) = handle_create_session(&state, b"{}").unwrap();
    let sid = created["id"].as_str().unwrap().to_string();

    let store = UserLessonsStore::open(&state.cfg.data_dir).unwrap();
    let saved = store
        .save_lesson(SaveLessonRequest {
            text: "Use mostly-minus metric (+,-,-,-).".into(),
            scope: LessonScope::global(),
            from: NodeRef {
                session: "s-orig".into(),
                mode: Mode::Solve,
                id: "e-1".into(),
                paper: None,
                node_fp: None,
            },
            source_kind: LessonSourceKind::StepCorrection,
            model_summary: false,
            created_ms: Some(1_000),
        })
        .unwrap();

    let ledger = state.open_ledger().unwrap();
    supply_lessons_to_ledger(
        &store,
        &ledger,
        &sid,
        "run-1",
        &LessonMatchContext::for_mode(Mode::Solve),
    )
    .unwrap()
    .expect("supplied");

    let events_before = ledger.read_session(&sid).unwrap();
    let upto_seq = events_before.last().unwrap().seq;
    let brief_before = build_resume_brief(&events_before, upto_seq, 8000);
    let (code_before, supplied_before) =
        handle_lessons(&state, "GET", &["supplied", &sid], &qmap(&[]), b"").unwrap();
    assert_eq!(code_before, 200);
    assert_eq!(supplied_before["disclosure"], json!("used 1 of your notes"));

    // Mutate and delete the live lesson, then remove the SQLite DB file entirely.
    store
        .update_lesson(&saved.id, "Completely rewritten text", LessonScope::global(), None)
        .unwrap();
    store.delete_lesson(&saved.id).unwrap();
    drop(store);
    let _ = std::fs::remove_file(state.cfg.data_dir.join("library").join(ledger::LESSONS_DB_NAME));

    let events_after = ledger.read_session(&sid).unwrap();
    let brief_after = build_resume_brief(&events_after, upto_seq, 8000);
    assert_eq!(brief_before, brief_after);

    let (code_after, supplied_after) =
        handle_lessons(&state, "GET", &["supplied", &sid], &qmap(&[]), b"").unwrap();
    assert_eq!(code_after, 200);
    assert_eq!(supplied_before, supplied_after);
}
