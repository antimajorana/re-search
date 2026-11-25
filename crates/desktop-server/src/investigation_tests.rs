use super::*;
use crate::auth::AuthGuard;
use crate::chatgpt::{offline_test_hooks, ChatGptHub};
use crate::investigation_question::{conventions_resume_prompt, question_context, scope, Scope};
use crate::sse::SseHub;
use crate::DesktopConfig;
use research_tools::{ingest_bytes, IngestRequest, LiteratureEndpoints, ResearchEnv};
use std::path::Path;
use std::sync::Mutex;

const SID: &str = "inv-unit";

fn test_state(data_dir: &Path) -> DesktopState {
    let cfg = DesktopConfig {
        port: 0,
        data_dir: data_dir.to_path_buf(),
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(Default::default()),
        chatgpt_test: Some(offline_test_hooks()),
    };
    let auth = AuthGuard::new(0).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    DesktopState::new(cfg, auth, sse, chatgpt)
}

fn seed_investigation_with_paper(state: &DesktopState) -> (Ledger, research_tools::IngestedSource) {
    let env = ResearchEnv::new(state.cfg.data_dir.clone());
    let ledger = Mutex::new(state.open_ledger().unwrap());
    ledger
        .lock()
        .unwrap()
        .append(SID, "investigation.started", 1, &json!({}))
        .unwrap();
    let paper = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: SID,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"Line one.\nLine two.\nEquation (1) holds.\n",
            source_id_hint: None,
            section_filter: None,
            title: Some("Sample Paper"),
        },
    )
    .unwrap();
    (ledger.into_inner().unwrap(), paper)
}

#[test]
fn long_model_context_keeps_source_locations_and_reports_omissions() {
    let anchor = json!({"source_id":"paper","path":"paper/main.txt","start_line":12,"end_line":14,"excerpt":"e".repeat(600)});
    let nodes: Vec<_> = (0..180)
        .map(|id| json!({"id":id,"sentence":"The observed frequency increased.","anchors":[anchor]}))
        .collect();
    let stored = json!({"node_id":"selected","dependencies":nodes});
    let text = context_text(&stored);
    let prompt: Value = serde_json::from_str(&text).unwrap();
    assert!(text.len() < 8192);
    assert_eq!(prompt["node_id"], "selected");
    assert_eq!(prompt["dependencies"][0]["anchors"][0]["start_line"], 12);
    assert!(prompt["dependencies"][0]["anchors"][0]["excerpt"].is_null());
    assert!(
        prompt["additional_saved_rows"]["dependencies"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        stored["dependencies"][0]["anchors"][0]["excerpt"],
        anchor["excerpt"]
    );
}

#[test]
fn scope_routes_plain_send_to_question_with_and_without_map() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let without_map = ledger.read_session(SID).unwrap();
    assert_eq!(scope(&without_map, &json!({})), Scope::Question);
    assert_eq!(
        scope(
            &without_map,
            &json!({"investigation_source_id": paper.source_id, "investigation_scope": "question"})
        ),
        Scope::Question
    );

    ledger
        .append(
            SID,
            "investigation.map",
            1,
            &json!({"source_id": paper.source_id, "nodes": [], "edges": []}),
        )
        .unwrap();
    let with_map = ledger.read_session(SID).unwrap();
    assert_eq!(scope(&with_map, &json!({})), Scope::Question);
}

#[test]
fn scope_keeps_mapping_for_source_id_without_scope_and_for_scope_map() {
    assert_eq!(
        scope(&[], &json!({"investigation_source_id": "src-1"})),
        Scope::Map
    );
    assert_eq!(
        scope(&[], &json!({"investigation_scope": "map"})),
        Scope::Map
    );
    assert_eq!(
        scope(
            &[],
            &json!({"investigation_source_id": "src-1", "investigation_scope": "map"})
        ),
        Scope::Map
    );
}

#[test]
fn scope_keeps_conventions_branch_action_precedence() {
    let all = json!({
        "investigation_conventions": {"map_seq": 3},
        "investigation_branch": {"map_seq": 3, "node_id": "n1", "question": "why?"},
        "investigation_action": {"map_seq": 3, "node_id": "n1", "action": "check"},
        "investigation_scope": "map",
        "investigation_source_id": "src-1"
    });
    assert_eq!(scope(&[], &all), Scope::Conventions);

    let mut no_conv = all.clone();
    no_conv.as_object_mut().unwrap().remove("investigation_conventions");
    assert_eq!(scope(&[], &no_conv), Scope::Branch);

    let mut no_branch = no_conv.clone();
    no_branch.as_object_mut().unwrap().remove("investigation_branch");
    assert_eq!(scope(&[], &no_branch), Scope::Action);

    let mut no_action = no_branch;
    no_action.as_object_mut().unwrap().remove("investigation_action");
    assert_eq!(scope(&[], &no_action), Scope::Map);
}

#[test]
fn prepare_question_pins_once_appends_request_and_strips_forged_key() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let mut first = json!({
        "investigation_source_id": paper.source_id,
        "investigation_scope": "question",
        "_paper_question_request": 9999,
        "verification_mode": true,
        "calculation_mode": true,
        "use_strategist": true
    });
    prepare(&state, SID, &mut first, "Explain eq (1)").unwrap();
    let first_seq = first["_paper_question_request"].as_u64().unwrap();
    assert_ne!(first_seq, 9999);
    assert_eq!(first["verification_mode"], false);
    assert_eq!(first["calculation_mode"], false);
    assert_eq!(first["use_strategist"], false);
    let ctx = first["_paper_context"].as_str().unwrap();
    assert!(ctx.contains(&paper.source_id));
    assert!(ctx.contains("[Lstart–end]"));
    assert!(!ctx.contains("ledger_append"));
    assert!(!ctx.contains("final_answer"));

    let mut second = json!({"_paper_question_request": 7777});
    prepare(&state, SID, &mut second, "Follow-up question").unwrap();
    let second_seq = second["_paper_question_request"].as_u64().unwrap();
    assert!(second_seq > first_seq);

    let events = ledger.read_session(SID).unwrap();
    let pins: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "investigation.paper")
        .collect();
    assert_eq!(pins.len(), 1);
    let reqs: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "investigation.question_requested")
        .collect();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].seq, first_seq);
    assert_eq!(reqs[0].payload["question"], "Explain eq (1)");
    assert!(reqs[0].payload["map_seq"].is_null());
    assert_eq!(reqs[1].seq, second_seq);
    assert_eq!(reqs[1].payload["question"], "Follow-up question");
}

#[test]
fn question_context_includes_map_when_present_and_page_sentence_when_set() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let events_no_map = ledger.read_session(SID).unwrap();
    let plain = question_context(
        &events_no_map,
        &paper.source_id,
        paper.primary_path.as_deref().unwrap(),
        (None, None),
    );
    assert!(!plain.contains("Current map:"));
    assert!(!plain.contains("physical PDF pages"));
    assert!(!plain.contains("ledger_append"));
    assert!(!plain.contains("final_answer"));

    let with_pages = question_context(
        &events_no_map,
        &paper.source_id,
        paper.primary_path.as_deref().unwrap(),
        (Some(2), Some(5)),
    );
    assert!(with_pages.contains("physical PDF pages 2 through 5 inclusive."));

    let map_seq = ledger
        .append(
            SID,
            "investigation.map",
            1,
            &json!({
                "source_id": paper.source_id,
                "title": "Sample Map",
                "nodes": [{"id": "r1", "sentence": "Main result.", "kind": "result", "main": true, "anchors": []}],
                "edges": []
            }),
        )
        .unwrap();
    let events_with_map = ledger.read_session(SID).unwrap();
    let ctx = question_context(
        &events_with_map,
        &paper.source_id,
        paper.primary_path.as_deref().unwrap(),
        (Some(2), Some(5)),
    );
    assert!(ctx.contains("physical PDF pages 2 through 5 inclusive."));
    assert!(ctx.contains("Current map:\n"));
    assert!(ctx.contains("Sample Map"));
    assert!(ctx.contains(&format!("\"map_seq\":{map_seq}")));
}

#[test]
fn mapping_request_writes_conventions_requested_false_and_omits_catalog_sentence() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let mut req = json!({
        "investigation_source_id": paper.source_id,
        "investigation_scope": "map"
    });
    prepare(&state, SID, &mut req, "Find main results").unwrap();
    let map_req_seq = req["_paper_map_request"].as_u64().unwrap();
    let ctx = req["_paper_context"].as_str().unwrap();
    assert!(!ctx.contains("record_paper_conventions"));
    assert!(!ctx.contains("finish the conventions catalog"));

    let events = ledger.read_session(SID).unwrap();
    let ev = events.iter().find(|e| e.seq == map_req_seq).unwrap();
    assert_eq!(ev.kind, "investigation.mapping_requested");
    assert_eq!(ev.payload["conventions_requested"], false);
    assert_eq!(ev.payload["summary_requested"], true);
}

#[test]
fn finish_maps_without_catalog_returns_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let mut req = json!({
        "investigation_source_id": paper.source_id,
        "investigation_scope": "map"
    });
    prepare(&state, SID, &mut req, "Find main results").unwrap();
    let map_req_seq = req["_paper_map_request"].as_u64().unwrap();
    ledger
        .append(
            SID,
            "investigation.map",
            1,
            &json!({"source_id": paper.source_id, "request_seq": map_req_seq, "nodes": [], "edges": []}),
        )
        .unwrap();

    let out = finish(&state, SID, &req, Ok("Mapped cleanly.".into()));
    assert_eq!(out, Ok("Mapped cleanly.".into()));

    // Without a saved map, the updated failure copy is returned.
    let mut req2 = json!({"investigation_scope": "map"});
    prepare(&state, SID, &mut req2, "Retry").unwrap();
    let err = finish(&state, SID, &req2, Ok("No map.".into())).unwrap_err();
    assert_eq!(
        err,
        "The model finished without saving the main results with their sources. Try again, or choose another model."
    );
}

#[test]
fn finish_question_answered_failed_stopped_never_changes_outcome() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let mut req_ok = json!({
        "investigation_source_id": paper.source_id,
        "investigation_scope": "question"
    });
    prepare(&state, SID, &mut req_ok, "Q1").unwrap();
    let seq1 = req_ok["_paper_question_request"].as_u64().unwrap();
    let long_reply = "a".repeat(25_000);
    assert_eq!(finish(&state, SID, &req_ok, Ok(long_reply.clone())), Ok(long_reply));

    let mut req_fail = json!({});
    prepare(&state, SID, &mut req_fail, "Q2").unwrap();
    let seq2 = req_fail["_paper_question_request"].as_u64().unwrap();
    let long_err = "e".repeat(2_500);
    assert_eq!(
        finish(&state, SID, &req_fail, Err(long_err.clone())),
        Err(long_err)
    );

    let mut req_stop = json!({});
    prepare(&state, SID, &mut req_stop, "Q3").unwrap();
    let seq3 = req_stop["_paper_question_request"].as_u64().unwrap();
    ledger
        .append(
            SID,
            "session.abort",
            1,
            &json!({"reason": "stopped by user", "stopped": true}),
        )
        .unwrap();
    assert_eq!(
        finish(&state, SID, &req_stop, Err("stopped by user".into())),
        Err("stopped by user".into())
    );

    let events = ledger.read_session(SID).unwrap();
    let ans: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "investigation.answer")
        .collect();
    assert_eq!(ans.len(), 3);
    assert_eq!(ans[0].payload["request_seq"], seq1);
    assert_eq!(ans[0].payload["status"], "answered");
    assert_eq!(ans[0].payload["text"].as_str().unwrap().chars().count(), 24_000);
    assert!(ans[0].payload["error"].is_null());

    assert_eq!(ans[1].payload["request_seq"], seq2);
    assert_eq!(ans[1].payload["status"], "failed");
    assert!(ans[1].payload["text"].is_null());
    assert_eq!(ans[1].payload["error"].as_str().unwrap().chars().count(), 2_000);

    assert_eq!(ans[2].payload["request_seq"], seq3);
    assert_eq!(ans[2].payload["status"], "stopped");
    assert!(ans[2].payload["text"].is_null());
    assert_eq!(ans[2].payload["error"], "stopped by user");
}

#[test]
fn view_answers_running_vs_interrupted() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let mut req = json!({
        "investigation_source_id": paper.source_id,
        "investigation_scope": "question"
    });
    prepare(&state, SID, &mut req, "What is equation 1?").unwrap();
    let seq = req["_paper_question_request"].as_u64().unwrap();

    let events = ledger.read_session(SID).unwrap();
    let running_view = view(&events, true);
    assert_eq!(running_view["answers"].as_array().unwrap().len(), 1);
    assert_eq!(running_view["answers"][0]["seq"], seq);
    assert_eq!(running_view["answers"][0]["question"], "What is equation 1?");
    assert_eq!(running_view["answers"][0]["status"], "running");

    let idle_view = view(&events, false);
    assert_eq!(idle_view["answers"][0]["status"], "interrupted");

    ledger
        .append(SID, "session.abort", 1, &json!({"reason": "stopped", "stopped": true}))
        .unwrap();
    let aborted_events = ledger.read_session(SID).unwrap();
    assert_eq!(view(&aborted_events, true)["answers"][0]["status"], "interrupted");

    finish(&state, SID, &req, Err("stopped".into())).ok();
    let finished_events = ledger.read_session(SID).unwrap();
    assert_eq!(view(&finished_events, false)["answers"][0]["status"], "stopped");
}

#[test]
fn view_mapping_requested_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(tmp.path());
    let (ledger, paper) = seed_investigation_with_paper(&state);

    let mut q_req = json!({
        "investigation_source_id": paper.source_id,
        "investigation_scope": "question"
    });
    prepare(&state, SID, &mut q_req, "Question only").unwrap();
    let events_before = ledger.read_session(SID).unwrap();
    assert_eq!(view(&events_before, false)["mapping_requested"], false);

    let mut m_req = json!({"investigation_scope": "map"});
    prepare(&state, SID, &mut m_req, "Map now").unwrap();
    let events_after = ledger.read_session(SID).unwrap();
    assert_eq!(view(&events_after, false)["mapping_requested"], true);
}

#[test]
fn resume_prompt_says_review_when_no_batch() {
    let path = json!("src-1/paper.txt");
    let first = conventions_resume_prompt(&path, "src-1", 4, &Value::Null);
    assert!(first.starts_with("Review only the paper's conventions."));
    assert!(first.contains("starting at batch_index=1"));

    let prior = json!({"batch_index": 2, "items": []});
    let next = conventions_resume_prompt(&path, "src-1", 4, &prior);
    assert!(next.starts_with("Continue only the paper's conventions review."));
    assert!(next.contains("starting at batch_index=3"));
}
