//! [`ChatGptAuth`]: sign-in attempts, status, the current access token (refreshed under a
//! mutex, rotated refresh tokens persisted atomically), and sign-out with revocation.

use crate::codec::{ct_eq, new_host_id, pkce_challenge, random_token, try_now_s, Secret};
use crate::jwt::verify_id_token;
use crate::login::{self, Attempt, LoginState, Pending};
use crate::oauth::{self, AuthorizeRequest, Endpoints, HttpCfg, OAuthError, DYNAMIC_CLIENT_ID, REDIRECT_PATH, TERMINAL_REFRESH_CODES};
use crate::vault::{self, Credentials, Registration, VaultLock};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

/// Refresh when the access token has less than this many seconds left (at most half of
/// the token's issued lifetime, so a short-lived token is not refreshed on every request).
pub const REFRESH_MARGIN_S: i64 = 300;
/// An access token with this many seconds or fewer left counts as expired.
pub const EXPIRY_SKEW_S: i64 = 10;
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// Default overall time for the network part of a sign-out (discovery, revocation
/// attempts and their backoff); it runs with no lock held.
pub const REVOKE_BUDGET: Duration = Duration::from_secs(45);
/// Overall time for revoking tokens of a completed code exchange that is then discarded.
const DISCARD_REVOKE_BUDGET: Duration = Duration::from_secs(10);
const MAX_EXPIRES_IN_S: i64 = 86_400 * 365;

#[derive(Debug, Clone)]
pub struct Options {
    pub endpoints: Endpoints,
    pub http: HttpCfg,
    pub login_timeout: Duration,
    /// Backoff between revocation retries on sign-out.
    pub revoke_backoff: Duration,
    /// Overall cap on sign-out network time (see [`REVOKE_BUDGET`]).
    pub revoke_budget: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            endpoints: Endpoints::production(),
            http: HttpCfg::default(),
            login_timeout: LOGIN_TIMEOUT,
            revoke_backoff: Duration::from_millis(500),
            revoke_budget: REVOKE_BUDGET,
        }
    }
}

/// Whether the saved access token should be refreshed now. A token that is expired (or
/// within [`EXPIRY_SKEW_S`] of it) always is. Otherwise a refresh happens only inside the
/// refresh margin (`min(REFRESH_MARGIN_S, lifetime / 2)`) and never before the server's
/// `earliest_refresh_at`.
pub(crate) fn refresh_due(c: &Credentials, now: i64) -> bool {
    let left = c.expires_at_s.saturating_sub(now);
    if left <= EXPIRY_SKEW_S {
        return true;
    }
    let lifetime = c.expires_at_s.saturating_sub(c.saved_at_s).max(0);
    if left > REFRESH_MARGIN_S.min(lifetime / 2) {
        return false;
    }
    !c.earliest_refresh_at.is_some_and(|t| now < t)
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
    /// Serializes refresh, sign-in start/completion and the storage part of sign-out.
    /// Status reads do not take it (a refresh holds it across a network call), and no
    /// sign-out network call runs under it.
    lock: Mutex<()>,
    note: Mutex<Option<String>>,
    attempt: Mutex<Option<Arc<Attempt>>>,
    login_gen: AtomicU64,
}

impl ChatGptAuth {
    pub fn new(data_dir: impl Into<PathBuf>, opts: Options) -> Self {
        Self {
            data_dir: data_dir.into(),
            opts,
            lock: Mutex::new(()),
            note: Mutex::new(None),
            attempt: Mutex::new(None),
            login_gen: AtomicU64::new(0),
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.opts.endpoints
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Monotonic counter incremented on completed sign-in or sign-out.
    pub fn login_generation(&self) -> u64 {
        self.login_gen.load(Ordering::Acquire)
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
        // No lock: the vault files are replaced atomically, and a refresh or sign-out may
        // be waiting on the network. A read that races a replace is retried briefly.
        match self.load_for_status() {
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

    fn load_for_status(&self) -> Result<Option<Credentials>, String> {
        let mut tries = 0;
        loop {
            match vault::load_credentials(&self.data_dir) {
                Err(_) if tries < 2 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                r => return r,
            }
        }
    }

    /// A current access token, refreshed first when [`refresh_due`] says so. Concurrent
    /// callers wait for one refresh and then share its result (the credentials are re-read
    /// under the lock).
    pub fn access_token(&self) -> Result<Secret, TokenError> {
        if !vault::supported() {
            return Err(TokenError::NotSupported);
        }
        let _g = self.lock.lock().map_err(|_| TokenError::Storage("sign-in lock poisoned".into()))?;
        let _vl = VaultLock::acquire(&self.data_dir).map_err(TokenError::Storage)?;
        let c = vault::load_credentials(&self.data_dir).map_err(TokenError::Storage)?.ok_or(TokenError::NotSignedIn)?;
        if !c.plan_enabled() {
            return Err(TokenError::PlanNotEnabled);
        }
        let now = try_now_s().map_err(|e| TokenError::Temporary(format!("Cannot verify token expiry: {e}.")))?;
        if !refresh_due(&c, now) {
            return Ok(c.access());
        }
        self.refresh_locked(c, now, true)
    }

    /// Force a token refresh after an HTTP 401 on `failed_token`. If another thread or
    /// process already rotated the access token, returns the newer token without calling
    /// the token endpoint again.
    pub fn force_refresh(&self, failed_token: &str) -> Result<Secret, TokenError> {
        if !vault::supported() {
            return Err(TokenError::NotSupported);
        }
        let _g = self.lock.lock().map_err(|_| TokenError::Storage("sign-in lock poisoned".into()))?;
        let _vl = VaultLock::acquire(&self.data_dir).map_err(TokenError::Storage)?;
        let c = vault::load_credentials(&self.data_dir).map_err(TokenError::Storage)?.ok_or(TokenError::NotSignedIn)?;
        if !c.plan_enabled() {
            return Err(TokenError::PlanNotEnabled);
        }
        if !ct_eq(c.access_token.as_bytes(), failed_token.as_bytes()) {
            return Ok(c.access());
        }
        let now = try_now_s().map_err(|e| TokenError::Temporary(format!("Cannot verify token expiry: {e}.")))?;
        self.refresh_locked(c, now, false)
    }

    fn refresh_locked(&self, mut c: Credentials, now: i64, allow_fallback: bool) -> Result<Secret, TokenError> {
        let Some(rt) = c.refresh_token.clone().map(Secret::new) else {
            return Err(self.disconnect("The ChatGPT access token expired and there is no refresh token. Sign in again.".into()));
        };
        match oauth::refresh(&self.opts.endpoints, &c.client_id, rt.as_str(), self.opts.http) {
            Ok(tr) => {
                let now = try_now_s().map_err(|e| TokenError::Temporary(format!("Cannot verify token expiry: {e}.")))?;
                let ttl = tr.expires_in.clamp(1, MAX_EXPIRES_IN_S);
                c.access_token.zeroize();
                c.access_token = tr.access_token.as_str().to_string();
                c.expires_at_s = now.saturating_add(ttl);
                if let Some(new_rt) = &tr.refresh_token {
                    if let Some(old_rt) = &mut c.refresh_token {
                        old_rt.zeroize();
                    }
                    c.refresh_token = Some(new_rt.as_str().to_string());
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
                self.clear_client_registration_locked(Some(&c.client_id));
                Err(self.disconnect("ChatGPT no longer accepts this app registration (invalid_client). Sign in again to register.".into()))
            }
            Err(e) if e.is_transient() => {
                if allow_fallback && c.expires_at_s.saturating_sub(now) > EXPIRY_SKEW_S {
                    return Ok(c.access());
                }
                Err(TokenError::Temporary(format!("Could not renew the ChatGPT sign-in right now ({e}); it is kept.")))
            }
            Err(e) => Err(TokenError::Temporary(format!("Could not renew the ChatGPT sign-in ({e}); it is kept."))),
        }
    }

    pub(crate) fn clear_client_registration(&self, expected_client: Option<&str>, attempt: &Attempt) {
        if !vault::supported() || attempt.is_cancelled() {
            return;
        }
        let Ok(_g) = self.lock.lock() else {
            return;
        };
        let Ok(_vl) = VaultLock::acquire(&self.data_dir) else {
            return;
        };
        if attempt.is_cancelled() {
            return;
        }
        self.clear_client_registration_locked(expected_client);
    }

    fn clear_client_registration_locked(&self, expected_client: Option<&str>) {
        if let Ok(Some(mut r)) = vault::load_registration(&self.data_dir) {
            if expected_client.is_none_or(|c| r.client_id.as_deref() == Some(c)) {
                r.client_id = None;
                r.subject = None;
                let _ = vault::save_registration(&self.data_dir, &r);
            }
        }
    }

    /// Delete the tokens (called with the lock held) and remember why.
    fn disconnect(&self, why: String) -> TokenError {
        let _ = vault::delete_credentials(&self.data_dir);
        self.set_note(Some(format!("Disconnected: {why}")));
        TokenError::Disconnected(why)
    }

    /// Best-effort revocation of `rt`: discovery once, then up to `attempts` tries
    /// (retrying network and 5xx failures), all inside `budget`. Callers hold no lock.
    fn revoke_bounded(&self, client_id: &str, rt: &Secret, attempts: u32, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        let left = || deadline.saturating_duration_since(Instant::now());
        let url = oauth::revocation_url(&self.opts.endpoints, self.opts.http.capped(budget / 3));
        for attempt in 0..attempts {
            if left().is_zero() {
                return false;
            }
            match oauth::revoke_at(&url, client_id, rt.as_str(), self.opts.http.capped(left())) {
                Ok(()) => return true,
                Err(e) if e.is_transient() && attempt + 1 < attempts => {
                    std::thread::sleep((self.opts.revoke_backoff * (attempt + 1)).min(left()));
                }
                Err(_) => return false,
            }
        }
        false
    }

    /// Sign out: stop a pending attempt, delete the stored tokens (under both locks, so no
    /// refresh in this or another process can save rotated tokens afterwards), then, with
    /// no lock held, try to revoke the refresh token from an in-memory copy (discovery once,
    /// network and 5xx failures retried twice, all within [`Options::revoke_budget`]). The
    /// copy is zeroized when dropped. The registration (issued client ID and host ID) is
    /// kept for the next sign-in.
    pub fn logout(&self) -> LogoutOutcome {
        self.cancel_login();
        if !vault::supported() {
            self.login_gen.fetch_add(1, Ordering::Release);
            self.set_note(None);
            return LogoutOutcome { revoked: None, message: "Signed out.".into() };
        }
        let (to_revoke, deleted) = {
            let Ok(_g) = self.lock.lock() else {
                return LogoutOutcome { revoked: Some(false), message: "sign-in lock poisoned".into() };
            };
            let Ok(_vl) = VaultLock::acquire(&self.data_dir) else {
                return LogoutOutcome { revoked: Some(false), message: "Not signed out: another ChatGPT sign-in operation is in progress, so the saved sign-in is still on this computer. Try Sign out again.".into() };
            };
            let creds = vault::load_credentials(&self.data_dir).ok().flatten();
            let deleted = vault::delete_credentials(&self.data_dir);
            self.login_gen.fetch_add(1, Ordering::Release);
            self.set_note(None);
            let copy = creds.and_then(|c| c.refresh_token.clone().map(|rt| (c.client_id.clone(), Secret::new(rt))));
            (copy, deleted)
        };
        let revoked = to_revoke.map(|(client_id, rt)| self.revoke_bounded(&client_id, &rt, 3, self.opts.revoke_budget));
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
        let _g = self.lock.lock().map_err(|_| "sign-in lock poisoned".to_string())?;
        let (reg, prompt_consent) = {
            let _vl = VaultLock::acquire(&self.data_dir)?;
            let reg = match vault::load_registration(&self.data_dir)? {
                Some(r) => r,
                None => {
                    let r = Registration { ext_agent_host_id: new_host_id()?, client_id: None, subject: None };
                    vault::save_registration(&self.data_dir, &r)?;
                    r
                }
            };
            let prompt_consent = matches!(vault::load_credentials(&self.data_dir), Ok(Some(c)) if !c.plan_enabled());
            (reg, prompt_consent)
        };
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
                prompt_consent,
            },
        );
        let attempt = Arc::new(Attempt::new(url.clone(), port, self.opts.login_timeout));
        login::spawn(listener, Arc::clone(&attempt), pending, Arc::clone(self))?;
        if let Ok(mut g) = self.attempt.lock() {
            if let Some(prev) = g.replace(attempt) {
                prev.cancel();
            }
        }
        self.set_note(None);
        Ok(LoginStart { authorize_url: url, port, expires_in_s: self.opts.login_timeout.as_secs() })
    }

    /// Code exchange, ID-token and scope checks, then storage. Returns whether ChatGPT
    /// plan usage was granted. Errors are fixed text plus a class.
    pub(crate) fn finish_login(&self, code: &str, client_id: &str, p: &Pending, attempt: &Attempt) -> Result<bool, String> {
        if attempt.is_cancelled() {
            return Err("sign-in was cancelled".into());
        }
        let ep = &self.opts.endpoints;
        let tr = match oauth::exchange_code(ep, client_id, code, &p.verifier, &p.redirect_uri, self.opts.http) {
            Ok(tr) => tr,
            Err(OAuthError::Rejected { code: "invalid_grant", .. }) => {
                return Err("the authorization code was refused (invalid_grant); start the sign-in again".to_string());
            }
            Err(e @ OAuthError::Rejected { code: "invalid_client", .. }) => {
                if let Some(known) = &p.known_client {
                    self.clear_client_registration(Some(known), attempt);
                }
                return Err(format!("the code exchange failed ({e})"));
            }
            Err(e) => return Err(format!("the code exchange failed ({e})")),
        };
        let res = self.accept_tokens(&tr, client_id, p, attempt);
        if res.is_err() {
            // The exchange succeeded but its tokens are discarded (cancelled attempt, other
            // account, failed ID token, storage error): revoke the fresh grant, best effort
            // and bounded, with every lock released. `tr` is zeroized when dropped.
            if let Some(rt) = &tr.refresh_token {
                self.revoke_bounded(client_id, rt, 1, DISCARD_REVOKE_BUDGET.min(self.opts.revoke_budget));
            }
        }
        res
    }

    fn accept_tokens(&self, tr: &oauth::TokenResponse, client_id: &str, p: &Pending, attempt: &Attempt) -> Result<bool, String> {
        let ep = &self.opts.endpoints;
        let id_token = tr.id_token.as_ref().ok_or("the token answer had no ID token")?;
        let jwks = oauth::fetch_jwks(ep, self.opts.http).map_err(|e| format!("could not fetch OpenAI's signing keys ({e})"))?;
        let now = try_now_s().map_err(|e| format!("cannot verify ID token time ({e})"))?;
        let claims = verify_id_token(id_token.as_str(), &jwks, &ep.issuer, client_id, &p.nonce, now).map_err(|e| e.to_string())?;

        let _g = self.lock.lock().map_err(|_| "sign-in lock poisoned".to_string())?;
        let _vl = VaultLock::acquire(&self.data_dir)?;
        if attempt.is_cancelled() {
            return Err("sign-in was cancelled".into());
        }
        let mut reg = vault::load_registration(&self.data_dir)?.ok_or("this host's registration is missing; start the sign-in again")?;
        if let (Some(known), Some(sub)) = (&p.known_client, &reg.subject) {
            if known == client_id && *sub != claims.sub {
                return Err("signed in as a different ChatGPT account than the saved one; nothing was changed".into());
            }
        }
        let scopes: Vec<String> = tr.scope.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect();
        let now = try_now_s().map_err(|e| format!("cannot save sign-in time ({e})"))?;
        let ttl = tr.expires_in.clamp(1, MAX_EXPIRES_IN_S);
        let creds = Credentials {
            issuer: ep.issuer.clone(),
            client_id: client_id.to_string(),
            subject: claims.sub.clone(),
            email: claims.verified_email.clone(),
            id_token: id_token.as_str().to_string(),
            access_token: tr.access_token.as_str().to_string(),
            refresh_token: tr.refresh_token.as_ref().map(|r| r.as_str().to_string()),
            token_type: tr.token_type.clone(),
            expires_at_s: now.saturating_add(ttl),
            scopes,
            saved_at_s: now,
            earliest_refresh_at: tr.earliest_refresh_at,
        };
        reg.client_id = Some(client_id.to_string());
        reg.subject = Some(claims.sub);
        vault::save_registration(&self.data_dir, &reg)?;
        vault::save_credentials(&self.data_dir, &creds)?;
        self.login_gen.fetch_add(1, Ordering::Release);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(saved_at_s: i64, expires_at_s: i64, earliest_refresh_at: Option<i64>) -> Credentials {
        Credentials {
            issuer: "i".into(), client_id: "c".into(), subject: "s".into(), email: None, id_token: "x".into(),
            access_token: "a".into(), refresh_token: Some("r".into()), token_type: "Bearer".into(),
            expires_at_s, scopes: vec![], saved_at_s, earliest_refresh_at,
        }
    }

    #[test]
    fn refresh_is_due_only_inside_a_margin_capped_by_half_the_lifetime_and_after_earliest_refresh_at() {
        let now = 1_800_000_000;
        // One-hour token: the margin is 300 s.
        assert!(!refresh_due(&creds(now - 3000, now + 600, None), now));
        assert!(refresh_due(&creds(now - 3400, now + 200, None), now));
        // Short-lived tokens (lifetime <= 600 s) are not refreshed while more than half is
        // left, so a fresh short token is not refreshed on every request.
        for ttl in [20, 60, 120, 300, 600] {
            assert!(!refresh_due(&creds(now, now + ttl, None), now), "fresh {ttl} s token");
        }
        assert!(refresh_due(&creds(now - 40, now + 20, None), now), "60 s token two-thirds used");
        // earliest_refresh_at in the future holds off a margin refresh...
        assert!(!refresh_due(&creds(now - 3400, now + 200, Some(now + 100)), now));
        assert!(refresh_due(&creds(now - 3400, now + 200, Some(now)), now));
        // ...but never an expired (or nearly expired) token.
        assert!(refresh_due(&creds(now - 3600, now + EXPIRY_SKEW_S, Some(now + 100)), now));
        assert!(refresh_due(&creds(now - 3600, now - 5, Some(now + 100)), now));
    }
}
