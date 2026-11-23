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
            // Includes `human.steer`, `human.checkpoint_requested`, `human.decision`, `explain.promoted`.
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

pub(crate) fn summarize_session_events(events: &[ledger::Event]) -> Value {
    let mut proof_entries = 0u64;
    let mut accepted_entries = 0u64;
    let mut verification_rounds = 0u64;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut worker_input_tokens = 0u64;
    let mut worker_output_tokens = 0u64;
    let mut delegation_count = 0u64;
    let mut honesty_warnings = 0u64;
    let mut model = String::new();
    let mut verified_status = "none";

    for ev in events {
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                if let Some(m) = ev.payload.get("model").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    model = m.to_string();
                }
            }
            "model.response" => {
                if let Some(m) = ev.payload.get("model").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    model = m.to_string();
                }
                if let Some(u) = ev.payload.get("usage") {
                    input_tokens += u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    output_tokens += u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
            }
            "delegate.end" | "explore.end" => {
                delegation_count += 1;
                worker_input_tokens += ev.payload.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                worker_output_tokens += ev.payload.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
            }
            "honesty.lint" => {
                let passed = ev.payload.get("passed").and_then(Value::as_bool).unwrap_or(false);
                honesty_warnings = if passed {
                    0
                } else {
                    ev.payload
                        .get("findings")
                        .and_then(Value::as_array)
                        .map(|f| f.len() as u64)
                        .unwrap_or(0)
                };
            }
            "proof.entry" => {
                proof_entries += 1;
                if ev.payload.get("status").and_then(Value::as_str) == Some("accepted") {
                    accepted_entries += 1;
                }
            }
            "proof.verification" => {
                verification_rounds += 1;
                match ev.payload.get("verdict").and_then(Value::as_str) {
                    Some("accepted") => {
                        accepted_entries = accepted_entries.max(1);
                        verified_status = "verified";
                    }
                    Some("rejected") => {
                        verified_status = "unverified";
                    }
                    _ => {}
                }
            }
            "session.end" => {
                if ev.payload.get("outcome").and_then(Value::as_str) == Some("completed_unverified") {
                    verified_status = "unverified";
                } else if ev.payload.get("verified_entry_id").and_then(Value::as_str).is_some() {
                    verified_status = "verified";
                }
            }
            _ => {}
        }
    }
    let campaign_phase = agent::infer_campaign_phase(events).as_str();

    json!({
        "proof_entries": proof_entries,
        "accepted_entries": accepted_entries,
        "verification_rounds": verification_rounds,
        "verified_status": verified_status,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "worker_input_tokens": worker_input_tokens,
        "worker_output_tokens": worker_output_tokens,
        "delegation_count": delegation_count,
        "honesty_warnings": honesty_warnings,
        "campaign_phase": campaign_phase,
        "model": model,
    })
}

fn is_primary_assistant(msg: &Value) -> bool {
    msg.get("role").and_then(Value::as_str) == Some("assistant")
        && !msg.get("narrator").and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn reconstruct_session_view(events: &[ledger::Event]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut messages: Vec<Value> = Vec::new();
    let mut traces = Vec::new();
    let mut sources = Vec::new();
    let mut pending_turn_traces: Vec<Value> = Vec::new();
    let mut pending_verifications: Vec<Value> = Vec::new();
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
                        "verifications": std::mem::take(&mut pending_verifications),
                    }));
                }
            }
            "proof.verification" => {
                let verdict = ev.payload.get("verdict").and_then(Value::as_str).unwrap_or("");
                let summary = ev.payload.get("summary").and_then(Value::as_str).unwrap_or("");
                let target = ev.payload.get("target_entry_id").and_then(Value::as_str).unwrap_or("");
                let line = format!("[Verifier {verdict} `{target}`: {summary}]\n");
                let v_obj = json!({
                    "seq": ev.seq,
                    "verdict": verdict,
                    "target_entry_id": target,
                    "summary": summary,
                    "issues": ev.payload.get("issues").cloned().unwrap_or_else(|| json!([])),
                });
                if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                    let cur = last.get("thinking").and_then(Value::as_str).unwrap_or("");
                    last["thinking"] = json!(format!("{cur}{line}"));
                    if let Some(arr) = last.get_mut("verifications").and_then(Value::as_array_mut) {
                        arr.push(v_obj);
                    } else {
                        last["verifications"] = json!([v_obj]);
                    }
                    last["verified_status"] = json!(if verdict == "accepted" { "verified" } else { "unverified" });
                } else {
                    pending_thinking.push_str(&line);
                    pending_verifications.push(v_obj);
                }
            }
            "strategist.consult" => {
                let note = "[Research Strategist consulted]\n";
                if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                    let cur = last.get("thinking").and_then(Value::as_str).unwrap_or("");
                    last["thinking"] = json!(format!("{cur}{note}"));
                } else {
                    pending_thinking.push_str(note);
                }
            }
            "autonomous.feedback" => {
                if messages.last().is_some_and(is_primary_assistant) {
                    if let Some(prev) = messages.pop() {
                        if let Some(old_think) = prev.get("thinking").and_then(Value::as_str) {
                            pending_thinking = format!("{old_think}{pending_thinking}");
                        }
                        if let Some(old_traces) = prev.get("tool_traces").and_then(Value::as_array) {
                            let mut combined = old_traces.clone();
                            combined.append(&mut pending_turn_traces);
                            pending_turn_traces = combined;
                        }
                        if let Some(old_vers) = prev.get("verifications").and_then(Value::as_array) {
                            let mut combined = old_vers.clone();
                            combined.append(&mut pending_verifications);
                            pending_verifications = combined;
                        }
                        pending_thinking.push_str("[Draft sent back for revision after Verifier check]\n");
                    }
                }
            }
            "guardrail.warning" => {
                if let Some(banner) = ev.payload.get("banner").and_then(Value::as_str) {
                    if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                        let cur = last.get("text").and_then(Value::as_str).unwrap_or("");
                        last["text"] = json!(format!("{banner}\n{cur}"));
                        last["verified_status"] = json!("unverified");
                    }
                }
            }
            "session.end" => {
                if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                    if let Some(rounds) = ev.payload.get("verification_rounds").and_then(Value::as_u64) {
                        last["verification_rounds"] = json!(rounds);
                    }
                    if let Some(vid) = ev.payload.get("verified_entry_id").and_then(Value::as_str) {
                        last["verified_entry_id"] = json!(vid);
                        last["verified_status"] = json!("verified");
                    } else if ev.payload.get("outcome").and_then(Value::as_str) == Some("completed_unverified") {
                        last["verified_status"] = json!("unverified");
                    }
                }
            }
            "narrator.report" => {
                let walkthrough = ev
                    .payload
                    .get("walkthrough")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let sheet = ev
                    .payload
                    .get("derivation_sheet")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let combined = match (walkthrough.is_empty(), sheet.is_empty()) {
                    (false, false) => format!("{walkthrough}\n\n{sheet}"),
                    (false, true) => walkthrough.to_string(),
                    (true, false) => sheet.to_string(),
                    (true, true) => String::new(),
                };
                if !combined.is_empty() {
                    messages.push(json!({
                        "role": "assistant",
                        "narrator": true,
                        "seq": ev.seq,
                        "text": combined,
                        "thinking": "",
                        "tool_traces": [],
                        "verifications": [],
                    }));
                }
            }
            "source.ingested" => {
                sources.push(ev.payload.clone());
            }
            "vendor.builtin_tool" => {
                messages.push(json!({ "role": "warning", "text": builtin_tool_warning(&ev.payload), "seq": ev.seq }));
            }
            "delegate.end" => {
                let wname = ev
                    .payload
                    .get("worker_name")
                    .or_else(|| ev.payload.get("worker"))
                    .and_then(Value::as_str)
                    .unwrap_or("Worker");
                let model = ev.payload.get("model").and_then(Value::as_str).unwrap_or("");
                let turns = ev.payload.get("turns").and_then(Value::as_u64).unwrap_or(0);
                let inp = ev.payload.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                let out = ev.payload.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                pending_thinking.push_str(&format!(
                    "[Worker {wname} ({model}): {turns} turn(s), {inp}+{out} tok]\n"
                ));
            }
            "explore.end" => {
                let model = ev.payload.get("model").and_then(Value::as_str).unwrap_or("");
                let branches = ev.payload.get("branch_count").and_then(Value::as_u64).unwrap_or(0);
                let turns = ev.payload.get("turns").and_then(Value::as_u64).unwrap_or(0);
                let inp = ev.payload.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                let out = ev.payload.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                pending_thinking.push_str(&format!(
                    "[Ultramath Explorer ({model}): {branches} branches, {turns} turns, {inp}+{out} tok]\n"
                ));
            }
            "honesty.lint" => {
                if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                    last["honesty"] = ev.payload.clone();
                }
            }
            "human.steer" => {
                let text = ev.payload.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if !text.is_empty() {
                    pending_thinking.push_str(&format!("[Human steer: {text}]\n"));
                }
            }
            "human.decision" => {
                let kind = ev.payload.get("kind").and_then(Value::as_str).unwrap_or("checkpoint");
                let cancelled = ev.payload.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                let opt = ev.payload.get("selected_option").and_then(Value::as_str).unwrap_or("").trim();
                let text = ev.payload.get("text").and_then(Value::as_str).unwrap_or("").trim();
                let detail = if cancelled {
                    if text.is_empty() { "cancelled".to_string() } else { format!("cancelled: {text}") }
                } else {
                    match (opt.is_empty(), text.is_empty()) {
                        (false, false) => format!("{opt} — {text}"),
                        (false, true) => opt.to_string(),
                        (true, false) => text.to_string(),
                        (true, true) => "confirmed".to_string(),
                    }
                };
                pending_thinking.push_str(&format!("[Checkpoint decision ({kind}): {detail}]\n"));
            }
            "campaign.phase" => {}
            _ => {}
        }
    }
    (messages, traces, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, kind: &str, payload: Value) -> ledger::Event {
        ledger::Event {
            session: "s-view".into(),
            seq,
            ts_ms: seq as i64 * 10,
            kind: kind.into(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "1".repeat(64),
        }
    }

    #[test]
    fn summarizes_and_reconstructs_delegation_honesty_and_phase() {
        let events = vec![
            ev(1, "session.start", json!({ "prompt": "Check", "model": "claude-sonnet-4-6" })),
            ev(2, "campaign.phase", json!({ "phase": "EXPLORE" })),
            ev(3, "explore.end", json!({ "model": "claude-haiku-4-5", "branch_count": 2, "turns": 4, "input_tokens": 30, "output_tokens": 12 })),
            ev(4, "delegate.end", json!({ "worker": "compute_worker", "worker_name": "Compute Worker", "model": "claude-haiku-4-5", "turns": 2, "input_tokens": 40, "output_tokens": 15 })),
            ev(5, "human.steer", json!({ "steer_id": "st-1", "text": "Use k=0 gauge", "ts_ms": 50 })),
            ev(6, "human.decision", json!({ "checkpoint_id": "chk-1", "kind": "plan", "selected_option": "Approve", "text": "", "cancelled": false })),
            ev(7, "model.text", json!({ "text": "Done." })),
            ev(8, "honesty.lint", json!({ "passed": false, "findings": [{ "severity": "warning", "claim": "q.e.d." }] })),
        ];
        let stats = summarize_session_events(&events);
        assert_eq!(stats["worker_input_tokens"], 70);
        assert_eq!(stats["worker_output_tokens"], 27);
        assert_eq!(stats["delegation_count"], 2);
        assert_eq!(stats["honesty_warnings"], 1);
        assert_eq!(stats["campaign_phase"], "EXPLORE");

        let (msgs, _, _) = reconstruct_session_view(&events);
        assert_eq!(msgs.len(), 2);
        let think = msgs[1]["thinking"].as_str().unwrap();
        assert!(think.contains("[Ultramath Explorer (claude-haiku-4-5): 2 branches, 4 turns, 30+12 tok]"));
        assert!(think.contains("[Worker Compute Worker (claude-haiku-4-5): 2 turn(s), 40+15 tok]"));
        assert!(think.contains("[Human steer: Use k=0 gauge]"));
        assert!(think.contains("[Checkpoint decision (plan): Approve]"));
        assert_eq!(msgs[1]["honesty"]["passed"], false);
        assert_eq!(msgs[1]["honesty"]["findings"][0]["claim"], "q.e.d.");
    }
}
