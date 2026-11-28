//! Unit tests for `cli_turn` / `cli_turn_setup`.
//!
//! Extracted from `cli_turn.rs` (Checkpoint S15a, shared seam with Explore X18).

use super::*;
use crate::state::{DesktopState, SessionMeta, ANTIGRAVITY_ROUTE, CLAUDE_CLI_ROUTE};
use crate::DesktopConfig;
use research_tools::LiteratureEndpoints;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn provisional_cli_switch_clears_stale_default_model() {
    let tmp = tempfile::TempDir::new().unwrap();
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
    let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(
        cfg,
        crate::auth::AuthGuard::new(1).unwrap(),
        Arc::new(crate::sse::SseHub::new()),
        chatgpt,
    );
    let meta = SessionMeta {
        id: "s-switch01".into(),
        title: "switch".into(),
        route: Some(CLAUDE_CLI_ROUTE.into()),
        default_provider: Some(CLAUDE_CLI_ROUTE.into()),
        default_model: Some("haiku".into()),
        ..SessionMeta::default()
    };
    state.save_meta(&meta).unwrap();

    let kind = wants_cli_route(&state, "s-switch01", &json!({ "provider": "antigravity" })).unwrap();
    assert_eq!(kind, Some(CliRouteKind::Antigravity));
    let updated = state.load_meta("s-switch01").unwrap();
    assert_eq!(updated.route.as_deref(), Some(ANTIGRAVITY_ROUTE));
    assert_eq!(updated.default_model, None, "stale claude-cli model must be cleared");
}

#[test]
fn connection_env_cannot_supply_claude_env_auth_opt_in_for_turns() {
    if std::env::var_os(cli_route::env_auth::ALLOW_ENV_AUTH_VAR).is_some() {
        return;
    }
    let tmp = tempfile::TempDir::new().unwrap();
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
        connection_env: Some(BTreeMap::from([
            ("ANTHROPIC_API_KEY".to_string(), "sk-ant-dummy".to_string()),
            (cli_route::env_auth::ALLOW_ENV_AUTH_VAR.to_string(), "1".to_string()),
        ])),
        chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
    };
    let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = Arc::new(DesktopState::new(
        cfg,
        crate::auth::AuthGuard::new(1).unwrap(),
        Arc::new(crate::sse::SseHub::new()),
        chatgpt,
    ));
    let err = run_cli_turn(&state, "s-env01", &json!({}), "hi", &StopHandle::new(), CliRouteKind::Claude).unwrap_err();
    assert!(err.contains("ANTHROPIC_API_KEY") && !err.contains("sk-ant-dummy"), "{err}");
}

#[test]
fn prompt_gating_keeps_research_instructions_strictly_out_of_solve_and_investigate() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ledger = ledger::Ledger::open(tmp.path()).unwrap();
    let sid = "s-solve-prompt";
    ledger.append_with_hash(sid, "solve.started", 1, &json!({})).unwrap();
    ledger
        .append_with_hash(
            sid,
            "solve.setup",
            1,
            &json!({
                "revision": 1,
                "statement": "Compute integral of x^2 from 0 to 1",
                "task_kind": "calculate",
                "target": { "description": "closed form", "form": "closed_form" },
                "done": [],
                "conventions": "real x",
                "symbols": [{ "name": "x", "assumptions": ["real"] }],
                "dimensions": {},
                "assumptions": [],
                "sources": [],
                "roles": {
                    "solver": { "provider": "claude-cli", "model": "sonnet" },
                    "verifier": { "provider": "", "model": "" }
                },
                "budgets": {
                    "max_model_calls": 60,
                    "max_wall_minutes": 30,
                    "max_model_calls_per_leg": 48,
                    "max_review_loops": 6,
                    "max_gate_bounces": 3
                },
                "review": "prose_only",
                "tool_profile": "solve/1",
                "math_sha256": "0".repeat(64)
            }),
        )
        .unwrap();
    let events = ledger.read_session(sid).unwrap();

    let solve_a = crate::cli_turn_setup::build_cli_prompt(
        research_tools::paper_key::Mode::Solve,
        &events,
        Some(&ledger),
        "Continue derivation",
        Some("real x"),
        None,
        false,
        true,
    );
    let solve_b = crate::cli_turn_setup::build_cli_prompt(
        research_tools::paper_key::Mode::Solve,
        &events,
        Some(&ledger),
        "Continue derivation",
        Some("real x"),
        None,
        false,
        true,
    );
    assert_eq!(solve_a.prompt, solve_b.prompt, "Solve CLI prompt must be byte-deterministic");
    assert!(!solve_a.replayed);
    assert!(solve_a.prompt.contains("Compute integral of x^2 from 0 to 1"));
    assert!(solve_a.prompt.contains(crate::cli_turn_setup::SOLVE_HANDLE_SURVIVAL_LINE));
    assert!(solve_a.prompt.contains("`step_record`") && solve_a.prompt.contains("`solve_finish`"));
    for forbidden in ["search_formal_library", "check_symbolic_identity", "lean_check", "tool_call_ref"] {
        assert!(!solve_a.prompt.contains(forbidden), "Solve prompt leaked `{forbidden}`:\n{}", solve_a.prompt);
    }

    let research = crate::cli_turn_setup::build_cli_prompt(
        research_tools::paper_key::Mode::Research,
        &[],
        None,
        "Hello",
        None,
        None,
        false,
        true,
    );
    assert!(research.prompt.contains("search_formal_library"));
    assert!(research.prompt.contains("check_symbolic_identity"));

    let inv = crate::cli_turn_setup::build_cli_prompt(
        research_tools::paper_key::Mode::Investigate,
        &[],
        None,
        "Inspect theorem",
        None,
        None,
        false,
        true,
    );
    assert!(!inv.prompt.contains("search_formal_library"));
    assert!(!inv.prompt.contains("check_symbolic_identity"));
}

#[test]
fn solve_core_env_propagates_profile_check_dirs_and_matches_antigravity_tool_ids() {
    let tmp = tempfile::TempDir::new().unwrap();
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
        connection_env: None,
        chatgpt_test: None,
    };
    let scratch = tmp.path().join("scratch");
    let pycache = tmp.path().join("pycache");
    let cscratch = tmp.path().join("check_scratch");
    let cpycache = tmp.path().join("check_pycache");

    let solve_env = crate::cli_turn_setup::core_env_for_profile(
        &cfg,
        "s-solve-env",
        "tok",
        "127.0.0.1:9",
        &scratch,
        &pycache,
        None,
        research_tools::paper_key::Mode::Solve,
        research_tools::tool_profile::PROFILE_SOLVE_V1,
        Some((&cscratch, &cpycache)),
    );
    assert_eq!(solve_env.get("CORE_TOOL_PROFILE").map(String::as_str), Some("solve/1"));
    assert!(!solve_env.contains_key("CORE_ENABLE_RESEARCH_TOOLS"));
    assert_eq!(solve_env.get("CORE_CHECK_TIMEOUT_S").map(String::as_str), Some("10"));
    assert!(solve_env.contains_key("CORE_CHECK_SCRATCH"));
    assert!(solve_env.contains_key("CORE_CHECK_PYCACHE"));

    let ids = cli_route::core_tools::core_tool_ids_for(&solve_env);
    assert_eq!(ids.len(), 17);
    assert!(ids.iter().any(|id| id == "step_record"));
    assert!(ids.iter().any(|id| id == "solve_finish"));
    assert!(ids.iter().any(|id| id == "set_session_title"));
    assert!(!ids.iter().any(|id| id == "fs_write"));

    let res_env = crate::cli_turn_setup::core_env_for_profile(
        &cfg,
        "s-res-env",
        "tok",
        "127.0.0.1:9",
        &scratch,
        &pycache,
        None,
        research_tools::paper_key::Mode::Research,
        research_tools::tool_profile::PROFILE_RESEARCH_V1,
        None,
    );
    assert_eq!(res_env.get("CORE_TOOL_PROFILE").map(String::as_str), Some("research/1"));
    assert_eq!(res_env.get("CORE_ENABLE_RESEARCH_TOOLS").map(String::as_str), Some("1"));
}

#[test]
fn resolve_cli_turn_profile_maps_modes_and_rejects_unbuilt_explore() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ledger = ledger::Ledger::open(tmp.path()).unwrap();

    ledger.append_with_hash("s-solve", "solve.started", 1, &json!({})).unwrap();
    let (mode, profile) =
        crate::cli_turn_setup::resolve_cli_turn_profile(&ledger.read_session("s-solve").unwrap()).unwrap();
    assert_eq!(mode, research_tools::paper_key::Mode::Solve);
    assert_eq!(profile.name, research_tools::tool_profile::PROFILE_SOLVE_V1);

    let (mode_r, profile_r) = crate::cli_turn_setup::resolve_cli_turn_profile(&[]).unwrap();
    assert_eq!(mode_r, research_tools::paper_key::Mode::Research);
    assert_eq!(profile_r.name, research_tools::tool_profile::PROFILE_RESEARCH_V1);

    ledger.append_with_hash("s-exp", "idea.started", 1, &json!({})).unwrap();
    let err = crate::cli_turn_setup::resolve_cli_turn_profile(&ledger.read_session("s-exp").unwrap())
        .unwrap_err();
    assert!(err.contains("profile not built yet"), "{err}");

    // Legacy `investigation/1` sessions upgrade to `investigation/2` just like `turn_runner.rs`.
    ledger.append_with_hash("s-inv1", "investigation.started", 1, &json!({})).unwrap();
    ledger
        .append_with_hash("s-inv1", "session.start", 1, &json!({ "tool_profile": "investigation/1" }))
        .unwrap();
    let (mode_i, profile_i) =
        crate::cli_turn_setup::resolve_cli_turn_profile(&ledger.read_session("s-inv1").unwrap())
            .unwrap();
    assert_eq!(mode_i, research_tools::paper_key::Mode::Investigate);
    assert_eq!(profile_i.name, research_tools::tool_profile::PROFILE_INVESTIGATION_V2);
}

#[test]
fn wants_cli_route_allows_antigravity_directly_on_solve_session_and_check_sandbox_id_isolates() {
    let tmp = tempfile::TempDir::new().unwrap();
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
    let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(
        cfg,
        crate::auth::AuthGuard::new(1).unwrap(),
        Arc::new(crate::sse::SseHub::new()),
        chatgpt,
    );
    let sid = "s-solve-agy";
    let ledger = ledger::Ledger::open(tmp.path()).unwrap();
    ledger.append_with_hash(sid, "solve.started", 1, &json!({})).unwrap();
    state
        .save_meta(&SessionMeta {
            id: sid.into(),
            title: "Solve Antigravity".into(),
            ..SessionMeta::default()
        })
        .unwrap();

    let kind = wants_cli_route(&state, sid, &json!({ "provider": "antigravity" })).unwrap();
    assert_eq!(kind, Some(CliRouteKind::Antigravity));

    for s in [sid, "s-1", &"x".repeat(45)] {
        let cid = crate::cli_turn_setup::check_sandbox_id(s);
        assert_ne!(cid, s.to_ascii_lowercase());
        assert!(!cid.is_empty() && cid.len() <= 43);
        assert!(cid.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')));
    }
}
