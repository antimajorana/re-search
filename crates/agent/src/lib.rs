//! Stage 4–5 model transport abstraction, multi-provider wire codecs,
//! verified session resume, and in-core agent loop.
//!
//! `agent` owns the model conversation loop when `re:Search` talks directly to
//! a provider HTTP API (`provenance: "in-core loop, full provenance"`). It does
//! not depend on `worker`: every tool call is dispatched exclusively through
//! [`tools::Router::try_call`], failing closed if the ledger cannot record.

pub mod anthropic;
pub mod gemini;
pub(crate) mod http;
pub mod openai;
pub mod resume;
pub mod runner;
pub mod transport;
pub(crate) mod wire;

pub use anthropic::{
    resolve_api_model, AnthropicConfig, AnthropicTransport, SecretKey, ANTHROPIC_VERSION, DEFAULT_API_BASE,
    DEFAULT_API_MODEL,
};
pub use gemini::{GeminiConfig, GeminiTransport};
pub use openai::{OpenAiConfig, OpenAiTransport};
pub use resume::{
    inspect_session_for_resume, resume_agent_loop, ResumeConfig, ResumeInspection, ResumeReject, ResumeRejectKind,
};
pub use runner::{
    run_agent_loop, AgentConfig, EventSink, NoopSink, SessionOutcome, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
    DEFAULT_SYSTEM_PROMPT, EVENT_VERSION,
};
pub use transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, PreparedRequest, ProviderKind, Role, StopReason, ToolSpec,
    Transport, TransportError, TransportErrorKind, TransportInfo, DEFAULT_GEMINI_BASE, DEFAULT_GEMINI_MODEL,
    DEFAULT_OLLAMA_BASE, DEFAULT_OLLAMA_MODEL, DEFAULT_OPENAI_BASE, DEFAULT_OPENAI_MODEL,
};
