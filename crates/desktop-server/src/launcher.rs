//! Native chromeless app-window launcher and readiness JSON stdout emitter.

use crate::ServerLaunchInfo;
use std::io::{self, Write};
use std::process::{Command, Stdio};

pub fn emit_launch_json(info: &ServerLaunchInfo) -> Result<(), String> {
    let line = serde_json::to_string(info).map_err(|e| e.to_string())?;
    let mut out = io::stdout().lock();
    writeln!(out, "{line}").and_then(|_| out.flush()).map_err(|e| e.to_string())
}

pub fn launch_browser_window(url: &str, app_mode: bool) {
    #[cfg(windows)]
    {
        launch_windows(url, app_mode);
    }
    #[cfg(not(windows))]
    {
        launch_unix(url, app_mode);
    }
}

#[cfg(windows)]
fn launch_windows(url: &str, app_mode: bool) {
    if app_mode {
        let app_arg = format!("--app={url}");
        for exe in [
            "msedge",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            "chrome",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        ] {
            if Command::new(exe)
                .arg(&app_arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
            {
                return;
            }
        }
    }
    let _ = Command::new("cmd")
        .args(["/C", "start", "", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(not(windows))]
fn launch_unix(url: &str, app_mode: bool) {
    let has_display = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    if !has_display {
        return;
    }
    if app_mode {
        let app_arg = format!("--app={url}");
        for exe in ["google-chrome", "chromium", "chromium-browser", "microsoft-edge"] {
            if Command::new(exe)
                .arg(&app_arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
            {
                return;
            }
        }
    }
    let _ = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
