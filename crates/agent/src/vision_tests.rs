use super::*;
use crate::anthropic::{AnthropicConfig, AnthropicTransport, SecretKey};
use crate::gemini::{GeminiConfig, GeminiTransport};
use crate::openai::{OpenAiConfig, OpenAiTransport};
use crate::responses::{serialize_responses_request, BearerSource, ChatGptConfig, ChatGptTransport};
use crate::transport::{Message, ModelRequest, Role, Transport, TransportErrorKind};
use std::sync::Arc;

fn sample_png() -> Vec<u8> {
    // Minimal synthetic PNG header + IHDR + IEND bytes (no third-party data).
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    bytes
}

fn sample_jpeg() -> Vec<u8> {
    vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0xFF, 0xD9]
}

fn sample_webp() -> Vec<u8> {
    b"RIFF\x10\x00\x00\x00WEBPVP8 \x04\x00\x00\x00test".to_vec()
}

struct DummyBearer;
impl BearerSource for DummyBearer {
    fn bearer(&self) -> Result<SecretKey, TransportError> {
        SecretKey::new("test-bearer-token".into())
    }
}

fn vision_request(model: &str, media_type: &str, raw: &[u8]) -> ModelRequest {
    let img = build_image_block(media_type, raw).expect("valid image block");
    ModelRequest {
        model: model.to_string(),
        system: Some("Transcribe the equation crop.".into()),
        messages: vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text { text: "Read equation 1.".into() },
                img,
            ],
        }],
        tools: Vec::new(),
        max_tokens: 512,
        reasoning_effort: None,
    }
}

#[test]
fn base64_rfc4648_vectors_and_strict_rejections() {
    // RFC 4648 Section 10 test vectors.
    let vectors: &[(&[u8], &str)] = &[
        (b"", ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"fooba", "Zm9vYmE="),
        (b"foobar", "Zm9vYmFy"),
        (&[0x00, 0xFF, 0x80, 0x7F], "AP+Afw=="),
    ];
    for &(raw, expected) in vectors {
        let enc = encode_base64(raw);
        assert_eq!(enc, expected, "encode mismatch for {raw:?}");
        if !raw.is_empty() {
            let dec = decode_base64_strict(&enc, 64).expect("valid base64 must decode");
            assert_eq!(dec, raw, "roundtrip mismatch for {raw:?}");
        }
    }
    assert!(decode_base64_strict("", 64).is_err());
    assert!(decode_base64_strict("abc", 64).is_err()); // non-multiple of 4
    assert!(decode_base64_strict("Zm 8=", 64).is_err()); // whitespace
    assert!(decode_base64_strict("Zm9v\n", 64).is_err()); // newline
    assert!(decode_base64_strict("Zh==", 64).is_err()); // non-canonical trailing bits (0x21 & 0x0F != 0)
    assert!(decode_base64_strict("Zm9=", 64).is_err()); // non-canonical trailing bits ('9'=61, 61 & 0x03 != 0)
    assert!(decode_base64_strict("====", 64).is_err());
    assert!(decode_base64_strict(&encode_base64(&[1u8; 10]), 8).is_err()); // exceeds max_decoded_bytes
}

#[test]
fn mime_allowlist_magic_bytes_and_size_caps_enforced() {
    assert_eq!(detect_image_media_type(&sample_png()), Some("image/png"));
    assert_eq!(detect_image_media_type(&sample_jpeg()), Some("image/jpeg"));
    assert_eq!(detect_image_media_type(&sample_webp()), Some("image/webp"));
    assert_eq!(detect_image_media_type(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);

    // Disallowed MIME types rejected.
    for bad_mime in ["image/svg+xml", "text/html", "application/pdf", "image/gif", "IMAGE/PNG"] {
        let err = build_image_block(bad_mime, &sample_png()).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config);
    }

    // Spoofed bytes rejected (JPEG bytes declared as PNG).
    let spoof = build_image_block("image/png", &sample_jpeg()).unwrap_err();
    assert_eq!(spoof.kind, TransportErrorKind::Config);
    assert!(spoof.message.contains("magic bytes"));

    // Oversized single image rejected.
    let mut big = sample_png();
    big.resize(MAX_IMAGE_BYTES + 1, 0u8);
    let too_big = build_image_block("image/png", &big).unwrap_err();
    assert_eq!(too_big.kind, TransportErrorKind::Config);

    // More than MAX_IMAGES_PER_REQUEST rejected.
    let img = build_image_block("image/png", &sample_png()).unwrap();
    let req_many = ModelRequest {
        model: "claude-sonnet-4-5".into(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![img; MAX_IMAGES_PER_REQUEST + 1],
        }],
        tools: Vec::new(),
        max_tokens: 128,
        reasoning_effort: None,
    };
    assert_eq!(
        validate_request_images(ProviderKind::Anthropic, &req_many).unwrap_err().kind,
        TransportErrorKind::Config
    );

    // Total request bytes cap (4 images of 4.5 MiB each = 18 MiB > 16 MiB).
    let mut chunk_4_5m = sample_png();
    chunk_4_5m.resize(4_500_000, 0x11);
    let large_block = build_image_block("image/png", &chunk_4_5m).unwrap();
    let req_heavy = ModelRequest {
        model: "gpt-4o".into(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![large_block; 4],
        }],
        tools: Vec::new(),
        max_tokens: 128,
        reasoning_effort: None,
    };
    let heavy_err = validate_request_images(ProviderKind::OpenAi, &req_heavy).unwrap_err();
    assert_eq!(heavy_err.kind, TransportErrorKind::Config);
    assert!(heavy_err.message.contains("total request image bytes"));

    // Assistant role image block rejected fail-closed.
    let req_asst = ModelRequest {
        model: "gpt-4o".into(),
        system: None,
        messages: vec![Message {
            role: Role::Assistant,
            content: vec![build_image_block("image/png", &sample_png()).unwrap()],
        }],
        tools: Vec::new(),
        max_tokens: 128,
        reasoning_effort: None,
    };
    assert_eq!(
        validate_request_images(ProviderKind::OpenAi, &req_asst).unwrap_err().kind,
        TransportErrorKind::Config
    );
}

#[test]
fn all_four_providers_serialize_multimodal_image_wire_json() {
    let png = sample_png();
    let png_b64 = encode_base64(&png);
    let jpeg = sample_jpeg();
    let jpeg_b64 = encode_base64(&jpeg);
    let webp = sample_webp();
    let webp_b64 = encode_base64(&webp);

    // 1. Anthropic (`wire.rs`): type = "image", source = { type: "base64", media_type, data }.
    let anthropic = AnthropicTransport::new(
        AnthropicConfig { base_url: "http://127.0.0.1:9".into(), ..AnthropicConfig::default() },
        SecretKey::new("test-key".into()).unwrap(),
        "TEST",
    )
    .unwrap();
    let prep = anthropic.prepare_request(&vision_request("claude-sonnet-4-5", "image/png", &png)).unwrap();
    let body: Value = serde_json::from_slice(&prep.body).unwrap();
    assert_eq!(body["messages"][0]["content"][0], json!({ "type": "text", "text": "Read equation 1." }));
    assert_eq!(
        body["messages"][0]["content"][1],
        json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": png_b64 }
        })
    );

    // 2a. OpenAI (`openai.rs`): image_url data URI in content array.
    let openai = OpenAiTransport::new(
        OpenAiConfig {
            provider: ProviderKind::OpenAi,
            base_url: "http://127.0.0.1:9".into(),
            ..OpenAiConfig::default()
        },
        Some(SecretKey::new("test-key".into()).unwrap()),
        "TEST",
    )
    .unwrap();
    let prep = openai.prepare_request(&vision_request("gpt-4.1-mini", "image/jpeg", &jpeg)).unwrap();
    let body: Value = serde_json::from_slice(&prep.body).unwrap();
    assert_eq!(body["messages"][1]["role"], json!("user"));
    assert_eq!(body["messages"][1]["content"][0], json!({ "type": "text", "text": "Read equation 1." }));
    assert_eq!(
        body["messages"][1]["content"][1],
        json!({
            "type": "image_url",
            "image_url": { "url": format!("data:image/jpeg;base64,{jpeg_b64}") }
        })
    );

    // 2b. Ollama (`openai.rs`): vision-capable model (`llava:13b`) uses image_url data URI.
    let ollama = OpenAiTransport::new(OpenAiConfig::ollama_default(), None, "none").unwrap();
    let prep = ollama.prepare_request(&vision_request("llava:13b", "image/webp", &webp)).unwrap();
    let body: Value = serde_json::from_slice(&prep.body).unwrap();
    assert_eq!(
        body["messages"][1]["content"][1],
        json!({
            "type": "image_url",
            "image_url": { "url": format!("data:image/webp;base64,{webp_b64}") }
        })
    );

    // 3. Gemini (`gemini.rs`): inlineData { mimeType, data }.
    let gemini = GeminiTransport::new(
        GeminiConfig {
            base_url: "http://127.0.0.1:9".into(),
            model: "gemini-2.5-flash".into(),
            ..GeminiConfig::default()
        },
        SecretKey::new("test-key".into()).unwrap(),
        "TEST",
    )
    .unwrap();
    let prep = gemini.prepare_request(&vision_request("gemini-2.5-flash", "image/png", &png)).unwrap();
    let body: Value = serde_json::from_slice(&prep.body).unwrap();
    assert_eq!(body["contents"][0]["parts"][0], json!({ "text": "Read equation 1." }));
    assert_eq!(
        body["contents"][0]["parts"][1],
        json!({ "inlineData": { "mimeType": "image/png", "data": png_b64 } })
    );

    // 4. ChatGPT Responses (`responses.rs`): input_text + input_image data URI.
    let chatgpt = ChatGptTransport::new(
        ChatGptConfig { base_url: "http://127.0.0.1:9".into(), ..ChatGptConfig::default() },
        Arc::new(DummyBearer),
    )
    .unwrap();
    let prep = chatgpt.prepare_request(&vision_request("gpt-6.2-sol", "image/png", &png)).unwrap();
    let body: Value = serde_json::from_slice(&prep.body).unwrap();
    assert_eq!(body["input"][0]["role"], json!("user"));
    assert_eq!(body["input"][0]["content"][0], json!({ "type": "input_text", "text": "Read equation 1." }));
    assert_eq!(
        body["input"][0]["content"][1],
        json!({
            "type": "input_image",
            "image_url": format!("data:image/png;base64,{png_b64}")
        })
    );
}

#[test]
fn non_vision_models_fail_closed_across_all_providers_without_dropping_images() {
    let png = sample_png();

    // Anthropic non-vision fails closed.
    let anthropic = AnthropicTransport::new(
        AnthropicConfig { base_url: "http://127.0.0.1:9".into(), ..AnthropicConfig::default() },
        SecretKey::new("test-key".into()).unwrap(),
        "TEST",
    )
    .unwrap();
    let err = anthropic.prepare_request(&vision_request("claude-2.1", "image/png", &png)).unwrap_err();
    assert_eq!(err.kind, TransportErrorKind::Config);
    assert!(err.message.contains("not vision-capable"));

    // OpenAI non-vision (`gpt-3.5-turbo`, `o1-mini`, `o3-mini`) fail closed.
    let openai = OpenAiTransport::new(
        OpenAiConfig {
            provider: ProviderKind::OpenAi,
            base_url: "http://127.0.0.1:9".into(),
            ..OpenAiConfig::default()
        },
        Some(SecretKey::new("test-key".into()).unwrap()),
        "TEST",
    )
    .unwrap();
    for bad in [
        "gpt-3.5-turbo",
        "o1-mini",
        "o3-mini",
        "unknown-text-model",
        "gpt-4o-audio-preview",
        "gpt-4o-realtime-preview",
        "gpt-4o-mini-tts",
        "gpt-4o-transcribe",
        "gpt-4o-search-preview",
        "gpt-4-turbo-preview",
        "gpt-4-turbo-0125-preview",
        "o3-deep-research",
        "o1-pro",
        "o3-pro",
        "gpt-5-codex",
    ] {
        let err = openai.prepare_request(&vision_request(bad, "image/png", &png)).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "expected Config rejection for {bad}");
    }
    assert!(supports_vision(ProviderKind::OpenAi, "gpt-4-turbo-2024-04-09"));

    // Ollama default code model (`qwen2.5-coder:7b`) and non-vision/ambiguous `gemma3` fail closed.
    let ollama = OpenAiTransport::new(OpenAiConfig::ollama_default(), None, "none").unwrap();
    for bad in [
        "qwen2.5-coder:7b",
        "gemma3:1b",
        "gemma3:1b-instruct-q4_0",
        "gemma3:270m",
        "gemma3:latest",
        "gemma3",
    ] {
        let err = ollama.prepare_request(&vision_request(bad, "image/png", &png)).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "expected Config rejection for {bad}");
    }
    for ok_ollama in ["gemma3:4b", "gemma3:12b-it-qat", "gemma3:27b"] {
        assert!(supports_vision(ProviderKind::Ollama, ok_ollama), "expected vision for {ok_ollama}");
    }

    // Gemini non-vision (`gemini-1.0-pro`, TTS, native-audio) fails closed.
    for bad in [
        "gemini-1.0-pro",
        "gemini-2.5-flash-preview-tts",
        "gemini-2.5-flash-native-audio-preview",
    ] {
        let gemini = GeminiTransport::new(
            GeminiConfig {
                base_url: "http://127.0.0.1:9".into(),
                model: bad.into(),
                ..GeminiConfig::default()
            },
            SecretKey::new("test-key".into()).unwrap(),
            "TEST",
        )
        .unwrap();
        let err = gemini.prepare_request(&vision_request(bad, "image/png", &png)).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "expected Config rejection for {bad}");
    }

    // ChatGPT Responses non-vision (`o3-mini`, `o3-deep-research`, `gpt-5-codex`) fails closed.
    for bad in ["o3-mini", "o3-deep-research", "gpt-5-codex"] {
        let err = serialize_responses_request(&vision_request(bad, "image/png", &png)).unwrap_err();
        assert_eq!(err.kind, TransportErrorKind::Config, "expected Config rejection for {bad}");
    }

    // Text-only requests on non-vision models remain unaffected.
    let text_only = ModelRequest {
        model: "qwen2.5-coder:7b".into(),
        system: None,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: "Hello".into() }],
        }],
        tools: Vec::new(),
        max_tokens: 64,
        reasoning_effort: None,
    };
    assert!(ollama.prepare_request(&text_only).is_ok());
}
