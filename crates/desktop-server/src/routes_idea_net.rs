//! Dedicated HTTP routes for Explore citation expansion, `"Who cited this since"`,
//! `"Search more about this node"`, and Thorough Round 2 (package X16).
//!
//! Dispatched directly from `api_dispatch.rs` behind `AuthGuard::authorize_api` so
//! `routes_idea.rs` stays untouched. Every outgoing search/citation request goes strictly
//! through `Router::try_call("host:idea:<session>", "idea_search", ...)` + F3 `NetRateGate`.

#[path = "idea_net_exec.rs"]
pub(crate) mod exec;
pub use exec::{run_cited_since_with_router, NetBatchOutcome};

use crate::idea::{latest_statement_seq, EVENT_IDEA_CANDIDATES, EVENT_IDEA_MAP};
use crate::idea_plan::{is_session_offline, next_search_round};
use crate::idea_view;
use crate::routers::build_idea_router;
use crate::routes_idea::ActiveExploreGuard;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::session_mode::session_mode;
use crate::state::DesktopState;
use exec::{execute_app_search_batch, AppSearchBatchMeta};
use ledger::{Event, Ledger};
use research_tools::idea_net::{default_citation_provider, supported_citation_providers};
use research_tools::idea_terms::extract as extract_idea_terms;
use research_tools::idea_tools::{MAX_IDEA_QUERY_CHARS, MAX_IDEA_SEARCH_QUERIES};
use research_tools::lit_providers::{LitProvider, DEFAULT_MAX_RESULTS, MAX_RESULTS_CAP};
use research_tools::paper_fetch::parse_paper_input;
use research_tools::paper_key::{Mode, PaperKey};
use research_tools::{validate_session_id, ResearchEnv};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Mandatory open-question caveat (`explore.md` section 12, risk 12).
pub const CITED_SINCE_NOT_ANSWERED_NOTE: &str =
    "\"Cited since\" does not mean \"answered\": follow-up citations must be read to check whether an open question was resolved.";

/// Default results requested per citation-relation lookup (within F11 `MAX_RESULTS_CAP = 25`).
pub const DEFAULT_CITED_SINCE_MAX_RESULTS: usize = 15;
/// Maximum queries emitted by a single `"Search more about this node"` request.
pub const MAX_NODE_MORE_QUERIES: usize = 6;
/// Maximum seeds expanded in a single explicit citation-expansion call.
pub const MAX_EXPANSION_SEEDS: usize = 8;

/// Parsed request for `"Who cited this since"` (`POST /api/sessions/{id}/idea/cited-since`).
#[derive(Debug, Clone)]
pub struct CitedSinceRequest {
    pub target_key: PaperKey,
    pub node_id: Option<String>,
    pub providers: Vec<LitProvider>,
    pub since_year: Option<i32>,
    pub is_open_question: bool,
    pub max_results: usize,
}

/// Top-level HTTP dispatcher mounted from `api_dispatch.rs` behind `AuthGuard::authorize_api`.
pub fn handle_idea_net_op(
    state: &Arc<DesktopState>,
    session_id: &str,
    op: &str,
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    if session_mode(&events) != Mode::Explore {
        return Err(bad(format!("session `{session_id}` is not an Explore session")));
    }
    if is_session_offline(&events) {
        return Err(bad("Explore session is in offline (library-only) mode"));
    }

    match op {
        "cited-since" => handle_cited_since_op(state, &ledger, session_id, &events, body),
        "more" => handle_more_op(state, &ledger, session_id, &events, body),
        "round2" => Err((501, "Thorough Round 2 self-check is wired in Checkpoint 2".to_string())),
        other => Err(not_found(format!("unknown Explore citation operation `{other}`"))),
    }
}

fn handle_cited_since_op(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    events: &[Event],
    body: &[u8],
) -> ApiResult {
    let val = parse_json_body(body)?;
    let req = parse_cited_since_request(events, &val)?;
    let guard = ActiveExploreGuard::acquire(state, session_id)?;
    let mut env = ResearchEnv::new(&state.cfg.data_dir);
    env.network_endpoints = state.cfg.literature_endpoints.clone();
    let router = build_idea_router(Arc::clone(state), Arc::new(env), session_id).map_err(internal)?;

    let (sse, sid_sse) = (Arc::clone(&state.sse), session_id.to_string());
    let res = run_cited_since_with_router(
        &state.cfg.data_dir,
        ledger,
        &router,
        session_id,
        &req,
        Some(&guard.cancel),
        move |kind, payload| {
            sse.emit(&sid_sse, "session_event", json!({ "session": sid_sse, "kind": kind, "payload": payload }));
        },
    );
    drop(guard);

    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    format_net_response(res, idea_view::view(&updated, false), Some(&req))
}

fn handle_more_op(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    events: &[Event],
    body: &[u8],
) -> ApiResult {
    let val = if body.is_empty() { json!({}) } else { parse_json_body(body)? };
    let action = val["action"].as_str().unwrap_or("").trim();
    let wants_round2 = action == "round2"
        || val["round2"].as_bool() == Some(true)
        || val["mode"].as_str() == Some("round2");
    if wants_round2 {
        return Err((501, "Thorough Round 2 self-check is wired in Checkpoint 2".to_string()));
    }

    let node_id = val["node_id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let explicit_providers = parse_optional_providers(&val)?;
    let max_results = parse_max_results(val.get("max_results"), DEFAULT_MAX_RESULTS)?;

    let (planned_queries, meta) = if let Some(nid) = node_id {
        build_node_more_plan(events, &nid, &explicit_providers, max_results)?
    } else {
        let explicit_seeds = parse_seed_keys(val.get("seeds").or_else(|| val.get("paper_keys")))?;
        let hops = val["hops"].as_u64().map(|h| (h as usize).clamp(1, 2)).unwrap_or(1);
        build_citation_expansion_plan(events, &explicit_seeds, &explicit_providers, hops, max_results)?
    };

    let guard = ActiveExploreGuard::acquire(state, session_id)?;
    let mut env = ResearchEnv::new(&state.cfg.data_dir);
    env.network_endpoints = state.cfg.literature_endpoints.clone();
    let router = build_idea_router(Arc::clone(state), Arc::new(env), session_id).map_err(internal)?;

    let (sse, sid_sse) = (Arc::clone(&state.sse), session_id.to_string());
    let res = execute_app_search_batch(
        &state.cfg.data_dir,
        ledger,
        &router,
        session_id,
        planned_queries,
        meta,
        Some(&guard.cancel),
        move |kind, payload| {
            sse.emit(&sid_sse, "session_event", json!({ "session": sid_sse, "kind": kind, "payload": payload }));
        },
    );
    drop(guard);

    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    format_net_response(res, idea_view::view(&updated, false), None)
}

fn parse_cited_since_request(events: &[Event], val: &Value) -> Result<CitedSinceRequest, (u16, String)> {
    let node_id = val["node_id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let map_node = node_id.as_deref().map(|nid| find_active_map_node(events, nid)).transpose()?;

    let raw_key = val.get("paper_key").or_else(|| val.get("key"))
        .and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
        .or_else(|| map_node.as_ref().and_then(extract_node_paper_key))
        .ok_or_else(|| bad("`paper_key` (or `node_id` linked to a paper) is required for `/idea/cited-since`"))?;
    let target_key = PaperKey::parse(&raw_key)
        .or_else(|_| parse_paper_input(&raw_key).map(|r| r.key))
        .map_err(|e| bad(format!("invalid `paper_key` `{raw_key}`: {e}")))?;

    let mut providers = parse_optional_providers(val)?;
    if providers.is_empty() {
        let def = default_citation_provider(&target_key, "cited_by").ok_or_else(|| {
            bad(format!("no allowlisted literature service supports `cited_by` lookup for `{target_key}`"))
        })?;
        providers.push(def);
    } else {
        let supported = supported_citation_providers(&target_key, "cited_by");
        for &p in &providers {
            if p.is_allowlisted() && !supported.contains(&p) {
                return Err(bad(format!("provider `{p}` does not support `cited_by` lookup for `{target_key}`")));
            }
        }
    }

    let is_open_question = map_node.as_ref().is_some_and(|n| n["kind"].as_str() == Some("open_question"))
        || val["is_open_question"].as_bool().unwrap_or(false)
        || paper_has_open_question_node(events, &target_key);

    let since_year = match val.get("since_year").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_i64().ok_or_else(|| bad("`since_year` must be an integer year"))? as i32),
        None => map_node.as_ref()
            .and_then(|n| n.get("stated_year").or_else(|| n.get("year")).and_then(Value::as_i64))
            .map(|y| y as i32)
            .or_else(|| find_candidate_year(events, &target_key)),
    };
    let max_results = parse_max_results(val.get("max_results"), DEFAULT_CITED_SINCE_MAX_RESULTS)?;
    Ok(CitedSinceRequest { target_key, node_id, providers, since_year, is_open_question, max_results })
}

fn build_node_more_plan(
    events: &[Event],
    node_id: &str,
    explicit_providers: &[LitProvider],
    max_results: usize,
) -> Result<(Vec<Value>, AppSearchBatchMeta), (u16, String)> {
    let node = find_active_map_node(events, node_id)?;
    let round = next_search_round(events);
    let label = node["label"].as_str().unwrap_or("").trim();
    let statement = node["statement"].as_str().unwrap_or("").trim();
    let is_open_q = node["kind"].as_str() == Some("open_question");

    let extracted = extract_idea_terms(&format!("{label} {statement}"));
    let mut terms: Vec<String> = extracted.terms.into_iter().take(4).map(|t| t.text).collect();
    if let Some(aliases) = node["aliases"].as_array() {
        for a in aliases.iter().filter_map(Value::as_str).take(2) {
            let t = a.trim();
            if !t.is_empty() && !terms.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                terms.push(t.to_string());
            }
        }
    }
    let joined = terms.join(" ");
    let query_text = clip_query(if terms.is_empty() { label } else { &joined });

    let mut queries = Vec::new();
    let mut q_idx = 0usize;
    if !query_text.is_empty() {
        let lex_provs = if explicit_providers.is_empty() {
            vec![LitProvider::Arxiv, LitProvider::Inspire]
        } else {
            explicit_providers.to_vec()
        };
        for p in lex_provs {
            if queries.len() >= MAX_NODE_MORE_QUERIES { break; }
            q_idx += 1;
            queries.push(json!({
                "qid": format!("q-more-r{round}-{q_idx}"), "perspective_id": Value::Null,
                "provider": p.as_str(), "q": query_text, "relation": { "kind": "none" },
                "max_results": max_results, "offset": 0, "origin": "app",
            }));
        }
    }

    let mut extra_seeds = Vec::new();
    for pk in extract_node_paper_keys(&node) {
        extra_seeds.push(pk.clone());
        for rel in ["cited_by", "cites"] {
            if queries.len() >= MAX_NODE_MORE_QUERIES { break; }
            let prov = explicit_providers.iter().copied()
                .find(|p| supported_citation_providers(&pk, rel).contains(p))
                .or_else(|| default_citation_provider(&pk, rel));
            if let Some(p) = prov {
                q_idx += 1;
                queries.push(json!({
                    "qid": format!("q-more-r{round}-{q_idx}"), "perspective_id": Value::Null,
                    "provider": p.as_str(), "q": "", "relation": { "kind": rel, "key": pk.as_str() },
                    "max_results": max_results, "offset": 0, "origin": "app",
                }));
            }
        }
    }
    if queries.is_empty() {
        return Err(bad(format!("node `{node_id}` yielded zero searchable terms or paper keys")));
    }

    let meta = AppSearchBatchMeta {
        kind: "node_more",
        target_key: extra_seeds.first().cloned(),
        node_id: Some(node_id.to_string()),
        since_year: node.get("stated_year").or_else(|| node.get("year")).and_then(Value::as_i64).map(|y| y as i32),
        is_open_question: is_open_q,
        hops: usize::from(!extra_seeds.is_empty()),
        extra_seeds,
        round2: false,
        thin_statements: Vec::new(),
    };
    Ok((queries, meta))
}

fn build_citation_expansion_plan(
    events: &[Event],
    explicit_seeds: &[PaperKey],
    explicit_providers: &[LitProvider],
    hops: usize,
    max_results: usize,
) -> Result<(Vec<Value>, AppSearchBatchMeta), (u16, String)> {
    let seeds = if !explicit_seeds.is_empty() {
        explicit_seeds.iter().take(MAX_EXPANSION_SEEDS).cloned().collect()
    } else {
        collect_session_expansion_seeds(events, MAX_EXPANSION_SEEDS)
    };
    if seeds.is_empty() {
        return Err(bad("no seed, core, or read papers available for citation expansion"));
    }
    let round = next_search_round(events);
    let mut queries = Vec::new();
    let mut q_idx = 0usize;
    for seed in &seeds {
        for rel in ["cites", "cited_by"] {
            if queries.len() >= MAX_IDEA_SEARCH_QUERIES { break; }
            let prov = explicit_providers.iter().copied()
                .find(|p| supported_citation_providers(seed, rel).contains(p))
                .or_else(|| default_citation_provider(seed, rel));
            if let Some(p) = prov {
                q_idx += 1;
                queries.push(json!({
                    "qid": format!("q-exp-r{round}-{q_idx}"), "perspective_id": Value::Null,
                    "provider": p.as_str(), "q": "", "relation": { "kind": rel, "key": seed.as_str() },
                    "max_results": max_results, "offset": 0, "origin": "app",
                }));
            }
        }
    }
    if queries.is_empty() {
        return Err(bad("none of the seed papers support citation-relation lookups"));
    }
    let meta = AppSearchBatchMeta {
        kind: "citation_expansion",
        target_key: seeds.first().cloned(),
        node_id: None,
        since_year: None,
        is_open_question: false,
        extra_seeds: seeds,
        hops: hops.max(1),
        round2: false,
        thin_statements: Vec::new(),
    };
    Ok((queries, meta))
}

pub(crate) fn collect_session_expansion_seeds(events: &[Event], limit: usize) -> Vec<PaperKey> {
    let stmt_seq = latest_statement_seq(events);
    let (mut out, mut seen) = (Vec::new(), BTreeSet::new());
    let mut push = |k: PaperKey| {
        if out.len() < limit && seen.insert(k.clone()) { out.push(k); }
    };
    for ev in events.iter().filter(|e| e.seq >= stmt_seq) {
        if ev.kind == "idea.paper" && matches!(ev.payload["status"].as_str(), Some("read_full" | "read_passages")) {
            if let Some(k) = ev.payload["key"].as_str().and_then(|s| PaperKey::parse(s).ok()) { push(k); }
        }
    }
    for ev in events.iter().filter(|e| e.seq >= stmt_seq) {
        if ev.kind == "idea.triage" {
            if let Some(arr) = ev.payload["items"].as_array() {
                for it in arr.iter().filter(|i| i["relevance"].as_str() == Some("core")) {
                    if let Some(k) = it["key"].as_str().and_then(|s| PaperKey::parse(s).ok()) { push(k); }
                }
            }
        }
    }
    if let Some(st) = events.iter().rev().find(|e| e.kind == "idea.stated") {
        if let Some(arr) = st.payload["seeds"].as_array() {
            for s in arr.iter().filter_map(Value::as_str) {
                if let Ok(k) = PaperKey::parse(s) { push(k); }
            }
        }
    }
    out
}

pub(crate) fn find_active_map_node(events: &[Event], node_id: &str) -> Result<Value, (u16, String)> {
    let stmt_seq = latest_statement_seq(events);
    let map_ev = events.iter().rev().find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq)
        .ok_or_else(|| bad("no active `idea.map` found for this session"))?;
    map_ev.payload["nodes"].as_array()
        .and_then(|arr| arr.iter().find(|n| n["id"].as_str() == Some(node_id)))
        .cloned()
        .ok_or_else(|| bad(format!("map node `{node_id}` not found on the active map")))
}

fn extract_node_paper_key(node: &Value) -> Option<String> {
    node["paper_key"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
        .or_else(|| node["papers"].as_array().and_then(|a| a.iter().filter_map(Value::as_str).find(|s| !s.trim().is_empty())).map(|s| s.trim().to_string()))
}

pub(crate) fn extract_node_paper_keys(node: &Value) -> Vec<PaperKey> {
    let (mut keys, mut seen) = (Vec::new(), BTreeSet::new());
    if let Some(pk) = node["paper_key"].as_str().and_then(|s| PaperKey::parse(s).ok()) {
        if seen.insert(pk.clone()) { keys.push(pk); }
    }
    if let Some(arr) = node["papers"].as_array() {
        for s in arr.iter().filter_map(Value::as_str) {
            if let Ok(pk) = PaperKey::parse(s) {
                if seen.insert(pk.clone()) { keys.push(pk); }
            }
        }
    }
    keys
}

fn paper_has_open_question_node(events: &[Event], key: &PaperKey) -> bool {
    let stmt_seq = latest_statement_seq(events);
    let Some(map_ev) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq) else { return false };
    map_ev.payload["nodes"].as_array().is_some_and(|arr| {
        arr.iter().any(|n| n["kind"].as_str() == Some("open_question") && extract_node_paper_keys(n).contains(key))
    })
}

fn find_candidate_year(events: &[Event], key: &PaperKey) -> Option<i32> {
    let stmt_seq = latest_statement_seq(events);
    events.iter().rev().find(|e| e.kind == EVENT_IDEA_CANDIDATES && e.seq > stmt_seq)
        .and_then(|e| e.payload["candidates"].as_array())
        .and_then(|arr| arr.iter().find(|c| c["key"].as_str() == Some(key.as_str())).and_then(|c| c["year"].as_i64()).map(|y| y as i32))
}

pub(crate) fn parse_optional_providers(val: &Value) -> Result<Vec<LitProvider>, (u16, String)> {
    let (mut out, mut seen) = (Vec::new(), BTreeSet::new());
    if let Some(p) = val.get("provider").filter(|v| !v.is_null()) {
        let s = p.as_str().ok_or_else(|| bad("`provider` must be a string"))?;
        let parsed = LitProvider::parse(s).ok_or_else(|| bad(format!("unknown provider `{s}`")))?;
        if seen.insert(parsed) { out.push(parsed); }
    }
    if let Some(arr) = val.get("providers").filter(|v| !v.is_null()) {
        let list = arr.as_array().ok_or_else(|| bad("`providers` must be an array of provider names"))?;
        for item in list {
            let s = item.as_str().ok_or_else(|| bad("each entry in `providers` must be a string"))?;
            let parsed = LitProvider::parse(s).ok_or_else(|| bad(format!("unknown provider `{s}`")))?;
            if seen.insert(parsed) { out.push(parsed); }
        }
    }
    Ok(out)
}

fn parse_seed_keys(val: Option<&Value>) -> Result<Vec<PaperKey>, (u16, String)> {
    let Some(v) = val.filter(|x| !x.is_null()) else { return Ok(Vec::new()) };
    let arr = v.as_array().ok_or_else(|| bad("`seeds` must be an array of paper keys"))?;
    let (mut out, mut seen) = (Vec::new(), BTreeSet::new());
    for item in arr {
        let s = item.as_str().ok_or_else(|| bad("each seed must be a string"))?;
        let key = PaperKey::parse(s.trim())
            .or_else(|_| parse_paper_input(s.trim()).map(|r| r.key))
            .map_err(|e| bad(format!("invalid seed `{s}`: {e}")))?;
        if seen.insert(key.clone()) { out.push(key); }
    }
    Ok(out)
}

pub(crate) fn parse_max_results(val: Option<&Value>, default: usize) -> Result<usize, (u16, String)> {
    match val.filter(|v| !v.is_null()) {
        None => Ok(default),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| bad("`max_results` must be an integer"))? as usize;
            if !(1..=MAX_RESULTS_CAP).contains(&n) {
                return Err(bad(format!("`max_results` must be in 1..={MAX_RESULTS_CAP}")));
            }
            Ok(n)
        }
    }
}

pub(crate) fn clip_query(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_IDEA_QUERY_CHARS).collect()
}

fn format_net_response(res: Result<NetBatchOutcome, String>, view: Value, cited_req: Option<&CitedSinceRequest>) -> ApiResult {
    match res {
        Ok(NetBatchOutcome::Finished { round, executed_queries, skipped_budget, stopped_at_cap, citation_net, citing_papers }) => {
            let mut body = json!({
                "ok": true, "model_calls": 0, "round": round, "executed_queries": executed_queries,
                "skipped_budget": skipped_budget, "stopped_at_cap": stopped_at_cap,
                "citation_net": citation_net, "citing_papers": citing_papers, "idea": view,
            });
            if let (Some(req), Some(obj)) = (cited_req, body.as_object_mut()) {
                obj.insert("target_key".into(), json!(req.target_key.as_str()));
                obj.insert("node_id".into(), json!(req.node_id));
                obj.insert("since_year".into(), json!(req.since_year));
                obj.insert("is_open_question".into(), json!(req.is_open_question));
                if req.is_open_question { obj.insert("note".into(), json!(CITED_SINCE_NOT_ANSWERED_NOTE)); }
            }
            Ok((200, body))
        }
        Ok(NetBatchOutcome::Denied { reason }) => Ok((200, json!({ "ok": false, "denied": true, "model_calls": 0, "reason": reason, "idea": view }))),
        Ok(NetBatchOutcome::StoppedAtCap { reason }) => Ok((200, json!({ "ok": false, "stopped_at_cap": true, "model_calls": 0, "reason": reason, "idea": view }))),
        Ok(NetBatchOutcome::Cancelled) => Ok((200, json!({ "ok": false, "cancelled": true, "model_calls": 0, "idea": view }))),
        Err(msg) if msg.starts_with("router ledger error") => Err(internal(msg)),
        Err(msg) => Err(bad(msg)),
    }
}
