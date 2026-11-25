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
use crate::routes_session::ApiResult;
use crate::state::{DesktopState, SessionMeta, ANTIGRAVITY_ROUTE, CLAUDE_CLI_ROUTE};
use crate::turn_runner::SseEventSink;
use crate::DesktopConfig;
use cli_route::antigravity::Antigravity;
use cli_route::claude::ClaudeCode;
use cli_route::core_tools::core_tool_set;
use cli_route::history::{build_session_prompt, DEFAULT_HISTORY_BUDGET};
use cli_route::{run_turn, CoreLaunch, Recorder, SessionGuard, StopHandle, TurnEnd, TurnSpec, VendorCli};
use research_tools::{strip_verbatim_prefix, validate_session_id, LiteratureEndpoints};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tools::canon::random_hex;

/// Model alias used when neither the request nor the session names one.
pub const DEFAULT_CLI_MODEL: &str = "sonnet";

/// Which subscription CLI a session runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliRouteKind {
    Claude,
    Antigravity,
}

impl CliRouteKind {
    /// The `provider` value (and [`SessionMeta::route`]) of the route.
    pub fn provider_id(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_CLI_ROUTE,
            Self::Antigravity => ANTIGRAVITY_ROUTE,
        }
    }

    /// The route label its ledger start events carry.
    pub fn ledger_route(self) -> &'static str {
        match self {
            Self::Claude => cli_route::claude::ROUTE,
            Self::Antigravity => cli_route::antigravity::ROUTE,
        }
    }

    pub fn from_provider(p: &str) -> Option<Self> {
        [Self::Claude, Self::Antigravity].into_iter().find(|k| k.provider_id() == p)
    }

    pub fn from_ledger_route(r: &str) -> Option<Self> {
        [Self::Claude, Self::Antigravity].into_iter().find(|k| k.ledger_route() == r)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Antigravity => "Antigravity",
        }
    }

    fn cli_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Antigravity => "The Antigravity CLI",
        }
    }
}

fn route_of_first_start(events: &[ledger::Event]) -> Option<String> {
    events
        .iter()
        .find(|e| {
            (e.kind == "session.start" || e.kind == "session.resume")
                && e.payload.get("route").and_then(Value::as_str).is_some()
        })
        .and_then(|e| e.payload.get("route").and_then(Value::as_str))
        .map(str::to_string)
}

/// The subscription CLI a session's metadata binds it to, if any.
fn meta_kind(meta: Option<&SessionMeta>) -> Option<CliRouteKind> {
    let m = meta?;
    match m.route.as_deref() {
        Some(r) => CliRouteKind::from_provider(r),
        None => m.default_provider.as_deref().and_then(CliRouteKind::from_provider),
    }
}

/// Route (and provenance) to show for a session: what its first start event recorded, or
/// the subscription-CLI route the session is bound to when it has no turn yet.
pub fn session_route(meta: Option<&SessionMeta>, events: &[ledger::Event]) -> (Value, Value) {
    if let Some(start) = events.iter().find(|e| {
        (e.kind == "session.start" || e.kind == "session.resume")
            && e.payload.get("route").and_then(Value::as_str).is_some()
    }) {
        return (start.payload.get("route").cloned().unwrap_or(Value::Null), start.payload.get("provenance").cloned().unwrap_or(Value::Null));
    }
    if let Some(kind) = meta_kind(meta) {
        return (json!(kind.ledger_route()), json!(cli_route::PROVENANCE));
    }
    (Value::Null, Value::Null)
}
/// Which subscription-CLI route this turn runs on, if any. Binds an unbound session that
/// has no turns yet when the request asks for `claude-cli` or `antigravity`; refuses to mix
/// routes in one session (409). The binding is final only once a `session.start` is in the
/// ledger: until then (a first turn refused before it started) another provider may take
/// the session.
pub(crate) fn wants_cli_route(
    state: &DesktopState,
    session_id: &str,
    req: &Value,
) -> Result<Option<CliRouteKind>, (u16, String)> {
    let requested = req.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let meta = state.load_meta(session_id);
    let bound = meta_kind(meta.as_ref());
    let events = state
        .open_ledger()
        .and_then(|l| l.read_session(session_id).map_err(|e| e.to_string()))
        .map_err(|e| (500, e))?;
    let recorded = route_of_first_start(&events);
    let recorded_kind = recorded.as_deref().and_then(CliRouteKind::from_ledger_route);
    if let Some(kind) = requested.and_then(CliRouteKind::from_provider) {
        if recorded.is_some() && recorded_kind != Some(kind) {
            return Err((409, format!(
                "session `{session_id}` already has turns on another route; start a new session for {}",
                kind.name()
            )));
        }
        if kind == CliRouteKind::Antigravity {
            crate::antigravity::require_acknowledged(state).map_err(|e| (403, e))?;
        }
        if bound != Some(kind) {
            // Provisional until the turn records `session.start` (see the rebinding below).
            let mut m = meta.ok_or_else(|| (404, format!("session `{session_id}` has no metadata to bind")))?;
            m.route = Some(kind.provider_id().to_string());
            m.default_provider = Some(kind.provider_id().to_string());
            m.default_model = req.get("model").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
            state.save_meta(&m).map_err(|e| (500, e))?;
        }
        return Ok(Some(kind));
    }
    let runs_on = |kind: CliRouteKind, other: &str| {
        (409, format!(
            "session `{session_id}` runs on {} ({}); start a new session to use `{other}`",
            kind.name(),
            kind.provider_id()
        ))
    };
    match (requested, recorded_kind, bound) {
        (Some(other), Some(kind), _) => Err(runs_on(kind, other)),
        // Bound in its metadata, but no turn ever started there (refused before
        // `session.start`, for example by the billing check or a missing CLI): the binding
        // was only provisional, so the session may move to the API route.
        (Some(_), None, Some(kind)) if recorded.is_none() => {
            if let Some(mut m) = meta {
                m.route = None;
                if m.default_provider.as_deref() == Some(kind.provider_id()) {
                    m.default_provider = None;
                    m.default_model = None;
                }
                state.save_meta(&m).map_err(|e| (500, e))?;
            }
            Ok(None)
        }
        (Some(other), None, Some(kind)) => Err(runs_on(kind, other)),
        (Some(_), None, None) => Ok(None),
        (None, recorded_kind, bound) => {
            let mut resolved = recorded_kind.or(bound);
            let meta_provider_unset = meta
                .as_ref()
                .and_then(|m| m.default_provider.as_deref())
                .is_none_or(|s| s.trim().is_empty());
            if resolved.is_none() && recorded.is_none() && meta_provider_unset {
                resolved = crate::agent_models::orchestrator_cli_override(state).map(|(k, _)| k);
            }
            if resolved == Some(CliRouteKind::Antigravity) {
                crate::antigravity::require_acknowledged(state).map_err(|e| (403, e))?;
            }
            Ok(resolved)
        }
    }
}
/// A model alias, full model name or slug: never something the CLI could read as a flag.
fn pick_model(
    req: &Value,
    meta: Option<&SessionMeta>,
    ui: &crate::state::UiSettings,
    kind: CliRouteKind,
) -> Result<String, String> {
    let default = match kind {
        CliRouteKind::Claude => DEFAULT_CLI_MODEL,
        CliRouteKind::Antigravity => crate::antigravity::DEFAULT_AGY_MODEL,
    };
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| meta.filter(|m| meta_kind(Some(m)) == Some(kind)).and_then(|m| m.default_model.clone()))
        .or_else(|| {
            ui.agent_models
                .get("orchestrator")
                .filter(|s| CliRouteKind::from_provider(&s.provider) == Some(kind))
                .map(|s| s.model.clone())
        })
        .unwrap_or_else(|| default.to_string());
    let ok = match kind {
        CliRouteKind::Claude => {
            !model.starts_with('-')
                && model.len() <= 100
                && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']' | '@'))
        }
        CliRouteKind::Antigravity => cli_route::antigravity::valid_model_id(&model),
    };
    if ok {
        Ok(model)
    } else {
        Err(format!("invalid model `{model}` for {}", kind.name()))
    }
}
fn resolve_core_exe(cfg: &DesktopConfig) -> Result<PathBuf, String> {
    let check = |p: PathBuf, from: &str| {
        if p.is_file() {
            Ok(p)
        } else {
            Err(format!("{} not found ({from})", p.display()))
        }
    };
    if let Some(p) = &cfg.core_exe {
        return check(p.clone(), "--core-exe");
    }
    if let Some(p) = std::env::var_os("RE_SEARCH_CORE_EXE").filter(|v| !v.is_empty()) {
        return check(PathBuf::from(p), "RE_SEARCH_CORE_EXE");
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let beside = exe.parent().ok_or("executable has no directory")?.join(format!("core{}", std::env::consts::EXE_SUFFIX));
    check(beside, "beside the running executable; pass --core-exe or set RE_SEARCH_CORE_EXE")
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Environment for `core.exe`: session, token, relay, data, worker, workspace and research
/// tools (always on for desktop sessions, as on the API route).
fn core_env(
    cfg: &DesktopConfig,
    session: &str,
    token: &str,
    relay_addr: &str,
    scratch: &Path,
    pycache: &Path,
    workspace: Option<&Path>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = [
        ("CORE_SESSION", session.to_string()),
        ("CORE_SESSION_TOKEN", token.to_string()),
        ("CORE_DATA_DIR", path_str(&cfg.data_dir)),
        ("CORE_APPROVAL_ADDR", relay_addr.to_string()),
        ("CORE_SCRATCH", path_str(scratch)),
        ("CORE_PYCACHE", path_str(pycache)),
        ("CORE_ENABLE_RESEARCH_TOOLS", "1".to_string()),
        ("CORE_EVAL_TIMEOUT_S", cfg.eval_timeout_s.max(30).to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    if let (Some(py), Some(script)) = (&cfg.python, &cfg.worker_script) {
        env.insert("CORE_PYTHON".into(), path_str(py));
        env.insert("CORE_WORKER_SCRIPT".into(), path_str(script));
    }
    if let Some(ws) = workspace {
        env.insert("CORE_WORKSPACE".into(), path_str(&strip_verbatim_prefix(ws)));
    }
    let ep = &cfg.literature_endpoints;
    let def = LiteratureEndpoints::default();
    for (name, val, default) in [
        ("RE_SEARCH_ARXIV_BASE", &ep.arxiv_api_base, &def.arxiv_api_base),
        ("RE_SEARCH_INSPIRE_BASE", &ep.inspire_api_base, &def.inspire_api_base),
        ("RE_SEARCH_CROSSREF_BASE", &ep.crossref_api_base, &def.crossref_api_base),
        ("RE_SEARCH_S2_BASE", &ep.semantic_scholar_api_base, &def.semantic_scholar_api_base),
    ] {
        if val != default {
            env.insert(name.into(), val.clone());
        }
    }
    env
}

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
    let mut eff_msg = match &subagent {
        Some(sa) => crate::subagents::apply_subagent_to_turn_prompt(sa, message),
        None => message.to_string(),
    };
    let needs_title = allow_model_title && crate::routes_session::model_session_title(&existing).is_none()
        && !crate::investigation::is_investigation(&existing);
    if needs_title {
        eff_msg = format!("Once you can state the main research question, use set_session_title once with a concise title in the user's language.\n\n{eff_msg}");
    }
    if let Some(text) = conventions.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        eff_msg = format!("Pinned research conventions\n{text}\n\n{eff_msg}");
    }
    eff_msg = format!(
        "Use `search_paper_passages` and `read_paper_lines` to locate and read exact numbered passages in ingested papers, and \
         `search_formal_library` to discover Lean declarations in the attached workspace. \
         Cite recorded source locations, and type-check any intended formal application.\n\n{eff_msg}"
    );
    if state.cfg.python.is_some() {
        eff_msg = format!(
            "The re:Search compute worker exposes `check_symbolic_identity(lhs, rhs, variable=x, domain=...)` \
             and `check_convergence(run, levels, axis=..., atol=..., rtol=..., reference=...)` in `sympy_eval`. \
             Return the report as the final Python expression and record its `tool_call_ref` with the proof entry. \
             Report excluded points, counterexamples, and the scope of numerical evidence. \n\n{eff_msg}"
        );
    }
    if let Some(context) = &project_context {
        eff_msg = format!("{context}\n\nCurrent user request\n{eff_msg}");
    }
    let replay = build_session_prompt(&existing, &eff_msg, DEFAULT_HISTORY_BUDGET);

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
    let _session_guard = SessionGuard::create(&state.cfg.data_dir, &sdir, session_id)?;

    let token = random_hex(32);
    let relay = cli_route::relay::start_for_caller(
        state.open_ledger()?,
        session_id.to_string(),
        token.clone(),
        vendor.caller(session_id),
        Arc::new(UiRelay { state: Arc::clone(state) }),
    )
    .map_err(|e| format!("could not start the approval relay: {e}"))?;
    let workspace = research.workspace_root.clone();
    let mut core = CoreLaunch {
        exe: core_exe,
        env: core_env(&state.cfg, session_id, &token, &relay.addr, &scratch, &pycache, workspace.as_deref()),
    };
    core.env.insert("CORE_CALLER_ROUTE".into(), kind.ledger_route().into());
    let tools = core_tool_set(Some(Arc::clone(&research)));

    let mut start_fields = json!({
        "prompt": message,
        "provider": kind.provider_id(),
        "surface": "desktop",
        "allow_model_title": needs_title,
        "history_replayed": replay.replayed,
        "history_chars": replay.history_chars,
        "history_turns": replay.turns_included,
        "history_turns_dropped": replay.turns_dropped,
        "workspace_root": workspace.as_deref().map(|w| path_str(&strip_verbatim_prefix(w))),
        "workspace_revision": workspace.as_deref().and_then(agent::workspace_revision),
        "project_context": project_context,
    });
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
        prompt: &replay.prompt,
        tool_defs: &tools.defs,
        policy_version: &tools.policy_version,
        start_kind: if !existing.iter().any(|e| e.kind == "session.start") { "session.start" } else { "session.resume" },
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
            let text = sink.final_text().to_string();
            Ok(if text.is_empty() { report.final_text } else { text })
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
mod tests {
    use super::*;

    #[test]
    fn provisional_cli_switch_clears_stale_default_model() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = DesktopConfig {
            port: 0,
            data_dir: tmp.path().to_path_buf(),
            workspace_root: None,
            python: None,
            worker_script: None,
            api_base_override: None,
            literature_endpoints: LiteratureEndpoints::default(),
            open_window: false,
            print_url_json: false,
            eval_timeout_s: 30,
            core_exe: None,
            claude_exe: None,
            agy_exe: None,
            connection_env: Some(std::collections::BTreeMap::new()),
            chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
        };
        let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
        let state = DesktopState::new(
            cfg,
            crate::auth::AuthGuard::new(1).unwrap(),
            Arc::new(crate::sse::SseHub::new()),
            chatgpt,
        );
        crate::antigravity::handle_acknowledge(&state).unwrap();
        let meta = SessionMeta {
            id: "s-switch01".into(),
            title: "switch".into(),
            route: Some(CLAUDE_CLI_ROUTE.into()),
            default_provider: Some(CLAUDE_CLI_ROUTE.into()),
            default_model: Some("haiku".into()),
            ..SessionMeta::default()
        };
        state.save_meta(&meta).unwrap();

        let kind = wants_cli_route(&state, "s-switch01", &json!({ "provider": "antigravity" })).unwrap();
        assert_eq!(kind, Some(CliRouteKind::Antigravity));
        let updated = state.load_meta("s-switch01").unwrap();
        assert_eq!(updated.route.as_deref(), Some(ANTIGRAVITY_ROUTE));
        assert_eq!(updated.default_model, None, "stale claude-cli model must be cleared");
    }

    #[test]
    fn connection_env_cannot_supply_claude_env_auth_opt_in_for_turns() {
        if std::env::var_os(cli_route::env_auth::ALLOW_ENV_AUTH_VAR).is_some() {
            return;
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = DesktopConfig {
            port: 0,
            data_dir: tmp.path().to_path_buf(),
            workspace_root: None,
            python: None,
            worker_script: None,
            api_base_override: None,
            literature_endpoints: LiteratureEndpoints::default(),
            open_window: false,
            print_url_json: false,
            eval_timeout_s: 30,
            core_exe: None,
            claude_exe: None,
            agy_exe: None,
            connection_env: Some(BTreeMap::from([
                ("ANTHROPIC_API_KEY".to_string(), "sk-ant-dummy".to_string()),
                (cli_route::env_auth::ALLOW_ENV_AUTH_VAR.to_string(), "1".to_string()),
            ])),
            chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
        };
        let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
        let state = Arc::new(DesktopState::new(
            cfg,
            crate::auth::AuthGuard::new(1).unwrap(),
            Arc::new(crate::sse::SseHub::new()),
            chatgpt,
        ));
        let err = run_cli_turn(&state, "s-env01", &json!({}), "hi", &StopHandle::new(), CliRouteKind::Claude).unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY") && !err.contains("sk-ant-dummy"), "{err}");
    }
}

