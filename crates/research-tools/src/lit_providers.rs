//! Unified literature provider query runners (package F11).
//!
//! Supports the five allowlisted scholarly services (arXiv, INSPIRE-HEP, Semantic Scholar,
//! Crossref, and OpenAlex [X19 Part 1]) with abstracts, paging, and citation relations, keyed
//! by F2 [`PaperKey`] / [`PaperRef`]. TheoremSearch remains disabled (not allowlisted) and
//! returns [`QueryStatus::Disabled`] without touching the network.
//!
//! Every outgoing HTTP request goes strictly through
//! [`crate::http_client::fetch_response_with_options_and_gate`] backed by F3's [`NetRateGate`].

#[path = "lit_providers_parse.rs"]
pub(crate) mod parse;
#[path = "lit_providers_url.rs"]
mod url;

pub use parse::{parse_provider_response, ParsedProviderPage};
pub use url::parse_retry_after_secs;

use crate::http_client::{
    fetch_response_with_options_and_gate, HttpRequestOptions, MAX_METADATA_BYTES,
};
use crate::lit_abstracts::AbstractRef;
use crate::net_rate::{global_gate, NetRateGate, Service};
use crate::paper_key::{PaperKey, PaperRef};
use crate::LiteratureEndpoints;
use ledger::{sha256_hex, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fmt;
use std::time::Duration;
use tools::EVENT_SCHEMA_VERSION;
use url::build_request_url;

/// Shared ledger event kind for raw literature fetches (`00-overview.md` 2.8, `explore.md` 4.2).
pub const LITERATURE_FETCHED_EVENT: &str = "literature.fetched";
/// Default number of results requested when not specified.
pub const DEFAULT_MAX_RESULTS: usize = 10;
/// Maximum results per single provider request (`explore.md` 5.2: `max_results 1..25`).
pub const MAX_RESULTS_CAP: usize = 25;

/// Scholarly literature service identifiers (`explore.md` section 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LitProvider {
    Arxiv,
    Inspire,
    SemanticScholar,
    Crossref,
    #[serde(rename = "openalex")]
    OpenAlex,
    #[serde(rename = "theoremsearch")]
    TheoremSearch,
}

impl LitProvider {
    pub const ALL_ALLOWLISTED: [LitProvider; 5] = [
        Self::Arxiv,
        Self::Inspire,
        Self::SemanticScholar,
        Self::Crossref,
        Self::OpenAlex,
    ];
    pub const ALL: [LitProvider; 6] = [
        Self::Arxiv,
        Self::Inspire,
        Self::SemanticScholar,
        Self::Crossref,
        Self::OpenAlex,
        Self::TheoremSearch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arxiv => "arxiv",
            Self::Inspire => "inspire",
            Self::SemanticScholar => "semantic_scholar",
            Self::Crossref => "crossref",
            Self::OpenAlex => "openalex",
            Self::TheoremSearch => "theoremsearch",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "arxiv" => Some(Self::Arxiv),
            "inspire" => Some(Self::Inspire),
            "semantic_scholar" => Some(Self::SemanticScholar),
            "crossref" => Some(Self::Crossref),
            "openalex" => Some(Self::OpenAlex),
            "theoremsearch" => Some(Self::TheoremSearch),
            _ => None,
        }
    }

    /// Corresponding F3 [`Service`] rate-gate bucket, or `None` if not allowlisted.
    pub fn net_service(self) -> Option<Service> {
        match self {
            Self::Arxiv => Some(Service::Arxiv),
            Self::Inspire => Some(Service::Inspire),
            Self::SemanticScholar => Some(Service::SemanticScholar),
            Self::Crossref => Some(Service::Crossref),
            Self::OpenAlex => Some(Service::OpenAlex),
            Self::TheoremSearch => None,
        }
    }

    pub fn is_allowlisted(self) -> bool {
        self.net_service().is_some()
    }
}

impl fmt::Display for LitProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Search vs citation-relation mode for a [`LitQuery`] (`explore.md` sections 4.1 & 5.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryRelation {
    #[default]
    None,
    /// Outgoing references: papers cited by `key`.
    Cites { key: PaperKey },
    /// Incoming citations: papers that cite `key`.
    CitedBy { key: PaperKey },
}

fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}

/// Request parameters for a single literature service query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LitQuery {
    pub provider: LitProvider,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub relation: QueryRelation,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl LitQuery {
    pub fn search(provider: LitProvider, query: impl Into<String>) -> Self {
        Self {
            provider,
            query: query.into(),
            relation: QueryRelation::None,
            max_results: DEFAULT_MAX_RESULTS,
            offset: 0,
            qid: None,
            run_id: None,
        }
    }

    pub fn citation_relation(provider: LitProvider, relation: QueryRelation) -> Self {
        Self {
            provider,
            query: String::new(),
            relation,
            max_results: DEFAULT_MAX_RESULTS,
            offset: 0,
            qid: None,
            run_id: None,
        }
    }

    pub fn with_paging(mut self, max_results: usize, offset: usize) -> Self {
        self.max_results = max_results;
        self.offset = offset;
        self
    }

    pub fn with_tracking(
        mut self,
        qid: Option<impl Into<String>>,
        run_id: Option<impl Into<String>>,
    ) -> Self {
        self.qid = qid.map(Into::into);
        self.run_id = run_id.map(Into::into);
        self
    }
}

/// Per-call provider options (optional API key and timeout).
///
/// Rule P8 (`explore.md` section 6): API keys are sent only as headers to their own host/origin,
/// never placed in URLs, logs, or ledger payloads.
#[derive(Clone, Default)]
pub struct ProviderConfig {
    pub semantic_scholar_api_key: Option<String>,
    pub openalex_api_key: Option<String>,
    pub request_timeout: Option<Duration>,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s2 = self.semantic_scholar_api_key.as_ref().map(|_| "<redacted>");
        let oa = self.openalex_api_key.as_ref().map(|_| "<redacted>");
        f.debug_struct("ProviderConfig")
            .field("semantic_scholar_api_key", &s2)
            .field("openalex_api_key", &oa)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// Outcome status of a provider query (`explore.md` 4.1, 4.2, 5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    Ok,
    Failed,
    RateLimited,
    NotConfigured,
    Disabled,
    Unsupported,
}

/// Normalized paper metadata returned by a provider, keyed by F2 [`PaperRef`] / [`PaperKey`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LitPaper {
    pub paper_ref: PaperRef,
    pub title: String,
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_ref: Option<AbstractRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u64>,
    #[serde(default)]
    pub withdrawn_or_retracted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bibtex: Option<String>,
    pub provider: LitProvider,
    /// Provider-local identifier preserved for backwards-compatible `literature.fetched` entries.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider_id: String,
}

/// Directed citation relation (`from` cites `to`) returned by citation-relation queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationEdge {
    pub from: PaperKey,
    pub to: PaperKey,
    pub provider: LitProvider,
}

/// Complete structured result of a single literature provider query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LitQueryResult {
    pub provider: LitProvider,
    pub status: QueryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_raw: Option<String>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_url: Option<String>,
    pub papers: Vec<LitPaper>,
    pub edges: Vec<CitationEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_results: Option<u64>,
    pub offset: usize,
    pub has_more: bool,
    #[serde(skip)]
    pub raw_bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
}

impl LitQueryResult {
    fn status_only(provider: LitProvider, offset: usize, status: QueryStatus, msg: String) -> Self {
        Self {
            provider,
            status,
            http_status: None,
            retry_after_secs: None,
            retry_after_raw: None,
            timed_out: false,
            error: Some(msg),
            request_url: None,
            papers: Vec::new(),
            edges: Vec::new(),
            total_results: None,
            offset,
            has_more: false,
            raw_bytes: None,
            raw_blob: None,
        }
    }
}

/// Execute `query` through the process-wide F3 [`global_gate`].
pub fn run_query(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    config: &ProviderConfig,
) -> LitQueryResult {
    run_query_with_gate(endpoints, query, config, global_gate())
}

/// Execute `query` through an explicit F3 [`NetRateGate`] (for deterministic fake-clock tests).
pub fn run_query_with_gate(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    config: &ProviderConfig,
    gate: &NetRateGate,
) -> LitQueryResult {
    let max_results = query.max_results.clamp(1, MAX_RESULTS_CAP);
    let url = match build_request_url(endpoints, query, max_results) {
        Ok(u) => u,
        Err((status, msg)) => {
            return LitQueryResult::status_only(query.provider, query.offset, status, msg);
        }
    };
    let options = build_http_options(query.provider, config);
    let resp = match fetch_response_with_options_and_gate(
        &url,
        endpoints,
        MAX_METADATA_BYTES,
        query.provider.net_service(),
        gate,
        &options,
    ) {
        Ok(r) => r,
        Err(err) => {
            let low = err.to_ascii_lowercase();
            let mut out = LitQueryResult::status_only(
                query.provider,
                query.offset,
                QueryStatus::Failed,
                err,
            );
            out.timed_out = low.contains("timed out") || low.contains("timeout");
            out.request_url = Some(url);
            return out;
        }
    };

    if resp.status == 429 {
        let retry_after_secs = resp.retry_after.as_deref().and_then(parse_retry_after_secs);
        let mut out = LitQueryResult::status_only(
            query.provider,
            query.offset,
            QueryStatus::RateLimited,
            format!("HTTP 429 rate-limited by `{}`", query.provider),
        );
        out.http_status = Some(429);
        out.retry_after_secs = retry_after_secs;
        out.retry_after_raw = resp.retry_after;
        out.request_url = Some(url);
        return out;
    }

    if resp.status != 200 {
        // Provider error bodies can reflect request headers, including API keys.
        let detail = format!("HTTP {} from `{}`", resp.status, query.provider);
        let mut out =
            LitQueryResult::status_only(query.provider, query.offset, QueryStatus::Failed, detail);
        out.http_status = Some(resp.status);
        out.request_url = Some(url);
        return out;
    }

    let raw_blob = sha256_hex(&resp.body);
    match parse_provider_response(
        query.provider,
        &resp.body,
        &query.relation,
        query.offset,
        Some(max_results),
    ) {
        Ok(page) => LitQueryResult {
            provider: query.provider,
            status: QueryStatus::Ok,
            http_status: Some(200),
            retry_after_secs: None,
            retry_after_raw: None,
            timed_out: false,
            error: None,
            request_url: Some(url),
            papers: page.papers,
            edges: page.edges,
            total_results: page.total_results,
            offset: query.offset,
            has_more: page.has_more,
            raw_bytes: Some(resp.body),
            raw_blob: Some(raw_blob),
        },
        Err(err) => {
            let mut out = LitQueryResult::status_only(
                query.provider,
                query.offset,
                QueryStatus::Failed,
                err,
            );
            out.http_status = Some(200);
            out.request_url = Some(url);
            out.raw_bytes = Some(resp.body);
            out.raw_blob = Some(raw_blob);
            out
        }
    }
}

/// Run `query` through [`global_gate`] and, when [`QueryStatus::Ok`], store the raw response
/// blob and append a backwards-compatible `literature.fetched` event to `ledger`.
pub fn run_query_and_record(
    endpoints: &LiteratureEndpoints,
    ledger: &Ledger,
    session: &str,
    query: &LitQuery,
    config: &ProviderConfig,
) -> Result<LitQueryResult, String> {
    run_query_and_record_with_gate(endpoints, ledger, session, query, config, global_gate())
}

/// Same as [`run_query_and_record`] using an explicit [`NetRateGate`].
pub fn run_query_and_record_with_gate(
    endpoints: &LiteratureEndpoints,
    ledger: &Ledger,
    session: &str,
    query: &LitQuery,
    config: &ProviderConfig,
    gate: &NetRateGate,
) -> Result<LitQueryResult, String> {
    let mut result = run_query_with_gate(endpoints, query, config, gate);
    if result.status == QueryStatus::Ok {
        record_literature_fetched(ledger, session, query, &mut result, None)?;
    }
    Ok(result)
}

/// Store `result.raw_bytes` as a content-addressed blob and append `literature.fetched`
/// (`explore.md` 4.2), preserving backwards compatibility with `library_rebuild_lit.rs`,
/// `desktop-server/src/session_view.rs`, and `desktop-server/assets/source_links.js`.
pub fn record_literature_fetched(
    ledger: &Ledger,
    session: &str,
    query: &LitQuery,
    result: &mut LitQueryResult,
    cached_from_ms: Option<u64>,
) -> Result<String, String> {
    let raw_bytes = result
        .raw_bytes
        .as_deref()
        .ok_or_else(|| "cannot record literature.fetched without raw_bytes".to_string())?;
    let blob_hash = ledger
        .put_blob(raw_bytes)
        .map_err(|e| format!("literature.fetched put_blob failed: {e}"))?;
    result.raw_blob = Some(blob_hash.clone());
    for p in &mut result.papers {
        if let Some(aref) = p.abstract_ref.as_mut() {
            aref.raw_blob = blob_hash.clone();
        }
    }

    let mut payload = Map::new();
    payload.insert("provider".into(), json!(result.provider.as_str()));
    payload.insert("query".into(), json!(query_label(query)));
    if let Some(url) = &result.request_url {
        payload.insert("url".into(), json!(url));
    }
    payload.insert("count".into(), json!(result.papers.len()));
    payload.insert("raw_blob".into(), json!(blob_hash));
    if let Some(qid) = &query.qid {
        payload.insert("qid".into(), json!(qid));
    }
    if let Some(run_id) = &query.run_id {
        payload.insert("run_id".into(), json!(run_id));
    }
    if let Some(ts) = cached_from_ms {
        payload.insert("cached_from_ms".into(), json!(ts));
    }
    if query.relation != QueryRelation::None {
        payload.insert("relation".into(), json!(query.relation));
    }
    if query.offset > 0 {
        payload.insert("offset".into(), json!(query.offset));
    }
    if let Some(total) = result.total_results {
        payload.insert("total_results".into(), json!(total));
    }

    if result.provider == LitProvider::Arxiv {
        let papers: Vec<Value> = result
            .papers
            .iter()
            .map(|p| {
                json!({
                    "key": p.paper_ref.key.as_str(),
                    "id": p.provider_id,
                    "base_id": p.paper_ref.ids.arxiv,
                    "title": p.title,
                    "withdrawn": p.withdrawn_or_retracted,
                    "doi": p.paper_ref.ids.doi,
                    "year": p.year,
                    "url": p.url,
                })
            })
            .collect();
        payload.insert("papers".into(), Value::Array(papers));
    } else {
        let hits: Vec<Value> = result
            .papers
            .iter()
            .map(|p| {
                json!({
                    "key": p.paper_ref.key.as_str(),
                    "id": p.provider_id,
                    "title": p.title,
                    "doi": p.paper_ref.ids.doi,
                    "arxiv_id": p.paper_ref.ids.arxiv,
                    "retracted": p.withdrawn_or_retracted,
                    "year": p.year,
                    "url": p.url,
                })
            })
            .collect();
        payload.insert("hits".into(), Value::Array(hits));
    }

    ledger
        .append(
            session,
            LITERATURE_FETCHED_EVENT,
            EVENT_SCHEMA_VERSION,
            &Value::Object(payload),
        )
        .map_err(|e| format!("literature.fetched append failed: {e}"))?;
    Ok(blob_hash)
}

fn query_label(query: &LitQuery) -> String {
    let trimmed = query.query.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    match &query.relation {
        QueryRelation::None => String::new(),
        QueryRelation::Cites { key } => format!("cites:{key}"),
        QueryRelation::CitedBy { key } => format!("cited_by:{key}"),
    }
}

fn build_http_options(provider: LitProvider, config: &ProviderConfig) -> HttpRequestOptions {
    let mut extra_headers = Vec::new();
    match provider {
        LitProvider::SemanticScholar => {
            if let Some(key) = config
                .semantic_scholar_api_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
            {
                extra_headers.push(("x-api-key".to_string(), key.to_string()));
            }
        }
        LitProvider::OpenAlex => crate::lit_openalex::apply_auth_header(config, &mut extra_headers),
        _ => {}
    }
    HttpRequestOptions {
        extra_headers,
        timeout_global: config.request_timeout,
    }
}

#[cfg(test)]
#[path = "lit_providers_tests.rs"]
mod tests;
