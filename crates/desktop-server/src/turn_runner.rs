//! Background solver turn runner and SSE EventSink. Routers are built in `routers.rs`.

pub(crate) use crate::routers::{build_desktop_router, build_lean_router};
pub(crate) use crate::session_view::{reconstruct_session_view, SseEventSink};
use crate::state::DesktopState;
use crate::turn_control::ApiTurnGuard;
use agent::{
    compute_worker_tools, explore_worker_tools, inspect_session_for_resume_with_profile,
    lean_worker_tools, literature_worker_tools, resume_autonomous_research_loop,
    run_autonomous_research_loop, AgentConfig, AutonomousConfig, AutonomousRoles, CancelToken,
    CustomSubagentDelegation, DelegatedWorkerSpec, DelegationConfig, GuardrailConfig, ResumeConfig,
    SessionOutcome, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TURNS, DEFAULT_WORKER_MAX_TURNS,
    EXPLORE_BRANCH_MAX_TURNS, WORKER_MAX_TOKENS,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) fn spawn_turn_thread(
    state: Arc<DesktopState>,
    session_id: String,
    mut req_json: Value,
    prompt: String,
) -> Result<(), (u16, String)> {
    crate::investigation_tasks::strip_internal_task_fields(&mut req_json);
    spawn_turn_thread_inner(state, session_id, req_json, prompt, false)
}

pub(crate) fn spawn_task_turn_thread(
    state: Arc<DesktopState>,
    session_id: String,
    req_json: Value,
    prompt: String,
) -> Result<(), (u16, String)> {
    spawn_turn_thread_inner(state, session_id, req_json, prompt, true)
}

fn spawn_turn_thread_inner(
    state: Arc<DesktopState>,
    session_id: String,
    mut req_json: Value,
    prompt: String,
    allow_task_spec: bool,
) -> Result<(), (u16, String)> {
    let ledger = state
        .open_ledger()
        .map_err(|e| (500, e))?;
    let pre_events = ledger
        .read_session(&session_id)
        .map_err(|e| (500, e.to_string()))?;
    if let Err(err) = crate::session_mode::profile_for_session(&pre_events) {
        return Err((400, err));
    }
    let pre_mode = crate::session_mode::session_mode(&pre_events);
    let is_solve = pre_mode == research_tools::paper_key::Mode::Solve;
    let is_explore = pre_mode == research_tools::paper_key::Mode::Explore;
    if is_solve
        && research_tools::solve_state::SolveState::from_events(&pre_events)
            .setup
            .is_none()
    {
        return Err((400, "Solve session requires setup before running a turn".to_string()));
    }
    if is_explore {
        crate::idea_chat::validate_explore_turn_preconditions(&state, &session_id, &pre_events, &req_json)?;
    }

    let activity = crate::session_lifecycle::reserve_turn(&state, &session_id)?;
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
    if allow_task_spec {
        let req_prov = req_json.get("provider").and_then(Value::as_str).map(str::trim);
        let bound_route = state.load_meta(&session_id).and_then(|m| m.route);
        if matches!(req_prov, Some("claude-cli" | "antigravity"))
            || matches!(bound_route.as_deref(), Some("claude-cli" | "antigravity"))
        {
            if let Ok(mut active) = state.active_turns.lock() {
                active.remove(&session_id);
            }
            return Err((
                400,
                "Investigate v2 tasks require an API or ChatGPT provider until `cli_turn.rs` passes `CORE_TOOL_PROFILE`.".to_string(),
            ));
        }
    }
    let on_cli_route = match crate::cli_turn::wants_cli_route(&state, &session_id, &req_json) {
        Ok(v) => v,
        Err(e) => {
            if let Ok(mut active) = state.active_turns.lock() {
                active.remove(&session_id);
            }
            return Err(e);
        }
    };
    if is_explore && on_cli_route.is_some() {
        if let Ok(mut active) = state.active_turns.lock() {
            active.remove(&session_id);
        }
        return Err((400, crate::idea_chat::EXPLORE_CHAT_CLI_UNSUPPORTED_MSG.to_string()));
    }
    if allow_task_spec && on_cli_route.is_some() {
        if let Ok(mut active) = state.active_turns.lock() {
            active.remove(&session_id);
        }
        return Err((
            400,
            "Investigate v2 tasks require an API or ChatGPT provider until `cli_turn.rs` passes `CORE_TOOL_PROFILE`.".to_string(),
        ));
    }

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

    let paper_route = if on_cli_route.is_some() {
        crate::investigation::PaperRoute::Cli
    } else {
        crate::investigation::PaperRoute::InCore
    };
    let is_v6_task = if allow_task_spec {
        match crate::investigation_tasks::prepare_task(&state, &session_id, &mut req_json) {
            Ok(v) => v,
            Err(error) => {
                if let Ok(mut active) = state.active_turns.lock() { active.remove(&session_id); }
                return Err(error);
            }
        }
    } else {
        false
    };
    if !is_v6_task && !is_solve && !is_explore {
        if let Err(error) =
            crate::investigation::prepare(&state, &session_id, &mut req_json, &prompt, paper_route)
        {
            if let Ok(mut active) = state.active_turns.lock() { active.remove(&session_id); }
            return Err(error);
        }
    }

    let spawn_fail_req = is_v6_task.then(|| req_json.clone());
    let thread_state = Arc::clone(&state);
    let thread_session = session_id.clone();
    std::thread::Builder::new()
        .name(format!("turn-{session_id}"))
        .spawn(move || {
            let _activity = activity;
            let (outcome, cancelled) = match (&registered_cli, &registered_api) {
                (Some((reg, kind)), _) => {
                    let caller = format!("{}:{}", kind.ledger_route(), thread_session);
                    let res = crate::investigation_tasks::record_supplied_for_caller(
                        &thread_state, &thread_session, &req_json, &caller,
                    ).and_then(|()| {
                        crate::cli_turn::run_cli_turn(&thread_state, &thread_session, &req_json, &prompt, &reg.stop, *kind)
                    });
                    (res, reg.stop.is_stopped())
                }
                (_, Some(api_guard)) => {
                    let res = run_session_turn(&thread_state, &thread_session, &req_json, &prompt, &api_guard.cancel);
                    (res, api_guard.cancel.is_cancelled())
                }
                (None, None) => (Err("missing turn registration".into()), false),
            };
            let outcome = if is_explore {
                outcome
            } else {
                crate::investigation_finish::finish_with_cancel(
                    &thread_state, &thread_session, &req_json, outcome, cancelled,
                )
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
            let err = format!("failed to spawn solver thread: {e}");
            if let Some(req_meta) = &spawn_fail_req {
                crate::investigation_tasks::fail_task_on_spawn_error(&state, &session_id, req_meta, &err);
            }
            if let Ok(mut active) = state.active_turns.lock() {
                active.remove(&session_id);
            }
            (500, err)
        })?;
    Ok(())
}

pub(crate) fn make_worker_spec<'a>(
    role: &str,
    display_name: &str,
    built: &'a crate::agent_models::BuiltWorkerTransport,
    system_prompt: &str,
    allowed_tools: Vec<String>,
    max_turns: u32,
    effort: Option<String>,
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
        reasoning_effort: effort,
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
    let pre_ledger = state.open_ledger()?;
    let pre_events = pre_ledger.read_session(session_id).map_err(|e| e.to_string())?;
    match crate::session_mode::session_mode(&pre_events) {
        research_tools::paper_key::Mode::Solve => {
            return crate::solve_turn_routes::run_desktop_solve_turn(state, session_id, req, prompt, cancel);
        }
        research_tools::paper_key::Mode::Explore => {
            return crate::idea_chat::run_desktop_explore_turn(state, session_id, req, prompt, cancel);
        }
        _ => {}
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
    let prepared_paper_reader = crate::paper_reader::prepare_for_turn(
        state,
        &env,
        &ui,
        req,
        session_id,
        provider,
        &model,
    )?;
    let project_context = crate::paper_reader::append_reader_guidance(
        crate::investigation::turn_context(crate::project_context::brief(state, session_id)?, req),
        prepared_paper_reader.is_some(),
    );
    let transport = state.build_transport(provider, &model)?;
    let in_core_caller = format!("agent:{}:{}", transport.info().provider, session_id);
    crate::investigation_tasks::record_supplied_for_caller(state, session_id, req, &in_core_caller)?;
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
    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let mode = crate::session_mode::session_mode(&existing);
    let profile = crate::session_mode::profile_for_session(&existing)?;
    let is_investigation = mode == research_tools::paper_key::Mode::Investigate;
    let lean_router = Some(build_lean_router(Arc::clone(state), Arc::clone(&env), session_id)?);

    let (router, containment, contained) = crate::routers::build_desktop_router_for_profile(
        Arc::clone(state),
        Arc::clone(&env),
        session_id,
        profile.name,
    )?;

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
    let paper_reading_turn = req.get("_paper_map_request").is_some()
        || req.get("_paper_branch_request").is_some()
        || req.get("_paper_conventions_request").is_some()
        || crate::investigation_tasks::is_task_turn(req);
    let delegation_cfg = DelegationConfig {
        compute_worker: if paper_reading_turn { None } else { compute_built.as_ref().map(|b| {
            make_worker_spec(
                "compute_worker",
                "Compute Worker",
                b,
                "You are the isolated Compute Worker for reSearch. Use `sympy_eval` to run Python/SymPy/NumPy. \
                 Use `check_symbolic_identity(lhs, rhs, variable=x, domain=...)` for explicit domains and branch checks. \
                 Supply `limits=[(point, direction)]` or `dimensions={symbol: {base: power}}` where relevant. \
                 Use `check_convergence(run, levels, axis=..., atol=..., rtol=..., reference=...)` for precision, resolution, or basis refinement. \
                 Choose tolerances and an independent reference when available before running. Return the helper report as the last expression. \
                 Record checks and counterexamples with `tool_call_ref` on a `symbolic_computation` or `numerical_computation` Proof DAG entry. \
                 State excluded points and limitations. Treat a stable numerical result as empirical evidence. \
                 Use `set_random_seed(seed)` for stochastic calculations. Leave the final answer to the Orchestrator.",
                compute_worker_tools(),
                DEFAULT_WORKER_MAX_TURNS,
                None,
            )
        }) },
        literature_worker: lit_built.as_ref().map(|b| {
            make_worker_spec(
                "literature_worker",
                "Literature Worker",
                b,
                "You are the isolated Literature Worker for reSearch. Use `arxiv_search`, `literature_search`, and `doc_ingest` to find or ingest papers. \
                 Use `search_paper_passages` to find supporting passages in ingested papers, `read_paper_lines` for numbered line ranges, and `fs_read` to inspect surrounding context. \
                 Return exact passages, source paths, line ranges and tool references alongside terms, conventions, and arXiv IDs or DOIs.",
                literature_worker_tools(),
                DEFAULT_WORKER_MAX_TURNS,
                None,
            )
        }),
        lean_worker: if paper_reading_turn { None } else { lean_built.as_ref().map(|b| {
            make_worker_spec(
                "lean_worker",
                "Lean Prover Worker",
                b,
                "You are the isolated Lean Prover Worker for reSearch. Use `search_formal_library` to find reusable Lean and Physlib declarations in the attached workspace. \
                 Compare their statements and assumptions to the precise target. Run `lean_check` on the formalized claim without `sorry`, `admit`, or unproven axioms. \
                 Report reusable declarations, verified status, diagnostics, and recorded Proof DAG entries.",
                lean_worker_tools(),
                DEFAULT_WORKER_MAX_TURNS,
                None,
            )
        }) },
        lean_router: if paper_reading_turn { None } else { lean_router.as_ref() },
        explorer_worker: if paper_reading_turn { None } else { explore_built.as_ref().map(|b| {
            make_worker_spec(
                "explorer_worker",
                "Ultramath Explorer",
                b,
                "You are an isolated Ultramath Parallel Explorer branch for reSearch. Test only your assigned approach independently using allowed read/compute tools without modifying the Proof DAG. Report viability, exact symbolic/numerical checks, obstructions or counterexamples, and key equations concisely.",
                explore_worker_tools(),
                EXPLORE_BRANCH_MAX_TURNS,
                None,
            )
        }) },
        paper_reader: prepared_paper_reader.as_ref().map(|r| r.as_config()),
        custom_subagents: if paper_reading_turn {
            Vec::new()
        } else {
            custom_built
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
                .collect()
        },
        pinned_conventions: guardrails.pinned_conventions.clone(),
        is_investigation,
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
        project_context,
        use_strategist: req.get("use_strategist").and_then(Value::as_bool).unwrap_or(false),
        interactive_plan_mode,
        pause_after_explore,
        ..AutonomousConfig::default()
    };

    let mut sink = SseEventSink::new(Arc::clone(&state.sse), session_id).with_model_title(
        meta.as_ref().is_some_and(crate::state::SessionMeta::allows_model_title)
            && crate::routes_session::model_session_title(&existing).is_none(),
    );

    let has_started = existing.iter().any(|e| e.kind == "session.start");
    let outcome = if !has_started {
        let base_prompt = crate::investigation_system_prompt::base_system_prompt(&existing);
        let system_prompt = match &subagent {
            Some(sa) => crate::subagents::apply_subagent_to_system_prompt(base_prompt, sa),
            None => base_prompt.to_string(),
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
        inspect_session_for_resume_with_profile(
            &ledger,
            session_id,
            router.policy().version.as_str(),
            &expected_defs,
            router.tool_profile(),
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
        SessionOutcome::Completed { .. }
        | SessionOutcome::Refusal { .. }
        | SessionOutcome::Paused { .. } => Ok(sink.final_text().to_string()),
    }
}
