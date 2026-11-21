//! OpenAI Chat Completions (`POST /v1/chat/completions`) and Ollama preset transport.

use crate::http::{
    build_ureq_agent, config_err, malformed_response_err, post_json, scrub_bytes, validate_base_url, SecretKey,
};
use crate::transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, PreparedRequest, ProviderKind, Role, StopReason, ToolSpec,
    Transport, TransportError, TransportInfo, DEFAULT_OPENAI_BASE,
};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub provider: ProviderKind,
    pub base_url: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OpenAi,
            base_url: DEFAULT_OPENAI_BASE.to_string(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(180),
        }
    }
}

impl OpenAiConfig {
    pub fn ollama_default() -> Self {
        Self {
            provider: ProviderKind::Ollama,
            base_url: ProviderKind::Ollama.default_base_url().to_string(),
            ..Self::default()
        }
    }
}

pub struct OpenAiTransport {
    provider: ProviderKind,
    endpoint: String,
    key: Option<SecretKey>,
    key_env: String,
    agent: ureq::Agent,
}

impl OpenAiTransport {
    pub fn from_env(config: OpenAiConfig) -> Result<Self, TransportError> {
        let validated = validate_base_url(&config.base_url)?;
        let (key, key_env) = match config.provider {
            ProviderKind::OpenAi => SecretKey::optional_from_env_vars(
                ProviderKind::OpenAi.key_env_vars(),
                true,
                validated.is_loopback_http,
                "RE_SEARCH_API_KEY or OPENAI_API_KEY",
            )?,
            ProviderKind::Ollama => SecretKey::optional_from_env_vars(
                ProviderKind::Ollama.key_env_vars(),
                true,
                validated.is_loopback_http,
                "RE_SEARCH_API_KEY or OLLAMA_API_KEY (or use http://127.0.0.1:11434/v1)",
            )?,
            other => {
                return Err(config_err(format!(
                    "OpenAiTransport requires `openai` or `ollama` provider, got `{}`",
                    other.cli_name()
                )));
            }
        };
        Self::new(config, key, key_env)
    }

    pub fn new(
        config: OpenAiConfig,
        key: Option<SecretKey>,
        key_env: impl Into<String>,
    ) -> Result<Self, TransportError> {
        if !matches!(config.provider, ProviderKind::OpenAi | ProviderKind::Ollama) {
            return Err(config_err(format!(
                "OpenAiTransport requires `openai` or `ollama` provider, got `{}`",
                config.provider.cli_name()
            )));
        }
        let validated = validate_base_url(&config.base_url)?;
        if key.is_none() && !validated.is_loopback_http {
            return Err(config_err("API key is required for non-loopback OpenAI/Ollama endpoint"));
        }
        let endpoint = build_openai_endpoint(&validated.scheme, &validated.authority, &validated.base_path);
        let agent = build_ureq_agent(validated.is_https, config.connect_timeout, config.request_timeout);
        Ok(Self {
            provider: config.provider,
            endpoint,
            key,
            key_env: key_env.into(),
            agent,
        })
    }
}

fn build_openai_endpoint(scheme: &str, authority: &str, base_path: &str) -> String {
    if base_path.ends_with("/v1/chat/completions") || base_path.ends_with("/chat/completions") {
        format!("{scheme}://{authority}{base_path}")
    } else if base_path.ends_with("/v1") {
        format!("{scheme}://{authority}{base_path}/chat/completions")
    } else if base_path.is_empty() {
        format!("{scheme}://{authority}/v1/chat/completions")
    } else {
        format!("{scheme}://{authority}{base_path}/v1/chat/completions")
    }
}

impl Transport for OpenAiTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: self.provider.wire_id(),
            endpoint: self.endpoint.clone(),
            api_key_env: self.key_env.clone(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest {
            body: serialize_openai_request(self.provider, req)?,
        })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        match &self.key {
            Some(key) => {
                let secret = key.as_str();
                let auth = format!("Bearer {secret}");
                let headers = [("Authorization", auth.as_str())];
                let raw = post_json(&self.agent, &self.endpoint, &headers, &prepared.body, secret)?;
                parse_openai_response(raw, secret)
            }
            None => {
                let raw = post_json(&self.agent, &self.endpoint, &[], &prepared.body, "")?;
                parse_openai_response(raw, "")
            }
        }
    }
}

fn serialize_openai_request(provider: ProviderKind, req: &ModelRequest) -> Result<Vec<u8>, TransportError> {
    if req.model.trim().is_empty() {
        return Err(config_err("model name is empty"));
    }
    if req.max_tokens == 0 {
        return Err(config_err("max_tokens must be >= 1"));
    }

    let mut obj = serde_json::Map::new();
    obj.insert("model".into(), json!(req.model));
    match provider {
        ProviderKind::OpenAi => {
            obj.insert("max_completion_tokens".into(), json!(req.max_tokens));
        }
        ProviderKind::Ollama => {
            obj.insert("max_tokens".into(), json!(req.max_tokens));
        }
        _ => {
            return Err(config_err("unsupported provider for OpenAiTransport"));
        }
    }

    if !req.tools.is_empty() {
        obj.insert("tools".into(), Value::Array(req.tools.iter().map(encode_tool).collect()));
    }

    let mut wire_messages = Vec::new();
    if let Some(sys) = &req.system {
        if !sys.is_empty() {
            wire_messages.push(json!({ "role": "system", "content": sys }));
        }
    }
    for msg in &req.messages {
        encode_message(msg, &mut wire_messages)?;
    }
    obj.insert("messages".into(), Value::Array(wire_messages));

    serde_json::to_vec(&Value::Object(obj))
        .map_err(|e| config_err(format!("failed to serialize OpenAI request: {e}")))
}

fn encode_tool(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.input_schema,
        }
    })
}

fn encode_message(msg: &Message, out: &mut Vec<Value>) -> Result<(), TransportError> {
    match msg.role {
        Role::User => {
            let mut text_parts: Vec<&str> = Vec::new();
            for block in &msg.content {
                match block {
                    ContentBlock::ToolResult { tool_use_id, is_error: _, content } => {
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": content,
                        }));
                    }
                    ContentBlock::Text { text } => text_parts.push(text.as_str()),
                    ContentBlock::Thinking { .. }
                    | ContentBlock::RedactedThinking { .. }
                    | ContentBlock::ToolUse { .. }
                    | ContentBlock::Other { .. } => {}
                }
            }
            if !text_parts.is_empty() {
                let joined = text_parts.join("\n");
                if !joined.is_empty() {
                    out.push(json!({ "role": "user", "content": joined }));
                }
            }
        }
        Role::Assistant => {
            let mut text_parts: Vec<&str> = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.as_str()),
                    ContentBlock::ToolUse { id, name, input } => {
                        let args_str = serde_json::to_string(input)
                            .map_err(|e| config_err(format!("failed to encode tool input: {e}")))?;
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args_str,
                            }
                        }));
                    }
                    ContentBlock::Thinking { .. }
                    | ContentBlock::RedactedThinking { .. }
                    | ContentBlock::ToolResult { .. }
                    | ContentBlock::Other { .. } => {}
                }
            }
            let text_joined = text_parts.join("\n");
            let mut m = serde_json::Map::new();
            m.insert("role".into(), json!("assistant"));
            if !text_joined.is_empty() {
                m.insert("content".into(), json!(text_joined));
            } else if !tool_calls.is_empty() {
                m.insert("content".into(), Value::Null);
            } else {
                m.insert("content".into(), json!(""));
            }
            if !tool_calls.is_empty() {
                m.insert("tool_calls".into(), Value::Array(tool_calls));
            }
            out.push(Value::Object(m));
        }
    }
    Ok(())
}

pub(crate) fn parse_openai_response(raw_body: Vec<u8>, secret: &str) -> Result<ModelResponse, TransportError> {
    let clean_bytes = scrub_bytes(&raw_body, secret.as_bytes());
    let malformed = |msg: String| malformed_response_err(clean_bytes.clone(), secret, msg);

    let parsed: Value =
        serde_json::from_slice(&clean_bytes).map_err(|e| malformed(format!("invalid response JSON: {e}")))?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| malformed("response JSON is not an object".into()))?;

    let id = obj.get("id").and_then(Value::as_str).map(str::to_string);
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("response missing `model` string".into()))?
        .to_string();
    let usage = obj.get("usage").cloned().unwrap_or(Value::Null);

    let choices = obj
        .get("choices")
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty())
        .ok_or_else(|| malformed("response missing non-empty `choices` array".into()))?;
    let choice = choices[0]
        .as_object()
        .ok_or_else(|| malformed("choices[0] is not an object".into()))?;

    let msg = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| malformed("choices[0] missing `message` object".into()))?;
    if let Some(role) = msg.get("role").and_then(Value::as_str) {
        if role != "assistant" {
            return Err(malformed(format!("expected assistant role, got `{role}`")));
        }
    }

    let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
    let refusal = msg.get("refusal").and_then(Value::as_str).filter(|s| !s.is_empty());

    if refusal.is_some() || finish_reason == Some("content_filter") {
        let mut content = Vec::new();
        if let Some(r_text) = refusal {
            content.push(ContentBlock::Text { text: r_text.to_string() });
        } else if let Some(c_text) = extract_message_text(msg.get("content")) {
            if !c_text.is_empty() {
                content.push(ContentBlock::Text { text: c_text });
            }
        }
        return Ok(ModelResponse {
            raw_body: clean_bytes,
            id,
            model,
            stop_reason: StopReason::Refusal,
            content,
            usage,
        });
    }

    let mut content = Vec::new();
    if let Some(reasoning) = msg
        .get("reasoning_content")
        .or_else(|| msg.get("reasoning"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        content.push(ContentBlock::Thinking { thinking: reasoning.to_string(), signature: None });
    }
    if let Some(text) = extract_message_text(msg.get("content")) {
        if !text.is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }

    let mut has_tool_calls = false;
    if let Some(tc_val) = msg.get("tool_calls") {
        if !tc_val.is_null() {
            let tcs = tc_val
                .as_array()
                .ok_or_else(|| malformed("message.tool_calls is not an array".into()))?;
            for (idx, tc) in tcs.iter().enumerate() {
                let block = decode_tool_call(idx, tc).map_err(&malformed)?;
                content.push(block);
                has_tool_calls = true;
            }
        }
    }

    let stop_reason = match finish_reason {
        Some("tool_calls") => StopReason::ToolUse,
        Some("stop") => {
            if has_tool_calls {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            }
        }
        Some("length") => StopReason::MaxTokens,
        Some("content_filter") => StopReason::Refusal,
        Some(other) => StopReason::Other(other.to_string()),
        None => return Err(malformed("choices[0] missing `finish_reason` string".into())),
    };

    Ok(ModelResponse {
        raw_body: clean_bytes,
        id,
        model,
        stop_reason,
        content,
        usage,
    })
}

fn extract_message_text(val: Option<&Value>) -> Option<String> {
    match val? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = Vec::new();
            for p in parts {
                if p.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(t) = p.get("text").and_then(Value::as_str) {
                        if !t.is_empty() {
                            out.push(t);
                        }
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out.join("\n"))
            }
        }
        _ => None,
    }
}

fn decode_tool_call(idx: usize, tc: &Value) -> Result<ContentBlock, String> {
    let tc_obj = tc
        .as_object()
        .ok_or_else(|| format!("tool_calls[{idx}] is not an object"))?;
    let func = tc_obj
        .get("function")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("tool_calls[{idx}] missing `function` object"))?;
    let name = func
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("tool_calls[{idx}].function missing non-empty `name`"))?
        .to_string();
    let id = tc_obj
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{idx}"));

    let args_val = func
        .get("arguments")
        .ok_or_else(|| format!("tool_calls[{idx}].function missing `arguments`"))?;
    let input = match args_val {
        Value::String(raw) => {
            let parsed: Value = serde_json::from_str(raw)
                .map_err(|e| format!("tool_calls[{idx}].function.arguments invalid JSON: {e}"))?;
            if !parsed.is_object() {
                return Err(format!("tool_calls[{idx}].function.arguments must decode to a JSON object"));
            }
            parsed
        }
        Value::Object(_) => args_val.clone(),
        _ => {
            return Err(format!(
                "tool_calls[{idx}].function.arguments must be a JSON string or object"
            ));
        }
    };

    Ok(ContentBlock::ToolUse { id, name, input })
}
