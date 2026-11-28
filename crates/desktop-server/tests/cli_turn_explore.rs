//! Integration tests for X18 Checkpoint 2: Explore `explore/1` CLI Conversation turns,
//! prompt gating, core env propagation, and F4 multi-paper line-citation checking.

mod common;

use cli_route::core_tools::core_tool_ids_for;
use common::test_cfg;
use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::cli_turn_explore::{
    build_explore_cli_prompt, check_explore_cli_citations, parse_multi_paper_citations,
    record_explore_cli_citations, resolve_explore_v1_profile, EVENT_IDEA_CHAT_CITATIONS,
};
use desktop_server::cli_turn_setup::{build_cli_prompt, core_env_for_profile};
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_BRIEF,
    EVENT_IDEA_MAP, EVENT_IDEA_PAPER,
};
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::lines_sent::{
    ContextSuppliedPayload, SuppliedRange, CONTEXT_SUPPLIED_EVENT,
};
use research_tools::paper_key::Mode;
use research_tools::tool_profile::{explore_v1, PROFILE_EXPLORE_V1};
use research_tools::ResearchEnv;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[test]
fn explore_core_env_propagates_explore_v1_excludes_research_and_python_and_matches_tool_ids() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = test_cfg(tmp.path(), None, None, None);
    cfg.python = Some(PathBuf::from("/fake/python"));
    cfg.worker_script = Some(PathBuf::from("/fake/worker.py"));

    let env = core_env_for_profile(
        &cfg,
        "s-exp-env",
        "tok",
        "127.0.0.1:9",
        &tmp.path().join("scratch"),
        &tmp.path().join("pycache"),
        None,
        Mode::Explore,
        PROFILE_EXPLORE_V1,
        None,
    );
    assert_eq!(env.get("CORE_TOOL_PROFILE").map(String::as_str), Some(PROFILE_EXPLORE_V1));
    assert!(!env.contains_key("CORE_ENABLE_RESEARCH_TOOLS"), "Explore must not set research flag");
    assert!(!env.contains_key("CORE_PYTHON"), "Explore must not launch compute worker");
    assert!(!env.contains_key("CORE_WORKER_SCRIPT"));

    let ids = core_tool_ids_for(&env);
    let expected: Vec<String> = explore_v1().mcp_defs().into_iter().map(|d| d.id).collect();
    assert_eq!(ids, expected);
    assert_eq!(ids.len(), 5);
    for forbidden in ["sympy_eval", "fs_write", "lean_check", "search_formal_library"] {
        assert!(!ids.iter().any(|id| id == forbidden), "explore/1 leaked {forbidden}");
    }
}

#[test]
fn explore_cli_prompt_injects_node_and_lessons_without_research_or_verified_wording() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-exp-prompt";
    let stmt = build_idea_statement(1, "Krylov complexity in monitored Majorana chains.", &[], &[]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Standard, true, &stmt).unwrap();

    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Check Lanczos growth across the entanglement transition.",
                "subclaims": [{ "id": "sc-1", "text": "b_n grows linearly in volume-law phase" }]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_MAP,
            1,
            &json!({
                "nodes": [
                    { "id": "k-1", "kind": "known_result", "label": "Linear growth", "statement": "Linear b_n in free chains.", "evidence": ["xe-10"] },
                    { "id": "k-2", "kind": "known_result", "label": "Other node", "statement": "Unselected node.", "evidence": ["xe-11"] }
                ]
            }),
        )
        .unwrap();
    let lesson_blob = ledger.put_blob(b"Remember: distinguish projective vs continuous monitoring.").unwrap();
    ledger
        .append_with_hash(sid, "lessons.supplied", 1, &json!({ "run_id": "r-1", "lesson_ids": ["l-1"], "blob": lesson_blob }))
        .unwrap();

    let events = ledger.read_session(sid).unwrap();
    assert_eq!(resolve_explore_v1_profile(&events).unwrap().name, PROFILE_EXPLORE_V1);

    let general = build_cli_prompt(
        Mode::Explore,
        &events,
        Some(&ledger),
        "Explain k-1",
        None,
        None,
        false,
        true,
    );
    assert!(general.prompt.contains("explore/1"));
    assert!(general.prompt.contains("idea_search"));
    assert!(general.prompt.contains("read_paper_lines"));
    assert!(general.prompt.contains("distinguish projective vs continuous monitoring"));
    assert!(general.prompt.contains("EXPLORE_CONTEXT_"));
    for forbidden in [
        "search_formal_library",
        "sympy_eval",
        "check_symbolic_identity",
        "lean_check",
        "verified",
    ] {
        assert!(
            !general.prompt.to_ascii_lowercase().contains(forbidden),
            "Explore CLI prompt leaked `{forbidden}`:\n{}",
            general.prompt
        );
    }

    let focused = build_explore_cli_prompt(
        &events,
        Some(&ledger),
        "Focus on k-1",
        None,
        None,
        Some("k-1"),
    );
    assert!(focused.prompt.contains("Linear growth"));
    assert!(!focused.prompt.contains("Unselected node."));
}

#[test]
fn multi_paper_line_citation_check_credits_only_caller_read_lines_via_f4() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let sid = "s-exp-cites";

    let env = ResearchEnv::new(&data);
    let ledger = Ledger::open(&data).unwrap();
    let stmt = build_idea_statement(1, "Multi-paper citation check test.", &[], &[]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Standard, true, &stmt).unwrap();

    let ledger_mux = Mutex::new(Ledger::open(&data).unwrap());
    let rec_a = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "paper_a.tex",
            origin: "upload",
            raw_bytes: b"line1 A\nline2 A\nline3 A\nline4 A\nline5 A\n",
            source_id_hint: Some("src-a"),
            section_filter: None,
            title: Some("Paper A"),
        },
    )
    .unwrap();
    let rec_b = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "paper_b.tex",
            origin: "upload",
            raw_bytes: b"line1 B\nline2 B\nline3 B\n",
            source_id_hint: Some("src-b"),
            section_filter: None,
            title: Some("Paper B"),
        },
    )
    .unwrap();
    drop(ledger_mux);

    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_PAPER,
            1,
            &json!({ "key": "arxiv:2501.01234", "source_id": rec_a.source_id, "origin": "upload", "text_basis": "tex", "status": "read_passages", "lines_sent": 2, "total_lines": 5 }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_PAPER,
            1,
            &json!({ "key": "arxiv:2502.09999", "source_id": rec_b.source_id, "origin": "upload", "text_basis": "tex", "status": "fetched", "lines_sent": 0, "total_lines": 3 }),
        )
        .unwrap();

    let events_pre = ledger.read_session(sid).unwrap();
    let path_a = rec_a.primary_path.as_deref().unwrap();
    let sha_a = events_pre
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == rec_a.source_id)
        .unwrap()
        .payload["files_sha256"][path_a]
        .as_str()
        .unwrap()
        .to_string();

    let turn_start_seq = events_pre.last().unwrap().seq;
    let cli_caller = format!("claude-code-cli:{sid}");

    // Credit lines 4-5 ONLY to X10 reader caller (must NOT leak to CLI conversation caller!).
    let reader_ctx = ContextSuppliedPayload::new(
        "rd-1",
        format!("reader:{sid}:arxiv:2501.01234"),
        vec![SuppliedRange {
            source_id: rec_a.source_id.clone(),
            path: path_a.to_string(),
            file_sha256: sha_a.clone(),
            start_line: 4,
            end_line: 5,
        }],
    )
    .unwrap();
    ledger
        .append_with_hash(sid, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(&reader_ctx).unwrap())
        .unwrap();

    // Credit lines 1-2 of Paper A to the CLI conversation caller via tool.request + tool.result (`read_paper_lines`).
    ledger
        .append_with_hash(
            sid,
            "tool.request",
            1,
            &json!({
                "call_id": "c-read-1",
                "tool": "read_paper_lines",
                "caller": cli_caller,
                "args": { "inline": json!({ "source_id": rec_a.source_id, "path": path_a, "start_line": 1, "end_line": 2 }).to_string() }
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "tool.result",
            1,
            &json!({
                "call_id": "c-read-1",
                "ok": true,
                "result": {
                    "inline": json!({
                        "a_meta": {
                            "source_id": rec_a.source_id,
                            "path": path_a,
                            "file_sha256": sha_a,
                            "start_line": 1,
                            "end_line": 2,
                            "total_lines": 5
                        },
                        "lines": ["1: line1 A", "2: line2 A"]
                    }).to_string()
                }
            }),
        )
        .unwrap();

    let reply = "Paper A states the result [arxiv:2501.01234: L1-2], also see [arxiv:2501.01234: L4-5], \
                 wrong paper [arxiv:2502.09999: L1-2], unknown [arxiv:9999.00001: L1], and ambiguous [L1-2].";
    assert_eq!(parse_multi_paper_citations(reply).len(), 5);

    let events_all = ledger.read_session(sid).unwrap();
    let cites = check_explore_cli_citations(Some(&ledger), &events_all, turn_start_seq, &cli_caller, reply);
    assert_eq!(cites.len(), 5);

    assert!(cites[0].read && cites[0].status == "matched" && cites[0].paper_key.as_deref() == Some("arxiv:2501.01234"));
    assert!(!cites[1].read && cites[1].status == "unread", "X10 reader lines must not credit CLI caller");
    assert!(!cites[2].read && cites[2].status == "unread", "Paper A lines must not credit Paper B");
    assert!(!cites[3].read && cites[3].status == "unread", "unknown paper must be unread");
    assert!(!cites[4].read && cites[4].status == "unread", "bare [L1-2] across 2 papers must be ambiguous/unread");

    // Persist via `record_explore_cli_citations` and confirm Rule S8 (zero "verified" wording).
    let cfg = test_cfg(&data, None, None, None);
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(cfg, AuthGuard::new(1).unwrap(), Arc::new(SseHub::new()), chatgpt);
    let recorded = record_explore_cli_citations(&state, sid, turn_start_seq, &cli_caller, reply).unwrap();
    assert_eq!(recorded.len(), 5);

    let post_evs = ledger.read_session(sid).unwrap();
    let cite_ev = post_evs.iter().find(|e| e.kind == EVENT_IDEA_CHAT_CITATIONS).unwrap();
    assert_eq!(cite_ev.payload["matched_count"], 1);
    assert_eq!(cite_ev.payload["unread_count"], 4);
    assert!(!cite_ev.payload.to_string().to_ascii_lowercase().contains("verified"));
}

#[test]
fn explore_cli_handles_tex_assembled_vs_primary_midturn_fetch_staleness_and_c7_boundaries() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-exp-audit";

    let stmt1 = build_idea_statement(1, "Original rev 1 statement.", &[], &[]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Standard, true, &stmt1).unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({ "idea_rev": 1, "restatement": "STALE_REV1_BRIEF", "subclaims": [] }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_MAP,
            1,
            &json!({ "nodes": [{ "id": "k-old", "kind": "known_result", "label": "STALE_REV1_NODE", "statement": "Old", "evidence": [] }] }),
        )
        .unwrap();
    // Revise statement to rev 2: stale rev 1 brief/map must NOT appear in prompt!
    let stmt2 = build_idea_statement(2, "Revised rev 2 statement.", &[], &[]).unwrap();
    ledger
        .append_with_hash(sid, "idea.stated", 1, &serde_json::to_value(&stmt2).unwrap())
        .unwrap();

    // Synthetic multi-file TeX bundle where assembled_path != primary_path, fetched mid-turn via
    // `idea_fetch_papers` (blob-backed `tool.result`, NO `idea.paper` event).
    let main_sha = "a".repeat(64);
    let app_sha = "b".repeat(64);
    let asm_sha = "c".repeat(64);
    ledger
        .append_with_hash(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": "src-tex-bundle",
                "primary_path": "src-tex-bundle/main.tex",
                "assembled_path": "src-tex-bundle/__assembled__.tex",
                "files_sha256": {
                    "src-tex-bundle/main.tex": main_sha,
                    "src-tex-bundle/app.tex": app_sha,
                    "src-tex-bundle/__assembled__.tex": asm_sha
                }
            }),
        )
        .unwrap();
    let fetch_blob = ledger
        .put_blob(
            &serde_json::to_vec(&json!({
                "papers": [{ "key": "doi:10.1000/test-bundle", "source_id": "src-tex-bundle", "status": "fetched" }]
            }))
            .unwrap(),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "tool.result",
            1,
            &json!({ "call_id": "c-fetch", "ok": true, "result": { "blob": fetch_blob } }),
        )
        .unwrap();

    let evs_pre = ledger.read_session(sid).unwrap();
    let turn_start_seq = evs_pre.last().unwrap().seq;

    // Prompt must:
    // 1. Hide stale rev 1 brief/map.
    // 2. Strip Research `project_context` (`check_convergence`).
    // 3. Wrap `conventions` INSIDE `EXPLORE_CONTEXT` (after `<<<EXPLORE_CONTEXT_`).
    // 4. List `primary_path` (NEVER `__assembled__.tex`).
    let built = build_explore_cli_prompt(
        &evs_pre,
        Some(&ledger),
        "Check appendix equation",
        Some("Use natural units hbar=1."),
        Some("Pass frozen tolerances to check_convergence as decimal strings."),
        None,
    );
    assert!(!built.prompt.contains("STALE_REV1_BRIEF"));
    assert!(!built.prompt.contains("STALE_REV1_NODE"));
    assert!(!built.prompt.contains("check_convergence"), "leaked Research project_context");
    assert!(built.prompt.contains("src-tex-bundle/main.tex"));
    assert!(!built.prompt.contains("__assembled__.tex"), "prompt must never advertise assembled_path");
    let ctx_open = built.prompt.find("EXPLORE_CONTEXT_").unwrap();
    let conv_pos = built.prompt.find("Use natural units hbar=1.").unwrap();
    assert!(conv_pos > ctx_open, "conventions must be inside Rule C7 untrusted-data wrapper");

    // Credit lines 1-3 of `main.tex` AND lines 10-12 of secondary `app.tex` to caller.
    let caller = format!("antigravity:{sid}");
    let read_main = json!({
        "a_meta": { "source_id": "src-tex-bundle", "path": "src-tex-bundle/main.tex", "file_sha256": main_sha, "start_line": 1, "end_line": 3, "total_lines": 20 },
        "text": "1\tL1\n2\tL2\n3\tL3"
    });
    let read_app = json!({
        "a_meta": { "source_id": "src-tex-bundle", "path": "src-tex-bundle/app.tex", "file_sha256": app_sha, "start_line": 10, "end_line": 12, "total_lines": 30 },
        "text": "10\tA10\n11\tA11\n12\tA12"
    });
    for (cid, payload) in [("c-main", read_main), ("c-app", read_app)] {
        ledger
            .append_with_hash(sid, "tool.request", 1, &json!({ "call_id": cid, "tool": "read_paper_lines", "caller": caller }))
            .unwrap();
        ledger
            .append_with_hash(sid, "tool.result", 1, &json!({ "call_id": cid, "ok": true, "result": { "inline": payload.to_string() } }))
            .unwrap();
    }

    // Test colon-in-DOI + `L1–L3` + secondary file + single-paper bare `[L1-2]` + invalid ranges (`L0`, `L5-2`).
    let reply = "Main [doi:10.1000/test-bundle: L1–L3], appendix [doi:10.1000/test-bundle L10-L12], \
                 bare [L1-2], zero [doi:10.1000/test-bundle: L0], inverted [doi:10.1000/test-bundle: L5-2].";
    let evs_all = ledger.read_session(sid).unwrap();
    let cites = check_explore_cli_citations(Some(&ledger), &evs_all, turn_start_seq, &caller, reply);
    assert_eq!(cites.len(), 5, "{cites:?}");
    assert!(cites[0].read && cites[0].path.as_deref() == Some("src-tex-bundle/main.tex"));
    assert!(cites[1].read && cites[1].path.as_deref() == Some("src-tex-bundle/app.tex"));
    assert!(cites[2].read && cites[2].paper_key.as_deref() == Some("doi:10.1000/test-bundle"));
    assert!(!cites[3].read && cites[3].reason.as_deref().unwrap_or("").contains("invalid line range"));
    assert!(!cites[4].read && cites[4].reason.as_deref().unwrap_or("").contains("invalid line range"));
}
