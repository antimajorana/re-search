//! SSE frame accumulators reassembling streamed Anthropic Messages and OpenAI/Ollama
//! Chat Completions into canonical JSON payloads for `put_blob` and `resume`.

use crate::http::{malformed_response_err, scrub_bytes, scrub_text};
use crate::stream_http::StreamMsg;
use crate::transport::{TransportError, TransportErrorKind};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::mpsc;

#[derive(Default)]
struct AnthropicBlockAcc {
    kind: String,
    id: String,
    name: String,
    text: String,
    thinking: String,
    signature: Option<String>,
    redacted_data: String,
    input_json: String,
    raw: Value,
}

#[derive(Default)]
pub(crate) struct AnthropicSseAcc {
    seen_message_stop: bool,
    id: Option<String>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Map<String, Value>,
    blocks: BTreeMap<usize, AnthropicBlockAcc>,
}

impl AnthropicSseAcc {
    pub(crate) fn ingest(
        &mut self,
        event_name: Option<&str>,
        data: &str,
        secret: &str,
        tx: &mpsc::Sender<StreamMsg>,
    ) -> Result<(), TransportError> {
        let ev: Value = serde_json::from_str(data).map_err(|e| {
            malformed_response_err(
                scrub_bytes(data.as_bytes(), secret.as_bytes()),
                secret,
                format!("invalid Anthropic SSE JSON: {e}"),
            )
        })?;
        let ty = ev.get("type").and_then(Value::as_str).or(event_name).unwrap_or("");
        match ty {
            "error" => {
                let clean = scrub_bytes(data.as_bytes(), secret.as_bytes());
                let msg = scrub_text(ev["error"]["message"].as_str().unwrap_or("Anthropic stream error"), secret);
                return Err(TransportError {
                    kind: TransportErrorKind::HttpStatus,
                    status: Some(200),
                    message: msg,
                    raw_body: Some(clean),
                });
            }
            "message_start" => {
                let m = &ev["message"];
                self.id = m.get("id").and_then(Value::as_str).map(str::to_string);
                self.model = m.get("model").and_then(Value::as_str).map(str::to_string);
                if let Some(u) = m.get("usage").and_then(Value::as_object) {
                    for (k, v) in u {
                        self.usage.insert(k.clone(), v.clone());
                    }
                }
            }
            "content_block_start" => {
                let idx = ev.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let cb = &ev["content_block"];
                let kind = cb.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                let mut b = AnthropicBlockAcc { kind: kind.clone(), raw: cb.clone(), ..Default::default() };
                match kind.as_str() {
                    "text" => b.text = cb.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                    "thinking" => {
                        b.thinking = cb.get("thinking").and_then(Value::as_str).unwrap_or("").to_string();
                        b.signature = cb.get("signature").and_then(Value::as_str).map(str::to_string);
                    }
                    "redacted_thinking" => {
                        b.redacted_data = cb.get("data").and_then(Value::as_str).unwrap_or("").to_string();
                    }
                    "tool_use" => {
                        b.id = cb.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                        b.name = cb.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                        let _ = tx.send(StreamMsg::Thinking(format!("[Preparing tool `{}`...]\n", b.name)));
                    }
                    _ => {}
                }
                self.blocks.insert(idx, b);
            }
            "content_block_delta" => {
                let idx = ev.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = &ev["delta"];
                if let Some(b) = self.blocks.get_mut(&idx) {
                    match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text_delta" => {
                            if let Some(t) = delta.get("text").and_then(Value::as_str) {
                                b.text.push_str(t);
                                let _ = tx.send(StreamMsg::Text(scrub_text(t, secret)));
                            }
                        }
                        "thinking_delta" => {
                            if let Some(t) = delta.get("thinking").and_then(Value::as_str) {
                                b.thinking.push_str(t);
                                let _ = tx.send(StreamMsg::Thinking(scrub_text(t, secret)));
                            }
                        }
                        "signature_delta" => {
                            if let Some(s) = delta.get("signature").and_then(Value::as_str) {
                                b.signature.get_or_insert_with(String::new).push_str(s);
                            }
                        }
                        "input_json_delta" => {
                            if let Some(p) = delta.get("partial_json").and_then(Value::as_str) {
                                b.input_json.push_str(p);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(sr) = ev["delta"].get("stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(sr.to_string());
                }
                if let Some(u) = ev.get("usage").and_then(Value::as_object) {
                    for (k, v) in u {
                        self.usage.insert(k.clone(), v.clone());
                    }
                }
            }
            "message_stop" => self.seen_message_stop = true,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn into_json_bytes(self, raw_sse: &[u8], secret: &str) -> Result<Vec<u8>, TransportError> {
        let clean = scrub_bytes(raw_sse, secret.as_bytes());
        if !self.seen_message_stop {
            return Err(malformed_response_err(clean, secret, "Anthropic stream ended without message_stop"));
        }
        let model = self
            .model
            .ok_or_else(|| malformed_response_err(clean.clone(), secret, "Anthropic stream missing model"))?;
        let stop_reason = self.stop_reason.unwrap_or_else(|| "end_turn".into());
        let mut content = Vec::with_capacity(self.blocks.len());
        for (_, b) in self.blocks {
            match b.kind.as_str() {
                "text" => content.push(json!({ "type": "text", "text": b.text })),
                "thinking" => {
                    content.push(json!({ "type": "thinking", "thinking": b.thinking, "signature": b.signature }));
                }
                "redacted_thinking" => content.push(json!({ "type": "redacted_thinking", "data": b.redacted_data })),
                "tool_use" => {
                    let input: Value = if b.input_json.trim().is_empty() {
                        b.raw.get("input").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}))
                    } else {
                        serde_json::from_str(&b.input_json).map_err(|e| {
                            malformed_response_err(
                                clean.clone(),
                                secret,
                                format!("invalid streamed tool_use JSON: {e}"),
                            )
                        })?
                    };
                    content.push(json!({ "type": "tool_use", "id": b.id, "name": b.name, "input": input }));
                }
                _ => content.push(b.raw),
            }
        }
        serde_json::to_vec(&json!({
            "id": self.id,
            "type": "message",
            "role": "assistant",
            "model": model,
            "stop_reason": stop_reason,
            "content": content,
            "usage": Value::Object(self.usage),
        }))
        .map_err(|e| malformed_response_err(clean, secret, e.to_string()))
    }
}

#[derive(Default)]
struct OpenAiToolAcc {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub(crate) struct OpenAiSseAcc {
    seen_chunk: bool,
    id: Option<String>,
    model: Option<String>,
    finish_reason: Option<String>,
    content: String,
    refusal: String,
    tools: BTreeMap<usize, OpenAiToolAcc>,
    usage: Value,
}

impl OpenAiSseAcc {
    pub(crate) fn ingest(
        &mut self,
        data: &str,
        secret: &str,
        tx: &mpsc::Sender<StreamMsg>,
    ) -> Result<(), TransportError> {
        let ev: Value = serde_json::from_str(data).map_err(|e| {
            malformed_response_err(
                scrub_bytes(data.as_bytes(), secret.as_bytes()),
                secret,
                format!("invalid OpenAI SSE JSON: {e}"),
            )
        })?;
        if let Some(err_msg) = ev["error"]["message"].as_str() {
            let clean = scrub_bytes(data.as_bytes(), secret.as_bytes());
            return Err(TransportError {
                kind: TransportErrorKind::HttpStatus,
                status: Some(200),
                message: scrub_text(err_msg, secret),
                raw_body: Some(clean),
            });
        }
        self.seen_chunk = true;
        if self.id.is_none() {
            self.id = ev.get("id").and_then(Value::as_str).map(str::to_string);
        }
        if self.model.is_none() {
            self.model = ev.get("model").and_then(Value::as_str).map(str::to_string);
        }
        if ev.get("usage").is_some_and(|u| !u.is_null()) {
            self.usage = ev["usage"].clone();
        }
        if let Some(choice) = ev.get("choices").and_then(Value::as_array).and_then(|a| a.first()) {
            if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(fr.to_string());
            }
            let delta = &choice["delta"];
            if let Some(r) = delta.get("reasoning_content").or_else(|| delta.get("reasoning")).and_then(Value::as_str) {
                let _ = tx.send(StreamMsg::Thinking(scrub_text(r, secret)));
            }
            if let Some(c) = delta.get("content").and_then(Value::as_str) {
                self.content.push_str(c);
                let _ = tx.send(StreamMsg::Text(scrub_text(c, secret)));
            }
            if let Some(rf) = delta.get("refusal").and_then(Value::as_str) {
                self.refusal.push_str(rf);
            }
            if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let entry = self.tools.entry(idx).or_default();
                    if let Some(id) = tc.get("id").and_then(Value::as_str) {
                        entry.id.push_str(id);
                    }
                    if let Some(n) = tc["function"].get("name").and_then(Value::as_str) {
                        if entry.name.is_empty() && !n.is_empty() {
                            let _ = tx.send(StreamMsg::Thinking(format!("[Preparing tool `{n}`...]\n")));
                        }
                        entry.name.push_str(n);
                    }
                    if let Some(a) = tc["function"].get("arguments").and_then(Value::as_str) {
                        entry.arguments.push_str(a);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn into_json_bytes(self, raw_sse: &[u8], secret: &str) -> Result<Vec<u8>, TransportError> {
        let clean = scrub_bytes(raw_sse, secret.as_bytes());
        if !self.seen_chunk {
            return Err(malformed_response_err(clean, secret, "OpenAI stream contained zero chunks"));
        }
        let model = self
            .model
            .ok_or_else(|| malformed_response_err(clean.clone(), secret, "OpenAI stream missing model"))?;
        let mut msg = Map::new();
        msg.insert("role".into(), json!("assistant"));
        if !self.content.is_empty() || self.tools.is_empty() {
            msg.insert("content".into(), json!(self.content));
        } else {
            msg.insert("content".into(), Value::Null);
        }
        if !self.refusal.is_empty() {
            msg.insert("refusal".into(), json!(self.refusal));
        }
        if !self.tools.is_empty() {
            let calls: Vec<Value> = self
                .tools
                .into_values()
                .map(|t| json!({ "id": t.id, "type": "function", "function": { "name": t.name, "arguments": t.arguments } }))
                .collect();
            msg.insert("tool_calls".into(), Value::Array(calls));
        }
        let finish_reason = self.finish_reason.unwrap_or_else(|| {
            if msg.contains_key("tool_calls") { "tool_calls".into() } else { "stop".into() }
        });
        serde_json::to_vec(&json!({
            "id": self.id,
            "model": model,
            "choices": [{ "index": 0, "finish_reason": finish_reason, "message": Value::Object(msg) }],
            "usage": self.usage,
        }))
        .map_err(|e| malformed_response_err(clean, secret, e.to_string()))
    }
}
