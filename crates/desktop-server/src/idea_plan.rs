//! Explore SearchPlan assembly and `POST /api/sessions/{id}/idea/plan` handler (package X7).
//!
//! Kept in a sibling module of `idea_plan_search.rs` so both files stay comfortably <= 600 lines.

use crate::idea::{
    latest_statement_seq, IdeaStage, EVENT_IDEA_BRIEF, EVENT_IDEA_SEARCH_PLANNED,
    EVENT_IDEA_STARTED,
};
use crate::idea_exchanges::emit_stage;
use crate::idea_view;
use crate::idea_view_coverage::fold_answers;
use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::state::DesktopState;
use ledger::{Event, Ledger};
use research_tools::idea_tools::{MAX_IDEA_QUERY_CHARS, MAX_IDEA_SEARCH_QUERIES};
use research_tools::lit_providers::{LitProvider, DEFAULT_MAX_RESULTS, MAX_RESULTS_CAP};
use research_tools::paper_key::PaperKey;
use research_tools::run_budget::ExploreStageState;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use tools::EVENT_SCHEMA_VERSION;

pub(crate) fn is_session_offline(events: &[Event]) -> bool {
    events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_STARTED)
        .and_then(|e| e.payload.get("offline").and_then(Value::as_bool))
        .unwrap_or(false)
}

pub(crate) fn next_search_round(events: &[Event]) -> u64 {
    events
        .iter()
        .filter(|e| e.kind == EVENT_IDEA_SEARCH_PLANNED)
        .count() as u64
        + 1
}

/// Assemble a `SearchPlan` JSON payload from either explicit validated `queries` in `req`
/// or the effective sharpened idea (after `idea.answer` edits).
pub fn assemble_search_plan(
    events: &[Event],
    req: &Value,
) -> Result<(u64, Value), (u16, String)> {
    let round = next_search_round(events);
    let offline = is_session_offline(events);
    let mut planned: Vec<Value> = Vec::new();

    if let Some(explicit) = req.get("queries").filter(|v| !v.is_null()) {
        let arr = explicit
            .as_array()
            .ok_or_else(|| bad("`queries` must be an array of query objects"))?;
        if arr.is_empty() || arr.len() > MAX_IDEA_SEARCH_QUERIES {
            return Err(bad(format!(
                "`queries` must contain 1..={MAX_IDEA_SEARCH_QUERIES} entries"
            )));
        }
        let mut seen_qids = BTreeSet::new();
        for (idx, item) in arr.iter().enumerate() {
            let obj = item
                .as_object()
                .ok_or_else(|| bad(format!("queries[{idx}] must be an object")))?;
            let prov_raw = obj
                .get("provider")
                .and_then(Value::as_str)
                .ok_or_else(|| bad(format!("queries[{idx}].provider must be a string")))?;
            let provider = LitProvider::parse(prov_raw)
                .ok_or_else(|| bad(format!("queries[{idx}].provider `{prov_raw}` is invalid")))?;
            let rel_kind = match obj.get("relation") {
                None | Some(Value::Null) => "none",
                Some(Value::String(s)) => s.as_str(),
                Some(Value::Object(m)) => m.get("kind").and_then(Value::as_str).unwrap_or("none"),
                _ => return Err(bad(format!("queries[{idx}].relation is invalid"))),
            };
            if !matches!(rel_kind, "none" | "cites" | "cited_by") {
                return Err(bad(format!(
                    "queries[{idx}].relation `{rel_kind}` must be none, cites, or cited_by"
                )));
            }
            let q_str = obj
                .get("q")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if q_str.chars().count() > MAX_IDEA_QUERY_CHARS {
                return Err(bad(format!(
                    "queries[{idx}].q exceeds {MAX_IDEA_QUERY_CHARS} characters"
                )));
            }
            let rel_key = obj
                .get("key")
                .and_then(Value::as_str)
                .or_else(|| {
                    obj.get("relation")
                        .and_then(|r| r.get("key"))
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if rel_kind == "none" && q_str.is_empty() {
                return Err(bad(format!("queries[{idx}].q must be non-empty")));
            }
            let relation_json = if rel_kind == "none" {
                json!({ "kind": "none" })
            } else {
                let raw_key = rel_key.ok_or_else(|| {
                    bad(format!("queries[{idx}].key is required for relation `{rel_kind}`"))
                })?;
                let pk = PaperKey::parse(raw_key)
                    .map_err(|e| bad(format!("queries[{idx}].key `{raw_key}` invalid: {e}")))?;
                json!({ "kind": rel_kind, "key": pk.as_str() })
            };
            let qid = obj
                .get("qid")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= 64)
                .map(str::to_string)
                .unwrap_or_else(|| format!("q-r{round}-{}", idx + 1));
            if !seen_qids.insert(qid.clone()) {
                return Err(bad(format!("duplicate `qid` `{qid}` in search plan")));
            }
            let max_results = match obj.get("max_results").filter(|v| !v.is_null()) {
                None => DEFAULT_MAX_RESULTS,
                Some(v) => {
                    let n = v
                        .as_u64()
                        .ok_or_else(|| bad(format!("queries[{idx}].max_results must be an integer")))?
                        as usize;
                    if !(1..=MAX_RESULTS_CAP).contains(&n) {
                        return Err(bad(format!(
                            "queries[{idx}].max_results must be in 1..={MAX_RESULTS_CAP}"
                        )));
                    }
                    n
                }
            };
            let offset = match obj.get("offset").filter(|v| !v.is_null()) {
                None => 0,
                Some(v) => {
                    let n = v
                        .as_u64()
                        .ok_or_else(|| bad(format!("queries[{idx}].offset must be a non-negative integer")))?
                        as usize;
                    if n > 500 {
                        return Err(bad(format!("queries[{idx}].offset must be <= 500")));
                    }
                    n
                }
            };
            let perspective_id = obj
                .get("perspective_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let perspective_kind = obj
                .get("perspective_kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let origin = match obj.get("origin").and_then(Value::as_str) {
                Some("user") => "user",
                Some("app") => "app",
                _ => "model",
            };
            planned.push(json!({
                "qid": qid,
                "perspective_id": perspective_id,
                "perspective_kind": perspective_kind,
                "provider": provider.as_str(),
                "q": q_str,
                "relation": relation_json,
                "max_results": max_results,
                "offset": offset,
                "origin": origin,
            }));
        }
    } else {
        let stmt_seq = latest_statement_seq(events);
        let latest_brief = events
            .iter()
            .rev()
            .find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > stmt_seq);
        let Some(brief_ev) = latest_brief else {
            return Err(bad(
                "run Sharpen or provide explicit `queries` before planning a follow-up search",
            ));
        };
        let applied = fold_answers(events, stmt_seq, Some(&brief_ev.payload), brief_ev.seq, 0);
        let Some(perspectives) = applied
            .sharpened_idea
            .as_ref()
            .and_then(|b| b.get("perspectives"))
            .and_then(Value::as_array)
        else {
            return Err(bad("sharpened idea has no perspectives to plan"));
        };
        let mut idx = 0usize;
        for p in perspectives {
            let pid = p.get("id").and_then(Value::as_str).map(str::to_string);
            let pkind = p.get("kind").and_then(Value::as_str).map(str::to_string);
            let Some(qs) = p.get("queries").and_then(Value::as_array) else { continue };
            for q in qs {
                if planned.len() >= MAX_IDEA_SEARCH_QUERIES {
                    break;
                }
                let Some(prov) = q
                    .get("provider")
                    .and_then(Value::as_str)
                    .and_then(LitProvider::parse)
                else {
                    continue;
                };
                let q_text = q.get("q").and_then(Value::as_str).unwrap_or("").trim();
                if q_text.is_empty() || q_text.chars().count() > MAX_IDEA_QUERY_CHARS {
                    continue;
                }
                idx += 1;
                planned.push(json!({
                    "qid": format!("q-r{round}-{idx}"),
                    "perspective_id": pid,
                    "perspective_kind": pkind,
                    "provider": prov.as_str(),
                    "q": q_text,
                    "relation": { "kind": "none" },
                    "max_results": DEFAULT_MAX_RESULTS,
                    "offset": 0,
                    "origin": "model",
                }));
            }
        }
        if planned.is_empty() {
            return Err(bad("sharpened idea yielded zero valid queries to plan"));
        }
    }

    Ok((
        round,
        json!({
            "round": round,
            "offline": offline,
            "queries": planned,
        }),
    ))
}

/// HTTP handler for `POST /api/sessions/{id}/idea/plan` (`explore.md` 5.4, package X7).
pub fn handle_plan_op(
    state: &Arc<DesktopState>,
    ledger: &Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    if crate::session_lifecycle::is_turn_running(state, session_id) {
        return Err(crate::routes_session::conflict(
            "cannot create a search plan while an Explore run is active",
        ));
    }
    let req_val = if body.is_empty() { json!({}) } else { parse_json_body(body)? };
    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger lock poisoned"))?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    if is_session_offline(&events) {
        return Err(bad("Explore session is in offline (library-only) mode"));
    }
    let (round, plan_payload) = assemble_search_plan(&events, &req_val)?;
    let request_id = format!("search-r{round}");

    ledger
        .append(
            session_id,
            EVENT_IDEA_SEARCH_PLANNED,
            EVENT_SCHEMA_VERSION,
            &plan_payload,
        )
        .map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "session_event",
        json!({
            "session": session_id,
            "kind": EVENT_IDEA_SEARCH_PLANNED,
            "payload": plan_payload,
        }),
    );

    let sse = Arc::clone(&state.sse);
    let sid_sse = session_id.to_string();
    let mut on_prog = move |kind: &str, payload: Value| {
        sse.emit(
            &sid_sse,
            "session_event",
            json!({ "session": sid_sse, "kind": kind, "payload": payload }),
        );
    };
    emit_stage(
        ledger,
        session_id,
        IdeaStage::Search,
        ExploreStageState::PlanApproval,
        &request_id,
        Some("Awaiting network approval for planned search batch"),
        &mut on_prog,
    )
    .map_err(internal)?;

    let updated = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    Ok((
        200,
        json!({
            "ok": true,
            "round": round,
            "plan": plan_payload,
            "idea": idea_view::view(&updated, false),
        }),
    ))
}

pub(crate) fn collect_completed_qids(events: &[Event], plan_seq: u64) -> BTreeSet<String> {
    let mut done = BTreeSet::new();
    for ev in events
        .iter()
        .filter(|e| e.kind == crate::idea::EVENT_IDEA_SEARCH_FINISHED && e.seq > plan_seq)
    {
        if let Some(arr) = ev
            .payload
            .get("queries")
            .or_else(|| ev.payload.get("results"))
            .and_then(Value::as_array)
        {
            for r in arr {
                let st = r.get("status").and_then(Value::as_str).unwrap_or("");
                if matches!(st, "ok" | "disabled") {
                    if let Some(qid) = r.get("qid").and_then(Value::as_str) {
                        done.insert(qid.to_string());
                    }
                }
            }
        }
    }
    done
}

pub(crate) fn build_idea_search_args(to_run: &[Value]) -> Vec<Value> {
    to_run
        .iter()
        .map(|q| {
            let qid = q.get("qid").and_then(Value::as_str).unwrap_or("q").to_string();
            let rel_kind = q
                .get("relation")
                .and_then(|r| r.get("kind").and_then(Value::as_str).or_else(|| r.as_str()))
                .unwrap_or("none");
            let mut obj = serde_json::Map::new();
            obj.insert("qid".into(), Value::String(qid));
            obj.insert(
                "provider".into(),
                q.get("provider").cloned().unwrap_or_else(|| json!("arxiv")),
            );
            if let Some(q_str) = q.get("q").and_then(Value::as_str) {
                obj.insert("q".into(), Value::String(q_str.to_string()));
            }
            obj.insert("relation".into(), Value::String(rel_kind.to_string()));
            if let Some(k) = q.get("relation").and_then(|r| r.get("key")).and_then(Value::as_str) {
                obj.insert("key".into(), Value::String(k.to_string()));
            }
            if let Some(mr) = q.get("max_results").and_then(Value::as_u64) {
                obj.insert("max_results".into(), json!(mr.clamp(1, MAX_RESULTS_CAP as u64)));
            }
            if let Some(off) = q.get("offset").and_then(Value::as_u64).filter(|&n| n > 0) {
                obj.insert("offset".into(), json!(off.min(500)));
            }
            if let Some(pid) = q.get("perspective_id").and_then(Value::as_str) {
                obj.insert("perspective_id".into(), Value::String(pid.to_string()));
            }
            Value::Object(obj)
        })
        .collect()
}

pub(crate) fn merge_with_prior_candidates(
    events: &[Event],
    stmt_seq: u64,
    new_ranked_lists: &[research_tools::idea_rank::RankedServiceList],
) -> Vec<research_tools::idea_rank::RankedCandidate> {
    use research_tools::idea_rank::{
        collect_canonical_keys, merge_and_rank, RankedCandidate, MAX_CANDIDATES_PER_ROUND,
        PERSPECTIVE_BONUS,
    };
    use research_tools::paper_key::PaperRef;

    let prior_candidates: Vec<RankedCandidate> = events
        .iter()
        .rev()
        .find(|e| e.kind == crate::idea::EVENT_IDEA_CANDIDATES && e.seq > stmt_seq)
        .and_then(|e| e.payload.get("candidates"))
        .and_then(|v| serde_json::from_value::<Vec<RankedCandidate>>(v.clone()).ok())
        .unwrap_or_default();

    let aliases_of = |c: &RankedCandidate| -> Vec<PaperKey> {
        let pref = PaperRef { key: c.key.clone(), version: c.version.clone(), ids: c.ids.clone() };
        collect_canonical_keys(&pref)
    };

    let new_candidates = merge_and_rank(new_ranked_lists);
    let mut slots: Vec<RankedCandidate> = Vec::with_capacity(prior_candidates.len() + new_candidates.len());
    let mut alias_to_slot: std::collections::BTreeMap<PaperKey, usize> = std::collections::BTreeMap::new();
    for prev in prior_candidates {
        let idx = slots.len();
        for k in aliases_of(&prev) {
            alias_to_slot.entry(k).or_insert(idx);
        }
        slots.push(prev);
    }

    for fresh in new_candidates {
        let fresh_aliases = aliases_of(&fresh);
        let matched_slot = fresh_aliases.iter().find_map(|k| alias_to_slot.get(k).copied());
        if let Some(idx) = matched_slot {
            let existing = &mut slots[idx];
            let old_persp = existing.perspectives.len();
            if existing.ids.arxiv.is_none() { existing.ids.arxiv = fresh.ids.arxiv.clone(); }
            if existing.ids.doi.is_none() { existing.ids.doi = fresh.ids.doi.clone(); }
            if existing.ids.inspire.is_none() { existing.ids.inspire = fresh.ids.inspire.clone(); }
            if existing.ids.s2.is_none() { existing.ids.s2 = fresh.ids.s2.clone(); }
            if existing.version.is_none() { existing.version = fresh.version.clone(); }
            if let Ok(pref) = PaperRef::from_ids_or(
                existing.ids.clone(),
                existing.version.clone(),
                Some(existing.key.clone()),
            ) {
                existing.key = pref.key;
            }
            for qid in fresh.found_by {
                if !existing.found_by.contains(&qid) {
                    existing.found_by.push(qid);
                }
            }
            for pid in fresh.perspectives {
                if !existing.perspectives.contains(&pid) {
                    existing.perspectives.push(pid);
                }
            }
            let fresh_better_abs = fresh.abstract_text.is_some()
                && (existing.abstract_text.is_none()
                    || (fresh.abstract_ref.as_ref().is_some_and(|r| r.verbatim)
                        && !existing.abstract_ref.as_ref().is_some_and(|r| r.verbatim)));
            if fresh_better_abs {
                existing.abstract_text = fresh.abstract_text;
                existing.abstract_ref = fresh.abstract_ref;
            }
            if existing.citation_count.is_none() {
                existing.citation_count = fresh.citation_count;
            }
            existing.withdrawn_or_retracted |= fresh.withdrawn_or_retracted;
            existing.in_library |= fresh.in_library;
            existing.owned |= fresh.owned;
            existing.user_seed |= fresh.user_seed;
            existing.cited_by_origin |= fresh.cited_by_origin;
            let added_persp = existing.perspectives.len().saturating_sub(old_persp) as f64;
            existing.score = existing.score.max(fresh.score) + added_persp * PERSPECTIVE_BONUS;
            for k in aliases_of(existing) {
                alias_to_slot.insert(k, idx);
            }
        } else {
            let idx = slots.len();
            for k in fresh_aliases {
                alias_to_slot.entry(k).or_insert(idx);
            }
            slots.push(fresh);
        }
    }

    slots.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.key.cmp(&b.key)));
    slots.truncate(MAX_CANDIDATES_PER_ROUND);
    slots
}

pub(crate) fn summarize_provider_states(
    provider_counts: std::collections::BTreeMap<String, (usize, usize, usize, bool)>,
) -> Vec<Value> {
    provider_counts
        .into_iter()
        .map(|(name, (ok, failed, rl, disabled))| {
            let state = if disabled && ok == 0 && failed == 0 && rl == 0 {
                "disabled"
            } else if ok > 0 && failed == 0 && rl == 0 {
                "ok"
            } else if ok > 0 {
                "partial"
            } else if rl > 0 && failed == 0 {
                "rate_limited"
            } else {
                "failed"
            };
            json!({ "provider": name, "state": state })
        })
        .collect()
}

pub(crate) fn library_counts(data_dir: &std::path::Path) -> (usize, usize) {
    let Ok(lib) = research_tools::library::Library::open(data_dir) else { return (0, 0); };
    let Ok(all) = lib.search(&research_tools::library::LibraryQuery::default()) else { return (0, 0); };
    (all.len(), all.iter().filter(|p| p.has_fulltext).count())
}

