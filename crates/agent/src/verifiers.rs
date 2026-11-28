//! Stateless Step Verifier & Full-Solution Verifier (F12).
//!
//! Verifiers receive problem context, proof entries, deterministic audit results,
//! and the delivered answer for autonomous turns. Every `put_blob` is referenced by a
//! ledger event (`verifier.request` / `proof.verification`) carrying `"raw_blob"`.

use crate::guardrails::materialize_proof_entries;
use crate::runner::{validate_turn, EVENT_VERSION};
use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, Role, StopReason, StreamObserver, Transport,
};
use crate::verifier_context::{
    ancestor_entries, collect_ancestor_slice, format_entries_xml, format_linked_checks,
    load_context_and_entries, problem_context, run_candidate_tex_audit, xml_text,
};
use crate::verifier_parse::{parse_verifier_response, raw_verdict, verifier_system_prompt};
use ledger::{Event, Ledger};
use research_tools::{
    format_deterministic_tex_audit_xml, AuditReport, ProofEntry, ProofStatus, ResearchEnv,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

pub use crate::verifier_context::AnswerClaim;
pub use crate::verifier_parse::{VerificationIssue, VerificationVerdict};

const VERIFIER_MAX_TOKENS: u32 = 8192;

struct CancelOnlyObserver<'a>(Option<&'a CancelToken>);
impl StreamObserver for CancelOnlyObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_some_and(CancelToken::is_cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub target_entry_id: String,
    pub kind: String,
    pub verdict: VerificationVerdict,
    pub summary: String,
    pub counterexample: Option<String>,
    pub issues: Vec<VerificationIssue>,
    pub tex_audit: AuditReport,
    pub raw_blob: String,
    pub event_seq: u64,
}

struct VerifierParams<'a> {
    target_entry_id: &'a str,
    kind: &'a str,
    entries_slice: &'a [ProofEntry],
    problem_context: &'a str,
    events: &'a [Event],
    delivered_answer: Option<&'a str>,
    model: &'a str,
    cancel: Option<&'a CancelToken>,
    on_demand: bool,
    claim: Option<AnswerClaim<'a>>,
}

fn execute_verifier(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    params: VerifierParams<'_>,
) -> Result<VerificationReport, String> {
    let target_entry_id = params.target_entry_id;
    let kind = params.kind;
    let entries_slice = params.entries_slice;
    let problem_context = params.problem_context;
    let model = params.model;
    if model.trim().is_empty() {
        return Err("verifier model name must not be empty".into());
    }
    let selected_claim = params.claim.map(|claim| claim.context_xml()).transpose()?.unwrap_or_default();
    // Durable pre-call attempt barrier BEFORE contacting transport.
    let attempt_seq = ledger
        .append_with_hash(
            session,
            "proof.verification_attempt",
            EVENT_VERSION,
            &json!({
                "target_entry_id": target_entry_id,
                "kind": kind,
                "phase": "started",
            }),
        )
        .map_err(|e| format!("failed to record proof.verification_attempt: {e}"))?
        .seq;

    let tex_audit = run_candidate_tex_audit(env, ledger, session, entries_slice, params.delivered_answer, params.claim)?;
    let tex_audit_xml = format_deterministic_tex_audit_xml(&tex_audit);
    let linked_checks = if kind == "full" {
        let ancestors = collect_ancestor_slice(entries_slice, target_entry_id)?;
        format_linked_checks(ledger, params.events, &ancestors, false)
    } else {
        format_linked_checks(ledger, params.events, entries_slice, params.on_demand)
    };
    let system_prompt = verifier_system_prompt(kind);
    let delivered_answer_xml = params.delivered_answer.map_or_else(String::new, |answer| {
        format!("<final_answer_text>\n{}\n</final_answer_text>\n\n", xml_text(answer))
    });
    let user_prompt = format!(
        "target_entry_id: {target_entry_id}\nverification_kind: {kind}\n\n\
         {problem_context}\n\n\
         {delivered_answer_xml}{selected_claim}{linked_checks}{}\n\n{tex_audit_xml}",
        format_entries_xml(entries_slice)
    );

    let req = ModelRequest {
        model: model.to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }],
        }],
        tools: Vec::new(),
        max_tokens: VERIFIER_MAX_TOKENS,
        reasoning_effort: None,
    };
    let prepared = transport.prepare_request(&req).map_err(|e| e.to_string())?;
    let req_blob = ledger
        .put_blob(&prepared.body)
        .map_err(|e| format!("verifier request put_blob failed: {e}"))?;
    ledger
        .append_with_hash(
            session,
            "verifier.request",
            EVENT_VERSION,
            &json!({
                "target_entry_id": target_entry_id,
                "kind": kind,
                "model": model,
                "raw_blob": req_blob,
            }),
        )
        .map_err(|e| format!("failed to record verifier.request: {e}"))?;

    let mut obs = CancelOnlyObserver(params.cancel);
    if obs.is_cancelled() {
        ledger.append_with_hash(session, "verifier.cancelled", EVENT_VERSION,
            &json!({"kind": kind, "attempt_seq": attempt_seq, "phase": "before_send"})).map_err(|e| e.to_string())?;
        return Err("Review cancelled before the model request.".into());
    }
    let resp = transport.send_prepared_streaming(&prepared, &mut obs).map_err(|e| e.to_string())?;
    let resp_blob = ledger
        .put_blob(&resp.raw_body)
        .map_err(|e| format!("verifier response put_blob failed: {e}"))?;

    let text: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let parsed = raw_verdict(&text);
    if params.on_demand && (!matches!(resp.stop_reason, StopReason::EndTurn | StopReason::StopSequence)
        || validate_turn(&resp).is_some()
        || !parsed.as_ref().is_some_and(|p| p.verdict.as_deref().is_some_and(|v|
            v.eq_ignore_ascii_case("accepted") || v.eq_ignore_ascii_case("rejected"))
            && p.summary.as_ref().is_some_and(|s| !s.trim().is_empty()))) {
        ledger.append_with_hash(session, "verifier.incomplete", EVENT_VERSION,
            &json!({"model": model, "kind": kind, "raw_blob": resp_blob, "attempt_seq": attempt_seq}))
            .map_err(|e| e.to_string())?;
        return Err("The verifier did not return a complete review. No verdict was recorded.".into());
    }
    if params.cancel.is_some_and(CancelToken::is_cancelled) {
        ledger.append_with_hash(session, "verifier.cancelled", EVENT_VERSION,
            &json!({"kind": kind, "raw_blob": resp_blob, "attempt_seq": attempt_seq})).map_err(|e| e.to_string())?;
        return Err("Review cancelled.".into());
    }
    let (mut verdict, mut summary, counterexample, mut issues) = match resp.stop_reason {
        StopReason::Refusal => (
            VerificationVerdict::Rejected,
            "Verifier refused to validate the proof.".to_string(),
            None,
            Vec::new(),
        ),
        StopReason::EndTurn | StopReason::StopSequence if validate_turn(&resp).is_none() => {
            parse_verifier_response(&text, parsed)
        }
        _ => (
            VerificationVerdict::Rejected,
            format!("Verifier did not deliver a complete verdict (stop reason: {}).", resp.stop_reason.as_str()),
            None,
            Vec::new(),
        ),
    };

    if verdict == VerificationVerdict::Accepted && params.delivered_answer.is_some() {
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        if events.iter().any(|ev| {
            ev.seq > attempt_seq
                && matches!(ev.kind.as_str(), "proof.entry" | "proof.supersede" | "session.start" | "session.resume")
        }) {
            verdict = VerificationVerdict::Rejected;
            summary = "The proof changed during verification. Verify the current answer again.".to_string();
        }
    }

    if tex_audit.blocker_count > 0 {
        verdict = VerificationVerdict::Rejected;
        summary = format!(
            "Blocked by deterministic tex_audit ({} blocker(s)): {summary}",
            tex_audit.blocker_count
        );
        for issue in tex_audit.issues.iter().filter(|i| i.severity.as_str() == "blocker") {
            issues.push(VerificationIssue {
                severity: "blocker".into(),
                message: issue.message.clone(),
            });
        }
    }

    let mut payload = json!({
        "target_entry_id": target_entry_id,
        "kind": kind,
        "verdict": verdict.as_str(),
        "summary": summary,
        "counterexample": counterexample,
        "issues": issues,
        "raw_blob": resp_blob,
    });
    if let Some(answer) = params.delivered_answer {
        payload["answer_sha256"] = json!(ledger::sha256_hex(answer.as_bytes()));
    }
    if params.on_demand {
        payload["on_demand"] = json!(true);
        payload["scope"] = json!(kind);
        payload["model"] = json!(model);
    }
    if let Some(c) = params.claim {
        payload["answer_seq"] = json!(c.answer_seq);
        payload["answer_sha256"] = json!(ledger::sha256_hex(c.answer.as_bytes()));
        payload["claim"] = json!(c.claim);
        payload["cited_steps"] = json!(c.cited_steps);
        payload["occurrence"] = json!(c.occurrence);
    }
    let event_seq = ledger
        .append_with_hash(
            session,
            if params.claim.is_some() { "claim.review" } else { "proof.verification" },
            EVENT_VERSION,
            &payload,
        )
        .map_err(|e| format!("failed to record proof.verification: {e}"))?
        .seq;

    Ok(VerificationReport {
        target_entry_id: target_entry_id.to_string(),
        kind: kind.to_string(),
        verdict,
        summary,
        counterexample,
        issues,
        tex_audit,
        raw_blob: resp_blob,
        event_seq,
    })
}

/// Stateless step verifier: isolates `entry_id` + its `depends_on` ancestors
/// and runs deterministic `tex_audit` before invoking the verifier model.
pub fn verify_step(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    entry_id: &str,
    model: &str,
) -> Result<VerificationReport, String> {
    verify_step_inner(transport, ledger, env, session, entry_id, model, None)
}

/// On-demand review uses the problem and conventions in effect when this step was written.
pub fn review_step(
    transport: &dyn Transport, ledger: &Ledger, env: &ResearchEnv, session: &str,
    entry_id: &str, model: &str, cancel: &CancelToken,
) -> Result<VerificationReport, String> {
    verify_step_inner(transport, ledger, env, session, entry_id, model, Some(cancel))
}

fn verify_step_inner(
    transport: &dyn Transport, ledger: &Ledger, env: &ResearchEnv, session: &str,
    entry_id: &str, model: &str, cancel: Option<&CancelToken>,
) -> Result<VerificationReport, String> {
    let (latest_context, entries, events) = load_context_and_entries(ledger, session)?;
    let slice = collect_ancestor_slice(&entries, entry_id)?;
    let context = if cancel.is_some() {
        let target = slice.iter().find(|e| e.entry_id == entry_id).ok_or("Proof step not found")?;
        if target.status == ProofStatus::Superseded {
            return Err("This proof step was superseded. Review its replacement.".into());
        }
        problem_context(&events[..events.partition_point(|e| e.seq <= target.seq)])
    } else { latest_context };
    execute_verifier(
        transport,
        ledger,
        env,
        session,
        VerifierParams {
            target_entry_id: entry_id,
            kind: "step",
            entries_slice: &slice,
            problem_context: &context,
            events: &events,
            delivered_answer: None,
            model,
            cancel,
            claim: None,
            on_demand: cancel.is_some(),
        },
    )
}

pub fn review_answer_claim(
    transport: &dyn Transport, ledger: &Ledger, env: &ResearchEnv, session: &str,
    selection: AnswerClaim<'_>, model: &str, cancel: &CancelToken,
) -> Result<VerificationReport, String> {
    if selection.claim.trim().is_empty() || selection.claim.chars().count() > 8192 {
        return Err("Select an exact excerpt of up to 8192 characters from the saved answer.".into());
    }
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let history = &events[..events.partition_point(|e| e.seq <= selection.answer_seq)];
    let context = problem_context(history);
    let historical_entries = materialize_proof_entries(history);
    let tokens: BTreeSet<_> = selection.claim.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_').collect();
    let mut cited = Vec::new();
    for id in selection.cited_steps {
        if !tokens.contains(id.as_str()) || !historical_entries.iter().any(|entry| entry.entry_id == *id) {
            return Err(format!("Proof reference {id} was not present in this claim when the answer was written."));
        }
        cited.push(id.as_str());
    }
    let evidence = ancestor_entries(&historical_entries, &cited);
    execute_verifier(transport, ledger, env, session, VerifierParams {
        target_entry_id: "", kind: "claim", entries_slice: &evidence, problem_context: &context,
        events: history, delivered_answer: None, model, cancel: Some(cancel), on_demand: true, claim: Some(selection),
    })
}

/// Stateless full-solution verifier: slims superseded entries out of the proof DAG
/// and enforces deterministic `tex_audit` blockers over the entire active proof.
pub fn verify_full_solution(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    model: &str,
) -> Result<VerificationReport, String> {
    verify_full_solution_cancellable(transport, ledger, env, session, model, None)
}

pub fn verify_full_solution_cancellable(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    model: &str,
    cancel: Option<&CancelToken>,
) -> Result<VerificationReport, String> {
    verify_full(transport, ledger, env, session, model, cancel, None)
}

pub(crate) struct DeliveredAnswer<'a> {
    pub(crate) entry_id: &'a str,
    pub(crate) text: &'a str,
}

pub(crate) fn verify_delivered_answer(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    model: &str,
    cancel: Option<&CancelToken>,
    answer: DeliveredAnswer<'_>,
) -> Result<VerificationReport, String> {
    if answer.text.trim().is_empty() {
        return Err("the solver produced no final answer text to verify".to_string());
    }
    verify_full(transport, ledger, env, session, model, cancel, Some(answer))
}

fn verify_full(
    transport: &dyn Transport,
    ledger: &Ledger,
    env: &ResearchEnv,
    session: &str,
    model: &str,
    cancel: Option<&CancelToken>,
    delivered_answer: Option<DeliveredAnswer<'_>>,
) -> Result<VerificationReport, String> {
    let (problem_context, entries, events) = load_context_and_entries(ledger, session)?;
    let active: Vec<ProofEntry> = entries
        .into_iter()
        .filter(|e| e.status != ProofStatus::Superseded)
        .collect();
    let latest_final = active.iter().rev().find(|e| e.entry_type == "final_answer");
    if let Some(answer) = &delivered_answer {
        if latest_final.is_none_or(|entry| entry.entry_id != answer.entry_id) {
            return Err(format!("final answer `{}` is no longer the active candidate", answer.entry_id));
        }
    }
    let target_entry_id = latest_final
        .or_else(|| active.last())
        .map(|e| e.entry_id.as_str())
        .ok_or_else(|| format!("session `{session}` has no active proof entries to verify"))?;

    execute_verifier(
        transport,
        ledger,
        env,
        session,
        VerifierParams {
            target_entry_id,
            kind: "full",
            entries_slice: &active,
            problem_context: &problem_context,
            events: &events,
            delivered_answer: delivered_answer.map(|answer| answer.text),
            model,
            cancel,
            claim: None,
            on_demand: false,
        },
    )
}
