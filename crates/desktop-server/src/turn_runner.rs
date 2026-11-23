//! Background solver turn runner, research router builder, and SSE EventSink.

use crate::approver::UiApprover;
pub(crate) use crate::session_view::{reconstruct_session_view, SseEventSink};
use crate::state::DesktopState;
use crate::turn_control::ApiTurnGuard;
use agent::{
    compute_worker_tools, explore_worker_tools, inspect_session_for_resume, lean_worker_tools,
    literature_worker_tools, resume_autonomous_research_loop, run_autonomous_research_loop,
    AgentConfig, AutonomousConfig, AutonomousRoles, CancelToken, CustomSubagentDelegation,
    DelegatedWorkerSpec, DelegationConfig, GuardrailConfig, ResumeConfig, SessionOutcome,
    DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS, DEFAULT_WORKER_MAX_TURNS, EXPLORE_BRANCH_MAX_TURNS,
    RESEARCH_SYSTEM_PROMPT, WORKER_MAX_TOKENS,
};
use research_tools::{register_lean_tool, register_research_tools, research_policy, ResearchEnv};
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

fn make_worker_spec<'a>(
    role: &str,
    display_name: &str,
    built: &'a crate::agent_models::BuiltWorkerTransport,
    system_prompt: &str,
    allowed_tools: Vec<String>,
    max_turns: u32,
) -> DelegatedWorkerSpec<'a> {
    DelegatedWorkerSpec {
        role: role.to_string(),
        display_name: display_name.to_string(),
        transport: &*built.transport,
        model: built.model.clone(),
        routing_reason: built.routing_reason.clone(),
        system_prompt: system_prompt.to_string(),
        allowed_tools,
        max_turns,
        max_tokens: WORKER_MAX_TOKENS,
        reasoning_effort: None,
    }
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
    let (reasoning_effort, subagent) =
        crate::subagents::resolve_turn_options(state, session_id, req, meta.clone());
    let (provider, model_requested, model) = crate::agent_models::resolve_orchestrator_model(
        subagent.as_ref(),
        req,
        meta.as_ref(),
        &ui,
    )?;

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
    let v_built = crate::agent_models::resolve_optional_role_transport(
        state,
        &ui,
        req,
        "full_verifier",
        provider,
        &model,
    );
    let s_built = crate::agent_models::resolve_optional_role_transport(
        state,
        &ui,
        req,
        "strategist",
        provider,
        &model,
    );
    let compute_built = crate::agent_models::resolve_worker_transport(
        state,
        &ui,
        req,
        "compute_worker",
        provider,
        &model,
    );
    let lit_built = crate::agent_models::resolve_worker_transport(
        state,
        &ui,
        req,
        "literature_worker",
        provider,
        &model,
    );
    let lean_built = crate::agent_models::resolve_worker_transport(
        state,
        &ui,
        req,
        "lean_worker",
        provider,
        &model,
    );
    let explore_built = crate::agent_models::resolve_worker_transport(
        state,
        &ui,
        req,
        "explorer_worker",
        provider,
        &model,
    );
    let active_subagent_id = subagent.as_ref().map(|s| s.id.as_str());
    let custom_built: Vec<(crate::subagents::SubagentDef, crate::agent_models::BuiltWorkerTransport)> =
        crate::subagents::load_subagents(&state.cfg.data_dir)
            .into_iter()
            .filter(|sa| Some(sa.id.as_str()) != active_subagent_id)
            .filter_map(|sa| {
                crate::agent_models::resolve_custom_subagent_transport(state, &sa, provider, &model)
                    .map(|built| (sa, built))
            })
            .collect();
    let lean_router = Some(build_lean_router(Arc::clone(state), Arc::clone(&env), session_id)?);

    let (router, containment, contained) =
        build_desktop_router(Arc::clone(state), Arc::clone(&env), session_id)?;

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
    let delegation_cfg = DelegationConfig {
        compute_worker: compute_built.as_ref().map(|b| {
            make_worker_spec(
                "compute_worker",
                "Compute Worker",
                b,
                "You are the isolated Compute Worker for reSearch. Write and retry sandboxed Python/SymPy/NumPy scripts via `sympy_eval`, check identities with `check_expression_sanity`, and record substantive passing calculations via `ledger_append` (`symbolic_computation` or `numerical_computation`). Never call `ledger_append` with `final_answer`; return exact computed values, equations, and Proof-DAG `entry_id`s.",
                compute_worker_tools(),
                DEFAULT_WORKER_MAX_TURNS,
            )
        }),
        literature_worker: lit_built.as_ref().map(|b| {
            make_worker_spec(
                "literature_worker",
                "Literature Worker",
                b,
                "You are the isolated Literature Worker for reSearch. Use `arxiv_search`, `literature_search`, `doc_ingest`, `fs_list`, and `fs_read` (plus `ledger_query`/`ledger_note`) to retrieve papers and inspect local or ingested sources. Extract exact theorem statements, conventions, arXiv IDs/DOIs/links, and section references concisely.",
                literature_worker_tools(),
                DEFAULT_WORKER_MAX_TURNS,
            )
        }),
        lean_worker: lean_built.as_ref().map(|b| {
            make_worker_spec(
                "lean_worker",
                "Lean Prover Worker",
                b,
                "You are the isolated Lean Prover Worker for reSearch. Formalize mathematical claims in Lean 4 without `sorry`, `admit`, or unproven `axiom` declarations, run `lean_check` to type-check and anti-cheat verify the code, iterate on diagnostics, and report verified status, diagnostics, and any recorded Proof-DAG `entry_id`s.",
                lean_worker_tools(),
                DEFAULT_WORKER_MAX_TURNS,
            )
        }),
        lean_router: lean_router.as_ref(),
        explorer_worker: explore_built.as_ref().map(|b| {
            make_worker_spec(
                "explorer_worker",
                "Ultramath Explorer",
                b,
                "You are an isolated Ultramath Parallel Explorer branch for reSearch. Test only your assigned approach independently using allowed read/compute tools without modifying the Proof DAG. Report viability, exact symbolic/numerical checks, obstructions or counterexamples, and key equations concisely.",
                explore_worker_tools(),
                EXPLORE_BRANCH_MAX_TURNS,
            )
        }),
        custom_subagents: custom_built
            .iter()
            .map(|(sa, b)| CustomSubagentDelegation {
                id: sa.id.clone(),
                name: sa.name.clone(),
                description: sa.description.clone(),
                tools_profile: sa.tools_profile.clone(),
                system_prompt: sa.system_prompt.clone(),
                transport: &*b.transport,
                model: b.model.clone(),
                routing_reason: b.routing_reason.clone(),
                reasoning_effort: sa.reasoning_effort.clone(),
            })
            .collect(),
        pinned_conventions: guardrails.pinned_conventions.clone(),
    };
    let interactive_plan_mode = req.get("interactive_plan_mode").and_then(Value::as_bool).unwrap_or(false);
    let pause_after_explore = req.get("pause_after_explore").and_then(Value::as_bool).unwrap_or(false);
    let human_channel = crate::hitl::DesktopHumanChannel::new(Arc::clone(state), session_id);
    let roles = AutonomousRoles {
        verifier: v_built.as_ref().map(|(t, m)| (&**t as &dyn agent::Transport, m.as_str())),
        strategist: s_built.as_ref().map(|(t, m)| (&**t as &dyn agent::Transport, m.as_str())),
        delegation: Some(delegation_cfg),
        human: Some(&human_channel),
    };
    let auto_cfg = AutonomousConfig {
        guardrails,
        use_strategist: req.get("use_strategist").and_then(Value::as_bool).unwrap_or(false),
        interactive_plan_mode,
        pause_after_explore,
        ..AutonomousConfig::default()
    };

    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let mut sink = SseEventSink::new(Arc::clone(&state.sse), session_id);

    let has_started = existing.iter().any(|e| e.kind == "session.start");
    let outcome = if !has_started {
        let system_prompt = match &subagent {
            Some(sa) => crate::subagents::apply_subagent_to_system_prompt(RESEARCH_SYSTEM_PROMPT, sa),
            None => RESEARCH_SYSTEM_PROMPT.to_string(),
        };
        let cfg = AgentConfig {
            prompt: prompt.to_string(),
            model_requested,
            model,
            system_prompt,
            max_turns,
            max_tokens,
            containment,
            contained,
            reasoning_effort,
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
        let eff_prompt = match &subagent {
            Some(sa) => crate::subagents::apply_subagent_to_turn_prompt(sa, prompt),
            None => prompt.to_string(),
        };
        let cfg = ResumeConfig {
            prompt: eff_prompt,
            model_requested,
            model,
            system_prompt: None,
            max_turns,
            max_tokens,
            containment,
            contained,
            reasoning_effort,
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

pub(crate) fn build_lean_router(
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    session: &str,
) -> Result<Router, String> {
    let mut registry = Registry::new();
    register_lean_tool(&mut registry, env);
    let ledger = state.open_ledger()?;
    let approver = Arc::new(UiApprover::new(state, "lean4".into(), false));
    Ok(Router::new(registry, research_policy(), ledger, session, approver))
}
