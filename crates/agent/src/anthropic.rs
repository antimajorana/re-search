//! Anthropic Messages API (`POST /v1/messages`) transport implementation.

pub use crate::http::SecretKey;
use crate::http::{build_ureq_agent, post_json};
use crate::transport::{ModelRequest, ModelResponse, PreparedRequest, Transport, TransportError, TransportInfo};
use crate::wire::{parse_response, serialize_request, validate_and_build_endpoint};
use std::time::Duration;

pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";
pub const DEFAULT_API_MODEL: &str = "claude-haiku-4-5";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Map CLI shorthand `"haiku"` to [`DEFAULT_API_MODEL`]; pass explicit IDs verbatim.
pub fn resolve_api_model(requested: &str) -> String {
    match requested.trim() {
        "" | "haiku" => DEFAULT_API_MODEL.to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub base_url: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_API_BASE.to_string(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(180),
        }
    }
}

pub struct AnthropicTransport {
    endpoint: String,
    key: SecretKey,
    key_env: String,
    agent: ureq::Agent,
}

impl AnthropicTransport {
    pub fn from_env(config: AnthropicConfig) -> Result<Self, TransportError> {
        let (key, key_env) = SecretKey::from_env()?;
        Self::new(config, key, key_env)
    }

    pub fn new(config: AnthropicConfig, key: SecretKey, key_env: impl Into<String>) -> Result<Self, TransportError> {
        let endpoint = validate_and_build_endpoint(&config.base_url)?;
        let is_https = endpoint.starts_with("https://");
        Ok(Self {
            endpoint,
            key,
            key_env: key_env.into(),
            agent: build_ureq_agent(is_https, config.connect_timeout, config.request_timeout),
        })
    }
}

impl Transport for AnthropicTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "anthropic-messages",
            endpoint: self.endpoint.clone(),
            api_key_env: self.key_env.clone(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest { body: serialize_request(req)? })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let secret = self.key.as_str();
        let headers = [("x-api-key", secret), ("anthropic-version", ANTHROPIC_VERSION)];
        let raw = post_json(&self.agent, &self.endpoint, &headers, &prepared.body, secret)?;
        parse_response(raw, secret)
    }
}
