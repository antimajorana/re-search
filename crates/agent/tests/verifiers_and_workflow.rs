//! Tests-first suite for Stage 7 research guardrails (F11), stateless verifiers (F12),
//! strategist/narrator/sidetalk (F13), role presets & 0600 provider config, and
//! router-policy-aware runner/resume (`v0.2` and `v0.4-research`).

mod support;

use agent::guardrails::{
    build_research_system_prompt, check_computation_interpretation, check_final_answer_gate,
    mask_old_tool_outputs, mask_old_tool_outputs_stepped, run_research_loop,
    validate_pinned_conventions, GuardrailConfig,
};
use agent::narrator::narrate_session;
use agent::presets::{PresetKind, RoleAssignment};
use agent::provider_config::ProviderKeyStore;
use agent::sidetalk::{ask_sidetalk, read_sidetalk_thread};
use agent::strategist::{consult_strategist, MAX_STRATEGIST_CONSULTATIONS};
use agent::verifiers::{verify_full_solution, verify_step, VerificationVerdict};
use agent::{
    inspect_session_for_resume, resume_agent_loop, run_agent_loop, ContentBlock, Message,
    ModelRequest, ModelResponse, NoopSink, PreparedRequest, ProviderKind, ResumeConfig, Role,
    SessionOutcome, StopReason, Transport, TransportError, TransportInfo, DEFAULT_MAX_TOKENS,
};
use ledger::Ledger;
use research_tools::{research_policy, ResearchEnv, RESEARCH_POLICY_VERSION};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use support::{default_cfg, ok_msg, MockApprover, MockEpoch, MockServer, TempDir};
use tools::{builtin, CallCtx, Registry, Router, WorkerEpoch};

struct CapturingTransport {
    provider: &'static str,
    replies: Mutex<VecDeque<Result<ModelResponse, TransportError>>>,
    captured: Mutex<Vec<ModelRequest>>,
}

impl CapturingTransport {
    fn new(provider: &'static str, replies: Vec<ModelResponse>) -> Self {
        Self {
            provider,
            replies: Mutex::new(replies.into_iter().map(Ok).collect()),
            captured: Mutex::new(Vec::new()),
        }
    }

    fn text_reply(model: &str, text: &str) -> ModelResponse {
        ModelResponse {
            raw_body: serde_json::to_vec(&json!({
                "id": "m_cap", "role": "assistant", "model": model,
                "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }], "usage": {}
            }))
            .unwrap(),
            id: Some("m_cap".into()),
            model: model.into(),
            stop_reason: StopReason::EndTurn,
            content: vec![ContentBlock::Text { text: text.into() }],
            usage: json!({}),
        }
    }

    fn captured(&self) -> Vec<ModelRequest> {
        self.captured.lock().unwrap().clone()
    }
}

impl Transport for CapturingTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: self.provider,
            endpoint: "http://127.0.0.1:0/mock".into(),
            api_key_env: "RE_SEARCH_API_KEY".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.captured.lock().unwrap().push(req.clone());
        Ok(PreparedRequest { body: serde_json::to_vec(req).unwrap() })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        self.replies.lock().unwrap().pop_front().expect("CapturingTransport out of replies")
    }
}

fn make_research_router(dir: &std::path::Path, session: &str) -> Router {
    let mut reg = Registry::new();
    reg.register(builtin::ledger_note_def(), builtin::ledger_note_handler());
    let epoch = Arc::new(MockEpoch::new(1, false));
    let runs = Arc::new(AtomicUsize::new(0));
    reg.register_worker_session(
        builtin::sympy_eval_def(),
        epoch.clone(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let ep = epoch.live_epoch().ok_or("no epoch")?;
            ctx.record_worker_epoch(ep);
            Ok(json!({ "value": format!("ok:{}", args["code"].as_str().unwrap_or("")) }))
        }),
    );
    Router::new(
        reg,
        research_policy(),
        Ledger::open(dir).unwrap(),
        session,
        Arc::new(MockApprover::new(true)),
    )
}

#[test]
fn runner_and_resume_use_router_policy_version() {
    let tmp = TempDir::new("policy-v03");
    let session = "s-pol-v03";
    let router = make_research_router(&tmp.path, session);
    let ledger = Ledger::open(&tmp.path).unwrap();
    let s1 = MockServer::start(vec![ok_msg("m1", "end_turn", json!([{ "type": "text", "text": "leg1" }]))]);
    let t1 = support::make_transport(s1.base_url.clone(), "sk-pol");
    let mut sink1 = support::RecordingSink::new();
    let out1 = run_agent_loop(&t1, &router, &ledger, &default_cfg("solve", false), &mut sink1).unwrap();
    assert_eq!(out1, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
    assert_eq!(s1.requests_received().len(), 1);
    s1.finish();

    let evs = ledger.read_session(session).unwrap();
    assert_eq!(evs[0].payload["policy_version"], RESEARCH_POLICY_VERSION);

    let defs: Vec<_> = router.registry().defs().into_iter().cloned().collect();
    let insp = inspect_session_for_resume(&ledger, session, RESEARCH_POLICY_VERSION, &defs).unwrap();
    assert_eq!(insp.leg, 2);

    let s2 = MockServer::start(vec![ok_msg("m2", "end_turn", json!([{ "type": "text", "text": "leg2" }]))]);
    let t2 = support::make_transport(s2.base_url.clone(), "sk-pol-2");
    let rcfg = ResumeConfig {
        prompt: "next".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: None,
        max_turns: 4,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(),
        contained: false,
        reasoning_effort: None,
    };
    let mut sink2 = support::RecordingSink::new();
    let out2 = resume_agent_loop(&t2, &router, &ledger, &rcfg, &mut sink2).unwrap();
    assert_eq!(out2, SessionOutcome::Completed { turns: 1, stop_reason: "end_turn".into() });
    assert_eq!(s2.requests_received().len(), 1);
    s2.finish();
    support::assert_secret_absent(&tmp.path, session, &sink2, "sk-pol-2");

    let evs2 = ledger.read_session(session).unwrap();
    let res_ev = evs2.iter().find(|e| e.kind == "session.resume").unwrap();
    assert_eq!(res_ev.payload["policy_version"], RESEARCH_POLICY_VERSION);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn guardrails_masking_conventions_computation_and_final_answer_gates() {
    let def_g = GuardrailConfig::default();
    assert!(!def_g.verification_mode && !def_g.calculation_mode && def_g.pinned_conventions.is_none());
    assert!(validate_pinned_conventions(&"a".repeat(2000)).is_ok());
    assert!(validate_pinned_conventions(&"a".repeat(2001)).is_err());

    let sys = build_research_system_prompt(
        "Base.",
        &GuardrailConfig {
            verification_mode: true,
            calculation_mode: true,
            pinned_conventions: Some("Use mostly-plus metric (-,+,+,+).".into()),
            step_mask_keep: 6,
            step_mask_stride: 8,
        },
    )
    .unwrap();
    assert!(sys.contains("mostly-plus metric"));
    assert!(sys.contains("<pinned_conventions>"));

    let mut dyn_sys = "Base prompt.".to_string();
    agent::upsert_pinned_conventions_in_prompt(&mut dyn_sys, Some("Metric (-,+,+,+)")).unwrap();
    assert!(dyn_sys.contains("<pinned_conventions>\nMetric (-,+,+,+)\n</pinned_conventions>"), "{dyn_sys}");
    agent::upsert_pinned_conventions_in_prompt(&mut dyn_sys, Some("Natural units")).unwrap();
    assert!(!dyn_sys.contains("Metric (-,+,+,+)") && dyn_sys.contains("<pinned_conventions>\nNatural units\n</pinned_conventions>"), "{dyn_sys}");
    assert!(agent::upsert_pinned_conventions_in_prompt(&mut dyn_sys, Some(&"x".repeat(2001))).is_err());
    agent::upsert_pinned_conventions_in_prompt(&mut dyn_sys, Some("   ")).unwrap();
    assert_eq!(dyn_sys, "Base prompt.");

    // Stepped tool-output masking keeps the newest `keep_recent` intact and masks older ones.
    let mut msgs = Vec::new();
    for i in 0..5 {
        msgs.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse { id: format!("t{i}"), name: "fs_read".into(), input: json!({}) }],
        });
        msgs.push(Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: format!("t{i}"),
                is_error: false,
                content: format!("payload-{i}:{}", "z".repeat(300)),
            }],
        });
    }
    let masked = mask_old_tool_outputs(&msgs, 2);
    for (idx, m) in masked.iter().filter(|m| m.role == Role::User).enumerate() {
        let ContentBlock::ToolResult { content, .. } = &m.content[0] else { panic!() };
        if idx < 3 {
            assert!(content.contains("masked") && content.len() < 120, "idx={idx}: {content}");
        } else {
            assert!(content.starts_with(&format!("payload-{idx}:")), "idx={idx} should be intact");
        }
    }
    let stepped = mask_old_tool_outputs_stepped(&msgs, 2, 2);
    let masked_count = stepped
        .iter()
        .filter(|m| m.role == Role::User)
        .filter(|m| matches!(&m.content[0], ContentBlock::ToolResult { content, .. } if content.contains("masked")))
        .count();
    assert_eq!(masked_count, 2);

    // Computation interpretation & full-verification gates.
    let tmp = TempDir::new("gates");
    let session = "s-gates";
    let ledger = Ledger::open(&tmp.path).unwrap();
    ledger.append_with_hash(session, "session.start", 1, &json!({ "route": "api", "prompt": "P" })).unwrap();
    assert!(check_final_answer_gate(&ledger, session, false).is_ok());
    assert!(check_final_answer_gate(&ledger, session, true).is_err());

    let sym_p = json!({"entry_id":"e-1","entry_type":"symbolic_computation","summary":"eval","details":"x^2","depends_on":[]});
    ledger.append_with_hash(session, "proof.entry", 1, &sym_p).unwrap();
    assert!(check_computation_interpretation(&ledger, session).is_err());

    let res_p = json!({"entry_id":"e-2","entry_type":"result","summary":"res","details":"x^2 >= 0","depends_on":["e-1"]});
    ledger.append_with_hash(session, "proof.entry", 1, &res_p).unwrap();
    assert!(check_computation_interpretation(&ledger, session).is_ok());

    ledger.append_with_hash(session, "proof.verification_attempt", 1, &json!({ "target_entry_id": "e-2", "kind": "full", "phase": "started" })).unwrap();
    assert!(check_final_answer_gate(&ledger, session, true).is_err());

    let b = ledger.put_blob(br#"{"verdict":"accepted"}"#).unwrap();
    let ver_p = json!({"target_entry_id":"e-2","kind":"full","verdict":"accepted","summary":"ok","issues":[],"raw_blob":b});
    ledger.append_with_hash(session, "proof.verification", 1, &ver_p).unwrap();
    assert!(check_final_answer_gate(&ledger, session, true).is_ok());
    ledger.append(session, "proof.verification_attempt", 1, &json!({ "target_entry_id": "e-1", "kind": "step" })).unwrap();
    assert!(research_tools::read_proof_dag(&ledger, session).unwrap().latest_full_verification_accepted);
    let mut conflicting = ver_p.clone();
    conflicting["issues"] = json!([{ "severity": "error", "message": "Missing hypothesis." }]);
    ledger.append(session, "proof.verification", 1, &conflicting).unwrap();
    assert!(check_final_answer_gate(&ledger, session, true).is_err());
    ledger.append(session, "proof.verification", 1, &ver_p).unwrap();
    ledger.append(session, "session.resume", 1, &json!({ "prompt": "New problem" })).unwrap();
    assert!(check_final_answer_gate(&ledger, session, true).is_err());
    ledger.append(session, "proof.entry", 1, &json!({
        "entry_id": "e-3", "entry_type": "final_answer", "summary": "New answer", "details": "New answer",
    })).unwrap();
    ledger.append(session, "proof.verification", 1, &ver_p).unwrap();
    assert!(check_final_answer_gate(&ledger, session, true).is_err());
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);

    // run_research_loop blocks EndTurn without accepted full verification when verification_mode=true.
    let tmp_loop = TempDir::new("gate-loop");
    let s_loop = "s-gate-loop";
    let r_loop = make_research_router(&tmp_loop.path, s_loop);
    let l_loop = Ledger::open(&tmp_loop.path).unwrap();
    let t_loop = CapturingTransport::new(
        "anthropic-messages",
        vec![CapturingTransport::text_reply("claude-haiku-4-5", "Unverified final answer")],
    );
    let out = run_research_loop(
        &t_loop,
        &r_loop,
        &l_loop,
        &default_cfg("Prove X", false),
        &GuardrailConfig { verification_mode: true, ..Default::default() },
        &mut NoopSink,
    )
    .unwrap();
    assert!(matches!(out, SessionOutcome::Aborted { .. }), "expected Aborted, got {out:?}");
    let events = l_loop.read_session(s_loop).unwrap();
    assert_eq!(events.last().unwrap().kind, "session.abort");
    assert!(!events.iter().any(|event| event.kind == "session.end"));
    assert_eq!(l_loop.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn verifiers_reject_unstructured_and_incomplete_verdicts() {
    let tmp = TempDir::new("incomplete-verdicts");
    let session = "s-ver-incomplete";
    let ledger = Ledger::open(&tmp.path).unwrap();
    let env = ResearchEnv::new(&tmp.path);
    ledger.append_with_hash(session, "session.start", 1, &json!({ "prompt": "What is 2+2?" })).unwrap();
    ledger.append_with_hash(session, "proof.entry", 1, &json!({
        "entry_id": "e-1", "entry_type": "final_answer", "summary": "2+2=4", "details": "Two plus two is four."
    })).unwrap();

    for (reason, reply, expected) in [
        (StopReason::EndTurn, "Rejected. This should never be called \"accepted\".", VerificationVerdict::Rejected),
        (StopReason::MaxTokens, r#"{"verdict":"accepted","summary":"Start"#, VerificationVerdict::Rejected),
        (StopReason::MaxTokens, r#"{"verdict":"accepted","summary":"Sound."}"#, VerificationVerdict::Rejected),
        (StopReason::EndTurn, r#"{"verdict":"accepted","issues":[{}]}"#, VerificationVerdict::Rejected),
        (StopReason::EndTurn, r#"{"verdict":"accepted","counterexample":"","issues":["A factor is missing."]}"#, VerificationVerdict::Rejected),
        (StopReason::EndTurn, r#"{"verdict":"accepted","issues":[{"severity":"BLOCKER","message":"Unsupported limit."}]}"#, VerificationVerdict::Rejected),
        (StopReason::EndTurn, r#"{"verdict":"accepted","counterexample":"n=0"}"#, VerificationVerdict::Rejected),
        (StopReason::EndTurn, r#"{"verdict":"accepted","issues":[{"severity":"warning","message":"Clarify notation."}]}"#, VerificationVerdict::Accepted),
        (StopReason::EndTurn, "```json\n{\"verdict\":\"accepted\",\"summary\":\"Sound.\"}", VerificationVerdict::Rejected),
        (StopReason::EndTurn, "```json\n{\"verdict\":\"accepted\",\"summary\":\"Sound.\"}\n```", VerificationVerdict::Accepted),
    ] {
        let mut response = CapturingTransport::text_reply("claude-haiku-4-5", reply);
        response.stop_reason = reason;
        let transport = CapturingTransport::new("anthropic-messages", vec![response]);
        let report = verify_full_solution(&transport, &ledger, &env, session, "claude-haiku-4-5").unwrap();
        assert_eq!(report.verdict, expected, "{reply}");
        let recorded = ledger.read_session(session).unwrap().pop().unwrap();
        assert_eq!(recorded.kind, "proof.verification");
        assert_eq!(recorded.payload["verdict"], expected.as_str());
    }
}

#[test]
fn verifiers_isolate_ancestors_emit_attempts_and_force_reject_on_tex_audit_blockers() {
    let tmp = TempDir::new("verifiers");
    let session = "s-ver-1";
    let ledger = Ledger::open(&tmp.path).unwrap();
    let env = ResearchEnv::new(&tmp.path);

    ledger
        .append_with_hash(
            session,
            "session.start",
            1,
            &json!({ "route": "api", "prompt": "Prove Z", "pinned_conventions": "Use mostly-plus metric." }),
        )
        .unwrap();
    // Main solver bias text that MUST NOT appear in stateless verifier prompt.
    ledger.append_with_hash(session, "model.text", 1, &json!({ "text": "SOLVER_PRIVATE_BIAS_TOKEN" })).unwrap();
    ledger.append_with_hash(session, "session.fork", 1, &json!({
        "modified_assumption": "Take k < 0 for this branch."
    })).unwrap();
    for (call_id, code) in [
        ("linked-lean", "theorem linked_check : True := trivial"),
        ("unrelated-lean", "theorem unrelated_check : True := trivial"),
    ] {
        ledger.append_with_hash(session, "tool.request", 1, &json!({
            "call_id": call_id, "tool": "lean_check", "args": { "inline": json!({ "code": code }).to_string() },
        })).unwrap();
        ledger.append_with_hash(session, "tool.result", 1, &json!({
            "call_id": call_id, "ok": true, "result": {
                "inline": json!({
                    "status": "verified", "ok": true, "axioms": [],
                    "source_sha256": ledger::sha256_hex(code.as_bytes()),
                }).to_string(),
            },
        })).unwrap();
    }
    for (call_id, point) in [("linked-cas", "x = -1"), ("unrelated-cas", "UNRELATED_PROBE")] {
        ledger.append_with_hash(session, "tool.request", 1, &json!({
            "call_id": call_id, "tool": "sympy_eval",
        })).unwrap();
        ledger.append_with_hash(session, "tool.result", 1, &json!({
            "call_id": call_id, "ok": true, "result": {
                "value": "report",
                "validation": {
                    "schema_version": 1, "kind": "symbolic_identity", "status": "counterexample",
                    "samples": [{"status": "counterexample", "point": point, "lhs": "1", "rhs": "-1"}],
                },
            },
        })).unwrap();
    }
    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-1", "entry_type": "assumption",
                "summary": "Assume A", "details": "A holds", "latex": "A = 1", "depends_on": [],
                "tool_call_ref": "linked-lean"
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-unrelated", "entry_type": "derivation",
                "summary": "Unrelated U", "details": "UNRELATED_BRANCH_TOKEN", "depends_on": [],
                "tool_call_ref": "unrelated-lean"
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-2", "entry_type": "derivation",
                "summary": "From A get B", "details": "B follows from A", "latex": "B = A", "depends_on": ["e-1"],
                "tool_call_ref": "linked-cas"
            }),
        )
        .unwrap();

    let accepted_json = json!({
        "verdict": "accepted", "summary": "Step is sound.", "counterexample": null, "issues": []
    })
    .to_string();
    let tr = CapturingTransport::new(
        "anthropic-messages",
        vec![CapturingTransport::text_reply("claude-haiku-4-5", &accepted_json)],
    );
    let vres = verify_step(&tr, &ledger, &env, session, "e-2", "claude-haiku-4-5").unwrap();
    assert_eq!(vres.verdict, VerificationVerdict::Accepted);

    let reqs = tr.captured();
    assert_eq!(reqs.len(), 1);
    let user_txt = match &reqs[0].messages[0].content[0] {
        ContentBlock::Text { text } => text.clone(),
        _ => panic!(),
    };
    assert!(user_txt.contains("Prove Z") && user_txt.contains("e-1") && user_txt.contains("e-2"));
    assert!(user_txt.contains("<deterministic_tex_audit"));
    assert!(user_txt.contains("<pinned_conventions>\nUse mostly-plus metric.\n</pinned_conventions>"));
    assert!(user_txt.contains("<fork_assumptions>\nTake k &lt; 0 for this branch.\n</fork_assumptions>"));
    assert!(user_txt.contains("<lean_checks>"));
    assert!(user_txt.contains("theorem linked_check : True := trivial"));
    assert!(!user_txt.contains("theorem unrelated_check"));
    assert!(user_txt.contains("<computational_checks>"));
    assert!(user_txt.contains("linked-cas") && user_txt.contains("x = -1"));
    assert!(!user_txt.contains("UNRELATED_PROBE"));
    assert!(reqs[0].system.as_deref().unwrap().contains("<pinned_conventions>"));
    assert!(!user_txt.contains("SOLVER_PRIVATE_BIAS_TOKEN"));
    assert!(!user_txt.contains("UNRELATED_BRANCH_TOKEN"));

    // Now test blocker override: withdrawn arXiv paper in ledger + final_answer citing it.
    let w_blob = ledger
        .put_blob(br#"{"arxiv_id":"2311.00001v2","withdrawn":true}"#)
        .unwrap();
    ledger
        .append_with_hash(session, "literature.fetched", 1, &json!({ "raw_blob": w_blob }))
        .unwrap();
    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-3", "entry_type": "final_answer",
                "summary": "Cite withdrawn", "details": "Uses arXiv:2311.00001v2",
                "latex": "x = 1", "depends_on": ["e-2"]
            }),
        )
        .unwrap();
    // Superseded entry must be slimmed out of full verification prompt.
    ledger
        .append_with_hash(
            session,
            "proof.supersede",
            1,
            &json!({ "entry_id": "e-unrelated", "replaced_by": "e-2", "reason": "unused" }),
        )
        .unwrap();

    ledger
        .append_with_hash(
            session,
            "session.resume",
            1,
            &json!({ "prompt": "Check this citation", "pinned_conventions": "Use mostly-minus metric and q < 0." }),
        )
        .unwrap();
    let tr_full = CapturingTransport::new(
        "anthropic-messages",
        vec![
            CapturingTransport::text_reply("claude-haiku-4-5", &accepted_json),
            CapturingTransport::text_reply("claude-haiku-4-5", &accepted_json),
        ],
    );
    let fres = verify_full_solution(&tr_full, &ledger, &env, session, "claude-haiku-4-5").unwrap();
    assert_eq!(fres.verdict, VerificationVerdict::Rejected, "withdrawn arXiv blocker must force Rejected");
    assert!(fres.tex_audit.blocker_count > 0);
    let full_user_txt = match &tr_full.captured()[0].messages[0].content[0] {
        ContentBlock::Text { text } => text.clone(),
        _ => panic!(),
    };
    assert!(!full_user_txt.contains("UNRELATED_BRANCH_TOKEN"));
    assert!(!full_user_txt.contains("mostly-plus"));
    assert!(full_user_txt.contains("<pinned_conventions>\nUse mostly-minus metric and q &lt; 0.\n</pinned_conventions>"));
    assert!(tr_full.captured()[0].system.as_deref().unwrap().contains("<pinned_conventions>"));

    ledger
        .append_with_hash(
            session,
            "session.resume",
            1,
            &json!({ "prompt": "Check again", "pinned_conventions": "" }),
        )
        .unwrap();
    verify_full_solution(&tr_full, &ledger, &env, session, "claude-haiku-4-5").unwrap();
    let reqs = tr_full.captured();
    let ContentBlock::Text { text } = &reqs[1].messages[0].content[0] else { panic!() };
    assert!(!text.contains("<pinned_conventions>"));
    assert!(!text.contains("mostly-minus"));

    let evs = ledger.read_session(session).unwrap();
    assert!(evs.iter().any(|e| e.kind == "proof.verification_attempt" && e.payload["phase"] == "started"));
    assert!(evs.iter().any(|e| e.kind == "verifier.request" && e.payload["raw_blob"].is_string()));
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn strategist_cap_narrator_and_isolated_sidetalk() {
    let tmp = TempDir::new("f13-roles");
    let session = "s-f13-1";
    let ledger = Ledger::open(&tmp.path).unwrap();
    let env = ResearchEnv::new(&tmp.path);

    ledger.append_with_hash(session, "session.start", 1, &json!({ "route": "api", "prompt": "Compute H" })).unwrap();
    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-1", "entry_type": "assumption",
                "summary": "H = p^2", "details": "Free particle", "latex": "H = p^2", "depends_on": []
            }),
        )
        .unwrap();
    let base_head_before = ledger.verify_session(session).unwrap().head_hash;
    let base_count_before = ledger.read_session(session).unwrap().len();

    // Side-talk validates session ID, never mutates base_session, and writes under sidetalk:<base>.
    assert!(ask_sidetalk(
        &CapturingTransport::new("anthropic-messages", vec![]),
        &ledger,
        &env,
        "../bad",
        "hi",
        "claude-haiku-4-5"
    )
    .is_err());

    let st_tr = CapturingTransport::new(
        "anthropic-messages",
        vec![CapturingTransport::text_reply("claude-haiku-4-5", "See step e-1 for H = p^2.")],
    );
    let turn = ask_sidetalk(&st_tr, &ledger, &env, session, "Explain e-1?", "claude-haiku-4-5").unwrap();
    assert!(turn.answer.contains("e-1"));
    assert!(turn.references.contains(&"e-1".to_string()));
    assert_eq!(read_sidetalk_thread(&ledger, session).unwrap().len(), 1);
    assert_eq!(ledger.verify_session(session).unwrap().head_hash, base_head_before);
    assert_eq!(ledger.read_session(session).unwrap().len(), base_count_before);
    assert!(!tmp.path.join("sessions").join(session).join("sidetalk.lock").exists());

    // Narrator generates read-only walkthrough & Derivation Sheet.
    let nar_tr = CapturingTransport::new(
        "anthropic-messages",
        vec![CapturingTransport::text_reply("claude-haiku-4-5", "## Walkthrough\nStep e-1 assumes H = p^2.")],
    );
    let nar = narrate_session(&nar_tr, &ledger, session, "claude-haiku-4-5").unwrap();
    assert!(nar.walkthrough.contains("Walkthrough"));
    assert!(nar.derivation_sheet.contains("e-1"));

    // Strategist enforces <= 20 consultations per session.
    let replies: Vec<_> = (0..MAX_STRATEGIST_CONSULTATIONS)
        .map(|i| CapturingTransport::text_reply("claude-haiku-4-5", &format!("Try approach {i}")))
        .collect();
    let strat_tr = CapturingTransport::new("anthropic-messages", replies);
    for i in 0..MAX_STRATEGIST_CONSULTATIONS {
        let adv = consult_strategist(&strat_tr, &ledger, session, &format!("Q{i}"), "claude-haiku-4-5").unwrap();
        assert_eq!(adv.status, "unverified_advice");
    }
    let capped = consult_strategist(&strat_tr, &ledger, session, "Q21", "claude-haiku-4-5");
    assert!(capped.is_err(), "21st strategist call must fail closed");

    // Mandatory strategist lock after rejected verification clears on consultation or 3 refusals.
    let rb = ledger.put_blob(br#"{"verdict":"rejected"}"#).unwrap();
    let rej_ev = json!({"target_entry_id":"e-1","kind":"step","verdict":"rejected","summary":"bad","issues":[],"raw_blob":rb});
    ledger.append_with_hash(session, "proof.verification", 1, &rej_ev).unwrap();
    assert!(agent::mandatory_strategist_required(&ledger, session).unwrap());
    for _ in 0..3 {
        ledger.append_with_hash(session, "strategist.refusal", 1, &json!({})).unwrap();
    }
    assert!(!agent::mandatory_strategist_required(&ledger, session).unwrap());
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn presets_and_provider_key_store_enforce_0600_and_redaction() {
    let ant = RoleAssignment::for_provider(ProviderKind::Anthropic);
    let oai = RoleAssignment::for_provider(ProviderKind::OpenAi);
    let oll = RoleAssignment::for_provider(ProviderKind::Ollama);
    let gem = RoleAssignment::for_provider(ProviderKind::Gemini);
    assert_eq!(ant.solver.provider, ProviderKind::Anthropic);
    assert_eq!(oai.step_verifier.provider, ProviderKind::OpenAi);
    assert_eq!(oll.narrator.provider, ProviderKind::Ollama);
    assert_eq!(gem.sidetalk.provider, ProviderKind::Gemini);
    assert_eq!(RoleAssignment::from_preset(PresetKind::Balanced).solver.provider, ProviderKind::Anthropic);

    let tmp = TempDir::new("key-store");
    let mut store = ProviderKeyStore::default();
    let secret = "sk-provider-store-super-secret-9876";
    store.set_key(ProviderKind::Anthropic, Some(secret.into())).unwrap();
    store.set_base_url(ProviderKind::Ollama, Some("http://127.0.0.1:11434/v1".into())).unwrap();
    store.save_to_data_dir(&tmp.path).unwrap();

    let cfg_path = tmp.path.join("providers.json");
    assert!(cfg_path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&cfg_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "providers.json must be 0600");
    }

    let loaded = ProviderKeyStore::load_from_data_dir(&tmp.path).unwrap();
    let debug_dump = format!("{loaded:?}");
    assert!(!debug_dump.contains(secret));
    let ui_view = serde_json::to_string(&loaded.redacted_view()).unwrap();
    assert!(!ui_view.contains(secret));
    assert!(loaded.redacted_view().providers["anthropic"].configured);

    // Ensure v0 router compatibility and legacy-v0 fail-closed verification on research loop.
    let tmp_v0 = TempDir::new("v0-compat");
    let approver = Arc::new(MockApprover::new(true));
    let epoch = Arc::new(MockEpoch::new(1, false));
    epoch.set_epoch(2);
    let runs = Arc::new(AtomicUsize::new(0));
    let r_v0 = support::make_router(&tmp_v0.path, "s-v0-compat", approver.clone(), epoch, runs);
    assert_eq!(approver.asked(), 0);
    assert_eq!(r_v0.policy().version, builtin::POLICY_VERSION);

    let tmp_leg = TempDir::new("legacy-v0-guard");
    std::fs::write(tmp_leg.path.join("ledger.db"), support::legacy_v0_db_bytes()).unwrap();
    let leg_ledger = Ledger::open_existing(&tmp_leg.path).unwrap();
    assert!(check_final_answer_gate(&leg_ledger, "s0", true).is_err());
}
