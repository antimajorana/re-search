//! One session on the Stage 4 in-core API route (`--route api`).
//!
//! `AgentLoop` owns the model conversation loop directly over `agent::Transport`
//! (`AnthropicTransport`, `POST /v1/messages`) and dispatches every tool call
//! through `tools::Router::try_call` and `ConsoleApprover`.

use crate::runtime::{build_in_process_router, WorkerSetup};
use crate::session::{Recorder, SessionGuard, EXIT_ABORTED, EXIT_REFUSAL};
use agent::{
    resolve_api_model, run_agent_loop, AgentConfig, AnthropicConfig, AnthropicTransport, EventSink, SessionOutcome,
    Transport, DEFAULT_SYSTEM_PROMPT,
};
use ledger::{AppendedEvent, Ledger};
use serde_json::Value;
use std::path::PathBuf;
use tools::canon::random_hex;

pub struct ApiRunOpts {
    pub prompt: String,
    pub model_requested: String,
    pub data_dir: PathBuf,
    pub python: Option<PathBuf>,
    pub worker_script: Option<PathBuf>,
    pub api_base: String,
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

pub fn run(opts: &ApiRunOpts) -> Result<i32, String> {
    // Validate and prepare transport before creating session directories or emitting session.start.
    let transport = AnthropicTransport::from_env(AnthropicConfig {
        base_url: opts.api_base.clone(),
        ..AnthropicConfig::default()
    })
    .map_err(|e| e.to_string())?;
    let info = transport.info();
    let model = resolve_api_model(&opts.model_requested);

    let session = format!("s-{}", random_hex(8));
    let io = |e: std::io::Error| e.to_string();
    let dir = opts.data_dir.join("sessions").join(&session);
    let (work, scratch, pycache) = (dir.join("work"), dir.join("scratch"), dir.join("pycache"));
    std::fs::create_dir_all(&work).map_err(io)?;
    std::fs::create_dir_all(&scratch).map_err(io)?;
    std::fs::create_dir_all(&pycache).map_err(io)?;
    let active_lock = dir.join(".active.lock");
    std::fs::write(&active_lock, std::process::id().to_string()).map_err(io)?;
    #[cfg(windows)]
    worker::appcontainer::sweep_stale_profiles(&opts.data_dir);

    // Declare `_session_guard` BEFORE `built` so `built.router` (and any Windows
    // `Worker` / `AppContainerSandbox`) drops *before* `SessionGuard::drop` cleans
    // up `.active.lock` and the per-session AppContainer profile.
    let _session_guard = SessionGuard {
        #[cfg(windows)]
        session: session.clone(),
        lock_path: active_lock,
    };

    let worker_setup = match (&opts.python, &opts.worker_script) {
        (Some(python), Some(script)) => Some(WorkerSetup {
            python: python.clone(),
            worker_script: script.clone(),
            scratch,
            pycache,
        }),
        _ => None,
    };
    let built = build_in_process_router(&opts.data_dir, &session, worker_setup, opts.eval_timeout_s)?;
    let rec = Recorder { ledger: Ledger::open(&opts.data_dir).map_err(|e| e.to_string())?, session: session.clone() };
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
    let outcome = run_agent_loop(&transport, &built.router, &rec.ledger, &cfg, &mut sink)?;
    eprintln!(
        "[supervisor] session {session} ended; see it with: re-search show {session} --data-dir \"{}\"",
        opts.data_dir.display()
    );
    match outcome {
        SessionOutcome::Completed { .. } => Ok(0),
        SessionOutcome::Refusal { .. } => Ok(EXIT_REFUSAL),
        SessionOutcome::Aborted { .. } => Ok(EXIT_ABORTED),
    }
}
