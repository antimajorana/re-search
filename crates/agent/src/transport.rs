//! Provider-agnostic single-turn model transport trait and turn types.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com";
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1-mini";
pub const DEFAULT_OLLAMA_BASE: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5-coder:7b";
pub const DEFAULT_GEMINI_BASE: &str = "https://generativelanguage.googleapis.com";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash";
/// The only production host the ChatGPT-plan route talks to (Responses API and model list).
pub const DEFAULT_CHATGPT_BASE: &str = "https://api.openai.com";
/// Fallback model id for the ChatGPT-plan route when the live list (`GET /v1/models`,
/// `visibility == "list"`) is unavailable. Taken from the SDK example in OpenAI's "Models
/// and inference" page for Sign in with ChatGPT; the live list is authoritative.
pub const DEFAULT_CHATGPT_MODEL: &str = "gpt-6.2-sol";
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
    /// The user's ChatGPT plan through OpenAI's "Sign in with ChatGPT" flow for open-source
    /// apps: OAuth access token, Responses API (`POST /v1/responses`, `store: false`,
    /// `stream: true`). Never reads an API-key variable.
    #[serde(rename = "chatgpt")]
    ChatGpt,
}

impl ProviderKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "anthropic-messages" => Ok(ProviderKind::Anthropic),
            "openai" | "openai-chat" => Ok(ProviderKind::OpenAi),
            "ollama" | "ollama-chat" => Ok(ProviderKind::Ollama),
            "gemini" | "gemini-generate-content" => Ok(ProviderKind::Gemini),
            "chatgpt" | "chatgpt-responses" => Ok(ProviderKind::ChatGpt),
            other => Err(format!(
                "unknown provider `{other}` (expected `anthropic`, `openai`, `ollama`, `gemini`, or `chatgpt`)"
            )),
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Gemini => "gemini",
            ProviderKind::ChatGpt => "chatgpt",
        }
    }

    pub fn wire_id(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic-messages",
            ProviderKind::OpenAi => "openai-chat",
            ProviderKind::Ollama => "ollama-chat",
            ProviderKind::Gemini => "gemini-generate-content",
            ProviderKind::ChatGpt => "chatgpt-responses",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => crate::anthropic::DEFAULT_API_BASE,
            ProviderKind::OpenAi => DEFAULT_OPENAI_BASE,
            ProviderKind::Ollama => DEFAULT_OLLAMA_BASE,
            ProviderKind::Gemini => DEFAULT_GEMINI_BASE,
            ProviderKind::ChatGpt => DEFAULT_CHATGPT_BASE,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => crate::anthropic::DEFAULT_API_MODEL,
            ProviderKind::OpenAi => DEFAULT_OPENAI_MODEL,
            ProviderKind::Ollama => DEFAULT_OLLAMA_MODEL,
            ProviderKind::Gemini => DEFAULT_GEMINI_MODEL,
            ProviderKind::ChatGpt => DEFAULT_CHATGPT_MODEL,
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
            // Signed in with ChatGPT only: no key variable (not even the generic one) is read.
            ProviderKind::ChatGpt => &[],
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
            ProviderKind::ChatGpt => &[],
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

/// Canonicalize an optional reasoning effort level (`low` | `medium` | `high` | `xhigh` | `max`).
/// Empty, `"default"`, or unknown values return `None` so wire transports omit the parameter.
pub fn normalize_reasoning_effort(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "low" | "medium" | "high" | "xhigh" | "max" => Some(trimmed),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
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
    Cancelled,
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
            TransportErrorKind::Cancelled => "cancelled",
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

impl TransportError {
    pub fn cancelled() -> Self {
        Self {
            kind: TransportErrorKind::Cancelled,
            status: None,
            message: "stopped by user".into(),
            raw_body: None,
        }
    }
}

/// Thread-safe cooperative cancellation token shared with active model/verifier calls.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Receives live token / reasoning deltas and reports cancellation during an in-flight model call.
pub trait StreamObserver {
    fn on_text_delta(&mut self, _delta: &str) {}
    fn on_thinking_delta(&mut self, _delta: &str) {}
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub struct NoopStreamObserver;
impl StreamObserver for NoopStreamObserver {}

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

    fn send_prepared_streaming(
        &self,
        prepared: &PreparedRequest,
        observer: &mut dyn StreamObserver,
    ) -> Result<ModelResponse, TransportError> {
        if observer.is_cancelled() {
            return Err(TransportError::cancelled());
        }
        let resp = self.send_prepared(prepared)?;
        if observer.is_cancelled() {
            return Err(TransportError::cancelled());
        }
        Ok(resp)
    }
}
