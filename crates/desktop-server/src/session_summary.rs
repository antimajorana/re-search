//! Session-level telemetry aggregation and Antigravity built-in tool notice formatting.

use serde_json::{json, Value};

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
