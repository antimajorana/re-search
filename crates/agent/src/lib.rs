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
pub mod delegate;
pub(crate) mod dispatch;
pub mod explore;
pub mod gemini;
pub mod guardrails;
pub mod honesty;
pub(crate) mod http;
pub mod interaction;
pub mod model_list;
pub mod narrator;
pub mod openai;
pub mod paper_read;
pub mod presets;
mod provenance;
pub use provenance::workspace_revision;
pub mod provider_config;
pub mod responses;
pub mod resume;
pub mod resume_brief;
pub mod resume_profile;
pub mod runner;
pub mod sidetalk;
pub mod solve_loop;
pub mod solve_prompts;
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
pub use delegate::{
    allowed_tools_for_profile, compute_worker_tools, lean_worker_tools, literature_worker_tools,
    CustomSubagentDelegation, DelegatedWorkerSpec, DelegationConfig, DEFAULT_WORKER_MAX_TURNS,
    DELEGATE_COMPUTE, DELEGATE_LEAN, DELEGATE_LITERATURE, DELEGATE_SUBAGENT, MAX_WORKER_MAX_TURNS,
    WORKER_MAX_TOKENS,
};
pub use explore::{explore_worker_tools, DELEGATE_EXPLORE, EXPLORE_BRANCH_MAX_TURNS, MAX_EXPLORE_BRANCHES};
pub use paper_read::{
    paper_reader_tools, PaperReaderConfig, PaperReaderSlice, DELEGATE_READ_PAPER,
    PAPER_READER_MAX_TURNS, PAPER_READER_RETURN_MAX_BYTES, PAPER_READER_TOOLS,
};
pub use gemini::{GeminiConfig, GeminiTransport};
pub use guardrails::{
    build_research_system_prompt, check_computation_interpretation, check_final_answer_gate,
    mask_old_tool_outputs, mask_old_tool_outputs_stepped, run_research_loop,
    upsert_pinned_conventions_in_prompt, validate_pinned_conventions, GuardrailConfig,
};
pub use honesty::{infer_campaign_phase, lint_honesty, CampaignPhase, HonestyFinding, HonestyReport};
pub use interaction::{is_human_tool, CheckpointPrompt, HumanChannel, HumanReply, SteerMessage, ASK_USER};
pub use narrator::{build_derivation_sheet, narrate_session, NarrationReport};
pub use openai::{OpenAiConfig, OpenAiTransport};
pub use presets::{ModelEndpointSpec, PresetKind, RoleAssignment};
pub use provider_config::{
    validate_roles, ProviderEntry, ProviderKeyStore, RedactedProviderConfigView, RedactedProviderEntry, PROVIDERS_FILE,
};
pub use responses::{BearerSource, ChatGptConfig, ChatGptTransport, TOOL_NAMESPACE};
pub use resume::{
    inspect_session_for_resume, inspect_session_for_resume_with_profile, resume_agent_loop,
    ResumeConfig, ResumeInspection, ResumeReject, ResumeRejectKind,
};
pub use resume_brief::{build_resume_brief, LESSONS_HEADING, NO_SURVIVING_PYTHON_VARS_LINE};
pub use resume_profile::{
    check_tool_defs_drift_for_profile, inspect_session_with_profile, resolve_session_profile,
    ResolvedSessionProfile,
};
pub use runner::{
    run_agent_loop, AgentConfig, EventSink, NoopSink, SessionOutcome, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
    DEFAULT_SYSTEM_PROMPT, EVENT_VERSION,
};
pub use sidetalk::{ask_sidetalk, read_sidetalk_thread, sidetalk_session_key, SideTalkTurn};
pub use solve_loop::{
    resume_solve_loop, run_solve_loop, NoopSolveHostHooks, SolveHostHooks, SolveLoopConfig,
    SolveWorkerRuntime, DEFAULT_SOLVE_BRIEF_BUDGET_CHARS, DEFAULT_SOLVE_CONTEXT_WINDOW_CHARS,
    SOLVE_CALLER, SOLVE_ROUTER_CALLER,
};
pub use solve_prompts::{build_solve_system_prompt, task_kind_instructions, task_kind_label};
pub use strategist::{
    consult_strategist, consult_strategist_cancellable, count_strategist_consultations, mandatory_strategist_required,
    StrategistAdvice, MAX_STRATEGIST_CONSULTATIONS,
};
pub use transport::{
    normalize_reasoning_effort, CancelToken, ContentBlock, Message, ModelRequest, ModelResponse,
    NoopStreamObserver, PreparedRequest, ProviderKind, Role, StopReason, StreamObserver, ToolSpec,
    Transport, TransportError, TransportErrorKind, TransportInfo, DEFAULT_CHATGPT_BASE,
    DEFAULT_CHATGPT_MODEL, DEFAULT_GEMINI_BASE, DEFAULT_GEMINI_MODEL, DEFAULT_OLLAMA_BASE,
    DEFAULT_OLLAMA_MODEL, DEFAULT_OPENAI_BASE, DEFAULT_OPENAI_MODEL,
};
pub use verifiers::{
    verify_full_solution, verify_full_solution_cancellable, verify_step, VerificationIssue, VerificationReport,
    VerificationVerdict,
};
