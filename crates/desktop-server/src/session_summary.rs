//! Session-level telemetry aggregation and Antigravity built-in tool notice formatting.

use serde_json::{json, Value};
use std::collections::HashMap;

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
    format!("Antigravity built-in tool `{name}` {outcome}.")
}

#[derive(Default)]
pub(crate) struct FinalAnswerVerification {
    active_finals: Vec<String>,
    answer: String,
    response_blocks: Option<usize>,
    pub(crate) status: Option<&'static str>,
}

impl FinalAnswerVerification {
    fn current_entry_id(&self) -> Option<&str> {
        self.active_finals.last().map(String::as_str)
    }

    pub(crate) fn answer(&self) -> &str {
        &self.answer
    }

    /// Returns a verdict only when the event verifies the current turn's active final answer.
    pub(crate) fn observe(&mut self, kind: &str, payload: &Value) -> Option<&'static str> {
        let is_full = payload["kind"].as_str() == Some("full");
        match kind {
            "session.start" | "session.resume" | "autonomous.feedback" => *self = Self::default(),
            "model.response" => {
                self.answer.clear();
                self.response_blocks = Some(0);
                self.status = None;
            }
            "model.text" | "model.refusal" => {
                let text = payload["text"].as_str().unwrap_or_default();
                match (&mut self.response_blocks, kind) {
                    (Some(count), "model.text") => {
                        if *count > 0 {
                            self.answer.push('\n');
                        }
                        self.answer.push_str(text);
                        *count += 1;
                    }
                    _ => self.answer = text.to_string(),
                }
                self.status = None;
            }
            "proof.entry" => {
                if payload["entry_type"].as_str() == Some("final_answer") {
                    if let Some(id) = payload["entry_id"].as_str().filter(|id| !id.is_empty()) {
                        self.active_finals.push(id.to_string());
                    }
                }
                self.status = None;
            }
            "proof.supersede" => {
                if let Some(id) = payload["entry_id"].as_str() {
                    self.active_finals.retain(|entry| entry != id);
                }
                self.status = None;
            }
            "proof.verification_attempt" if is_full => self.status = None,
            "session.abort" => self.status = None,
            "proof.verification" if is_full => {
                if payload["target_entry_id"].as_str().is_some_and(|id| self.current_entry_id() == Some(id)) {
                    self.status = match research_tools::effective_verification_verdict(payload) {
                        Some("accepted") => match payload.get("answer_sha256") {
                            None => Some("proof_reviewed"),
                            Some(hash)
                                if !self.answer.trim().is_empty()
                                    && hash.as_str() == Some(ledger::sha256_hex(self.answer.as_bytes()).as_str()) =>
                            {
                                Some("verified")
                            }
                            Some(_) => Some("unverified"),
                        },
                        Some("rejected") => Some("unverified"),
                        _ => None,
                    };
                    return self.status;
                }
            }
            "guardrail.warning" => self.status = Some("unverified"),
            "session.end" if payload["outcome"].as_str() == Some("completed_unverified") => {
                self.status = Some("unverified");
            }
            _ => {}
        }
        None
    }
}

pub(crate) fn summarize_session_events(events: &[ledger::Event]) -> Value {
    let mut proof_entries = 0u64;
    let mut entry_acceptance = HashMap::new();
    let mut verification_rounds = 0u64;
    let mut step_checks = 0u64;
    let mut strategist_consultations = 0u64;
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut worker_input_tokens = 0u64;
    let mut worker_output_tokens = 0u64;
    let mut delegation_count = 0u64;
    let mut honesty_warnings = 0u64;
    let mut model = String::new();
    let mut verification = FinalAnswerVerification::default();

    for ev in events {
        verification.observe(&ev.kind, &ev.payload);
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
                if let Some(id) = ev.payload["entry_id"].as_str() {
                    entry_acceptance.insert(id, ev.payload["status"].as_str() == Some("accepted"));
                }
            }
            "proof.supersede" => {
                if let Some(id) = ev.payload["entry_id"].as_str() {
                    entry_acceptance.remove(id);
                }
            }
            "proof.verification" => {
                if ev.payload["kind"].as_str() == Some("full") {
                    verification_rounds += 1;
                } else if ev.payload["kind"].as_str() == Some("step") {
                    step_checks += 1;
                }
                if let Some(accepted) = ev.payload["target_entry_id"]
                    .as_str()
                    .and_then(|id| entry_acceptance.get_mut(id))
                {
                    *accepted = research_tools::effective_verification_verdict(&ev.payload) == Some("accepted");
                }
            }
            "strategist.consult" => strategist_consultations += 1,
            _ => {}
        }
    }
    let campaign_phase = agent::infer_campaign_phase(events).as_str();

    json!({
        "proof_entries": proof_entries,
        "accepted_entries": entry_acceptance.values().filter(|&&accepted| accepted).count(),
        "verification_rounds": verification_rounds,
        "step_checks": step_checks,
        "strategist_consultations": strategist_consultations,
        "verified_status": verification.status.unwrap_or("none"),
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
