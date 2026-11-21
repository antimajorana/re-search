//! "Sign in with ChatGPT" for open-source and locally hosted apps (OpenAI preview).
//!
//! Implements the flow documented at
//! <https://developers.openai.com/siwc/token-sharing-open-source> and its sub-pages:
//! first-time dynamic registration (`client_id=dynamic_agent_client`, `agent_name_hint`,
//! a persisted `ext_agent_host_id`), PKCE S256 with `state` and `nonce`, a one-shot
//! loopback callback on `127.0.0.1` with a fixed path, the form-encoded code exchange
//! without a client secret, ID-token validation against OpenAI's JWKS, the
//! `chatgpt.tokens.use.direct` scope check, DPAPI-encrypted storage, serialized refresh
//! with rotating refresh tokens, and revocation on sign-out.
//!
//! Nothing in this crate prints, logs or returns a token in an error or status string.

pub mod codec;
pub mod jwt;
pub mod login;
pub mod manager;
pub mod oauth;
pub mod vault;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

pub use codec::Secret;
pub use login::LoginState;
pub use manager::{AuthStatus, ChatGptAuth, LoginStart, LogoutOutcome, Options, TokenError};
pub use oauth::{Endpoints, HttpCfg};

/// Opens `url` with the operating system's default handler, only when it is a sign-in URL
/// of `endpoints` (`<authorize>?…`). Used when the person clicks "Open in browser"; tests
/// replace it. Never goes through a shell, so `&` in the URL cannot start a command.
pub fn open_authorize_url(endpoints: &Endpoints, url: &str) -> Result<(), String> {
    let prefix = format!("{}?", endpoints.authorize);
    if !url.starts_with(&prefix) || url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"') {
        return Err("refusing to open an address that is not the ChatGPT sign-in page".into());
    }
    open_with_os(url)
}

#[cfg(windows)]
fn open_with_os(url: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let r = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    // ShellExecuteW returns a value greater than 32 on success.
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err("the default browser could not be started; use Copy link instead".into())
    }
}

#[cfg(not(windows))]
fn open_with_os(url: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "the default browser could not be started; use Copy link instead".into())
}
