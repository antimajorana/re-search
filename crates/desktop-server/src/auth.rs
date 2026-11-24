//! Per-launch 256-bit loopback credentials, `Host`/`Origin` validation, and a separate
//! `HttpOnly; SameSite=Strict` stream cookie.
//!
//! Three independent 256-bit secrets are issued per launch:
//! - the **token**, sent by the UI as `X-ReSearch-Token` (or `Authorization: Bearer`). It
//!   authorizes every `/api/` route and is the only credential accepted for mutations. It is
//!   never accepted as a URL query parameter on any route.
//! - the **stream cookie**, for the two things a browser cannot attach a header to: the
//!   `EventSource` stream and export downloads. It is never valid as the token, and the
//!   page is not re-issued the token for it. Browsers send a `127.0.0.1` cookie to every
//!   port on that host, so a cookie captured by another local service must not be enough
//!   to mutate anything or to list sessions.
//! - the **launch nonce**, a single-use, short-lived (`LAUNCH_NONCE_TTL`) secret placed in
//!   the `re-search ui` browser launch URL (`/?launch=<nonce>`). It is accepted at most once
//!   on `GET /` or `GET /index.html` to deliver the token in `<meta>` and set the stream
//!   cookie, and is never accepted on `/api/` routes.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const COOKIE_PREFIX: &str = "__research_stream_";
pub const LAUNCH_NONCE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct AuthGuard {
    token: String,
    cookie_secret: String,
    launch_nonce: Arc<Mutex<Option<(String, Instant)>>>,
    port: u16,
}

fn random_hex_256() -> Result<String, String> {
    let mut raw = [0u8; 32];
    getrandom::getrandom(&mut raw).map_err(|e| format!("CSPRNG failed: {e}"))?;
    let mut out = String::with_capacity(64);
    for b in raw {
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

fn extract_launch_nonce(query: Option<&str>) -> Option<&str> {
    let q = query?;
    let mut found: Option<&str> = None;
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (pair, None),
        };
        if k == "launch" {
            if found.is_some() {
                return None;
            }
            let val = v?.trim();
            if val.is_empty() {
                return None;
            }
            found = Some(val);
        }
    }
    found
}

impl AuthGuard {
    pub fn new(port: u16) -> Result<Self, String> {
        Self::new_with_nonce_ttl(port, LAUNCH_NONCE_TTL)
    }

    pub fn new_with_nonce_ttl(port: u16, ttl: Duration) -> Result<Self, String> {
        let token = random_hex_256()?;
        let cookie_secret = random_hex_256()?;
        let nonce = random_hex_256()?;
        let expires_at = Instant::now() + ttl;
        Ok(Self {
            token,
            cookie_secret,
            launch_nonce: Arc::new(Mutex::new(Some((nonce, expires_at)))),
            port,
        })
    }

    /// Cookie name, unique per port so two running instances do not overwrite each other.
    pub fn cookie_name(&self) -> String {
        format!("{COOKIE_PREFIX}{}", self.port)
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Returns the current unconsumed launch nonce, if still present.
    pub fn launch_nonce(&self) -> Option<String> {
        let slot = self.launch_nonce.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_ref().map(|(nonce, _)| nonce.clone())
    }

    /// Validates and atomically consumes a single-use `?launch=<nonce>` query parameter.
    ///
    /// Rejects missing, empty, or duplicate `launch` parameters, expired nonces (clearing the
    /// slot), and mismatches (without consuming a valid pending nonce).
    pub fn consume_launch_nonce(&self, query: Option<&str>) -> bool {
        let Some(candidate) = extract_launch_nonce(query) else {
            return false;
        };
        let mut slot = self.launch_nonce.lock().unwrap_or_else(|e| e.into_inner());
        let Some((expected, expires_at)) = slot.as_ref() else {
            return false;
        };
        if Instant::now() > *expires_at {
            *slot = None;
            return false;
        }
        if ct_eq(candidate, expected) {
            *slot = None;
            return true;
        }
        false
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set_cookie_header(&self) -> String {
        format!("{}={}; HttpOnly; SameSite=Strict; Path=/", self.cookie_name(), self.cookie_secret)
    }

    pub fn inject_csrf_meta(&self, html: &str) -> String {
        let placeholder = r#"<meta name="re-search-csrf" content="">"#;
        let replacement = format!(r#"<meta name="re-search-csrf" content="{}">"#, self.token);
        if html.contains(placeholder) {
            html.replacen(placeholder, &replacement, 1)
        } else {
            html.replacen("</head>", &format!("  {replacement}\n</head>"), 1)
        }
    }

    pub fn validate_host(&self, host_hdr: Option<&str>) -> Result<(), (u16, &'static str)> {
        let Some(host) = host_hdr.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err((403, "missing Host header"));
        };
        let exp_ip = format!("127.0.0.1:{}", self.port);
        let exp_lh = format!("localhost:{}", self.port);
        if host.eq_ignore_ascii_case(&exp_ip)
            || host.eq_ignore_ascii_case(&exp_lh)
            || host == "127.0.0.1"
            || host.eq_ignore_ascii_case("localhost")
        {
            Ok(())
        } else {
            Err((403, "forbidden Host header"))
        }
    }

    pub fn validate_origin(&self, origin_hdr: Option<&str>) -> Result<(), (u16, &'static str)> {
        let Some(origin) = origin_hdr.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(());
        };
        let exp_ip = format!("http://127.0.0.1:{}", self.port);
        let exp_lh = format!("http://localhost:{}", self.port);
        if origin.eq_ignore_ascii_case(&exp_ip) || origin.eq_ignore_ascii_case(&exp_lh) {
            Ok(())
        } else {
            Err((403, "forbidden Origin header"))
        }
    }

    pub fn has_valid_cookie(&self, cookie_hdr: Option<&str>) -> bool {
        let Some(raw) = cookie_hdr else {
            return false;
        };
        let name = self.cookie_name();
        for pair in raw.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                if k.trim() == name && ct_eq(v.trim(), &self.cookie_secret) {
                    return true;
                }
            }
        }
        false
    }

    pub fn has_valid_header_token(&self, token_hdr: Option<&str>, auth_hdr: Option<&str>) -> bool {
        if let Some(tok) = token_hdr.map(str::trim).filter(|s| !s.is_empty()) {
            if ct_eq(tok, &self.token) {
                return true;
            }
        }
        if let Some(auth) = auth_hdr.map(str::trim) {
            if let Some(bearer) = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer ")) {
                if ct_eq(bearer.trim(), &self.token) {
                    return true;
                }
            }
        }
        false
    }

    /// The stream cookie can open session events and session or project downloads.
    fn is_cookie_route(path: &str) -> bool {
        let segs: Vec<&str> = path.split('/').collect();
        matches!(
            segs.as_slice(),
            ["", "api", "sessions", id, "events" | "export"] | ["", "api", "projects", id, "export"]
                if !id.is_empty()
        )
    }

    /// The whole credential rule for `/api/` routes other than `/api/health`.
    ///
    /// - A valid header token authorizes any route; a non-GET/HEAD request must also carry a
    ///   loopback `Origin` when it carries one at all.
    /// - Without the token, only a `GET` of a cookie route with the valid stream cookie is
    ///   accepted, and only if the browser does not mark it as coming from another origin
    ///   (`Sec-Fetch-Site`, when present, must be `same-origin`; `Origin`, when present, ours).
    pub fn authorize_api(&self, req: &ApiCredentials<'_>) -> Result<(), (u16, &'static str)> {
        if self.has_valid_header_token(req.token_hdr, req.auth_hdr) {
            if !matches!(req.method, "GET" | "HEAD") {
                self.validate_origin(req.origin)?;
            }
            return Ok(());
        }
        let same_origin = req.sec_fetch_site.map(str::trim).is_none_or(|v| v.eq_ignore_ascii_case("same-origin"));
        if req.method == "GET"
            && Self::is_cookie_route(req.path)
            && self.has_valid_cookie(req.cookie)
            && same_origin
            && self.validate_origin(req.origin).is_ok()
        {
            return Ok(());
        }
        Err((401, "authentication required: missing or invalid X-ReSearch-Token"))
    }
}

/// What `authorize_api` looks at, taken from one request.
#[derive(Debug, Clone, Copy)]
pub struct ApiCredentials<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub cookie: Option<&'a str>,
    pub token_hdr: Option<&'a str>,
    pub auth_hdr: Option<&'a str>,
    pub origin: Option<&'a str>,
    pub sec_fetch_site: Option<&'a str>,
}

fn ct_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    if ab.len() != bb.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in ab.iter().zip(bb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_nonce_is_independent_of_token_and_cookie_secret() {
        let auth = AuthGuard::new(4123).unwrap();
        let nonce = auth.launch_nonce().expect("launch nonce present");
        assert_eq!(nonce.len(), 64);
        assert_eq!(auth.token().len(), 64);
        assert_eq!(auth.cookie_secret.len(), 64);
        assert_ne!(nonce, auth.token());
        assert_ne!(nonce, auth.cookie_secret);
        assert_ne!(auth.token(), auth.cookie_secret);
        assert!(!auth.has_valid_header_token(Some(&nonce), None));
        assert!(!auth.has_valid_cookie(Some(&format!("{}={nonce}", auth.cookie_name()))));
    }

    #[test]
    fn valid_nonce_succeeds_once_and_replay_fails_across_clones() {
        let auth = AuthGuard::new(4123).unwrap();
        let cloned = auth.clone();
        let nonce = auth.launch_nonce().unwrap();
        let query = format!("launch={nonce}");

        assert!(auth.consume_launch_nonce(Some(&query)));
        assert!(auth.launch_nonce().is_none());
        assert!(cloned.launch_nonce().is_none());
        assert!(!auth.consume_launch_nonce(Some(&query)));
        assert!(!cloned.consume_launch_nonce(Some(&query)));
    }

    #[test]
    fn wrong_nonce_fails_without_consuming_valid_nonce() {
        let auth = AuthGuard::new(4123).unwrap();
        let nonce = auth.launch_nonce().unwrap();
        let wrong = "0".repeat(nonce.len());

        assert!(!auth.consume_launch_nonce(None));
        assert!(!auth.consume_launch_nonce(Some("")));
        assert!(!auth.consume_launch_nonce(Some("launch=")));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch={wrong}"))));
        assert!(!auth.consume_launch_nonce(Some(&format!("token={}", auth.token()))));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch={}", auth.token()))));

        // The legitimate nonce must still be intact and redeemable once.
        assert_eq!(auth.launch_nonce().as_deref(), Some(nonce.as_str()));
        assert!(auth.consume_launch_nonce(Some(&format!("foo=1&launch={nonce}&bar=2"))));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch={nonce}"))));
    }

    #[test]
    fn expired_nonce_fails_and_is_cleared() {
        let auth = AuthGuard::new_with_nonce_ttl(4123, Duration::from_millis(5)).unwrap();
        let nonce = auth.launch_nonce().unwrap();
        std::thread::sleep(Duration::from_millis(15));

        assert!(!auth.consume_launch_nonce(Some(&format!("launch={nonce}"))));
        assert!(auth.launch_nonce().is_none());
    }

    #[test]
    fn duplicate_launch_params_fail_without_consuming() {
        let auth = AuthGuard::new(4123).unwrap();
        let nonce = auth.launch_nonce().unwrap();

        assert!(!auth.consume_launch_nonce(Some(&format!("launch={nonce}&launch={nonce}"))));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch=other&launch={nonce}"))));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch={nonce}&launch="))));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch={nonce}&launch"))));
        assert!(!auth.consume_launch_nonce(Some(&format!("launch&launch={nonce}"))));

        // Still unconsumed after rejected duplicate attempts.
        assert!(auth.consume_launch_nonce(Some(&format!("launch={nonce}"))));
    }
}
