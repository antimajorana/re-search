//! Integration tests for package S21: `POST /api/sessions/{id}/open-in-solve` (`solve.md` 8.4 & D19).
//!
//! Zero `#[ignore]`, zero real network calls, zero browser/CLI launches, self-written synthetic fixtures only.

mod common;
mod eval_set_r_support;

use agent::{
    inspect_session_for_resume, inspect_session_with_profile, resume_autonomous_research_loop,
    AutonomousConfig, AutonomousRoles, NoopSink, ResumeRejectKind, SessionOutcome,
    DEFAULT_MAX_TOKENS,
};
use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::solve_migrate::{LEGACY_LEAN_CHECK_RECEIPT_LABEL, LEGACY_SYMPY_RECEIPT_LABEL};
use desktop_server::start_desktop_server;
use eval_set_r_support::{
    build_current_investigation_session, build_profile_router, defs_json, ingest_synthetic_paper,
    resume_cfg, ScriptedAnthropic,
};
use ledger::{sha256_hex, Ledger};
use research_tools::paper_key::Mode;
use research_tools::solve_model::event_kinds;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{compute_all_tiers, wording, Tier};
use research_tools::tool_profile::{
    research_v1, session_mode_from_events, solve_v1, MARKER_IDEA_STARTED,
    MARKER_INVESTIGATION_HANDOFF, MARKER_INVESTIGATION_STARTED, MARKER_SOLVE_STARTED,
    PROFILE_RESEARCH_V1, TOOL_PROFILE_FIELD,
};
use research_tools::{research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn seed_pref1_research_session(data_dir: &Path, sid: &str) -> (String, String, String, Vec<u8>) {
    let env = Arc::new(ResearchEnv::new(data_dir));
    let ledger = Ledger::open(data_dir).unwrap();
    let (source_id, primary_path) = ingest_synthetic_paper(data_dir, &env, sid);
    let src_bytes = fs::read(data_dir.join("sessions").join(sid).join("sources").join(&primary_path)).unwrap();
    let src_sha = sha256_hex(&src_bytes);

    let prompt = "Compute the quartic oscillator correction using $\\omega = \\sqrt{k/m}$.";
    let sys_prompt = "Pre-F1 system prompt.\n\n<pinned_conventions>\nUse hbar = 1, m > 0.\n</pinned_conventions>";
    ledger
        .append_with_hash(
            sid,
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
                "system_prompt": sys_prompt,
                "max_turns": 16,
                "max_tokens": DEFAULT_MAX_TOKENS,
                "policy_version": research_policy().version,
                "containment": "none",
                "contained": false,
                "tool_defs": defs_json(&research_v1().defs),
            }),
        )
        .unwrap();

    let req_blob = ledger.put_blob(&serde_json::to_vec(&json!({
        "model": "claude-haiku-4-5",
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": prompt }] }]
    })).unwrap()).unwrap();
    ledger.append_with_hash(sid, "model.request", 1, &json!({
        "turn": 1, "model": "claude-haiku-4-5", "message_count": 1, "max_tokens": DEFAULT_MAX_TOKENS, "raw_blob": req_blob,
    })).unwrap();

    // e-1 active assumption, e-1b superseded assumption (must not be copied into Solve setup).
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-1", "entry_type": "assumption",
        "summary": "Small anharmonicity", "details": "0 < eps << 1", "depends_on": []
    })).unwrap();
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-1b", "entry_type": "assumption",
        "summary": "Deprecated linear guess", "details": "replaced", "depends_on": []
    })).unwrap();
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-1c", "entry_type": "correction",
        "summary": "Use perturbation series", "details": "Keep quartic term", "depends_on": []
    })).unwrap();
    ledger.append_with_hash(sid, "proof.supersede", 1, &json!({
        "entry_id": "e-1b", "replaced_by": "e-1c", "reason": "Superseded before migration"
    })).unwrap();

    // Passing SymPy receipt + failed Python-exception SymPy receipt + step e-2 (no handles -> Unchecked + "ran").
    let sympy_args = ledger.put_blob(br#"{"code":"sp.simplify(k/m)"}"#).unwrap();
    let sympy_res = ledger.put_blob(&serde_json::to_vec(&json!({
        "value": "k/m", "error": null,
        "validation": { "schema_version": 1, "kind": "symbolic_identity", "passed": true }
    })).unwrap()).unwrap();
    ledger.append_with_hash(sid, "tool.request", 1, &json!({
        "call_id": "c-sympy", "tool": "sympy_eval", "args": { "blob": sympy_args }
    })).unwrap();
    ledger.append_with_hash(sid, "tool.result", 1, &json!({
        "call_id": "c-sympy", "ok": true, "result": { "blob": sympy_res }
    })).unwrap();
    let sympy_err_args = ledger.put_blob(br#"{"code":"1/0"}"#).unwrap();
    let sympy_err_res = ledger.put_blob(&serde_json::to_vec(&json!({
        "value": null, "error": "ZeroDivisionError: division by zero"
    })).unwrap()).unwrap();
    ledger.append_with_hash(sid, "tool.request", 1, &json!({
        "call_id": "c-sympy-err", "tool": "sympy_eval", "args": { "blob": sympy_err_args }
    })).unwrap();
    ledger.append_with_hash(sid, "tool.result", 1, &json!({
        "call_id": "c-sympy-err", "ok": true, "result": { "blob": sympy_err_res }
    })).unwrap();
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-2", "entry_type": "symbolic_computation",
        "summary": "SymPy ratio check", "details": "Evaluated k/m",
        "depends_on": [], "tool_call_ref": "c-sympy"
    })).unwrap();

    // Same-model accepted review (e-3), different-model (e-4), unattributed (e-5).
    for (eid, vm) in [
        ("e-3", Some("claude-haiku-4-5")),
        ("e-4", Some("gpt-5")),
        ("e-5", None),
    ] {
        ledger.append_with_hash(sid, "proof.entry", 1, &json!({
            "entry_id": eid, "entry_type": "derivation",
            "summary": format!("Step {eid}"), "details": "Derived step", "depends_on": []
        })).unwrap();
        let mut rv = json!({ "kind": "step", "target_entry_id": eid, "verdict": "accepted" });
        if let Some(m) = vm {
            rv["verifier_model"] = json!(m);
        }
        ledger.append_with_hash(sid, "proof.verification", 1, &rv).unwrap();
    }

    // Legacy lean_check calls (passing and rejected_sorry): receipts only, NEVER grant any tier (D19).
    let lean_args = ledger.put_blob(br#"{"code":"theorem t : True := trivial"}"#).unwrap();
    ledger.append_with_hash(sid, "tool.request", 1, &json!({
        "call_id": "c-lean", "tool": "lean_check", "args": { "blob": lean_args }
    })).unwrap();
    ledger.append_with_hash(sid, "tool.result", 1, &json!({
        "call_id": "c-lean", "ok": true, "result": { "ok": true, "status": "verified" }
    })).unwrap();
    let lean_bad_args = ledger.put_blob(br#"{"code":"theorem bad : False := by sorry"}"#).unwrap();
    ledger.append_with_hash(sid, "tool.request", 1, &json!({
        "call_id": "c-lean-bad", "tool": "lean_check", "args": { "blob": lean_bad_args }
    })).unwrap();
    ledger.append_with_hash(sid, "tool.result", 1, &json!({
        "call_id": "c-lean-bad", "ok": true, "result": { "ok": false, "status": "rejected_sorry" }
    })).unwrap();
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-6", "entry_type": "derivation",
        "summary": "Lean lemma", "details": "Checked earlier in Lean",
        "depends_on": [], "tool_call_ref": "c-lean"
    })).unwrap();

    // Rejected review (e-7) -> Failed; final answer e-8 resting on e-1 and e-4.
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-7", "entry_type": "derivation",
        "summary": "Flawed step", "details": "Missing factor", "depends_on": []
    })).unwrap();
    ledger.append_with_hash(sid, "proof.verification", 1, &json!({
        "kind": "step", "target_entry_id": "e-7", "verdict": "rejected"
    })).unwrap();
    ledger.append_with_hash(sid, "proof.entry", 1, &json!({
        "entry_id": "e-8", "entry_type": "final_answer",
        "summary": "Perturbative shift", "details": "delta_omega = 3*eps/(8*omega)",
        "depends_on": ["e-1", "e-4"]
    })).unwrap();

    let resp_blob = ledger.put_blob(&serde_json::to_vec(&json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Finished leg 1." }],
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    })).unwrap()).unwrap();
    ledger.append_with_hash(sid, "model.response", 1, &json!({
        "turn": 1, "response_id": "msg_1", "model": "claude-haiku-4-5",
        "stop_reason": "end_turn", "usage": { "input_tokens": 10, "output_tokens": 5 },
        "duration_ms": 10, "raw_blob": resp_blob,
    })).unwrap();
    ledger.append_with_hash(sid, "model.text", 1, &json!({ "text": "Finished leg 1." })).unwrap();
    ledger.append_with_hash(sid, "session.end", 1, &json!({
        "outcome": "completed", "stop_reason": "end_turn", "num_turns": 1, "last_turn": 1, "duration_ms": 12
    })).unwrap();

    (source_id, primary_path, src_sha, src_bytes)
}

#[test]
fn open_in_solve_migrates_legacy_research_with_d19_tiers_sources_and_intact_parent() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    let parent = "s-legacy-research-d19";
    let (source_id, primary_path, src_sha, src_bytes) = seed_pref1_research_session(&data_dir, parent);

    // First migrate a CLI-style Research session with NO meta.json to exercise <pinned_conventions> fallback.
    let (st_cli, _, cli_raw) = raw_http(port, "POST", &format!("/api/sessions/{parent}/open-in-solve"), &auth, b"");
    assert_eq!(st_cli, 201, "{}", String::from_utf8_lossy(&cli_raw));
    let cli_res: Value = serde_json::from_slice(&cli_raw).unwrap();
    assert_eq!(cli_res["setup"]["conventions"], "Use hbar = 1, m > 0.");

    // Now attach parent to a project and set meta.pinned_conventions, then migrate again.
    let (st, _, proj_raw) = raw_http(port, "POST", "/api/projects", &auth, br#"{"title":"Oscillator Study"}"#);
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&proj_raw));
    let project_id = serde_json::from_slice::<Value>(&proj_raw).unwrap()["id"].as_str().unwrap().to_string();
    let (st, _, _) = raw_http(port, "PUT", &format!("/api/projects/{project_id}/sessions/{parent}"), &auth, b"{}");
    assert_eq!(st, 200);
    let (st, _, _) = raw_http(
        port,
        "PUT",
        &format!("/api/sessions/{parent}/conventions"),
        &auth,
        br#"{"pinned_conventions":"Natural units hbar = c = 1; m > 0, k > 0."}"#,
    );
    assert_eq!(st, 200);

    let ledger = Ledger::open(&data_dir).unwrap();
    let parent_before = ledger.read_session(parent).unwrap();
    let last_ev = parent_before.last().unwrap();

    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{parent}/open-in-solve"), &auth, b"{}");
    assert_eq!(st, 201, "open-in-solve failed: {}", String::from_utf8_lossy(&raw));
    let res: Value = serde_json::from_slice(&raw).unwrap();
    let child = res["session_id"].as_str().unwrap().to_string();
    assert_ne!(child, parent);
    assert_eq!(res["mode"], "solve");
    assert_eq!(res["parent_session"], parent);
    assert_eq!(res["project_id"], project_id);
    assert_eq!(res["ask_before_running"], false);

    // Exact origin NodeRef.
    assert_eq!(res["origin"]["session"], parent);
    assert_eq!(res["origin"]["mode"], "research");
    assert_eq!(res["origin"]["id"], format!("seq:{}", last_ev.seq));
    assert_eq!(res["origin"]["node_fp"], last_ev.event_hash);

    // Setup carries prompt, conventions, active assumption e-1 (not superseded e-1b), source, canonical solver role.
    assert!(res["setup"]["statement"].as_str().unwrap().contains("quartic oscillator correction"));
    assert_eq!(res["setup"]["conventions"], "Natural units hbar = c = 1; m > 0, k > 0.");
    assert_eq!(res["setup"]["sources"], json!([source_id]));
    assert_eq!(res["setup"]["roles"]["solver"]["provider"], "anthropic");
    assert_eq!(res["setup"]["roles"]["solver"]["model"], "claude-haiku-4-5");
    let setup_assumptions = res["setup"]["assumptions"].as_array().unwrap();
    assert_eq!(setup_assumptions.len(), 1);
    assert_eq!(setup_assumptions[0]["id"], "e-1");

    // Copied source file in child matches parent bytes and SHA-256.
    let child_src = data_dir.join("sessions").join(&child).join("sources").join(&primary_path);
    assert!(child_src.is_file(), "copied source missing at {}", child_src.display());
    let child_bytes = fs::read(&child_src).unwrap();
    assert_eq!(child_bytes, src_bytes);
    assert_eq!(sha256_hex(&child_bytes), src_sha);

    // Child ledger event sequence: ... -> session.fork -> solve.started -> solve.setup (with schema fields).
    let child_events = ledger.read_session(&child).unwrap();
    assert_eq!(session_mode_from_events(&child_events), Mode::Solve);
    let tail: Vec<&str> = child_events[child_events.len() - 3..].iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(tail, vec!["session.fork", MARKER_SOLVE_STARTED, event_kinds::SOLVE_SETUP]);
    let setup_ev = child_events.last().unwrap();
    assert_eq!(setup_ev.payload["revision"], 1);
    assert_eq!(setup_ev.payload["tool_profile"], event_kinds::SOLVE_TOOL_PROFILE_V1);
    assert_eq!(setup_ev.payload["ask_before_running"], false);
    assert_eq!(setup_ev.payload["math_sha256"].as_str().unwrap().len(), 64);

    // Legacy receipts report lean_check and sympy_eval honestly (including inner failures) without granting tiers.
    let receipts = res["legacy_receipts"].as_array().unwrap();
    let lean_rec = receipts.iter().find(|r| r["call_id"] == "c-lean").expect("lean receipt");
    assert_eq!(lean_rec["label"], LEGACY_LEAN_CHECK_RECEIPT_LABEL);
    assert_eq!(lean_rec["ok"], true);
    assert_eq!(lean_rec["status"], "verified");
    assert_eq!(lean_rec["grants_tier"], false);
    let lean_bad = receipts.iter().find(|r| r["call_id"] == "c-lean-bad").expect("bad lean receipt");
    assert_eq!(lean_bad["label"], LEGACY_LEAN_CHECK_RECEIPT_LABEL);
    assert_eq!(lean_bad["ok"], false);
    assert_eq!(lean_bad["status"], "rejected_sorry");
    assert_eq!(lean_bad["grants_tier"], false);
    let sympy_rec = receipts.iter().find(|r| r["call_id"] == "c-sympy").expect("sympy receipt");
    assert_eq!(sympy_rec["label"], LEGACY_SYMPY_RECEIPT_LABEL);
    assert_eq!(sympy_rec["ok"], true);
    assert_eq!(sympy_rec["grants_tier"], false);
    let sympy_err = receipts.iter().find(|r| r["call_id"] == "c-sympy-err").expect("failed sympy receipt");
    assert_eq!(sympy_err["ok"], false);
    assert!(sympy_err["error"].as_str().unwrap().contains("ZeroDivisionError"));
    assert_eq!(sympy_err["grants_tier"], false);

    // Check D19 tiers across response, GET /solve/view, and compute_all_tiers.
    let (st_v, _, view_raw) = raw_http(port, "GET", &format!("/api/sessions/{child}/solve/view"), &auth, b"");
    assert_eq!(st_v, 200);
    let view: Value = serde_json::from_slice(&view_raw).unwrap();
    let folded = SolveState::from_events(&child_events);
    let computed = compute_all_tiers(&folded);

    let step_badge = |eid: &str| -> Value {
        view["steps"].as_array().unwrap().iter().find(|s| s["entry_id"] == eid).unwrap()["badge"].clone()
    };
    assert_eq!(computed["e-1"].own, Tier::AssumedByModel);
    assert_eq!(step_badge("e-1")["effective"], "assumed_by_model");
    assert_eq!(computed["e-2"].own, Tier::Unchecked);
    assert_eq!(computed["e-2"].sublabels, vec![wording::SUBLABEL_RAN.to_string()]);
    assert_eq!(step_badge("e-2")["effective"], "unchecked");
    assert_eq!(computed["e-3"].own, Tier::ModelAgreedSameModel);
    assert!(computed["e-3"].flags.contains(&wording::FLAG_SAME_MODEL.to_string()));
    assert_eq!(computed["e-4"].own, Tier::ModelAgreed);
    assert!(!computed["e-4"].flags.contains(&wording::FLAG_SAME_MODEL.to_string()));
    assert_eq!(computed["e-5"].own, Tier::ModelAgreedSameModel);
    assert_eq!(computed["e-6"].own, Tier::Unchecked);
    assert_eq!(computed["e-6"].effective, Tier::Unchecked);
    assert_eq!(computed["e-7"].own, Tier::Failed);
    assert_eq!(computed["e-8"].effective, Tier::Unchecked);
    assert!(computed["e-8"].flags.contains(&"rests on 1 assumption made by the model".to_string()));

    // Child inherits project membership and clears conventions cleanly when parent clears them.
    let (st_p, _, sess_raw) = raw_http(port, "GET", &format!("/api/sessions/{child}"), &auth, b"");
    assert_eq!(st_p, 200);
    assert_eq!(serde_json::from_slice::<Value>(&sess_raw).unwrap()["project_id"], project_id);

    let (st_clr, _, _) = raw_http(port, "PUT", &format!("/api/sessions/{parent}/conventions"), &auth, br#"{"pinned_conventions":""}"#);
    assert_eq!(st_clr, 200);
    let (st_c2, _, c2_raw) = raw_http(port, "POST", &format!("/api/sessions/{parent}/open-in-solve"), &auth, b"{}");
    assert_eq!(st_c2, 201);
    assert_eq!(serde_json::from_slice::<Value>(&c2_raw).unwrap()["setup"]["conventions"], "");

    // Ledger integrity + zero orphan blobs before resuming parent.
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);

    // Parent Research session is completely untouched and still resumes cleanly under `research/1`.
    let parent_after = ledger.read_session(parent).unwrap();
    assert_eq!(parent_after, parent_before);
    assert_eq!(session_mode_from_events(&parent_after), Mode::Research);

    let (insp, resolved) = inspect_session_with_profile(&ledger, parent).unwrap();
    assert_eq!(resolved.profile.name, PROFILE_RESEARCH_V1);
    assert_eq!(insp.tool_profile, PROFILE_RESEARCH_V1);
    inspect_session_for_resume(&ledger, parent, &research_policy().version, &research_v1().defs).unwrap();
    assert_eq!(inspect_session_with_profile(&ledger, &child).unwrap_err().kind, ResumeRejectKind::Drift);

    let env = Arc::new(ResearchEnv::new(&data_dir));
    let router = build_profile_router(&data_dir, env, parent, PROFILE_RESEARCH_V1);
    let transport = ScriptedAnthropic::new(&["Parent continues cleanly in Research."]);
    let out = resume_autonomous_research_loop(
        &transport,
        &AutonomousRoles::default(),
        &router,
        &ledger,
        &ResearchEnv::new(&data_dir),
        &resume_cfg("Check the next term."),
        &AutonomousConfig::default(),
        None,
        &mut NoopSink,
    )
    .unwrap();
    assert!(matches!(out, SessionOutcome::Completed { .. }), "{out:?}");
    assert_eq!(transport.requests().len(), 1);
    let parent_resumed = ledger.read_session(parent).unwrap();
    let resume_ev = parent_resumed.iter().rev().find(|e| e.kind == "session.resume").unwrap();
    assert_eq!(resume_ev.payload[TOOL_PROFILE_FIELD], PROFILE_RESEARCH_V1);
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);

    srv.shutdown();
}

#[test]
fn open_in_solve_enforces_auth_origin_and_refuses_non_research_or_invalid_requests() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    let res_sid = "s-auth-research";
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append_new_session(res_sid, &[("session.start", 1, &json!({ "prompt": "Test auth" }))]).unwrap();
    let sessions_before = ledger.sessions().unwrap().len();

    // 401 without token or with stream cookie only; 403 on foreign Origin.
    let path = format!("/api/sessions/{res_sid}/open-in-solve");
    let (st, _, _) = raw_http(port, "POST", &path, &[], b"{}");
    assert_eq!(st, 401);
    assert_eq!(ledger.sessions().unwrap().len(), sessions_before);

    let (st, _, _) = raw_http(port, "POST", &path, &[("Cookie", cookie.as_str())], b"{}");
    assert_eq!(st, 401);
    assert_eq!(ledger.sessions().unwrap().len(), sessions_before);

    let (st, _, _) = raw_http(
        port, "POST", &path, &[("X-ReSearch-Token", tok.as_str()), ("Origin", "http://evil.invalid")], b"{}",
    );
    assert_eq!(st, 403);
    assert_eq!(ledger.sessions().unwrap().len(), sessions_before);

    // Invalid id -> 400; missing session -> 404; empty meta-only Research session -> 400.
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/BAD_ID/open-in-solve", &auth, b"{}");
    assert_eq!(st, 400);
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-missing/open-in-solve", &auth, b"{}");
    assert_eq!(st, 404);
    let (st, _, _) = raw_http(port, "POST", "/api/sessions", &auth, br#"{"id":"s-empty-res","mode":"research"}"#);
    assert_eq!(st, 200);
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-empty-res/open-in-solve", &auth, b"{}");
    assert_eq!(st, 400);

    // Non-Research modes (Investigate, Explore, Solve) are refused with 400.
    for (sid, marker) in [
        ("s-mode-inv", MARKER_INVESTIGATION_STARTED),
        ("s-mode-exp", MARKER_IDEA_STARTED),
        ("s-mode-slv", MARKER_SOLVE_STARTED),
    ] {
        ledger.append_new_session(sid, &[(marker, 1, &json!({}))]).unwrap();
        let (st, _, _) = raw_http(port, "POST", &format!("/api/sessions/{sid}/open-in-solve"), &auth, b"{}");
        assert_eq!(st, 400, "non-research session {sid} must be refused");
    }

    // Invalid JSON, unknown key, solver override (B1 D19 guard), blank/oversized verifier -> 400 + ZERO orphans.
    let count_before_invalid = ledger.sessions().unwrap().len();
    let (st_solver, _, solver_raw) =
        raw_http(port, "POST", &path, &auth, br#"{"solver":{"provider":"openai","model":"gpt-5"}}"#);
    assert_eq!(st_solver, 400);
    assert!(String::from_utf8_lossy(&solver_raw).contains("D19"), "{}", String::from_utf8_lossy(&solver_raw));
    let long_model = serde_json::to_vec(&json!({"solver": {"provider": "openai", "model": "m".repeat(201)}})).unwrap();
    let long_ver = serde_json::to_vec(&json!({"verifier": {"provider": "openai", "model": "m".repeat(201)}})).unwrap();
    for bad_body in [
        b"{not json" as &[u8], b"[]", br#"{"atSeq":1}"#, br#"{"at_seq":0}"#, br#"{"at_seq":99}"#,
        br#"{"verifier":{"provider":"","model":"gpt-5"}}"#, long_model.as_slice(), long_ver.as_slice(),
    ] {
        let (st, _, _) = raw_http(port, "POST", &path, &auth, bad_body);
        assert_eq!(st, 400);
    }
    assert_eq!(ledger.sessions().unwrap().len(), count_before_invalid, "invalid body must never commit an orphan fork");

    // Prefix pointing at Investigate marker inside a Research-ending session is refused cleanly (zero orphan forks).
    let mixed_sid = "s-mixed-prefix";
    ledger.append_new_session(mixed_sid, &[
        (MARKER_INVESTIGATION_STARTED, 1, &json!({})),
        (MARKER_INVESTIGATION_HANDOFF, 1, &json!({ "question": "Follow up" })),
    ]).unwrap();
    let count_before_mixed = ledger.sessions().unwrap().len();
    let (st, _, _) = raw_http(port, "POST", &format!("/api/sessions/{mixed_sid}/open-in-solve"), &auth, br#"{"at_seq":1}"#);
    assert_eq!(st, 400, "non-Research prefix at_seq must be refused");
    assert_eq!(ledger.sessions().unwrap().len(), count_before_mixed);

    // Tampered parent ledger (corrupted referenced blob) is refused cleanly with zero new sessions created.
    let tampered = "s-tampered-res";
    let bad_blob = ledger.put_blob(br#"{"model":"gpt-5","messages":[]}"#).unwrap();
    ledger.append_new_session(tampered, &[
        ("session.start", 1, &json!({ "prompt": "Untampered" })),
        ("model.request", 1, &json!({ "turn": 1, "raw_blob": bad_blob })),
    ]).unwrap();
    fs::write(data_dir.join("blobs").join(&bad_blob[..2]).join(&bad_blob), b"corrupted-blob-bytes").unwrap();
    let count_before_tamper = ledger.sessions().unwrap().len();
    let (st, _, _) = raw_http(port, "POST", &format!("/api/sessions/{tampered}/open-in-solve"), &auth, b"{}");
    assert_eq!(st, 400, "tampered parent ledger must fail verification cleanly");
    assert_eq!(ledger.sessions().unwrap().len(), count_before_tamper);

    // Tampered source bytes on disk are also refused cleanly with zero new sessions created.
    let env = ResearchEnv::new(&data_dir);
    let (_, tampered_src_path) = ingest_synthetic_paper(&data_dir, &env, "s-tampered-src");
    ledger.append_with_hash("s-tampered-src", "session.start", 1, &json!({ "prompt": "Src check" })).unwrap();
    fs::write(
        data_dir.join("sessions").join("s-tampered-src").join("sources").join(&tampered_src_path),
        b"tampered source content",
    ).unwrap();
    let count_before_src = ledger.sessions().unwrap().len();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-tampered-src/open-in-solve", &auth, b"{}");
    assert_eq!(st, 400, "tampered source file must be refused");
    assert_eq!(ledger.sessions().unwrap().len(), count_before_src);

    // Valid token + loopback Origin succeeds on untampered Research session.
    let origin = format!("http://127.0.0.1:{port}");
    let (st, _, ok_raw) = raw_http(
        port, "POST", &path, &[("X-ReSearch-Token", tok.as_str()), ("Origin", origin.as_str())], b"{}",
    );
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&ok_raw));
    assert_eq!(serde_json::from_slice::<Value>(&ok_raw).unwrap()["mode"], "solve");

    srv.shutdown();
}

#[test]
fn open_in_solve_supports_at_seq_prefix_and_legacy_investigation_handoff_research_sessions() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // (a) Prefix fork via `at_seq` excludes events after `at_seq`.
    let ledger = Ledger::open(&data_dir).unwrap();
    let prefix_sid = "s-prefix-research";
    ledger.append_with_hash(prefix_sid, "session.start", 1, &json!({
        "prompt": "Derive the virial ratio.",
        "provider": "openai-chat",
        "model": "gpt-5"
    })).unwrap();
    ledger.append_with_hash(prefix_sid, "proof.entry", 1, &json!({
        "entry_id": "e-1", "entry_type": "assumption",
        "summary": "Bounded orbit", "details": "Finite phase-space trajectory", "depends_on": []
    })).unwrap();
    let seq2_hash = ledger.read_session(prefix_sid).unwrap().last().unwrap().event_hash.clone();
    ledger.append_with_hash(prefix_sid, "proof.entry", 1, &json!({
        "entry_id": "e-2", "entry_type": "assumption",
        "summary": "Post-cutoff assumption", "details": "Must not appear in seq 2 prefix", "depends_on": []
    })).unwrap();

    let (st, _, raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{prefix_sid}/open-in-solve"),
        &auth,
        br#"{"at_seq":2}"#,
    );
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&raw));
    let pref_res: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(pref_res["origin"]["id"], "seq:2");
    assert_eq!(pref_res["origin"]["node_fp"], seq2_hash);
    assert_eq!(pref_res["fork"]["copied_events"], 2);
    assert_eq!(pref_res["setup"]["roles"]["solver"]["provider"], "openai");
    let pref_assumptions = pref_res["setup"]["assumptions"].as_array().unwrap();
    assert_eq!(pref_assumptions.len(), 1);
    assert_eq!(pref_assumptions[0]["id"], "e-1");
    assert!(pref_res["tiers"].get("e-2").is_none());

    // (b) Legacy `investigation.handoff` child is `Mode::Research` and falls back to `question`.
    let env = Arc::new(ResearchEnv::new(&data_dir));
    let inv_parent = "s-inv-for-handoff";
    let (source_id, primary_path, map_seq, map_hash) =
        build_current_investigation_session(&data_dir, &env, inv_parent);
    let handoff_ctx = json!({
        "parent_session": inv_parent,
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
    let handoff_child = export::fork::research_from_paper(&data_dir, inv_parent, &handoff_ctx).unwrap();
    let handoff_events = ledger.read_session(&handoff_child).unwrap();
    assert_eq!(session_mode_from_events(&handoff_events), Mode::Research);
    assert!(handoff_events.iter().any(|e| e.kind == MARKER_INVESTIGATION_HANDOFF));

    let (st_h, _, raw_h) = raw_http(port, "POST", &format!("/api/sessions/{handoff_child}/open-in-solve"), &auth, b"{}");
    assert_eq!(st_h, 201, "{}", String::from_utf8_lossy(&raw_h));
    let h_res: Value = serde_json::from_slice(&raw_h).unwrap();
    assert_eq!(h_res["mode"], "solve");
    assert_eq!(h_res["setup"]["statement"], "Extend Theorem T1 to a quartic anharmonic perturbation.");
    assert_eq!(h_res["setup"]["sources"], json!([source_id]));
    let solve_child = h_res["session_id"].as_str().unwrap();
    assert!(data_dir.join("sessions").join(solve_child).join("sources").join(&primary_path).is_file());

    // (c) Post-F1 Research session (`tool_profile: "research/1"`, `route: "api"` without `provider`):
    // migrates cleanly, does not leak `"api"` into `solver.provider`, honors `verifier` override,
    // and fails closed with Rule R2 `Drift` on replay-resume under `solve/1`.
    let pf1 = "s-postf1-research";
    ledger.append_with_hash(pf1, "session.start", 1, &json!({
        "route": "api", "model": "gpt-5", "prompt": "Post-F1 Research problem.",
        TOOL_PROFILE_FIELD: PROFILE_RESEARCH_V1,
        "policy_version": research_policy().version,
        "tool_defs": defs_json(&research_v1().defs),
    })).unwrap();
    let (st_p, _, raw_p) = raw_http(
        port, "POST", &format!("/api/sessions/{pf1}/open-in-solve"), &auth,
        br#"{"verifier":{"provider":"anthropic-messages","model":"claude-sonnet-4-6"}}"#,
    );
    assert_eq!(st_p, 201, "{}", String::from_utf8_lossy(&raw_p));
    let p_res: Value = serde_json::from_slice(&raw_p).unwrap();
    assert_eq!(p_res["setup"]["roles"]["solver"], json!({ "provider": "", "model": "gpt-5" }));
    assert_eq!(p_res["setup"]["roles"]["verifier"], json!({ "provider": "anthropic", "model": "claude-sonnet-4-6" }));
    let pf1_child = p_res["session_id"].as_str().unwrap();
    let sv1 = solve_v1();
    assert_eq!(
        inspect_session_for_resume(&ledger, pf1_child, &sv1.policy.version, &sv1.defs).unwrap_err().kind,
        ResumeRejectKind::Drift,
    );
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);

    srv.shutdown();
}
