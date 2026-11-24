//! Autonomous Solver -> Independent Stateless Verifier -> Research Strategist -> Revise loop.

pub use crate::autonomous_prompts::RESEARCH_SYSTEM_PROMPT;
use crate::autonomous_prompts::{
    build_revision_prompt, build_unverified_recovery, ensure_active_final_answer_entry, format_unverified_banner,
    inject_feedback, maybe_run_strategist,
};
use crate::guardrails::{
    build_research_system_prompt, check_computation_interpretation, check_final_answer_gate,
    GuardrailConfig,
};
use crate::honesty::{infer_campaign_phase, lint_honesty_with_ledger, CampaignPhase};
use crate::resume::{fork_assumption, format_fork_assumption, rehydrate_history, verify_and_read, ResumeConfig};
use crate::runner::{
    run_turn_leg, tool_defs_json, AgentConfig, Audit, EventSink, LegResult, LoopParams, SessionOutcome,
};
use crate::transport::{CancelToken, ContentBlock, Message, ProviderKind, Role, Transport};
use crate::verifiers::{verify_delivered_answer, DeliveredAnswer, VerificationVerdict};
use ledger::Ledger;
use research_tools::{read_proof_dag, ProofStatus, ResearchEnv};
use serde_json::json;
use std::time::Instant;
use tools::{Router, ToolDef};

pub const DEFAULT_MAX_VERIFICATION_ROUNDS: u32 = 3;
const REVISION_TURN_RESERVE: u32 = 8;

/// Optional independent role transports & models for Verifier, Strategist, and Worker delegation.
/// When `verifier` or `strategist` is `None`, falls back to the Orchestrator transport/model.
#[derive(Default)]
pub struct AutonomousRoles<'a> {
    pub verifier: Option<(&'a dyn Transport, &'a str)>,
    pub strategist: Option<(&'a dyn Transport, &'a str)>,
    pub delegation: Option<crate::delegate::DelegationConfig<'a>>,
    pub human: Option<&'a dyn crate::interaction::HumanChannel>,
}

#[derive(Debug, Clone)]
pub struct AutonomousConfig {
    pub guardrails: GuardrailConfig,
    pub use_strategist: bool,
    pub max_verification_rounds: u32,
    pub interactive_plan_mode: bool,
    pub pause_after_explore: bool,
    pub project_context: Option<String>,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            guardrails: GuardrailConfig::default(),
            use_strategist: false,
            max_verification_rounds: DEFAULT_MAX_VERIFICATION_ROUNDS,
            interactive_plan_mode: false,
            pause_after_explore: false,
            project_context: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_autonomous_research_loop(
    solver_transport: &dyn Transport,
    roles: &AutonomousRoles<'_>,
    router: &Router,
    ledger: &Ledger,
    env: &ResearchEnv,
    cfg: &AgentConfig,
    auto_cfg: &AutonomousConfig,
    cancel: Option<&CancelToken>,
    sink: &mut dyn EventSink,
) -> Result<SessionOutcome, String> {
    if cfg.max_turns == 0 {
        return Err("max_turns must be >= 1".into());
    }
    if cfg.max_tokens == 0 {
        return Err("max_tokens must be >= 1".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("model name must not be empty".into());
    }
    let base_prompt = if cfg.system_prompt.trim().is_empty()
        || cfg.system_prompt == crate::runner::DEFAULT_SYSTEM_PROMPT
    {
        RESEARCH_SYSTEM_PROMPT
    } else {
        cfg.system_prompt.as_str()
    };
    let mut system_prompt = build_research_system_prompt(base_prompt, &auto_cfg.guardrails)?;
    if let Some(dcfg) = roles.delegation.as_ref() {
        let sec = crate::delegate::format_delegation_prompt_section(dcfg);
        if !sec.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&sec);
        }
    }
    if roles.human.is_some() {
        crate::interaction::upsert_interactive_prompt_section(
            &mut system_prompt,
            auto_cfg.interactive_plan_mode,
            auto_cfg.pause_after_explore,
        );
    }
    let session = router.session();
    let info = solver_transport.info();
    let defs = router.registry().defs();
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let last_seq = events.last().map_or(0, |e| e.seq);
    let mut audit = Audit { ledger, session, sink, last_seq };
    audit.emit(
        "session.start",
        json!({
            "route": "api",
            "provenance": "in-core loop, full provenance",
            "provider": info.provider,
            "endpoint": info.endpoint,
            "api_key_env": info.api_key_env,
            "model_requested": cfg.model_requested,
            "model": cfg.model,
            "prompt": cfg.prompt,
            "project_context": auto_cfg.project_context,
            "workspace_revision": env.workspace_root.as_deref().and_then(crate::workspace_revision),
            "system_prompt": system_prompt,
            "max_turns": cfg.max_turns,
            "max_tokens": cfg.max_tokens,
            "policy_version": router.policy().version.as_str(),
            "containment": cfg.containment,
            "contained": cfg.contained,
            "verification_mode": auto_cfg.guardrails.verification_mode,
            "calculation_mode": auto_cfg.guardrails.calculation_mode,
            "use_strategist": auto_cfg.use_strategist,
            "interactive_plan_mode": auto_cfg.interactive_plan_mode,
            "pause_after_explore": auto_cfg.pause_after_explore,
            "pinned_conventions": auto_cfg.guardrails.pinned_conventions,
            "tool_defs": tool_defs_json(&defs),
        }),
    )?;

    let mut initial_content: Vec<_> = events.iter().filter_map(fork_assumption)
        .map(|assumption| ContentBlock::Text { text: format_fork_assumption(assumption) })
        .collect();
    if let Some(context) = &auto_cfg.project_context {
        initial_content.push(ContentBlock::Text { text: context.clone() });
    }
    initial_content.push(ContentBlock::Text { text: cfg.prompt.clone() });
    let initial_messages = vec![Message { role: Role::User, content: initial_content }];
    drive_autonomous_rounds(
        solver_transport,
        roles,
        router,
        env,
        &mut audit,
        &cfg.model,
        &system_prompt,
        cfg.max_turns,
        cfg.max_tokens,
        cfg.reasoning_effort.as_deref(),
        1,
        initial_messages,
        auto_cfg,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn resume_autonomous_research_loop(
    solver_transport: &dyn Transport,
    roles: &AutonomousRoles<'_>,
    router: &Router,
    ledger: &Ledger,
    env: &ResearchEnv,
    cfg: &ResumeConfig,
    auto_cfg: &AutonomousConfig,
    cancel: Option<&CancelToken>,
    sink: &mut dyn EventSink,
) -> Result<SessionOutcome, String> {
    if cfg.max_turns == 0 {
        return Err("max_turns must be >= 1".into());
    }
    if cfg.max_tokens == 0 {
        return Err("max_tokens must be >= 1".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("model name must not be empty".into());
    }
    if cfg.prompt.trim().is_empty() {
        return Err("prompt must not be empty".into());
    }

    let session = router.session();
    let defs = router.registry().defs();
    let expected_defs: Vec<ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
    let policy_version = router.policy().version.as_str();
    let (inspection, events) =
        verify_and_read(ledger, session, policy_version, &expected_defs).map_err(|e| e.to_string())?;

    let info = solver_transport.info();
    let target_provider = ProviderKind::parse(info.provider)?;
    let mut messages = rehydrate_history(ledger, &events, target_provider).map_err(|e| e.to_string())?;
    let current_content: Vec<_> = auto_cfg.project_context.iter().chain(std::iter::once(&cfg.prompt))
        .map(|text| ContentBlock::Text { text: text.clone() }).collect();
    if let Some(last) = messages.last_mut().filter(|m| m.role == Role::User) {
        last.content.extend(current_content);
    } else {
        messages.push(Message { role: Role::User, content: current_content });
    }

    let base_sys = cfg.system_prompt.as_deref().unwrap_or(&inspection.system_prompt);
    let mut system_prompt = if base_sys == crate::runner::DEFAULT_SYSTEM_PROMPT {
        build_research_system_prompt(RESEARCH_SYSTEM_PROMPT, &auto_cfg.guardrails)?
    } else {
        base_sys.to_string()
    };
    if let Some(dcfg) = roles.delegation.as_ref() {
        if !system_prompt.contains("Orchestrator -> Worker Delegation:") {
            let sec = crate::delegate::format_delegation_prompt_section(dcfg);
            if !sec.is_empty() {
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&sec);
            }
        }
    }
    crate::guardrails::upsert_pinned_conventions_in_prompt(
        &mut system_prompt,
        auto_cfg.guardrails.pinned_conventions.as_deref(),
    )?;
    if roles.human.is_some() {
        crate::interaction::upsert_interactive_prompt_section(
            &mut system_prompt,
            auto_cfg.interactive_plan_mode,
            auto_cfg.pause_after_explore,
        );
    }

    let mut audit = Audit { ledger, session, sink, last_seq: inspection.resumed_from_seq };
    audit.emit(
        "session.resume",
        json!({
            "route": "api",
            "provenance": "in-core loop, full provenance",
            "leg": inspection.leg,
            "resumed_from_seq": inspection.resumed_from_seq,
            "prior_outcome": inspection.prior_outcome,
            "start_turn": inspection.start_turn,
            "provider": info.provider,
            "endpoint": info.endpoint,
            "api_key_env": info.api_key_env,
            "model_requested": cfg.model_requested,
            "model": cfg.model,
            "prompt": cfg.prompt,
            "project_context": auto_cfg.project_context,
            "workspace_revision": env.workspace_root.as_deref().and_then(crate::workspace_revision),
            "system_prompt": system_prompt,
            "max_turns": cfg.max_turns,
            "max_tokens": cfg.max_tokens,
            "policy_version": policy_version,
            "containment": cfg.containment,
            "contained": cfg.contained,
            "verification_mode": auto_cfg.guardrails.verification_mode,
            "calculation_mode": auto_cfg.guardrails.calculation_mode,
            "use_strategist": auto_cfg.use_strategist,
            "interactive_plan_mode": auto_cfg.interactive_plan_mode,
            "pause_after_explore": auto_cfg.pause_after_explore,
            "pinned_conventions": auto_cfg.guardrails.pinned_conventions,
            "history_messages": messages.len(),
            "tool_defs": tool_defs_json(&defs),
        }),
    )?;

    drive_autonomous_rounds(
        solver_transport,
        roles,
        router,
        env,
        &mut audit,
        &cfg.model,
        &system_prompt,
        cfg.max_turns,
        cfg.max_tokens,
        cfg.reasoning_effort.as_deref(),
        inspection.start_turn,
        messages,
        auto_cfg,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
fn drive_autonomous_rounds(
    solver_transport: &dyn Transport,
    roles: &AutonomousRoles<'_>,
    router: &Router,
    env: &ResearchEnv,
    audit: &mut Audit<'_>,
    solver_model: &str,
    system_prompt: &str,
    max_turns_budget: u32,
    max_tokens: u32,
    reasoning_effort: Option<&str>,
    initial_start_turn: u32,
    mut messages: Vec<Message>,
    auto_cfg: &AutonomousConfig,
    cancel: Option<&CancelToken>,
) -> Result<SessionOutcome, String> {
    let overall_started = Instant::now();
    let max_rounds = auto_cfg.max_verification_rounds.max(1);
    let mut effective_turn_budget = max_turns_budget;
    let mut turns_used = 0u32;
    let mut next_turn = initial_start_turn;
    let mut round = 0u32;
    let mut calc_repair_attempted = false;
    let mut latest_strategist_advice: Option<String> = None;

    loop {
        let remaining_turns = effective_turn_budget.saturating_sub(turns_used);
        if remaining_turns == 0 {
            let reason = format!("reached max_turns ({effective_turn_budget}) during autonomous verification");
            audit.emit("session.abort", json!({ "reason": reason }))?;
            return Ok(SessionOutcome::Aborted { turns: turns_used, reason });
        }
        let start_events = audit.ledger.read_session(audit.session).map_err(|e| e.to_string())?;
        let start_phase = infer_campaign_phase(&start_events);
        if !matches!(start_phase, CampaignPhase::Verify | CampaignPhase::Revise | CampaignPhase::Complete) {
            audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Synthesize.as_str() }))?;
        }
        let leg_start_seq = audit.last_seq;
        let leg: LegResult = run_turn_leg(
            solver_transport,
            router,
            audit,
            LoopParams {
                model: solver_model,
                system_prompt,
                max_turns: remaining_turns,
                max_tokens,
                reasoning_effort,
                start_turn: next_turn,
                initial_messages: messages,
                mask_keep_recent: Some((auto_cfg.guardrails.step_mask_keep, auto_cfg.guardrails.step_mask_stride)),
                cancel,
                emit_end_event: false,
                delegation: roles.delegation.as_ref(),
                human: roles.human,
                pause_after_explore: auto_cfg.pause_after_explore,
            },
        )?;

        let leg_turns = match &leg.outcome {
            SessionOutcome::Completed { turns, .. }
            | SessionOutcome::Refusal { turns, .. }
            | SessionOutcome::Aborted { turns, .. } => *turns,
        };
        turns_used += leg_turns;
        next_turn = leg.last_turn + 1;
        messages = leg.messages;

        let SessionOutcome::Completed { stop_reason, .. } = &leg.outcome else {
            return Ok(match leg.outcome {
                SessionOutcome::Refusal { text, .. } => SessionOutcome::Refusal { turns: turns_used, text },
                SessionOutcome::Aborted { reason, .. } => SessionOutcome::Aborted { turns: turns_used, reason },
                SessionOutcome::Completed { .. } => unreachable!(),
            });
        };

        let dag = read_proof_dag(audit.ledger, audit.session)?;
        let events = audit.ledger.read_session(audit.session).map_err(|e| e.to_string())?;
        let honesty = lint_honesty_with_ledger(&leg.last_text, &dag, &events, audit.ledger);
        audit.emit("honesty.lint", serde_json::to_value(&honesty).unwrap())?;
        if !honesty.passed {
            audit.sink.on_thinking_delta(&format!(
                "\n[Honesty Linter: {} finding(s) flagged]\n",
                honesty.findings.len()
            ));
        }

        if auto_cfg.guardrails.calculation_mode {
            if let Err(calc_err) = check_computation_interpretation(audit.ledger, audit.session) {
                if !calc_repair_attempted {
                    calc_repair_attempted = true;
                    if turns_used >= effective_turn_budget {
                        effective_turn_budget = turns_used + 2;
                    }
                    audit.sink.on_thinking_delta(
                        "\n[Calculation guardrail: symbolic computation needs an interpreting `result` entry — prompting Orchestrator...]\n",
                    );
                    let fb = format!(
                        "<guardrail_feedback>\n{calc_err}\n\
                         Please call `ledger_append` with `entry_type: \"result\"` (referencing the computation \
                         entry in `depends_on`) to interpret the calculation, append/update your `final_answer`, \
                         and state the complete final response.\n</guardrail_feedback>"
                    );
                    audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Revise.as_str() }))?;
                    inject_feedback(audit, &mut messages, 0, "calculation_mode", &fb)?;
                    continue;
                }
                audit.emit("guardrail.blocked", json!({ "reason": calc_err }))?;
                audit.emit("session.abort", json!({ "reason": calc_err }))?;
                return Ok(SessionOutcome::Aborted { turns: turns_used, reason: calc_err });
            }
        }

        if !auto_cfg.guardrails.verification_mode {
            audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Complete.as_str() }))?;
            audit.emit(
                "session.end",
                json!({
                    "outcome": "completed",
                    "stop_reason": stop_reason,
                    "num_turns": turns_used,
                    "last_turn": leg.last_turn,
                    "duration_ms": overall_started.elapsed().as_millis() as u64,
                }),
            )?;
            return Ok(SessionOutcome::Completed { turns: turns_used, stop_reason: stop_reason.clone() });
        }

        round += 1;
        if cancel.is_some_and(CancelToken::is_cancelled) {
            let reason = "stopped by user".to_string();
            audit.emit("session.abort", json!({ "reason": reason, "stopped": true }))?;
            return Ok(SessionOutcome::Aborted { turns: turns_used, reason });
        }

        let target_entry_id = match ensure_active_final_answer_entry(router, audit, leg_start_seq, round, &leg.last_text) {
            Ok(id) => id,
            Err(reason) => {
                audit.emit("session.abort", json!({ "reason": reason }))?;
                return Ok(SessionOutcome::Aborted { turns: turns_used, reason });
            }
        };

        let (v_transport, v_model) = roles.verifier.unwrap_or((solver_transport, solver_model));
        audit.sink.on_thinking_delta(&format!(
            "\n[Verifier round {round}/{max_rounds} ({v_model}): auditing active Proof DAG & LaTeX...]\n"
        ));

        audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Verify.as_str() }))?;
        let report = match verify_delivered_answer(
            v_transport,
            audit.ledger,
            env,
            audit.session,
            v_model,
            cancel,
            DeliveredAnswer { entry_id: &target_entry_id, text: &leg.last_text },
        ) {
            Ok(r) => r,
            Err(err) => {
                audit.flush_interleaved()?;
                if cancel.is_some_and(CancelToken::is_cancelled) || err.contains("stopped by user") {
                    let reason = "stopped by user".to_string();
                    audit.emit("session.abort", json!({ "reason": reason, "stopped": true }))?;
                    return Ok(SessionOutcome::Aborted { turns: turns_used, reason });
                }
                let reason = format!("verifier error: {err}");
                audit.emit("session.abort", json!({ "reason": reason }))?;
                return Ok(SessionOutcome::Aborted { turns: turns_used, reason });
            }
        };
        audit.flush_interleaved()?;

        if report.verdict == VerificationVerdict::Accepted {
            let current = read_proof_dag(audit.ledger, audit.session)?;
            let active_final = current.entries.iter().rev().find(|entry| {
                entry.entry_type == "final_answer" && entry.status != ProofStatus::Superseded
            });
            if active_final.is_none_or(|entry| entry.seq <= leg_start_seq || entry.entry_id != report.target_entry_id) {
                let reason = "The verified candidate is no longer the current final answer.".to_string();
                audit.emit("session.abort", json!({ "reason": reason }))?;
                return Ok(SessionOutcome::Aborted { turns: turns_used, reason });
            }
            check_final_answer_gate(audit.ledger, audit.session, true)?;
            if honesty.findings.iter().any(|finding| finding.severity == "warning") {
                let events = audit.ledger.read_session(audit.session).map_err(|e| e.to_string())?;
                let confirmed = lint_honesty_with_ledger(&leg.last_text, &current, &events, audit.ledger);
                audit.emit("honesty.lint", serde_json::to_value(&confirmed).unwrap())?;
            }
            audit.sink.on_thinking_delta(&format!(
                "[Verifier ACCEPTED `{}`: {}]\n",
                report.target_entry_id, report.summary
            ));
            audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Complete.as_str() }))?;
            audit.emit(
                "session.end",
                json!({
                    "outcome": "completed",
                    "stop_reason": stop_reason,
                    "verification_rounds": round,
                    "verified_entry_id": report.target_entry_id,
                    "num_turns": turns_used,
                    "last_turn": leg.last_turn,
                    "duration_ms": overall_started.elapsed().as_millis() as u64,
                }),
            )?;
            return Ok(SessionOutcome::Completed { turns: turns_used, stop_reason: stop_reason.clone() });
        }

        audit.sink.on_thinking_delta(&format!(
            "[Verifier REJECTED `{}`: {}]\n",
            report.target_entry_id, report.summary
        ));

        if round >= max_rounds {
            let latest_dag = read_proof_dag(audit.ledger, audit.session)?;
            let warn = format_unverified_banner(&report, round);
            let recovery = build_unverified_recovery(
                &report,
                round,
                &latest_dag,
                latest_strategist_advice.as_deref(),
            );
            audit.emit(
                "guardrail.warning",
                json!({
                    "rounds_attempted": round,
                    "target_entry_id": report.target_entry_id,
                    "summary": report.summary,
                    "banner": warn,
                    "recovery": recovery,
                }),
            )?;
            audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Complete.as_str() }))?;
            audit.emit(
                "session.end",
                json!({
                    "outcome": "completed_unverified",
                    "stop_reason": stop_reason,
                    "verification_rounds": round,
                    "num_turns": turns_used,
                    "last_turn": leg.last_turn,
                    "duration_ms": overall_started.elapsed().as_millis() as u64,
                }),
            )?;
            return Ok(SessionOutcome::Completed { turns: turns_used, stop_reason: "completed_unverified".into() });
        }

        if turns_used >= effective_turn_budget {
            effective_turn_budget = turns_used + REVISION_TURN_RESERVE;
            audit.sink.on_thinking_delta(&format!(
                "[Reserved +{REVISION_TURN_RESERVE} repair turns for verification round {}/{max_rounds}]\n",
                round + 1
            ));
        }

        let strategist_note = maybe_run_strategist(
            solver_transport,
            solver_model,
            roles,
            audit,
            auto_cfg,
            &report,
            cancel,
        )?;
        if strategist_note.is_some() {
            latest_strategist_advice = strategist_note.clone();
        }
        let feedback = build_revision_prompt(&report, round, max_rounds, strategist_note.as_deref(), &honesty);
        audit.emit("campaign.phase", json!({ "phase": CampaignPhase::Revise.as_str() }))?;
        inject_feedback(audit, &mut messages, round, &report.target_entry_id, &feedback)?;
    }
}
