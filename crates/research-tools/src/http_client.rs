//! Synchronous HTTPS-allowlisted & loopback-mock HTTP client for literature tools.

use crate::LiteratureEndpoints;
use std::io::Read;
use std::time::Duration;
use ureq::http::Uri;

pub const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EPRINT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 3;
pub const USER_AGENT: &str = "re-search/0.0.1";

const ALLOWLISTED_HTTPS_HOSTS: &[&str] = &[
    "export.arxiv.org",
    "arxiv.org",
    "inspirehep.net",
    "api.crossref.org",
    "api.semanticscholar.org",
];

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
        let lower = host.to_ascii_lowercase();
        if ALLOWLISTED_HTTPS_HOSTS.contains(&lower.as_str()) {
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

fn build_agent(is_https: bool) -> ureq::Agent {
    let mut cfg = ureq::Agent::config_builder()
        .https_only(is_https)
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)));
    if !is_https {
        cfg = cfg.proxy(None);
    }
    ureq::Agent::new_with_config(cfg.build())
}

/// Performs a bounded GET request enforcing HTTPS allowlist (or loopback mock ports),
/// following up to 3 redirects across allowlisted origins with `User-Agent: re-search/0.0.1`.
pub fn fetch_bytes_allowlisted(
    url: &str,
    endpoints: &LiteratureEndpoints,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut current: Uri = url.trim().parse().map_err(|e| format!("invalid URL `{url}`: {e}"))?;
    let mut redirects = 0usize;

    loop {
        let is_https = is_url_permitted(&current, endpoints)?;
        let agent = build_agent(is_https);
        let mut resp = agent
            .get(&current.to_string())
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("HTTP request to `{current}` failed: {e}"))?;

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

        if status != 200 {
            let mut err_buf = Vec::new();
            let _ = resp.body_mut().as_reader().take(2048).read_to_end(&mut err_buf);
            let snippet = String::from_utf8_lossy(&err_buf);
            return Err(format!("HTTP {status} from `{current}`: {}", snippet.trim()));
        }

        let mut buf = Vec::new();
        resp.body_mut()
            .as_reader()
            .take((max_bytes as u64) + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed reading HTTP body from `{current}`: {e}"))?;
        if buf.len() > max_bytes {
            return Err(format!("response from `{current}` exceeds {max_bytes} byte limit"));
        }
        return Ok(buf);
    }
}
