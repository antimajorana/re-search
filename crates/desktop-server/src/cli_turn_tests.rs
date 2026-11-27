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
    crate::antigravity::handle_acknowledge(&state).unwrap();
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
