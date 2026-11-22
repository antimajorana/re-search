//! Stage 4–5 model transport abstraction, multi-provider wire codecs,
//! verified session resume, and in-core agent loop.
//!
//! `agent` owns the model conversation loop when `re:Search` talks directly to
//! a provider HTTP API (`provenance: "in-core loop, full provenance"`). It does
//! not depend on `worker`: every tool call is dispatched exclusively through
//! [`tools::Router::try_call`], failing closed if the ledger cannot record.

pub mod anthropic;
pub mod autonomous;
pub(crate) mod autonomous_prompts;
pub mod gemini;
pub mod guardrails;
pub(crate) mod http;
pub mod model_list;
pub mod narrator;
pub mod openai;
pub mod presets;
pub mod provider_config;
pub mod responses;
pub mod resume;
pub mod runner;
pub mod sidetalk;
pub mod strategist;
pub(crate) mod stream_codecs;
pub(crate) mod stream_http;
pub mod transport;
pub mod verifiers;
pub(crate) mod wire;

pub use anthropic::{
    resolve_api_model, AnthropicConfig, AnthropicTransport, SecretKey, ANTHROPIC_VERSION, DEFAULT_API_BASE,
    DEFAULT_API_MODEL,
};
pub use autonomous::{
    resume_autonomous_research_loop, run_autonomous_research_loop, AutonomousConfig, AutonomousRoles,
    DEFAULT_MAX_VERIFICATION_ROUNDS, RESEARCH_SYSTEM_PROMPT,
};
pub use gemini::{GeminiConfig, GeminiTransport};
pub use guardrails::{
    build_research_system_prompt, check_computation_interpretation, check_final_answer_gate,
    mask_old_tool_outputs, mask_old_tool_outputs_stepped, run_research_loop,
    validate_pinned_conventions, GuardrailConfig,
};
pub use narrator::{build_derivation_sheet, narrate_session, NarrationReport};
pub use openai::{OpenAiConfig, OpenAiTransport};
pub use presets::{ModelEndpointSpec, PresetKind, RoleAssignment};
pub use provider_config::{
    validate_roles, ProviderEntry, ProviderKeyStore, RedactedProviderConfigView, RedactedProviderEntry, PROVIDERS_FILE,
};
pub use responses::{BearerSource, ChatGptConfig, ChatGptTransport, TOOL_NAMESPACE};
pub use resume::{
    inspect_session_for_resume, resume_agent_loop, ResumeConfig, ResumeInspection, ResumeReject, ResumeRejectKind,
};
pub use runner::{
    run_agent_loop, AgentConfig, EventSink, NoopSink, SessionOutcome, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
    DEFAULT_SYSTEM_PROMPT, EVENT_VERSION,
};
pub use sidetalk::{ask_sidetalk, read_sidetalk_thread, sidetalk_session_key, SideTalkTurn};
pub use strategist::{
    consult_strategist, consult_strategist_cancellable, count_strategist_consultations, mandatory_strategist_required,
    StrategistAdvice, MAX_STRATEGIST_CONSULTATIONS,
};
pub use transport::{
    CancelToken, ContentBlock, Message, ModelRequest, ModelResponse, NoopStreamObserver, PreparedRequest,
    ProviderKind, Role, StopReason, StreamObserver, ToolSpec, Transport, TransportError, TransportErrorKind,
    TransportInfo, DEFAULT_CHATGPT_BASE, DEFAULT_CHATGPT_MODEL, DEFAULT_GEMINI_BASE, DEFAULT_GEMINI_MODEL,
    DEFAULT_OLLAMA_BASE, DEFAULT_OLLAMA_MODEL, DEFAULT_OPENAI_BASE, DEFAULT_OPENAI_MODEL,
};
pub use verifiers::{
    verify_full_solution, verify_full_solution_cancellable, verify_step, VerificationIssue, VerificationReport,
    VerificationVerdict,
};
