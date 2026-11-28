//! Solve multi-turn verifier loop with tools (`solve.md` 4.1, 4.3, 4.4, 5.2–5.4,
//! rules R3, R4, R13, G6, D2, D3; package S14).
//!
//! Runs under the stable caller id `"verifier:solve"` (`SOLVE_VERIFIER_CALLER`),
//! up to 12 model calls and 16,384 tokens per batch of 1..=5 target steps.
//! Spawns an isolated verifier Python worker lazily via [`IsolatedWorkerFactory`]
//! and drops it when the loop ends.

#[path = "verifier_finish.rs"]
mod finish;
#[path = "verifier_loop_support.rs"]
mod support;

pub use self::finish::{
    batch_review_targets, count_review_loops, handle_review_steps, run_finish_review_batches,
    select_finish_review_targets, FinishReviewSummary,
};
pub use self::support::build_solve_review_prompt;

use self::support::{
    entry_digest, pause_trigger_json, persist_solve_verdicts, solve_verifier_system_prompt,
    validate_report_verdict, validate_target_batch,
};
use crate::runner::EVENT_VERSION;
use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, Role, StopReason, StreamObserver, Transport,
};
use crate::verifier_parse::VerificationVerdict;
use crate::verifier_tools::{
    build_verifier_router, dispatch_verifier_registry_tool, verifier_model_tool_specs,
    IsolatedWorkerFactory, REPORT_VERDICT,
};
use ledger::Ledger;
use research_tools::policy_preset::SOLVE_VERIFIER_CALLER;
use research_tools::proof_closure::closure_entries;
use research_tools::solve_gates::detect_g6_verification_stuck;
use research_tools::solve_model::{compute_review_independence, ReviewIndependence, RoleModel};
use research_tools::solve_state::SolveState;
use research_tools::ResearchEnv;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tools::Approver;

/// Maximum model calls per verifier batch (`solve.md` 5.3).
pub const SOLVE_VERIFIER_MAX_MODEL_CALLS: u32 = 12;
/// Token ceiling per verifier model call (`solve.md` 5.3).
pub const SOLVE_VERIFIER_MAX_TOKENS: u32 = 16_384;
/// Maximum number of target steps reviewed in one verifier loop (`solve.md` 5.2 & 5.3).
pub const SOLVE_VERIFIER_MAX_BATCH_SIZE: usize = 5;
/// Default character budget for the serialized union closure prompt (deepest-first clipping).
pub const SOLVE_VERIFIER_CLOSURE_CHAR_BUDGET: usize = 48_000;
/// Event kind written when a verifier loop exhausts model calls without a valid verdict (`solve.md` 5.3).
pub const EVENT_VERIFIER_INCOMPLETE: &str = "verifier.incomplete";
/// `kind` tag recorded on Solve verifier ledger events (`verifier.request`, `proof.verification`, `verifier.incomplete`).
pub const SOLVE_VERIFIER_KIND: &str = "solve_review";

struct CancelOnlyObserver<'a>(Option<&'a CancelToken>);
impl StreamObserver for CancelOnlyObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_some_and(CancelToken::is_cancelled)
    }
}

/// Configuration for one Solve verifier loop invocation.
#[derive(Debug, Clone)]
pub struct SolveVerifierConfig {
    pub verifier_role: RoleModel,
    pub max_model_calls: u32,
    pub max_tokens: u32,
    pub closure_char_budget: usize,
    pub run_id: Option<String>,
}

impl SolveVerifierConfig {
    pub fn new(verifier_role: RoleModel) -> Self {
        Self {
            verifier_role,
            max_model_calls: SOLVE_VERIFIER_MAX_MODEL_CALLS,
            max_tokens: SOLVE_VERIFIER_MAX_TOKENS,
            closure_char_budget: SOLVE_VERIFIER_CLOSURE_CHAR_BUDGET,
            run_id: None,
        }
    }
}

/// Per-step verdict persisted as `proof.verification` by the Solve verifier loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveStepVerdictRecord {
    pub entry_id: String,
    pub verdict: VerificationVerdict,
    pub summary: String,
    pub counterexample: Option<String>,
    pub issues: Vec<Value>,
    pub checks_cited: Vec<String>,
    pub entry_sha256: String,
    pub event_seq: u64,
}

/// Outcome of one Solve verifier loop (`Completed` or `Incomplete` after exhausting model calls).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SolveReviewOutcome {
    Completed {
        verdicts: Vec<SolveStepVerdictRecord>,
        independence: ReviewIndependence,
        verifier_role: RoleModel,
        model_calls_used: u32,
        worker_spawned: bool,
        verification_stuck: Option<Value>,
    },
    Incomplete {
        target_entry_ids: Vec<String>,
        independence: ReviewIndependence,
        verifier_role: RoleModel,
        model_calls_used: u32,
        reason: String,
        event_seq: u64,
        worker_spawned: bool,
    },
}

impl SolveReviewOutcome {
    pub fn independence(&self) -> ReviewIndependence {
        match self {
            Self::Completed { independence, .. } | Self::Incomplete { independence, .. } => {
                *independence
            }
        }
    }
}

/// Borrowed host dependencies for running one or more Solve verifier batches.
pub struct SolveVerifierRunCtx<'a> {
    pub transport: &'a dyn Transport,
    pub ledger: &'a Ledger,
    pub env: Arc<ResearchEnv>,
    pub session: &'a str,
    pub config: &'a SolveVerifierConfig,
    pub approver: Arc<dyn Approver>,
    pub factory: &'a Arc<dyn IsolatedWorkerFactory>,
    pub cancel: Option<&'a CancelToken>,
}

/// Execute the Solve verifier loop over 1..=5 target steps (`solve.md` 5.3, rules R3 & R13).
pub fn run_solve_verifier_loop(
    ctx: &SolveVerifierRunCtx<'_>,
    target_entry_ids: &[String],
) -> Result<SolveReviewOutcome, String> {
    if ctx.config.verifier_role.model.trim().is_empty() {
        return Err("verifier model name must not be empty".into());
    }
    let targets = validate_target_batch(target_entry_ids)?;
    let events = ctx
        .ledger
        .read_session(ctx.session)
        .map_err(|e| format!("cannot read session `{}`: {e}", ctx.session))?;
    let state = SolveState::from_events(&events);
    let setup = state
        .setup
        .as_ref()
        .ok_or("cannot run Solve verifier before `solve.setup` is recorded")?
        .clone();

    let mut initial_shas: BTreeMap<String, String> = BTreeMap::new();
    for tid in &targets {
        let entry = state
            .entry(tid)
            .ok_or_else(|| format!("target step `{tid}` not found in session `{}`", ctx.session))?;
        if entry.superseded_by.is_some() {
            return Err(format!(
                "target step `{tid}` is superseded; review its replacement instead"
            ));
        }
        initial_shas.insert(tid.clone(), entry_digest(entry));
    }

    let target_refs: Vec<&str> = targets.iter().map(String::as_str).collect();
    let closure_list = closure_entries(&state.entries, &target_refs);
    let closure_set: BTreeSet<String> = closure_list.iter().map(|e| e.entry_id.clone()).collect();
    let initial_prompt = build_solve_review_prompt(
        &setup,
        &state,
        &targets,
        &closure_list,
        ctx.config.closure_char_budget,
    );

    let router_ledger = Ledger::open(&ctx.env.data_dir)
        .map_err(|e| format!("cannot open ledger for verifier router: {e}"))?;
    let (router, lazy_worker) = build_verifier_router(
        Arc::clone(&ctx.env),
        router_ledger,
        ctx.session,
        Arc::clone(&ctx.approver),
        ctx.factory,
    )?;
    let tool_specs = verifier_model_tool_specs(&router);
    let max_calls = ctx
        .config
        .max_model_calls
        .clamp(1, SOLVE_VERIFIER_MAX_MODEL_CALLS);
    let max_tokens = ctx.config.max_tokens.clamp(256, SOLVE_VERIFIER_MAX_TOKENS);
    let independence = compute_review_independence(&setup.roles.solver, &ctx.config.verifier_role);

    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: initial_prompt }],
    }];
    let mut model_calls_used = 0u32;
    let mut batch_id: Option<String> = None;
    let mut last_resp_blob = String::new();
    let mut incomplete_reason = format!(
        "verifier exhausted {max_calls} model call(s) without submitting a valid `report_verdict`"
    );

    let loop_result = (|| -> Result<SolveReviewOutcome, String> {
        while model_calls_used < max_calls {
            if ctx.cancel.is_some_and(CancelToken::is_cancelled) {
                let _ = ctx.ledger.append_with_hash(
                    ctx.session,
                    "verifier.cancelled",
                    EVENT_VERSION,
                    &json!({
                        "kind": SOLVE_VERIFIER_KIND,
                        "caller": SOLVE_VERIFIER_CALLER,
                        "target_entry_ids": targets,
                        "model_calls_used": model_calls_used,
                    }),
                );
                return Err("Solve review cancelled.".into());
            }

            let req = ModelRequest {
                model: ctx.config.verifier_role.model.clone(),
                system: Some(solve_verifier_system_prompt(&targets)),
                messages: messages.clone(),
                tools: tool_specs.clone(),
                max_tokens,
                reasoning_effort: None,
            };
            let prepared = ctx.transport.prepare_request(&req).map_err(|e| e.to_string())?;
            let req_blob = ctx
                .ledger
                .put_blob(&prepared.body)
                .map_err(|e| format!("verifier request put_blob failed: {e}"))?;

            model_calls_used += 1;
            let mut req_payload = json!({
                "kind": SOLVE_VERIFIER_KIND,
                "caller": SOLVE_VERIFIER_CALLER,
                "target_entry_ids": targets,
                "provider": ctx.config.verifier_role.provider,
                "model": ctx.config.verifier_role.model,
                "call_index": model_calls_used,
                "raw_blob": req_blob,
            });
            if let Some(run_id) = &ctx.config.run_id {
                req_payload["run_id"] = json!(run_id);
            }
            let req_seq = ctx
                .ledger
                .append_with_hash(ctx.session, "verifier.request", EVENT_VERSION, &req_payload)
                .map_err(|e| format!("failed to record verifier.request: {e}"))?
                .seq;
            let current_batch = batch_id.get_or_insert_with(|| format!("rv-{req_seq}")).clone();

            let mut obs = CancelOnlyObserver(ctx.cancel);
            let resp = ctx
                .transport
                .send_prepared_streaming(&prepared, &mut obs)
                .map_err(|e| e.to_string())?;
            last_resp_blob = ctx
                .ledger
                .put_blob(&resp.raw_body)
                .map_err(|e| format!("verifier response put_blob failed: {e}"))?;

            if ctx.cancel.is_some_and(CancelToken::is_cancelled) {
                let _ = ctx.ledger.append_with_hash(
                    ctx.session,
                    "verifier.cancelled",
                    EVENT_VERSION,
                    &json!({
                        "kind": SOLVE_VERIFIER_KIND,
                        "batch_id": current_batch,
                        "caller": SOLVE_VERIFIER_CALLER,
                        "target_entry_ids": targets,
                        "model_calls_used": model_calls_used,
                        "raw_blob": last_resp_blob,
                    }),
                );
                return Err("Solve review cancelled.".into());
            }

            if resp.stop_reason == StopReason::Refusal {
                incomplete_reason = "verifier model refused the review request".to_string();
                break;
            }

            messages.push(Message {
                role: Role::Assistant,
                content: resp.content.clone(),
            });

            let tool_uses: Vec<(String, String, Value)> = resp
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect();

            if tool_uses.is_empty() {
                if model_calls_used < max_calls {
                    messages.push(Message {
                        role: Role::User,
                        content: vec![ContentBlock::Text {
                            text: format!(
                                "You must submit your final judgements by calling `{REPORT_VERDICT}` \
                                 covering exactly these target step IDs: {}.",
                                targets.join(", ")
                            ),
                        }],
                    });
                }
                continue;
            }

            let mut tool_results: Vec<ContentBlock> = Vec::with_capacity(tool_uses.len());
            for (call_id, name, input) in tool_uses {
                if name == REPORT_VERDICT {
                    let fresh_events = ctx
                        .ledger
                        .read_session(ctx.session)
                        .map_err(|e| format!("cannot re-read session `{}`: {e}", ctx.session))?;
                    let fresh_state = SolveState::from_events(&fresh_events);
                    match validate_report_verdict(&fresh_state, &targets, &initial_shas, &input) {
                        Ok(validated) => {
                            let persisted = persist_solve_verdicts(
                                ctx.ledger,
                                ctx.session,
                                &fresh_state,
                                &validated,
                                independence,
                                &ctx.config.verifier_role,
                                model_calls_used,
                                &last_resp_blob,
                                &current_batch,
                            )?;
                            let after_events = ctx
                                .ledger
                                .read_session(ctx.session)
                                .map_err(|e| e.to_string())?;
                            let after_state = SolveState::from_events(&after_events);
                            let stuck = detect_g6_verification_stuck(&after_state, &after_events)
                                .map(pause_trigger_json);
                            return Ok(SolveReviewOutcome::Completed {
                                verdicts: persisted,
                                independence,
                                verifier_role: ctx.config.verifier_role.clone(),
                                model_calls_used,
                                worker_spawned: lazy_worker.was_spawned(),
                                verification_stuck: stuck,
                            });
                        }
                        Err(err) => {
                            incomplete_reason = format!("rejected `report_verdict`: {err}");
                            tool_results.push(ContentBlock::ToolResult {
                                tool_use_id: call_id,
                                content: json!({ "error": err }).to_string(),
                                is_error: true,
                            });
                        }
                    }
                } else {
                    match dispatch_verifier_registry_tool(&router, &name, &input, &closure_set) {
                        Ok(val) => tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: call_id,
                            content: val.to_string(),
                            is_error: false,
                        }),
                        Err(err) => tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: call_id,
                            content: json!({ "error": err }).to_string(),
                            is_error: true,
                        }),
                    }
                }
            }

            messages.push(Message {
                role: Role::User,
                content: tool_results,
            });
        }

        let final_batch = batch_id.unwrap_or_else(|| "rv-0".into());
        let incomplete_payload = json!({
            "kind": SOLVE_VERIFIER_KIND,
            "batch_id": final_batch,
            "caller": SOLVE_VERIFIER_CALLER,
            "target_entry_ids": targets,
            "provider": ctx.config.verifier_role.provider,
            "model": ctx.config.verifier_role.model,
            "model_calls_used": model_calls_used,
            "max_model_calls": max_calls,
            "reason": incomplete_reason,
            "raw_blob": last_resp_blob,
        });
        let seq = ctx
            .ledger
            .append_with_hash(
                ctx.session,
                EVENT_VERIFIER_INCOMPLETE,
                EVENT_VERSION,
                &incomplete_payload,
            )
            .map_err(|e| format!("failed to record {EVENT_VERIFIER_INCOMPLETE}: {e}"))?
            .seq;
        Ok(SolveReviewOutcome::Incomplete {
            target_entry_ids: targets,
            independence,
            verifier_role: ctx.config.verifier_role.clone(),
            model_calls_used,
            reason: incomplete_reason,
            event_seq: seq,
            worker_spawned: lazy_worker.was_spawned(),
        })
    })();

    lazy_worker.shutdown();
    drop(router);
    loop_result
}

#[cfg(test)]
#[path = "verifier_test_support.rs"]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "verifier_loop_tests.rs"]
mod tests;
