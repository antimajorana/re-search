//! Semantic Scholar and Crossref JSON response parsers (package F11).

use super::{
    build_bibtex, default_bib_key, has_more_from_total, make_abstract_ref, make_edge,
    sanitized_paper_ref, ParsedProviderPage, MAX_AUTHORS_PER_PAPER,
};
use crate::lit_abstracts::{normalize_abstract, normalize_whitespace};
use crate::lit_providers::{CitationEdge, LitPaper, LitProvider, QueryRelation};
use crate::paper_key::PaperKey;
use serde_json::Value;

pub(super) fn parse_s2_json(
    bytes: &[u8],
    raw_blob: &str,
    relation: &QueryRelation,
    offset: usize,
) -> Result<ParsedProviderPage, String> {
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("invalid JSON from `semantic_scholar`: {e}"))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "malformed Semantic Scholar JSON: root is not an object".to_string())?;
    // ASSUMPTION (beyond `literature.rs`): S2 search/references/citations return `total`, `offset`,
    // and optional `next` offset token; an empty search may omit `data` when `total == 0`.
    let total_results = obj.get("total").and_then(Value::as_u64);
    let next_offset = obj.get("next").and_then(Value::as_u64);

    let items: Vec<&Value> = if let Some(arr) = obj.get("data").and_then(Value::as_array) {
        arr.iter().collect()
    } else if obj.get("paperId").and_then(Value::as_str).is_some() {
        vec![&root]
    } else if total_results == Some(0) {
        Vec::new()
    } else {
        return Err("malformed Semantic Scholar JSON: missing `data` array".into());
    };

    let mut papers = Vec::new();
    let mut edges = Vec::new();
    for raw_item in items {
        // ASSUMPTION (beyond `literature.rs`): `/references` wraps each entry in `citedPaper`
        // and `/citations` wraps each entry in `citingPaper`.
        let p = match relation {
            QueryRelation::Cites { .. } => raw_item.get("citedPaper").unwrap_or(raw_item),
            QueryRelation::CitedBy { .. } => raw_item.get("citingPaper").unwrap_or(raw_item),
            QueryRelation::None => raw_item
                .get("citedPaper")
                .or_else(|| raw_item.get("citingPaper"))
                .unwrap_or(raw_item),
        };
        if !p.is_object() {
            continue;
        }
        let s2_id = p["paperId"].as_str().map(str::trim).filter(|s| !s.is_empty());
        let doi_raw = p["externalIds"]["DOI"].as_str();
        let arxiv_raw = p["externalIds"]["ArXiv"].as_str();
        let Some(paper_ref) = sanitized_paper_ref(arxiv_raw, doi_raw, None, s2_id) else {
            continue;
        };
        let provider_id = paper_ref
            .ids
            .s2
            .clone()
            .or_else(|| s2_id.map(str::to_string))
            .unwrap_or_else(|| paper_ref.key.value().to_string());
        let title = normalize_whitespace(p["title"].as_str().unwrap_or_default());
        let authors: Vec<String> = p["authors"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a["name"].as_str().map(normalize_whitespace))
                    .filter(|s| !s.is_empty())
                    .take(MAX_AUTHORS_PER_PAPER)
                    .collect()
            })
            .unwrap_or_default();
        let year = p["year"].as_i64().and_then(|y| i32::try_from(y).ok());
        // ASSUMPTION (beyond `literature.rs`): S2 `abstract` field holds verbatim abstract text.
        let abstract_text = p["abstract"]
            .as_str()
            .and_then(|raw| normalize_abstract(raw, false));
        let abstract_ref = abstract_text
            .as_ref()
            .map(|_| make_abstract_ref(raw_blob, LitProvider::SemanticScholar, true));
        let citation_count = p["citationCount"].as_u64();
        let url = p["url"].as_str().map(str::to_string);
        let bib_key = default_bib_key(&authors, year, &provider_id);
        let bibtex = Some(build_bibtex(
            &bib_key,
            &title,
            &authors,
            year,
            paper_ref.ids.doi.as_deref(),
            paper_ref.ids.arxiv.as_deref(),
        ));

        if let Some(edge) = make_edge(relation, &paper_ref.key, LitProvider::SemanticScholar) {
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
            withdrawn_or_retracted: false,
            url,
            bibtex,
            provider: LitProvider::SemanticScholar,
            provider_id,
        });
    }

    let has_more = next_offset.is_some() || has_more_from_total(offset, papers.len(), total_results);
    Ok(ParsedProviderPage {
        papers,
        edges,
        total_results,
        has_more,
    })
}

pub(super) fn parse_crossref_json(
    bytes: &[u8],
    raw_blob: &str,
    relation: &QueryRelation,
    offset: usize,
    max_results: Option<usize>,
) -> Result<ParsedProviderPage, String> {
    let root: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON from `crossref`: {e}"))?;
    let message = root
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "malformed Crossref JSON: missing `message` object".to_string())?;

    if let QueryRelation::Cites { key } = relation {
        return parse_crossref_references(message, key, offset, max_results);
    }

    // ASSUMPTION (beyond `literature.rs`): Crossref search responses report total hits in
    // `message.total-results` and JATS-XML abstracts in `message.items[].abstract`.
    let total_results = message.get("total-results").and_then(Value::as_u64);
    let items: Vec<&Value> = if let Some(arr) = message.get("items").and_then(Value::as_array) {
        arr.iter().collect()
    } else if message.contains_key("DOI") {
        vec![&root["message"]]
    } else {
        return Err("malformed Crossref JSON: missing `message.items` array".into());
    };

    let mut papers = Vec::new();
    for w in items {
        let doi_raw = w["DOI"].as_str();
        let Some(paper_ref) = sanitized_paper_ref(None, doi_raw, None, None) else {
            continue;
        };
        let provider_id = paper_ref
            .ids
            .doi
            .clone()
            .unwrap_or_else(|| paper_ref.key.value().to_string());
        let title = normalize_whitespace(
            w["title"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let authors: Vec<String> = w["author"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        if let Some(name) = a["name"].as_str() {
                            let clean = normalize_whitespace(name);
                            return (!clean.is_empty()).then_some(clean);
                        }
                        let given = a["given"].as_str().unwrap_or("").trim();
                        let family = a["family"].as_str().unwrap_or("").trim();
                        let joined = normalize_whitespace(&format!("{given} {family}"));
                        (!joined.is_empty()).then_some(joined)
                    })
                    .take(MAX_AUTHORS_PER_PAPER)
                    .collect()
            })
            .unwrap_or_default();
        let year = extract_crossref_year(w);
        let abstract_text = w["abstract"]
            .as_str()
            .and_then(|raw| normalize_abstract(raw, true));
        let abstract_ref = abstract_text
            .as_ref()
            .map(|_| make_abstract_ref(raw_blob, LitProvider::Crossref, true));
        let citation_count = w["is-referenced-by-count"].as_u64();
        let retracted = has_crossref_retraction(w);
        let url = w["URL"]
            .as_str()
            .map(str::to_string)
            .or_else(|| paper_ref.ids.doi.as_ref().map(|d| format!("https://doi.org/{d}")));
        let bib_key = default_bib_key(&authors, year, &provider_id);
        let bibtex = Some(build_bibtex(
            &bib_key,
            &title,
            &authors,
            year,
            paper_ref.ids.doi.as_deref(),
            None,
        ));

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
            provider: LitProvider::Crossref,
            provider_id,
        });
    }

    let has_more = has_more_from_total(offset, papers.len(), total_results);
    Ok(ParsedProviderPage {
        papers,
        edges: Vec::new(),
        total_results,
        has_more,
    })
}

/// Parse Crossref `/works/{doi}` `message.reference[]` for `QueryRelation::Cites`.
///
/// ASSUMPTION (beyond `literature.rs`): Crossref work records list references in
/// `message.reference[]` with optional `DOI`, `article-title`, `volume-title`, `unstructured`,
/// `author`, and `year` fields. References without a valid DOI are skipped cleanly.
fn parse_crossref_references(
    message: &serde_json::Map<String, Value>,
    source_key: &PaperKey,
    offset: usize,
    max_results: Option<usize>,
) -> Result<ParsedProviderPage, String> {
    let refs = match message.get("reference") {
        Some(Value::Array(arr)) => arr.as_slice(),
        None => &[],
        Some(_) => return Err("malformed Crossref JSON: `message.reference` is not an array".into()),
    };
    let mut all_papers = Vec::new();
    for r in refs {
        let doi_raw = r["DOI"].as_str();
        let Some(paper_ref) = sanitized_paper_ref(None, doi_raw, None, None) else {
            continue;
        };
        let provider_id = paper_ref
            .ids
            .doi
            .clone()
            .unwrap_or_else(|| paper_ref.key.value().to_string());
        let title = normalize_whitespace(
            r["article-title"]
                .as_str()
                .or_else(|| r["volume-title"].as_str())
                .or_else(|| r["unstructured"].as_str())
                .unwrap_or_default(),
        );
        let authors = r["author"]
            .as_str()
            .map(normalize_whitespace)
            .filter(|s| !s.is_empty())
            .into_iter()
            .collect::<Vec<_>>();
        let year = r["year"]
            .as_str()
            .and_then(|s| s.get(..4))
            .and_then(|s| s.parse::<i32>().ok())
            .or_else(|| r["year"].as_i64().and_then(|y| i32::try_from(y).ok()));
        let url = paper_ref.ids.doi.as_ref().map(|d| format!("https://doi.org/{d}"));
        let bib_key = default_bib_key(&authors, year, &provider_id);
        let bibtex = Some(build_bibtex(
            &bib_key,
            &title,
            &authors,
            year,
            paper_ref.ids.doi.as_deref(),
            None,
        ));
        all_papers.push(LitPaper {
            paper_ref,
            title,
            authors,
            year,
            abstract_text: None,
            abstract_ref: None,
            citation_count: None,
            withdrawn_or_retracted: false,
            url,
            bibtex,
            provider: LitProvider::Crossref,
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
            provider: LitProvider::Crossref,
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

fn extract_crossref_year(w: &Value) -> Option<i32> {
    for k in ["published-print", "published-online", "issued", "created"] {
        if let Some(y) = w[k]["date-parts"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|r| r.as_array())
            .and_then(|p| p.first())
            .and_then(Value::as_i64)
            .and_then(|y| i32::try_from(y).ok())
        {
            return Some(y);
        }
    }
    None
}

fn has_crossref_retraction(w: &Value) -> bool {
    if w["relation"]["is-retracted-by"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
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
