//! Explicit single-step and answer-excerpt reviews. They share the session's turn exclusion
//! and cancellation handles, so starting a solver turn cannot change a review in flight.

use crate::routes_session::{bad, internal, not_found, ApiResult};
use crate::state::DesktopState;
use crate::turn_control::ApiTurnGuard;
use agent::verifiers::{review_answer_claim, review_step, AnswerClaim};
use research_tools::validate_session_id;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Target {
    Step { entry_id: String },
    Claim { answer_seq: u64, claim: String },
}

struct ReviewGuard<'a> {
    state: &'a DesktopState,
    session: &'a str,
}

impl Drop for ReviewGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.active_turns.lock() {
            active.remove(self.session);
            self.state.sse.emit(self.session, "claim_review_finished", json!({}));
        }
    }
}

pub(crate) fn handle_review(state: &Arc<DesktopState>, session: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session).map_err(|e| bad(e.to_string()))?;
    let target: Target = serde_json::from_slice(body).map_err(|_| bad("Choose one proof step or a passage from a saved answer."))?;
    if !state.session_exists(session).map_err(internal)? {
        return Err(not_found("Session not found."));
    }
    {
        let mut active = state.active_turns.lock().map_err(|_| internal("Session lock unavailable."))?;
        if !active.insert(session.to_string()) {
            return Err((409, "Wait for the current turn or review to finish.".into()));
        }
    }
    let _reservation = ReviewGuard { state, session };
    let worker = ApiTurnGuard::new(state, session).map_err(internal)?;
    let ledger = state.open_ledger().map_err(internal)?;
    let (_, events) = ledger.read_verified_session(session).map_err(|_| bad("The saved session could not be verified."))?;
    let answer = match &target {
        Target::Claim { answer_seq, claim } => {
            let (messages, _, _) = crate::session_view::reconstruct_session_view(&events);
            let message = messages.iter().find(|m| m["role"] == "assistant" && m["seq"].as_u64() == Some(*answer_seq)
                && m["narrator"] != true).ok_or_else(|| bad("The selected answer could not be found."))?;
            let text = message["text"].as_str().unwrap_or_default();
            if claim.trim().is_empty() || claim.chars().count() > 8192 || !text.contains(claim) {
                return Err(bad("Select an exact excerpt of up to 8192 characters from the saved answer."));
            }
            Some(text.to_owned())
        }
        Target::Step { entry_id } => {
            if !events.iter().any(|event| event.kind == "proof.entry" && event.payload["entry_id"].as_str() == Some(entry_id)) {
                return Err(not_found("This proof step no longer exists."));
            }
            None
        }
    };
    let (provider, model) = crate::agent_models::resolve_in_core_role_model(state, "step_verifier", None, true)
        .map_err(|(status, msg)| if msg == crate::agent_models::NEEDS_API_PROVIDER {
            bad("Claim review needs a connected model. Choose one in Settings → Providers or assign a Step Verifier in Agents & Models.")
        } else { (status, msg) })?;
    let transport = state.build_transport(provider, &model).map_err(bad)?;
    let env = state.research_env_for_session(session).map_err(bad)?;
    state.sse.emit(session, "claim_review_started", json!({ "model": model }));
    let report = match &target {
        Target::Step { entry_id } => review_step(&*transport, &ledger, &env, session, entry_id, &model, &worker.cancel),
        Target::Claim { claim, answer_seq } => review_answer_claim(&*transport, &ledger, &env, session,
            AnswerClaim { claim, answer: answer.as_deref().unwrap_or_default(), answer_seq: *answer_seq }, &model, &worker.cancel),
    }.map_err(bad)?;
    let mut result = serde_json::to_value(&report).map_err(|e| internal(e.to_string()))?;
    result["scope"] = json!(report.kind);
    result["model"] = json!(model);
    if let Target::Claim { answer_seq, claim } = target {
        result["claim"] = Value::String(claim);
        result["answer_seq"] = json!(answer_seq);
        result["answer_sha256"] = json!(ledger::sha256_hex(answer.unwrap_or_default().as_bytes()));
    }
    Ok((200, result))
}
