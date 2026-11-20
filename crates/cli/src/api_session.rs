//! One session (or resumed leg) on the Stage 4–5 in-core API route (`--route api`).
//!
//! `AgentLoop` owns the model conversation loop directly over `agent::Transport`
//! (`AnthropicTransport`, `OpenAiTransport` for OpenAI/Ollama, or `GeminiTransport`)
//! and dispatches every tool call through `tools::Router::try_call` and
//! `ConsoleApprover`.

use crate::runtime::{build_in_process_router, WorkerSetup};
use crate::session::{Recorder, EXIT_ABORTED, EXIT_REFUSAL};
use crate::session_lock::acquire_session_lock;
use agent::{
    inspect_session_for_resume, resume_agent_loop, run_agent_loop, AgentConfig, AnthropicConfig, AnthropicTransport,
    EventSink, GeminiConfig, GeminiTransport, OpenAiConfig, OpenAiTransport, ProviderKind, ResumeConfig,
    ResumeRejectKind, SessionOutcome, Transport, DEFAULT_SYSTEM_PROMPT,
};
use ledger::{AppendedEvent, Ledger};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tools::builtin;
use tools::canon::random_hex;

pub struct ApiRunOpts {
    pub prompt: String,
    pub provider: ProviderKind,
    pub model_requested: String,
    pub data_dir: PathBuf,
    pub python: Option<PathBuf>,
    pub worker_script: Option<PathBuf>,
    pub api_base: String,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub eval_timeout_s: u64,
}

pub struct ApiResumeOpts {
    pub session: String,
    pub prompt: String,
    pub provider_override: Option<ProviderKind>,
    pub model_override: Option<String>,
    pub data_dir: PathBuf,
    pub python: Option<PathBuf>,
    pub worker_script: Option<PathBuf>,
    pub api_base_override: Option<String>,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub eval_timeout_s: u64,
}

struct CliSink<'a> {
    rec: &'a Recorder,
}

impl EventSink for CliSink<'_> {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String> {
        trace_event(kind, data);
        self.rec.write_ndjson_line(&mut std::io::stdout().lock(), receipt, kind, data)
    }
}

fn trace_event(kind: &str, data: &Value) {
    match kind {
        "session.start" => eprintln!(
            "[supervisor] api session started: model {}, endpoint {}, containment: {}",
            data["model"], data["endpoint"], data["containment"]
        ),
        "session.resume" => eprintln!(
            "[supervisor] api session resumed (leg {}, turn {}): model {}, endpoint {}, containment: {}",
            data["leg"], data["start_turn"], data["model"], data["endpoint"], data["containment"]
        ),
        "model.request" => eprintln!(
            "[api] -> turn {} (messages: {}, max_tokens: {})",
            data["turn"], data["message_count"], data["max_tokens"]
        ),
        "model.response" => eprintln!(
            "[api] <- turn {} (stop_reason: {}, {} ms)",
            data["turn"], data["stop_reason"], data["duration_ms"]
        ),
        "model.text" => eprintln!("[model] {}", data["text"].as_str().unwrap_or("")),
        "model.tool_use" => eprintln!("[model calls] {} {}", data["name"], data["input"]),
        "model.tool_result" => eprintln!("[tool result] {}", data["text"].as_str().unwrap_or("")),
        "model.refusal" => eprintln!("[model] REFUSAL: {}", data["text"].as_str().unwrap_or("")),
        "model.error" => eprintln!(
            "[api] error on turn {}: {}",
            data["turn"],
            data["message"].as_str().unwrap_or("unknown error")
        ),
        "session.abort" => eprintln!("[supervisor] ABORTED: {}", data["reason"].as_str().unwrap_or("")),
        "session.end" => eprintln!(
            "[supervisor] finished: {}, {} turns, {} ms",
            data["outcome"].as_str().unwrap_or(""),
            data["num_turns"],
            data["duration_ms"]
        ),
        _ => {}
    }
}

pub(crate) fn build_transport(
    provider: ProviderKind,
    base_url: &str,
    model: &str,
) -> Result<Box<dyn Transport>, String> {
    match provider {
        ProviderKind::Anthropic => AnthropicTransport::from_env(AnthropicConfig {
            base_url: base_url.to_string(),
            ..AnthropicConfig::default()
        })
        .map(|t| Box::new(t) as Box<dyn Transport>)
        .map_err(|e| e.to_string()),
        ProviderKind::OpenAi => OpenAiTransport::from_env(OpenAiConfig {
            provider: ProviderKind::OpenAi,
            base_url: base_url.to_string(),
            ..OpenAiConfig::default()
        })
        .map(|t| Box::new(t) as Box<dyn Transport>)
        .map_err(|e| e.to_string()),
        ProviderKind::Ollama => OpenAiTransport::from_env(OpenAiConfig {
            provider: ProviderKind::Ollama,
            base_url: base_url.to_string(),
            ..OpenAiConfig::default()
        })
        .map(|t| Box::new(t) as Box<dyn Transport>)
        .map_err(|e| e.to_string()),
        ProviderKind::Gemini => GeminiTransport::from_env(GeminiConfig {
            base_url: base_url.to_string(),
            model: model.to_string(),
            ..GeminiConfig::default()
        })
        .map(|t| Box::new(t) as Box<dyn Transport>)
        .map_err(|e| e.to_string()),
    }
}

fn ensure_session_dirs(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let io = |e: std::io::Error| e.to_string();
    let (work, scratch, pycache) = (dir.join("work"), dir.join("scratch"), dir.join("pycache"));
    std::fs::create_dir_all(&work).map_err(io)?;
    std::fs::create_dir_all(&scratch).map_err(io)?;
    std::fs::create_dir_all(&pycache).map_err(io)?;
    Ok((scratch, pycache))
}

fn worker_setup(
    python: &Option<PathBuf>,
    worker_script: &Option<PathBuf>,
    scratch: PathBuf,
    pycache: PathBuf,
) -> Option<WorkerSetup> {
    match (python, worker_script) {
        (Some(py), Some(script)) => Some(WorkerSetup {
            python: py.clone(),
            worker_script: script.clone(),
            scratch,
            pycache,
        }),
        _ => None,
    }
}

fn map_outcome(outcome: SessionOutcome) -> i32 {
    match outcome {
        SessionOutcome::Completed { .. } => 0,
        SessionOutcome::Refusal { .. } => EXIT_REFUSAL,
        SessionOutcome::Aborted { .. } => EXIT_ABORTED,
    }
}

pub fn run(opts: &ApiRunOpts) -> Result<i32, String> {
    // Validate and prepare transport before creating session directories or emitting session.start.
    let model = opts.provider.resolve_model(&opts.model_requested);
    let transport = build_transport(opts.provider, &opts.api_base, &model)?;
    let info = transport.info();

    let session = format!("s-{}", random_hex(8));
    let dir = opts.data_dir.join("sessions").join(&session);
    let (scratch, pycache) = ensure_session_dirs(&dir)?;

    // Acquire `_session_guard` BEFORE `sweep_stale_profiles` and BEFORE `built`
    // so `built.router` (and any Windows `Worker` / `AppContainerSandbox`) drops
    // *before* `SessionGuard::drop` cleans `.active.lock` and the AppContainer profile.
    let _session_guard = acquire_session_lock(&dir, &session)?;
    #[cfg(windows)]
    worker::appcontainer::sweep_stale_profiles(&opts.data_dir);

    let setup = worker_setup(&opts.python, &opts.worker_script, scratch, pycache);
    let built = build_in_process_router(&opts.data_dir, &session, setup, opts.eval_timeout_s)?;
    let rec = Recorder {
        ledger: Ledger::open(&opts.data_dir).map_err(|e| e.to_string())?,
        session: session.clone(),
    };
    let mut sink = CliSink { rec: &rec };

    eprintln!(
        "[supervisor] session {session} (api route: {} via {}, key env {}); data in {}",
        info.provider,
        info.endpoint,
        info.api_key_env,
        opts.data_dir.display()
    );

    let cfg = AgentConfig {
        prompt: opts.prompt.clone(),
        model_requested: opts.model_requested.clone(),
        model,
        system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        max_turns: opts.max_turns,
        max_tokens: opts.max_tokens,
        containment: built.containment,
        contained: built.contained,
    };
    let outcome = run_agent_loop(&*transport, &built.router, &rec.ledger, &cfg, &mut sink)?;
    eprintln!(
        "[supervisor] session {session} ended; see it with: re-search show {session} --data-dir \"{}\"",
        opts.data_dir.display()
    );
    Ok(map_outcome(outcome))
}

pub fn resume(opts: &ApiResumeOpts) -> Result<i32, String> {
    let ledger = match Ledger::open_existing(&opts.data_dir) {
        Ok(l) => l,
        Err(e @ ledger::Error::LegacyUnchainedSchema) => {
            eprintln!("re-search resume: {e}");
            return Ok(4);
        }
        Err(e @ (ledger::Error::Tamper(_) | ledger::Error::Sqlite(_) | ledger::Error::BadHash(_))) => {
            eprintln!("re-search resume: {e}");
            return Ok(1);
        }
        Err(e) => return Err(e.to_string()),
    };

    let inspection = match inspect_session_for_resume(
        &ledger,
        &opts.session,
        builtin::POLICY_VERSION,
        &builtin::defs(),
    ) {
        Ok(info) => info,
        Err(r) => match r.kind {
            ResumeRejectKind::LegacySchema => {
                eprintln!("re-search resume: {r}");
                return Ok(4);
            }
            ResumeRejectKind::TamperOrMissing | ResumeRejectKind::CorruptHistory => {
                eprintln!("re-search resume: {r}");
                return Ok(1);
            }
            ResumeRejectKind::UnsupportedRoute | ResumeRejectKind::Drift => return Err(r.to_string()),
        },
    };

    let provider = opts.provider_override.unwrap_or(inspection.last_provider);
    let model_requested = opts.model_override.clone().unwrap_or_else(|| {
        if provider == inspection.last_provider {
            inspection.last_model.clone()
        } else {
            provider.default_model().to_string()
        }
    });
    let model = provider.resolve_model(&model_requested);
    let api_base = opts
        .api_base_override
        .clone()
        .unwrap_or_else(|| provider.default_base_url().to_string());

    // Validate API key and endpoint BEFORE creating directories or `.active.lock`.
    let transport = build_transport(provider, &api_base, &model)?;
    let info = transport.info();

    let dir = opts.data_dir.join("sessions").join(&opts.session);
    let (scratch, pycache) = ensure_session_dirs(&dir)?;

    // Acquire `_session_guard` BEFORE `sweep_stale_profiles` and BEFORE `built`
    // so `built.router` drops before `SessionGuard::drop`.
    let _session_guard = acquire_session_lock(&dir, &opts.session)?;
    #[cfg(windows)]
    worker::appcontainer::sweep_stale_profiles(&opts.data_dir);

    let setup = worker_setup(&opts.python, &opts.worker_script, scratch, pycache);
    let built = build_in_process_router(&opts.data_dir, &opts.session, setup, opts.eval_timeout_s)?;
    let rec = Recorder { ledger, session: opts.session.clone() };
    let mut sink = CliSink { rec: &rec };

    eprintln!(
        "[supervisor] resuming session {} leg {} from seq {} (api route: {} via {}, key env {}); data in {}",
        opts.session,
        inspection.leg,
        inspection.resumed_from_seq,
        info.provider,
        info.endpoint,
        info.api_key_env,
        opts.data_dir.display()
    );

    let cfg = ResumeConfig {
        prompt: opts.prompt.clone(),
        model_requested,
        model,
        system_prompt: None,
        max_turns: opts.max_turns,
        max_tokens: opts.max_tokens,
        containment: built.containment,
        contained: built.contained,
    };
    let outcome = resume_agent_loop(&*transport, &built.router, &rec.ledger, &cfg, &mut sink)?;
    eprintln!(
        "[supervisor] session {} ended; see it with: re-search show {} --data-dir \"{}\"",
        opts.session,
        opts.session,
        opts.data_dir.display()
    );
    Ok(map_outcome(outcome))
}

pub struct ResearchOpts {
    pub workspace_root: Option<PathBuf>,
    pub enable_research_tools: bool,
    pub literature_endpoints: research_tools::LiteratureEndpoints,
    pub verification_mode: bool,
    pub calculation_mode: bool,
    pub pinned_conventions: Option<String>,
}

pub fn run_with_research(opts: &ApiRunOpts, res: &ResearchOpts) -> Result<i32, String> {
    if !res.enable_research_tools
        && res.workspace_root.is_none()
        && !res.verification_mode
        && !res.calculation_mode
        && res.pinned_conventions.is_none()
    {
        return run(opts);
    }
    if let Some(ws) = &res.workspace_root {
        if !ws.exists() || !ws.is_dir() {
            return Err(format!("workspace `{}` does not exist or is not a directory", ws.display()));
        }
    }
    let model = opts.provider.resolve_model(&opts.model_requested);
    let transport = build_transport(opts.provider, &opts.api_base, &model)?;
    let info = transport.info();

    let session = format!("s-{}", random_hex(8));
    let dir = opts.data_dir.join("sessions").join(&session);
    let (scratch, pycache) = ensure_session_dirs(&dir)?;
    let _session_guard = acquire_session_lock(&dir, &session)?;
    #[cfg(windows)]
    worker::appcontainer::sweep_stale_profiles(&opts.data_dir);

    let setup = worker_setup(&opts.python, &opts.worker_script, scratch, pycache);
    let built = crate::runtime::build_research_router(
        &opts.data_dir,
        &session,
        setup,
        opts.eval_timeout_s,
        crate::runtime::ResearchRouterOpts {
            workspace_root: res.workspace_root.clone(),
            literature_endpoints: res.literature_endpoints.clone(),
        },
    )?;
    let rec = Recorder {
        ledger: Ledger::open(&opts.data_dir).map_err(|e| e.to_string())?,
        session: session.clone(),
    };
    let mut sink = CliSink { rec: &rec };

    eprintln!(
        "[supervisor] session {session} (api research route: {} via {}, key env {}); data in {}",
        info.provider,
        info.endpoint,
        info.api_key_env,
        opts.data_dir.display()
    );

    let guardrails = agent::GuardrailConfig {
        verification_mode: res.verification_mode,
        calculation_mode: res.calculation_mode,
        pinned_conventions: res.pinned_conventions.clone(),
        ..agent::GuardrailConfig::default()
    };
    let cfg = AgentConfig {
        prompt: opts.prompt.clone(),
        model_requested: opts.model_requested.clone(),
        model,
        system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        max_turns: opts.max_turns,
        max_tokens: opts.max_tokens,
        containment: built.containment,
        contained: built.contained,
    };
    let outcome = agent::run_research_loop(&*transport, &built.router, &rec.ledger, &cfg, &guardrails, &mut sink)?;
    eprintln!(
        "[supervisor] session {session} ended; see it with: re-search show {session} --data-dir \"{}\"",
        opts.data_dir.display()
    );
    Ok(map_outcome(outcome))
}

pub fn resume_with_research(opts: &ApiResumeOpts, res: &ResearchOpts) -> Result<i32, String> {
    if !res.enable_research_tools && res.workspace_root.is_none() {
        return resume(opts);
    }
    if let Some(ws) = &res.workspace_root {
        if !ws.exists() || !ws.is_dir() {
            return Err(format!("workspace `{}` does not exist or is not a directory", ws.display()));
        }
    }
    let ledger = match Ledger::open_existing(&opts.data_dir) {
        Ok(l) => l,
        Err(e @ ledger::Error::LegacyUnchainedSchema) => {
            eprintln!("re-search resume: {e}");
            return Ok(4);
        }
        Err(e @ (ledger::Error::Tamper(_) | ledger::Error::Sqlite(_) | ledger::Error::BadHash(_))) => {
            eprintln!("re-search resume: {e}");
            return Ok(1);
        }
        Err(e) => return Err(e.to_string()),
    };

    let dir = opts.data_dir.join("sessions").join(&opts.session);
    let (scratch, pycache) = ensure_session_dirs(&dir)?;
    let _session_guard = acquire_session_lock(&dir, &opts.session)?;
    #[cfg(windows)]
    worker::appcontainer::sweep_stale_profiles(&opts.data_dir);

    let setup = worker_setup(&opts.python, &opts.worker_script, scratch, pycache);
    let built = crate::runtime::build_research_router(
        &opts.data_dir,
        &opts.session,
        setup,
        opts.eval_timeout_s,
        crate::runtime::ResearchRouterOpts {
            workspace_root: res.workspace_root.clone(),
            literature_endpoints: res.literature_endpoints.clone(),
        },
    )?;
    let defs: Vec<tools::ToolDef> = built.router.registry().defs().into_iter().cloned().collect();
    let inspection = match inspect_session_for_resume(
        &ledger,
        &opts.session,
        built.router.policy().version.as_str(),
        &defs,
    ) {
        Ok(info) => info,
        Err(r) => match r.kind {
            ResumeRejectKind::LegacySchema => {
                eprintln!("re-search resume: {r}");
                return Ok(4);
            }
            ResumeRejectKind::TamperOrMissing | ResumeRejectKind::CorruptHistory => {
                eprintln!("re-search resume: {r}");
                return Ok(1);
            }
            ResumeRejectKind::UnsupportedRoute | ResumeRejectKind::Drift => return Err(r.to_string()),
        },
    };

    let provider = opts.provider_override.unwrap_or(inspection.last_provider);
    let model_requested = opts.model_override.clone().unwrap_or_else(|| {
        if provider == inspection.last_provider {
            inspection.last_model.clone()
        } else {
            provider.default_model().to_string()
        }
    });
    let model = provider.resolve_model(&model_requested);
    let api_base = opts
        .api_base_override
        .clone()
        .unwrap_or_else(|| provider.default_base_url().to_string());
    let transport = build_transport(provider, &api_base, &model)?;
    let rec = Recorder { ledger, session: opts.session.clone() };
    let mut sink = CliSink { rec: &rec };

    let cfg = ResumeConfig {
        prompt: opts.prompt.clone(),
        model_requested,
        model,
        system_prompt: None,
        max_turns: opts.max_turns,
        max_tokens: opts.max_tokens,
        containment: built.containment,
        contained: built.contained,
    };
    let outcome = resume_agent_loop(&*transport, &built.router, &rec.ledger, &cfg, &mut sink)?;
    eprintln!(
        "[supervisor] session {} ended; see it with: re-search show {} --data-dir \"{}\"",
        opts.session,
        opts.session,
        opts.data_dir.display()
    );
    Ok(map_outcome(outcome))
}
