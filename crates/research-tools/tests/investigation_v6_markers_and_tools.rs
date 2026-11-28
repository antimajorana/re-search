//! End-to-end integration tests for V6 Investigate markers, `read_map_node`, `record_map_reading`,
//! `map_fold`, and `investigation/1` vs `investigation/2` vs `research/1` tool-profile compatibility.

use ledger::Ledger;
use research_tools::investigation::{
    RECORD_PAPER_BRANCH, RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP, RECORD_PAPER_SUMMARY,
};
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON,
    INVESTIGATION_REQUEST_MARKERS, MARKER_BRANCH_REQUESTED, MARKER_CONVENTIONS_RESUMED,
    MARKER_MAPPING_REQUESTED, MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED,
};
use research_tools::map_defs::{
    read_map_node_def, record_map_reading_def, ReadingRole, READ_MAP_NODE, RECORD_MAP_READING,
};
use research_tools::map_fold::fold_map;
use research_tools::paper_key::PaperKey;
use research_tools::skeleton::{build_tex_skeleton_from_ingested, SkNodeKind};
use research_tools::tool_profile::{
    investigation_v1, investigation_v2, register_profile, research_v1, ProfileHostTools,
    MARKER_INVESTIGATION_STARTED,
};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::{
    ApprovalReply, ApprovalRequest, Approver, CallCtx, CallOutcome, Registry, Router, ToolHandler,
    WorkerEpoch,
};

struct NoEpoch;
impl WorkerEpoch for NoEpoch {
    fn live_epoch(&self) -> Option<u64> { None }
    fn is_contained(&self) -> bool { false }
}

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "v6-test".into() }
    }
}

const SAMPLE_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\begin{abstract}
We establish an energy decay estimate for damped linear waves on bounded domains.
\end{abstract}
\section{Framework}\label{sec:fw}
\begin{lemma}[Coercivity]\label{lem:coer}
For every smooth displacement $u$, the quadratic energy satisfies $E(u) \ge c \|u\|^2$.
\end{lemma}
\begin{theorem}[Exponential Decay]\label{thm:decay}
Under Lemma~\ref{lem:coer}, every solution satisfies $E(u(t)) \le C e^{-\gamma t} E(u(0))$.
\end{theorem}
\begin{proof}
Differentiating the energy along trajectories and applying Lemma~\ref{lem:coer} gives exponential decay.
\end{proof}
\section{Conclusion}\label{sec:conc}
The coercivity bound controls long-time damping.
\begin{thebibliography}{9}
\bibitem{Toy20} Toy Author, Energy estimates, 2020.
\end{thebibliography}
\end{document}
"#;

fn build_v2_router(data_dir: &std::path::Path, env: Arc<ResearchEnv>, sid: &str) -> Router {
    let profile = investigation_v2();
    let mut reg = Registry::new();
    let noop: Arc<dyn ToolHandler> = Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({})));
    let pol = register_profile(
        &profile,
        &mut reg,
        env,
        ProfileHostTools {
            sympy_epoch: Arc::new(NoEpoch),
            sympy_handler: noop,
            session_title_handler: None,
            solve_check: None,
        },
    )
    .unwrap();
    let ledger = Ledger::open(data_dir).unwrap();
    Router::new(reg, pol, ledger, sid, Arc::new(ApproveAll)).with_tool_profile(profile.name)
}

fn setup_pinned_skeleton(
    dir: &std::path::Path,
    sid: &str,
) -> (Arc<ResearchEnv>, String, String, u64, research_tools::skeleton::PaperSkeleton) {
    let env = Arc::new(ResearchEnv::new(dir));
    let ledger = Mutex::new(Ledger::open(dir).unwrap());
    ledger.lock().unwrap().append(sid, MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "decay.tex",
            origin: "https://arxiv.org/e-print/2405.00042v1",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Energy decay"),
        },
    )
    .unwrap();
    let primary_path = ing.primary_path.clone().unwrap();
    let receipt = ledger
        .lock()
        .unwrap()
        .read_session(sid)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "source.ingested")
        .unwrap();
    let (out, _) = build_tex_skeleton_from_ingested(
        &env.sandbox(),
        sid,
        &receipt,
        PaperKey::from_arxiv("2405.00042").unwrap(),
        Some("v1".into()),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&out.skeleton).unwrap();
    let blob = ledger.lock().unwrap().put_blob(&bytes).unwrap();
    ledger
        .lock()
        .unwrap()
        .append(sid, "investigation.paper", 1, &json!({ "source_id": ing.source_id }))
        .unwrap();
    let skel_seq = ledger
        .lock()
        .unwrap()
        .append(
            sid,
            EVENT_INVESTIGATION_SKELETON,
            1,
            &json!({
                "source_id": ing.source_id,
                "source_seq": ing.event_seq,
                "builder": "tex-1",
                "paper_key": "arxiv:2405.00042",
                "blob": blob,
            }),
        )
        .unwrap();
    (env, ing.source_id, primary_path, skel_seq, out.skeleton)
}

#[test]
fn router_bidirectional_marker_gate_closes_old_during_v6_and_new_during_legacy() {
    let tmp = tempfile::tempdir().unwrap();
    let sid = "s-v6-gate";
    let (env, source_id, primary_path, skel_seq, skeleton) = setup_pinned_skeleton(tmp.path(), sid);
    let router = build_v2_router(tmp.path(), Arc::clone(&env), sid);
    let ledger = Ledger::open(tmp.path()).unwrap();
    let caller = "agent:anthropic:s-v6-gate";

    let thm = skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let thm_tag = thm.tag.as_deref().unwrap();
    let thm_span = &thm.spans[0];

    // Direction 1: active V6 `task_requested` closes ALL four legacy recorders.
    let task_seq = ledger
        .append(
            sid,
            MARKER_TASK_REQUESTED,
            1,
            &json!({ "task": "orient", "tags": [], "source_id": source_id, "skeleton_seq": skel_seq }),
        )
        .unwrap();

    for old_tool in [
        RECORD_PAPER_MAP,
        RECORD_PAPER_SUMMARY,
        RECORD_PAPER_BRANCH,
        RECORD_PAPER_CONVENTIONS,
    ] {
        let out = router
            .try_call(caller, old_tool, &json!({ "source_id": source_id }))
            .unwrap();
        assert!(
            matches!(out, CallOutcome::Error(_)),
            "old recorder `{old_tool}` must be closed during active V6 task: {out:?}"
        );
    }

    // Read the theorem via `read_map_node`, then record via `record_map_reading` — MUST succeed!
    let read_out = router
        .try_call(
            caller,
            READ_MAP_NODE,
            &json!({ "source_id": source_id, "node": "Theorem 1.2" }),
        )
        .unwrap();
    assert!(matches!(read_out, CallOutcome::Ok(_)), "read_map_node failed: {read_out:?}");

    let rec_args = json!({
        "request_seq": task_seq,
        "source_id": source_id,
        "items": [{
            "tag": thm_tag,
            "role": "main_result",
            "gist": "Exponential energy decay follows from coercivity.",
            "anchors": [{
                "source_id": source_id,
                "path": primary_path,
                "start_line": thm_span.start_line,
                "end_line": thm_span.end_line,
                "quote": "every solution satisfies $E(u(t)) \\le C e^{-\\gamma t} E(u(0))$"
            }]
        }],
        "edges": [],
        "limitations": [],
        "done": true
    });
    let rec_out = router.try_call(caller, RECORD_MAP_READING, &rec_args).unwrap();
    match rec_out {
        CallOutcome::Ok(val) => {
            assert_eq!(val["status"], "recorded", "{val}");
            assert_eq!(val["accepted_items"], 1);
        }
        other => panic!("expected Ok from record_map_reading, got {other:?}"),
    }

    // Wrong request_seq rejected even while task_requested is active.
    let mut bad_seq_args = rec_args.clone();
    bad_seq_args["request_seq"] = json!(task_seq + 99);
    assert!(matches!(
        router.try_call(caller, RECORD_MAP_READING, &bad_seq_args).unwrap(),
        CallOutcome::Error(_)
    ));

    // Direction 2: closing with task_finished or any legacy marker closes `record_map_reading`.
    for &marker in INVESTIGATION_REQUEST_MARKERS {
        if marker == MARKER_TASK_REQUESTED {
            continue;
        }
        let payload = match marker {
            MARKER_TASK_FINISHED => json!({ "request_seq": task_seq, "status": "done" }),
            MARKER_MAPPING_REQUESTED | MARKER_BRANCH_REQUESTED => json!({ "source_id": source_id }),
            MARKER_CONVENTIONS_RESUMED => json!({ "for_request_seq": 1, "source_id": source_id }),
            _ => json!({}),
        };
        ledger.append(sid, marker, 1, &payload).unwrap();
        let closed = router.try_call(caller, RECORD_MAP_READING, &rec_args).unwrap();
        assert!(
            matches!(closed, CallOutcome::Error(_)),
            "record_map_reading must be closed after `{marker}`, got {closed:?}"
        );
    }
}

#[test]
fn end_to_end_read_record_rerun_and_fold_preserves_history_and_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let sid = "s-v6-e2e";
    let (env, source_id, primary_path, skel_seq, skeleton) = setup_pinned_skeleton(tmp.path(), sid);
    let router = build_v2_router(tmp.path(), Arc::clone(&env), sid);
    let ledger = Ledger::open(tmp.path()).unwrap();
    let caller = "agent:anthropic:s-v6-e2e";

    let lem = skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).unwrap();
    let thm = skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let proof = skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Proof).unwrap();
    let lem_tag = lem.tag.as_deref().unwrap();
    let thm_tag = thm.tag.as_deref().unwrap();

    // Run 1: orient task reads theorem (which also returns proof!) and saves valid + invalid siblings.
    let seq1 = ledger
        .append(
            sid,
            MARKER_TASK_REQUESTED,
            1,
            &json!({ "task": "orient", "tags": [], "source_id": source_id, "skeleton_seq": skel_seq }),
        )
        .unwrap();
    assert!(matches!(
        router.try_call(caller, READ_MAP_NODE, &json!({ "source_id": source_id, "node": thm_tag })).unwrap(),
        CallOutcome::Ok(_)
    ));
    let call1 = router
        .try_call(
            caller,
            RECORD_MAP_READING,
            &json!({
                "request_seq": seq1,
                "source_id": source_id,
                "items": [
                    {
                        "tag": thm_tag,
                        "role": "main_result",
                        "gist": "Run 1 gist: exponential decay bound.",
                        "anchors": [{
                            "source_id": source_id,
                            "path": primary_path,
                            "start_line": thm.spans[0].start_line,
                            "end_line": thm.spans[0].end_line,
                            "quote": "every solution satisfies $E(u(t)) \\le C e^{-\\gamma t} E(u(0))$"
                        }]
                    },
                    {
                        "tag": proof.tag.as_deref().unwrap(),
                        "role": "main_result",
                        "gist": "Invalid role on proof node.",
                        "anchors": [{
                            "source_id": source_id,
                            "path": primary_path,
                            "start_line": proof.spans[0].start_line,
                            "end_line": proof.spans[0].end_line,
                            "quote": "Differentiating the energy along trajectories"
                        }]
                    }
                ],
                "edges": [
                    {
                        "from": lem_tag,
                        "to": thm_tag,
                        "role": "proof",
                        "basis": "explicit",
                        "reason": "Proof invokes Lemma 1.1.",
                        "anchor": {
                            "source_id": source_id,
                            "path": primary_path,
                            "start_line": proof.spans[0].start_line,
                            "end_line": proof.spans[0].end_line,
                            "quote": "applying Lemma~\\ref{lem:coer} gives exponential decay"
                        }
                    },
                    {
                        "from": lem_tag,
                        "to": thm_tag,
                        "role": "proof",
                        "basis": "explicit",
                        "reason": "Anchor outside proof must fail.",
                        "anchor": {
                            "source_id": source_id,
                            "path": primary_path,
                            "start_line": thm.spans[0].start_line,
                            "end_line": thm.spans[0].end_line,
                            "quote": "Under Lemma~\\ref{lem:coer}"
                        }
                    }
                ],
                "limitations": [],
                "done": true
            }),
        )
        .unwrap();
    let val1 = match call1 { CallOutcome::Ok(v) => v, other => panic!("{other:?}") };
    assert_eq!(val1["accepted_items"], 1);
    assert_eq!(val1["accepted_edges"], 1);
    assert_eq!(val1["rejected_items"][0]["index"], 1);
    assert_eq!(val1["rejected_items"][0]["code"], "invalid_role_for_kind");
    assert_eq!(val1["rejected_edges"][0]["index"], 1);
    assert_eq!(val1["rejected_edges"][0]["code"], "scope_mismatch");

    ledger.append(sid, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": seq1, "status": "done" })).unwrap();
    // Reader adds a note on the theorem.
    ledger
        .append(
            sid,
            EVENT_INVESTIGATION_NOTE,
            1,
            &json!({ "tag": thm_tag, "node_fp": thm.fp, "text": "Key decay rate gamma." }),
        )
        .unwrap();

    // Run 2: orient re-run updates gist without losing run-1 history or reader note (rule 6.12).
    let seq2 = ledger
        .append(
            sid,
            MARKER_TASK_REQUESTED,
            1,
            &json!({ "task": "orient", "tags": [], "source_id": source_id, "skeleton_seq": skel_seq }),
        )
        .unwrap();
    assert!(matches!(
        router.try_call(caller, READ_MAP_NODE, &json!({ "source_id": source_id, "node": thm_tag })).unwrap(),
        CallOutcome::Ok(_)
    ));
    let call2 = router
        .try_call(
            caller,
            RECORD_MAP_READING,
            &json!({
                "request_seq": seq2,
                "source_id": source_id,
                "items": [{
                    "tag": thm_tag,
                    "role": "main_result",
                    "gist": "Run 2 gist: refined exponential energy decay statement.",
                    "anchors": [{
                        "source_id": source_id,
                        "path": primary_path,
                        "start_line": thm.spans[0].start_line,
                        "end_line": thm.spans[0].end_line,
                        "quote": "every solution satisfies $E(u(t)) \\le C e^{-\\gamma t} E(u(0))$"
                    }]
                }],
                "edges": [],
                "limitations": [],
                "done": true
            }),
        )
        .unwrap();
    assert!(matches!(call2, CallOutcome::Ok(_)));
    ledger.append(sid, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": seq2, "status": "done" })).unwrap();

    let events = ledger.read_session(sid).unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == EVENT_INVESTIGATION_READING).count(), 2);

    let folded = fold_map(&skeleton, &events, false);
    let node_fold = folded.nodes.get(thm_tag).expect("theorem folded");
    assert_eq!(node_fold.role.current.as_ref().unwrap().value, ReadingRole::MainResult);
    assert_eq!(
        node_fold.gist.current.as_ref().unwrap().value,
        "Run 2 gist: refined exponential energy decay statement."
    );
    assert_eq!(node_fold.gist.history.len(), 1);
    assert_eq!(node_fold.gist.history[0].value, "Run 1 gist: exponential decay bound.");
    assert_eq!(node_fold.notes.len(), 1);
    assert_eq!(node_fold.notes[0].text, "Key decay rate gamma.");
    assert_eq!(folded.edges.len(), 1);
    assert_eq!(folded.tasks.len(), 2);
}

#[test]
fn investigation_v1_may_gain_v6_tools_while_research_v1_forbids_them() {
    let r1 = research_v1();
    let i1 = investigation_v1();
    let i2 = investigation_v2();
    let cc = research_tools::map_context_defs::record_context_cards_def();

    assert_eq!(i2.defs.len(), r1.defs.len() + 3);
    assert!(i1.allows_gain(&read_map_node_def()));
    assert!(i1.allows_gain(&record_map_reading_def()));
    assert!(i1.allows_gain(&cc));
    assert!(!r1.allows_gain(&read_map_node_def()));
    assert!(!r1.allows_gain(&record_map_reading_def()));
    assert!(!r1.allows_gain(&cc));
}
