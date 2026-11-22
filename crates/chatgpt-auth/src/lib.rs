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

/// This crate's own unit tests can never start a browser.
#[cfg(test)]
fn open_with_os(_url: &str) -> Result<(), String> {
    Err("test: browser opening disabled".into())
}

#[cfg(all(windows, not(test)))]
fn open_with_os(url: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: `CoInitializeEx` initializes COM for `ShellExecuteW` on this thread and
    // `S_OK` (0) / `S_FALSE` (1) is paired with `CoUninitialize`. Both strings are
    // NUL-terminated and outlive `ShellExecuteW`.
    let r = unsafe {
        let hr = CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        );
        let r = ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
        if hr == 0 || hr == 1 {
            CoUninitialize();
        }
        r
    };
    // ShellExecuteW returns a value greater than 32 on success.
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err("the default browser could not be started; use Copy link instead".into())
    }
}

#[cfg(all(not(windows), not(test)))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_authorize_url_rejects_untrusted_and_malformed_urls() {
        let ep = Endpoints::production();
        for bad in [
            "https://evil.example/api/accounts/authorize?x=1",
            "https://auth.openai.com/api/accounts/authorize",
            "https://auth.openai.com/api/accounts/authorize?a=1 b=2",
            "https://auth.openai.com/api/accounts/authorize?a=1\nb=2",
            "https://auth.openai.com/api/accounts/authorize?a=1\x07",
            "https://auth.openai.com/api/accounts/authorize?a=\"1\"",
            "javascript:alert(1)",
        ] {
            let err = open_authorize_url(&ep, bad).unwrap_err();
            assert!(err.contains("refusing to open"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_valid_sign_in_url_still_opens_nothing_in_unit_tests() {
        let ep = Endpoints::production();
        let err = open_authorize_url(&ep, "https://auth.openai.com/api/accounts/authorize?client_id=x").unwrap_err();
        assert_eq!(err, "test: browser opening disabled");
    }
}
