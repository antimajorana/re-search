//! Live model lists: one bounded, read-only `GET` per provider, for the model picker.
//!
//! Each call goes only to the provider's configured base URL (the same value the
//! transport would use), with the same rules as the transports: `https://` except on
//! loopback, no redirects, bounded response size, the key sent only in a header (Gemini's
//! in `x-goog-api-key`, never in the URL). Errors carry only a status class
//! ([`describe_list_error`]): an error body is never read, so a provider message quoting
//! part of a key cannot reach the UI. Which key a list call may use is the caller's
//! choice; the desktop uses only provider-specific credentials
//! ([`ProviderKind::own_key_env_vars`]).
//!
//! Model ids and names come from the provider and are untrusted text: ids are kept only
//! when they are short and free of whitespace and control characters (Gemini ids also
//! pass the transport's own model-name check), labels are stripped of control characters
//! and shortened, and the list is capped.

use crate::gemini::validate_gemini_model;
use crate::http::{build_ureq_agent, config_err, get_json, validate_base_url, SecretKey};
use crate::transport::{ProviderKind, TransportError, TransportErrorKind};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

/// Largest model-list response accepted (bytes).
pub const MAX_LIST_BYTES: u64 = 4 * 1024 * 1024;
/// Most models kept from one list.
pub const MAX_MODELS: usize = 500;
const MAX_ID_CHARS: usize = 160;
const MAX_LABEL_CHARS: usize = 120;

/// One model a provider offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedModel {
    pub id: String,
    pub label: String,
    /// Creation (or, for Ollama, modification) time in Unix seconds, when the API gives one.
    pub created_s: Option<i64>,
}

/// The URL the list is fetched from (no secret in it). `Err` when the base URL is refused.
pub fn models_url(provider: ProviderKind, base_url: &str) -> Result<String, TransportError> {
    let v = validate_base_url(base_url)?;
    let root = format!("{}://{}", v.scheme, v.authority);
    let base = v.base_path.as_str();
    Ok(match provider {
        ProviderKind::Anthropic => {
            let b = base.strip_suffix("/v1/messages").or_else(|| base.strip_suffix("/v1")).unwrap_or(base);
            format!("{root}{b}/v1/models?limit=1000")
        }
        ProviderKind::OpenAi => {
            let b = base.strip_suffix("/chat/completions").unwrap_or(base);
            if b.ends_with("/v1") {
                format!("{root}{b}/models")
            } else {
                format!("{root}{b}/v1/models")
            }
        }
        ProviderKind::Gemini => {
            if base.ends_with("/v1beta") {
                format!("{root}{base}/models?pageSize=1000")
            } else {
                format!("{root}{base}/v1beta/models?pageSize=1000")
            }
        }
        ProviderKind::Ollama => {
            let b = base.strip_suffix("/chat/completions").unwrap_or(base);
            let b = b.strip_suffix("/v1").unwrap_or(b);
            format!("{root}{b}/api/tags")
        }
    })
}

/// Fetch and parse the provider's model list. `key` is required except for Ollama and
/// for OpenAI-compatible servers on plaintext loopback (as for the transports).
pub fn list_models(
    provider: ProviderKind,
    base_url: &str,
    key: Option<&SecretKey>,
    timeout: Duration,
) -> Result<Vec<ListedModel>, TransportError> {
    let v = validate_base_url(base_url)?;
    let url = models_url(provider, base_url)?;
    let secret = key.map(SecretKey::as_str).unwrap_or("");
    let keyless_ok = provider == ProviderKind::Ollama || (provider == ProviderKind::OpenAi && v.is_loopback_http);
    if secret.is_empty() && !keyless_ok {
        return Err(config_err(format!("no API key configured for {}", provider.cli_name())));
    }
    let bearer = format!("Bearer {secret}");
    let mut headers: Vec<(&str, &str)> = Vec::new();
    match provider {
        ProviderKind::Anthropic => {
            headers.push(("x-api-key", secret));
            headers.push(("anthropic-version", crate::anthropic::ANTHROPIC_VERSION));
        }
        ProviderKind::Gemini => headers.push(("x-goog-api-key", secret)),
        ProviderKind::OpenAi | ProviderKind::Ollama => {
            if !secret.is_empty() {
                headers.push(("Authorization", bearer.as_str()));
            }
        }
    }
    let agent = build_ureq_agent(v.is_https, timeout.min(Duration::from_secs(10)), timeout);
    let raw = get_json(&agent, &url, &headers, MAX_LIST_BYTES)?;
    let mut list = parse_model_list(provider, &raw).map_err(|_| TransportError {
        kind: TransportErrorKind::MalformedResponse,
        status: Some(200),
        message: "unexpected response".to_string(),
        raw_body: None,
    })?;
    if provider == ProviderKind::OpenAi && is_official_openai(&v.authority, v.is_https) {
        list.retain(|m| is_openai_official_chat_model(&m.id));
    }
    Ok(list)
}

/// A list error as shown to the person: only its class (`auth (HTTP 401)`,
/// `rate limit (HTTP 429)`, `HTTP n`, `timeout`, `network`, `oversized response`,
/// `unexpected response`), never text from the provider. Configuration errors are
/// re:Search's own messages and are kept.
pub fn describe_list_error(e: &TransportError) -> String {
    let code = e.status.map(|s| format!(" (HTTP {s})")).unwrap_or_default();
    match e.kind {
        TransportErrorKind::Auth => format!("auth{code}"),
        TransportErrorKind::RateLimit => format!("rate limit{code}"),
        TransportErrorKind::HttpStatus => e.status.map(|s| format!("HTTP {s}")).unwrap_or_else(|| "HTTP error".into()),
        TransportErrorKind::Network => if e.message == "timeout" { "timeout" } else { "network" }.to_string(),
        TransportErrorKind::OversizedResponse => "oversized response".to_string(),
        TransportErrorKind::MalformedResponse => "unexpected response".to_string(),
        TransportErrorKind::Config => e.message.clone(),
    }
}

fn is_official_openai(authority: &str, is_https: bool) -> bool {
    is_https && authority.eq_ignore_ascii_case("api.openai.com")
}

/// OpenAI's `/v1/models` lists every model the key can use, including embeddings, audio,
/// image and moderation models. Keep only ids that look like chat models: a conservative
/// deny-list on the id, applied to every OpenAI-compatible server.
pub fn is_openai_chat_model(id: &str) -> bool {
    const NOT_CHAT: &[&str] = &[
        "embedding", "whisper", "tts", "dall-e", "moderation", "realtime", "audio", "image", "transcribe",
        "babbage", "davinci", "sora", "search", "computer-use",
    ];
    let lower = id.to_ascii_lowercase();
    !NOT_CHAT.iter().any(|w| lower.contains(w))
}

/// Id fragments of official OpenAI models that `/chat/completions` (the only endpoint the
/// OpenAI transport calls) does not serve: Responses-only models (`o1-pro`, `o3-pro`,
/// `o3-deep-research`, `codex-mini-latest`) and completion-only ones
/// (`gpt-3.5-turbo-instruct`). A deny-list, applied only for `https://api.openai.com`:
/// another OpenAI-compatible server may serve such names through chat.
pub const OPENAI_OFFICIAL_NOT_CHAT: &[&str] = &["-pro", "deep-research", "codex", "instruct"];

/// [`is_openai_chat_model`] plus [`OPENAI_OFFICIAL_NOT_CHAT`], for the official API.
pub fn is_openai_official_chat_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    is_openai_chat_model(id) && !OPENAI_OFFICIAL_NOT_CHAT.iter().any(|w| lower.contains(w))
}

fn clean_id(raw: &str) -> Option<String> {
    let t = raw.trim();
    let ok = !t.is_empty()
        && t.chars().count() <= MAX_ID_CHARS
        && !t.starts_with('-')
        && !t.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| t.to_string())
}

fn clean_label(raw: Option<&str>, id: &str) -> String {
    let text: String = raw
        .unwrap_or(id)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let text = if text.is_empty() { id.to_string() } else { text };
    text.chars().take(MAX_LABEL_CHARS).collect()
}

/// Parse a list body. Pure, so it is unit-tested without a network.
pub fn parse_model_list(provider: ProviderKind, body: &[u8]) -> Result<Vec<ListedModel>, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("model list is not JSON: {e}"))?;
    let (items, list_key) = match provider {
        ProviderKind::Anthropic | ProviderKind::OpenAi => (v.get("data"), "data"),
        ProviderKind::Gemini | ProviderKind::Ollama => (v.get("models"), "models"),
    };
    let items = items
        .and_then(Value::as_array)
        .ok_or_else(|| format!("model list has no `{list_key}` array"))?;
    let mut out: Vec<ListedModel> = Vec::new();
    for item in items {
        let entry = match provider {
            ProviderKind::Anthropic => item["id"].as_str().and_then(clean_id).map(|id| ListedModel {
                label: clean_label(item["display_name"].as_str(), &id),
                created_s: item["created_at"].as_str().and_then(rfc3339_seconds),
                id,
            }),
            ProviderKind::OpenAi => item["id"]
                .as_str()
                .and_then(clean_id)
                .filter(|id| is_openai_chat_model(id))
                .map(|id| ListedModel { label: clean_label(None, &id), created_s: item["created"].as_i64(), id }),
            ProviderKind::Gemini => {
                let generates = item["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|m| m.iter().any(|x| x.as_str() == Some("generateContent")));
                item["name"]
                    .as_str()
                    .filter(|_| generates)
                    .map(|n| n.strip_prefix("models/").unwrap_or(n))
                    .and_then(clean_id)
                    .filter(|id| validate_gemini_model(id).is_ok())
                    .map(|id| ListedModel {
                        label: clean_label(item["displayName"].as_str(), &id),
                        created_s: None,
                        id,
                    })
            }
            ProviderKind::Ollama => item["name"].as_str().or(item["model"].as_str()).and_then(clean_id).map(|id| {
                ListedModel {
                    label: clean_label(None, &id),
                    created_s: item["modified_at"].as_str().and_then(rfc3339_seconds),
                    id,
                }
            }),
        };
        if let Some(m) = entry {
            if !out.iter().any(|o| o.id == m.id) {
                out.push(m);
            }
        }
        if out.len() >= MAX_MODELS {
            break;
        }
    }
    // Newest first where the API gives a time; models without one keep their order, after.
    out.sort_by(|a, b| match (a.created_s, b.created_s) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(out)
}

/// Seconds since 1970 of the date and time at the start of an RFC 3339 string
/// (`YYYY-MM-DDTHH:MM:SS`; fraction and offset ignored, which is precise enough to sort).
pub fn rfc3339_seconds(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // Days from civil (proleptic Gregorian), Howard Hinnant's algorithm.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(list: &[ListedModel]) -> Vec<&str> {
        list.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn urls_follow_the_transport_base_rules_and_carry_no_key() {
        let a = ProviderKind::Anthropic;
        assert_eq!(models_url(a, "https://api.anthropic.com").unwrap(), "https://api.anthropic.com/v1/models?limit=1000");
        assert_eq!(models_url(a, "http://127.0.0.1:9/v1/messages").unwrap(), "http://127.0.0.1:9/v1/models?limit=1000");
        let o = ProviderKind::OpenAi;
        assert_eq!(models_url(o, "https://api.openai.com").unwrap(), "https://api.openai.com/v1/models");
        assert_eq!(models_url(o, "https://h.example/v1").unwrap(), "https://h.example/v1/models");
        assert_eq!(models_url(o, "https://h.example/v1/chat/completions").unwrap(), "https://h.example/v1/models");
        let g = ProviderKind::Gemini;
        assert_eq!(
            models_url(g, "https://generativelanguage.googleapis.com").unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000"
        );
        let l = ProviderKind::Ollama;
        assert_eq!(models_url(l, "http://127.0.0.1:11434/v1").unwrap(), "http://127.0.0.1:11434/api/tags");
        assert_eq!(models_url(l, "http://localhost:11434").unwrap(), "http://localhost:11434/api/tags");
        // Plaintext off loopback, userinfo and queries are refused as for the transports.
        assert!(models_url(o, "http://api.openai.com").is_err());
        assert!(models_url(o, "https://u:p@h.example").is_err());
        assert!(models_url(g, "https://h.example/?key=x").is_err());
    }

    #[test]
    fn keyless_calls_are_refused_except_local_servers() {
        let t = Duration::from_millis(200);
        for p in [ProviderKind::Anthropic, ProviderKind::Gemini] {
            let err = list_models(p, "http://127.0.0.1:9", None, t).unwrap_err();
            assert_eq!(err.kind, TransportErrorKind::Config, "{err}");
        }
        let err = list_models(ProviderKind::OpenAi, "https://api.openai.com", None, t).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "{err}");
    }

    #[test]
    fn anthropic_list_is_sorted_newest_first_with_display_names() {
        let body = json!({ "data": [
            { "id": "claude-old", "display_name": "Old", "created_at": "2024-01-02T00:00:00Z" },
            { "id": "claude-new", "display_name": "New\u{0007} model", "created_at": "2025-06-01T10:00:00Z" },
            { "id": "bad id", "display_name": "x" },
            { "id": "claude-new", "display_name": "dup" }
        ], "has_more": false });
        let list = parse_model_list(ProviderKind::Anthropic, body.to_string().as_bytes()).unwrap();
        assert_eq!(ids(&list), ["claude-new", "claude-old"]);
        assert_eq!(list[0].label, "New model");
    }

    #[test]
    fn openai_keeps_only_chat_models() {
        let body = json!({ "data": [
            { "id": "gpt-4.1-mini", "created": 100 },
            { "id": "text-embedding-3-large", "created": 300 },
            { "id": "whisper-1", "created": 5 },
            { "id": "tts-1-hd", "created": 6 },
            { "id": "dall-e-3", "created": 7 },
            { "id": "omni-moderation-latest", "created": 8 },
            { "id": "gpt-4o-realtime-preview", "created": 9 },
            { "id": "gpt-4o-audio-preview", "created": 10 },
            { "id": "gpt-image-1", "created": 11 },
            { "id": "o3", "created": 200 }
        ]});
        let list = parse_model_list(ProviderKind::OpenAi, body.to_string().as_bytes()).unwrap();
        assert_eq!(ids(&list), ["o3", "gpt-4.1-mini"]);
    }

    #[test]
    fn official_openai_also_drops_models_chat_completions_cannot_run() {
        for id in ["o1-pro", "o3-pro-2025-06-10", "o3-deep-research", "codex-mini-latest", "gpt-3.5-turbo-instruct"] {
            assert!(!is_openai_official_chat_model(id), "{id}");
        }
        // Other OpenAI-compatible servers keep such names ("deep-research" already falls
        // under the general "search" entry).
        for id in ["o1-pro", "codex-mini-latest", "gpt-3.5-turbo-instruct"] {
            assert!(is_openai_chat_model(id), "{id}");
        }
        for id in ["gpt-4.1-mini", "o3", "o4-mini", "gpt-4o-mini-2024-07-18", "gpt-4.1-preview", "chatgpt-4o-latest"] {
            assert!(is_openai_official_chat_model(id), "{id}");
        }
        assert!(!is_openai_official_chat_model("text-embedding-3-large"));
        assert!(is_official_openai("api.openai.com", true));
        assert!(!is_official_openai("api.openai.com", false));
        assert!(!is_official_openai("127.0.0.1:8080", false));
        assert!(!is_official_openai("proxy.example", true));
    }

    #[test]
    fn list_errors_are_described_by_class_only() {
        let e = |kind, status: Option<u16>, message: &str| TransportError {
            kind,
            status,
            message: message.to_string(),
            raw_body: None,
        };
        let k = TransportErrorKind::Auth;
        assert_eq!(describe_list_error(&e(k, Some(401), "Incorrect API key provided: sk-proj-****abcd")), "auth (HTTP 401)");
        assert_eq!(describe_list_error(&e(TransportErrorKind::RateLimit, Some(429), "slow down")), "rate limit (HTTP 429)");
        assert_eq!(describe_list_error(&e(TransportErrorKind::HttpStatus, Some(503), "body text")), "HTTP 503");
        assert_eq!(describe_list_error(&e(TransportErrorKind::Network, None, "timeout")), "timeout");
        assert_eq!(describe_list_error(&e(TransportErrorKind::Network, None, "dns: host x")), "network");
        assert_eq!(describe_list_error(&e(TransportErrorKind::MalformedResponse, Some(200), "x")), "unexpected response");
        assert_eq!(describe_list_error(&e(TransportErrorKind::OversizedResponse, Some(200), "x")), "oversized response");
    }

    #[test]
    fn own_key_variables_exclude_generic_and_other_providers() {
        use crate::transport::GENERIC_KEY_ENV_VARS;
        for p in [ProviderKind::Anthropic, ProviderKind::OpenAi, ProviderKind::Gemini, ProviderKind::Ollama] {
            for name in p.own_key_env_vars() {
                assert!(p.key_env_vars().contains(name), "{name} is also read by the {p:?} transport");
                assert!(!GENERIC_KEY_ENV_VARS.contains(name), "{name}");
            }
            for g in GENERIC_KEY_ENV_VARS {
                assert!(p.key_env_vars().contains(&g), "the transport still reads {g}");
            }
        }
        assert!(!ProviderKind::Ollama.own_key_env_vars().contains(&"OPENAI_API_KEY"));
    }

    #[test]
    fn gemini_keeps_generate_content_models_and_strips_the_prefix() {
        let body = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash",
              "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/evil:generateContent", "supportedGenerationMethods": ["generateContent"] }
        ]});
        let list = parse_model_list(ProviderKind::Gemini, body.to_string().as_bytes()).unwrap();
        assert_eq!(ids(&list), ["gemini-2.5-flash"]);
        assert_eq!(list[0].label, "Gemini 2.5 Flash");
    }

    #[test]
    fn ollama_tags_are_parsed_and_malformed_bodies_refused() {
        let body = json!({ "models": [
            { "name": "llama3:8b", "modified_at": "2024-05-01T10:00:00.5-07:00" },
            { "name": "qwen2.5-coder:7b", "modified_at": "2025-01-01T00:00:00Z" }
        ]});
        let list = parse_model_list(ProviderKind::Ollama, body.to_string().as_bytes()).unwrap();
        assert_eq!(ids(&list), ["qwen2.5-coder:7b", "llama3:8b"]);
        assert!(parse_model_list(ProviderKind::Ollama, b"<html>").is_err());
        assert!(parse_model_list(ProviderKind::OpenAi, b"{\"models\":[]}").is_err());
    }

    #[test]
    fn rfc3339_known_values() {
        assert_eq!(rfc3339_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_seconds("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(rfc3339_seconds("2024-02-29T12:30:15.123+02:00"), Some(1_709_209_815));
        assert_eq!(rfc3339_seconds("2024-13-01T00:00:00Z"), None);
        assert_eq!(rfc3339_seconds("yesterday"), None);
    }
}
