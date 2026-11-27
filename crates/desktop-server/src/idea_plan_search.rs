//! Explore Router-enforced `idea_search` execution (package X7).
//!
//! Governance: `explore.md` 3, 4.1 (`SearchPlan`, `Coverage`), 5.2 (`idea_search`),
//! 6 (Rules P3, P5, P6, P7, P8).
//!
//! Every follow-up / model-composed search query executes STRICTLY through
//! `Router::try_call("host:idea:<session>", "idea_search", &args)` on `build_idea_router`
//! (`explore/1`) so the real policy engine, `UiApprover`, session grants, and ledger audit apply.
//! No direct bypass path exists (Rule P5).

pub use crate::idea_plan::{assemble_search_plan, handle_plan_op};

use crate::idea::search::{annotate_and_record_library, lit_paper_to_raw_hit};
use crate::idea::{
    budget_snapshot, latest_statement_seq, IdeaStage, EVENT_IDEA_BRIEF, EVENT_IDEA_CANDIDATES,
    EVENT_IDEA_SEARCH_FINISHED, EVENT_IDEA_SEARCH_PLANNED,
};
use crate::idea_exchanges::{emit_stage, latest_idea_statement};
use crate::idea_plan::{
    build_idea_search_args, collect_completed_qids, is_session_offline, library_counts,
    merge_with_prior_candidates, next_search_round, summarize_provider_states,
};
use crate::idea_view;
use crate::routers::build_idea_router;
use crate::routes_idea::ActiveExploreGuard;
use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::DesktopState;
use agent::CancelToken;
use ledger::Ledger;
use research_tools::idea_rank::{RankedCandidate, RankedServiceList, RawHit};
use research_tools::idea_tools::{IDEA_SEARCH, MAX_IDEA_SEARCH_QUERIES};
use research_tools::lit_providers::{LitPaper, LitProvider};
use research_tools::paper_key::PaperKey;
use research_tools::run_budget::ExploreStageState;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use tools::{CallOutcome, Router, EVENT_SCHEMA_VERSION};

/// Outcome of executing a planned Explore search batch through `Router::try_call`.
#[derive(Debug, Clone)]
pub enum SearchRunResult {
    Finished {
        round: u64,
        candidates: Vec<RankedCandidate>,
        executed_queries: usize,
        skipped_budget: usize,
    },
    Denied {
        reason: String,
    },
    StoppedAtCap {
        reason: String,
    },
    Cancelled,
}

/// Execute the latest planned Explore search strictly through `Router::try_call` (Rule P5).
#[allow(clippy::too_many_arguments)]
pub fn run_planned_search_with_router<F>(
    data_dir: &Path,
    ledger: &Ledger,
    router: &Router,
    session_id: &str,
    provider_filter: Option<LitProvider>,
    cancel: Option<&CancelToken>,
    mut on_progress: F,
) -> Result<SearchRunResult, String>
where
    F: FnMut(&str, Value),
{
    let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    if is_session_offline(&events) {
        return Err("Explore session is in offline (library-only) mode".to_string());
    }
    let stmt = latest_idea_statement(&events)?;
    let stmt_seq = stmt.seq;
    let seeds: Vec<PaperKey> = stmt
        .seeds
        .iter()
        .filter_map(|s| PaperKey::parse(s).ok())
        .collect();

    let plan_ev = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_SEARCH_PLANNED && e.seq > stmt_seq)
        .ok_or_else(|| "no `idea.search_planned` event found; call `/idea/plan` first".to_string())?;
    let round = plan_ev
        .payload
        .get("round")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| next_search_round(&events));
    let request_id = format!("search-r{round}");

    // When resuming after StoppedAtCap (not an explicit provider retry), skip qids already `ok`.
    let completed_qids: BTreeSet<String> = if provider_filter.is_none() {
        collect_completed_qids(&events, plan_ev.seq)
    } else {
        BTreeSet::new()
    };

    let all_planned: Vec<Value> = plan_ev
        .payload
        .get("queries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|q| {
            let prov_ok = match provider_filter {
                None => true,
                Some(target) => q
                    .get("provider")
                    .and_then(Value::as_str)
                    .and_then(LitProvider::parse)
                    == Some(target),
            };
            let qid = q.get("qid").and_then(Value::as_str).unwrap_or("");
            prov_ok && !completed_qids.contains(qid)
        })
        .collect();
    if all_planned.is_empty() {
        return Err(match provider_filter {
            Some(p) => format!("no planned queries found for provider `{p}`"),
            None => "all queries in the latest search plan have already succeeded".to_string(),
        });
    }

    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok(SearchRunResult::Cancelled);
    }

    // Rule P3: check F9 query budget cap BEFORE dispatching through the router.
    // Disabled (non-allowlisted) services make zero external requests and do not consume cap.
    let snap = budget_snapshot(&events, true);
    let remaining = snap.max_queries.saturating_sub(snap.queries_used) as usize;
    let has_allowlisted = all_planned.iter().any(|q| {
        q.get("provider")
            .and_then(Value::as_str)
            .and_then(LitProvider::parse)
            .is_some_and(LitProvider::is_allowlisted)
    });
    if remaining == 0 && has_allowlisted {
        let reason = format!(
            "Query budget cap ({}) reached before search round {round}",
            snap.max_queries
        );
        emit_stage(
            ledger,
            session_id,
            IdeaStage::Search,
            ExploreStageState::StoppedAtCap,
            &request_id,
            Some(&reason),
            &mut on_progress,
        )?;
        return Ok(SearchRunResult::StoppedAtCap { reason });
    }

    let mut to_run: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut allowlisted_budget_used = 0usize;
    for q in &all_planned {
        let is_allowlisted = q
            .get("provider")
            .and_then(Value::as_str)
            .and_then(LitProvider::parse)
            .is_some_and(LitProvider::is_allowlisted);
        if to_run.len() >= MAX_IDEA_SEARCH_QUERIES {
            skipped.push(q.clone());
        } else if is_allowlisted {
            if allowlisted_budget_used < remaining {
                allowlisted_budget_used += 1;
                to_run.push(q.clone());
            } else {
                skipped.push(q.clone());
            }
        } else {
            to_run.push(q.clone());
        }
    }

    emit_stage(
        ledger,
        session_id,
        IdeaStage::Search,
        ExploreStageState::Requested,
        &request_id,
        None,
        &mut on_progress,
    )?;

    // Preserve exact original `qid` even on retry so F12 `search_coverage_events` replaces
    // the failed query entry cleanly and retains planned `perspective_id` / `seed_lookup`.
    let tool_queries = build_idea_search_args(&to_run);
    let tool_args = json!({ "run_id": request_id, "queries": tool_queries });
    let caller = format!("host:idea:{session_id}");
    // Rule P5: every model-composed search goes strictly through Router::try_call.
    let outcome = router
        .try_call(&caller, IDEA_SEARCH, &tool_args)
        .map_err(|e| format!("router ledger error during idea_search: {e}"))?;

    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok(SearchRunResult::Cancelled);
    }

    let tool_output = match outcome {
        CallOutcome::Ok(val) => val,
        CallOutcome::Denied(reason) => {
            emit_stage(
                ledger,
                session_id,
                IdeaStage::Search,
                ExploreStageState::Cancelled,
                &request_id,
                Some(&reason),
                &mut on_progress,
            )?;
            return Ok(SearchRunResult::Denied { reason });
        }
        CallOutcome::Error(err) => {
            emit_stage(
                ledger,
                session_id,
                IdeaStage::Search,
                ExploreStageState::Failed,
                &request_id,
                Some(&err),
                &mut on_progress,
            )?;
            return Err(err);
        }
    };

    let result_rows = tool_output
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut ranked_lists: Vec<RankedServiceList> = Vec::new();
    let mut query_outcomes: Vec<Value> = Vec::new();
    let mut ok_count = 0usize;
    let mut attempted_allowlisted = 0usize;
    let mut provider_counts: BTreeMap<String, (usize, usize, usize, bool)> = BTreeMap::new();

    for (idx, row) in result_rows.iter().enumerate() {
        let qid = row.get("qid").and_then(Value::as_str).unwrap_or("").to_string();
        let prov = row.get("provider").and_then(Value::as_str).unwrap_or("").to_string();
        let q_text = row.get("q").and_then(Value::as_str).unwrap_or("").to_string();
        let status = row.get("status").and_then(Value::as_str).unwrap_or("failed").to_string();
        let returned = row.get("returned").and_then(Value::as_u64).unwrap_or(0) as usize;
        let pid = row
            .get("perspective_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                to_run
                    .get(idx)
                    .and_then(|pq| pq.get("perspective_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let pkind = to_run
            .get(idx)
            .and_then(|pq| pq.get("perspective_kind"))
            .and_then(Value::as_str)
            .map(str::to_string);

        if status != "disabled" && status != "not_configured" {
            attempted_allowlisted += 1;
        }
        let entry = provider_counts.entry(prov.clone()).or_insert((0, 0, 0, false));
        match status.as_str() {
            "ok" => {
                ok_count += 1;
                entry.0 += 1;
                if let Some(papers_arr) = row.get("papers").and_then(Value::as_array) {
                    let hits: Vec<RawHit> = papers_arr
                        .iter()
                        .filter_map(|v| serde_json::from_value::<LitPaper>(v.clone()).ok())
                        .map(|p| lit_paper_to_raw_hit(&p))
                        .collect();
                    if !hits.is_empty() {
                        ranked_lists.push(RankedServiceList {
                            qid: qid.clone(),
                            provider: prov.clone(),
                            perspective_id: pid.clone(),
                            hits,
                        });
                    }
                }
            }
            "rate_limited" => entry.2 += 1,
            "disabled" => entry.3 = true,
            _ => entry.1 += 1,
        }

        let outcome_row = json!({
            "qid": qid,
            "perspective_id": pid,
            "perspective_kind": pkind,
            "provider": prov,
            "q": q_text,
            "status": status,
            "http_status": row.get("http_status").cloned().unwrap_or(Value::Null),
            "count": returned,
            "hits": returned,
            "new_papers": row.get("new_papers").and_then(Value::as_u64).unwrap_or(0),
            "error": row.get("error").cloned().unwrap_or(Value::Null),
            "retry_after_secs": row.get("retry_after_secs").cloned().unwrap_or(Value::Null),
        });
        query_outcomes.push(outcome_row.clone());
        on_progress("idea.query_result", outcome_row);
    }

    for sk in &skipped {
        let sk_row = json!({
            "qid": sk.get("qid").and_then(Value::as_str).unwrap_or(""),
            "perspective_id": sk.get("perspective_id").cloned().unwrap_or(Value::Null),
            "perspective_kind": sk.get("perspective_kind").cloned().unwrap_or(Value::Null),
            "provider": sk.get("provider").and_then(Value::as_str).unwrap_or(""),
            "q": sk.get("q").and_then(Value::as_str).unwrap_or(""),
            "status": "skipped_budget",
            "http_status": Value::Null,
            "count": 0,
            "hits": 0,
        });
        query_outcomes.push(sk_row.clone());
        on_progress("idea.query_result", sk_row);
    }

    let mut candidates = merge_with_prior_candidates(&events, stmt_seq, &ranked_lists);
    annotate_and_record_library(data_dir, session_id, &seeds, &mut candidates);

    let (lib_total, lib_ft) = library_counts(data_dir);
    let lib_hits = candidates.iter().filter(|c| c.in_library).count();
    let providers_json = summarize_provider_states(provider_counts);

    // Emit BOTH `"results"` (X3 budget) and `"queries"` / `"providers"` / `"library"` (F12 coverage).
    let finished_payload = json!({
        "round": round,
        "offline": false,
        "results": query_outcomes,
        "queries": query_outcomes,
        "providers": providers_json,
        "library": {
            "papers": lib_total,
            "with_full_text": lib_ft,
            "hits": lib_hits,
        },
    });
    ledger
        .append(
            session_id,
            EVENT_IDEA_SEARCH_FINISHED,
            EVENT_SCHEMA_VERSION,
            &finished_payload,
        )
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_SEARCH_FINISHED, finished_payload);

    let cands_payload = json!({ "round": round, "candidates": candidates });
    ledger
        .append(
            session_id,
            EVENT_IDEA_CANDIDATES,
            EVENT_SCHEMA_VERSION,
            &cands_payload,
        )
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_CANDIDATES, cands_payload);

    let all_failed = attempted_allowlisted > 0 && ok_count == 0;
    let (final_state, reason) = if !skipped.is_empty() {
        (ExploreStageState::StoppedAtCap, Some("Query budget cap reached".to_string()))
    } else if all_failed {
        (ExploreStageState::Failed, Some("All services failed: no result is shown".to_string()))
    } else {
        (ExploreStageState::Finished, None)
    };
    emit_stage(
        ledger,
        session_id,
        IdeaStage::Search,
        final_state,
        &request_id,
        reason.as_deref(),
        &mut on_progress,
    )?;

    Ok(SearchRunResult::Finished {
        round,
        candidates,
        executed_queries: to_run.len(),
        skipped_budget: skipped.len(),
    })
}

/// HTTP handler for `POST /api/sessions/{id}/idea/search` (`explore.md` 5.4, package X7).
pub fn handle_search_op(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let req_val = if body.is_empty() { json!({}) } else { parse_json_body(body)? };
    let provider_filter = match req_val.get("provider").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::String(s)) => {
            Some(LitProvider::parse(s).ok_or_else(|| bad(format!("unknown provider `{s}`")))?)
        }
        Some(_) => return Err(bad("`provider` must be a string")),
    };

    let guard = ActiveExploreGuard::acquire(state, session_id)?;
    let mut env = ResearchEnv::new(&state.cfg.data_dir);
    env.network_endpoints = state.cfg.literature_endpoints.clone();
    let router = build_idea_router(Arc::clone(state), Arc::new(env), session_id).map_err(internal)?;

    {
        let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
        let existing = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
        let stmt_seq = latest_statement_seq(&existing);
        let latest_brief_seq = existing
            .iter()
            .rev()
            .find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > stmt_seq)
            .map(|e| e.seq)
            .unwrap_or(0);
        let baseline_seq = latest_brief_seq.max(stmt_seq);
        let has_recent_plan = existing
            .iter()
            .any(|e| e.kind == EVENT_IDEA_SEARCH_PLANNED && e.seq > baseline_seq);
        let has_explicit_queries = req_val.get("queries").is_some_and(|v| !v.is_null());
        if (has_explicit_queries || !has_recent_plan) && provider_filter.is_none() {
            let (_, plan_payload) = assemble_search_plan(&existing, &req_val)?;
            ledger
                .append(session_id, EVENT_IDEA_SEARCH_PLANNED, EVENT_SCHEMA_VERSION, &plan_payload)
                .map_err(|e| internal(e.to_string()))?;
            state.sse.emit(
                session_id,
                "session_event",
                json!({ "session": session_id, "kind": EVENT_IDEA_SEARCH_PLANNED, "payload": plan_payload }),
            );
        }
    }

    let sse = Arc::clone(&state.sse);
    let sid_sse = session_id.to_string();
    let res = run_planned_search_with_router(
        &state.cfg.data_dir,
        ledger,
        &router,
        session_id,
        provider_filter,
        Some(&guard.cancel),
        move |kind, payload| {
            sse.emit(
                &sid_sse,
                "session_event",
                json!({ "session": sid_sse, "kind": kind, "payload": payload }),
            );
        },
    );
    drop(guard);

    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let view = idea_view::view(&updated, false);
    match res {
        Ok(SearchRunResult::Finished { round, executed_queries, skipped_budget, .. }) => Ok((
            200,
            json!({
                "ok": true, "round": round,
                "executed_queries": executed_queries,
                "skipped_budget": skipped_budget,
                "idea": view,
            }),
        )),
        Ok(SearchRunResult::Denied { reason }) => Ok((
            200,
            json!({ "ok": false, "denied": true, "reason": reason, "idea": view }),
        )),
        Ok(SearchRunResult::StoppedAtCap { reason }) => Ok((
            200,
            json!({ "ok": false, "stopped_at_cap": true, "reason": reason, "idea": view }),
        )),
        Ok(SearchRunResult::Cancelled) => Ok((
            200,
            json!({ "ok": false, "cancelled": true, "idea": view }),
        )),
        Err(msg) => Err(bad(msg)),
    }
}
