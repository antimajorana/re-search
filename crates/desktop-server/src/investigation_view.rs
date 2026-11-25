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
                }
            }
            _ => (),
        }
    }
    if running {
        mark_last_running(&mut activities, events);
        mark_last_running(&mut answers, events);
    }
    json!({"source_id":paper.and_then(|p| p.get("source_id")), "question":paper.and_then(|p| p.get("question")),
        "original_sha256":source.and_then(|e|e.payload.get("raw_blob")), "arxiv_cover_id":source.and_then(|e|e.payload.get("arxiv_cover_id")),
        "summary":summary.map(|e|&e.payload), "summary_seq":summary.map(|e|e.seq),
        "summary_is_current":summary.is_some_and(|e|e.payload["request_seq"].as_u64()==mapping.map(|m|m.seq)),
        "conventions":conventions, "conventions_requested":mapping.is_some_and(|e|e.payload["conventions_requested"]==true || resumed==Some(e.seq)),
        "conventions_is_current":catalog_request.is_some() && catalog_request==mapping.map(|e|e.seq),
        "conventions_error":catalog_finish.map(|e|&e.payload["error"]).or_else(||finished.and_then(|e|e.payload.get("conventions_error"))),
        "mapping_requested":mapping.is_some(),
        "mapping_status":finished.map(|e|&e.payload["status"]), "mapping_error":finished.and_then(|e|e.payload.get("error")),
        "page_start":paper.and_then(|p|p.get("page_start")), "page_end":paper.and_then(|p|p.get("page_end")),
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
            ev.seq > seq && matches!(ev.kind.as_str(), "session.end" | "session.abort")
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
                )
        }) {
            row["status"] = json!("running");
        }
    }
    rows
}
