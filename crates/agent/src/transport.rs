//! Provider-agnostic single-turn model transport trait and turn types.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1-mini";
pub const DEFAULT_OLLAMA_BASE: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5-coder:7b";
pub const DEFAULT_GEMINI_BASE: &str = "https://generativelanguage.googleapis.com";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";
/// Key variables every provider's transport reads first (not tied to one provider).
pub const GENERIC_KEY_ENV_VARS: [&str; 2] = ["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY"];
/// Variables that name the provider explicitly (as the console's `--provider` default).
pub const PROVIDER_ENV_VARS: [&str; 2] = ["RE_SEARCH_PROVIDER", "RESEARCH_CORE_PROVIDER"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Ollama,
    Gemini,
}

impl ProviderKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "anthropic-messages" => Ok(ProviderKind::Anthropic),
            "openai" | "openai-chat" => Ok(ProviderKind::OpenAi),
            "ollama" | "ollama-chat" => Ok(ProviderKind::Ollama),
            "gemini" | "gemini-generate-content" => Ok(ProviderKind::Gemini),
            other => Err(format!(
                "unknown provider `{other}` (expected `anthropic`, `openai`, `ollama`, or `gemini`)"
            )),
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Gemini => "gemini",
        }
    }

    pub fn wire_id(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic-messages",
            ProviderKind::OpenAi => "openai-chat",
            ProviderKind::Ollama => "ollama-chat",
            ProviderKind::Gemini => "gemini-generate-content",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => crate::anthropic::DEFAULT_API_BASE,
            ProviderKind::OpenAi => DEFAULT_OPENAI_BASE,
            ProviderKind::Ollama => DEFAULT_OLLAMA_BASE,
            ProviderKind::Gemini => DEFAULT_GEMINI_BASE,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => crate::anthropic::DEFAULT_API_MODEL,
            ProviderKind::OpenAi => DEFAULT_OPENAI_MODEL,
            ProviderKind::Ollama => DEFAULT_OLLAMA_MODEL,
            ProviderKind::Gemini => DEFAULT_GEMINI_MODEL,
        }
    }

    /// Environment variables the transport reads its API key from, in order (the first
    /// non-empty one wins); the first two are [`GENERIC_KEY_ENV_VARS`], read by every
    /// provider. The desktop server uses this list to report which key a turn would use;
    /// its model-list calls use [`ProviderKind::own_key_env_vars`] instead.
    pub fn key_env_vars(self) -> &'static [&'static str] {
        match self {
            ProviderKind::Anthropic => &["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY"],
            ProviderKind::OpenAi => &["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "OPENAI_API_KEY"],
            ProviderKind::Ollama => &["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "OLLAMA_API_KEY", "OPENAI_API_KEY"],
            ProviderKind::Gemini => &["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"],
        }
    }

    /// Only this provider's own key variables: no generic variable
    /// ([`GENERIC_KEY_ENV_VARS`]) and no other provider's (Ollama does not fall back to
    /// `OPENAI_API_KEY` here). Model-list calls, which the desktop makes on its own rather
    /// than because the person chose this provider, read only these.
    pub fn own_key_env_vars(self) -> &'static [&'static str] {
        match self {
            ProviderKind::Anthropic => &["ANTHROPIC_API_KEY"],
            ProviderKind::OpenAi => &["OPENAI_API_KEY"],
            ProviderKind::Ollama => &["OLLAMA_API_KEY"],
            ProviderKind::Gemini => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        }
    }

    pub fn resolve_model(self, requested: &str) -> String {
        let trimmed = requested.trim();
        if trimmed.is_empty() {
            return self.default_model().to_string();
        }
        if self == ProviderKind::Anthropic && trimmed == "haiku" {
            return crate::anthropic::DEFAULT_API_MODEL.to_string();
        }
        trimmed.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportInfo {
    pub provider: &'static str,
    /// Sanitized `scheme://host[:port]/path` (no userinfo, query, fragment, or secret).
    pub endpoint: String,
    /// Name of the environment variable from which the API key was read, or `"none"` on keyless loopback.
    pub api_key_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    /// Exact UTF-8 JSON body sent over HTTP (contains zero auth headers or secrets).
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        is_error: bool,
        content: String,
    },
    Other {
        block_type: String,
        raw: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    Refusal,
    PauseTurn,
    Other(String),
}

impl StopReason {
    pub fn from_wire(s: &str) -> Self {
        match s {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "stop_sequence" => StopReason::StopSequence,
            "refusal" => StopReason::Refusal,
            "pause_turn" => StopReason::PauseTurn,
            other => StopReason::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            StopReason::EndTurn => "end_turn",
            StopReason::ToolUse => "tool_use",
            StopReason::MaxTokens => "max_tokens",
            StopReason::StopSequence => "stop_sequence",
            StopReason::Refusal => "refusal",
            StopReason::PauseTurn => "pause_turn",
            StopReason::Other(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelResponse {
    pub raw_body: Vec<u8>,
    pub id: Option<String>,
    pub model: String,
    pub stop_reason: StopReason,
    pub content: Vec<ContentBlock>,
    pub usage: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    Config,
    Auth,
    RateLimit,
    HttpStatus,
    Network,
    MalformedResponse,
    OversizedResponse,
}

impl TransportErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TransportErrorKind::Config => "config",
            TransportErrorKind::Auth => "auth",
            TransportErrorKind::RateLimit => "rate_limit",
            TransportErrorKind::HttpStatus => "http_status",
            TransportErrorKind::Network => "network",
            TransportErrorKind::MalformedResponse => "malformed_response",
            TransportErrorKind::OversizedResponse => "oversized_response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub status: Option<u16>,
    pub message: String,
    pub raw_body: Option<Vec<u8>>,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(code) => write!(f, "{} (HTTP {code}): {}", self.kind.as_str(), self.message),
            None => write!(f, "{}: {}", self.kind.as_str(), self.message),
        }
    }
}

impl std::error::Error for TransportError {}

pub trait Transport: Send + Sync {
    fn info(&self) -> TransportInfo;
    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError>;
    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError>;
}
