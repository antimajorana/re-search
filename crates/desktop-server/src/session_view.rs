//! SSE live event sink (`text_delta`, `thinking_delta`, `round_reset`, `tool_trace`,
//! `dag_updated`) and lossless session view reconstruction from ledger events.

use crate::session_summary::FinalAnswerVerification;
use crate::sse::SseHub;
use agent::EventSink;
use agent::runner::SESSION_TITLE_EVENT;
use ledger::AppendedEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct SseEventSink {
    sse: Arc<SseHub>,
    session_id: String,
    allow_model_title: bool,
    streamed_thinking_in_turn: bool,
    tool_calls: HashMap<String, (String, Value)>,
    verification: FinalAnswerVerification,
}

impl SseEventSink {
    pub(crate) fn new(sse: Arc<SseHub>, session_id: &str) -> Self {
        Self {
            sse,
            session_id: session_id.to_string(),
            allow_model_title: false,
            streamed_thinking_in_turn: false,
            tool_calls: HashMap::new(),
            verification: FinalAnswerVerification::default(),
        }
    }

    pub(crate) fn final_text(&self) -> &str {
        self.verification.answer()
    }

    pub(crate) fn with_model_title(mut self, allowed: bool) -> Self {
        self.allow_model_title = allowed;
        self
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
    fn wants_session_title(&self) -> bool {
        self.allow_model_title
    }

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
        if kind == SESSION_TITLE_EVENT {
            self.allow_model_title = false;
        }
        let had_verdict = self.verification.status.is_some();
        let answer_verdict = self.verification.observe(kind, data);
        let verification_invalidated = had_verdict && self.verification.status.is_none();
        match kind {
            "model.request" => {
                self.streamed_thinking_in_turn = false;
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
            "model.text" | "model.refusal" => {
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
                    json!({
                        "seq": receipt.seq,
                        "kind": kind,
                        "payload": data,
                        "verified_status": answer_verdict,
                        "verification_invalidated": verification_invalidated,
                    }),
                );
            }
            // Includes `human.steer`, `human.checkpoint_requested`, `human.decision`, `explain.promoted`.
            _ => {
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({
                        "seq": receipt.seq,
                        "kind": kind,
                        "data": data,
                        "verification_invalidated": verification_invalidated,
                    }),
                );
            }
        }
        Ok(())
    }
}

pub(crate) use crate::session_summary::{builtin_tool_warning, summarize_session_events};

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
    let mut lit_titles: HashMap<String, String> = HashMap::new();
    let mut verification = FinalAnswerVerification::default();
    let mut in_model_response = false;
    let mut response_message = None;
    let mut cli_result = None;
    let mut has_turn_text = false;

    for ev in events {
        let had_verdict = verification.status.is_some();
        let answer_verdict = verification.observe(&ev.kind, &ev.payload);
        if had_verdict
            && verification.status.is_none()
            && !matches!(ev.kind.as_str(), "session.start" | "session.resume")
        {
            if let Some(last) = messages.last_mut()
                .filter(|m| is_primary_assistant(m))
                .and_then(Value::as_object_mut)
            {
                last.remove("verified_status");
                last.remove("verified_entry_id");
            }
        }
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                in_model_response = false;
                cli_result = None;
                has_turn_text = false;
                response_message = None;
                pending_verifications.clear();
                if let Some(prompt) = ev.payload.get("prompt").and_then(Value::as_str) {
                    messages.push(json!({ "role": "user", "text": prompt, "seq": ev.seq }));
                }
            }
            "model.request" => {
                in_model_response = false;
                response_message = None;
            }
            "model.response" => {
                in_model_response = true;
                response_message = None;
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
                    has_turn_text = !text.is_empty();
                    if let Some(index) = response_message {
                        let message: &mut Value = &mut messages[index];
                        if ev.kind == "model.refusal" {
                            message["text"] = json!(text);
                        } else {
                            let prior = message["text"].as_str().unwrap_or_default();
                            message["text"] = json!(format!("{prior}\n{text}"));
                        }
                        continue;
                    }
                    if in_model_response {
                        response_message = Some(messages.len());
                    }
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
            // Manual step reviews belong to their targeted node, which can be from an older turn.
            "proof.verification" if ev.payload["on_demand"] != true => {
                let recorded = ev.payload["verdict"].as_str().unwrap_or_default();
                let verdict = research_tools::effective_verification_verdict(&ev.payload).unwrap_or_default();
                let summary = ev.payload["summary"].as_str().unwrap_or_default();
                let summary = if verdict != recorded {
                    format!("The verifier recorded acceptance alongside blocking issues. {summary}")
                } else {
                    summary.to_string()
                };
                let target = ev.payload.get("target_entry_id").and_then(Value::as_str).unwrap_or("");
                let kind = ev.payload["kind"].as_str().unwrap_or("");
                let scope = if kind == "step" { "Step Verifier" } else { "Verifier" };
                let line = format!("[{scope} {verdict} `{target}`: {summary}]\n");
                let v_obj = json!({
                    "seq": ev.seq,
                    "kind": kind,
                    "verified_status": answer_verdict,
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
                    if let Some(status) = answer_verdict {
                        last["verified_status"] = json!(status);
                        last["verified_entry_id"] = if status == "verified" { json!(target) } else { Value::Null };
                    }
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
                in_model_response = false;
                response_message = None;
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
                if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                    last["verified_status"] = json!("unverified");
                    last["verified_entry_id"] = Value::Null;
                    if let Some(rec) = ev.payload.get("recovery").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        last["recovery"] = json!(rec);
                    }
                }
            }
            "cli.result" => {
                cli_result = ev.payload["result"].as_str().filter(|text| !text.is_empty());
            }
            "session.end" => {
                if let Some(text) = cli_result.take().filter(|_| !has_turn_text) {
                    messages.push(json!({
                        "role": "assistant", "text": text, "seq": ev.seq,
                        "thinking": std::mem::take(&mut pending_thinking),
                        "tool_traces": std::mem::take(&mut pending_turn_traces),
                        "verifications": std::mem::take(&mut pending_verifications),
                    }));
                }
                if let Some(last) = messages.last_mut().filter(|m| is_primary_assistant(m)) {
                    if let Some(rounds) = ev.payload.get("verification_rounds").and_then(Value::as_u64) {
                        last["verification_rounds"] = json!(rounds);
                    }
                    if ev.payload["outcome"].as_str() == Some("completed_unverified") {
                        last["verified_status"] = json!("unverified");
                        last["verified_entry_id"] = Value::Null;
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
            "literature.fetched" => {
                for key in ["papers", "hits"] {
                    if let Some(arr) = ev.payload.get(key).and_then(Value::as_array) {
                        for item in arr {
                            let Some(title) = item.get("title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else { continue };
                            for k in ["id", "base_id", "arxiv_id"] {
                                if let Some(id) = item.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                                    lit_titles.entry(id.to_ascii_lowercase()).or_insert_with(|| title.to_string());
                                }
                            }
                        }
                    }
                }
            }
            "source.ingested" => {
                let mut src = ev.payload.clone();
                if src.get("title").and_then(Value::as_str).is_none_or(|t| t.trim().is_empty()) {
                    let hay = format!(
                        "{} {}",
                        src.get("origin").and_then(Value::as_str).unwrap_or(""),
                        src.get("source_id").and_then(Value::as_str).unwrap_or("")
                    ).to_ascii_lowercase();
                    if let Some((_, t)) = lit_titles.iter().find(|(id, _)| hay.contains(id.as_str())) {
                        src["title"] = json!(t);
                    }
                }
                sources.push(src);
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
    fn completed_cli_result_without_text_survives_reopening() {
        let events = vec![
            ev(1, "session.start", json!({"prompt": "Compute"})),
            ev(2, "cli.result", json!({"result": "Four"})),
            ev(3, "session.end", json!({"cli_exit_code": 0})),
            ev(4, "session.resume", json!({"prompt": "Continue"})),
            ev(5, "cli.result", json!({"result": "Aborted draft"})),
            ev(6, "session.abort", json!({"reason": "stopped"})),
        ];
        let (messages, _, _) = reconstruct_session_view(&events);
        assert_eq!(messages.iter().filter(|msg| msg["role"] == "assistant")
            .map(|msg| msg["text"].as_str().unwrap()).collect::<Vec<_>>(), ["Four"]);
    }

    #[test]
    fn saved_acceptance_with_blocking_issues_is_unverified() {
        let events = vec![
            ev(1, "session.start", json!({ "prompt": "Prove a claim" })),
            ev(2, "proof.entry", json!({ "entry_id": "e-1", "entry_type": "final_answer" })),
            ev(3, "model.text", json!({ "text": "The claim follows." })),
            ev(4, "proof.verification", json!({
                "kind": "full", "target_entry_id": "e-1", "verdict": "accepted",
                "issues": [{ "severity": "error", "message": "A required hypothesis is unstated." }],
            })),
            ev(5, "session.end", json!({ "outcome": "completed", "verified_entry_id": "e-1" })),
        ];
        let stats = summarize_session_events(&events);
        assert_eq!(stats["verified_status"], "unverified");
        assert_eq!(stats["accepted_entries"], 0);
        let (messages, _, _) = reconstruct_session_view(&events);
        assert_eq!(messages[1]["verified_status"], "unverified");
        assert!(messages[1]["verifications"][0]["summary"].as_str().unwrap().contains("recorded acceptance"));
        assert_eq!(events[3].payload["verdict"], "accepted");
    }

    #[test]
    fn step_verdicts_and_unscoped_receipts_do_not_verify_the_answer() {
        let mut events = vec![
            ev(1, "session.start", json!({ "prompt": "Prove A" })),
            ev(2, "proof.entry", json!({ "entry_id": "e-1", "entry_type": "derivation" })),
            ev(3, "proof.entry", json!({ "entry_id": "e-2", "entry_type": "final_answer" })),
            ev(4, "model.text", json!({ "text": "Proof of A" })),
            ev(5, "proof.verification", json!({ "kind": "step", "target_entry_id": "e-2", "verdict": "accepted" })),
        ];
        let stats = summarize_session_events(&events);
        assert_eq!(stats["verified_status"], "none");
        assert_eq!(stats["accepted_entries"], 1);
        assert_eq!(stats["verification_rounds"], 0);
        let (messages, _, _) = reconstruct_session_view(&events);
        assert!(messages[1]["verified_status"].is_null());
        assert_eq!(messages[1]["verifications"][0]["kind"], "step");
        assert!(messages[1]["thinking"].as_str().unwrap().contains("Step Verifier accepted"));

        events.extend([
            ev(6, "proof.verification", json!({ "target_entry_id": "e-2", "verdict": "accepted" })),
            ev(7, "proof.verification", json!({ "kind": "full", "target_entry_id": "e-1", "verdict": "accepted" })),
            ev(8, "session.end", json!({ "outcome": "completed", "verified_entry_id": "e-2" })),
        ]);
        assert_eq!(summarize_session_events(&events)["verified_status"], "none");
        assert!(reconstruct_session_view(&events).0[1]["verified_status"].is_null());
    }

    #[test]
    fn full_verdict_applies_only_to_the_current_active_final_answer() {
        let mut events = vec![
            ev(1, "session.start", json!({ "prompt": "Prove A" })),
            ev(2, "proof.entry", json!({ "entry_id": "e-1", "entry_type": "final_answer" })),
            ev(3, "model.text", json!({ "text": "Proof of A" })),
            ev(4, "proof.verification", json!({ "kind": "full", "target_entry_id": "e-1", "verdict": "accepted",
                "answer_sha256": ledger::sha256_hex(b"Proof of A") })),
            ev(5, "proof.verification", json!({ "kind": "step", "target_entry_id": "e-1", "verdict": "rejected" })),
            ev(6, "session.end", json!({ "outcome": "completed", "verified_entry_id": "e-1" })),
        ];
        assert_eq!(summarize_session_events(&events)["verified_status"], "verified");
        assert_eq!(reconstruct_session_view(&events).0[1]["verified_entry_id"], "e-1");

        events.extend([
            ev(7, "session.resume", json!({ "prompt": "Prove B" })),
            ev(8, "model.text", json!({ "text": "Proof of B" })),
            ev(9, "proof.verification", json!({ "kind": "full", "target_entry_id": "e-1", "verdict": "accepted" })),
        ]);
        let (messages, _, _) = reconstruct_session_view(&events);
        assert_eq!(messages[1]["verified_status"], "verified");
        assert!(messages[3]["verified_status"].is_null());
        assert_eq!(summarize_session_events(&events)["verified_status"], "none");

        events.extend([
            ev(10, "proof.entry", json!({ "entry_id": "e-2", "entry_type": "final_answer" })),
            ev(11, "proof.verification", json!({ "kind": "full", "target_entry_id": "e-2", "verdict": "accepted",
                "answer_sha256": ledger::sha256_hex(b"Proof of B") })),
        ]);
        assert_eq!(reconstruct_session_view(&events).0[3]["verified_entry_id"], "e-2");
        assert_eq!(summarize_session_events(&events)["verified_status"], "verified");

        events.extend([
            ev(12, "proof.entry", json!({ "entry_id": "e-3", "entry_type": "final_answer" })),
            ev(13, "proof.supersede", json!({ "entry_id": "e-2", "replaced_by": "e-3" })),
            ev(14, "proof.verification", json!({ "kind": "full", "target_entry_id": "e-2", "verdict": "accepted" })),
        ]);
        assert!(reconstruct_session_view(&events).0[3]["verified_status"].is_null());
        assert_eq!(summarize_session_events(&events)["verified_status"], "none");
    }

    #[test]
    fn live_verification_status_follows_current_final_answer() {
        let sse = Arc::new(SseHub::new());
        let (_, frames) = sse.subscribe("s-live");
        let mut sink = SseEventSink::new(sse, "s-live");
        let mut receipt = AppendedEvent {
            seq: 0,
            ts_ms: 0,
            prev_hash: String::new(),
            event_hash: String::new(),
        };
        let mut emit = |kind, payload| -> Value {
            receipt.seq += 1;
            EventSink::on_event(&mut sink, &receipt, kind, &payload).unwrap();
            let crate::sse::SseFrame::Event { data, .. } = frames.try_recv().unwrap() else {
                panic!("expected a live event");
            };
            serde_json::from_str(&data).unwrap()
        };

        emit("proof.entry", json!({ "entry_id": "e-1", "entry_type": "final_answer" }));
        let step = emit("proof.verification", json!({ "kind": "step", "target_entry_id": "e-1", "verdict": "accepted" }));
        assert!(step["verified_status"].is_null());
        emit("model.response", json!({}));
        emit("model.text", json!({"text": "First block."}));
        emit("model.text", json!({"text": "Second block."}));
        let old_review = emit("proof.verification", json!({ "kind": "full", "target_entry_id": "e-1", "verdict": "accepted" }));
        assert_eq!(old_review["verified_status"], "proof_reviewed");
        let mismatch = emit("proof.verification", json!({ "kind": "full", "target_entry_id": "e-1", "verdict": "accepted",
            "answer_sha256": ledger::sha256_hex(b"Second block.") }));
        assert_eq!(mismatch["verified_status"], "unverified");
        let full = emit("proof.verification", json!({ "kind": "full", "target_entry_id": "e-1", "verdict": "accepted",
            "answer_sha256": ledger::sha256_hex(b"First block.\nSecond block.") }));
        assert_eq!(full["verified_status"], "verified");
        let replacement = emit("proof.entry", json!({ "entry_id": "e-2", "entry_type": "final_answer" }));
        assert_eq!(replacement["verification_invalidated"], true);
        assert_eq!(sink.final_text(), "First block.\nSecond block.");
    }

    #[test]
    fn reconstructed_verdict_is_attached_to_entire_model_response() {
        let events = [
            ev(1, "session.start", json!({"prompt": "Expand"})),
            ev(2, "proof.entry", json!({"entry_id": "e-1", "entry_type": "final_answer"})),
            ev(3, "model.response", json!({})),
            ev(4, "model.text", json!({"text": "First block."})),
            ev(5, "model.text", json!({"text": "Second block."})),
            ev(6, "proof.verification", json!({"kind": "full", "target_entry_id": "e-1",
                "verdict": "accepted", "answer_sha256": ledger::sha256_hex(b"First block.\nSecond block.")})),
        ];
        let (messages, _, _) = reconstruct_session_view(&events);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["text"], "First block.\nSecond block.");
        assert_eq!(messages[1]["verified_status"], "verified");
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
