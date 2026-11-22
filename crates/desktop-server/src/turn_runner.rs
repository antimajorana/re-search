//! Background solver turn runner, research router builder, and SSE EventSink.

use crate::approver::UiApprover;
pub(crate) use crate::session_view::{reconstruct_session_view, SseEventSink};
use crate::state::DesktopState;
use crate::turn_control::ApiTurnGuard;
use agent::{
    inspect_session_for_resume, resume_autonomous_research_loop, run_autonomous_research_loop, AgentConfig,
    AutonomousConfig, AutonomousRoles, CancelToken, GuardrailConfig, ProviderKind,
    ResumeConfig, SessionOutcome, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS, RESEARCH_SYSTEM_PROMPT,
};
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
    // `provider: "claude-cli"` / `"antigravity"` (or a session bound to one) runs on that
    // subscription-CLI route; anything else on the in-core API route. A session never mixes
    // routes.
    let on_cli_route = match crate::cli_turn::wants_cli_route(&state, &session_id, &req_json) {
        Ok(v) => v,
        Err(e) => {
            if let Ok(mut active) = state.active_turns.lock() {
                active.remove(&session_id);
            }
            return Err(e);
        }
    };

    // Register stop handles before spawning the worker thread so an immediate `POST .../stop`
    // after `202 Accepted` reaches both CLI and in-core API/ChatGPT turns.
    let (registered_cli, registered_api) = if let Some(kind) = on_cli_route {
        match crate::cli_turn::Registered::new(&state, &session_id, kind) {
            Ok(r) => (Some((r, kind)), None),
            Err(e) => {
                if let Ok(mut active) = state.active_turns.lock() {
                    active.remove(&session_id);
                }
                return Err((500, e));
            }
        }
    } else {
        match ApiTurnGuard::new(&state, &session_id) {
            Ok(g) => (None, Some(g)),
            Err(e) => {
                if let Ok(mut active) = state.active_turns.lock() {
                    active.remove(&session_id);
                }
                return Err((500, e));
            }
        }
    };

    let thread_state = Arc::clone(&state);
    let thread_session = session_id.clone();
    std::thread::Builder::new()
        .name(format!("turn-{session_id}"))
        .spawn(move || {
            let outcome = match (&registered_cli, &registered_api) {
                (Some((reg, kind)), _) => {
                    crate::cli_turn::run_cli_turn(&thread_state, &thread_session, &req_json, &prompt, &reg.stop, *kind)
                }
                (_, Some(api_guard)) => {
                    run_session_turn(&thread_state, &thread_session, &req_json, &prompt, &api_guard.cancel)
                }
                (None, None) => Err("missing turn registration".into()),
            };
            drop(registered_cli);
            drop(registered_api);
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
    cancel: &CancelToken,
) -> Result<String, String> {
    if cancel.is_cancelled() {
        return Err("stopped by user".into());
    }
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

    let env = Arc::new(state.research_env_for_session(session_id)?);
    let transport = state.build_transport(provider, &model)?;
    let resolve_role = |role_key: &str| -> Option<(Box<dyn agent::Transport>, String)> {
        let obj = req
            .get(role_key)
            .or_else(|| ui.role_presets.as_ref().and_then(|p| p.get(role_key)))?;
        let p = ProviderKind::parse(obj.get("provider")?.as_str()?).ok()?;
        let m = p.resolve_model(obj.get("model").and_then(Value::as_str).unwrap_or(""));
        if p == provider && m == model {
            return None;
        }
        state.build_transport(p, &m).ok().map(|t| (t, m))
    };
    let v_built = resolve_role("full_verifier").or_else(|| resolve_role("verifier"));
    let s_built = resolve_role("strategist");
    let roles = AutonomousRoles {
        verifier: v_built.as_ref().map(|(t, m)| (&**t as &dyn agent::Transport, m.as_str())),
        strategist: s_built.as_ref().map(|(t, m)| (&**t as &dyn agent::Transport, m.as_str())),
    };

    let (router, containment, contained) =
        build_desktop_router(Arc::clone(state), Arc::clone(&env), session_id)?;
    let caller = format!("agent:{}:{session_id}", transport.info().provider);
    if let Ok(grants) = state.network_session_grants.lock() {
        if let Some(g) = grants.get(&(session_id.to_string(), caller.clone())) {
            router.seed_network_read_grant(&caller, g.clone());
        }
    }

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
    let auto_cfg = AutonomousConfig {
        guardrails,
        use_strategist: req.get("use_strategist").and_then(Value::as_bool).unwrap_or(false),
        ..AutonomousConfig::default()
    };

    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let mut sink = SseEventSink::new(Arc::clone(&state.sse), session_id);

    let outcome = if existing.is_empty() {
        let cfg = AgentConfig {
            prompt: prompt.to_string(),
            model_requested,
            model,
            system_prompt: RESEARCH_SYSTEM_PROMPT.to_string(),
            max_turns,
            max_tokens,
            containment,
            contained,
        };
        run_autonomous_research_loop(
            &*transport,
            &roles,
            &router,
            &ledger,
            &env,
            &cfg,
            &auto_cfg,
            Some(cancel),
            &mut sink,
        )?
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
        resume_autonomous_research_loop(
            &*transport,
            &roles,
            &router,
            &ledger,
            &env,
            &cfg,
            &auto_cfg,
            Some(cancel),
            &mut sink,
        )?
    };

    if let Some(grant) = router.active_network_read_grant(&caller) {
        if let Ok(mut grants) = state.network_session_grants.lock() {
            grants.insert((session_id.to_string(), caller), grant);
        }
    }

    match outcome {
        SessionOutcome::Aborted { reason, .. } => Err(reason),
        SessionOutcome::Completed { .. } | SessionOutcome::Refusal { .. } => Ok(sink.final_text),
    }
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
    let approver = Arc::new(
        UiApprover::new(state, containment.clone(), contained).with_session_network_grant(true),
    );
    let router = Router::new(registry, research_policy(), ledger, session, approver);
    Ok((router, containment, contained))
}
