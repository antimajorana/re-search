//! OAuth endpoints and calls for Sign in with ChatGPT (open-source / local apps).
//!
//! Parameter names follow OpenAI's "Registration and sign-in", "Accounts and sessions" and
//! "Token reference" pages. Production endpoints are fixed (the values of
//! `https://auth.openai.com/.well-known/openid-configuration`); tests may point every
//! endpoint at one plaintext loopback server, and nothing else ([`Endpoints::loopback_mock`]).
//!
//! Errors never contain a token, a code or server text: an HTTP failure is reduced to a
//! status class and, for OAuth errors, to a known error code.

use crate::codec::{form_body, url_encode, Secret};
use serde_json::Value;
use std::time::Duration;

pub const ISSUER: &str = "https://auth.openai.com";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const REVOKE_URL: &str = "https://auth.openai.com/api/accounts/oauth/revoke";
pub const JWKS_URL: &str = "https://auth.openai.com/.well-known/jwks.json";
pub const API_BASE: &str = "https://api.openai.com";
/// `resource` on authorize, code exchange and refresh.
pub const RESOURCE: &str = "https://api.openai.com/v1";
/// Identity scopes plus the ChatGPT plan usage scopes.
pub const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// The scope without which inference on the user's plan is not authorized.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// First-time registration entry point (never saved as the issued client ID).
pub const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
/// `agent_name_hint` on first-time registration: the app's actual name.
pub const AGENT_NAME_HINT: &str = "re:Search";
/// Fixed loopback callback path (only the port may vary between sign-ins).
pub const REDIRECT_PATH: &str = "/auth/callback";

const BODY_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub revoke: String,
    pub jwks: String,
    /// Where the Responses API and the model list live.
    pub api_base: String,
}

impl Endpoints {
    pub fn production() -> Self {
        Self {
            issuer: ISSUER.into(),
            authorize: AUTHORIZE_URL.into(),
            token: TOKEN_URL.into(),
            revoke: REVOKE_URL.into(),
            jwks: JWKS_URL.into(),
            api_base: API_BASE.into(),
        }
    }

    /// Every endpoint on one `http://127.0.0.1:<port>` test server (same paths as
    /// production). Anything but a plaintext 127.0.0.1 address is refused, so a test hook
    /// can never send tokens to another host.
    pub fn loopback_mock(base: &str) -> Result<Self, String> {
        let base = base.trim_end_matches('/');
        let rest = base.strip_prefix("http://127.0.0.1:").ok_or("test endpoints must be http://127.0.0.1:<port>")?;
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return Err("test endpoints must be http://127.0.0.1:<port>".into());
        }
        Ok(Self {
            issuer: base.into(),
            authorize: format!("{base}/api/accounts/authorize"),
            token: format!("{base}/api/accounts/oauth/token"),
            revoke: format!("{base}/api/accounts/oauth/revoke"),
            jwks: format!("{base}/.well-known/jwks.json"),
            api_base: base.into(),
        })
    }

    pub fn is_production(&self) -> bool {
        *self == Self::production()
    }
}

/// Values of one authorization attempt.
pub struct AuthorizeRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    pub code_challenge: &'a str,
    pub ext_agent_host_id: &'a str,
}

/// The authorize URL. `agent_name_hint` is sent only with `dynamic_agent_client`. No
/// `id_token_hint` is ever sent: the URL is shown to the user and may be copied, and a
/// token must not travel in it.
pub fn authorize_url(ep: &Endpoints, r: &AuthorizeRequest<'_>) -> String {
    let mut pairs: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", r.client_id),
        ("redirect_uri", r.redirect_uri),
        ("scope", SCOPES),
        ("resource", RESOURCE),
        ("state", r.state),
        ("nonce", r.nonce),
        ("code_challenge", r.code_challenge),
        ("code_challenge_method", "S256"),
        ("ext_agent_host_id", r.ext_agent_host_id),
    ];
    if r.client_id == DYNAMIC_CLIENT_ID {
        pairs.push(("agent_name_hint", AGENT_NAME_HINT));
    }
    let q: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect();
    format!("{}?{}", ep.authorize, q.join("&"))
}

/// A failed OAuth call, without any server text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthError {
    /// `error` code from a JSON error body (only codes from [`KNOWN_CODES`]; others are
    /// `"other"`), with the HTTP status.
    Rejected { status: u16, code: &'static str },
    /// Non-JSON or unexpected answer, by status.
    Http(u16),
    Network(&'static str),
    Malformed(&'static str),
}

/// OAuth error codes kept verbatim (RFC 6749 plus the refresh codes the documentation
/// lists in "Errors and recovery").
pub const KNOWN_CODES: &[&str] = &[
    "invalid_request", "invalid_client", "invalid_grant", "unauthorized_client", "unsupported_grant_type",
    "invalid_scope", "access_denied", "server_error", "temporarily_unavailable", "invalid_refresh_token",
    "token_expired", "refresh_token_expired", "refresh_token_invalidated", "refresh_token_reused",
];

/// Refresh error codes after which the saved tokens are unusable.
pub const TERMINAL_REFRESH_CODES: &[&str] = &[
    "invalid_grant", "invalid_refresh_token", "token_expired", "refresh_token_expired",
    "refresh_token_invalidated", "refresh_token_reused",
];

pub fn known_code(raw: &str) -> &'static str {
    KNOWN_CODES.iter().find(|c| **c == raw).copied().unwrap_or("other")
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OAuthError::Rejected { status, code } => write!(f, "{code} (HTTP {status})"),
            OAuthError::Http(s) => write!(f, "HTTP {s}"),
            OAuthError::Network(m) => f.write_str(m),
            OAuthError::Malformed(m) => write!(f, "unexpected answer ({m})"),
        }
    }
}

impl OAuthError {
    pub fn is_transient(&self) -> bool {
        match self {
            OAuthError::Network(_) => true,
            OAuthError::Http(s) | OAuthError::Rejected { status: s, .. } => *s >= 500,
            OAuthError::Malformed(_) => false,
        }
    }
}

/// HTTP client settings shared by every call.
#[derive(Debug, Clone, Copy)]
pub struct HttpCfg {
    pub connect_timeout: Duration,
    pub timeout: Duration,
}

impl Default for HttpCfg {
    fn default() -> Self {
        Self { connect_timeout: Duration::from_secs(15), timeout: Duration::from_secs(30) }
    }
}

fn agent_for(url: &str, cfg: HttpCfg) -> ureq::Agent {
    let https = url.starts_with("https://");
    let mut b = ureq::Agent::config_builder()
        .https_only(https)
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(cfg.connect_timeout))
        .timeout_global(Some(cfg.timeout));
    if !https {
        b = b.proxy(None);
    }
    ureq::Agent::new_with_config(b.build())
}

fn net_err(e: &ureq::Error) -> OAuthError {
    let timed_out = matches!(e, ureq::Error::Timeout(_))
        || matches!(e, ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut);
    match e {
        ureq::Error::BodyExceedsLimit(_) => OAuthError::Malformed("oversized"),
        _ if timed_out => OAuthError::Network("timeout"),
        _ => OAuthError::Network("network"),
    }
}

/// POST a form; returns the status and the (bounded) body.
fn post_form(url: &str, pairs: &[(&str, &str)], cfg: HttpCfg) -> Result<(u16, Vec<u8>), OAuthError> {
    let mut body = form_body(pairs);
    let res = agent_for(url, cfg)
        .post(url)
        .header("Accept", "application/json")
        .content_type("application/x-www-form-urlencoded")
        .send(body.as_bytes());
    zeroize::Zeroize::zeroize(&mut body);
    let mut resp = res.map_err(|e| net_err(&e))?;
    let status = resp.status().as_u16();
    let bytes = resp.body_mut().with_config().limit(BODY_LIMIT).read_to_vec().map_err(|e| net_err(&e))?;
    Ok((status, bytes))
}

fn rejected(status: u16, body: &[u8]) -> OAuthError {
    match serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("error").and_then(Value::as_str).map(known_code)) {
        Some(code) => OAuthError::Rejected { status, code },
        None => OAuthError::Http(status),
    }
}

/// A token-endpoint answer.
#[derive(Debug)]
pub struct TokenResponse {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub id_token: Option<Secret>,
    pub token_type: String,
    pub expires_in: i64,
    /// Space-separated granted scopes (`None` when the answer has no `scope`).
    pub scope: Option<String>,
    pub earliest_refresh_at: Option<i64>,
}

fn parse_token_response(body: &[u8]) -> Result<TokenResponse, OAuthError> {
    let v: Value = serde_json::from_slice(body).map_err(|_| OAuthError::Malformed("not JSON"))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(|x| Secret::new(x.to_string()));
    let access_token = s("access_token").ok_or(OAuthError::Malformed("no access_token"))?;
    let token_type = v.get("token_type").and_then(Value::as_str).unwrap_or("Bearer").chars().take(20).collect::<String>();
    if !token_type.eq_ignore_ascii_case("bearer") {
        return Err(OAuthError::Malformed("token_type is not Bearer"));
    }
    Ok(TokenResponse {
        access_token,
        refresh_token: s("refresh_token"),
        id_token: s("id_token"),
        token_type,
        expires_in: v.get("expires_in").and_then(Value::as_i64).filter(|n| *n > 0).unwrap_or(3600),
        scope: v.get("scope").and_then(Value::as_str).map(|x| x.chars().take(1000).collect()),
        earliest_refresh_at: v.get("earliest_refresh_at").and_then(Value::as_i64),
    })
}

fn token_call(ep: &Endpoints, pairs: &[(&str, &str)], cfg: HttpCfg) -> Result<TokenResponse, OAuthError> {
    let (status, body) = post_form(&ep.token, pairs, cfg)?;
    if status != 200 {
        return Err(rejected(status, &body));
    }
    parse_token_response(&body)
}

/// Authorization-code exchange (form-encoded, public client: no secret).
pub fn exchange_code(
    ep: &Endpoints,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    cfg: HttpCfg,
) -> Result<TokenResponse, OAuthError> {
    token_call(
        ep,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("resource", RESOURCE),
        ],
        cfg,
    )
}

/// `grant_type=refresh_token` with the issued client ID; `scope` omitted to keep the grant.
pub fn refresh(ep: &Endpoints, client_id: &str, refresh_token: &str, cfg: HttpCfg) -> Result<TokenResponse, OAuthError> {
    token_call(
        ep,
        &[("grant_type", "refresh_token"), ("client_id", client_id), ("refresh_token", refresh_token), ("resource", RESOURCE)],
        cfg,
    )
}

/// Revoke the renewable session. HTTP 200 (even empty) is success.
pub fn revoke(ep: &Endpoints, client_id: &str, refresh_token: &str, cfg: HttpCfg) -> Result<(), OAuthError> {
    let (status, body) =
        post_form(&ep.revoke, &[("token", refresh_token), ("token_type_hint", "refresh_token"), ("client_id", client_id)], cfg)?;
    if status == 200 {
        Ok(())
    } else {
        Err(rejected(status, &body))
    }
}

/// The issuer's JSON Web Key Set.
pub fn fetch_jwks(ep: &Endpoints, cfg: HttpCfg) -> Result<Value, OAuthError> {
    let mut resp = agent_for(&ep.jwks, cfg).get(&ep.jwks).call().map_err(|e| net_err(&e))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(OAuthError::Http(status));
    }
    let bytes = resp.body_mut().with_config().limit(BODY_LIMIT).read_to_vec().map_err(|e| net_err(&e))?;
    serde_json::from_slice(&bytes).map_err(|_| OAuthError::Malformed("JWKS is not JSON"))
}

/// Whether a space-separated scope string grants ChatGPT plan usage.
pub fn grants_plan(scope: Option<&str>) -> bool {
    scope.is_some_and(|s| s.split_whitespace().any(|x| x == PLAN_SCOPE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_uses_documented_names_and_no_token_hint() {
        let ep = Endpoints::production();
        let r = AuthorizeRequest {
            client_id: DYNAMIC_CLIENT_ID,
            redirect_uri: "http://127.0.0.1:5555/auth/callback",
            state: "st",
            nonce: "no",
            code_challenge: "ch",
            ext_agent_host_id: "urn:uuid:00000000-0000-4000-8000-000000000000",
        };
        let u = authorize_url(&ep, &r);
        assert!(u.starts_with("https://auth.openai.com/api/accounts/authorize?response_type=code&client_id=dynamic_agent_client&"));
        for want in [
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555%2Fauth%2Fcallback",
            "scope=openid%20profile%20email%20offline_access%20resource.invoke%20chatgpt.tokens.use.direct",
            "resource=https%3A%2F%2Fapi.openai.com%2Fv1",
            "code_challenge_method=S256",
            "ext_agent_host_id=urn%3Auuid%3A00000000-0000-4000-8000-000000000000",
            "agent_name_hint=re%3ASearch",
        ] {
            assert!(u.contains(want), "{want} missing in {u}");
        }
        assert!(!u.contains("id_token_hint"));
        let again = authorize_url(&ep, &AuthorizeRequest { client_id: "oaiapp_x", ..r });
        assert!(!again.contains("agent_name_hint"), "hint only on first registration");
    }

    #[test]
    fn mock_endpoints_are_loopback_only() {
        assert!(Endpoints::loopback_mock("http://127.0.0.1:8080").is_ok());
        for bad in ["https://127.0.0.1:1", "http://localhost:1", "http://127.0.0.1:1@evil", "http://10.0.0.1:1", "http://127.0.0.1:"] {
            assert!(Endpoints::loopback_mock(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn oauth_errors_keep_only_known_codes() {
        assert_eq!(rejected(400, br#"{"error":"invalid_grant","error_description":"tok"}"#), OAuthError::Rejected { status: 400, code: "invalid_grant" });
        assert_eq!(rejected(400, br#"{"error":"rt_abc123"}"#), OAuthError::Rejected { status: 400, code: "other" });
        assert_eq!(rejected(502, b"<html>"), OAuthError::Http(502));
        assert!(grants_plan(Some("openid chatgpt.tokens.use.direct")));
        assert!(!grants_plan(Some("openid chatgpt.tokens.use")));
        assert!(!grants_plan(None));
    }
}
