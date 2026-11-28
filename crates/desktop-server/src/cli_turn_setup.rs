//! Subscription-CLI turn setup helpers: route detection, model selection,
//! `core.exe` resolution, and launch environment construction.
//!
//! Split out of `cli_turn.rs` (Checkpoint S15a, shared seam with Explore X18)
//! so both files stay well below the 600-line limit.

use crate::state::{DesktopState, SessionMeta, ANTIGRAVITY_ROUTE, CLAUDE_CLI_ROUTE};
use crate::DesktopConfig;
use cli_route::core_tools::CORE_TOOL_PROFILE_ENV;
use cli_route::history::{build_session_prompt, DEFAULT_HISTORY_BUDGET};
use ledger::Ledger;
use research_tools::paper_key::Mode;
use research_tools::solve_model::{CheckOutcome, ReviewVerdict, StepOrigin};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use research_tools::tool_profile::{
    active_profile_for_session_events, session_mode_from_events, ToolProfile,
};
use research_tools::{strip_verbatim_prefix, LiteratureEndpoints};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Model alias used when neither the request nor the session names one.
pub const DEFAULT_CLI_MODEL: &str = "sonnet";

/// Solve check-process environment variable names passed to `core.exe` (`solve.md` 5.1, S15).
pub const CORE_CHECK_SCRATCH_ENV: &str = "CORE_CHECK_SCRATCH";
pub const CORE_CHECK_PYCACHE_ENV: &str = "CORE_CHECK_PYCACHE";
pub const CORE_CHECK_TIMEOUT_ENV: &str = "CORE_CHECK_TIMEOUT_S";
/// Default host auto-check timeout in seconds (`solve.md` 5.1).
pub const DEFAULT_SOLVE_CHECK_TIMEOUT_S: u64 = 10;
/// Fixed Solve resume-brief reminder (`solve.md` 5.5).
pub const SOLVE_HANDLE_SURVIVAL_LINE: &str = "Python variables do not survive; only handles do.";

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

/// Detect the session's mode and active frozen [`ToolProfile`] from its ledger events.
pub fn resolve_cli_turn_profile(events: &[ledger::Event]) -> Result<(Mode, ToolProfile), String> {
    let mode = session_mode_from_events(events);
    if mode == Mode::Explore && !crate::session_mode::DESKTOP_MODE_EXPLORE_CLI_ENABLED {
        return Err(
            "Explore CLI turns are not built yet (awaiting X18; CLI profile not built yet)"
                .to_string(),
        );
    }
    let profile = active_profile_for_session_events(events)?;
    Ok((mode, profile))
}

/// Distinct AppContainer sandbox session id (`1..=43` lowercase ASCII `[a-z0-9._-]`) for the
/// isolated Solve check worker; mirrors `mcp-server/src/solve_wire::check_sandbox_id`.
pub fn check_sandbox_id(session: &str) -> String {
    let lower = session.to_ascii_lowercase();
    let valid = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
    };
    let candidate = format!("{lower}-chk");
    if valid(&candidate) && candidate.len() <= 43 && candidate != lower {
        candidate
    } else {
        format!("chk-{}", &ledger::sha256_hex(session.as_bytes())[..16])
    }
}

/// Mode-aware environment for `core.exe`: always propagates `CORE_TOOL_PROFILE` (shared seam
/// with Explore X18), sets Solve check-worker scratch/pycache/timeout vars for `Mode::Solve`
/// without requiring `CORE_ENABLE_RESEARCH_TOOLS`, and keeps `CORE_ENABLE_RESEARCH_TOOLS=1`
/// for Research/Investigate sessions.
#[allow(clippy::too_many_arguments)]
pub fn core_env_for_profile(
    cfg: &DesktopConfig,
    session: &str,
    token: &str,
    relay_addr: &str,
    scratch: &Path,
    pycache: &Path,
    workspace: Option<&Path>,
    mode: Mode,
    profile_name: &str,
    check_dirs: Option<(&Path, &Path)>,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = [
        ("CORE_SESSION", session.to_string()),
        ("CORE_SESSION_TOKEN", token.to_string()),
        ("CORE_DATA_DIR", path_str(&cfg.data_dir)),
        ("CORE_APPROVAL_ADDR", relay_addr.to_string()),
        ("CORE_SCRATCH", path_str(scratch)),
        ("CORE_PYCACHE", path_str(pycache)),
        (CORE_TOOL_PROFILE_ENV, profile_name.to_string()),
        ("CORE_EVAL_TIMEOUT_S", cfg.eval_timeout_s.max(30).to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    if mode != Mode::Solve {
        env.insert("CORE_ENABLE_RESEARCH_TOOLS".into(), "1".into());
    } else {
        env.insert(CORE_CHECK_TIMEOUT_ENV.into(), DEFAULT_SOLVE_CHECK_TIMEOUT_S.to_string());
        if let Some((check_scratch, check_pycache)) = check_dirs {
            env.insert(CORE_CHECK_SCRATCH_ENV.into(), path_str(check_scratch));
            env.insert(CORE_CHECK_PYCACHE_ENV.into(), path_str(check_pycache));
        }
    }
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
        ("RE_SEARCH_OPENALEX_BASE", &ep.openalex_api_base, &def.openalex_api_base),
    ] {
        if val != default {
            env.insert(name.into(), val.clone());
        }
    }
    env
}

/// Constructed CLI prompt and history-replay metadata recorded on `session.start`/`session.resume`.
#[derive(Debug, Clone)]
pub struct CliPromptBuild {
    pub prompt: String,
    pub replayed: bool,
    pub history_chars: usize,
    pub turns_included: usize,
    pub turns_dropped: usize,
}

/// Build the mode-specific prompt sent to a subscription CLI on stdin.
///
/// Research `search_formal_library` and `sympy_eval` instructions are strictly gated to
/// `Mode::Research` so Solve/Investigate/Explore turns never receive Research or Lean guidance.
#[allow(clippy::too_many_arguments)]
pub fn build_cli_prompt(
    mode: Mode,
    existing: &[ledger::Event],
    ledger: Option<&Ledger>,
    eff_msg: &str,
    conventions: Option<&str>,
    project_context: Option<&str>,
    needs_title: bool,
    has_python: bool,
) -> CliPromptBuild {
    if mode == Mode::Solve {
        let prompt = build_solve_cli_prompt(
            existing,
            ledger,
            eff_msg,
            conventions,
            project_context,
            needs_title,
        );
        return CliPromptBuild {
            prompt,
            replayed: false,
            history_chars: 0,
            turns_included: 0,
            turns_dropped: 0,
        };
    }

    let mut msg = eff_msg.to_string();
    if needs_title {
        msg = format!(
            "Once you can state the main research question, use set_session_title once with a concise title in the user's language.\n\n{msg}"
        );
    }
    if let Some(text) = conventions.map(str::trim).filter(|t| !t.is_empty()) {
        msg = format!("Pinned research conventions\n{text}\n\n{msg}");
    }
    if mode == Mode::Research {
        msg = format!(
            "Use `search_paper_passages` to locate exact passages in ingested papers and \
             `search_formal_library` to discover Lean declarations in the attached workspace. \
             Cite recorded source locations, and type-check any intended formal application.\n\n{msg}"
        );
        if has_python {
            msg = format!(
                "The re:Search compute worker exposes `check_symbolic_identity(lhs, rhs, variable=x, domain=...)` \
                 and `check_convergence(run, levels, axis=..., atol=..., rtol=..., reference=...)` in `sympy_eval`. \
                 Return the report as the final Python expression and record its `tool_call_ref` with the proof entry. \
                 Report excluded points, counterexamples, and the scope of numerical evidence. \n\n{msg}"
            );
        }
    }
    if let Some(context) = project_context {
        msg = format!("{context}\n\nCurrent user request\n{msg}");
    }
    let replay = build_session_prompt(existing, &msg, DEFAULT_HISTORY_BUDGET);
    CliPromptBuild {
        prompt: replay.prompt,
        replayed: replay.replayed,
        history_chars: replay.history_chars,
        turns_included: replay.turns_included,
        turns_dropped: replay.turns_dropped,
    }
}

/// Pure deterministic Solve CLI prompt synthesized from `SolveState` (setup + active steps +
/// open items + handles + frozen `lessons.supplied` notes), never replaying Research transcripts.
///
/// Merge seam (S12): once `agent::resume_brief::build_resume_brief` lands, the state summary
/// section below can delegate directly to it.
pub fn build_solve_cli_prompt(
    events: &[ledger::Event],
    ledger: Option<&Ledger>,
    user_message: &str,
    conventions: Option<&str>,
    project_context: Option<&str>,
    needs_title: bool,
) -> String {
    let state = SolveState::from_events(events);
    let mut out = String::new();
    out.push_str(
        "Solve mode (`solve/1`): use `sympy_eval` (`save_expr`/`expr`), `expr_show`, `step_record`, \
         `check_run`, `solve_finish`, `ledger_note`, `ledger_query`, and `ledger_supersede`. \
         Formulas come from saved handles; host auto-checks run after `step_record`. \
         If a tool returns `GATE <id>: ...`, follow the gate instruction (record via `step_record` \
         or discard via `ledger_note` starting with `discard:`). Call `solve_finish` when done.\n",
    );
    if needs_title {
        out.push_str("Use `set_session_title` once with a concise title in the user's language.\n");
    }
    if let Some(ctx) = project_context.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = writeln!(out, "\nProject context\n{ctx}");
    }

    if let Some(setup) = &state.setup {
        let _ = writeln!(out, "\n## Solve setup (rev {}, kind: {:?})", setup.revision, setup.task_kind);
        let _ = writeln!(out, "Statement: {}", setup.statement);
        if !setup.target.description.trim().is_empty() {
            let _ = writeln!(out, "Target ({:?}): {}", setup.target.form, setup.target.description);
        }
        if !setup.conventions.trim().is_empty() {
            let _ = writeln!(out, "Conventions: {}", setup.conventions);
        }
        if !setup.symbols.is_empty() {
            let syms: Vec<String> = setup
                .symbols
                .iter()
                .map(|s| {
                    if s.assumptions.is_empty() {
                        s.name.clone()
                    } else {
                        format!("{} ({})", s.name, s.assumptions.join(", "))
                    }
                })
                .collect();
            let _ = writeln!(out, "Symbols: {}", syms.join("; "));
        }
        if !setup.done.is_empty() {
            let ids: Vec<&str> = setup.done.iter().map(|c| c.id.as_str()).collect();
            let _ = writeln!(out, "Done criteria: {}", ids.join(", "));
        }
    } else if let Some(conv) = conventions.map(str::trim).filter(|s| !s.is_empty()) {
        let _ = writeln!(out, "\nPinned conventions\n{conv}");
    }

    let active = state.active_entries();
    if !active.is_empty() {
        out.push_str("\n## Active steps\n");
        for e in &active {
            let tier = state.tier(&e.entry_id);
            let own = tier.map_or("Unchecked", |t| t.own.label());
            let eff = tier.map_or("Unchecked", |t| t.effective.label());
            let deps = if e.depends_on.is_empty() { "-".to_string() } else { e.depends_on.join(",") };
            let _ = writeln!(out, "- {} [{}] own={own} eff={eff} deps=[{deps}]: {}", e.entry_id, e.entry_type.as_str(), e.summary);
        }
    }

    let mut open_items: Vec<String> = Vec::new();
    for chk in &state.checks {
        if chk.outcome == CheckOutcome::Fail {
            let eid = chk.entry_id.as_deref().unwrap_or("unknown");
            open_items.push(format!("failed check `{}` on `{eid}`: {}", chk.name, chk.evidence));
        }
    }
    for rev in &state.reviews {
        if rev.verdict == ReviewVerdict::Rejected {
            let detail = serde_json::to_string(&rev.issues).unwrap_or_default();
            open_items.push(format!("rejected review on `{}`: {detail}", rev.target_entry_id));
        }
    }
    for e in &active {
        if state.tier(&e.entry_id).is_some_and(|t| t.stale || t.effective == Tier::Stale) {
            open_items.push(format!("stale step `{}` needs re-check", e.entry_id));
        }
        if e.origin == Some(StepOrigin::Solver)
            && state.tier(&e.entry_id).is_some_and(|t| t.own == Tier::AssumedByModel)
        {
            open_items.push(format!("unacknowledged model assumption `{}`", e.entry_id));
        }
    }
    if !state.pending_runs.is_empty() {
        open_items.push(format!("uninterpreted sympy_eval runs: {}", state.pending_runs.join(", ")));
    }
    if !open_items.is_empty() {
        out.push_str("\n## Open items\n");
        for item in &open_items {
            let _ = writeln!(out, "- {item}");
        }
    }

    if !state.handles.is_empty() {
        out.push_str("\n## Handles\n");
        for (h, expr) in &state.handles {
            if expr.portable {
                let _ = writeln!(out, "- {h}: {}", expr.str_preview);
            } else {
                let _ = writeln!(out, "- {h} (non-portable)");
            }
        }
    }

    // Rule R24: read frozen `lessons.supplied` events from the ledger only (never live lessons.db).
    if let Some(l) = ledger {
        let mut supplied_blocks = Vec::new();
        for ev in events.iter().filter(|e| e.kind == "lessons.supplied") {
            if let Some(blob_hash) = ev.payload.get("blob").and_then(Value::as_str) {
                if let Ok(bytes) = l.get_blob(blob_hash) {
                    if let Ok(text) = String::from_utf8(bytes) {
                        if !text.trim().is_empty() {
                            supplied_blocks.push(text);
                        }
                    }
                }
            }
        }
        if !supplied_blocks.is_empty() {
            out.push_str("\n## Notes from the user's earlier corrections\n");
            for block in supplied_blocks {
                let _ = writeln!(out, "> {}", block.replace('\n', "\n> "));
            }
        }
    }

    let _ = writeln!(out, "\n{SOLVE_HANDLE_SURVIVAL_LINE}");
    if !user_message.trim().is_empty() {
        let _ = write!(out, "\n## Current request\n{user_message}");
    }
    out
}
