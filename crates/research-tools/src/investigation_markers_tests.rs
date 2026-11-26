use super::*;
use serde_json::json;

fn ev(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-markers".into(),
        seq,
        ts_ms: seq as i64 * 1000,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: "0".repeat(64),
        event_hash: format!("{seq:064x}"),
    }
}

#[test]
fn marker_table_contains_all_legacy_v6_and_session_markers_without_duplicates() {
    let mut seen = std::collections::BTreeSet::new();
    for &kind in INVESTIGATION_REQUEST_MARKERS {
        assert!(seen.insert(kind), "duplicate marker kind `{kind}`");
        assert!(is_investigation_marker(kind));
    }
    assert_eq!(INVESTIGATION_REQUEST_MARKERS.len(), 14);
    assert!(is_investigation_marker(MARKER_TASK_REQUESTED));
    assert!(is_investigation_marker(MARKER_TASK_FINISHED));
    assert!(!is_investigation_marker("investigation.reading"));
    assert!(!is_investigation_marker("investigation.skeleton"));
    assert!(!is_investigation_marker("context.supplied"));
}

#[test]
fn table_all_four_old_and_three_new_recorders_against_every_marker_kind() {
    // Every marker kind in INVESTIGATION_REQUEST_MARKERS, expanding `task_requested` into all 4 tasks.
    let mut scenarios: Vec<(String, Event)> = Vec::new();
    for &kind in INVESTIGATION_REQUEST_MARKERS {
        if kind == MARKER_TASK_REQUESTED {
            for task in InvestigationTaskKind::ALL {
                scenarios.push((
                    format!("{kind}({})", task.as_str()),
                    ev(10, kind, json!({ "task": task.as_str(), "source_id": "src-1", "skeleton_seq": 3 })),
                ));
            }
        } else {
            scenarios.push((kind.to_string(), ev(10, kind, json!({ "source_id": "src-1" }))));
        }
    }

    for (label, marker_ev) in &scenarios {
        let history = [ev(1, MARKER_INVESTIGATION_STARTED, json!({})), marker_ev.clone()];

        // Check all 4 legacy recorders.
        for old in OldRecorderKind::ALL {
            let expected_ok = matches!(
                (old, marker_ev.kind.as_str()),
                (OldRecorderKind::Map | OldRecorderKind::Summary, MARKER_MAPPING_REQUESTED)
                    | (OldRecorderKind::Branch, MARKER_BRANCH_REQUESTED)
                    | (
                        OldRecorderKind::Conventions,
                        MARKER_MAPPING_REQUESTED | MARKER_CONVENTIONS_RESUMED
                    )
            );
            let actual_ok = check_old_recorder_gate(&history, old).is_ok();
            assert_eq!(
                actual_ok,
                expected_ok,
                "old recorder `{}` against marker `{label}`: expected {expected_ok}, got {actual_ok}",
                old.tool_id()
            );
        }

        // Check all 3 new recorders with matching request_seq = 10 and wrong request_seq = 99.
        for new_rec in NewRecorderKind::ALL {
            let expected_ok = if marker_ev.kind == MARKER_TASK_REQUESTED {
                let task = InvestigationTaskKind::from_str_exact(
                    marker_ev.payload["task"].as_str().unwrap(),
                )
                .unwrap();
                new_rec.accepts_task(task)
            } else {
                false
            };
            let actual_ok = check_new_recorder_gate(&history, new_rec, 10).is_ok();
            assert_eq!(
                actual_ok,
                expected_ok,
                "new recorder `{}` against marker `{label}` (matching seq): expected {expected_ok}, got {actual_ok}",
                new_rec.tool_id()
            );
            assert!(
                check_new_recorder_gate(&history, new_rec, 99).is_err(),
                "new recorder `{}` must reject wrong request_seq on `{label}`",
                new_rec.tool_id()
            );
        }
    }
}

#[test]
fn no_marker_closes_all_old_and_new_recorders() {
    let history = [ev(1, MARKER_INVESTIGATION_STARTED, json!({}))];
    for old in OldRecorderKind::ALL {
        assert!(check_old_recorder_gate(&history, old).is_err());
    }
    for new_rec in NewRecorderKind::ALL {
        assert!(check_new_recorder_gate(&history, new_rec, 1).is_err());
    }
}

#[test]
fn newer_marker_closes_previous_request_in_both_directions() {
    // Old mapping request followed by new task_requested closes old recorders and opens new MapReading.
    let hist_new_wins = [
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(5, MARKER_MAPPING_REQUESTED, json!({ "source_id": "s1" })),
        ev(9, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "s1", "skeleton_seq": 3 })),
    ];
    for old in OldRecorderKind::ALL {
        assert!(check_old_recorder_gate(&hist_new_wins, old).is_err());
    }
    assert!(check_new_recorder_gate(&hist_new_wins, NewRecorderKind::MapReading, 9).is_ok());

    // New task_requested followed by task_finished closes new recorders.
    let hist_finished = [
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(9, MARKER_TASK_REQUESTED, json!({ "task": "orient", "source_id": "s1", "skeleton_seq": 3 })),
        ev(12, MARKER_TASK_FINISHED, json!({ "request_seq": 9, "status": "done" })),
    ];
    assert!(check_new_recorder_gate(&hist_finished, NewRecorderKind::MapReading, 9).is_err());

    // New task_requested followed by old mapping_requested closes new recorders and opens old Map.
    let hist_old_wins = [
        ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
        ev(9, MARKER_TASK_REQUESTED, json!({ "task": "open", "source_id": "s1", "skeleton_seq": 3 })),
        ev(14, MARKER_MAPPING_REQUESTED, json!({ "source_id": "s1" })),
    ];
    assert!(check_new_recorder_gate(&hist_old_wins, NewRecorderKind::MapReading, 9).is_err());
    assert!(check_old_recorder_gate(&hist_old_wins, OldRecorderKind::Map).is_ok());
}

#[test]
fn active_investigation_slice_respects_all_mode_markers() {
    assert!(active_investigation_slice(&[]).is_err());
    let active = [ev(1, MARKER_INVESTIGATION_STARTED, json!({}))];
    assert_eq!(active_investigation_slice(&active).unwrap().1.seq, 1);

    for closing in [MARKER_INVESTIGATION_HANDOFF, MARKER_IDEA_STARTED, MARKER_SOLVE_STARTED] {
        let events = [
            ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
            ev(2, closing, json!({})),
        ];
        assert!(active_investigation_slice(&events).is_err(), "{closing} must close Investigation");
    }
}
