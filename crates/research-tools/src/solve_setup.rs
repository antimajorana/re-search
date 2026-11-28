//! Deterministic Solve setup validation and zero-model prefill (package S4).
//!
//! Pure code: zero I/O, zero network, zero model calls. Reusable both by
//! `desktop-server` (`POST /api/sessions/{id}/solve/prefill` and `/solve/setup`)
//! and by F7 hand-off packet consumers (`00-overview.md` 4.3).

use crate::lessons_select::SuggestedSolveDefaults;
use crate::paper_key::NodeRef;
use crate::solve_model::{
    compute_math_sha256, event_kinds, validate_expr_name, DoneCriterion, DoneCriterionKind,
    GivenSource, ReviewPolicy, RoleModel, SetupAssumption, SolveBudgets, SolveRoles, SolveSetup,
    SolveTarget, SymbolSpec, TaskKind, MAX_CONVENTIONS_CHARS, MAX_STATEMENT_CHARS,
};
pub use crate::solve_setup_prefill::{
    default_done_criteria, detect_show_vanishes_cue, guess_task_kind, is_same_model,
    prefill_solve_setup, scan_statement_symbols,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Explicit prose-proof notice shown on the setup card when `task_kind == prove`
/// (owner decision `00-overview.md` 8.3 question 9; `solve.md` D21).
pub const PROVE_PROSE_NOTICE: &str =
    "a proof in prose is reviewed by a model; only calculations are computed";

/// Same-model verifier warning (`solve.md` section 2 & D2).
pub const SAME_MODEL_WARNING: &str = "Same model: reviews will be labelled as weaker";

/// Named criterion tier value for exact symbolic checks (`solve.md` D1, D4).
pub const CRITERION_TIER_VERIFIED: &str = "verified";
/// Named criterion tier value for numeric checks (`solve.md` D1, D4; label is `"Numerically checked"`).
pub const CRITERION_TIER_NUMERIC: &str = "checked_numerically";
/// Named criterion tier value for model-agreed reviews (`solve.md` D4, D21).
pub const CRITERION_TIER_MODEL_AGREED: &str = "model_agreed";

pub const MAX_SYMBOLS: usize = 128;
pub const MAX_ASSUMPTIONS: usize = 128;
pub const MAX_DONE_CRITERIA: usize = 32;
pub const MAX_SOURCES: usize = 64;
pub const MAX_BUDGET_MODEL_CALLS: u32 = 500;
pub const MAX_BUDGET_WALL_MINUTES: u32 = 240;
pub const MAX_BUDGET_REVIEW_LOOPS: u32 = 32;
pub const MAX_BUDGET_GATE_BOUNCES: u32 = 20;

const ALLOWED_CRITERION_TIERS: &[&str] = &[
    CRITERION_TIER_VERIFIED,
    CRITERION_TIER_NUMERIC,
    CRITERION_TIER_MODEL_AGREED,
    "checked_by_you",
    "model_agreed_same_model",
];

/// Optional user-corrected defaults plugged in from F16a (`LessonsStore`) without coupling S4 to SQLite.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SetupDefaults {
    #[serde(default)]
    pub conventions: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<SymbolSpec>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, BTreeMap<String, Value>>,
}

impl SetupDefaults {
    /// Convert F16a [`SuggestedSolveDefaults`] into [`SetupDefaults`].
    pub fn from_suggested_lessons(suggested: &SuggestedSolveDefaults) -> Self {
        let conventions = suggested
            .conventions
            .iter()
            .filter_map(|c| {
                let text = if c.key.trim().is_empty() {
                    c.value.trim().to_string()
                } else {
                    format!("{}: {}", c.key.trim(), c.value.trim())
                };
                (!text.trim().is_empty()).then_some(text)
            })
            .collect();
        let mut symbols = Vec::new();
        let mut dimensions = BTreeMap::new();
        for s in &suggested.symbols {
            let name = s.symbol.trim();
            if validate_expr_name(name).is_err() {
                continue;
            }
            if !s.assumptions.is_empty() {
                symbols.push(SymbolSpec {
                    name: name.to_string(),
                    assumptions: s.assumptions.clone(),
                });
            }
            if let Some(dim) = s.dimensions.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                if let Some(parsed) = crate::solve_setup_prefill::parse_compact_dimension(dim) {
                    dimensions.insert(name.to_string(), parsed);
                }
            }
        }
        Self { conventions, symbols, dimensions }
    }
}

/// Request payload accepted by `POST /api/sessions/{id}/solve/setup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveSetupInput {
    pub statement: String,
    pub task_kind: TaskKind,
    pub target: SolveTarget,
    #[serde(default)]
    pub done: Vec<DoneCriterion>,
    #[serde(default)]
    pub conventions: String,
    #[serde(default)]
    pub symbols: Vec<SymbolSpec>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(default)]
    pub assumptions: Vec<SetupAssumption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_source: Option<GivenSource>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default = "empty_solve_roles")]
    pub roles: SolveRoles,
    #[serde(default)]
    pub budgets: SolveBudgets,
    #[serde(default)]
    pub review: ReviewPolicy,
    #[serde(default = "default_tool_profile")]
    pub tool_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<NodeRef>,
    /// Opt-in switch (`solve.md` D20: false by default).
    #[serde(default)]
    pub ask_before_running: bool,
}

fn empty_solve_roles() -> SolveRoles {
    SolveRoles {
        solver: RoleModel { provider: String::new(), model: String::new() },
        verifier: RoleModel { provider: String::new(), model: String::new() },
        compute: None,
    }
}

fn default_tool_profile() -> String {
    event_kinds::SOLVE_TOOL_PROFILE_V1.to_string()
}

/// Request payload for deterministic prefill (`POST /api/sessions/{id}/solve/prefill`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrefillRequest {
    #[serde(default)]
    pub statement: String,
    #[serde(default)]
    pub pinned_conventions: String,
    #[serde(default)]
    pub defaults: Option<SetupDefaults>,
    #[serde(default)]
    pub solver: Option<RoleModel>,
    #[serde(default)]
    pub verifier: Option<RoleModel>,
    #[serde(default)]
    pub task_kind_override: Option<TaskKind>,
    #[serde(default)]
    pub allow_numeric_target: bool,
}

/// Detected `"show <expr> = 0 at order <var>^<n>"` cue for `vanishes_to_order`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VanishesCue {
    pub expression: String,
    pub order_variable: String,
    pub order_power: i32,
}

/// Output of deterministic prefill (zero model calls, `< 1 s`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolvePrefillResult {
    pub task_kind: TaskKind,
    pub target: SolveTarget,
    pub symbols: Vec<SymbolSpec>,
    pub conventions: String,
    pub dimensions: BTreeMap<String, BTreeMap<String, Value>>,
    pub done: Vec<DoneCriterion>,
    pub budgets: SolveBudgets,
    pub review: ReviewPolicy,
    pub tool_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prove_notice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_model_warning: Option<String>,
    pub same_model: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vanishes_cue: Option<VanishesCue>,
    pub math_sha256: String,
    /// Display labels aligned with S2 (`solve_badge::wording`) for UI criterion toggles.
    pub target_tier_labels: BTreeMap<String, String>,
}

fn has_disallowed_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn validate_dimension_power(val: &Value) -> Result<(), String> {
    match val {
        Value::Number(n) => {
            if n.as_i64().is_some()
                || n.as_u64().is_some()
                || n.as_f64().is_some_and(f64::is_finite)
            {
                Ok(())
            } else {
                Err("dimension power must be a finite number".into())
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if let Some((num, den)) = trimmed.split_once('/') {
                let n: i64 = num
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid rational dimension `{trimmed}`"))?;
                let d: i64 = den
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid rational dimension `{trimmed}`"))?;
                if d <= 0 || n.abs() > 100 || d > 100 {
                    return Err(format!("invalid rational dimension `{trimmed}`"));
                }
                Ok(())
            } else {
                let n: f64 =
                    trimmed.parse().map_err(|_| format!("invalid dimension power `{trimmed}`"))?;
                if !n.is_finite() {
                    return Err(format!("invalid dimension power `{trimmed}`"));
                }
                Ok(())
            }
        }
        _ => Err("dimension power must be a number or rational string".into()),
    }
}

fn validate_role(label: &str, role: &RoleModel) -> Result<RoleModel, String> {
    let provider = role.provider.trim().to_string();
    let model = role.model.trim().to_string();
    if provider.chars().count() > 200 || model.chars().count() > 200 {
        return Err(format!("role `{label}` provider/model exceeds 200 characters"));
    }
    if has_disallowed_control(&provider) || has_disallowed_control(&model) {
        return Err(format!("role `{label}` contains control characters"));
    }
    Ok(RoleModel { provider, model })
}

fn validate_done_criterion(c: &DoneCriterion) -> Result<(), String> {
    let id = c.id.trim();
    if id.is_empty() || id.chars().count() > 64 || has_disallowed_control(id) {
        return Err(format!("invalid done criterion id `{id}`"));
    }
    match c.kind {
        DoneCriterionKind::FinalTierAtLeast | DoneCriterionKind::ClosureTierAtLeast => {
            let tier = c
                .params
                .get("tier")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("criterion `{id}` requires `params.tier`"))?;
            if !ALLOWED_CRITERION_TIERS.contains(&tier) {
                return Err(format!("criterion `{id}` has unsupported tier `{tier}`"));
            }
            if let Some(v) = c.params.get("require_different_model") {
                if !v.is_boolean() {
                    return Err(format!(
                        "criterion `{id}` `params.require_different_model` must be a boolean"
                    ));
                }
            }
        }
        DoneCriterionKind::NamedCheckPassed => {
            let check = c
                .params
                .get("check")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("criterion `{id}` requires `params.check`"))?;
            if check.is_empty()
                || check.len() > 64
                || !check.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                || !check.as_bytes()[0].is_ascii_lowercase()
            {
                return Err(format!("criterion `{id}` has invalid check name `{check}`"));
            }
            if let Some(on) = c.params.get("on").and_then(Value::as_str) {
                if !matches!(on, "final" | "all_results") {
                    return Err(format!("criterion `{id}` has invalid `on` target `{on}`"));
                }
            }
        }
        DoneCriterionKind::NoOpenFlags => {
            if !c.params.is_object() && !c.params.is_null() {
                return Err(format!("criterion `{id}` params must be an object"));
            }
        }
        DoneCriterionKind::UserConfirms => {
            let text = c
                .params
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if text.is_empty() || text.chars().count() > 400 || has_disallowed_control(text) {
                return Err(format!("criterion `{id}` requires `params.text` (1..=400 chars)"));
            }
        }
    }
    Ok(())
}

/// Validate [`SolveSetupInput`] and construct a canonical [`SolveSetup`] with incremented
/// `revision` and computed `math_sha256`.
pub fn validate_and_build_setup(
    input: &SolveSetupInput,
    previous_revision: u32,
) -> Result<SolveSetup, String> {
    let statement = input.statement.trim().to_string();
    let stmt_chars = statement.chars().count();
    if stmt_chars == 0 {
        return Err("statement must not be empty".into());
    }
    if stmt_chars > MAX_STATEMENT_CHARS {
        return Err(format!(
            "statement exceeds {MAX_STATEMENT_CHARS} characters (got {stmt_chars})"
        ));
    }
    if has_disallowed_control(&statement) {
        return Err("statement contains disallowed control characters".into());
    }

    let conventions = input.conventions.trim().to_string();
    let conv_chars = conventions.chars().count();
    if conv_chars > MAX_CONVENTIONS_CHARS {
        return Err(format!(
            "conventions exceeds {MAX_CONVENTIONS_CHARS} characters (got {conv_chars})"
        ));
    }
    if has_disallowed_control(&conventions) {
        return Err("conventions contains disallowed control characters".into());
    }
    if input.tool_profile != event_kinds::SOLVE_TOOL_PROFILE_V1 {
        return Err(format!(
            "unsupported tool_profile `{}` (expected `{}`)",
            input.tool_profile,
            event_kinds::SOLVE_TOOL_PROFILE_V1
        ));
    }

    let target_desc = input.target.description.trim().to_string();
    if target_desc.chars().count() > 1_000 || has_disallowed_control(&target_desc) {
        return Err("target description is invalid or exceeds 1,000 characters".into());
    }
    if let Some(digits) = input.target.digits {
        if digits == 0 || digits > 1_000 {
            return Err(format!("target digits must be in 1..=1000 (got {digits})"));
        }
    }

    let symbols = validate_symbols(&input.symbols)?;
    let dimensions = validate_dimensions(&input.dimensions)?;
    let assumptions = validate_assumptions(&input.assumptions)?;
    let done = validate_done_list(&input.done)?;
    validate_budgets(input.budgets)?;
    let (given_source, sources) = validate_sources(input.given_source.as_ref(), &input.sources)?;

    let roles = SolveRoles {
        solver: validate_role("solver", &input.roles.solver)?,
        verifier: validate_role("verifier", &input.roles.verifier)?,
        compute: match &input.roles.compute {
            Some(c) => Some(validate_role("compute", c)?),
            None => None,
        },
    };
    let revision = previous_revision
        .checked_add(1)
        .ok_or_else(|| "setup revision overflow".to_string())?;
    let math_sha256 = compute_math_sha256(&symbols, &dimensions, &assumptions, &conventions);

    Ok(SolveSetup {
        revision,
        statement,
        task_kind: input.task_kind,
        target: SolveTarget {
            description: target_desc,
            form: input.target.form,
            order: input.target.order.clone(),
            digits: input.target.digits,
        },
        done,
        conventions,
        symbols,
        dimensions,
        assumptions,
        given_source,
        sources,
        roles,
        budgets: input.budgets,
        review: input.review,
        tool_profile: input.tool_profile.clone(),
        origin: input.origin.clone(),
        math_sha256,
    })
}

fn validate_symbols(raw: &[SymbolSpec]) -> Result<Vec<SymbolSpec>, String> {
    if raw.len() > MAX_SYMBOLS {
        return Err(format!("too many symbols (max {MAX_SYMBOLS})"));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for sym in raw {
        let name = sym.name.trim();
        validate_expr_name(name)?;
        if !seen.insert(name.to_string()) {
            return Err(format!("duplicate symbol `{name}`"));
        }
        let mut clean = Vec::new();
        for a in &sym.assumptions {
            let trimmed = a.trim();
            if trimmed.is_empty() || trimmed.chars().count() > 64 || has_disallowed_control(trimmed) {
                return Err(format!("invalid assumption `{trimmed}` on symbol `{name}`"));
            }
            if !clean.iter().any(|x| x == trimmed) {
                clean.push(trimmed.to_string());
            }
        }
        out.push(SymbolSpec { name: name.to_string(), assumptions: clean });
    }
    Ok(out)
}

fn validate_dimensions(
    raw: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<BTreeMap<String, BTreeMap<String, Value>>, String> {
    if raw.len() > MAX_SYMBOLS {
        return Err(format!("too many dimension entries (max {MAX_SYMBOLS})"));
    }
    let mut out = BTreeMap::new();
    for (sym, dims) in raw {
        let sym_name = sym.trim();
        validate_expr_name(sym_name)?;
        if out.contains_key(sym_name) {
            return Err(format!("duplicate dimension symbol `{sym_name}`"));
        }
        if dims.is_empty() || dims.len() > 16 {
            return Err(format!("dimensions for `{sym_name}` must have 1..=16 base units"));
        }
        let mut clean_bases = BTreeMap::new();
        for (base, power) in dims {
            let b = base.trim();
            if b.is_empty()
                || b.len() > 32
                || !b.as_bytes()[0].is_ascii_alphabetic()
                || !b.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
            {
                return Err(format!("invalid dimension base `{b}` for `{sym_name}`"));
            }
            if clean_bases.contains_key(b) {
                return Err(format!("duplicate dimension base `{b}` for `{sym_name}`"));
            }
            validate_dimension_power(power)?;
            clean_bases.insert(b.to_string(), power.clone());
        }
        out.insert(sym_name.to_string(), clean_bases);
    }
    Ok(out)
}

fn validate_assumptions(raw: &[SetupAssumption]) -> Result<Vec<SetupAssumption>, String> {
    if raw.len() > MAX_ASSUMPTIONS {
        return Err(format!("too many assumptions (max {MAX_ASSUMPTIONS})"));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for a in raw {
        let id = a.id.trim();
        let text = a.text.trim();
        if id.is_empty() || id.chars().count() > 64 || has_disallowed_control(id) {
            return Err(format!("invalid assumption id `{id}`"));
        }
        if !seen.insert(id.to_string()) {
            return Err(format!("duplicate assumption id `{id}`"));
        }
        if text.is_empty() || text.chars().count() > 2_000 || has_disallowed_control(text) {
            return Err(format!("invalid assumption text for `{id}`"));
        }
        out.push(SetupAssumption { id: id.to_string(), text: text.to_string() });
    }
    Ok(out)
}

fn validate_done_list(raw: &[DoneCriterion]) -> Result<Vec<DoneCriterion>, String> {
    if raw.len() > MAX_DONE_CRITERIA {
        return Err(format!("too many done criteria (max {MAX_DONE_CRITERIA})"));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for c in raw {
        validate_done_criterion(c)?;
        let clean_id = c.id.trim().to_string();
        if !seen.insert(clean_id.clone()) {
            return Err(format!("duplicate done criterion id `{clean_id}`"));
        }
        out.push(DoneCriterion { id: clean_id, kind: c.kind, params: c.params.clone() });
    }
    Ok(out)
}

fn validate_budgets(b: SolveBudgets) -> Result<(), String> {
    if b.max_model_calls == 0 || b.max_model_calls > MAX_BUDGET_MODEL_CALLS {
        return Err(format!("max_model_calls must be in 1..={MAX_BUDGET_MODEL_CALLS}"));
    }
    if b.max_wall_minutes == 0 || b.max_wall_minutes > MAX_BUDGET_WALL_MINUTES {
        return Err(format!("max_wall_minutes must be in 1..={MAX_BUDGET_WALL_MINUTES}"));
    }
    if b.max_model_calls_per_leg == 0 || b.max_model_calls_per_leg > b.max_model_calls {
        return Err("max_model_calls_per_leg must be in 1..=max_model_calls".into());
    }
    if b.max_review_loops > MAX_BUDGET_REVIEW_LOOPS {
        return Err(format!("max_review_loops must be <= {MAX_BUDGET_REVIEW_LOOPS}"));
    }
    if b.max_gate_bounces == 0 || b.max_gate_bounces > MAX_BUDGET_GATE_BOUNCES {
        return Err(format!("max_gate_bounces must be in 1..={MAX_BUDGET_GATE_BOUNCES}"));
    }
    Ok(())
}

fn validate_sources(
    given: Option<&GivenSource>,
    sources: &[String],
) -> Result<(Option<GivenSource>, Vec<String>), String> {
    let clean_given = if let Some(gs) = given {
        let sid = gs.source_id.trim();
        if sid.is_empty() || sid.len() > 128 || has_disallowed_control(sid) {
            return Err("given_source.source_id is invalid".into());
        }
        if !is_sha256_hex(&gs.file_sha256) {
            return Err("given_source.file_sha256 must be a 64-character hex digest".into());
        }
        Some(GivenSource { source_id: sid.to_string(), file_sha256: gs.file_sha256.clone() })
    } else {
        None
    };
    if sources.len() > MAX_SOURCES {
        return Err(format!("too many sources (max {MAX_SOURCES})"));
    }
    let mut clean_sources = Vec::with_capacity(sources.len());
    for src in sources {
        let trimmed = src.trim();
        if trimmed.is_empty() || trimmed.len() > 128 || has_disallowed_control(trimmed) {
            return Err(format!("invalid source id `{src}`"));
        }
        clean_sources.push(trimmed.to_string());
    }
    Ok((clean_given, clean_sources))
}

#[cfg(test)]
#[path = "solve_setup_tests.rs"]
mod tests;
