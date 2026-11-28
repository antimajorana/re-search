//! End-to-end HTTP integration tests for Explore Conversation turns (package X15):
//! `POST /api/sessions/{id}/messages` -> `turn_runner.rs` (`Mode::Explore`) -> `idea_chat.rs`
//! -> loopback mock Anthropic API -> `GET /api/sessions/{id}`.
//!
//! Zero live network/model calls, zero third-party paper text, zero skipped tests.

mod common;

use common::{raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_PAPER,
};
use desktop_server::idea_chat::{
    CHAT_CITE_STATUS_MATCHED, CHAT_CITE_STATUS_UNREAD, EVENT_IDEA_CHAT_CITATIONS,
};
use desktop_server::start_desktop_server;
use ledger::{sha256_hex, Ledger};
use research_tools::tool_profile::PROFILE_EXPLORE_V1;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn anthropic_reply(id: &str, stop_reason: &str, content: Value) -> MockReply {
    MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&json!({
            "id": id,
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": stop_reason,
            "content": content,
            "usage": { "input_tokens": 30, "output_tokens": 20 }
        }))
        .unwrap(),
    }
}

fn seed_paper(
    data_dir: &Path,
    ledger: &Ledger,
    sid: &str,
    source_id: &str,
    paper_key: &str,
    filename: &str,
    content: &str,
) {
    let rel = format!("{source_id}/{filename}");
    let full = data_dir
        .join("sessions")
        .join(sid)
        .join("sources")
        .join(source_id)
        .join(filename);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let raw_blob = ledger.put_blob(content.as_bytes()).unwrap();
    let src_seq = ledger
        .append(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": source_id,
                "kind": "tex",
                "primary_path": rel,
                "files": [rel],
                "files_sha256": { rel.clone(): sha.clone() },
                "file_sha256": sha,
                "raw_blob": raw_blob,
                "bytes": content.len(),
                "paper_key": paper_key,
            }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_PAPER,
            1,
            &json!({
                "key": paper_key,
                "source_id": source_id,
                "source_seq": src_seq,
                "origin": "library",
                "text_basis": "tex",
                "status": "read_passages",
                "total_lines": content.lines().count(),
            }),
        )
        .unwrap();
}

fn wait_for_citation_events(ledger: &Ledger, sid: &str, want: usize) -> Vec<ledger::Event> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let evs = ledger.read_session(sid).unwrap();
        let count = evs
            .iter()
            .filter(|e| e.kind == EVENT_IDEA_CHAT_CITATIONS)
            .count();
        if count >= want {
            return evs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {want} `{EVENT_IDEA_CHAT_CITATIONS}` event(s); saw {count}; kinds={:?}",
            evs.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn http_messages_dispatches_explore_turn_1_turn_2_without_investigation_hooks_and_guards_cli_budget() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();

    let sid = "s-x15-http-e2e";
    let ledger = Ledger::open(&data_dir).unwrap();
    let stmt = build_idea_statement(
        1,
        "Compare Krylov exponent bounds in monitored Majorana chains across two papers.",
        &["quant-ph".into()],
        &[],
    )
    .unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Quick, false, &stmt).unwrap();

    let mut p1 = String::from(concat!(
        "Line 1: Paper A introduction.\n",
        "Line 2: Theorem A1 derives linear Krylov growth rate alpha_A.\n",
        "Line 3: Volume-law scaling holds up to L = 512.\n",
    ));
    for i in 4..=24 {
        p1.push_str(&format!("Line {i}: Neutral spacing line for synthetic paper A.\n"));
    }
    p1.push_str("Line 25: Unread appendix remark in paper A.\n");

    let p2 = concat!(
        "Line 1: Paper B header.\n",
        "Line 2: Proposition B2 proves saturation plateau S_inf in area-law regime.\n",
        "Line 3: Plateau scales as 1/gamma for strong monitoring.\n",
        "Line 4: Unread closing note in paper B.\n",
    );
    seed_paper(&data_dir, &ledger, sid, "src-a", "arxiv:2503.00001", "a.tex", &p1);
    seed_paper(&data_dir, &ledger, sid, "src-b", "arxiv:2503.00002", "b.tex", p2);

    fs::write(
        data_dir.join("providers.json"),
        serde_json::to_vec(&json!({
            "providers": { "anthropic": { "api_key": "test-loopback-only-key" } }
        }))
        .unwrap(),
    )
    .unwrap();

    // Turn 1: call `search_paper_passages` on src-a + `read_paper_lines` on src-b (1..=3),
    // then emit answer citing 2 matched + 2 unread ranges. Turn 2: recap from earlier read.
    let mock = MockHttp::start(vec![
        anthropic_reply(
            "msg_x15_t1a",
            "tool_use",
            json!([
                {
                    "type": "tool_use",
                    "id": "tu_a",
                    "name": "search_paper_passages",
                    "input": { "query": "Krylov growth rate alpha_A", "source_id": "src-a", "limit": 2 }
                },
                {
                    "type": "tool_use",
                    "id": "tu_b",
                    "name": "read_paper_lines",
                    "input": { "source_id": "src-b", "start_line": 1, "end_line": 3 }
                }
            ]),
        ),
        anthropic_reply(
            "msg_x15_t1b",
            "end_turn",
            json!([{
                "type": "text",
                "text": "Paper A shows linear growth [arxiv:2503.00001: L2-3], Paper B shows saturation [src-b: L2-3], while [arxiv:2503.00002: L4] and [arxiv:2503.99999: L1-2] were not read."
            }]),
        ),
        anthropic_reply(
            "msg_x15_t2",
            "end_turn",
            json!([{
                "type": "text",
                "text": "Follow-up referencing earlier read [arxiv:2503.00002: L2-3]."
            }]),
        ),
    ]);

    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None))
        .unwrap();
    let port = srv.addr.port();
    let auth = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // Turn 1 over HTTP POST /api/sessions/{id}/messages.
    let body1 = serde_json::to_vec(&json!({
        "prompt": "Compare Paper A and Paper B.",
        "provider": "anthropic",
        "model": "claude-sonnet-4-5"
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &auth, &body1);
    assert_eq!(st, 202, "{}", String::from_utf8_lossy(&raw));

    let evs_t1 = wait_for_citation_events(&ledger, sid, 1);
    assert!(!evs_t1.iter().any(|e| e.kind.starts_with("investigation.")));
    let start_ev = evs_t1.iter().find(|e| e.kind == "session.start").unwrap();
    assert_eq!(start_ev.payload["tool_profile"], PROFILE_EXPLORE_V1);

    // Wait for active turn guard to release before Turn 2.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (st, _, s_raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &auth, b"");
        assert_eq!(st, 200);
        let s_val: Value = serde_json::from_slice(&s_raw).unwrap();
        assert_eq!(s_val["mode"], "explore");
        if s_val["running"] == false {
            let cites = s_val["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["kind"] == EVENT_IDEA_CHAT_CITATIONS)
                .unwrap();
            assert_eq!(cites["payload"]["matched_count"], 2);
            assert_eq!(cites["payload"]["unread_count"], 2);
            let list = cites["payload"]["citations"].as_array().unwrap();
            assert_eq!(list[0]["status"], CHAT_CITE_STATUS_MATCHED);
            assert_eq!(list[1]["status"], CHAT_CITE_STATUS_MATCHED);
            assert_eq!(list[2]["status"], CHAT_CITE_STATUS_UNREAD);
            assert_eq!(list[3]["status"], CHAT_CITE_STATUS_UNREAD);
            assert!(!serde_json::to_string(cites).unwrap().to_lowercase().contains("verified"));
            break;
        }
        assert!(Instant::now() < deadline, "turn 1 did not release active_turns");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Turn 2 over HTTP: fail-closed F1 resume under `explore/1`.
    let body2 = serde_json::to_vec(&json!({
        "prompt": "Recap Paper B.",
        "provider": "anthropic",
        "model": "claude-sonnet-4-5"
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &auth, &body2);
    assert_eq!(st, 202, "{}", String::from_utf8_lossy(&raw));

    let evs_t2 = wait_for_citation_events(&ledger, sid, 2);
    let resume_ev = evs_t2.iter().find(|e| e.kind == "session.resume").unwrap();
    assert_eq!(resume_ev.payload["tool_profile"], PROFILE_EXPLORE_V1);
    assert!(!evs_t2.iter().any(|e| e.kind.starts_with("investigation.")));

    // Wait for Turn 2 to finish before negative precondition checks.
    let deadline2 = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, _, s_raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &auth, b"");
        if serde_json::from_slice::<Value>(&s_raw).unwrap()["running"] == false {
            break;
        }
        assert!(Instant::now() < deadline2, "turn 2 did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
    mock.finish();

    // CLI routes (`claude-cli`, `antigravity`, mixed case, and omitted-provider ui.default_provider)
    // are refused synchronously with HTTP 400 until X18 (Carbon B2 regression).
    let len_before_cli = ledger.read_session(sid).unwrap().len();
    for cli_prov in ["claude-cli", "antigravity", "Claude-CLI"] {
        let cli_body = serde_json::to_vec(&json!({ "prompt": "hi", "provider": cli_prov })).unwrap();
        let (st, _, err_raw) =
            raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &auth, &cli_body);
        assert_eq!(st, 400, "CLI provider {cli_prov} must be refused");
        assert!(String::from_utf8_lossy(&err_raw).contains("X18"));
    }
    for (label, ui_json) in [
        ("default_provider=claude-cli", json!({ "default_provider": "claude-cli" })),
        ("orchestrator=antigravity", json!({ "agent_models": { "orchestrator": { "provider": "antigravity", "model": "gemini-3-pro" } } })),
    ] {
        fs::write(data_dir.join("ui_settings.json"), serde_json::to_vec(&ui_json).unwrap()).unwrap();
        let omitted_body = serde_json::to_vec(&json!({ "prompt": "hi without explicit provider" })).unwrap();
        let (st, _, err_raw) =
            raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &auth, &omitted_body);
        assert_eq!(st, 400, "omitted provider with {label} must return 400: {}", String::from_utf8_lossy(&err_raw));
        assert!(String::from_utf8_lossy(&err_raw).contains("X18"));
    }
    let _ = fs::remove_file(data_dir.join("ui_settings.json"));
    assert_eq!(ledger.read_session(sid).unwrap().len(), len_before_cli);

    // Exhaust Quick budget (`max_model_calls = 5`; 3 already spent -> add 2 exchanges) -> HTTP 400.
    for i in 0..2 {
        ledger
            .append(sid, "idea.exchange", 1, &json!({ "exchange": format!("cap-{i}") }))
            .unwrap();
    }
    let len_before_cap = ledger.read_session(sid).unwrap().len();
    let (st, _, cap_raw) =
        raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &auth, &body2);
    assert_eq!(st, 400, "exhausted budget must be refused");
    assert!(String::from_utf8_lossy(&cap_raw).contains("budget exhausted"));
    assert_eq!(ledger.read_session(sid).unwrap().len(), len_before_cap);
}
