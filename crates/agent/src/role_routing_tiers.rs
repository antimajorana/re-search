//! Provider-scoped API key resolution and deterministic smaller-tier selection for `role_routing`.

use crate::http::validate_base_url;
use crate::transport::{
    ProviderKind, DEFAULT_CHATGPT_BASE, GENERIC_KEY_ENV_VARS, PROVIDER_ENV_VARS,
};
use crate::SecretKey;
use std::collections::BTreeMap;

use super::RoleRoutingInputs;

const EXCLUDED_MEDIA_SLUGS: &[&str] = &[
    "embedding", "embed", "tts", "whisper", "image", "audio", "realtime", "moderation",
    "transcribe", "dall-e", "sora",
];
const SMALLER_TIER_KEYWORDS: &[&str] = &["haiku", "mini", "flash", "nano", "lite"];

/// Origin of a provider-scoped API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeySource {
    GenericEnv,
    ProviderEnv,
    Store,
    ChatGptOAuth,
    KeylessLoopback,
}

pub struct ScopedApiKey {
    pub key: Option<SecretKey>,
    pub source: ApiKeySource,
    pub source_label: String,
}

fn read_env(env: Option<&BTreeMap<String, String>>, name: &str) -> Option<String> {
    match env {
        Some(map) => map.get(name).cloned(),
        None => std::env::var(name).ok(),
    }
}

fn generic_key_owner(env: Option<&BTreeMap<String, String>>) -> Option<ProviderKind> {
    PROVIDER_ENV_VARS
        .into_iter()
        .find_map(|n| read_env(env, n).filter(|v| !v.trim().is_empty()))
        .and_then(|v| ProviderKind::parse(&v).ok())
        .filter(|&p| p != ProviderKind::ChatGpt)
}

/// Resolve an API key strictly scoped to `provider` (`F-9` fix).
///
/// Reads `GENERIC_KEY_ENV_VARS` (`RE_SEARCH_API_KEY`, `RESEARCH_CORE_API_KEY`) **only** when
/// `RE_SEARCH_PROVIDER` / `RESEARCH_CORE_PROVIDER` explicitly matches `provider`, then
/// `provider.own_key_env_vars()`, then `store.key_for(provider)`.
pub fn resolve_provider_scoped_api_key(
    provider: ProviderKind,
    base_url: &str,
    store: &crate::provider_config::ProviderKeyStore,
    env: Option<&BTreeMap<String, String>>,
) -> Result<ScopedApiKey, String> {
    let validated = validate_base_url(base_url).map_err(|e| e.message)?;
    if provider == ProviderKind::ChatGpt {
        return Ok(ScopedApiKey {
            key: None,
            source: ApiKeySource::ChatGptOAuth,
            source_label: "chatgpt-sign-in".into(),
        });
    }

    if generic_key_owner(env) == Some(provider) {
        for &name in &GENERIC_KEY_ENV_VARS {
            if let Some(val) = read_env(env, name).filter(|v| !v.trim().is_empty()) {
                let key = SecretKey::new(val).map_err(|e| format!("{name}: {}", e.message))?;
                return Ok(ScopedApiKey {
                    key: Some(key),
                    source: ApiKeySource::GenericEnv,
                    source_label: name.to_string(),
                });
            }
        }
    }

    for &name in provider.own_key_env_vars() {
        if let Some(val) = read_env(env, name).filter(|v| !v.trim().is_empty()) {
            let key = SecretKey::new(val).map_err(|e| format!("{name}: {}", e.message))?;
            return Ok(ScopedApiKey {
                key: Some(key),
                source: ApiKeySource::ProviderEnv,
                source_label: name.to_string(),
            });
        }
    }

    if let Some(raw) = store.key_for(provider) {
        let key =
            SecretKey::new(raw.to_string()).map_err(|e| format!("providers.json: {}", e.message))?;
        return Ok(ScopedApiKey {
            key: Some(key),
            source: ApiKeySource::Store,
            source_label: "providers.json".into(),
        });
    }

    let keyless_ok = provider == ProviderKind::Ollama
        || (provider == ProviderKind::OpenAi && validated.is_loopback_http);
    if keyless_ok {
        return Ok(ScopedApiKey {
            key: None,
            source: ApiKeySource::KeylessLoopback,
            source_label: "none".into(),
        });
    }

    Err(format!(
        "no API key configured for `{}` (checked {} and providers.json)",
        provider.cli_name(),
        provider.own_key_env_vars().join(", ")
    ))
}

pub(crate) fn effective_base_url(
    provider: ProviderKind,
    role_base_url: Option<&str>,
    inputs: &RoleRoutingInputs,
) -> Result<String, String> {
    if provider == ProviderKind::ChatGpt {
        if role_base_url.is_some() {
            return Err("Sign in with ChatGPT does not take a custom base_url".into());
        }
        return Ok(inputs
            .chatgpt_api_base
            .as_deref()
            .unwrap_or(DEFAULT_CHATGPT_BASE)
            .to_string());
    }
    if let Some(launch) = inputs.api_base_override.as_deref() {
        return Ok(launch.to_string());
    }
    if let Some(role_url) = role_base_url.map(str::trim).filter(|s| !s.is_empty()) {
        validate_base_url(role_url).map_err(|e| e.message)?;
        return Ok(role_url.to_string());
    }
    if let Some(saved) = inputs
        .store
        .providers
        .get(provider.cli_name())
        .and_then(|e| e.base_url.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        validate_base_url(saved).map_err(|e| e.message)?;
        return Ok(saved.to_string());
    }
    Ok(provider.default_base_url().to_string())
}

pub(crate) fn check_provider_ready(
    provider: ProviderKind,
    role_base_url: Option<&str>,
    inputs: &RoleRoutingInputs,
) -> Result<(), String> {
    let base = effective_base_url(provider, role_base_url, inputs)?;
    if provider == ProviderKind::ChatGpt {
        if !inputs.chatgpt_ready {
            return Err("Sign in with ChatGPT is not connected".into());
        }
        return Ok(());
    }
    let _ = resolve_provider_scoped_api_key(provider, &base, &inputs.store, inputs.env.as_ref())?;
    Ok(())
}

fn is_excluded_slug(slug: &str) -> bool {
    let low = slug.to_ascii_lowercase();
    EXCLUDED_MEDIA_SLUGS.iter().any(|bad| low.contains(bad))
}

fn already_smaller_tier(model: &str) -> bool {
    let low = model.to_ascii_lowercase();
    SMALLER_TIER_KEYWORDS.iter().any(|kw| low.contains(kw))
}

/// Parse an Ollama parameter tag like `:7b`, `:1.5b`, `:32b`, `:500m` into millions of params.
pub fn parse_ollama_param_millions(model: &str) -> Option<u64> {
    let tail = model.rsplit(':').next().unwrap_or(model);
    for raw_tok in [tail].into_iter().chain(model.split([':', '-'])) {
        let tok = raw_tok.trim().to_ascii_lowercase();
        let (num_str, mult) = if let Some(s) = tok.strip_suffix('b') {
            (s, 1_000u64)
        } else if let Some(s) = tok.strip_suffix('m') {
            (s, 1u64)
        } else {
            continue;
        };
        if num_str.is_empty() || !num_str.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            continue;
        }
        if let Some((whole, frac)) = num_str.split_once('.') {
            if frac.is_empty() || frac.contains('.') {
                continue;
            }
            let w: u64 = whole.parse().ok()?;
            let scale = 10u64.checked_pow(frac.len().min(3) as u32)?;
            let f: u64 = frac[..frac.len().min(3)].parse().ok()?;
            return Some(w.saturating_mul(mult).saturating_add(f.saturating_mul(mult) / scale));
        }
        let w: u64 = num_str.parse().ok()?;
        return Some(w.saturating_mul(mult));
    }
    None
}

/// Pick a smaller-tier model on `provider` distinct from `primary_model`.
///
/// Uses deterministic static tier defaults first (`Anthropic -> claude-haiku-4-5`,
/// `OpenAi/ChatGpt -> gpt-5-mini / gpt-4.1-mini`, `Gemini -> gemini-2.5-flash`),
/// cross-checked with `catalog_slugs` when non-empty (skipping deprecated slugs), plus
/// Ollama parameter-tag comparison (`:7b < :32b`).
pub fn pick_smaller_tier_model(
    provider: ProviderKind,
    primary_model: &str,
    catalog_slugs: &[String],
) -> Option<String> {
    let primary_low = primary_model.trim().to_ascii_lowercase();
    let has_catalog = !catalog_slugs.is_empty();

    if provider == ProviderKind::Ollama {
        let primary_params = parse_ollama_param_millions(primary_model);
        let mut best: Option<(u64, String)> = None;
        for slug in catalog_slugs {
            let low = slug.to_ascii_lowercase();
            if low.is_empty() || low == primary_low || is_excluded_slug(&low) {
                continue;
            }
            if let (Some(p), Some(cand)) = (primary_params, parse_ollama_param_millions(slug)) {
                if cand < p && best.as_ref().is_none_or(|(b, _)| cand < *b) {
                    best = Some((cand, slug.clone()));
                }
            } else if primary_params.is_none() && already_smaller_tier(slug) {
                return Some(slug.clone());
            }
        }
        return best.map(|(_, s)| s);
    }

    if already_smaller_tier(primary_model) {
        return None;
    }

    let static_candidates: &[&str] = match provider {
        ProviderKind::Anthropic => &["claude-haiku-4-5"],
        ProviderKind::OpenAi | ProviderKind::ChatGpt => {
            if primary_low.contains("gpt-5") || primary_low.contains("gpt-6") {
                &["gpt-5-mini", "gpt-4.1-mini"]
            } else {
                &["gpt-4.1-mini", "gpt-5-mini"]
            }
        }
        ProviderKind::Gemini => &["gemini-2.5-flash"],
        ProviderKind::Ollama => &[],
    };

    if !has_catalog {
        return static_candidates
            .iter()
            .find(|c| !c.eq_ignore_ascii_case(&primary_low))
            .map(|s| (*s).to_string());
    }

    for cand in static_candidates {
        let cand_low = cand.to_ascii_lowercase();
        if let Some(matched) = catalog_slugs.iter().find(|s| {
            let low = s.to_ascii_lowercase();
            !is_excluded_slug(&low)
                && low != primary_low
                && (low == cand_low || low.starts_with(&format!("{cand_low}-")))
        }) {
            return Some(matched.clone());
        }
    }

    for kw in SMALLER_TIER_KEYWORDS {
        if let Some(matched) = catalog_slugs.iter().find(|s| {
            let low = s.to_ascii_lowercase();
            !low.is_empty() && low != primary_low && !is_excluded_slug(&low) && low.contains(kw)
        }) {
            return Some(matched.clone());
        }
    }
    None
}
