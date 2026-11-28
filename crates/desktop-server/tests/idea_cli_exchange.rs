//! Integration tests for X18 Checkpoint 1: Explore pipeline exchanges over subscription CLIs
//! (`claude-cli` and `antigravity`).
//!
//! Uses compiled fake CLI subprocesses + synthetic ledgers only (zero live vendor CLI/network,
//! zero third-party paper text).

mod common;
mod idea_cli_support;

use cli_route::testkit::FakeCli;
use common::{raw_http, test_cfg};
use desktop_server::cli_turn_setup::CliRouteKind;
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_BRIEF,
    EVENT_IDEA_CANDIDATES, EVENT_IDEA_EXCHANGE, EVENT_IDEA_MAP, EVENT_IDEA_STAGE,
    EVENT_IDEA_VERDICT,
};
use desktop_server::idea_cli_exchange::{
    check_claude_exchange_init, exchange_agy_args, exchange_agy_settings, exchange_claude_args,
    extract_json_object, CliExchangeTransport,
};
use desktop_server::idea_exchanges::{run_sharpen_with_transport, SharpenRunResult};
use desktop_server::idea_exchanges_triage::{run_triage_with_transport, TriageRunResult};
use desktop_server::idea_map_exchange::{run_map_with_transport, MapRunResult};
use desktop_server::idea_read::IdeaPaperEventPayload;
use desktop_server::idea_read_passages::run_reading_passages_with_transport;
use desktop_server::start_desktop_server;
use idea_cli_support::{agy_exchange_lines, claude_exchange_lines, fenced_json, MultiAttemptCli};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::run_budget::{count_model_calls, RunScope};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

const IDEA_TEXT: &str =
    "Use Krylov complexity growth b_n ~ alpha*n in the volume-law phase and saturation in the area-law phase of a monitored Majorana chain.";

fn valid_sharpen_json() -> Value {
    json!({
        "restatement": "Test whether Lanczos coefficient growth detects the monitored Majorana chain transition.",
        "assumptions": ["Free-fermion chain"],
        "subclaims": [
            {
                "text": "b_n grows linearly in volume-law phase",
                "idea_quote": "b_n ~ alpha*n in the volume-law phase"
            },
            {
                "text": "b_n saturates in area-law phase",
                "idea_quote": "saturation in the area-law phase"
            }
        ],
        "terms": [{ "term": "Krylov complexity", "aliases": ["spread complexity"] }],
        "perspectives": [
            {
                "kind": "exact",
                "question": "Direct Krylov complexity work on monitored Majorana chains",
                "queries": [{ "provider": "arxiv", "q": "Krylov complexity Majorana monitored" }]
            },
            {
                "kind": "other_names",
                "question": "Alternative spread complexity terminology",
                "queries": [{ "provider": "inspire", "q": "spread complexity entanglement transition" }]
            }
        ],
        "questions": [],
        "exclusions": []
    })
}

#[test]
fn pure_extract_json_and_pinned_exchange_flags_and_handshakes() {
    let fenced = "Here is the record:\n```json\n{\"a\": 1, \"nested\": {\"b\": \"x\"}}\n```";
    assert_eq!(extract_json_object(fenced).unwrap()["a"], 1);
    assert_eq!(extract_json_object("{\"ok\": true}").unwrap()["ok"], true);
    assert!(extract_json_object("[\"not\", \"object\"]").is_err());
    assert!(extract_json_object("plain prose only").is_err());

    let claude_args = exchange_claude_args("sonnet", Path::new("mcp.json"), Some("high"));
    assert!(claude_args.windows(2).any(|w| w == ["--tools", ""]));
    assert!(claude_args.contains(&"--strict-mcp-config".to_string()));
    assert!(claude_args.windows(2).any(|w| w == ["--permission-prompts", "none"]));
    assert!(claude_args.windows(2).any(|w| w == ["--effort", "high"]));
    assert!(!claude_args.iter().any(|a| a == "--allowedTools"));

    let agy_cfg = exchange_agy_settings();
    assert_eq!(agy_cfg["permissions"]["allow"], json!([]));
    assert_eq!(agy_cfg["toolPermission"], "request-review");
    let agy_args = exchange_agy_args("gemini-3.8-flash-medium", None);
    assert!(agy_args.windows(2).any(|w| w == ["--input-format", "stream-json"]));
    assert!(agy_args.contains(&"--sandbox".to_string()));

    let good_init = json!({
        "type": "system", "subtype": "init", "tools": [], "mcp_servers": [], "apiKeySource": "none"
    });
    assert!(check_claude_exchange_init(&good_init, false).is_ok());

    let bad_tools = json!({
        "type": "system", "subtype": "init", "tools": ["Bash"], "mcp_servers": [], "apiKeySource": "none"
    });
    assert!(check_claude_exchange_init(&bad_tools, false).is_err());

    let bad_billing = json!({
        "type": "system", "subtype": "init", "tools": [], "mcp_servers": [], "apiKeySource": "ANTHROPIC_API_KEY"
    });
    assert!(check_claude_exchange_init(&bad_billing, false).is_err());
    assert!(check_claude_exchange_init(&bad_billing, true).is_ok());
}

#[test]
fn http_sharpen_runs_end_to_end_on_fake_claude_and_antigravity_with_consent_and_auth_guards() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let fake_claude = FakeCli::new(tmp.path(), "fake-claude");
    fake_claude.script(&claude_exchange_lines(&fenced_json(&valid_sharpen_json())));
    let fake_agy = FakeCli::new_agy(tmp.path(), "fake-agy");
    fake_agy.script(&agy_exchange_lines(&fenced_json(&valid_sharpen_json())));

    let mut cfg = test_cfg(&data, None, None, None);
    cfg.claude_exe = Some(fake_claude.exe.clone());
    cfg.agy_exe = Some(fake_agy.exe.clone());
    cfg.connection_env = Some(BTreeMap::new());
    let srv = start_desktop_server(cfg).unwrap();
    let port = srv.addr.port();
    let auth = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // 1. Claude CLI route: create session and run HTTP Sharpen -> succeeds with CLI provenance.
    let create_cl = serde_json::to_vec(&json!({
        "id": "s-x18-cl", "mode": "explore", "text": IDEA_TEXT,
        "provider": "claude-cli", "offline": true, "queries": []
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions", &auth, &create_cl).0, 200);
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-x18-cl/idea/sharpen", &auth, b"{}");
    assert_eq!(st, 200, "claude sharpen failed: {}", String::from_utf8_lossy(&raw));
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["attempts"], 1);
    assert_eq!(fake_claude.read("mcp_copy.json").as_deref(), Some(r#"{"mcpServers":{}}"#));

    let ledger = Ledger::open(&data).unwrap();
    let cl_events = ledger.read_session("s-x18-cl").unwrap();
    let cl_ex = cl_events.iter().find(|e| e.kind == EVENT_IDEA_EXCHANGE).unwrap();
    assert_eq!(cl_ex.payload["provider"], "claude-code-cli");
    assert_eq!(cl_ex.payload["accepted"], true);
    assert_eq!(cl_ex.payload["tokens_in"], 111);
    assert_eq!(cl_ex.payload["tokens_out"], 52);
    let req_blob: Value =
        serde_json::from_slice(&ledger.get_blob(cl_ex.payload["request_blob"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(req_blob["route"], "claude-code-cli");
    assert_eq!(req_blob["provenance"], cli_route::PROVENANCE);

    // 2. Antigravity route: MUST return 403 before acknowledgement and NEVER invoke the CLI.
    let create_agy = serde_json::to_vec(&json!({
        "id": "s-x18-agy", "mode": "explore", "text": IDEA_TEXT,
        "provider": "antigravity", "offline": true, "queries": []
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions", &auth, &create_agy).0, 200);
    let (st_unack, _, body_unack) =
        raw_http(port, "POST", "/api/sessions/s-x18-agy/idea/sharpen", &auth, b"{}");
    assert_eq!(st_unack, 403, "unacknowledged agy must 403: {}", String::from_utf8_lossy(&body_unack));
    assert!(fake_agy.argv().is_none(), "unacknowledged agy must never spawn CLI");

    // 3. Acknowledge Antigravity -> HTTP Sharpen now succeeds and audits antigravity provenance.
    let (st_ack, _, _) =
        raw_http(port, "POST", "/api/connections/antigravity/acknowledge", &auth, b"{}");
    assert_eq!(st_ack, 200);
    let (st_ok, _, raw_agy) =
        raw_http(port, "POST", "/api/sessions/s-x18-agy/idea/sharpen", &auth, b"{}");
    assert_eq!(st_ok, 200, "acknowledged agy sharpen failed: {}", String::from_utf8_lossy(&raw_agy));
    let agy_events = ledger.read_session("s-x18-agy").unwrap();
    let agy_ex = agy_events.iter().find(|e| e.kind == EVENT_IDEA_EXCHANGE).unwrap();
    assert_eq!(agy_ex.payload["provider"], "antigravity");
    assert_eq!(agy_ex.payload["accepted"], true);
    let agy_resp_blob: Value = serde_json::from_slice(
        &ledger.get_blob(agy_ex.payload["response_blob"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(agy_resp_blob["route"], "antigravity");
    assert_eq!(agy_resp_blob["confinement"], cli_route::antigravity::CONFINEMENT);
    assert_eq!(agy_resp_blob["provenance"], cli_route::PROVENANCE);
    srv.shutdown();

    // 4. Billing safety: ANTHROPIC_API_KEY in connection_env blocks claude-cli even if opt-in is spoofed in connection_env.
    if std::env::var_os(cli_route::env_auth::ALLOW_ENV_AUTH_VAR).is_none() {
        let data_auth = tmp.path().join("data-auth");
        let mut cfg_auth = test_cfg(&data_auth, None, None, None);
        cfg_auth.claude_exe = Some(fake_claude.exe.clone());
        cfg_auth.connection_env = Some(BTreeMap::from([
            ("ANTHROPIC_API_KEY".into(), "sk-ant-dummy".into()),
            (cli_route::env_auth::ALLOW_ENV_AUTH_VAR.into(), "1".into()),
        ]));
        let srv_auth = start_desktop_server(cfg_auth).unwrap();
        let p2 = srv_auth.addr.port();
        let a2 = [
            ("X-ReSearch-Token", srv_auth.info.token.as_str()),
            ("Content-Type", "application/json"),
        ];
        assert_eq!(raw_http(p2, "POST", "/api/sessions", &a2, &create_cl).0, 200);
        let (st_blk, _, body_blk) =
            raw_http(p2, "POST", "/api/sessions/s-x18-cl/idea/sharpen", &a2, b"{}");
        assert_eq!(st_blk, 400);
        let txt = String::from_utf8_lossy(&body_blk);
        assert!(txt.contains("ANTHROPIC_API_KEY") && !txt.contains("sk-ant-dummy"), "{txt}");
        srv_auth.shutdown();
    }
}

#[test]
fn cli_exchange_repairs_malformed_json_stops_at_two_repairs_and_honors_budget_cap_and_rule_p2() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &[]).unwrap();

    // A. Malformed JSON on attempt 1 -> valid JSON on attempt 2 (1 repair).
    let sid_rep = "s-x18-repair";
    append_explore_session_start(&ledger, sid_rep, IdeaPreset::Standard, true, &stmt).unwrap();
    let cli_rep = MultiAttemptCli::new(tmp.path(), "cli-rep", "2.1.318 (Claude Code)");
    cli_rep.set_attempt(1, &claude_exchange_lines("```json\n{ malformed json\n```"));
    cli_rep.set_attempt(2, &claude_exchange_lines(&fenced_json(&valid_sharpen_json())));
    let transport_rep = CliExchangeTransport::new(
        CliRouteKind::Claude,
        cli_rep.exe.clone(),
        tmp.path().join("work-rep"),
        false,
    );
    let res = run_sharpen_with_transport(&ledger, sid_rep, &transport_rep, "sonnet", None, |_, _| {})
        .expect("repair on attempt 2 must succeed");
    assert!(matches!(res, SharpenRunResult::Finished { attempts: 2, .. }), "{res:?}");
    assert_eq!(cli_rep.attempts_run(), 2);
    let stdin2 = cli_rep.stdin_of(2).unwrap();
    assert!(stdin2.contains("JSON extraction fault") || stdin2.contains("record_idea_brief"), "{stdin2}");
    let evs = ledger.read_session(sid_rep).unwrap();
    assert_eq!(count_model_calls(&evs, RunScope::All), 2);
    assert!(evs.iter().any(|e| e.kind == EVENT_IDEA_BRIEF));

    // B. Malformed JSON on all attempts -> stops after 3 calls (1 initial + 2 repairs max).
    let sid_ex = "s-x18-exhaust";
    append_explore_session_start(&ledger, sid_ex, IdeaPreset::Standard, true, &stmt).unwrap();
    let cli_ex = MultiAttemptCli::new(tmp.path(), "cli-ex", "2.1.318 (Claude Code)");
    for n in 1..=4 {
        cli_ex.set_attempt(n, &claude_exchange_lines("No JSON object here at all."));
    }
    let transport_ex = CliExchangeTransport::new(
        CliRouteKind::Claude,
        cli_ex.exe.clone(),
        tmp.path().join("work-ex"),
        false,
    );
    let err = run_sharpen_with_transport(&ledger, sid_ex, &transport_ex, "sonnet", None, |_, _| {})
        .unwrap_err();
    assert!(err.contains("validation failed after 3 attempt"), "{err}");
    assert_eq!(cli_ex.attempts_run(), 3, "must cap at 1 + 2 repairs");

    // C. Budget cap (Rule P3): Quick preset cap=5 reached -> StoppedAtCap with ZERO CLI calls.
    let sid_cap = "s-x18-cap";
    append_explore_session_start(&ledger, sid_cap, IdeaPreset::Quick, true, &stmt).unwrap();
    for i in 1..=5 {
        ledger
            .append_with_hash(
                sid_cap,
                EVENT_IDEA_EXCHANGE,
                1,
                &json!({ "exchange": "prior", "attempt": i, "accepted": true, "faults": [] }),
            )
            .unwrap();
    }
    let cli_cap = MultiAttemptCli::new(tmp.path(), "cli-cap", "2.1.318 (Claude Code)");
    let transport_cap = CliExchangeTransport::new(
        CliRouteKind::Claude,
        cli_cap.exe.clone(),
        tmp.path().join("work-cap"),
        false,
    );
    let cap_res =
        run_sharpen_with_transport(&ledger, sid_cap, &transport_cap, "sonnet", None, |_, _| {})
            .unwrap();
    assert!(matches!(cap_res, SharpenRunResult::StoppedAtCap { .. }), "{cap_res:?}");
    assert_eq!(cli_cap.attempts_run(), 0, "must never spawn CLI when budget cap is reached");

    // D. Rule P2: unexpected tool_use emitted by CLI fails immediately on attempt 1.
    let sid_p2 = "s-x18-p2";
    append_explore_session_start(&ledger, sid_p2, IdeaPreset::Standard, true, &stmt).unwrap();
    let cli_p2 = MultiAttemptCli::new(tmp.path(), "cli-p2", "2.1.318 (Claude Code)");
    cli_p2.set_attempt(
        1,
        &[
            json!({
                "type": "system", "subtype": "init", "tools": [], "mcp_servers": [],
                "model": "claude-sonnet-4-6", "claude_code_version": "2.1.318", "apiKeySource": "none"
            })
            .to_string(),
            json!({
                "type": "assistant",
                "message": { "content": [{ "type": "tool_use", "id": "tu-bad", "name": "fs_write", "input": {} }] }
            })
            .to_string(),
            json!({ "type": "result", "subtype": "success", "is_error": false, "result": "" }).to_string(),
        ],
    );
    let transport_p2 = CliExchangeTransport::new(
        CliRouteKind::Claude,
        cli_p2.exe.clone(),
        tmp.path().join("work-p2"),
        false,
    );
    let err_p2 = run_sharpen_with_transport(&ledger, sid_p2, &transport_p2, "sonnet", None, |_, _| {})
        .unwrap_err();
    assert!(err_p2.contains("Rule P2") && err_p2.contains("fs_write"), "{err_p2}");
    assert_eq!(cli_p2.attempts_run(), 1);
}

#[test]
fn cli_passage_reader_returns_fabricated_quote_fault_and_triage_and_map_work_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let ledger = Ledger::open(&data).unwrap();
    let env = ResearchEnv::new(&data);
    let sid = "s-x18-pipeline";

    let stmt = build_idea_statement(1, IDEA_TEXT, &[], &[]).unwrap();
    append_explore_session_start(&ledger, sid, IdeaPreset::Standard, true, &stmt).unwrap();

    // 1. Sharpen on Claude CLI.
    let cli_sh = MultiAttemptCli::new(tmp.path(), "cli-sh", "2.1.318 (Claude Code)");
    cli_sh.set_attempt(1, &claude_exchange_lines(&fenced_json(&valid_sharpen_json())));
    let t_sh = CliExchangeTransport::new(CliRouteKind::Claude, cli_sh.exe.clone(), tmp.path().join("w-sh"), false);
    run_sharpen_with_transport(&ledger, sid, &t_sh, "sonnet", None, |_, _| {}).unwrap();

    // 2. Seed Search finished + verbatim candidate abstract (Rule C3 arXiv Atom XML blob), then run Triage over Antigravity CLI.
    let raw_abs_blob = ledger
        .put_blob(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2501.01234v1</id>
    <title>Synthetic Krylov Growth in Monitored Chains</title>
    <summary>We prove that b_n grows linearly in the volume-law phase for monitored fermions.</summary>
  </entry>
</feed>"#,
        )
        .unwrap();
    ledger
        .append_with_hash(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": "search", "state": "finished", "request_id": "s-1" }))
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_CANDIDATES,
            1,
            &json!({
                "round": 1,
                "candidates": [{
                    "key": "arxiv:2501.01234",
                    "ids": { "arxiv": "2501.01234" },
                    "title": "Synthetic Krylov Growth in Monitored Chains",
                    "authors": ["A. Author"],
                    "year": 2025,
                    "abstract_text": "We prove that b_n grows linearly in the volume-law phase for monitored fermions.",
                    "abstract_ref": { "raw_blob": raw_abs_blob, "provider": "arxiv", "verbatim": true },
                    "found_by": ["q-1"],
                    "perspectives": ["exact"],
                    "score": 1.0,
                    "in_library": false,
                    "owned": false,
                    "user_seed": false,
                    "cited_by_origin": false,
                    "withdrawn_or_retracted": false
                }]
            }),
        )
        .unwrap();

    let cli_tr = MultiAttemptCli::new(tmp.path(), "cli-tr", "1.5.14");
    let triage_json = json!({
        "items": [{
            "key": "arxiv:2501.01234",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Directly studies b_n growth in the volume-law phase.",
            "abstract_quote": "b_n grows linearly in the volume-law phase"
        }],
        "reading_list": ["arxiv:2501.01234"]
    });
    cli_tr.set_attempt(1, &agy_exchange_lines(&fenced_json(&triage_json)));
    let t_tr = CliExchangeTransport::new(CliRouteKind::Antigravity, cli_tr.exe.clone(), tmp.path().join("w-tr"), false);
    let tr_res = run_triage_with_transport(&data, &ledger, sid, &t_tr, "gemini-3.8-flash-medium", None, |_, _| {})
        .expect("triage on Antigravity CLI must succeed");
    assert!(matches!(tr_res, TriageRunResult::Finished { .. }), "{tr_res:?}");

    // 3. Ingest a synthetic paper, open Read stage, and test Passage Reader fabricated-quote repair!
    let paper_tex = "\\section{Krylov Complexity}\n\
                     We study a monitored Majorana chain across the entanglement transition.\n\
                     Theorem 1: The Lanczos coefficients grow as b_n = alpha * n in the volume-law phase.\n\
                     In the area-law phase, the coefficients saturate to a finite constant.\n";
    let ledger_mux = Mutex::new(Ledger::open(&data).unwrap());
    let receipt = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.tex",
            origin: "upload",
            raw_bytes: paper_tex.as_bytes(),
            source_id_hint: Some("src-2501"),
            section_filter: None,
            title: Some("Synthetic Krylov Growth in Monitored Chains"),
        },
    )
    .unwrap();
    drop(ledger_mux);
    ledger
        .append_with_hash(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": "read", "state": "requested", "request_id": "rd-1" }))
        .unwrap();

    let fetched = vec![IdeaPaperEventPayload {
        key: "arxiv:2501.01234".into(),
        version: Some("v1".into()),
        source_id: Some(receipt.source_id.clone()),
        source_seq: Some(receipt.event_seq),
        origin: "upload".into(),
        text_basis: "tex".into(),
        status: "fetched".into(),
        reason: None,
        lines_sent: 0,
        total_lines: 4,
        raw_blob: Some(receipt.raw_blob.clone()),
    }];

    // Attempt 1 fabricates a quote; Rust validator returns fault + true lines -> Attempt 2 repairs!
    let cli_rd = MultiAttemptCli::new(tmp.path(), "cli-rd", "2.1.318 (Claude Code)");
    let bad_ev = json!({
        "items": [{
            "start_line": 3, "end_line": 3,
            "quote": "Completely fabricated sentence not present anywhere in paper",
            "role": "states_result",
            "gist": "Claims linear growth",
            "subclaim_ids": ["sc-1"]
        }],
        "want_lines": [],
        "status": "done"
    });
    let good_ev = json!({
        "items": [{
            "start_line": 3, "end_line": 3,
            "quote": "The Lanczos coefficients grow as b_n = alpha * n in the volume-law phase.",
            "role": "states_result",
            "gist": "Lanczos coefficients grow linearly in the volume-law phase.",
            "subclaim_ids": ["sc-1"]
        }],
        "want_lines": [],
        "status": "done"
    });
    cli_rd.set_attempt(1, &claude_exchange_lines(&fenced_json(&bad_ev)));
    cli_rd.set_attempt(2, &claude_exchange_lines(&fenced_json(&good_ev)));

    let t_rd = CliExchangeTransport::new(CliRouteKind::Claude, cli_rd.exe.clone(), tmp.path().join("w-rd"), false);
    let write_lock = Mutex::new(());
    let batch = run_reading_passages_with_transport(
        &env, &ledger, &write_lock, sid, "rd-1", &fetched, &t_rd, "sonnet", 1, None, &|_, _| {},
    )
    .expect("passage reader repair on CLI must succeed");
    assert_eq!(cli_rd.attempts_run(), 2);
    assert_eq!(batch.saved_quote_ids.len(), 1);
    let xe_id = batch.saved_quote_ids[0].clone();
    let rd_stdin2 = cli_rd.stdin_of(2).unwrap();
    assert!(
        rd_stdin2.contains("The Lanczos coefficients grow as b_n = alpha * n"),
        "fabricated quote fault must return true text to CLI:\n{rd_stdin2}"
    );
    ledger
        .append_with_hash(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": "read", "state": "finished", "request_id": "rd-1" }))
        .unwrap();

    // 4. Run Map exchange over Claude CLI referencing the verified `xe-<seq>` -> atomic map + verdict!
    let cli_map = MultiAttemptCli::new(tmp.path(), "cli-map", "2.1.318 (Claude Code)");
    let map_json = json!({
        "nodes": [{
            "temp_id": "n-1",
            "kind": "known_result",
            "label": "Linear b_n growth in volume-law phase",
            "statement": "Paper shows b_n grows linearly in the volume-law phase.",
            "evidence": [xe_id.clone()],
            "relation_to_idea": "same",
            "subclaim_ids": ["sc-1"],
            "hypotheses": ["monitored Majorana chain"],
            "regime": "volume-law"
        }],
        "edges": [],
        "overlaps": [{
            "subclaim_id": "sc-1",
            "paper_key": "arxiv:2501.01234",
            "relation": "same",
            "evidence": [xe_id],
            "difference": "Matched in the monitored Majorana chain regime."
        }]
    });
    cli_map.set_attempt(1, &claude_exchange_lines(&fenced_json(&map_json)));
    let t_map = CliExchangeTransport::new(CliRouteKind::Claude, cli_map.exe.clone(), tmp.path().join("w-map"), false);
    let map_res = run_map_with_transport(&ledger, &write_lock, sid, &t_map, "sonnet", None, |_, _| {})
        .expect("map on CLI must succeed");
    assert!(matches!(map_res, MapRunResult::Finished { attempts: 1, .. }), "{map_res:?}");

    let final_evs = ledger.read_session(sid).unwrap();
    assert!(final_evs.iter().any(|e| e.kind == EVENT_IDEA_MAP));
    assert!(final_evs.iter().any(|e| e.kind == EVENT_IDEA_VERDICT));
}
