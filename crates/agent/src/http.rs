//! Shared HTTP client, base-URL validation, `SecretKey` hygiene, and secret
//! scrubbing helpers across all model [`crate::Transport`] implementations.

use crate::transport::{TransportError, TransportErrorKind};
use serde_json::Value;
use std::fmt;
use std::time::Duration;
use zeroize::Zeroize;

pub(crate) const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_ERROR_BYTES: u64 = 1024 * 1024;

pub(crate) fn config_err(message: impl Into<String>) -> TransportError {
    TransportError {
        kind: TransportErrorKind::Config,
        status: None,
        message: message.into(),
        raw_body: None,
    }
}

pub(crate) fn malformed_response_err(clean_bytes: Vec<u8>, secret: &str, msg: impl AsRef<str>) -> TransportError {
    TransportError {
        kind: TransportErrorKind::MalformedResponse,
        status: Some(200),
        message: scrub_text(msg.as_ref(), secret),
        raw_body: Some(clean_bytes),
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
        Self::from_env_vars(
            &["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY"],
            "RE_SEARCH_API_KEY or ANTHROPIC_API_KEY",
        )
    }

    pub fn from_env_vars(names: &[&str], hint: &str) -> Result<(Self, String), TransportError> {
        for &name in names {
            if let Ok(mut val) = std::env::var(name) {
                if !val.trim().is_empty() {
                    return Ok((Self::new(val)?, name.to_string()));
                }
                val.zeroize();
            }
        }
        Err(config_err(format!("no API key configured: set {hint}")))
    }

    pub fn optional_from_env_vars(
        names: &[&str],
        allow_keyless_loopback: bool,
        is_loopback_http: bool,
        hint: &str,
    ) -> Result<(Option<Self>, String), TransportError> {
        for &name in names {
            if let Ok(mut val) = std::env::var(name) {
                if !val.trim().is_empty() {
                    return Ok((Some(Self::new(val)?), name.to_string()));
                }
                val.zeroize();
            }
        }
        if allow_keyless_loopback && is_loopback_http {
            Ok((None, "none".to_string()))
        } else {
            Err(config_err(format!("no API key configured: set {hint}")))
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedBaseUrl {
    pub scheme: String,
    pub authority: String,
    /// Normalized path without trailing slash (`""` when root `/` or empty).
    pub base_path: String,
    pub is_https: bool,
    pub is_loopback_http: bool,
}

/// Validate `base_url`, rejecting userinfo, queries, fragments, and plaintext
/// `http://` outside loopback (`127.0.0.1`, `[::1]`, `localhost`).
pub(crate) fn validate_base_url(base_url: &str) -> Result<ValidatedBaseUrl, TransportError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(config_err("API base URL is empty"));
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(config_err("API base URL must not contain query parameters or fragments"));
    }
    let uri: ureq::http::Uri = trimmed
        .parse()
        .map_err(|e| config_err(format!("invalid API base URL: {e}")))?;
    let scheme = uri
        .scheme_str()
        .ok_or_else(|| config_err("API base URL is missing a scheme"))?;
    let authority = uri
        .authority()
        .ok_or_else(|| config_err("API base URL is missing a host"))?;
    if authority.as_str().contains('@') {
        return Err(config_err("API base URL must not contain userinfo"));
    }
    let host = authority.host();
    if host.is_empty() {
        return Err(config_err("API base URL host is empty"));
    }
    let (is_https, is_loopback_http) = match scheme {
        "https" => (true, false),
        "http" => {
            if !is_loopback_host(host) {
                return Err(config_err(format!(
                    "plaintext http:// API base URL is only allowed for loopback hosts (got {host})"
                )));
            }
            (false, true)
        }
        other => {
            return Err(config_err(format!(
                "unsupported API base URL scheme `{other}` (expected https)"
            )));
        }
    };
    let raw_path = uri.path().trim_end_matches('/');
    let base_path = if raw_path == "/" { String::new() } else { raw_path.to_string() };
    Ok(ValidatedBaseUrl {
        scheme: scheme.to_string(),
        authority: authority.to_string(),
        base_path,
        is_https,
        is_loopback_http,
    })
}

fn is_loopback_host(host: &str) -> bool {
    let bare = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(host);
    bare.eq_ignore_ascii_case("localhost") || bare == "127.0.0.1" || bare == "::1"
}

pub(crate) fn build_ureq_agent(
    is_https: bool,
    connect_timeout: Duration,
    request_timeout: Duration,
) -> ureq::Agent {
    let mut builder = ureq::Agent::config_builder()
        .https_only(is_https)
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(connect_timeout))
        .timeout_global(Some(request_timeout));
    if !is_https {
        builder = builder.proxy(None);
    }
    ureq::Agent::new_with_config(builder.build())
}

fn map_ureq_error(err: ureq::Error, status: Option<u16>, secret: &str) -> TransportError {
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

pub(crate) fn post_json(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    secret: &str,
) -> Result<Vec<u8>, TransportError> {
    let mut req = agent.post(url);
    for &(name, val) in headers {
        req = req.header(name, val);
    }
    let mut resp = req
        .content_type("application/json")
        .send(body)
        .map_err(|e| map_ureq_error(e, None, secret))?;

    let status = resp.status().as_u16();
    let limit = if status == 200 { MAX_RESPONSE_BYTES } else { MAX_ERROR_BYTES };
    let raw = resp
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| map_ureq_error(e, Some(status), secret))?;

    if status == 200 {
        return Ok(raw);
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

pub(crate) fn scrub_text(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    let out = text.replace(secret, "[REDACTED]");
    if let Ok(quoted) = serde_json::to_string(secret) {
        let escaped = &quoted[1..quoted.len() - 1];
        if escaped != secret && !escaped.is_empty() {
            return out.replace(escaped, "[REDACTED]");
        }
    }
    out
}

fn scrub_json_strings(v: &mut Value, secret: &str) -> bool {
    match v {
        Value::String(s) => {
            if s.contains(secret) {
                *s = s.replace(secret, "[REDACTED]");
                true
            } else {
                false
            }
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                if scrub_json_strings(item, secret) {
                    changed = true;
                }
            }
            changed
        }
        Value::Object(map) => {
            let mut changed = false;
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (k, mut val) in entries {
                let clean_k = if k.contains(secret) {
                    changed = true;
                    k.replace(secret, "[REDACTED]")
                } else {
                    k
                };
                if scrub_json_strings(&mut val, secret) {
                    changed = true;
                }
                map.insert(clean_k, val);
            }
            changed
        }
        _ => false,
    }
}

pub(crate) fn scrub_bytes(input: &[u8], needle: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return input.to_vec();
    }
    if let Ok(secret_str) = std::str::from_utf8(needle) {
        if let Ok(mut parsed) = serde_json::from_slice::<Value>(input) {
            if scrub_json_strings(&mut parsed, secret_str) {
                if let Ok(clean_json) = serde_json::to_vec(&parsed) {
                    return clean_json;
                }
            } else {
                return input.to_vec();
            }
        }
    }
    replace_subslice(input, needle, b"[REDACTED]")
}

fn replace_subslice(input: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if needle.is_empty() || input.len() < needle.len() {
        return input.to_vec();
    }
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i + needle.len() <= input.len() {
        if &input[i..i + needle.len()] == needle {
            out.extend_from_slice(replacement);
            i += needle.len();
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    out.extend_from_slice(&input[i..]);
    out
}

pub(crate) fn summarize_error_body(status: u16, clean_body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<Value>(clean_body) {
        if let Some(s) = v["error"].as_str() {
            return format!("HTTP {status}: {s}");
        }
        let err_tag = v["error"]["type"].as_str().or_else(|| v["error"]["status"].as_str());
        let err_msg = v["error"]["message"].as_str();
        if let (Some(t), Some(m)) = (err_tag, err_msg) {
            return format!("HTTP {status} ({t}): {m}");
        }
        if let Some(m) = err_msg {
            return format!("HTTP {status}: {m}");
        }
    }
    let text = String::from_utf8_lossy(clean_body);
    let snippet: String = text.trim().chars().take(256).collect();
    if snippet.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {snippet}")
    }
}
