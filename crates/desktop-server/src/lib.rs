//! Loopback `127.0.0.1` HTTP + SSE desktop server, UI approval broker, and
//! embedded Gemini-inspired research UI.

pub mod agent_models;
mod api_dispatch;
mod investigation;
pub mod investigation_cited;
mod investigation_cited_support;
mod investigation_cites;
pub mod investigation_finish;
pub mod investigation_handoff;
pub mod investigation_handoff_receive;
mod investigation_legacy_model;
pub mod investigation_legacy_view;
pub mod investigation_mapview;
pub mod investigation_mapview_v7;
mod investigation_mapview_v8;
pub mod investigation_notes;
pub mod investigation_open;
pub mod investigation_open_html;
mod investigation_question;
mod investigation_scope;
pub mod investigation_settings;
pub mod investigation_skeleton;
mod investigation_system_prompt;
pub mod investigation_task_context;
pub mod investigation_task_prompts;
pub mod investigation_tasks;
mod investigation_title;
mod investigation_view;
pub mod antigravity;
pub mod approver;
pub mod assets;
pub mod auth;
pub mod chatgpt;
mod cli_relay;
mod claim_review;
pub mod cli_turn;
pub mod connections;
mod explain_export;
mod evidence;
mod experiments;
mod project_context;
pub mod handoff;
pub mod hitl;
pub mod http_srv;
pub mod idea;
pub mod idea_view;
pub mod launcher;
mod manuscript_drafts;
pub mod model_catalog;
mod original_pdf;
pub mod paper_reader;
mod projects;
mod routers;
pub mod routes_idea;
pub mod routes_lessons;
pub mod routes_library;
pub mod routes_session;
pub mod routes_settings;
pub mod routes_sources;
pub mod routes_sources_ext;
pub mod routes_subagents;
pub mod routes_tools;
pub mod session_lifecycle;
pub mod session_mode;
mod session_folders;
mod session_summary;
mod session_view;
pub mod solve;
pub mod solve_view;
pub mod sse;
pub mod state;
pub mod subagents;
pub mod turn_control;
pub mod turn_runner;
mod upload_codec;
mod validation_plans;
pub mod workspace_ops;

use auth::AuthGuard;
use http_srv::serve_loop;
use launcher::{emit_launch_json, launch_browser_window};
use research_tools::LiteratureEndpoints;
use serde::{Deserialize, Serialize};
use sse::SseHub;
use state::DesktopState;
use std::collections::BTreeMap;
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
    pub print_url_json: bool,
    pub eval_timeout_s: u64,
    /// `core.exe` for the Claude subscription route; `None`: `RE_SEARCH_CORE_EXE`, then
    /// `core.exe` beside the running executable.
    pub core_exe: Option<PathBuf>,
    /// `claude.exe` for the Claude subscription route; `None`: `RE_SEARCH_CLAUDE`, `PATH`,
    /// then the npm install location.
    pub claude_exe: Option<PathBuf>,
    /// `agy.exe` for the Antigravity route; `None`: `RE_SEARCH_AGY`, `PATH`, then WinGet's
    /// link and package folders.
    pub agy_exe: Option<PathBuf>,
    /// Environment used by `GET /api/connections` and `GET /api/models` (provider key
    /// variables, the Claude CLI's location and the billing-safety variables). `None`: the
    /// process environment, which is what turns use. Tests pass a fixed map so the answer
    /// does not depend on the machine. A turn's billing-safety refusal always reads the
    /// process environment (what the CLI inherits) as well: this map can add variables to
    /// that check but never hide one.
    pub connection_env: Option<BTreeMap<String, String>>,
    /// Test-only: points Sign in with ChatGPT at a loopback mock and replaces the browser
    /// opener. `None` (always, outside tests): OpenAI's production endpoints.
    pub chatgpt_test: Option<chatgpt::ChatGptTestHooks>,
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
    state: Arc<DesktopState>,
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

    /// Signals the accept loop and active SSE streams to stop, cancels running API/ChatGPT
    /// turns, and kills the process tree of every running subscription-CLI turn.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.sse.shutdown();
        self.state.chatgpt.auth.cancel_login();
        turn_control::stop_all_turns(&self.state);
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
    let nonce = auth
        .launch_nonce()
        .ok_or_else(|| "launch nonce unavailable at startup".to_string())?;
    let base_url = format!("http://127.0.0.1:{actual_port}");
    let url = format!("{base_url}/?launch={nonce}");
    let info = ServerLaunchInfo {
        url: url.clone(),
        base_url,
        port: actual_port,
        token,
    };

    let sse = Arc::new(SseHub::new());
    let shutdown = Arc::new(AtomicBool::new(false));
    let chatgpt = chatgpt::ChatGptHub::new(&cfg)?;
    let state = Arc::new(DesktopState::new(cfg.clone(), auth, Arc::clone(&sse), chatgpt));
    // `mcp.json` files (they hold a turn's session token) left by a killed earlier run.
    cli_turn::sweep_all_tool_configs(&state);

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
        launch_browser_window(&url);
    }

    Ok(RunningDesktopServer {
        info,
        addr,
        shutdown,
        sse,
        state,
        handle: Some(handle),
    })
}

/// Starts the desktop server and blocks until shutdown (SIGINT/Ctrl-C or termination).
pub fn run_blocking(cfg: DesktopConfig) -> Result<i32, String> {
    let srv = start_desktop_server(cfg)?;
    srv.wait()?;
    Ok(0)
}
