//! arXiv Atom 1.0 search and `.tar.gz` / `.gz` e-print LaTeX source ingestion.

use crate::http_client::{fetch_bytes_with_gate, MAX_EPRINT_BYTES, MAX_METADATA_BYTES};
use crate::ingest::{ingest_bytes, IngestRequest};
use crate::net_rate::{global_gate, NetRateGate, Service};
use crate::tex_scan::base_arxiv_id;
use crate::ResearchEnv;
use ledger::Ledger;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::{ApprovalScope, CallCtx, Registry, ToolDef, EVENT_SCHEMA_VERSION};

pub const ARXIV_SEARCH: &str = "arxiv_search";
const MAX_EPRINT_PAPERS: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArxivSearchRequest {
    pub query: String,
    pub field: Option<String>,
    pub max_results: Option<usize>,
    pub include_source: Option<bool>,
    pub section_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArxivPaper {
    pub id: String,
    pub base_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: String,
    pub published: String,
    pub updated: String,
    pub doi: Option<String>,
    pub primary_category: Option<String>,
    pub comment: Option<String>,
    pub journal_ref: Option<String>,
    pub withdrawn: bool,
    pub url: String,
    pub bibtex: String,
    pub source: Option<Value>,
}

pub fn normalize_arxiv_id(raw: &str) -> Option<String> {
    crate::tex_scan::normalize_arxiv_id(raw)
}

pub fn arxiv_search_def() -> ToolDef {
    use policy::EffectClass;
    ToolDef {
        id: ARXIV_SEARCH.into(),
        description: "Search arXiv metadata/BibTeX (supports inline prefixes like `au:Matsui AND ti:split` or `cat:math-ph`, or bare arXiv IDs) and optionally unpack `.tar.gz`/`.gz` LaTeX e-print sources.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "maxLength": 500 },
                "field": { "type": "string", "enum": ["all", "title", "author", "abstract", "category", "id"] },
                "max_results": { "type": "integer", "minimum": 1, "maximum": 20 },
                "include_source": { "type": "boolean" },
                "section_filter": { "type": "string", "maxLength": 200 }
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
        arxiv_search_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let req: ArxivSearchRequest =
                serde_json::from_value(args.clone()).map_err(|e| format!("invalid arxiv_search args: {e}"))?;
            let papers = run_arxiv_search_locked(&env, ctx.ledger, ctx.session, &req, global_gate())?;
            Ok(json!({ "count": papers.len(), "papers": papers }))
        }),
    );
}

pub fn run_arxiv_search(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    req: &ArxivSearchRequest,
) -> Result<Vec<ArxivPaper>, String> {
    run_arxiv_search_with_gate(env, ledger, session, req, global_gate())
}

/// Same as [`run_arxiv_search`] using an explicit [`NetRateGate`] (for fake-clock tests).
pub fn run_arxiv_search_with_gate(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    req: &ArxivSearchRequest,
    gate: &NetRateGate,
) -> Result<Vec<ArxivPaper>, String> {
    let raw_bytes = fetch_atom_feed(env, req, gate)?;
    let mut papers = parse_atom_feed(&raw_bytes)?;

    let blob_hash = ledger
        .put_blob(&raw_bytes)
        .map_err(|e| format!("arxiv_search put_blob failed: {e}"))?;
    ledger
        .append(
            session,
            "literature.fetched",
            EVENT_SCHEMA_VERSION,
            &json!({
                "provider": "arxiv",
                "query": req.query,
                "field": req.field,
                "count": papers.len(),
                "raw_blob": blob_hash,
                "papers": papers.iter().map(|p| json!({
                    "id": p.id,
                    "base_id": p.base_id,
                    "title": p.title,
                    "withdrawn": p.withdrawn,
                    "doi": p.doi,
                })).collect::<Vec<_>>(),
            }),
        )
        .map_err(|e| format!("literature.fetched append failed: {e}"))?;

    if req.include_source == Some(true) {
        let m = Mutex::new(LedgerDummy);
        let _ = m;
        // Reuse shared ingestion path via a temporary handleless flow? We already own `&Ledger`,
        // so ingest using a helper that writes directly under `ledger`.
        ingest_eprints_direct(env, ledger, session, &mut papers, req.section_filter.as_deref(), gate)?;
    }
    Ok(papers)
}

fn run_arxiv_search_locked(
    env: &ResearchEnv,
    ledger: &Mutex<Ledger>,
    session: &str,
    req: &ArxivSearchRequest,
    gate: &NetRateGate,
) -> Result<Vec<ArxivPaper>, String> {
    let raw_bytes = fetch_atom_feed(env, req, gate)?;
    let mut papers = parse_atom_feed(&raw_bytes)?;

    {
        let guard = ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let blob_hash = guard
            .put_blob(&raw_bytes)
            .map_err(|e| format!("arxiv_search put_blob failed: {e}"))?;
        guard
            .append(
                session,
                "literature.fetched",
                EVENT_SCHEMA_VERSION,
                &json!({
                    "provider": "arxiv",
                    "query": req.query,
                    "field": req.field,
                    "count": papers.len(),
                    "raw_blob": blob_hash,
                    "papers": papers.iter().map(|p| json!({
                        "id": p.id,
                        "base_id": p.base_id,
                        "title": p.title,
                        "withdrawn": p.withdrawn,
                        "doi": p.doi,
                    })).collect::<Vec<_>>(),
                }),
            )
            .map_err(|e| format!("literature.fetched append failed: {e}"))?;
    }

    if req.include_source == Some(true) {
        ingest_eprints_locked(env, ledger, session, &mut papers, req.section_filter.as_deref(), gate)?;
    }
    Ok(papers)
}

fn fetch_atom_feed(
    env: &ResearchEnv,
    req: &ArxivSearchRequest,
    gate: &NetRateGate,
) -> Result<Vec<u8>, String> {
    let q = req.query.trim();
    if q.is_empty() {
        return Err("`query` must not be empty".into());
    }
    let max_results = req.max_results.unwrap_or(5).clamp(1, 20);
    let base = env.network_endpoints.arxiv_api_base.trim_end_matches('/');

    let url = if req.field.as_deref() == Some("id") || normalize_arxiv_id(q).is_some() {
        let id = normalize_arxiv_id(q).unwrap_or_else(|| q.to_string());
        let enc = utf8_percent_encode(&id, NON_ALPHANUMERIC).to_string();
        format!("{base}/api/query?id_list={enc}&max_results={max_results}")
    } else {
        let search_expr = build_arxiv_search_expr(req.field.as_deref(), q)?;
        let enc = utf8_percent_encode(&search_expr, NON_ALPHANUMERIC).to_string();
        format!("{base}/api/query?search_query={enc}&max_results={max_results}")
    };

    fetch_bytes_with_gate(&url, &env.network_endpoints, MAX_METADATA_BYTES, Some(Service::Arxiv), gate)
}

const ARXIV_FIELD_PREFIXES: &[&str] = &[
    "all:", "ti:", "au:", "abs:", "co:", "jr:", "cat:", "rn:", "id:",
];

fn build_arxiv_search_expr(field: Option<&str>, raw: &str) -> Result<String, String> {
    let explicit_prefix = match field.unwrap_or("all") {
        "all" => None,
        "title" | "ti" => Some("ti"),
        "author" | "au" => Some("au"),
        "abstract" | "abs" => Some("abs"),
        "category" | "cat" => Some("cat"),
        other => return Err(format!("unsupported arXiv field `{other}`")),
    };
    let sanitized = sanitize_arxiv_query(raw);
    if sanitized.is_empty() {
        return Err("`query` must not be empty after sanitization".into());
    }
    let has_inline_prefix = query_has_field_prefix(&sanitized);
    Ok(match explicit_prefix {
        None if has_inline_prefix => sanitized,
        None => format!("all:{sanitized}"),
        Some(p) if has_inline_prefix && sanitized.to_ascii_lowercase().starts_with(&format!("{p}:")) => {
            sanitized
        }
        Some(p) if has_inline_prefix => format!("{p}:({sanitized})"),
        Some(p) => format!("{p}:{sanitized}"),
    })
}

fn query_has_field_prefix(q: &str) -> bool {
    q.split_whitespace().any(|tok| {
        let trimmed = tok.trim_start_matches(['(', '"']);
        let low = trimmed.to_ascii_lowercase();
        ARXIV_FIELD_PREFIXES
            .iter()
            .any(|p| low.starts_with(p) && low.len() > p.len())
    })
}

fn normalize_inline_prefix(tok: &str) -> String {
    let leading_parens: String = tok.chars().take_while(|&c| c == '(').collect();
    let rest = &tok[leading_parens.len()..];
    let Some((head, tail)) = rest.split_once(':') else {
        return tok.to_string();
    };
    if tail.is_empty() {
        return tok.to_string();
    }
    let mapped = match head.to_ascii_lowercase().as_str() {
        "author" => Some("au"),
        "title" => Some("ti"),
        "abstract" => Some("abs"),
        "category" => Some("cat"),
        "all" => Some("all"),
        "ti" => Some("ti"),
        "au" => Some("au"),
        "abs" => Some("abs"),
        "co" => Some("co"),
        "jr" => Some("jr"),
        "cat" => Some("cat"),
        "rn" => Some("rn"),
        "id" => Some("id"),
        _ => None,
    };
    match mapped {
        Some(p) => format!("{leading_parens}{p}:{tail}"),
        None => tok.to_string(),
    }
}

fn sanitize_arxiv_query(raw: &str) -> String {
    let quotes_even = raw.bytes().filter(|&b| b == b'"').count().is_multiple_of(2);
    let mut depth = 0i32;
    let mut parens_balanced = true;
    for ch in raw.chars() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth < 0 {
                parens_balanced = false;
                break;
            }
        }
    }
    if depth != 0 {
        parens_balanced = false;
    }
    let cleaned: String = raw
        .chars()
        .map(|ch| match ch {
            '"' if !quotes_even => ' ',
            '(' | ')' if !parens_balanced => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    cleaned
        .split_whitespace()
        .map(|tok| match tok.to_ascii_uppercase().as_str() {
            "AND" | "OR" | "ANDNOT" => tok.to_ascii_uppercase(),
            _ => normalize_inline_prefix(tok),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_atom_feed(bytes: &[u8]) -> Result<Vec<ArxivPaper>, String> {
    let xml = std::str::from_utf8(bytes).map_err(|_| "arXiv Atom feed is not valid UTF-8".to_string())?;
    let head_upper = xml.chars().take(4096).collect::<String>().to_ascii_uppercase();
    if head_upper.contains("<!DOCTYPE") || head_upper.contains("<!ENTITY") {
        return Err("rejected arXiv XML feed containing DOCTYPE or ENTITY declaration (XXE guard)".into());
    }
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("invalid arXiv Atom XML: {e}"))?;
    let mut papers = Vec::new();
    for entry in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "entry") {
        let raw_id = child_text(&entry, "id").unwrap_or_default();
        if raw_id.contains("arxiv.org/api/errors") {
            let msg = child_text(&entry, "summary").unwrap_or_else(|| "arXiv API error".into());
            return Err(format!("arXiv API error: {msg}"));
        }
        let id = normalize_arxiv_id(&raw_id).unwrap_or_else(|| {
            raw_id.rsplit("/abs/").next().unwrap_or(&raw_id).trim().to_string()
        });
        let base_id = base_arxiv_id(&id).unwrap_or_else(|| id.to_ascii_lowercase());
        let title = collapse_ws(&child_text(&entry, "title").unwrap_or_default());
        let abstract_text = collapse_ws(&child_text(&entry, "summary").unwrap_or_default());
        let published = child_text(&entry, "published").unwrap_or_default();
        let updated = child_text(&entry, "updated").unwrap_or_default();
        let authors: Vec<String> = entry
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "author")
            .filter_map(|a| child_text(&a, "name"))
            .map(|s| collapse_ws(&s))
            .filter(|s| !s.is_empty())
            .collect();
        let doi = child_text(&entry, "doi").map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
        let comment = child_text(&entry, "comment").map(|c| collapse_ws(&c)).filter(|c| !c.is_empty());
        let journal_ref = child_text(&entry, "journal_ref").map(|j| collapse_ws(&j)).filter(|j| !j.is_empty());
        let primary_category = entry
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "primary_category")
            .and_then(|c| c.attribute("term"))
            .map(str::to_string);
        let withdrawn = is_withdrawn(comment.as_deref(), &abstract_text);
        let url = format!("https://arxiv.org/abs/{id}");
        let bibtex = build_arxiv_bibtex(&id, &title, &authors, &published, doi.as_deref(), primary_category.as_deref());
        papers.push(ArxivPaper {
            id,
            base_id,
            title,
            authors,
            abstract_text,
            published,
            updated,
            doi,
            primary_category,
            comment,
            journal_ref,
            withdrawn,
            url,
            bibtex,
            source: None,
        });
    }
    Ok(papers)
}

fn child_text(node: &roxmltree::Node<'_, '_>, local_name: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == local_name)
        .and_then(|c| c.text())
        .map(str::to_string)
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_withdrawn(comment: Option<&str>, summary: &str) -> bool {
    let check = |t: &str| {
        let low = t.to_ascii_lowercase();
        low.contains("withdrawn") || low.contains("retracted")
    };
    comment.is_some_and(check) || check(summary)
}

fn build_arxiv_bibtex(
    id: &str,
    title: &str,
    authors: &[String],
    published: &str,
    doi: Option<&str>,
    primary_class: Option<&str>,
) -> String {
    let year = published.get(..4).filter(|y| y.bytes().all(|b| b.is_ascii_digit())).unwrap_or("2025");
    let first_last = authors
        .first()
        .and_then(|a| a.split_whitespace().last())
        .unwrap_or("arxiv")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    let id_slug: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let key = format!("{first_last}:{year}:{id_slug}");
    let author_str = authors.join(" and ");
    let mut lines = vec![
        format!("@article{{{key},"),
        format!("  author = {{{author_str}}},"),
        format!("  title = {{{{{title}}}}},"),
        format!("  year = {{{year}}},"),
        format!("  eprint = {{{id}}},"),
        "  archivePrefix = {arXiv},".to_string(),
    ];
    if let Some(pc) = primary_class {
        lines.push(format!("  primaryClass = {{{pc}}},"));
    }
    if let Some(d) = doi {
        lines.push(format!("  doi = {{{d}}},"));
    }
    lines.push("}".into());
    lines.join("\n")
}

struct LedgerDummy;

fn ingest_eprints_direct(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    papers: &mut [ArxivPaper],
    section_filter: Option<&str>,
    gate: &NetRateGate,
) -> Result<(), String> {
    let sandbox = env.sandbox();
    let eprint_base = env.network_endpoints.arxiv_eprint_base.trim_end_matches('/');
    let mut fetched = 0usize;
    for paper in papers.iter_mut() {
        if paper.withdrawn || fetched >= MAX_EPRINT_PAPERS {
            continue;
        }
        fetched += 1;
        let url = format!("{eprint_base}/e-print/{}", paper.id);
        let bytes = match fetch_bytes_with_gate(
            &url,
            &env.network_endpoints,
            MAX_EPRINT_BYTES,
            Some(Service::Arxiv),
            gate,
        ) {
            Ok(b) => b,
            Err(e) => {
                paper.source = Some(json!({ "source_available": false, "reason": e }));
                continue;
            }
        };
        if bytes.starts_with(b"%PDF-") {
            paper.source = Some(json!({ "source_available": false, "reason": "pdf_only" }));
            continue;
        }
        // Temporarily open a separate connection? No: reuse `ingest::ingest_bytes` via `Ledger::open`.
        // Note: `Ledger` allows concurrent connections on SQLite WAL, and here we hold `&Ledger`.
        let temp_ledger = Mutex::new(Ledger::open(&env.data_dir).map_err(|e| e.to_string())?);
        let _ = ledger;
        paper.source = Some(materialize_eprint(&sandbox, &temp_ledger, session, &paper.id, &paper.title, &url, &bytes, section_filter)?);
    }
    Ok(())
}

fn ingest_eprints_locked(
    env: &ResearchEnv,
    ledger: &Mutex<Ledger>,
    session: &str,
    papers: &mut [ArxivPaper],
    section_filter: Option<&str>,
    gate: &NetRateGate,
) -> Result<(), String> {
    let sandbox = env.sandbox();
    let eprint_base = env.network_endpoints.arxiv_eprint_base.trim_end_matches('/');
    let mut fetched = 0usize;
    for paper in papers.iter_mut() {
        if paper.withdrawn || fetched >= MAX_EPRINT_PAPERS {
            continue;
        }
        fetched += 1;
        let url = format!("{eprint_base}/e-print/{}", paper.id);
        let bytes = match fetch_bytes_with_gate(
            &url,
            &env.network_endpoints,
            MAX_EPRINT_BYTES,
            Some(Service::Arxiv),
            gate,
        ) {
            Ok(b) => b,
            Err(e) => {
                paper.source = Some(json!({ "source_available": false, "reason": e }));
                continue;
            }
        };
        if bytes.starts_with(b"%PDF-") {
            paper.source = Some(json!({ "source_available": false, "reason": "pdf_only" }));
            continue;
        }
        paper.source = Some(materialize_eprint(&sandbox, ledger, session, &paper.id, &paper.title, &url, &bytes, section_filter)?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize_eprint(
    sandbox: &crate::sandbox::WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    session: &str,
    paper_id: &str,
    paper_title: &str,
    origin_url: &str,
    bytes: &[u8],
    section_filter: Option<&str>,
) -> Result<Value, String> {
    let hint = format!("arxiv_{}.gz", paper_id.replace('/', "_"));
    let clean_title = paper_title.trim();
    let ingested = ingest_bytes(
        sandbox,
        ledger,
        &IngestRequest {
            session_id: session,
            file_name: &hint,
            origin: origin_url,
            raw_bytes: bytes,
            source_id_hint: Some(&hint),
            section_filter,
            title: (!clean_title.is_empty()).then_some(clean_title),
        },
    )?;
    if ingested.status == "pdf_only" {
        return Ok(json!({
            "source_available": false,
            "reason": "pdf_only",
            "source_id": ingested.source_id,
            "raw_blob": ingested.raw_blob,
        }));
    }
    Ok(json!({
        "source_available": true,
        "source_id": ingested.source_id,
        "primary_path": ingested.primary_path,
        "assembled_path": ingested.assembled_path,
        "files": ingested.files,
        "raw_blob": ingested.raw_blob,
        "excerpt": ingested.excerpt,
    }))
}
