//! Helper projections for `idea_view.rs`: F12 coverage/where-I-looked projection,
//! sharpened-idea answer merging, and F6 GraphView map edge translation (X6–X8).
//!
//! Kept in a sibling module so `idea_view.rs` remains comfortably under 600 lines.

use ledger::Event;
use research_tools::idea_brief::{EVENT_IDEA_ANSWER, UI_SHARPENED_IDEA_LABEL};
use research_tools::prior_work_wording::{
    format_coverage_scope, format_library_coverage, format_provider_coverage,
};
use research_tools::search_coverage::{
    compute_coverage_from_events, ProviderCoverageState, SearchCoverage,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Fixed app-authored scope caveats shown under "Where I looked" (`explore.md` 4.1 Coverage).
pub const NOT_SEARCHED_ITEMS: &[&str] = &[
    "books and monographs",
    "theses not on arXiv",
    "talks and unpublished notes",
    "non-English literature",
    "most work before 1991 without a DOI unless it is in your library",
    "paywalled full text that is not in your library",
];

const KNOWN_PROVIDERS: &[(&str, &str)] = &[
    ("arxiv", "arXiv"),
    ("inspire", "INSPIRE-HEP"),
    ("crossref", "Crossref"),
    ("semantic_scholar", "Semantic Scholar"),
    ("openalex", "OpenAlex"),
    ("theoremsearch", "TheoremSearch"),
];

/// Effective sharpened-idea state after applying subsequent `idea.answer` events.
pub struct AppliedAnswers {
    pub sharpened_idea: Option<Value>,
    pub question_answers: BTreeMap<String, String>,
    pub reading_list_override: Option<Vec<String>>,
    pub pause_before_reading: bool,
}

/// Fold `idea.answer` events (after the active `idea.stated` revision) onto the latest
/// `idea.brief` payload without mutating the original ledger record.
pub fn fold_answers(
    events: &[Event],
    latest_statement_seq: u64,
    latest_brief: Option<&Value>,
    latest_brief_seq: u64,
    latest_triage_seq: u64,
) -> AppliedAnswers {
    let mut merged = latest_brief.cloned();
    let mut question_answers: BTreeMap<String, String> = BTreeMap::new();
    let mut reading_list_override: Option<Vec<String>> = None;
    let mut pause_before_reading = false;

    for ev in events {
        if ev.kind != EVENT_IDEA_ANSWER || ev.seq <= latest_statement_seq {
            continue;
        }
        let p = &ev.payload;
        if let Some(pause) = p.get("pause_before_reading").and_then(Value::as_bool) {
            pause_before_reading = pause;
        }
        if ev.seq >= latest_triage_seq {
            if let Some(rl) = p.get("reading_list").and_then(Value::as_array) {
                let keys: Vec<String> = rl
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                reading_list_override = Some(keys);
            }
        }
        if ev.seq < latest_brief_seq {
            continue;
        }
        if let Some(ans_obj) = p.get("answers").and_then(Value::as_object) {
            for (qid, ans) in ans_obj {
                if let Some(text) = ans.as_str() {
                    question_answers.insert(qid.clone(), text.to_string());
                }
            }
        } else if let Some(ans_arr) = p.get("answers").and_then(Value::as_array) {
            for item in ans_arr {
                if let (Some(qid), Some(ans)) = (
                    item.get("question_id").and_then(Value::as_str),
                    item.get("answer").and_then(Value::as_str),
                ) {
                    question_answers.insert(qid.to_string(), ans.to_string());
                }
            }
        } else if let (Some(qid), Some(ans)) = (
            p.get("question_id").and_then(Value::as_str),
            p.get("answer").and_then(Value::as_str),
        ) {
            question_answers.insert(qid.to_string(), ans.to_string());
        }

        if let Some(Value::Object(ref mut bobj)) = merged {
            if let Some(restatement) = p.get("restatement").and_then(Value::as_str) {
                bobj.insert("restatement".into(), Value::String(restatement.to_string()));
            }
            if let Some(assumptions) = p.get("assumptions").and_then(Value::as_array) {
                bobj.insert("assumptions".into(), Value::Array(assumptions.clone()));
            }
            if let Some(perspectives) = p.get("perspectives").and_then(Value::as_array) {
                bobj.insert("perspectives".into(), Value::Array(perspectives.clone()));
            }
            if let Some(exclusions) = p.get("exclusions").and_then(Value::as_array) {
                bobj.insert("exclusions".into(), Value::Array(exclusions.clone()));
            }
        }
    }

    if let Some(Value::Object(ref mut bobj)) = merged {
        bobj.insert(
            "heading".into(),
            Value::String(UI_SHARPENED_IDEA_LABEL.to_string()),
        );
        let ans_map: Map<String, Value> = question_answers
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        bobj.insert("answers".into(), Value::Object(ans_map));
    }

    AppliedAnswers {
        sharpened_idea: merged,
        question_answers,
        reading_list_override,
        pause_before_reading,
    }
}

/// Build `"Where I looked"` combining per-query ledger outcomes with F12 [`SearchCoverage`]
/// and [`prior_work_wording`] disclosures (`explore.md` 3 & 4.1).
pub fn build_where_i_looked(
    events: &[Event],
    offline: bool,
    plan: Option<&Value>,
    _finished: Option<&Value>,
    candidates: &[Value],
) -> (Value, SearchCoverage) {
    let coverage = compute_coverage_from_events(events);
    let latest_stmt_seq = events
        .iter()
        .rev()
        .find(|e| e.kind == "idea.stated")
        .map(|e| e.seq)
        .unwrap_or(0);

    // Accumulate latest query outcome per `qid` across all rounds since the latest statement
    // so single-provider retries update their own rows without erasing other services' rows.
    let mut query_order: Vec<String> = Vec::new();
    let mut by_qid: BTreeMap<String, Value> = BTreeMap::new();
    for ev in events.iter().filter(|e| e.seq > latest_stmt_seq) {
        if ev.kind != "idea.search_finished" {
            continue;
        }
        let Some(arr) = ev
            .payload
            .get("queries")
            .or_else(|| ev.payload.get("results"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for r in arr {
            let qid = r.get("qid").and_then(Value::as_str).unwrap_or("").to_string();
            let key = if qid.is_empty() {
                format!("seq-{}-{}", ev.seq, query_order.len())
            } else {
                qid.clone()
            };
            if !by_qid.contains_key(&key) {
                query_order.push(key.clone());
            }
            let count = r
                .get("count")
                .or_else(|| r.get("hits"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            by_qid.insert(
                key,
                json!({
                    "qid": qid,
                    "provider": r.get("provider").and_then(Value::as_str).unwrap_or("unknown"),
                    "q": r.get("q").and_then(Value::as_str).unwrap_or(""),
                    "status": r.get("status").and_then(Value::as_str).unwrap_or("unknown"),
                    "http_status": r.get("http_status").cloned().unwrap_or(Value::Null),
                    "count": count,
                    "error": r.get("error").cloned().unwrap_or(Value::Null),
                }),
            );
        }
    }

    let mut query_rows: Vec<Value> = query_order
        .into_iter()
        .filter_map(|k| by_qid.remove(&k))
        .collect();
    if query_rows.is_empty() {
        if let Some(planned_qs) = plan.and_then(|p| p.get("queries")).and_then(Value::as_array) {
            for q in planned_qs {
                query_rows.push(json!({
                    "qid": q.get("qid").and_then(Value::as_str).unwrap_or(""),
                    "provider": q.get("provider").and_then(Value::as_str).unwrap_or(""),
                    "q": q.get("q").and_then(Value::as_str).unwrap_or(""),
                    "status": if offline { "offline" } else { "planned" },
                    "http_status": Value::Null,
                    "count": 0,
                    "error": Value::Null,
                }));
            }
        }
    }

    let providers: Vec<Value> = KNOWN_PROVIDERS
        .iter()
        .map(|&(id, label)| {
            let cov_prov = coverage.providers.iter().find(|p| p.provider.as_str() == id);
            let (ok, failed, rate_limited, hits) = cov_prov
                .map(|cp| (cp.queries_ok, cp.queries_failed, cp.queries_rate_limited, cp.hits))
                .unwrap_or((0, 0, 0, 0));
            let state = if offline {
                "offline"
            } else if matches!(id, "openalex" | "theoremsearch") {
                "disabled"
            } else if let Some(cp) = cov_prov {
                match cp.state {
                    ProviderCoverageState::Ok => "ok",
                    ProviderCoverageState::Partial => "partial",
                    ProviderCoverageState::Failed => "failed",
                    ProviderCoverageState::RateLimited => "rate_limited",
                    ProviderCoverageState::NotConfigured => "not_configured",
                    ProviderCoverageState::Disabled => "disabled",
                    ProviderCoverageState::NotAttempted => {
                        if matches!(id, "arxiv" | "inspire" | "crossref") {
                            "not_attempted"
                        } else {
                            "not_configured"
                        }
                    }
                }
            } else if matches!(id, "arxiv" | "inspire" | "crossref") {
                "not_attempted"
            } else {
                "not_configured"
            };
            let retryable = !offline && matches!(state, "partial" | "failed" | "rate_limited");
            let summary = cov_prov.map(format_provider_coverage);
            json!({
                "name": id,
                "label": label,
                "state": state,
                "queries_ok": ok,
                "queries_failed": failed,
                "queries_rate_limited": rate_limited,
                "hits": hits,
                "retryable": retryable,
                "summary": summary,
            })
        })
        .collect();

    let lib_hits = candidates
        .iter()
        .filter(|c| c.get("in_library").and_then(Value::as_bool).unwrap_or(false))
        .count();
    let owned_hits = candidates
        .iter()
        .filter(|c| c.get("owned").and_then(Value::as_bool).unwrap_or(false))
        .count();

    let where_val = json!({
        "offline": offline,
        "providers": providers,
        "queries": query_rows,
        "library": {
            "papers": coverage.library.papers,
            "with_full_text": coverage.library.with_full_text,
            "hits": lib_hits.max(coverage.library.hits),
            "owned_hits": owned_hits,
            "summary": format_library_coverage(&coverage.library),
        },
        "known_paper_recall": coverage.known_paper_recall,
        "gate": coverage.gate,
        "scope_summary": format_coverage_scope(&coverage),
        "not_searched": NOT_SEARCHED_ITEMS,
    });
    (where_val, coverage)
}

/// Build the batched-approval descriptor for the shared `approval_card.js` component when
/// `Search` is awaiting `plan_approval` (`explore.md` 3 & 5.2, `00-overview.md` 3.1 row 13).
pub fn build_plan_approval(
    latest_plan: Option<&Value>,
    search_awaiting_approval: bool,
) -> Value {
    let Some(plan) = latest_plan.filter(|_| search_awaiting_approval) else {
        return Value::Null;
    };
    let queries = plan
        .get("queries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if queries.is_empty() {
        return Value::Null;
    }
    let mut services: Vec<String> = Vec::new();
    for q in &queries {
        if let Some(p) = q.get("provider").and_then(Value::as_str) {
            if !services.iter().any(|s| s == p) {
                services.push(p.to_string());
            }
        }
    }
    let svc_list = services.join(", ");
    json!({
        "round": plan.get("round").and_then(Value::as_u64).unwrap_or(1),
        "queries": queries,
        "services": services,
        "grants": [{
            "id": "explore-network-read",
            "kind": "session_capability",
            "label": format!("Allow literature search ({svc_list}) for this session"),
            "tool_id": "idea_search",
            "effect": "network-read",
            "granted": false,
            "status": "required",
            "note": "Covers the listed search batch through Router::try_call."
        }]
    })
}

/// Returns a `GraphView`-shaped payload (`00-overview.md` 2.6: `{ nodes, edges, legend }`)
/// translating `explore.md` 4.1 edge direction (`from`/`to` -> `src`/`dst`) and `basis`.
pub fn build_map_graph_view(latest_map: Option<&Value>) -> Value {
    let Some(map_val) = latest_map else {
        return json!({ "nodes": [], "edges": [], "legend": [] });
    };
    let nodes = map_val
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges: Vec<Value> = map_val
        .get("edges")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(translate_map_edge).collect())
        .unwrap_or_default();
    let legend = map_val
        .get("legend")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    json!({
        "nodes": nodes,
        "edges": edges,
        "legend": legend,
    })
}

fn translate_map_edge(raw: &Value) -> Value {
    let kind = raw.get("kind").and_then(Value::as_str).unwrap_or("");
    let from = raw
        .get("src")
        .or_else(|| raw.get("from"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let to = raw
        .get("dst")
        .or_else(|| raw.get("to"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let (src, dst, symmetric) = match kind {
        "uses" | "special_case_of" | "answers" | "cites" => (to, from, false),
        "contradicts" | "same_as" => (from, to, true),
        _ => (from, to, false),
    };
    let basis = match raw.get("basis").and_then(Value::as_str).unwrap_or("model") {
        "metadata" | "code" => "code",
        "evidence" => "evidence",
        _ => "model",
    };
    json!({
        "src": src,
        "dst": dst,
        "kind": kind,
        "role": raw.get("role").and_then(Value::as_str).unwrap_or(kind),
        "basis": basis,
        "symmetric": symmetric,
    })
}
