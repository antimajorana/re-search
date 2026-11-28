//! Strict per-run model role routing, pinning, and transport construction
//! (package F9b, `00-overview.md` §3.4, `audit-orchestration-routing.md` §2.1).
//!
//! Eliminates silent fallbacks to `primary_model`:
//! - Explicit misconfigured roles yield [`ResolvedRole::Unavailable`].
//! - Unassigned worker roles with no distinct smaller model yield [`ResolvedRole::Disabled`]
//!   unless `allow_frontier_fallback == true` ([`RoutingReason::ExplicitFrontierOptIn`]).
//! - `GENERIC_KEY_ENV_VARS` (`RE_SEARCH_API_KEY`) only applies when `RE_SEARCH_PROVIDER`
//!   matches the target provider, preventing cross-provider key contamination (`F-9`).

#[path = "role_routing_tiers.rs"]
mod tiers;

pub use tiers::{
    parse_ollama_param_millions, pick_smaller_tier_model, resolve_provider_scoped_api_key,
    ApiKeySource, ScopedApiKey,
};
use tiers::{check_provider_ready, effective_base_url};

use crate::anthropic::{AnthropicConfig, AnthropicTransport};
use crate::gemini::{validate_gemini_model, GeminiConfig, GeminiTransport};
use crate::openai::{OpenAiConfig, OpenAiTransport};
use crate::presets::{ModelEndpointSpec, RoleAssignment};
use crate::provider_config::ProviderKeyStore;
use crate::responses::{BearerSource, ChatGptConfig, ChatGptTransport};
use crate::transport::{ProviderKind, Transport};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const ROLE_ORCHESTRATOR: &str = "orchestrator";
pub const ROLE_SOLVER: &str = "solver";
pub const ROLE_VERIFIER: &str = "verifier";
pub const ROLE_FULL_VERIFIER: &str = "full_verifier";
pub const ROLE_STEP_VERIFIER: &str = "step_verifier";
pub const ROLE_PAPER_READER: &str = "paper_reader";
pub const ROLE_IDEA_READER: &str = "idea_reader";
pub const ROLE_LITERATURE_WORKER: &str = "literature_worker";
pub const ROLE_COMPUTE_WORKER: &str = "compute_worker";
pub const ROLE_COMPUTE_BLIND: &str = "compute_blind";
pub const ROLE_EXPLORER_WORKER: &str = "explorer_worker";
pub const ROLE_STRATEGIST: &str = "strategist";
pub const ROLE_NARRATOR: &str = "narrator";
pub const ROLE_EXPLAIN: &str = "explain";
pub const ROLE_SIDETALK: &str = "sidetalk";
pub const ROLE_LEAN_WORKER: &str = "lean_worker";
pub const ROLE_SUBAGENT_BUILDER: &str = "subagent_builder";

pub const REASON_NO_SMALLER_MODEL: &str = "no_smaller_model_configured";

/// Why a role resolved to its pinned provider/model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingReason {
    ExplicitRole,
    PresetRole,
    AutoSmallerTier,
    ExplicitFrontierOptIn,
    ExplicitSameModel,
}

impl RoutingReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitRole => "explicit_role",
            Self::PresetRole => "preset_role",
            Self::AutoSmallerTier => "auto_smaller_tier",
            Self::ExplicitFrontierOptIn => "explicit_frontier_opt_in",
            Self::ExplicitSameModel => "explicit_same_model",
        }
    }
}

/// Outcome of resolving one worker or orchestrator role for a run or stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResolvedRole {
    Pinned {
        provider: ProviderKind,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        routing_reason: RoutingReason,
    },
    Disabled {
        reason: &'static str,
    },
    Unavailable {
        configured_provider: String,
        configured_model: String,
        error: String,
    },
}

impl ResolvedRole {
    pub fn is_pinned(&self) -> bool {
        matches!(self, Self::Pinned { .. })
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinnedRoleStatus {
    Pinned,
    Disabled,
    Unavailable,
}

/// Serializable pinned-role snapshot written onto mode start/task/stage/setup events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedRoleRecord {
    pub role: String,
    pub status: PinnedRoleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_reason: Option<RoutingReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PinnedRoleRecord {
    pub fn from_resolved(role: impl Into<String>, resolved: &ResolvedRole) -> Self {
        let role = role.into();
        match resolved {
            ResolvedRole::Pinned { provider, model, base_url, routing_reason } => Self {
                role,
                status: PinnedRoleStatus::Pinned,
                provider: Some(provider.cli_name().to_string()),
                model: Some(model.clone()),
                base_url: base_url.clone(),
                routing_reason: Some(*routing_reason),
                reason: None,
                error: None,
            },
            ResolvedRole::Disabled { reason } => Self {
                role,
                status: PinnedRoleStatus::Disabled,
                provider: None,
                model: None,
                base_url: None,
                routing_reason: None,
                reason: Some((*reason).to_string()),
                error: None,
            },
            ResolvedRole::Unavailable { configured_provider, configured_model, error } => Self {
                role,
                status: PinnedRoleStatus::Unavailable,
                provider: Some(configured_provider.clone()),
                model: Some(configured_model.clone()),
                base_url: None,
                routing_reason: None,
                reason: None,
                error: Some(error.clone()),
            },
        }
    }

    /// Reconstruct a pinned role during retry/repair/resume without re-reading mutable settings.
    pub fn to_resolved(&self) -> Result<ResolvedRole, String> {
        match self.status {
            PinnedRoleStatus::Pinned => {
                let p_raw = self.provider.as_deref().ok_or("pinned role missing `provider`")?;
                let provider = ProviderKind::parse(p_raw)?;
                let model = self
                    .model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .ok_or("pinned role missing `model`")?;
                let routing_reason = self.routing_reason.unwrap_or(RoutingReason::ExplicitRole);
                Ok(ResolvedRole::Pinned {
                    provider,
                    model,
                    base_url: self.base_url.clone(),
                    routing_reason,
                })
            }
            PinnedRoleStatus::Disabled => Ok(ResolvedRole::Disabled {
                reason: match self.reason.as_deref() {
                    Some(REASON_NO_SMALLER_MODEL) | None => REASON_NO_SMALLER_MODEL,
                    Some(_) => "disabled",
                },
            }),
            PinnedRoleStatus::Unavailable => Ok(ResolvedRole::Unavailable {
                configured_provider: self.provider.clone().unwrap_or_default(),
                configured_model: self.model.clone().unwrap_or_default(),
                error: self.error.clone().unwrap_or_else(|| "role unavailable".into()),
            }),
        }
    }
}

/// Raw role model assignment supplied by run overrides, `UiSettings.agent_models`, or presets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModelInput {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

impl RoleModelInput {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self { provider: provider.into(), model: model.into(), base_url: None }
    }

    pub fn with_base_url(mut self, base_url: Option<String>) -> Self {
        self.base_url = base_url;
        self
    }
}

/// Policy controlling strictness of role resolution for a run or stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRoutingPolicy {
    pub primary_provider: ProviderKind,
    pub primary_model: String,
    pub allow_frontier_fallback: bool,
    pub allow_same_model_explicit: bool,
    pub allow_cli_for_workers: bool,
}

impl RoleRoutingPolicy {
    pub fn new(primary_provider: ProviderKind, primary_model: impl Into<String>) -> Self {
        Self {
            primary_provider,
            primary_model: primary_model.into(),
            allow_frontier_fallback: false,
            allow_same_model_explicit: true,
            allow_cli_for_workers: false,
        }
    }

    pub fn with_frontier_fallback(mut self, allow: bool) -> Self {
        self.allow_frontier_fallback = allow;
        self
    }
}

/// Controls how `ProviderKeyStore.roles` participates when `providers.json` holds default values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StoreRolesMode {
    /// Honor only roles in `ProviderKeyStore.roles` that differ from `RoleAssignment::default()`.
    #[default]
    Auto,
    /// Honor every role in `ProviderKeyStore.roles` unconditionally.
    Always,
    /// Ignore `ProviderKeyStore.roles`.
    Never,
}

/// Complete inputs for deterministic [`resolve_role`] / [`resolve_run_roles`].
#[derive(Debug, Clone, Default)]
pub struct RoleRoutingInputs {
    pub run_overrides: BTreeMap<String, RoleModelInput>,
    pub agent_models: BTreeMap<String, RoleModelInput>,
    pub role_presets: BTreeMap<String, RoleModelInput>,
    pub store: ProviderKeyStore,
    pub store_roles_mode: StoreRolesMode,
    pub catalog_models: BTreeMap<String, Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub api_base_override: Option<String>,
    pub chatgpt_ready: bool,
    pub chatgpt_api_base: Option<String>,
}

fn role_aliases(role: &str) -> Vec<&str> {
    match role.trim() {
        "verifier" => vec!["verifier", "full_verifier", "step_verifier"],
        "full_verifier" => vec!["full_verifier", "verifier"],
        "step_verifier" => vec!["step_verifier", "verifier"],
        "idea_reader" => vec!["idea_reader", "literature_worker"],
        "compute_worker" | "compute" => vec!["compute_worker", "compute"],
        "explain" => vec!["explain", "sidetalk"],
        "sidetalk" => vec!["sidetalk", "explain"],
        "orchestrator" | "solver" => vec!["orchestrator", "solver"],
        other => vec![other],
    }
}

fn lookup_map_with_aliases(map: &BTreeMap<String, RoleModelInput>, role: &str) -> Option<RoleModelInput> {
    role_aliases(role).into_iter().find_map(|a| map.get(a).cloned())
}

fn store_role_spec<'a>(
    roles: &'a RoleAssignment,
    key: &str,
) -> Option<(&'static str, &'a ModelEndpointSpec)> {
    match key {
        "solver" | "orchestrator" => Some(("solver", &roles.solver)),
        "step_verifier" => Some(("step_verifier", &roles.step_verifier)),
        "full_verifier" | "verifier" => Some(("full_verifier", &roles.full_verifier)),
        "strategist" => Some(("strategist", &roles.strategist)),
        "narrator" => Some(("narrator", &roles.narrator)),
        "sidetalk" | "explain" => Some(("sidetalk", &roles.sidetalk)),
        _ => None,
    }
}

fn lookup_store_role(store: &ProviderKeyStore, mode: StoreRolesMode, role: &str) -> Option<RoleModelInput> {
    if mode == StoreRolesMode::Never {
        return None;
    }
    let defaults = RoleAssignment::default();
    for alias in role_aliases(role) {
        let Some((canon, spec)) = store_role_spec(&store.roles, alias) else { continue };
        if mode == StoreRolesMode::Auto {
            let Some((_, def_spec)) = store_role_spec(&defaults, canon) else { continue };
            if spec == def_spec {
                continue;
            }
        }
        return Some(RoleModelInput {
            provider: spec.provider.cli_name().to_string(),
            model: spec.model.clone(),
            base_url: spec.base_url.clone(),
        });
    }
    None
}

fn is_cli_route(raw_provider: &str) -> bool {
    matches!(
        raw_provider.trim().to_ascii_lowercase().as_str(),
        "claude-cli" | "claude-code-cli" | "antigravity" | "agy"
    )
}

fn validate_model_slug(provider: ProviderKind, raw_model: &str) -> Result<String, String> {
    let m = provider.resolve_model(raw_model);
    if m.is_empty()
        || m.len() > 160
        || m.starts_with('-')
        || m.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("invalid model id `{m}` for `{}`", provider.cli_name()));
    }
    if provider == ProviderKind::Gemini {
        validate_gemini_model(&m).map_err(|e| e.message)?;
    }
    Ok(m)
}

fn validate_candidate(
    role: &str,
    spec: RoleModelInput,
    base_reason: RoutingReason,
    policy: &RoleRoutingPolicy,
    inputs: &RoleRoutingInputs,
) -> ResolvedRole {
    let p_trim = spec.provider.trim().to_string();
    let m_trim = spec.model.trim().to_string();
    if is_cli_route(&p_trim) && !policy.allow_cli_for_workers {
        return ResolvedRole::Unavailable {
            configured_provider: p_trim.clone(),
            configured_model: m_trim,
            error: format!("role `{role}` cannot use subscription CLI route `{p_trim}`"),
        };
    }
    let provider = match ProviderKind::parse(&p_trim) {
        Ok(p) => p,
        Err(e) => {
            return ResolvedRole::Unavailable {
                configured_provider: p_trim,
                configured_model: m_trim,
                error: e,
            };
        }
    };
    let model = match validate_model_slug(provider, &m_trim) {
        Ok(m) => m,
        Err(e) => {
            return ResolvedRole::Unavailable {
                configured_provider: p_trim,
                configured_model: m_trim,
                error: e,
            };
        }
    };
    let role_base = spec.base_url.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Err(err) = check_provider_ready(provider, role_base, inputs) {
        return ResolvedRole::Unavailable {
            configured_provider: provider.cli_name().to_string(),
            configured_model: model,
            error: err,
        };
    }

    let same_as_primary = provider == policy.primary_provider
        && model.eq_ignore_ascii_case(&policy.primary_model);
    if same_as_primary && !policy.allow_same_model_explicit {
        return ResolvedRole::Unavailable {
            configured_provider: provider.cli_name().to_string(),
            configured_model: model,
            error: format!("role `{role}` must differ from primary model `{}`", policy.primary_model),
        };
    }
    let routing_reason = if same_as_primary { RoutingReason::ExplicitSameModel } else { base_reason };
    ResolvedRole::Pinned {
        provider,
        model,
        base_url: role_base.map(str::to_string),
        routing_reason,
    }
}

/// Resolve a single role strictly without silent fallback to `primary_model`.
pub fn resolve_role(role: &str, policy: &RoleRoutingPolicy, inputs: &RoleRoutingInputs) -> ResolvedRole {
    if let Some(spec) = lookup_map_with_aliases(&inputs.run_overrides, role) {
        return validate_candidate(role, spec, RoutingReason::ExplicitRole, policy, inputs);
    }
    if let Some(spec) = lookup_map_with_aliases(&inputs.agent_models, role) {
        return validate_candidate(role, spec, RoutingReason::ExplicitRole, policy, inputs);
    }
    if let Some(spec) = lookup_map_with_aliases(&inputs.role_presets, role) {
        return validate_candidate(role, spec, RoutingReason::PresetRole, policy, inputs);
    }
    if let Some(spec) = lookup_store_role(&inputs.store, inputs.store_roles_mode, role) {
        return validate_candidate(role, spec, RoutingReason::PresetRole, policy, inputs);
    }

    if matches!(role.trim(), ROLE_ORCHESTRATOR | ROLE_SOLVER) {
        return validate_candidate(
            role,
            RoleModelInput::new(policy.primary_provider.cli_name(), policy.primary_model.clone()),
            RoutingReason::ExplicitRole,
            policy,
            inputs,
        );
    }

    let catalog_slugs = inputs
        .catalog_models
        .get(policy.primary_provider.cli_name())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if let Some(smaller) =
        pick_smaller_tier_model(policy.primary_provider, &policy.primary_model, catalog_slugs)
    {
        if check_provider_ready(policy.primary_provider, None, inputs).is_ok() {
            return ResolvedRole::Pinned {
                provider: policy.primary_provider,
                model: smaller,
                base_url: None,
                routing_reason: RoutingReason::AutoSmallerTier,
            };
        }
    }

    if policy.allow_frontier_fallback {
        if let Err(err) = check_provider_ready(policy.primary_provider, None, inputs) {
            return ResolvedRole::Unavailable {
                configured_provider: policy.primary_provider.cli_name().to_string(),
                configured_model: policy.primary_model.clone(),
                error: err,
            };
        }
        return ResolvedRole::Pinned {
            provider: policy.primary_provider,
            model: policy.primary_model.clone(),
            base_url: None,
            routing_reason: RoutingReason::ExplicitFrontierOptIn,
        };
    }

    ResolvedRole::Disabled { reason: REASON_NO_SMALLER_MODEL }
}

/// Resolve a slice of role keys in one deterministic pass.
pub fn resolve_run_roles(
    roles: &[&str],
    policy: &RoleRoutingPolicy,
    inputs: &RoleRoutingInputs,
) -> BTreeMap<String, ResolvedRole> {
    roles
        .iter()
        .map(|&role| (role.to_string(), resolve_role(role, policy, inputs)))
        .collect()
}

/// Convert resolved roles into serializable [`PinnedRoleRecord`] entries for ledger markers.
pub fn to_pinned_records(
    resolved: &BTreeMap<String, ResolvedRole>,
) -> BTreeMap<String, PinnedRoleRecord> {
    resolved
        .iter()
        .map(|(k, v)| (k.clone(), PinnedRoleRecord::from_resolved(k, v)))
        .collect()
}

/// Construct a concrete [`Transport`] for a pinned role using provider-scoped credentials
/// and role/store `base_url` precedence.
pub fn build_role_transport(
    resolved: &ResolvedRole,
    inputs: &RoleRoutingInputs,
    chatgpt_bearer: Option<Arc<dyn BearerSource>>,
) -> Result<(Box<dyn Transport>, String), String> {
    let (provider, model, role_base) = match resolved {
        ResolvedRole::Pinned { provider, model, base_url, .. } => {
            (*provider, model.clone(), base_url.as_deref())
        }
        ResolvedRole::Disabled { reason } => {
            return Err(format!("role is disabled ({reason})"));
        }
        ResolvedRole::Unavailable { configured_provider, configured_model, error } => {
            return Err(format!(
                "role `{configured_provider}/{configured_model}` unavailable: {error}"
            ));
        }
    };

    let base_url = effective_base_url(provider, role_base, inputs)?;
    let stream = inputs.api_base_override.is_none() && base_url.starts_with("https://");
    let scoped =
        resolve_provider_scoped_api_key(provider, &base_url, &inputs.store, inputs.env.as_ref())?;

    let transport: Box<dyn Transport> = match provider {
        ProviderKind::ChatGpt => {
            let bearer = chatgpt_bearer
                .ok_or_else(|| "ChatGPT BearerSource is not connected".to_string())?;
            let cfg = ChatGptConfig { base_url, ..ChatGptConfig::default() };
            Box::new(ChatGptTransport::new(cfg, bearer).map_err(|e| e.to_string())?)
        }
        ProviderKind::Anthropic => {
            let key = scoped.key.ok_or_else(|| "Anthropic transport requires an API key".to_string())?;
            let cfg = AnthropicConfig { base_url, stream, ..AnthropicConfig::default() };
            Box::new(
                AnthropicTransport::new(cfg, key, scoped.source_label).map_err(|e| e.to_string())?,
            )
        }
        ProviderKind::OpenAi | ProviderKind::Ollama => {
            let cfg = OpenAiConfig { provider, base_url, stream, ..OpenAiConfig::default() };
            Box::new(
                OpenAiTransport::new(cfg, scoped.key, scoped.source_label)
                    .map_err(|e| e.to_string())?,
            )
        }
        ProviderKind::Gemini => {
            let key = scoped.key.ok_or_else(|| "Gemini transport requires an API key".to_string())?;
            let cfg = GeminiConfig { base_url, model: model.clone(), ..GeminiConfig::default() };
            Box::new(GeminiTransport::new(cfg, key, scoped.source_label).map_err(|e| e.message)?)
        }
    };
    Ok((transport, model))
}

#[cfg(test)]
#[path = "role_routing_tests.rs"]
mod tests;
