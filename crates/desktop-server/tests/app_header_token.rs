//! `GET /` as the `re-search-app` window loads it: with the token in the `X-ReSearch-Token`
//! request header instead of the URL.
//! - a valid header token returns the page WITH the token in `<meta>` and sets the cookie;
//! - a wrong header token without a cookie is 403;
//! - the cookie alone still returns the page WITHOUT the token, even next to a wrong header.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use tempfile::TempDir;

fn meta_with(token: &str) -> String {
    format!(r#"<meta name="re-search-csrf" content="{token}">"#)
}

const EMPTY_META: &str = r#"<meta name="re-search-csrf" content="">"#;

fn start() -> (TempDir, desktop_server::RunningDesktopServer) {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(&tmp.path().join("data"), None, None, None)).unwrap();
    (tmp, srv)
}

#[test]
fn header_token_gets_the_page_with_the_token_and_the_stream_cookie() {
    let (_tmp, srv) = start();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    for path in ["/", "/index.html"] {
        let (st, hdrs, body) = raw_http(port, "GET", path, &[("X-ReSearch-Token", &tok)], b"");
        assert_eq!(st, 200, "{path}");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains(&meta_with(&tok)), "{path}: token must be in <meta>");
        let cookie = hdrs.iter().find(|(k, _)| k == "set-cookie").map(|(_, v)| v.as_str()).unwrap_or("");
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"), "{cookie}");
        assert!(!cookie.contains(&tok), "the cookie is a separate secret, not the token");
        assert!(hdrs.iter().any(|(k, v)| k == "cache-control" && v == "no-store"));
        assert!(hdrs.iter().any(|(k, v)| k == "x-frame-options" && v == "DENY"));
    }
    // `Authorization: Bearer` is the same header credential as everywhere else.
    let bearer = format!("Bearer {tok}");
    let (st, _, body) = raw_http(port, "GET", "/", &[("Authorization", &bearer)], b"");
    assert_eq!(st, 200);
    assert!(String::from_utf8_lossy(&body).contains(&meta_with(&tok)));
}

#[test]
fn wrong_header_token_without_cookie_is_refused() {
    let (_tmp, srv) = start();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    let wrong = "0".repeat(tok.len());
    for headers in [
        vec![("X-ReSearch-Token", wrong.as_str())],
        vec![("X-ReSearch-Token", "")],
        vec![("Authorization", "Bearer nope")],
    ] {
        let (st, hdrs, body) = raw_http(port, "GET", "/", &headers, b"");
        assert_eq!(st, 403, "{headers:?}");
        assert!(!String::from_utf8_lossy(&body).contains(&tok));
        assert!(!hdrs.iter().any(|(k, _)| k == "set-cookie"), "no cookie for a refused load");
    }
    let (st, _, _) = raw_http(port, "GET", "/", &[], b"");
    assert_eq!(st, 403, "no credential at all is still 403");
}

#[test]
fn cookie_alone_still_gets_the_page_without_the_token() {
    let (_tmp, srv) = start();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());
    let cookie = launch_cookie(port, &tok);

    let (st, _, body) = raw_http(port, "GET", "/", &[("Cookie", &cookie)], b"");
    assert_eq!(st, 200);
    let html = String::from_utf8_lossy(&body);
    assert!(!html.contains(&tok), "the cookie must never be exchanged for the token");
    assert!(html.contains(EMPTY_META));

    // A wrong header next to a valid cookie: the cookie admits the page, the token stays out.
    let wrong = "f".repeat(tok.len());
    let (st, _, body) = raw_http(port, "GET", "/", &[("Cookie", &cookie), ("X-ReSearch-Token", &wrong)], b"");
    assert_eq!(st, 200);
    assert!(!String::from_utf8_lossy(&body).contains(&tok));
}

#[test]
fn launch_url_uses_nonce_and_header_loads_do_not_consume_or_depend_on_it() {
    let (_tmp, srv) = start();
    let (port, tok) = (srv.addr.port(), srv.info.token.clone());

    assert!(srv.info.url.contains("?launch="), "{}", srv.info.url);
    assert!(!srv.info.url.contains(&tok), "launch URL must not expose the API token");
    let launch_path = srv.info.url.strip_prefix(&srv.info.base_url).unwrap().to_string();

    // Repeated header-token loads (including one carrying ?launch=<nonce>) work without
    // consuming the single-use launch nonce.
    let bearer = format!("Bearer {tok}");
    for (path, hdr) in [
        ("/", ("X-ReSearch-Token", tok.as_str())),
        ("/index.html", ("X-ReSearch-Token", tok.as_str())),
        (launch_path.as_str(), ("Authorization", bearer.as_str())),
    ] {
        let (st, _, body) = raw_http(port, "GET", path, &[hdr], b"");
        assert_eq!(st, 200, "{path}");
        assert!(String::from_utf8_lossy(&body).contains(&meta_with(&tok)));
    }

    // The launch nonce is still unconsumed and redeems once; replay is 403.
    let (st, _, body) = raw_http(port, "GET", &launch_path, &[], b"");
    assert_eq!(st, 200);
    assert!(String::from_utf8_lossy(&body).contains(&meta_with(&tok)));
    assert_eq!(raw_http(port, "GET", &launch_path, &[], b"").0, 403);

    // Header-token loads (`re-search-app`) continue working after the launch nonce is spent.
    let (st, _, body) = raw_http(port, "GET", "/", &[("X-ReSearch-Token", &tok)], b"");
    assert_eq!(st, 200);
    assert!(String::from_utf8_lossy(&body).contains(&meta_with(&tok)));
}
