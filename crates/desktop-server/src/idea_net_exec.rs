//! Shared Rule-P3/P5/P6 Router-enforced batch executor for Explore citation expansion,
//! `"Who cited this since"`, `"Search more about this node"`, and Thorough Round 2 (package X16).
//!
//! Never overwrites Round-1 `IdeaStage::Search` or `idea.search_planned` and marks appended
//! `idea.candidates` with `"origin": "app"` so completed Triage/Read/Map sessions never regress.

use crate::idea::search::{annotate_and_record_library, lit_paper_to_raw_hit};
use crate::idea::{
    budget_snapshot, EVENT_IDEA_CANDIDATES, EVENT_IDEA_CITATION_NET, EVENT_IDEA_SEARCH_FINISHED,
};
use crate::idea_exchanges::latest_idea_statement;
use crate::idea_plan::{
    build_idea_search_args, is_session_offline, library_counts, merge_with_prior_candidates,
    next_search_round, summarize_provider_states,
};
use crate::routes_idea_net::{CitedSinceRequest, CITED_SINCE_NOT_ANSWERED_NOTE};
use agent::CancelToken;
use ledger::Ledger;
use research_tools::idea_net::{
    build_from_events, supported_citation_providers, CitationNetwork, CitationPaperMeta,
};
use research_tools::idea_rank::RankedServiceList;
use research_tools::idea_tools::{IDEA_SEARCH, MAX_IDEA_SEARCH_QUERIES};
use research_tools::lit_providers::{CitationEdge, LitPaper, LitProvider};
use research_tools::paper_key::PaperKey;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use tools::{CallOutcome, Router, EVENT_SCHEMA_VERSION};

/// Metadata describing an app-originated citation or Round-2 follow-up search batch.
#[derive(Debug, Clone)]
pub(crate) struct AppSearchBatchMeta {
    pub kind: &'static str,
    pub target_key: Option<PaperKey>,
    pub node_id: Option<String>,
    pub since_year: Option<i32>,
    pub is_open_question: bool,
    pub extra_seeds: Vec<PaperKey>,
    pub hops: usize,
    pub round2: bool,
    pub thin_statements: Vec<Value>,
}

/// Outcome of executing an app-originated `idea_search` batch through `Router::try_call`.
#[derive(Debug, Clone)]
pub enum NetBatchOutcome {
    Finished {
        round: u64,
        executed_queries: usize,
        skipped_budget: usize,
        stopped_at_cap: bool,
        citation_net: CitationNetwork,
        citing_papers: Vec<Value>,
    },
    Denied { reason: String },
    StoppedAtCap { reason: String },
    Cancelled,
}

/// Run `"Who cited this since"` (`origin: "app"`, 0 model calls, `relation: cited_by`)
/// strictly through `Router::try_call("host:idea:<session>", "idea_search", ...)`.
pub fn run_cited_since_with_router<F>(
    data_dir: &Path,
    ledger: &Ledger,
    router: &Router,
    session_id: &str,
    req: &CitedSinceRequest,
    cancel: Option<&CancelToken>,
    on_progress: F,
) -> Result<NetBatchOutcome, String>
where
    F: FnMut(&str, Value),
{
    run_cited_since_with_lock(data_dir, ledger, None, router, session_id, req, cancel, on_progress)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_cited_since_with_lock<F>(
    data_dir: &Path,
    ledger: &Ledger,
    write_lock: Option<&Mutex<()>>,
    router: &Router,
    session_id: &str,
    req: &CitedSinceRequest,
    cancel: Option<&CancelToken>,
    on_progress: F,
) -> Result<NetBatchOutcome, String>
where
    F: FnMut(&str, Value),
{
    let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    if is_session_offline(&events) {
        return Err("Explore session is in offline (library-only) mode".to_string());
    }
    if req.providers.is_empty() {
        return Err("at least one citation provider is required".to_string());
    }
    let supported = supported_citation_providers(&req.target_key, "cited_by");
    for &p in &req.providers {
        if !p.is_allowlisted() {
            return Err(format!("provider `{p}` is disabled and cannot run `cited_by` lookups"));
        }
        if !supported.contains(&p) {
            return Err(format!(
                "provider `{p}` does not support `cited_by` lookup for `{}`",
                req.target_key
            ));
        }
    }

    let round = next_search_round(&events);
    let planned_queries: Vec<Value> = req
        .providers
        .iter()
        .enumerate()
        .map(|(idx, prov)| {
            json!({
                "qid": format!("q-cite-r{round}-{}", idx + 1),
                "perspective_id": Value::Null,
                "perspective_kind": Value::Null,
                "provider": prov.as_str(),
                "q": "",
                "relation": { "kind": "cited_by", "key": req.target_key.as_str() },
                "max_results": req.max_results,
                "offset": 0,
                "origin": "app",
            })
        })
        .collect();

    let meta = AppSearchBatchMeta {
        kind: "cited_since",
        target_key: Some(req.target_key.clone()),
        node_id: req.node_id.clone(),
        since_year: req.since_year,
        is_open_question: req.is_open_question,
        extra_seeds: vec![req.target_key.clone()],
        hops: 1,
        round2: false,
        thin_statements: Vec::new(),
    };
    execute_app_search_batch(data_dir, ledger, write_lock, router, session_id, planned_queries, meta, cancel, on_progress)
}

/// Shared Rule-P3/P5/P6 executor for all app-originated citation/follow-up batches.
/// Holds `write_lock` strictly around ledger writes (never across `router.try_call`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_app_search_batch<F>(
    data_dir: &Path,
    ledger: &Ledger,
    write_lock: Option<&Mutex<()>>,
    router: &Router,
    session_id: &str,
    planned_queries: Vec<Value>,
    meta: AppSearchBatchMeta,
    cancel: Option<&CancelToken>,
    mut on_progress: F,
) -> Result<NetBatchOutcome, String>
where
    F: FnMut(&str, Value),
{
    if planned_queries.is_empty() {
        return Err("no valid queries to execute".to_string());
    }
    let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let stmt = latest_idea_statement(&events)?;
    let baseline_stmt_seq = stmt.seq;
    let stmt_seeds: Vec<PaperKey> = stmt.seeds.iter().filter_map(|s| PaperKey::parse(s).ok()).collect();
    let round = next_search_round(&events);
    let request_id = format!("{}-r{round}", meta.kind);

    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok(NetBatchOutcome::Cancelled);
    }

    // Rule P3: check F9/X3 query budget cap BEFORE dispatching any network query.
    let snap = budget_snapshot(&events, true);
    let remaining = snap.max_queries.saturating_sub(snap.queries_used) as usize;
    let is_allowlisted_q = |q: &Value| {
        q["provider"].as_str().and_then(LitProvider::parse).is_some_and(LitProvider::is_allowlisted)
    };
    if remaining == 0 && planned_queries.iter().any(is_allowlisted_q) {
        let reason = format!("Query budget cap ({}) reached before {} round {round}", snap.max_queries, meta.kind);
        return Ok(NetBatchOutcome::StoppedAtCap { reason });
    }

    // Separate F9 `max_queries` budget skips from `MAX_IDEA_SEARCH_QUERIES` (24) single-batch slicing (Audit H5).
    let (mut to_run, mut budget_skipped, mut used) = (Vec::new(), Vec::new(), 0usize);
    for q in planned_queries.iter().take(MAX_IDEA_SEARCH_QUERIES) {
        if is_allowlisted_q(q) {
            if used < remaining {
                used += 1;
                to_run.push(q.clone());
            } else {
                budget_skipped.push(q.clone());
            }
        } else if budget_skipped.is_empty() {
            to_run.push(q.clone());
        }
    }
    if to_run.is_empty() {
        let reason = format!("Query budget cap ({}) reached before {} round {round}", snap.max_queries, meta.kind);
        return Ok(NetBatchOutcome::StoppedAtCap { reason });
    }

    let tool_args = json!({ "run_id": request_id, "queries": build_idea_search_args(&to_run) });
    let caller = format!("host:idea:{session_id}");
    // Rule P5: every app-composed citation/follow-up query goes strictly through Router::try_call.
    let outcome = router
        .try_call(&caller, IDEA_SEARCH, &tool_args)
        .map_err(|e| format!("router ledger error during idea_search: {e}"))?;
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok(NetBatchOutcome::Cancelled);
    }

    let tool_output = match outcome {
        CallOutcome::Ok(v) => v,
        CallOutcome::Denied(reason) => return Ok(NetBatchOutcome::Denied { reason }),
        CallOutcome::Error(err) => return Err(err),
    };

    let result_rows = tool_output["results"].as_array().cloned().unwrap_or_default();
    let parsed = collect_batch_results(&result_rows, &budget_skipped, &meta, &mut on_progress);
    let stopped_at_cap = !budget_skipped.is_empty();

    // Hold `ledger_write_lock` around multi-event ledger appends and re-verify active revision (Audit H3).
    let _write_guard = match write_lock {
        Some(lk) => Some(lk.lock().map_err(|_| "ledger lock poisoned".to_string())?),
        None => None,
    };
    let fresh_events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let current_stmt_seq = latest_idea_statement(&fresh_events)?.seq;
    if current_stmt_seq != baseline_stmt_seq {
        return Err("idea statement was revised while citation lookup was in flight".to_string());
    }

    let mut candidates = merge_with_prior_candidates(&fresh_events, current_stmt_seq, &parsed.ranked_lists);
    annotate_and_record_library(data_dir, session_id, &stmt_seeds, &mut candidates);
    let (lib_total, lib_ft) = library_counts(data_dir);
    let lib_hits = candidates.iter().filter(|c| c.in_library).count();
    let finished_payload = json!({
        "round": round, "offline": false, "origin": "app", "kind": meta.kind, "round2": meta.round2,
        "plan_queries": to_run,
        "results": parsed.query_outcomes, "queries": parsed.query_outcomes,
        "providers": summarize_provider_states(parsed.provider_counts),
        "library": { "papers": lib_total, "with_full_text": lib_ft, "hits": lib_hits },
    });
    ledger.append(session_id, EVENT_IDEA_SEARCH_FINISHED, EVENT_SCHEMA_VERSION, &finished_payload).map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_SEARCH_FINISHED, finished_payload);

    let cands_payload = json!({ "round": round, "origin": "app", "candidates": candidates });
    ledger.append(session_id, EVENT_IDEA_CANDIDATES, EVENT_SCHEMA_VERSION, &cands_payload).map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_CANDIDATES, cands_payload);

    let latest_events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    // Audit M2: do not inflate `seeds`/`hops` on `idea.citation_net` when zero queries succeeded.
    let (eff_seeds, eff_hops) = if parsed.ok_count > 0 {
        (meta.extra_seeds.as_slice(), meta.hops.min(1))
    } else {
        (&[][..], 0usize)
    };
    let citation_net = build_from_events(&latest_events, &parsed.edges, &parsed.metas, eff_seeds, eff_hops);
    let net_payload = build_citation_net_payload(&citation_net, &meta, round, stopped_at_cap, &parsed.citing_papers, parsed.ok_count)?;
    if parsed.ok_count > 0 || meta.round2 {
        ledger.append(session_id, EVENT_IDEA_CITATION_NET, EVENT_SCHEMA_VERSION, &net_payload).map_err(|e| e.to_string())?;
        on_progress(EVENT_IDEA_CITATION_NET, net_payload);
    }

    Ok(NetBatchOutcome::Finished {
        round,
        executed_queries: to_run.len(),
        skipped_budget: budget_skipped.len(),
        stopped_at_cap,
        citation_net,
        citing_papers: parsed.citing_papers,
    })
}

struct ParsedBatchResults {
    ranked_lists: Vec<RankedServiceList>,
    query_outcomes: Vec<Value>,
    edges: Vec<CitationEdge>,
    metas: Vec<CitationPaperMeta>,
    citing_papers: Vec<Value>,
    provider_counts: BTreeMap<String, (usize, usize, usize, bool)>,
    ok_count: usize,
}

fn collect_batch_results<F>(
    rows: &[Value],
    skipped: &[Value],
    meta: &AppSearchBatchMeta,
    on_progress: &mut F,
) -> ParsedBatchResults
where
    F: FnMut(&str, Value),
{
    let (mut ranked_lists, mut query_outcomes, mut edges, mut metas) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut citing_by_key: BTreeMap<PaperKey, (Option<i32>, Value)> = BTreeMap::new();
    let mut provider_counts: BTreeMap<String, (usize, usize, usize, bool)> = BTreeMap::new();
    let mut ok_count = 0usize;

    for row in rows {
        let qid = row["qid"].as_str().unwrap_or("").to_string();
        let prov_str = row["provider"].as_str().unwrap_or("").to_string();
        let prov_enum = LitProvider::parse(&prov_str);
        let rel_str = row["relation"].as_str().unwrap_or("none");
        let rel_key = row["key"].as_str().and_then(|s| PaperKey::parse(s).ok());
        let status = row["status"].as_str().unwrap_or("failed").to_string();
        let returned = row["returned"].as_u64().unwrap_or(0) as usize;

        let entry = provider_counts.entry(prov_str.clone()).or_insert((0, 0, 0, false));
        match status.as_str() {
            "ok" => {
                ok_count += 1;
                entry.0 += 1;
                let parsed_papers: Vec<LitPaper> = row["papers"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| serde_json::from_value::<LitPaper>(v.clone()).ok()).collect())
                    .unwrap_or_default();
                let mut hits = Vec::with_capacity(parsed_papers.len());
                for p in &parsed_papers {
                    hits.push(lit_paper_to_raw_hit(p));
                    metas.push(CitationPaperMeta::from_lit_paper(p));
                    if rel_str == "cited_by" && meta.target_key.as_ref() == rel_key.as_ref() {
                        let matches_since = match (meta.since_year, p.year) {
                            (Some(sy), Some(py)) => py >= sy,
                            _ => true,
                        };
                        let item = json!({
                            "key": p.paper_ref.key.as_str(), "title": p.title, "authors": p.authors,
                            "year": p.year, "citation_count": p.citation_count, "provider": prov_str,
                            "matches_since_year": matches_since,
                        });
                        // Update if an earlier provider lacked `year` and this provider supplied it (Audit M1).
                        match citing_by_key.get_mut(&p.paper_ref.key) {
                            Some((prev_y, prev_val)) if prev_y.is_none() && p.year.is_some() => {
                                *prev_y = p.year;
                                *prev_val = item;
                            }
                            None => { citing_by_key.insert(p.paper_ref.key.clone(), (p.year, item)); }
                            _ => {}
                        }
                    }
                }
                if !hits.is_empty() {
                    ranked_lists.push(RankedServiceList { qid: qid.clone(), provider: prov_str.clone(), perspective_id: None, hits });
                }
                let parsed_edges: Vec<CitationEdge> = row["edges"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| serde_json::from_value::<CitationEdge>(v.clone()).ok()).collect())
                    .unwrap_or_default();
                if !parsed_edges.is_empty() {
                    edges.extend(parsed_edges);
                } else if let (Some(rk), Some(pe)) = (&rel_key, prov_enum) {
                    for p in &parsed_papers {
                        if rel_str == "cited_by" {
                            edges.push(CitationEdge { from: p.paper_ref.key.clone(), to: rk.clone(), provider: pe });
                        } else if rel_str == "cites" {
                            edges.push(CitationEdge { from: rk.clone(), to: p.paper_ref.key.clone(), provider: pe });
                        }
                    }
                }
            }
            "rate_limited" => entry.2 += 1,
            "disabled" => entry.3 = true,
            "not_configured" | "unsupported" => {}
            _ => entry.1 += 1,
        }

        let outcome_row = json!({
            "qid": qid, "provider": prov_str, "q": row["q"].as_str().unwrap_or(""),
            "relation": rel_str, "key": rel_key.as_ref().map(PaperKey::as_str), "status": status,
            "http_status": row["http_status"], "count": returned, "hits": returned,
            "new_papers": row["new_papers"].as_u64().unwrap_or(0),
            "error": row["error"], "retry_after_secs": row["retry_after_secs"],
        });
        query_outcomes.push(outcome_row.clone());
        on_progress("idea.query_result", outcome_row);
    }

    for sk in skipped {
        let sk_row = json!({
            "qid": sk["qid"].as_str().unwrap_or(""), "provider": sk["provider"].as_str().unwrap_or(""),
            "q": sk["q"].as_str().unwrap_or(""), "status": "skipped_budget",
            "http_status": Value::Null, "count": 0, "hits": 0,
        });
        query_outcomes.push(sk_row.clone());
        on_progress("idea.query_result", sk_row);
    }

    // Audit M1: filter out papers explicitly earlier than `since_year` from `citing_papers`
    // while preserving all edges in the underlying `CitationNetwork`.
    let citing_papers: Vec<Value> = citing_by_key
        .into_values()
        .map(|(_, v)| v)
        .filter(|v| v["matches_since_year"].as_bool() != Some(false))
        .collect();

    ParsedBatchResults { ranked_lists, query_outcomes, edges, metas, citing_papers, provider_counts, ok_count }
}

fn build_citation_net_payload(
    net: &CitationNetwork,
    meta: &AppSearchBatchMeta,
    round: u64,
    stopped_at_cap: bool,
    citing_papers: &[Value],
    ok_count: usize,
) -> Result<Value, String> {
    let mut val = serde_json::to_value(net).map_err(|e| e.to_string())?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert("origin".into(), json!("app"));
        obj.insert("kind".into(), json!(meta.kind));
        obj.insert("round".into(), json!(round));
        obj.insert("round2".into(), json!(meta.round2));
        obj.insert("stopped_at_cap".into(), json!(stopped_at_cap));
        obj.insert("ok_count".into(), json!(ok_count));
        if let Some(ref tk) = meta.target_key { obj.insert("target_key".into(), json!(tk.as_str())); }
        if let Some(ref nid) = meta.node_id { obj.insert("node_id".into(), json!(nid)); }
        if let Some(sy) = meta.since_year { obj.insert("since_year".into(), json!(sy)); }
        if !citing_papers.is_empty() || meta.kind == "cited_since" {
            obj.insert("citing_papers".into(), json!(citing_papers));
        }
        if meta.is_open_question {
            obj.insert("is_open_question".into(), json!(true));
            obj.insert("note".into(), json!(CITED_SINCE_NOT_ANSWERED_NOTE));
        }
        if !meta.thin_statements.is_empty() {
            obj.insert("thin_statements".into(), json!(meta.thin_statements));
        }
    }
    Ok(val)
}
