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
        if let Some(rl) = p.get("reading_list").and_then(Value::as_array) {
            let keys: Vec<String> = rl
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            reading_list_override = Some(keys);
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
    finished: Option<&Value>,
    candidates: &[Value],
) -> (Value, SearchCoverage) {
    let coverage = compute_coverage_from_events(events);
    let mut by_provider: BTreeMap<String, (u64, u64, u64, u64)> = BTreeMap::new();
    for &(id, _) in KNOWN_PROVIDERS {
        by_provider.insert(id.to_string(), (0, 0, 0, 0));
    }

    let mut query_rows: Vec<Value> = Vec::new();
    let results_arr = finished
        .and_then(|f| f.get("results").or_else(|| f.get("queries")))
        .and_then(Value::as_array);

    if let Some(results) = results_arr {
        for r in results {
            let prov = r.get("provider").and_then(Value::as_str).unwrap_or("unknown");
            let status = r.get("status").and_then(Value::as_str).unwrap_or("unknown");
            let count = r
                .get("count")
                .or_else(|| r.get("hits"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let entry = by_provider.entry(prov.to_string()).or_insert((0, 0, 0, 0));
            match status {
                "ok" => {
                    entry.0 += 1;
                    entry.3 += count;
                }
                "rate_limited" => entry.2 += 1,
                "skipped_budget" | "not_configured" | "disabled" => {}
                _ => entry.1 += 1,
            }
            query_rows.push(json!({
                "qid": r.get("qid").and_then(Value::as_str).unwrap_or(""),
                "provider": prov,
                "q": r.get("q").and_then(Value::as_str).unwrap_or(""),
                "status": status,
                "http_status": r.get("http_status").cloned().unwrap_or(Value::Null),
                "count": count,
                "error": r.get("error").cloned().unwrap_or(Value::Null),
            }));
        }
    } else if let Some(planned_qs) = plan.and_then(|p| p.get("queries")).and_then(Value::as_array) {
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

    let providers: Vec<Value> = KNOWN_PROVIDERS
        .iter()
        .map(|&(id, label)| {
            let (ok, failed, rate_limited, hits) =
                by_provider.get(id).copied().unwrap_or((0, 0, 0, 0));
            let cov_prov = coverage.providers.iter().find(|p| p.provider.as_str() == id);
            let state = if offline {
                "offline"
            } else if ok == 0 && failed == 0 && rate_limited == 0 {
                if let Some(cp) = cov_prov {
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
                }
            } else if failed == 0 && rate_limited == 0 {
                "ok"
            } else if ok > 0 {
                "partial"
            } else if rate_limited > 0 && failed == 0 {
                "rate_limited"
            } else {
                "failed"
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
