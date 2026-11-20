//! Stage 4 model transport abstraction and in-core agent loop.
//!
//! `agent` owns the model conversation loop when `re:Search` talks directly to
//! a provider HTTP API (`provenance: "in-core loop, full provenance"`). It does
//! not depend on `worker`: every tool call is dispatched exclusively through
//! [`tools::Router::try_call`], failing closed if the ledger cannot record.

pub mod anthropic;
pub mod runner;
pub mod transport;
mod wire;

pub use anthropic::{
    resolve_api_model, AnthropicConfig, AnthropicTransport, SecretKey, ANTHROPIC_VERSION, DEFAULT_API_BASE,
    DEFAULT_API_MODEL,
};
pub use runner::{
    run_agent_loop, AgentConfig, EventSink, NoopSink, SessionOutcome, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
    DEFAULT_SYSTEM_PROMPT, EVENT_VERSION,
};
pub use transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, PreparedRequest, Role, StopReason, ToolSpec, Transport,
    TransportError, TransportErrorKind, TransportInfo,
};
