//! Response parsers for allowlisted literature providers (package F11).
//!
//! arXiv Atom XML and INSPIRE-HEP JSON live here; Semantic Scholar and Crossref JSON live
//! in `lit_providers_parse_ext.rs`. External-service fields beyond `arxiv.rs` and
//! `literature.rs` are explicitly marked `ASSUMPTION`.

#[path = "lit_providers_parse_ext.rs"]
mod ext;

use crate::lit_abstracts::{normalize_abstract, normalize_whitespace, AbstractRef};
use crate::lit_providers::{CitationEdge, LitPaper, LitProvider, QueryRelation};
use crate::paper_key::{canonical_doi, split_arxiv_id, PaperIds, PaperKey, PaperRef};
use ledger::sha256_hex;
use serde_json::Value;

pub(super) const MAX_AUTHORS_PER_PAPER: usize = 100;

/// Parsed result page returned by [`parse_provider_response`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedProviderPage {
    pub papers: Vec<LitPaper>,
    pub edges: Vec<CitationEdge>,
    pub total_results: Option<u64>,
    pub has_more: bool,
}

/// Parse raw provider response bytes into F2-keyed [`LitPaper`] records and optional
/// [`CitationEdge`]s.
pub fn parse_provider_response(
    provider: LitProvider,
    raw_bytes: &[u8],
    relation: &QueryRelation,
    offset: usize,
    max_results: Option<usize>,
) -> Result<ParsedProviderPage, String> {
    let raw_blob = sha256_hex(raw_bytes);
    match provider {
        LitProvider::Arxiv => parse_arxiv_atom(raw_bytes, &raw_blob, offset),
        LitProvider::Inspire => parse_inspire_json(raw_bytes, &raw_blob, relation, offset),
        LitProvider::SemanticScholar => ext::parse_s2_json(raw_bytes, &raw_blob, relation, offset),
        LitProvider::Crossref => {
            ext::parse_crossref_json(raw_bytes, &raw_blob, relation, offset, max_results)
        }
        LitProvider::OpenAlex | LitProvider::TheoremSearch => {
            Err(format!("provider `{provider}` is disabled and has no active parser"))
        }
    }
}

pub(super) fn sanitized_paper_ref(
    arxiv_raw: Option<&str>,
    doi_raw: Option<&str>,
    inspire_raw: Option<&str>,
    s2_raw: Option<&str>,
) -> Option<PaperRef> {
    let (arxiv, version) = arxiv_raw
        .and_then(|raw| split_arxiv_id(raw).ok())
        .map(|(base, ver)| (Some(base), ver))
        .unwrap_or((None, None));
    let doi = doi_raw.and_then(|raw| canonical_doi(raw).ok());
    let inspire = inspire_raw
        .and_then(|raw| PaperKey::from_inspire(raw).ok())
        .map(|k| k.value().to_string());
    let s2 = s2_raw
        .and_then(|raw| PaperKey::from_s2(raw).ok())
        .map(|k| k.value().to_string());
    PaperRef::from_ids(PaperIds { arxiv, doi, inspire, s2 }, version).ok()
}

pub(super) fn make_edge(
    relation: &QueryRelation,
    paper_key: &PaperKey,
    provider: LitProvider,
) -> Option<CitationEdge> {
    match relation {
        QueryRelation::None => None,
        QueryRelation::Cites { key } => Some(CitationEdge {
            from: key.clone(),
            to: paper_key.clone(),
            provider,
        }),
        QueryRelation::CitedBy { key } => Some(CitationEdge {
            from: paper_key.clone(),
            to: key.clone(),
            provider,
        }),
    }
}

pub(super) fn make_abstract_ref(
    raw_blob: &str,
    provider: LitProvider,
    verbatim: bool,
) -> AbstractRef {
    AbstractRef {
        raw_blob: raw_blob.to_string(),
        provider: provider.as_str().to_string(),
        verbatim,
    }
}

pub(super) fn has_more_from_total(offset: usize, returned: usize, total: Option<u64>) -> bool {
    returned > 0 && total.is_some_and(|t| (offset as u64).saturating_add(returned as u64) < t)
}

fn parse_arxiv_atom(
    bytes: &[u8],
    raw_blob: &str,
    offset: usize,
) -> Result<ParsedProviderPage, String> {
    let xml =
        std::str::from_utf8(bytes).map_err(|_| "arXiv Atom feed is not valid UTF-8".to_string())?;
    let upper = xml.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err(
            "rejected arXiv XML feed containing DOCTYPE or ENTITY declaration (XXE guard)".into(),
        );
    }
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("invalid arXiv Atom XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "feed" {
        return Err(format!(
            "unexpected arXiv XML root element `<{}>` (expected `<feed>`)",
            root.tag_name().name()
        ));
    }

    // ASSUMPTION (beyond `arxiv.rs`): arXiv Atom 1.0 feeds expose OpenSearch paging metadata
    // via `<opensearch:totalResults>` and `<opensearch:startIndex>`.
    let total_results = child_text(&root, "totalResults").and_then(|s| s.trim().parse::<u64>().ok());
    let feed_start = child_text(&root, "startIndex")
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(offset);

    let mut papers = Vec::new();
    for entry in root
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "entry")
    {
        let raw_id = child_text(&entry, "id").unwrap_or_default();
        if raw_id.contains("arxiv.org/api/errors") {
            let msg = child_text(&entry, "summary").unwrap_or_else(|| "arXiv API error".into());
            return Err(format!("arXiv API error: {}", normalize_whitespace(&msg)));
        }
        let doi_raw = child_text(&entry, "doi")
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        let Some(paper_ref) = sanitized_paper_ref(Some(&raw_id), doi_raw.as_deref(), None, None)
        else {
            continue;
        };
        let provider_id = match (&paper_ref.ids.arxiv, &paper_ref.version) {
            (Some(base), Some(ver)) => format!("{base}{ver}"),
            (Some(base), None) => base.clone(),
            _ => paper_ref.key.value().to_string(),
        };
        let title = normalize_whitespace(&child_text(&entry, "title").unwrap_or_default());
        let summary_raw = child_text(&entry, "summary").unwrap_or_default();
        let abstract_text = normalize_abstract(&summary_raw, false);
        let abstract_ref = abstract_text
            .as_ref()
            .map(|_| make_abstract_ref(raw_blob, LitProvider::Arxiv, true));
        let published = child_text(&entry, "published").unwrap_or_default();
        let year = published.get(..4).and_then(|s| s.parse::<i32>().ok());
        let authors: Vec<String> = entry
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "author")
            .filter_map(|a| child_text(&a, "name"))
            .map(|s| normalize_whitespace(&s))
            .filter(|s| !s.is_empty())
            .take(MAX_AUTHORS_PER_PAPER)
            .collect();
        let comment = child_text(&entry, "comment")
            .map(|c| normalize_whitespace(&c))
            .filter(|c| !c.is_empty());
        let primary_category = entry
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "primary_category")
            .and_then(|c| c.attribute("term"))
            .map(str::to_string);
        let withdrawn = is_withdrawn(comment.as_deref(), &summary_raw);
        let url = Some(format!("https://arxiv.org/abs/{provider_id}"));
        let bibtex = Some(build_arxiv_bibtex(
            &provider_id,
            &title,
            &authors,
            year,
            paper_ref.ids.doi.as_deref(),
            primary_category.as_deref(),
        ));

        papers.push(LitPaper {
            paper_ref,
            title,
            authors,
            year,
            abstract_text,
            abstract_ref,
            citation_count: None,
            withdrawn_or_retracted: withdrawn,
            url,
            bibtex,
            provider: LitProvider::Arxiv,
            provider_id,
        });
    }

    let has_more = has_more_from_total(feed_start, papers.len(), total_results);
    Ok(ParsedProviderPage {
        papers,
        edges: Vec::new(),
        total_results,
        has_more,
    })
}

fn parse_inspire_json(
    bytes: &[u8],
    raw_blob: &str,
    relation: &QueryRelation,
    offset: usize,
) -> Result<ParsedProviderPage, String> {
    let root: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid JSON from `inspire`: {e}"))?;
    let hits_obj = root
        .get("hits")
        .and_then(Value::as_object)
        .ok_or_else(|| "malformed INSPIRE-HEP JSON: missing `hits` object".to_string())?;
    let items = hits_obj
        .get("hits")
        .and_then(Value::as_array)
        .ok_or_else(|| "malformed INSPIRE-HEP JSON: missing `hits.hits` array".to_string())?;
    // ASSUMPTION (beyond `literature.rs`): INSPIRE-HEP exposes total match count at `hits.total`.
    let total_results = hits_obj.get("total").and_then(Value::as_u64);

    let mut papers = Vec::new();
    let mut edges = Vec::new();
    for item in items {
        let meta = &item["metadata"];
        let recid = item["id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| meta["control_number"].as_u64().map(|n| n.to_string()));
        let doi_raw = meta["dois"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|d| d["value"].as_str());
        let arxiv_raw = meta["arxiv_eprints"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e["value"].as_str());
        let Some(paper_ref) = sanitized_paper_ref(arxiv_raw, doi_raw, recid.as_deref(), None)
        else {
            continue;
        };
        let provider_id = recid.clone().unwrap_or_else(|| paper_ref.key.value().to_string());
        let title = normalize_whitespace(
            meta["titles"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t["title"].as_str())
                .unwrap_or_default(),
        );
        let authors: Vec<String> = meta["authors"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a["full_name"].as_str().map(normalize_whitespace))
                    .filter(|s| !s.is_empty())
                    .take(MAX_AUTHORS_PER_PAPER)
                    .collect()
            })
            .unwrap_or_default();
        let year = meta["preprint_date"]
            .as_str()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse::<i32>().ok())
            .or_else(|| {
                meta["publication_info"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|p| p["year"].as_i64())
                    .and_then(|y| i32::try_from(y).ok())
            });
        // ASSUMPTION (beyond `literature.rs`): INSPIRE-HEP returns verbatim abstracts under
        // `metadata.abstracts[].value` and withdrawal status under `metadata.withdrawn`.
        let abstract_text = meta["abstracts"].as_array().and_then(|arr| {
            arr.iter()
                .filter_map(|a| a["value"].as_str())
                .find_map(|raw| normalize_abstract(raw, false))
        });
        let abstract_ref = abstract_text
            .as_ref()
            .map(|_| make_abstract_ref(raw_blob, LitProvider::Inspire, true));
        let citation_count = meta["citation_count"].as_u64();
        let withdrawn = meta["withdrawn"].as_bool().unwrap_or(false);
        let texkey = meta["texkeys"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        let bib_key = texkey.unwrap_or_else(|| default_bib_key(&authors, year, &provider_id));
        let url = recid.as_ref().map(|r| format!("https://inspirehep.net/literature/{r}"));
        let bibtex = Some(build_bibtex(
            &bib_key,
            &title,
            &authors,
            year,
            paper_ref.ids.doi.as_deref(),
            paper_ref.ids.arxiv.as_deref(),
        ));

        if let Some(edge) = make_edge(relation, &paper_ref.key, LitProvider::Inspire) {
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
            withdrawn_or_retracted: withdrawn,
            url,
            bibtex,
            provider: LitProvider::Inspire,
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

fn child_text(node: &roxmltree::Node<'_, '_>, local_name: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == local_name)
        .and_then(|c| c.text())
        .map(str::to_string)
}

fn is_withdrawn(comment: Option<&str>, summary: &str) -> bool {
    let check = |t: &str| {
        let low = t.to_ascii_lowercase();
        low.contains("withdrawn") || low.contains("retracted")
    };
    comment.is_some_and(check) || check(summary)
}

pub(super) fn default_bib_key(authors: &[String], year: Option<i32>, id: &str) -> String {
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

fn build_arxiv_bibtex(
    id: &str,
    title: &str,
    authors: &[String],
    year: Option<i32>,
    doi: Option<&str>,
    primary_class: Option<&str>,
) -> String {
    let key = default_bib_key(authors, year, id);
    let mut lines = vec![
        format!("@article{{{key},"),
        format!("  author = {{{}}},", authors.join(" and ")),
        format!("  title = {{{{{title}}}}},"),
    ];
    if let Some(y) = year {
        lines.push(format!("  year = {{{y}}},"));
    }
    lines.push(format!("  eprint = {{{id}}},"));
    lines.push("  archivePrefix = {arXiv},".to_string());
    if let Some(pc) = primary_class {
        lines.push(format!("  primaryClass = {{{pc}}},"));
    }
    if let Some(d) = doi {
        lines.push(format!("  doi = {{{d}}},"));
    }
    lines.push("}".into());
    lines.join("\n")
}

pub(super) fn build_bibtex(
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
