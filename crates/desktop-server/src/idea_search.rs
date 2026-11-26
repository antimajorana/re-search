//! Explore first-search execution (Rule P4 validation, F11 + F3 gate execution, X1
//! merge/rank, and F10 library preview/annotation/seen recording; package X3).

use crate::idea::{
    budget_snapshot, IdeaStage, IdeaStatement, EVENT_IDEA_CANDIDATES, EVENT_IDEA_SEARCH_FINISHED,
    EVENT_IDEA_SEARCH_PLANNED, EVENT_IDEA_STAGE, FIRST_SEARCH_MAX_RESULTS,
    MAX_PREVIEW_LIBRARY_MATCHES,
};
use agent::CancelToken;
use ledger::Ledger;
use research_tools::idea_queries::{
    first_queries_for, FirstQueryOptions, PlannedQuery, QueryOrigin, QueryService,
    MAX_USER_PREVIEW_QUERIES,
};
use research_tools::idea_rank::{
    merge_and_rank, AbstractRef, RankedCandidate, RankedServiceList, RawHit,
};
use research_tools::idea_terms::{extract as extract_idea_terms, ExtractedIdea};
use research_tools::library::{
    Library, LibraryIds, LibraryQuery, PaperFlags, PaperRecord, PaperSummary, SeenRecord, SeenRole,
    SourceItemState, ORIGIN_USER_LIBRARY,
};
use research_tools::lit_providers::{
    run_query_and_record_with_gate, LitPaper, LitProvider, LitQuery, LitQueryResult,
    ProviderConfig, QueryStatus,
};
use research_tools::net_rate::{global_gate, NetRateGate};
use research_tools::paper_fetch::parse_paper_input;
use research_tools::paper_key::{PaperKey, PaperRef};
use research_tools::run_budget::ExploreStageState;
use research_tools::LiteratureEndpoints;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use tools::EVENT_SCHEMA_VERSION;

/// Submitted first-search query item from the Explore form (`explore.md` Rule P4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedFirstQuery {
    pub provider: String,
    pub q: String,
}

/// Compute deterministic first queries shown on preview (`explore.md` Rule P4: arXiv & INSPIRE, <= 6).
pub fn preview_first_queries(extracted: &ExtractedIdea, fields: &[String]) -> Vec<PlannedQuery> {
    first_queries_for(
        extracted,
        &FirstQueryOptions {
            services: vec![QueryService::Arxiv, QueryService::Inspire],
            max_queries: MAX_USER_PREVIEW_QUERIES,
            categories: fields.to_vec(),
        },
    )
}

/// Rule P4 enforcement at session creation:
/// - When `offline == false`: `submitted` MUST be present and non-empty (unless preview itself
///   produced 0 queries), at most 6, strictly on `arxiv` or `inspire`, non-duplicate, and
///   byte-for-byte equal to `preview_first_queries`.
/// - When `offline == true`: validates any supplied queries against P4 and returns empty so no
///   unexecuted remote queries are planned or sent.
pub fn validate_first_queries(
    extracted: &ExtractedIdea,
    fields: &[String],
    submitted: Option<&[SubmittedFirstQuery]>,
    offline: bool,
) -> Result<Vec<PlannedQuery>, String> {
    let expected = preview_first_queries(extracted, fields);
    let items = match submitted {
        Some(list) => list,
        None if offline => return Ok(Vec::new()),
        None => {
            return Err(
                "`queries` returned by /api/idea/preview are required when `offline` is false (Rule P4); pass `offline: true` for library-only mode".to_string(),
            );
        }
    };
    if items.len() > MAX_USER_PREVIEW_QUERIES {
        return Err(format!(
            "at most {MAX_USER_PREVIEW_QUERIES} user-shown first queries are allowed at session creation (got {})",
            items.len()
        ));
    }
    if !offline && items.is_empty() && !expected.is_empty() {
        return Err(
            "at least one preview query is required when `offline` is false (pass `offline: true` for library-only mode)".to_string(),
        );
    }

    let mut out = Vec::with_capacity(items.len());
    let mut seen = BTreeSet::new();
    for item in items {
        let prov = match item.provider.trim().to_ascii_lowercase().as_str() {
            "arxiv" => QueryService::Arxiv,
            "inspire" => QueryService::Inspire,
            other => {
                return Err(format!(
                    "provider `{other}` is not allowed for user-shown first queries (only `arxiv` and `inspire`)"
                ));
            }
        };
        let Some(matched) = expected.iter().find(|e| e.provider == prov && e.q == item.q) else {
            return Err(format!(
                "query `{}` for `{}` does not match the exact preview queries (Rule P4)",
                item.q,
                prov.as_str()
            ));
        };
        if !seen.insert((prov, item.q.clone())) {
            return Err(format!("duplicate first query `{}` for `{}`", item.q, prov.as_str()));
        }
        out.push(PlannedQuery {
            qid: format!("q-{}", out.len() + 1),
            provider: prov,
            q: matched.q.clone(),
            label: matched.label.clone(),
            origin: QueryOrigin::User,
        });
    }
    if offline {
        return Ok(Vec::new());
    }
    Ok(out)
}

/// True iff `text` is solely a bare arXiv ID, DOI, or arXiv link (`00-overview.md` 5.1 paste rule).
pub fn is_single_paper_identifier(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.split_whitespace().count() != 1 {
        return false;
    }
    parse_paper_input(trimmed).is_ok()
}

/// Query F10's local library for matches against extracted terms, detected IDs, and seeds (zero network).
pub fn search_library_matches(
    data_dir: &Path,
    extracted: &ExtractedIdea,
    seeds: &[PaperKey],
    limit: usize,
) -> Vec<PaperSummary> {
    let Ok(lib) = Library::open(data_dir) else {
        return Vec::new();
    };
    let cap = limit.clamp(1, MAX_PREVIEW_LIBRARY_MATCHES);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    let mut push_key = |key: &PaperKey| {
        if out.len() >= cap { return; }
        if let Ok(rows) = lib.search(&LibraryQuery {
            key: Some(key.clone()),
            limit: Some(1),
            ..Default::default()
        }) {
            for r in rows {
                if seen.insert(r.paper.key.clone()) {
                    out.push(r);
                }
            }
        }
    };

    for s in seeds { push_key(s); }
    for r in &extracted.paper_refs { push_key(&r.key); }

    let mut query_terms: Vec<String> = Vec::new();
    for t in extracted
        .quoted_phrases
        .iter()
        .chain(extracted.proper_names.iter())
        .chain(extracted.keywords.iter())
    {
        if query_terms.len() >= 6 { break; }
        if !query_terms.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            query_terms.push(t.clone());
        }
    }
    for term in query_terms {
        if out.len() >= cap { break; }
        if let Ok(rows) = lib.search(&LibraryQuery {
            q: Some(term),
            limit: Some(cap),
            ..Default::default()
        }) {
            for r in rows {
                if out.len() >= cap { break; }
                if seen.insert(r.paper.key.clone()) {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// Run the initial user-shown first search (plus local F10 library matches) via an explicit F3
/// [`NetRateGate`], write `idea.search_planned`, `literature.fetched`, `idea.search_finished`,
/// `idea.candidates`, and `idea.stage`, and record F10 `seen` (`Candidate`).
#[allow(clippy::too_many_arguments)]
pub fn run_first_search_with_gate<F>(
    data_dir: &Path,
    endpoints: &LiteratureEndpoints,
    ledger: &Ledger,
    session_id: &str,
    stmt: &IdeaStatement,
    planned: &[PlannedQuery],
    offline: bool,
    gate: &NetRateGate,
    cancel: Option<&CancelToken>,
    mut on_progress: F,
) -> Result<Vec<RankedCandidate>, String>
where
    F: FnMut(&str, Value),
{
    let existing = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let prior_rounds = existing
        .iter()
        .filter(|e| e.kind == EVENT_IDEA_SEARCH_PLANNED)
        .count() as u64;
    let round = prior_rounds + 1;
    let request_id = format!("search-r{round}");

    let stage_req = json!({
        "stage": IdeaStage::Search.as_str(),
        "state": ExploreStageState::Requested.as_str(),
        "request_id": request_id,
    });
    ledger
        .append(session_id, EVENT_IDEA_STAGE, EVENT_SCHEMA_VERSION, &stage_req)
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_STAGE, stage_req);

    let effective_planned: &[PlannedQuery] = if offline { &[] } else { planned };
    let plan_queries: Vec<Value> = effective_planned
        .iter()
        .map(|q| {
            json!({
                "qid": q.qid,
                "perspective_id": Value::Null,
                "provider": q.provider.as_str(),
                "q": q.q,
                "relation": { "kind": "none" },
                "origin": "user",
            })
        })
        .collect();
    let plan_payload = json!({ "round": round, "offline": offline, "queries": plan_queries });
    ledger
        .append(session_id, EVENT_IDEA_SEARCH_PLANNED, EVENT_SCHEMA_VERSION, &plan_payload)
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_SEARCH_PLANNED, plan_payload);

    let mut ranked_lists: Vec<RankedServiceList> = Vec::new();
    let mut query_outcomes: Vec<Value> = Vec::new();
    let mut attempted_ok = 0usize;
    let mut attempted_total = 0usize;
    let mut hit_cap = false;
    let mut cancelled = false;

    if !offline {
        let snap = budget_snapshot(&existing, true);
        let prior_used = snap.queries_used as usize;
        let max_q = snap.max_queries as usize;
        let cfg = ProviderConfig::default();

        for pq in effective_planned {
            if cancel.is_some_and(CancelToken::is_cancelled) {
                cancelled = true;
                break;
            }
            if prior_used.saturating_add(attempted_total) >= max_q {
                hit_cap = true;
                query_outcomes.push(json!({
                    "qid": pq.qid, "provider": pq.provider.as_str(), "q": pq.q,
                    "status": "skipped_budget", "http_status": Value::Null, "count": 0,
                }));
                continue;
            }
            attempted_total += 1;
            let lit_prov = match pq.provider {
                QueryService::Arxiv => LitProvider::Arxiv,
                QueryService::Inspire => LitProvider::Inspire,
                QueryService::SemanticScholar => LitProvider::SemanticScholar,
                QueryService::Crossref => LitProvider::Crossref,
            };
            let lq = LitQuery::search(lit_prov, pq.q.clone())
                .with_paging(FIRST_SEARCH_MAX_RESULTS, 0)
                .with_tracking(Some(pq.qid.clone()), Some(request_id.clone()));
            let res: LitQueryResult =
                run_query_and_record_with_gate(endpoints, ledger, session_id, &lq, &cfg, gate)?;
            if res.status == QueryStatus::Ok {
                attempted_ok += 1;
                let hits: Vec<RawHit> = res.papers.iter().map(lit_paper_to_raw_hit).collect();
                ranked_lists.push(RankedServiceList {
                    qid: pq.qid.clone(),
                    provider: pq.provider.as_str().to_string(),
                    perspective_id: None,
                    hits,
                });
            }
            let outcome = json!({
                "qid": pq.qid,
                "provider": pq.provider.as_str(),
                "q": pq.q,
                "status": res.status,
                "http_status": res.http_status,
                "count": res.papers.len(),
                "error": res.error,
                "retry_after_secs": res.retry_after_secs,
            });
            query_outcomes.push(outcome.clone());
            on_progress("idea.query_result", outcome);
        }
    }

    // Include local F10 library matches in both offline and online modes (except when all online
    // services failed, where `explore.md` 3 mandates "All services failed: no result is shown").
    let all_online_failed = !offline && attempted_total > 0 && attempted_ok == 0;
    if !all_online_failed {
        let extracted = extract_idea_terms(&stmt.text);
        let matches =
            search_library_matches(data_dir, &extracted, &stmt.seeds, MAX_PREVIEW_LIBRARY_MATCHES);
        let hits: Vec<RawHit> = matches.into_iter().map(summary_to_raw_hit).collect();
        if !hits.is_empty() {
            ranked_lists.push(RankedServiceList {
                qid: "library".to_string(),
                provider: "library".to_string(),
                perspective_id: None,
                hits,
            });
        }
    }

    let mut candidates = merge_and_rank(&ranked_lists);
    annotate_and_record_library(data_dir, session_id, &stmt.seeds, &mut candidates);

    let finished_payload = json!({ "round": round, "offline": offline, "results": query_outcomes });
    ledger
        .append(session_id, EVENT_IDEA_SEARCH_FINISHED, EVENT_SCHEMA_VERSION, &finished_payload)
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_SEARCH_FINISHED, finished_payload);

    let cands_payload = json!({ "round": round, "candidates": candidates });
    ledger
        .append(session_id, EVENT_IDEA_CANDIDATES, EVENT_SCHEMA_VERSION, &cands_payload)
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_CANDIDATES, cands_payload);

    let (final_state, reason) = if cancelled {
        (ExploreStageState::Cancelled, Some("Stopped by user".to_string()))
    } else if hit_cap {
        (ExploreStageState::StoppedAtCap, Some("Query budget cap reached".to_string()))
    } else if all_online_failed {
        (ExploreStageState::Failed, Some("All services failed: no result is shown".to_string()))
    } else {
        (ExploreStageState::Finished, None)
    };
    let stage_end = json!({
        "stage": IdeaStage::Search.as_str(),
        "state": final_state.as_str(),
        "request_id": request_id,
        "reason": reason,
    });
    ledger
        .append(session_id, EVENT_IDEA_STAGE, EVENT_SCHEMA_VERSION, &stage_end)
        .map_err(|e| e.to_string())?;
    on_progress(EVENT_IDEA_STAGE, stage_end);
    Ok(candidates)
}

/// Convenience wrapper using F3's process-wide [`global_gate`] (Rule P6).
#[allow(clippy::too_many_arguments)]
pub fn run_first_search<F>(
    data_dir: &Path,
    endpoints: &LiteratureEndpoints,
    ledger: &Ledger,
    session_id: &str,
    stmt: &IdeaStatement,
    planned: &[PlannedQuery],
    offline: bool,
    cancel: Option<&CancelToken>,
    on_progress: F,
) -> Result<Vec<RankedCandidate>, String>
where
    F: FnMut(&str, Value),
{
    run_first_search_with_gate(
        data_dir, endpoints, ledger, session_id, stmt, planned, offline, global_gate(), cancel,
        on_progress,
    )
}

fn lit_paper_to_raw_hit(p: &LitPaper) -> RawHit {
    RawHit {
        paper: p.paper_ref.clone(),
        title: p.title.clone(),
        authors: p.authors.clone(),
        year: p.year,
        abstract_text: p.abstract_text.clone(),
        abstract_verbatim: p.abstract_ref.as_ref().is_some_and(|r| r.verbatim),
        abstract_raw_blob: p.abstract_ref.as_ref().map(|r| r.raw_blob.clone()),
        citation_count: p.citation_count,
        withdrawn_or_retracted: p.withdrawn_or_retracted,
        in_library: false,
        owned: false,
        user_seed: false,
        cited_by_origin: false,
    }
}

fn summary_to_raw_hit(s: PaperSummary) -> RawHit {
    let paper = PaperRef {
        key: s.paper.key.clone(),
        version: None,
        ids: s.paper.ids.to_paper_ids(),
    };
    RawHit {
        paper,
        title: s.paper.title,
        authors: s.paper.authors,
        year: s.paper.year,
        abstract_text: (!s.paper.abstract_text.is_empty()).then_some(s.paper.abstract_text),
        abstract_verbatim: false,
        abstract_raw_blob: None,
        citation_count: None,
        withdrawn_or_retracted: s.paper.flags.withdrawn || s.paper.flags.retracted,
        in_library: true,
        owned: s.owned,
        user_seed: false,
        cited_by_origin: false,
    }
}

/// Annotate candidates with existing F10 `in_library` / `owned` status (resolving all canonical
/// aliases and F13a `source_items`) BEFORE recording `SeenRole::Candidate`.
fn annotate_and_record_library(
    data_dir: &Path,
    session_id: &str,
    seeds: &[PaperKey],
    candidates: &mut [RankedCandidate],
) {
    let seed_set: BTreeSet<&PaperKey> = seeds.iter().collect();
    let lib = Library::open(data_dir).ok();
    let now = crate::state::now_ms();

    for cand in candidates.iter_mut() {
        let lib_ids = LibraryIds::from_paper_ids(&cand.ids);
        let mut lookup_keys: Vec<PaperKey> = vec![cand.key.clone()];
        for alias in lib_ids.canonical_aliases() {
            if let Ok(pk) = PaperKey::parse(&alias) {
                if !lookup_keys.contains(&pk) {
                    lookup_keys.push(pk);
                }
            }
        }
        if lookup_keys.iter().any(|k| seed_set.contains(k)) {
            cand.user_seed = true;
        }
        if let Some(ref l) = lib {
            let mut found_detail = None;
            for k in &lookup_keys {
                if let Ok(Some(d)) = l.get_paper(k) {
                    found_detail = Some(d);
                    break;
                }
            }
            if let Some(detail) = found_detail {
                cand.in_library = true;
                if detail.paper.flags.owned
                    || detail.fulltext.iter().any(|ft| ft.origin == ORIGIN_USER_LIBRARY)
                    || detail.source_items.iter().any(|si| si.state != SourceItemState::Missing)
                {
                    cand.owned = true;
                }
            } else {
                let rec = PaperRecord {
                    key: cand.key.clone(),
                    ids: lib_ids,
                    title: cand.title.clone(),
                    authors: cand.authors.clone(),
                    year: cand.year,
                    abstract_text: cand.abstract_text.clone().unwrap_or_default(),
                    bibtex: String::new(),
                    flags: PaperFlags {
                        withdrawn: cand.withdrawn_or_retracted,
                        retracted: false,
                        owned: cand.owned,
                        license: None,
                        venue: None,
                    },
                    first_seen_ms: now,
                    last_seen_ms: now,
                };
                let _ = l.upsert_paper(&rec);
            }
            let _ = l.record_seen(&SeenRecord {
                key: cand.key.clone(),
                session_id: session_id.to_string(),
                role: SeenRole::Candidate,
                seen_ms: now,
            });
        }
    }
}

/// Expose `AbstractRef` conversion helper cleanly if needed by X8/X11.
pub fn candidate_abstract_ref(c: &RankedCandidate) -> Option<&AbstractRef> {
    c.abstract_ref.as_ref()
}
