//! Role model routing presets (F11) across Anthropic, OpenAI, Ollama, and Gemini.

use crate::transport::ProviderKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEndpointSpec {
    pub provider: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
}

impl ModelEndpointSpec {
    pub fn new(provider: ProviderKind, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
            base_url: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub solver: ModelEndpointSpec,
    pub step_verifier: ModelEndpointSpec,
    pub full_verifier: ModelEndpointSpec,
    pub strategist: ModelEndpointSpec,
    pub narrator: ModelEndpointSpec,
    pub sidetalk: ModelEndpointSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetKind {
    Balanced,
    FastLocal,
    HighRigor,
}

impl PresetKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "balanced" | "default" => Ok(Self::Balanced),
            "fast_local" | "local" | "ollama" => Ok(Self::FastLocal),
            "high_rigor" | "rigor" => Ok(Self::HighRigor),
            other => Err(format!(
                "unknown preset `{other}` (expected `balanced`, `fast_local`, or `high_rigor`)"
            )),
        }
    }
}

impl RoleAssignment {
    pub fn for_provider(provider: ProviderKind) -> Self {
        let model = match provider {
            ProviderKind::Anthropic => "claude-haiku-4-5",
            ProviderKind::OpenAi => "gpt-4.1-mini",
            ProviderKind::Ollama => "qwen2.5-coder:7b",
            ProviderKind::Gemini => "gemini-2.5-flash",
        };
        let spec = ModelEndpointSpec::new(provider, model);
        Self {
            solver: spec.clone(),
            step_verifier: spec.clone(),
            full_verifier: spec.clone(),
            strategist: spec.clone(),
            narrator: spec.clone(),
            sidetalk: spec,
        }
    }

    pub fn from_preset(kind: PresetKind) -> Self {
        match kind {
            PresetKind::Balanced => Self::for_provider(ProviderKind::Anthropic),
            PresetKind::FastLocal => Self::for_provider(ProviderKind::Ollama),
            PresetKind::HighRigor => Self {
                solver: ModelEndpointSpec::new(ProviderKind::Anthropic, "claude-sonnet-4-5"),
                step_verifier: ModelEndpointSpec::new(ProviderKind::Anthropic, "claude-haiku-4-5"),
                full_verifier: ModelEndpointSpec::new(ProviderKind::Anthropic, "claude-sonnet-4-5"),
                strategist: ModelEndpointSpec::new(ProviderKind::OpenAi, "gpt-4.1-mini"),
                narrator: ModelEndpointSpec::new(ProviderKind::Gemini, "gemini-2.5-flash"),
                sidetalk: ModelEndpointSpec::new(ProviderKind::Anthropic, "claude-haiku-4-5"),
            },
        }
    }
}

impl Default for RoleAssignment {
    fn default() -> Self {
        Self::from_preset(PresetKind::Balanced)
    }
}
