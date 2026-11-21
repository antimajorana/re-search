//! The one-shot loopback callback listener of a sign-in attempt.
//!
//! Bound to `127.0.0.1` on a port the system picks (the documentation lets only the port
//! vary; the path is fixed: [`crate::oauth::REDIRECT_PATH`]). It accepts only `GET` on that
//! exact path with the exact `Host` and the attempt's `state`; anything else gets a short
//! static error page and the listener keeps waiting (up to [`MAX_BAD_REQUESTS`]). The
//! first request with the right `state` ends the attempt, successful or not, and the
//! listener closes: a replayed callback finds nothing listening. The attempt expires
//! after its timeout. Pages are static text: no code, token or claim is ever echoed.

use crate::codec::{ct_eq, parse_query};
use crate::manager::ChatGptAuth;
use crate::oauth::{known_code, REDIRECT_PATH};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_BAD_REQUESTS: u32 = 32;
const MAX_HEAD: usize = 8 * 1024;
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
        if attempt.cancel.load(Ordering::Acquire) || !attempt.is_pending() {
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

fn read_head(stream: &mut TcpStream) -> Option<String> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        if buf.len() > MAX_HEAD {
            return None;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(buf).ok()
}

fn handle(stream: &mut TcpStream, host: &str, attempt: &Attempt, pending: &Pending, auth: &ChatGptAuth) -> Handled {
    let Some(head) = read_head(stream) else {
        page(stream, 400, "Bad request", "This address only accepts the sign-in callback.");
        return Handled::Rejected;
    };
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let host_ok = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("host"))
        .is_some_and(|(_, v)| v.trim() == host);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if method != "GET" || path != REDIRECT_PATH || !host_ok {
        page(stream, 404, "Not found", "This address only accepts the sign-in callback.");
        return Handled::Rejected;
    }
    let params = parse_query(query);
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let state_ok = get("state").is_some_and(|s| ct_eq(s.as_bytes(), pending.state.as_bytes()));
    if !state_ok {
        page(stream, 400, "Sign-in link not recognized", "This is not the current re:Search sign-in. Start the sign-in again from re:Search.");
        return Handled::Rejected;
    }

    // From here the attempt ends, whatever the outcome.
    let outcome = if let Some(err) = get("error") {
        let code = known_code(err);
        if code == "access_denied" {
            Err("ChatGPT sign-in was declined (access_denied). ChatGPT plan use stays off; you can sign in again later.".to_string())
        } else {
            Err(format!("ChatGPT sign-in returned an error ({code})."))
        }
    } else {
        complete(auth, pending, get("code"), get("client_id"))
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

fn complete(auth: &ChatGptAuth, pending: &Pending, code: Option<&str>, cb_client: Option<&str>) -> Result<bool, String> {
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
    auth.finish_login(code, &client_id, pending)
}

fn page(stream: &mut TcpStream, status: u16, title: &str, text: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body style=\"font-family:sans-serif;max-width:36em;margin:4em auto\"><h1>{title}</h1><p>{text}</p></body></html>"
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}
