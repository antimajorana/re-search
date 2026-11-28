use super::*;
use serde_json::json;

#[test]
fn patch_validates_roles_providers_and_models() {
    let mut map = BTreeMap::new();
    assert!(parse_agent_models_patch(&json!([]), &mut map).is_err());
    assert!(parse_agent_models_patch(&json!({ "unknown_role": null }), &mut map).is_err());
    // Helper and worker roles forbid CLI routes.
    let err = parse_agent_models_patch(
        &json!({ "narrator": { "provider": "claude-cli", "model": "sonnet" } }),
        &mut map,
    )
    .unwrap_err();
    assert_eq!(err.0, 400);
    let w_err = parse_agent_models_patch(
        &json!({ "compute_worker": { "provider": "antigravity", "model": "gemini" } }),
        &mut map,
    )
    .unwrap_err();
    assert_eq!(w_err.0, 400);
    // Orchestrator permits CLI routes; worker roles accept API providers.
    parse_agent_models_patch(
        &json!({
            "orchestrator": { "provider": "claude-cli", "model": "sonnet" },
            "explain": { "provider": "OpenAI", "model": "gpt-4.1-mini" },
            "compute_worker": { "provider": "anthropic", "model": "haiku" },
            "literature_worker": { "provider": "openai", "model": "gpt-4.1-mini" },
            "lean_worker": { "provider": "openai", "model": "gpt-4.1-mini" },
            "explorer_worker": { "provider": "gemini", "model": "gemini-2.5-flash" }
        }),
        &mut map,
    )
    .unwrap();
    assert_eq!(map["orchestrator"].provider, "claude-cli");
    assert_eq!(map["explain"].provider, "openai");
    assert_eq!(map["compute_worker"].provider, "anthropic");
    assert_eq!(map["explorer_worker"].provider, "gemini");
    // Model starting with '-' or containing whitespace fails.
    assert!(parse_agent_models_patch(
        &json!({ "explain": { "provider": "openai", "model": "-bad" } }),
        &mut map,
    )
    .is_err());
    assert!(parse_agent_models_patch(
        &json!({ "explain": { "provider": "openai", "model": "gpt 4" } }),
        &mut map,
    )
    .is_err());
    // Null or blank removes role.
    parse_agent_models_patch(
        &json!({
            "orchestrator": null,
            "explain": { "provider": "", "model": "" },
            "compute_worker": null,
            "literature_worker": null,
            "lean_worker": null,
            "explorer_worker": null
        }),
        &mut map,
    )
    .unwrap();
    assert!(map.is_empty());
}

#[test]
fn orchestrator_precedence_respects_subagent_and_agent_models() {
    let mut ui = UiSettings::default();
    ui.agent_models.insert(
        "orchestrator".into(),
        AgentModelSpec { provider: "openai".into(), model: "gpt-4.1".into() },
    );
    ui.default_provider = Some("gemini".into());
    ui.default_model = Some("gemini-2.5-pro".into());

    let (p, req_m, m) = resolve_orchestrator_model(None, &json!({}), None, &ui).unwrap();
    assert_eq!((p, req_m.as_str(), m.as_str()), (ProviderKind::OpenAi, "gpt-4.1", "gpt-4.1"));

    let sa = SubagentDef {
        id: "qft".into(),
        name: "QFT".into(),
        description: String::new(),
        system_prompt: "test".into(),
        provider: Some("anthropic".into()),
        model: Some("haiku".into()),
        reasoning_effort: None,
        rigor: None,
        tools_profile: "full_research".into(),
        created_ms: 0,
    };
    let (p2, _, m2) = resolve_orchestrator_model(
        Some(&sa),
        &json!({ "provider": "openai", "model": "gpt-4.1" }),
        None,
        &ui,
    )
    .unwrap();
    assert_eq!(p2, ProviderKind::Anthropic);
    assert_ne!(m2, "gpt-4.1");
}

#[test]
fn solve_verifier_role_key_accepted_and_d2_same_model_still_returns_transport() {
    use crate::auth::AuthGuard;
    use crate::chatgpt::ChatGptHub;
    use crate::sse::SseHub;
    use crate::DesktopConfig;
    use agent::{ProviderEntry, ProviderKeyStore};
    use research_tools::solve_model::{ReviewIndependence, RoleModel, SolveRoles};
    use research_tools::LiteratureEndpoints;
    use std::sync::Arc;
    use tempfile::TempDir;

    // `"verifier"` is accepted by `agent_models` patch and forbids CLI routes.
    let mut map = BTreeMap::new();
    parse_agent_models_patch(
        &json!({ "verifier": { "provider": "openai", "model": "gpt-4.1" } }),
        &mut map,
    )
    .unwrap();
    assert_eq!(map["verifier"].provider, "openai");
    assert!(parse_agent_models_patch(
        &json!({ "verifier": { "provider": "claude-cli", "model": "sonnet" } }),
        &mut map,
    )
    .is_err());

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut store = ProviderKeyStore::default();
    store.providers.insert(
        "openai".into(),
        ProviderEntry {
            api_key: Some("test-fake-key-openai".into()),
            base_url: Some("http://127.0.0.1:9".into()),
            default_model: None,
        },
    );
    store.providers.insert(
        "anthropic".into(),
        ProviderEntry {
            api_key: Some("test-fake-key-anthropic".into()),
            base_url: Some("http://127.0.0.1:9".into()),
            default_model: None,
        },
    );
    store.save_to_data_dir(&data_dir).unwrap();

    let cfg = DesktopConfig {
        port: 0,
        data_dir,
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
        chatgpt_test: None,
    };
    let auth = AuthGuard::new(0).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    let state = DesktopState::new(cfg, auth, sse, chatgpt);
    let ui = UiSettings::default();

    // D2: When verifier == solver, legacy Research returns None, while Solve STILL returns a transport + SameModel!
    let legacy = resolve_optional_role_transport(
        &state,
        &ui,
        &json!({ "full_verifier": { "provider": "openai", "model": "gpt-4.1" } }),
        "full_verifier",
        ProviderKind::OpenAi,
        "gpt-4.1",
    );
    assert!(legacy.is_none(), "legacy Research helper must remain unchanged");

    let same_roles = SolveRoles {
        solver: RoleModel {
            provider: "openai".into(),
            model: "gpt-4.1".into(),
        },
        verifier: RoleModel {
            provider: "openai".into(),
            model: "gpt-4.1".into(),
        },
        compute: None,
    };
    let resolved_same = resolve_solve_verifier_transport(
        &state,
        &ui,
        Some(&same_roles),
        None,
        None,
    )
    .expect("D2: same-model Solve verifier must still resolve a transport");
    assert_eq!(resolved_same.independence, ReviewIndependence::SameModel);
    assert_eq!(resolved_same.role.provider, "openai");
    assert_eq!(resolved_same.role.model, "gpt-4.1");

    // Different provider resolves DifferentProvider.
    let diff_roles = SolveRoles {
        solver: RoleModel {
            provider: "openai".into(),
            model: "gpt-4.1".into(),
        },
        verifier: RoleModel {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
        },
        compute: None,
    };
    let resolved_diff = resolve_solve_verifier_transport(
        &state,
        &ui,
        Some(&diff_roles),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        resolved_diff.independence,
        ReviewIndependence::DifferentProvider
    );
    assert_eq!(resolved_diff.role.provider, "anthropic");

    // Alias normalization: solver `"haiku"` vs verifier `"claude-haiku-4-5"` -> SameModel.
    let alias_roles = SolveRoles {
        solver: RoleModel {
            provider: "anthropic".into(),
            model: "haiku".into(),
        },
        verifier: RoleModel {
            provider: "anthropic".into(),
            model: "claude-haiku-4-5".into(),
        },
        compute: None,
    };
    let resolved_alias = resolve_solve_verifier_transport(
        &state,
        &ui,
        Some(&alias_roles),
        None,
        None,
    )
    .unwrap();
    assert_eq!(resolved_alias.independence, ReviewIndependence::SameModel);

    // Explicit malformed, CLI-route, or unknown `req.verifier` must return 400 instead of falling back.
    for bad_req in [
        json!({ "verifier": "not-an-object" }),
        json!({ "verifier": { "provider": "claude-cli", "model": "opus" } }),
        json!({ "verifier": { "provider": "unknown-vendor", "model": "x" } }),
        json!({ "verifier": { "provider": "openai", "model": "" } }),
    ] {
        let err = resolve_solve_verifier_transport(
            &state,
            &ui,
            Some(&diff_roles),
            Some(&bad_req),
            None,
        )
        .err()
        .expect("explicit invalid req.verifier must fail with 400");
        assert_eq!(err.0, 400, "unexpected error for {bad_req}: {err:?}");
    }
}
