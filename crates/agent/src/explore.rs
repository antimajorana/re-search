//! "Ultramath" parallel multi-branch exploration (`delegate_explore`).

use crate::delegate::{
    clip_chars, collect_text, extract_usage_tokens, is_delegation_tool, CancelOnlyObserver,
    DelegatedWorkerSpec, DelegationConfig,
};
use crate::runner::{validate_turn, Audit};
use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, Role, StopReason, ToolSpec,
    TransportErrorKind,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::time::Instant;
use tools::{CallOutcome, Router};

pub const DELEGATE_EXPLORE: &str = "delegate_explore";
pub const MAX_EXPLORE_BRANCHES: usize = 4;
pub const EXPLORE_BRANCH_MAX_TURNS: u32 = 6;
const EXPLORE_TOOL_OUTPUT_CHARS: usize = 6_000;
const MAX_EXPLORE_RETURN_JSON_LEN: usize = 3_800;

pub fn explore_worker_tools() -> Vec<String> {
    ["sympy_eval", "check_expression_sanity", "fs_list", "fs_read", "ledger_query"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub fn explore_tool_spec(worker: &DelegatedWorkerSpec<'_>) -> ToolSpec {
    ToolSpec {
        name: DELEGATE_EXPLORE.to_string(),
        description: format!(
            "Launch 2..={MAX_EXPLORE_BRANCHES} independent Ultramath exploration branches in parallel ({}) \
             to test competing proof strategies or Ansaetze concurrently before committing to one.",
            worker.model
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "problem": { "type": "string", "description": "Mathematical or physics problem to explore." },
                "approaches": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 2,
                    "maxItems": MAX_EXPLORE_BRANCHES,
                    "description": "2..=4 independent approaches to evaluate concurrently."
                },
                "context": { "type": "string", "description": "Optional definitions, conventions, or Proof-DAG IDs (`e-N`)." }
            },
            "required": ["problem", "approaches"],
            "additionalProperties": false
        }),
    }
}

struct BranchOutcome {
    index: usize,
    approach: String,
    ok: bool,
    turns: u32,
    input_tokens: u64,
    output_tokens: u64,
    tools_used: Vec<String>,
    full_text: String,
    error: Option<String>,
}

fn build_branch_system_prompt(base: &str, conventions: Option<&str>) -> String {
    let mut parts = vec![base.trim().to_string()];
    if let Some(conv) = conventions.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("<pinned_conventions>\n{conv}\n</pinned_conventions>"));
    }
    parts.push(
        "Ultramath Parallel Explorer Rules:\n\
         - Test only your assigned approach independently using allowed read/compute tools.\n\
         - You are BARRED from calling `delegate_*` or `ledger_append`.\n\
         - Report concisely: viability, exact symbolic/numerical checks, obstructions or counterexamples, and key equations."
            .to_string(),
    );
    parts.join("\n\n")
}

#[allow(clippy::too_many_arguments)]
fn run_branch(
    idx: usize,
    approach: &str,
    problem: &str,
    context: Option<&str>,
    total: usize,
    worker: &DelegatedWorkerSpec<'_>,
    provider: &str,
    session: &str,
    router: &Router,
    sys_prompt: &str,
    worker_tools: &[ToolSpec],
    allowed_set: &BTreeSet<String>,
    max_turns: u32,
    cancel: Option<&CancelToken>,
) -> BranchOutcome {
    let caller = format!("worker:explorer_b{}:{provider}:{session}", idx + 1);
    let mut user_msg = format!("Problem:\n{problem}\n\nAssigned Approach ({}/{total}):\n{approach}", idx + 1);
    if let Some(ctx) = context {
        user_msg.push_str(&format!("\n\nRelevant Context:\n{ctx}"));
    }
    let mut messages = vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: user_msg }] }];
    let (mut ok, mut turns_used, mut total_in, mut total_out) = (false, 0u32, 0u64, 0u64);
    let (mut full_text, mut err_msg, mut tools_used) = (String::new(), None::<String>, Vec::<String>::new());

    for turn in 1..=max_turns {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            err_msg = Some("stopped by user".to_string());
            break;
        }
        turns_used = turn;
        let req = ModelRequest {
            model: worker.model.clone(),
            system: Some(sys_prompt.to_string()),
            messages: messages.clone(),
            tools: worker_tools.to_vec(),
            max_tokens: worker.max_tokens,
            reasoning_effort: worker.reasoning_effort.clone(),
        };
        let prepared = match worker.transport.prepare_request(&req) {
            Ok(p) => p,
            Err(e) => {
                err_msg = Some(e.to_string());
                break;
            }
        };
        let mut obs = CancelOnlyObserver(cancel);
        let resp = match worker.transport.send_prepared_streaming(&prepared, &mut obs) {
            Ok(r) => r,
            Err(e) if e.kind == TransportErrorKind::Cancelled => {
                err_msg = Some("stopped by user".to_string());
                break;
            }
            Err(e) => {
                err_msg = Some(e.to_string());
                break;
            }
        };
        let (in_tok, out_tok) = extract_usage_tokens(&resp.usage);
        total_in = total_in.saturating_add(in_tok);
        total_out = total_out.saturating_add(out_tok);
        if let Some(reason) = validate_turn(&resp) {
            err_msg = Some(reason);
            break;
        }
        match resp.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                full_text = collect_text(&resp.content);
                ok = true;
                break;
            }
            StopReason::Refusal => {
                full_text = collect_text(&resp.content);
                err_msg = Some("explorer branch refused request".to_string());
                break;
            }
            StopReason::ToolUse => {
                if turn == max_turns {
                    full_text = collect_text(&resp.content);
                    err_msg = Some(format!("explorer branch reached max_turns ({max_turns})"));
                    break;
                }
                messages.push(Message { role: Role::Assistant, content: resp.content.clone() });
                let mut results = Vec::new();
                let mut aborted = false;
                for block in &resp.content {
                    let ContentBlock::ToolUse { id, name, input: tinput } = block else { continue };
                    if cancel.is_some_and(CancelToken::is_cancelled) {
                        err_msg = Some("stopped by user".to_string());
                        aborted = true;
                        break;
                    }
                    let (is_err, txt) = if !allowed_set.contains(name.as_str()) {
                        (true, format!("DENIED: tool `{name}` is not permitted in parallel exploration"))
                    } else {
                        if !tools_used.iter().any(|t| t == name) {
                            tools_used.push(name.clone());
                        }
                        match router.try_call(&caller, name, tinput) {
                            Ok(CallOutcome::Ok(val)) => {
                                (false, clip_chars(&tools::canon::canonical_json(&val), EXPLORE_TOOL_OUTPUT_CHARS))
                            }
                            Ok(CallOutcome::Denied(why)) => (true, format!("DENIED: {why}")),
                            Ok(CallOutcome::Error(why)) => (true, format!("ERROR: {why}")),
                            Err(e) => {
                                err_msg = Some(e);
                                aborted = true;
                                break;
                            }
                        }
                    };
                    results.push(ContentBlock::ToolResult { tool_use_id: id.clone(), is_error: is_err, content: txt });
                }
                if aborted {
                    break;
                }
                messages.push(Message { role: Role::User, content: results });
            }
            StopReason::MaxTokens | StopReason::PauseTurn | StopReason::Other(_) => unreachable!(),
        }
    }

    BranchOutcome {
        index: idx,
        approach: approach.to_string(),
        ok,
        turns: turns_used,
        input_tokens: total_in,
        output_tokens: total_out,
        tools_used,
        full_text,
        error: err_msg,
    }
}

fn fit_explore_return_json(base: &Map<String, Value>, outcomes: &[BranchOutcome], blobs: &[String]) -> (Value, String) {
    let mut sum_limit = (2_400 / outcomes.len().max(1)).max(120);
    let mut app_limit = 220usize;
    loop {
        let branches: Vec<Value> = outcomes
            .iter()
            .zip(blobs.iter())
            .map(|(b, blob)| {
                let raw = if !b.full_text.trim().is_empty() {
                    b.full_text.as_str()
                } else {
                    b.error.as_deref().unwrap_or("Branch produced empty output.")
                };
                let mut item = json!({
                    "index": b.index,
                    "approach": clip_chars(&b.approach, app_limit),
                    "ok": b.ok,
                    "turns": b.turns,
                    "input_tokens": b.input_tokens,
                    "output_tokens": b.output_tokens,
                    "tools_used": b.tools_used,
                    "raw_blob": blob,
                    "summary": clip_chars(raw, sum_limit),
                });
                if let (false, Some(err)) = (b.ok, &b.error) {
                    item["error"] = json!(clip_chars(err, sum_limit.min(200)));
                }
                item
            })
            .collect();
        let mut obj = base.clone();
        obj.insert("branches".into(), Value::Array(branches));
        let candidate = Value::Object(obj);
        let encoded = tools::canon::canonical_json(&candidate);
        if encoded.len() <= MAX_EXPLORE_RETURN_JSON_LEN || (sum_limit <= 20 && app_limit <= 60) {
            return (candidate, encoded);
        }
        if sum_limit > 20 {
            sum_limit = (sum_limit / 2).max(20);
        } else {
            app_limit = 60;
        }
    }
}

pub(crate) fn run_parallel_exploration(
    audit: &mut Audit<'_>,
    router: &Router,
    cfg: &DelegationConfig<'_>,
    tool_use_id: &str,
    input: &Value,
    cancel: Option<&CancelToken>,
) -> Result<(bool, String), String> {
    let problem = input.get("problem").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if problem.is_empty() {
        return Ok((true, "ERROR: `problem` must be a non-empty string".to_string()));
    }
    let Some(raw_arr) = input.get("approaches").and_then(Value::as_array) else {
        return Ok((true, "ERROR: `approaches` must be an array of 2..=4 non-empty strings".to_string()));
    };
    if raw_arr.len() < 2 || raw_arr.len() > MAX_EXPLORE_BRANCHES {
        return Ok((
            true,
            format!("ERROR: `approaches` must contain 2..={MAX_EXPLORE_BRANCHES} approaches (got {})", raw_arr.len()),
        ));
    }
    let mut approaches = Vec::with_capacity(raw_arr.len());
    for item in raw_arr {
        let s = item.as_str().map(str::trim).unwrap_or("");
        if s.is_empty() {
            return Ok((true, "ERROR: every entry in `approaches` must be a non-empty string".to_string()));
        }
        approaches.push(s.to_string());
    }
    let Some(worker) = cfg.explorer_worker.as_ref() else {
        return Ok((true, "ERROR: Explorer Worker is not configured".to_string()));
    };
    let context = input.get("context").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let allowed_set: BTreeSet<String> = worker
        .allowed_tools
        .iter()
        .filter(|id| !is_delegation_tool(id) && id.as_str() != "ledger_append" && id.as_str() != "ledger_supersede")
        .cloned()
        .collect();
    let worker_tools: Vec<ToolSpec> = router
        .registry()
        .defs()
        .iter()
        .filter(|d| allowed_set.contains(&d.id))
        .map(|d| ToolSpec { name: d.id.clone(), description: d.description.clone(), input_schema: d.input_schema.clone() })
        .collect();
    let worker_info = worker.transport.info();
    let sys_prompt = build_branch_system_prompt(&worker.system_prompt, cfg.pinned_conventions.as_deref());
    let max_turns = worker.max_turns.clamp(1, EXPLORE_BRANCH_MAX_TURNS);

    audit.emit(
        "explore.start",
        json!({
            "call_id": tool_use_id,
            "problem": clip_chars(problem, 500),
            "approaches": approaches,
            "provider": worker_info.provider,
            "model": worker.model,
            "routing_reason": worker.routing_reason,
            "branch_count": approaches.len(),
        }),
    )?;
    audit.sink.on_thinking_delta(&format!(
        "\n[Ultramath Explorer ({}): launching {} approaches in parallel...]\n",
        worker.model,
        approaches.len()
    ));

    let started = Instant::now();
    let session = audit.session;
    let outcomes: Vec<BranchOutcome> = std::thread::scope(|s| {
        let handles: Vec<_> = approaches
            .iter()
            .enumerate()
            .map(|(idx, app)| {
                let (sys, tools, allowed, count) = (&sys_prompt, &worker_tools, &allowed_set, approaches.len());
                s.spawn(move || {
                    run_branch(idx, app, problem, context, count, worker, worker_info.provider, session, router, sys, tools, allowed, max_turns, cancel)
                })
            })
            .collect();
        handles.into_iter().enumerate().map(|(idx, h)| {
            h.join().unwrap_or_else(|_| BranchOutcome {
                index: idx, approach: approaches[idx].clone(), ok: false, turns: 0,
                input_tokens: 0, output_tokens: 0, tools_used: Vec::new(),
                full_text: String::new(), error: Some("explorer branch panicked".to_string()),
            })
        }).collect()
    });
    audit.flush_interleaved()?;

    let mut blobs = Vec::with_capacity(outcomes.len());
    for b in &outcomes {
        let text_src = if !b.full_text.trim().is_empty() {
            b.full_text.as_str()
        } else {
            b.error.as_deref().unwrap_or("(empty explorer branch output)")
        };
        let blob = audit.ledger.put_blob(text_src.as_bytes()).map_err(|e| format!("blob write failed: {e}"))?;
        audit.emit(
            "explore.branch",
            json!({
                "call_id": tool_use_id, "index": b.index, "approach": b.approach, "ok": b.ok,
                "turns": b.turns, "input_tokens": b.input_tokens, "output_tokens": b.output_tokens,
                "tools_used": b.tools_used, "error": b.error, "raw_blob": blob,
            }),
        )?;
        blobs.push(blob);
    }

    let any_ok = outcomes.iter().any(|b| b.ok);
    let total_turns: u32 = outcomes.iter().map(|b| b.turns).sum();
    let total_in: u64 = outcomes.iter().map(|b| b.input_tokens).sum();
    let total_out: u64 = outcomes.iter().map(|b| b.output_tokens).sum();
    let mut base = Map::new();
    base.insert("worker".into(), json!("explorer_worker"));
    base.insert("worker_name".into(), json!(worker.display_name));
    base.insert("provider".into(), json!(worker_info.provider));
    base.insert("model".into(), json!(worker.model));
    base.insert("routing_reason".into(), json!(worker.routing_reason));
    base.insert("ok".into(), json!(any_ok));
    base.insert("branch_count".into(), json!(outcomes.len()));
    base.insert("turns".into(), json!(total_turns));
    base.insert("input_tokens".into(), json!(total_in));
    base.insert("output_tokens".into(), json!(total_out));

    let (return_payload, canonical) = fit_explore_return_json(&base, &outcomes, &blobs);
    let mut end_payload = return_payload;
    if let Some(obj) = end_payload.as_object_mut() {
        obj.insert("call_id".into(), json!(tool_use_id));
        obj.insert("duration_ms".into(), json!(started.elapsed().as_millis() as u64));
    }
    audit.emit("explore.end", end_payload)?;
    audit.sink.on_thinking_delta(&format!(
        "[Ultramath Explorer finished: {}/{} branches succeeded, {total_turns} turns, {total_in}+{total_out} tok]\n",
        outcomes.iter().filter(|b| b.ok).count(),
        outcomes.len(),
    ));
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok((true, "DENIED: stopped by user".to_string()));
    }
    Ok((!any_ok, canonical))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::NoopSink;
    use crate::transport::{ModelResponse, PreparedRequest, Transport, TransportError, TransportInfo};
    use ledger::Ledger;
    use research_tools::{register_research_tools, research_policy, ResearchEnv};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;
    use tools::{ApprovalReply, ApprovalRequest, Approver, Registry};

    struct AllowAll;
    impl Approver for AllowAll {
        fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
            ApprovalReply { approved: true, digest: req.digest.clone(), by: "test".into() }
        }
    }

    struct ConcurrentStub {
        barrier: Barrier,
        in_flight: AtomicUsize,
        peak_in_flight: AtomicUsize,
    }
    impl Transport for ConcurrentStub {
        fn info(&self) -> TransportInfo {
            TransportInfo { provider: "anthropic-messages", endpoint: "http://127.0.0.1".into(), api_key_env: "TEST".into() }
        }
        fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
            Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
        }
        fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
            let cur = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_in_flight.fetch_max(cur, Ordering::SeqCst);
            self.barrier.wait();
            let txt = String::from_utf8_lossy(&prepared.body);
            let branch = if txt.contains("Assigned Approach (1/2)") { "Branch 1 viable" } else { "Branch 2 viable" };
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(ModelResponse {
                raw_body: b"{}".to_vec(), id: Some("exp".into()), model: "claude-haiku-4-5".into(),
                stop_reason: StopReason::EndTurn, content: vec![ContentBlock::Text { text: branch.into() }],
                usage: json!({ "input_tokens": 15, "output_tokens": 9 }),
            })
        }
    }

    #[test]
    fn parallel_exploration_executes_branches_concurrently_and_verifies_ledger() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        let env = Arc::new(ResearchEnv::new(&data));
        let mut reg = Registry::new();
        register_research_tools(&mut reg, env);
        let session = "s-exp-conc";
        let router = Router::new(reg, research_policy(), Ledger::open(&data).unwrap(), session, Arc::new(AllowAll));
        let ledger = Ledger::open(&data).unwrap();
        let stub = ConcurrentStub { barrier: Barrier::new(2), in_flight: AtomicUsize::new(0), peak_in_flight: AtomicUsize::new(0) };
        let cfg = DelegationConfig {
            explorer_worker: Some(DelegatedWorkerSpec {
                role: "explorer_worker".into(), display_name: "Ultramath Explorer".into(),
                transport: &stub, model: "claude-haiku-4-5".into(), routing_reason: "test".into(),
                system_prompt: "Explore.".into(), allowed_tools: explore_worker_tools(),
                max_turns: EXPLORE_BRANCH_MAX_TURNS, max_tokens: 2048, reasoning_effort: None,
            }),
            ..Default::default()
        };
        let mut sink = NoopSink;
        let mut audit = Audit { ledger: &ledger, session, sink: &mut sink, last_seq: 0 };
        audit.emit("session.start", json!({ "route": "api" })).unwrap();

        let (is_err, out) = run_parallel_exploration(
            &mut audit, &router, &cfg, "call_exp",
            &json!({ "problem": "Find gap", "approaches": ["Jordan-Wigner", "Majorana pairing"] }), None,
        ).unwrap();
        assert!(!is_err);
        assert_eq!(stub.peak_in_flight.load(Ordering::SeqCst), 2);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["branch_count"], 2);
        assert_eq!(parsed["ok"], true);
        let evs = ledger.read_session(session).unwrap();
        assert!(evs.iter().any(|e| e.kind == "explore.start"));
        assert!(evs.iter().any(|e| e.kind == "explore.end" && e.payload["branch_count"] == 2));
        ledger.verify_session(session).unwrap();
        assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
    }
}
