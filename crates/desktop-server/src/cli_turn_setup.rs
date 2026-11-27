//! Subscription-CLI turn setup helpers: route detection, model selection,
//! `core.exe` resolution, and launch environment construction.
//!
//! Split out of `cli_turn.rs` (Checkpoint S15a, shared seam with Explore X18)
//! so both files stay well below the 600-line limit.

use crate::state::{DesktopState, SessionMeta, ANTIGRAVITY_ROUTE, CLAUDE_CLI_ROUTE};
use crate::DesktopConfig;
use research_tools::{strip_verbatim_prefix, LiteratureEndpoints};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Antigravity => "Antigravity",
        }
    }

    pub(crate) fn cli_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Antigravity => "The Antigravity CLI",
        }
    }
}

pub(crate) fn route_of_first_start(events: &[ledger::Event]) -> Option<String> {
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
pub(crate) fn meta_kind(meta: Option<&SessionMeta>) -> Option<CliRouteKind> {
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
        return (
            start.payload.get("route").cloned().unwrap_or(Value::Null),
            start.payload.get("provenance").cloned().unwrap_or(Value::Null),
        );
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
pub(crate) fn pick_model(
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

pub(crate) fn resolve_core_exe(cfg: &DesktopConfig) -> Result<PathBuf, String> {
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
    let beside = exe
        .parent()
        .ok_or("executable has no directory")?
        .join(format!("core{}", std::env::consts::EXE_SUFFIX));
    check(beside, "beside the running executable; pass --core-exe or set RE_SEARCH_CORE_EXE")
}

pub(crate) fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Environment for `core.exe`: session, token, relay, data, worker, workspace and research
/// tools (always on for desktop sessions, as on the API route).
pub(crate) fn core_env(
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
