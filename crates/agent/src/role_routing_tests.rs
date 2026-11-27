use super::*;

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn store_with_keys(pairs: &[(ProviderKind, &str)]) -> ProviderKeyStore {
    let mut store = ProviderKeyStore::default();
    for &(p, k) in pairs {
        store.set_key(p, Some(k.to_string())).unwrap();
    }
    store
}

#[test]
fn scoped_api_key_prevents_cross_provider_env_contamination() {
    // F-9 regression: RE_SEARCH_API_KEY belonging to Anthropic must NEVER authenticate OpenAI.
    let store = store_with_keys(&[(ProviderKind::OpenAi, "sk-openai-saved")]);
    let env = env_map(&[
        ("RE_SEARCH_PROVIDER", "anthropic"),
        ("RE_SEARCH_API_KEY", "sk-ant-generic"),
    ]);

    let ant = resolve_provider_scoped_api_key(
        ProviderKind::Anthropic,
        ProviderKind::Anthropic.default_base_url(),
        &store,
        Some(&env),
    )
    .unwrap();
    assert_eq!(ant.source, ApiKeySource::GenericEnv);
    assert_eq!(ant.source_label, "RE_SEARCH_API_KEY");

    let oai = resolve_provider_scoped_api_key(
        ProviderKind::OpenAi,
        ProviderKind::OpenAi.default_base_url(),
        &store,
        Some(&env),
    )
    .unwrap();
    assert_eq!(oai.source, ApiKeySource::Store);
    assert_eq!(oai.source_label, "providers.json");

    // Gemini has neither env key nor saved key -> error rather than using RE_SEARCH_API_KEY.
    let gem_err = resolve_provider_scoped_api_key(
        ProviderKind::Gemini,
        ProviderKind::Gemini.default_base_url(),
        &store,
        Some(&env),
    )
    .err()
    .expect("must not leak Anthropic generic env key to Gemini");
    assert!(gem_err.contains("no API key configured"));
}

#[test]
fn precedence_and_aliases_respect_override_settings_store_and_base_url() {
    let mut store = store_with_keys(&[
        (ProviderKind::Anthropic, "sk-ant"),
        (ProviderKind::OpenAi, "sk-oai"),
    ]);
    store.roles.full_verifier = ModelEndpointSpec {
        provider: ProviderKind::OpenAi,
        model: "gpt-4.1-mini".into(),
        base_url: Some("https://proxy.example/v1".into()),
    };

    let mut inputs = RoleRoutingInputs {
        store,
        env: Some(BTreeMap::new()),
        ..Default::default()
    };
    let policy = RoleRoutingPolicy::new(ProviderKind::Anthropic, "claude-sonnet-4-6");

    // 1. ProviderKeyStore.roles applies via `"verifier"` alias -> `full_verifier`, keeping base_url.
    let from_store = resolve_role(ROLE_VERIFIER, &policy, &inputs);
    assert_eq!(
        from_store,
        ResolvedRole::Pinned {
            provider: ProviderKind::OpenAi,
            model: "gpt-4.1-mini".into(),
            base_url: Some("https://proxy.example/v1".into()),
            routing_reason: RoutingReason::PresetRole,
        }
    );

    // 2. UiSettings.agent_models overrides store.roles (supporting `"literature_worker"` alias for `"idea_reader"`).
    inputs.agent_models.insert(
        ROLE_LITERATURE_WORKER.into(),
        RoleModelInput::new("openai", "gpt-5-mini"),
    );
    inputs.agent_models.insert(
        ROLE_VERIFIER.into(),
        RoleModelInput::new("anthropic", "claude-haiku-4-5"),
    );
    assert_eq!(
        resolve_role(ROLE_IDEA_READER, &policy, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::OpenAi,
            model: "gpt-5-mini".into(),
            base_url: None,
            routing_reason: RoutingReason::ExplicitRole,
        }
    );
    assert_eq!(
        resolve_role(ROLE_VERIFIER, &policy, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::Anthropic,
            model: "claude-haiku-4-5".into(),
            base_url: None,
            routing_reason: RoutingReason::ExplicitRole,
        }
    );

    // 3. Explicit run_override wins over everything, including ExplicitSameModel.
    inputs.run_overrides.insert(
        ROLE_VERIFIER.into(),
        RoleModelInput::new("anthropic", "claude-sonnet-4-6"),
    );
    assert_eq!(
        resolve_role(ROLE_VERIFIER, &policy, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::Anthropic,
            model: "claude-sonnet-4-6".into(),
            base_url: None,
            routing_reason: RoutingReason::ExplicitSameModel,
        }
    );
}

#[test]
fn explicit_misconfigured_role_returns_unavailable_never_silent_frontier_fallback() {
    let store = store_with_keys(&[(ProviderKind::Anthropic, "sk-ant")]);
    let mut inputs = RoleRoutingInputs {
        store,
        env: Some(BTreeMap::new()),
        ..Default::default()
    };
    inputs.agent_models.insert(
        ROLE_COMPUTE_BLIND.into(),
        RoleModelInput::new("openai", "gpt-4.1-mini"),
    );
    let policy = RoleRoutingPolicy::new(ProviderKind::Anthropic, "claude-sonnet-4-6")
        .with_frontier_fallback(true);

    // Missing OpenAI key -> Unavailable even when allow_frontier_fallback == true!
    let res = resolve_role(ROLE_COMPUTE_BLIND, &policy, &inputs);
    match res {
        ResolvedRole::Unavailable { configured_provider, configured_model, error } => {
            assert_eq!(configured_provider, "openai");
            assert_eq!(configured_model, "gpt-4.1-mini");
            assert!(error.contains("no API key"), "{error}");
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }

    // CLI route assigned to worker -> Unavailable.
    inputs.run_overrides.insert(
        ROLE_PAPER_READER.into(),
        RoleModelInput::new("claude-cli", "haiku"),
    );
    assert!(resolve_role(ROLE_PAPER_READER, &policy, &inputs).is_unavailable());
}

#[test]
fn unassigned_role_uses_static_tier_offline_and_disables_without_opt_in() {
    // Offline (--api-base, empty catalog) picks static smaller tier cleanly.
    let mut inputs = RoleRoutingInputs {
        store: store_with_keys(&[(ProviderKind::OpenAi, "sk-oai")]),
        env: Some(BTreeMap::new()),
        api_base_override: Some("http://127.0.0.1:19999".into()),
        ..Default::default()
    };
    let oai_policy = RoleRoutingPolicy::new(ProviderKind::OpenAi, "gpt-5");
    assert_eq!(
        resolve_role(ROLE_COMPUTE_WORKER, &oai_policy, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::OpenAi,
            model: "gpt-5-mini".into(),
            base_url: None,
            routing_reason: RoutingReason::AutoSmallerTier,
        }
    );

    // Default ProviderKeyStore.roles placeholder must NOT leak Anthropic onto an OpenAI session.
    assert_eq!(
        resolve_role(ROLE_VERIFIER, &oai_policy, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::OpenAi,
            model: "gpt-5-mini".into(),
            base_url: None,
            routing_reason: RoutingReason::AutoSmallerTier,
        }
    );

    // When primary is already smallest tier (claude-haiku-4-5), unassigned role is Disabled unless opt-in.
    inputs.store = store_with_keys(&[(ProviderKind::Anthropic, "sk-ant")]);
    let small_policy = RoleRoutingPolicy::new(ProviderKind::Anthropic, "claude-haiku-4-5");
    assert_eq!(
        resolve_role(ROLE_PAPER_READER, &small_policy, &inputs),
        ResolvedRole::Disabled { reason: REASON_NO_SMALLER_MODEL }
    );

    let opt_in = small_policy.with_frontier_fallback(true);
    assert_eq!(
        resolve_role(ROLE_PAPER_READER, &opt_in, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::Anthropic,
            model: "claude-haiku-4-5".into(),
            base_url: None,
            routing_reason: RoutingReason::ExplicitFrontierOptIn,
        }
    );
}

#[test]
fn ollama_parameter_tags_and_pinned_record_roundtrip() {
    let mut inputs = RoleRoutingInputs::default();
    inputs.catalog_models.insert(
        "ollama".into(),
        vec![
            "nomic-embed-text:latest".into(),
            "qwen2.5:32b".into(),
            "llama3.1:8b".into(),
            "qwen2.5-coder:7b".into(),
        ],
    );
    let ollama_policy = RoleRoutingPolicy::new(ProviderKind::Ollama, "qwen2.5:32b");
    let resolved = resolve_role(ROLE_COMPUTE_WORKER, &ollama_policy, &inputs);
    assert_eq!(
        resolved,
        ResolvedRole::Pinned {
            provider: ProviderKind::Ollama,
            model: "qwen2.5-coder:7b".into(),
            base_url: None,
            routing_reason: RoutingReason::AutoSmallerTier,
        }
    );

    // Serialize + roundtrip PinnedRoleRecord for ledger markers.
    let map = resolve_run_roles(&[ROLE_COMPUTE_WORKER, ROLE_VERIFIER], &ollama_policy, &inputs);
    let records = to_pinned_records(&map);
    let json = serde_json::to_string(&records).unwrap();
    let decoded: BTreeMap<String, PinnedRoleRecord> = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded[ROLE_COMPUTE_WORKER].to_resolved().unwrap(), resolved);
}
