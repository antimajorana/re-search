//! Regression tests for Package X18 Checkpoint 1 Carbon audit fixes:
//! - Antigravity `mcp(*)` deny rule in `exchange_agy_settings()`
//! - Antigravity `vendor.builtin_tool` Rule P2 (completed non-denied, unresolved active-only,
//!   and cleanly denied non-false-positive)
//! - Claude multi-block assistant text accumulation + bad `init` live subprocess abort
//! - `extract_json_object` edge cases (4-backtick fences, unclosed fences, no stray prose fallback)
//! - Bounded 4 MiB stdout ceiling on a single line without `\n`, cooperative cancellation,
//!   `--version` caching, parallel concurrent passage readers, and `API -> CLI` 409 mixing.

mod common;
mod idea_cli_support;

use agent::{CancelToken, ModelRequest, Role, StreamObserver, ToolSpec, Transport, TransportErrorKind};
use common::test_cfg;
use desktop_server::cli_turn_setup::CliRouteKind;
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_BRIEF,
    EVENT_IDEA_EXCHANGE, EVENT_IDEA_STAGE,
};
use desktop_server::idea_cli_exchange::{
    exchange_agy_settings, extract_json_object, CliExchangeTransport,
};
use desktop_server::idea_exchanges::{
    resolve_explore_transport, run_sharpen_with_transport, SharpenRunResult,
};
use desktop_server::idea_read::IdeaPaperEventPayload;
use desktop_server::idea_read_passages::run_reading_passages_with_transport;
use desktop_server::state::DesktopState;
use idea_cli_support::{claude_exchange_lines, fenced_json, MultiAttemptCli};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::ResearchEnv;
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct CancelObserver(CancelToken);
impl StreamObserver for CancelObserver {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

fn minimal_request() -> ModelRequest {
    ModelRequest {
        model: "claude-sonnet-4-6".into(),
        system: Some("Test exchange".into()),
        messages: vec![agent::Message {
            role: Role::User,
            content: vec![agent::ContentBlock::Text {
                text: "Emit JSON".into(),
            }],
        }],
        tools: vec![ToolSpec {
            name: "record_test".into(),
            description: "Record test".into(),
            input_schema: json!({"type": "object", "properties": {"ok": {"type": "boolean"}}}),
        }],
        max_tokens: 512,
        reasoning_effort: None,
    }
}

#[test]
fn agy_settings_denies_mcp_and_extract_json_handles_fences_strictly() {
    let settings = exchange_agy_settings();
    let deny: Vec<&str> = settings["permissions"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        deny.contains(&"mcp(*)"),
        "plain Antigravity exchange must deny mcp(*) too: {deny:?}"
    );
    assert_eq!(settings["permissions"]["allow"], json!([]));

    // 4-backtick outer fence containing triple backticks inside text/prose.
    let four_tick = "````json\n{\"note\": \"use ```code``` here\", \"ok\": true}\n````";
    assert_eq!(extract_json_object(four_tick).unwrap()["ok"], json!(true));

    // Unclosed ```json fence at EOF still extracts cleanly.
    let unclosed = "Here is the object:\n```json\n{\"ok\": true}";
    assert_eq!(extract_json_object(unclosed).unwrap()["ok"], json!(true));

    // Malformed ```json block must NOT silently fall back to a stray `{...}` in prose!
    let broken_with_prose = "Earlier I noticed {\"stray\": 1}.\n```json\n{\"ok\": }\n```";
    let err = extract_json_object(broken_with_prose).unwrap_err();
    assert!(
        err.contains("invalid JSON"),
        "expected JSON syntax error from fenced block, not prose fallback: {err}"
    );
}

#[test]
fn agy_builtin_tool_rule_p2_catches_completed_and_unresolved_active_but_allows_denied() {
    let tmp = tempfile::TempDir::new().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-x18-agy-p2";
    let stmt = build_idea_statement(
        1,
        "Synthetic gauge duality produces an effective Chern-Simons coupling.",
        &["hep-th".into()],
        &[],
    )
    .unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Standard, true, &stmt).unwrap();

    let valid_brief = json!({
        "restatement": "Gauge duality induces an effective Chern-Simons coupling.",
        "subclaims": [{
            "text": "Effective Chern-Simons coupling arises from gauge duality.",
            "idea_quote": "effective Chern-Simons coupling"
        }],
        "terms": [{ "term": "Chern-Simons", "aliases": ["topological mass"] }],
        "assumptions": ["3D effective field theory"],
        "perspectives": [
            {
                "kind": "exact",
                "question": "Direct Chern-Simons gauge duality literature",
                "queries": [{ "provider": "arxiv", "q": "Chern-Simons gauge duality" }]
            },
            {
                "kind": "other_names",
                "question": "Topological mass term duality",
                "queries": [{ "provider": "inspire", "q": "topological mass term duality" }]
            }
        ],
        "questions": [],
        "exclusions": []
    });

    let init_line = json!({
        "event": "init",
        "conversation_id": "agy-p2",
        "init": { "permission_mode": "request-review", "tools": ["run_command"], "model": "gemini-3.8-flash-medium", "cwd": "." }
    })
    .to_string();
    let ok_resp_line = json!({
        "event": "step_update",
        "step_update": { "step_index": 2, "step_type": "agent_response", "state": "DONE", "text_delta": fenced_json(&valid_brief) }
    })
    .to_string();
    let result_line = json!({
        "event": "result",
        "result": { "status": "SUCCESS", "response": fenced_json(&valid_brief), "num_turns": 1 }
    })
    .to_string();

    // 1. Non-denied DONE built-in tool -> Rule P2 UnexpectedToolCall (and kills CLI immediately).
    let cli_done = MultiAttemptCli::new(tmp.path(), "agy_done", "1.5.14");
    cli_done.set_attempt(
        1,
        &[
            init_line.clone(),
            json!({
                "event": "step_update",
                "step_update": {
                    "step_index": 1,
                    "step_type": "tool",
                    "tool_name": "run_command",
                    "state": "DONE",
                    "tool_info": { "name": "run_command", "parameters": { "command": "ls" } }
                }
            })
            .to_string(),
            ok_resp_line.clone(),
            result_line.clone(),
        ],
    );
    let tr_done = CliExchangeTransport::new(CliRouteKind::Antigravity, cli_done.exe.clone(), work.clone(), false);
    let err = run_sharpen_with_transport(&ledger, sid, &tr_done, "gemini-3.8-flash-medium", None, |_, _| {})
        .unwrap_err();
    assert!(err.contains("unexpected tool call") && err.contains("run_command"), "{err}");

    // 2. Unresolved ACTIVE-only built-in tool (no denial before stream end) -> Rule P2.
    let cli_active = MultiAttemptCli::new(tmp.path(), "agy_active", "1.5.14");
    cli_active.set_attempt(
        1,
        &[
            init_line.clone(),
            json!({
                "event": "step_update",
                "step_update": {
                    "step_index": 1,
                    "step_type": "tool",
                    "tool_name": "call_mcp_tool",
                    "state": "ACTIVE",
                    "tool_info": { "name": "call_mcp_tool", "parameters": {} }
                }
            })
            .to_string(),
            ok_resp_line.clone(),
            result_line.clone(),
        ],
    );
    let tr_active = CliExchangeTransport::new(CliRouteKind::Antigravity, cli_active.exe.clone(), work.clone(), false);
    let err = run_sharpen_with_transport(&ledger, sid, &tr_active, "gemini-3.8-flash-medium", None, |_, _| {})
        .unwrap_err();
    assert!(err.contains("unexpected tool call") && err.contains("call_mcp_tool"), "{err}");

    // 3. Soft-denied built-in tool (ACTIVE then FAILED with error) followed by valid JSON -> succeeds.
    let cli_denied = MultiAttemptCli::new(tmp.path(), "agy_denied", "1.5.14");
    cli_denied.set_attempt(
        1,
        &[
            init_line,
            json!({
                "event": "step_update",
                "step_update": {
                    "step_index": 1,
                    "step_type": "tool",
                    "tool_name": "run_command",
                    "state": "ACTIVE",
                    "tool_info": { "name": "run_command", "parameters": { "command": "ls" } }
                }
            })
            .to_string(),
            json!({
                "event": "step_update",
                "step_update": {
                    "step_index": 1,
                    "step_type": "tool",
                    "tool_name": "run_command",
                    "state": "FAILED",
                    "tool_info": { "name": "run_command", "error": { "message": "denied by policy" } }
                }
            })
            .to_string(),
            ok_resp_line,
            result_line,
        ],
    );
    let tr_denied = CliExchangeTransport::new(CliRouteKind::Antigravity, cli_denied.exe.clone(), work, false);
    let res = run_sharpen_with_transport(&ledger, sid, &tr_denied, "gemini-3.8-flash-medium", None, |_, _| {})
        .unwrap();
    assert!(matches!(res, SharpenRunResult::Finished { attempts: 1, .. }));
}

#[test]
fn claude_multiblock_text_bad_init_cancel_and_4mib_ceiling() {
    let tmp = tempfile::TempDir::new().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let req = minimal_request();

    // 1. Multi-block assistant text: block 1 has JSON, block 2 has trailing prose -> JSON preserved.
    let cli_multi = MultiAttemptCli::new(tmp.path(), "claude_multi", "2.1.318 (Claude Code)");
    cli_multi.set_attempt(
        1,
        &[
            json!({"type": "system", "subtype": "init", "tools": [], "mcp_servers": [], "model": "claude-sonnet-4-6", "apiKeySource": "none"}).to_string(),
            json!({"type": "assistant", "message": { "content": [
                { "type": "text", "text": "```json\n{\"ok\": true}\n```" },
                { "type": "text", "text": "Hope that helps." }
            ]}}).to_string(),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "Hope that helps."}).to_string(),
        ],
    );
    let tr_multi = CliExchangeTransport::new(CliRouteKind::Claude, cli_multi.exe.clone(), work.clone(), false);
    let prep = tr_multi.prepare_request(&req).unwrap();
    let resp = tr_multi.send_prepared(&prep).unwrap();
    assert!(matches!(
        resp.content.first(),
        Some(agent::ContentBlock::ToolUse { name, input, .. }) if name == "record_test" && input["ok"] == json!(true)
    ));

    // 2. Bad Claude init handshake (non-empty tools) aborts live subprocess fail-closed.
    let cli_bad = MultiAttemptCli::new(tmp.path(), "claude_bad_init", "2.1.318 (Claude Code)");
    cli_bad.set_attempt(
        1,
        &[json!({"type": "system", "subtype": "init", "tools": ["Bash"], "mcp_servers": [], "apiKeySource": "none"}).to_string()],
    );
    let tr_bad = CliExchangeTransport::new(CliRouteKind::Claude, cli_bad.exe.clone(), work.clone(), false);
    let err = tr_bad.send_prepared(&tr_bad.prepare_request(&req).unwrap()).unwrap_err();
    assert!(err.message.contains("zero tools"), "{}", err.message);

    // 3. Cooperative CancelToken kills sleeping subprocess promptly.
    let cli_sleep = MultiAttemptCli::new(tmp.path(), "claude_sleep", "2.1.318 (Claude Code)");
    cli_sleep.set_attempt(
        1,
        &[json!({"type": "system", "subtype": "init", "tools": [], "mcp_servers": [], "apiKeySource": "none"}).to_string()],
    );
    cli_sleep.set_sleep_ms(15_000);
    let tr_sleep = CliExchangeTransport::new(CliRouteKind::Claude, cli_sleep.exe.clone(), work.clone(), false);
    let prep_sleep = tr_sleep.prepare_request(&req).unwrap();
    let cancel = CancelToken::new();
    let cancel_bg = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(180));
        cancel_bg.cancel();
    });
    let started = Instant::now();
    let err = tr_sleep.send_prepared_streaming(&prep_sleep, &mut CancelObserver(cancel)).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5), "cancel took {:?}", started.elapsed());

    // 4. Single huge line (> 4 MiB without newline) hits 4 MiB safety ceiling cleanly.
    let cli_huge = MultiAttemptCli::new(tmp.path(), "claude_huge", "2.1.318 (Claude Code)");
    cli_huge.set_huge_line_bytes(5 * 1024 * 1024);
    let tr_huge = CliExchangeTransport::new(CliRouteKind::Claude, cli_huge.exe.clone(), work, false);
    let err = tr_huge.send_prepared(&tr_huge.prepare_request(&req).unwrap()).unwrap_err();
    assert!(err.message.contains("4 MiB"), "{}", err.message);
}

#[test]
fn parallel_passage_readers_isolate_attempt_dirs_and_cache_version() {
    let tmp = tempfile::TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let env = ResearchEnv::new(&data);
    let ledger = Ledger::open(&data).unwrap();
    let sid = "s-x18-parallel";
    let stmt = build_idea_statement(
        1,
        "Quantum Hall parity matches Lanczos growth.",
        &["cond-mat".into()],
        &[],
    )
    .unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Standard, true, &stmt).unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Quantum Hall parity matches Lanczos growth.",
                "subclaims": [{
                    "id": "sc-1",
                    "text": "Parity matches",
                    "idea_quote": "Quantum Hall parity",
                    "keywords": ["parity"]
                }],
                "assumptions": [],
                "perspectives": [
                    {"id": "p-1", "name": "exact", "queries": ["Quantum Hall parity"]},
                    {"id": "p-2", "name": "other_names", "queries": ["Hall parity"]}
                ],
                "questions": [],
                "exclusions": []
            }),
        )
        .unwrap();

    let tex = "Line one.\nExact shared test quote in synthetic paper.\nLine three.\n";
    let ledger_mux = Mutex::new(Ledger::open(&data).unwrap());
    let r1 = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "p1.tex",
            origin: "upload",
            raw_bytes: tex.as_bytes(),
            source_id_hint: Some("src-par-1"),
            section_filter: None,
            title: Some("Paper One"),
        },
    )
    .unwrap();
    let r2 = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "p2.tex",
            origin: "upload",
            raw_bytes: tex.as_bytes(),
            source_id_hint: Some("src-par-2"),
            section_filter: None,
            title: Some("Paper Two"),
        },
    )
    .unwrap();
    drop(ledger_mux);
    ledger
        .append_with_hash(sid, "idea.triage", 1, &json!({"items": []}))
        .unwrap();
    for stg in ["sharpen", "search", "triage"] {
        ledger
            .append_with_hash(sid, EVENT_IDEA_STAGE, 1, &json!({"stage": stg, "state": "finished", "request_id": format!("{stg}-par")}))
            .unwrap();
    }
    ledger
        .append_with_hash(sid, EVENT_IDEA_STAGE, 1, &json!({"stage": "read", "state": "requested", "request_id": "rd-par"}))
        .unwrap();

    let fetched = vec![
        IdeaPaperEventPayload {
            key: "arxiv:2501.00001".into(),
            version: Some("v1".into()),
            source_id: Some(r1.source_id.clone()),
            source_seq: Some(r1.event_seq),
            origin: "upload".into(),
            text_basis: "tex".into(),
            status: "fetched".into(),
            reason: None,
            lines_sent: 0,
            total_lines: 3,
            raw_blob: Some(r1.raw_blob.clone()),
        },
        IdeaPaperEventPayload {
            key: "arxiv:2501.00002".into(),
            version: Some("v1".into()),
            source_id: Some(r2.source_id.clone()),
            source_seq: Some(r2.event_seq),
            origin: "upload".into(),
            text_basis: "tex".into(),
            status: "fetched".into(),
            reason: None,
            lines_sent: 0,
            total_lines: 3,
            raw_blob: Some(r2.raw_blob.clone()),
        },
    ];

    let ev_json = json!({
        "items": [{
            "start_line": 2,
            "end_line": 2,
            "quote": "Exact shared test quote in synthetic paper.",
            "role": "states_result",
            "gist": "Exact match.",
            "subclaim_ids": ["sc-1"]
        }],
        "want_lines": [],
        "status": "done"
    });
    let cli = MultiAttemptCli::new(tmp.path(), "claude_par", "2.1.318 (Claude Code)");
    cli.set_default_script(&claude_exchange_lines(&fenced_json(&ev_json)));
    let tr = CliExchangeTransport::new(CliRouteKind::Claude, cli.exe.clone(), work, false);
    let write_lock = Mutex::new(());
    let batch = run_reading_passages_with_transport(
        &env,
        &ledger,
        &write_lock,
        sid,
        "rd-par",
        &fetched,
        &tr,
        "claude-sonnet-4-6",
        2,
        None,
        &|_, _| {},
    )
    .expect("parallel CLI passage readers must succeed");
    assert_eq!(batch.saved_quote_ids.len(), 2);
    let cwds: BTreeSet<String> = cli.recorded_cwds().into_iter().collect();
    assert_eq!(cwds.len(), 2, "concurrent readers must run in distinct attempt dirs: {cwds:?}");
    assert_eq!(cli.version_calls(), 1, "--version must be cached once per transport");
}

#[test]
fn resolve_explore_transport_refuses_api_to_cli_and_cross_cli_mixing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cli = MultiAttemptCli::new(tmp.path(), "claude_mix", "2.1.318 (Claude Code)");
    let mut cfg = test_cfg(tmp.path(), None, None, None);
    cfg.claude_exe = Some(cli.exe.clone());
    cfg.agy_exe = Some(cli.exe.clone());
    let chatgpt = desktop_server::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(
        cfg,
        desktop_server::auth::AuthGuard::new(1).unwrap(),
        Arc::new(desktop_server::sse::SseHub::new()),
        chatgpt,
    );
    desktop_server::antigravity::handle_acknowledge(&state).unwrap();
    let ledger = state.open_ledger().unwrap();

    // 1. Session with prior `session.start` on `anthropic` refuses `claude-cli` with 409.
    let sid_api = "s-x18-api-first";
    ledger.append_with_hash(sid_api, "idea.started", 1, &json!({})).unwrap();
    ledger
        .append_with_hash(sid_api, "session.start", 1, &json!({"provider": "anthropic", "model": "claude-sonnet-4-6"}))
        .unwrap();
    let (code, err) = resolve_explore_transport(&state, sid_api, Some(&json!({"provider": "claude-cli"})))
        .err()
        .expect("API -> CLI mixing must return 409");
    assert_eq!(code, 409, "{err}");

    // 2. Session with prior `idea.exchange` on `claude-code-cli` refuses `antigravity` with 409.
    let sid_cli = "s-x18-cli-first";
    ledger.append_with_hash(sid_cli, "idea.started", 1, &json!({})).unwrap();
    ledger
        .append_with_hash(
            sid_cli,
            EVENT_IDEA_EXCHANGE,
            1,
            &json!({"exchange": "sharpen", "provider": "claude-code-cli", "accepted": true}),
        )
        .unwrap();
    let (code, err) = resolve_explore_transport(&state, sid_cli, Some(&json!({"provider": "antigravity"})))
        .err()
        .expect("claude-cli -> antigravity mixing must return 409");
    assert_eq!(code, 409, "{err}");
}
