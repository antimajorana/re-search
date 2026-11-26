//! Provider request URL builders and HTTP `Retry-After` parser (package F11).

use crate::lit_providers::{LitProvider, LitQuery, QueryRelation, QueryStatus};
use crate::paper_key::{PaperKey, PaperKeyKind};
use crate::tex_scan::normalize_arxiv_id;
use crate::LiteratureEndpoints;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::time::{SystemTime, UNIX_EPOCH};

// ASSUMPTION (beyond `literature.rs`): INSPIRE-HEP `/api/literature` accepts `page` (1-indexed)
// and `fields` to select metadata keys including `abstracts` and `withdrawn`.
const INSPIRE_FIELDS: &str = "control_number,texkeys,titles,authors,citation_count,dois,arxiv_eprints,preprint_date,publication_info,abstracts,withdrawn";
// ASSUMPTION (beyond `literature.rs`): Semantic Scholar Graph API accepts `abstract` in `fields`.
const S2_FIELDS: &str = "paperId,title,authors,year,abstract,citationCount,externalIds,url";

const ARXIV_FIELD_PREFIXES: &[&str] = &[
    "all:", "ti:", "au:", "abs:", "co:", "jr:", "cat:", "rn:", "id:",
];

pub(crate) fn build_request_url(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    max_results: usize,
) -> Result<String, (QueryStatus, String)> {
    match query.provider {
        LitProvider::OpenAlex => Err((
            QueryStatus::Disabled,
            "OpenAlex (`api.openalex.org`) is not in http_client.rs ALLOWLISTED_HTTPS_HOSTS yet (deferred to package X19)".into(),
        )),
        LitProvider::TheoremSearch => Err((
            QueryStatus::Disabled,
            "TheoremSearch is disabled by default and not allowlisted yet (deferred to package X19)".into(),
        )),
        LitProvider::Arxiv => build_arxiv_url(endpoints, query, max_results),
        LitProvider::Inspire => build_inspire_url(endpoints, query, max_results),
        LitProvider::SemanticScholar => build_s2_url(endpoints, query, max_results),
        LitProvider::Crossref => build_crossref_url(endpoints, query, max_results),
    }
}

fn build_arxiv_url(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    max_results: usize,
) -> Result<String, (QueryStatus, String)> {
    if query.relation != QueryRelation::None {
        return Err((
            QueryStatus::Unsupported,
            "arXiv Atom API does not support citation-relation queries (`cites` / `cited_by`)"
                .into(),
        ));
    }
    let q = query.query.trim();
    if q.is_empty() {
        return Err((QueryStatus::Failed, "`query` must not be empty".into()));
    }
    let base = endpoints.arxiv_api_base.trim_end_matches('/');
    let start = query.offset;
    if let Some(id) = normalize_arxiv_id(q) {
        let enc = utf8_percent_encode(&id, NON_ALPHANUMERIC).to_string();
        return Ok(format!(
            "{base}/api/query?id_list={enc}&start={start}&max_results={max_results}"
        ));
    }
    let has_prefix = q.split_whitespace().any(|tok| {
        let low = tok.trim_start_matches(['(', '"']).to_ascii_lowercase();
        ARXIV_FIELD_PREFIXES
            .iter()
            .any(|p| low.starts_with(p) && low.len() > p.len())
    });
    let expr = if has_prefix { q.to_string() } else { format!("all:{q}") };
    let enc = utf8_percent_encode(&expr, NON_ALPHANUMERIC).to_string();
    Ok(format!(
        "{base}/api/query?search_query={enc}&start={start}&max_results={max_results}"
    ))
}

fn build_inspire_url(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    max_results: usize,
) -> Result<String, (QueryStatus, String)> {
    let base = endpoints.inspire_api_base.trim_end_matches('/');
    let page = (query.offset / max_results) + 1;
    let q_expr = match &query.relation {
        QueryRelation::None => {
            let q = query.query.trim();
            if q.is_empty() {
                return Err((QueryStatus::Failed, "`query` must not be empty".into()));
            }
            q.to_string()
        }
        // ASSUMPTION (beyond `literature.rs`): INSPIRE-HEP search syntax uses `citedby:` for
        // papers cited by the target (outgoing references) and `refersto:` for papers citing
        // the target (incoming citations), accepting `recid:`, `doi:`, or `eprint:`.
        QueryRelation::Cites { key } => inspire_relation_query("citedby", key)?,
        QueryRelation::CitedBy { key } => inspire_relation_query("refersto", key)?,
    };
    let enc = utf8_percent_encode(&q_expr, NON_ALPHANUMERIC).to_string();
    Ok(format!(
        "{base}/api/literature?q={enc}&size={max_results}&page={page}&fields={INSPIRE_FIELDS}"
    ))
}

fn inspire_relation_query(op: &str, key: &PaperKey) -> Result<String, (QueryStatus, String)> {
    match key.kind() {
        PaperKeyKind::Inspire => Ok(format!("{op}:recid:{}", key.value())),
        PaperKeyKind::Doi => Ok(format!("{op}:doi:{}", key.value())),
        PaperKeyKind::Arxiv => Ok(format!("{op}:eprint:{}", key.value())),
        _ => Err((
            QueryStatus::Unsupported,
            format!("INSPIRE-HEP citation relation `{op}` does not support key `{key}`"),
        )),
    }
}

fn build_s2_url(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    max_results: usize,
) -> Result<String, (QueryStatus, String)> {
    let base = endpoints.semantic_scholar_api_base.trim_end_matches('/');
    let offset = query.offset;
    match &query.relation {
        QueryRelation::None => {
            let q = query.query.trim();
            if q.is_empty() {
                return Err((QueryStatus::Failed, "`query` must not be empty".into()));
            }
            let enc = utf8_percent_encode(q, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/graph/v1/paper/search?query={enc}&limit={max_results}&offset={offset}&fields={S2_FIELDS}"
            ))
        }
        // ASSUMPTION (beyond `literature.rs`): S2 citation endpoints `/graph/v1/paper/{id}/references`
        // and `/citations` accept raw 40-hex paperId, `ARXIV:<id>`, or `DOI:<doi>`.
        QueryRelation::Cites { key } => {
            let pid = s2_paper_id(key)?;
            let enc = utf8_percent_encode(&pid, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/graph/v1/paper/{enc}/references?limit={max_results}&offset={offset}&fields={S2_FIELDS}"
            ))
        }
        QueryRelation::CitedBy { key } => {
            let pid = s2_paper_id(key)?;
            let enc = utf8_percent_encode(&pid, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/graph/v1/paper/{enc}/citations?limit={max_results}&offset={offset}&fields={S2_FIELDS}"
            ))
        }
    }
}

fn s2_paper_id(key: &PaperKey) -> Result<String, (QueryStatus, String)> {
    match key.kind() {
        PaperKeyKind::S2 => Ok(key.value().to_string()),
        PaperKeyKind::Arxiv => Ok(format!("ARXIV:{}", key.value())),
        PaperKeyKind::Doi => Ok(format!("DOI:{}", key.value())),
        _ => Err((
            QueryStatus::Unsupported,
            format!(
                "Semantic Scholar citation endpoints require an s2, arxiv, or doi key (got `{key}`)"
            ),
        )),
    }
}

fn build_crossref_url(
    endpoints: &LiteratureEndpoints,
    query: &LitQuery,
    max_results: usize,
) -> Result<String, (QueryStatus, String)> {
    let base = endpoints.crossref_api_base.trim_end_matches('/');
    match &query.relation {
        QueryRelation::None => {
            let q = query.query.trim();
            if q.is_empty() {
                return Err((QueryStatus::Failed, "`query` must not be empty".into()));
            }
            let enc = utf8_percent_encode(q, NON_ALPHANUMERIC).to_string();
            Ok(format!(
                "{base}/works?query={enc}&rows={max_results}&offset={}",
                query.offset
            ))
        }
        QueryRelation::Cites { key } => {
            if key.kind() != PaperKeyKind::Doi {
                return Err((
                    QueryStatus::Unsupported,
                    format!(
                        "Crossref `/works/{{doi}}` references lookup requires a DOI key (got `{key}`)"
                    ),
                ));
            }
            let enc = utf8_percent_encode(key.value(), NON_ALPHANUMERIC).to_string();
            Ok(format!("{base}/works/{enc}"))
        }
        QueryRelation::CitedBy { .. } => Err((
            QueryStatus::Unsupported,
            "Crossref public REST API does not expose an incoming `cited_by` paper list".into(),
        )),
    }
}

/// Parse an HTTP `Retry-After` header value (`delay-seconds` or IMF-fixdate HTTP-date) into
/// seconds from now. Returns `None` for empty or unparseable values.
pub fn parse_retry_after_secs(raw: &str) -> Option<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())?;
    parse_retry_after_secs_at(raw, now)
}

pub(crate) fn parse_retry_after_secs_at(raw: &str, now_unix_secs: u64) -> Option<u64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u64>().ok();
    }
    parse_imf_fixdate_unix_secs(s).map(|target| target.saturating_sub(now_unix_secs))
}

/// Parse RFC 9110 IMF-fixdate (`"Wed, 21 Oct 2026 07:28:00 GMT"`) to Unix epoch seconds.
pub(crate) fn parse_imf_fixdate_unix_secs(s: &str) -> Option<u64> {
    let (_, rest) = s.split_once(',')?;
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() != 5 || !parts[4].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let day: u32 = parts[0].parse().ok()?;
    let month: u32 = match parts[1] {
        "Jan" => 1, "Feb" => 2, "Mar" => 3, "Apr" => 4, "May" => 5, "Jun" => 6,
        "Jul" => 7, "Aug" => 8, "Sep" => 9, "Oct" => 10, "Nov" => 11, "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts[2].parse().ok()?;
    let hms: Vec<&str> = parts[3].split(':').collect();
    if hms.len() != 3 {
        return None;
    }
    let hour: u64 = hms[0].parse().ok()?;
    let min: u64 = hms[1].parse().ok()?;
    let sec: u64 = hms[2].parse().ok()?;
    if !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60 || year < 1970 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    Some((days as u64) * 86_400 + hour * 3_600 + min * 60 + sec)
}

/// Howard Hinnant's exact civil-to-days algorithm (days since 1970-01-01).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = u64::from(m);
    let d = u64::from(d);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + (doe as i64) - 719_468
}
