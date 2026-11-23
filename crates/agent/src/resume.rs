//! Verified, lossless session inspection and multi-prompt resume over the
//! hash-chained ledger and content-addressed blob store.

use crate::runner::{
    clip_text, run_turn_loop, tool_defs_json, validate_turn, Audit, EventSink, LoopParams, SessionOutcome,
    DEFAULT_SYSTEM_PROMPT,
};
use crate::transport::{ContentBlock, Message, ModelResponse, ProviderKind, Role, StopReason, Transport};
use ledger::{Event, Ledger};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use tools::{Router, ToolDef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeRejectKind {
    TamperOrMissing,
    LegacySchema,
    UnsupportedRoute,
    Drift,
    CorruptHistory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeReject {
    pub kind: ResumeRejectKind,
    pub message: String,
}

impl fmt::Display for ResumeReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ResumeReject {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeInspection {
    pub session: String,
    pub last_provider: ProviderKind,
    pub last_model: String,
    pub system_prompt: String,
    pub resumed_from_seq: u64,
    pub leg: u32,
    pub start_turn: u32,
    pub prior_outcome: String,
}

#[derive(Debug, Clone)]
pub struct ResumeConfig {
    pub prompt: String,
    pub model_requested: String,
    pub model: String,
    pub system_prompt: Option<String>,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub containment: String,
    pub contained: bool,
    pub reasoning_effort: Option<String>,
}

struct TaggedMessage {
    role: Role,
    source_provider: Option<ProviderKind>,
    content: Vec<ContentBlock>,
}

fn reject(kind: ResumeRejectKind, message: impl Into<String>) -> ResumeReject {
    ResumeReject { kind, message: message.into() }
}

fn corrupt(message: impl Into<String>) -> ResumeReject {
    reject(ResumeRejectKind::CorruptHistory, message)
}

fn req_str<'a>(ev: &'a Event, field: &str) -> Result<&'a str, ResumeReject> {
    ev.payload
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| corrupt(format!("session `{}` seq {} missing `{field}`", ev.session, ev.seq)))
}

pub(crate) fn verify_and_read(
    ledger: &Ledger,
    session: &str,
    expected_policy_version: &str,
    expected_defs: &[ToolDef],
) -> Result<(ResumeInspection, Vec<Event>), ResumeReject> {
    if let Err(err) = ledger.verify_session(session) {
        let kind = match err {
            ledger::Error::LegacyUnchainedSchema => ResumeRejectKind::LegacySchema,
            _ => ResumeRejectKind::TamperOrMissing,
        };
        return Err(reject(kind, format!("cannot resume session `{session}`: {err}")));
    }
    let events = ledger
        .read_session(session)
        .map_err(|e| reject(ResumeRejectKind::TamperOrMissing, format!("cannot read session `{session}`: {e}")))?;
    if events.is_empty() {
        return Err(corrupt(format!("session `{session}` has no events")));
    }
    let start_ev = events
        .iter()
        .find(|e| e.kind == "session.start")
        .ok_or_else(|| corrupt(format!("session `{session}` has no `session.start` event")))?;
    let route = start_ev.payload.get("route").and_then(Value::as_str);
    if route != Some("api") {
        let label = route.unwrap_or("unknown");
        return Err(reject(
            ResumeRejectKind::UnsupportedRoute,
            format!("cannot resume session `{session}`: route `{label}` has partial provenance; only `--route api` sessions can be resumed"),
        ));
    }
    let recorded_policy = start_ev.payload.get("policy_version").and_then(Value::as_str);
    if recorded_policy != Some(expected_policy_version) {
        return Err(reject(
            ResumeRejectKind::Drift,
            format!("cannot resume session `{session}`: policy_version `{}` differs from expected `{expected_policy_version}`", recorded_policy.unwrap_or("<missing>")),
        ));
    }
    check_tool_defs_drift(session, &start_ev.payload, expected_defs)?;

    let (mut resume_count, mut max_turn_seen, mut last_leg_idx) = (0u32, 0u32, 0usize);
    let (mut last_provider, mut last_model) = (None, None);
    for (idx, ev) in events.iter().enumerate() {
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                if ev.kind == "session.resume" {
                    resume_count += 1;
                }
                last_leg_idx = idx;
                last_provider = Some(ProviderKind::parse(req_str(ev, "provider")?).map_err(|e| corrupt(format!("seq {}: {e}", ev.seq)))?);
                last_model = Some(req_str(ev, "model")?.to_string());
            }
            "model.request" | "model.response" | "model.error" | "model.tool_result" => {
                if let Some(t) = ev.payload.get("turn").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()) {
                    max_turn_seen = max_turn_seen.max(t);
                }
            }
            _ => {}
        }
    }

    let mut prior_outcome = "interrupted".to_string();
    for ev in &events[last_leg_idx + 1..] {
        match ev.kind.as_str() {
            "session.end" => {
                prior_outcome = if ev.payload.get("outcome").and_then(Value::as_str) == Some("refusal") {
                    "refusal"
                } else {
                    "completed"
                }
                .to_string();
            }
            "session.abort" => prior_outcome = "aborted".to_string(),
            _ => {}
        }
    }
    let system_prompt = start_ev.payload.get("system_prompt").and_then(Value::as_str).unwrap_or(DEFAULT_SYSTEM_PROMPT).to_string();
    Ok((
        ResumeInspection {
            session: session.to_string(),
            last_provider: last_provider.ok_or_else(|| corrupt(format!("session `{session}` missing provider")))?,
            last_model: last_model.ok_or_else(|| corrupt(format!("session `{session}` missing model")))?,
            system_prompt,
            resumed_from_seq: events.last().map(|e| e.seq).unwrap_or(0),
            leg: 2 + resume_count,
            start_turn: max_turn_seen + 1,
            prior_outcome,
        },
        events,
    ))
}

fn check_tool_defs_drift(session: &str, payload: &Value, expected_defs: &[ToolDef]) -> Result<(), ResumeReject> {
    let recorded = payload
        .get("tool_defs")
        .and_then(Value::as_array)
        .ok_or_else(|| reject(ResumeRejectKind::Drift, format!("session `{session}` missing `tool_defs`")))?;
    if recorded.len() != expected_defs.len() {
        return Err(reject(
            ResumeRejectKind::Drift,
            format!("cannot resume session `{session}`: tool count changed (recorded {}, expected {})", recorded.len(), expected_defs.len()),
        ));
    }
    for (rec, exp) in recorded.iter().zip(expected_defs.iter()) {
        let same_id = rec.get("id").and_then(Value::as_str) == Some(exp.id.as_str());
        let same_hash = rec.get("def_hash").and_then(Value::as_str) == Some(exp.def_hash().as_str());
        if !same_id || !same_hash {
            return Err(reject(ResumeRejectKind::Drift, format!("cannot resume session `{session}`: tool definition drift for `{}`", exp.id)));
        }
    }
    Ok(())
}

pub fn inspect_session_for_resume(
    ledger: &Ledger,
    session: &str,
    expected_policy_version: &str,
    expected_defs: &[ToolDef],
) -> Result<ResumeInspection, ResumeReject> {
    let (insp, events) = verify_and_read(ledger, session, expected_policy_version, expected_defs)?;
    // Validate full blob & tool-output rehydration during pre-flight inspection
    // before any caller creates session directories, locks, or workers.
    rehydrate_history(ledger, &events, insp.last_provider)?;
    Ok(insp)
}

fn append_user_prompt(history: &mut Vec<TaggedMessage>, prompt: String) {
    if let Some(last) = history.last_mut().filter(|m| m.role == Role::User) {
        last.content.push(ContentBlock::Text { text: prompt });
        return;
    }
    history.push(TaggedMessage {
        role: Role::User,
        source_provider: None,
        content: vec![ContentBlock::Text { text: prompt }],
    });
}

fn decode_provider_response(provider: ProviderKind, raw: Vec<u8>) -> Result<ModelResponse, ResumeReject> {
    let res = match provider {
        ProviderKind::Anthropic => crate::wire::parse_response(raw, ""),
        ProviderKind::OpenAi | ProviderKind::Ollama => crate::openai::parse_openai_response(raw, ""),
        ProviderKind::Gemini => crate::gemini::parse_gemini_response(raw, ""),
        ProviderKind::ChatGpt => crate::responses::parse_responses_stream(raw, ""),
    };
    res.map_err(|e| corrupt(format!("failed to decode model.response for {}: {e}", provider.wire_id())))
}

fn decode_inline_or_blob_str(ledger: &Ledger, val: &Value) -> Result<String, ResumeReject> {
    if let Some(s) = val.get("inline").and_then(Value::as_str) {
        return Ok(s.to_string());
    }
    if let Some(h) = val.get("blob").and_then(Value::as_str) {
        let bytes = ledger
            .get_blob(h)
            .map_err(|e| reject(ResumeRejectKind::TamperOrMissing, format!("failed to load blob `{h}`: {e}")))?;
        return String::from_utf8(bytes).map_err(|e| corrupt(format!("tool result blob `{h}` is not UTF-8: {e}")));
    }
    Err(corrupt("tool.result payload missing inline or blob value"))
}

fn recover_tool_output(
    ledger: &Ledger,
    events: &[Event],
    mtr_idx: usize,
    recorded_text: &str,
    recorded_truncated: bool,
) -> Result<String, ResumeReject> {
    if !recorded_truncated {
        return Ok(recorded_text.to_string());
    }
    for ev in events[..mtr_idx].iter().rev() {
        match ev.kind.as_str() {
            "tool.result" => {
                let ok = ev.payload.get("ok").and_then(Value::as_bool)
                    .ok_or_else(|| corrupt(format!("seq {} tool.result missing `ok`", ev.seq)))?;
                let full = if ok {
                    let slot = ev.payload.get("result")
                        .ok_or_else(|| corrupt(format!("seq {} tool.result missing `result`", ev.seq)))?;
                    let raw = decode_inline_or_blob_str(ledger, slot)?;
                    serde_json::from_str::<Value>(&raw).ok().map(|v| tools::canon::canonical_json(&v)).unwrap_or(raw)
                } else {
                    let slot = ev.payload.get("error")
                        .ok_or_else(|| corrupt(format!("seq {} tool.result missing `error`", ev.seq)))?;
                    format!("ERROR: {}", decode_inline_or_blob_str(ledger, slot)?)
                };
                if clip_text(&full) != (recorded_text.to_string(), true) {
                    return Err(corrupt(format!("tool output reconstruction mismatch at seq {}", ev.seq)));
                }
                return Ok(full);
            }
            "tool.denied" => {
                let full = format!("DENIED: {}", req_str(ev, "why")?);
                if clip_text(&full) != (recorded_text.to_string(), true) {
                    return Err(corrupt(format!("denied tool output mismatch at seq {}", ev.seq)));
                }
                return Ok(full);
            }
            "delegate.end" | "explore.end" => {
                let mut cleaned = ev.payload.clone();
                if let Some(obj) = cleaned.as_object_mut() {
                    obj.remove("call_id");
                    obj.remove("duration_ms");
                }
                let canon = tools::canon::canonical_json(&cleaned);
                if clip_text(&canon) == (recorded_text.to_string(), true) {
                    return Ok(canon);
                }
                if ev.kind == "delegate.end" {
                    if let Some(h) = ev.payload.get("raw_blob").and_then(Value::as_str) {
                        let bytes = ledger.get_blob(h).map_err(|e| {
                            reject(ResumeRejectKind::TamperOrMissing, format!("failed to load blob `{h}`: {e}"))
                        })?;
                        let raw = String::from_utf8(bytes)
                            .map_err(|e| corrupt(format!("delegate.end blob `{h}` is not UTF-8: {e}")))?;
                        if clip_text(&raw) == (recorded_text.to_string(), true) {
                            return Ok(raw);
                        }
                    }
                }
                return Err(corrupt(format!("{} output reconstruction mismatch at seq {}", ev.kind, ev.seq)));
            }
            "model.tool_result" | "model.response" | "model.request" | "session.start" | "session.resume" => break,
            _ => {}
        }
    }
    Err(corrupt(format!("truncated model.tool_result at seq {} has no matching tool.result/tool.denied", events[mtr_idx].seq)))
}

fn collect_committed_tool_results(
    ledger: &Ledger,
    events: &[Event],
    resp_idx: usize,
    resp: &ModelResponse,
) -> Result<Option<Vec<ContentBlock>>, ResumeReject> {
    let expected_ids: Vec<&str> = resp.content.iter().filter_map(|b| match b {
        ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
        _ => None,
    }).collect();
    if expected_ids.is_empty() {
        return Ok(None);
    }

    let mut found: HashMap<String, ContentBlock> = HashMap::new();
    for (j, ev) in events.iter().enumerate().skip(resp_idx + 1) {
        if matches!(ev.kind.as_str(), "model.request" | "session.resume" | "session.abort" | "session.end") {
            break;
        }
        if ev.kind == "model.tool_result" {
            let tid = req_str(ev, "tool_use_id")?;
            let is_error = ev.payload.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let text = req_str(ev, "text")?;
            let truncated = ev.payload.get("truncated").and_then(Value::as_bool).unwrap_or(false);
            let content = recover_tool_output(ledger, events, j, text, truncated)?;
            found.insert(tid.to_string(), ContentBlock::ToolResult { tool_use_id: tid.to_string(), is_error, content });
        }
    }

    let mut ordered = Vec::with_capacity(expected_ids.len());
    for id in expected_ids {
        let Some(block) = found.remove(id) else {
            return Ok(None);
        };
        ordered.push(block);
    }
    Ok(Some(ordered))
}

pub(crate) fn rehydrate_history(
    ledger: &Ledger,
    events: &[Event],
    target_provider: ProviderKind,
) -> Result<Vec<Message>, ResumeReject> {
    let mut history: Vec<TaggedMessage> = Vec::new();
    let mut active_provider: Option<ProviderKind> = None;

    for (idx, ev) in events.iter().enumerate() {
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                active_provider = Some(ProviderKind::parse(req_str(ev, "provider")?).map_err(|e| corrupt(format!("seq {}: {e}", ev.seq)))?);
                append_user_prompt(&mut history, req_str(ev, "prompt")?.to_string());
            }
            "autonomous.feedback" => {
                if let Some(text) = ev.payload.get("text").and_then(Value::as_str) {
                    append_user_prompt(&mut history, text.to_string());
                }
            }
            "human.steer" => {
                if let Some(text) = ev.payload.get("text").and_then(Value::as_str) {
                    append_user_prompt(&mut history, format!("<human_steer>\n{text}\n</human_steer>"));
                }
            }
            "model.response" => {
                let leg_provider = active_provider.ok_or_else(|| corrupt(format!("seq {} model.response before start", ev.seq)))?;
                let blob_hash = req_str(ev, "raw_blob")?;
                let raw_bytes = ledger
                    .get_blob(blob_hash)
                    .map_err(|e| reject(ResumeRejectKind::TamperOrMissing, format!("seq {} blob `{blob_hash}`: {e}", ev.seq)))?;
                let mut resp = decode_provider_response(leg_provider, raw_bytes)?;

                if resp.stop_reason == StopReason::Refusal {
                    resp.content.retain(|b| {
                        !matches!(b, ContentBlock::ToolUse { .. })
                            && !matches!(b, ContentBlock::Other { block_type, .. } if block_type == "gemini_thought_signature")
                    });
                    if !resp.content.is_empty() {
                        history.push(TaggedMessage { role: Role::Assistant, source_provider: Some(leg_provider), content: resp.content });
                    }
                } else if validate_turn(&resp).is_some() {
                    continue;
                } else if matches!(resp.stop_reason, StopReason::EndTurn | StopReason::StopSequence) {
                    if !resp.content.is_empty() {
                        history.push(TaggedMessage { role: Role::Assistant, source_provider: Some(leg_provider), content: resp.content });
                    }
                } else if resp.stop_reason == StopReason::ToolUse {
                    if let Some(tool_results) = collect_committed_tool_results(ledger, events, idx, &resp)? {
                        history.push(TaggedMessage { role: Role::Assistant, source_provider: Some(leg_provider), content: resp.content });
                        history.push(TaggedMessage { role: Role::User, source_provider: None, content: tool_results });
                    }
                }
            }
            _ => {}
        }
    }

    Ok(history
        .into_iter()
        .map(|tm| {
            let content = if tm.role == Role::Assistant && tm.source_provider != Some(target_provider) {
                let mut kept: Vec<ContentBlock> = tm
                    .content
                    .into_iter()
                    .filter(|b| {
                        matches!(b, ContentBlock::Text { text } if !text.is_empty())
                            || matches!(b, ContentBlock::ToolUse { .. })
                    })
                    .collect();
                if kept.is_empty() {
                    kept.push(ContentBlock::Text { text: "[prior reasoning omitted]".to_string() });
                }
                kept
            } else {
                tm.content
            };
            Message { role: tm.role, content }
        })
        .collect())
}

pub fn resume_agent_loop(
    transport: &dyn Transport,
    router: &Router,
    ledger: &Ledger,
    cfg: &ResumeConfig,
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
    if cfg.prompt.trim().is_empty() {
        return Err("prompt must not be empty".into());
    }

    let session = router.session();
    let defs = router.registry().defs();
    let expected_defs: Vec<ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
    let policy_version = router.policy().version.as_str();
    let (inspection, events) =
        verify_and_read(ledger, session, policy_version, &expected_defs).map_err(|e| e.to_string())?;

    let info = transport.info();
    let target_provider = ProviderKind::parse(info.provider)?;
    let mut messages = rehydrate_history(ledger, &events, target_provider).map_err(|e| e.to_string())?;
    if let Some(last) = messages.last_mut().filter(|m| m.role == Role::User) {
        last.content.push(ContentBlock::Text { text: cfg.prompt.clone() });
    } else {
        messages.push(Message { role: Role::User, content: vec![ContentBlock::Text { text: cfg.prompt.clone() }] });
    }

    let system_prompt = cfg
        .system_prompt
        .as_deref()
        .unwrap_or(&inspection.system_prompt)
        .to_string();
    let mut audit = Audit { ledger, session, sink, last_seq: inspection.resumed_from_seq };
    audit.emit(
        "session.resume",
        json!({
            "route": "api",
            "provenance": "in-core loop, full provenance",
            "leg": inspection.leg,
            "resumed_from_seq": inspection.resumed_from_seq,
            "prior_outcome": inspection.prior_outcome,
            "start_turn": inspection.start_turn,
            "provider": info.provider,
            "endpoint": info.endpoint,
            "api_key_env": info.api_key_env,
            "model_requested": cfg.model_requested,
            "model": cfg.model,
            "prompt": cfg.prompt,
            "system_prompt": system_prompt,
            "max_turns": cfg.max_turns,
            "max_tokens": cfg.max_tokens,
            "policy_version": policy_version,
            "containment": cfg.containment,
            "contained": cfg.contained,
            "history_messages": messages.len(),
            "tool_defs": tool_defs_json(&defs),
        }),
    )?;

    run_turn_loop(
        transport,
        router,
        &mut audit,
        LoopParams {
            model: &cfg.model,
            system_prompt: &system_prompt,
            max_turns: cfg.max_turns,
            max_tokens: cfg.max_tokens,
            reasoning_effort: cfg.reasoning_effort.as_deref(),
            start_turn: inspection.start_turn,
            initial_messages: messages,
            mask_keep_recent: None,
            cancel: None,
            emit_end_event: true,
            delegation: None,
            human: None,
            pause_after_explore: false,
        },
    )
}
