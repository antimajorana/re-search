//! Frozen tool profiles as data (shared package F1, overview section 2.2).
//!
//! One mechanism for "which tools and policy does this session have", shared by session start,
//! resume, the desktop router, the CLI research router, `cli-route`, and the MCP server (`core`).
//! Adding a new mode adds a profile here without mutating `research/1` or `investigation/1`.

use crate::map_context_defs::record_context_cards_def;
use crate::map_defs::{
    read_map_node_def, record_map_reading_def, register_map_tools, INVESTIGATION_V2_MAY_GAIN,
};
use crate::paper_key::Mode;
use crate::{
    all_research_tool_defs, idea_tools, investigation, register_research_tools, research_policy,
    solve_defs, solve_exprs, solve_policy, ResearchEnv,
};
use ledger::Event;
use policy::{EffectClass, Policy};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use tools::{builtin, Registry, ToolDef, ToolHandler, WorkerEpoch};

/// Event payload field name recording the session's tool profile (`00-overview.md` 2.2, 8.2 decision 1).
pub const TOOL_PROFILE_FIELD: &str = "tool_profile";

/// Frozen profile for earlier and current Research sessions (`00-overview.md` 2.2, 8.2 decision 1).
pub const PROFILE_RESEARCH_V1: &str = "research/1";
/// Frozen profile for pre-F1 and current Investigation sessions (`00-overview.md` 2.2, 8.2 decision 1).
pub const PROFILE_INVESTIGATION_V1: &str = "investigation/1";
/// Built profile for Investigate v2 (package V6; `00-overview.md` 2.2, 8.2 decision 1).
pub const PROFILE_INVESTIGATION_V2: &str = "investigation/2";
/// Frozen profile for Explore (`00-overview.md` 2.2, `explore.md` 5.2, 8.2 decision 1, package X7).
pub const PROFILE_EXPLORE_V1: &str = "explore/1";
/// Frozen profile for Solve (`00-overview.md` 2.2, `solve.md` 5.1, 8.2 decision 1, package S7).
pub const PROFILE_SOLVE_V1: &str = "solve/1";

/// Built profiles available in this build, in canonical order.
pub const BUILT_PROFILE_NAMES: &[&str] = &[
    PROFILE_RESEARCH_V1,
    PROFILE_INVESTIGATION_V1,
    PROFILE_INVESTIGATION_V2,
    PROFILE_SOLVE_V1,
    PROFILE_EXPLORE_V1,
];
/// Known future profiles whose tools are not implemented yet.
pub const RESERVED_PROFILE_NAMES: &[&str] = &[];

/// Host-written mode marker events (`00-overview.md` 3.1 row 3, 5.2, 8.2 decision 1).
pub const MARKER_INVESTIGATION_STARTED: &str = "investigation.started";
/// Legacy hand-off child marker: stays Research (`00-overview.md` 1, 3.1 row 8, 5.3).
pub const MARKER_INVESTIGATION_HANDOFF: &str = "investigation.handoff";
/// Host-written Explore mode marker (`00-overview.md` 3.1 row 3, 5.2).
pub const MARKER_IDEA_STARTED: &str = "idea.started";
/// Host-written Solve mode marker (`00-overview.md` 3.1 row 3, 5.2).
pub const MARKER_SOLVE_STARTED: &str = "solve.started";

/// The 7 `ReadWorkspace` tool ids tolerated as additions when resuming pre-F1 sessions
/// (moved into data from `crates/agent/src/resume.rs`).
pub const LEGACY_MAY_GAIN: &[&str] = &[
    "search_formal_library",
    "search_paper_passages",
    "read_paper_lines",
    "record_paper_map",
    "record_paper_summary",
    "record_paper_branch",
    "record_paper_conventions",
];

/// Failure looking up a tool profile by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// A known profile from the three-modes design whose tools have not been built yet.
    NotBuiltYet { name: String },
    /// An unrecognized profile name.
    Unknown { name: String },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBuiltYet { name } => write!(f, "tool profile `{name}`: profile not built yet"),
            Self::Unknown { name } => write!(f, "unknown tool profile `{name}`"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// A frozen tool profile as data (`00-overview.md` 2.2, 3.1 row 12).
#[derive(Debug, Clone)]
pub struct ToolProfile {
    pub name: &'static str,
    /// Tool definitions in id order (`all_research_tool_defs()` plus host tools registered on the router).
    pub defs: Vec<ToolDef>,
    /// Tool ids an earlier session under this profile may gain on resume (must have `ReadWorkspace` effect).
    pub may_gain: Vec<&'static str>,
    /// Prior released `ToolDef` schemas accepted for `id` on resume when the running tool matches
    /// this profile's canonical definition for `id`.
    pub earlier_defs: BTreeMap<String, Vec<ToolDef>>,
    /// Policy belonging to this profile so resume compares the right policy version.
    pub policy: Policy,
}

impl ToolProfile {
    /// Canonical tool definition for `id` in this profile, if present.
    pub fn def_for(&self, id: &str) -> Option<&ToolDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    /// True iff `candidate` matches this profile's canonical definition for its id byte for byte.
    pub fn is_canonical_def(&self, candidate: &ToolDef) -> bool {
        self.def_for(&candidate.id)
            .is_some_and(|canon| canon.def_hash() == candidate.def_hash())
    }

    /// True iff an earlier session under this profile may gain `def` without triggering drift.
    pub fn allows_gain(&self, def: &ToolDef) -> bool {
        self.may_gain.contains(&def.id.as_str()) && def.effect == EffectClass::ReadWorkspace
    }

    /// Prior released definitions accepted for `id` under this profile.
    pub fn earlier_defs_for(&self, id: &str) -> &[ToolDef] {
        self.earlier_defs.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Definitions served by `core.exe` over MCP, which additionally registers `set_session_title`
    /// on the router (in-core loops handle `set_session_title` outside `Router`).
    pub fn mcp_defs(&self) -> Vec<ToolDef> {
        let mut defs = self.defs.clone();
        if !defs.iter().any(|d| d.id == builtin::SET_SESSION_TITLE) {
            defs.push(builtin::session_title_def());
            defs.sort_by(|a, b| a.id.cmp(&b.id));
        }
        defs
    }
}

/// Optional Solve host bindings for `solve/1` registration (check-worker epoch and S8/S9/S11
/// handler drop-in slots). Leaving any handler `None` uses the fail-closed S7 stub handler.
pub struct SolveCheckHost {
    pub check_epoch: Arc<dyn WorkerEpoch>,
    pub check_handler: Option<Arc<dyn ToolHandler>>,
    pub step_record_handler: Option<Arc<dyn ToolHandler>>,
    pub solve_finish_handler: Option<Arc<dyn ToolHandler>>,
}

impl SolveCheckHost {
    pub fn new(check_epoch: Arc<dyn WorkerEpoch>) -> Self {
        Self {
            check_epoch,
            check_handler: None,
            step_record_handler: None,
            solve_finish_handler: None,
        }
    }

    pub fn with_check_handler(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        self.check_handler = Some(handler);
        self
    }

    pub fn with_step_record_handler(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        self.step_record_handler = Some(handler);
        self
    }

    pub fn with_solve_finish_handler(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        self.solve_finish_handler = Some(handler);
        self
    }
}

/// Host bindings supplied by a router builder when registering a profile's tools.
pub struct ProfileHostTools {
    pub sympy_epoch: Arc<dyn WorkerEpoch>,
    pub sympy_handler: Arc<dyn ToolHandler>,
    /// Supplied only by `cli-route` / `mcp-server` (`core.exe`), which exposes `set_session_title`
    /// through MCP. In-core routers leave this `None`.
    pub session_title_handler: Option<Arc<dyn ToolHandler>>,
    /// Optional Solve check-worker epoch and S8/S9/S11 handler bindings for `solve/1`.
    /// When `None`, `register_profile` binds an uncontained fallback epoch and fail-closed S7 stubs.
    pub solve_check: Option<SolveCheckHost>,
}

impl ProfileHostTools {
    pub fn new(sympy_epoch: Arc<dyn WorkerEpoch>, sympy_handler: Arc<dyn ToolHandler>) -> Self {
        Self {
            sympy_epoch,
            sympy_handler,
            session_title_handler: None,
            solve_check: None,
        }
    }

    pub fn with_session_title(mut self, handler: Arc<dyn ToolHandler>) -> Self {
        self.session_title_handler = Some(handler);
        self
    }

    pub fn with_solve_check(mut self, solve_check: SolveCheckHost) -> Self {
        self.solve_check = Some(solve_check);
        self
    }
}

/// Research tool list plus host tools registered on the desktop and CLI research routers, sorted by id.
fn research_v1_defs() -> Vec<ToolDef> {
    let mut defs = all_research_tool_defs();
    defs.push(builtin::ledger_note_def());
    defs.push(builtin::sympy_eval_def());
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
}

/// The two special cases formerly hard-coded in `check_tool_defs_drift`, represented as data:
/// 1. Current `record_paper_map` without the optional `answer_node_ids` property.
/// 2. Pre-pointer (`7c36a0e`) schemas of the four Investigation recorders.
fn legacy_recorder_earlier_defs() -> BTreeMap<String, Vec<ToolDef>> {
    let mut map = BTreeMap::new();

    let mut current_map_no_answer_index = investigation::def();
    if let Some(props) = current_map_no_answer_index.input_schema["properties"].as_object_mut() {
        props.remove("answer_node_ids");
    }
    let mut map_defs = vec![current_map_no_answer_index];
    map_defs.extend(investigation::earlier_defs(investigation::RECORD_PAPER_MAP));
    map.insert(investigation::RECORD_PAPER_MAP.to_string(), map_defs);

    for id in [
        investigation::RECORD_PAPER_SUMMARY,
        investigation::RECORD_PAPER_BRANCH,
        investigation::RECORD_PAPER_CONVENTIONS,
    ] {
        map.insert(id.to_string(), investigation::earlier_defs(id));
    }
    map
}

/// Profile `research/1`: frozen today's research tool set plus host tools (`ledger_note`, `sympy_eval`).
///
/// Retains the 7 historical `ReadWorkspace` `may_gain` ids and 4 recorder `earlier_defs` so pre-F1
/// Research sessions (and pre-pointer hand-off children) keep resuming identically to today.
/// New mode tools (such as Investigate v2's tools) are never added here.
pub fn research_v1() -> ToolProfile {
    ToolProfile {
        name: PROFILE_RESEARCH_V1,
        defs: research_v1_defs(),
        may_gain: LEGACY_MAY_GAIN.to_vec(),
        earlier_defs: legacy_recorder_earlier_defs(),
        policy: research_policy(),
    }
}

/// `may_gain` ids for `investigation/1` and `investigation/2`: the 7 historical `ReadWorkspace`
/// additions plus the Investigate v2 `ReadWorkspace` tools (never added to `research/1`).
fn investigation_may_gain() -> Vec<&'static str> {
    let mut ids = LEGACY_MAY_GAIN.to_vec();
    ids.extend_from_slice(INVESTIGATION_V2_MAY_GAIN);
    ids
}

/// Profile `investigation/1`: same frozen 22-tool list as `research/1`, carrying the 7 historical
/// `may_gain` ids, the Investigate v2 `ReadWorkspace` compatibility `may_gain` ids, and the 2
/// historical `earlier_defs` special cases as data (`00-overview.md` 2.2, `investigate.md` §8.2).
pub fn investigation_v1() -> ToolProfile {
    ToolProfile {
        name: PROFILE_INVESTIGATION_V1,
        defs: research_v1_defs(),
        may_gain: investigation_may_gain(),
        earlier_defs: legacy_recorder_earlier_defs(),
        policy: research_policy(),
    }
}

/// Profile `investigation/2`: `research/1` plus V6 (`read_map_node`, `record_map_reading`) and
/// V12 (`record_context_cards`), all `ReadWorkspace`, sorted by id (`investigate.md` §5.1, `00-overview.md` 2.2).
/// `record_prior_work_check` (deferred V14) remains under `may_gain` so it can join later without
/// breaking `investigation/2` resume.
pub fn investigation_v2() -> ToolProfile {
    let mut defs = research_v1_defs();
    defs.push(read_map_node_def());
    defs.push(record_map_reading_def());
    defs.push(record_context_cards_def());
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    ToolProfile {
        name: PROFILE_INVESTIGATION_V2,
        defs,
        may_gain: investigation_may_gain(),
        earlier_defs: legacy_recorder_earlier_defs(),
        policy: research_policy(),
    }
}

/// Profile `solve/1`: frozen 16-tool Solve tool list (12 reused `research/1` tools +
/// `expr_show`, `step_record`, `check_run`, `solve_finish`) and `solve_policy()` (`v0.5-solve`).
pub fn solve_v1() -> ToolProfile {
    ToolProfile {
        name: PROFILE_SOLVE_V1,
        defs: solve_defs::solve_v1_defs(),
        may_gain: Vec::new(),
        earlier_defs: BTreeMap::new(),
        policy: solve_policy(),
    }
}

/// Profile `explore/1`: frozen 4-tool Explore tool list (`idea_fetch_papers`, `idea_search`,
/// `read_paper_lines`, `search_paper_passages`) and `research_policy()` (`explore.md` 5.2, X7).
pub fn explore_v1() -> ToolProfile {
    ToolProfile {
        name: PROFILE_EXPLORE_V1,
        defs: idea_tools::explore_v1_defs(),
        may_gain: Vec::new(),
        earlier_defs: BTreeMap::new(),
        policy: research_policy(),
    }
}

/// Look up a profile by name.
pub fn lookup(name: &str) -> Result<ToolProfile, ProfileError> {
    match name {
        PROFILE_RESEARCH_V1 => Ok(research_v1()),
        PROFILE_INVESTIGATION_V1 => Ok(investigation_v1()),
        PROFILE_INVESTIGATION_V2 => Ok(investigation_v2()),
        PROFILE_SOLVE_V1 => Ok(solve_v1()),
        PROFILE_EXPLORE_V1 => Ok(explore_v1()),
        _ => Err(ProfileError::Unknown { name: name.to_string() }),
    }
}

/// All profiles built in this binary, in canonical order.
pub fn built_profiles() -> Vec<ToolProfile> {
    vec![research_v1(), investigation_v1(), investigation_v2(), solve_v1(), explore_v1()]
}

/// Detect session mode from the latest host-written mode marker (`00-overview.md` 1, 3.1 row 3).
///
/// Latest of `investigation.started`, `investigation.handoff`, `idea.started`, `solve.started`
/// wins; `investigation.handoff` and no marker both resolve to `Mode::Research`.
pub fn session_mode_from_events(events: &[Event]) -> Mode {
    for ev in events.iter().rev() {
        match ev.kind.as_str() {
            MARKER_INVESTIGATION_STARTED => return Mode::Investigate,
            MARKER_INVESTIGATION_HANDOFF => return Mode::Research,
            MARKER_IDEA_STARTED => return Mode::Explore,
            MARKER_SOLVE_STARTED => return Mode::Solve,
            _ => {}
        }
    }
    Mode::Research
}

/// Profile name implied for a session that carries no explicit `tool_profile` field.
pub fn legacy_profile_name_for_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::Research => PROFILE_RESEARCH_V1,
        Mode::Investigate => PROFILE_INVESTIGATION_V1,
        Mode::Explore => PROFILE_EXPLORE_V1,
        Mode::Solve => PROFILE_SOLVE_V1,
    }
}

/// Profile name implied by a session's mode markers when `session.start` has no `tool_profile`.
pub fn legacy_profile_name_for_events(events: &[Event]) -> &'static str {
    legacy_profile_name_for_mode(session_mode_from_events(events))
}

/// Resolve the [`ToolProfile`] for a session from its ledger events:
/// - If `session.start` carries a `tool_profile` field, it must be a string naming a built profile.
/// - If `session.start` exists without `tool_profile`, falls back to the legacy mode profile
///   (`research/1`, `investigation/1`, or `solve/1`).
/// - If `session.start` resolved to `investigation/1` and a `session.resume` event in `events`
///   recorded `"tool_profile": "investigation/2"`, returns `investigation/2`.
/// - If `session.start` does not exist yet (new session before its first turn), new Investigate
///   sessions start under `investigation/2`; other modes resolve via `legacy_profile_name_for_events`.
pub fn profile_for_session_events(events: &[Event]) -> Result<ToolProfile, String> {
    if let Some(start) = events.iter().find(|e| e.kind == "session.start") {
        let mut profile = if let Some(val) = start.payload.get(TOOL_PROFILE_FIELD) {
            let name = val
                .as_str()
                .ok_or_else(|| format!("invalid non-string `{TOOL_PROFILE_FIELD}` in `session.start`"))?;
            lookup(name).map_err(|e| e.to_string())?
        } else {
            let inferred = legacy_profile_name_for_events(events);
            lookup(inferred).map_err(|e| e.to_string())?
        };
        if profile.name == PROFILE_INVESTIGATION_V1
            && events.iter().any(|e| {
                e.kind == "session.resume"
                    && e.payload.get(TOOL_PROFILE_FIELD).and_then(Value::as_str)
                        == Some(PROFILE_INVESTIGATION_V2)
            })
        {
            profile = investigation_v2();
        }
        return Ok(profile);
    }
    let name = match session_mode_from_events(events) {
        Mode::Investigate => PROFILE_INVESTIGATION_V2,
        other => legacy_profile_name_for_mode(other),
    };
    lookup(name).map_err(|e| e.to_string())
}

/// Resolve the active [`ToolProfile`] for running or resuming `events`: calls
/// [`profile_for_session_events`] and upgrades `investigation/1` to `investigation/2`
/// (`00-overview.md` 2.2, `investigate.md` §8.2), leaving `research/1` (including explicit
/// `"tool_profile": "research/1"` and `investigation.handoff`) and `solve/1` untouched.
pub fn active_profile_for_session_events(events: &[Event]) -> Result<ToolProfile, String> {
    let profile = profile_for_session_events(events)?;
    if profile.name == PROFILE_INVESTIGATION_V1 {
        Ok(investigation_v2())
    } else {
        Ok(profile)
    }
}

/// True iff `actual` and `expected` have the exact same length, ids, and `def_hash` in order.
pub fn defs_match_exact(actual: &[&ToolDef], expected: &[ToolDef]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected.iter())
            .all(|(a, e)| a.id == e.id && a.def_hash() == e.def_hash())
}

/// Register every tool required by `profile` into `registry` and return the profile's policy.
///
/// Verifies fail-closed that the resulting registry definitions match `profile.defs`
/// (or `profile.mcp_defs()` when `host.session_title_handler` is provided) in id and hash order.
pub fn register_profile(
    profile: &ToolProfile,
    registry: &mut Registry,
    env: Arc<ResearchEnv>,
    host: ProfileHostTools,
) -> Result<Policy, String> {
    let has_title = host.session_title_handler.is_some();
    match profile.name {
        PROFILE_RESEARCH_V1 | PROFILE_INVESTIGATION_V1 | PROFILE_INVESTIGATION_V2 => {
            registry.register_worker_session(
                builtin::sympy_eval_def(),
                host.sympy_epoch,
                host.sympy_handler,
            );
            registry.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
            if let Some(title_handler) = host.session_title_handler {
                registry.register(builtin::session_title_def(), title_handler);
            }
            if profile.name == PROFILE_INVESTIGATION_V2 {
                register_map_tools(registry, Arc::clone(&env));
            }
            register_research_tools(registry, env);
        }
        PROFILE_SOLVE_V1 => {
            registry.register_worker_session(
                builtin::sympy_eval_def(),
                host.sympy_epoch,
                host.sympy_handler,
            );
            registry.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
            if let Some(title_handler) = host.session_title_handler {
                registry.register(builtin::session_title_def(), title_handler);
            }
            solve_defs::register_solve_v1_reused_tools(registry, env);
            registry.register(solve_defs::expr_show_def(), solve_exprs::expr_show_handler());

            let (check_epoch, check_handler, step_record_handler, solve_finish_handler) =
                match host.solve_check {
                    Some(sc) => (
                        sc.check_epoch,
                        sc.check_handler.unwrap_or_else(solve_defs::check_run_stub_handler),
                        sc.step_record_handler
                            .unwrap_or_else(solve_defs::step_record_stub_handler),
                        sc.solve_finish_handler
                            .unwrap_or_else(solve_defs::solve_finish_stub_handler),
                    ),
                    None => (
                        Arc::new(solve_defs::FallbackCheckEpoch) as Arc<dyn WorkerEpoch>,
                        solve_defs::check_run_stub_handler(),
                        solve_defs::step_record_stub_handler(),
                        solve_defs::solve_finish_stub_handler(),
                    ),
                };
            registry.register_worker_session(
                solve_defs::check_run_def(),
                check_epoch,
                check_handler,
            );
            registry.register(solve_defs::step_record_def(), step_record_handler);
            registry.register(solve_defs::solve_finish_def(), solve_finish_handler);
        }
        PROFILE_EXPLORE_V1 => {
            if let Some(title_handler) = host.session_title_handler {
                registry.register(builtin::session_title_def(), title_handler);
            }
            idea_tools::register_explore_v1_tools(registry, env, None);
        }
        other => {
            return Err(format!(
                "tool profile `{other}` cannot be registered: profile not built yet"
            ));
        }
    }

    let expected = if has_title {
        profile.mcp_defs()
    } else {
        profile.defs.clone()
    };
    let actual = registry.defs();
    if !defs_match_exact(&actual, &expected) {
        return Err(format!(
            "registered tools do not match profile `{}` (expected {} tools, got {})",
            profile.name,
            expected.len(),
            actual.len()
        ));
    }
    Ok(profile.policy.clone())
}

/// True when `payload` carries an explicit `tool_profile` field (even if malformed).
pub fn has_explicit_profile(payload: &Value) -> bool {
    payload.get(TOOL_PROFILE_FIELD).is_some()
}

#[cfg(test)]
#[path = "tool_profile_tests.rs"]
mod tests;
