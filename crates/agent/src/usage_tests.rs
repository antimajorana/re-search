use super::*;
use serde_json::json;

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-usage".to_string(),
        seq,
        ts_ms: (seq as i64) * 1000,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: ledger::GENESIS_HASH.to_string(),
        event_hash: ledger::GENESIS_HASH.to_string(),
    }
}

#[test]
fn extract_usage_handles_null_missing_and_precedence_honestly() {
    assert_eq!(extract_usage(&Value::Null), TokenUsage { input: None, output: None });
    assert_eq!(extract_usage(&json!({})), TokenUsage { input: None, output: None });

    // Crucial regression for delegate.rs:232 Value::Null short-circuit bug.
    assert_eq!(
        extract_usage(&json!({ "input_tokens": null, "prompt_tokens": 42, "output_tokens": null, "completion_tokens": 9 })),
        TokenUsage { input: Some(42), output: Some(9) }
    );
    assert_eq!(
        extract_usage(&json!({ "promptTokenCount": 100, "candidatesTokenCount": 25 })),
        TokenUsage { input: Some(100), output: Some(25) }
    );
    // Genuine zero stays Some(0), distinct from None.
    assert_eq!(
        extract_usage(&json!({ "input_tokens": 0, "output_tokens": 0 })),
        TokenUsage { input: Some(0), output: Some(0) }
    );
    // Negative or string fields never coerce to 0.
    assert_eq!(
        extract_usage(&json!({ "input_tokens": -5, "output_tokens": "12" })),
        TokenUsage { input: None, output: None }
    );
}

#[test]
fn inject_openai_stream_usage_only_applies_to_streaming_openai() {
    let raw = serde_json::to_vec(&json!({ "model": "gpt-4.1-mini", "stream": true })).unwrap();

    // OpenAI + stream = true injects stream_options.include_usage = true.
    let oai_stream = inject_openai_stream_usage(raw.clone(), ProviderKind::OpenAi, true);
    let parsed: Value = serde_json::from_slice(&oai_stream).unwrap();
    assert_eq!(parsed["stream_options"]["include_usage"], json!(true));

    // Preserves unrelated stream_options fields.
    let with_extra = serde_json::to_vec(&json!({
        "model": "gpt-4.1-mini",
        "stream_options": { "foo": "bar" }
    }))
    .unwrap();
    let merged: Value = serde_json::from_slice(&inject_openai_stream_usage(
        with_extra,
        ProviderKind::OpenAi,
        true,
    ))
    .unwrap();
    assert_eq!(merged["stream_options"]["foo"], json!("bar"));
    assert_eq!(merged["stream_options"]["include_usage"], json!(true));

    // Strip helper removes stream_options cleanly for 400 fallback gateways.
    let stripped: Value = serde_json::from_slice(&strip_openai_stream_options(oai_stream)).unwrap();
    assert!(stripped.get("stream_options").is_none());

    // Non-streaming OpenAI, Ollama, Anthropic, Gemini, ChatGpt remain untouched.
    assert_eq!(inject_openai_stream_usage(raw.clone(), ProviderKind::OpenAi, false), raw);
    assert_eq!(inject_openai_stream_usage(raw.clone(), ProviderKind::Ollama, true), raw);
    assert_eq!(inject_openai_stream_usage(raw.clone(), ProviderKind::Anthropic, true), raw);
    assert_eq!(inject_openai_stream_usage(raw.clone(), ProviderKind::Gemini, true), raw);
    assert_eq!(inject_openai_stream_usage(raw.clone(), ProviderKind::ChatGpt, true), raw);
}

#[test]
fn rollup_serializes_purely_unknown_tokens_as_null_and_mixes_honestly() {
    let events = vec![
        ev(1, "session.start", json!({ "provider": "anthropic", "model": "claude-sonnet-4-6" })),
        ev(2, "model.request", json!({ "turn": 1 })),
        ev(3, "model.response", json!({
            "turn": 1, "model": "claude-sonnet-4-6",
            "usage": { "input_tokens": 200, "output_tokens": 50 }
        })),
        // Worker with purely unknown usage (e.g., local Ollama SSE without usage).
        ev(4, "delegate.start", json!({
            "call_id": "c-ollama", "role": "paper_reader", "provider": "ollama", "model": "qwen2.5:7b"
        })),
        ev(5, "delegate.turn", json!({
            "call_id": "c-ollama", "role": "paper_reader", "turn": 1, "model": "qwen2.5:7b", "usage": null
        })),
        ev(6, "delegate.end", json!({
            "call_id": "c-ollama", "worker": "paper_reader", "provider": "ollama",
            "model": "qwen2.5:7b", "turns": 2, "input_tokens": null, "output_tokens": null
        })),
        // Verifier with mixed known and unknown calls.
        ev(7, "verifier.request", json!({
            "role": "verifier", "provider": "openai", "verifier_model": "gpt-4.1-mini"
        })),
        ev(8, "proof.verification", json!({
            "kind": "step", "target_entry_id": "e-1", "verdict": "accepted",
            "verifier_model": "gpt-4.1-mini",
            "usage": { "prompt_tokens": 110, "completion_tokens": 30 }
        })),
        ev(9, "verifier.request", json!({
            "role": "verifier", "provider": "openai", "verifier_model": "gpt-4.1-mini"
        })),
        ev(10, "verifier.cancelled", json!({ "kind": "step" })),
    ];

    let rollup = RoleUsageRollup::from_events(&events);
    let json_val = rollup.to_json();
    let roles = json_val["roles"].as_array().unwrap();
    assert_eq!(roles.len(), 3);

    let reader = roles.iter().find(|r| r["role"] == "paper_reader").unwrap();
    assert_eq!(reader["provider"], "ollama");
    assert_eq!(reader["used_model"], "qwen2.5:7b");
    assert_eq!(reader["calls"], 2);
    assert!(reader["input_tokens"].is_null(), "unknown usage must serialize as null: {reader}");
    assert!(reader["output_tokens"].is_null(), "unknown usage must serialize as null: {reader}");
    assert_eq!(reader["unknown_usage_calls"], 2);

    let verifier = roles.iter().find(|r| r["role"] == "verifier").unwrap();
    assert_eq!(verifier["calls"], 2);
    assert_eq!(verifier["input_tokens"], 110);
    assert_eq!(verifier["output_tokens"], 30);
    assert_eq!(verifier["unknown_usage_calls"], 1);

    assert_eq!(json_val["totals"]["calls"], 5);
    assert_eq!(json_val["totals"]["input_tokens"], 310);
    assert_eq!(json_val["totals"]["output_tokens"], 80);
    assert_eq!(json_val["totals"]["unknown_usage_calls"], 3);
}
