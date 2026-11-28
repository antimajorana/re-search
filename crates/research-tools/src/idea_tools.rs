//! Frozen `explore/1` tool definitions, `idea_search` handler, and fail-closed X9 stub for
//! `idea_fetch_papers` (`explore.md` 5.2, rules P5, P6, P7, P8; package X7).
//!
//! All 6 literature services (`arxiv`, `inspire`, `semantic_scholar`, `crossref`, `openalex`,
//! `theoremsearch`) are listed in `idea_search`'s provider enum from day one so X19 never
//! changes `def_hash`. Both `idea_search` and `idea_fetch_papers` are frozen here so X9 and
//! X15 wire `idea_fetch_papers` and Conversation turns with zero `def_hash` churn.

use crate::lit_providers::{
    record_literature_fetched, run_query_with_gate, LitProvider, LitQuery, ProviderConfig,
    QueryRelation, QueryStatus, DEFAULT_MAX_RESULTS, MAX_RESULTS_CAP,
};
use crate::net_rate::{global_gate, NetRateGate};
use crate::paper_key::PaperKey;
use crate::{paper_lines, paper_passages, ResearchEnv};
use policy::EffectClass;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef, ToolHandler};

/// Frozen tool id for batched Explore literature search (`explore.md` 5.2).
pub const IDEA_SEARCH: &str = "idea_search";
/// Frozen tool id for fetching papers on the reading list (`explore.md` 5.2; wired in X9).
pub const IDEA_FETCH_PAPERS: &str = "idea_fetch_papers";

/// Maximum queries permitted in a single `idea_search` batch (`explore.md` 5.2).
pub const MAX_IDEA_SEARCH_QUERIES: usize = 24;
/// Maximum character length of a single search query string (`explore.md` 5.2).
pub const MAX_IDEA_QUERY_CHARS: usize = 300;
/// Maximum keys permitted in a single `idea_fetch_papers` call (`explore.md` 5.2).
pub const MAX_IDEA_FETCH_KEYS: usize = 8;

/// Exact sorted 4-tool inventory of profile `explore/1` (`explore.md` 5.2).
/// Zero `ExecuteCode`, zero `WriteWorkspace`, zero LaTeX tools (Rule P7).
pub const EXPLORE_V1_TOOL_IDS: &[&str; 4] = &[
    IDEA_FETCH_PAPERS,
    IDEA_SEARCH,
    "read_paper_lines",
    "search_paper_passages",
];

const ALL_PROVIDER_ENUM: &[&str; 6] = &[
    "arxiv",
    "inspire",
    "semantic_scholar",
    "crossref",
    "openalex",
    "theoremsearch",
];

/// Canonical frozen definition of `idea_search` (`explore.md` 5.2, package X7).
pub fn idea_search_def() -> ToolDef {
    ToolDef {
        id: IDEA_SEARCH.into(),
        description: "Run a batched literature search (1..=24 queries) across scholarly services \
                      (`arxiv`, `inspire`, `semantic_scholar`, `crossref`, `openalex`, \
                      `theoremsearch`). Disabled services return status `disabled` without \
                      network access. One batched approval covers all listed queries."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_IDEA_SEARCH_QUERIES,
                    "items": {
                        "type": "object",
                        "properties": {
                            "qid": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 64
                            },
                            "provider": {
                                "type": "string",
                                "enum": ALL_PROVIDER_ENUM
                            },
                            "q": {
                                "type": "string",
                                "maxLength": MAX_IDEA_QUERY_CHARS
                            },
                            "relation": {
                                "type": "string",
                                "enum": ["none", "cites", "cited_by"]
                            },
                            "key": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128
                            },
                            "max_results": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": MAX_RESULTS_CAP
                            },
                            "offset": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 500
                            },
                            "perspective_id": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 64
                            }
                        },
                        "required": ["qid", "provider"],
                        "additionalProperties": false
                    }
                },
                "run_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64
                }
            },
            "required": ["queries"],
            "additionalProperties": false
        }),
        effect: EffectClass::NetworkRead,
        approval_scope: ApprovalScope::Once,
    }
}

/// Canonical frozen definition of `idea_fetch_papers` (`explore.md` 5.2, frozen in X7, wired in X9).
pub fn idea_fetch_papers_def() -> ToolDef {
    ToolDef {
        id: IDEA_FETCH_PAPERS.into(),
        description: "Fetch 1..=8 candidate papers by canonical PaperKey (library first, then \
                      arXiv e-print, then PDF) and return per-key `source_id`, `status`, \
                      `total_lines`, and `origin`."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "keys": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_IDEA_FETCH_KEYS,
                    "items": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 128
                    }
                }
            },
            "required": ["keys"],
            "additionalProperties": false
        }),
        effect: EffectClass::NetworkRead,
        approval_scope: ApprovalScope::Once,
    }
}

pub use crate::idea_fetch::{idea_fetch_papers_handler, idea_fetch_papers_handler_with_gate};

/// Deprecated X7 placeholder handler retained for explicit stub tests.
pub fn idea_fetch_papers_stub_handler() -> Arc<dyn ToolHandler> {
    Arc::new(|_ctx: &CallCtx<'_>, _args: &Value| {
        Err("idea_fetch_papers: not yet wired (package X9)".to_string())
    })
}

/// Complete sorted 4-tool definitions for profile `explore/1` (`explore.md` 5.2).
pub fn explore_v1_defs() -> Vec<ToolDef> {
    let mut defs = vec![
        idea_fetch_papers_def(),
        idea_search_def(),
        paper_lines::def(),
        paper_passages::def(),
    ];
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
}

/// Register the 4 frozen `explore/1` tools into `registry`.
pub fn register_explore_v1_tools(
    registry: &mut Registry,
    env: Arc<ResearchEnv>,
    fetch_handler: Option<Arc<dyn ToolHandler>>,
) {
    registry.register(idea_search_def(), idea_search_handler(Arc::clone(&env)));
    registry.register(
        idea_fetch_papers_def(),
        fetch_handler.unwrap_or_else(|| idea_fetch_papers_handler(Arc::clone(&env))),
    );
    paper_lines::register(registry, Arc::clone(&env));
    paper_passages::register(registry, env);
}

#[derive(Debug, Clone)]
struct ParsedBatchQuery {
    qid: String,
    perspective_id: Option<String>,
    provider: LitProvider,
    q: String,
    relation: QueryRelation,
    relation_str: &'static str,
    key_str: Option<String>,
    max_results: usize,
    offset: usize,
}

fn parse_batch_queries(args: &Value) -> Result<(Vec<ParsedBatchQuery>, Option<String>), String> {
    let arr = args
        .get("queries")
        .and_then(Value::as_array)
        .ok_or_else(|| "`queries` must be a non-empty array".to_string())?;
    if arr.is_empty() || arr.len() > MAX_IDEA_SEARCH_QUERIES {
        return Err(format!(
            "`queries` must contain 1..={MAX_IDEA_SEARCH_QUERIES} items (got {})",
            arr.len()
        ));
    }
    let run_id = match args.get("run_id").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::String(s)) if !s.trim().is_empty() && s.chars().count() <= 64 => {
            Some(s.trim().to_string())
        }
        _ => return Err("`run_id` must be a non-empty string (<= 64 chars)".to_string()),
    };

    const ALLOWED_QUERY_KEYS: &[&str] = &[
        "qid",
        "provider",
        "q",
        "relation",
        "key",
        "max_results",
        "offset",
        "perspective_id",
    ];
    let mut seen_qids = BTreeSet::new();
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("queries[{idx}] must be an object"))?;
        for k in obj.keys() {
            if !ALLOWED_QUERY_KEYS.contains(&k.as_str()) {
                return Err(format!("queries[{idx}] has unknown field `{k}`"));
            }
        }
        let qid = obj
            .get("qid")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= 64)
            .ok_or_else(|| format!("queries[{idx}].qid must be a non-empty string (<= 64 chars)"))?
            .to_string();
        if !seen_qids.insert(qid.clone()) {
            return Err(format!("duplicate `qid` `{qid}` in `idea_search` batch"));
        }
        let prov_raw = obj
            .get("provider")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("queries[{idx}].provider must be a string"))?;
        let provider = LitProvider::parse(prov_raw)
            .ok_or_else(|| format!("queries[{idx}].provider `{prov_raw}` is not recognized"))?;

        let rel_raw = match obj.get("relation").filter(|v| !v.is_null()) {
            None => "none",
            Some(Value::String(s)) => s.as_str(),
            Some(_) => return Err(format!("queries[{idx}].relation must be a string")),
        };
        let q_raw = match obj.get("q").filter(|v| !v.is_null()) {
            None => String::new(),
            Some(Value::String(s)) => s.trim().to_string(),
            Some(_) => return Err(format!("queries[{idx}].q must be a string")),
        };
        if q_raw.chars().count() > MAX_IDEA_QUERY_CHARS {
            return Err(format!(
                "queries[{idx}].q exceeds {MAX_IDEA_QUERY_CHARS} characters"
            ));
        }

        let (relation, relation_str, key_str) = match rel_raw {
            "none" => {
                if q_raw.is_empty() {
                    return Err(format!(
                        "queries[{idx}].q must be non-empty when relation is `none`"
                    ));
                }
                if obj.get("key").is_some_and(|v| !v.is_null()) {
                    return Err(format!("queries[{idx}].key is only valid for `cites`/`cited_by`"));
                }
                (QueryRelation::None, "none", None)
            }
            "cites" | "cited_by" => {
                let raw_key = obj
                    .get("key")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        format!("queries[{idx}].key is required when relation is `{rel_raw}`")
                    })?;
                let parsed_key = PaperKey::parse(raw_key)
                    .map_err(|e| format!("queries[{idx}].key `{raw_key}` invalid: {e}"))?;
                let key_s = parsed_key.as_str().to_string();
                let rel = if rel_raw == "cites" {
                    QueryRelation::Cites { key: parsed_key }
                } else {
                    QueryRelation::CitedBy { key: parsed_key }
                };
                (
                    rel,
                    if rel_raw == "cites" { "cites" } else { "cited_by" },
                    Some(key_s),
                )
            }
            other => {
                return Err(format!(
                    "queries[{idx}].relation `{other}` must be `none`, `cites`, or `cited_by`"
                ));
            }
        };

        let max_results = match obj.get("max_results").filter(|v| !v.is_null()) {
            None => DEFAULT_MAX_RESULTS,
            Some(v) => {
                let n = v
                    .as_u64()
                    .ok_or_else(|| format!("queries[{idx}].max_results must be an integer"))?
                    as usize;
                if !(1..=MAX_RESULTS_CAP).contains(&n) {
                    return Err(format!(
                        "queries[{idx}].max_results must be in 1..={MAX_RESULTS_CAP}"
                    ));
                }
                n
            }
        };
        let offset = match obj.get("offset").filter(|v| !v.is_null()) {
            None => 0,
            Some(v) => {
                let n = v
                    .as_u64()
                    .ok_or_else(|| format!("queries[{idx}].offset must be a non-negative integer"))?
                    as usize;
                if n > 500 {
                    return Err(format!("queries[{idx}].offset must be <= 500"));
                }
                n
            }
        };
        let perspective_id = match obj.get("perspective_id").filter(|v| !v.is_null()) {
            None => None,
            Some(Value::String(s)) if !s.trim().is_empty() && s.trim().chars().count() <= 64 => {
                Some(s.trim().to_string())
            }
            Some(_) => {
                return Err(format!(
                    "queries[{idx}].perspective_id must be a non-empty string (<= 64 chars)"
                ));
            }
        };

        out.push(ParsedBatchQuery {
            qid,
            perspective_id,
            provider,
            q: q_raw,
            relation,
            relation_str,
            key_str,
            max_results,
            offset,
        });
    }
    Ok((out, run_id))
}

fn default_provider_config() -> ProviderConfig {
    let s2_key = std::env::var("SEMANTIC_SCHOLAR_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let oa_key = std::env::var("OPENALEX_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    ProviderConfig {
        semantic_scholar_api_key: s2_key,
        openalex_api_key: oa_key,
        request_timeout: None,
    }
}

/// Build the `idea_search` [`ToolHandler`] using F3's process-wide [`global_gate`].
pub fn idea_search_handler(env: Arc<ResearchEnv>) -> Arc<dyn ToolHandler> {
    idea_search_handler_with_gate_and_config(env, None, None)
}

/// Build an `idea_search` [`ToolHandler`] with optional explicit [`NetRateGate`] and
/// [`ProviderConfig`] (for deterministic tests verifying Rule P6 rate gating & Rule P8 headers).
pub fn idea_search_handler_with_gate_and_config(
    env: Arc<ResearchEnv>,
    gate: Option<Arc<NetRateGate>>,
    config_override: Option<ProviderConfig>,
) -> Arc<dyn ToolHandler> {
    Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let (queries, run_id) = parse_batch_queries(args)?;
        let cfg = config_override
            .clone()
            .unwrap_or_else(default_provider_config);
        let active_gate: &NetRateGate = match gate.as_deref() {
            Some(g) => g,
            None => global_gate(),
        };

        let mut seen_keys: BTreeSet<PaperKey> = BTreeSet::new();
        let mut results = Vec::with_capacity(queries.len());
        let mut total_returned = 0usize;
        let mut total_new_papers = 0usize;

        for q in queries {
            if !q.provider.is_allowlisted() {
                results.push(json!({
                    "qid": q.qid,
                    "perspective_id": q.perspective_id,
                    "provider": q.provider.as_str(),
                    "q": q.q,
                    "relation": q.relation_str,
                    "key": q.key_str,
                    "status": QueryStatus::Disabled,
                    "http_status": Value::Null,
                    "returned": 0,
                    "new_papers": 0,
                    "total_results": Value::Null,
                    "has_more": false,
                    "raw_blob": Value::Null,
                    "retry_after_secs": Value::Null,
                    "error": format!("provider `{}` is disabled in this build", q.provider),
                    "papers": [],
                    "edges": [],
                }));
                continue;
            }

            let lq = match &q.relation {
                QueryRelation::None => LitQuery::search(q.provider, q.q.clone()),
                rel => LitQuery::citation_relation(q.provider, rel.clone()),
            }
            .with_paging(q.max_results, q.offset)
            .with_tracking(Some(q.qid.clone()), run_id.clone());

            let mut res = run_query_with_gate(&env.network_endpoints, &lq, &cfg, active_gate);
            if res.status == QueryStatus::Ok {
                let ledger_guard = ctx
                    .ledger
                    .lock()
                    .map_err(|_| "ledger lock poisoned".to_string())?;
                record_literature_fetched(&ledger_guard, ctx.session, &lq, &mut res, None)?;
            }

            let returned = res.papers.len();
            let mut new_papers = 0usize;
            for p in &res.papers {
                if seen_keys.insert(p.paper_ref.key.clone()) {
                    new_papers += 1;
                }
            }
            total_returned += returned;
            total_new_papers += new_papers;

            results.push(json!({
                "qid": q.qid,
                "perspective_id": q.perspective_id,
                "provider": q.provider.as_str(),
                "q": q.q,
                "relation": q.relation_str,
                "key": q.key_str,
                "status": res.status,
                "http_status": res.http_status,
                "returned": returned,
                "new_papers": new_papers,
                "total_results": res.total_results,
                "has_more": res.has_more,
                "raw_blob": res.raw_blob,
                "retry_after_secs": res.retry_after_secs,
                "error": res.error,
                "papers": res.papers,
                "edges": res.edges,
            }));
        }

        Ok(json!({
            "results": results,
            "total_returned": total_returned,
            "total_new_papers": total_new_papers,
        }))
    })
}

#[cfg(test)]
#[path = "idea_tools_tests.rs"]
mod tests;
