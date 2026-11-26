//! OpenAlex literature provider runner & response parser (X19 Part 1).
//!
//! Splits OpenAlex URL construction, Rule-P8 header auth, deterministic
//! `abstract_inverted_index` reconstruction (`verbatim = false`, Rule C3), and JSON
//! work/citation parsing out of `lit_providers.rs` so every module stays `<= 600` lines.
//!
//! External-service fields not already used in `literature.rs` are explicitly marked
//! `ASSUMPTION` per `docs/design/modes-2026-11/agents/RULES.md`.

use crate::lit_abstracts::{normalize_abstract, normalize_whitespace};
use crate::lit_providers::parse::{
    build_bibtex, default_bib_key, has_more_from_total, make_abstract_ref, make_edge,
    ParsedProviderPage, MAX_AUTHORS_PER_PAPER,
};
use crate::lit_providers::{
    CitationEdge, LitPaper, LitProvider, LitQuery, ProviderConfig, QueryRelation, QueryStatus,
};
use crate::paper_key::{canonical_doi, split_arxiv_id, PaperIds, PaperKey, PaperKeyKind, PaperRef};
use crate::LiteratureEndpoints;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::Value;

/// Safety cap on token positions extracted from a single `abstract_inverted_index`.
const MAX_INVERTED_INDEX_TOKENS: usize = 16_384;

/// Attach optional OpenAlex API key strictly as an `Authorization: Bearer <key>` header.
///
/// Rule P8 (`explore.md` section 6): API keys never appear in query strings, logs, or
/// ledger payloads, and `http_client::fetch_response_with_options_and_gate` strips all
/// `extra_headers` on any cross-origin redirect.
pub fn apply_auth_header(config: &ProviderConfig, extra_headers: &mut Vec<(String, String)>) {
    if let Some(key) = config
        .openalex_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        extra_headers.push(("Authorization".to_string(), format!("Bearer {key}")));
    }
}

/// Build an OpenAlex `/works` request URL for free-text search, explicit `filter:` queries,
/// or citation relations (`QueryRelation::Cites` / `QueryRelation::CitedBy`).
///
/// ASSUMPTION (OpenAlex REST API):
/// - `/works` accepts `search=<q>`, `filter=<expr>`, `per-page=<n>`, and 1-indexed `page=<p>`.
/// - `filter=cited_by:<work>` returns works cited BY `<work>` (outgoing references = `Cites`).
/// - `filter=cites:<work>` returns works that cite `<work>` (incoming citations = `CitedBy`).
/// - Both `W<digits>` and `https://doi.org/<doi>` are accepted as `<work>` targets.
pub fn build_request_url(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    max_results: usize,
) -> Result<String, (QueryStatus, String)> {
    let base = endpoints.openalex_api_base.trim_end_matches('/');
    let page = (query.offset / max_results.max(1)).saturating_add(1);
    match &query.relation {
        QueryRelation::None => {
            let q = query.query.trim();
            if q.is_empty() {
                return Err((QueryStatus::Failed, "`query` must not be empty".into()));
            }
            if q.get(..7)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("filter:"))
            {
                let expr = q[7..].trim();
                if expr.is_empty() {
                    return Err((
                        QueryStatus::Failed,
                        "OpenAlex `filter:` expression must not be empty".into(),
                    ));
                }
                let enc = utf8_percent_encode(expr, NON_ALPHANUMERIC).to_string();
                return Ok(format!(
                    "{base}/works?filter={enc}&per-page={max_results}&page={page}"
                ));
            }
            let enc = utf8_percent_encode(q, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/works?search={enc}&per-page={max_results}&page={page}"
            ))
        }
        QueryRelation::Cites { key } => {
            let target = openalex_filter_target(key)?;
            let filter = format!("cited_by:{target}");
            let enc = utf8_percent_encode(&filter, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/works?filter={enc}&per-page={max_results}&page={page}"
            ))
        }
        QueryRelation::CitedBy { key } => {
            let target = openalex_filter_target(key)?;
            let filter = format!("cites:{target}");
            let enc = utf8_percent_encode(&filter, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/works?filter={enc}&per-page={max_results}&page={page}"
            ))
        }
    }
}

fn openalex_filter_target(key: &PaperKey) -> Result<String, (QueryStatus, String)> {
    match key.kind() {
        PaperKeyKind::Openalex => Ok(key.value().to_string()),
        PaperKeyKind::Doi => Ok(format!("https://doi.org/{}", key.value())),
        _ => Err((
            QueryStatus::Unsupported,
            format!(
                "OpenAlex citation relations require an openalex or doi key (got `{key}`)"
            ),
        )),
    }
}

/// Deterministically reconstruct plain text from an OpenAlex `abstract_inverted_index` JSON
/// object (`{ "<token>": [pos0, pos1, ...], ... }`).
///
/// Sorts collected `(position, token)` pairs without allocating by `max_pos`, and stops
/// appending as soon as [`crate::lit_abstracts::MAX_ABSTRACT_CHARS`] folded characters are
/// reached so neither sparse positions nor repeated oversized tokens can trigger OOM.
/// Always accompanied by `AbstractRef { verbatim: false, .. }` so Rule C3 prevents rebuilt
/// abstracts from ever becoming `evidence.abstract`.
pub fn reconstruct_abstract_inverted_index(val: &Value) -> Option<String> {
    let map = val.as_object()?;
    if map.is_empty() {
        return None;
    }
    let mut pairs: Vec<(u64, &str)> = Vec::new();
    for (token, positions) in map {
        let word = token.trim();
        if word.is_empty() {
            continue;
        }
        let Some(arr) = positions.as_array() else {
            continue;
        };
        for pos_val in arr {
            let Some(pos) = pos_val.as_u64() else {
                continue;
            };
            if pairs.len() >= MAX_INVERTED_INDEX_TOKENS {
                break;
            }
            pairs.push((pos, word));
        }
        if pairs.len() >= MAX_INVERTED_INDEX_TOKENS {
            break;
        }
    }
    if pairs.is_empty() {
        return None;
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    let max_chars = crate::lit_abstracts::MAX_ABSTRACT_CHARS;
    let mut joined = String::with_capacity(max_chars.min(pairs.len().saturating_mul(6)));
    let mut chars_written = 0usize;
    let mut prev_space = true;
    for (_, word) in pairs {
        if chars_written >= max_chars {
            break;
        }
        if !prev_space {
            joined.push(' ');
            chars_written += 1;
            prev_space = true;
        }
        for ch in word.chars() {
            if chars_written >= max_chars {
                break;
            }
            if ch.is_whitespace() {
                if !prev_space {
                    joined.push(' ');
                    chars_written += 1;
                    prev_space = true;
                }
            } else {
                joined.push(ch);
                chars_written += 1;
                prev_space = false;
            }
        }
    }
    normalize_abstract(&joined, false)
}

/// Parse an OpenAlex JSON response into F2-keyed [`LitPaper`] records and optional
/// [`CitationEdge`]s.
///
/// Every reconstructed abstract receives `abstract_ref.verbatim = false` (`explore.md` 4.1
/// and Rule C3).
pub fn parse_openalex_json(
    bytes: &[u8],
    raw_blob: &str,
    relation: &QueryRelation,
    offset: usize,
    max_results: Option<usize>,
) -> Result<ParsedProviderPage, String> {
    let root: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON from `openalex`: {e}"))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "malformed OpenAlex JSON: root is not an object".to_string())?;
    // ASSUMPTION (OpenAlex REST API): paginated work queries return `meta.count` and `results[]`.
    let total_results = obj
        .get("meta")
        .and_then(|m| m.get("count"))
        .and_then(Value::as_u64);

    let items: Vec<&Value> = if let Some(arr) = obj.get("results").and_then(Value::as_array) {
        arr.iter().collect()
    } else if obj.get("id").and_then(Value::as_str).is_some() {
        if let QueryRelation::Cites { key } = relation {
            if let Some(refs) = obj.get("referenced_works").and_then(Value::as_array) {
                return parse_referenced_works(refs, key, offset, max_results);
            }
        }
        vec![&root]
    } else if total_results == Some(0) {
        Vec::new()
    } else {
        return Err("malformed OpenAlex JSON: missing `results` array".into());
    };

    let mut papers = Vec::new();
    let mut edges = Vec::new();
    for w in items {
        if !w.is_object() {
            continue;
        }
        let openalex_key = w["id"]
            .as_str()
            .and_then(|s| PaperKey::from_openalex(s).ok())
            .or_else(|| {
                w["ids"]["openalex"]
                    .as_str()
                    .and_then(|s| PaperKey::from_openalex(s).ok())
            });
        let doi = w["doi"]
            .as_str()
            .and_then(|s| canonical_doi(s).ok())
            .or_else(|| w["ids"]["doi"].as_str().and_then(|s| canonical_doi(s).ok()));
        let arxiv_split = extract_openalex_arxiv(w);
        let Some(paper_ref) = build_openalex_paper_ref(arxiv_split, doi, openalex_key.clone())
        else {
            continue;
        };
        let provider_id = openalex_key
            .as_ref()
            .map(|k| k.value().to_string())
            .unwrap_or_else(|| paper_ref.key.value().to_string());
        let title = normalize_whitespace(
            w["title"]
                .as_str()
                .or_else(|| w["display_name"].as_str())
                .unwrap_or_default(),
        );
        let authors = extract_openalex_authors(w);
        let year = w["publication_year"]
            .as_i64()
            .and_then(|y| i32::try_from(y).ok());
        // Rule C3 (`explore.md` 4.1 & 6): OpenAlex inverted-index reconstructions MUST set
        // `verbatim = false` so they can be displayed/triaged but NEVER become `evidence.abstract`.
        let abstract_text = w
            .get("abstract_inverted_index")
            .and_then(reconstruct_abstract_inverted_index);
        let abstract_ref = abstract_text
            .as_ref()
            .map(|_| make_abstract_ref(raw_blob, LitProvider::OpenAlex, false));
        let citation_count = w["cited_by_count"].as_u64();
        let retracted = w["is_retracted"].as_bool().unwrap_or(false);
        let url = w["primary_location"]["landing_page_url"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| paper_ref.ids.doi.as_ref().map(|d| format!("https://doi.org/{d}")))
            .or_else(|| {
                openalex_key
                    .as_ref()
                    .map(|k| format!("https://openalex.org/{}", k.value()))
            });
        let bib_key = default_bib_key(&authors, year, &provider_id);
        let bibtex = Some(build_bibtex(
            &bib_key,
            &title,
            &authors,
            year,
            paper_ref.ids.doi.as_deref(),
            paper_ref.ids.arxiv.as_deref(),
        ));

        if let Some(edge) = make_edge(relation, &paper_ref.key, LitProvider::OpenAlex) {
            edges.push(edge);
        }
        papers.push(LitPaper {
            paper_ref,
            title,
            authors,
            year,
            abstract_text,
            abstract_ref,
            citation_count,
            withdrawn_or_retracted: retracted,
            url,
            bibtex,
            provider: LitProvider::OpenAlex,
            provider_id,
        });
    }

    let has_more = has_more_from_total(offset, papers.len(), total_results);
    Ok(ParsedProviderPage {
        papers,
        edges,
        total_results,
        has_more,
    })
}

fn parse_referenced_works(
    refs: &[Value],
    source_key: &PaperKey,
    offset: usize,
    max_results: Option<usize>,
) -> Result<ParsedProviderPage, String> {
    let mut all_papers = Vec::new();
    for entry in refs {
        let Some(raw) = entry.as_str() else { continue };
        let Ok(oa_key) = PaperKey::from_openalex(raw) else {
            continue;
        };
        let provider_id = oa_key.value().to_string();
        let url = Some(format!("https://openalex.org/{provider_id}"));
        let Ok(paper_ref) = PaperRef::from_ids_or(PaperIds::default(), None, Some(oa_key)) else {
            continue;
        };
        all_papers.push(LitPaper {
            paper_ref,
            title: String::new(),
            authors: Vec::new(),
            year: None,
            abstract_text: None,
            abstract_ref: None,
            citation_count: None,
            withdrawn_or_retracted: false,
            url,
            bibtex: None,
            provider: LitProvider::OpenAlex,
            provider_id,
        });
    }
    let total = all_papers.len() as u64;
    let sliced: Vec<LitPaper> = match max_results {
        Some(limit) => all_papers.into_iter().skip(offset).take(limit).collect(),
        None => all_papers.into_iter().skip(offset).collect(),
    };
    let edges = sliced
        .iter()
        .map(|p| CitationEdge {
            from: source_key.clone(),
            to: p.paper_ref.key.clone(),
            provider: LitProvider::OpenAlex,
        })
        .collect();
    let has_more = has_more_from_total(offset, sliced.len(), Some(total));
    Ok(ParsedProviderPage {
        papers: sliced,
        edges,
        total_results: Some(total),
        has_more,
    })
}

fn build_openalex_paper_ref(
    arxiv_split: Option<(String, Option<String>)>,
    doi: Option<String>,
    openalex_fallback: Option<PaperKey>,
) -> Option<PaperRef> {
    let (arxiv, version) = match arxiv_split {
        Some((base, ver)) => (Some(base), ver),
        None => (None, None),
    };
    PaperRef::from_ids_or(
        PaperIds {
            arxiv,
            doi,
            inspire: None,
            s2: None,
        },
        version,
        openalex_fallback,
    )
    .ok()
}

fn extract_openalex_arxiv(w: &Value) -> Option<(String, Option<String>)> {
    if let Some(parsed) = w["ids"]["arxiv"]
        .as_str()
        .and_then(|s| split_arxiv_id(s).ok())
    {
        return Some(parsed);
    }
    let landing = w["primary_location"]["landing_page_url"].as_str()?;
    if !landing.to_ascii_lowercase().contains("arxiv.org/") {
        return None;
    }
    split_arxiv_id(landing).ok()
}

fn extract_openalex_authors(w: &Value) -> Vec<String> {
    w["authorships"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    a["author"]["display_name"]
                        .as_str()
                        .or_else(|| a["raw_author_name"].as_str())
                        .map(normalize_whitespace)
                })
                .filter(|s| !s.is_empty())
                .take(MAX_AUTHORS_PER_PAPER)
                .collect()
        })
        .unwrap_or_default()
}
