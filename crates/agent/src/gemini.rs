//! Google Gemini `generateContent` (`POST /v1beta/models/{model}:generateContent`) transport.

use crate::http::{
    build_ureq_agent, config_err, malformed_response_err, post_json, scrub_bytes, validate_base_url, SecretKey,
};
use crate::stream_http::post_json_cancellable;
use crate::transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, PreparedRequest, ProviderKind, Role, StopReason,
    StreamObserver, ToolSpec, Transport, TransportError, TransportInfo, DEFAULT_GEMINI_BASE, DEFAULT_GEMINI_MODEL,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GeminiConfig {
    pub base_url: String,
    pub model: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_GEMINI_BASE.to_string(),
            model: DEFAULT_GEMINI_MODEL.to_string(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(180),
        }
    }
}

pub struct GeminiTransport {
    model: String,
    endpoint: String,
    key: SecretKey,
    key_env: String,
    agent: ureq::Agent,
}

impl GeminiTransport {
    pub fn from_env(config: GeminiConfig) -> Result<Self, TransportError> {
        let (key, key_env) = SecretKey::from_env_vars(
            ProviderKind::Gemini.key_env_vars(),
            "RE_SEARCH_API_KEY or GEMINI_API_KEY",
        )?;
        Self::new(config, key, key_env)
    }

    pub fn new(config: GeminiConfig, key: SecretKey, key_env: impl Into<String>) -> Result<Self, TransportError> {
        let model = validate_gemini_model(&config.model)?;
        let validated = validate_base_url(&config.base_url)?;
        let endpoint = build_gemini_endpoint(&validated.scheme, &validated.authority, &validated.base_path, &model);
        let agent = build_ureq_agent(validated.is_https, config.connect_timeout, config.request_timeout);
        Ok(Self {
            model,
            endpoint,
            key,
            key_env: key_env.into(),
            agent,
        })
    }
}

pub(crate) fn validate_gemini_model(raw: &str) -> Result<String, TransportError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(config_err("Gemini model name is empty"));
    }
    if trimmed.contains("..")
        || trimmed.chars().any(|c| {
            matches!(c, '/' | '\\' | '?' | '#' | ':' | '%' | '@') || c.is_ascii_whitespace() || c.is_ascii_control()
        })
    {
        return Err(config_err(format!("invalid Gemini model name `{trimmed}`")));
    }
    Ok(trimmed.to_string())
}

fn build_gemini_endpoint(scheme: &str, authority: &str, base_path: &str, model: &str) -> String {
    if base_path.ends_with("/v1beta") {
        format!("{scheme}://{authority}{base_path}/models/{model}:generateContent")
    } else if base_path.is_empty() {
        format!("{scheme}://{authority}/v1beta/models/{model}:generateContent")
    } else {
        format!("{scheme}://{authority}{base_path}/v1beta/models/{model}:generateContent")
    }
}

impl Transport for GeminiTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "gemini-generate-content",
            endpoint: self.endpoint.clone(),
            api_key_env: self.key_env.clone(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest {
            body: serialize_gemini_request(&self.model, req)?,
        })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let secret = self.key.as_str();
        let headers = [("x-goog-api-key", secret)];
        let raw = post_json(&self.agent, &self.endpoint, &headers, &prepared.body, secret)?;
        parse_gemini_response(raw, secret)
    }

    fn send_prepared_streaming(
        &self,
        prepared: &PreparedRequest,
        observer: &mut dyn StreamObserver,
    ) -> Result<ModelResponse, TransportError> {
        let secret = self.key.as_str();
        let headers = [("x-goog-api-key", secret)];
        let raw = post_json_cancellable(&self.agent, &self.endpoint, &headers, &prepared.body, secret, observer)?;
        parse_gemini_response(raw, secret)
    }
}

fn serialize_gemini_request(expected_model: &str, req: &ModelRequest) -> Result<Vec<u8>, TransportError> {
    if req.max_tokens == 0 {
        return Err(config_err("max_tokens must be >= 1"));
    }
    let req_model = validate_gemini_model(&req.model)?;
    if req_model != expected_model {
        return Err(config_err(format!(
            "request model `{req_model}` does not match GeminiTransport model `{expected_model}`"
        )));
    }
    crate::vision::validate_request_images(ProviderKind::Gemini, req)?;

    let mut tool_names: HashMap<&str, &str> = HashMap::new();
    let mut contents = Vec::with_capacity(req.messages.len());
    for msg in &req.messages {
        contents.push(encode_message(msg, &mut tool_names)?);
    }

    let mut obj = serde_json::Map::new();
    if let Some(sys) = &req.system {
        if !sys.is_empty() {
            obj.insert("systemInstruction".into(), json!({ "parts": [{ "text": sys }] }));
        }
    }
    obj.insert("contents".into(), Value::Array(contents));
    if !req.tools.is_empty() {
        obj.insert(
            "tools".into(),
            json!([{ "functionDeclarations": req.tools.iter().map(encode_tool).collect::<Vec<_>>() }]),
        );
    }
    let mut gen_cfg = serde_json::Map::new();
    gen_cfg.insert("maxOutputTokens".into(), json!(req.max_tokens));
    if let Some(effort) = crate::transport::normalize_reasoning_effort(req.reasoning_effort.as_deref()) {
        let level = match effort.as_str() {
            "low" => "LOW",
            "medium" => "MEDIUM",
            _ => "HIGH",
        };
        gen_cfg.insert("thinkingConfig".into(), json!({ "thinkingLevel": level }));
    }
    obj.insert("generationConfig".into(), Value::Object(gen_cfg));

    serde_json::to_vec(&Value::Object(obj))
        .map_err(|e| config_err(format!("failed to serialize Gemini request: {e}")))
}

fn encode_tool(spec: &ToolSpec) -> Value {
    json!({
        "name": spec.name,
        "description": spec.description,
        "parametersJsonSchema": spec.input_schema,
    })
}

fn encode_message<'a>(msg: &'a Message, tool_names: &mut HashMap<&'a str, &'a str>) -> Result<Value, TransportError> {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "model",
    };

    let mut sig_by_tool_use_id: HashMap<&str, &str> = HashMap::new();
    if msg.role == Role::Assistant {
        for block in &msg.content {
            if let ContentBlock::Other { block_type, raw } = block {
                if block_type == "gemini_thought_signature" {
                    if let (Some(tid), Some(sig)) = (
                        raw.get("tool_use_id").and_then(Value::as_str),
                        raw.get("thought_signature").and_then(Value::as_str),
                    ) {
                        sig_by_tool_use_id.insert(tid, sig);
                    }
                }
            }
        }
    }

    let mut parts = Vec::new();
    for block in &msg.content {
        match block {
            ContentBlock::Text { text } => {
                parts.push(json!({ "text": text }));
            }
            ContentBlock::Thinking { thinking, signature } => {
                let mut part = serde_json::Map::new();
                part.insert("text".into(), json!(thinking));
                part.insert("thought".into(), json!(true));
                if let Some(sig) = signature {
                    part.insert("thoughtSignature".into(), json!(sig));
                }
                parts.push(Value::Object(part));
            }
            ContentBlock::ToolUse { id, name, input } => {
                tool_names.insert(id.as_str(), name.as_str());
                let mut part = serde_json::Map::new();
                part.insert(
                    "functionCall".into(),
                    json!({ "id": id, "name": name, "args": input }),
                );
                if let Some(sig) = sig_by_tool_use_id.get(id.as_str()) {
                    part.insert("thoughtSignature".into(), json!(sig));
                }
                parts.push(Value::Object(part));
            }
            ContentBlock::ToolResult { tool_use_id, is_error, content } => {
                let resolved_name = tool_names
                    .get(tool_use_id.as_str())
                    .copied()
                    .ok_or_else(|| config_err(format!("unknown tool_use_id `{tool_use_id}` for Gemini functionResponse")))?;
                let response_obj = if *is_error {
                    json!({ "error": content })
                } else if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(content) {
                    json!({ "result": Value::Object(map) })
                } else {
                    json!({ "result": content })
                };
                parts.push(json!({
                    "functionResponse": {
                        "id": tool_use_id,
                        "name": resolved_name,
                        "response": response_obj,
                    }
                }));
            }
            ContentBlock::Image { media_type, data_base64 } => {
                parts.push(crate::vision::encode_gemini_image(media_type, data_base64));
            }
            ContentBlock::RedactedThinking { .. } | ContentBlock::Other { .. } => {}
        }
    }

    Ok(json!({ "role": role, "parts": parts }))
}

pub(crate) fn parse_gemini_response(raw_body: Vec<u8>, secret: &str) -> Result<ModelResponse, TransportError> {
    let clean_bytes = scrub_bytes(&raw_body, secret.as_bytes());
    let malformed = |msg: String| malformed_response_err(clean_bytes.clone(), secret, msg);

    let parsed: Value =
        serde_json::from_slice(&clean_bytes).map_err(|e| malformed(format!("invalid response JSON: {e}")))?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| malformed("response JSON is not an object".into()))?;

    let id = obj.get("responseId").and_then(Value::as_str).map(str::to_string);
    let model = obj
        .get("modelVersion")
        .and_then(Value::as_str)
        .unwrap_or("gemini")
        .to_string();
    let usage = obj.get("usageMetadata").cloned().unwrap_or(Value::Null);

    if let Some(reason) = obj
        .get("promptFeedback")
        .and_then(|pf| pf.get("blockReason"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Ok(ModelResponse {
            raw_body: clean_bytes,
            id,
            model,
            stop_reason: StopReason::Refusal,
            content: vec![ContentBlock::Text {
                text: format!("blocked by promptFeedback: {reason}"),
            }],
            usage,
        });
    }

    let candidates = obj
        .get("candidates")
        .and_then(Value::as_array)
        .filter(|arr| !arr.is_empty())
        .ok_or_else(|| malformed("response missing non-empty `candidates` array".into()))?;
    let cand = candidates[0]
        .as_object()
        .ok_or_else(|| malformed("candidates[0] is not an object".into()))?;

    let finish = cand.get("finishReason").and_then(Value::as_str);
    let parts = cand
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    if matches!(
        finish,
        Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY")
    ) {
        let mut texts: Vec<ContentBlock> = parts
            .iter()
            .filter_map(|p| {
                if p.get("thought").and_then(Value::as_bool) == Some(true) {
                    return None;
                }
                p.get("text").and_then(Value::as_str).map(|t| ContentBlock::Text { text: t.to_string() })
            })
            .collect();
        if texts.is_empty() {
            texts.push(ContentBlock::Text {
                text: format!("blocked by finishReason: {}", finish.unwrap_or("SAFETY")),
            });
        }
        return Ok(ModelResponse {
            raw_body: clean_bytes,
            id,
            model,
            stop_reason: StopReason::Refusal,
            content: texts,
            usage,
        });
    }

    let mut content = Vec::new();
    let mut call_idx = 0usize;
    let mut has_tool_use = false;

    for part in parts {
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            let thinking = part.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            let signature = part.get("thoughtSignature").and_then(Value::as_str).map(str::to_string);
            content.push(ContentBlock::Thinking { thinking, signature });
        } else if let Some(fc) = part.get("functionCall") {
            let fc_obj = fc
                .as_object()
                .ok_or_else(|| malformed("functionCall is not an object".into()))?;
            let name = fc_obj
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| malformed("functionCall missing non-empty `name`".into()))?
                .to_string();
            let args = match fc_obj.get("args") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => return Err(malformed("functionCall `args` must be a JSON object".into())),
            };
            let call_id = fc_obj
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("gemini_call_{call_idx}"));
            call_idx += 1;
            has_tool_use = true;

            content.push(ContentBlock::ToolUse { id: call_id.clone(), name, input: args });
            if let Some(sig) = part.get("thoughtSignature").and_then(Value::as_str) {
                content.push(ContentBlock::Other {
                    block_type: "gemini_thought_signature".into(),
                    raw: json!({ "tool_use_id": call_id, "thought_signature": sig }),
                });
            }
        } else if let Some(t) = part.get("text").and_then(Value::as_str) {
            content.push(ContentBlock::Text { text: t.to_string() });
        }
    }

    let stop_reason = match finish {
        Some("MAX_TOKENS") => StopReason::MaxTokens,
        Some("MALFORMED_FUNCTION_CALL") | Some("UNEXPECTED_TOOL_CALL") => {
            StopReason::Other(finish.unwrap().to_ascii_lowercase())
        }
        Some("STOP") => {
            if has_tool_use {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            }
        }
        Some(other) => StopReason::Other(other.to_ascii_lowercase()),
        None => return Err(malformed("candidates[0] missing `finishReason` string".into())),
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
