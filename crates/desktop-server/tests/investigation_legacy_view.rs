//! End-to-end HTTP integration tests for Investigate V9 read-side legacy adapter
//! (`GET /api/sessions/{id}/investigation/legacy`), Set R shapes, tampered fallback, auth,
//! innermost span placement, determinism, zero `"verified"`, and zero guessed `"proof"` edges.

mod common;
#[allow(dead_code)]
mod eval_set_r_support;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::investigation_legacy_view::LEGACY_EDGE_ROLE;
use desktop_server::start_desktop_server;
use eval_set_r_support::{
    build_current_investigation_session, build_profile_router, defs_json, ingest_synthetic_paper,
    write_pref1_completed_leg1,
};
use ledger::Ledger;
use research_tools::investigation::{
    GOLDEN_BRANCH_7C36A0E, GOLDEN_CONVENTIONS_7C36A0E, GOLDEN_MAP_7C36A0E,
    GOLDEN_SUMMARY_7C36A0E, RECORD_PAPER_BRANCH, RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP,
    RECORD_PAPER_SUMMARY,
};
use research_tools::tool_profile::{
    investigation_v1, research_v1, MARKER_INVESTIGATION_STARTED, PROFILE_RESEARCH_V1,
};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::CallOutcome;

const NESTED_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\section{Framework}\label{sec:fw}
\begin{lemma}[Quadratic Coercivity]\label{lem:coer}
Assume $V(x) = \frac{1}{2} k x^2$ with $k > 0$.
\end{lemma}
\begin{theorem}[Harmonic Period]\label{thm:per}
The angular frequency is $\omega = \sqrt{k / m}$ regardless of amplitude.
\end{theorem}
\begin{proof}
Follows directly from Lemma~\ref{lem:coer} and energy conservation.
\end{proof}
\end{document}
"#;

fn get_legacy(port: u16, tok: &str, sid: &str) -> (u16, Value) {
    let (st, _, raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/legacy"),
        &[("X-ReSearch-Token", tok)],
        b"",
    );
    let val = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    (st, val)
}

fn get_mapview(port: u16, tok: &str, sid: &str) -> (u16, Value) {
    let (st, _, raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/mapview"),
        &[("X-ReSearch-Token", tok)],
        b"",
    );
    let val = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    (st, val)
}

#[test]
fn legacy_route_enforces_auth_set_r_shapes_fallback_and_pure_v2() {
    let dir = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.info.port;
    let tok = srv.info.token.clone();
    let tok_hdr = ("X-ReSearch-Token", tok.as_str());
    let env = Arc::new(ResearchEnv::new(dir.path()));

    // 1. Auth guard (401 missing token / cookie-only, 403 foreign Host) and invalid session id (400).
    let (unauth, _, _) = raw_http(port, "GET", "/api/sessions/s-any/investigation/legacy", &[], b"");
    assert_eq!(unauth, 401);
    let cookie = launch_cookie(port, &tok);
    let (cookie_only, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-any/investigation/legacy",
        &[("Cookie", cookie.as_str())],
        b"",
    );
    assert_eq!(cookie_only, 401);
    let (bad_host, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-any/investigation/legacy",
        &[tok_hdr, ("Host", "evil.example")],
        b"",
    );
    assert_eq!(bad_host, 403);
    let (bad_sid, _) = get_legacy(port, &tok, "..");
    assert_eq!(bad_sid, 400);

    // 2. Set R.1: Pre-F1 Research session -> has_legacy = false, zero ledger mutations.
    let sid_r1 = "s-v9-setr-research";
    {
        let ledger = Ledger::open(dir.path()).unwrap();
        let router = build_profile_router(dir.path(), Arc::clone(&env), sid_r1, PROFILE_RESEARCH_V1);
        write_pref1_completed_leg1(
            &ledger,
            sid_r1,
            "Compute harmonic period.",
            "Period is 2*pi*sqrt(m/k).",
            defs_json(&research_v1().defs),
        );
        let call = router
            .try_call(
                &format!("agent:anthropic-messages:{sid_r1}"),
                "ledger_append",
                &json!({
                    "entry_type": "assumption",
                    "summary": "Hookean potential",
                    "details": "k > 0 and m > 0.",
                    "depends_on": []
                }),
            )
            .unwrap();
        assert!(matches!(call, CallOutcome::Ok(_)));
    }
    let before_r1 = Ledger::open(dir.path()).unwrap().read_session(sid_r1).unwrap().len();
    let (st_r1, v_r1) = get_legacy(port, &tok, sid_r1);
    assert_eq!(st_r1, 200);
    assert_eq!(v_r1["has_legacy"], json!(false));
    assert_eq!(Ledger::open(dir.path()).unwrap().read_session(sid_r1).unwrap().len(), before_r1);

    // 3. Set R.2: Pre-pointer 7c36a0e Investigation (one span-placed node, one pre-pointer node without lines).
    let sid_r2 = "s-v9-setr-7c36a0e";
    {
        let ledger = Ledger::open(dir.path()).unwrap();
        ledger
            .append_with_hash(sid_r2, MARKER_INVESTIGATION_STARTED, 1, &json!({ "title": "Pre-pointer" }))
            .unwrap();
        let (source_id, primary_path) = ingest_synthetic_paper(dir.path(), &env, sid_r2);
        ledger
            .append_with_hash(
                sid_r2,
                "investigation.paper",
                1,
                &json!({ "source_id": source_id, "primary_path": primary_path }),
            )
            .unwrap();
        let pre_defs: Vec<Value> = investigation_v1()
            .defs
            .iter()
            .filter(|d| d.id != "read_paper_lines")
            .map(|d| {
                let hash = match d.id.as_str() {
                    RECORD_PAPER_MAP => GOLDEN_MAP_7C36A0E.to_string(),
                    RECORD_PAPER_SUMMARY => GOLDEN_SUMMARY_7C36A0E.to_string(),
                    RECORD_PAPER_BRANCH => GOLDEN_BRANCH_7C36A0E.to_string(),
                    RECORD_PAPER_CONVENTIONS => GOLDEN_CONVENTIONS_7C36A0E.to_string(),
                    _ => d.def_hash(),
                };
                json!({ "id": d.id, "effect": d.effect.as_str(), "approval_scope": d.approval_scope.as_str(), "def_hash": hash })
            })
            .collect();
        write_pref1_completed_leg1(&ledger, sid_r2, "Map paper.", "Saved old map.", pre_defs);
        ledger
            .append_with_hash(
                sid_r2,
                "investigation.map",
                1,
                &json!({
                    "source_id": source_id,
                    "title": "Harmonic Scaling",
                    "coverage": "paper",
                    "limitations": [],
                    "nodes": [
                        {
                            "id": "t1",
                            "sentence": "Angular frequency is sqrt(k/m).",
                            "kind": "result",
                            "main": true,
                            "anchors": [{ "source_id": source_id, "path": primary_path, "start_line": 4, "end_line": 4, "excerpt": "Theorem T1", "kind": "passage" }]
                        },
                        {
                            "id": "old_n",
                            "sentence": "Legacy note lacking physical line numbers.",
                            "kind": "claim",
                            "main": false,
                            "anchors": [{ "source_id": source_id, "excerpt": "Old quote only", "kind": "passage" }]
                        }
                    ],
                    "edges": [{
                        "from": "old_n",
                        "to": "t1",
                        "kind": "depends_on",
                        "basis": "inferred",
                        "reason": "Early link.",
                        "anchor": { "source_id": source_id, "path": primary_path, "start_line": 4, "end_line": 4, "kind": "passage" }
                    }]
                }),
            )
            .unwrap();
    }
    let (st_r2, v_r2) = get_legacy(port, &tok, sid_r2);
    assert_eq!(st_r2, 200);
    assert_eq!(v_r2["has_legacy"], json!(true));
    assert_eq!(v_r2["fallback_old_view"], json!(false));
    assert_eq!(v_r2["placed"].as_object().unwrap().len(), 1);
    assert_eq!(v_r2["unplaced"].as_array().unwrap().len(), 1);
    assert_eq!(v_r2["edges"][0]["role"], json!(LEGACY_EDGE_ROLE));
    assert_eq!(v_r2["edges"][0]["basis"], json!("model"));
    assert_eq!(v_r2["edges"][0]["dashed"], json!(true));
    // Determinism & idempotent skeleton open upgrade (second call returns identical projection and zero duplicate skeletons).
    let (st_r2b, v_r2b) = get_legacy(port, &tok, sid_r2);
    assert_eq!(st_r2b, 200);
    assert_eq!(v_r2, v_r2b);
    let r2_events = Ledger::open(dir.path()).unwrap().read_session(sid_r2).unwrap();
    assert_eq!(r2_events.iter().filter(|e| e.kind == "investigation.skeleton").count(), 1);
    assert!(!r2_events.iter().skip_while(|e| e.kind != "investigation.map").any(|e| e.kind == "model.request"));

    // 4. Set R.3 + Set R.4: Current pointer-cited Investigation + handoff child.
    let sid_r3 = "s-v9-setr-current";
    let (src_r3, path_r3, map_seq_r3, map_hash_r3) =
        build_current_investigation_session(dir.path(), &env, sid_r3);
    let handoff_child = export::fork::research_from_paper(
        dir.path(),
        sid_r3,
        &json!({
            "parent_session": sid_r3, "map_seq": map_seq_r3, "map_hash": map_hash_r3,
            "source_id": src_r3, "primary_source_id": src_r3,
            "question": "Extend Theorem T1.", "sources": [{ "source_id": src_r3, "path": path_r3, "start_line": 4, "end_line": 4, "kind": "passage" }]
        }),
    )
    .unwrap();
    let (st_r3, v_r3) = get_legacy(port, &tok, sid_r3);
    assert_eq!(st_r3, 200);
    assert_eq!(v_r3["has_legacy"], json!(true));
    assert_eq!(v_r3["edges"][0]["role"], json!(LEGACY_EDGE_ROLE));
    assert!(!v_r3.to_string().to_ascii_lowercase().contains("verified"));
    let (st_r4, v_r4) = get_legacy(port, &tok, &handoff_child);
    assert_eq!(st_r4, 200);
    assert_eq!(v_r4["has_legacy"], json!(false));

    srv.shutdown();
    let ledger = Ledger::open(dir.path()).unwrap();
    for s in [sid_r1, sid_r2, sid_r3, handoff_child.as_str()] {
        ledger.verify_session(s).expect("session hash chain valid");
    }
    let skel_blobs: std::collections::BTreeSet<String> = [sid_r2, sid_r3]
        .into_iter()
        .flat_map(|s| ledger.read_session(s).unwrap())
        .filter(|e| e.kind == "investigation.skeleton")
        .filter_map(|e| e.payload["blob"].as_str().map(str::to_string))
        .collect();
    assert_eq!(skel_blobs.len(), 1);
    // Note: Ledger::verify_all counts top-level `investigation.skeleton.blob` outside its
    // `raw_blob`/`tool.*` allowlist (`ledger/src/verify.rs:280-306`), so exactly those distinct
    // skeleton blobs account for `unreferenced_blobs` while zero tamper errors occur.
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, skel_blobs.len() as u64);
}

#[test]
fn nested_tex_innermost_placement_branch_correction_summary_conventions_and_tampered_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.info.port;
    let tok = srv.info.token.clone();
    let env = ResearchEnv::new(dir.path());

    // Rich nested TeX session with map, branch, correction, summary, conventions, answers, and activities.
    let sid = "s-v9-rich";
    let ledger_mux = Mutex::new(Ledger::open(dir.path()).unwrap());
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "nested.tex",
            origin: "upload",
            raw_bytes: NESTED_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Nested Fixture"),
        },
    )
    .unwrap();
    drop(ledger_mux);
    let ledger = Ledger::open(dir.path()).unwrap();
    let src_id = ingested.source_id;
    let p_path = ingested.primary_path.unwrap();
    ledger.append_with_hash(sid, MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash(sid, "investigation.paper", 1, &json!({ "source_id": src_id })).unwrap();
    let map_req = ledger.append_with_hash(sid, "investigation.mapping_requested", 1, &json!({ "source_id": src_id })).unwrap().seq;
    ledger
        .append_with_hash(
            sid,
            "investigation.summary",
            1,
            &json!({
                "source_id": src_id, "request_seq": map_req,
                "what": { "text": "Bounds frequency.", "anchors": [] },
                "why": { "unstated": true },
                "how": { "text": "Via energy.", "anchors": [] }
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "investigation.conventions",
            1,
            &json!({
                "request_seq": map_req, "source_id": src_id, "batch_index": 1, "done": true,
                "coverage": "paper", "coverage_scope": "paper", "limitations": [],
                "items": [{ "id": "omega", "symbol": "\\omega", "meaning": "angular frequency", "anchors": [] }]
            }),
        )
        .unwrap();
    let map_seq = ledger
        .append_with_hash(
            sid,
            "investigation.map",
            1,
            &json!({
                "source_id": src_id, "title": "Nested Map", "coverage": "paper", "limitations": [],
                "nodes": [
                    {
                        "id": "lem", "sentence": "Coercivity assumption.", "kind": "assumption", "main": false,
                        "anchors": [{ "source_id": src_id, "path": p_path, "start_line": 7, "end_line": 7, "kind": "passage" }]
                    },
                    {
                        "id": "thm", "sentence": "Main frequency bound.", "kind": "result", "main": true,
                        "anchors": [{ "source_id": src_id, "path": p_path, "start_line": 10, "end_line": 10, "kind": "passage" }]
                    },
                    {
                        "id": "unp", "sentence": "Out-of-bounds legacy node.", "kind": "claim", "main": false,
                        "anchors": [{ "source_id": src_id, "path": p_path, "start_line": 250, "end_line": 250, "kind": "passage" }]
                    }
                ],
                "edges": [{ "from": "lem", "to": "thm", "kind": "depends_on", "basis": "explicit", "reason": "Direct proof step.", "anchor": { "source_id": src_id, "path": p_path, "start_line": 13, "end_line": 13, "kind": "passage" } }]
            }),
        )
        .unwrap()
        .seq;
    let branch_seq = ledger
        .append_with_hash(
            sid,
            "investigation.branch",
            1,
            &json!({
                "map_seq": map_seq, "source_id": src_id, "request_seq": 99, "node_id": "thm", "question": "Check proof.", "limitations": [],
                "nodes": [{ "id": "b1", "sentence": "Proof conservation step.", "kind": "detail", "main": false, "anchors": [{ "source_id": src_id, "path": p_path, "start_line": 13, "end_line": 13, "kind": "passage" }] }],
                "edges": [{ "from": "b1", "to": "thm", "kind": "depends_on", "basis": "inferred", "reason": "Proof supports theorem.", "anchor": { "source_id": src_id, "path": p_path, "start_line": 13, "end_line": 13, "kind": "passage" } }]
            }),
        )
        .unwrap()
        .seq;
    ledger
        .append_with_hash(
            sid,
            "investigation.correction",
            1,
            &json!({ "map_seq": map_seq, "node_id": "thm", "edge_from": "lem", "text": "Clarify constant k > 0." }),
        )
        .unwrap();
    let q_seq = ledger.append_with_hash(sid, "investigation.question_requested", 1, &json!({ "question": "Is omega independent of A?" })).unwrap().seq;
    ledger.append_with_hash(sid, "investigation.answer", 1, &json!({ "request_seq": q_seq, "status": "answered", "text": "Yes.", "cites": [] })).unwrap();
    let r_seq = ledger.append_with_hash(sid, "investigation.requested", 1, &json!({ "question": "Check small amplitude limit." })).unwrap().seq;
    ledger.append_with_hash(sid, "investigation.result", 1, &json!({ "request_seq": r_seq, "status": "complete", "text": "Confirmed." })).unwrap();

    let (st, v) = get_legacy(port, &tok, sid);
    assert_eq!(st, 200);
    assert_eq!(v["has_legacy"], json!(true));
    assert_eq!(v["fallback_old_view"], json!(false));
    let placed_map = v["placed"].as_object().unwrap();
    // Should attach to innermost nodes (Lemma, Theorem, Proof), NOT outer Section!
    assert_eq!(placed_map.len(), 3, "expected 3 distinct inner skeleton nodes, got {placed_map:?}");
    let all_placed_ids: Vec<&str> = placed_map
        .values()
        .flat_map(|arr| arr.as_array().unwrap())
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(all_placed_ids.contains(&format!("legacy:{map_seq}:lem").as_str()));
    assert!(all_placed_ids.contains(&format!("legacy:{map_seq}:thm").as_str()));
    assert!(all_placed_ids.contains(&format!("legacy:{branch_seq}:b1").as_str()));
    assert_eq!(v["unplaced"].as_array().unwrap().len(), 1);
    assert_eq!(v["unplaced"][0]["id"], json!(format!("legacy:{map_seq}:unp")));
    assert_eq!(v["corrections"][0]["text"], json!("Clarify constant k > 0."));
    assert_eq!(v["summary"]["what"]["text"], json!("Bounds frequency."));
    assert_eq!(v["conventions"]["items"][0]["symbol"], json!("\\omega"));
    assert_eq!(v["answers"][0]["text"], json!("Yes."));
    assert_eq!(v["activities"][0]["result"], json!("Confirmed."));
    assert!(!v.to_string().to_ascii_lowercase().contains("verified"));

    // Direct integration: GET /investigation/mapview embeds `legacy` and per-node `earlier_map`.
    let (mv_st, mv) = get_mapview(port, &tok, sid);
    assert_eq!(mv_st, 200);
    assert_eq!(mv["legacy"]["has_legacy"], json!(true));
    assert_eq!(mv["legacy"]["unplaced"].as_array().unwrap().len(), 1);
    let attached_nodes: Vec<&Value> = mv["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n.get("earlier_map").is_some_and(Value::is_array))
        .collect();
    assert_eq!(attached_nodes.len(), 3);
    assert!(!mv.to_string().to_ascii_lowercase().contains("verified"));

    // Tampered source file on legacy session -> fallback_old_view: true with zero new skeleton event.
    let sid_tamp = "s-v9-tampered";
    let (tamp_src, tamp_path) = ingest_synthetic_paper(dir.path(), &env, sid_tamp);
    ledger.append_with_hash(sid_tamp, MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash(sid_tamp, "investigation.paper", 1, &json!({ "source_id": tamp_src })).unwrap();
    ledger
        .append_with_hash(
            sid_tamp,
            "investigation.map",
            1,
            &json!({
                "source_id": tamp_src, "title": "Tampered", "coverage": "paper", "limitations": [],
                "nodes": [{ "id": "t1", "sentence": "Result.", "kind": "result", "main": true, "anchors": [{ "source_id": tamp_src, "path": tamp_path, "start_line": 4, "end_line": 4, "kind": "passage" }] }],
                "edges": []
            }),
        )
        .unwrap();
    let disk_file = dir.path().join("sessions").join(sid_tamp).join("sources").join(&tamp_path);
    std::fs::write(&disk_file, b"tampered file contents").unwrap();
    let (st_tamp, v_tamp) = get_legacy(port, &tok, sid_tamp);
    assert_eq!(st_tamp, 200);
    assert_eq!(v_tamp["has_legacy"], json!(true));
    assert_eq!(v_tamp["fallback_old_view"], json!(true));
    assert!(v_tamp["diagnostic_note"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(v_tamp["unplaced"].as_array().unwrap().len(), 1);
    assert!(!v_tamp.to_string().to_ascii_lowercase().contains("verified"));
    let (mv_tamp_st, mv_tamp) = get_mapview(port, &tok, sid_tamp);
    assert_eq!(mv_tamp_st, 200);
    assert!(mv_tamp["skeleton_seq"].is_null());
    assert_eq!(mv_tamp["legacy"]["fallback_old_view"], json!(true));

    // Pure V2 session (pinned skeleton + notes, zero legacy records) -> has_legacy: false, mapview.legacy = null.
    let sid_v2 = "s-v9-pure-v2";
    let (v2_src, _) = ingest_synthetic_paper(dir.path(), &env, sid_v2);
    ledger.append_with_hash(sid_v2, MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    let (_, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid_v2}/investigation/skeleton"),
        &[("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")],
        serde_json::to_vec(&json!({ "source_id": v2_src, "pin": true })).unwrap().as_slice(),
    );
    let (st_v2, v_v2) = get_legacy(port, &tok, sid_v2);
    assert_eq!(st_v2, 200);
    assert_eq!(v_v2["has_legacy"], json!(false));
    assert_eq!(v_v2["fallback_old_view"], json!(false));
    let (mv_v2_st, mv_v2) = get_mapview(port, &tok, sid_v2);
    assert_eq!(mv_v2_st, 200);
    assert_eq!(mv_v2["legacy"], Value::Null);
    assert!(mv_v2["nodes"].as_array().unwrap().iter().all(|n| n.get("earlier_map").is_none()));

    srv.shutdown();
}
