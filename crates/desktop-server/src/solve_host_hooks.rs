//! Desktop `SolveHostHooks` wiring S14 (`review_steps` & finish verifier batches) and
//! S20 (`compute_blind`) into the in-core `solve/1` execution loop (`solve_turn_routes.rs`).

use crate::agent_models::{resolve_in_core_role_model, resolve_solve_verifier_transport};
use crate::approver::UiApprover;
use crate::cli_turn::CliRouteKind;
use crate::solve_workers::DesktopIsolatedWorkerFactory;
use crate::state::DesktopState;
use agent::delegate_blind::{
    compute_blind_tool_spec, run_compute_blind, BlindComputeConfig, BlindComputeRunCtx,
};
use agent::verifier_loop::{
    handle_review_steps, run_finish_review_batches, select_finish_review_targets,
    SolveVerifierConfig, SolveVerifierRunCtx,
};
use agent::verifier_tools::{review_steps_tool_spec, IsolatedWorkerFactory, REVIEW_STEPS};
use agent::{CancelToken, ProviderKind, SolveHostHooks, ToolSpec, Transport};
use ledger::Ledger;
use research_tools::solve_gates::{PauseTrigger, COMPUTE_BLIND};
use research_tools::solve_model::{PauseReason, ReviewPolicy, RoleModel, SolveSetup};
use research_tools::solve_state::SolveState;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::sync::Arc;
use tools::Approver;

pub(crate) struct DesktopSolveHostHooks {
    state: Arc<DesktopState>,
    env: Arc<ResearchEnv>,
    req: Value,
    solver_fallback: RoleModel,
}

type BuiltRoleTransport = (Box<dyn Transport>, RoleModel);

impl DesktopSolveHostHooks {
    pub(crate) fn new(
        state: Arc<DesktopState>,
        env: Arc<ResearchEnv>,
        req: Value,
        solver_fallback: RoleModel,
    ) -> Self {
        Self {
            state,
            env,
            req,
            solver_fallback,
        }
    }

    fn approver_and_factory(&self) -> (Arc<dyn Approver>, Arc<dyn IsolatedWorkerFactory>) {
        let factory: Arc<dyn IsolatedWorkerFactory> =
            Arc::new(DesktopIsolatedWorkerFactory::new(Arc::clone(&self.state)));
        let contained = factory.isolated_workers_contained();
        let containment = if contained {
            "AppContainer isolated Solve worker".to_string()
        } else {
            "desktop host (no sandboxed Python isolated worker configured)".to_string()
        };
        let approver: Arc<dyn Approver> =
            Arc::new(UiApprover::new(Arc::clone(&self.state), containment, contained));
        (approver, factory)
    }

    fn parse_role_transport(
        &self,
        role: &RoleModel,
    ) -> Option<Result<BuiltRoleTransport, String>> {
        let p_raw = role.provider.trim();
        if p_raw.is_empty() || CliRouteKind::from_provider(&p_raw.to_ascii_lowercase()).is_some() {
            return None;
        }
        let pk = ProviderKind::parse(p_raw).ok()?;
        let m = if role.model.trim().is_empty() {
            pk.default_model().to_string()
        } else {
            pk.resolve_model(role.model.trim())
        };
        Some(
            self.state
                .build_transport(pk, &m)
                .map(|transport| {
                    (
                        transport,
                        RoleModel {
                            provider: pk.cli_name().to_string(),
                            model: m,
                        },
                    )
                })
                .map_err(|e| format!("failed to build Solve compute transport: {e}")),
        )
    }

    fn build_compute_transport(
        &self,
        setup: &SolveSetup,
    ) -> Result<BuiltRoleTransport, String> {
        if let Some(ref compute_role) = setup.roles.compute {
            if let Some(res) = self.parse_role_transport(compute_role) {
                return res;
            }
        }
        if let Some(res) = self.parse_role_transport(&self.solver_fallback) {
            return res;
        }
        let (pk, m) = resolve_in_core_role_model(&self.state, "compute_worker", None, true)
            .map_err(|(_, msg)| msg)?;
        let transport = self
            .state
            .build_transport(pk, &m)
            .map_err(|e| format!("failed to build Solve compute transport: {e}"))?;
        Ok((
            transport,
            RoleModel {
                provider: pk.cli_name().to_string(),
                model: m,
            },
        ))
    }

    fn exec_review_steps(
        &self,
        ledger: &Ledger,
        session: &str,
        input: &Value,
        cancel: Option<&CancelToken>,
    ) -> (bool, String) {
        let events = match ledger.read_session(session) {
            Ok(evs) => evs,
            Err(e) => return (true, json!({ "error": e.to_string() }).to_string()),
        };
        let state = SolveState::from_events(&events);
        let setup_roles = state.setup.as_ref().map(|s| &s.roles);
        let ui = self.state.load_ui_settings();
        let target = match resolve_solve_verifier_transport(
            &self.state,
            &ui,
            setup_roles,
            Some(&self.req),
            Some(&self.solver_fallback),
        ) {
            Ok(t) => t,
            Err((_, msg)) => return (true, json!({ "error": msg }).to_string()),
        };
        let (approver, factory) = self.approver_and_factory();
        let config = SolveVerifierConfig::new(target.role);
        let ctx = SolveVerifierRunCtx {
            transport: target.transport.as_ref(),
            ledger,
            env: Arc::clone(&self.env),
            session,
            config: &config,
            approver,
            factory: &factory,
            cancel,
        };
        match handle_review_steps(&ctx, input) {
            Ok(val) => (false, val.to_string()),
            Err(err) => (true, json!({ "error": err }).to_string()),
        }
    }

    fn exec_compute_blind(
        &self,
        ledger: &Ledger,
        session: &str,
        call_id: &str,
        input: &Value,
        cancel: Option<&CancelToken>,
    ) -> (bool, String) {
        let events = match ledger.read_session(session) {
            Ok(evs) => evs,
            Err(e) => return (true, json!({ "error": e.to_string() }).to_string()),
        };
        let state = SolveState::from_events(&events);
        let Some(ref setup) = state.setup else {
            return (
                true,
                json!({ "error": "Solve setup must be recorded before `compute_blind`" })
                    .to_string(),
            );
        };
        let (transport, role) = match self.build_compute_transport(setup) {
            Ok(pair) => pair,
            Err(err) => return (true, json!({ "error": err }).to_string()),
        };
        let (_, factory) = self.approver_and_factory();
        let config = BlindComputeConfig::new(role);
        let ctx = BlindComputeRunCtx {
            transport: transport.as_ref(),
            ledger,
            env: Arc::clone(&self.env),
            session,
            config: &config,
            factory: &factory,
            cancel,
        };
        match run_compute_blind(&ctx, call_id, input) {
            Ok(res) => match serde_json::to_string(&res) {
                Ok(s) => (false, s),
                Err(e) => (true, json!({ "error": e.to_string() }).to_string()),
            },
            Err(err) => (true, json!({ "error": err }).to_string()),
        }
    }
}

impl SolveHostHooks for DesktopSolveHostHooks {
    fn extra_tool_specs(&self) -> Vec<ToolSpec> {
        vec![review_steps_tool_spec(), compute_blind_tool_spec()]
    }

    fn handle_host_tool(
        &self,
        ledger: &Ledger,
        session: &str,
        tool_name: &str,
        call_id: &str,
        input: &Value,
        cancel: Option<&CancelToken>,
    ) -> Option<Result<(bool, String), String>> {
        match tool_name {
            REVIEW_STEPS => Some(Ok(self.exec_review_steps(ledger, session, input, cancel))),
            COMPUTE_BLIND => {
                Some(Ok(self.exec_compute_blind(ledger, session, call_id, input, cancel)))
            }
            _ => None,
        }
    }

    fn before_solve_finish(
        &self,
        ledger: &Ledger,
        session: &str,
        input: &Value,
        cancel: Option<&CancelToken>,
    ) -> Result<Option<PauseTrigger>, String> {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Ok(Some(PauseTrigger {
                reason: PauseReason::Interrupted,
                message: "stopped by user".into(),
                open_items: Vec::new(),
            }));
        }
        let final_entry_id = input
            .get("final_entry_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if final_entry_id.is_empty() {
            return Ok(None);
        }
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        let state = SolveState::from_events(&events);
        let Some(ref setup) = state.setup else {
            return Ok(None);
        };
        if setup.review == ReviewPolicy::Off {
            return Ok(None);
        }
        let Some(entry) = state.entry(final_entry_id) else {
            return Ok(None);
        };
        if entry.superseded_by.is_some() {
            return Ok(None);
        }
        if select_finish_review_targets(&state, final_entry_id).is_empty() {
            return Ok(None);
        }

        let ui = self.state.load_ui_settings();
        let target = match resolve_solve_verifier_transport(
            &self.state,
            &ui,
            Some(&setup.roles),
            Some(&self.req),
            Some(&self.solver_fallback),
        ) {
            Ok(t) => t,
            Err((_, msg)) => {
                let reason = if cancel.is_some_and(CancelToken::is_cancelled) {
                    PauseReason::Interrupted
                } else {
                    PauseReason::TransportError
                };
                return Ok(Some(PauseTrigger {
                    reason,
                    message: msg,
                    open_items: Vec::new(),
                }));
            }
        };

        let (approver, factory) = self.approver_and_factory();
        let config = SolveVerifierConfig::new(target.role);
        let ctx = SolveVerifierRunCtx {
            transport: target.transport.as_ref(),
            ledger,
            env: Arc::clone(&self.env),
            session,
            config: &config,
            approver,
            factory: &factory,
            cancel,
        };
        match run_finish_review_batches(&ctx, final_entry_id) {
            Ok(summary) => {
                if let Some(stuck) = summary.verification_stuck {
                    let message = stuck
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Gate G6: verification stuck (2+ consecutive rejected or incomplete batches)")
                        .to_string();
                    let open_items = stuck
                        .get("open_items")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    return Ok(Some(PauseTrigger {
                        reason: PauseReason::VerificationStuck,
                        message,
                        open_items,
                    }));
                }
                Ok(None)
            }
            Err(err) => {
                let reason = if cancel.is_some_and(CancelToken::is_cancelled) {
                    PauseReason::Interrupted
                } else {
                    PauseReason::TransportError
                };
                Ok(Some(PauseTrigger {
                    reason,
                    message: err,
                    open_items: Vec::new(),
                }))
            }
        }
    }
}
