//! Per-role agent model assignments (`agent_models`), validation, and resolution helpers.

use crate::cli_turn::CliRouteKind;
use crate::connections::{own_key, API_PROVIDERS};
use crate::state::{DesktopState, SessionMeta, UiSettings};
use crate::subagents::SubagentDef;
use agent::{ProviderKeyStore, ProviderKind, Transport};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentModelSpec {
    pub provider: String,
    pub model: String,
}

pub const AGENT_ROLE_KEYS: &[&str] = &[
    "orchestrator",
    "compute_worker",
    "literature_worker",
    "lean_worker",
    "explorer_worker",
    "paper_reader",
    "verifier",
    "step_verifier",
    "full_verifier",
    "strategist",
    "narrator",
    "explain",
    "subagent_builder",
];

pub struct BuiltWorkerTransport {
    pub transport: Box<dyn Transport>,
    pub provider: String,
    pub model: String,
    pub routing_reason: String,
}

/// Message when Proof Narrator, Explain, or another helper agent has no in-core provider.
pub const NEEDS_API_PROVIDER: &str = "Proof Narrator and Explain need an API-key provider or Sign in with ChatGPT \
     (Anthropic, OpenAI, Sign in with ChatGPT, Gemini, or Ollama); they cannot run on the subscription-CLI routes \
     (claude-cli, antigravity). Connect a provider under Settings -> Providers, or assign a model under Settings -> Agents & Models.";

fn validate_model_id(raw: &str, role: &str) -> Result<String, (u16, String)> {
    let m = raw.trim();
    if m.is_empty()
        || m.len() > 160
        || m.starts_with('-')
        || m.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err((
            400,
            format!("invalid model `{m}` for `agent_models.{role}`"),
        ));
    }
    Ok(m.to_string())
}

pub fn parse_agent_models_patch(
    val: &Value,
    existing: &mut BTreeMap<String, AgentModelSpec>,
) -> Result<(), (u16, String)> {
    let obj = val
        .as_object()
        .ok_or_else(|| (400, "`agent_models` must be a JSON object".to_string()))?;

    for (k, v) in obj {
        if !AGENT_ROLE_KEYS.contains(&k.as_str()) {
            return Err((
                400,
                format!(
                    "unknown agent role `{k}` (expected one of: {})",
                    AGENT_ROLE_KEYS.join(", ")
                ),
            ));
        }
        if v.is_null() {
            existing.remove(k);
            continue;
        }
        let spec = v.as_object().ok_or_else(|| {
            (
                400,
                format!("`agent_models.{k}` must be an object {{ provider, model }} or null"),
            )
        })?;
        if let Some(pv) = spec.get("provider").filter(|pv| !pv.is_null()) {
            if pv.as_str().is_none() {
                return Err((400, format!("`agent_models.{k}.provider` must be a string")));
            }
        }
        if let Some(mv) = spec.get("model").filter(|mv| !mv.is_null()) {
            if mv.as_str().is_none() {
                return Err((400, format!("`agent_models.{k}.model` must be a string")));
            }
        }
        let p_raw = spec.get("provider").and_then(Value::as_str).map(str::trim).unwrap_or("");
        let m_raw = spec.get("model").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if p_raw.is_empty() || m_raw.is_empty() {
            existing.remove(k);
            continue;
        }

        let cli_match = CliRouteKind::from_provider(&p_raw.to_ascii_lowercase());
        let norm_provider = if k == "orchestrator" {
            if let Some(cli) = cli_match {
                cli.provider_id().to_string()
            } else {
                ProviderKind::parse(p_raw)
                    .map_err(|e| (400, format!("invalid provider for `agent_models.{k}`: {e}")))?
                    .cli_name()
                    .to_string()
            }
        } else if cli_match.is_some() {
            return Err((
                400,
                format!(
                    "agent role `{k}` cannot use `{p_raw}`; helper roles require an in-core API, ChatGPT, or Ollama provider (anthropic, openai, chatgpt, gemini, or ollama)"
                ),
            ));
        } else {
            ProviderKind::parse(p_raw)
                .map_err(|e| (400, format!("invalid provider for `agent_models.{k}`: {e}")))?
                .cli_name()
                .to_string()
        };

        let norm_model = validate_model_id(m_raw, k)?;
        existing.insert(
            k.clone(),
            AgentModelSpec {
                provider: norm_provider,
                model: norm_model,
            },
        );
    }
    Ok(())
}

fn req_in_core_model(req: Option<&Value>) -> Option<(ProviderKind, String)> {
    let r = req?;
    let p_raw = r
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if CliRouteKind::from_provider(&p_raw.to_ascii_lowercase()).is_some() {
        return None;
    }
    let pk = ProviderKind::parse(p_raw).ok()?;
    let m = r
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| pk.resolve_model(s))
        .unwrap_or_else(|| pk.default_model().to_string());
    Some((pk, m))
}

fn role_in_core_model(ui: &UiSettings, role: &str) -> Option<(ProviderKind, String)> {
    let spec = ui.agent_models.get(role)?;
    let pk = ProviderKind::parse(&spec.provider).ok()?;
    Some((pk, pk.resolve_model(&spec.model)))
}

pub fn resolve_in_core_role_model(
    state: &DesktopState,
    role: &str,
    req: Option<&Value>,
    allow_ollama: bool,
) -> Result<(ProviderKind, String), (u16, String)> {
    let ui = state.load_ui_settings();
    let explicit = req
        .and_then(|r| r.get("explicit_model"))
        .and_then(Value::as_bool)
        == Some(true);

    let first_two = if explicit {
        req_in_core_model(req).or_else(|| role_in_core_model(&ui, role))
    } else {
        role_in_core_model(&ui, role).or_else(|| req_in_core_model(req))
    };
    if let Some(pair) = first_two {
        return Ok(pair);
    }

    if let Some(p_raw) = ui.default_provider.as_deref() {
        if let Ok(pk) = ProviderKind::parse(p_raw) {
            let m = ui
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| pk.resolve_model(s))
                .unwrap_or_else(|| pk.default_model().to_string());
            if state.build_transport(pk, &m).is_ok() {
                return Ok((pk, m));
            }
        }
    }

    if state.chatgpt.ready() {
        return Ok((
            ProviderKind::ChatGpt,
            ProviderKind::ChatGpt.default_model().to_string(),
        ));
    }

    let store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).map_err(|e| (500, e))?;
    for p in API_PROVIDERS {
        if p == ProviderKind::Ollama {
            let configured_or_up = own_key(state, p, &store).is_ok_and(|k| k.is_some())
                || state.catalog.has_ok_entry(p.cli_name());
            if allow_ollama
                && configured_or_up
                && state.build_transport(p, p.default_model()).is_ok()
            {
                return Ok((p, p.default_model().to_string()));
            }
            continue;
        }
        if own_key(state, p, &store).is_ok_and(|k| k.is_some()) {
            return Ok((p, p.default_model().to_string()));
        }
    }

    Err((400, NEEDS_API_PROVIDER.to_string()))
}

fn parse_role_obj(val: &Value) -> Option<(ProviderKind, String)> {
    let p = ProviderKind::parse(val.get("provider")?.as_str()?).ok()?;
    let raw_model = val.get("model").and_then(Value::as_str).unwrap_or("");
    Some((p, p.resolve_model(raw_model)))
}

pub fn resolve_optional_role_transport(
    state: &DesktopState,
    ui: &UiSettings,
    req: &Value,
    role: &str,
    primary_provider: ProviderKind,
    primary_model: &str,
) -> Option<(Box<dyn Transport>, String)> {
    let resolved = req
        .get(role)
        .and_then(parse_role_obj)
        .or_else(|| role_in_core_model(ui, role))
        .or_else(|| match role {
            "full_verifier" => req.get("verifier").and_then(parse_role_obj).or_else(|| {
                ui.role_presets
                    .as_ref()
                    .and_then(|p| p.get("full_verifier").or_else(|| p.get("verifier")))
                    .and_then(parse_role_obj)
            }),
            "strategist" => ui
                .role_presets
                .as_ref()
                .and_then(|p| p.get("strategist"))
                .and_then(parse_role_obj),
            _ => None,
        })?;

    let (p, m) = resolved;
    if p == primary_provider && m == primary_model {
        return None;
    }
    state.build_transport(p, &m).ok().map(|t| (t, m))
}

fn build_worker_entry(
    state: &DesktopState,
    provider: ProviderKind,
    model: String,
    reason: &str,
) -> Option<BuiltWorkerTransport> {
    state
        .build_transport(provider, &model)
        .ok()
        .map(|transport| BuiltWorkerTransport {
            transport,
            provider: provider.cli_name().to_string(),
            model,
            routing_reason: reason.to_string(),
        })
}

fn resolve_smaller_or_primary(
    state: &DesktopState,
    primary_provider: ProviderKind,
    primary_model: &str,
) -> Option<BuiltWorkerTransport> {
    if let Some(smaller) = state
        .catalog
        .pick_smaller_model(primary_provider.cli_name(), primary_model)
    {
        if let Some(built) =
            build_worker_entry(state, primary_provider, smaller, "auto_smaller_catalog_model")
        {
            return Some(built);
        }
    }
    build_worker_entry(
        state,
        primary_provider,
        primary_model.to_string(),
        "isolated_primary_model",
    )
}

pub fn resolve_worker_transport(
    state: &DesktopState,
    ui: &UiSettings,
    req: &Value,
    role: &str,
    primary_provider: ProviderKind,
    primary_model: &str,
) -> Option<BuiltWorkerTransport> {
    if let Some((p, m)) = req.get(role).and_then(parse_role_obj) {
        if let Some(built) = build_worker_entry(state, p, m, "request_override") {
            return Some(built);
        }
    }
    if let Some((p, m)) = role_in_core_model(ui, role) {
        if let Some(built) = build_worker_entry(state, p, m, "assigned_in_settings") {
            return Some(built);
        }
    }
    resolve_smaller_or_primary(state, primary_provider, primary_model)
}

/// Resolves the `paper_reader` transport per Decision 4: uses the primary provider/model
/// (`isolated_primary_model`) unless explicitly assigned in settings or request overrides.
pub fn resolve_paper_reader_transport(
    state: &DesktopState,
    ui: &UiSettings,
    req: &Value,
    primary_provider: ProviderKind,
    primary_model: &str,
) -> Option<BuiltWorkerTransport> {
    if let Some((p, m)) = req.get("paper_reader").and_then(parse_role_obj) {
        if let Some(built) = build_worker_entry(state, p, m, "request_override") {
            return Some(built);
        }
    }
    if let Some((p, m)) = role_in_core_model(ui, "paper_reader") {
        if let Some(built) = build_worker_entry(state, p, m, "assigned_in_settings") {
            return Some(built);
        }
    }
    build_worker_entry(
        state,
        primary_provider,
        primary_model.to_string(),
        "isolated_primary_model",
    )
}

pub fn resolve_custom_subagent_transport(
    state: &DesktopState,
    sa: &SubagentDef,
    primary_provider: ProviderKind,
    primary_model: &str,
) -> Option<BuiltWorkerTransport> {
    if let (Some(p_raw), Some(m_raw)) = (sa.provider.as_deref(), sa.model.as_deref()) {
        let trimmed_m = m_raw.trim();
        if !trimmed_m.is_empty() {
            if let Ok(pk) = ProviderKind::parse(p_raw) {
                let model = pk.resolve_model(trimmed_m);
                if let Some(built) =
                    build_worker_entry(state, pk, model, "subagent_assigned_model")
                {
                    return Some(built);
                }
            }
        }
    }
    resolve_smaller_or_primary(state, primary_provider, primary_model)
}

/// Resolves the Orchestrator `(provider, model_requested, model_resolved)` for an in-core turn.
pub fn resolve_orchestrator_model(
    subagent: Option<&SubagentDef>,
    req: &Value,
    meta: Option<&SessionMeta>,
    ui: &UiSettings,
) -> Result<(ProviderKind, String, String), String> {
    if let Some(sa) = subagent {
        if let (Some(p_raw), Some(m_raw)) = (sa.provider.as_deref(), sa.model.as_deref()) {
            let trimmed_m = m_raw.trim();
            if !trimmed_m.is_empty() {
                if let Ok(pk) = ProviderKind::parse(p_raw) {
                    return Ok((pk, trimmed_m.to_string(), pk.resolve_model(trimmed_m)));
                }
            }
        }
    }

    let req_p = req.get("provider").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let meta_p = meta.and_then(|m| m.default_provider.as_deref()).map(str::trim).filter(|s| !s.is_empty());
    let am_orch = ui
        .agent_models
        .get("orchestrator")
        .filter(|s| ProviderKind::parse(&s.provider).is_ok());
    let ui_p = ui.default_provider.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let provider_str = req_p
        .or(meta_p)
        .or_else(|| am_orch.map(|s| s.provider.as_str()))
        .or(ui_p)
        .unwrap_or("anthropic");
    let provider = ProviderKind::parse(provider_str)?;

    let req_m = req.get("model").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let meta_m = meta
        .and_then(|m| m.default_model.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|_| meta_p.is_none_or(|p| ProviderKind::parse(p).ok() == Some(provider)));
    let am_m = am_orch
        .filter(|s| ProviderKind::parse(&s.provider).ok() == Some(provider))
        .map(|s| s.model.trim())
        .filter(|s| !s.is_empty());
    let ui_m = ui
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|_| ui_p.is_none_or(|p| ProviderKind::parse(p).ok() == Some(provider)));

    let model_requested = req_m
        .or(meta_m)
        .or(am_m)
        .or(ui_m)
        .unwrap_or_else(|| provider.default_model())
        .to_string();
    let model = provider.resolve_model(&model_requested);
    Ok((provider, model_requested, model))
}

/// Returns the CLI route and model configured for `"orchestrator"` in `ui_settings`, if any.
pub fn orchestrator_cli_override(state: &DesktopState) -> Option<(CliRouteKind, String)> {
    let ui = state.load_ui_settings();
    let spec = ui.agent_models.get("orchestrator")?;
    let kind = CliRouteKind::from_provider(&spec.provider)?;
    Some((kind, spec.model.clone()))
}

/// Resolved Solve verifier target (`solve.md` 5.3, D2, R13).
///
/// Unlike legacy Research's `resolve_optional_role_transport`, Solve verification ALWAYS runs
/// even when `verifier == solver`, stamping [`ReviewIndependence::SameModel`] on the host.
pub struct SolveVerifierTarget {
    pub transport: Box<dyn Transport>,
    pub role: research_tools::solve_model::RoleModel,
    pub independence: research_tools::solve_model::ReviewIndependence,
    pub routing_reason: String,
}

fn parse_solve_role_pair(
    role: &research_tools::solve_model::RoleModel,
) -> Option<(ProviderKind, String)> {
    let p_raw = role.provider.trim();
    if p_raw.is_empty() || CliRouteKind::from_provider(&p_raw.to_ascii_lowercase()).is_some() {
        return None;
    }
    let pk = ProviderKind::parse(p_raw).ok()?;
    let m_raw = role.model.trim();
    let m = if m_raw.is_empty() {
        pk.default_model().to_string()
    } else {
        pk.resolve_model(m_raw)
    };
    Some((pk, m))
}

fn parse_explicit_req_verifier(req: &Value) -> Result<Option<(ProviderKind, String)>, (u16, String)> {
    let Some(v) = req.get("verifier") else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let obj = v.as_object().ok_or_else(|| {
        (
            400,
            "`verifier` must be an object { provider, model } or null".to_string(),
        )
    })?;
    let p_raw = obj
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| (400, "`verifier.provider` must be a non-empty string".to_string()))?;
    if CliRouteKind::from_provider(&p_raw.to_ascii_lowercase()).is_some() {
        return Err((
            400,
            format!(
                "`verifier.provider` cannot use subscription CLI route `{p_raw}`; Solve verifier requires an in-core API, ChatGPT, or Ollama provider"
            ),
        ));
    }
    let pk = ProviderKind::parse(p_raw)
        .map_err(|e| (400, format!("invalid `verifier.provider`: {e}")))?;
    let m_raw = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| (400, "`verifier.model` must be a non-empty string".to_string()))?;
    let validated = validate_model_id(m_raw, "verifier")?;
    Ok(Some((pk, pk.resolve_model(&validated))))
}

/// Resolve the Solve verifier transport obeying Decision D2:
/// - Checks explicit request override (`req.verifier` or `req.{provider,model}`),
///   then `setup_roles.verifier`, then Settings `agent_models.{verifier,full_verifier,step_verifier}`,
///   then falls back to the solver role or connected in-core provider.
/// - Never skips or returns `None` when `verifier == solver`: builds a live transport and
///   computes `independence` strictly on the host via `compute_review_independence`.
pub fn resolve_solve_verifier_transport(
    state: &DesktopState,
    ui: &UiSettings,
    setup_roles: Option<&research_tools::solve_model::SolveRoles>,
    req: Option<&Value>,
    solver_fallback: Option<&research_tools::solve_model::RoleModel>,
) -> Result<SolveVerifierTarget, (u16, String)> {
    use research_tools::solve_model::{compute_review_independence, RoleModel};

    let solver_role = setup_roles
        .map(|r| r.solver.clone())
        .or_else(|| solver_fallback.cloned())
        .unwrap_or_else(|| RoleModel {
            provider: ui.default_provider.clone().unwrap_or_default(),
            model: ui.default_model.clone().unwrap_or_default(),
        });
    let normalized_solver = parse_solve_role_pair(&solver_role)
        .map(|(pk, m)| RoleModel {
            provider: pk.cli_name().to_string(),
            model: m,
        })
        .unwrap_or_else(|| solver_role.clone());

    let explicit_verifier = match req {
        Some(r) => parse_explicit_req_verifier(r)?,
        None => None,
    };
    let req_override = explicit_verifier.or_else(|| req_in_core_model(req));
    let (provider, model, routing_reason) = if let Some((p, m)) = req_override {
        (p, m, "request_override")
    } else if let Some((p, m)) = setup_roles.and_then(|r| parse_solve_role_pair(&r.verifier)) {
        (p, m, "setup_verifier_role")
    } else if let Some((p, m)) = role_in_core_model(ui, "verifier") {
        (p, m, "assigned_in_settings")
    } else if let Some((p, m)) = role_in_core_model(ui, "full_verifier")
        .or_else(|| role_in_core_model(ui, "step_verifier"))
    {
        (p, m, "assigned_legacy_verifier")
    } else if let Some((p, m)) = parse_solve_role_pair(&solver_role) {
        (p, m, "same_model_fallback")
    } else {
        let (p, m) = resolve_in_core_role_model(state, "verifier", req, true)?;
        (p, m, "connected_default_fallback")
    };

    let transport = state
        .build_transport(provider, &model)
        .map_err(|e| (400, format!("failed to build Solve verifier transport: {e}")))?;
    let verifier_role = RoleModel {
        provider: provider.cli_name().to_string(),
        model,
    };
    let independence = compute_review_independence(&normalized_solver, &verifier_role);
    Ok(SolveVerifierTarget {
        transport,
        role: verifier_role,
        independence,
        routing_reason: routing_reason.to_string(),
    })
}

#[cfg(test)]
#[path = "agent_models_tests.rs"]
mod tests;
