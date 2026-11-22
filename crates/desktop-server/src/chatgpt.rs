//! "Sign in with ChatGPT" in the desktop server: the Connect entry, the sign-in routes,
//! and the in-core transport for sessions on the `chatgpt` provider.
//!
//! - `POST /api/connections/chatgpt/login` starts a sign-in (a one-shot listener on
//!   `127.0.0.1`, see `chatgpt_auth::login`) and returns the authorize URL for the UI to
//!   show with "Open in browser" and "Copy link".
//! - `POST /api/connections/chatgpt/open` opens the pending authorize URL with the system's
//!   default handler. Only the server's own pending URL is opened, never one from a
//!   request, and only when the person clicks (tests inject the opener).
//! - `GET /api/connections/chatgpt/login` reports the attempt (`pending`, `succeeded`, ...).
//! - `POST /api/connections/chatgpt/logout` revokes and deletes the tokens.
//!
//! All four need the launch token like every `/api/` route. Tokens never leave
//! `chatgpt_auth` except as the `Authorization` header of a request to OpenAI's API host:
//! not in an API answer, the SSE stream, the ledger or an error string. The ChatGPT
//! route ignores `--api-base` and saved base URLs; only [`ChatGptTestHooks`] (integration
//! tests, through [`crate::DesktopConfig::chatgpt_test`]) can point it at a loopback mock.

use crate::routes_session::ApiResult;
use crate::state::DesktopState;
use crate::DesktopConfig;
use agent::{BearerSource, ChatGptConfig, ChatGptTransport, SecretKey, Transport, TransportError, TransportErrorKind};
use chatgpt_auth::{ChatGptAuth, Endpoints, Options, TokenError};
use serde_json::{json, Value};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const CHATGPT_ID: &str = "chatgpt";
pub const CHATGPT_LABEL: &str = "ChatGPT (your subscription, Sign in with ChatGPT)";
pub const CHATGPT_NOTE: &str = "runs on your ChatGPT plan";
pub const CHATGPT_HOW_TO: &str = "Press Continue with ChatGPT, sign in to ChatGPT in your browser and allow re:Search to use your plan. This is OpenAI's Sign in with ChatGPT preview for open-source apps; usage counts toward your ChatGPT plan's limits (chatgpt.com/settings/usage).";
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

/// Opens a URL with the system's default handler.
pub type UrlOpener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// What a test opener answers.
pub const TEST_OPENER_REFUSAL: &str = "test: browser opening disabled";

/// An opener that never opens anything; every test configuration uses it.
pub fn refusing_opener() -> UrlOpener {
    Arc::new(|_url: &str| Err(TEST_OPENER_REFUSAL.to_string()))
}

/// The real opener: `chatgpt_auth::open_authorize_url` (ShellExecuteW; `xdg-open`
/// elsewhere), limited to `ep`'s sign-in page.
#[cfg(not(test))]
fn system_opener(ep: Endpoints) -> UrlOpener {
    Arc::new(move |url: &str| chatgpt_auth::open_authorize_url(&ep, url))
}

/// This crate's unit tests never get the real opener, whatever their configuration says.
#[cfg(test)]
fn system_opener(_ep: Endpoints) -> UrlOpener {
    refusing_opener()
}

/// Test hooks for unit tests: a refusing opener and every endpoint on a closed loopback
/// port, so nothing can reach a browser or OpenAI.
#[cfg(test)]
pub(crate) fn offline_test_hooks() -> ChatGptTestHooks {
    ChatGptTestHooks {
        mock_base: chatgpt_auth::testkit::closed_loopback_base(),
        opener: refusing_opener(),
        login_timeout: Duration::from_secs(5),
    }
}

/// Test-only: a loopback mock for every OpenAI endpoint, a replacement for the browser
/// opener, and a short sign-in timeout. Production leaves [`DesktopConfig::chatgpt_test`]
/// at `None`.
#[derive(Clone)]
pub struct ChatGptTestHooks {
    /// `http://127.0.0.1:<port>`; anything else is refused at start.
    pub mock_base: String,
    pub opener: UrlOpener,
    pub login_timeout: Duration,
}

impl fmt::Debug for ChatGptTestHooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatGptTestHooks").field("mock_base", &self.mock_base).finish_non_exhaustive()
    }
}

pub struct ChatGptHub {
    pub auth: Arc<ChatGptAuth>,
    opener: UrlOpener,
    last_login_gen: AtomicU64,
}

impl ChatGptHub {
    pub fn new(cfg: &DesktopConfig) -> Result<Self, String> {
        let (opts, opener): (Options, UrlOpener) = match &cfg.chatgpt_test {
            Some(h) => (
                Options { endpoints: Endpoints::loopback_mock(&h.mock_base)?, login_timeout: h.login_timeout, ..Options::default() },
                Arc::clone(&h.opener),
            ),
            None => {
                let ep = Endpoints::production();
                (Options { endpoints: ep.clone(), ..Options::default() }, system_opener(ep))
            }
        };
        let auth = Arc::new(ChatGptAuth::new(cfg.data_dir.clone(), opts));
        let last_login_gen = AtomicU64::new(auth.login_generation());
        Ok(Self { auth, opener, last_login_gen })
    }

    /// Clear `catalog` once whenever a new sign-in or sign-out advanced the login generation.
    pub fn sync_catalog(&self, catalog: &crate::model_catalog::Catalog) {
        let cur = self.auth.login_generation();
        let mut prev = self.last_login_gen.load(Ordering::Acquire);
        while cur > prev {
            match self.last_login_gen.compare_exchange_weak(prev, cur, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    catalog.clear();
                    return;
                }
                Err(actual) => prev = actual,
            }
        }
    }

    /// Where the Responses API and the model list are (fixed; never a request or setting).
    pub fn api_base(&self) -> String {
        self.auth.endpoints().api_base.clone()
    }

    pub fn ready(&self) -> bool {
        let st = self.auth.status();
        st.signed_in && st.plan_enabled
    }
}

fn map_token_result(res: Result<chatgpt_auth::Secret, TokenError>) -> Result<SecretKey, TransportError> {
    match res {
        Ok(t) => SecretKey::new(t.as_str().to_string()),
        Err(e) => {
            let kind = match e {
                TokenError::Temporary(_) => TransportErrorKind::Network,
                TokenError::NotSupported | TokenError::Storage(_) => TransportErrorKind::Config,
                _ => TransportErrorKind::Auth,
            };
            Err(TransportError { kind, status: None, message: e.to_string(), raw_body: None })
        }
    }
}

/// The access token as the agent transports' key type; errors without token material.
pub(crate) fn bearer(auth: &ChatGptAuth) -> Result<SecretKey, TransportError> {
    map_token_result(auth.access_token())
}

struct HubBearer(Arc<ChatGptAuth>);

impl BearerSource for HubBearer {
    fn bearer(&self) -> Result<SecretKey, TransportError> {
        bearer(&self.0)
    }

    fn force_refresh(&self, failed_token: &str) -> Result<SecretKey, TransportError> {
        map_token_result(self.0.force_refresh(failed_token))
    }
}

/// The in-core transport for a `chatgpt` turn (every tool call still goes through the
/// router, policy, approvals and ledger).
pub fn build_transport(state: &DesktopState) -> Result<Box<dyn Transport>, String> {
    let st = state.chatgpt.auth.status();
    if !st.signed_in {
        return Err(TokenError::NotSignedIn.to_string());
    }
    if !st.plan_enabled {
        return Err(TokenError::PlanNotEnabled.to_string());
    }
    let cfg = ChatGptConfig { base_url: state.chatgpt.api_base(), ..ChatGptConfig::default() };
    let t = ChatGptTransport::new(cfg, Arc::new(HubBearer(Arc::clone(&state.chatgpt.auth)))).map_err(|e| e.to_string())?;
    Ok(Box::new(t))
}

/// The Connect panel row (same generic fields as the other providers, plus `action`).
pub fn connection_entry(state: &DesktopState) -> Value {
    let st = state.chatgpt.auth.status();
    let (login_state, login_msg) = state.chatgpt.auth.login_view();
    let connected = st.signed_in && st.plan_enabled;
    let status = if connected {
        match state.catalog.last_error(CHATGPT_ID) {
            Some(err) => format!("{} The model list failed: {err}", st.message),
            None => st.message.clone(),
        }
    } else if login_state == "pending" {
        login_msg.clone()
    } else {
        st.message.clone()
    };
    json!({
        "id": CHATGPT_ID,
        "label": CHATGPT_LABEL,
        "kind": "subscription",
        "route": "api",
        "connected": connected,
        "plan_enabled": st.plan_enabled,
        "can_signout": st.signed_in,
        "status": status,
        "how_to": (!connected).then_some(CHATGPT_HOW_TO),
        "version": null,
        "accepts_key": false,
        "key_required": false,
        "has_saved_key": false,
        "key_source": null,
        "key_env": null,
        "base_url": null,
        "session_note": CHATGPT_NOTE,
        "action": if st.signed_in && st.plan_enabled { "signout" } else { "signin" },
        "action_label": if st.signed_in && !st.plan_enabled {
            "Allow ChatGPT plan"
        } else if st.signed_in {
            "Sign out"
        } else {
            "Continue with ChatGPT"
        },
        "action_supported": st.supported,
        "account": st.account,
        "login": { "state": login_state, "message": login_msg },
        "pending_authorize_url": state.chatgpt.auth.pending_authorize_url(),
        "usage_url": USAGE_URL,
        "preview": true,
    })
}

/// `POST /api/connections/chatgpt/login`.
pub fn handle_login(state: &DesktopState) -> ApiResult {
    let start = state.chatgpt.auth.start_login().map_err(|e| (400, e))?;
    Ok((
        200,
        json!({
            "state": "pending",
            "authorize_url": start.authorize_url,
            "expires_in_s": start.expires_in_s,
            "message": "Open the link in your browser, sign in to ChatGPT and allow re:Search to use your plan.",
        }),
    ))
}

/// `GET /api/connections/chatgpt/login`.
pub fn handle_login_status(state: &DesktopState) -> ApiResult {
    let (login_state, message) = state.chatgpt.auth.login_view();
    state.chatgpt.sync_catalog(&state.catalog);
    Ok((200, json!({ "state": login_state, "message": message, "connected": state.chatgpt.ready() })))
}

/// `POST /api/connections/chatgpt/open`: open the pending authorize URL (nothing from the
/// request is used).
pub fn handle_open(state: &DesktopState) -> ApiResult {
    let url = state
        .chatgpt
        .auth
        .pending_authorize_url()
        .ok_or((409, "No sign-in is waiting. Press Continue with ChatGPT first.".to_string()))?;
    (state.chatgpt.opener)(&url).map_err(|e| (500, e))?;
    Ok((200, json!({ "opened": true })))
}

/// `POST /api/connections/chatgpt/logout`.
pub fn handle_logout(state: &DesktopState) -> ApiResult {
    let out = state.chatgpt.auth.logout();
    state.catalog.clear();
    // Report what is actually on disk now: sign-out can fail to take the folder lock, and
    // then the saved sign-in is still there.
    let st = state.chatgpt.auth.status();
    Ok((200, json!({ "revoked": out.revoked, "message": out.message, "connected": st.signed_in && st.plan_enabled, "signed_in": st.signed_in })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_test_builds_have_no_real_opener() {
        let ep = Endpoints::production();
        let url = format!("{}?client_id=x", ep.authorize);
        assert_eq!(system_opener(ep)(&url).unwrap_err(), TEST_OPENER_REFUSAL);
        let hooks = offline_test_hooks();
        assert_eq!((hooks.opener)(&url).unwrap_err(), TEST_OPENER_REFUSAL);
        assert!(hooks.mock_base.starts_with("http://127.0.0.1:"));
    }
}
