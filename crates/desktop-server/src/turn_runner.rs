//! Background solver turn runner, research router builder, and SSE EventSink.

use crate::approver::UiApprover;
use crate::sse::SseHub;
use crate::state::DesktopState;
use agent::{
    inspect_session_for_resume, resume_agent_loop, run_research_loop, AgentConfig, EventSink,
    GuardrailConfig, ProviderKind, ResumeConfig, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS,
    DEFAULT_SYSTEM_PROMPT,
};
use ledger::AppendedEvent;
use research_tools::{register_research_tools, research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{builtin, CallCtx, Registry, Router, WorkerEpoch};
#[cfg(windows)]
use worker::appcontainer::AppContainerSandbox;
#[cfg(windows)]
use worker::sandbox::Sandbox;
use worker::{EpochHandle, Worker};
#[cfg(windows)]
use worker::WorkerConfig;

pub(crate) fn spawn_turn_thread(
    state: Arc<DesktopState>,
    session_id: String,
    req_json: Value,
    prompt: String,
) -> Result<(), (u16, String)> {
    {
        let mut active = state
            .active_turns
            .lock()
            .map_err(|_| (500, "active turn lock poisoned".to_string()))?;
        if !active.insert(session_id.clone()) {
            return Err((409, format!("a turn is already running for `{session_id}`")));
        }
    }

    let thread_state = Arc::clone(&state);
    let thread_session = session_id.clone();
    std::thread::Builder::new()
        .name(format!("turn-{session_id}"))
        .spawn(move || {
            let outcome = run_session_turn(&thread_state, &thread_session, &req_json, &prompt);
            if let Ok(mut active) = thread_state.active_turns.lock() {
                active.remove(&thread_session);
            }
            match outcome {
                Ok(text) => {
                    thread_state.sse.emit(
                        &thread_session,
                        "turn_complete",
                        json!({ "session": thread_session, "text": text }),
                    );
                }
                Err(err) => {
                    thread_state.sse.emit(
                        &thread_session,
                        "turn_error",
                        json!({ "session": thread_session, "error": err }),
                    );
                }
            }
        })
        .map_err(|e| {
            if let Ok(mut active) = state.active_turns.lock() {
                active.remove(&session_id);
            }
            (500, format!("failed to spawn solver thread: {e}"))
        })?;
    Ok(())
}

fn run_session_turn(
    state: &Arc<DesktopState>,
    session_id: &str,
    req: &Value,
    prompt: &str,
) -> Result<String, String> {
    let meta = state.load_meta(session_id);
    let ui = state.load_ui_settings();
    let provider_str = req
        .get("provider")
        .and_then(Value::as_str)
        .or_else(|| meta.as_ref().and_then(|m| m.default_provider.as_deref()))
        .or(ui.default_provider.as_deref())
        .unwrap_or("anthropic");
    let provider = ProviderKind::parse(provider_str)?;

    let model_requested = req
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| meta.as_ref().and_then(|m| m.default_model.as_deref()))
        .or(ui.default_model.as_deref())
        .unwrap_or_else(|| provider.default_model())
        .to_string();
    let model = provider.resolve_model(&model_requested);

    let max_turns = req
        .get("max_turns")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MAX_TURNS);
    let max_tokens = req
        .get("max_tokens")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MAX_TOKENS);

    let env = state.research_env_for_session(session_id)?;
    let transport = state.build_transport(provider, &model)?;
    let (router, containment, contained) =
        build_desktop_router(Arc::clone(state), Arc::new(env), session_id)?;
    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let mut sink = SseEventSink {
        sse: Arc::clone(&state.sse),
        session_id: session_id.to_string(),
        final_text: String::new(),
        tool_calls: std::collections::HashMap::new(),
    };

    if existing.is_empty() {
        let guardrails = GuardrailConfig {
            verification_mode: req
                .get("verification_mode")
                .and_then(Value::as_bool)
                .or_else(|| meta.as_ref().map(|m| m.verification_mode))
                .unwrap_or(false),
            calculation_mode: req
                .get("calculation_mode")
                .and_then(Value::as_bool)
                .or_else(|| meta.as_ref().map(|m| m.calculation_mode))
                .unwrap_or(false),
            pinned_conventions: req
                .get("pinned_conventions")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| meta.as_ref().and_then(|m| m.pinned_conventions.clone()))
                .or(ui.pinned_conventions),
            ..GuardrailConfig::default()
        };
        let cfg = AgentConfig {
            prompt: prompt.to_string(),
            model_requested,
            model,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            max_turns,
            max_tokens,
            containment,
            contained,
        };
        run_research_loop(&*transport, &router, &ledger, &cfg, &guardrails, &mut sink)?;
    } else {
        let defs = router.registry().defs();
        let expected_defs: Vec<tools::ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
        inspect_session_for_resume(
            &ledger,
            session_id,
            router.policy().version.as_str(),
            &expected_defs,
        )
        .map_err(|e| e.to_string())?;
        let cfg = ResumeConfig {
            prompt: prompt.to_string(),
            model_requested,
            model,
            system_prompt: None,
            max_turns,
            max_tokens,
            containment,
            contained,
        };
        resume_agent_loop(&*transport, &router, &ledger, &cfg, &mut sink)?;
    }

    Ok(sink.final_text)
}

struct WorkerEpochAdapter {
    epoch: Option<EpochHandle>,
    contained: bool,
}

impl WorkerEpoch for WorkerEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

#[cfg(windows)]
fn build_desktop_worker(state: &DesktopState, session: &str) -> Result<Option<Worker>, String> {
    let (Some(python), Some(script)) = (&state.cfg.python, &state.cfg.worker_script) else {
        return Ok(None);
    };
    let sdir = state.cfg.data_dir.join("sessions").join(session);
    let scratch = sdir.join("scratch");
    let pycache = sdir.join("pycache");
    let sandbox: Box<dyn Sandbox> = Box::new(
        AppContainerSandbox::new(
            state.cfg.data_dir.clone(),
            session,
            python.clone(),
            script.clone(),
            scratch.clone(),
            pycache.clone(),
        )
        .map_err(|e| format!("AppContainer sandbox setup failed: {e}"))?,
    );
    Ok(Some(Worker::new(
        WorkerConfig {
            python: python.clone(),
            script: script.clone(),
            scratch,
            pycache,
            startup_timeout: Duration::from_secs(90),
        },
        sandbox,
    )))
}

#[cfg(not(windows))]
fn build_desktop_worker(_state: &DesktopState, _session: &str) -> Result<Option<Worker>, String> {
    Ok(None)
}

pub(crate) fn build_desktop_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
) -> Result<(Router, String, bool), String> {
    let worker = build_desktop_worker(&state, session)?;
    let epoch_handle = worker.as_ref().map(Worker::epoch_handle);
    let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let containment = match &worker {
        Some(w) => w.containment().summary(),
        None => "desktop host (no sandboxed Python worker configured)".to_string(),
    };
    let worker = Arc::new(Mutex::new(worker));
    let timeout = Duration::from_secs(state.cfg.eval_timeout_s.max(30));

    let mut registry = Registry::new();
    registry.register_worker_session(
        builtin::sympy_eval_def(),
        Arc::new(WorkerEpochAdapter {
            epoch: epoch_handle,
            contained,
        }),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let code = args["code"].as_str().ok_or("`code` must be a string")?;
            let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
            let w = guard
                .as_mut()
                .ok_or("no Python compute worker configured (`--python` required)")?;
            let (result, ran_epoch) = w
                .evaluate_in_epoch(code, timeout, ctx.required_epoch)
                .map_err(|e| e.to_string())?;
            ctx.record_worker_epoch(ran_epoch);
            Ok(result)
        }),
    );
    registry.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
    register_research_tools(&mut registry, env);

    let ledger = state.open_ledger()?;
    let approver = Arc::new(UiApprover::new(state, containment.clone(), contained));
    let router = Router::new(registry, research_policy(), ledger, session, approver);
    Ok((router, containment, contained))
}

struct SseEventSink {
    sse: Arc<SseHub>,
    session_id: String,
    final_text: String,
    tool_calls: std::collections::HashMap<String, (String, Value)>,
}

impl EventSink for SseEventSink {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String> {
        match kind {
            "model.text" => {
                let text = data.get("text").and_then(Value::as_str).unwrap_or("");
                self.final_text = text.to_string();
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
            "model.thinking" => {
                let chars = data.get("chars").and_then(Value::as_u64).unwrap_or(0);
                let redacted = data.get("redacted").and_then(Value::as_bool).unwrap_or(false);
                let note = if redacted {
                    format!("[redacted reasoning block: {chars} chars]\n")
                } else {
                    format!("[reasoning block: {chars} chars]\n")
                };
                self.sse.emit(
                    &self.session_id,
                    "thinking_delta",
                    json!({ "seq": receipt.seq, "delta": note }),
                );
            }
            "model.tool_use" => {
                let cid = data.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let tname = data.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let targs = data.get("input").cloned().unwrap_or(Value::Null);
                if !cid.is_empty() {
                    self.tool_calls.insert(cid.clone(), (tname.clone(), targs.clone()));
                }
                self.sse.emit(
                    &self.session_id,
                    "tool_trace",
                    json!({
                        "seq": receipt.seq,
                        "call_id": cid,
                        "tool": tname,
                        "args": targs,
                        "status": "running",
                    }),
                );
            }
            "model.tool_result" => {
                let ok = !data.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let cid = data.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                let (tname, targs) = self
                    .tool_calls
                    .get(cid)
                    .cloned()
                    .unwrap_or_else(|| ("tool".to_string(), Value::Null));
                self.sse.emit(
                    &self.session_id,
                    "tool_trace",
                    json!({
                        "seq": receipt.seq,
                        "call_id": cid,
                        "tool": tname,
                        "args": targs,
                        "output": data.get("text").and_then(Value::as_str).unwrap_or(""),
                        "ok": ok,
                        "status": if ok { "ok" } else { "error" },
                    }),
                );
            }
            "proof.entry" | "proof.supersede" | "proof.verification" => {
                self.sse.emit(
                    &self.session_id,
                    "dag_updated",
                    json!({ "seq": receipt.seq, "kind": kind, "payload": data }),
                );
            }
            _ => {
                self.sse.emit(
                    &self.session_id,
                    "session_event",
                    json!({ "seq": receipt.seq, "kind": kind, "data": data }),
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn reconstruct_session_view(events: &[ledger::Event]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut messages = Vec::new();
    let mut traces = Vec::new();
    let mut sources = Vec::new();
    let mut pending_turn_traces: Vec<Value> = Vec::new();
    let mut pending_thinking = String::new();
    let mut tool_calls: std::collections::HashMap<String, (String, Value)> =
        std::collections::HashMap::new();

    for ev in events {
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                if let Some(prompt) = ev.payload.get("prompt").and_then(Value::as_str) {
                    messages.push(json!({ "role": "user", "text": prompt, "seq": ev.seq }));
                }
            }
            "model.thinking" => {
                let chars = ev.payload.get("chars").and_then(Value::as_u64).unwrap_or(0);
                pending_thinking.push_str(&format!("[reasoning block: {chars} chars]\n"));
            }
            "model.tool_use" => {
                let cid = ev.payload.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                let tname = ev.payload.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let targs = ev.payload.get("input").cloned().unwrap_or(Value::Null);
                if !cid.is_empty() {
                    tool_calls.insert(cid.clone(), (tname.clone(), targs.clone()));
                }
                traces.push(json!({
                    "seq": ev.seq,
                    "call_id": cid,
                    "tool": tname,
                    "args": targs,
                    "status": "running",
                }));
            }
            "model.tool_result" => {
                let ok = !ev.payload.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let cid = ev.payload.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                let (tname, targs) = tool_calls
                    .get(cid)
                    .cloned()
                    .unwrap_or_else(|| ("tool".to_string(), Value::Null));
                let item = json!({
                    "seq": ev.seq,
                    "call_id": cid,
                    "tool": tname,
                    "args": targs,
                    "output": ev.payload.get("text").and_then(Value::as_str).unwrap_or(""),
                    "ok": ok,
                    "status": if ok { "ok" } else { "error" },
                });
                traces.push(item.clone());
                pending_turn_traces.push(item);
            }
            "model.text" | "model.refusal" => {
                if let Some(text) = ev.payload.get("text").and_then(Value::as_str) {
                    messages.push(json!({
                        "role": "assistant",
                        "text": text,
                        "seq": ev.seq,
                        "thinking": std::mem::take(&mut pending_thinking),
                        "tool_traces": std::mem::take(&mut pending_turn_traces),
                    }));
                }
            }
            "source.ingested" => {
                sources.push(ev.payload.clone());
            }
            _ => {}
        }
    }
    (messages, traces, sources)
}
