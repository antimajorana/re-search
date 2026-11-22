//! Wire-format serialization and deserialization for the Anthropic Messages
//! API (`POST /v1/messages`).

use crate::http::{config_err, malformed_response_err, scrub_bytes, validate_base_url};
use crate::transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, Role, StopReason, ToolSpec, TransportError,
};
use serde_json::{json, Value};

/// Validate `base_url` and build the sanitized `scheme://host[:port]/v1/messages`
/// endpoint string.
pub(crate) fn validate_and_build_endpoint(base_url: &str) -> Result<String, TransportError> {
    let v = validate_base_url(base_url)?;
    if v.base_path.ends_with("/v1/messages") {
        Ok(format!("{}://{}{}", v.scheme, v.authority, v.base_path))
    } else if v.base_path.is_empty() {
        Ok(format!("{}://{}/v1/messages", v.scheme, v.authority))
    } else {
        Ok(format!("{}://{}{}/v1/messages", v.scheme, v.authority, v.base_path))
    }
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
    if let Some(effort) = crate::transport::normalize_reasoning_effort(req.reasoning_effort.as_deref()) {
        let mapped = if effort == "xhigh" { "max" } else { effort.as_str() };
        obj.insert("output_config".into(), json!({ "effort": mapped }));
    }
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
    let malformed = |msg: String| malformed_response_err(clean_bytes.clone(), secret, msg);

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
