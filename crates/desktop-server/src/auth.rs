//! Per-launch 256-bit loopback token, `Host`/`Origin` validation, and a separate
//! `HttpOnly; SameSite=Strict` stream cookie.
//!
//! Two independent secrets are issued per launch:
//! - the **token**, sent by the UI as `X-ReSearch-Token` (or `Authorization: Bearer`). It
//!   authorizes every `/api/` route and is the only credential accepted for mutations.
//! - the **stream cookie**, for the two things a browser cannot attach a header to: the
//!   `EventSource` stream and export downloads. It is never valid as the token, and the
//!   page is not re-issued the token for it. Browsers send a `127.0.0.1` cookie to every
//!   port on that host, so a cookie captured by another local service must not be enough
//!   to mutate anything or to list sessions.

use std::fmt::Write as _;

pub const COOKIE_PREFIX: &str = "__research_stream_";

#[derive(Debug, Clone)]
pub struct AuthGuard {
    token: String,
    cookie_secret: String,
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

impl AuthGuard {
    pub fn new(port: u16) -> Result<Self, String> {
        Ok(Self { token: random_hex_256()?, cookie_secret: random_hex_256()?, port })
    }

    /// Cookie name, unique per port so two running instances do not overwrite each other.
    pub fn cookie_name(&self) -> String {
        format!("{COOKIE_PREFIX}{}", self.port)
    }

    pub fn token(&self) -> &str {
        &self.token
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

    pub fn validate_query_token(&self, query: Option<&str>) -> bool {
        let Some(q) = query else {
            return false;
        };
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "token" && ct_eq(v.trim(), &self.token) {
                    return true;
                }
            }
        }
        false
    }

    /// `GET /api/sessions/{id}/events` and `.../export`: the only routes the cookie opens.
    fn is_cookie_route(path: &str) -> bool {
        let segs: Vec<&str> = path.split('/').collect();
        matches!(segs.as_slice(), ["", "api", "sessions", id, "events" | "export"] if !id.is_empty())
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
