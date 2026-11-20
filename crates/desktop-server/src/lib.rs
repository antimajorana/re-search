//! Loopback `127.0.0.1` HTTP + SSE desktop server, UI approval broker, and
//! embedded Gemini-inspired research UI.

pub mod approver;
pub mod assets;
pub mod auth;
pub mod http_srv;
pub mod launcher;
pub mod routes_session;
pub mod routes_tools;
pub mod sse;
pub mod state;
pub mod turn_runner;

use auth::AuthGuard;
use http_srv::serve_loop;
use launcher::{emit_launch_json, launch_browser_window};
use research_tools::LiteratureEndpoints;
use serde::{Deserialize, Serialize};
use sse::SseHub;
use state::DesktopState;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

#[derive(Debug, Clone)]
pub struct DesktopConfig {
    pub port: u16,
    pub data_dir: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub python: Option<PathBuf>,
    pub worker_script: Option<PathBuf>,
    pub api_base_override: Option<String>,
    pub literature_endpoints: LiteratureEndpoints,
    pub open_window: bool,
    pub app_mode: bool,
    pub print_url_json: bool,
    pub eval_timeout_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerLaunchInfo {
    pub url: String,
    pub base_url: String,
    pub port: u16,
    pub token: String,
}

pub struct RunningDesktopServer {
    pub info: ServerLaunchInfo,
    pub addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    sse: Arc<SseHub>,
    handle: Option<JoinHandle<()>>,
}

impl RunningDesktopServer {
    /// Blocks until the HTTP accept loop terminates.
    pub fn wait(mut self) -> Result<(), String> {
        if let Some(h) = self.handle.take() {
            h.join()
                .map_err(|_| "desktop server accept thread panicked".to_string())?;
        }
        Ok(())
    }

    /// Signals the accept loop and active SSE streams to stop.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.sse.shutdown();
    }
}

impl Drop for RunningDesktopServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Binds a loopback HTTP + SSE desktop server on `127.0.0.1:<port>` (`0` for ephemeral)
/// and launches the background accept loop.
pub fn start_desktop_server(cfg: DesktopConfig) -> Result<RunningDesktopServer, String> {
    std::fs::create_dir_all(&cfg.data_dir)
        .map_err(|e| format!("failed to create data directory `{}`: {e}", cfg.data_dir.display()))?;
    if let Some(ws) = &cfg.workspace_root {
        if !ws.exists() || !ws.is_dir() {
            return Err(format!(
                "workspace directory `{}` does not exist or is not a directory",
                ws.display()
            ));
        }
        // Fail the launch, not each later request, if the workspace overlaps the data directory.
        research_tools::ResearchEnv::with_workspace(cfg.data_dir.clone(), ws).map_err(|e| e.to_string())?;
    }

    let listener = TcpListener::bind(("127.0.0.1", cfg.port))
        .map_err(|e| format!("failed to bind 127.0.0.1:{}: {e}", cfg.port))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to inspect bound address: {e}"))?;
    let actual_port = addr.port();

    let auth = AuthGuard::new(actual_port)?;
    let token = auth.token().to_string();
    let base_url = format!("http://127.0.0.1:{actual_port}");
    let url = format!("{base_url}/?token={token}");
    let info = ServerLaunchInfo {
        url: url.clone(),
        base_url,
        port: actual_port,
        token,
    };

    let sse = Arc::new(SseHub::new());
    let shutdown = Arc::new(AtomicBool::new(false));
    let state = Arc::new(DesktopState::new(cfg.clone(), auth, Arc::clone(&sse)));

    let loop_state = Arc::clone(&state);
    let loop_shutdown = Arc::clone(&shutdown);
    let handle = std::thread::Builder::new()
        .name("desktop-accept".to_string())
        .spawn(move || serve_loop(listener, loop_state, loop_shutdown))
        .map_err(|e| format!("failed to start desktop server thread: {e}"))?;

    if cfg.print_url_json {
        emit_launch_json(&info)?;
    }
    if cfg.open_window {
        launch_browser_window(&url, cfg.app_mode);
    }

    Ok(RunningDesktopServer {
        info,
        addr,
        shutdown,
        sse,
        handle: Some(handle),
    })
}

/// Starts the desktop server and blocks until shutdown (SIGINT/Ctrl-C or termination).
pub fn run_blocking(cfg: DesktopConfig) -> Result<i32, String> {
    let srv = start_desktop_server(cfg)?;
    srv.wait()?;
    Ok(0)
}
