//! Wire-format serialization, deserialization, URL validation, and secret
//! scrubbing helpers for the Anthropic Messages API (`POST /v1/messages`).

use crate::transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, Role, StopReason, ToolSpec, TransportError,
    TransportErrorKind,
};
use serde_json::{json, Value};

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

/// Validate `base_url` and build the sanitized `scheme://host[:port]/v1/messages`
/// endpoint string. Rejects userinfo, queries, fragments, and plaintext `http://`
/// outside loopback (`127.0.0.1`, `[::1]`, `localhost`).
pub(crate) fn validate_and_build_endpoint(base_url: &str) -> Result<String, TransportError> {
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
    match scheme {
        "https" => {}
        "http" => {
            if !is_loopback_host(host) {
                return Err(config_err(format!(
                    "plaintext http:// API base URL is only allowed for loopback hosts (got {host})"
                )));
            }
        }
        other => {
            return Err(config_err(format!(
                "unsupported API base URL scheme `{other}` (expected https)"
            )));
        }
    }

    let base_path = uri.path().trim_end_matches('/');
    if base_path.ends_with("/v1/messages") {
        Ok(format!("{scheme}://{authority}{base_path}"))
    } else if base_path.is_empty() || base_path == "/" {
        Ok(format!("{scheme}://{authority}/v1/messages"))
    } else {
        Ok(format!("{scheme}://{authority}{base_path}/v1/messages"))
    }
}

fn is_loopback_host(host: &str) -> bool {
    let bare = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(host);
    bare.eq_ignore_ascii_case("localhost") || bare == "127.0.0.1" || bare == "::1"
}

pub(crate) fn serialize_request(req: &ModelRequest) -> Result<Vec<u8>, TransportError> {
    if req.model.trim().is_empty() {
        return Err(config_err("model name is empty"));
    }
    if req.max_tokens == 0 {
        return Err(config_err("max_tokens must be >= 1"));
    }
    let mut obj = serde_json::Map::new();
    obj.insert("model".into(), json!(req.model));
    obj.insert("max_tokens".into(), json!(req.max_tokens));
    if let Some(sys) = &req.system {
        if !sys.is_empty() {
            obj.insert("system".into(), json!(sys));
        }
    }
    if !req.tools.is_empty() {
        obj.insert("tools".into(), Value::Array(req.tools.iter().map(encode_tool).collect()));
    }
    obj.insert(
        "messages".into(),
        Value::Array(req.messages.iter().map(encode_message).collect()),
    );
    serde_json::to_vec(&Value::Object(obj))
        .map_err(|e| config_err(format!("failed to serialize request: {e}")))
}

fn encode_tool(spec: &ToolSpec) -> Value {
    json!({
        "name": spec.name,
        "description": spec.description,
        "input_schema": spec.input_schema,
    })
}

fn encode_message(msg: &Message) -> Value {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let blocks: Vec<Value> = msg.content.iter().map(encode_block).collect();
    json!({ "role": role, "content": blocks })
}

fn encode_block(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
        ContentBlock::Thinking { thinking, signature } => {
            let mut m = serde_json::Map::new();
            m.insert("type".into(), json!("thinking"));
            m.insert("thinking".into(), json!(thinking));
            if let Some(sig) = signature {
                m.insert("signature".into(), json!(sig));
            }
            Value::Object(m)
        }
        ContentBlock::RedactedThinking { data } => json!({ "type": "redacted_thinking", "data": data }),
        ContentBlock::ToolUse { id, name, input } => json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
        }),
        ContentBlock::ToolResult { tool_use_id, is_error, content } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "is_error": is_error,
            "content": content,
        }),
        ContentBlock::Other { raw, .. } => raw.clone(),
    }
}

pub(crate) fn parse_response(raw_body: Vec<u8>, secret: &str) -> Result<ModelResponse, TransportError> {
    let clean_bytes = scrub_bytes(&raw_body, secret.as_bytes());
    let malformed = |msg: String| TransportError {
        kind: TransportErrorKind::MalformedResponse,
        status: Some(200),
        message: scrub_text(&msg, secret),
        raw_body: Some(clean_bytes.clone()),
    };

    let parsed: Value =
        serde_json::from_slice(&clean_bytes).map_err(|e| malformed(format!("invalid response JSON: {e}")))?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| malformed("response JSON is not an object".into()))?;

    let role = obj.get("role").and_then(Value::as_str).unwrap_or("");
    if role != "assistant" {
        return Err(malformed(format!("expected assistant role, got `{role}`")));
    }

    let id = obj.get("id").and_then(Value::as_str).map(str::to_string);
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("response missing `model` string".into()))?
        .to_string();
    let stop_str = obj
        .get("stop_reason")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("response missing `stop_reason` string".into()))?;
    let stop_reason = StopReason::from_wire(stop_str);

    let arr = obj
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("response missing `content` array".into()))?;
    let mut content = Vec::with_capacity(arr.len());
    for block in arr {
        content.push(decode_block(block).map_err(&malformed)?);
    }

    let usage = obj.get("usage").cloned().unwrap_or(Value::Null);
    Ok(ModelResponse {
        raw_body: clean_bytes,
        id,
        model,
        stop_reason,
        content,
        usage,
    })
}

fn decode_block(block: &Value) -> Result<ContentBlock, String> {
    let obj = block
        .as_object()
        .ok_or_else(|| "content block is not an object".to_string())?;
    let ty = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "content block missing `type` string".to_string())?;
    match ty {
        "text" => {
            let text = obj
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "text block missing `text` string".to_string())?;
            Ok(ContentBlock::Text { text: text.to_string() })
        }
        "thinking" => {
            let thinking = obj
                .get("thinking")
                .and_then(Value::as_str)
                .ok_or_else(|| "thinking block missing `thinking` string".to_string())?;
            let signature = obj.get("signature").and_then(Value::as_str).map(str::to_string);
            Ok(ContentBlock::Thinking { thinking: thinking.to_string(), signature })
        }
        "redacted_thinking" => {
            let data = obj
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "redacted_thinking block missing `data` string".to_string())?;
            Ok(ContentBlock::RedactedThinking { data: data.to_string() })
        }
        "tool_use" => {
            let id = obj
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "tool_use block missing `id` string".to_string())?;
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "tool_use block missing `name` string".to_string())?;
            let input = obj
                .get("input")
                .cloned()
                .ok_or_else(|| "tool_use block missing `input`".to_string())?;
            Ok(ContentBlock::ToolUse { id: id.to_string(), name: name.to_string(), input })
        }
        "tool_result" => {
            let tool_use_id = obj
                .get("tool_use_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "tool_result block missing `tool_use_id` string".to_string())?;
            let is_error = obj.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let content = obj
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Ok(ContentBlock::ToolResult { tool_use_id: tool_use_id.to_string(), is_error, content })
        }
        other => Ok(ContentBlock::Other { block_type: other.to_string(), raw: block.clone() }),
    }
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
                // Keep exact wire bytes when no string value contained the secret.
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
        let err_type = v["error"]["type"].as_str();
        let err_msg = v["error"]["message"].as_str();
        if let (Some(t), Some(m)) = (err_type, err_msg) {
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
