//! Desktop turns on the subscription-CLI routes: the Claude subscription route
//! (`provider: "claude-cli"`) and the Antigravity route (`provider: "antigravity"`).
//!
//! An unmodified vendor CLI the user signed into runs the agent loop through `cli_route`.
//! re:Search's tools are served by `core.exe` over MCP (with the session's workspace and
//! the research policy), every call of them goes through the router in `core.exe`, and
//! approvals are answered by a person on the desktop approval cards. Events go to the
//! ledger and to the same SSE events the API route emits. Later turns replay a bounded
//! transcript of the session.
//!
//! On the Claude route the model gets ONLY those tools. The Antigravity route is
//! "vendor-sandboxed, not confined to re:Search tools": the CLI keeps its built-in tools,
//! which re:Search can only ask it to deny ([`cli_route::antigravity`]), and it needs the
//! person's acknowledgement first ([`crate::antigravity`]).

use crate::cli_relay::UiRelay;
pub use crate::cli_turn_setup::{session_route, CliRouteKind, DEFAULT_CLI_MODEL};
pub(crate) use crate::cli_turn_setup::wants_cli_route;
use crate::cli_turn_setup::{
    build_cli_prompt, check_sandbox_id, core_env_for_profile, path_str, pick_model,
    resolve_cli_turn_profile, resolve_core_exe, route_of_first_start,
};
use crate::routes_session::ApiResult;
use crate::state::{DesktopState, SessionMeta};
use crate::turn_runner::SseEventSink;
use cli_route::antigravity::Antigravity;
use cli_route::claude::ClaudeCode;
use cli_route::core_tools::core_tool_set_for_profile;
use cli_route::{run_turn, CoreLaunch, Recorder, SessionGuard, StopHandle, TurnEnd, TurnSpec, VendorCli};
use research_tools::paper_key::Mode;
use research_tools::policy_preset::SOLVE_CHECK_CALLER;
use research_tools::tool_profile::TOOL_PROFILE_FIELD;
use research_tools::{strip_verbatim_prefix, validate_session_id};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tools::canon::random_hex;

/// A subscription-CLI turn's stop handle, registered in [`DesktopState::cli_turns`] before its
/// thread is spawned (so a stop right after the 202 reaches it); removed when dropped.
pub(crate) struct Registered {
    state: Arc<DesktopState>,
    session: String,
    pub(crate) stop: StopHandle,
}

impl Registered {
    pub(crate) fn new(state: &Arc<DesktopState>, session_id: &str, kind: CliRouteKind) -> Result<Registered, String> {
        let stop = StopHandle::new();
        state
            .cli_turns
            .lock()
            .map_err(|_| "turn registry poisoned".to_string())?
            .insert(session_id.to_string(), (kind, stop.clone()));
        Ok(Registered { state: Arc::clone(state), session: session_id.to_string(), stop })
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut map) = self.state.cli_turns.lock() {
            map.remove(&self.session);
        }
    }
}

/// Run one desktop turn on a subscription-CLI route; returns the final reply text.
pub(crate) fn run_cli_turn(
    state: &Arc<DesktopState>,
    session_id: &str,
    req: &Value,
    message: &str,
    stop: &StopHandle,
    kind: CliRouteKind,
) -> Result<String, String> {
    // Locating the CLI follows the configured environment (tests point it at a fake).
    let env_lookup = |n: &str| state.env_var(n);
    // Billing safety checks credential vars against both the real process environment and
    // `connection_env` (a test map can add a blocked credential var, never hide a real one),
    // while opt-in bypass flags come strictly from the real process environment.
    let auth_env = |n: &str| {
        if n == cli_route::env_auth::ALLOW_ENV_AUTH_VAR
            || n == cli_route::antigravity::ALLOW_ENV_AUTH_VAR
        {
            std::env::var_os(n)
        } else {
            std::env::var_os(n).or_else(|| state.env_var(n))
        }
    };
    // The person's acknowledgement (Antigravity), then billing safety: nothing is started
    // while either is missing.
    let (vendor, allowed_env, allow_env_auth, explicit_exe): (Box<dyn VendorCli>, Vec<&str>, bool, Option<&Path>) =
        match kind {
            CliRouteKind::Claude => {
                let allowed = cli_route::env_auth::check(ClaudeCode.env_auth_vars(), &auth_env)?;
                (Box::new(ClaudeCode), allowed, cli_route::env_auth::opted_in(&auth_env), state.cfg.claude_exe.as_deref())
            }
            CliRouteKind::Antigravity => {
                crate::antigravity::require_acknowledged(state)?;
                let allowed = cli_route::antigravity::check_env_auth(&auth_env)?;
                let opted = cli_route::env_auth::flag_set(cli_route::antigravity::ALLOW_ENV_AUTH_VAR, &auth_env);
                (Box::new(Antigravity::new()), allowed, opted, state.cfg.agy_exe.as_deref())
            }
        };
    if stop.is_stopped() {
        return Err("turn stopped by the user".to_string());
    }
    let cli_exe = vendor.locate(explicit_exe, &env_lookup)?;
    let core_exe = resolve_core_exe(&state.cfg)?;
    let meta = state.load_meta(session_id);
    let allow_model_title = meta.as_ref().is_some_and(SessionMeta::allows_model_title);
    let ui = state.load_ui_settings();
    let model = pick_model(req, meta.as_ref(), &ui, kind)?;
    let conventions = req["pinned_conventions"].as_str()
        .or_else(|| meta.as_ref().and_then(|session| session.pinned_conventions.as_deref()))
        .or(ui.pinned_conventions.as_deref()).map(str::to_owned);
    let (reasoning_effort, subagent) =
        crate::subagents::resolve_turn_options(state, session_id, req, meta);
    let research = Arc::new(state.research_env_for_session(session_id)?);
    let project_context = crate::investigation::turn_context(crate::project_context::brief(state, session_id)?, req);

    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    if let Some(route) = route_of_first_start(&existing).filter(|r| r != kind.ledger_route()) {
        return Err(format!("session `{session_id}` was started on route `{route}`; it cannot continue on {}", kind.name()));
    }
    if !existing.is_empty() {
        ledger.verify_session(session_id).map_err(|e| format!("cannot continue session `{session_id}`: {e}"))?;
    }
    let (mode, profile) = resolve_cli_turn_profile(&existing)?;
    let turn_start_seq = existing.last().map_or(0, |e| e.seq);
    let eff_msg = if mode == Mode::Explore {
        message.to_string()
    } else {
        match &subagent {
            Some(sa) => crate::subagents::apply_subagent_to_turn_prompt(sa, message),
            None => message.to_string(),
        }
    };
    let needs_title = allow_model_title
        && crate::routes_session::model_session_title(&existing).is_none()
        && !matches!(mode, Mode::Investigate | Mode::Explore);
    let prompt_build = if mode == Mode::Explore {
        let focus_node = req["focus_node_id"]
            .as_str()
            .or_else(|| req["node_id"].as_str());
        crate::cli_turn_explore::build_explore_cli_prompt(
            &existing,
            Some(&ledger),
            &eff_msg,
            conventions.as_deref(),
            None,
            focus_node,
        )
    } else {
        build_cli_prompt(
            mode,
            &existing,
            Some(&ledger),
            &eff_msg,
            conventions.as_deref(),
            project_context.as_deref(),
            needs_title,
            state.cfg.python.is_some(),
        )
    };

    validate_session_id(session_id).map_err(|e| e.to_string())?;
    let sdir = state.session_dir(session_id)?;
    // The CLI's working directory, an empty per-session folder; for Antigravity also the
    // CLI's workspace, holding `.agents/mcp_config.json` during the turn.
    let work = sdir.join(match kind {
        CliRouteKind::Claude => "work",
        CliRouteKind::Antigravity => "agy",
    });
    let (scratch, pycache) = (sdir.join("scratch"), sdir.join("pycache"));
    for d in [&work, &scratch, &pycache] {
        std::fs::create_dir_all(d).map_err(|e| format!("could not create {}: {e}", d.display()))?;
    }
    let check_sdir = state.cfg.data_dir.join("sessions").join(check_sandbox_id(session_id));
    let (check_scratch, check_pycache) = (check_sdir.join("scratch"), check_sdir.join("pycache"));
    let check_dirs = if mode == Mode::Solve {
        for d in [&check_scratch, &check_pycache] {
            std::fs::create_dir_all(d).map_err(|e| format!("could not create {}: {e}", d.display()))?;
        }
        Some((check_scratch.as_path(), check_pycache.as_path()))
    } else {
        None
    };
    let _session_guard = SessionGuard::create(&state.cfg.data_dir, &sdir, session_id)?;

    let token = random_hex(32);
    let relay_decider = Arc::new(UiRelay { state: Arc::clone(state) });
    let relay = if mode == Mode::Solve {
        cli_route::relay::start_for_callers(
            state.open_ledger()?,
            session_id.to_string(),
            token.clone(),
            vec![vendor.caller(session_id), SOLVE_CHECK_CALLER.to_string()],
            relay_decider,
        )
    } else {
        cli_route::relay::start_for_caller(
            state.open_ledger()?,
            session_id.to_string(),
            token.clone(),
            vendor.caller(session_id),
            relay_decider,
        )
    }
    .map_err(|e| format!("could not start the approval relay: {e}"))?;
    let workspace = research.workspace_root.clone();
    let mut core = CoreLaunch {
        exe: core_exe,
        env: core_env_for_profile(
            &state.cfg,
            session_id,
            &token,
            &relay.addr,
            &scratch,
            &pycache,
            workspace.as_deref(),
            mode,
            profile.name,
            check_dirs,
        ),
    };
    core.env.insert("CORE_CALLER_ROUTE".into(), kind.ledger_route().into());
    let tools = core_tool_set_for_profile(Arc::clone(&research), profile.name)?;

    let start_kind = if !existing.iter().any(|e| e.kind == "session.start") {
        "session.start"
    } else {
        "session.resume"
    };
    let mut start_fields = json!({
        "prompt": message,
        "provider": kind.provider_id(),
        "surface": "desktop",
        "allow_model_title": needs_title,
        "history_replayed": prompt_build.replayed,
        "history_chars": prompt_build.history_chars,
        "history_turns": prompt_build.turns_included,
        "history_turns_dropped": prompt_build.turns_dropped,
        "workspace_root": workspace.as_deref().map(|w| path_str(&strip_verbatim_prefix(w))),
        "workspace_revision": workspace.as_deref().and_then(agent::workspace_revision),
        "project_context": project_context,
        TOOL_PROFILE_FIELD: profile.name,
    });
    if mode == Mode::Solve && start_kind == "session.resume" {
        start_fields["resume_mode"] = json!("brief");
        start_fields["brief_sha256"] = json!(ledger::sha256_hex(prompt_build.prompt.as_bytes()));
    }
    if kind == CliRouteKind::Antigravity {
        start_fields["confinement"] = json!(cli_route::antigravity::CONFINEMENT);
        start_fields["route_label"] = json!(crate::antigravity::AGY_LABEL);
        start_fields["vendor_builtin_tools"] =
            json!("not removable; denied only through the CLI's own --settings deny rules, enforced by the CLI");
    }
    if !allowed_env.is_empty() {
        start_fields["env_auth_vars_allowed"] = json!(allowed_env);
    }
    let tool_config = match kind {
        CliRouteKind::Claude => sdir.join(cli_route::TOOL_CONFIG_FILE),
        CliRouteKind::Antigravity => work.join(cli_route::antigravity::TOOL_CONFIG_REL),
    };
    let spec = TurnSpec {
        session: session_id,
        exe: &cli_exe,
        work_dir: &work,
        tool_config: &tool_config,
        core: &core,
        model: &model,
        reasoning_effort: reasoning_effort.as_deref(),
        prompt: &prompt_build.prompt,
        tool_defs: &tools.defs,
        policy_version: &tools.policy_version,
        start_kind,
        start_fields,
        inherit_stderr: false,
        allow_env_auth,
    };
    let rec = Recorder { ledger, session: session_id.to_string() };
    let mut sink = SseEventSink::new(Arc::clone(&state.sse), session_id);
    let outcome = run_turn(vendor.as_ref(), &rec, &mut sink, &spec, stop);
    drop(relay);
    let report = outcome?;
    let cli = kind.cli_name();
    match report.end {
        TurnEnd::Completed { exit_code: Some(0), init_verified: true } => {
            if mode == Mode::Solve {
                crate::cli_turn_solve::maybe_run_post_cli_solve_review(
                    state,
                    &research,
                    session_id,
                    stop,
                )?;
            }
            let text = sink.final_text().to_string();
            let final_text = if text.is_empty() { report.final_text } else { text };
            if mode == Mode::Explore {
                let caller = vendor.caller(session_id);
                crate::cli_turn_explore::record_explore_cli_citations(
                    state,
                    session_id,
                    turn_start_seq,
                    &caller,
                    &final_text,
                )?;
            }
            Ok(final_text)
        }
        TurnEnd::Completed { init_verified: false, .. } => match kind {
            CliRouteKind::Claude => Err("Claude Code ended without a verified system/init event; turn aborted".to_string()),
            CliRouteKind::Antigravity => Err(format!("{cli} ended without a verified init event; turn aborted")),
        },
        TurnEnd::Completed { exit_code, .. } => Err(format!("{cli} exited with code {exit_code:?}")),
        TurnEnd::Aborted(reason) => Err(format!("turn aborted: {reason}")),
        TurnEnd::Stopped => Err("turn stopped by the user".to_string()),
    }
}

/// Delete the session's tool configs if no live turn owns them (left behind when re:Search was
/// killed before its cleanup ran). Called when a session is opened.
pub fn sweep_session_tool_config(state: &DesktopState, session_id: &str) {
    let live = state.active_turns.lock().map(|a| a.contains(session_id)).unwrap_or(true);
    if let Ok(dir) = state.session_dir(session_id) {
        cli_route::remove_stale_tool_config(&dir, live);
    }
}

/// [`sweep_session_tool_config`] for every session, at server start.
pub fn sweep_all_tool_configs(state: &DesktopState) -> usize {
    cli_route::sweep_stale_tool_configs(&state.cfg.data_dir, &|id| {
        state.active_turns.lock().map(|a| a.contains(id)).unwrap_or(true)
    })
}

/// `POST /api/sessions/{id}/stop`: delegates to [`crate::turn_control::handle_stop_turn`].
pub fn handle_stop_turn(state: &DesktopState, session_id: &str) -> ApiResult {
    crate::turn_control::handle_stop_turn(state, session_id)
}

#[cfg(test)]
#[path = "cli_turn_tests.rs"]
mod tests;
