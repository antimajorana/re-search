//! [`ChatGptAuth`]: sign-in attempts, status, the current access token (refreshed under a
//! mutex, rotated refresh tokens persisted atomically), and sign-out with revocation.

use crate::codec::{new_host_id, now_s, pkce_challenge, random_token, Secret};
use crate::jwt::verify_id_token;
use crate::login::{self, Attempt, LoginState, Pending};
use crate::oauth::{self, AuthorizeRequest, Endpoints, HttpCfg, OAuthError, DYNAMIC_CLIENT_ID, REDIRECT_PATH, TERMINAL_REFRESH_CODES};
use crate::vault::{self, Credentials, Registration};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Refresh when the access token has less than this many seconds left.
pub const REFRESH_MARGIN_S: i64 = 300;
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct Options {
    pub endpoints: Endpoints,
    pub http: HttpCfg,
    pub login_timeout: Duration,
    /// Backoff between revocation retries on sign-out.
    pub revoke_backoff: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self { endpoints: Endpoints::production(), http: HttpCfg::default(), login_timeout: LOGIN_TIMEOUT, revoke_backoff: Duration::from_millis(500) }
    }
}

/// Status for the Connect panel. `account` is the verified email when the ID token had
/// one (shown so the user sees which ChatGPT account is active), never a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthStatus {
    pub supported: bool,
    pub signed_in: bool,
    pub plan_enabled: bool,
    pub account: Option<String>,
    pub message: String,
}

/// Why no access token is available. Messages carry no token material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    NotSupported,
    NotSignedIn,
    PlanNotEnabled,
    /// The saved sign-in can no longer be used (tokens deleted); sign in again.
    Disconnected(String),
    /// Network or server trouble; the sign-in is kept.
    Temporary(String),
    Storage(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::NotSupported => f.write_str(vault::NOT_SUPPORTED),
            TokenError::NotSignedIn => f.write_str("not signed in with ChatGPT (Connect > ChatGPT > Continue with ChatGPT)"),
            TokenError::PlanNotEnabled => f.write_str("ChatGPT plan use was not granted at sign-in; sign in again and allow it"),
            TokenError::Disconnected(m) | TokenError::Temporary(m) | TokenError::Storage(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for TokenError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginStart {
    pub authorize_url: String,
    pub port: u16,
    pub expires_in_s: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoutOutcome {
    /// `Some(true)`: revocation confirmed; `Some(false)`: not confirmed; `None`: nothing to revoke.
    pub revoked: Option<bool>,
    pub message: String,
}

pub struct ChatGptAuth {
    data_dir: PathBuf,
    opts: Options,
    /// Serializes refresh, sign-in completion and sign-out (rotating refresh tokens).
    lock: Mutex<()>,
    note: Mutex<Option<String>>,
    attempt: Mutex<Option<Arc<Attempt>>>,
}

impl ChatGptAuth {
    pub fn new(data_dir: impl Into<PathBuf>, opts: Options) -> Self {
        Self { data_dir: data_dir.into(), opts, lock: Mutex::new(()), note: Mutex::new(None), attempt: Mutex::new(None) }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.opts.endpoints
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn set_note(&self, n: Option<String>) {
        if let Ok(mut g) = self.note.lock() {
            *g = n;
        }
    }

    fn note(&self) -> Option<String> {
        self.note.lock().ok().and_then(|g| g.clone())
    }

    pub fn status(&self) -> AuthStatus {
        let off = |message: String| AuthStatus { supported: vault::supported(), signed_in: false, plan_enabled: false, account: None, message };
        if !vault::supported() {
            return off(vault::NOT_SUPPORTED.to_string());
        }
        match vault::load_credentials(&self.data_dir) {
            Ok(Some(c)) => {
                let who = c.email.clone();
                let plan = c.plan_enabled();
                let message = match (plan, &who) {
                    (true, Some(e)) => format!("Signed in with ChatGPT as {e}; requests use your ChatGPT plan (OpenAI preview)."),
                    (true, None) => "Signed in with ChatGPT; requests use your ChatGPT plan (OpenAI preview).".to_string(),
                    (false, _) => "Signed in, but ChatGPT plan use was not granted; sign in again and allow it.".to_string(),
                };
                AuthStatus { supported: true, signed_in: true, plan_enabled: plan, account: who, message }
            }
            Ok(None) => off(self.note().unwrap_or_else(|| "Not signed in.".to_string())),
            Err(e) => off(format!("The saved sign-in cannot be used: {e}. Sign in again.")),
        }
    }

    /// A current access token, refreshed first when it expires within
    /// [`REFRESH_MARGIN_S`]. Concurrent callers wait for one refresh and then share its
    /// result (the credentials are re-read under the lock).
    pub fn access_token(&self) -> Result<Secret, TokenError> {
        if !vault::supported() {
            return Err(TokenError::NotSupported);
        }
        let _g = self.lock.lock().map_err(|_| TokenError::Storage("sign-in lock poisoned".into()))?;
        let mut c = vault::load_credentials(&self.data_dir).map_err(TokenError::Storage)?.ok_or(TokenError::NotSignedIn)?;
        if !c.plan_enabled() {
            return Err(TokenError::PlanNotEnabled);
        }
        if c.expires_at_s - now_s() > REFRESH_MARGIN_S {
            return Ok(c.access());
        }
        let Some(rt) = c.refresh_token.clone().map(Secret::new) else {
            return Err(self.disconnect("The ChatGPT access token expired and there is no refresh token. Sign in again.".into()));
        };
        match oauth::refresh(&self.opts.endpoints, &c.client_id, rt.as_str(), self.opts.http) {
            Ok(tr) => {
                let now = now_s();
                c.access_token = tr.access_token.as_str().to_string();
                c.expires_at_s = now + tr.expires_in;
                if let Some(new_rt) = &tr.refresh_token {
                    c.refresh_token = Some(new_rt.as_str().to_string());
                }
                if let Some(id) = &tr.id_token {
                    c.id_token = id.as_str().to_string();
                }
                if let Some(scope) = &tr.scope {
                    c.scopes = scope.split_whitespace().map(str::to_string).collect();
                }
                c.earliest_refresh_at = tr.earliest_refresh_at;
                c.saved_at_s = now;
                vault::save_credentials(&self.data_dir, &c).map_err(TokenError::Storage)?;
                if !c.plan_enabled() {
                    return Err(TokenError::PlanNotEnabled);
                }
                self.set_note(None);
                Ok(c.access())
            }
            Err(OAuthError::Rejected { code, .. }) if TERMINAL_REFRESH_CODES.contains(&code) => Err(self.disconnect(format!(
                "ChatGPT ended this sign-in ({code} on refresh). Sign in again."
            ))),
            Err(OAuthError::Rejected { code: "invalid_client", .. }) => {
                if let Ok(Some(mut r)) = vault::load_registration(&self.data_dir) {
                    r.client_id = None;
                    r.subject = None;
                    let _ = vault::save_registration(&self.data_dir, &r);
                }
                Err(self.disconnect("ChatGPT no longer accepts this app registration (invalid_client). Sign in again to register.".into()))
            }
            Err(e) if e.is_transient() => Err(TokenError::Temporary(format!("Could not renew the ChatGPT sign-in right now ({e}); it is kept."))),
            Err(e) => Err(TokenError::Temporary(format!("Could not renew the ChatGPT sign-in ({e}); it is kept."))),
        }
    }

    /// Delete the tokens (called with the lock held) and remember why.
    fn disconnect(&self, why: String) -> TokenError {
        let _ = vault::delete_credentials(&self.data_dir);
        self.set_note(Some(format!("Disconnected: {why}")));
        TokenError::Disconnected(why)
    }

    /// Sign out: stop a pending attempt, try to revoke the refresh token (retrying network
    /// and 5xx failures twice), then delete the tokens. The registration (issued client ID
    /// and host ID) is kept for the next sign-in.
    pub fn logout(&self) -> LogoutOutcome {
        self.cancel_login();
        let Ok(_g) = self.lock.lock() else {
            return LogoutOutcome { revoked: Some(false), message: "sign-in lock poisoned".into() };
        };
        let creds = vault::load_credentials(&self.data_dir).ok().flatten();
        let mut revoked = None;
        if let Some(c) = &creds {
            if let Some(rt) = c.refresh_token.clone().map(Secret::new) {
                let mut ok = false;
                for attempt in 0..3 {
                    match oauth::revoke(&self.opts.endpoints, &c.client_id, rt.as_str(), self.opts.http) {
                        Ok(()) => {
                            ok = true;
                            break;
                        }
                        Err(e) if e.is_transient() && attempt < 2 => std::thread::sleep(self.opts.revoke_backoff * (attempt + 1)),
                        Err(_) => break,
                    }
                }
                revoked = Some(ok);
            }
        }
        let deleted = vault::delete_credentials(&self.data_dir);
        self.set_note(None);
        let message = match (revoked, deleted) {
            (_, Err(e)) => e,
            (Some(false), Ok(())) => "Signed out on this computer, but ChatGPT did not confirm the revocation. You can disconnect re:Search in ChatGPT Settings.".into(),
            (Some(true), Ok(())) => "Signed out; the ChatGPT session was revoked.".into(),
            (None, Ok(())) => "Signed out.".into(),
        };
        LogoutOutcome { revoked, message }
    }

    pub fn cancel_login(&self) {
        if let Some(a) = self.attempt.lock().ok().and_then(|mut g| g.take()) {
            a.cancel();
        }
    }

    /// The current (or last) attempt.
    pub fn login_attempt(&self) -> Option<Arc<Attempt>> {
        self.attempt.lock().ok().and_then(|g| g.clone())
    }

    /// The authorize URL of the pending attempt (for "Open in browser").
    pub fn pending_authorize_url(&self) -> Option<String> {
        self.login_attempt().filter(|a| a.is_pending() && !a.remaining().is_zero()).map(|a| a.authorize_url.clone())
    }

    /// Start a sign-in: persist (or reuse) this host's ID, start the one-shot listener on
    /// `127.0.0.1`, and return the authorize URL. A previous pending attempt is cancelled.
    pub fn start_login(self: &Arc<Self>) -> Result<LoginStart, String> {
        if !vault::supported() {
            return Err(vault::NOT_SUPPORTED.to_string());
        }
        let reg = match vault::load_registration(&self.data_dir) {
            Ok(Some(r)) => r,
            Ok(None) | Err(_) => {
                let r = Registration { ext_agent_host_id: new_host_id()?, client_id: None, subject: None };
                vault::save_registration(&self.data_dir, &r)?;
                r
            }
        };
        self.cancel_login();
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "could not open a local port for the sign-in callback".to_string())?;
        let port = listener.local_addr().map_err(|_| "could not read the callback port".to_string())?.port();
        let pending = Pending {
            state: random_token()?,
            nonce: random_token()?,
            verifier: random_token()?,
            redirect_uri: format!("http://127.0.0.1:{port}{REDIRECT_PATH}"),
            known_client: reg.client_id.clone(),
        };
        let client_id = reg.client_id.as_deref().unwrap_or(DYNAMIC_CLIENT_ID);
        let url = oauth::authorize_url(
            &self.opts.endpoints,
            &AuthorizeRequest {
                client_id,
                redirect_uri: &pending.redirect_uri,
                state: &pending.state,
                nonce: &pending.nonce,
                code_challenge: &pkce_challenge(&pending.verifier),
                ext_agent_host_id: &reg.ext_agent_host_id,
            },
        );
        let attempt = Arc::new(Attempt::new(url.clone(), port, self.opts.login_timeout));
        if let Ok(mut g) = self.attempt.lock() {
            *g = Some(Arc::clone(&attempt));
        }
        login::spawn(listener, attempt, pending, Arc::clone(self))?;
        self.set_note(None);
        Ok(LoginStart { authorize_url: url, port, expires_in_s: self.opts.login_timeout.as_secs() })
    }

    /// Code exchange, ID-token and scope checks, then storage. Returns whether ChatGPT
    /// plan usage was granted. Errors are fixed text plus a class.
    pub(crate) fn finish_login(&self, code: &str, client_id: &str, p: &Pending) -> Result<bool, String> {
        let ep = &self.opts.endpoints;
        let tr = oauth::exchange_code(ep, client_id, code, &p.verifier, &p.redirect_uri, self.opts.http)
            .map_err(|e| match e {
                OAuthError::Rejected { code: "invalid_grant", .. } => "the authorization code was refused (invalid_grant); start the sign-in again".to_string(),
                e => format!("the code exchange failed ({e})"),
            })?;
        let id_token = tr.id_token.as_ref().ok_or("the token answer had no ID token")?;
        let jwks = oauth::fetch_jwks(ep, self.opts.http).map_err(|e| format!("could not fetch OpenAI's signing keys ({e})"))?;
        let claims = verify_id_token(id_token.as_str(), &jwks, &ep.issuer, client_id, &p.nonce, now_s()).map_err(|e| e.to_string())?;

        let _g = self.lock.lock().map_err(|_| "sign-in lock poisoned".to_string())?;
        let mut reg = vault::load_registration(&self.data_dir)?.ok_or("this host's registration is missing; start the sign-in again")?;
        if let (Some(known), Some(sub)) = (&p.known_client, &reg.subject) {
            if known == client_id && *sub != claims.sub {
                return Err("signed in as a different ChatGPT account than the saved one; nothing was changed".into());
            }
        }
        let scopes: Vec<String> = tr.scope.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect();
        let now = now_s();
        let creds = Credentials {
            issuer: ep.issuer.clone(),
            client_id: client_id.to_string(),
            subject: claims.sub.clone(),
            email: claims.verified_email.clone(),
            id_token: id_token.as_str().to_string(),
            access_token: tr.access_token.as_str().to_string(),
            refresh_token: tr.refresh_token.as_ref().map(|r| r.as_str().to_string()),
            token_type: tr.token_type.clone(),
            expires_at_s: now + tr.expires_in,
            scopes,
            saved_at_s: now,
            earliest_refresh_at: tr.earliest_refresh_at,
        };
        reg.client_id = Some(client_id.to_string());
        reg.subject = Some(claims.sub);
        vault::save_registration(&self.data_dir, &reg)?;
        vault::save_credentials(&self.data_dir, &creds)?;
        self.set_note(None);
        Ok(creds.plan_enabled())
    }

    /// The attempt's state for the UI: `(state, message)`.
    pub fn login_view(&self) -> (&'static str, String) {
        match self.login_attempt() {
            None => ("none", String::new()),
            Some(a) => {
                let s = a.state();
                let msg = match &s {
                    LoginState::Pending => "Waiting for you to finish signing in in the browser.".to_string(),
                    LoginState::Succeeded { plan_enabled: true } => "Signed in; re:Search uses your ChatGPT plan.".to_string(),
                    LoginState::Succeeded { plan_enabled: false } => {
                        "Signed in, but ChatGPT plan use was not granted. Sign in again and allow it, or use an API key.".to_string()
                    }
                    LoginState::Failed(m) => m.clone(),
                    LoginState::Expired => "The sign-in link expired. Start again.".to_string(),
                    LoginState::Cancelled => "The sign-in was cancelled.".to_string(),
                };
                (s.as_str(), msg)
            }
        }
    }
}
