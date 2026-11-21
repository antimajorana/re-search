//! `re-search-app`: re:Search in its own window.
//!
//! One process runs the loopback desktop server (`crates/desktop-server`) and a single
//! WebView2 window (wry + tao) pointed at it. The per-launch token reaches the window in a
//! request header (or, if WebView2 drops the header, through a same-origin script); it is
//! never put into a URL or onto a process command line. See `delivery.rs`.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg_attr(not(windows), allow(dead_code))]
mod config;
#[cfg_attr(not(windows), allow(dead_code))]
mod delivery;
#[cfg_attr(not(windows), allow(dead_code))]
mod rules;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod win32;

#[cfg(windows)]
fn startup() -> Result<(), String> {
    let args = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().map_err(|a| format!("argument is not valid Unicode: {}", a.to_string_lossy())))
        .collect::<Result<Vec<String>, String>>()?;
    let flags = config::parse_flags(args)?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate re-search-app.exe: {e}"))?;
    let exe_dir = exe.parent().ok_or("re-search-app.exe has no parent directory")?;
    let file = config::load_file_config(exe_dir)?;
    let env = |name: &str| std::env::var_os(name);
    let cfg = config::resolve(&flags, &env, file.as_ref(), exe_dir)?;
    let webview_dir = config::webview_data_dir(&env)?;
    config::check_webview_dir(&webview_dir, &cfg)?;
    app::run(cfg, &webview_dir)
}

#[cfg(windows)]
fn main() {
    // Before any window exists, so the window and taskbar button share one identity.
    win32::set_app_user_model_id();
    // A second launch leaves the running one alone and exits quietly.
    let Some(_instance) = win32::acquire_single_instance() else {
        return;
    };
    if let Err(e) = startup() {
        win32::error_box(&format!("re:Search could not start.\n\n{e}"));
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "re-search-app: the desktop window needs Windows and the WebView2 runtime. \
         On this platform run `re-search ui` and open the printed address in a browser."
    );
    std::process::exit(2);
}
