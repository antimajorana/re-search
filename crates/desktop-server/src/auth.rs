//! Per-launch 256-bit loopback token, `Host`/`Origin` validation,
//! `HttpOnly; SameSite=Strict` cookie, and CSRF token verification.

use std::fmt::Write as _;

pub const COOKIE_NAME: &str = "__research_token";

#[derive(Debug, Clone)]
pub struct AuthGuard {
    token: String,
    port: u16,
}

impl AuthGuard {
    pub fn new(port: u16) -> Result<Self, String> {
        let mut raw = [0u8; 32];
        getrandom::getrandom(&mut raw).map_err(|e| format!("CSPRNG failed: {e}"))?;
        let mut token = String::with_capacity(64);
        for b in raw {
            let _ = write!(token, "{b:02x}");
        }
        Ok(Self { token, port })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set_cookie_header(&self) -> String {
        format!("{COOKIE_NAME}={}; HttpOnly; SameSite=Strict; Path=/", self.token)
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
        for pair in raw.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                if k.trim() == COOKIE_NAME && ct_eq(v.trim(), &self.token) {
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

    pub fn authorize_mutation(
        &self,
        origin_hdr: Option<&str>,
        token_hdr: Option<&str>,
        auth_hdr: Option<&str>,
    ) -> Result<(), (u16, &'static str)> {
        self.validate_origin(origin_hdr)?;
        if !self.has_valid_header_token(token_hdr, auth_hdr) {
            return Err((401, "missing or invalid X-ReSearch-Token"));
        }
        Ok(())
    }
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
