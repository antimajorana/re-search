//! `core`: the stdio MCP server that exposes re:Search core tools to an
//! external agent loop (v0: the Claude Code CLI).
//!
//! stdout carries MCP and nothing else. Diagnostics go to stderr. Every tool
//! call goes through `tools::Router` (policy, approval, ledger).
//!
//! Configuration comes from environment variables set by the supervisor in
//! the per-session MCP config:
//!   CORE_SESSION, CORE_DATA_DIR                          required
//!   CORE_APPROVAL_ADDR, CORE_SESSION_TOKEN               approval channel (else: deny)
//!   CORE_PYTHON, CORE_WORKER_SCRIPT, CORE_SCRATCH, CORE_PYCACHE  compute worker
//!   CORE_EVAL_TIMEOUT_S                                  default 120
//!   CORE_WORKSPACE, CORE_ENABLE_RESEARCH_TOOLS           research tools + `v0.4-research`
//!                                                        (also when sessions/<id>/sources exists)
//!   RE_SEARCH_{ARXIV,INSPIRE,CROSSREF,S2}_BASE           literature endpoint overrides

mod approver;

use approver::{DenyAll, TcpApprover};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{CallCtx, CallOutcome, Registry, Router, ToolHandler};
#[cfg(windows)]
use worker::appcontainer::AppContainerSandbox;
#[cfg(windows)]
use worker::sandbox::Sandbox;
#[cfg(windows)]
use worker::WorkerConfig;
use worker::{EpochHandle, Worker};

struct WorkerEpochAdapter {
    epoch: Option<EpochHandle>,
    contained: bool,
}

impl tools::WorkerEpoch for WorkerEpochAdapter {
    fn live_epoch(&self) -> Option<u64> {
        self.epoch.as_ref().and_then(EpochHandle::live)
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

struct CoreServer {
    router: Arc<Router>,
    /// Who is on the other end of stdin/stdout. Fixed when the process
    /// starts; never taken from a request.
    caller: String,
}

impl ServerHandler for CoreServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("core", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Local re:Search core. `sympy_eval` runs Python/SymPy after the user approves it \
                 (each call when uncontained; once per live worker session when sandboxed). \
                 `ledger_note` records an unverified note. Results are computations, not verified claims.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self
            .router
            .registry()
            .defs()
            .into_iter()
            .map(|d| {
                let schema = d.input_schema.as_object().cloned().unwrap_or_default();
                Tool::new(d.id.clone(), d.description.clone(), Arc::new(schema))
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let router = self.router.clone();
        let caller = self.caller.clone();
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        // The router blocks (approval prompt, worker), so keep it off the
        // async thread that serves the MCP stream.
        let outcome = tokio::task::spawn_blocking(move || router.call(&caller, &name, &args))
            .await
            .map_err(|e| McpError::internal_error(format!("tool task failed: {e}"), None))?;
        let result = match outcome {
            CallOutcome::Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            CallOutcome::Denied(why) => CallToolResult::error(vec![ContentBlock::text(format!("DENIED: {why}"))]),
            CallOutcome::Error(why) => CallToolResult::error(vec![ContentBlock::text(format!("ERROR: {why}"))]),
        };
        Ok(result.into())
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[cfg(windows)]
fn build_worker(data_dir: &std::path::Path, session: &str, python: String, script: String) -> Result<Worker, String> {
    let scratch =
        PathBuf::from(env("CORE_SCRATCH").ok_or("CORE_SCRATCH must be set when CORE_PYTHON is configured")?);
    let pycache =
        PathBuf::from(env("CORE_PYCACHE").ok_or("CORE_PYCACHE must be set when CORE_PYTHON is configured")?);
    let sandbox: Box<dyn Sandbox> = Box::new(
        AppContainerSandbox::new(
            data_dir.to_path_buf(),
            session,
            PathBuf::from(&python),
            PathBuf::from(&script),
            scratch.clone(),
            pycache.clone(),
        )
        .map_err(|e| format!("AppContainer sandbox setup failed: {e}"))?,
    );
    Ok(Worker::new(
        WorkerConfig {
            python: PathBuf::from(python),
            script: PathBuf::from(script),
            scratch,
            pycache,
            startup_timeout: Duration::from_secs(90),
        },
        sandbox,
    ))
}

#[cfg(not(windows))]
fn build_worker(_data_dir: &std::path::Path, _session: &str, _python: String, _script: String) -> Result<Worker, String> {
    let _ = env("CORE_SCRATCH").ok_or("CORE_SCRATCH must be set when CORE_PYTHON is configured")?;
    let _ = env("CORE_PYCACHE").ok_or("CORE_PYCACHE must be set when CORE_PYTHON is configured")?;
    Err("AppContainer sandbox requires Windows".to_string())
}

fn build_router() -> Result<Router, String> {
    let session = env("CORE_SESSION").ok_or("CORE_SESSION is not set")?;
    let data_dir = PathBuf::from(env("CORE_DATA_DIR").ok_or("CORE_DATA_DIR is not set")?);
    let ledger = ledger::Ledger::open(&data_dir).map_err(|e| e.to_string())?;

    let timeout = Duration::from_secs(env("CORE_EVAL_TIMEOUT_S").and_then(|s| s.parse().ok()).unwrap_or(120));
    let worker = match (env("CORE_PYTHON"), env("CORE_WORKER_SCRIPT")) {
        (Some(python), Some(script)) => Some(build_worker(&data_dir, &session, python, script)?),
        _ => None,
    };
    let epoch_handle = worker.as_ref().map(Worker::epoch_handle);
    let contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let containment = match &worker {
        Some(w) => w.containment().summary(),
        None => "no compute worker configured".to_string(),
    };
    let worker = Arc::new(Mutex::new(worker));

    let workspace_root = env("CORE_WORKSPACE")
        .or_else(|| env("RE_SEARCH_WORKSPACE"))
        .map(PathBuf::from);
    if let Some(ws) = &workspace_root {
        if !ws.exists() || !ws.is_dir() {
            return Err(format!("CORE_WORKSPACE `{}` does not exist or is not a directory", ws.display()));
        }
    }
    let sources_exist = research_tools::validate_session_id(&session).is_ok()
        && data_dir.join("sessions").join(&session).join("sources").exists();
    let enable_flag = env("CORE_ENABLE_RESEARCH_TOOLS")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let use_research = workspace_root.is_some() || sources_exist || enable_flag;

    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args["code"].as_str().ok_or("`code` must be a string")?;
        let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
        let w = guard.as_mut().ok_or("no compute worker is configured for this session")?;
        let (result, ran_epoch) = w.evaluate_in_epoch(code, timeout, ctx.required_epoch).map_err(|e| e.to_string())?;
        ctx.record_worker_epoch(ran_epoch);
        Ok(result)
    });
    let sympy_epoch = Arc::new(WorkerEpochAdapter { epoch: epoch_handle, contained });

    // The same registration the supervisor uses to compute the tool list it expects the
    // agent CLI to offer (`cli_route::core_tools::core_tool_set`).
    let research = if use_research {
        let mut endpoints = research_tools::LiteratureEndpoints::default();
        if let Some(u) = env("RE_SEARCH_ARXIV_BASE") {
            endpoints.arxiv_api_base = u.trim_end_matches('/').to_string();
            endpoints.arxiv_eprint_base = endpoints.arxiv_api_base.clone();
        }
        if let Some(u) = env("RE_SEARCH_INSPIRE_BASE") {
            endpoints.inspire_api_base = u.trim_end_matches('/').to_string();
        }
        if let Some(u) = env("RE_SEARCH_CROSSREF_BASE") {
            endpoints.crossref_api_base = u.trim_end_matches('/').to_string();
        }
        if let Some(u) = env("RE_SEARCH_S2_BASE").or_else(|| env("RE_SEARCH_SEMANTIC_SCHOLAR_BASE")) {
            endpoints.semantic_scholar_api_base = u.trim_end_matches('/').to_string();
        }
        let r_env = match workspace_root {
            Some(ws) => research_tools::ResearchEnv::with_workspace(data_dir.clone(), ws)
                .map_err(|e| format!("invalid CORE_WORKSPACE: {e}"))?,
            None => research_tools::ResearchEnv::new(data_dir.clone()),
        }
        .with_endpoints(endpoints);
        Some(Arc::new(r_env))
    } else {
        None
    };
    let mut registry = Registry::new();
    let policy = cli_route::core_tools::register_core_tools(&mut registry, sympy_epoch, sympy_handler, research);

    let approver: Arc<dyn tools::Approver> = match (env("CORE_APPROVAL_ADDR"), env("CORE_SESSION_TOKEN")) {
        (Some(addr), Some(token)) => {
            Arc::new(TcpApprover { addr, token, containment, contained, wait: Duration::from_secs(900) })
        }
        _ => Arc::new(DenyAll),
    };
    Ok(Router::new(registry, policy, ledger, &session, approver))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let router = match build_router() {
        Ok(r) => Arc::new(r),
        Err(e) => {
            eprintln!("core: cannot start: {e}");
            std::process::exit(2);
        }
    };
    let caller = format!("claude-code-cli:{}", router.session());
    let server = CoreServer { router, caller };
    match server.serve(rmcp::transport::io::stdio()).await {
        Ok(running) => {
            if let Err(e) = running.waiting().await {
                eprintln!("core: server task ended abnormally: {e}");
            }
        }
        Err(e) => {
            eprintln!("core: MCP initialisation failed: {e}");
            std::process::exit(1);
        }
    }
    // Dropping the router drops the worker, which closes its job and kills
    // the Python process tree.
}
