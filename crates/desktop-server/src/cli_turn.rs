//! Desktop turns on the Claude subscription route (`provider: "claude-cli"`).
//!
//! The unmodified Claude Code CLI the user signed into runs the agent loop, exactly as
//! `re-search run --route cli` does on the console, through `cli_route`. The model gets
//! only re:Search's tools, served by `core.exe` over MCP (with the session's workspace
//! and the research policy), every tool call goes through the router in `core.exe`, and
//! approvals are answered by a person on the desktop approval cards. Events go to the
//! ledger and to the same SSE events the API route emits, so the UI needs no
//! route-specific code. Later turns replay a bounded transcript of the session.

use crate::cli_relay::UiRelay;
use crate::routes_session::ApiResult;
use crate::state::{DesktopState, SessionMeta, CLAUDE_CLI_ROUTE};
use crate::turn_runner::SseEventSink;
use crate::DesktopConfig;
use cli_route::claude::{ClaudeCode, ROUTE};
use cli_route::core_tools::core_tool_set;
use cli_route::history::{build_prompt, prior_turns, DEFAULT_HISTORY_BUDGET};
use cli_route::{run_turn, CoreLaunch, Recorder, SessionGuard, StopHandle, TurnEnd, TurnSpec, VendorCli};
use research_tools::{strip_verbatim_prefix, validate_session_id, LiteratureEndpoints};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tools::canon::random_hex;

/// Model alias used when neither the request nor the session names one.
pub const DEFAULT_CLI_MODEL: &str = "sonnet";

fn route_of_first_start(events: &[ledger::Event]) -> Option<String> {
    events
        .iter()
        .find(|e| e.kind == "session.start")
        .and_then(|e| e.payload.get("route").and_then(Value::as_str))
        .map(str::to_string)
}

fn meta_bound(meta: Option<&SessionMeta>) -> bool {
    meta.is_some_and(|m| match m.route.as_deref() {
        Some(r) => r == CLAUDE_CLI_ROUTE,
        None => m.default_provider.as_deref() == Some(CLAUDE_CLI_ROUTE),
    })
}

/// Route (and provenance) to show for a session: what its first start event recorded, or
/// the Claude subscription route when the session is bound to it but has no turn yet.
pub fn session_route(meta: Option<&SessionMeta>, events: &[ledger::Event]) -> (Value, Value) {
    if let Some(start) = events.iter().find(|e| e.kind == "session.start") {
        return (start.payload.get("route").cloned().unwrap_or(Value::Null), start.payload.get("provenance").cloned().unwrap_or(Value::Null));
    }
    if meta_bound(meta) {
        return (json!(ROUTE), json!(cli_route::PROVENANCE));
    }
    (Value::Null, Value::Null)
}

/// Whether this turn runs on the Claude subscription route. Binds an unbound session that
/// has no turns yet when the request asks for `claude-cli`; refuses to mix routes in one
/// session (409). The binding is final only once a `session.start` is in the ledger: until
/// then (a first turn refused before it started) another provider may take the session.
pub(crate) fn wants_cli_route(state: &DesktopState, session_id: &str, req: &Value) -> Result<bool, (u16, String)> {
    let requested = req.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let meta = state.load_meta(session_id);
    let bound = meta_bound(meta.as_ref());
    let events = state
        .open_ledger()
        .and_then(|l| l.read_session(session_id).map_err(|e| e.to_string()))
        .map_err(|e| (500, e))?;
    let recorded = route_of_first_start(&events);
    let ledger_cli = recorded.as_deref() == Some(ROUTE);
    let ledger_other = recorded.is_some() && !ledger_cli;
    match requested {
        Some(CLAUDE_CLI_ROUTE) => {
            if ledger_other {
                return Err((409, format!(
                    "session `{session_id}` already has turns on another route; start a new session for the Claude subscription route"
                )));
            }
            if !bound {
                // Provisional until the turn records `session.start` (see the rebinding below).
                let mut m = meta.ok_or_else(|| (404, format!("session `{session_id}` has no metadata to bind")))?;
                m.route = Some(CLAUDE_CLI_ROUTE.to_string());
                m.default_provider = Some(CLAUDE_CLI_ROUTE.to_string());
                if let Some(model) = req.get("model").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                    m.default_model = Some(model.to_string());
                }
                state.save_meta(&m).map_err(|e| (500, e))?;
            }
            Ok(true)
        }
        Some(other) if ledger_cli => Err((409, format!(
            "session `{session_id}` runs on the Claude subscription route (claude-cli); start a new session to use `{other}`"
        ))),
        // Bound in its metadata, but no turn ever started there (refused before
        // `session.start`, for example by the billing check or a missing claude.exe): the
        // binding was only provisional, so the session may move to the API route.
        Some(_) if bound && recorded.is_none() => {
            if let Some(mut m) = meta {
                m.route = None;
                if m.default_provider.as_deref() == Some(CLAUDE_CLI_ROUTE) {
                    m.default_provider = None;
                    m.default_model = None;
                }
                state.save_meta(&m).map_err(|e| (500, e))?;
            }
            Ok(false)
        }
        Some(other) if bound => Err((409, format!(
            "session `{session_id}` runs on the Claude subscription route (claude-cli); start a new session to use `{other}`"
        ))),
        Some(_) => Ok(false),
        None => Ok(bound || ledger_cli),
    }
}

/// A model alias or full model name: never something the CLI could read as a flag.
fn pick_model(req: &Value, meta: Option<&SessionMeta>) -> Result<String, String> {
    let model = req
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| meta.filter(|m| meta_bound(Some(m))).and_then(|m| m.default_model.clone()))
        .unwrap_or_else(|| DEFAULT_CLI_MODEL.to_string());
    let ok = !model.starts_with('-')
        && model.len() <= 100
        && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']' | '@'));
    if ok {
        Ok(model)
    } else {
        Err(format!("invalid model `{model}` for the Claude subscription route"))
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

/// A `claude-cli` turn's stop handle, registered in [`DesktopState::cli_turns`] before its
/// thread is spawned (so a stop right after the 202 reaches it); removed when dropped.
pub(crate) struct Registered {
    state: Arc<DesktopState>,
    session: String,
    pub(crate) stop: StopHandle,
}

impl Registered {
    pub(crate) fn new(state: &Arc<DesktopState>, session_id: &str) -> Result<Registered, String> {
        let stop = StopHandle::new();
        state.cli_turns.lock().map_err(|_| "turn registry poisoned".to_string())?.insert(session_id.to_string(), stop.clone());
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

/// Run one desktop turn on the Claude subscription route; returns the final reply text.
pub(crate) fn run_cli_turn(
    state: &Arc<DesktopState>,
    session_id: &str,
    req: &Value,
    message: &str,
    stop: &StopHandle,
) -> Result<String, String> {
    let vendor = ClaudeCode;
    let env_lookup = |n: &str| std::env::var_os(n);
    // Billing safety first: nothing is started while environment credentials would win.
    let allowed_env = cli_route::env_auth::check(vendor.env_auth_vars(), &env_lookup)?;
    if stop.is_stopped() {
        return Err("turn stopped by the user".to_string());
    }
    let claude = vendor.locate(state.cfg.claude_exe.as_deref(), &env_lookup)?;
    let core_exe = resolve_core_exe(&state.cfg)?;
    let meta = state.load_meta(session_id);
    let model = pick_model(req, meta.as_ref())?;
    let research = Arc::new(state.research_env_for_session(session_id)?);

    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    if let Some(route) = route_of_first_start(&existing).filter(|r| r != ROUTE) {
        return Err(format!("session `{session_id}` was started on route `{route}`; it cannot continue on the Claude subscription route"));
    }
    if !existing.is_empty() {
        ledger.verify_session(session_id).map_err(|e| format!("cannot continue session `{session_id}`: {e}"))?;
    }
    let replay = build_prompt(&prior_turns(&existing), message, DEFAULT_HISTORY_BUDGET);

    validate_session_id(session_id).map_err(|e| e.to_string())?;
    let sdir = state.session_dir(session_id)?;
    let (work, scratch, pycache) = (sdir.join("work"), sdir.join("scratch"), sdir.join("pycache"));
    for d in [&work, &scratch, &pycache] {
        std::fs::create_dir_all(d).map_err(|e| format!("could not create {}: {e}", d.display()))?;
    }
    let _session_guard = SessionGuard::create(&state.cfg.data_dir, &sdir, session_id)?;

    let token = random_hex(32);
    let relay = cli_route::relay::start(
        state.open_ledger()?,
        session_id.to_string(),
        token.clone(),
        Arc::new(UiRelay { state: Arc::clone(state) }),
    )
    .map_err(|e| format!("could not start the approval relay: {e}"))?;
    let workspace = research.workspace_root.clone();
    let core = CoreLaunch {
        exe: core_exe,
        env: core_env(&state.cfg, session_id, &token, &relay.addr, &scratch, &pycache, workspace.as_deref()),
    };
    let tools = core_tool_set(Some(Arc::clone(&research)));

    let mut start_fields = json!({
        "prompt": message,
        "provider": CLAUDE_CLI_ROUTE,
        "surface": "desktop",
        "history_replayed": replay.replayed,
        "history_chars": replay.history_chars,
        "history_turns": replay.turns_included,
        "history_turns_dropped": replay.turns_dropped,
        "workspace_root": workspace.as_deref().map(|w| path_str(&strip_verbatim_prefix(w))),
    });
    if !allowed_env.is_empty() {
        start_fields["env_auth_vars_allowed"] = json!(allowed_env);
    }
    let tool_config = sdir.join("mcp.json");
    let spec = TurnSpec {
        session: session_id,
        exe: &claude,
        work_dir: &work,
        tool_config: &tool_config,
        core: &core,
        model: &model,
        prompt: &replay.prompt,
        tool_defs: &tools.defs,
        policy_version: &tools.policy_version,
        start_kind: if existing.is_empty() { "session.start" } else { "session.resume" },
        start_fields,
        inherit_stderr: false,
        allow_env_auth: cli_route::env_auth::opted_in(&env_lookup),
    };
    let rec = Recorder { ledger, session: session_id.to_string() };
    let mut sink = SseEventSink::new(Arc::clone(&state.sse), session_id);
    let outcome = run_turn(&vendor, &rec, &mut sink, &spec, stop);
    drop(relay);
    let report = outcome?;
    match report.end {
        TurnEnd::Completed { exit_code: Some(0), init_verified: true } => {
            let text = sink.final_text().to_string();
            Ok(if text.is_empty() { report.final_text } else { text })
        }
        TurnEnd::Completed { init_verified: false, .. } => {
            Err("Claude Code ended without a verified system/init event; turn aborted".to_string())
        }
        TurnEnd::Completed { exit_code, .. } => Err(format!("Claude Code exited with code {exit_code:?}")),
        TurnEnd::Aborted(reason) => Err(format!("turn aborted: {reason}")),
        TurnEnd::Stopped => Err("turn stopped by the user".to_string()),
    }
}

/// Delete the session's `mcp.json` if no live turn owns it (left behind when re:Search was
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

/// `POST /api/sessions/{id}/stop`: kill the running `claude-cli` turn's process tree and
/// deny its open approval cards. The turn records `session.abort`.
pub fn handle_stop_turn(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| (400, e.to_string()))?;
    let handle = state.cli_turns.lock().map_err(|_| (500, "turn registry poisoned".to_string()))?.get(session_id).cloned();
    match handle {
        Some(h) => {
            h.stop();
            let denied = state.deny_pending(session_id, None, "supervisor: turn stopped by the user");
            Ok((200, json!({ "ok": true, "session": session_id, "stopped": true, "approvals_denied": denied })))
        }
        None => {
            let running = state.active_turns.lock().map(|a| a.contains(session_id)).unwrap_or(false);
            Err((409, if running {
                "only Claude subscription (claude-cli) turns can be stopped".to_string()
            } else {
                format!("no turn is running for `{session_id}`")
            }))
        }
    }
}
