//! End-to-end tests for Explore X10 parallel passage readers, follow-up `want_lines`,
//! `context.supplied`, Rule C7 wrapping, F10 `seen`/`finding` updates, and live SSE streaming.
//!
//! Zero live network/model calls, zero third-party paper text, zero skipped tests.

use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, Transport,
    TransportError, TransportInfo,
};
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, IdeaStage, EVENT_IDEA_BRIEF,
    EVENT_IDEA_CANDIDATES, EVENT_IDEA_PAPER, EVENT_IDEA_STAGE, EVENT_IDEA_TRIAGE,
};
use desktop_server::idea_read::ReadQueueResult;
use desktop_server::idea_read_passages::{
    run_reading_with_router_and_transport, DEFAULT_READER_CONCURRENCY,
};
use desktop_server::idea_view;
use ledger::{sha256_hex, Ledger};
use research_tools::idea_evidence::RECORDER_PAPER_EVIDENCE;
use research_tools::idea_tools::{idea_fetch_papers_handler_with_gate, register_explore_v1_tools};
use research_tools::library::{
    FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord, SeenRole, SourceKind,
    SourceRecord, ORIGIN_USER_LIBRARY,
};
use research_tools::lines_sent::CONTEXT_SUPPLIED_EVENT;
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::quote_core::EVIDENCE_QUOTE_EVENT;
use research_tools::{research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router};

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "x10-test".into(),
        }
    }
}

/// Deterministic thread-safe scripted transport keyed by paper key + call count per paper.
struct PaperScriptTransport {
    scripts: BTreeMap<String, Vec<Value>>,
    calls_by_paper: Mutex<BTreeMap<String, usize>>,
    seen_prompts: Mutex<Vec<String>>,
}

impl PaperScriptTransport {
    fn new(scripts: BTreeMap<String, Vec<Value>>) -> Self {
        Self {
            scripts,
            calls_by_paper: Mutex::new(BTreeMap::new()),
            seen_prompts: Mutex::new(Vec::new()),
        }
    }
}

impl Transport for PaperScriptTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted-x10",
            endpoint: "mock://x10".into(),
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
        self.seen_prompts.lock().unwrap().push(prompt.clone());
        let body = serde_json::to_vec(&json!({ "prompt": prompt })).unwrap();
        Ok(PreparedRequest { body })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let v: Value = serde_json::from_slice(&prepared.body).unwrap();
        let prompt = v["prompt"].as_str().unwrap_or("");
        let mut chosen_key = String::new();
        for k in self.scripts.keys() {
            if prompt.contains(k) {
                chosen_key = k.clone();
                break;
            }
        }
        let mut counts = self.calls_by_paper.lock().unwrap();
        let idx = {
            let entry = counts.entry(chosen_key.clone()).or_insert(0);
            let cur = *entry;
            *entry += 1;
            cur
        };
        let input = self
            .scripts
            .get(&chosen_key)
            .and_then(|list| list.get(idx).or_else(|| list.last()))
            .cloned()
            .unwrap_or_else(|| json!({ "items": [], "want_lines": [], "status": "nothing_relevant" }));
        let raw = serde_json::to_vec(&input).unwrap();
        Ok(ModelResponse {
            id: Some(format!("resp-{chosen_key}-{idx}")),
            model: "mock-reader".into(),
            content: vec![ContentBlock::ToolUse {
                id: format!("tu-{chosen_key}-{idx}"),
                name: RECORDER_PAPER_EVIDENCE.to_string(),
                input,
            }],
            stop_reason: StopReason::ToolUse,
            usage: json!({ "input_tokens": 120, "output_tokens": 45 }),
            raw_body: raw,
        })
    }
}

fn add_owned_tex_paper(
    data_dir: &std::path::Path,
    vault: &std::path::Path,
    key: &str,
    file_name: &str,
    body: &str,
) {
    let path = vault.join(file_name);
    std::fs::write(&path, body).unwrap();
    let lib = Library::open(data_dir).unwrap();
    let pkey = PaperKey::parse(key).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: pkey.clone(),
        ids: LibraryIds::default(),
        title: format!("Synthetic {key}"),
        authors: vec!["Ada Example".into()],
        year: Some(2025),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, ..Default::default() },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_user_library_fulltext(&FulltextRecord {
        key: pkey,
        version: None,
        raw_blob: None,
        format: "tex".into(),
        bytes: body.len() as u64,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 1,
        file_path: Some(path.to_string_lossy().to_string()),
        file_size: Some(body.len() as u64),
        file_mtime_ms: Some(1),
        file_sha256: Some(sha256_hex(body.as_bytes())),
    })
    .unwrap();
}

fn seed_session(data_dir: &std::path::Path, sid: &str, preset: IdeaPreset, keys: &[&str]) {
    let ledger = Ledger::open(data_dir).unwrap();
    let stmt = build_idea_statement(
        1,
        "Investigate Lanczos coefficients and Krylov complexity in monitored Majorana chains.",
        &[],
        &[],
    )
    .unwrap();
    append_explore_session_start(&ledger, sid, preset, false, &stmt).unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Lanczos growth in monitored Majorana chains.",
                "assumptions": ["free fermions"],
                "subclaims": [{ "id": "sc-1", "text": "b_n grows linearly", "idea_quote": "Lanczos coefficients" }],
                "terms": [{ "term": "Lanczos coefficients", "aliases": ["Krylov complexity", "Majorana"] }],
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "question": "Exact?", "queries": [{ "provider": "arxiv", "q": "Lanczos Majorana" }] },
                    { "id": "p-2", "kind": "other_names", "question": "Other?", "queries": [{ "provider": "inspire", "q": "Krylov complexity" }] }
                ],
                "questions": [],
                "exclusions": []
            }),
        )
        .unwrap();
    for st in [IdeaStage::Sharpen, IdeaStage::Search] {
        ledger
            .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": st.as_str(), "state": "finished" }))
            .unwrap();
    }
    let cands: Vec<Value> = keys
        .iter()
        .map(|k| json!({ "key": k, "title": format!("Title {k}"), "authors": ["Ada Example"], "year": 2025 }))
        .collect();
    ledger
        .append(sid, EVENT_IDEA_CANDIDATES, 1, &json!({ "round": 1, "candidates": cands }))
        .unwrap();
    let items: Vec<Value> = keys
        .iter()
        .map(|k| json!({ "key": k, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Core fixture." }))
        .collect();
    ledger
        .append(sid, EVENT_IDEA_TRIAGE, 1, &json!({ "items": items, "reading_list": keys }))
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Triage.as_str(), "state": "finished" }))
        .unwrap();
}

#[test]
fn x10_parallel_passage_readers_stream_quotes_followups_and_update_library() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&vault).unwrap();

    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_source(&SourceRecord {
        id: "src-x10".into(),
        kind: SourceKind::Folder,
        root: vault.to_string_lossy().to_string(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();

    // Paper 1: short file (read_full), 1 quote saved on first call.
    let key1 = "doi:10.5555/x10-short-1";
    let tex1 = "\\section{Result}\nTheorem 1: Lanczos coefficients grow linearly in chaotic Majorana chains.\n";
    add_owned_tex_paper(&data_dir, &vault, key1, "p1.tex", tex1);

    // Paper 2: >500 lines (read_passages), requests 1 follow-up window at lines 595..=605.
    let key2 = "doi:10.5555/x10-long-2";
    let mut long_lines = Vec::new();
    long_lines.push("\\section{Introduction}".to_string());
    long_lines.push("Overview of Majorana Lanczos growth.".to_string());
    for i in 3..=598 {
        long_lines.push(format!("Filler derivation step {i} in neutral prose."));
    }
    long_lines.push("\\section{Appendix Bound}".to_string());
    long_lines.push("Corollary 7: Krylov complexity obeys a strict logarithmic ceiling in area-law phases.".to_string());
    for i in 601..=640 {
        long_lines.push(format!("Trailing verification remark {i}."));
    }
    add_owned_tex_paper(&data_dir, &vault, key2, "p2.tex", &long_lines.join("\n"));

    // Paper 3: short file with nothing relevant.
    let key3 = "doi:10.5555/x10-irrel-3";
    let tex3 = "\\section{Notes}\nThis note discusses classical fluid turbulence only.\n";
    add_owned_tex_paper(&data_dir, &vault, key3, "p3.tex", tex3);

    let sid = "s-x10-parallel";
    seed_session(&data_dir, sid, IdeaPreset::Standard, &[key1, key2, key3]);

    let mut scripts = BTreeMap::new();
    scripts.insert(
        key1.to_string(),
        vec![json!({
            "items": [{
                "start_line": 2,
                "end_line": 2,
                "quote": "Theorem 1: Lanczos coefficients grow linearly in chaotic Majorana chains.",
                "role": "states_result",
                "gist": "Linear Lanczos growth in chaotic Majorana chains.",
                "subclaim_ids": ["sc-1"]
            }],
            "want_lines": [],
            "status": "done"
        })],
    );
    scripts.insert(
        key2.to_string(),
        vec![
            // First call requests follow-up lines 598..=602.
            json!({
                "items": [],
                "want_lines": [{ "start_line": 598, "end_line": 602 }],
                "status": "done"
            }),
            // Second call cites line 600 from the follow-up window!
            json!({
                "items": [{
                    "start_line": 600,
                    "end_line": 600,
                    "quote": "Corollary 7: Krylov complexity obeys a strict logarithmic ceiling in area-law phases.",
                    "role": "states_result",
                    "gist": "Logarithmic Krylov ceiling in area-law phases.",
                    "subclaim_ids": ["sc-1"]
                }],
                "want_lines": [],
                "status": "done"
            }),
        ],
    );
    scripts.insert(
        key3.to_string(),
        vec![json!({
            "items": [],
            "want_lines": [],
            "status": "nothing_relevant"
        })],
    );
    let transport = PaperScriptTransport::new(scripts);

    let env = Arc::new(ResearchEnv::new(&data_dir));
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let mut reg = Registry::new();
    register_explore_v1_tools(
        &mut reg,
        Arc::clone(&env),
        Some(idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(gate))),
    );
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(ApproveAll),
    );
    let ledger = Ledger::open(&data_dir).unwrap();
    let lock = Mutex::new(());
    let sse_events: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());

    let res = run_reading_with_router_and_transport(
        &env,
        &ledger,
        &lock,
        &router,
        sid,
        None,
        &transport,
        "mock-reader",
        DEFAULT_READER_CONCURRENCY,
        None,
        |kind, payload| {
            sse_events.lock().unwrap().push((kind.to_string(), payload));
        },
    )
    .unwrap();

    let papers = match res {
        ReadQueueResult::Finished { papers, .. } => papers,
        other => panic!("expected Finished, got {other:?}"),
    };
    assert_eq!(papers.len(), 3);
    assert_eq!(papers[0].status, "read_full");
    assert_eq!(papers[0].lines_sent, 2);
    assert_eq!(papers[1].status, "read_passages");
    assert!(papers[1].lines_sent > 0 && papers[1].lines_sent < papers[1].total_lines);
    assert_eq!(papers[2].status, "nothing_relevant");

    // Verify Rule C7 nonce delimiter was applied to every prompt.
    let prompts = transport.seen_prompts.lock().unwrap();
    assert_eq!(prompts.len(), 4, "1 call for p1, 2 calls for p2 (initial + followup), 1 for p3");
    assert!(prompts.iter().all(|p| p.contains("<PAPER_TEXT_") && p.contains("untrusted data")));

    // Verify ledger events: context.supplied (>= 4: initial for 3 papers + followup for p2), evidence.quote == 2.
    let evs = ledger.read_session(sid).unwrap();
    assert!(evs.iter().filter(|e| e.kind == CONTEXT_SUPPLIED_EVENT).count() >= 4);
    assert_eq!(evs.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT).count(), 2);
    assert!(evs.iter().any(|e| e.kind == EVENT_IDEA_PAPER && e.payload["status"] == "read_full"));
    assert!(evs.iter().any(|e| e.kind == EVENT_IDEA_PAPER && e.payload["status"] == "read_passages"));
    assert!(evs.iter().any(|e| e.kind == EVENT_IDEA_PAPER && e.payload["status"] == "nothing_relevant"));

    // Verify live SSE streamed both evidence.quote events.
    let sse = sse_events.lock().unwrap();
    let sse_quotes: Vec<_> = sse.iter().filter(|(k, _)| k == EVIDENCE_QUOTE_EVENT).collect();
    assert_eq!(sse_quotes.len(), 2);
    assert!(sse_quotes.iter().all(|(_, v)| v["id"].as_str().is_some_and(|s| s.starts_with("xe-"))));

    // Verify F10 library updated with SeenRole::Read and FindingRecord rows.
    let detail1 = lib.get_paper(&PaperKey::parse(key1).unwrap()).unwrap().unwrap();
    assert!(detail1.seen.iter().any(|s| s.role == SeenRole::Read && s.session_id == sid));
    assert_eq!(detail1.findings.len(), 1);
    let detail2 = lib.get_paper(&PaperKey::parse(key2).unwrap()).unwrap().unwrap();
    assert!(detail2.seen.iter().any(|s| s.role == SeenRole::Read && s.session_id == sid));
    assert_eq!(detail2.findings.len(), 1);

    // Verify idea_view projection exposes both quotes and terminal paper statuses.
    let view = idea_view::view(&evs, false);
    assert_eq!(view["evidence"].as_array().unwrap().len(), 2);
    assert_eq!(view["next_stage"], "map");
}

#[test]
fn x10_thorough_allows_two_followups_and_stops_at_model_call_cap() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&vault).unwrap();

    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_source(&SourceRecord {
        id: "src-x10-th".into(),
        kind: SourceKind::Folder,
        root: vault.to_string_lossy().to_string(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();

    let key = "doi:10.5555/x10-thorough";
    let mut lines = vec!["\\section{Intro}".to_string(), "Lanczos overview.".to_string()];
    for i in 3..=620 {
        if i == 560 {
            lines.push("Lemma 5: Intermediate Majorana spectral bound holds for all L.".into());
        } else if i == 610 {
            lines.push("Theorem 9: Asymptotic Krylov spread slope equals pi over two.".into());
        } else {
            lines.push(format!("Neutral derivation line {i}."));
        }
    }
    add_owned_tex_paper(&data_dir, &vault, key, "thorough.tex", &lines.join("\n"));

    let sid = "s-x10-thorough";
    seed_session(&data_dir, sid, IdeaPreset::Thorough, &[key]);

    let mut scripts = BTreeMap::new();
    scripts.insert(
        key.to_string(),
        vec![
            // Call 1: requests follow-up #1 (558..=562).
            json!({ "items": [], "want_lines": [{ "start_line": 558, "end_line": 562 }], "status": "done" }),
            // Call 2: cites line 560 AND requests follow-up #2 (608..=612).
            json!({
                "items": [{
                    "start_line": 560, "end_line": 560,
                    "quote": "Lemma 5: Intermediate Majorana spectral bound holds for all L.",
                    "role": "states_result", "gist": "Intermediate spectral bound.", "subclaim_ids": ["sc-1"]
                }],
                "want_lines": [{ "start_line": 608, "end_line": 612 }],
                "status": "done"
            }),
            // Call 3 (second follow-up): cites line 610 and finishes.
            json!({
                "items": [{
                    "start_line": 610, "end_line": 610,
                    "quote": "Theorem 9: Asymptotic Krylov spread slope equals pi over two.",
                    "role": "states_result", "gist": "Asymptotic Krylov slope.", "subclaim_ids": ["sc-1"]
                }],
                "want_lines": [],
                "status": "done"
            }),
        ],
    );
    let transport = PaperScriptTransport::new(scripts);

    let env = Arc::new(ResearchEnv::new(&data_dir));
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let mut reg = Registry::new();
    register_explore_v1_tools(
        &mut reg,
        Arc::clone(&env),
        Some(idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(gate))),
    );
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(ApproveAll),
    );
    let ledger = Ledger::open(&data_dir).unwrap();
    let lock = Mutex::new(());

    let res = run_reading_with_router_and_transport(
        &env, &ledger, &lock, &router, sid, None, &transport, "mock-reader", 2, None, |_, _| {},
    )
    .unwrap();
    assert!(matches!(res, ReadQueueResult::Finished { .. }));
    assert_eq!(transport.seen_prompts.lock().unwrap().len(), 3, "Thorough allows 2 follow-ups (3 total calls)");
    let evs = ledger.read_session(sid).unwrap();
    assert_eq!(evs.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT).count(), 2);

    // Now test session model-call cap enforcement (Rule P3): pre-spend all 24 Standard calls -> StoppedAtCap.
    let sid_cap = "s-x10-cap";
    seed_session(&data_dir, sid_cap, IdeaPreset::Standard, &[key]);
    for i in 1..=24 {
        ledger
            .append(
                sid_cap,
                "idea.exchange",
                1,
                &json!({ "exchange": "prior", "attempt": i, "model": "mock", "accepted": true, "faults": [] }),
            )
            .unwrap();
    }
    let mut reg2 = Registry::new();
    let gate2 = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    register_explore_v1_tools(
        &mut reg2,
        Arc::clone(&env),
        Some(idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(gate2))),
    );
    let router2 = Router::new(
        reg2, research_policy(), Ledger::open(&data_dir).unwrap(), sid_cap, Arc::new(ApproveAll),
    );
    let cap_transport = PaperScriptTransport::new(BTreeMap::new());
    let cap_res = run_reading_with_router_and_transport(
        &env, &ledger, &lock, &router2, sid_cap, None, &cap_transport, "mock-reader", 1, None, |_, _| {},
    )
    .unwrap();
    assert!(matches!(cap_res, ReadQueueResult::StoppedAtCap { .. }));
    assert_eq!(cap_transport.seen_prompts.lock().unwrap().len(), 0, "Rule P3: zero provider calls after cap");
}
