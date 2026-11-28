//! Metadata, version list, and identifier resolution helpers for [`crate::paper_fetch`].
//!
//! All outgoing HTTP requests go through `crate::http_client::fetch_response_with_gate`
//! with an explicit logical [`crate::net_rate::Service`]. Paper identities use F2's
//! [`PaperKey`], [`PaperRef`], and [`PaperIds`] exclusively.

use crate::arxiv::parse_atom_feed;
use crate::http_client::{fetch_response_with_gate, MAX_METADATA_BYTES};
use crate::literature::parse_hits;
use crate::net_rate::{NetRateGate, Service};
use crate::paper_key::{canonical_doi, split_arxiv_id, PaperIds, PaperKey, PaperKeyKind, PaperRef};
use crate::LiteratureEndpoints;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

const MAX_ARXIV_VERSIONS: u32 = 200;

/// Metadata returned by `paper_fetch::fetch(..., FetchWant::Metadata)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperMetadata {
    pub paper: PaperRef,
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    pub withdrawn: bool,
    pub retracted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
}

/// One version of a paper returned by `paper_fetch::fetch(..., FetchWant::Versions)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperVersion {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    pub latest: bool,
}

/// Normalize a user-supplied arXiv id, arXiv URL, DOI, DOI URL, or canonical `PaperKey`
/// into an F2 [`PaperRef`].
pub fn parse_paper_input(raw: &str) -> Result<PaperRef, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("paper identifier must not be empty".into());
    }
    if let Ok(key) = PaperKey::parse(trimmed) {
        let ids = match key.kind() {
            PaperKeyKind::Arxiv => PaperIds {
                arxiv: Some(key.value().to_string()),
                ..PaperIds::default()
            },
            PaperKeyKind::Doi => PaperIds {
                doi: Some(key.value().to_string()),
                ..PaperIds::default()
            },
            PaperKeyKind::Inspire => PaperIds {
                inspire: Some(key.value().to_string()),
                ..PaperIds::default()
            },
            PaperKeyKind::S2 => PaperIds {
                s2: Some(key.value().to_string()),
                ..PaperIds::default()
            },
            PaperKeyKind::Openalex | PaperKeyKind::File => PaperIds::default(),
        };
        return PaperRef::from_ids_or(ids, None, Some(key));
    }
    if let Ok((base, version)) = split_arxiv_id(trimmed) {
        return PaperRef::from_ids(
            PaperIds {
                arxiv: Some(base),
                ..PaperIds::default()
            },
            version,
        );
    }
    if let Ok(doi) = canonical_doi(trimmed) {
        return PaperRef::from_ids(
            PaperIds {
                doi: Some(doi),
                ..PaperIds::default()
            },
            None,
        );
    }
    Err(format!(
        "unrecognised paper identifier `{}` (expected an arXiv id/URL, DOI, or canonical PaperKey)",
        trimmed.chars().take(80).collect::<String>()
    ))
}

/// Format the arXiv identifier to request from arXiv API / e-print / PDF endpoints.
pub(crate) fn arxiv_target_id(base_id: &str, version: Option<&str>) -> String {
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => format!("{base_id}{v}"),
        None => base_id.to_string(),
    }
}

/// Extract the arXiv base id already present on `paper`, without any network call.
pub(crate) fn local_arxiv_base(paper: &PaperRef) -> Option<String> {
    if paper.key.kind() == PaperKeyKind::Arxiv {
        return Some(paper.key.value().to_string());
    }
    paper
        .ids
        .arxiv
        .as_deref()
        .and_then(|raw| split_arxiv_id(raw).ok().map(|(b, _)| b))
}

/// Attempt to resolve `paper` to an arXiv-backed [`PaperRef`] (plus any discovered title).
///
/// Returns `Ok(None)` when the queried services authoritatively report no arXiv version
/// (so the caller should return `NeedsFile`), and `Err(_)` only when every attempted
/// lookup failed due to a transport or server error.
pub(crate) fn resolve_to_arxiv(
    paper: &PaperRef,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<(PaperRef, Option<String>)>, String> {
    if let Some(base) = local_arxiv_base(paper) {
        let mut ids = paper.ids.clone();
        ids.arxiv = Some(base);
        let resolved = PaperRef::from_ids(ids, paper.version.clone())?;
        return Ok(Some((resolved, None)));
    }

    if let Some(doi) = paper
        .ids
        .doi
        .as_deref()
        .or_else(|| (paper.key.kind() == PaperKeyKind::Doi).then_some(paper.key.value()))
    {
        return resolve_doi_to_arxiv(paper, doi, endpoints, gate);
    }

    if let Some(recid) = paper
        .ids
        .inspire
        .as_deref()
        .or_else(|| (paper.key.kind() == PaperKeyKind::Inspire).then_some(paper.key.value()))
    {
        return resolve_inspire_recid_to_arxiv(paper, recid, endpoints, gate);
    }

    if let Some(s2_id) = paper
        .ids
        .s2
        .as_deref()
        .or_else(|| (paper.key.kind() == PaperKeyKind::S2).then_some(paper.key.value()))
    {
        return resolve_s2_id_to_arxiv(paper, s2_id, endpoints, gate);
    }

    Ok(None)
}

fn resolve_doi_to_arxiv(
    paper: &PaperRef,
    doi: &str,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<(PaperRef, Option<String>)>, String> {
    let mut saw_clean_miss = false;
    let mut last_err: Option<String> = None;

    // 1. INSPIRE-HEP exact DOI lookup.
    let enc_doi_query = utf8_percent_encode(&format!("doi:{doi}"), NON_ALPHANUMERIC).to_string();
    let inspire_url = format!(
        "{}/api/literature?q={enc_doi_query}&size=1",
        endpoints.inspire_api_base.trim_end_matches('/')
    );
    match fetch_response_with_gate(
        &inspire_url,
        endpoints,
        MAX_METADATA_BYTES,
        Some(Service::Inspire),
        gate,
    ) {
        Ok(resp) if resp.status == 200 => match parse_hits("inspire", &resp.body) {
            Ok(hits) => {
                if let Some(found) = extract_arxiv_from_hits(paper, &hits) {
                    return Ok(Some(found));
                }
                saw_clean_miss = true;
            }
            Err(e) => last_err = Some(e),
        },
        Ok(resp) if resp.status == 404 => saw_clean_miss = true,
        Ok(resp) => last_err = Some(format!("INSPIRE HTTP {}", resp.status)),
        Err(e) => last_err = Some(e),
    }

    // 2. Semantic Scholar DOI lookup (covers fields outside HEP).
    let enc_doi = utf8_percent_encode(doi, NON_ALPHANUMERIC).to_string();
    let s2_url = format!(
        "{}/graph/v1/paper/DOI:{enc_doi}?fields=title,authors,year,citationCount,externalIds,url",
        endpoints.semantic_scholar_api_base.trim_end_matches('/')
    );
    match fetch_response_with_gate(
        &s2_url,
        endpoints,
        MAX_METADATA_BYTES,
        Some(Service::SemanticScholar),
        gate,
    ) {
        Ok(resp) if resp.status == 200 => match parse_hits("semantic_scholar", &resp.body) {
            Ok(hits) => {
                if let Some(found) = extract_arxiv_from_hits(paper, &hits) {
                    return Ok(Some(found));
                }
                saw_clean_miss = true;
            }
            Err(e) => last_err = Some(e),
        },
        Ok(resp) if resp.status == 404 => saw_clean_miss = true,
        Ok(resp) => last_err = Some(format!("Semantic Scholar HTTP {}", resp.status)),
        Err(e) => last_err = Some(e),
    }

    if saw_clean_miss {
        Ok(None)
    } else {
        Err(last_err.unwrap_or_else(|| "unable to resolve DOI to arXiv".into()))
    }
}

fn resolve_inspire_recid_to_arxiv(
    paper: &PaperRef,
    recid: &str,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<(PaperRef, Option<String>)>, String> {
    let enc = utf8_percent_encode(recid, NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/api/literature?q=recid%3A{enc}&size=1",
        endpoints.inspire_api_base.trim_end_matches('/')
    );
    let resp = fetch_response_with_gate(
        &url,
        endpoints,
        MAX_METADATA_BYTES,
        Some(Service::Inspire),
        gate,
    )?;
    if resp.status == 404 {
        return Ok(None);
    }
    if resp.status != 200 {
        return Err(format!("INSPIRE HTTP {} resolving recid {recid}", resp.status));
    }
    let hits = parse_hits("inspire", &resp.body)?;
    Ok(extract_arxiv_from_hits(paper, &hits))
}

fn resolve_s2_id_to_arxiv(
    paper: &PaperRef,
    s2_id: &str,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<(PaperRef, Option<String>)>, String> {
    let enc = utf8_percent_encode(s2_id, NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/graph/v1/paper/{enc}?fields=title,authors,year,citationCount,externalIds,url",
        endpoints.semantic_scholar_api_base.trim_end_matches('/')
    );
    let resp = fetch_response_with_gate(
        &url,
        endpoints,
        MAX_METADATA_BYTES,
        Some(Service::SemanticScholar),
        gate,
    )?;
    if resp.status == 404 {
        return Ok(None);
    }
    if resp.status != 200 {
        return Err(format!("Semantic Scholar HTTP {} resolving {s2_id}", resp.status));
    }
    let hits = parse_hits("semantic_scholar", &resp.body)?;
    Ok(extract_arxiv_from_hits(paper, &hits))
}

fn extract_arxiv_from_hits(
    paper: &PaperRef,
    hits: &[crate::literature::LiteratureHit],
) -> Option<(PaperRef, Option<String>)> {
    for hit in hits {
        let Some(raw_ax) = hit.arxiv_id.as_deref() else {
            continue;
        };
        let Ok((base, ver)) = split_arxiv_id(raw_ax) else {
            continue;
        };
        let mut ids = paper.ids.clone();
        ids.arxiv = Some(base);
        if ids.doi.is_none() {
            ids.doi = hit.doi.as_deref().and_then(|d| canonical_doi(d).ok());
        }
        let version = paper.version.clone().or(ver);
        if let Ok(resolved) = PaperRef::from_ids(ids, version) {
            let title = (!hit.title.trim().is_empty()).then(|| hit.title.trim().to_string());
            return Some((resolved, title));
        }
    }
    None
}

/// Fetch arXiv Atom metadata for `paper` (which must have an arXiv base id).
pub(crate) fn fetch_arxiv_metadata(
    paper: &PaperRef,
    base_id: &str,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<PaperMetadata>, String> {
    let target = arxiv_target_id(base_id, paper.version.as_deref());
    let enc = utf8_percent_encode(&target, NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/api/query?id_list={enc}&max_results=1",
        endpoints.arxiv_api_base.trim_end_matches('/')
    );
    let resp = fetch_response_with_gate(
        &url,
        endpoints,
        MAX_METADATA_BYTES,
        Some(Service::Arxiv),
        gate,
    )?;
    if resp.status == 404 {
        return Ok(None);
    }
    if resp.status != 200 {
        return Err(format!("arXiv metadata HTTP {} from `{}`", resp.status, resp.final_url));
    }
    let licence = extract_atom_license(&resp.body);
    let mut entries = parse_atom_feed(&resp.body)?;
    let Some(entry) = entries.pop() else {
        return Ok(None);
    };
    let (norm_base, latest_version) = match split_arxiv_id(&entry.id) {
        Ok(pair) => pair,
        Err(_) => (base_id.to_string(), paper.version.clone()),
    };
    let mut ids = paper.ids.clone();
    ids.arxiv = Some(norm_base);
    if ids.doi.is_none() {
        ids.doi = entry.doi.as_deref().and_then(|d| canonical_doi(d).ok());
    }
    let enriched = PaperRef::from_ids(ids, paper.version.clone().or_else(|| latest_version.clone()))?;
    Ok(Some(PaperMetadata {
        paper: enriched,
        title: entry.title,
        authors: entry.authors,
        abstract_text: entry.abstract_text,
        published: (!entry.published.is_empty()).then_some(entry.published),
        updated: (!entry.updated.is_empty()).then_some(entry.updated),
        primary_category: entry.primary_category,
        comment: entry.comment,
        journal_ref: entry.journal_ref,
        licence,
        withdrawn: entry.withdrawn,
        retracted: false,
        url: Some(entry.url),
        latest_version,
    }))
}

/// Fetch arXiv version list in a single Atom query (`id_list=<base_id>`).
///
/// arXiv versions are strictly contiguous `v1..=vN`, so a single metadata request reveals
/// both the latest version `vN` and the full version sequence (matching `investigate.md`
/// Stage M's "one metadata request" budget without scraping HTML).
pub(crate) fn fetch_arxiv_versions(
    paper: &PaperRef,
    base_id: &str,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<(PaperRef, Vec<PaperVersion>)>, String> {
    let mut base_ref = paper.clone();
    base_ref.version = None;
    let Some(meta) = fetch_arxiv_metadata(&base_ref, base_id, endpoints, gate)? else {
        return Ok(None);
    };
    let versions = synthesize_versions(
        meta.latest_version.as_deref(),
        meta.published.as_deref(),
        meta.updated.as_deref(),
    );
    Ok(Some((meta.paper, versions)))
}

pub(crate) fn synthesize_versions(
    latest_version: Option<&str>,
    published: Option<&str>,
    updated: Option<&str>,
) -> Vec<PaperVersion> {
    let latest_num = latest_version
        .and_then(|v| v.strip_prefix('v'))
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(1)
        .min(MAX_ARXIV_VERSIONS);
    (1..=latest_num)
        .map(|n| {
            let is_latest = n == latest_num;
            let date = if is_latest {
                updated.or(published).map(str::to_string)
            } else if n == 1 {
                published.map(str::to_string)
            } else {
                None
            };
            PaperVersion {
                version: format!("v{n}"),
                date,
                latest: is_latest,
            }
        })
        .collect()
}

/// Fetch Crossref metadata for a DOI paper that has no arXiv version.
pub(crate) fn fetch_doi_metadata(
    paper: &PaperRef,
    doi: &str,
    endpoints: &LiteratureEndpoints,
    gate: &NetRateGate,
) -> Result<Option<PaperMetadata>, String> {
    let enc = utf8_percent_encode(doi, NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{}/works/{enc}",
        endpoints.crossref_api_base.trim_end_matches('/')
    );
    let resp = fetch_response_with_gate(
        &url,
        endpoints,
        MAX_METADATA_BYTES,
        Some(Service::Crossref),
        gate,
    )?;
    if resp.status == 404 {
        return Ok(None);
    }
    if resp.status != 200 {
        return Err(format!("Crossref HTTP {} from `{}`", resp.status, resp.final_url));
    }
    let mut hits = parse_hits("crossref", &resp.body)?;
    let Some(hit) = hits.pop() else {
        return Ok(None);
    };
    Ok(Some(PaperMetadata {
        paper: paper.clone(),
        title: hit.title,
        authors: hit.authors,
        abstract_text: String::new(),
        published: hit.year.map(|y| y.to_string()),
        updated: None,
        primary_category: None,
        comment: None,
        journal_ref: None,
        licence: None,
        withdrawn: false,
        retracted: hit.retracted,
        url: hit.url,
        latest_version: None,
    }))
}

fn extract_atom_license(bytes: &[u8]) -> Option<String> {
    let xml = std::str::from_utf8(bytes).ok()?;
    let head_upper = xml.chars().take(4096).collect::<String>().to_ascii_uppercase();
    if head_upper.contains("<!DOCTYPE") || head_upper.contains("<!ENTITY") {
        return None;
    }
    let doc = roxmltree::Document::parse(xml).ok()?;
    let entry = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "entry")?;
    for child in entry.children().filter(|n| n.is_element()) {
        if child.tag_name().name() == "license" {
            if let Some(text) = child.text().map(str::trim).filter(|s| !s.is_empty()) {
                return Some(text.to_string());
            }
        }
        if child.tag_name().name() == "link" && child.attribute("rel") == Some("license") {
            if let Some(href) = child.attribute("href").map(str::trim).filter(|s| !s.is_empty()) {
                return Some(href.to_string());
            }
        }
    }
    None
}
