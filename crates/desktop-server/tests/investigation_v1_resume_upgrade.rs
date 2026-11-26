//! Regression & integration tests for `investigation/1 -> investigation/2` forward upgrade
//! on session resume and Investigate v2 task execution (`00-overview.md` §2.2, `investigate.md` §8.2, §8.4).

mod common;

use agent::{
    inspect_session_for_resume, inspect_session_for_resume_with_profile,
    inspect_session_with_profile, resume_autonomous_research_loop, AutonomousConfig,
    AutonomousRoles, ContentBlock, ModelRequest, ModelResponse, NoopSink, PreparedRequest,
    ResumeConfig, ResumeRejectKind, SessionOutcome, StopReason, Transport, TransportError,
    TransportInfo, DEFAULT_MAX_TOKENS,
};
use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use policy::EffectClass;
use research_tools::investigation::{
    GOLDEN_BRANCH_7C36A0E, GOLDEN_CONVENTIONS_7C36A0E, GOLDEN_MAP_7C36A0E, GOLDEN_SUMMARY_7C36A0E,
    RECORD_PAPER_BRANCH, RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP, RECORD_PAPER_SUMMARY,
};
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON, MARKER_TASK_FINISHED,
    MARKER_TASK_REQUESTED,
};
use research_tools::map_defs::{
    read_map_node_def, record_map_reading_def, INVESTIGATION_V2_MAY_GAIN, READ_MAP_NODE,
    RECORD_CONTEXT_CARDS, RECORD_MAP_READING, RECORD_PRIOR_WORK_CHECK,
};
use research_tools::paper_key::PaperKey;
use research_tools::skeleton::{build_tex_skeleton_from_ingested, PaperSkeleton, SkNodeKind};
use research_tools::tool_profile::{
    active_profile_for_session_events, investigation_v1, investigation_v2,
    lookup as lookup_profile, profile_for_session_events, register_profile, research_v1,
    ProfileHostTools, MARKER_INVESTIGATION_STARTED, PROFILE_INVESTIGATION_V1,
    PROFILE_INVESTIGATION_V2, PROFILE_RESEARCH_V1, TOOL_PROFILE_FIELD,
};
use research_tools::{ingest_bytes, research_policy, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tools::{
    ApprovalReply, ApprovalRequest, Approver, CallCtx, Registry, Router, ToolDef, ToolHandler,
    WorkerEpoch,
};

const SYNTHETIC_INV_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\begin{abstract}
We establish uniform energy damping for a linear coupled oscillator chain.
\end{abstract}
\section{Model}\label{sec:model}
\begin{lemma}[Coupling Bound]\label{lem:couple}
Every displacement vector $q$ satisfies $U(q) \ge \kappa \|q\|^2$.
\end{lemma}
\begin{theorem}[Chain Decay]\label{thm:chain}
Under Lemma~\ref{lem:couple}, the total oscillator energy decays as $E(t) \le E(0) e^{-\mu t}$.
\end{theorem}
\begin{proof}
Differentiating $E(t)$ and substituting Lemma~\ref{lem:couple} gives exponential decay.
\end{proof}
\end{document}
"#;

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "upgrade-test".into() }
    }
}

struct NoEpoch;
impl WorkerEpoch for NoEpoch {
    fn live_epoch(&self) -> Option<u64> { None }
    fn is_contained(&self) -> bool { false }
}

struct ScriptedTurnTransport {
    replies: Mutex<VecDeque<ModelResponse>>,
}

impl ScriptedTurnTransport {
    fn new(steps: Vec<ModelResponse>) -> Self {
        Self { replies: Mutex::new(VecDeque::from(steps)) }
    }

    fn text(id: &str, text: &str) -> ModelResponse {
        let raw = json!({
            "id": id, "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
            "usage": { "input_tokens": 10, "output_tokens": 8 }
        });
        ModelResponse {
            raw_body: serde_json::to_vec(&raw).unwrap(),
            id: Some(id.into()),
            model: "claude-haiku-4-5".into(),
            stop_reason: StopReason::EndTurn,
            content: vec![ContentBlock::Text { text: text.into() }],
            usage: json!({ "input_tokens": 10, "output_tokens": 8 }),
        }
    }

    fn tool(id: &str, call_id: &str, name: &str, input: Value) -> ModelResponse {
        let raw = json!({
            "id": id, "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "tool_use",
            "content": [{ "type": "tool_use", "id": call_id, "name": name, "input": input }],
            "usage": { "input_tokens": 12, "output_tokens": 16 }
        });
        ModelResponse {
            raw_body: serde_json::to_vec(&raw).unwrap(),
            id: Some(id.into()),
            model: "claude-haiku-4-5".into(),
            stop_reason: StopReason::ToolUse,
            content: vec![ContentBlock::ToolUse { id: call_id.into(), name: name.into(), input }],
            usage: json!({ "input_tokens": 12, "output_tokens": 16 }),
        }
    }
}

impl Transport for ScriptedTurnTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "anthropic-messages",
            endpoint: "http://127.0.0.1:0/v1/messages".into(),
            api_key_env: "RE_SEARCH_API_KEY".into(),
        }
    }
    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }
    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        Ok(self.replies.lock().unwrap().pop_front().expect("scripted response"))
    }
}

fn build_router(dir: &Path, env: Arc<ResearchEnv>, sid: &str, profile_name: &str) -> Router {
    let profile = lookup_profile(profile_name).unwrap();
    let mut reg = Registry::new();
    let stub: Arc<dyn ToolHandler> = Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({})));
    let host = ProfileHostTools { sympy_epoch: Arc::new(NoEpoch), sympy_handler: stub, session_title_handler: None, solve_check: None };
    let pol = register_profile(&profile, &mut reg, env, host).unwrap();
    Router::new(reg, pol, Ledger::open(dir).unwrap(), sid, Arc::new(AllowAll)).with_tool_profile(profile.name)
}

fn resume_cfg(prompt: &str) -> ResumeConfig {
    ResumeConfig {
        prompt: prompt.into(), model_requested: "claude-haiku-4-5".into(), model: "claude-haiku-4-5".into(),
        system_prompt: None, max_turns: 8, max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(), contained: false, reasoning_effort: None,
    }
}

fn write_old_inv_leg1(
    dir: &Path, sid: &str, explicit_v1: bool, with_skeleton: bool,
) -> (Arc<ResearchEnv>, String, String, Option<(u64, PaperSkeleton)>) {
    let env = Arc::new(ResearchEnv::new(dir));
    let ledger = Ledger::open(dir).unwrap();
    ledger.append(sid, MARKER_INVESTIGATION_STARTED, 1, &json!({ "title": "Chain decay" })).unwrap();
    let ledger_m = Mutex::new(Ledger::open(dir).unwrap());
    let ing = ingest_bytes(&env.sandbox(), &ledger_m, &IngestRequest {
        session_id: sid, file_name: "chain.tex", origin: "https://arxiv.org/e-print/2408.00123v1",
        raw_bytes: SYNTHETIC_INV_TEX, source_id_hint: None, section_filter: None, title: Some("Chain decay"),
    }).unwrap();
    drop(ledger_m);
    let primary_path = ing.primary_path.clone().unwrap();
    ledger.append(sid, "investigation.paper", 1, &json!({ "source_id": ing.source_id })).unwrap();

    let skel_info = if with_skeleton {
        let receipt = ledger.read_session(sid).unwrap().into_iter().find(|e| e.kind == "source.ingested").unwrap();
        let (out, _) = build_tex_skeleton_from_ingested(
            &env.sandbox(), sid, &receipt, PaperKey::from_arxiv("2408.00123").unwrap(), Some("v1".into()),
        ).unwrap();
        let blob = ledger.put_blob(&serde_json::to_vec(&out.skeleton).unwrap()).unwrap();
        let seq = ledger.append(sid, EVENT_INVESTIGATION_SKELETON, 1, &json!({
            "source_id": ing.source_id, "source_seq": ing.event_seq,
            "builder": "tex-1", "paper_key": "arxiv:2408.00123", "blob": blob,
        })).unwrap();
        Some((seq, out.skeleton))
    } else {
        None
    };

    let tool_defs: Vec<Value> = if explicit_v1 {
        investigation_v1().defs.iter().map(|d| json!({ "id": d.id, "def_hash": d.def_hash() })).collect()
    } else {
        investigation_v1().defs.iter().filter(|d| d.id != "read_paper_lines").map(|d| {
            let h = match d.id.as_str() {
                RECORD_PAPER_MAP => GOLDEN_MAP_7C36A0E.to_string(),
                RECORD_PAPER_SUMMARY => GOLDEN_SUMMARY_7C36A0E.to_string(),
                RECORD_PAPER_BRANCH => GOLDEN_BRANCH_7C36A0E.to_string(),
                RECORD_PAPER_CONVENTIONS => GOLDEN_CONVENTIONS_7C36A0E.to_string(),
                _ => d.def_hash(),
            };
            json!({ "id": d.id, "def_hash": h })
        }).collect()
    };

    let mut start_payload = json!({
        "route": "api", "provenance": "in-core loop, full provenance",
        "provider": "anthropic-messages", "endpoint": "http://127.0.0.1:0/v1/messages",
        "api_key_env": "RE_SEARCH_API_KEY", "model_requested": "claude-haiku-4-5", "model": "claude-haiku-4-5",
        "prompt": "Map the oscillator paper.", "system_prompt": "Investigation v1 prompt.",
        "max_turns": 8, "max_tokens": DEFAULT_MAX_TOKENS, "policy_version": research_policy().version,
        "containment": "none", "contained": false, "tool_defs": tool_defs,
    });
    if explicit_v1 { start_payload[TOOL_PROFILE_FIELD] = json!(PROFILE_INVESTIGATION_V1); }
    ledger.append(sid, "session.start", 1, &start_payload).unwrap();

    let req_blob = ledger.put_blob(&serde_json::to_vec(&json!({ "model": "claude-haiku-4-5", "messages": [] })).unwrap()).unwrap();
    ledger.append(sid, "model.request", 1, &json!({
        "turn": 1, "model": "claude-haiku-4-5", "message_count": 1, "max_tokens": DEFAULT_MAX_TOKENS, "raw_blob": req_blob
    })).unwrap();
    let resp_body = json!({
        "id": "msg_v1_1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Mapped v1 paper." }],
        "usage": { "input_tokens": 10, "output_tokens": 6 }
    });
    let resp_blob = ledger.put_blob(&serde_json::to_vec(&resp_body).unwrap()).unwrap();
    ledger.append(sid, "model.response", 1, &json!({
        "turn": 1, "response_id": "msg_v1_1", "model": "claude-haiku-4-5", "stop_reason": "end_turn",
        "usage": { "input_tokens": 10, "output_tokens": 6 }, "duration_ms": 5, "raw_blob": resp_blob
    })).unwrap();
    ledger.append(sid, "model.text", 1, &json!({ "text": "Mapped v1 paper." })).unwrap();
    ledger.append(sid, "session.end", 1, &json!({
        "outcome": "completed", "stop_reason": "end_turn", "num_turns": 1, "last_turn": 1, "duration_ms": 8
    })).unwrap();
    (env, ing.source_id, primary_path, skel_info)
}

#[test]
fn old_investigation_v1_sessions_upgrade_to_v2_on_resume_execute_v6_tools_and_reject_downgrade() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();

    for (sid, explicit_v1) in [("s-upg-pref1-7c36a0e", false), ("s-upg-explicit-v1", true)] {
        let (env, source_id, primary_path, skel_opt) = write_old_inv_leg1(dir.path(), sid, explicit_v1, true);
        let (skel_seq, skel) = skel_opt.unwrap();
        let thm = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
        let thm_tag = thm.tag.as_deref().unwrap();

        // Before upgrade resume: recorded profile is investigation/1, active profile is investigation/2.
        let pre_events = ledger.read_session(sid).unwrap();
        assert_eq!(profile_for_session_events(&pre_events).unwrap().name, PROFILE_INVESTIGATION_V1);
        assert_eq!(active_profile_for_session_events(&pre_events).unwrap().name, PROFILE_INVESTIGATION_V2);
        let (insp, resolved) = inspect_session_with_profile(&ledger, sid).unwrap();
        assert_eq!(resolved.profile.name, PROFILE_INVESTIGATION_V1);
        assert_eq!(resolved.explicit, explicit_v1);
        assert_eq!(insp.tool_profile, PROFILE_INVESTIGATION_V1);

        // Open a V6 task marker so `record_map_reading` is open during leg 2.
        let req_seq = ledger
            .append(sid, MARKER_TASK_REQUESTED, 1, &json!({ "task": "orient", "tags": [], "source_id": source_id, "skeleton_seq": skel_seq }))
            .unwrap();

        let v2_router = build_router(dir.path(), Arc::clone(&env), sid, PROFILE_INVESTIGATION_V2);
        let transport = ScriptedTurnTransport::new(vec![
            ScriptedTurnTransport::tool("msg_leg2_t1", "tu_rmn", READ_MAP_NODE, json!({ "source_id": source_id, "node": "Theorem 1.2" })),
            ScriptedTurnTransport::tool(
                "msg_leg2_t2",
                "tu_rmr",
                RECORD_MAP_READING,
                json!({
                    "request_seq": req_seq,
                    "source_id": source_id,
                    "items": [{
                        "tag": thm_tag,
                        "role": "main_result",
                        "gist": "Coupled oscillator energy decays exponentially.",
                        "anchors": [{
                            "source_id": source_id,
                            "path": primary_path,
                            "start_line": thm.spans[0].start_line,
                            "end_line": thm.spans[0].end_line,
                            "quote": "the total oscillator energy decays as $E(t) \\le E(0) e^{-\\mu t}$"
                        }]
                    }],
                    "edges": [], "limitations": [], "done": true
                }),
            ),
            ScriptedTurnTransport::text("msg_leg2_end", "Saved V6 reading on resumed v1 session."),
            ScriptedTurnTransport::text("msg_leg3_end", "Leg 3 follow-up under investigation/2."),
        ]);

        let out2 = resume_autonomous_research_loop(
            &transport,
            &AutonomousRoles::default(),
            &v2_router,
            &ledger,
            &env,
            &resume_cfg("Run V6 orient on upgraded session."),
            &AutonomousConfig::default(),
            None,
            &mut NoopSink,
        )
        .unwrap();
        assert!(matches!(out2, SessionOutcome::Completed { .. }), "{out2:?}");
        ledger.append(sid, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req_seq, "status": "done" })).unwrap();

        let after_leg2 = ledger.read_session(sid).unwrap();
        assert_eq!(profile_for_session_events(&after_leg2).unwrap().name, PROFILE_INVESTIGATION_V2);
        assert_eq!(after_leg2.iter().filter(|e| e.kind == EVENT_INVESTIGATION_READING).count(), 1);
        let res1 = after_leg2.iter().find(|e| e.kind == "session.resume").unwrap();
        assert_eq!(res1.payload[TOOL_PROFILE_FIELD], PROFILE_INVESTIGATION_V2);

        // Leg 3 resume under investigation/2 succeeds.
        let out3 = resume_autonomous_research_loop(
            &transport,
            &AutonomousRoles::default(),
            &v2_router,
            &ledger,
            &env,
            &resume_cfg("Confirm Theorem 1.2 assumptions."),
            &AutonomousConfig::default(),
            None,
            &mut NoopSink,
        )
        .unwrap();
        assert!(matches!(out3, SessionOutcome::Completed { .. }), "{out3:?}");

        // Downgrading back to investigation/1 or switching to research/1 is rejected fail-closed!
        for bad_profile in [PROFILE_INVESTIGATION_V1, PROFILE_RESEARCH_V1] {
            let bad_router = build_router(dir.path(), Arc::clone(&env), sid, bad_profile);
            let err = resume_autonomous_research_loop(
                &ScriptedTurnTransport::new(vec![]),
                &AutonomousRoles::default(),
                &bad_router,
                &ledger,
                &env,
                &resume_cfg("Must reject downgrade or cross-mode"),
                &AutonomousConfig::default(),
                None,
                &mut NoopSink,
            )
            .unwrap_err();
            assert!(err.contains("active router profile"), "expected Drift error, got: {err}");
        }
    }
    let verified = ledger.verify_all().unwrap();
    assert_eq!(verified.sessions.len(), 2);
}

#[test]
fn investigation_v1_accepts_all_four_v2_may_gain_read_workspace_tools_and_rejects_unapproved() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();

    let mk_reserved_def = |id: &str, effect: EffectClass| ToolDef {
        id: id.into(),
        description: format!("Reserved Investigate v2 tool {id}"),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        effect,
        approval_scope: tools::ApprovalScope::Once,
    };

    let mut all_four_defs = investigation_v1().defs;
    all_four_defs.push(read_map_node_def());
    all_four_defs.push(record_map_reading_def());
    all_four_defs.push(mk_reserved_def(RECORD_CONTEXT_CARDS, EffectClass::ReadWorkspace));
    all_four_defs.push(mk_reserved_def(RECORD_PRIOR_WORK_CHECK, EffectClass::ReadWorkspace));
    all_four_defs.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(INVESTIGATION_V2_MAY_GAIN.len(), 4);

    let mut with_unapproved = all_four_defs.clone();
    with_unapproved.push(mk_reserved_def("unapproved_map_tool", EffectClass::ReadWorkspace));
    with_unapproved.sort_by(|a, b| a.id.cmp(&b.id));
    let mut wrong_effect = investigation_v1().defs;
    wrong_effect.push(mk_reserved_def(RECORD_CONTEXT_CARDS, EffectClass::WriteWorkspace));
    wrong_effect.sort_by(|a, b| a.id.cmp(&b.id));

    // Unlabelled pre-F1 `investigation/1`: may_gain tolerates all 4 ReadWorkspace tools even with active_router_profile = None,
    // and also upgrades cleanly via inspect_session_for_resume_with_profile(..., Some(PROFILE_INVESTIGATION_V2)).
    write_old_inv_leg1(dir.path(), "s-gain4-pref1", false, false);
    let insp = inspect_session_for_resume(&ledger, "s-gain4-pref1", &research_policy().version, &all_four_defs).unwrap();
    assert_eq!(insp.tool_profile, PROFILE_INVESTIGATION_V1);
    let upg = inspect_session_for_resume_with_profile(
        &ledger, "s-gain4-pref1", &research_policy().version, &investigation_v2().defs, Some(PROFILE_INVESTIGATION_V2),
    ).unwrap();
    assert_eq!(upg.tool_profile, PROFILE_INVESTIGATION_V2);
    assert_eq!(inspect_session_for_resume(&ledger, "s-gain4-pref1", &research_policy().version, &with_unapproved).unwrap_err().kind, ResumeRejectKind::Drift);
    assert_eq!(inspect_session_for_resume(&ledger, "s-gain4-pref1", &research_policy().version, &wrong_effect).unwrap_err().kind, ResumeRejectKind::Drift);

    // Explicitly labelled `investigation/1`: when active_router_profile = None, defs_match_exact against `investigation/1`
    // rejects `investigation/2` and `all_four_defs` with Drift, while explicit upgrade with Some("investigation/2") succeeds.
    write_old_inv_leg1(dir.path(), "s-gain4-explicit", true, false);
    let insp_exact = inspect_session_for_resume(&ledger, "s-gain4-explicit", &research_policy().version, &investigation_v1().defs).unwrap();
    assert_eq!(insp_exact.tool_profile, PROFILE_INVESTIGATION_V1);
    assert_eq!(inspect_session_for_resume(&ledger, "s-gain4-explicit", &research_policy().version, &investigation_v2().defs).unwrap_err().kind, ResumeRejectKind::Drift);
    assert_eq!(inspect_session_for_resume(&ledger, "s-gain4-explicit", &research_policy().version, &all_four_defs).unwrap_err().kind, ResumeRejectKind::Drift);
    let upg_exp = inspect_session_for_resume_with_profile(
        &ledger, "s-gain4-explicit", &research_policy().version, &investigation_v2().defs, Some(PROFILE_INVESTIGATION_V2),
    ).unwrap();
    assert_eq!(upg_exp.tool_profile, PROFILE_INVESTIGATION_V2);
    assert_eq!(
        inspect_session_for_resume_with_profile(&ledger, "s-gain4-explicit", &research_policy().version, &all_four_defs, Some(PROFILE_INVESTIGATION_V2)).unwrap_err().kind,
        ResumeRejectKind::Drift
    );
    assert_eq!(inspect_session_for_resume(&ledger, "s-gain4-explicit", &research_policy().version, &with_unapproved).unwrap_err().kind, ResumeRejectKind::Drift);
    assert_eq!(inspect_session_for_resume(&ledger, "s-gain4-explicit", &research_policy().version, &wrong_effect).unwrap_err().kind, ResumeRejectKind::Drift);

    // `research/1` session must reject gaining any of the four Investigate v2 tools.
    let r1_defs: Vec<Value> = research_v1().defs.iter().map(|d| json!({ "id": d.id, "def_hash": d.def_hash() })).collect();
    ledger
        .append(
            "s-res1-frozen",
            "session.start",
            1,
            &json!({
                "route": "api", "provider": "anthropic-messages", "model": "claude-haiku-4-5",
                "prompt": "hi", "policy_version": research_policy().version,
                "tool_profile": PROFILE_RESEARCH_V1, "tool_defs": r1_defs,
            }),
        )
        .unwrap();
    let err = inspect_session_for_resume(&ledger, "s-res1-frozen", &research_policy().version, &all_four_defs).unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
}

fn start_mock_anthropic_http(replies: Vec<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            if len > 0 {
                let mut buf = vec![0u8; len];
                let _ = reader.read_exact(&mut buf);
            }
            let reply = queue.lock().unwrap().pop_front().unwrap_or_else(|| {
                json!({
                    "id": "msg_done", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                    "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Done." }],
                    "usage": { "input_tokens": 8, "output_tokens": 6 }
                })
            });
            let bytes = serde_json::to_vec(&reply).unwrap();
            let hdr = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
            let _ = stream.write_all(hdr.as_bytes());
            let _ = stream.write_all(&bytes);
        }
    });
    format!("http://{addr}")
}

fn http_json(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
    let hdr = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
}

#[test]
fn live_http_v6_task_on_old_investigation_v1_session_auto_builds_skeleton_and_finishes_202() {
    let dir = tempfile::tempdir().unwrap();
    let sid = "s-http-old-inv-v1";
    // Old explicit `investigation/1` session WITH pinned `investigation.paper` and ingested TeX,
    // but WITHOUT any `investigation.skeleton` event yet (§8.4).
    let (env, source_id, primary_path, _) = write_old_inv_leg1(dir.path(), sid, true, false);
    let ledger = Ledger::open(dir.path()).unwrap();
    let pre_evs = ledger.read_session(sid).unwrap();
    assert!(!pre_evs.iter().any(|e| e.kind == EVENT_INVESTIGATION_SKELETON));

    let receipt = pre_evs.iter().find(|e| e.kind == "source.ingested").unwrap();
    let (expected_skel, _) = build_tex_skeleton_from_ingested(
        &env.sandbox(),
        sid,
        receipt,
        PaperKey::from_arxiv("2408.00123").unwrap(),
        Some("v1".into()),
    )
    .unwrap();
    let thm = expected_skel.skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let thm_tag = thm.tag.as_deref().unwrap().to_string();
    // Auto-built skeleton is appended at `last_seq + 1`, and `task_requested` at `last_seq + 2`.
    let expected_req_seq = pre_evs.last().unwrap().seq + 2;

    let base_url = start_mock_anthropic_http(vec![
        json!({
            "id": "m1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "tool_use",
            "content": [{ "type": "tool_use", "id": "tu1", "name": READ_MAP_NODE, "input": { "source_id": source_id, "node": "Theorem 1.2" } }],
            "usage": { "input_tokens": 12, "output_tokens": 16 }
        }),
        json!({
            "id": "m2", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use", "id": "tu2", "name": RECORD_MAP_READING,
                "input": {
                    "request_seq": expected_req_seq,
                    "source_id": source_id,
                    "items": [{
                        "tag": thm_tag,
                        "role": "main_result",
                        "gist": "Uniform exponential energy decay along the oscillator chain.",
                        "anchors": [{
                            "source_id": source_id,
                            "path": primary_path,
                            "start_line": thm.spans[0].start_line,
                            "end_line": thm.spans[0].end_line,
                            "quote": "the total oscillator energy decays as $E(t) \\le E(0) e^{-\\mu t}$"
                        }]
                    }],
                    "edges": [], "limitations": [], "done": true
                }
            }],
            "usage": { "input_tokens": 15, "output_tokens": 22 }
        }),
        json!({
            "id": "m3", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "Orient complete on upgraded v1 session." }],
            "usage": { "input_tokens": 10, "output_tokens": 10 }
        }),
    ]);

    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(base_url), None)).unwrap();
    let (cfg_st, _, cfg_raw) = http_json(
        &srv,
        "PUT",
        "/api/settings/providers",
        Some(&json!({
            "default_provider": "anthropic", "default_model": "claude-haiku-4-5",
            "auto_approve_tools": true, "keys": { "anthropic": "fixture-key" }
        })),
    );
    assert_eq!(cfg_st, 200, "{cfg_raw}");

    let (st, val, raw) = http_json(
        &srv,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "orient" })),
    );
    assert_eq!(st, 202, "expected HTTP 202 on old investigation/1 session, got {st}: {raw}");
    assert_eq!(val["request_seq"].as_u64(), Some(expected_req_seq));

    let deadline = Instant::now() + Duration::from_secs(15);
    let final_sv = loop {
        let (gst, sv, _) = http_json(&srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(gst, 200);
        let fin = sv["events"].as_array().into_iter().flatten().any(|e| e["kind"] == MARKER_TASK_FINISHED);
        if sv["running"] == json!(false) && fin {
            break sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for V6 task completion: {sv}");
        std::thread::sleep(Duration::from_millis(25));
    };

    let evs = final_sv["events"].as_array().unwrap();
    assert!(evs.iter().any(|e| e["kind"] == EVENT_INVESTIGATION_SKELETON), "skeleton auto-built per §8.4");
    let resume_ev = evs.iter().find(|e| e["kind"] == "session.resume").expect("session.resume recorded");
    assert_eq!(resume_ev["payload"][TOOL_PROFILE_FIELD], PROFILE_INVESTIGATION_V2);
    let fin_ev = evs.iter().rev().find(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin_ev["payload"]["status"], "done", "{fin_ev}");
    assert_eq!(fin_ev["payload"]["counts"]["items"], 1);
    srv.shutdown();
}
