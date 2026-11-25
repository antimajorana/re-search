use crate::auth::AuthGuard;
use crate::chatgpt::ChatGptHub;
use crate::investigation::{finish, prepare, view, PaperRoute};
use crate::sse::SseHub;
use crate::state::DesktopState;
use crate::DesktopConfig;
use ledger::Ledger;
use research_tools::investigation::{
    paper_conventions, record, record_conventions, record_summary,
};
use research_tools::{ingest_bytes, IngestRequest, LiteratureEndpoints, ResearchEnv};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const APS_TEXT: &str = "\
Abstract
We analyze dislocation braiding in two-dimensional topological phases.

I. INTRODUCTION
Throughout the paper, G denotes the global symmetry group.

II. SYMMETRY DEFECTS
We realize defects on a lattice.

A. Twist Defects
Twist defects permute bulk anyons.

1. Dislocations
In this subsection, b denotes the Burgers vector of a lattice dislocation.
  d_1 x d_2 = sum_a N_{12}^a a                                    (52)

III. CONCLUSIONS
Dislocations behave as non-Abelian twist defects.
";

fn test_state() -> (tempfile::TempDir, Arc<DesktopState>, ResearchEnv) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = DesktopConfig {
        port: 0,
        data_dir: tmp.path().to_path_buf(),
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
        connection_env: Some(BTreeMap::new()),
        chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
    };
    let state = Arc::new(DesktopState::new(
        cfg.clone(),
        AuthGuard::new(1).unwrap(),
        Arc::new(SseHub::new()),
        ChatGptHub::new(&cfg).unwrap(),
    ));
    let env = ResearchEnv::new(tmp.path().to_path_buf());
    (tmp, state, env)
}

fn seed_paper(
    state: &DesktopState,
    env: &ResearchEnv,
    sid: &str,
) -> research_tools::IngestedSource {
    let db = Mutex::new(state.open_ledger().unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "aps.txt",
            origin: "upload",
            raw_bytes: APS_TEXT.as_bytes(),
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger.append(sid, "investigation.started", 1, &json!({})).unwrap();
    src
}

fn anchor(src: &research_tools::IngestedSource, line: usize, excerpt: &str) -> Value {
    json!({
        "source_id": src.source_id,
        "path": src.primary_path,
        "start_line": line,
        "end_line": line,
        "excerpt": excerpt,
        "kind": "passage"
    })
}

fn record_summary_and_map(
    env: &ResearchEnv,
    ledger: &Ledger,
    sid: &str,
    src: &research_tools::IngestedSource,
) -> u64 {
    let a_sum = anchor(
        src,
        2,
        "We analyze dislocation braiding in two-dimensional topological phases.",
    );
    record_summary(
        env,
        ledger,
        sid,
        &json!({
            "source_id": src.source_id,
            "what": {"text": "Analyzes dislocation braiding.", "anchors": [a_sum]},
            "why": {"unstated": true},
            "how": {"unstated": true}
        }),
    )
    .unwrap();
    record(
        env,
        ledger,
        sid,
        &json!({
            "source_id": src.source_id,
            "title": "Dislocation braiding",
            "coverage": "paper",
            "limitations": [],
            "nodes": [{"id": "n1", "kind": "result", "main": true, "sentence": "Dislocations braid non-trivially.", "anchors": [a_sum]}],
            "edges": []
        }),
    )
    .unwrap()["map_seq"]
        .as_u64()
        .unwrap()
}

#[test]
fn finish_accepts_section_catalog() {
    let (_tmp, state, env) = test_state();
    let sid = "s-section-finish";
    let src = seed_paper(&state, &env, sid);

    let mut req = json!({"investigation_source_id": src.source_id});
    prepare(&state, sid, &mut req, "Help me understand section II.A.1", PaperRoute::InCore).unwrap();
    let req_seq = req["_paper_map_request"].as_u64().unwrap();
    let ctx = req["_paper_context"].as_str().unwrap();
    assert!(ctx.contains("Paper structure"), "{ctx}");
    assert!(ctx.contains("Section II.A.1"), "{ctx}");
    assert!(ctx.contains("record conventions for the scope plus global notation"), "{ctx}");

    let ledger = state.open_ledger().unwrap();
    let events = ledger.read_session(sid).unwrap();
    let mapping_ev = events.iter().find(|e| e.seq == req_seq).unwrap();
    assert_eq!(mapping_ev.payload["conventions_scope"], "section");
    assert_eq!(mapping_ev.payload["scope"]["status"], "resolved");
    let scope_ev = events
        .iter()
        .find(|e| e.kind == "investigation.scope")
        .expect("investigation.scope recorded");
    assert_eq!(scope_ev.payload["request_seq"], req_seq);
    assert_eq!(scope_ev.payload["status"], "resolved");

    record_summary_and_map(&env, &ledger, sid, &src);
    let a_b = anchor(
        &src,
        14,
        "In this subsection, b denotes the Burgers vector of a lattice dislocation.",
    );
    record_conventions(
        &env,
        &ledger,
        sid,
        &json!({
            "request_seq": req_seq,
            "source_id": src.source_id,
            "batch_index": 1,
            "done": true,
            "coverage": "paper",
            "limitations": [],
            "items": [{
                "id": "burgers",
                "category": "notation",
                "label": "b",
                "meaning": "Burgers vector.",
                "scope": "Section II.A.1",
                "basis": "explicit",
                "anchors": [a_b]
            }]
        }),
    )
    .unwrap();

    let events = ledger.read_session(sid).unwrap();
    let cat = paper_conventions(&events, req_seq).unwrap();
    assert_eq!(cat["coverage_scope"], "section");
    assert_eq!(cat["done"], true);

    let outcome = finish(&state, sid, &req, Ok("Answered section II.A.1.".into()));
    assert_eq!(outcome.unwrap(), "Answered section II.A.1.");
    let events = ledger.read_session(sid).unwrap();
    let fin = events
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.mapping_finished")
        .unwrap();
    assert_eq!(fin.payload["status"], "mapped");
    assert!(fin.payload["conventions_error"].is_null());

    let v = view(&events, false);
    assert_eq!(v["scope"]["status"], "resolved");
    assert_eq!(v["conventions"]["coverage_scope"], "section");
}

#[test]
fn whole_paper_unchanged() {
    let (_tmp, state, env) = test_state();
    let sid = "s-whole-paper";
    let src = seed_paper(&state, &env, sid);

    let mut req = json!({"investigation_source_id": src.source_id});
    prepare(&state, sid, &mut req, "Identify the paper's main results.", PaperRoute::InCore).unwrap();
    let req_seq = req["_paper_map_request"].as_u64().unwrap();
    let ctx = req["_paper_context"].as_str().unwrap();
    assert!(!ctx.contains("Question scope:"));

    let ledger = state.open_ledger().unwrap();
    let events = ledger.read_session(sid).unwrap();
    let mapping_ev = events.iter().find(|e| e.seq == req_seq).unwrap();
    assert!(mapping_ev.payload.get("conventions_scope").is_none());
    assert!(mapping_ev.payload.get("scope").is_none());
    let scope_ev = events
        .iter()
        .find(|e| e.kind == "investigation.scope")
        .unwrap();
    assert_eq!(scope_ev.payload["status"], "unresolved");

    let map_seq = record_summary_and_map(&env, &ledger, sid, &src);
    let a_g = anchor(
        &src,
        5,
        "Throughout the paper, G denotes the global symmetry group.",
    );
    record_conventions(
        &env,
        &ledger,
        sid,
        &json!({
            "request_seq": req_seq,
            "source_id": src.source_id,
            "batch_index": 1,
            "done": true,
            "coverage": "paper",
            "limitations": [],
            "items": [{
                "id": "group-g",
                "category": "notation",
                "label": "G",
                "meaning": "Global symmetry group.",
                "scope": "Throughout the paper",
                "basis": "explicit",
                "anchors": [a_g]
            }]
        }),
    )
    .unwrap();
    finish(&state, sid, &req, Ok("Mapped.".into())).unwrap();

    let events = ledger.read_session(sid).unwrap();
    let cat = paper_conventions(&events, req_seq).unwrap();
    assert!(cat.get("coverage_scope").is_none());

    // Whole-paper done catalog still refuses conventions resume (409).
    let mut resume_req = json!({"investigation_conventions": {"map_seq": map_seq}});
    let err = prepare(&state, sid, &mut resume_req, "Continue", PaperRoute::InCore).unwrap_err();
    assert_eq!(err.0, 409);
}

#[test]
fn catalog_all_after_section_done_allowed() {
    let (_tmp, state, env) = test_state();
    let sid = "s-catalog-all";
    let src = seed_paper(&state, &env, sid);

    let mut req = json!({"investigation_source_id": src.source_id});
    prepare(&state, sid, &mut req, "Help me understand section II.A.1", PaperRoute::InCore).unwrap();
    let req_seq = req["_paper_map_request"].as_u64().unwrap();

    let ledger = state.open_ledger().unwrap();
    let map_seq = record_summary_and_map(&env, &ledger, sid, &src);
    let a_b = anchor(
        &src,
        14,
        "In this subsection, b denotes the Burgers vector of a lattice dislocation.",
    );
    let a_g = anchor(
        &src,
        5,
        "Throughout the paper, G denotes the global symmetry group.",
    );
    record_conventions(
        &env,
        &ledger,
        sid,
        &json!({
            "request_seq": req_seq,
            "source_id": src.source_id,
            "batch_index": 1,
            "done": true,
            "coverage": "paper",
            "limitations": [],
            "items": [{
                "id": "burgers",
                "category": "notation",
                "label": "b",
                "meaning": "Burgers vector.",
                "scope": "Section II.A.1",
                "basis": "explicit",
                "anchors": [a_b]
            }]
        }),
    )
    .unwrap();
    finish(&state, sid, &req, Ok("Done section.".into())).unwrap();

    // Resuming after a section-done catalog is allowed and emits conventions_resumed{scope:"paper"}.
    let mut resume_req = json!({"investigation_conventions": {"map_seq": map_seq}});
    prepare(&state, sid, &mut resume_req, "Catalog all conventions", PaperRoute::InCore).unwrap();
    assert_eq!(resume_req["_paper_conventions_request"], req_seq);
    assert!(resume_req["_paper_context"]
        .as_str()
        .unwrap()
        .contains("Expand the section conventions catalog"));

    let events = ledger.read_session(sid).unwrap();
    let resumed = events
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.conventions_resumed")
        .unwrap();
    assert_eq!(resumed.payload["scope"], "paper");
    assert_eq!(view(&events, true)["conventions_expanding"], true);

    // If the turn finishes without recording a paper-scope batch, finish marks it incomplete.
    let early = finish(&state, sid, &resume_req, Ok("Stopped early.".into()));
    assert!(early.is_err());

    // Start again and record batch 2 with done=true under scope="paper".
    let mut resume_req2 = json!({"investigation_conventions": {"map_seq": map_seq}});
    prepare(&state, sid, &mut resume_req2, "Catalog all conventions", PaperRoute::InCore).unwrap();
    record_conventions(
        &env,
        &ledger,
        sid,
        &json!({
            "request_seq": req_seq,
            "source_id": src.source_id,
            "batch_index": 2,
            "done": true,
            "coverage": "paper",
            "limitations": [],
            "items": [{
                "id": "group-g",
                "category": "notation",
                "label": "G",
                "meaning": "Global symmetry group.",
                "scope": "Throughout the paper",
                "basis": "explicit",
                "anchors": [a_g]
            }]
        }),
    )
    .unwrap();

    let done_outcome = finish(&state, sid, &resume_req2, Ok("Cataloged whole paper.".into()));
    assert_eq!(done_outcome.unwrap(), "Cataloged whole paper.");
    let events = ledger.read_session(sid).unwrap();
    let full = paper_conventions(&events, req_seq).unwrap();
    assert_eq!(full["coverage_scope"], "paper");
    assert_eq!(full["items"].as_array().unwrap().len(), 2);

    // Once upgraded to paper scope and done, another resume is refused (409).
    let mut again = json!({"investigation_conventions": {"map_seq": map_seq}});
    assert_eq!(prepare(&state, sid, &mut again, "Again", PaperRoute::InCore).unwrap_err().0, 409);
}

#[test]
fn question_turn_writes_scope_and_injects_outline() {
    let (_tmp, state, env) = test_state();
    let sid = "s-question-scope";
    let src = seed_paper(&state, &env, sid);

    let mut req = json!({
        "investigation_source_id": src.source_id,
        "investigation_scope": "question"
    });
    prepare(
        &state,
        sid,
        &mut req,
        "Help me understand section II.A.1",
        PaperRoute::InCore,
    )
    .unwrap();
    let q_seq = req["_paper_question_request"].as_u64().unwrap();
    assert!(req.get("_paper_map_request").is_none());
    let ctx = req["_paper_context"].as_str().unwrap();
    assert!(ctx.contains("Paper structure"), "{ctx}");
    assert!(ctx.contains("Question scope: Section II.A.1"), "{ctx}");
    assert!(!ctx.contains("record conventions for the scope"), "{ctx}");
    assert!(!ctx.contains("record_paper_map"), "{ctx}");

    let ledger = state.open_ledger().unwrap();
    let events = ledger.read_session(sid).unwrap();
    let q_ev = events.iter().find(|e| e.seq == q_seq).unwrap();
    assert_eq!(q_ev.kind, "investigation.question_requested");
    assert_eq!(q_ev.payload["scope"]["status"], "resolved");
    let scope_ev = events
        .iter()
        .find(|e| e.kind == "investigation.scope")
        .expect("investigation.scope recorded on question turn");
    assert_eq!(scope_ev.payload["request_seq"], q_seq);
    assert_eq!(scope_ev.payload["status"], "resolved");

    let v = view(&events, true);
    assert_eq!(v["mapping_requested"], false);
    assert_eq!(v["scope"]["status"], "resolved");
    assert_eq!(v["answers"][0]["status"], "running");
}
