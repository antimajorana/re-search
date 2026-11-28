//! Solve CLI parity helpers: ledger-driven result-sheet honesty (`solve.md` 5.4, 12 risk 13;
//! rules R15, R22) and D16 subscription-CLI verifier status / CP2 API-transport seam.

use crate::approver::UiApprover;
use crate::cli_turn_setup::{meta_kind, CliRouteKind};
use crate::solve_workers::DesktopIsolatedWorkerFactory;
use crate::state::DesktopState;
use agent::verifier_loop::{
    batch_review_targets, count_review_loops, run_finish_review_batches, run_solve_verifier_loop,
    SolveReviewOutcome, SolveVerifierConfig, SolveVerifierRunCtx,
};
use agent::verifier_tools::IsolatedWorkerFactory;
use agent::{CancelToken, ProviderKind, Transport};
use cli_route::StopHandle;
use research_tools::solve_model::{ReviewPolicy, RoleModel, SolveDoneStatus, SolveSetup, StepOrigin};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::sync::Arc;
use tools::Approver;

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
/// `"Not done: N steps unchecked"`. Post-`solve.done` invalidation (superseded final entry,
/// failed checks, or stale steps after a D15 setup revision) also reverts the sheet to
/// `"Not done"` and live tiers are always derived from `state.tier(...)` rather than cached
/// `done.effective_tier`.
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
        let final_active = state
            .entry(&done.final_entry_id)
            .is_some_and(|e| e.superseded_by.is_none());
        let live_tier = final_active.then(|| state.tier(&done.final_entry_id)).flatten();
        if let Some(tier) = live_tier {
            if failed_steps == 0 && stale_steps == 0 {
                let effective_status =
                    if done.status == SolveDoneStatus::Finished && unchecked_steps > 0 {
                        SolveDoneStatus::FinishedOpenItems
                    } else {
                        done.status
                    };
                let status_line = match effective_status {
                    SolveDoneStatus::Finished => "Done",
                    SolveDoneStatus::FinishedOpenItems => "Done with open items",
                };
                return json!({
                    "done": true,
                    "status": effective_status,
                    "status_line": status_line,
                    "unchecked_steps": unchecked_steps,
                    "failed_steps": failed_steps,
                    "stale_steps": stale_steps,
                    "final_entry_id": done.final_entry_id,
                    "own_tier": tier.own,
                    "effective_tier": tier.effective,
                    "badge_label": tier.effective.label(),
                    "criteria": done.criteria,
                    "rests_on": done.rests_on,
                });
            }
        }
    }

    let status_line = if failed_steps > 0 {
        let word = if failed_steps == 1 { "step" } else { "steps" };
        format!("Not done: {failed_steps} {word} failed")
    } else if stale_steps > 0 {
        let word = if stale_steps == 1 { "step" } else { "steps" };
        format!("Not done: {stale_steps} {word} stale")
    } else if unchecked_steps > 0 {
        let word = if unchecked_steps == 1 { "step" } else { "steps" };
        format!("Not done: {unchecked_steps} {word} unchecked")
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
        let prov = role.provider.trim();
        if !prov.is_empty() {
            return CliRouteKind::from_provider(&prov.to_ascii_lowercase());
        }
    }
    if let Some(setup) = setup {
        let prov = setup.roles.solver.provider.trim();
        if !prov.is_empty() {
            return CliRouteKind::from_provider(&prov.to_ascii_lowercase());
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

fn resolve_verifier_spec(
    state: &DesktopState,
    provider: &str,
    model: &str,
) -> Result<ResolvedCliVerifier, String> {
    let prov = provider.trim();
    if CliRouteKind::from_provider(&prov.to_ascii_lowercase()).is_some() {
        return Err(D16_CLI_VERIFIER_REJECTED_NOTICE.to_string());
    }
    let pk = ProviderKind::parse(prov)
        .map_err(|e| format!("invalid verifier provider `{prov}`: {e}"))?;
    let resolved_model = if model.trim().is_empty() {
        pk.default_model().to_string()
    } else {
        pk.resolve_model(model.trim())
    };
    let transport = state.build_transport(pk, &resolved_model)?;
    Ok((transport, pk.cli_name().to_string(), resolved_model))
}

/// Resolve an in-core API `Transport` for Solve verifier review on a subscription-CLI session
/// (CP2 seam for S14 `verifier_loop`).
///
/// Strictly enforces `agent_models.rs`'s helper-role rule: any CLI provider (`claude-cli`,
/// `antigravity`) in `verifier_role` or `ui.agent_models` verifier entries is rejected with
/// `Err(...)`, never routed as a helper. Never fabricates an unrequested verifier from unrelated
/// provider keys when the user left Verifier blank.
pub fn resolve_api_verifier_transport_for_cli(
    state: &DesktopState,
    verifier_role: Option<&RoleModel>,
) -> Result<Option<ResolvedCliVerifier>, String> {
    if let Some(role) = verifier_role {
        if !role.provider.trim().is_empty() {
            return resolve_verifier_spec(state, &role.provider, &role.model).map(Some);
        }
    }

    let ui = state.load_ui_settings();
    if let Some(spec) = ui
        .agent_models
        .get("step_verifier")
        .or_else(|| ui.agent_models.get("full_verifier"))
        .or_else(|| ui.agent_models.get("verifier"))
    {
        if !spec.provider.trim().is_empty() {
            return resolve_verifier_spec(state, &spec.provider, &spec.model).map(Some);
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
            "automatic_review_available": true,
            "cli_helper_rejected": false,
            "verifier_transport": "in_core_api",
            "verifier": { "provider": provider, "model": model },
            "s14_verifier_loop_ready": true,
            "notice": Value::Null,
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

/// Run automatic post-CLI-turn API `verifier_loop` review (CP2) when `Mode::Solve` completes
/// on a subscription-CLI route and an in-core API verifier is configured (`solve.md` 5.2-5.3, D16).
pub fn maybe_run_post_cli_solve_review(
    state: &Arc<DesktopState>,
    research: &Arc<ResearchEnv>,
    session_id: &str,
    stop: &StopHandle,
) -> Result<(), String> {
    let ledger = state.open_ledger()?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| format!("failed to read session `{session_id}` for CLI review: {e}"))?;
    let folded = SolveState::from_events(&events);
    let Some(ref setup) = folded.setup else {
        return Ok(());
    };
    if setup.review == ReviewPolicy::Off {
        return Ok(());
    }
    let explicit_verifier = Some(&setup.roles.verifier).filter(|r| !r.provider.trim().is_empty());
    let Some((transport, provider, model)) =
        resolve_api_verifier_transport_for_cli(state, explicit_verifier)?
    else {
        return Ok(());
    };

    let cancel = CancelToken::new();
    if stop.is_stopped() {
        cancel.cancel();
        return Err("turn stopped by the user".to_string());
    }

    let factory: Arc<dyn IsolatedWorkerFactory> =
        Arc::new(DesktopIsolatedWorkerFactory::new(Arc::clone(state)));
    let contained = factory.isolated_workers_contained();
    let containment = if contained {
        "AppContainer isolated verifier worker".to_string()
    } else {
        "desktop host (no sandboxed Python verifier worker configured)".to_string()
    };
    let approver: Arc<dyn Approver> =
        Arc::new(UiApprover::new(Arc::clone(state), containment, contained));
    let config = SolveVerifierConfig::new(RoleModel { provider, model });
    let ctx = SolveVerifierRunCtx {
        transport: transport.as_ref(),
        ledger: &ledger,
        env: Arc::clone(research),
        session: session_id,
        config: &config,
        approver,
        factory: &factory,
        cancel: Some(&cancel),
    };

    if let Some(ref done) = folded.done {
        let summary = run_finish_review_batches(&ctx, &done.final_entry_id)?;
        if summary.batches_run > 0 {
            let independence = summary.outcomes.first().map(|o| o.independence());
            let completed = summary
                .outcomes
                .iter()
                .all(|o| matches!(o, SolveReviewOutcome::Completed { .. }));
            state.sse.emit(
                session_id,
                "solve_updated",
                json!({
                    "session": session_id,
                    "review_completed": completed,
                    "independence": independence,
                }),
            );
        }
        return Ok(());
    }

    if setup.review != ReviewPolicy::All {
        return Ok(());
    }

    let remaining_u32 = setup
        .budgets
        .max_review_loops
        .saturating_sub(count_review_loops(&events));
    let remaining_loops = usize::try_from(remaining_u32).unwrap_or(usize::MAX);
    if remaining_loops == 0 {
        return Ok(());
    }

    let targets: Vec<String> = folded
        .active_entries()
        .into_iter()
        .filter(|e| e.origin != Some(StepOrigin::Given))
        .filter(|e| folded.tier(&e.entry_id).is_none_or(|t| t.own == Tier::Unchecked))
        .map(|e| e.entry_id.clone())
        .collect();
    if targets.is_empty() {
        return Ok(());
    }

    for batch in batch_review_targets(&targets).into_iter().take(remaining_loops) {
        if stop.is_stopped() {
            cancel.cancel();
            return Err("turn stopped by the user".to_string());
        }
        let outcome = run_solve_verifier_loop(&ctx, &batch)?;
        state.sse.emit(
            session_id,
            "solve_updated",
            json!({
                "session": session_id,
                "review_completed": matches!(outcome, SolveReviewOutcome::Completed { .. }),
                "independence": outcome.independence(),
            }),
        );
    }

    Ok(())
}
