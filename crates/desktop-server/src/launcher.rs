//! Default-browser launcher for `re-search ui` and the readiness JSON stdout emitter.
//!
//! `re-search ui` passes only a single-use, short-lived `?launch=<nonce>` URL to the default
//! browser — never the live API token. The desktop window is the separate `re-search-app`
//! binary (`crates/desktop-app`), which delivers the token in a request header.

use crate::ServerLaunchInfo;
use std::io::{self, Write};
#[cfg(not(test))]
use std::process::{Command, Stdio};

pub fn emit_launch_json(info: &ServerLaunchInfo) -> Result<(), String> {
    let line = serde_json::to_string(info).map_err(|e| e.to_string())?;
    let mut out = io::stdout().lock();
    writeln!(out, "{line}").and_then(|_| out.flush()).map_err(|e| e.to_string())
}

/// Opens `url` in the user's default browser (`re-search ui`).
///
/// Compiled out under `#[cfg(test)]` so unit tests can never invoke an OS browser launcher.
#[cfg(test)]
pub fn launch_browser_window(_url: &str) {}

/// Opens `url` in the user's default browser (`re-search ui`).
#[cfg(not(test))]
pub fn launch_browser_window(url: &str) {
    #[cfg(windows)]
    {
        let _ = Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let has_display = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if !has_display {
            return;
        }
        let _ = Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
