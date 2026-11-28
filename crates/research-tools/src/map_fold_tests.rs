use super::*;
use crate::investigation_markers::{
    MARKER_SESSION_ABORT, MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED,
};
use crate::map_defs::{
    AuthorsClaimLabel, AuthorsClaimRecord, IndexedFault, MapReadingEventPayload, ModelEdgeBasis,
    ModelEdgeRecord, ModelEdgeRole, ReadingItemRecord, ReadingRejected, ReadingRole,
    VerifiedMapAnchor,
};
use crate::paper_key::PaperKey;
use crate::run_budget::{InvestigateTaskState, RunState, RunStateAction};
use crate::skeleton::{build_tex_skeleton, TexSkeletonInput};
use crate::tool_profile::MARKER_INVESTIGATION_STARTED;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const TEX: &str = r#"\documentclass{article}
\newtheorem{theorem}{Theorem}
\newtheorem{lemma}{Lemma}
\begin{document}
\section{Main}
\begin{lemma}\label{lem:a}
Every finite sum of bounded operators is bounded.
\end{lemma}
\begin{theorem}\label{thm:b}
By Lemma~\ref{lem:a}, the iterated flow converges uniformly.
\end{theorem}
\end{document}
"#;

fn make_skeleton() -> PaperSkeleton {
    let mut files = BTreeMap::new();
    files.insert("src1/paper.tex".to_string(), TEX.to_string());
    build_tex_skeleton(&TexSkeletonInput {
        source_id: "src1".into(),
        source_seq: 1,
        paper_key: PaperKey::parse("arxiv:2501.00002").unwrap(),
        version: Some("v1".into()),
        files,
        primary_path: Some("src1/paper.tex".into()),
        files_sha256: None,
    })
    .skeleton
}

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-fold".into(),
        seq,
        ts_ms: seq as i64 * 1000,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: String::new(),
    }
}

fn sample_anchor(line: usize, quote: &str) -> VerifiedMapAnchor {
    VerifiedMapAnchor {
        source_id: "src1".into(),
        path: "src1/paper.tex".into(),
        start_line: line,
        end_line: line,
        file_sha256: "0".repeat(64),
        quote_sha256: "1".repeat(64),
        quote: quote.into(),
        excerpt: quote.into(),
        source_seq: 1,
        cited_range: None,
        range_basis: Vec::new(),
        excerpt_window: None,
    }
}

fn tag_and_fp(skel: &PaperSkeleton, label: &str) -> (String, String) {
    let node = skel
        .nodes
        .iter()
        .find(|n| n.label.as_deref() == Some(label))
        .unwrap();
    (node.tag.clone().unwrap(), node.fp.clone())
}

#[test]
fn orient_run_twice_keeps_history_notes_pins_and_untouched_fields() {
    let skel = make_skeleton();
    let (lem_tag, lem_fp) = tag_and_fp(&skel, "lem:a");
    let (thm_tag, thm_fp) = tag_and_fp(&skel, "thm:b");

    let r1 = MapReadingEventPayload {
        schema_version: 1,
        request_seq: 3,
        source_id: "src1".into(),
        skeleton_seq: 2,
        done: true,
        items: vec![ReadingItemRecord {
            tag: thm_tag.clone(),
            node_fp: thm_fp.clone(),
            attach_to: None,
            model_kind: None,
            role: Some(ReadingRole::Supporting),
            gist: Some("First orient gist.".into()),
            purpose: Some("Original purpose stays across re-runs.".into()),
            hypotheses: Vec::new(),
            anchors: vec![sample_anchor(10, "iterated flow converges uniformly")],
            authors_claim: Some(AuthorsClaimRecord {
                label: AuthorsClaimLabel::New,
                relation_note: None,
                attributed_to: Vec::new(),
                anchors: vec![sample_anchor(10, "iterated flow converges uniformly")],
                cues: Vec::new(),
            }),
        }],
        edges: vec![ModelEdgeRecord {
            from: lem_tag.clone(),
            to: thm_tag.clone(),
            role: ModelEdgeRole::Statement,
            basis: ModelEdgeBasis::Explicit,
            reason: "Run 1 reason".into(),
            anchor: sample_anchor(10, "iterated flow converges uniformly"),
        }],
        limitations: vec!["Run 1 limitation".into()],
        rejected: ReadingRejected {
            items: vec![IndexedFault {
                index: 1,
                code: "unknown_tag".into(),
                message: "bad".into(),
                true_text: None,
            }],
            edges: Vec::new(),
        },
        anchor_report: json!({ "dropped": [], "corrected": [] }),
    };

    let r2 = MapReadingEventPayload {
        schema_version: 1,
        request_seq: 8,
        source_id: "src1".into(),
        skeleton_seq: 2,
        done: true,
        items: vec![ReadingItemRecord {
            tag: thm_tag.clone(),
            node_fp: thm_fp.clone(),
            attach_to: None,
            model_kind: None,
            role: Some(ReadingRole::MainResult),
            gist: Some("Second orient gist.".into()),
            purpose: None,
            hypotheses: Vec::new(),
            anchors: vec![sample_anchor(10, "iterated flow converges uniformly")],
            authors_claim: None,
        }],
        edges: vec![ModelEdgeRecord {
            from: lem_tag.clone(),
            to: thm_tag.clone(),
            role: ModelEdgeRole::Statement,
            basis: ModelEdgeBasis::Explicit,
            reason: "Run 2 refined reason".into(),
            anchor: sample_anchor(10, "iterated flow converges uniformly"),
        }],
        limitations: vec!["Run 2 limitation".into()],
        rejected: ReadingRejected::default(),
        anchor_report: json!({ "dropped": [], "corrected": [] }),
    };

    let events = vec![
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(2, EVENT_INVESTIGATION_SKELETON, json!({ "source_id": "src1" })),
        ev(3, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2 })),
        ev(4, "model.request", json!({})),
        ev(5, EVENT_INVESTIGATION_READING, serde_json::to_value(&r1).unwrap()),
        ev(6, MARKER_TASK_FINISHED, json!({ "request_seq": 3, "status": "done" })),
        ev(7, EVENT_INVESTIGATION_NOTE, json!({ "tag": thm_tag, "node_fp": thm_fp, "text": "Reader note 1", "pin": "main_result" })),
        ev(8, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2 })),
        ev(9, "model.request", json!({})),
        ev(10, EVENT_INVESTIGATION_READING, serde_json::to_value(&r2).unwrap()),
        ev(11, MARKER_TASK_FINISHED, json!({ "request_seq": 8, "status": "done" })),
    ];

    let folded = fold_map(&skel, &events, false);
    assert_eq!(folded.skeleton_seq, Some(2));
    assert_eq!(folded.limitations, vec!["Run 1 limitation", "Run 2 limitation"]);

    let thm = &folded.nodes[&thm_tag];
    assert_eq!(thm.role.current.as_ref().unwrap().value, ReadingRole::MainResult);
    assert_eq!(thm.role.history.len(), 1);
    assert_eq!(thm.role.history[0].value, ReadingRole::Supporting);
    assert_eq!(thm.gist.current.as_ref().unwrap().value, "Second orient gist.");
    assert_eq!(thm.gist.history.len(), 1);
    assert_eq!(thm.gist.history[0].value, "First orient gist.");
    // Purpose and authors_claim from run 1 are untouched by run 2.
    assert_eq!(thm.purpose.current.as_ref().unwrap().value, "Original purpose stays across re-runs.");
    assert!(thm.authors_claim.current.is_some());
    // Reader note and pin from between runs are preserved.
    assert_eq!(thm.notes.len(), 1);
    assert_eq!(thm.notes[0].text, "Reader note 1");
    assert_eq!(thm.pin, Some(ReaderPin::MainResult));
    assert!(thm.effective_is_main_result);
    assert!(!thm.any_stale);
    // Edge history preserved.
    assert_eq!(folded.edges.len(), 1);
    assert_eq!(folded.edges[0].current.value.reason, "Run 2 refined reason");
    assert_eq!(folded.edges[0].history.len(), 1);
    assert_eq!(folded.edges[0].history[0].value.reason, "Run 1 reason");
    // Ensure unused lem_fp doesn't warn.
    assert!(!lem_fp.is_empty());
    // Task counts recorded accurately.
    assert_eq!(folded.tasks.len(), 2);
    assert_eq!(folded.tasks[0].saved_items, 1);
    assert_eq!(folded.tasks[0].saved_edges, 1);
    assert_eq!(folded.tasks[0].rejected_items, 1);
}

#[test]
fn reader_pin_overrides_role_and_stale_node_fp_flagged() {
    let skel = make_skeleton();
    let (lem_tag, lem_fp) = tag_and_fp(&skel, "lem:a");
    let (thm_tag, thm_fp) = tag_and_fp(&skel, "thm:b");

    let reading = MapReadingEventPayload {
        schema_version: 1,
        request_seq: 3,
        source_id: "src1".into(),
        skeleton_seq: 2,
        done: true,
        items: vec![
            ReadingItemRecord {
                tag: lem_tag.clone(),
                node_fp: lem_fp.clone(),
                attach_to: None,
                model_kind: None,
                role: Some(ReadingRole::Supporting),
                gist: Some("Lemma gist.".into()),
                purpose: None,
                hypotheses: Vec::new(),
                anchors: vec![sample_anchor(7, "finite sum of bounded operators")],
                authors_claim: None,
            },
            ReadingItemRecord {
                tag: thm_tag.clone(),
                node_fp: "deadbeef0000".into(), // stale fingerprint!
                attach_to: None,
                model_kind: None,
                role: Some(ReadingRole::MainResult),
                gist: Some("Stale theorem gist.".into()),
                purpose: None,
                hypotheses: Vec::new(),
                anchors: vec![sample_anchor(10, "iterated flow converges uniformly")],
                authors_claim: None,
            },
        ],
        edges: Vec::new(),
        limitations: Vec::new(),
        rejected: ReadingRejected::default(),
        anchor_report: json!({ "dropped": [], "corrected": [] }),
    };

    let events = vec![
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(2, EVENT_INVESTIGATION_SKELETON, json!({ "source_id": "src1" })),
        ev(3, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2 })),
        ev(4, EVENT_INVESTIGATION_READING, serde_json::to_value(&reading).unwrap()),
        ev(5, MARKER_TASK_FINISHED, json!({ "request_seq": 3, "status": "done" })),
        // Reader pins Supporting lemma as MainResult, then adds a text-only note with pin: null.
        ev(6, EVENT_INVESTIGATION_NOTE, json!({ "tag": lem_tag, "node_fp": lem_fp, "pin": "main_result" })),
        ev(7, EVENT_INVESTIGATION_NOTE, json!({ "tag": lem_tag, "node_fp": lem_fp, "text": "Keep pin!", "pin": null })),
        // Reader note with stale fp on thm_tag, no pin -> stale model MainResult loses main status.
        ev(8, EVENT_INVESTIGATION_NOTE, json!({ "tag": thm_tag, "node_fp": "000000000000", "text": "Old note" })),
    ];

    let folded = fold_map(&skel, &events, false);
    let lem = &folded.nodes[&lem_tag];
    assert_eq!(lem.pin, Some(ReaderPin::MainResult));
    assert!(lem.effective_is_main_result);
    assert!(!lem.any_stale);
    assert_eq!(lem.notes.len(), 1);

    let thm = &folded.nodes[&thm_tag];
    assert!(thm.role.current.as_ref().unwrap().stale);
    assert!(thm.any_stale);
    assert!(thm.notes[0].stale);
    assert!(!thm.effective_is_main_result);

    // Reader explicitly pinning NotMainResult overrides a fresh model MainResult.
    let fresh_thm = MapReadingEventPayload {
        items: vec![ReadingItemRecord {
            tag: thm_tag.clone(),
            node_fp: thm_fp.clone(),
            attach_to: None,
            model_kind: None,
            role: Some(ReadingRole::MainResult),
            gist: None,
            purpose: None,
            hypotheses: Vec::new(),
            anchors: vec![sample_anchor(10, "iterated flow converges uniformly")],
            authors_claim: None,
        }],
        ..reading
    };
    let events2 = vec![
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(2, EVENT_INVESTIGATION_SKELETON, json!({ "source_id": "src1" })),
        ev(3, EVENT_INVESTIGATION_READING, serde_json::to_value(&fresh_thm).unwrap()),
        ev(4, EVENT_INVESTIGATION_NOTE, json!({ "tag": thm_tag, "node_fp": thm_fp, "pin": "not_main_result" })),
    ];
    let folded2 = fold_map(&skel, &events2, false);
    let thm2 = &folded2.nodes[&thm_tag];
    assert_eq!(thm2.pin, Some(ReaderPin::NotMainResult));
    assert!(!thm2.effective_is_main_result);
    assert!(!thm2.any_stale);
}

#[test]
fn task_fold_uses_f9_model_calls_and_maps_all_run_states() {
    let skel = make_skeleton();
    let (thm_tag, _) = tag_and_fp(&skel, "thm:b");

    // Sequence of finished tasks covering done, partial, stopped, approval, interrupted, failed,
    // plus an active/unfinished task at the end.
    let cases = [
        ("done", InvestigateTaskState::Done, RunState::Done, None),
        ("partial", InvestigateTaskState::Partial, RunState::Paused, Some(RunStateAction::Continue)),
        ("stopped", InvestigateTaskState::Stopped, RunState::Paused, Some(RunStateAction::Continue)),
        ("approval", InvestigateTaskState::Approval, RunState::WaitingForYou, None),
        ("interrupted", InvestigateTaskState::Interrupted, RunState::Interrupted, Some(RunStateAction::Retry)),
        ("failed", InvestigateTaskState::Failed, RunState::Failed, Some(RunStateAction::Retry)),
    ];

    let mut events = vec![
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(2, EVENT_INVESTIGATION_SKELETON, json!({ "source_id": "src1" })),
    ];
    let mut seq = 3u64;
    for (idx, &(status_str, _, _, _)) in cases.iter().enumerate() {
        let req = seq;
        events.push(ev(
            req,
            MARKER_TASK_REQUESTED,
            json!({
                "task": if idx % 2 == 0 { "orient" } else { "open" },
                "tags": [thm_tag],
                "source_id": "src1",
                "skeleton_seq": 2
            }),
        ));
        seq += 1;
        // Emit (idx + 1) model calls inside this task's range.
        for _ in 0..=idx {
            events.push(ev(seq, "model.request", json!({})));
            seq += 1;
        }
        let mut fin_payload = json!({ "request_seq": req, "status": status_str });
        if status_str == "failed" {
            fin_payload["error"] = json!("provider timeout");
        }
        events.push(ev(seq, MARKER_TASK_FINISHED, fin_payload));
        seq += 1;
    }

    // Add one unfinished open task with 2 model calls after it.
    let active_req = seq;
    events.push(ev(
        active_req,
        MARKER_TASK_REQUESTED,
        json!({ "task": "open", "tags": [thm_tag], "source_id": "src1", "skeleton_seq": 2 }),
    ));
    seq += 1;
    events.push(ev(seq, "model.request", json!({})));
    seq += 1;
    events.push(ev(seq, "verifier.request", json!({})));
    seq += 1;

    // With running = true, latest unfinished task is Requested -> Running.
    let folded_running = fold_map(&skel, &events, true);
    assert_eq!(folded_running.tasks.len(), cases.len() + 1);
    for (idx, &(_, exp_stored, exp_run, exp_action)) in cases.iter().enumerate() {
        let t = &folded_running.tasks[idx];
        assert_eq!(t.stored_status, exp_stored);
        assert_eq!(t.run_state, exp_run);
        assert_eq!(t.run_state_label, exp_run.label());
        assert_eq!(t.run_state_action, exp_action);
        assert_eq!(t.model_calls.used, (idx + 1) as u64);
        assert_eq!(t.model_calls.cap, if idx % 2 == 0 { 8 } else { 10 });
    }
    assert_eq!(folded_running.tasks[5].error.as_deref(), Some("provider timeout"));
    let active = folded_running.tasks.last().unwrap();
    assert_eq!(active.stored_status, InvestigateTaskState::Requested);
    assert_eq!(active.run_state, RunState::Running);
    assert_eq!(active.run_state_action, None);
    assert_eq!(active.model_calls.used, 2);
    assert_eq!(active.model_calls.cap, 10);

    // With running = false, unfinished task becomes Interrupted -> Retry.
    let folded_idle = fold_map(&skel, &events, false);
    let interrupted = folded_idle.tasks.last().unwrap();
    assert_eq!(interrupted.stored_status, InvestigateTaskState::Interrupted);
    assert_eq!(interrupted.run_state, RunState::Interrupted);
    assert_eq!(interrupted.run_state_action, Some(RunStateAction::Retry));

    // Even with running = true, a later session.abort marks the unfinished task Interrupted.
    events.push(ev(seq, MARKER_SESSION_ABORT, json!({})));
    let folded_aborted = fold_map(&skel, &events, true);
    let aborted = folded_aborted.tasks.last().unwrap();
    assert_eq!(aborted.stored_status, InvestigateTaskState::Interrupted);
    assert_eq!(aborted.run_state, RunState::Interrupted);
}

fn reading_item(tag: &str, fp: &str, line: usize, quote: &str) -> ReadingItemRecord {
    ReadingItemRecord {
        tag: tag.into(),
        node_fp: fp.into(),
        attach_to: None,
        model_kind: None,
        role: Some(ReadingRole::Supporting),
        gist: Some(format!("Gist for {tag}.")),
        purpose: None,
        hypotheses: Vec::new(),
        anchors: vec![sample_anchor(line, quote)],
        authors_claim: None,
    }
}

fn reading_payload(
    request_seq: u64,
    items: Vec<ReadingItemRecord>,
    rejected_items: Vec<IndexedFault>,
    done: bool,
) -> Value {
    serde_json::to_value(MapReadingEventPayload {
        schema_version: 1,
        request_seq,
        source_id: "src1".into(),
        skeleton_seq: 2,
        done,
        items,
        edges: Vec::new(),
        limitations: Vec::new(),
        rejected: ReadingRejected { items: rejected_items, edges: Vec::new() },
        anchor_report: json!({ "dropped": [], "corrected": [] }),
    })
    .unwrap()
}

#[test]
fn unresolved_rejected_items_tracks_retries_within_and_across_request_seq() {
    let skel = make_skeleton();
    let (lem_tag, lem_fp) = tag_and_fp(&skel, "lem:a");
    let (thm_tag, thm_fp) = tag_and_fp(&skel, "thm:b");

    // Task 1 (req_seq=3): thm_tag rejected via anchor_outside_node (message tag) AND lem_tag rejected
    // via quote_not_found (attributed from tool.request inline args); then BOTH retried & accepted in req_seq=3.
    let tool_req_args = json!({
        "request_seq": 3, "source_id": "src1", "done": false, "edges": [], "limitations": [],
        "items": [{ "tag": thm_tag }, { "tag": lem_tag }]
    })
    .to_string();
    let t1_first = reading_payload(
        3,
        vec![],
        vec![
            IndexedFault {
                index: 0,
                code: "anchor_outside_node".into(),
                message: format!("No matched anchor lies inside node `{thm_tag}` spans (src1/paper.tex:9-11)."),
                true_text: None,
            },
            IndexedFault {
                index: 1,
                code: "quote_not_found".into(),
                message: "Quote did not match lines 6-8.".into(),
                true_text: Some("Every finite sum...".into()),
            },
        ],
        false,
    );
    let t1_retry_thm = reading_payload(3, vec![reading_item(&thm_tag, &thm_fp, 10, "iterated flow")], vec![], false);
    let t1_retry_lem = reading_payload(3, vec![reading_item(&lem_tag, &lem_fp, 7, "finite sum")], vec![], true);

    // Task 2 (req_seq=10): thm_tag rejected and NOT accepted in req_seq=10, then accepted only in Task 3 (req_seq=15).
    let t2_rej = reading_payload(
        10,
        vec![reading_item(&lem_tag, &lem_fp, 7, "finite sum")],
        vec![IndexedFault {
            index: 1,
            code: "anchor_outside_node".into(),
            message: format!("No matched anchor lies inside node `{thm_tag}` spans (src1/paper.tex:9-11)."),
            true_text: None,
        }],
        true,
    );
    let t3_accept = reading_payload(15, vec![reading_item(&thm_tag, &thm_fp, 10, "iterated flow")], vec![], true);

    // Step 1: midway through Task 1 after retrying only thm_tag -> 2 raw rejected, 1 unresolved (lem_tag).
    let mut events = vec![
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(2, EVENT_INVESTIGATION_SKELETON, json!({ "source_id": "src1" })),
        ev(3, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2 })),
        ev(4, "tool.request", json!({ "call_id": "c1", "tool": "record_map_reading", "args": { "inline": tool_req_args } })),
        ev(5, EVENT_INVESTIGATION_READING, t1_first),
        ev(6, EVENT_INVESTIGATION_READING, t1_retry_thm),
    ];
    let mid = fold_map(&skel, &events, true);
    assert_eq!(mid.tasks[0].rejected_items, 2);
    assert_eq!(mid.tasks[0].unresolved_rejected_items, 1);

    // Step 2: lem_tag accepted in same request_seq -> raw rejected_items stays 2, unresolved drops to 0.
    events.extend([
        ev(7, EVENT_INVESTIGATION_READING, t1_retry_lem),
        ev(8, MARKER_TASK_FINISHED, json!({ "request_seq": 3, "status": "done" })),
        ev(10, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2 })),
        ev(11, EVENT_INVESTIGATION_READING, t2_rej),
        ev(12, MARKER_TASK_FINISHED, json!({ "request_seq": 10, "status": "done" })),
        ev(15, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2 })),
        ev(16, EVENT_INVESTIGATION_READING, t3_accept),
        ev(17, MARKER_TASK_FINISHED, json!({ "request_seq": 15, "status": "done" })),
    ]);

    let folded = fold_map(&skel, &events, false);
    assert_eq!(folded.tasks.len(), 3);
    assert_eq!(folded.tasks[0].rejected_items, 2);
    assert_eq!(folded.tasks[0].unresolved_rejected_items, 0);
    // Task 2's rejected thm_tag must NOT be cleared by Task 3's separate request_seq!
    assert_eq!(folded.tasks[1].rejected_items, 1);
    assert_eq!(folded.tasks[1].unresolved_rejected_items, 1);
    assert_eq!(folded.tasks[2].rejected_items, 0);
    assert_eq!(folded.tasks[2].unresolved_rejected_items, 0);

    // Verify clean JSON serialization for desktop-server and JS UI consumers.
    let task0_json = serde_json::to_value(&folded.tasks[0]).unwrap();
    assert_eq!(task0_json["rejected_items"], 2);
    assert_eq!(task0_json["unresolved_rejected_items"], 0);
}

#[test]
fn task_fold_prefers_cap_key_over_max_calls_and_call_cap() {
    let skel = make_skeleton();
    let events = vec![
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(2, EVENT_INVESTIGATION_SKELETON, json!({ "source_id": "src1" })),
        ev(3, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2, "cap": 14 })),
        ev(4, MARKER_TASK_FINISHED, json!({ "request_seq": 3, "status": "done" })),
        ev(5, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2, "cap": 15, "max_calls": 20, "call_cap": 25 })),
        ev(6, MARKER_TASK_FINISHED, json!({ "request_seq": 5, "status": "done" })),
        ev(7, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2, "cap": null, "max_calls": 20, "call_cap": 25 })),
        ev(8, MARKER_TASK_FINISHED, json!({ "request_seq": 7, "status": "done" })),
        ev(9, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "src1", "skeleton_seq": 2, "call_cap": 25 })),
        ev(10, MARKER_TASK_FINISHED, json!({ "request_seq": 9, "status": "done" })),
    ];

    let folded = fold_map(&skel, &events, false);
    assert_eq!(folded.tasks.len(), 4);
    assert_eq!(folded.tasks[0].model_calls.cap, 14);
    assert_eq!(folded.tasks[1].model_calls.cap, 15);
    assert_eq!(folded.tasks[2].model_calls.cap, 20);
    assert_eq!(folded.tasks[3].model_calls.cap, 25);
}

