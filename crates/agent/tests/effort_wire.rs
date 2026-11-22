//! Reasoning-effort wire serialization across Anthropic, OpenAI (official vs Ollama),
//! ChatGPT Responses API, and Google Gemini.

use agent::responses::serialize_responses_request;
use agent::{
    normalize_reasoning_effort, AnthropicConfig, AnthropicTransport, BearerSource, ChatGptConfig,
    ChatGptTransport, ContentBlock, GeminiConfig, GeminiTransport, Message, ModelRequest, OpenAiConfig,
    OpenAiTransport, ProviderKind, Role, SecretKey, Transport, TransportError,
};
use serde_json::{json, Value};
use std::sync::Arc;

struct StaticBearer;
impl BearerSource for StaticBearer {
    fn bearer(&self) -> Result<SecretKey, TransportError> {
        SecretKey::new("tok-effort-test".into())
    }
}

fn sample_req(model: &str, effort: Option<&str>) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        system: Some("You are a test solver.".into()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: "Prove 1+1=2".into() }],
        }],
        tools: vec![],
        max_tokens: 128,
        reasoning_effort: effort.map(str::to_string),
    }
}

#[test]
fn normalize_effort_accepts_canonical_values_and_omits_default() {
    for valid in ["low", "medium", "high", "xhigh", "max"] {
        assert_eq!(normalize_reasoning_effort(Some(valid)).as_deref(), Some(valid));
    }
    assert_eq!(normalize_reasoning_effort(Some("  HIGH ")).as_deref(), Some("high"));
    for ignored in [None, Some(""), Some("   "), Some("default"), Some("unknown")] {
        assert_eq!(normalize_reasoning_effort(ignored), None, "{ignored:?}");
    }
}

#[test]
fn anthropic_serializes_output_config_only_when_effort_set() {
    let t = AnthropicTransport::new(
        AnthropicConfig::default(),
        SecretKey::new("sk-ant-test".into()).unwrap(),
        "RE_SEARCH_API_KEY",
    )
    .unwrap();

    let none_json: Value = serde_json::from_slice(&t.prepare_request(&sample_req("claude-sonnet-4-6", None)).unwrap().body).unwrap();
    assert!(none_json.get("output_config").is_none());

    let def_json: Value = serde_json::from_slice(&t.prepare_request(&sample_req("claude-sonnet-4-6", Some("default"))).unwrap().body).unwrap();
    assert!(def_json.get("output_config").is_none());

    let high_json: Value = serde_json::from_slice(&t.prepare_request(&sample_req("claude-sonnet-4-6", Some("high"))).unwrap().body).unwrap();
    assert_eq!(high_json["output_config"], json!({ "effort": "high" }));

    let xhigh_json: Value = serde_json::from_slice(&t.prepare_request(&sample_req("claude-sonnet-4-6", Some("xhigh"))).unwrap().body).unwrap();
    assert_eq!(xhigh_json["output_config"], json!({ "effort": "max" }));
}

#[test]
fn openai_serializes_reasoning_effort_while_ollama_omits_it() {
    let openai = OpenAiTransport::new(
        OpenAiConfig::default(),
        Some(SecretKey::new("sk-openai-test".into()).unwrap()),
        "OPENAI_API_KEY",
    )
    .unwrap();
    let ollama = OpenAiTransport::from_env(OpenAiConfig::ollama_default()).unwrap();

    let none_json: Value = serde_json::from_slice(&openai.prepare_request(&sample_req("gpt-5", None)).unwrap().body).unwrap();
    assert!(none_json.get("reasoning_effort").is_none());

    let med_json: Value = serde_json::from_slice(&openai.prepare_request(&sample_req("gpt-5", Some("medium"))).unwrap().body).unwrap();
    assert_eq!(med_json["reasoning_effort"], json!("medium"));

    let max_json: Value = serde_json::from_slice(&openai.prepare_request(&sample_req("gpt-5", Some("max"))).unwrap().body).unwrap();
    assert_eq!(max_json["reasoning_effort"], json!("high"));

    let ollama_json: Value = serde_json::from_slice(&ollama.prepare_request(&sample_req("qwen2.5-coder:7b", Some("high"))).unwrap().body).unwrap();
    assert_eq!(ollama.info().provider, ProviderKind::Ollama.wire_id());
    assert!(ollama_json.get("reasoning_effort").is_none());
}

#[test]
fn chatgpt_responses_serializes_reasoning_effort_when_set() {
    let t = ChatGptTransport::new(ChatGptConfig::default(), Arc::new(StaticBearer)).unwrap();
    let none_json: Value = serde_json::from_slice(&t.prepare_request(&sample_req("gpt-6.2-sol", None)).unwrap().body).unwrap();
    assert!(none_json.get("reasoning").is_none());

    let xhigh_json: Value = serde_json::from_slice(&serialize_responses_request(&sample_req("gpt-6.2-sol", Some("xhigh"))).unwrap()).unwrap();
    assert_eq!(xhigh_json["reasoning"], json!({ "effort": "xhigh" }));

    let max_json: Value = serde_json::from_slice(&serialize_responses_request(&sample_req("gpt-6.2-sol", Some("max"))).unwrap()).unwrap();
    assert_eq!(max_json["reasoning"], json!({ "effort": "xhigh" }));
}

#[test]
fn gemini_serializes_thinking_level_when_set() {
    let t = GeminiTransport::new(
        GeminiConfig::default(),
        SecretKey::new("AIza-test-key".into()).unwrap(),
        "GEMINI_API_KEY",
    )
    .unwrap();
    let none_json: Value = serde_json::from_slice(&t.prepare_request(&sample_req("gemini-2.5-flash", None)).unwrap().body).unwrap();
    assert!(none_json["generationConfig"].get("thinkingConfig").is_none());

    for (raw, expected) in [("low", "LOW"), ("medium", "MEDIUM"), ("high", "HIGH"), ("max", "HIGH")] {
        let j: Value = serde_json::from_slice(&t.prepare_request(&sample_req("gemini-2.5-flash", Some(raw))).unwrap().body).unwrap();
        assert_eq!(j["generationConfig"]["thinkingConfig"], json!({ "thinkingLevel": expected }), "{raw}");
    }
}
