//! Thorough Round 2 self-check and citation-expansion orchestration (package X16).
//!
//! Governance: `explore.md` 3 (`Thorough` preset: `36` queries, `80` screened, `12` papers,
//! `<= 4` calls/paper, hard cap `55` model calls), 4.2 (`idea.citation_net`), 6 (Rules P3, P5,
//! P6, S8, V9), and 11 (OpenScholar self-check pattern re-coded in Rust).

use crate::idea::{
    budget_snapshot, effective_budget, latest_statement_seq, max_reader_calls_per_paper,
    supports_thorough_round2, IdeaPreset, IdeaStage, EVENT_IDEA_BRIEF, EVENT_IDEA_CITATION_NET,
    EVENT_IDEA_MAP, EVENT_IDEA_VERDICT,
};
use crate::idea_exchanges::emit_stage;
use crate::idea_plan::{is_session_offline, next_search_round};
use crate::idea_view;
use crate::routers::build_idea_router;
use crate::routes_idea::ActiveExploreGuard;
use crate::routes_idea_net::exec::{execute_app_search_batch, AppSearchBatchMeta, NetBatchOutcome};
use crate::routes_idea_net::{
    clip_query, collect_session_expansion_seeds, extract_node_paper_keys, parse_max_results,
    parse_optional_providers, CITED_SINCE_NOT_ANSWERED_NOTE,
};
use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::DesktopState;
use agent::CancelToken;
use ledger::{Event, Ledger};
use research_tools::idea_net::{
    build_from_events, default_citation_provider, supported_citation_providers, CitationNetwork,
};
use research_tools::idea_terms::extract as extract_idea_terms;
use research_tools::idea_tools::MAX_IDEA_SEARCH_QUERIES;
use research_tools::lit_providers::{LitProvider, DEFAULT_MAX_RESULTS};
use research_tools::paper_key::PaperKey;
use research_tools::run_budget::ExploreStageState;
use research_tools::ResearchEnv;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use tools::Router;

/// Thorough preset canonical ceilings (`explore.md` section 3).
pub const THOROUGH_MAX_QUERIES: u64 = 36;
pub const THOROUGH_MAX_SCREENED: u64 = 80;
pub const THOROUGH_MAX_PAPERS: usize = 12;
pub const THOROUGH_CALLS_PER_PAPER: u64 = 4;
pub const THOROUGH_MAX_MODEL_CALLS: u64 = 55;

const MAX_ROUND2_SEEDS: usize = 6;
const MAX_ROUND2_THIN_LEXICAL: usize = 6;

/// One thin or under-supported statement identified on the active `idea.map` or `idea.verdict`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinStatement {
    pub target_kind: String,
    pub target_id: String,
    pub label: String,
    pub reason_code: String,
    pub summary: String,
    #[serde(default)]
    pub paper_keys: Vec<String>,
    #[serde(default)]
    pub suggested_query: String,
    pub priority: u8,
}

/// Request options for Thorough Round 2 self-check.
#[derive(Debug, Clone)]
pub struct Round2Request {
    pub providers: Vec<LitProvider>,
    pub max_results: usize,
    pub allow_non_thorough: bool,
}

impl Default for Round2Request {
    fn default() -> Self {
        Self { providers: Vec::new(), max_results: DEFAULT_MAX_RESULTS, allow_non_thorough: false }
    }
}

/// Outcome of running Thorough Round 2 self-check.
#[derive(Debug, Clone)]
pub enum Round2RunResult {
    Finished {
        round: u64,
        thin_statements: Vec<ThinStatement>,
        executed_queries: usize,
        skipped_budget: usize,
        stopped_at_cap: bool,
        citation_net: CitationNetwork,
    },
    Denied { reason: String },
    StoppedAtCap { cap: &'static str, reason: String, thin_statements: Vec<ThinStatement> },
    Cancelled,
}

/// Identify thin/under-supported statements on the active revision's `idea.map` and `idea.verdict`
/// (pure deterministic OpenScholar self-check pattern; zero model calls, Rule S8 compliant).
pub fn find_thin_statements(events: &[Event]) -> Vec<ThinStatement> {
    let stmt_seq = latest_statement_seq(events);
    let (mut out, mut seen) = (Vec::new(), BTreeSet::<(String, String)>::new());

    if let Some(map_ev) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq) {
        if let Some(nodes) = map_ev.payload["nodes"].as_array() {
            for node in nodes {
                if node["status"].as_str().unwrap_or("active") != "active" { continue; }
                let id = node["id"].as_str().unwrap_or("").trim();
                if id.is_empty() { continue; }
                let kind = node["kind"].as_str().unwrap_or("");
                let label = node["label"].as_str().unwrap_or(id).trim();
                let statement = node["statement"].as_str().unwrap_or("").trim();
                let xe_count = node["evidence"]
                    .as_array()
                    .map_or(0, |a| a.iter().filter_map(Value::as_str).filter(|e| e.starts_with("xe-")).count());
                let later_checked = node["later_work_checked"].as_bool();

                let check = if kind == "open_question" && later_checked != Some(true) {
                    Some((1u8, "open_question_later_work_unchecked", "Open question has not had later citations checked yet."))
                } else if xe_count == 0 {
                    Some((1u8, "abstract_or_idea_only", "Statement currently rests only on abstract or user-idea quotes without full-text support."))
                } else if matches!(kind, "known_result" | "definition" | "concept") && xe_count == 1 {
                    Some((2u8, "single_fulltext_quote", "Statement rests on a single full-text quote; follow-up retrieval checks for corroboration or exceptions."))
                } else {
                    None
                };

                if let Some((priority, code, summary)) = check {
                    if seen.insert(("node".into(), id.to_string())) {
                        let paper_keys = extract_node_paper_keys(node).into_iter().map(|k| k.to_string()).collect();
                        out.push(ThinStatement {
                            target_kind: "node".into(),
                            target_id: id.to_string(),
                            label: label.to_string(),
                            reason_code: code.into(),
                            summary: summary.into(),
                            paper_keys,
                            suggested_query: build_thin_query(label, statement, node["aliases"].as_array()),
                            priority,
                        });
                    }
                }
            }
        }
    }

    let mut subclaim_texts: BTreeMap<String, String> = BTreeMap::new();
    if let Some(brief_ev) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > stmt_seq) {
        if let Some(arr) = brief_ev.payload["subclaims"].as_array() {
            for sc in arr {
                if let (Some(id), Some(txt)) = (sc["id"].as_str(), sc["text"].as_str()) {
                    subclaim_texts.insert(id.to_string(), txt.trim().to_string());
                }
            }
        }
    }
    if let Some(verd_ev) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_VERDICT && e.seq > stmt_seq) {
        if let Some(arr) = verd_ev.payload["verdicts"].as_array() {
            for v in arr {
                let sc_id = v["subclaim_id"].as_str().unwrap_or("").trim();
                let st = v["status"].as_str().unwrap_or("");
                if sc_id.is_empty() || !matches!(st, "none_in_scope" | "incomplete" | "abstract_only") { continue; }
                if seen.insert(("subclaim".into(), sc_id.to_string())) {
                    let label = subclaim_texts.get(sc_id).cloned().unwrap_or_else(|| sc_id.to_string());
                    let (code, summary) = match st {
                        "abstract_only" => ("subclaim_abstract_only", "Part currently has abstract-only matches without full-text support."),
                        "incomplete" => ("subclaim_incomplete_coverage", "Part search coverage is incomplete; targeted follow-up retrieval is recommended."),
                        _ => ("subclaim_no_fulltext_overlap", "Part had no matched overlap in round 1; targeted follow-up retrieval checks wider phrasing."),
                    };
                    out.push(ThinStatement {
                        target_kind: "subclaim".into(),
                        target_id: sc_id.to_string(),
                        suggested_query: build_thin_query(&label, "", None),
                        label,
                        reason_code: code.into(),
                        summary: summary.into(),
                        paper_keys: Vec::new(),
                        priority: 1,
                    });
                }
            }
        }
    }

    out.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.target_kind.cmp(&b.target_kind)).then_with(|| a.target_id.cmp(&b.target_id)));
    out
}

/// HTTP handler for Thorough Round 2 (`POST /api/sessions/{id}/idea/round2` or `/idea/more` Round 2).
pub fn handle_round2_op(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    _events: &[Event],
    body: &[u8],
) -> ApiResult {
    let val = if body.is_empty() { json!({}) } else { parse_json_body(body)? };
    let req = Round2Request {
        providers: parse_optional_providers(&val)?,
        max_results: parse_max_results(val.get("max_results"), DEFAULT_MAX_RESULTS)?,
        allow_non_thorough: val["allow_non_thorough"].as_bool().unwrap_or(false),
    };

    let guard = ActiveExploreGuard::acquire(state, session_id)?;
    let mut env = ResearchEnv::new(&state.cfg.data_dir);
    env.network_endpoints = state.cfg.literature_endpoints.clone();
    let router = build_idea_router(Arc::clone(state), Arc::new(env), session_id).map_err(internal)?;

    let (sse, sid_sse) = (Arc::clone(&state.sse), session_id.to_string());
    let res = run_round2_with_router(&state.cfg.data_dir, ledger, &router, session_id, &req, Some(&guard.cancel), move |k, p| {
        sse.emit(&sid_sse, "session_event", json!({ "session": sid_sse, "kind": k, "payload": p }));
    });
    drop(guard);

    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let view = idea_view::view(&updated, false);
    match res {
        Ok(Round2RunResult::Finished { round, thin_statements, executed_queries, skipped_budget, stopped_at_cap, citation_net }) => Ok((
            200,
            json!({
                "ok": true, "round": round, "round2": true, "model_calls": 0,
                "thin_statements": thin_statements, "executed_queries": executed_queries,
                "skipped_budget": skipped_budget, "stopped_at_cap": stopped_at_cap,
                "citation_net": citation_net, "idea": view,
            }),
        )),
        Ok(Round2RunResult::Denied { reason }) => Ok((200, json!({ "ok": false, "denied": true, "round2": true, "reason": reason, "idea": view }))),
        Ok(Round2RunResult::StoppedAtCap { cap, reason, thin_statements }) => Ok((
            200,
            json!({ "ok": false, "stopped_at_cap": true, "cap": cap, "round2": true, "reason": reason, "thin_statements": thin_statements, "idea": view }),
        )),
        Ok(Round2RunResult::Cancelled) => Ok((200, json!({ "ok": false, "cancelled": true, "round2": true, "idea": view }))),
        Err(msg) if msg.starts_with("router ledger error") => Err(internal(msg)),
        Err(msg) => Err(bad(msg)),
    }
}

/// Execute Thorough Round 2 self-check strictly through `Router::try_call("host:idea:...", "idea_search", ...)`
/// while enforcing ALL five Thorough hard ceilings (`36`/`80`/`12`/`4`/`55`, Rule P3).
pub fn run_round2_with_router<F>(
    data_dir: &Path,
    ledger: &Ledger,
    router: &Router,
    session_id: &str,
    req: &Round2Request,
    cancel: Option<&CancelToken>,
    mut on_progress: F,
) -> Result<Round2RunResult, String>
where
    F: FnMut(&str, Value),
{
    let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    if is_session_offline(&events) {
        return Err("Explore session is in offline (library-only) mode".to_string());
    }
    let cfg = effective_budget(&events);
    if !supports_thorough_round2(&events) && !req.allow_non_thorough {
        return Err(format!("Thorough Round 2 self-check requires preset `thorough` (active preset: `{}`)", cfg.preset.as_str()));
    }

    let snap = budget_snapshot(&events, true);
    let thin_statements = find_thin_statements(&events);
    let round = next_search_round(&events);
    let request_id = format!("round2-r{round}");

    // Rule P3: enforce ALL Thorough hard ceilings BEFORE dispatching any network or model work.
    let cap_hit: Option<(&'static str, String)> = if snap.model_calls.is_exhausted() {
        Some(("model_calls", format!("Model-call budget cap ({}) reached before Thorough Round 2", snap.model_calls.cap)))
    } else if cfg.max_papers > 0 && snap.papers_fetched >= cfg.max_papers {
        Some(("papers", format!("Paper fetch budget cap ({}) reached before Thorough Round 2", cfg.max_papers)))
    } else if snap.screened_used >= cfg.max_screened {
        Some(("screened", format!("Screened candidate cap ({}) reached before Thorough Round 2", cfg.max_screened)))
    } else if snap.queries_used >= cfg.max_queries {
        Some(("queries", format!("Query budget cap ({}) reached before Thorough Round 2", cfg.max_queries)))
    } else {
        None
    };

    if let Some((cap, reason)) = cap_hit {
        emit_stage(ledger, session_id, IdeaStage::Search, ExploreStageState::StoppedAtCap, &request_id, Some(&reason), &mut on_progress)?;
        return Ok(Round2RunResult::StoppedAtCap { cap, reason, thin_statements });
    }

    let (planned_queries, extra_seeds, hops) = build_round2_queries(&events, &thin_statements, req, round);
    if planned_queries.is_empty() {
        return Err("no thin statements or citation seeds available for Thorough Round 2".to_string());
    }

    let thin_json: Vec<Value> = thin_statements.iter().filter_map(|t| serde_json::to_value(t).ok()).collect();
    let has_open_q = thin_statements.iter().any(|t| t.reason_code == "open_question_later_work_unchecked");
    let meta = AppSearchBatchMeta {
        kind: "round2",
        target_key: extra_seeds.first().cloned(),
        node_id: None,
        since_year: None,
        is_open_question: has_open_q,
        extra_seeds,
        hops,
        round2: true,
        thin_statements: thin_json,
    };

    match execute_app_search_batch(data_dir, ledger, router, session_id, planned_queries, meta, cancel, on_progress)? {
        NetBatchOutcome::Finished { round, executed_queries, skipped_budget, stopped_at_cap, citation_net, .. } => {
            Ok(Round2RunResult::Finished { round, thin_statements, executed_queries, skipped_budget, stopped_at_cap, citation_net })
        }
        NetBatchOutcome::Denied { reason } => Ok(Round2RunResult::Denied { reason }),
        NetBatchOutcome::StoppedAtCap { reason } => Ok(Round2RunResult::StoppedAtCap { cap: "queries", reason, thin_statements }),
        NetBatchOutcome::Cancelled => Ok(Round2RunResult::Cancelled),
    }
}

fn build_round2_queries(
    events: &[Event],
    thin: &[ThinStatement],
    req: &Round2Request,
    round: u64,
) -> (Vec<Value>, Vec<PaperKey>, usize) {
    let mut seeds = collect_session_expansion_seeds(events, MAX_ROUND2_SEEDS);
    let mut seed_set: BTreeSet<PaperKey> = seeds.iter().cloned().collect();
    for t in thin {
        for raw in &t.paper_keys {
            if seeds.len() >= MAX_ROUND2_SEEDS { break; }
            if let Ok(pk) = PaperKey::parse(raw) {
                if seed_set.insert(pk.clone()) { seeds.push(pk); }
            }
        }
    }

    let stmt_seq = latest_statement_seq(events);
    let prior_hops = events.iter()
        .filter(|e| e.kind == EVENT_IDEA_CITATION_NET && e.seq > stmt_seq)
        .filter_map(|e| e.payload["hops"].as_u64())
        .max()
        .unwrap_or(0) as usize;
    let hops = if seeds.is_empty() { prior_hops } else { (prior_hops + 1).clamp(1, 2) };

    let (mut queries, mut q_idx) = (Vec::new(), 0usize);
    for seed in &seeds {
        for rel in ["cited_by", "cites"] {
            if queries.len() >= MAX_IDEA_SEARCH_QUERIES { break; }
            let prov = req.providers.iter().copied()
                .find(|p| supported_citation_providers(seed, rel).contains(p))
                .or_else(|| default_citation_provider(seed, rel));
            if let Some(p) = prov {
                q_idx += 1;
                queries.push(json!({
                    "qid": format!("q-r2-cite-r{round}-{q_idx}"), "perspective_id": Value::Null,
                    "provider": p.as_str(), "q": "", "relation": { "kind": rel, "key": seed.as_str() },
                    "max_results": req.max_results, "offset": 0, "origin": "app",
                }));
            }
        }
    }

    let lex_provs = if req.providers.is_empty() { vec![LitProvider::Arxiv, LitProvider::Inspire] } else { req.providers.clone() };
    let mut lex_added = 0usize;
    for t in thin {
        if lex_added >= MAX_ROUND2_THIN_LEXICAL || queries.len() >= MAX_IDEA_SEARCH_QUERIES { break; }
        if t.suggested_query.trim().is_empty() { continue; }
        for &p in &lex_provs {
            if lex_added >= MAX_ROUND2_THIN_LEXICAL || queries.len() >= MAX_IDEA_SEARCH_QUERIES { break; }
            q_idx += 1;
            lex_added += 1;
            queries.push(json!({
                "qid": format!("q-r2-thin-r{round}-{q_idx}"), "perspective_id": Value::Null,
                "provider": p.as_str(), "q": t.suggested_query, "relation": { "kind": "none" },
                "max_results": req.max_results, "offset": 0, "origin": "app",
            }));
        }
    }
    (queries, seeds, hops)
}

fn build_thin_query(label: &str, statement: &str, aliases: Option<&Vec<Value>>) -> String {
    let extracted = extract_idea_terms(&format!("{label} {statement}"));
    let mut terms: Vec<String> = extracted.terms.into_iter().take(4).map(|t| t.text).collect();
    if let Some(arr) = aliases {
        for a in arr.iter().filter_map(Value::as_str).take(2) {
            let trimmed = a.trim();
            if !trimmed.is_empty() && !terms.iter().any(|x| x.eq_ignore_ascii_case(trimmed)) {
                terms.push(trimmed.to_string());
            }
        }
    }
    let joined = terms.join(" ");
    clip_query(if terms.is_empty() { label } else { &joined })
}

/// Pure projection of `citation_net` and `round2` state for `idea_view_coverage.rs` (`<= 600`L).
pub fn project_citation_and_round2_view(events: &[Event], latest_stmt_seq: u64) -> (Value, Value) {
    let net = build_from_events(events, &[], &[], &[], 0);
    let mut cited_since_runs = Vec::new();
    let (mut r2_status, mut r2_stopped, mut r2_round, mut r2_reason) = ("idle", false, 0u64, None);

    for ev in events.iter().filter(|e| e.seq > latest_stmt_seq) {
        if ev.kind == EVENT_IDEA_CITATION_NET {
            if ev.payload["kind"].as_str() == Some("cited_since") {
                let open_q = ev.payload["is_open_question"].as_bool().unwrap_or(false);
                cited_since_runs.push(json!({
                    "seq": ev.seq, "target_key": ev.payload["target_key"], "node_id": ev.payload["node_id"],
                    "since_year": ev.payload["since_year"], "is_open_question": open_q,
                    "note": if open_q { json!(CITED_SINCE_NOT_ANSWERED_NOTE) } else { ev.payload["note"].clone() },
                    "citing_papers": ev.payload["citing_papers"].as_array().cloned().unwrap_or_default(),
                }));
            }
            if ev.payload["round2"].as_bool() == Some(true) {
                r2_round = ev.payload["round"].as_u64().unwrap_or(r2_round);
                r2_stopped = ev.payload["stopped_at_cap"].as_bool().unwrap_or(false);
                r2_status = if r2_stopped { "stopped_at_cap" } else { "done" };
            }
        } else if ev.kind == "idea.stage" && ev.payload["request_id"].as_str().is_some_and(|r| r.starts_with("round2-")) {
            r2_status = match ev.payload["state"].as_str().unwrap_or("") {
                "stopped_at_cap" => { r2_stopped = true; "stopped_at_cap" }
                "finished" => "done",
                "requested" => "running",
                "failed" => "failed",
                "cancelled" => "paused",
                _ => r2_status,
            };
            if let Some(r) = ev.payload["reason"].as_str() { r2_reason = Some(r.to_string()); }
        }
    }

    let cfg = effective_budget(events);
    let snap = budget_snapshot(events, false);
    let citation_net_view = json!({
        "seeds": net.seeds, "hops": net.hops, "providers": net.providers,
        "nodes_count": net.nodes.len(), "edges_count": net.edges.len(),
        "nodes": net.nodes, "edges": net.edges,
        "unread_core_citations": net.unread_core_citations,
        "cited_since_runs": cited_since_runs,
    });
    let round2_view = json!({
        "eligible": cfg.preset == IdeaPreset::Thorough,
        "preset": cfg.preset.as_str(),
        "preset_label": cfg.preset.label(),
        "status": r2_status,
        "stopped_at_cap": r2_stopped,
        "reason": r2_reason,
        "round": r2_round,
        "thin_statements": find_thin_statements(events),
        "ceilings": {
            "max_queries": cfg.max_queries, "max_screened": cfg.max_screened,
            "max_papers": cfg.max_papers, "calls_per_paper": cfg.calls_per_paper,
            "max_model_calls": cfg.max_model_calls,
        },
        "usage": {
            "queries_used": snap.queries_used, "screened_used": snap.screened_used,
            "papers_fetched": snap.papers_fetched,
            "max_paper_calls": max_reader_calls_per_paper(events),
            "model_calls_used": snap.model_calls.used,
        },
    });
    (citation_net_view, round2_view)
}
