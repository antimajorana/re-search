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
