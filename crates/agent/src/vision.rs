//! Multimodal image content blocks, pure-Rust base64 codec, magic-byte & size validation,
//! fail-closed vision-capability checks, and provider wire encoders (package F14c).
//!
//! Zero external crates added (`Cargo.lock` untouched). Images are never silently dropped:
//! if a request carries any [`ContentBlock::Image`] and the selected `(ProviderKind, model)`
//! is not explicitly vision-capable, serialization fails closed with
//! [`crate::TransportErrorKind::Config`].

use crate::http::config_err;
use crate::transport::{ContentBlock, ModelRequest, ProviderKind, Role, TransportError};
use serde_json::{json, Value};

/// Strict MIME allowlist for multimodal image blocks (SVG/HTML/PDF/etc. are refused).
pub const ALLOWED_IMAGE_MEDIA_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

/// Maximum decoded byte size for a single image block (5 MiB).
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Maximum total decoded image bytes across one [`ModelRequest`] (16 MiB).
pub const MAX_REQUEST_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum number of image blocks allowed in a single [`ModelRequest`].
pub const MAX_IMAGES_PER_REQUEST: usize = 8;

const MAX_IMAGE_BASE64_LEN: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;
const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Pure-Rust RFC 4648 standard base64 encoder with `=` padding (no external crate).
pub fn encode_base64(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_TABLE[((triple >> 18) & 0x3F) as usize]);
        out.push(B64_TABLE[((triple >> 12) & 0x3F) as usize]);
        out.push(if chunk.len() > 1 {
            B64_TABLE[((triple >> 6) & 0x3F) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            B64_TABLE[(triple & 0x3F) as usize]
        } else {
            b'='
        });
    }
    String::from_utf8(out).unwrap_or_default()
}

fn decode_b64_char(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Strict RFC 4648 standard base64 decoder.
///
/// Rejects whitespace, control characters, invalid padding, non-canonical trailing bits,
/// and inputs exceeding `max_decoded_bytes` before allocating full output.
pub fn decode_base64_strict(input: &str, max_decoded_bytes: usize) -> Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return Err("base64 image payload is empty".into());
    }
    if !bytes.len().is_multiple_of(4) {
        return Err("base64 image payload length must be a multiple of 4".into());
    }
    let max_b64 = max_decoded_bytes.div_ceil(3).saturating_mul(4);
    if bytes.len() > max_b64 {
        return Err(format!("base64 image payload exceeds {max_decoded_bytes} decoded bytes limit"));
    }
    let pad = if bytes.ends_with(b"==") {
        2
    } else if bytes.ends_with(b"=") {
        1
    } else {
        0
    };
    let expected_len = (bytes.len() / 4) * 3 - pad;
    if expected_len == 0 {
        return Err("base64 image payload decodes to zero bytes".into());
    }
    if expected_len > max_decoded_bytes {
        return Err(format!("decoded image payload ({expected_len} bytes) exceeds {max_decoded_bytes} bytes"));
    }

    let mut out = Vec::with_capacity(expected_len);
    let last_chunk_idx = bytes.len() / 4 - 1;
    let (chunks, _) = bytes.as_chunks::<4>();
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = idx == last_chunk_idx;
        let a = decode_b64_char(chunk[0]).ok_or_else(|| "invalid base64 character".to_string())?;
        let b = decode_b64_char(chunk[1]).ok_or_else(|| "invalid base64 character".to_string())?;
        match (chunk[2], chunk[3]) {
            (b'=', b'=') if is_last => {
                if (b & 0x0F) != 0 {
                    return Err("non-canonical base64 trailing bits".into());
                }
                out.push((a << 2) | (b >> 4));
            }
            (c2, b'=') if is_last => {
                let c = decode_b64_char(c2).ok_or_else(|| "invalid base64 character".to_string())?;
                if (c & 0x03) != 0 {
                    return Err("non-canonical base64 trailing bits".into());
                }
                out.push((a << 2) | (b >> 4));
                out.push((b << 4) | (c >> 2));
            }
            (c2, c3) => {
                let c = decode_b64_char(c2).ok_or_else(|| "invalid base64 character".to_string())?;
                let d = decode_b64_char(c3).ok_or_else(|| "invalid base64 character".to_string())?;
                out.push((a << 2) | (b >> 4));
                out.push((b << 4) | (c >> 2));
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

/// Detect the image MIME type strictly from container magic bytes.
pub fn detect_image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        Some("image/png")
    } else if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Validate declared MIME type, byte bounds, and magic-byte header match.
pub fn validate_image_bytes(media_type: &str, raw_bytes: &[u8]) -> Result<(), TransportError> {
    if !ALLOWED_IMAGE_MEDIA_TYPES.contains(&media_type) {
        return Err(config_err(format!(
            "unsupported image media_type `{media_type}` (allowed: image/png, image/jpeg, image/webp)"
        )));
    }
    if raw_bytes.is_empty() {
        return Err(config_err("image bytes must not be empty"));
    }
    if raw_bytes.len() > MAX_IMAGE_BYTES {
        return Err(config_err(format!(
            "image size {} bytes exceeds maximum {} bytes",
            raw_bytes.len(),
            MAX_IMAGE_BYTES
        )));
    }
    match detect_image_media_type(raw_bytes) {
        Some(detected) if detected == media_type => Ok(()),
        Some(detected) => Err(config_err(format!(
            "image magic bytes indicate `{detected}` but media_type declared `{media_type}`"
        ))),
        None => Err(config_err(format!(
            "image bytes do not match a valid `{media_type}` container header"
        ))),
    }
}

/// Construct a validated [`ContentBlock::Image`] from raw PNG/JPEG/WebP bytes.
pub fn build_image_block(media_type: &str, raw_bytes: &[u8]) -> Result<ContentBlock, TransportError> {
    validate_image_bytes(media_type, raw_bytes)?;
    Ok(ContentBlock::Image {
        media_type: media_type.to_string(),
        data_base64: encode_base64(raw_bytes),
    })
}

/// Validate a [`ContentBlock::Image`]'s MIME type, base64 encoding, size cap, and magic bytes.
/// Returns the decoded byte length on success.
pub fn validate_image_block(media_type: &str, data_base64: &str) -> Result<usize, TransportError> {
    if !ALLOWED_IMAGE_MEDIA_TYPES.contains(&media_type) {
        return Err(config_err(format!(
            "unsupported image media_type `{media_type}` (allowed: image/png, image/jpeg, image/webp)"
        )));
    }
    if data_base64.len() > MAX_IMAGE_BASE64_LEN {
        return Err(config_err(format!(
            "base64 image payload exceeds maximum {} bytes",
            MAX_IMAGE_BYTES
        )));
    }
    let decoded = decode_base64_strict(data_base64, MAX_IMAGE_BYTES)
        .map_err(|e| config_err(format!("invalid image base64: {e}")))?;
    validate_image_bytes(media_type, &decoded)?;
    Ok(decoded.len())
}

/// Return `true` if any message in `req` contains a [`ContentBlock::Image`].
pub fn request_has_images(req: &ModelRequest) -> bool {
    req.messages
        .iter()
        .any(|m| m.content.iter().any(|b| matches!(b, ContentBlock::Image { .. })))
}

/// Explicit, fail-closed vision capability check per `(ProviderKind, model)`.
///
/// Unknown and text-only models return `false` so callers never silently drop images.
/// Model-family allowlists reflect published multimodal families (see F14c report assumptions).
pub fn supports_vision(provider: ProviderKind, model: &str) -> bool {
    let raw = model.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return false;
    }
    match provider {
        ProviderKind::Anthropic => {
            raw.starts_with("claude-3-")
                || raw.starts_with("claude-3.")
                || raw.starts_with("claude-4-")
                || raw.starts_with("claude-4.")
                || raw.starts_with("claude-sonnet-")
                || raw.starts_with("claude-opus-")
                || raw.starts_with("claude-haiku-4")
                || raw.starts_with("claude-fable")
        }
        ProviderKind::OpenAi | ProviderKind::ChatGpt => {
            if raw == "o1-mini"
                || raw.starts_with("o1-mini-")
                || raw == "o1-preview"
                || raw.starts_with("o1-preview-")
                || raw == "o3-mini"
                || raw.starts_with("o3-mini-")
            {
                return false;
            }
            raw == "gpt-4o"
                || raw.starts_with("gpt-4o-")
                || raw == "chatgpt-4o"
                || raw.starts_with("chatgpt-4o-")
                || raw == "gpt-4-turbo"
                || raw.starts_with("gpt-4-turbo-")
                || raw.starts_with("gpt-4-vision")
                || raw == "gpt-4.1"
                || raw.starts_with("gpt-4.1-")
                || raw.starts_with("gpt-4.5")
                || raw == "gpt-5"
                || raw.starts_with("gpt-5-")
                || raw.starts_with("gpt-5.")
                || raw == "gpt-6"
                || raw.starts_with("gpt-6-")
                || raw.starts_with("gpt-6.")
                || raw == "o1"
                || raw.starts_with("o1-")
                || raw == "o3"
                || raw.starts_with("o3-")
                || raw == "o4-mini"
                || raw.starts_with("o4-mini-")
        }
        ProviderKind::Gemini => {
            let m = raw.strip_prefix("models/").unwrap_or(&raw);
            m.starts_with("gemini-1.5-")
                || m.starts_with("gemini-2.")
                || m.starts_with("gemini-3.")
                || m.starts_with("gemini-4.")
                || m.starts_with("gemini-pro-vision")
                || m.starts_with("gemini-exp-")
        }
        ProviderKind::Ollama => {
            let base = raw.split(':').next().unwrap_or(&raw);
            base == "llava"
                || base.starts_with("llava-")
                || base == "bakllava"
                || base.starts_with("bakllava-")
                || base == "moondream"
                || base.starts_with("moondream-")
                || base == "minicpm-v"
                || base.starts_with("minicpm-v-")
                || base == "llama3.2-vision"
                || base.starts_with("llama3.2-vision-")
                || base.starts_with("llama-3.2-vision")
                || base.starts_with("qwen2-vl")
                || base.starts_with("qwen2.5-vl")
                || base.starts_with("qwen2.5vl")
                || base.starts_with("qwen3-vl")
                || base == "gemma3"
                || base.starts_with("gemma3-")
                || base.starts_with("pixtral")
                || base.starts_with("granite3.2-vision")
                || base.starts_with("mistral-small3.1")
                || base.starts_with("kimi-vl")
        }
    }
}

/// Validate all image blocks in `req` and enforce the fail-closed vision capability gate.
///
/// Text-only requests (zero [`ContentBlock::Image`] blocks) return `Ok(())` immediately.
pub fn validate_request_images(provider: ProviderKind, req: &ModelRequest) -> Result<(), TransportError> {
    if !request_has_images(req) {
        return Ok(());
    }
    if !supports_vision(provider, &req.model) {
        return Err(config_err(format!(
            "model `{}` on provider `{}` is not vision-capable; refusing to send or drop image blocks",
            req.model,
            provider.cli_name()
        )));
    }
    let mut count = 0usize;
    let mut total_bytes = 0usize;
    for msg in &req.messages {
        for block in &msg.content {
            if let ContentBlock::Image { media_type, data_base64 } = block {
                if msg.role != Role::User {
                    return Err(config_err("image content blocks are only allowed in user messages"));
                }
                count += 1;
                if count > MAX_IMAGES_PER_REQUEST {
                    return Err(config_err(format!(
                        "request contains more than {MAX_IMAGES_PER_REQUEST} image blocks"
                    )));
                }
                let decoded_len = validate_image_block(media_type, data_base64)?;
                total_bytes = total_bytes.saturating_add(decoded_len);
                if total_bytes > MAX_REQUEST_IMAGE_BYTES {
                    return Err(config_err(format!(
                        "total request image bytes ({total_bytes}) exceeds maximum {MAX_REQUEST_IMAGE_BYTES}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Anthropic Messages API image block (`type: "image"`, base64 source).
pub(crate) fn encode_anthropic_image(media_type: &str, data_base64: &str) -> Value {
    json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": media_type,
            "data": data_base64,
        }
    })
}

/// OpenAI / Ollama Chat Completions image part (`type: "image_url"`, data URI).
pub(crate) fn encode_openai_image(media_type: &str, data_base64: &str) -> Value {
    json!({
        "type": "image_url",
        "image_url": {
            "url": format!("data:{media_type};base64,{data_base64}"),
        }
    })
}

/// Google Gemini `generateContent` image part (`inlineData`).
pub(crate) fn encode_gemini_image(media_type: &str, data_base64: &str) -> Value {
    json!({
        "inlineData": {
            "mimeType": media_type,
            "data": data_base64,
        }
    })
}

/// OpenAI / ChatGPT Responses API image input item (`type: "input_image"`, data URI).
pub(crate) fn encode_responses_image(media_type: &str, data_base64: &str) -> Value {
    json!({
        "type": "input_image",
        "image_url": format!("data:{media_type};base64,{data_base64}"),
    })
}

#[cfg(test)]
#[path = "vision_tests.rs"]
mod tests;
