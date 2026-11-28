//! Investigation session view projection and branch history.
use crate::investigation::{latest, list, marker};
use ledger::Event;
use serde_json::{json, Value};

pub(crate) fn view(events: &[Event], running: bool) -> Value {
    let Some(start) = marker(events) else {
        return Value::Null;
    };
    if start.kind == "investigation.handoff" {
        return json!({"origin": start.payload});
    }
    let paper = latest(events, "investigation.paper").map(|e| &e.payload);
    let source = paper.and_then(|p| {
        events
            .iter()
            .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == p["source_id"])
    });
    let map = latest(events, "investigation.map");
    let skeleton = latest(events, "investigation.skeleton");
    let mapping = latest(events, "investigation.mapping_requested");
    let summary = latest(events, "investigation.summary");
    let catalog_request =
        latest(events, "investigation.conventions").and_then(|e| e.payload["request_seq"].as_u64());
    let resumed = latest(events, "investigation.conventions_resumed")
        .and_then(|e| e.payload["for_request_seq"].as_u64());
    let conventions = catalog_request
        .and_then(|seq| research_tools::investigation::paper_conventions(events, seq));
    let finished = mapping.and_then(|request| {
        events.iter().rev().find(|event| {
            event.kind == "investigation.mapping_finished"
                && event.payload["request_seq"].as_u64() == Some(request.seq)
        })
    });
    let catalog_finish = mapping.and_then(|request| {
        events.iter().rev().find(|e| {
            e.kind == "investigation.conventions_finished"
                && e.payload["for_request_seq"].as_u64() == Some(request.seq)
        })
    });
    let mut activities = Vec::new();
    let mut positions = std::collections::HashMap::new();
    let mut answers = Vec::new();
    let mut answer_positions = std::collections::HashMap::new();
    let mut corrections = Vec::new();
    for event in events.iter().filter(|e| e.seq > start.seq) {
        match event.kind.as_str() {
            "investigation.correction" => {
                let mut row = event.payload.clone();
                row["seq"] = json!(event.seq);
                corrections.push(row);
            }
            "investigation.requested" => {
                let mut row = event.payload.clone();
                row["seq"] = json!(event.seq);
                row["status"] = json!("interrupted");
                positions.insert(event.seq, activities.len());
                activities.push(row);
            }
            "investigation.result" => {
                if let Some(index) = event.payload["request_seq"]
                    .as_u64()
                    .and_then(|seq| positions.get(&seq))
                {
                    let row = &mut activities[*index];
                    row["status"] = event.payload["status"].clone();
                    row["result"] = event.payload["text"].clone();
                    row["end_seq"] = json!(event.seq);
                }
            }
            "investigation.question_requested" => {
                let row = json!({
                    "seq": event.seq,
                    "question": event.payload["question"],
                    "status": "interrupted",
                    "text": Value::Null,
                    "error": Value::Null,
                    "cites": [],
                });
                answer_positions.insert(event.seq, answers.len());
                answers.push(row);
            }
            "investigation.answer" => {
                if let Some(&index) = event.payload["request_seq"]
                    .as_u64()
                    .and_then(|seq| answer_positions.get(&seq))
                {
                    let row = &mut answers[index];
                    row["status"] = event.payload["status"].clone();
                    row["text"] = event.payload["text"].clone();
                    row["error"] = event.payload["error"].clone();
                    row["cites"] = event
                        .payload
                        .get("cites")
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                }
            }
            _ => (),
        }
    }
    if running {
        mark_last_running(&mut activities, events);
        mark_last_running(&mut answers, events);
    }
    let latest_scope_req = events.iter().rev().find(|e| {
        matches!(
            e.kind.as_str(),
            "investigation.mapping_requested" | "investigation.question_requested"
        )
    });
    let scope =
        latest_scope_req.and_then(|req| crate::investigation_scope::latest_scope(events, req.seq));
    let conventions_expanding = mapping.is_some_and(|request| {
        crate::investigation_scope::conventions_view_expanding(
            events,
            request.seq,
            conventions.as_ref(),
            running,
        )
    });
    let latest_resumed = mapping.and_then(|request| {
        events.iter().rev().find(|e| {
            e.kind == "investigation.conventions_resumed"
                && e.payload["for_request_seq"].as_u64() == Some(request.seq)
        })
    });
    let conventions_is_current =
        catalog_request.is_some() && catalog_request == mapping.map(|m| m.seq);
    let current_catalog_done =
        conventions_is_current && conventions.as_ref().is_some_and(|c| c["done"] == true);
    let conventions_error = if let Some(res) = latest_resumed {
        catalog_finish
            .filter(|e| e.seq > res.seq)
            .and_then(|e| e.payload.get("error"))
            .filter(|v| !v.is_null())
    } else if current_catalog_done {
        None
    } else {
        finished
            .and_then(|e| e.payload.get("conventions_error"))
            .filter(|v| !v.is_null())
    };
    json!({"source_id":paper.and_then(|p| p.get("source_id")), "question":paper.and_then(|p| p.get("question")),
        "original_sha256":source.and_then(|e|e.payload.get("raw_blob")), "arxiv_cover_id":source.and_then(|e|e.payload.get("arxiv_cover_id")),
        "summary":summary.map(|e|&e.payload), "summary_seq":summary.map(|e|e.seq),
        "summary_is_current":summary.is_some_and(|e|e.payload["request_seq"].as_u64()==mapping.map(|m|m.seq)),
        "conventions":conventions, "conventions_requested":mapping.is_some_and(|e|e.payload["conventions_requested"]==true || resumed==Some(e.seq)),
        "conventions_is_current":conventions_is_current,
        "conventions_expanding":conventions_expanding,
        "conventions_error":conventions_error,
        "mapping_requested":mapping.is_some(),
        "mapping_status":finished.map(|e|&e.payload["status"]), "mapping_error":finished.and_then(|e|e.payload.get("error")),
        "scope":scope,
        "page_start":paper.and_then(|p|p.get("page_start")), "page_end":paper.and_then(|p|p.get("page_end")),
        "skeleton_seq":skeleton.map(|e| e.seq), "skeleton":skeleton.map(|e| &e.payload),
        "map_seq":map.map(|e| e.seq), "map":map.map(|e|research_tools::investigation::expanded_map(events,e)),
        "branches":branch_history(events,running), "corrections":corrections, "activities":activities, "answers":answers})
}

fn mark_last_running(rows: &mut [Value], events: &[Event]) {
    if let Some(row) = rows
        .last_mut()
        .filter(|row| row["status"] == "interrupted")
    {
        let seq = row["seq"].as_u64().unwrap_or_default();
        if !events.iter().any(|ev| {
            ev.seq > seq
                && matches!(
                    ev.kind.as_str(),
                    "session.end"
                        | "session.abort"
                        | "investigation.mapping_requested"
                        | "investigation.conventions_resumed"
                        | "investigation.branch_requested"
                        | "investigation.question_requested"
                        | "investigation.requested"
                        | "investigation.task_requested"
                        | "investigation.task_finished"
                )
        }) {
            row["status"] = json!("running");
        }
    }
}

pub(crate) fn branch_history(events: &[Event], running: bool) -> Vec<Value> {
    let mut rows: Vec<Value> = Vec::new();
    let mut positions = std::collections::HashMap::new();
    for event in events {
        match event.kind.as_str() {
            "investigation.branch_requested" => {
                let mut row = event.payload.clone();
                row["request_seq"] = json!(event.seq);
                row["status"] = json!("interrupted");
                positions.insert(event.seq, rows.len());
                rows.push(row);
            }
            "investigation.branch" | "investigation.branch_finished" => {
                let Some(i) = event.payload["request_seq"]
                    .as_u64()
                    .and_then(|seq| positions.get(&seq))
                else {
                    continue;
                };
                let row = &mut rows[*i];
                if event.kind == "investigation.branch" {
                    row["status"] = json!("built");
                    row["limitations"] = event.payload["limitations"].clone();
                    row["node_ids"] = json!(list(&event.payload["nodes"])
                        .iter()
                        .filter_map(|n| n["id"].as_str())
                        .collect::<Vec<_>>());
                } else if row["status"] != "built" {
                    row["status"] = event.payload["status"].clone();
                    row["error"] = event.payload["error"].clone();
                }
            }
            _ => (),
        }
    }
    if let Some(row) = rows
        .last_mut()
        .filter(|r| running && r["status"] == "interrupted")
    {
        let seq = row["request_seq"].as_u64().unwrap_or_default();
        if !events.iter().any(|e| {
            e.seq > seq
                && matches!(
                    e.kind.as_str(),
                    "session.abort"
                        | "session.end"
                        | "investigation.mapping_requested"
                        | "investigation.conventions_resumed"
                        | "investigation.question_requested"
                        | "investigation.requested"
                        | "investigation.task_requested"
                        | "investigation.task_finished"
                )
        }) {
            row["status"] = json!("running");
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::view;
    use ledger::Ledger;
    use serde_json::json;

    #[test]
    fn review_ui_stale_mapping_conventions_error_not_leaked() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let sid = "s-view-conv-err";
        ledger.append(sid, "investigation.started", 1, &json!({})).unwrap();
        ledger.append(sid, "investigation.paper", 1, &json!({"source_id": "src-1"})).unwrap();
        let map_req = ledger
            .append(
                sid,
                "investigation.mapping_requested",
                1,
                &json!({"source_id": "src-1", "conventions_requested": true}),
            )
            .unwrap();
        ledger
            .append(
                sid,
                "investigation.mapping_finished",
                1,
                &json!({
                    "request_seq": map_req,
                    "status": "mapped",
                    "conventions_error": "Stale mapping-era conventions error"
                }),
            )
            .unwrap();

        // 1. Before any resume or done catalog, mapping-era conventions_error IS shown.
        let v1 = view(&ledger.read_session(sid).unwrap(), false);
        assert_eq!(v1["conventions_error"], "Stale mapping-era conventions error");

        // 2. Once a done=true section catalog exists for the current mapping, stale mapping error clears.
        ledger
            .append(
                sid,
                "investigation.conventions",
                1,
                &json!({
                    "request_seq": map_req,
                    "source_id": "src-1",
                    "batch_index": 1,
                    "done": true,
                    "coverage": "partial",
                    "coverage_scope": "section",
                    "limitations": [],
                    "items": []
                }),
            )
            .unwrap();
        let v2 = view(&ledger.read_session(sid).unwrap(), false);
        assert!(v2["conventions_error"].is_null(), "done catalog leaked stale error: {v2}");

        // 3. After a new conventions_resumed (while running or stopped without its own finish),
        // stale mapping-era conventions_error must NOT leak back.
        ledger
            .append(
                sid,
                "investigation.conventions_resumed",
                1,
                &json!({"for_request_seq": map_req, "source_id": "src-1", "scope": "paper"}),
            )
            .unwrap();
        let v3 = view(&ledger.read_session(sid).unwrap(), true);
        assert!(v3["conventions_error"].is_null(), "running resume leaked stale error: {v3}");

        // 4. A real conventions_finished error AFTER latest conventions_resumed IS returned.
        ledger
            .append(
                sid,
                "investigation.conventions_finished",
                1,
                &json!({
                    "for_request_seq": map_req,
                    "status": "incomplete",
                    "error": "Paper expansion failed."
                }),
            )
            .unwrap();
        let v4 = view(&ledger.read_session(sid).unwrap(), false);
        assert_eq!(v4["conventions_error"], "Paper expansion failed.");

        // 5. Starting a fresh conventions_resumed clears the previous conventions_finished error,
        // and a succeeding conventions_finished (error: null) never falls back to mapping-era error.
        ledger
            .append(
                sid,
                "investigation.conventions_resumed",
                1,
                &json!({"for_request_seq": map_req, "source_id": "src-1", "scope": "paper"}),
            )
            .unwrap();
        let v5 = view(&ledger.read_session(sid).unwrap(), true);
        assert!(v5["conventions_error"].is_null(), "retry resume leaked prior error: {v5}");
        ledger
            .append(
                sid,
                "investigation.conventions_finished",
                1,
                &json!({"for_request_seq": map_req, "status": "complete", "error": null}),
            )
            .unwrap();
        let v6 = view(&ledger.read_session(sid).unwrap(), false);
        assert!(v6["conventions_error"].is_null(), "completed resume fell back to stale error: {v6}");
    }

    #[test]
    fn review_flow_question_turn_does_not_resurrect_interrupted_activity() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let sid = "s-view-act-q";
        ledger.append(sid, "investigation.started", 1, &json!({})).unwrap();
        ledger.append(sid, "investigation.paper", 1, &json!({"source_id": "src-1"})).unwrap();
        // Earlier claim-check activity that crashed without emitting investigation.result or session.end.
        ledger
            .append(
                sid,
                "investigation.requested",
                1,
                &json!({"map_seq": 2, "node_id": "r1", "action": "check", "question": "Check claim"}),
            )
            .unwrap();
        // Later question turn is now in flight.
        ledger
            .append(
                sid,
                "investigation.question_requested",
                1,
                &json!({"source_id": "src-1", "question": "What is Eq. 1?"}),
            )
            .unwrap();
        let v = view(&ledger.read_session(sid).unwrap(), true);
        assert_eq!(
            v["activities"][0]["status"], "interrupted",
            "interrupted activity resurrected as running during question turn: {v}"
        );
        assert_eq!(v["answers"][0]["status"], "running");
    }
}
