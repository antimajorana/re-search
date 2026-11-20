//! Anthropic Messages API (`POST /v1/messages`) transport implementation.

use crate::transport::{
    ModelRequest, ModelResponse, PreparedRequest, Transport, TransportError, TransportErrorKind, TransportInfo,
};
use crate::wire::{
    config_err, parse_response, scrub_bytes, scrub_text, serialize_request, summarize_error_body,
    validate_and_build_endpoint, MAX_ERROR_BYTES, MAX_RESPONSE_BYTES,
};
use std::fmt;
use std::time::Duration;
use zeroize::Zeroize;

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

pub struct SecretKey(String);

impl SecretKey {
    pub fn new(mut raw: String) -> Result<Self, TransportError> {
        let trimmed = raw.trim().to_string();
        raw.zeroize();
        if trimmed.is_empty() {
            return Err(config_err("API key is empty"));
        }
        if trimmed.bytes().any(|b| b < 0x20 || b == 0x7f) {
            let mut bad = trimmed;
            bad.zeroize();
            return Err(config_err("API key contains control characters"));
        }
        Ok(SecretKey(trimmed))
    }

    pub fn from_env() -> Result<(Self, String), TransportError> {
        for name in ["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY"] {
            if let Ok(mut val) = std::env::var(name) {
                if !val.trim().is_empty() {
                    return Ok((Self::new(val)?, name.to_string()));
                }
                val.zeroize();
            }
        }
        Err(config_err(
            "no API key configured: set RE_SEARCH_API_KEY or ANTHROPIC_API_KEY",
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey([REDACTED])")
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.0.zeroize();
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
        let mut builder = ureq::Agent::config_builder()
            .https_only(is_https)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(config.connect_timeout))
            .timeout_global(Some(config.request_timeout));
        if !is_https {
            builder = builder.proxy(None);
        }
        let ureq_cfg = builder.build();
        Ok(Self {
            endpoint,
            key,
            key_env: key_env.into(),
            agent: ureq::Agent::new_with_config(ureq_cfg),
        })
    }

    fn map_ureq_error(&self, err: ureq::Error, status: Option<u16>) -> TransportError {
        let secret = self.key.as_str();
        let kind = match &err {
            ureq::Error::BodyExceedsLimit(_) | ureq::Error::LargeResponseHeader(_, _) => {
                TransportErrorKind::OversizedResponse
            }
            ureq::Error::BadUri(_) | ureq::Error::InvalidProxyUrl => TransportErrorKind::Config,
            _ => TransportErrorKind::Network,
        };
        TransportError {
            kind,
            status,
            message: scrub_text(&err.to_string(), secret),
            raw_body: None,
        }
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
        let mut resp = self
            .agent
            .post(&self.endpoint)
            .header("x-api-key", secret)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .content_type("application/json")
            .send(prepared.body.as_slice())
            .map_err(|e| self.map_ureq_error(e, None))?;

        let status = resp.status().as_u16();
        let limit = if status == 200 { MAX_RESPONSE_BYTES } else { MAX_ERROR_BYTES };
        let raw = resp
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| self.map_ureq_error(e, Some(status)))?;

        if status == 200 {
            return parse_response(raw, secret);
        }

        let clean_body = scrub_bytes(&raw, secret.as_bytes());
        let kind = match status {
            401 | 403 => TransportErrorKind::Auth,
            429 => TransportErrorKind::RateLimit,
            _ => TransportErrorKind::HttpStatus,
        };
        let message = scrub_text(&summarize_error_body(status, &clean_body), secret);
        Err(TransportError {
            kind,
            status: Some(status),
            message,
            raw_body: Some(clean_body),
        })
    }
}
