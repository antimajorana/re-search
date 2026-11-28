//! Shared fixture builders for Evaluation Set R (`eval_set_r.rs`).

use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, ResumeConfig, StopReason,
    Transport, TransportError, TransportInfo, DEFAULT_MAX_TOKENS,
};
use ledger::Ledger;
use research_tools::investigation::RECORD_PAPER_MAP;
use research_tools::tool_profile::{
    investigation_v1, lookup as lookup_profile, register_profile, ProfileHostTools,
    MARKER_INVESTIGATION_STARTED, PROFILE_INVESTIGATION_V1,
};
use research_tools::{ingest_bytes, research_policy, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tools::{
    ApprovalReply, ApprovalRequest, Approver, CallCtx, CallOutcome, Registry, Router, ToolDef,
    ToolHandler, WorkerEpoch,
};

/// Self-written synthetic note used only as an in-test paper fixture (zero third-party text).
pub const SYNTHETIC_PAPER_TEX: &[u8] = br"\section{Harmonic Scaling Model}
We consider a one-dimensional oscillator with mass $m > 0$ and stiffness $k > 0$.
Assumption A1: the potential is strictly quadratic, $V(x) = \frac{1}{2} k x^2$.
Theorem T1: the angular frequency is $\omega = \sqrt{k / m}$ and is independent of amplitude.
Proof: energy conservation gives $\dot{x}^2 + (k/m) x^2 = (k/m) A^2$, yielding sinusoidal motion.
";

pub struct AllowAll;

impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "set-r-test".into() }
    }
}

struct NoWorkerEpoch;

impl WorkerEpoch for NoWorkerEpoch {
    fn live_epoch(&self) -> Option<u64> {
        None
    }

    fn is_contained(&self) -> bool {
        false
    }
}

pub struct ScriptedAnthropic {
    replies: Mutex<VecDeque<ModelResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedAnthropic {
    pub fn new(texts: &[&str]) -> Self {
        let replies = texts
            .iter()
            .enumerate()
            .map(|(idx, text)| {
                let id = format!("msg_setr_{}", idx + 1);
                let raw = json!({
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-haiku-4-5",
                    "stop_reason": "end_turn",
                    "content": [{ "type": "text", "text": text }],
                    "usage": { "input_tokens": 12, "output_tokens": 8 }
                });
                ModelResponse {
                    raw_body: serde_json::to_vec(&raw).unwrap(),
                    id: Some(id),
                    model: "claude-haiku-4-5".into(),
                    stop_reason: StopReason::EndTurn,
                    content: vec![ContentBlock::Text { text: (*text).to_string() }],
                    usage: json!({ "input_tokens": 12, "output_tokens": 8 }),
                }
            })
            .collect();
        Self { replies: Mutex::new(replies), requests: Mutex::new(Vec::new()) }
    }

    pub fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for ScriptedAnthropic {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "anthropic-messages",
            endpoint: "http://127.0.0.1:0/v1/messages".into(),
            api_key_env: "RE_SEARCH_API_KEY".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.requests.lock().unwrap().push(req.clone());
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        Ok(self.replies.lock().unwrap().pop_front().expect("scripted reply"))
    }
}

pub fn build_profile_router(
    data_dir: &Path,
    env: Arc<ResearchEnv>,
    session: &str,
    profile_name: &str,
) -> Router {
    let profile = lookup_profile(profile_name).unwrap();
    let mut registry = Registry::new();
    let stub: Arc<dyn ToolHandler> =
        Arc::new(|_: &CallCtx<'_>, _: &Value| Err("no compute worker in Set R test".into()));
    let policy = register_profile(
        &profile,
        &mut registry,
        env,
        ProfileHostTools {
            sympy_epoch: Arc::new(NoWorkerEpoch),
            sympy_handler: stub,
            session_title_handler: None,
            solve_check: None,
        },
    )
    .unwrap();
    let ledger = Ledger::open(data_dir).unwrap();
    Router::new(registry, policy, ledger, session, Arc::new(AllowAll))
        .with_tool_profile(profile.name)
}

pub fn defs_json(defs: &[ToolDef]) -> Vec<Value> {
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

pub fn resume_cfg(prompt: &str) -> ResumeConfig {
    ResumeConfig {
        prompt: prompt.into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: None,
        max_turns: 8,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(),
        contained: false,
        reasoning_effort: None,
    }
}

/// Write a real hash-chained pre-F1 `session.start` (no `tool_profile` field),
/// `model.request`, `model.response`, `model.text`, and `session.end` using real ledger blobs.
pub fn write_pref1_completed_leg1(
    ledger: &Ledger,
    session: &str,
    prompt: &str,
    reply_text: &str,
    tool_defs: Vec<Value>,
) {
    ledger
        .append_with_hash(
            session,
            "session.start",
            1,
            &json!({
                "route": "api",
                "provenance": "in-core loop, full provenance",
                "provider": "anthropic-messages",
                "endpoint": "http://127.0.0.1:0/v1/messages",
                "api_key_env": "RE_SEARCH_API_KEY",
                "model_requested": "claude-haiku-4-5",
                "model": "claude-haiku-4-5",
                "prompt": prompt,
                "system_prompt": "Pre-F1 recorded system prompt.",
                "max_turns": 16,
                "max_tokens": DEFAULT_MAX_TOKENS,
                "policy_version": research_policy().version,
                "containment": "none",
                "contained": false,
                "tool_defs": tool_defs,
            }),
        )
        .unwrap();

    let req_body = json!({
        "model": "claude-haiku-4-5",
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": prompt }] }]
    });
    let req_blob = ledger.put_blob(&serde_json::to_vec(&req_body).unwrap()).unwrap();
    ledger
        .append_with_hash(
            session,
            "model.request",
            1,
            &json!({
                "turn": 1,
                "model": "claude-haiku-4-5",
                "message_count": 1,
                "max_tokens": DEFAULT_MAX_TOKENS,
                "raw_blob": req_blob,
            }),
        )
        .unwrap();

    let resp_body = json!({
        "id": "msg_leg1",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": reply_text }],
        "usage": { "input_tokens": 10, "output_tokens": 6 }
    });
    let resp_blob = ledger.put_blob(&serde_json::to_vec(&resp_body).unwrap()).unwrap();
    ledger
        .append_with_hash(
            session,
            "model.response",
            1,
            &json!({
                "turn": 1,
                "response_id": "msg_leg1",
                "model": "claude-haiku-4-5",
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 10, "output_tokens": 6 },
                "duration_ms": 12,
                "raw_blob": resp_blob,
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(session, "model.text", 1, &json!({ "text": reply_text }))
        .unwrap();
    ledger
        .append_with_hash(
            session,
            "session.end",
            1,
            &json!({
                "outcome": "completed",
                "stop_reason": "end_turn",
                "num_turns": 1,
                "last_turn": 1,
                "duration_ms": 15
            }),
        )
        .unwrap();
}

/// Ingest the self-written synthetic paper using the real `research_tools::ingest_bytes` writer.
pub fn ingest_synthetic_paper(
    data_dir: &Path,
    env: &ResearchEnv,
    session: &str,
) -> (String, String) {
    let ledger = Mutex::new(Ledger::open(data_dir).unwrap());
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: session,
            file_name: "harmonic_scaling.tex",
            origin: "upload",
            raw_bytes: SYNTHETIC_PAPER_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Harmonic Scaling Model"),
        },
    )
    .unwrap();
    (ingested.source_id, ingested.primary_path.expect("synthetic tex must have primary_path"))
}

/// Build an Investigation session with a real ingested synthetic paper and a real `record_paper_map`
/// call (used by both Set R.3 and as the parent for Set R.4).
pub fn build_current_investigation_session(
    data_dir: &Path,
    env: &Arc<ResearchEnv>,
    session: &str,
) -> (String, String, u64, String) {
    let ledger = Ledger::open(data_dir).unwrap();
    ledger
        .append_with_hash(session, MARKER_INVESTIGATION_STARTED, 1, &json!({ "title": "Harmonic Scaling" }))
        .unwrap();
    let (source_id, primary_path) = ingest_synthetic_paper(data_dir, env, session);
    ledger
        .append_with_hash(
            session,
            "investigation.paper",
            1,
            &json!({ "source_id": source_id, "primary_path": primary_path }),
        )
        .unwrap();
    write_pref1_completed_leg1(
        &ledger,
        session,
        "Map the harmonic scaling paper.",
        "Recorded the paper argument map.",
        defs_json(&investigation_v1().defs),
    );
    ledger
        .append_with_hash(
            session,
            "investigation.mapping_requested",
            1,
            &json!({ "source_id": source_id, "summary_requested": false }),
        )
        .unwrap();
    let router = build_profile_router(data_dir, Arc::clone(env), session, PROFILE_INVESTIGATION_V1);
    let map_args = json!({
        "source_id": source_id,
        "title": "Harmonic Scaling Model",
        "coverage": "paper",
        "limitations": [],
        "nodes": [
            {
                "id": "a1",
                "sentence": "The potential is strictly quadratic.",
                "kind": "assumption",
                "main": false,
                "anchors": [{
                    "source_id": source_id,
                    "path": primary_path,
                    "start_line": 3,
                    "end_line": 3,
                    "quote": "Assumption A1: the potential is strictly quadratic, $V(x) = \\frac{1}{2} k x^2$.",
                    "kind": "passage"
                }]
            },
            {
                "id": "t1",
                "sentence": "The angular frequency is sqrt(k/m) independent of amplitude.",
                "kind": "result",
                "main": true,
                "anchors": [{
                    "source_id": source_id,
                    "path": primary_path,
                    "start_line": 4,
                    "end_line": 4,
                    "quote": "Theorem T1: the angular frequency is $\\omega = \\sqrt{k / m}$ and is independent of amplitude.",
                    "kind": "passage"
                }]
            }
        ],
        "edges": [{
            "from": "a1",
            "to": "t1",
            "kind": "depends_on",
            "basis": "explicit",
            "reason": "Quadratic potential yields amplitude-independent angular frequency.",
            "anchor": {
                "source_id": source_id,
                "path": primary_path,
                "start_line": 5,
                "end_line": 5,
                "quote": "Proof: energy conservation gives $\\dot{x}^2 + (k/m) x^2 = (k/m) A^2$, yielding sinusoidal motion.",
                "kind": "passage"
            }
        }]
    });
    let outcome = router
        .try_call(&format!("agent:anthropic-messages:{session}"), RECORD_PAPER_MAP, &map_args)
        .unwrap();
    assert!(matches!(outcome, CallOutcome::Ok(_)), "record_paper_map failed: {outcome:?}");

    let events = ledger.read_session(session).unwrap();
    let map_ev = events.iter().rev().find(|e| e.kind == "investigation.map").unwrap();
    (source_id, primary_path, map_ev.seq, map_ev.event_hash.clone())
}
