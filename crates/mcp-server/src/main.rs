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
//!   CORE_CHECK_SCRATCH, CORE_CHECK_PYCACHE, CORE_CHECK_TIMEOUT_S Solve check worker
//!   CORE_EVAL_TIMEOUT_S                                  default 120
//!   CORE_TOOL_PROFILE                                    frozen tool profile (e.g. `solve/1`)
//!   CORE_WORKSPACE, CORE_ENABLE_RESEARCH_TOOLS           research tools + `v0.4-research`
//!                                                        (also when sessions/<id>/sources exists)
//!   RE_SEARCH_{ARXIV,INSPIRE,CROSSREF,S2,OPENALEX}_BASE  literature endpoint overrides

use mcp_server::approver::{DenyAll, TcpApprover};
use mcp_server::solve_wire::{
    self, build_check_worker, build_solve_router_with_bindings, parse_check_timeout,
    solve_sympy_handler, HarvestPreviews, McpCheckRunner, WorkerEpochAdapter,
};
use research_tools::tool_profile::{PROFILE_EXPLORE_V1, PROFILE_SOLVE_V1};
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
use worker::Worker;

const DEFAULT_CORE_INSTRUCTIONS: &str =
    "Local re:Search core. `sympy_eval` runs Python/SymPy after the user approves it \
     (each call when uncontained; once per live worker session when sandboxed). \
     `ledger_note` records an unverified note. Results are computations, not verified claims.";

/// Explore `explore/1` MCP instructions (`explore.md` 5.2, Rule S8: no code-exec tools, no "verified").
const EXPLORE_CORE_INSTRUCTIONS: &str =
    "Local re:Search Explore core (`explore/1`). Available tools: `idea_search` and \
     `idea_fetch_papers` (network reads requiring approval), plus `search_paper_passages` and \
     `read_paper_lines` (read-only access to ingested session papers). Cite only lines returned \
     by these tools; quotes are checked by code and shown as matched.";

struct CoreServer {
    router: Arc<Router>,
    /// Who is on the other end of stdin/stdout. Fixed when the process
    /// starts; never taken from a request.
    caller: String,
    /// Optional Solve handle-harvest preview store (`Some` only on `solve/1`).
    harvest_previews: Option<HarvestPreviews>,
    /// Profile-appropriate MCP server instructions.
    instructions: &'static str,
}

impl ServerHandler for CoreServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("core", env!("CARGO_PKG_VERSION")))
            .with_instructions(self.instructions)
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
        let previews = self.harvest_previews.clone();
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        // The router blocks (approval prompt, worker), so keep it off the
        // async thread that serves the MCP stream.
        let outcome = tokio::task::spawn_blocking(move || {
            let mut outcome = router.call(&caller, &name, &args);
            if name == "sympy_eval" {
                if let (Some(previews), CallOutcome::Ok(val)) = (&previews, &mut outcome) {
                    previews.attach_to_result(val);
                }
            }
            outcome
        })
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
            job_memory_limit_bytes: None,
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

fn build_research_env(
    data_dir: &std::path::Path,
    workspace_root: Option<PathBuf>,
) -> Result<Arc<research_tools::ResearchEnv>, String> {
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
    if let Some(u) = env("RE_SEARCH_OPENALEX_BASE") {
        endpoints.openalex_api_base = u.trim_end_matches('/').to_string();
    }
    let r_env = match workspace_root {
        Some(ws) => research_tools::ResearchEnv::with_workspace(data_dir.to_path_buf(), ws)
            .map_err(|e| format!("invalid CORE_WORKSPACE: {e}"))?,
        None => research_tools::ResearchEnv::new(data_dir.to_path_buf()),
    }
    .with_endpoints(endpoints);
    Ok(Arc::new(r_env))
}

fn resolve_profile_name(
    explicit_profile: Option<String>,
    ledger: &ledger::Ledger,
    session: &str,
) -> Result<String, String> {
    if let Some(name) = explicit_profile {
        return Ok(name);
    }
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    // Subscription-CLI callers that do not pass `CORE_TOOL_PROFILE` expect the
    // legacy profile list until `cli_turn.rs` passes `CORE_TOOL_PROFILE` explicitly.
    let explicit = events
        .iter()
        .find(|e| e.kind == "session.start")
        .and_then(|e| e.payload.get(research_tools::tool_profile::TOOL_PROFILE_FIELD));
    if explicit.is_some() {
        Ok(research_tools::tool_profile::active_profile_for_session_events(&events)?
            .name
            .to_string())
    } else {
        let legacy = research_tools::tool_profile::legacy_profile_name_for_events(&events);
        Ok(research_tools::tool_profile::lookup(legacy)
            .map_err(|e| e.to_string())?
            .name
            .to_string())
    }
}

fn session_has_mode_profile(ledger: &ledger::Ledger, session: &str) -> bool {
    let Ok(events) = ledger.read_session(session) else {
        return false;
    };
    events.iter().any(|e| {
        matches!(
            e.kind.as_str(),
            "idea.started" | "solve.started" | "investigation.started"
        ) || (e.kind == "session.start"
            && e.payload
                .get(research_tools::tool_profile::TOOL_PROFILE_FIELD)
                .is_some())
    })
}

fn build_router() -> Result<(Router, Option<HarvestPreviews>), String> {
    let session = env("CORE_SESSION").ok_or("CORE_SESSION is not set")?;
    let data_dir = PathBuf::from(env("CORE_DATA_DIR").ok_or("CORE_DATA_DIR is not set")?);
    let ledger = ledger::Ledger::open(&data_dir).map_err(|e| e.to_string())?;

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
    let explicit_profile = env(cli_route::core_tools::CORE_TOOL_PROFILE_ENV)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let use_profile = workspace_root.is_some()
        || sources_exist
        || enable_flag
        || explicit_profile.is_some()
        || session_has_mode_profile(&ledger, &session);
    let resolved_profile = if use_profile {
        Some(resolve_profile_name(explicit_profile, &ledger, &session)?)
    } else {
        None
    };

    let timeout = Duration::from_secs(env("CORE_EVAL_TIMEOUT_S").and_then(|s| s.parse().ok()).unwrap_or(120));
    let is_explore = resolved_profile.as_deref() == Some(PROFILE_EXPLORE_V1);
    let python_cfg = (!is_explore)
        .then(|| env("CORE_PYTHON").zip(env("CORE_WORKER_SCRIPT")))
        .flatten();
    let worker = match &python_cfg {
        Some((python, script)) => Some(build_worker(&data_dir, &session, python.clone(), script.clone())?),
        None => None,
    };
    let epoch_handle = worker.as_ref().map(Worker::epoch_handle);
    let solver_contained = worker.as_ref().is_some_and(|w| w.containment().is_contained());
    let solver_containment = match &worker {
        Some(w) => w.containment().summary(),
        None => "no compute worker configured".to_string(),
    };
    let worker = Arc::new(Mutex::new(worker));

    let sympy_epoch = Arc::new(WorkerEpochAdapter { epoch: epoch_handle, contained: solver_contained });

    if let Some(profile_name) = resolved_profile {
        let r_env = build_research_env(&data_dir, workspace_root)?;

        if profile_name == PROFILE_SOLVE_V1 {
            let check_worker = match &python_cfg {
                Some((python, script)) => build_check_worker(
                    &data_dir,
                    &session,
                    python,
                    script,
                    env("CORE_CHECK_SCRATCH"),
                    env("CORE_CHECK_PYCACHE"),
                )?,
                None => None,
            };
            let check_epoch_handle = check_worker.as_ref().map(Worker::epoch_handle);
            let check_contained = check_worker.as_ref().is_some_and(|w| w.containment().is_contained());
            let contained = solver_contained && check_contained;
            let containment = match &check_worker {
                Some(cw) => format!("{solver_containment}; check: {}", cw.containment().summary()),
                None => format!("{solver_containment}; check: not configured"),
            };
            let approver: Arc<dyn tools::Approver> = match (env("CORE_APPROVAL_ADDR"), env("CORE_SESSION_TOKEN")) {
                (Some(addr), Some(token)) => {
                    Arc::new(TcpApprover { addr, token, containment, contained, wait: Duration::from_secs(900) })
                }
                _ => Arc::new(DenyAll),
            };
            let check_timeout = parse_check_timeout(env("CORE_CHECK_TIMEOUT_S"));
            let check_runner = Arc::new(McpCheckRunner::new(check_worker, check_timeout));
            let check_epoch = Arc::new(WorkerEpochAdapter { epoch: check_epoch_handle, contained: check_contained });
            let sympy_handler = solve_sympy_handler(worker, data_dir.clone(), timeout);
            let wired = build_solve_router_with_bindings(
                r_env,
                ledger,
                &session,
                sympy_epoch,
                sympy_handler,
                check_epoch,
                check_runner.clone(),
                check_runner,
                approver,
            )?;
            return Ok((wired.router, Some(wired.previews)));
        }

        let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let code = args["code"].as_str().ok_or("`code` must be a string")?;
            let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
            let w = guard.as_mut().ok_or("no compute worker is configured for this session")?;
            let (result, ran_epoch) = w.evaluate_in_epoch(code, timeout, ctx.required_epoch).map_err(|e| e.to_string())?;
            ctx.record_worker_execution(ran_epoch, w.last_request_id());
            if let Some(hello) = w.hello() {
                ctx.record_worker_environment(hello);
            }
            Ok(result)
        });
        let mut registry = Registry::new();
        let policy = cli_route::core_tools::register_core_tools_for_profile(
            &mut registry,
            sympy_epoch,
            sympy_handler,
            r_env,
            &profile_name,
        )?;
        let approver: Arc<dyn tools::Approver> = match (env("CORE_APPROVAL_ADDR"), env("CORE_SESSION_TOKEN")) {
            (Some(addr), Some(token)) => Arc::new(TcpApprover {
                addr,
                token,
                containment: solver_containment,
                contained: solver_contained,
                wait: Duration::from_secs(900),
            }),
            _ => Arc::new(DenyAll),
        };
        let router = Router::new(registry, policy, ledger, &session, approver).with_tool_profile(profile_name);
        return Ok((router, None));
    }

    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args["code"].as_str().ok_or("`code` must be a string")?;
        let mut guard = worker.lock().map_err(|_| "worker lock poisoned".to_string())?;
        let w = guard.as_mut().ok_or("no compute worker is configured for this session")?;
        let (result, ran_epoch) = w.evaluate_in_epoch(code, timeout, ctx.required_epoch).map_err(|e| e.to_string())?;
        ctx.record_worker_execution(ran_epoch, w.last_request_id());
        if let Some(hello) = w.hello() {
            ctx.record_worker_environment(hello);
        }
        Ok(result)
    });
    let mut registry = Registry::new();
    let policy = cli_route::core_tools::register_core_tools(&mut registry, sympy_epoch, sympy_handler, None);
    let approver: Arc<dyn tools::Approver> = match (env("CORE_APPROVAL_ADDR"), env("CORE_SESSION_TOKEN")) {
        (Some(addr), Some(token)) => Arc::new(TcpApprover {
            addr,
            token,
            containment: solver_containment,
            contained: solver_contained,
            wait: Duration::from_secs(900),
        }),
        _ => Arc::new(DenyAll),
    };
    let _ = solve_wire::parse_check_timeout;
    Ok((Router::new(registry, policy, ledger, &session, approver), None))
}

/// Resolve the fixed caller identity for this server process: `antigravity:<session>` when
/// `CORE_CALLER_ROUTE` is `"antigravity"`, and `claude-code-cli:<session>` otherwise.
fn resolve_caller(session: &str, route_env: Option<&str>) -> String {
    let route = match route_env {
        Some("antigravity") => "antigravity",
        _ => "claude-code-cli",
    };
    format!("{route}:{session}")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (router, harvest_previews) = match build_router() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("core: cannot start: {e}");
            std::process::exit(2);
        }
    };
    let instructions = if router.tool_profile() == Some(PROFILE_EXPLORE_V1) {
        EXPLORE_CORE_INSTRUCTIONS
    } else {
        DEFAULT_CORE_INSTRUCTIONS
    };
    let router = Arc::new(router);
    // The route that started this server (it labels every tool call and is part of each
    // approval digest): `CORE_CALLER_ROUTE=antigravity` on the Antigravity route; anything
    // else, or nothing, is the Claude Code route as before.
    let caller = resolve_caller(router.session(), std::env::var("CORE_CALLER_ROUTE").ok().as_deref());
    let server = CoreServer { router, caller, harvest_previews, instructions };
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

#[cfg(test)]
mod tests {
    use super::resolve_caller;

    #[test]
    fn resolve_caller_maps_known_routes_and_defaults_unknown() {
        assert_eq!(resolve_caller("s-1", Some("antigravity")), "antigravity:s-1");
        assert_eq!(resolve_caller("s-1", Some("claude-code-cli")), "claude-code-cli:s-1");
        assert_eq!(resolve_caller("s-1", None), "claude-code-cli:s-1");
        assert_eq!(resolve_caller("s-1", Some("")), "claude-code-cli:s-1");
        assert_eq!(resolve_caller("s-1", Some("something-else")), "claude-code-cli:s-1");
    }
}
