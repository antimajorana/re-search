//! Isolated worker delegation (`delegate_compute`, `delegate_literature`, `delegate_lean`,
//! `delegate_explore`, `delegate_subagent`).
//!
//! Each delegated worker runs in an isolated context with an explicit tool whitelist,
//! is barred in Rust from recursive delegation or recording `final_answer`, and logs every
//! turn via `delegate.start` / `delegate.turn` / `delegate.end` without polluting Orchestrator
//! chat history. All tool calls still flow through `tools::Router::try_call`.

use crate::explore::{explore_tool_spec, run_parallel_exploration, DELEGATE_EXPLORE};
use crate::runner::{validate_turn, Audit};
use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, Role, StopReason, StreamObserver, ToolSpec,
    Transport, TransportErrorKind,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Instant;
use tools::{CallOutcome, Router, ToolDef};

pub const DELEGATE_COMPUTE: &str = "delegate_compute";
pub const DELEGATE_LITERATURE: &str = "delegate_literature";
pub const DELEGATE_LEAN: &str = "delegate_lean";
pub const DELEGATE_SUBAGENT: &str = "delegate_subagent";
pub const DEFAULT_WORKER_MAX_TURNS: u32 = 8;
pub const MAX_WORKER_MAX_TURNS: u32 = 12;
pub const WORKER_MAX_TOKENS: u32 = 8_192;
const DELEGATE_RETURN_SUMMARY_CHARS: usize = 2_800;
const WORKER_TOOL_OUTPUT_CHARS: usize = 8_000;
const MAX_RETURN_JSON_LEN: usize = 3_800;

pub struct DelegatedWorkerSpec<'a> {
    pub role: String,
    pub display_name: String,
    pub transport: &'a dyn Transport,
    pub model: String,
    pub routing_reason: String,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub reasoning_effort: Option<String>,
}

pub struct CustomSubagentDelegation<'a> {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tools_profile: String,
    pub system_prompt: String,
    pub transport: &'a dyn Transport,
    pub model: String,
    pub routing_reason: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Default)]
pub struct DelegationConfig<'a> {
    pub compute_worker: Option<DelegatedWorkerSpec<'a>>,
    pub literature_worker: Option<DelegatedWorkerSpec<'a>>,
    pub lean_worker: Option<DelegatedWorkerSpec<'a>>,
    pub lean_router: Option<&'a Router>,
    pub explorer_worker: Option<DelegatedWorkerSpec<'a>>,
    pub custom_subagents: Vec<CustomSubagentDelegation<'a>>,
    pub pinned_conventions: Option<String>,
}

pub(crate) struct CancelOnlyObserver<'a>(pub(crate) Option<&'a CancelToken>);
impl StreamObserver for CancelOnlyObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_some_and(CancelToken::is_cancelled)
    }
}

struct TargetWorker<'a> {
    role: String,
    display_name: String,
    transport: &'a dyn Transport,
    model: String,
    routing_reason: String,
    system_prompt: String,
    allowed_tools: Vec<String>,
    max_turns: u32,
    max_tokens: u32,
    reasoning_effort: Option<String>,
}

impl<'a> TargetWorker<'a> {
    fn from_spec(w: &DelegatedWorkerSpec<'a>) -> Self {
        Self {
            role: w.role.clone(),
            display_name: w.display_name.clone(),
            transport: w.transport,
            model: w.model.clone(),
            routing_reason: w.routing_reason.clone(),
            system_prompt: w.system_prompt.clone(),
            allowed_tools: w.allowed_tools.clone(),
            max_turns: w.max_turns,
            max_tokens: w.max_tokens,
            reasoning_effort: w.reasoning_effort.clone(),
        }
    }
}

pub fn is_delegation_tool(name: &str) -> bool {
    matches!(
        name,
        DELEGATE_COMPUTE | DELEGATE_LITERATURE | DELEGATE_LEAN | DELEGATE_EXPLORE | DELEGATE_SUBAGENT
    )
}

pub fn compute_worker_tools() -> Vec<String> {
    ["sympy_eval", "check_expression_sanity", "ledger_append", "ledger_query", "ledger_note"]
        .into_iter().map(str::to_string).collect()
}

pub fn literature_worker_tools() -> Vec<String> {
    ["arxiv_search", "literature_search", "doc_ingest", "fs_list", "fs_read", "ledger_query", "ledger_note"]
        .into_iter().map(str::to_string).collect()
}

pub fn lean_worker_tools() -> Vec<String> {
    ["lean_check", "fs_list", "fs_read", "ledger_append", "ledger_query", "ledger_note"]
        .into_iter().map(str::to_string).collect()
}

pub fn allowed_tools_for_profile(profile: &str, router: &Router) -> Vec<String> {
    let available: BTreeSet<&str> = router.registry().defs().iter().map(|d| d.id.as_str()).collect();
    let keep = |ids: &[&str]| -> Vec<String> {
        ids.iter().filter(|id| available.contains(**id)).map(|id| (*id).to_string()).collect()
    };
    match profile.trim().to_ascii_lowercase().as_str() {
        "read_only" => keep(&["fs_list", "fs_read", "arxiv_search", "literature_search", "doc_ingest", "ledger_query", "tex_audit"]),
        "compute" => keep(&["sympy_eval", "check_expression_sanity", "fs_list", "fs_read", "ledger_append", "ledger_query", "ledger_note"]),
        _ => available.into_iter().filter(|id| !is_delegation_tool(id)).map(str::to_string).collect(),
    }
}

fn mk_spec(name: &str, description: String, properties: Value, required: &[&str]) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description,
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }),
    }
}

fn task_ctx_props(extra: Option<(&str, Value)>) -> Value {
    let mut m = json!({
        "task": { "type": "string", "description": "Concrete subtask, calculation, or query to execute." },
        "context": { "type": "string", "description": "Optional definitions, conventions, or Proof-DAG IDs (`e-N`)." }
    });
    if let (Some((k, v)), Some(obj)) = (extra, m.as_object_mut()) { obj.insert(k.into(), v); }
    m
}

pub fn delegation_tool_specs(cfg: &DelegationConfig<'_>) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    if let Some(w) = &cfg.compute_worker {
        specs.push(mk_spec(DELEGATE_COMPUTE, format!("Delegate symbolic/numerical Python, SymPy, or expression sanity checks to the Compute Worker ({}).", w.model), task_ctx_props(Some(("record_proof_entry", json!({ "type": "boolean", "description": "Record `symbolic_computation`/`numerical_computation` entry (default true)." })))), &["task"]));
    }
    if let Some(w) = &cfg.literature_worker {
        specs.push(mk_spec(DELEGATE_LITERATURE, format!("Delegate arXiv/literature searches, ingestion, and TeX reading to the Literature Worker ({}).", w.model), task_ctx_props(None), &["task"]));
    }
    if let Some(w) = &cfg.lean_worker {
        specs.push(mk_spec(DELEGATE_LEAN, format!("Delegate Lean 4 formalization and `lean_check` verification to the isolated Lean Worker ({}).", w.model), task_ctx_props(None), &["task"]));
    }
    if let Some(w) = &cfg.explorer_worker {
        specs.push(explore_tool_spec(w));
    }
    if !cfg.custom_subagents.is_empty() {
        let roster = cfg.custom_subagents.iter().map(|sa| {
            let desc = if sa.description.trim().is_empty() { "specialist" } else { sa.description.trim() };
            format!("`{}` ({}, profile={}, model={}): {desc}", sa.id, sa.name, sa.tools_profile, sa.model)
        }).collect::<Vec<_>>().join("; ");
        specs.push(mk_spec(DELEGATE_SUBAGENT, format!("Delegate a focused subtask to a saved custom specialist subagent: {roster}."), task_ctx_props(Some(("subagent_id", json!({ "type": "string", "description": "Exact ID of the custom subagent." })))), &["subagent_id", "task"]));
    }
    specs
}

pub fn format_delegation_prompt_section(cfg: &DelegationConfig<'_>) -> String {
    let mut lines = vec![
        "Orchestrator -> Worker Delegation:".to_string(),
        "- Act as the high-level Orchestrator: plan derivations, interpret computation outputs (`ledger_append` \
         with `entry_type: \"result\"` / `\"derivation\"`), and synthesize the `final_answer`.".to_string(),
    ];
    if let Some(w) = &cfg.compute_worker {
        lines.push(format!("- Prefer `{DELEGATE_COMPUTE}` ({}) for iterative `sympy_eval` scripts, traceback retries, and `check_expression_sanity`.", w.model));
    }
    if let Some(w) = &cfg.literature_worker {
        lines.push(format!("- Prefer `{DELEGATE_LITERATURE}` ({}) for `arxiv_search`, `literature_search`, `doc_ingest`, and bulky `.tex` reading.", w.model));
    }
    if let Some(w) = &cfg.lean_worker {
        lines.push(format!("- Prefer `{DELEGATE_LEAN}` ({}) for Lean 4 formalization and `lean_check` machine-checked proofs.", w.model));
    }
    if let Some(w) = &cfg.explorer_worker {
        lines.push(format!("- Prefer `{DELEGATE_EXPLORE}` ({}) to test 2..=4 independent approaches concurrently before committing to one.", w.model));
    }
    if !cfg.custom_subagents.is_empty() {
        let ids = cfg.custom_subagents.iter().map(|sa| format!("`{}` ({})", sa.id, sa.name)).collect::<Vec<_>>().join(", ");
        lines.push(format!("- Use `{DELEGATE_SUBAGENT}` when a task matches a custom specialist: {ids}."));
    }
    lines.join("\n")
}

pub(crate) fn clip_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

pub(crate) fn extract_usage_tokens(usage: &Value) -> (u64, u64) {
    let inp = usage.get("input_tokens").or_else(|| usage.get("prompt_tokens")).or_else(|| usage.get("promptTokenCount")).and_then(Value::as_u64).unwrap_or(0);
    let out = usage.get("output_tokens").or_else(|| usage.get("completion_tokens")).or_else(|| usage.get("candidatesTokenCount")).and_then(Value::as_u64).unwrap_or(0);
    (inp, out)
}

pub(crate) fn collect_text(content: &[ContentBlock]) -> String {
    content.iter().filter_map(|b| match b {
        ContentBlock::Text { text } => Some(text.as_str()),
        _ => None,
    }).collect::<Vec<_>>().join("\n")
}

fn resolve_target_worker<'a>(
    cfg: &'a DelegationConfig<'a>,
    router: &Router,
    tool_name: &str,
    input: &Value,
) -> Result<TargetWorker<'a>, String> {
    match tool_name {
        DELEGATE_COMPUTE => cfg.compute_worker.as_ref().map(TargetWorker::from_spec).ok_or_else(|| "Compute Worker is not configured".into()),
        DELEGATE_LITERATURE => cfg.literature_worker.as_ref().map(TargetWorker::from_spec).ok_or_else(|| "Literature Worker is not configured".into()),
        DELEGATE_LEAN => cfg.lean_worker.as_ref().map(TargetWorker::from_spec).ok_or_else(|| "Lean Worker is not configured".into()),
        DELEGATE_SUBAGENT => {
            let sub_id = input.get("subagent_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| "`subagent_id` is required".to_string())?;
            let sa = cfg.custom_subagents.iter().find(|s| s.id == sub_id).ok_or_else(|| format!("unknown custom subagent `{sub_id}`"))?;
            Ok(TargetWorker {
                role: format!("subagent:{}", sa.id),
                display_name: sa.name.clone(),
                transport: sa.transport,
                model: sa.model.clone(),
                routing_reason: sa.routing_reason.clone(),
                system_prompt: sa.system_prompt.clone(),
                allowed_tools: allowed_tools_for_profile(&sa.tools_profile, router),
                max_turns: DEFAULT_WORKER_MAX_TURNS,
                max_tokens: WORKER_MAX_TOKENS,
                reasoning_effort: sa.reasoning_effort.clone(),
            })
        }
        other => Err(format!("unknown delegation tool `{other}`")),
    }
}

fn build_worker_system_prompt(base: &str, conventions: Option<&str>) -> String {
    let mut parts = vec![base.trim().to_string()];
    if let Some(conv) = conventions.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("<pinned_conventions>\n{conv}\n</pinned_conventions>"));
    }
    parts.push(
        "Delegated Worker Rules:\n\
         - You operate in an isolated context with a strict tool whitelist and cannot delegate further.\n\
         - Evidence != proof: you are BARRED from calling `ledger_append` with `entry_type: \"final_answer\"`.\n\
         - Never claim a computation, Lean check, or source check succeeded unless the tool actually ran.\n\
         - Keep your final reply concise, exact, and structured (values, equations, caveats, and Proof-DAG `e-N` IDs)."
            .to_string(),
    );
    parts.join("\n\n")
}

fn build_worker_user_prompt(tool_name: &str, task: &str, input: &Value) -> String {
    let mut msg = format!("Task:\n{task}");
    if let Some(ctx) = input.get("context").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        msg.push_str(&format!("\n\nRelevant Context:\n{ctx}"));
    }
    if tool_name == DELEGATE_COMPUTE {
        match input.get("record_proof_entry").and_then(Value::as_bool) {
            Some(false) => msg.push_str("\n\nRun the computation without appending a Proof-DAG entry unless essential."),
            _ => msg.push_str("\n\nWhen a substantive calculation succeeds, record it via `ledger_append` (`symbolic_computation` or `numerical_computation`) and cite `entry_id`."),
        }
    }
    msg
}

fn fit_return_json(base: &Value, raw_summary: &str) -> Value {
    let mut limit = DELEGATE_RETURN_SUMMARY_CHARS;
    loop {
        let mut obj = base.as_object().cloned().unwrap_or_default();
        obj.insert("summary".into(), Value::String(clip_chars(raw_summary, limit)));
        let candidate = Value::Object(obj);
        if tools::canon::canonical_json(&candidate).len() <= MAX_RETURN_JSON_LEN || limit <= 120 {
            return candidate;
        }
        limit = (limit / 2).max(120);
    }
}

fn to_tool_spec(d: &ToolDef) -> ToolSpec {
    ToolSpec { name: d.id.clone(), description: d.description.clone(), input_schema: d.input_schema.clone() }
}

fn collect_worker_tools(router: &Router, lean_router: Option<&Router>, tool_name: &str, allowed: &BTreeSet<String>) -> Vec<ToolSpec> {
    let mut specs: Vec<ToolSpec> = router.registry().defs().into_iter().filter(|d| allowed.contains(&d.id)).map(to_tool_spec).collect();
    if tool_name == DELEGATE_LEAN && allowed.contains("lean_check") {
        if let Some(lr) = lean_router {
            for d in lr.registry().defs() {
                if d.id == "lean_check" && !specs.iter().any(|s| s.name == "lean_check") {
                    specs.push(to_tool_spec(d));
                }
            }
        }
    }
    specs
}

#[allow(clippy::too_many_arguments)]
fn call_worker_tool(
    audit: &mut Audit<'_>,
    router: &Router,
    lean_router: Option<&Router>,
    tool_name: &str,
    worker: &TargetWorker<'_>,
    worker_caller: &str,
    allowed_set: &BTreeSet<String>,
    name: &str,
    tinput: &Value,
    tools_used: &mut Vec<String>,
    proof_entries: &mut Vec<String>,
) -> Result<(bool, String), String> {
    if !allowed_set.contains(name) {
        return Ok((true, format!("DENIED: tool `{name}` is not permitted for `{}`", worker.role)));
    }
    if name == "ledger_append" && tinput.get("entry_type").and_then(Value::as_str).map(str::trim) == Some("final_answer") {
        return Ok((true, "DENIED: delegated workers cannot record `final_answer`; record `symbolic_computation` or `numerical_computation` and return your summary to the Orchestrator".to_string()));
    }
    audit.sink.on_thinking_delta(&format!("  [{} -> {name}]\n", worker.display_name));
    if !tools_used.iter().any(|t| t == name) {
        tools_used.push(name.to_string());
    }
    let outcome = if tool_name == DELEGATE_LEAN && name == "lean_check" {
        let Some(lr) = lean_router else {
            return Ok((true, "DENIED: Lean verification router (`lean_router`) is not configured".to_string()));
        };
        lr.try_call(worker_caller, name, tinput)?
    } else {
        router.try_call(worker_caller, name, tinput)?
    };
    audit.flush_interleaved()?;
    Ok(match outcome {
        CallOutcome::Ok(val) => {
            if name == "ledger_append" {
                if let Some(eid) = val.get("entry_id").and_then(Value::as_str) {
                    if !proof_entries.iter().any(|p| p == eid) {
                        proof_entries.push(eid.to_string());
                    }
                }
            }
            (false, clip_chars(&tools::canon::canonical_json(&val), WORKER_TOOL_OUTPUT_CHARS))
        }
        CallOutcome::Denied(why) => (true, format!("DENIED: {why}")),
        CallOutcome::Error(why) => (true, format!("ERROR: {why}")),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_delegation_call(
    audit: &mut Audit<'_>,
    router: &Router,
    cfg: &DelegationConfig<'_>,
    tool_use_id: &str,
    tool_name: &str,
    input: &Value,
    cancel: Option<&CancelToken>,
) -> Result<(bool, String), String> {
    if tool_name == DELEGATE_EXPLORE {
        return run_parallel_exploration(audit, router, cfg, tool_use_id, input, cancel);
    }
    let task = input.get("task").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if task.is_empty() {
        return Ok((true, "ERROR: `task` must be a non-empty string".to_string()));
    }
    let worker = match resolve_target_worker(cfg, router, tool_name, input) {
        Ok(w) => w,
        Err(why) => return Ok((true, format!("ERROR: {why}"))),
    };

    let allowed_set: BTreeSet<String> = worker.allowed_tools.iter().filter(|id| !is_delegation_tool(id)).cloned().collect();
    let worker_tools = collect_worker_tools(router, cfg.lean_router, tool_name, &allowed_set);
    let exposed_ids: Vec<String> = worker_tools.iter().map(|t| t.name.clone()).collect();
    let max_turns = worker.max_turns.clamp(1, MAX_WORKER_MAX_TURNS);
    let worker_info = worker.transport.info();
    let worker_caller = format!("worker:{}:{}:{}", worker.role, worker_info.provider, audit.session);
    let sys_prompt = build_worker_system_prompt(&worker.system_prompt, cfg.pinned_conventions.as_deref());
    let mut messages = vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: build_worker_user_prompt(tool_name, task, input) }] }];

    audit.emit("delegate.start", json!({
        "call_id": tool_use_id, "role": worker.role, "worker_name": worker.display_name,
        "provider": worker_info.provider, "model": worker.model, "routing_reason": worker.routing_reason,
        "task": clip_chars(task, 500), "allowed_tools": exposed_ids, "max_turns": max_turns,
    }))?;
    audit.sink.on_thinking_delta(&format!("\n[{} ({}): {}]\n", worker.display_name, worker.model, clip_chars(&task.replace(['\r', '\n'], " "), 120)));

    let started = Instant::now();
    let (mut ok, mut cancelled, mut turns_used, mut total_in, mut total_out) = (false, false, 0u32, 0u64, 0u64);
    let (mut full_text, mut err_msg, mut tools_used, mut proof_entries) = (String::new(), None::<String>, Vec::new(), Vec::new());

    for wturn in 1..=max_turns {
        if cancel.is_some_and(CancelToken::is_cancelled) { cancelled = true; err_msg = Some("stopped by user".into()); break; }
        turns_used = wturn;
        let req = ModelRequest {
            model: worker.model.clone(), system: Some(sys_prompt.clone()), messages: messages.clone(),
            tools: worker_tools.clone(), max_tokens: worker.max_tokens, reasoning_effort: worker.reasoning_effort.clone(),
        };
        let prepared = match worker.transport.prepare_request(&req) { Ok(p) => p, Err(e) => { err_msg = Some(e.to_string()); break; } };
        let mut obs = CancelOnlyObserver(cancel);
        let resp = match worker.transport.send_prepared_streaming(&prepared, &mut obs) {
            Ok(r) => r,
            Err(e) if e.kind == TransportErrorKind::Cancelled => { cancelled = true; err_msg = Some("stopped by user".into()); break; }
            Err(e) => { err_msg = Some(e.to_string()); break; }
        };
        let resp_blob = audit.ledger.put_blob(&resp.raw_body).map_err(|e| format!("blob write failed: {e}"))?;
        let (in_tok, out_tok) = extract_usage_tokens(&resp.usage);
        total_in = total_in.saturating_add(in_tok);
        total_out = total_out.saturating_add(out_tok);
        let tc_meta: Vec<Value> = resp.content.iter().filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, .. } => Some(json!({ "id": id, "name": name })),
            _ => None,
        }).collect();
        let used_model = if resp.model.trim().is_empty() { &worker.model } else { &resp.model };
        audit.emit("delegate.turn", json!({
            "call_id": tool_use_id, "role": worker.role, "turn": wturn, "model": used_model,
            "stop_reason": resp.stop_reason.as_str(), "usage": resp.usage, "tool_calls": tc_meta, "raw_blob": resp_blob,
        }))?;
        if let Some(reason) = validate_turn(&resp) { err_msg = Some(reason); break; }
        match resp.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => { full_text = collect_text(&resp.content); ok = true; break; }
            StopReason::Refusal => { full_text = collect_text(&resp.content); err_msg = Some("worker model refused request".into()); break; }
            StopReason::ToolUse => {
                if wturn == max_turns { full_text = collect_text(&resp.content); err_msg = Some(format!("worker reached max_turns ({max_turns})")); break; }
                messages.push(Message { role: Role::Assistant, content: resp.content.clone() });
                let mut tool_results = Vec::new();
                for block in &resp.content {
                    let ContentBlock::ToolUse { id, name, input: tinput } = block else { continue };
                    if cancel.is_some_and(CancelToken::is_cancelled) { cancelled = true; err_msg = Some("stopped by user".into()); break; }
                    let (is_err, txt) = call_worker_tool(audit, router, cfg.lean_router, tool_name, &worker, &worker_caller, &allowed_set, name, tinput, &mut tools_used, &mut proof_entries)?;
                    tool_results.push(ContentBlock::ToolResult { tool_use_id: id.clone(), is_error: is_err, content: txt });
                }
                if cancelled { break; }
                messages.push(Message { role: Role::User, content: tool_results });
            }
            StopReason::MaxTokens | StopReason::PauseTurn | StopReason::Other(_) => unreachable!(),
        }
    }

    let raw_summary = if !full_text.trim().is_empty() { full_text.clone() } else if let Some(err) = &err_msg { format!("Worker failed: {err}") } else { "Worker finished with empty output.".into() };
    let blob_src = if full_text.trim().is_empty() { &raw_summary } else { &full_text };
    let out_blob = audit.ledger.put_blob(blob_src.as_bytes()).map_err(|e| format!("blob write failed: {e}"))?;
    let base_payload = json!({
        "worker": worker.role, "worker_name": worker.display_name, "provider": worker_info.provider,
        "model": worker.model, "routing_reason": worker.routing_reason, "ok": ok, "turns": turns_used,
        "input_tokens": total_in, "output_tokens": total_out, "tools_used": tools_used,
        "proof_entries": proof_entries, "raw_blob": out_blob,
    });
    let mut return_payload = fit_return_json(&base_payload, &raw_summary);
    let mut end_payload = return_payload.clone();
    if let Some(obj) = end_payload.as_object_mut() {
        obj.insert("call_id".into(), json!(tool_use_id));
        obj.insert("duration_ms".into(), json!(started.elapsed().as_millis() as u64));
        if let Some(err) = &err_msg { obj.insert("error".into(), json!(err)); }
    }
    audit.emit("delegate.end", end_payload)?;
    audit.sink.on_thinking_delta(&format!("[{} finished: {} turn(s), {}+{} tok{}]\n", worker.display_name, turns_used, total_in, total_out, if ok { "" } else { " (error)" }));
    if cancelled { return Ok((true, "DENIED: stopped by user".to_string())); }
    if let (false, Some(err), Some(obj)) = (ok, &err_msg, return_payload.as_object_mut()) {
        obj.insert("error".into(), json!(err));
    }
    Ok((!ok, tools::canon::canonical_json(&return_payload)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::NoopSink;
    use crate::transport::{ModelResponse, PreparedRequest, TransportError, TransportInfo};
    use ledger::Ledger;
    use policy::EffectClass;
    use research_tools::{register_research_tools, research_policy, ResearchEnv};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use tools::{ApprovalReply, ApprovalRequest, ApprovalScope, Approver, Registry};

    struct AllowAll;
    impl Approver for AllowAll {
        fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
            ApprovalReply { approved: true, digest: req.digest.clone(), by: "test".into() }
        }
    }

    struct Stub(Mutex<Vec<ModelResponse>>, Mutex<Vec<ModelRequest>>);
    impl Stub {
        fn new(mut r: Vec<ModelResponse>) -> Self { r.reverse(); Self(Mutex::new(r), Mutex::new(Vec::new())) }
    }
    impl Transport for Stub {
        fn info(&self) -> TransportInfo { TransportInfo { provider: "anthropic-messages", endpoint: "http://127.0.0.1".into(), api_key_env: "TEST".into() } }
        fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
            self.1.lock().unwrap().push(req.clone());
            Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
        }
        fn send_prepared(&self, _p: &PreparedRequest) -> Result<ModelResponse, TransportError> { Ok(self.0.lock().unwrap().pop().unwrap()) }
    }

    fn r(stop: StopReason, content: Vec<ContentBlock>) -> ModelResponse {
        ModelResponse { raw_body: b"{}".to_vec(), id: Some("r1".into()), model: "claude-haiku-4-5".into(), stop_reason: stop, content, usage: json!({ "input_tokens": 9, "output_tokens": 5 }) }
    }
    fn spec<'a>(role: &str, t: &'a dyn Transport, allowed: Vec<String>) -> DelegatedWorkerSpec<'a> {
        DelegatedWorkerSpec { role: role.into(), display_name: role.into(), transport: t, model: "claude-haiku-4-5".into(), routing_reason: "test".into(), system_prompt: "Worker.".into(), allowed_tools: allowed, max_turns: 4, max_tokens: 2048, reasoning_effort: None }
    }

    #[test]
    fn worker_guards_and_lean_router_routing() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let env = Arc::new(ResearchEnv::new(&data));
        let mut reg = Registry::new();
        register_research_tools(&mut reg, env);
        let ledger = Ledger::open(&data).unwrap();
        let session = "s-del-all";
        let router = Router::new(reg, research_policy(), Ledger::open(&data).unwrap(), session, Arc::new(AllowAll));

        let long = "x\\n\"\n".repeat(1600);
        let stub = Stub::new(vec![
            r(StopReason::ToolUse, vec![
                ContentBlock::ToolUse { id: "t1".into(), name: "fs_write".into(), input: json!({ "path": "x.tex", "content": "hi" }) },
                ContentBlock::ToolUse { id: "t2".into(), name: "ledger_append".into(), input: json!({ "entry_type": "final_answer", "summary": "S", "details": "D" }) },
            ]),
            r(StopReason::EndTurn, vec![ContentBlock::Text { text: long.clone() }]),
        ]);
        let dcfg = DelegationConfig { compute_worker: Some(spec("compute_worker", &stub, compute_worker_tools())), ..Default::default() };
        assert_eq!(delegation_tool_specs(&dcfg).into_iter().map(|s| s.name).collect::<Vec<_>>(), vec![DELEGATE_COMPUTE]);
        let mut sink = NoopSink;
        let mut audit = Audit { ledger: &ledger, session, sink: &mut sink, last_seq: 0 };
        audit.emit("session.start", json!({ "route": "api" })).unwrap();
        let (is_err, out) = run_delegation_call(&mut audit, &router, &dcfg, "c1", DELEGATE_COMPUTE, &json!({ "task": "test" }), None).unwrap();
        assert!(!is_err && out.len() < 3900);
        let seen = stub.1.lock().unwrap();
        assert!(matches!(&seen[1].messages[2].content[0], ContentBlock::ToolResult { is_error: true, content, .. } if content.contains("not permitted")));
        assert!(matches!(&seen[1].messages[2].content[1], ContentBlock::ToolResult { is_error: true, content, .. } if content.contains("final_answer")));
        let val: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(String::from_utf8(ledger.get_blob(val["raw_blob"].as_str().unwrap()).unwrap()).unwrap(), long);

        let mut lean_reg = Registry::new();
        lean_reg.register(ToolDef { id: "lean_check".into(), description: "Check Lean".into(), input_schema: json!({"type":"object","properties":{"code":{"type":"string"}},"required":["code"]}), effect: EffectClass::ExecuteCode, approval_scope: ApprovalScope::Once }, Arc::new(|_: &tools::CallCtx<'_>, _: &Value| Ok(json!({ "status": "verified" }))));
        let lean_router = Router::new(lean_reg, research_policy(), Ledger::open(&data).unwrap(), session, Arc::new(AllowAll));

        let mut run_lean = |lr: Option<&Router>, cid: &str| {
            let ls = Stub::new(vec![r(StopReason::ToolUse, vec![ContentBlock::ToolUse { id: "l1".into(), name: "lean_check".into(), input: json!({"code":"t"}) }]), r(StopReason::EndTurn, vec![ContentBlock::Text { text: "Done".into() }])]);
            let (err, _) = run_delegation_call(&mut audit, &router, &DelegationConfig { lean_worker: Some(spec("lean_worker", &ls, lean_worker_tools())), lean_router: lr, ..Default::default() }, cid, DELEGATE_LEAN, &json!({ "task": "check" }), None).unwrap();
            (err, ls.1.into_inner().unwrap())
        };
        let (_, reqs_none) = run_lean(None, "c2");
        assert!(matches!(&reqs_none[1].messages[2].content[0], ContentBlock::ToolResult { is_error: true, content, .. } if content.contains("DENIED")));
        let (lerr, reqs_ok) = run_lean(Some(&lean_router), "c3");
        assert!(!lerr && reqs_ok[0].tools.iter().any(|t| t.name == "lean_check"));
        assert!(matches!(&reqs_ok[1].messages[2].content[0], ContentBlock::ToolResult { is_error: false, content, .. } if content.contains("verified")));
        ledger.verify_session(session).unwrap();
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }
}
