//! One process: the loopback desktop server on a background thread and a tao event loop
//! with one locked-down wry/WebView2 window on the main thread.

use crate::config::AppConfig;
use crate::delivery::{fallback_script, probe_script, Action, Delivery, Input, TokenPath};
use crate::rules;
use desktop_server::{start_desktop_server, DesktopConfig, RunningDesktopServer};
use research_tools::LiteratureEndpoints;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::WindowBuilderExtWindows;
use tao::window::{Icon, WindowBuilder};
use wry::http::{HeaderMap, HeaderName, HeaderValue};
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebView, WebViewBuilder};

static ICON_32: &[u8] = include_bytes!("../assets/icon-32.rgba");
static ICON_256: &[u8] = include_bytes!("../assets/icon-256.rgba");

pub const WINDOW_TITLE: &str = "reSearch";
const FIRST_LOAD_TIMEOUT: Duration = Duration::from_secs(10);
const STEP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_EVAL_TIMEOUT_S: u64 = 120;

#[derive(Debug)]
enum UserEvent {
    PageFinished(String),
    Probe(String),
}

fn icon(rgba: &[u8], side: u32) -> Option<Icon> {
    Icon::from_rgba(rgba.to_vec(), side, side).ok()
}

fn start_server(cfg: &AppConfig) -> Result<RunningDesktopServer, String> {
    start_desktop_server(DesktopConfig {
        port: cfg.port,
        data_dir: cfg.data_dir.clone(),
        workspace_root: cfg.workspace.clone(),
        python: cfg.python.clone(),
        worker_script: cfg.worker_script.clone(),
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: DEFAULT_EVAL_TIMEOUT_S,
        core_exe: cfg.core_exe.clone(),
        claude_exe: cfg.claude.clone(),
        agy_exe: cfg.agy.clone(),
        connection_env: None,
        chatgpt_test: None,
    })
}

/// Allows only this server's session export URLs, saved into Downloads under a fresh name.
fn allow_download(url: &str, path: &mut PathBuf, base_url: &str) -> bool {
    if !rules::is_allowed_export(url, base_url) {
        return false;
    }
    let Some(dir) = rules::downloads_dir(std::env::var_os("USERPROFILE")) else {
        return false;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let suggested = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match rules::unique_download_path(&dir, &rules::sanitize_file_name(&suggested)) {
        Some(target) => {
            *path = target;
            true
        }
        None => false,
    }
}

fn build_webview(
    window: &tao::window::Window,
    context: &mut WebContext,
    base_url: &str,
    token: &str,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<WebView, String> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(token).map_err(|_| "launch token is not a valid header value".to_string())?;
    headers.insert(HeaderName::from_static("x-research-token"), value);

    let nav_base = base_url.to_string();
    let dl_base = base_url.to_string();
    WebViewBuilder::new_with_web_context(context)
        // InPrivate profile: intended to keep cookies, sessionStorage and cache session-only
        // (see THREAT-MODEL.md for what is and is not established about that).
        .with_incognito(true)
        // DevTools would show the token; only on explicit request, never by default.
        .with_devtools(std::env::var_os("RE_SEARCH_DEVTOOLS").is_some_and(|v| v == "1"))
        .with_url_and_headers(format!("{base_url}/"), headers)
        .with_navigation_handler(move |url| rules::is_allowed_navigation(&url, &nav_base))
        .with_new_window_req_handler(|url, _| {
            if rules::is_allowed_external_source_url(&url) {
                crate::win32::open_external_source_url(&url);
            }
            NewWindowResponse::Deny
        })
        .with_download_started_handler(move |url, path| allow_download(&url, path, &dl_base))
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = proxy.send_event(UserEvent::PageFinished(url));
            }
        })
        .build(window)
        .map_err(|e| format!("could not create the WebView2 window (is the WebView2 runtime installed?): {e}"))
}

fn perform(
    action: Action,
    webview: &WebView,
    window: &tao::window::Window,
    base_url: &str,
    token: &str,
    proxy: &EventLoopProxy<UserEvent>,
) {
    match action {
        Action::None => {}
        Action::Probe => {
            let p = proxy.clone();
            let _ = webview.evaluate_script_with_callback(&probe_script(base_url), move |raw| {
                let answer = serde_json::from_str::<String>(&raw).unwrap_or(raw);
                let _ = p.send_event(UserEvent::Probe(answer));
            });
        }
        Action::LoadBase => {
            let _ = webview.load_url(&format!("{base_url}/"));
        }
        Action::RunFallback => {
            let _ = webview.evaluate_script(&fallback_script(base_url, token));
        }
        Action::Authenticated(path) => {
            let how = match path {
                TokenPath::Header => "request header",
                TokenPath::Fallback => "fallback script",
            };
            eprintln!("re-search-app: window authenticated via {how}");
        }
        Action::Failed(why) => {
            eprintln!("re-search-app: could not hand the session token to the window: {why}");
            window.set_title(&format!("{WINDOW_TITLE} (not connected)"));
        }
    }
}

/// Starts the server, opens the window and runs the event loop. Returns only on a startup
/// error; closing the window shuts the server down and exits the process.
pub fn run(cfg: AppConfig, webview_dir: &Path) -> Result<(), String> {
    let server = start_server(&cfg)?;
    let base_url = server.info.base_url.clone();
    let token = server.info.token.clone();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title(WINDOW_TITLE)
        .with_window_icon(icon(ICON_32, 32))
        .with_taskbar_icon(icon(ICON_256, 256))
        .with_inner_size(LogicalSize::new(1360.0, 900.0))
        .build(&event_loop)
        .map_err(|e| format!("could not create the window: {e}"))?;

    std::fs::create_dir_all(webview_dir)
        .map_err(|e| format!("could not create the WebView2 folder `{}`: {e}", webview_dir.display()))?;
    let mut context = WebContext::new(Some(webview_dir.to_path_buf()));
    let webview = build_webview(&window, &mut context, &base_url, &token, proxy.clone())?;

    let mut server = Some(server);
    let mut delivery = Delivery::new(base_url.clone());
    let mut deadline = Instant::now() + FIRST_LOAD_TIMEOUT;
    let mut closing = false;

    event_loop.run(move |event, _, control_flow| {
        // Keep the WebView2 context alive for as long as the loop runs.
        let _ = &context;
        if closing {
            return;
        }
        let input = match &event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                closing = true;
                if let Some(srv) = server.take() {
                    srv.shutdown();
                }
                *control_flow = ControlFlow::ExitWithCode(0);
                return;
            }
            Event::UserEvent(UserEvent::PageFinished(url)) => Some(Input::PageFinished(url.as_str())),
            Event::UserEvent(UserEvent::Probe(answer)) => Some(Input::ProbeResult(answer.as_str())),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => Some(Input::Timeout),
            _ => None,
        };
        if let Some(input) = input {
            let action = delivery.step(input);
            if action != Action::None {
                deadline = Instant::now() + STEP_TIMEOUT;
            }
            perform(action, &webview, &window, &base_url, &token, &proxy);
        }
        *control_flow = if delivery.is_done() { ControlFlow::Wait } else { ControlFlow::WaitUntil(deadline) };
    })
}
