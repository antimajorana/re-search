//! Explore Conversation turns on frozen profile `explore/1` (package X15).
//!
//! Governance: `00-overview.md` 2.2 (`explore/1`), 2.4 (F3), 2.5 (F4), 3.2 row 20 (`"matched"`,
//! never `"verified"`), 3.3 (F9), 8.2 decision 1; `explore.md` 3 (`Afterwards`), 5.2, 6 (Rules
//! C1, C2, C7, P3, P5, P6, P7, S2, S8), 8 (items 3-4), 9 (row X15).

pub use crate::idea_chat_cites::{
    chat_citations_from_events, check_explore_answer_citations, parse_multi_paper_line_citations,
    CheckedPaperCitation, ExploreChatCitationReport, ParsedPaperCitation, CHAT_CITE_STATUS_MATCHED,
    CHAT_CITE_STATUS_UNREAD, EVENT_IDEA_CHAT_CITATIONS,
};

use crate::idea::{budget_snapshot, latest_statement_seq};
use crate::idea_system_prompt::{
    build_explore_chat_system_prompt, build_selected_node_turn_context,
};
use crate::routers::build_idea_router;
use crate::session_mode::session_mode;
use crate::session_view::SseEventSink;
use crate::state::DesktopState;
use agent::{
    inspect_session_for_resume_with_profile, resume_autonomous_research_loop,
    run_autonomous_research_loop, AgentConfig, AutonomousConfig, AutonomousRoles, CancelToken,
    GuardrailConfig, ResumeConfig, SessionOutcome, Transport, DEFAULT_MAX_TOKENS,
    DEFAULT_MAX_TURNS,
};
use ledger::Event;
use policy::EffectClass;
use research_tools::paper_key::Mode;
use research_tools::tool_profile::PROFILE_EXPLORE_V1;
use serde_json::{json, Value};
use std::sync::Arc;
use tools::EVENT_SCHEMA_VERSION;

/// Refusal message when a bare Explore session has no `idea.stated` event yet.
pub const EXPLORE_CHAT_STATEMENT_REQUIRED_MSG: &str =
    "Explore session requires an idea statement before running a conversation turn.";

/// Refusal message when an Explore Conversation turn targets a CLI route before X18.
pub const EXPLORE_CHAT_CLI_UNSUPPORTED_MSG: &str =
    "Explore Conversation turns require an API or ChatGPT provider until CLI turns land in X18.";

fn is_cli_provider_str(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "claude-cli" | "antigravity"
    )
}

/// Pre-turn synchronous guard invoked by `turn_runner.rs` before reserving/spawning an Explore
/// Conversation turn (`POST /api/sessions/{id}/messages`).
pub fn validate_explore_turn_preconditions(
    state: &DesktopState,
    session_id: &str,
    pre_events: &[Event],
    req: &Value,
) -> Result<(), (u16, String)> {
    if latest_statement_seq(pre_events) == 0 {
        return Err((400, EXPLORE_CHAT_STATEMENT_REQUIRED_MSG.to_string()));
    }
    let meta = state.load_meta(session_id);
    let ui = state.load_ui_settings();
    if meta
        .as_ref()
        .and_then(|m| m.route.as_deref())
        .is_some_and(is_cli_provider_str)
    {
        return Err((400, EXPLORE_CHAT_CLI_UNSUPPORTED_MSG.to_string()));
    }
    let req_prov = req
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let effective_prov = req_prov
        .or_else(|| {
            meta.as_ref()
                .and_then(|m| m.default_provider.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            ui.agent_models
                .get("orchestrator")
                .map(|r| r.provider.trim())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            ui.default_provider
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        });
    if effective_prov.is_some_and(is_cli_provider_str) {
        return Err((400, EXPLORE_CHAT_CLI_UNSUPPORTED_MSG.to_string()));
    }
    let budget = budget_snapshot(pre_events, false);
    if budget.model_calls.is_exhausted() {
        return Err((
            400,
            format!(
                "Explore model-call budget exhausted ({} of {} model calls used); raise the budget via Continue before starting another conversation turn.",
                budget.model_calls.used, budget.model_calls.cap
            ),
        ));
    }
    Ok(())
}

/// Desktop server entry point invoked from `turn_runner.rs` for `Mode::Explore` turns.
pub fn run_desktop_explore_turn(
    state: &Arc<DesktopState>,
    session_id: &str,
    req: &Value,
    prompt: &str,
    cancel: &CancelToken,
) -> Result<String, String> {
    if cancel.is_cancelled() {
        return Err("stopped by user".into());
    }
    let meta = state.load_meta(session_id);
    let ui = state.load_ui_settings();
    let (reasoning_effort, subagent) =
        crate::subagents::resolve_turn_options(state, session_id, req, meta.clone());
    let (provider, model_requested, model) = crate::agent_models::resolve_orchestrator_model(
        subagent.as_ref(),
        req,
        meta.as_ref(),
        &ui,
    )?;
    let transport = state.build_transport(provider, &model)?;
    run_explore_chat_turn_with_transport(
        state,
        session_id,
        req,
        prompt,
        &*transport,
        model_requested,
        model,
        reasoning_effort,
        cancel,
    )
}

/// Execute one Explore Conversation turn against an explicit [`Transport`] (used both by
/// `run_desktop_explore_turn` and scripted-transport integration tests).
#[allow(clippy::too_many_arguments)]
pub fn run_explore_chat_turn_with_transport(
    state: &Arc<DesktopState>,
    session_id: &str,
    req: &Value,
    prompt: &str,
    transport: &dyn Transport,
    model_requested: String,
    model: String,
    reasoning_effort: Option<String>,
    cancel: &CancelToken,
) -> Result<String, String> {
    if cancel.is_cancelled() {
        return Err("stopped by user".into());
    }
    let ledger = state.open_ledger()?;
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    if session_mode(&existing) != Mode::Explore {
        return Err("Explore conversation turn refused: session is not in Explore mode".into());
    }
    if latest_statement_seq(&existing) == 0 {
        return Err(EXPLORE_CHAT_STATEMENT_REQUIRED_MSG.to_string());
    }

    // Rule P3: enforce F9 model-call budget before spending any provider call.
    let budget = budget_snapshot(&existing, false);
    if budget.model_calls.is_exhausted() {
        return Err(format!(
            "Explore model-call budget exhausted ({} of {} model calls used); raise the budget via Continue before starting another conversation turn.",
            budget.model_calls.used, budget.model_calls.cap
        ));
    }
    let req_max_turns = req
        .get("max_turns")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MAX_TURNS);
    let remaining_turns = u32::try_from(budget.model_calls.remaining).unwrap_or(u32::MAX);
    let max_turns = req_max_turns.min(remaining_turns).max(1);
    let max_tokens = req
        .get("max_tokens")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MAX_TOKENS);

    let env = Arc::new(state.research_env_for_session(session_id)?);
    let router = build_idea_router(Arc::clone(state), Arc::clone(&env), session_id)?;
    if router.tool_profile() != Some(PROFILE_EXPLORE_V1) {
        return Err(format!(
            "Explore conversation router must bind `{PROFILE_EXPLORE_V1}`"
        ));
    }
    for def in router.registry().defs() {
        if matches!(def.effect, EffectClass::ExecuteCode | EffectClass::WriteWorkspace) {
            return Err(format!(
                "Rule P7 violation: explore/1 conversation router must not expose `{}`",
                def.id
            ));
        }
    }

    let selected_node_id = req
        .get("node_id")
        .or_else(|| req.get("idea_node_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let roles = AutonomousRoles::default();
    let mut sink = SseEventSink::new(Arc::clone(&state.sse), session_id).with_model_title(false);
    let has_started = existing.iter().any(|e| e.kind == "session.start");
    let containment = "explore host".to_string();

    let outcome = if !has_started {
        let system_prompt = build_explore_chat_system_prompt(&existing, selected_node_id);
        let auto_cfg = AutonomousConfig {
            guardrails: GuardrailConfig::default(),
            project_context: None,
            ..AutonomousConfig::default()
        };
        let cfg = AgentConfig {
            prompt: prompt.to_string(),
            model_requested,
            model,
            system_prompt,
            max_turns,
            max_tokens,
            containment,
            contained: false,
            reasoning_effort,
        };
        run_autonomous_research_loop(
            transport,
            &roles,
            &router,
            &ledger,
            &env,
            &cfg,
            &auto_cfg,
            Some(cancel),
            &mut sink,
        )?
    } else {
        let defs = router.registry().defs();
        let expected_defs: Vec<tools::ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
        inspect_session_for_resume_with_profile(
            &ledger,
            session_id,
            router.policy().version.as_str(),
            &expected_defs,
            router.tool_profile(),
        )
        .map_err(|e| e.to_string())?;
        let turn_node_context =
            selected_node_id.and_then(|nid| build_selected_node_turn_context(&existing, nid));
        let auto_cfg = AutonomousConfig {
            guardrails: GuardrailConfig::default(),
            project_context: turn_node_context,
            ..AutonomousConfig::default()
        };
        let cfg = ResumeConfig {
            prompt: prompt.to_string(),
            model_requested,
            model,
            system_prompt: None,
            max_turns,
            max_tokens,
            containment,
            contained: false,
            reasoning_effort,
        };
        resume_autonomous_research_loop(
            transport,
            &roles,
            &router,
            &ledger,
            &env,
            &cfg,
            &auto_cfg,
            Some(cancel),
            &mut sink,
        )?
    };

    match outcome {
        SessionOutcome::Aborted { reason, .. } => Err(reason),
        SessionOutcome::Completed { .. }
        | SessionOutcome::Refusal { .. }
        | SessionOutcome::Paused { .. } => {
            let reply = sink.final_text().to_string();
            let post_events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
            let caller = format!("agent:{}:{}", transport.info().provider, session_id);
            let turn_seq = post_events
                .iter()
                .rev()
                .find(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"))
                .map_or(0, |e| e.seq);
            let report =
                check_explore_answer_citations(Some(&ledger), &post_events, &caller, &reply);
            let payload = json!({
                "turn_seq": turn_seq,
                "caller": report.caller,
                "node_id": selected_node_id,
                "matched_count": report.matched_count,
                "unread_count": report.unread_count,
                "citations": report.citations,
            });
            ledger
                .append(
                    session_id,
                    EVENT_IDEA_CHAT_CITATIONS,
                    EVENT_SCHEMA_VERSION,
                    &payload,
                )
                .map_err(|e| format!("failed to record `{EVENT_IDEA_CHAT_CITATIONS}`: {e}"))?;
            state.sse.emit(
                session_id,
                "session_event",
                json!({
                    "session": session_id,
                    "kind": EVENT_IDEA_CHAT_CITATIONS,
                    "payload": payload,
                }),
            );
            Ok(reply)
        }
    }
}
