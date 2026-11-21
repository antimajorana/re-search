//! The one-shot loopback callback listener of a sign-in attempt.
//!
//! Bound to `127.0.0.1` on a port the system picks (the documentation lets only the port
//! vary; the path is fixed: [`crate::oauth::REDIRECT_PATH`]). It accepts only `GET` on that
//! exact path with the exact `Host` and the attempt's `state`; anything else gets a short
//! static error page and the listener keeps waiting (up to [`MAX_BAD_REQUESTS`]). The
//! first request with the right `state` ends the attempt, successful or not, and the
//! listener closes: a replayed callback finds nothing listening. The attempt expires
//! after its timeout. Pages are static text: no code, token or claim is ever echoed.

use crate::codec::{ct_eq, parse_query, Secret};
use crate::manager::ChatGptAuth;
use crate::oauth::{known_code, REDIRECT_PATH};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

pub const MAX_BAD_REQUESTS: u32 = 32;
const MAX_HEAD: usize = 8 * 1024;
const HEAD_DEADLINE: Duration = Duration::from_secs(5);
const READ_SLICE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginState {
    Pending,
    Succeeded { plan_enabled: bool },
    Failed(String),
    Expired,
    Cancelled,
}

impl LoginState {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoginState::Pending => "pending",
            LoginState::Succeeded { .. } => "succeeded",
            LoginState::Failed(_) => "failed",
            LoginState::Expired => "expired",
            LoginState::Cancelled => "cancelled",
        }
    }
}

/// One sign-in attempt as the server and UI see it.
pub struct Attempt {
    pub authorize_url: String,
    pub port: u16,
    pub started: Instant,
    pub timeout: Duration,
    state: Mutex<LoginState>,
    cancel: AtomicBool,
}

impl Attempt {
    pub(crate) fn new(authorize_url: String, port: u16, timeout: Duration) -> Self {
        Self { authorize_url, port, started: Instant::now(), timeout, state: Mutex::new(LoginState::Pending), cancel: AtomicBool::new(false) }
    }

    pub fn state(&self) -> LoginState {
        self.state.lock().map(|s| s.clone()).unwrap_or(LoginState::Failed("internal error".into()))
    }

    pub fn is_pending(&self) -> bool {
        self.state() == LoginState::Pending
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    fn set(&self, s: LoginState) {
        if let Ok(mut g) = self.state.lock() {
            if *g == LoginState::Pending {
                *g = s;
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.set(LoginState::Cancelled);
    }

    pub fn remaining(&self) -> Duration {
        self.timeout.saturating_sub(self.started.elapsed())
    }
}

/// The per-attempt secrets the callback is checked against.
pub(crate) struct Pending {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
    /// The issued client ID when this is a reauthorization, else `None` (new registration).
    pub known_client: Option<String>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.state.zeroize();
        self.nonce.zeroize();
        self.verifier.zeroize();
    }
}

pub(crate) fn spawn(listener: TcpListener, attempt: Arc<Attempt>, pending: Pending, auth: Arc<ChatGptAuth>) -> Result<(), String> {
    listener.set_nonblocking(true).map_err(|_| "could not prepare the sign-in listener".to_string())?;
    std::thread::Builder::new()
        .name("chatgpt-login".into())
        .spawn(move || run(listener, &attempt, &pending, &auth))
        .map(|_| ())
        .map_err(|_| "could not start the sign-in listener".to_string())
}

fn run(listener: TcpListener, attempt: &Attempt, pending: &Pending, auth: &ChatGptAuth) {
    let host = format!("127.0.0.1:{}", attempt.port);
    let mut bad = 0u32;
    loop {
        if attempt.is_cancelled() || !attempt.is_pending() {
            return;
        }
        if attempt.remaining().is_zero() {
            attempt.set(LoginState::Expired);
            return;
        }
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(POLL);
                continue;
            }
            Err(_) => {
                attempt.set(LoginState::Failed("the sign-in listener stopped".into()));
                return;
            }
        };
        match handle(&mut stream, &host, attempt, pending, auth) {
            Handled::Done => return,
            Handled::Rejected => {
                bad += 1;
                if bad >= MAX_BAD_REQUESTS {
                    attempt.set(LoginState::Failed("too many invalid requests reached the sign-in listener".into()));
                    return;
                }
            }
        }
    }
}

enum Handled {
    Done,
    Rejected,
}

fn read_head(stream: &mut TcpStream, attempt: &Attempt) -> Option<Secret> {
    read_head_with_deadline(stream, attempt, HEAD_DEADLINE)
}

fn read_head_with_deadline(stream: &mut TcpStream, attempt: &Attempt, max_wait: Duration) -> Option<Secret> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(HEAD_DEADLINE));
    let deadline = Instant::now() + max_wait;
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            if pos > MAX_HEAD {
                buf.zeroize();
                return None;
            }
            buf[pos..].zeroize();
            buf.truncate(pos);
            return match String::from_utf8(buf) {
                Ok(s) => Some(Secret::new(s)),
                Err(e) => {
                    let mut raw = e.into_bytes();
                    raw.zeroize();
                    None
                }
            };
        }
        if buf.len() > MAX_HEAD || attempt.is_cancelled() {
            buf.zeroize();
            return None;
        }
        let Some(rem) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) else {
            buf.zeroize();
            return None;
        };
        let _ = stream.set_read_timeout(Some(rem.min(READ_SLICE)));
        match stream.read(&mut chunk) {
            Ok(0) => {
                buf.zeroize();
                return None;
            }
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                chunk[..n].zeroize();
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {
                continue;
            }
            Err(_) => {
                buf.zeroize();
                return None;
            }
        }
    }
}

fn unique_param(params: &[(String, String)], key: &str) -> Result<Option<String>, ()> {
    let mut found: Option<&str> = None;
    for (n, v) in params {
        if n == key {
            if found.is_some() {
                return Err(());
            }
            found = Some(v.as_str());
        }
    }
    Ok(found.map(str::to_string))
}

fn handle(stream: &mut TcpStream, host: &str, attempt: &Attempt, pending: &Pending, auth: &ChatGptAuth) -> Handled {
    let Some(head) = read_head(stream, attempt) else {
        page(stream, 400, "Bad request", "This address only accepts the sign-in callback.");
        return Handled::Rejected;
    };
    let mut lines = head.as_str().split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let Some((method, target)) = (match (first.next(), first.next(), first.next(), first.next()) {
        (Some(m), Some(t), Some(v), None) if v == "HTTP/1.1" || v == "HTTP/1.0" => Some((m, t)),
        _ => None,
    }) else {
        page(stream, 400, "Bad request", "This address only accepts the sign-in callback.");
        return Handled::Rejected;
    };
    let mut host_count = 0usize;
    let mut host_matches = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("host") {
                host_count += 1;
                host_matches = v.trim() == host;
            }
        }
    }
    let host_ok = host_count == 1 && host_matches;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if method != "GET" || path != REDIRECT_PATH || !host_ok {
        page(stream, 404, "Not found", "This address only accepts the sign-in callback.");
        return Handled::Rejected;
    }
    let mut params = parse_query(query);
    let extracted = (
        unique_param(&params, "state"),
        unique_param(&params, "code"),
        unique_param(&params, "client_id"),
        unique_param(&params, "error"),
    );
    for (k, v) in &mut params {
        k.zeroize();
        v.zeroize();
    }
    drop(head);
    let (Ok(q_state), Ok(q_code), Ok(q_client_id), Ok(q_error)) = extracted else {
        page(stream, 400, "Bad request", "This address only accepts the sign-in callback.");
        return Handled::Rejected;
    };
    let q_state = q_state.map(Secret::new);
    let q_code = q_code.map(Secret::new);
    let state_ok = q_state.as_ref().is_some_and(|s| ct_eq(s.as_str().as_bytes(), pending.state.as_bytes()));
    if !state_ok {
        page(stream, 400, "Sign-in link not recognized", "This is not the current re:Search sign-in. Start the sign-in again from re:Search.");
        return Handled::Rejected;
    }

    // From here the attempt ends, whatever the outcome.
    let outcome = if let Some(err) = q_error.as_deref() {
        let code = known_code(err);
        if code == "invalid_client" {
            if let Some(known) = &pending.known_client {
                auth.clear_client_registration(Some(known), attempt);
            }
        }
        if code == "access_denied" {
            Err("ChatGPT sign-in was declined (access_denied). ChatGPT plan use stays off; you can sign in again later.".to_string())
        } else {
            Err(format!("ChatGPT sign-in returned an error ({code})."))
        }
    } else {
        complete(auth, attempt, pending, q_code.as_ref().map(Secret::as_str), q_client_id.as_deref())
    };
    match outcome {
        Ok(plan_enabled) => {
            attempt.set(LoginState::Succeeded { plan_enabled });
            let body = if plan_enabled {
                "You're using your ChatGPT plan in re:Search. You can close this tab and return to re:Search."
            } else {
                "Signed in, but ChatGPT plan use was not granted, so re:Search will not use your plan. You can close this tab."
            };
            page(stream, 200, "Signed in to re:Search", body);
        }
        Err(msg) => {
            attempt.set(LoginState::Failed(msg));
            page(stream, 400, "Sign-in did not complete", "re:Search could not complete the sign-in. Return to re:Search for details.");
        }
    }
    Handled::Done
}

fn complete(
    auth: &ChatGptAuth,
    attempt: &Attempt,
    pending: &Pending,
    code: Option<&str>,
    cb_client: Option<&str>,
) -> Result<bool, String> {
    let code = code.filter(|c| !c.is_empty()).ok_or("the sign-in callback carried no authorization code")?;
    let client_id = match (&pending.known_client, cb_client.filter(|c| !c.is_empty())) {
        (Some(known), Some(cb)) if cb != known => {
            return Err("the sign-in callback named a different client than this registration; nothing was changed".into());
        }
        (Some(known), _) => known.clone(),
        (None, Some(cb)) if cb != crate::oauth::DYNAMIC_CLIENT_ID && cb.len() <= 200 && cb.bytes().all(|b| b.is_ascii_graphic()) => {
            cb.to_string()
        }
        (None, _) => return Err("registration is incomplete: the callback did not return an issued client ID".into()),
    };
    auth.finish_login(code, &client_id, pending, attempt)
}

fn page(stream: &mut TcpStream, status: u16, title: &str, text: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><h1>{title}</h1><p>{text}</p></body></html>"
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::Options;

    fn send_raw(port: u16, raw: &[u8]) -> Option<String> {
        let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        s.write_all(raw).ok()?;
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    fn spawn_test_listener() -> (tempfile::TempDir, Arc<Attempt>, u16) {
        let tmp = tempfile::tempdir().unwrap();
        let auth = Arc::new(ChatGptAuth::new(tmp.path(), Options::default()));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let attempt = Arc::new(Attempt::new("https://auth.openai.com/api/accounts/authorize?x=1".into(), port, Duration::from_secs(30)));
        let pending = Pending {
            state: "st-good".into(),
            nonce: "no-good".into(),
            verifier: "ver-good".into(),
            redirect_uri: format!("http://127.0.0.1:{port}{REDIRECT_PATH}"),
            known_client: None,
        };
        spawn(listener, Arc::clone(&attempt), pending, auth).unwrap();
        (tmp, attempt, port)
    }

    #[test]
    fn read_head_enforces_total_deadline_and_cancellation() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let attempt = Attempt::new("u".into(), port, Duration::from_secs(30));

        // Slowloris client trickling 1 byte every 40ms must hit total deadline (~180ms).
        let drip = std::thread::spawn(move || {
            let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
            for _ in 0..15 {
                if c.write_all(b"A").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let (mut srv, _) = listener.accept().unwrap();
        let t0 = Instant::now();
        assert!(read_head_with_deadline(&mut srv, &attempt, Duration::from_millis(180)).is_none());
        assert!(t0.elapsed() < Duration::from_secs(2), "slowloris must abort promptly");
        let _ = drip.join();

        // Cancellation aborts read_head immediately even with a long deadline.
        let holder = std::thread::spawn(move || {
            let _c = TcpStream::connect(("127.0.0.1", port)).unwrap();
            std::thread::sleep(Duration::from_millis(500));
        });
        let (mut srv2, _) = listener.accept().unwrap();
        attempt.cancel();
        let t1 = Instant::now();
        assert!(read_head_with_deadline(&mut srv2, &attempt, Duration::from_secs(10)).is_none());
        assert!(t1.elapsed() < Duration::from_millis(300));
        let _ = holder.join();
    }

    #[test]
    fn rejects_body_host_spoof_duplicate_host_bad_version_and_duplicate_query_params() {
        let (_tmp, attempt, port) = spawn_test_listener();
        let host = format!("127.0.0.1:{port}");

        // Host header only in body must be rejected (404).
        let body_spoof = format!("GET /auth/callback?state=st-good HTTP/1.1\r\nX-A: 1\r\n\r\nHost: {host}\r\n");
        let resp = send_raw(port, body_spoof.as_bytes()).unwrap();
        assert!(resp.starts_with("HTTP/1.1 404"), "{resp}");

        // Duplicate Host header (even both valid or evil+valid) must be rejected (404).
        let dup_host = format!("GET /auth/callback?state=st-good HTTP/1.1\r\nHost: {host}\r\nHost: {host}\r\n\r\n");
        assert!(send_raw(port, dup_host.as_bytes()).unwrap().starts_with("HTTP/1.1 404"));
        let spaced_host = format!("GET /auth/callback?state=st-good HTTP/1.1\r\nHost : {host}\r\n\r\n");
        assert!(send_raw(port, spaced_host.as_bytes()).unwrap().starts_with("HTTP/1.1 404"));

        // Unsupported HTTP version or malformed request line rejected (400).
        let bad_ver = format!("GET /auth/callback?state=st-good HTTP/2.0\r\nHost: {host}\r\n\r\n");
        assert!(send_raw(port, bad_ver.as_bytes()).unwrap().starts_with("HTTP/1.1 400"));

        // Duplicate query parameters for state, code, client_id, error rejected (400).
        for q in [
            "state=st-good&state=st-good",
            "state=st-good&code=a&code=b",
            "state=st-good&client_id=a&client_id=b",
            "state=st-good&error=a&error=b",
        ] {
            let req = format!("GET /auth/callback?{q} HTTP/1.1\r\nHost: {host}\r\n\r\n");
            let r = send_raw(port, req.as_bytes()).unwrap();
            assert!(r.starts_with("HTTP/1.1 400"), "{q}: {r}");
        }

        // Security headers present and no inline style in HTML body.
        assert!(resp.contains("Cache-Control: no-store"));
        assert!(resp.contains("Referrer-Policy: no-referrer"));
        assert!(resp.contains("Content-Security-Policy: default-src 'none'; frame-ancestors 'none'"));
        assert!(resp.contains("X-Content-Type-Options: nosniff"));
        assert!(resp.contains("X-Frame-Options: DENY"));
        assert!(!resp.contains("style="), "no inline style allowed: {resp}");
        assert!(attempt.is_pending());
        attempt.cancel();
    }

    #[test]
    fn closes_after_max_bad_requests() {
        let (_tmp, attempt, port) = spawn_test_listener();
        let req = format!("GET /wrong HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        for _ in 0..MAX_BAD_REQUESTS {
            let r = send_raw(port, req.as_bytes()).unwrap();
            assert!(r.starts_with("HTTP/1.1 404"));
        }
        for _ in 0..20 {
            if !attempt.is_pending() {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        assert!(matches!(attempt.state(), LoginState::Failed(m) if m.contains("too many")));
    }

    #[test]
    fn complete_rejects_dynamic_client_id_and_mismatched_known_client() {
        let tmp = tempfile::tempdir().unwrap();
        let auth = ChatGptAuth::new(tmp.path(), Options::default());
        let attempt = Attempt::new("u".into(), 1234, Duration::from_secs(30));
        let fresh = Pending { state: "s".into(), nonce: "n".into(), verifier: "v".into(), redirect_uri: "r".into(), known_client: None };
        assert!(complete(&auth, &attempt, &fresh, Some("c"), Some(crate::oauth::DYNAMIC_CLIENT_ID)).unwrap_err().contains("issued client ID"));
        assert!(complete(&auth, &attempt, &fresh, None, Some("oaiapp_1")).unwrap_err().contains("no authorization code"));
        let reauth = Pending { state: "s".into(), nonce: "n".into(), verifier: "v".into(), redirect_uri: "r".into(), known_client: Some("oaiapp_1".into()) };
        assert!(complete(&auth, &attempt, &reauth, Some("c"), Some("oaiapp_other")).unwrap_err().contains("different client"));
    }
}
