//! Multi-threaded HTTP/1.1 loopback parser, auth dispatcher, and route table.

use crate::assets::{lookup_asset, INDEX_HTML};
use crate::routes_session::{
    handle_bootstrap, handle_create_session, handle_diff, handle_fork, handle_get_session,
    handle_get_settings, handle_health, handle_list_sessions, handle_post_approval,
    handle_post_message, handle_put_settings, handle_sse_stream,
};
use crate::routes_tools::{
    handle_dag, handle_export, handle_get_sidetalk, handle_latex_patch, handle_literature,
    handle_narrate, handle_post_sidetalk, handle_tex_audit, handle_upload, handle_verify,
    handle_workspace, handle_workspace_file,
};
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

struct ParsedRequest {
    method: String,
    path: String,
    raw_query: Option<String>,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl ParsedRequest {
    fn header(&self, name: &str) -> Option<&str> {
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

    let req = match read_http_request(&mut stream) {
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

    let path = req.path.as_str();
    let method = req.method.as_str();

    if method == "GET" && (path == "/" || path == "/index.html") {
        let valid_query = state.auth.validate_query_token(req.raw_query.as_deref());
        let valid_cookie = state.auth.has_valid_cookie(req.header("cookie"));
        if !valid_query && !valid_cookie {
            write_json(&mut stream, 403, json!({ "error": "invalid or missing launch token" }), &[]);
            return;
        }
        let html = state.auth.inject_csrf_meta(INDEX_HTML);
        let set_cookie = state.auth.set_cookie_header();
        write_bytes(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            html.as_bytes(),
            &[("Set-Cookie", set_cookie.as_str()), ("Cache-Control", "no-store")],
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

    if method == "GET" && path == "/api/bootstrap" {
        let ok = state.auth.has_valid_cookie(req.header("cookie"))
            || state
                .auth
                .has_valid_header_token(req.header("x-research-token"), req.header("authorization"));
        if !ok {
            write_json(&mut stream, 401, json!({ "error": "authentication required" }), &[]);
            return;
        }
        write_api_res(&mut stream, handle_bootstrap(&state));
        return;
    }

    if path.starts_with("/api/") {
        if matches!(method, "POST" | "PUT" | "DELETE" | "PATCH") {
            if let Err((st, msg)) = state.auth.authorize_mutation(
                req.header("origin"),
                req.header("x-research-token"),
                req.header("authorization"),
            ) {
                write_json(&mut stream, st, json!({ "error": msg }), &[]);
                return;
            }
        }
        dispatch_api(stream, &state, &shutdown, &req);
        return;
    }

    write_json(&mut stream, 404, json!({ "error": format!("route not found: {method} {path}") }), &[]);
}

fn dispatch_api(
    mut stream: TcpStream,
    state: &Arc<DesktopState>,
    shutdown: &Arc<AtomicBool>,
    req: &ParsedRequest,
) {
    let segs: Vec<&str> = req
        .path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let method = req.method.as_str();

    match (method, segs.as_slice()) {
        ("GET", ["api", "sessions"]) => write_api_res(&mut stream, handle_list_sessions(state)),
        ("POST", ["api", "sessions"]) => {
            write_api_res(&mut stream, handle_create_session(state, &req.body));
        }
        ("GET", ["api", "sessions", id]) => write_api_res(&mut stream, handle_get_session(state, id)),
        ("GET", ["api", "sessions", id, "events"]) => {
            if let Err((st, msg)) = handle_sse_stream(stream, state, id, shutdown) {
                let _ = st;
                let _ = msg;
            }
        }
        ("POST", ["api", "sessions", id, "messages"]) => {
            write_api_res(&mut stream, handle_post_message(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "approvals", call_id]) => {
            write_api_res(&mut stream, handle_post_approval(state, id, call_id, &req.body));
        }
        ("POST", ["api", "sessions", id, "upload"]) => {
            let ct = req.header("content-type");
            write_api_res(&mut stream, handle_upload(state, id, ct, &req.body));
        }
        ("GET", ["api", "sessions", id, "workspace"]) => {
            write_api_res(&mut stream, handle_workspace(state, id, &req.query));
        }
        ("GET", ["api", "sessions", id, "workspace", "file"]) => {
            write_api_res(&mut stream, handle_workspace_file(state, id, &req.query));
        }
        ("POST", ["api", "sessions", id, "latex", "patch"]) => {
            write_api_res(&mut stream, handle_latex_patch(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "tex-audit"]) => {
            write_api_res(&mut stream, handle_tex_audit(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "literature"]) => {
            write_api_res(&mut stream, handle_literature(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "dag"]) => {
            write_api_res(&mut stream, handle_dag(state, id));
        }
        ("POST", ["api", "sessions", id, "verify"]) => {
            write_api_res(&mut stream, handle_verify(state, id));
        }
        ("POST", ["api", "sessions", id, "fork"]) => {
            write_api_res(&mut stream, handle_fork(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "diff", other_id]) => {
            write_api_res(&mut stream, handle_diff(state, id, other_id));
        }
        ("POST", ["api", "sessions", id, "narrate"]) => {
            write_api_res(&mut stream, handle_narrate(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "sidetalk"]) => {
            write_api_res(&mut stream, handle_get_sidetalk(state, id));
        }
        ("POST", ["api", "sessions", id, "sidetalk"]) => {
            write_api_res(&mut stream, handle_post_sidetalk(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "export"]) => match handle_export(state, id, &req.query) {
            Ok((filename, ct, bytes)) => {
                let disp = format!("attachment; filename=\"{filename}\"");
                write_bytes(
                    &mut stream,
                    200,
                    ct,
                    &bytes,
                    &[("Content-Disposition", disp.as_str())],
                );
            }
            Err((st, msg)) => write_json(&mut stream, st, json!({ "error": msg }), &[]),
        },
        ("GET", ["api", "settings", "providers"]) => {
            write_api_res(&mut stream, handle_get_settings(state));
        }
        ("PUT", ["api", "settings", "providers"]) => {
            write_api_res(&mut stream, handle_put_settings(state, &req.body));
        }
        _ => write_json(
            &mut stream,
            404,
            json!({ "error": format!("unknown endpoint: {} {}", req.method, req.path) }),
            &[],
        ),
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<ParsedRequest, (u16, String)> {
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
    if content_length > MAX_BODY_BYTES {
        return Err((413, format!("request body exceeds {MAX_BODY_BYTES} bytes")));
    }

    let mut body = buf[hdr_len..].to_vec();
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

    Ok(ParsedRequest {
        method,
        path,
        raw_query,
        query,
        headers,
        body,
    })
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

fn write_api_res(stream: &mut TcpStream, res: Result<(u16, Value), (u16, String)>) {
    match res {
        Ok((st, val)) => write_json(stream, st, val, &[]),
        Err((st, msg)) => write_json(stream, st, json!({ "error": msg }), &[]),
    }
}

fn write_json(stream: &mut TcpStream, status: u16, val: Value, extra: &[(&str, &str)]) {
    let body = serde_json::to_vec(&val).unwrap_or_else(|_| b"{}".to_vec());
    write_bytes(stream, status, "application/json; charset=utf-8", &body, extra);
}

fn write_bytes(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) {
    let reason = match status {
        200 => "OK",
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
