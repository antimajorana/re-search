//! SSE live event sink (`text_delta`, `thinking_delta`, `round_reset`, `tool_trace`,
//! `dag_updated`) and lossless session view reconstruction from ledger events.

use crate::sse::SseHub;
use agent::EventSink;
use ledger::AppendedEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct SseEventSink {
    sse: Arc<SseHub>,
    session_id: String,
    pub(crate) final_text: String,
    streamed_thinking_in_turn: bool,
    tool_calls: HashMap<String, (String, Value)>,
}

impl SseEventSink {
    pub(crate) fn new(sse: Arc<SseHub>, session_id: &str) -> Self {
        Self {
            sse,
            session_id: session_id.to_string(),
            final_text: String::new(),
            streamed_thinking_in_turn: false,
            tool_calls: HashMap::new(),
        }
    }

    pub(crate) fn final_text(&self) -> &str {
        &self.final_text
    }
}

/// The subscription-CLI routes (`cli_route`) feed the same SSE events as the API route;
/// `vendor.builtin_tool` (Antigravity) goes out as a `session_event` the UI shows as a warning.
impl cli_route::EventSink for SseEventSink {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String> {
        EventSink::on_event(self, receipt, kind, data)
    }
}

impl EventSink for SseEventSink {
    fn on_text_delta(&mut self, delta: &str) {
        if !delta.is_empty() {
            self.sse
                .emit(&self.session_id, "text_delta", json!({ "delta": delta }));
        }
    }

    fn on_thinking_delta(&mut self, delta: &str) {
        if !delta.is_empty() {
            self.streamed_thinking_in_turn = true;
            self.sse
                .emit(&self.session_id, "thinking_delta", json!({ "delta": delta }));
        }
    }

    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String> {
        match kind {
            "model.request" => {
                self.streamed_thinking_in_turn = false;
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
            "model.text" => {
                let text = data.get("text").and_then(Value::as_str).unwrap_or("");
                self.final_text = text.to_string();
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
            "model.thinking" => {
                let chars = data.get("chars").and_then(Value::as_u64).unwrap_or(0);
                let redacted = data.get("redacted").and_then(Value::as_bool).unwrap_or(false);
                if redacted || (chars > 0 && !self.streamed_thinking_in_turn) {
                    let note = if redacted {
                        format!("[redacted reasoning block: {chars} chars]\n")
                    } else {
                        format!("[reasoning block: {chars} chars]\n")
                    };
                    self.sse.emit(
                        &self.session_id,
                        "thinking_delta",
                        json!({ "seq": receipt.seq, "delta": note }),
                    );
                }
            }
            "model.tool_use" => {
                let cid = data.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let tname = data.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let targs = data.get("input").cloned().unwrap_or(Value::Null);
                if !cid.is_empty() {
                    self.tool_calls.insert(cid.clone(), (tname.clone(), targs.clone()));
                }
                self.sse.emit(
                    &self.session_id,
                    "tool_trace",
                    json!({
                        "seq": receipt.seq,
                        "call_id": cid,
                        "tool": tname,
                        "args": targs,
                        "status": "running",
                    }),
                );
            }
            "model.tool_result" => {
                let ok = !data.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let cid = data.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                let (tname, targs) = self
                    .tool_calls
                    .get(cid)
                    .cloned()
                    .unwrap_or_else(|| ("tool".to_string(), Value::Null));
                self.sse.emit(
                    &self.session_id,
                    "tool_trace",
                    json!({
                        "seq": receipt.seq,
                        "call_id": cid,
                        "tool": tname,
                        "args": targs,
                        "output": data.get("text").and_then(Value::as_str).unwrap_or(""),
                        "ok": ok,
                        "status": if ok { "ok" } else { "error" },
                    }),
                );
            }
            "autonomous.feedback" => {
                self.final_text.clear();
                self.sse.emit(
                    &self.session_id,
                    "round_reset",
                    json!({
                        "seq": receipt.seq,
                        "round": data.get("round"),
                        "target": data.get("target"),
                    }),
                );
            }
            "guardrail.warning" => {
                if let Some(banner) = data.get("banner").and_then(Value::as_str) {
                    if self.final_text.is_empty() {
                        self.final_text = banner.to_string();
                    } else {
                        self.final_text = format!("{banner}\n{}", self.final_text);
                    }
                }
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
            "proof.entry" | "proof.supersede" | "proof.verification" => {
                self.sse.emit(
                    &self.session_id,
                    "dag_updated",
                    json!({ "seq": receipt.seq, "kind": kind, "payload": data }),
                );
            }
            _ => {
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
        }
        Ok(())
    }
}

/// One warning line for a `vendor.builtin_tool` event.
pub(crate) fn builtin_tool_warning(data: &Value) -> String {
    let name: String = data.get("tool_name").and_then(Value::as_str).unwrap_or("?").chars().take(80).collect();
    let state = data.get("state").and_then(Value::as_str).unwrap_or("");
    let outcome = if data.get("denied_or_failed").and_then(Value::as_bool).unwrap_or(false) {
        "reported denied or failed"
    } else if state == "DONE" {
        "reported finished"
    } else {
        "reported running"
    };
    format!(
        "Warning: the Antigravity CLI's built-in tool `{name}` {outcome}. It is outside re:Search's tools, policy and approvals; re:Search only records what the CLI reported (vendor-sandboxed, not confined to re:Search tools)."
    )
}

pub(crate) fn reconstruct_session_view(events: &[ledger::Event]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut messages: Vec<Value> = Vec::new();
    let mut traces = Vec::new();
    let mut sources = Vec::new();
    let mut pending_turn_traces: Vec<Value> = Vec::new();
    let mut pending_thinking = String::new();
    let mut tool_calls: HashMap<String, (String, Value)> = HashMap::new();

    for ev in events {
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                if let Some(prompt) = ev.payload.get("prompt").and_then(Value::as_str) {
                    messages.push(json!({ "role": "user", "text": prompt, "seq": ev.seq }));
                }
            }
            "model.thinking" => {
                let chars = ev.payload.get("chars").and_then(Value::as_u64).unwrap_or(0);
                if chars > 0 {
                    pending_thinking.push_str(&format!("[reasoning block: {chars} chars]\n"));
                }
            }
            "model.tool_use" => {
                let cid = ev.payload.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let tname = ev.payload.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let targs = ev.payload.get("input").cloned().unwrap_or(Value::Null);
                if !cid.is_empty() {
                    tool_calls.insert(cid.clone(), (tname.clone(), targs.clone()));
                }
                traces.push(json!({
                    "seq": ev.seq,
                    "call_id": cid,
                    "tool": tname,
                    "args": targs,
                    "status": "running",
                }));
            }
            "model.tool_result" => {
                let ok = !ev.payload.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let cid = ev.payload.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                let (tname, targs) = tool_calls
                    .get(cid)
                    .cloned()
                    .unwrap_or_else(|| ("tool".to_string(), Value::Null));
                let item = json!({
                    "seq": ev.seq,
                    "call_id": cid,
                    "tool": tname,
                    "args": targs,
                    "output": ev.payload.get("text").and_then(Value::as_str).unwrap_or(""),
                    "ok": ok,
                    "status": if ok { "ok" } else { "error" },
                });
                traces.push(item.clone());
                pending_turn_traces.push(item);
            }
            "model.text" | "model.refusal" => {
                if let Some(text) = ev.payload.get("text").and_then(Value::as_str) {
                    messages.push(json!({
                        "role": "assistant",
                        "text": text,
                        "seq": ev.seq,
                        "thinking": std::mem::take(&mut pending_thinking),
                        "tool_traces": std::mem::take(&mut pending_turn_traces),
                    }));
                }
            }
            "proof.verification" => {
                let verdict = ev.payload.get("verdict").and_then(Value::as_str).unwrap_or("");
                let summary = ev.payload.get("summary").and_then(Value::as_str).unwrap_or("");
                let target = ev.payload.get("target_entry_id").and_then(Value::as_str).unwrap_or("");
                pending_thinking.push_str(&format!("[Verifier {verdict} `{target}`: {summary}]\n"));
            }
            "strategist.consult" => {
                pending_thinking.push_str("[Research Strategist consulted]\n");
            }
            "autonomous.feedback" => {
                if messages.last().and_then(|m| m.get("role")).and_then(Value::as_str) == Some("assistant") {
                    if let Some(prev) = messages.pop() {
                        if let Some(old_think) = prev.get("thinking").and_then(Value::as_str) {
                            pending_thinking = format!("{old_think}{pending_thinking}");
                        }
                        if let Some(old_traces) = prev.get("tool_traces").and_then(Value::as_array) {
                            let mut combined = old_traces.clone();
                            combined.append(&mut pending_turn_traces);
                            pending_turn_traces = combined;
                        }
                        pending_thinking.push_str("[Draft sent back for revision after Verifier check]\n");
                    }
                }
            }
            "guardrail.warning" => {
                if let Some(banner) = ev.payload.get("banner").and_then(Value::as_str) {
                    if let Some(last) = messages.last_mut().filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant")) {
                        let cur = last.get("text").and_then(Value::as_str).unwrap_or("");
                        last["text"] = json!(format!("{banner}\n{cur}"));
                    }
                }
            }
            "source.ingested" => {
                sources.push(ev.payload.clone());
            }
            "vendor.builtin_tool" => {
                messages.push(json!({ "role": "warning", "text": builtin_tool_warning(&ev.payload), "seq": ev.seq }));
            }
            _ => {}
        }
    }
    (messages, traces, sources)
}
