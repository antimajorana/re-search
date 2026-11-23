//! Stage 15 Interactive Human-in-the-Loop hub, `HumanChannel` implementation,
//! and HTTP endpoints for live steering, checkpoints, and Explain promotions.

use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::{now_ms, DesktopState, SessionMeta};
use agent::{CancelToken, CheckpointPrompt, HumanChannel, HumanReply, SteerMessage, EVENT_VERSION};
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tools::canon::random_hex;

const CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_HITL_TEXT_CHARS: usize = 4_000;

pub struct PendingCheckpointSlot {
    pub prompt: CheckpointPrompt,
    pub reply: Mutex<Option<HumanReply>>,
    pub cv: Condvar,
}

#[derive(Default)]
pub struct HitlHub {
    steers: Mutex<HashMap<String, Vec<SteerMessage>>>,
    checkpoints: Mutex<HashMap<(String, String), Arc<PendingCheckpointSlot>>>,
}

impl HitlHub {
    pub fn queue_steer(&self, session_id: &str, text: String) -> SteerMessage {
        let msg = SteerMessage { steer_id: format!("st-{}", random_hex(8)), text, ts_ms: now_ms() };
        if let Ok(mut map) = self.steers.lock() {
            map.entry(session_id.to_string()).or_default().push(msg.clone());
        }
        msg
    }

    pub fn drain_steers(&self, session_id: &str) -> Vec<SteerMessage> {
        self.steers.lock().map(|mut m| m.remove(session_id).unwrap_or_default()).unwrap_or_default()
    }

    pub fn register_checkpoint(&self, session_id: &str, prompt: CheckpointPrompt) -> Arc<PendingCheckpointSlot> {
        let cid = prompt.checkpoint_id.clone();
        let slot = Arc::new(PendingCheckpointSlot { prompt, reply: Mutex::new(None), cv: Condvar::new() });
        if let Ok(mut map) = self.checkpoints.lock() {
            map.insert((session_id.to_string(), cid), Arc::clone(&slot));
        }
        slot
    }

    pub fn resolve_checkpoint(
        &self, session_id: &str, checkpoint_id: &str, selected_option: Option<String>, text: String, cancelled: bool,
    ) -> Result<HumanReply, String> {
        let slot = {
            let map = self.checkpoints.lock().map_err(|_| "checkpoint map poisoned".to_string())?;
            map.get(&(session_id.to_string(), checkpoint_id.to_string())).cloned()
        }
        .ok_or_else(|| format!("no pending checkpoint `{checkpoint_id}` for `{session_id}`"))?;
        let mut guard = slot.reply.lock().map_err(|_| "checkpoint slot poisoned".to_string())?;
        if guard.is_some() {
            return Err(format!("checkpoint `{checkpoint_id}` already resolved"));
        }
        let reply = HumanReply { checkpoint_id: checkpoint_id.to_string(), selected_option, text, cancelled };
        *guard = Some(reply.clone());
        slot.cv.notify_all();
        Ok(reply)
    }

    pub fn list_pending(&self, session_id: &str) -> Vec<CheckpointPrompt> {
        let slots: Vec<_> = match self.checkpoints.lock() {
            Ok(map) => map.iter().filter(|((s, _), _)| s == session_id).map(|(_, slot)| Arc::clone(slot)).collect(),
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<_> = slots
            .into_iter()
            .filter(|s| s.reply.lock().is_ok_and(|g| g.is_none()))
            .map(|s| s.prompt.clone())
            .collect();
        out.sort_by(|a, b| a.checkpoint_id.cmp(&b.checkpoint_id));
        out
    }

    pub fn cancel_session(&self, session_id: &str) {
        if let Ok(mut steers) = self.steers.lock() {
            steers.remove(session_id);
        }
        let slots: Vec<_> = match self.checkpoints.lock() {
            Ok(mut map) => {
                let keys: Vec<_> = map.keys().filter(|(s, _)| s == session_id).cloned().collect();
                keys.into_iter().filter_map(|k| map.remove(&k)).collect()
            }
            Err(_) => return,
        };
        for slot in slots {
            let mut guard = slot.reply.lock().unwrap_or_else(|p| p.into_inner());
            if guard.is_none() {
                *guard = Some(HumanReply {
                    checkpoint_id: slot.prompt.checkpoint_id.clone(),
                    selected_option: None,
                    text: "stopped by user".to_string(),
                    cancelled: true,
                });
                slot.cv.notify_all();
            }
        }
    }
}

pub struct DesktopHumanChannel {
    state: Arc<DesktopState>,
    session_id: String,
}

impl DesktopHumanChannel {
    pub fn new(state: Arc<DesktopState>, session_id: impl Into<String>) -> Self {
        Self { state, session_id: session_id.into() }
    }
}

impl HumanChannel for DesktopHumanChannel {
    fn drain_steers(&self) -> Vec<SteerMessage> {
        self.state.hitl.drain_steers(&self.session_id)
    }

    fn ask_checkpoint(&self, prompt: &CheckpointPrompt, cancel: Option<&CancelToken>) -> HumanReply {
        let slot = self.state.hitl.register_checkpoint(&self.session_id, prompt.clone());
        let mut payload = serde_json::to_value(prompt).unwrap_or_else(|_| json!({}));
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("session".to_string(), json!(&self.session_id));
        }
        self.state.sse.emit(&self.session_id, "checkpoint_requested", payload);

        let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
        let mut arrived = None;
        {
            let mut guard = slot.reply.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(r) = guard.take() { arrived = Some(r); break; }
                if cancel.is_some_and(CancelToken::is_cancelled) || self.state.sse.is_shutdown() { break; }
                let rem = deadline.saturating_duration_since(Instant::now());
                if rem.is_zero() { break; }
                guard = slot.cv.wait_timeout(guard, rem.min(Duration::from_millis(100)))
                    .unwrap_or_else(|p| p.into_inner()).0;
            }
        }
        if let Ok(mut map) = self.state.hitl.checkpoints.lock() {
            map.remove(&(self.session_id.clone(), prompt.checkpoint_id.clone()));
        }

        let aborted = cancel.is_some_and(CancelToken::is_cancelled) || self.state.sse.is_shutdown();
        if let Some(reply) = arrived {
            if reply.cancelled && aborted {
                self.state.sse.emit(
                    &self.session_id,
                    "checkpoint_resolved",
                    json!({ "session": &self.session_id, "checkpoint_id": &prompt.checkpoint_id, "cancelled": true }),
                );
            }
            return reply;
        }
        self.state.sse.emit(
            &self.session_id,
            "checkpoint_resolved",
            json!({ "session": &self.session_id, "checkpoint_id": &prompt.checkpoint_id, "cancelled": true }),
        );
        let reason = if cancel.is_some_and(CancelToken::is_cancelled) {
            "stopped by user"
        } else if self.state.sse.is_shutdown() {
            "desktop server shutting down"
        } else {
            "checkpoint timed out after 600s"
        };
        HumanReply {
            checkpoint_id: prompt.checkpoint_id.clone(),
            selected_option: None,
            text: reason.to_string(),
            cancelled: true,
        }
    }
}

fn validate_nonempty_text(val: &Value) -> Result<String, (u16, String)> {
    let text = val.get("text").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`text` must be a non-empty string"))?;
    if text.chars().count() > MAX_HITL_TEXT_CHARS {
        return Err(bad(format!("`text` exceeds {MAX_HITL_TEXT_CHARS} characters")));
    }
    Ok(text.to_string())
}

fn opt_str_field(val: &Value, key: &str) -> Result<Option<String>, (u16, String)> {
    match val.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok((!s.trim().is_empty()).then(|| s.trim().to_string())),
        Some(_) => Err(bad(format!("`{key}` must be a string or null"))),
    }
}

fn is_turn_active(state: &DesktopState, session_id: &str) -> Result<bool, (u16, String)> {
    state.active_turns.lock().map(|s| s.contains(session_id)).map_err(|_| internal("active turn lock poisoned"))
}

fn enqueue_and_emit_steer(state: &DesktopState, session_id: &str, text: String) -> SteerMessage {
    let msg = state.hitl.queue_steer(session_id, text);
    state.sse.emit(
        session_id,
        "steer_queued",
        json!({ "session": session_id, "steer_id": msg.steer_id, "text": msg.text, "ts_ms": msg.ts_ms }),
    );
    msg
}

fn record_explain_promoted(state: &DesktopState, session_id: &str, target: &str, text: &str) -> Result<(), (u16, String)> {
    let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
    let ledger = state.open_ledger().map_err(internal)?;
    ledger
        .append_with_hash(session_id, "explain.promoted", EVENT_VERSION, &json!({ "target": target, "text": text }))
        .map_err(|e| internal(format!("failed to record explain.promoted: {e}")))?;
    Ok(())
}

pub fn handle_post_steer(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let val = parse_json_body(body)?;
    let text = validate_nonempty_text(&val)?;
    if !is_turn_active(state, session_id)? {
        return Err((409, "no active turn to steer; send a normal message instead".into()));
    }
    let msg = enqueue_and_emit_steer(state, session_id, text);
    Ok((202, json!({ "ok": true, "steer": msg })))
}

pub fn handle_answer_checkpoint(state: &DesktopState, session_id: &str, checkpoint_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let cid = checkpoint_id.trim();
    if cid.is_empty() {
        return Err(bad("`checkpoint_id` must not be empty"));
    }
    let val = parse_json_body(body)?;
    let selected_option = opt_str_field(&val, "selected_option")?;
    let text = opt_str_field(&val, "text")?.unwrap_or_default();
    let cancelled = match val.get("cancelled") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad("`cancelled` must be a boolean")),
    };
    let reply = state
        .hitl
        .resolve_checkpoint(session_id, cid, selected_option, text, cancelled)
        .map_err(|e| if e.contains("already resolved") { conflict(e) } else { not_found(e) })?;
    state.sse.emit(
        session_id,
        "checkpoint_resolved",
        json!({
            "session": session_id, "checkpoint_id": reply.checkpoint_id,
            "selected_option": reply.selected_option, "text": reply.text, "cancelled": reply.cancelled,
        }),
    );
    Ok((200, json!({ "ok": true, "reply": reply })))
}

pub fn handle_explain_promote(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let target = val.get("target").and_then(Value::as_str).map(str::trim)
        .filter(|t| matches!(*t, "conventions" | "steer" | "prompt"))
        .ok_or_else(|| bad("`target` must be \"conventions\", \"steer\", or \"prompt\""))?;
    let text = validate_nonempty_text(&val)?;

    if target == "conventions" {
        let mut meta = state.load_meta(session_id).unwrap_or_else(|| SessionMeta {
            id: session_id.to_string(), title: session_id.to_string(), created_ms: now_ms(), ..SessionMeta::default()
        });
        let existing = meta.pinned_conventions.as_deref().map(str::trim).unwrap_or("");
        let combined = if existing.is_empty() { text.clone() } else { format!("{existing}\n{text}") };
        agent::validate_pinned_conventions(&combined).map_err(bad)?;
        meta.pinned_conventions = Some(combined.clone());
        state.save_meta(&meta).map_err(internal)?;
        record_explain_promoted(state, session_id, "conventions", &text)?;
        return Ok((200, json!({ "ok": true, "target": "conventions", "pinned_conventions": combined })));
    }
    record_explain_promoted(state, session_id, target, &text)?;
    if target == "steer" && is_turn_active(state, session_id)? {
        let msg = enqueue_and_emit_steer(state, session_id, text);
        return Ok((202, json!({ "ok": true, "mode": "steer", "steer": msg })));
    }
    Ok((200, json!({ "ok": true, "mode": "prompt", "text": text })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_prompt(id: &str) -> CheckpointPrompt {
        CheckpointPrompt {
            checkpoint_id: id.into(), kind: "plan".into(), question: "Approve plan?".into(),
            options: vec!["Approve".into(), "Adjust".into()], context: None, branches: Vec::new(),
        }
    }

    #[test]
    fn hitl_hub_steer_checkpoint_and_cancel() {
        let hub = Arc::new(HitlHub::default());
        assert!(hub.queue_steer("sess-a", "Focus on boundary".into()).steer_id.starts_with("st-"));
        hub.queue_steer("sess-b", "Other".into());
        assert_eq!(hub.drain_steers("sess-a").len(), 1);
        assert!(hub.drain_steers("sess-a").is_empty());

        let slot1 = hub.register_checkpoint("sess-1", sample_prompt("chk-1"));
        assert_eq!(hub.list_pending("sess-1").len(), 1);
        let reply = hub.resolve_checkpoint("sess-1", "chk-1", Some("Approve".into()), "go".into(), false).unwrap();
        assert_eq!(reply.selected_option.as_deref(), Some("Approve"));
        assert!(!reply.cancelled && slot1.reply.lock().unwrap().is_some() && hub.list_pending("sess-1").is_empty());

        let slot2 = hub.register_checkpoint("sess-1", sample_prompt("chk-2"));
        hub.queue_steer("sess-1", "stale".into());
        let waiter = std::thread::spawn(move || {
            let mut g = slot2.reply.lock().unwrap();
            while g.is_none() { g = slot2.cv.wait(g).unwrap(); }
            g.clone().unwrap()
        });
        hub.cancel_session("sess-1");
        let cancelled = waiter.join().unwrap();
        assert!(cancelled.cancelled && cancelled.text == "stopped by user");
        assert!(hub.drain_steers("sess-1").is_empty() && hub.list_pending("sess-1").is_empty());
    }
}
