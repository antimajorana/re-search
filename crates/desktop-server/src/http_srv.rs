//! Multi-threaded HTTP/1.1 loopback parser, auth dispatcher, and route table.

use crate::api_dispatch::dispatch_api;
use crate::assets::{lookup_asset, INDEX_HTML};
use crate::auth::ApiCredentials;
use crate::routes_session::handle_health;
use crate::routes_tools::MAX_UPLOAD_BODY_BYTES;
use crate::state::DesktopState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 60 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_POLL_MS: Duration = Duration::from_millis(100);

pub(crate) struct ParsedRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) raw_query: Option<String>,
    pub(crate) query: HashMap<String, String>,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
}

impl ParsedRequest {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

pub fn serve_loop(
    listener: TcpListener,
    state: Arc<DesktopState>,
    shutdown: Arc<AtomicBool>,
) {
    let _ = listener.set_nonblocking(true);
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                let st = Arc::clone(&state);
                let sd = Arc::clone(&shutdown);
                let _ = std::thread::Builder::new()
                    .name("desktop-http".to_string())
                    .spawn(move || handle_connection(stream, st, sd));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL_MS);
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: Arc<DesktopState>, shutdown: Arc<AtomicBool>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));

    // Read the body only after the Host and credential checks.
    let (mut req, content_length) = match read_http_head(&mut stream) {
        Ok(r) => r,
        Err((status, msg)) => {
            write_json(&mut stream, status, json!({ "error": msg }), &[]);
            return;
        }
    };

    if let Err((st, msg)) = state.auth.validate_host(req.header("host")) {
        write_json(&mut stream, st, json!({ "error": msg }), &[]);
        return;
    }

    let path = req.path.clone();
    let path = path.as_str();
    let method = req.method.clone();
    let method = method.as_str();

    if method == "GET" && (path == "/" || path == "/index.html") {
        // `re-search-app` navigates its window here with the token in a request header, so
        // the token never appears in a URL or on a command line.
        let valid_header =
            state.auth.has_valid_header_token(req.header("x-research-token"), req.header("authorization"));
        // `re-search ui` opens the default browser with a single-use, short-lived `?launch=<nonce>`
        // URL. Only attempt to consume it when no valid header token was provided.
        let valid_nonce = !valid_header && state.auth.consume_launch_nonce(req.raw_query.as_deref());
        let valid_cookie = state.auth.has_valid_cookie(req.header("cookie"));
        if !valid_header && !valid_nonce && !valid_cookie {
            write_json(
                &mut stream,
                403,
                json!({ "error": "invalid, expired, or missing launch credential" }),
                &[],
            );
            return;
        }
        // The token is written into the page only for a request authenticated by header token
        // or redeeming the single-use launch nonce. A reload carries just the stream cookie and
        // gets the page without the token (the UI keeps it in per-origin sessionStorage), so
        // the cookie can never be traded for it.
        let html = if valid_header || valid_nonce {
            state.auth.inject_csrf_meta(INDEX_HTML)
        } else {
            INDEX_HTML.to_string()
        };
        let set_cookie = state.auth.set_cookie_header();
        write_bytes(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            html.as_bytes(),
            &[
                ("Set-Cookie", set_cookie.as_str()),
                ("Cache-Control", "no-store"),
                ("Referrer-Policy", "no-referrer"),
                // Never render inside another page's frame (a page on another local port is same-site).
                ("X-Frame-Options", "DENY"),
                // No framing by others, and no frames, plugins or <base> inside the page: the app
                // window's navigation rule only sees top-level navigations.
                ("Content-Security-Policy", "frame-ancestors 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'"),
            ],
        );
        return;
    }

    if method == "GET" && (path.starts_with("/assets/") || path.starts_with("/vendor/")) {
        if let Some(asset) = lookup_asset(path) {
            write_bytes(
                &mut stream,
                200,
                asset.content_type,
                asset.bytes,
                &[("Cache-Control", "public, max-age=3600")],
            );
        } else {
            write_json(&mut stream, 404, json!({ "error": "asset not found" }), &[]);
        }
        return;
    }

    if method == "GET" && path == "/api/health" {
        write_api_res(&mut stream, handle_health());
        return;
    }

    if path.starts_with("/api/") {
        // Every API route except `/api/health` needs the launch token in a header; the stream
        // cookie opens only the SSE stream and export downloads (see `AuthGuard::authorize_api`).
        let creds = ApiCredentials {
            method,
            path,
            cookie: req.header("cookie"),
            token_hdr: req.header("x-research-token"),
            auth_hdr: req.header("authorization"),
            origin: req.header("origin"),
            sec_fetch_site: req.header("sec-fetch-site"),
        };
        if let Err((st, msg)) = state.auth.authorize_api(&creds) {
            write_json(&mut stream, st, json!({ "error": msg }), &[]);
            drain_refused(&mut stream);
            return;
        }
        if let Err((st, msg)) = read_http_body(&mut stream, &mut req, content_length) {
            write_json(&mut stream, st, json!({ "error": msg }), &[]);
            return;
        }
        dispatch_api(stream, &state, &shutdown, &req);
        return;
    }

    write_json(&mut stream, 404, json!({ "error": format!("route not found: {method} {path}") }), &[]);
}

/// Reads and parses the request line and headers. `body` holds whatever bytes of the body
/// arrived with them; `read_http_body` fetches the rest once the request is authorized.
fn read_http_head(stream: &mut TcpStream) -> Result<(ParsedRequest, usize), (u16, String)> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() >= MAX_HEADER_BYTES {
            return Err((431, "request headers too large".to_string()));
        }
        let n = stream.read(&mut chunk).map_err(|e| (400, format!("read failed: {e}")))?;
        if n == 0 {
            return Err((400, "unexpected EOF before HTTP headers".to_string()));
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let mut parsed_headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut parsed_headers);
    let hdr_len = match req.parse(&buf[..header_end + 4]) {
        Ok(httparse::Status::Complete(len)) => len,
        Ok(httparse::Status::Partial) => return Err((400, "incomplete HTTP request headers".to_string())),
        Err(e) => return Err((400, format!("malformed HTTP request: {e}"))),
    };

    let method = req.method.unwrap_or("GET").to_ascii_uppercase();
    let raw_target = req.path.unwrap_or("/");
    let (path, raw_query, query) = split_target_query(raw_target);

    let mut headers = HashMap::new();
    for h in req.headers.iter() {
        let name = h.name.to_ascii_lowercase();
        let val = String::from_utf8_lossy(h.value).trim().to_string();
        headers.entry(name).and_modify(|e: &mut String| {
            e.push_str("; ");
            e.push_str(&val);
        }).or_insert(val);
    }

    let content_length: usize = match headers.get("content-length") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| (400, "invalid Content-Length".to_string()))?,
        None => 0,
    };
    let is_upload = method == "POST"
        && matches!(
            path.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>().as_slice(),
            ["api", "sessions", _, "upload"]
        );
    let max_body_bytes = if is_upload { MAX_UPLOAD_BODY_BYTES } else { MAX_BODY_BYTES };
    if content_length > max_body_bytes {
        return Err((413, format!("request body exceeds {max_body_bytes} bytes")));
    }

    let mut body = buf[hdr_len..].to_vec();
    body.truncate(content_length);
    Ok((
        ParsedRequest {
            method,
            path,
            raw_query,
            query,
            headers,
            body,
        },
        content_length,
    ))
}

fn read_http_body(
    stream: &mut TcpStream,
    req: &mut ParsedRequest,
    content_length: usize,
) -> Result<(), (u16, String)> {
    let body = &mut req.body;
    while body.len() < content_length {
        let need = (content_length - body.len()).min(65_536);
        let mut tmp = vec![0u8; need];
        let n = stream.read(&mut tmp).map_err(|e| (400, format!("body read failed: {e}")))?;
        if n == 0 {
            return Err((400, "unexpected EOF while reading body".to_string()));
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);
    Ok(())
}

/// After refusing a request whose body was not read: stop sending, then discard what the
/// client is still writing so it can read the refusal instead of seeing a connection reset.
/// Bounded in bytes (256 KiB) and in total time (3 s), so an unauthenticated peer cannot hold
/// the thread by trickling; a larger refused upload ends in a reset.
fn drain_refused(stream: &mut TcpStream) {
    const BUDGET: Duration = Duration::from_secs(3);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let started = std::time::Instant::now();
    let mut sink = [0u8; 16 * 1024];
    let mut left = 256 * 1024usize;
    while left > 0 {
        let Some(remaining) = BUDGET.checked_sub(started.elapsed()).filter(|d| !d.is_zero()) else {
            break;
        };
        let _ = stream.set_read_timeout(Some(remaining));
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => left = left.saturating_sub(n),
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn split_target_query(raw: &str) -> (String, Option<String>, HashMap<String, String>) {
    let mut query = HashMap::new();
    let (path_raw, qs_opt) = match raw.split_once('?') {
        Some((p, q)) => (p, Some(q.to_string())),
        None => (raw, None),
    };
    if let Some(qs) = &qs_opt {
        for pair in qs.split('&').filter(|s| !s.is_empty()) {
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (percent_decode(k), percent_decode(v)),
                None => (percent_decode(pair), String::new()),
            };
            query.insert(k, v);
        }
    }
    (percent_decode(path_raw), qs_opt, query)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub(crate) fn write_api_res(stream: &mut TcpStream, res: Result<(u16, Value), (u16, String)>) {
    match res {
        Ok((st, val)) => write_json(stream, st, val, &[]),
        Err((st, msg)) => write_json(stream, st, json!({ "error": msg }), &[]),
    }
}

pub(crate) fn write_json(stream: &mut TcpStream, status: u16, val: Value, extra: &[(&str, &str)]) {
    let body = serde_json::to_vec(&val).unwrap_or_else(|_| b"{}".to_vec());
    write_bytes(stream, status, "application/json; charset=utf-8", &body, extra);
}

pub(crate) fn write_bytes(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
