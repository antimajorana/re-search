//! End-to-end tests for Explore X13 Directions exchange and F7 hand-over buttons
//! (`"Open in Solve"` & `"Investigate this paper"`).
//!
//! Covers:
//! - Scripted Directions exchange repair on Rule D1 / `"lean"` rejection + Concrete/Sketch persistence;
//! - `"Open in Solve"` (Concrete and Loose directions) via `POST /api/sessions/{id}/idea/handoff`:
//!   0 model calls, 0 network calls, Solve field alignment (`task_kind`, `target`), `check.reproduces`
//!   -> `suggested_done`, prose in `model_text`, 100% `QuoteRef` re-verification (`quotes_dropped == 0`);
//! - Tampered `QuoteRef` is dropped in child (`quotes_dropped >= 1`), never arrives as checked/cited;
//! - `"Investigate this paper"` hand-over: 0 model calls, child `[source.ingested..., handoff.packet,
//!   investigation.started]`, 100% `earlier_quotes` re-verification, idea subclaims as `focus_claims`;
//! - Strictly read-only child status (`GET /api/sessions/{id}/handoffs`) never mutates parent ledger;
//! - Rule S8 check on Directions view payloads.
//!
//! Zero live network/model calls, zero third-party paper text, zero `#[ignore]`.

mod common;

use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, Transport,
    TransportError, TransportInfo,
};
use common::{raw_http, test_cfg};
use desktop_server::idea::{
    build_idea_statement, IdeaPreset, IdeaStage, EVENT_IDEA_BRIEF, EVENT_IDEA_CANDIDATES,
    EVENT_IDEA_DIRECTIONS, EVENT_IDEA_MAP, EVENT_IDEA_PAPER, EVENT_IDEA_SEARCH_FINISHED,
    EVENT_IDEA_STAGE, EVENT_IDEA_STARTED, EVENT_IDEA_TRIAGE,
};
use desktop_server::idea_directions_exchange::{run_directions_with_transport, DirectionsRunResult};
use desktop_server::idea_view;
use desktop_server::start_desktop_server;
use ledger::{sha256_hex, Ledger};
use research_tools::idea_directions::{Concreteness, RECORDER_IDEA_DIRECTIONS};
use research_tools::idea_map::{MapNodeStatus, RelationToIdea};
use research_tools::prior_work_wording::contains_forbidden_prior_work_word;
use research_tools::quote_core::{quote_sha256, EVIDENCE_QUOTE_EVENT};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tempfile::TempDir;

struct SequenceDirectionsTransport {
    responses: Vec<Value>,
    calls: AtomicUsize,
}

impl Transport for SequenceDirectionsTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted-x13",
            endpoint: "mock://x13".into(),
            api_key_env: String::new(),
        }
    }

    fn prepare_request(&self, _req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest { body: b"{}".to_vec() })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let idx = self.calls.fetch_add(1, Ordering::SeqCst);
        let payload = self
            .responses
            .get(idx)
            .or_else(|| self.responses.last())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let raw = serde_json::to_vec(&payload).unwrap();
        Ok(ModelResponse {
            id: Some(format!("resp-dir-{idx}")),
            model: "mock-directions".into(),
            content: vec![ContentBlock::ToolUse {
                id: format!("tu-dir-{idx}"),
                name: RECORDER_IDEA_DIRECTIONS.into(),
                input: payload,
            }],
            stop_reason: StopReason::ToolUse,
            usage: json!({ "input_tokens": 120, "output_tokens": 80 }),
            raw_body: raw,
        })
    }
}

fn seed_mapped_explore_session(data_dir: &Path, sid: &str, with_directions: bool, tamper_quote: bool) -> String {
    fs::create_dir_all(data_dir).unwrap();
    let ledger = Ledger::open(data_dir).unwrap();
    let budget = IdeaPreset::Standard.default_budget();
    ledger.append(sid, EVENT_IDEA_STARTED, 1, &json!({ "preset": "standard", "budget": budget, "pipeline_version": 1, "offline": false })).unwrap();
    let stmt = build_idea_statement(1, "Bi-Lanczos growth separates volume-law and area-law phases in monitored Majorana chains.", &[], &[]).unwrap();
    ledger.append(sid, "idea.stated", 1, &serde_json::to_value(&stmt).unwrap()).unwrap();
    ledger.append(sid, EVENT_IDEA_BRIEF, 1, &json!({
        "restatement": "Test whether bi-Lanczos slope distinguishes monitored phases.",
        "subclaims": [
            { "id": "sc-1", "text": "Unmonitored Majorana chain has linear Lanczos growth", "idea_quote": "Bi-Lanczos growth" },
            { "id": "sc-2", "text": "Monitoring changes the asymptotic slope", "idea_quote": "monitored Majorana chains" }
        ]
    })).unwrap();

    // Real source file + source.ingested + verified evidence.quote so F7 child re-verification runs end-to-end.
    let source_id = "src-1";
    let rel_path = format!("{source_id}/paper.tex");
    let source_text = "Line 1: Introduction to Krylov complexity.\n\
                       Line 2: Lanczos coefficients grow linearly in the free Majorana chain.\n\
                       Line 3: Open question: how does projective monitoring alter the slope?\n";
    let quote_text = if tamper_quote {
        "Tampered sentence that does not exist anywhere in line 2 of paper.tex."
    } else {
        "Lanczos coefficients grow linearly in the free Majorana chain."
    };
    let full_path = data_dir.join("sessions").join(sid).join("sources").join(source_id).join("paper.tex");
    fs::create_dir_all(full_path.parent().unwrap()).unwrap();
    fs::write(&full_path, source_text).unwrap();
    let file_hash = sha256_hex(source_text.as_bytes());

    let src_seq = ledger.append(sid, "source.ingested", 1, &json!({
        "source_id": source_id, "kind": "tex", "primary_path": rel_path,
        "files": [rel_path], "files_sha256": { rel_path.clone(): file_hash.clone() },
        "file_sha256": file_hash.clone(), "bytes": source_text.len(),
    })).unwrap();

    ledger.append(sid, EVENT_IDEA_CANDIDATES, 1, &json!({
        "round": 1,
        "candidates": [{ "key": "arxiv:2405.01111", "title": "Krylov Growth in Quadratic Chains", "ids": { "arxiv": "2405.01111" }, "version": "v1" }]
    })).unwrap();
    ledger.append(sid, EVENT_IDEA_SEARCH_FINISHED, 1, &json!({ "round": 1, "offline": false, "queries": [] })).unwrap();
    ledger.append(sid, EVENT_IDEA_TRIAGE, 1, &json!({ "reading_list": ["arxiv:2405.01111"], "screened": [] })).unwrap();
    ledger.append(sid, EVENT_IDEA_PAPER, 1, &json!({
        "key": "arxiv:2405.01111", "status": "read_full", "source_id": source_id, "version": "v1"
    })).unwrap();

    let xe_seq = ledger.append(sid, EVIDENCE_QUOTE_EVENT, 1, &json!({
        "source_id": source_id, "path": rel_path, "start_line": 2, "end_line": 2,
        "file_sha256": file_hash, "quote_sha256": quote_sha256(quote_text),
        "quote": quote_text, "excerpt": source_text, "source_seq": src_seq,
        "caller": "host:idea:read", "paper_key": "arxiv:2405.01111",
        "role": "states_result", "subclaim_ids": ["sc-1"], "gist": "Free chain exhibits linear Lanczos growth."
    })).unwrap();

    let q_text = "Open question: how does projective monitoring alter the slope?";
    let xe_q_seq = ledger
        .append(
            sid,
            EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "source_id": source_id,
                "path": rel_path,
                "start_line": 3,
                "end_line": 3,
                "file_sha256": file_hash,
                "quote_sha256": quote_sha256(q_text),
                "quote": q_text,
                "excerpt": source_text,
                "source_seq": src_seq,
                "caller": "host:idea:read",
                "paper_key": "arxiv:2405.01111",
                "role": "poses_open_question",
                "subclaim_ids": ["sc-2"],
                "gist": "Monitoring effect left open."
            }),
        )
        .unwrap();

    let xe_id = format!("xe-{xe_seq}");
    let xe_q_id = format!("xe-{xe_q_seq}");
    let map_seq = ledger
        .append(
            sid,
            EVENT_IDEA_MAP,
            1,
            &json!({
                "idea_rev": 1,
                "nodes": [
                    {
                        "id": "k-1",
                        "kind": "known_result",
                        "rev": 1,
                        "label": "Linear Lanczos growth",
                        "statement": "Unmonitored free Majorana chain shows linear Lanczos growth.",
                        "evidence": [xe_id],
                        "relation_to_idea": RelationToIdea::SpecialCase,
                        "subclaim_ids": ["sc-1"],
                        "papers": ["arxiv:2405.01111"],
                        "status": MapNodeStatus::Active,
                        "identity_key": xe_id
                    },
                    {
                        "id": "q-1",
                        "kind": "open_question",
                        "rev": 1,
                        "label": "Monitoring effect on Lanczos slope",
                        "statement": "Authors ask how projective monitoring alters the slope.",
                        "evidence": [xe_q_id],
                        "relation_to_idea": RelationToIdea::Same,
                        "subclaim_ids": ["sc-2"],
                        "papers": ["arxiv:2405.01111"],
                        "status": MapNodeStatus::Active,
                        "identity_key": xe_q_id
                    },
                    {
                        "id": "w-1",
                        "kind": "related_work",
                        "rev": 1,
                        "label": "Abstract-only work",
                        "statement": "Related abstract hint.",
                        "evidence": ["xa-99"],
                        "relation_to_idea": RelationToIdea::Background,
                        "subclaim_ids": [],
                        "papers": ["arxiv:2405.02222"],
                        "status": MapNodeStatus::Active,
                        "identity_key": "xa-99"
                    }
                ],
                "edges": [],
                "overlaps": [],
                "dropped": []
            }),
        )
        .unwrap();

    for st in [IdeaStage::Sharpen, IdeaStage::Search, IdeaStage::Triage, IdeaStage::Read, IdeaStage::Map] {
        ledger
            .append(
                sid,
                EVENT_IDEA_STAGE,
                1,
                &json!({ "stage": st.as_str(), "state": "finished", "request_id": "seed" }),
            )
            .unwrap();
    }

    if with_directions {
        let dir_payload = research_tools::idea_directions::validate_record_idea_directions(
            &ledger.read_session(sid).unwrap(),
            Some(map_seq),
            &sample_directions_input(),
        )
        .unwrap();
        ledger
            .append(sid, EVENT_IDEA_DIRECTIONS, 1, &serde_json::to_value(&dir_payload).unwrap())
            .unwrap();
        ledger
            .append(
                sid,
                EVENT_IDEA_STAGE,
                1,
                &json!({ "stage": "directions", "state": "finished", "request_id": "seed-dir" }),
            )
            .unwrap();
    }

    sid.to_string()
}

fn sample_directions_input() -> Value {
    json!({
        "directions": [
            {
                "title": "Bi-Lanczos slope along the no-click trajectory",
                "question": "Does the bi-Lanczos slope change across the measurement transition?",
                "known": [
                    { "text": "Free Majorana chain shows linear growth", "node_ids": ["k-1"] }
                ],
                "missing": [
                    { "text": "Monitored bi-Lanczos slope", "basis": "open_question(q-1)" }
                ],
                "first_calculation": {
                    "setup": "Construct the quadratic no-click Hamiltonian on L = 64..256 sites and evaluate bi-Lanczos coefficients for the first calculation.",
                    "inputs": ["k-1"],
                    "method": "numeric",
                    "expected_form": "Numeric table of b_n slopes across measurement rate gamma",
                    "check": {
                        "text": "Zero-measurement limit must reproduce linear slope of k-1",
                        "reproduces": "k-1"
                    },
                    "effort": "days"
                },
                "risks": [
                    { "text": "No-click trajectory may differ from averaged trajectories", "kind": "technical", "node_ids": ["k-1"] }
                ],
                "stop_if": "Slopes coincide within uncertainty at L = 256",
                "solve_setup": {
                    "statement": "Compute single-particle bi-Lanczos coefficients b_n for the monitored Majorana chain.",
                    "task_kind": "calculate",
                    "target": {
                        "description": "Numeric slope alpha(gamma) of b_n vs n",
                        "form": "numeric"
                    },
                    "conventions": "Natural units hbar = 1, periodic boundary conditions",
                    "source_keys": ["arxiv:2405.01111"]
                }
            },
            {
                "title": "Sketch spectral-flow estimate",
                "question": "Can spectral flow constrain bi-Lanczos growth?",
                "known": [
                    { "text": "Abstract-only hint", "node_ids": ["w-1"] }
                ],
                "missing": [
                    { "text": "Explicit bound", "basis": "no_evidence_in_scope" }
                ],
                "first_calculation": {
                    "setup": "Short sketch",
                    "inputs": [],
                    "method": "by_hand",
                    "expected_form": "Bound",
                    "check": { "text": "" },
                    "effort": "weeks"
                },
                "risks": [],
                "stop_if": "",
                "solve_setup": {
                    "statement": "Check whether spectral flow bounds bi-Lanczos growth.",
                    "task_kind": "prove",
                    "target": {
                        "description": "Upper bound on b_n",
                        "form": "bound"
                    },
                    "conventions": "",
                    "source_keys": ["arxiv:2405.01111"]
                }
            }
        ]
    })
}

#[test]
fn x13_directions_exchange_repairs_bad_input_and_persists_concrete_and_sketch() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let sid = seed_mapped_explore_session(&data_dir, "s-x13-dir", false, false);
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let lock = Mutex::new(());

    // Attempt 1 uses forbidden `lean` method -> exchange repairs -> Attempt 2 succeeds.
    let mut bad_first = sample_directions_input();
    bad_first["directions"][0]["first_calculation"]["method"] = json!("lean");
    let transport = SequenceDirectionsTransport {
        responses: vec![bad_first, sample_directions_input()],
        calls: AtomicUsize::new(0),
    };

    let res = run_directions_with_transport(
        &ledger,
        &lock,
        &sid,
        &transport,
        "mock-directions",
        None,
        |_, _| {},
    )
    .expect("directions exchange should succeed after repair");

    match res {
        DirectionsRunResult::Finished { payload, attempts, .. } => {
            assert_eq!(attempts, 2, "must repair invalid attempt 1");
            assert_eq!(payload.directions.len(), 2);
            assert_eq!(payload.directions[0].id, "r-1");
            assert_eq!(payload.directions[0].concreteness, Concreteness::Concrete);
            assert_eq!(payload.directions[1].id, "r-2");
            assert_eq!(payload.directions[1].concreteness, Concreteness::Sketch);
        }
        other => panic!("expected Finished, got {other:?}"),
    }

    // Verify idea_view projects directions and stays Rule S8 clean.
    let events = ledger.read_session(&sid).unwrap();
    let view = idea_view::view(&events, false);
    assert_eq!(view["directions"].as_array().unwrap().len(), 2);
    let view_str = view.to_string();
    assert_eq!(
        contains_forbidden_prior_work_word(&view_str),
        None,
        "Rule S8 forbidden word found in Explore directions view: {view_str}"
    );
}

#[test]
fn x13_open_in_solve_concrete_and_loose_zero_model_calls_reverify_and_read_only_status() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    // No model API keys/providers configured -> proves 0 model calls and 0 network calls!
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    let sid = seed_mapped_explore_session(&data_dir, "s-x13-solve", true, false);
    // Attach user notes on direction r-1 and node k-1 to verify user_notes carry-over.
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    ledger.append(&sid, desktop_server::idea::EVENT_IDEA_NOTE, 1, &json!({
        "target_kind": "direction", "target_id": "r-1", "action": "pin", "comment": "Start with L=64"
    })).unwrap();
    ledger.append(&sid, desktop_server::idea::EVENT_IDEA_NOTE, 1, &json!({
        "target_kind": "node", "target_id": "k-1", "action": "comment", "comment": "Check PBC factor of 2"
    })).unwrap();

    // 1. Hand over Concrete direction r-1 -> Solve (HTTP 201, 0 model calls).
    let body_r1 = serde_json::to_vec(&json!({ "action": "open_in_solve", "direction_id": "r-1" })).unwrap();
    let (st1, _, raw1) = raw_http(port, "POST", "/api/sessions/s-x13-solve/idea/handoff", &auth, &body_r1);
    assert_eq!(st1, 201, "r-1 handoff failed: {}", String::from_utf8_lossy(&raw1));
    let j1: Value = serde_json::from_slice(&raw1).unwrap();
    let child_r1 = j1["session_id"].as_str().unwrap().to_string();
    assert_eq!(j1["mode"], "solve");
    assert_eq!(j1["receipt"]["quotes_dropped"], 0, "100% QuoteRefs must survive re-verification");
    assert_eq!(j1["prefill"]["task_kind"], "calculate");
    assert_eq!(j1["prefill"]["target"]["form"], "numeric");
    assert_eq!(j1["prefill"]["steps"][0]["kind"], "cited");
    assert!(!j1["prefill"]["suggested_done"].as_array().unwrap().is_empty());
    assert_eq!(j1["prefill"]["suggested_done"][0]["params"]["reproduces"], "k-1");
    assert_eq!(j1["prefill"]["suggested_done"][0]["params"]["check"], "reproduce_k_1");
    let model_notes = j1["prefill"]["model_notes"].as_array().unwrap();
    assert!(!model_notes.is_empty(), "direction prose travels in model_notes");
    assert!(model_notes.iter().all(|n| n["label"].as_str().unwrap_or("").contains("(model)")));
    let user_notes = j1["prefill"]["user_notes"].as_array().unwrap();
    assert!(user_notes.iter().any(|u| u.as_str().unwrap_or("").contains("Start with L=64")));
    assert!(user_notes.iter().any(|u| u.as_str().unwrap_or("").contains("Check PBC factor of 2")));

    // CRITICAL audit regression: handed-off Solve prefill + suggested_done MUST pass Solve's own setup validator!
    let solve_input: research_tools::solve_setup::SolveSetupInput = serde_json::from_value(json!({
        "statement": j1["prefill"]["statement"],
        "task_kind": j1["prefill"]["task_kind"],
        "target": j1["prefill"]["target"],
        "done": j1["prefill"]["suggested_done"],
        "conventions": j1["prefill"]["conventions"],
        "origin": j1["prefill"]["origin"],
    }))
    .unwrap();
    research_tools::solve_setup::validate_and_build_setup(&solve_input, 0)
        .expect("handed-off Solve prefill and suggested_done must pass Solve setup validation");

    // Child ledger starts with [source.ingested..., handoff.packet, solve.started].
    let child1_events = ledger.read_session(&child_r1).unwrap();
    let kinds: Vec<&str> = child1_events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["source.ingested", "handoff.packet", "solve.started"]);

    // 2. Hand over Loose/Sketch direction r-2 -> Solve (also works!).
    let body_r2 = serde_json::to_vec(&json!({ "action": "open_in_solve", "direction_id": "r-2" })).unwrap();
    let (st2, _, raw2) = raw_http(port, "POST", "/api/sessions/s-x13-solve/idea/handoff", &auth, &body_r2);
    assert_eq!(st2, 201, "r-2 loose handoff failed: {}", String::from_utf8_lossy(&raw2));
    let j2: Value = serde_json::from_slice(&raw2).unwrap();
    let child_r2 = j2["session_id"].as_str().unwrap().to_string();
    assert_eq!(j2["prefill"]["task_kind"], "prove");
    assert_eq!(j2["prefill"]["target"]["form"], "bound");

    // Error paths: 404 unknown direction, 400 invalid paper_key.
    let (st404, _, _) = raw_http(port, "POST", "/api/sessions/s-x13-solve/idea/handoff", &auth, br#"{"action":"open_in_solve","direction_id":"r-99"}"#);
    assert_eq!(st404, 404);
    let (st400, _, _) = raw_http(port, "POST", "/api/sessions/s-x13-solve/idea/handoff", &auth, br#"{"action":"investigate_paper","paper_key":"not-a-key"}"#);
    assert_eq!(st400, 400);

    // 3. Record solve.done in both children and verify strictly read-only status flow-back.
    let parent_before = ledger.read_session(&sid).unwrap();
    let parent_len_before = parent_before.len();
    let parent_head_before = parent_before.last().unwrap().event_hash.clone();

    ledger.append(&child_r1, "solve.done", 1, &json!({ "outcome": "finished" })).unwrap();
    ledger.append(&child_r2, "solve.done", 1, &json!({ "outcome": "finished_open_items" })).unwrap();

    let (st_h, _, raw_h) = raw_http(port, "GET", "/api/sessions/s-x13-solve/handoffs", &auth, b"");
    assert_eq!(st_h, 200);
    let h_view: Value = serde_json::from_slice(&raw_h).unwrap();
    let outgoing = h_view["outgoing"].as_array().unwrap();
    assert_eq!(outgoing.len(), 2);
    assert_eq!(outgoing[0]["from"]["id"], "r-1");
    assert_eq!(outgoing[0]["child_status"]["label"], "Opened in Solve: Done");
    assert_eq!(outgoing[1]["from"]["id"], "r-2");
    assert_eq!(outgoing[1]["child_status"]["label"], "Opened in Solve: Done with open items");

    // Rule 5: reading child status NEVER writes to parent Explore ledger.
    let parent_after = ledger.read_session(&sid).unwrap();
    assert_eq!(parent_after.len(), parent_len_before);
    assert_eq!(parent_after.last().unwrap().event_hash, parent_head_before);
}

#[test]
fn x13_tampered_quote_drops_in_child_and_investigate_paper_handoff_re_verifies() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // Part A: Investigate this paper while `map_stale == true` (after idea rev 2) STILL preserves
    // matched quotes, node summary, and user notes!
    let clean_sid = seed_mapped_explore_session(&data_dir, "s-x13-inv", true, false);
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    ledger.append(&clean_sid, desktop_server::idea::EVENT_IDEA_NOTE, 1, &json!({
        "target_kind": "node", "target_id": "k-1", "action": "pin", "comment": "Verify Appendix B"
    })).unwrap();
    let rev2 = build_idea_statement(2, "Revised monitored chain question.", &[], &[]).unwrap();
    ledger.append(&clean_sid, "idea.stated", 1, &serde_json::to_value(&rev2).unwrap()).unwrap();

    let inv_body = serde_json::to_vec(&json!({
        "action": "investigate_paper",
        "paper_key": "arxiv:2405.01111",
        "node_id": "k-1"
    }))
    .unwrap();
    let (st_inv, _, raw_inv) = raw_http(port, "POST", "/api/sessions/s-x13-inv/idea/handoff", &auth, &inv_body);
    assert_eq!(st_inv, 201, "investigate handoff failed: {}", String::from_utf8_lossy(&raw_inv));
    let j_inv: Value = serde_json::from_slice(&raw_inv).unwrap();
    let inv_child = j_inv["session_id"].as_str().unwrap().to_string();
    assert_eq!(j_inv["mode"], "investigation");
    assert_eq!(j_inv["receipt"]["quotes_dropped"], 0);
    assert_eq!(j_inv["prefill"]["matched_quotes"].as_array().unwrap().len(), 2);
    assert_eq!(j_inv["prefill"]["focus_claims"].as_array().unwrap().len(), 2);
    assert!(j_inv["prefill"]["user_notes"].as_array().unwrap().iter().any(|u| u.as_str().unwrap_or("").contains("Verify Appendix B")));

    let inv_events = ledger.read_session(&inv_child).unwrap();
    assert_eq!(inv_events[0].kind, "source.ingested");
    assert_eq!(inv_events[1].kind, "handoff.packet");
    assert_eq!(inv_events[2].kind, "investigation.started");

    // Append investigation.skeleton in Investigate child -> parent reads "Skeleton ready" without writes.
    let parent_before = ledger.read_session(&clean_sid).unwrap();
    ledger.append(&inv_child, "investigation.skeleton", 1, &json!({ "sections": [] })).unwrap();
    let (st_get, _, raw_get) = raw_http(port, "GET", "/api/sessions/s-x13-inv/handoffs", &auth, b"");
    assert_eq!(st_get, 200);
    let gv: Value = serde_json::from_slice(&raw_get).unwrap();
    assert_eq!(gv["outgoing"][0]["child_status"]["label"], "Opened in Investigate: Skeleton ready");
    assert_eq!(ledger.read_session(&clean_sid).unwrap().len(), parent_before.len());

    // Part B: Tampered evidence.quote text (does not match intact source file) -> dropped in child!
    let _tampered_sid = seed_mapped_explore_session(&data_dir, "s-x13-tamper", true, true);
    let body_t = serde_json::to_vec(&json!({ "action": "open_in_solve", "direction_id": "r-1" })).unwrap();
    let (st_t, _, raw_t) = raw_http(port, "POST", "/api/sessions/s-x13-tamper/idea/handoff", &auth, &body_t);
    assert_eq!(st_t, 201, "tampered quote handoff should still create child with dropped quotes: {}", String::from_utf8_lossy(&raw_t));
    let jt: Value = serde_json::from_slice(&raw_t).unwrap();
    assert!(
        jt["receipt"]["quotes_dropped"].as_u64().unwrap() >= 1,
        "tampered QuoteRef MUST be dropped during child re-verification: {jt}"
    );
    assert_eq!(
        jt["prefill"]["steps"][0]["kind"], "given",
        "dropped tampered quote must never arrive as cited/checked"
    );
}
