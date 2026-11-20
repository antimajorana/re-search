//! Literature search across INSPIRE-HEP, Crossref, and Semantic Scholar.

use crate::http_client::{fetch_bytes_allowlisted, MAX_METADATA_BYTES};
use crate::tex_scan::normalize_doi;
use crate::ResearchEnv;
use ledger::Ledger;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::{ApprovalScope, CallCtx, Registry, ToolDef, EVENT_SCHEMA_VERSION};

pub const LITERATURE_SEARCH: &str = "literature_search";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteratureRequest {
    pub source: Option<String>,
    pub query: String,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteratureHit {
    pub source: String,
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub citation_count: Option<u64>,
    pub retracted: bool,
    pub url: Option<String>,
    pub bibtex: String,
}

pub fn literature_search_def() -> ToolDef {
    use policy::EffectClass;
    ToolDef {
        id: LITERATURE_SEARCH.into(),
        description: "Search INSPIRE-HEP, Crossref, or Semantic Scholar for papers, DOIs, and BibTeX.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "enum": ["inspire", "crossref", "semantic_scholar"] },
                "query": { "type": "string", "maxLength": 500 },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 20 }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        effect: EffectClass::NetworkRead,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        literature_search_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let req: LiteratureRequest = serde_json::from_value(args.clone())
                .map_err(|e| format!("invalid literature_search args: {e}"))?;
            let hits = run_literature_search_locked(&env, ctx.ledger, ctx.session, &req)?;
            Ok(json!({ "count": hits.len(), "hits": hits }))
        }),
    );
}

pub fn run_literature_search(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    req: &LiteratureRequest,
) -> Result<Vec<LiteratureHit>, String> {
    let (source, url, raw_bytes) = fetch_source(env, req)?;
    let hits = parse_hits(&source, &raw_bytes)?;
    record_fetch(ledger, session, &source, &url, &req.query, &raw_bytes, &hits)?;
    Ok(hits)
}

fn run_literature_search_locked(
    env: &ResearchEnv,
    ledger: &Mutex<Ledger>,
    session: &str,
    req: &LiteratureRequest,
) -> Result<Vec<LiteratureHit>, String> {
    let (source, url, raw_bytes) = fetch_source(env, req)?;
    let hits = parse_hits(&source, &raw_bytes)?;
    let guard = ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
    record_fetch(&guard, session, &source, &url, &req.query, &raw_bytes, &hits)?;
    Ok(hits)
}

fn fetch_source(env: &ResearchEnv, req: &LiteratureRequest) -> Result<(String, String, Vec<u8>), String> {
    let q = req.query.trim();
    if q.is_empty() {
        return Err("`query` must not be empty".into());
    }
    let source = req.source.as_deref().unwrap_or("inspire").trim().to_ascii_lowercase();
    let max_results = req.max_results.unwrap_or(5).clamp(1, 20);
    let enc_q = utf8_percent_encode(q, NON_ALPHANUMERIC).to_string();

    let url = match source.as_str() {
        "inspire" => {
            let base = env.network_endpoints.inspire_api_base.trim_end_matches('/');
            format!("{base}/api/literature?q={enc_q}&size={max_results}")
        }
        "crossref" => {
            let base = env.network_endpoints.crossref_api_base.trim_end_matches('/');
            format!("{base}/works?query={enc_q}&rows={max_results}")
        }
        "semantic_scholar" => {
            let base = env.network_endpoints.semantic_scholar_api_base.trim_end_matches('/');
            format!(
                "{base}/graph/v1/paper/search?query={enc_q}&limit={max_results}&fields=title,authors,year,citationCount,externalIds,url"
            )
        }
        other => return Err(format!("unsupported literature source `{other}`")),
    };
    let raw_bytes = fetch_bytes_allowlisted(&url, &env.network_endpoints, MAX_METADATA_BYTES)?;
    Ok((source, url, raw_bytes))
}

fn record_fetch(
    ledger: &Ledger,
    session: &str,
    source: &str,
    url: &str,
    query: &str,
    raw_bytes: &[u8],
    hits: &[LiteratureHit],
) -> Result<(), String> {
    let blob_hash = ledger
        .put_blob(raw_bytes)
        .map_err(|e| format!("literature_search put_blob failed: {e}"))?;
    ledger
        .append(
            session,
            "literature.fetched",
            EVENT_SCHEMA_VERSION,
            &json!({
                "provider": source,
                "url": url,
                "query": query,
                "count": hits.len(),
                "raw_blob": blob_hash,
                "hits": hits.iter().map(|h| json!({
                    "id": h.id,
                    "doi": h.doi,
                    "arxiv_id": h.arxiv_id,
                    "retracted": h.retracted,
                })).collect::<Vec<_>>(),
            }),
        )
        .map_err(|e| format!("literature.fetched append failed: {e}"))?;
    Ok(())
}

fn parse_hits(source: &str, raw_bytes: &[u8]) -> Result<Vec<LiteratureHit>, String> {
    let root: Value = serde_json::from_slice(raw_bytes)
        .map_err(|e| format!("invalid JSON from `{source}`: {e}"))?;
    match source {
        "inspire" => Ok(parse_inspire(&root)),
        "crossref" => Ok(parse_crossref(&root)),
        "semantic_scholar" => Ok(parse_s2(&root)),
        _ => Err(format!("unsupported source `{source}`")),
    }
}

fn parse_inspire(root: &Value) -> Vec<LiteratureHit> {
    let Some(items) = root["hits"]["hits"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            let meta = &item["metadata"];
            let id = item["id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| meta["control_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let texkey = meta["texkeys"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .map(str::to_string);
            let title = meta["titles"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t["title"].as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            let authors: Vec<String> = meta["authors"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a["full_name"].as_str().map(|s| s.trim().to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let citation_count = meta["citation_count"].as_u64();
            let doi = meta["dois"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|d| d["value"].as_str())
                .and_then(normalize_doi);
            let arxiv_id = meta["arxiv_eprints"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|e| e["value"].as_str())
                .map(str::to_string);
            let year = meta["preprint_date"]
                .as_str()
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse::<i32>().ok())
                .or_else(|| {
                    meta["publication_info"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|p| p["year"].as_i64())
                        .map(|y| y as i32)
                });
            let key = texkey.unwrap_or_else(|| default_bib_key(&authors, year, &id));
            let url = (!id.is_empty()).then(|| format!("https://inspirehep.net/literature/{id}"));
            let bibtex = build_bibtex(&key, &title, &authors, year, doi.as_deref(), arxiv_id.as_deref());
            LiteratureHit {
                source: "inspire".into(),
                id,
                title,
                authors,
                year,
                doi,
                arxiv_id,
                citation_count,
                retracted: false,
                url,
                bibtex,
            }
        })
        .collect()
}

fn parse_crossref(root: &Value) -> Vec<LiteratureHit> {
    let items = if let Some(arr) = root["message"]["items"].as_array() {
        arr.clone()
    } else if root["message"].is_object() {
        vec![root["message"].clone()]
    } else {
        return Vec::new();
    };
    items
        .iter()
        .map(|w| {
            let doi = w["DOI"].as_str().and_then(normalize_doi);
            let id = doi.clone().unwrap_or_default();
            let title = w["title"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let authors: Vec<String> = w["author"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| {
                            if let Some(name) = a["name"].as_str() {
                                return Some(name.trim().to_string());
                            }
                            let given = a["given"].as_str().unwrap_or("").trim();
                            let family = a["family"].as_str().unwrap_or("").trim();
                            let joined = format!("{given} {family}").trim().to_string();
                            (!joined.is_empty()).then_some(joined)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let year = extract_crossref_year(w);
            let citation_count = w["is-referenced-by-count"].as_u64();
            let retracted = has_crossref_retraction(w);
            let url = w["URL"]
                .as_str()
                .map(str::to_string)
                .or_else(|| doi.as_ref().map(|d| format!("https://doi.org/{d}")));
            let key = default_bib_key(&authors, year, &id);
            let bibtex = build_bibtex(&key, &title, &authors, year, doi.as_deref(), None);
            LiteratureHit {
                source: "crossref".into(),
                id,
                title,
                authors,
                year,
                doi,
                arxiv_id: None,
                citation_count,
                retracted,
                url,
                bibtex,
            }
        })
        .collect()
}

fn extract_crossref_year(w: &Value) -> Option<i32> {
    for k in ["published-print", "published-online", "issued", "created"] {
        if let Some(y) = w[k]["date-parts"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|r| r.as_array())
            .and_then(|p| p.first())
            .and_then(Value::as_i64)
        {
            return Some(y as i32);
        }
    }
    None
}

fn has_crossref_retraction(w: &Value) -> bool {
    if w["relation"]["is-retracted-by"].as_array().is_some_and(|a| !a.is_empty()) {
        return true;
    }
    if let Some(updates) = w["update-to"].as_array() {
        for u in updates {
            let kind = u["type"].as_str().unwrap_or("").to_ascii_lowercase();
            let label = u["label"].as_str().unwrap_or("").to_ascii_lowercase();
            if kind.contains("retract") || label.contains("retract") {
                return true;
            }
        }
    }
    false
}

fn parse_s2(root: &Value) -> Vec<LiteratureHit> {
    let items = if let Some(arr) = root["data"].as_array() {
        arr.clone()
    } else if root["paperId"].is_string() {
        vec![root.clone()]
    } else {
        return Vec::new();
    };
    items
        .iter()
        .map(|p| {
            let id = p["paperId"].as_str().unwrap_or_default().to_string();
            let title = p["title"].as_str().unwrap_or_default().trim().to_string();
            let authors: Vec<String> = p["authors"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a["name"].as_str().map(|s| s.trim().to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let year = p["year"].as_i64().map(|y| y as i32);
            let citation_count = p["citationCount"].as_u64();
            let doi = p["externalIds"]["DOI"].as_str().and_then(normalize_doi);
            let arxiv_id = p["externalIds"]["ArXiv"]
                .as_str()
                .map(str::to_string)
                .filter(|s| !s.is_empty());
            let url = p["url"].as_str().map(str::to_string);
            let key = default_bib_key(&authors, year, &id);
            let bibtex = build_bibtex(&key, &title, &authors, year, doi.as_deref(), arxiv_id.as_deref());
            LiteratureHit {
                source: "semantic_scholar".into(),
                id,
                title,
                authors,
                year,
                doi,
                arxiv_id,
                citation_count,
                retracted: false,
                url,
                bibtex,
            }
        })
        .collect()
}

fn default_bib_key(authors: &[String], year: Option<i32>, id: &str) -> String {
    let surname = authors
        .first()
        .map(|a| {
            if let Some((last, _)) = a.split_once(',') {
                last.trim()
            } else {
                a.split_whitespace().last().unwrap_or("ref")
            }
        })
        .unwrap_or("ref")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    let y = year.map(|n| n.to_string()).unwrap_or_else(|| "nd".into());
    let slug: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
    let s = if surname.is_empty() { "ref".into() } else { surname };
    format!("{s}:{y}:{slug}")
}

fn build_bibtex(
    key: &str,
    title: &str,
    authors: &[String],
    year: Option<i32>,
    doi: Option<&str>,
    arxiv_id: Option<&str>,
) -> String {
    let mut lines = vec![
        format!("@article{{{key},"),
        format!("  author = {{{}}},", authors.join(" and ")),
        format!("  title = {{{{{title}}}}},"),
    ];
    if let Some(y) = year {
        lines.push(format!("  year = {{{y}}},"));
    }
    if let Some(d) = doi {
        lines.push(format!("  doi = {{{d}}},"));
    }
    if let Some(ax) = arxiv_id {
        lines.push(format!("  eprint = {{{ax}}},"));
        lines.push("  archivePrefix = {arXiv},".to_string());
    }
    lines.push("}".into());
    lines.join("\n")
}
