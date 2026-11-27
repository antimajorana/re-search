//! Synchronous HTTPS-allowlisted & loopback-mock HTTP client for literature tools.

use crate::net_rate::{global_gate, NetRateGate, Service};
use crate::LiteratureEndpoints;
use std::io::Read;
use std::time::Duration;
use ureq::http::Uri;

pub const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EPRINT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 3;
pub const USER_AGENT: &str = concat!("re-search/", env!("CARGO_PKG_VERSION"));

const ALLOWLISTED_HTTPS_HOSTS: &[&str] = &[
    "export.arxiv.org",
    "arxiv.org",
    "inspirehep.net",
    "api.crossref.org",
    "api.semanticscholar.org",
    // Owner decision 2026-11-26 (X19 Part 1): OpenAlex allowlisted through F3 gate;
    // TheoremSearch remains excluded/disabled.
    "api.openalex.org",
];

/// Returns `true` when `host` (case-insensitive) is in the production HTTPS literature allowlist.
pub fn is_allowlisted_https_host(host: &str) -> bool {
    let lower = host.trim().to_ascii_lowercase();
    ALLOWLISTED_HTTPS_HOSTS.contains(&lower.as_str())
}

/// Production HTTPS literature hosts permitted by the allowlist.
pub fn allowlisted_https_hosts() -> &'static [&'static str] {
    ALLOWLISTED_HTTPS_HOSTS
}

/// Bounded HTTP response returned by [`fetch_response_with_gate`] and
/// [`fetch_response_with_options_and_gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub retry_after: Option<String>,
    pub final_url: String,
    pub body: Vec<u8>,
}

/// Optional per-request settings for [`fetch_response_with_options_and_gate`].
///
/// Extra headers (such as a provider API key) are sent ONLY to the initial request origin
/// `(scheme, host, port)` and are stripped on any cross-origin redirect (`explore.md` rule P8).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HttpRequestOptions {
    pub extra_headers: Vec<(String, String)>,
    pub timeout_global: Option<Duration>,
}

impl std::fmt::Debug for HttpRequestOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let header_names: Vec<&str> = self.extra_headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("HttpRequestOptions")
            .field("extra_header_names", &header_names)
            .field("timeout_global", &self.timeout_global)
            .finish()
    }
}

fn parse_loopback_origin(base: &str) -> Option<(String, u16)> {
    let uri: Uri = base.trim().parse().ok()?;
    if uri.scheme_str()? != "http" {
        return None;
    }
    let auth = uri.authority()?;
    if auth.host() != "127.0.0.1" {
        return None;
    }
    let port = auth.port_u16()?;
    Some(("127.0.0.1".into(), port))
}

fn uri_origin(uri: &Uri) -> Option<(String, String, u16)> {
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    let auth = uri.authority()?;
    let host = auth.host().to_ascii_lowercase();
    let port = auth.port_u16().unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some((scheme, host, port))
}

fn validate_extra_headers(headers: &[(String, String)]) -> Result<(), String> {
    for (name, value) in headers {
        let trimmed = name.trim();
        if trimmed.is_empty()
            || trimmed.bytes().any(|b| b <= 0x20 || b >= 0x7f || b == b':')
        {
            return Err(format!("invalid HTTP header name `{name}`"));
        }
        if trimmed.eq_ignore_ascii_case("user-agent") {
            return Err("overriding User-Agent via extra_headers is not permitted".into());
        }
        if value
            .bytes()
            .any(|b| (b < 0x20 && b != b'\t') || b >= 0x7f)
        {
            return Err(format!("invalid HTTP header value for `{trimmed}`"));
        }
    }
    Ok(())
}

fn is_url_permitted(uri: &Uri, endpoints: &LiteratureEndpoints) -> Result<bool, String> {
    let scheme = uri.scheme_str().ok_or("URL missing scheme")?;
    let auth = uri.authority().ok_or("URL missing host")?;
    if auth.as_str().contains('@') {
        return Err("URL must not contain userinfo".into());
    }
    let host = auth.host();
    if host.is_empty() {
        return Err("URL host is empty".into());
    }
    if scheme.eq_ignore_ascii_case("https") {
        if is_allowlisted_https_host(host) {
            return Ok(true);
        }
        return Err(format!("HTTPS host `{host}` is not in the literature allowlist"));
    }
    if scheme.eq_ignore_ascii_case("http") {
        if host == "127.0.0.1" {
            let port = auth.port_u16().ok_or("loopback http:// URL must specify a port")?;
            let allowed_bases = [
                endpoints.arxiv_api_base.as_str(),
                endpoints.arxiv_eprint_base.as_str(),
                endpoints.inspire_api_base.as_str(),
                endpoints.crossref_api_base.as_str(),
                endpoints.semantic_scholar_api_base.as_str(),
                endpoints.openalex_api_base.as_str(),
            ];
            for base in allowed_bases {
                if let Some((_, p)) = parse_loopback_origin(base) {
                    if p == port {
                        return Ok(false);
                    }
                }
            }
        }
        return Err(format!("plaintext http:// URL disallowed for `{}`", uri));
    }
    Err(format!("unsupported URL scheme `{scheme}`"))
}

fn resolve_redirect(current: &Uri, location: &str) -> Result<Uri, String> {
    let trimmed = location.trim();
    if trimmed.is_empty() {
        return Err("empty Location header on redirect".into());
    }
    if trimmed.contains("://") {
        return trimmed
            .parse::<Uri>()
            .map_err(|e| format!("invalid redirect URL `{trimmed}`: {e}"));
    }
    let scheme = current.scheme_str().ok_or("missing current scheme")?;
    let auth = current.authority().ok_or("missing current authority")?;
    let path_and_query = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        let base_path = current.path();
        let dir = base_path.rfind('/').map(|idx| &base_path[..=idx]).unwrap_or("/");
        format!("{dir}{trimmed}")
    };
    format!("{scheme}://{auth}{path_and_query}")
        .parse::<Uri>()
        .map_err(|e| format!("invalid relative redirect `{trimmed}`: {e}"))
}

fn build_agent(is_https: bool, timeout_global: Option<Duration>) -> ureq::Agent {
    let global = timeout_global.unwrap_or(Duration::from_secs(30));
    let connect = Duration::from_secs(10).min(global);
    let mut cfg = ureq::Agent::config_builder()
        .https_only(is_https)
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(connect))
        .timeout_global(Some(global));
    if !is_https {
        cfg = cfg.proxy(None);
    }
    ureq::Agent::new_with_config(cfg.build())
}

/// Performs a bounded GET request through `gate`, enforcing the HTTPS allowlist (or loopback
/// mock ports) and following up to 3 redirects across allowlisted origins.
///
/// Every outgoing hop passes `gate` after URL validation succeeds. Non-200 responses return
/// `Ok(HttpResponse)` with at most 2048 bytes of error body so callers can distinguish HTTP
/// 404/429 from transport errors without fragile string matching.
pub fn fetch_response_with_gate(
    url: &str,
    endpoints: &LiteratureEndpoints,
    max_bytes: usize,
    service_hint: Option<Service>,
    gate: &NetRateGate,
) -> Result<HttpResponse, String> {
    fetch_response_with_options_and_gate(
        url,
        endpoints,
        max_bytes,
        service_hint,
        gate,
        &HttpRequestOptions::default(),
    )
}

/// Same as [`fetch_response_with_gate`] with per-request [`HttpRequestOptions`].
///
/// Extra headers in `options` are attached only when the hop's `(scheme, host, port)` matches
/// the initial request origin, preventing credentials such as `x-api-key` from leaking across
/// redirects (`explore.md` rule P8).
pub fn fetch_response_with_options_and_gate(
    url: &str,
    endpoints: &LiteratureEndpoints,
    max_bytes: usize,
    service_hint: Option<Service>,
    gate: &NetRateGate,
    options: &HttpRequestOptions,
) -> Result<HttpResponse, String> {
    validate_extra_headers(&options.extra_headers)?;
    let mut current: Uri = url.trim().parse().map_err(|e| format!("invalid URL `{url}`: {e}"))?;
    let initial_origin = uri_origin(&current);
    let mut redirects = 0usize;

    loop {
        let is_https = is_url_permitted(&current, endpoints)?;
        let _grant = gate.acquire_for_url(&current, endpoints, service_hint);
        let agent = build_agent(is_https, options.timeout_global);
        let mut req = agent
            .get(&current.to_string())
            .header("User-Agent", USER_AGENT);
        if initial_origin.is_some() && uri_origin(&current) == initial_origin {
            for (name, value) in &options.extra_headers {
                req = req.header(name.trim(), value.as_str());
            }
        }
        let mut resp = req.call().map_err(|e| match e {
            ureq::Error::Timeout(_) => format!("HTTP request to `{current}` timed out: {e}"),
            other => format!("HTTP request to `{current}` failed: {other}"),
        })?;

        let status = resp.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if redirects >= MAX_REDIRECTS {
                return Err(format!("exceeded maximum redirects ({MAX_REDIRECTS}) fetching `{url}`"));
            }
            let loc = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("HTTP {status} redirect missing Location header"))?;
            current = resolve_redirect(&current, loc)?;
            redirects += 1;
            continue;
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if status != 200 {
            let mut err_buf = Vec::new();
            let _ = resp.body_mut().as_reader().take(2048).read_to_end(&mut err_buf);
            return Ok(HttpResponse {
                status,
                content_type,
                retry_after,
                final_url: current.to_string(),
                body: err_buf,
            });
        }

        let mut buf = Vec::new();
        resp.body_mut()
            .as_reader()
            .take((max_bytes as u64) + 1)
            .read_to_end(&mut buf)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut {
                    format!("HTTP body read from `{current}` timed out: {e}")
                } else {
                    format!("failed reading HTTP body from `{current}`: {e}")
                }
            })?;
        if buf.len() > max_bytes {
            return Err(format!("response from `{current}` exceeds {max_bytes} byte limit"));
        }
        return Ok(HttpResponse {
            status,
            content_type,
            retry_after,
            final_url: current.to_string(),
            body: buf,
        });
    }
}

/// Performs a bounded GET request through `gate`, returning an error for non-200 responses.
pub fn fetch_bytes_with_gate(
    url: &str,
    endpoints: &LiteratureEndpoints,
    max_bytes: usize,
    service_hint: Option<Service>,
    gate: &NetRateGate,
) -> Result<Vec<u8>, String> {
    let resp = fetch_response_with_gate(url, endpoints, max_bytes, service_hint, gate)?;
    if resp.status != 200 {
        let snippet = String::from_utf8_lossy(&resp.body);
        return Err(format!("HTTP {} from `{}`: {}", resp.status, resp.final_url, snippet.trim()));
    }
    Ok(resp.body)
}

/// Performs a bounded GET request through the process-wide gate with an explicit logical
/// [`Service`] hint (used by `arxiv.rs`, `literature.rs` and `paper_fetch.rs`).
pub fn fetch_bytes_for_service(
    service: Service,
    url: &str,
    endpoints: &LiteratureEndpoints,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    fetch_bytes_with_gate(url, endpoints, max_bytes, Some(service), global_gate())
}

/// Performs a bounded GET request enforcing HTTPS allowlist (or loopback mock ports),
/// passing through the process-wide network rate gate and following up to 3 redirects
/// across allowlisted origins with the versioned reSearch user agent.
pub fn fetch_bytes_allowlisted(
    url: &str,
    endpoints: &LiteratureEndpoints,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    fetch_bytes_with_gate(url, endpoints, max_bytes, None, global_gate())
}
