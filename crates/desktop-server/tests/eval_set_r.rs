//! Evaluation plan Set R (00-overview.md 7.2, 7.3; package F1):
//!
//! Hand-builds ledger fixtures for all four existing session types with the real writer code
//! (`Ledger`, `ingest_bytes`, `Router::try_call`, `export::fork::research_from_paper`) and
//! asserts that every one resumes cleanly under the new tool-profile resume path, records
//! `tool_profile` on `session.resume`, passes a follow-up leg-3 resume, preserves prior content,
//! and passes `Ledger::verify_all()` with zero unreferenced blobs.

mod eval_set_r_support;

use agent::{
    inspect_session_for_resume, inspect_session_with_profile, resume_autonomous_research_loop,
    run_autonomous_research_loop, AgentConfig, AutonomousConfig, AutonomousRoles, NoopSink,
    ResumeRejectKind, SessionOutcome, DEFAULT_MAX_TOKENS,
};
use eval_set_r_support::{
    build_current_investigation_session, build_profile_router, defs_json, ingest_synthetic_paper,
    resume_cfg, write_pref1_completed_leg1, ScriptedAnthropic,
};
use ledger::Ledger;
use research_tools::investigation::{
    GOLDEN_BRANCH_7C36A0E, GOLDEN_CONVENTIONS_7C36A0E, GOLDEN_MAP_7C36A0E,
    GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E, GOLDEN_SUMMARY_7C36A0E, RECORD_PAPER_BRANCH,
    RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP, RECORD_PAPER_SUMMARY,
};
use research_tools::tool_profile::{
    investigation_v1, research_v1, LEGACY_MAY_GAIN, MARKER_INVESTIGATION_HANDOFF,
    MARKER_INVESTIGATION_STARTED, PROFILE_INVESTIGATION_V1, PROFILE_RESEARCH_V1,
    TOOL_PROFILE_FIELD,
};
use research_tools::{read_proof_dag, research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::sync::Arc;
use tools::{CallOutcome, ToolDef};

#[test]
fn set_r_1_research_session_resumes_under_research_v1() {
    let dir = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(dir.path()));
    let ledger = Ledger::open(dir.path()).unwrap();
    let sid = "s-setr-research";

    // (a) Full pre-F1 research session (no tool_profile) with a real Proof DAG entry.
    let router = build_profile_router(dir.path(), Arc::clone(&env), sid, PROFILE_RESEARCH_V1);
    write_pref1_completed_leg1(
        &ledger,
        sid,
        "Compute the harmonic oscillator period.",
        "The period is 2*pi*sqrt(m/k).",
        defs_json(&research_v1().defs),
    );
    let call = router
        .try_call(
            &format!("agent:anthropic-messages:{sid}"),
            "ledger_append",
            &json!({
                "entry_type": "assumption",
                "summary": "Quadratic potential V(x) = 1/2 k x^2",
                "details": "Standard Hooke's law potential with m > 0, k > 0.",
                "depends_on": []
            }),
        )
        .unwrap();
    assert!(matches!(call, CallOutcome::Ok(_)), "{call:?}");
    let dag_before = read_proof_dag(&ledger, sid).unwrap();
    assert_eq!(dag_before.entries.len(), 1);
    let dag_before_json = serde_json::to_value(&dag_before.entries).unwrap();

    let (insp, resolved) = inspect_session_with_profile(&ledger, sid).unwrap();
    assert_eq!(resolved.profile.name, PROFILE_RESEARCH_V1);
    assert!(!resolved.explicit);
    assert_eq!(insp.tool_profile, PROFILE_RESEARCH_V1);
    inspect_session_for_resume(&ledger, sid, &research_policy().version, &research_v1().defs).unwrap();

    // Resuming a Research session with an investigation/1 router must fail closed (Rule R2).
    let wrong_router = build_profile_router(dir.path(), Arc::clone(&env), sid, PROFILE_INVESTIGATION_V1);
    let err = resume_autonomous_research_loop(
        &ScriptedAnthropic::new(&[]),
        &AutonomousRoles::default(),
        &wrong_router,
        &ledger,
        &env,
        &resume_cfg("Should fail"),
        &AutonomousConfig::default(),
        None,
        &mut NoopSink,
    )
    .unwrap_err();
    assert!(err.contains("active router profile"), "{err}");

    // Resume leg 2 and leg 3 under research/1.
    let t = ScriptedAnthropic::new(&["Checked scaling.", "Confirmed leg 3."]);
    for prompt in ["Check scaling with m.", "Confirm units."] {
        let out = resume_autonomous_research_loop(
            &t,
            &AutonomousRoles::default(),
            &router,
            &ledger,
            &env,
            &resume_cfg(prompt),
            &AutonomousConfig::default(),
            None,
            &mut NoopSink,
        )
        .unwrap();
        assert!(matches!(out, SessionOutcome::Completed { .. }), "{out:?}");
    }
    let events = ledger.read_session(sid).unwrap();
    let resumes: Vec<_> = events.iter().filter(|e| e.kind == "session.resume").collect();
    assert_eq!(resumes.len(), 2);
    assert!(resumes.iter().all(|e| e.payload[TOOL_PROFILE_FIELD] == PROFILE_RESEARCH_V1));
    assert_eq!(serde_json::to_value(&read_proof_dag(&ledger, sid).unwrap().entries).unwrap(), dag_before_json);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);

    // (b) Earlier pre-F1 Research session missing the 7 tolerated ReadWorkspace additions.
    let sid_old = "s-setr-research-old";
    let old_defs: Vec<ToolDef> = research_v1()
        .defs
        .into_iter()
        .filter(|d| !LEGACY_MAY_GAIN.contains(&d.id.as_str()))
        .collect();
    write_pref1_completed_leg1(&ledger, sid_old, "Older research session.", "Done.", defs_json(&old_defs));
    let (insp_old, res_old) = inspect_session_with_profile(&ledger, sid_old).unwrap();
    assert_eq!(res_old.profile.name, PROFILE_RESEARCH_V1);
    assert_eq!(insp_old.tool_profile, PROFILE_RESEARCH_V1);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn set_r_2_investigation_before_pointer_citations_resumes_under_investigation_v1() {
    let dir = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(dir.path()));
    let ledger = Ledger::open(dir.path()).unwrap();

    for (sid, map_hash) in [
        ("s-setr-inv-7c36a0e", GOLDEN_MAP_7C36A0E),
        ("s-setr-inv-7c36a0e-noindex", GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E),
    ] {
        ledger
            .append_with_hash(sid, MARKER_INVESTIGATION_STARTED, 1, &json!({ "title": "Pre-pointer paper" }))
            .unwrap();
        let (source_id, primary_path) = ingest_synthetic_paper(dir.path(), &env, sid);
        ledger
            .append_with_hash(
                sid,
                "investigation.paper",
                1,
                &json!({ "source_id": source_id, "primary_path": primary_path }),
            )
            .unwrap();

        // Build exact 7c36a0e tool_defs: pre-pointer recorder hashes and without `read_paper_lines`.
        let pre_pointer_defs: Vec<Value> = investigation_v1()
            .defs
            .iter()
            .filter(|d| d.id != "read_paper_lines")
            .map(|d| {
                let hash = match d.id.as_str() {
                    RECORD_PAPER_MAP => map_hash.to_string(),
                    RECORD_PAPER_SUMMARY => GOLDEN_SUMMARY_7C36A0E.to_string(),
                    RECORD_PAPER_BRANCH => GOLDEN_BRANCH_7C36A0E.to_string(),
                    RECORD_PAPER_CONVENTIONS => GOLDEN_CONVENTIONS_7C36A0E.to_string(),
                    _ => d.def_hash(),
                };
                json!({
                    "id": d.id,
                    "effect": d.effect.as_str(),
                    "approval_scope": d.approval_scope.as_str(),
                    "def_hash": hash,
                })
            })
            .collect();
        write_pref1_completed_leg1(
            &ledger,
            sid,
            "Map this paper.",
            "Saved pre-pointer map.",
            pre_pointer_defs,
        );
        let legacy_map_payload = json!({
            "source_id": source_id,
            "title": "Harmonic Scaling Model",
            "coverage": "paper",
            "limitations": [],
            "nodes": [{
                "id": "t1",
                "sentence": "Angular frequency is sqrt(k/m).",
                "kind": "result",
                "main": true,
                "anchors": [{
                    "source_id": source_id,
                    "path": primary_path,
                    "start_line": 4,
                    "end_line": 4,
                    "excerpt": "Theorem T1: the angular frequency is $\\omega = \\sqrt{k / m}$ and is independent of amplitude.",
                    "kind": "passage"
                }]
            }],
            "edges": []
        });
        ledger.append_with_hash(sid, "investigation.map", 1, &legacy_map_payload).unwrap();

        let (insp, resolved) = inspect_session_with_profile(&ledger, sid).unwrap();
        assert_eq!(resolved.profile.name, PROFILE_INVESTIGATION_V1);
        assert!(!resolved.explicit);
        assert_eq!(insp.tool_profile, PROFILE_INVESTIGATION_V1);

        let router = build_profile_router(dir.path(), Arc::clone(&env), sid, PROFILE_INVESTIGATION_V1);
        let t = ScriptedAnthropic::new(&["Leg 2 answer.", "Leg 3 answer."]);
        for prompt in ["Explain Theorem T1.", "Follow up on proof."] {
            let out = resume_autonomous_research_loop(
                &t,
                &AutonomousRoles::default(),
                &router,
                &ledger,
                &env,
                &resume_cfg(prompt),
                &AutonomousConfig::default(),
                None,
                &mut NoopSink,
            )
            .unwrap();
            assert!(matches!(out, SessionOutcome::Completed { .. }), "{out:?}");
        }
        let events = ledger.read_session(sid).unwrap();
        let resumes: Vec<_> = events.iter().filter(|e| e.kind == "session.resume").collect();
        assert_eq!(resumes.len(), 2);
        assert!(resumes.iter().all(|e| e.payload[TOOL_PROFILE_FIELD] == PROFILE_INVESTIGATION_V1));
        assert_eq!(events.iter().find(|e| e.kind == "investigation.map").unwrap().payload, legacy_map_payload);
    }
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn set_r_3_investigation_current_resumes_under_investigation_v1() {
    let dir = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(dir.path()));
    let sid = "s-setr-inv-current";
    build_current_investigation_session(dir.path(), &env, sid);

    let ledger = Ledger::open(dir.path()).unwrap();
    let map_before = ledger
        .read_session(sid)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "investigation.map")
        .unwrap()
        .payload;

    let (insp, resolved) = inspect_session_with_profile(&ledger, sid).unwrap();
    assert_eq!(resolved.profile.name, PROFILE_INVESTIGATION_V1);
    assert!(!resolved.explicit);
    assert_eq!(insp.tool_profile, PROFILE_INVESTIGATION_V1);

    let router = build_profile_router(dir.path(), Arc::clone(&env), sid, PROFILE_INVESTIGATION_V1);
    let t = ScriptedAnthropic::new(&["Pointer-checked reply.", "Leg 3 reply."]);
    for prompt in ["Verify T1 dependence on A1.", "Summarize assumptions."] {
        let out = resume_autonomous_research_loop(
            &t,
            &AutonomousRoles::default(),
            &router,
            &ledger,
            &env,
            &resume_cfg(prompt),
            &AutonomousConfig::default(),
            None,
            &mut NoopSink,
        )
        .unwrap();
        assert!(matches!(out, SessionOutcome::Completed { .. }), "{out:?}");
    }
    let events = ledger.read_session(sid).unwrap();
    let resumes: Vec<_> = events.iter().filter(|e| e.kind == "session.resume").collect();
    assert_eq!(resumes.len(), 2);
    assert!(resumes.iter().all(|e| e.payload[TOOL_PROFILE_FIELD] == PROFILE_INVESTIGATION_V1));
    assert_eq!(events.iter().find(|e| e.kind == "investigation.map").unwrap().payload, map_before);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn set_r_4_handoff_child_resumes_under_research_v1() {
    let dir = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(dir.path()));
    let parent = "s-setr-parent-inv";
    let (source_id, primary_path, map_seq, map_hash) =
        build_current_investigation_session(dir.path(), &env, parent);

    let handoff_ctx = json!({
        "parent_session": parent,
        "map_seq": map_seq,
        "map_hash": map_hash,
        "source_id": source_id,
        "primary_source_id": source_id,
        "question": "Extend Theorem T1 to a quartic anharmonic perturbation.",
        "sources": [{
            "source_id": source_id,
            "path": primary_path,
            "start_line": 4,
            "end_line": 4,
            "kind": "passage"
        }],
    });
    let child = export::fork::research_from_paper(dir.path(), parent, &handoff_ctx).unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let initial_child_events = ledger.read_session(&child).unwrap();
    assert!(initial_child_events.iter().any(|e| e.kind == "source.ingested"));
    assert!(initial_child_events.iter().any(|e| e.kind == MARKER_INVESTIGATION_HANDOFF));

    write_pref1_completed_leg1(
        &ledger,
        &child,
        "Extend Theorem T1 to quartic perturbation.",
        "First-order frequency shift scales with amplitude squared.",
        defs_json(&research_v1().defs),
    );

    let (insp, resolved) = inspect_session_with_profile(&ledger, &child).unwrap();
    assert_eq!(resolved.profile.name, PROFILE_RESEARCH_V1);
    assert!(!resolved.explicit);
    assert_eq!(insp.tool_profile, PROFILE_RESEARCH_V1);

    let inv_router = build_profile_router(dir.path(), Arc::clone(&env), &child, PROFILE_INVESTIGATION_V1);
    assert!(resume_autonomous_research_loop(
        &ScriptedAnthropic::new(&[]),
        &AutonomousRoles::default(),
        &inv_router,
        &ledger,
        &env,
        &resume_cfg("Wrong profile"),
        &AutonomousConfig::default(),
        None,
        &mut NoopSink,
    )
    .is_err());

    let res_router = build_profile_router(dir.path(), Arc::clone(&env), &child, PROFILE_RESEARCH_V1);
    let t = ScriptedAnthropic::new(&["Computed shift.", "Verified order."]);
    for prompt in ["Compute coefficient.", "Check perturbation order."] {
        let out = resume_autonomous_research_loop(
            &t,
            &AutonomousRoles::default(),
            &res_router,
            &ledger,
            &env,
            &resume_cfg(prompt),
            &AutonomousConfig::default(),
            None,
            &mut NoopSink,
        )
        .unwrap();
        assert!(matches!(out, SessionOutcome::Completed { .. }), "{out:?}");
    }
    let child_events = ledger.read_session(&child).unwrap();
    assert_eq!(
        child_events.iter().find(|e| e.kind == MARKER_INVESTIGATION_HANDOFF).unwrap().payload,
        handoff_ctx
    );
    let resumes: Vec<_> = child_events.iter().filter(|e| e.kind == "session.resume").collect();
    assert_eq!(resumes.len(), 2);
    assert!(resumes.iter().all(|e| e.payload[TOOL_PROFILE_FIELD] == PROFILE_RESEARCH_V1));
    assert!(!t.requests().is_empty());

    // Post-F1 hand-off child records `tool_profile: "research/1"` on `session.start` and resumes.
    let child_new = export::fork::research_from_paper(dir.path(), parent, &handoff_ctx).unwrap();
    let new_router = build_profile_router(dir.path(), Arc::clone(&env), &child_new, PROFILE_RESEARCH_V1);
    let t_new = ScriptedAnthropic::new(&["Post-F1 leg 1.", "Post-F1 leg 2."]);
    let cfg = AgentConfig {
        prompt: "Compute quartic shift.".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: "Research prompt.".into(),
        max_turns: 4,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(),
        contained: false,
        reasoning_effort: None,
    };
    run_autonomous_research_loop(
        &t_new,
        &AutonomousRoles::default(),
        &new_router,
        &ledger,
        &env,
        &cfg,
        &AutonomousConfig::default(),
        None,
        &mut NoopSink,
    )
    .unwrap();
    let new_start = ledger
        .read_session(&child_new)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "session.start")
        .unwrap();
    assert_eq!(new_start.payload[TOOL_PROFILE_FIELD], PROFILE_RESEARCH_V1);

    let (insp_new, res_new) = inspect_session_with_profile(&ledger, &child_new).unwrap();
    assert!(res_new.explicit);
    assert_eq!(insp_new.tool_profile, PROFILE_RESEARCH_V1);

    resume_autonomous_research_loop(
        &t_new,
        &AutonomousRoles::default(),
        &new_router,
        &ledger,
        &env,
        &resume_cfg("Follow up."),
        &AutonomousConfig::default(),
        None,
        &mut NoopSink,
    )
    .unwrap();

    let err = inspect_session_for_resume(&ledger, &child_new, "v0.2", &research_v1().defs).unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn guardrails_run_research_loop_records_tool_profile_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(dir.path()));
    let ledger = Ledger::open(dir.path()).unwrap();
    let sid = "s-setr-guardrails";
    let router = build_profile_router(dir.path(), env, sid, PROFILE_RESEARCH_V1);
    let t = ScriptedAnthropic::new(&["Guardrails leg 1.", "Guardrails leg 2."]);
    let cfg = AgentConfig {
        prompt: "Check oscillator energy.".into(),
        model_requested: "claude-haiku-4-5".into(),
        model: "claude-haiku-4-5".into(),
        system_prompt: "Research prompt.".into(),
        max_turns: 4,
        max_tokens: DEFAULT_MAX_TOKENS,
        containment: "none".into(),
        contained: false,
        reasoning_effort: None,
    };
    agent::run_research_loop(&t, &router, &ledger, &cfg, &agent::GuardrailConfig::default(), &mut NoopSink).unwrap();
    let start = ledger.read_session(sid).unwrap().into_iter().find(|e| e.kind == "session.start").unwrap();
    assert_eq!(start.payload[TOOL_PROFILE_FIELD], PROFILE_RESEARCH_V1);
    let (insp, resolved) = inspect_session_with_profile(&ledger, sid).unwrap();
    assert!(resolved.explicit);
    assert_eq!(insp.tool_profile, PROFILE_RESEARCH_V1);
    agent::resume_agent_loop(&t, &router, &ledger, &resume_cfg("Continue."), &mut NoopSink).unwrap();
    let resume = ledger.read_session(sid).unwrap().into_iter().find(|e| e.kind == "session.resume").unwrap();
    assert_eq!(resume.payload[TOOL_PROFILE_FIELD], PROFILE_RESEARCH_V1);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}
