//! End-to-end tests for Explore X11 Map exchange, atomic `idea.map` + `idea.verdict` via
//! `append_at_head`, `GraphView` + verdict projection, Rule V6 confirmation notes, and Rule V9
//! origin-citation marking in `idea_view`.
//!
//! Zero live network/model calls, zero third-party paper text, zero skipped tests.

use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, Transport,
    TransportError, TransportInfo,
};
use desktop_server::idea::{
    build_idea_statement, IdeaPreset, IdeaStage, EVENT_IDEA_BRIEF,
    EVENT_IDEA_CANDIDATES, EVENT_IDEA_MAP, EVENT_IDEA_NOTE, EVENT_IDEA_PAPER,
    EVENT_IDEA_SEARCH_FINISHED, EVENT_IDEA_SEARCH_PLANNED, EVENT_IDEA_STAGE, EVENT_IDEA_TRIAGE,
    EVENT_IDEA_VERDICT,
};
use desktop_server::idea_map_exchange::{run_map_with_transport, MapRunResult};
use desktop_server::idea_view;
use ledger::Ledger;
use research_tools::idea_map::RECORDER_IDEA_MAP;
use research_tools::idea_verdict;
use research_tools::prior_work::PriorWorkStatus;
use research_tools::quote_core::{quote_sha256, EVIDENCE_QUOTE_EVENT};
use serde_json::{json, Value};
use std::sync::Mutex;
use tempfile::TempDir;

struct FixedMapTransport {
    payload: Value,
    seen_prompts: Mutex<Vec<String>>,
}

impl Transport for FixedMapTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted-x11",
            endpoint: "mock://x11".into(),
            api_key_env: String::new(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        let prompt = req
            .messages
            .first()
            .and_then(|m| m.content.first())
            .and_then(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        self.seen_prompts.lock().unwrap().push(prompt);
        Ok(PreparedRequest { body: b"{}".to_vec() })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let raw = serde_json::to_vec(&self.payload).unwrap();
        Ok(ModelResponse {
            id: Some("resp-map-1".into()),
            model: "mock-map".into(),
            content: vec![ContentBlock::ToolUse {
                id: "tu-map-1".into(),
                name: RECORDER_IDEA_MAP.into(),
                input: self.payload.clone(),
            }],
            stop_reason: StopReason::ToolUse,
            usage: json!({ "input_tokens": 200, "output_tokens": 90 }),
            raw_body: raw,
        })
    }
}

fn sha64(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

#[test]
fn x11_map_appends_map_and_verdict_atomically_and_projects_graph_view_with_v6_v9() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-x11-atomic";

    // Session arrived from Investigate with origin paper arxiv:2001.00001 citing arxiv:2401.00002.
    ledger
        .append(
            sid,
            "handoff.packet",
            1,
            &json!({
                "from": {
                    "session": "s-inv",
                    "mode": "investigate",
                    "id": "n-1",
                    "paper": { "key": "arxiv:2001.00001", "ids": { "arxiv": "2001.00001" } }
                },
                "papers": [
                    { "key": "arxiv:2001.00001", "ids": { "arxiv": "2001.00001" } },
                    { "key": "arxiv:2401.00002", "ids": { "arxiv": "2401.00002" } }
                ]
            }),
        )
        .unwrap();

    let stmt = build_idea_statement(
        1,
        "Check whether Krylov complexity grows linearly in monitored Majorana chains.",
        &[],
        &[],
    )
    .unwrap();
    let budget = IdeaPreset::Standard.default_budget();
    ledger
        .append(
            sid,
            desktop_server::idea::EVENT_IDEA_STARTED,
            1,
            &json!({ "preset": "standard", "budget": budget, "pipeline_version": 1, "offline": false }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            desktop_server::idea::EVENT_IDEA_STATED,
            1,
            &serde_json::to_value(&stmt).unwrap(),
        )
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Linear Krylov growth in monitored Majorana chains.",
                "subclaims": [{ "id": "sc-1", "text": "b_n grows linearly", "idea_quote": "Krylov complexity" }],
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "question": "Exact?", "queries": [{ "provider": "arxiv", "q": "Krylov Majorana" }] },
                    { "id": "p-2", "kind": "other_names", "question": "Other?", "queries": [{ "provider": "inspire", "q": "Lanczos growth" }] }
                ]
            }),
        )
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Sharpen.as_str(), "state": "finished" }))
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_SEARCH_PLANNED,
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "Krylov Majorana", "origin": "model" },
                    { "qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "q": "Lanczos growth", "origin": "model" }
                ]
            }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_SEARCH_FINISHED,
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 2 },
                    { "qid": "q-2", "provider": "inspire", "status": "ok", "count": 2 }
                ]
            }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_CANDIDATES,
            1,
            &json!({
                "round": 1,
                "candidates": [
                    { "key": "arxiv:2001.00001", "title": "Origin Paper", "year": 2020 },
                    { "key": "arxiv:2401.00002", "title": "Cited Follow-up", "year": 2024 }
                ]
            }),
        )
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Search.as_str(), "state": "finished" }))
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_TRIAGE,
            1,
            &json!({
                "items": [{ "key": "arxiv:2401.00002", "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Core." }],
                "reading_list": ["arxiv:2401.00002"]
            }),
        )
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Triage.as_str(), "state": "finished" }))
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_PAPER,
            1,
            &json!({ "key": "arxiv:2401.00002", "status": "read_full", "text_basis": "tex", "lines_sent": 40, "total_lines": 40, "origin": "library" }),
        )
        .unwrap();
    let quote_txt = "Theorem 1: Lanczos coefficients grow linearly in monitored Majorana chains.";
    let xe_seq = ledger
        .append(
            sid,
            EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "source_id": "src-1",
                "path": "src-1/main.tex",
                "start_line": 4,
                "end_line": 5,
                "file_sha256": sha64('a'),
                "quote_sha256": quote_sha256(quote_txt),
                "quote": quote_txt,
                "excerpt": quote_txt,
                "source_seq": 2,
                "caller": "reader:s-x11-atomic:arxiv:2401.00002",
                "paper_key": "arxiv:2401.00002",
                "role": "states_result",
                "subclaim_ids": ["sc-1"],
                "gist": "Linear Lanczos growth in monitored Majorana chains."
            }),
        )
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Read.as_str(), "state": "finished" }))
        .unwrap();

    let xe_id = format!("xe-{xe_seq}");
    let transport = FixedMapTransport {
        payload: json!({
            "nodes": [
                {
                    "temp_id": "c1", "kind": "concept", "label": "Krylov complexity",
                    "statement": "Krylov spread complexity in monitored chains.",
                    "evidence": [], "idea_quote": "Krylov complexity",
                    "relation_to_idea": "same"
                },
                {
                    "temp_id": "k1", "kind": "known_result", "label": "Linear Lanczos slope",
                    "statement": "Lanczos coefficients grow linearly in monitored Majorana chains.",
                    "evidence": [xe_id], "relation_to_idea": "same", "subclaim_ids": ["sc-1"]
                }
            ],
            "edges": [
                { "from": "k1", "to": "c1", "kind": "uses", "basis": "evidence", "evidence": xe_id }
            ],
            "overlaps": [{
                "subclaim_id": "sc-1",
                "paper_key": "arxiv:2401.00002",
                "relation": "same",
                "evidence": [xe_id],
                "difference": "Matches monitored Majorana boundary conditions."
            }]
        }),
        seen_prompts: Mutex::new(Vec::new()),
    };

    let lock = Mutex::new(());
    let sse_kinds: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let res = run_map_with_transport(&ledger, &lock, sid, &transport, "mock-map", None, |k, _| {
        sse_kinds.lock().unwrap().push(k.to_string());
    })
    .unwrap();

    let (map_seq, verdict_seq, verdict) = match res {
        MapRunResult::Finished { map_seq, verdict_seq, verdict, .. } => (map_seq, verdict_seq, verdict),
        other => panic!("expected Finished, got {other:?}"),
    };
    assert_eq!(verdict_seq, map_seq + 1, "idea.verdict must be atomically chained right after idea.map");
    assert_eq!(verdict.verdicts[0].status, PriorWorkStatus::JudgedSame);
    assert!(!verdict.verdicts[0].confirmed_by_user);

    // Cryptographic hash-chain check: verdict.prev_hash == map.event_hash AND full session verifies.
    let report = ledger.verify_session(sid).unwrap();
    assert!(report.event_count >= verdict_seq, "ledger hash chain must verify cleanly: {:?}", report);
    let evs = ledger.read_session(sid).unwrap();
    let map_ev = evs.iter().find(|e| e.seq == map_seq && e.kind == EVENT_IDEA_MAP).unwrap();
    let verd_ev = evs.iter().find(|e| e.seq == verdict_seq && e.kind == EVENT_IDEA_VERDICT).unwrap();
    assert_eq!(verd_ev.prev_hash, map_ev.event_hash);
    assert!(idea_verdict::matches_recomputed(&verdict, &evs, None));

    // Rule C7 wrapping verified on prompt.
    let prompts = transport.seen_prompts.lock().unwrap();
    assert!(prompts[0].contains("<SESSION_EVIDENCE_") && prompts[0].contains("untrusted data"));

    // Verify `idea_view::view` emits rich GraphView + verdicts + Rule V9 `cited_by_origin = true`.
    let view = idea_view::view(&evs, false);
    assert_eq!(view["map"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(view["map"]["edges"][0]["src"], "c-1");
    assert_eq!(view["map"]["edges"][0]["dst"], "k-1");
    assert_eq!(view["map"]["edges"][0]["style"], "solid");
    assert_eq!(view["verdicts"]["verdicts"][0]["chip"], "Model judges: same");
    let cands = view["candidates"].as_array().unwrap();
    let cited_cand = cands.iter().find(|c| c["key"] == "arxiv:2401.00002").unwrap();
    assert_eq!(cited_cand["cited_by_origin"], true, "Rule V9: origin reference must be marked cited_by_origin");
    let orig_cand = cands.iter().find(|c| c["key"] == "arxiv:2001.00001").unwrap();
    assert_ne!(orig_cand.get("cited_by_origin").and_then(Value::as_bool), Some(true));

    // Rule V6: user clicks "Looks the same to me" (`idea.note confirm_same`) -> chip updates immediately!
    ledger
        .append(
            sid,
            EVENT_IDEA_NOTE,
            1,
            &json!({ "target_kind": "verdict", "target_id": "sc-1", "action": "confirm_same", "comment": "" }),
        )
        .unwrap();
    let evs_after = ledger.read_session(sid).unwrap();
    let view_after = idea_view::view(&evs_after, false);
    assert_eq!(view_after["verdicts"]["verdicts"][0]["confirmed_by_user"], true);
    assert_eq!(view_after["verdicts"]["verdicts"][0]["chip"], "found (same)");
}
