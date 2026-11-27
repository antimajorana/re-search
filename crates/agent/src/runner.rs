//! In-core agent loop driving a [`crate::Transport`] and [`tools::Router`].

use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, ModelResponse, Role, StopReason, StreamObserver, ToolSpec,
    Transport, TransportError, TransportErrorKind,
};
use ledger::{AppendedEvent, Ledger};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Instant;
use tools::Router;

pub const EVENT_VERSION: u32 = 1;
pub const DEFAULT_MAX_TURNS: u32 = 48;
pub const DEFAULT_MAX_TOKENS: u32 = 16_384;
pub const DEFAULT_SYSTEM_PROMPT: &str = "Local re:Search core. `sympy_eval` runs Python/SymPy after the user approves \
    it (each call when uncontained; once per live worker session when sandboxed). `ledger_note` records an unverified \
    note. Results are computations, not verified claims.";

pub(crate) use crate::dispatch::clip_text;
use crate::dispatch::{collect_text, dispatch_tool_calls};
use tools::builtin::SET_SESSION_TITLE;
pub use tools::builtin::SESSION_TITLE_EVENT;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub prompt: String,
    pub model_requested: String,
    pub model: String,
    pub system_prompt: String,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub containment: String,
    pub contained: bool,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    Completed { turns: u32, stop_reason: String },
    Refusal { turns: u32, text: String },
    Aborted { turns: u32, reason: String },
}

pub trait EventSink {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String>;

    fn wants_session_title(&self) -> bool {
        false
    }

    fn on_text_delta(&mut self, _delta: &str) {}

    fn on_thinking_delta(&mut self, _delta: &str) {}
}

pub struct NoopSink;

impl EventSink for NoopSink {
    fn on_event(&mut self, _receipt: &AppendedEvent, _kind: &str, _data: &Value) -> Result<(), String> {
        Ok(())
    }
}

pub(crate) struct Audit<'a> {
    pub(crate) ledger: &'a Ledger,
    pub(crate) session: &'a str,
    pub(crate) sink: &'a mut dyn EventSink,
    pub(crate) last_seq: u64,
}

impl<'a> Audit<'a> {
    pub(crate) fn emit(&mut self, kind: &str, data: Value) -> Result<AppendedEvent, String> {
        let receipt = self
            .ledger
            .append_with_hash(self.session, kind, EVENT_VERSION, &data)
            .map_err(|e| format!("ledger could not record `{kind}`: {e}"))?;
        self.last_seq = receipt.seq;
        self.sink.on_event(&receipt, kind, &data)?;
        Ok(receipt)
    }

    pub(crate) fn flush_interleaved(&mut self) -> Result<(), String> {
        let cutoff = self.last_seq;
        let events = self
            .ledger
            .read_session(self.session)
            .map_err(|e| format!("ledger could not read session `{}`: {e}", self.session))?;
        for ev in events.into_iter().filter(|e| e.seq > cutoff) {
            let receipt = AppendedEvent {
                seq: ev.seq,
                ts_ms: ev.ts_ms,
                prev_hash: ev.prev_hash,
                event_hash: ev.event_hash,
            };
            self.sink.on_event(&receipt, &ev.kind, &ev.payload)?;
            self.last_seq = ev.seq;
        }
        Ok(())
    }

    pub(crate) fn put_blob(&self, bytes: &[u8]) -> Result<String, String> {
        self.ledger
            .put_blob(bytes)
            .map_err(|e| format!("ledger could not write blob: {e}"))
    }
}

pub(crate) struct LoopParams<'a> {
    pub(crate) model: &'a str,
    pub(crate) system_prompt: &'a str,
    pub(crate) max_turns: u32,
    pub(crate) max_tokens: u32,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) start_turn: u32,
    pub(crate) initial_messages: Vec<Message>,
    pub(crate) mask_keep_recent: Option<(usize, usize)>,
    pub(crate) cancel: Option<&'a CancelToken>,
    pub(crate) emit_end_event: bool,
    pub(crate) delegation: Option<&'a crate::delegate::DelegationConfig<'a>>,
    pub(crate) human: Option<&'a dyn crate::interaction::HumanChannel>,
    pub(crate) pause_after_explore: bool,
}

pub(crate) struct LegResult {
    pub(crate) outcome: SessionOutcome,
    pub(crate) messages: Vec<Message>,
    pub(crate) last_turn: u32,
    pub(crate) last_text: String,
}

pub(crate) struct AuditStreamObserver<'b, 'a> {
    pub(crate) audit: &'b mut Audit<'a>,
    pub(crate) cancel: Option<&'b CancelToken>,
}

impl StreamObserver for AuditStreamObserver<'_, '_> {
    fn on_text_delta(&mut self, delta: &str) {
        self.audit.sink.on_text_delta(delta);
    }

    fn on_thinking_delta(&mut self, delta: &str) {
        self.audit.sink.on_thinking_delta(delta);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_some_and(CancelToken::is_cancelled)
    }
}

pub(crate) fn tool_defs_json(defs: &[&tools::ToolDef]) -> Vec<Value> {
    defs.iter()
        .map(|d| {
            json!({
                "id": d.id,
                "effect": d.effect.as_str(),
                "approval_scope": d.approval_scope.as_str(),
                "def_hash": d.def_hash(),
            })
        })
        .collect()
}

pub fn run_agent_loop(
    transport: &dyn Transport,
    router: &Router,
    ledger: &Ledger,
    cfg: &AgentConfig,
    sink: &mut dyn EventSink,
) -> Result<SessionOutcome, String> {
    if cfg.max_turns == 0 {
        return Err("max_turns must be >= 1".into());
    }
    if cfg.max_tokens == 0 {
        return Err("max_tokens must be >= 1".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("model name must not be empty".into());
    }
    let session = router.session();
    let info = transport.info();
    let defs = router.registry().defs();
    let mut audit = Audit { ledger, session, sink, last_seq: 0 };
    audit.emit(
        "session.start",
        json!({
            "route": "api",
            "provenance": "in-core loop, full provenance",
            "provider": info.provider,
            "endpoint": info.endpoint,
            "api_key_env": info.api_key_env,
            "model_requested": cfg.model_requested,
            "model": cfg.model,
            "prompt": cfg.prompt,
            "system_prompt": cfg.system_prompt,
            "max_turns": cfg.max_turns,
            "max_tokens": cfg.max_tokens,
            "policy_version": router.policy().version.as_str(),
            "containment": cfg.containment,
            "contained": cfg.contained,
            "tool_defs": tool_defs_json(&defs),
        }),
    )?;

    let initial_messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: cfg.prompt.clone() }],
    }];
    run_turn_loop(
        transport,
        router,
        &mut audit,
        LoopParams {
            model: &cfg.model,
            system_prompt: &cfg.system_prompt,
            max_turns: cfg.max_turns,
            max_tokens: cfg.max_tokens,
            reasoning_effort: cfg.reasoning_effort.as_deref(),
            start_turn: 1,
            initial_messages,
            mask_keep_recent: None,
            cancel: None,
            emit_end_event: true,
            delegation: None,
            human: None,
            pause_after_explore: false,
        },
    )
}

pub(crate) fn run_turn_loop(
    transport: &dyn Transport,
    router: &Router,
    audit: &mut Audit<'_>,
    params: LoopParams<'_>,
) -> Result<SessionOutcome, String> {
    Ok(run_turn_leg(transport, router, audit, params)?.outcome)
}

pub(crate) fn run_turn_leg(
    transport: &dyn Transport,
    router: &Router,
    audit: &mut Audit<'_>,
    mut params: LoopParams<'_>,
) -> Result<LegResult, String> {
    let info = transport.info();
    let caller = format!("agent:{}:{}", info.provider, audit.session);
    let mut tool_specs: Vec<ToolSpec> = router
        .registry()
        .defs()
        .iter()
        .map(|d| ToolSpec { name: d.id.clone(), description: d.description.clone(), input_schema: d.input_schema.clone() })
        .collect();
    if let Some(dcfg) = params.delegation {
        tool_specs.extend(crate::delegate::delegation_tool_specs(dcfg));
    }
    if params.human.is_some() {
        tool_specs.push(crate::interaction::ask_user_tool_spec());
    }
    let started = Instant::now();
    let mut messages = std::mem::take(&mut params.initial_messages);
    let title_in_history = messages.iter().flat_map(|message| &message.content).any(|block| {
        matches!(block, ContentBlock::ToolUse { name, .. } if name == SET_SESSION_TITLE)
    });
    let use_title_tool = !tool_specs.iter().any(|tool| tool.name == SET_SESSION_TITLE)
        && (title_in_history || (params.max_turns > 1 && audit.sink.wants_session_title()));
    let mut system = params.system_prompt.to_string();
    if use_title_tool {
        if !system.is_empty() {
            system.push_str("\n\n");
        }
        system.push_str(
            "Early in the research, once you can state the main question, call `set_session_title` \
             once with a short, specific title in the user's language. Include it alongside your \
             first research tool call when possible, then continue the research.",
        );
        let def = tools::builtin::session_title_def();
        tool_specs.push(ToolSpec { name: def.id, description: def.description, input_schema: def.input_schema });
    }

    for offset in 0..params.max_turns {
        let leg_turns = offset + 1;
        let turn = params.start_turn + offset;
        if params.cancel.is_some_and(CancelToken::is_cancelled) {
            let outcome = abort_stopped(audit, leg_turns)?;
            return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
        }
        if let Some(h) = params.human {
            crate::interaction::inject_pending_steers(audit, h, &mut messages)?;
        }
        let outgoing_messages = match params.mask_keep_recent {
            Some((keep, stride)) => crate::guardrails::mask_old_tool_outputs_stepped(&messages, keep, stride),
            None => messages.clone(),
        };
        let req = ModelRequest {
            model: params.model.to_string(),
            system: Some(system.clone()),
            messages: outgoing_messages,
            tools: tool_specs.clone(),
            max_tokens: params.max_tokens,
            reasoning_effort: params.reasoning_effort.map(str::to_string),
        };
        let prepared = match transport.prepare_request(&req) {
            Ok(p) => p,
            Err(err) => {
                let outcome = abort_on_transport(audit, leg_turns, turn, err)?;
                return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
            }
        };
        let req_blob = audit.put_blob(&prepared.body)?;
        audit.emit(
            "model.request",
            json!({
                "turn": turn,
                "model": params.model,
                "message_count": req.messages.len(),
                "max_tokens": params.max_tokens,
                "raw_blob": req_blob,
            }),
        )?;

        let turn_start = Instant::now();
        let resp_res = {
            let mut obs = AuditStreamObserver { audit, cancel: params.cancel };
            transport.send_prepared_streaming(&prepared, &mut obs)
        };
        let resp = match resp_res {
            Ok(r) => r,
            Err(err) if err.kind == TransportErrorKind::Cancelled => {
                let outcome = abort_stopped(audit, leg_turns)?;
                return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
            }
            Err(err) => {
                let outcome = abort_on_transport(audit, leg_turns, turn, err)?;
                return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
            }
        };
        let duration_ms = turn_start.elapsed().as_millis() as u64;

        let resp_blob = audit.put_blob(&resp.raw_body)?;
        audit.emit(
            "model.response",
            json!({
                "turn": turn,
                "response_id": resp.id,
                "model": resp.model,
                "stop_reason": resp.stop_reason.as_str(),
                "usage": resp.usage,
                "duration_ms": duration_ms,
                "raw_blob": resp_blob,
            }),
        )?;
        record_content_blocks(audit, &resp.content)?;

        if resp.stop_reason == StopReason::Refusal {
            let text = collect_text(&resp.content);
            audit.emit(
                "model.refusal",
                json!({ "turn": turn, "response_id": resp.id, "text": text }),
            )?;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            audit.emit(
                "session.end",
                json!({
                    "outcome": "refusal",
                    "stop_reason": "refusal",
                    "num_turns": leg_turns,
                    "last_turn": turn,
                    "duration_ms": elapsed_ms,
                }),
            )?;
            return Ok(LegResult {
                outcome: SessionOutcome::Refusal { turns: leg_turns, text: text.clone() },
                messages,
                last_turn: turn,
                last_text: text,
            });
        }

        if let Some(reason) = validate_turn(&resp) {
            let outcome = abort_session(audit, leg_turns, reason)?;
            return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
        }

        match resp.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                let stop_str = resp.stop_reason.as_str().to_string();
                let last_text = collect_text(&resp.content);
                let elapsed_ms = started.elapsed().as_millis() as u64;
                messages.push(Message { role: Role::Assistant, content: resp.content });
                if params.emit_end_event {
                    audit.emit(
                        "session.end",
                        json!({
                            "outcome": "completed",
                            "stop_reason": stop_str,
                            "num_turns": leg_turns,
                            "last_turn": turn,
                            "duration_ms": elapsed_ms,
                        }),
                    )?;
                }
                return Ok(LegResult {
                    outcome: SessionOutcome::Completed { turns: leg_turns, stop_reason: stop_str },
                    messages,
                    last_turn: turn,
                    last_text,
                });
            }
            StopReason::ToolUse => {
                if leg_turns == params.max_turns {
                    let outcome = abort_session(
                        audit,
                        leg_turns,
                        format!("reached max_turns ({}) before model completed", params.max_turns),
                    )?;
                    return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
                }
                messages.push(Message { role: Role::Assistant, content: resp.content.clone() });
                let results = dispatch_tool_calls(audit, router, &caller, turn, &resp.content, &params, use_title_tool)?;
                if params.cancel.is_some_and(CancelToken::is_cancelled) {
                    messages.push(Message { role: Role::User, content: results });
                    let outcome = abort_stopped(audit, leg_turns)?;
                    return Ok(LegResult { outcome, messages, last_turn: turn, last_text: String::new() });
                }
                messages.push(Message { role: Role::User, content: results });
            }
            StopReason::Refusal | StopReason::MaxTokens | StopReason::PauseTurn | StopReason::Other(_) => {
                unreachable!("handled by validate_turn or Refusal branch");
            }
        }
    }

    let outcome = abort_session(audit, params.max_turns, "agent loop exited unexpectedly".into())?;
    Ok(LegResult {
        outcome,
        messages,
        last_turn: params.start_turn + params.max_turns.saturating_sub(1),
        last_text: String::new(),
    })
}

fn abort_stopped(audit: &mut Audit<'_>, turn: u32) -> Result<SessionOutcome, String> {
    let reason = "stopped by user".to_string();
    audit.emit("session.abort", json!({ "reason": reason, "stopped": true }))?;
    Ok(SessionOutcome::Aborted { turns: turn, reason })
}

fn abort_on_transport(
    audit: &mut Audit<'_>,
    leg_turns: u32,
    turn: u32,
    err: TransportError,
) -> Result<SessionOutcome, String> {
    let raw_blob = match &err.raw_body {
        Some(bytes) if !bytes.is_empty() => Some(audit.put_blob(bytes)?),
        _ => None,
    };
    let mut err_payload = json!({
        "turn": turn,
        "kind": err.kind.as_str(),
        "status": err.status,
        "message": err.message,
    });
    if let (Some(h), Some(obj)) = (raw_blob, err_payload.as_object_mut()) {
        obj.insert("raw_blob".into(), json!(h));
    }
    audit.emit("model.error", err_payload)?;
    abort_session(audit, leg_turns, err.to_string())
}

fn abort_session(audit: &mut Audit<'_>, turn: u32, reason: String) -> Result<SessionOutcome, String> {
    audit.emit("session.abort", json!({ "reason": reason }))?;
    Ok(SessionOutcome::Aborted { turns: turn, reason })
}

pub(crate) fn record_content_blocks(audit: &mut Audit<'_>, content: &[ContentBlock]) -> Result<(), String> {
    for block in content {
        let (kind, data) = match block {
            ContentBlock::Text { text } => ("model.text", json!({ "text": text })),
            ContentBlock::Thinking { thinking, .. } => ("model.thinking", json!({ "chars": thinking.chars().count() })),
            ContentBlock::RedactedThinking { data } => {
                ("model.thinking", json!({ "chars": data.chars().count(), "redacted": true }))
            }
            ContentBlock::ToolUse { id, name, input } => {
                ("model.tool_use", json!({ "id": id, "name": name, "input": input }))
            }
            ContentBlock::ToolResult { .. } => ("model.other", json!({ "block_type": "tool_result" })),
            ContentBlock::Other { block_type, .. } => ("model.other", json!({ "block_type": block_type })),
        };
        audit.emit(kind, data)?;
    }
    Ok(())
}

pub(crate) fn validate_turn(resp: &ModelResponse) -> Option<String> {
    match &resp.stop_reason {
        StopReason::MaxTokens => return Some("model response truncated at max_tokens".into()),
        StopReason::PauseTurn => return Some("model returned unsupported stop_reason `pause_turn`".into()),
        StopReason::Other(s) => return Some(format!("model returned unknown stop_reason `{s}`")),
        StopReason::EndTurn | StopReason::StopSequence | StopReason::ToolUse | StopReason::Refusal => {}
    }

    let mut seen_ids = HashSet::new();
    let mut tool_count = 0usize;
    for block in &resp.content {
        match block {
            ContentBlock::ToolUse { id, name, .. } => {
                tool_count += 1;
                if id.trim().is_empty() {
                    return Some("model emitted tool_use block with empty id".into());
                }
                if name.trim().is_empty() {
                    return Some("model emitted tool_use block with empty name".into());
                }
                if !seen_ids.insert(id.as_str()) {
                    return Some(format!("model emitted duplicate tool_use id `{id}`"));
                }
            }
            ContentBlock::ToolResult { .. } => {
                return Some("model emitted unexpected tool_result block in assistant message".into());
            }
            _ => {}
        }
    }

    match resp.stop_reason {
        StopReason::ToolUse if tool_count == 0 => {
            Some("model returned stop_reason `tool_use` without any tool_use blocks".into())
        }
        StopReason::EndTurn | StopReason::StopSequence if tool_count > 0 => Some(format!(
            "model returned `{}` with {} tool_use block(s)",
            resp.stop_reason.as_str(),
            tool_count
        )),
        _ => None,
    }
}
