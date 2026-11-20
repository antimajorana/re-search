//! Provider-agnostic single-turn model transport trait and turn types.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportInfo {
    pub provider: &'static str,
    /// Sanitized `scheme://host[:port]/path` (no userinfo, query, fragment, or secret).
    pub endpoint: String,
    /// Name of the environment variable from which the API key was read, never its value.
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
