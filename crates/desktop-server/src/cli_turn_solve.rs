//! Solve CLI parity helpers: ledger-driven result-sheet honesty (`solve.md` 5.4, 12 risk 13;
//! rules R15, R22) and D16 subscription-CLI verifier status / CP2 API-transport seam.

use crate::agent_models::resolve_in_core_role_model;
use crate::cli_turn_setup::{meta_kind, CliRouteKind};
use crate::state::DesktopState;
use agent::{ProviderKind, Transport};
use research_tools::solve_model::{RoleModel, SolveDoneStatus, SolveSetup};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use serde_json::{json, Value};

/// Plain D16 notice when a subscription-CLI user has no in-core API verifier configured.
/// Kept as a single named constant (`solve.md` D16; zero R22-forbidden wording).
pub const D16_CLI_REVIEW_UNAVAILABLE_NOTICE: &str =
    "Solve runs on this subscription-CLI route with host algebraic and numeric checks. \
     Automatic model review needs an in-core API, ChatGPT, or Ollama provider configured \
     under Settings -> Providers or Agents & Models; until then, prose steps stay Unchecked.";

/// Plain D16 notice when `roles.verifier` names a subscription-CLI provider (disallowed by
/// `agent_models.rs`'s helper-role guard).
pub const D16_CLI_VERIFIER_REJECTED_NOTICE: &str =
    "Helper verifier roles cannot use subscription-CLI providers (`claude-cli`, `antigravity`). \
     Choose an in-core API, ChatGPT, or Ollama provider for the verifier role; host algebraic \
     and numeric checks still run.";

/// Plain D16 notice when an in-core API verifier is configured and ready for S14 `verifier_loop`
/// integration (CP2 seam).
pub const D16_CP2_PENDING_S14_NOTICE: &str =
    "In-core API verifier transport is configured for this subscription-CLI Solve session; \
     automatic verifier-loop execution activates when Solve S14 (`verifier_loop`) is wired.";

/// Compute the ledger-driven Solve result sheet strictly from `SolveState` (`solve.md` 5.4,
/// 12 risk 13; rules R15 & R22).
///
/// Never inspects CLI assistant prose (`model.text` or `cli.result`), so unchecked CLI output
/// claiming `"Final answer is verified!"` without `solve_finish` still renders
/// `"Not done: N steps unchecked"`.
pub fn build_ledger_result_sheet(state: &SolveState) -> Value {
    let mut unchecked_steps: usize = 0;
    let mut failed_steps: usize = 0;
    let mut stale_steps: usize = 0;

    for entry in state.active_entries() {
        let Some(tier) = state.tier(&entry.entry_id) else {
            unchecked_steps += 1;
            continue;
        };
        if tier.effective == Tier::Failed || tier.own == Tier::Failed {
            failed_steps += 1;
        } else if tier.stale || tier.effective == Tier::Stale {
            stale_steps += 1;
        } else if matches!(tier.effective, Tier::Unchecked | Tier::AssumedByModel) {
            unchecked_steps += 1;
        }
    }

    if let Some(done) = &state.done {
        let status_line = match done.status {
            SolveDoneStatus::Finished => "Done",
            SolveDoneStatus::FinishedOpenItems => "Done with open items",
        };
        let badge_label = state
            .tier(&done.final_entry_id)
            .map(|t| t.effective.label())
            .or_else(|| {
                serde_json::from_value::<Tier>(json!(done.effective_tier))
                    .ok()
                    .map(Tier::label)
            })
            .unwrap_or(Tier::Unchecked.label());
        return json!({
            "done": true,
            "status": done.status,
            "status_line": status_line,
            "unchecked_steps": unchecked_steps,
            "failed_steps": failed_steps,
            "stale_steps": stale_steps,
            "final_entry_id": done.final_entry_id,
            "own_tier": done.own_tier,
            "effective_tier": done.effective_tier,
            "badge_label": badge_label,
            "criteria": done.criteria,
            "rests_on": done.rests_on,
        });
    }

    let status_line = if unchecked_steps > 0 {
        let word = if unchecked_steps == 1 { "step" } else { "steps" };
        format!("Not done: {unchecked_steps} {word} unchecked")
    } else if failed_steps > 0 {
        let word = if failed_steps == 1 { "step" } else { "steps" };
        format!("Not done: {failed_steps} {word} failed")
    } else if stale_steps > 0 {
        let word = if stale_steps == 1 { "step" } else { "steps" };
        format!("Not done: {stale_steps} {word} stale")
    } else {
        "Not done".to_string()
    };

    json!({
        "done": false,
        "status": "not_done",
        "status_line": status_line,
        "unchecked_steps": unchecked_steps,
        "failed_steps": failed_steps,
        "stale_steps": stale_steps,
        "final_entry_id": Value::Null,
        "own_tier": Value::Null,
        "effective_tier": Value::Null,
        "badge_label": Value::Null,
    })
}

fn cli_kind_for_solver(
    state: &DesktopState,
    session_id: &str,
    setup: Option<&SolveSetup>,
    solver_override: Option<&RoleModel>,
) -> Option<CliRouteKind> {
    if let Some(role) = solver_override {
        if let Some(k) = CliRouteKind::from_provider(role.provider.trim()) {
            return Some(k);
        }
    }
    if let Some(setup) = setup {
        if let Some(k) = CliRouteKind::from_provider(setup.roles.solver.provider.trim()) {
            return Some(k);
        }
    }
    let meta = state.load_meta(session_id);
    if let Some(k) = meta_kind(meta.as_ref()) {
        return Some(k);
    }
    crate::agent_models::orchestrator_cli_override(state).map(|(k, _)| k)
}

/// Resolved in-core API verifier `(transport, provider, model)` for CP2 S14 `verifier_loop`.
pub type ResolvedCliVerifier = (Box<dyn Transport>, String, String);

/// Resolve an in-core API `Transport` for Solve verifier review on a subscription-CLI session
/// (CP2 seam for S14 `verifier_loop`).
///
/// Strictly enforces `agent_models.rs`'s helper-role rule: any CLI provider (`claude-cli`,
/// `antigravity`) in `verifier_role` is rejected with `Err(...)`, never routed as a helper.
pub fn resolve_api_verifier_transport_for_cli(
    state: &DesktopState,
    verifier_role: Option<&RoleModel>,
) -> Result<Option<ResolvedCliVerifier>, String> {
    if let Some(role) = verifier_role {
        let prov = role.provider.trim();
        if !prov.is_empty() {
            if CliRouteKind::from_provider(&prov.to_ascii_lowercase()).is_some() {
                return Err(D16_CLI_VERIFIER_REJECTED_NOTICE.to_string());
            }
            let pk = ProviderKind::parse(prov)
                .map_err(|e| format!("invalid verifier provider `{prov}`: {e}"))?;
            let model = if role.model.trim().is_empty() {
                pk.default_model().to_string()
            } else {
                pk.resolve_model(role.model.trim())
            };
            let transport = state.build_transport(pk, &model)?;
            return Ok(Some((transport, pk.cli_name().to_string(), model)));
        }
    }

    let ui = state.load_ui_settings();
    if let Some(spec) = ui
        .agent_models
        .get("verifier")
        .or_else(|| ui.agent_models.get("step_verifier"))
        .or_else(|| ui.agent_models.get("full_verifier"))
    {
        if CliRouteKind::from_provider(&spec.provider.to_ascii_lowercase()).is_some() {
            return Err(D16_CLI_VERIFIER_REJECTED_NOTICE.to_string());
        }
        if let Ok(pk) = ProviderKind::parse(&spec.provider) {
            let model = pk.resolve_model(&spec.model);
            if let Ok(transport) = state.build_transport(pk, &model) {
                return Ok(Some((transport, pk.cli_name().to_string(), model)));
            }
        }
    }

    if let Ok((pk, model)) = resolve_in_core_role_model(state, "step_verifier", None, true) {
        if let Ok(transport) = state.build_transport(pk, &model) {
            return Ok(Some((transport, pk.cli_name().to_string(), model)));
        }
    }

    Ok(None)
}

/// Build the D16 subscription-CLI review availability block returned on `/solve/prefill` and
/// `/solve/view` so the setup card surfaces an honest notice without hiding Solve from CLI users.
pub fn resolve_cli_verifier_status(
    state: &DesktopState,
    session_id: &str,
    setup: Option<&SolveSetup>,
    solver_override: Option<&RoleModel>,
    verifier_override: Option<&RoleModel>,
) -> Value {
    let cli_route = cli_kind_for_solver(state, session_id, setup, solver_override);
    let Some(kind) = cli_route else {
        return json!({
            "on_cli_route": false,
            "cli_provider": Value::Null,
            "api_verifier_configured": false,
            "automatic_review_available": false,
            "cli_helper_rejected": false,
            "verifier_transport": Value::Null,
            "verifier": Value::Null,
            "s14_verifier_loop_ready": false,
            "notice": Value::Null,
        });
    };

    let explicit_verifier = verifier_override
        .filter(|r| !r.provider.trim().is_empty())
        .or_else(|| setup.map(|s| &s.roles.verifier).filter(|r| !r.provider.trim().is_empty()));

    match resolve_api_verifier_transport_for_cli(state, explicit_verifier) {
        Ok(Some((_transport, provider, model))) => json!({
            "on_cli_route": true,
            "cli_provider": kind.provider_id(),
            "api_verifier_configured": true,
            "automatic_review_available": false,
            "cli_helper_rejected": false,
            "verifier_transport": "in_core_api",
            "verifier": { "provider": provider, "model": model },
            "s14_verifier_loop_ready": false,
            "notice": D16_CP2_PENDING_S14_NOTICE,
        }),
        Ok(None) => json!({
            "on_cli_route": true,
            "cli_provider": kind.provider_id(),
            "api_verifier_configured": false,
            "automatic_review_available": false,
            "cli_helper_rejected": false,
            "verifier_transport": Value::Null,
            "verifier": Value::Null,
            "s14_verifier_loop_ready": false,
            "notice": D16_CLI_REVIEW_UNAVAILABLE_NOTICE,
        }),
        Err(reason) => json!({
            "on_cli_route": true,
            "cli_provider": kind.provider_id(),
            "api_verifier_configured": false,
            "automatic_review_available": false,
            "cli_helper_rejected": reason == D16_CLI_VERIFIER_REJECTED_NOTICE,
            "verifier_transport": Value::Null,
            "verifier": Value::Null,
            "s14_verifier_loop_ready": false,
            "notice": reason,
        }),
    }
}
