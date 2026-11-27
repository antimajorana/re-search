//! Read-only child hand-off status projection and F12 wording enforcement
//! (package V16, `00-overview.md` §4, `investigate.md` §9 V16).

use export::handoff::{
    ExploreQuestionKind, HandoffAsk, HandoffPacket, HANDOFF_PACKET_EVENT, HANDOFF_SENT_EVENT,
};
use ledger::{Event, Ledger};
use research_tools::paper_key::Mode;
use research_tools::prior_work::PriorWorkStatus;
use research_tools::prior_work_wording::wording::FORBIDDEN_GENERATED_WORDS;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Read-only projection of a child session status triggered from a parent Investigate node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildHandoffStatusView {
    pub sent_seq: u64,
    pub child_session: String,
    pub parent_node_id: String,
    pub target_mode: Mode,
    pub action_kind: String,
    pub title: String,
    pub state: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_seq: Option<u64>,
}

/// Strictly read-only projection of child session statuses from `handoff.sent` events in `parent_events`.
/// Never writes back to any parent or child ledger (F7 Rule 5) and enforces F12 wording invariants.
pub fn child_handoff_statuses(
    ledger: &Ledger,
    parent_events: &[Event],
) -> Result<Vec<ChildHandoffStatusView>, String> {
    let mut out = Vec::new();
    for ev in parent_events.iter().filter(|e| e.kind == HANDOFF_SENT_EVENT) {
        let Some(child_session) = ev.payload["child_session"].as_str() else {
            continue;
        };
        let parent_node_id = ev.payload["from"]["id"].as_str().unwrap_or_default().to_string();
        let target_mode = serde_json::from_value::<Mode>(ev.payload["target_mode"].clone())
            .unwrap_or(Mode::Research);
        let title = ev.payload["title"].as_str().unwrap_or_default().to_string();
        let child_events = ledger.read_session(child_session).map_err(|e| e.to_string())?;
        let action_kind = infer_child_action_kind(&child_events, target_mode);
        let (state, label, source_event_kind, source_event_seq) =
            summarize_child_status(target_mode, &child_events);
        out.push(ChildHandoffStatusView {
            sent_seq: ev.seq,
            child_session: child_session.to_string(),
            parent_node_id,
            target_mode,
            action_kind,
            title,
            state,
            label: sanitize_f12_status_label(&label),
            source_event_kind,
            source_event_seq,
        });
    }
    Ok(out)
}

/// Attach read-only `handoff_statuses` to an Investigate `mapview` JSON response without writes.
pub fn attach_child_statuses_to_mapview(
    ledger: &Ledger,
    parent_events: &[Event],
    mv: &mut Value,
) {
    if let Ok(statuses) = child_handoff_statuses(ledger, parent_events) {
        mv["handoff_statuses"] = serde_json::to_value(statuses).unwrap_or_else(|_| json!([]));
    }
}

/// Enforce F12 invariant on right half-badge / Explore hand-off labels: never emit `"new"`,
/// `"novel"`, `"novelty"`, `"verified"`, `"proved"`, or `"unexplored"` as whole words.
pub fn sanitize_f12_status_label(label: &str) -> String {
    let contains_forbidden = label
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| !tok.is_empty() && FORBIDDEN_GENERATED_WORDS.contains(&tok.to_ascii_lowercase().as_str()));
    if contains_forbidden {
        "Opened in Explore: status recorded".to_string()
    } else {
        label.to_string()
    }
}

fn infer_child_action_kind(child_events: &[Event], target_mode: Mode) -> String {
    let packet = child_events
        .iter()
        .find(|e| e.kind == HANDOFF_PACKET_EVENT)
        .and_then(|e| serde_json::from_value::<HandoffPacket>(e.payload.clone()).ok());
    if let Some(pkt) = packet {
        return match pkt.ask {
            HandoffAsk::Solve(_) => "derive_solve".into(),
            HandoffAsk::Explore(e) => match e.question_kind {
                ExploreQuestionKind::Extended => "extend_explore".into(),
                ExploreQuestionKind::PriorWork => "prior_work_explore".into(),
            },
            HandoffAsk::Investigate(_) => "investigate".into(),
        };
    }
    match target_mode {
        Mode::Solve => "derive_solve".into(),
        Mode::Explore => "extend_explore".into(),
        Mode::Investigate | Mode::Research => "investigate".into(),
    }
}

fn summarize_child_status(
    mode: Mode,
    events: &[Event],
) -> (String, String, Option<String>, Option<u64>) {
    if events.is_empty() {
        return ("missing".into(), "Session not found".into(), None, None);
    }
    match mode {
        Mode::Solve => {
            if let Some(ev) = events.iter().rev().find(|e| e.kind == "solve.done") {
                let outcome = ev.payload["outcome"].as_str().unwrap_or("finished");
                let label = if outcome == "finished_open_items" {
                    "Opened in Solve: Done with open items"
                } else {
                    "Opened in Solve: Done"
                };
                return (outcome.into(), label.into(), Some(ev.kind.clone()), Some(ev.seq));
            }
            if let Some(ev) = events.iter().rev().find(|e| e.kind == "solve.paused") {
                let reason = ev.payload["reason"].as_str().unwrap_or("paused");
                return (
                    format!("paused:{reason}"),
                    format!("Opened in Solve: Paused ({reason})"),
                    Some(ev.kind.clone()),
                    Some(ev.seq),
                );
            }
            let active = events.iter().any(|e| {
                matches!(e.kind.as_str(), "session.start" | "session.resume" | "model.request" | "tool.request")
            });
            let label = if active { "Opened in Solve: in progress" } else { "Opened in Solve: Not run yet" };
            ("opened".into(), label.into(), None, None)
        }
        Mode::Explore => {
            if let Some(ev) = events.iter().rev().find(|e| e.kind == "idea.verdict") {
                let (state, text) = format_explore_verdict(&ev.payload);
                return (state, format!("Opened in Explore: {text}"), Some(ev.kind.clone()), Some(ev.seq));
            }
            ("opened".into(), "Opened in Explore: not finished".into(), None, None)
        }
        Mode::Investigate | Mode::Research => {
            if let Some(ev) = events.iter().rev().find(|e| {
                matches!(e.kind.as_str(), "investigation.map" | "investigation.skeleton")
            }) {
                let (state, label) = if ev.kind == "investigation.map" {
                    ("mapped", "Opened in Investigate: Mapped")
                } else {
                    ("skeleton", "Opened in Investigate: Skeleton ready")
                };
                return (state.into(), label.into(), Some(ev.kind.clone()), Some(ev.seq));
            }
            ("opened".into(), "Opened in Investigate: Not run yet".into(), None, None)
        }
    }
}

fn format_explore_verdict(payload: &Value) -> (String, String) {
    if let Some(raw_status) = payload.get("status").and_then(Value::as_str) {
        if let Ok(st) = serde_json::from_value::<PriorWorkStatus>(json!(raw_status)) {
            return (raw_status.to_string(), explore_status_phrase(st).to_string());
        }
        return (raw_status.to_string(), raw_status.to_string());
    }
    if let Some(summary) = payload.get("summary").and_then(Value::as_str) {
        return (summary.to_string(), summary.to_string());
    }
    if let Some(first) = payload.get("verdicts").and_then(Value::as_array).and_then(|a| a.first()) {
        if let Some(raw_status) = first.get("status").and_then(Value::as_str) {
            if let Ok(st) = serde_json::from_value::<PriorWorkStatus>(json!(raw_status)) {
                return (raw_status.to_string(), explore_status_phrase(st).to_string());
            }
            return (raw_status.to_string(), raw_status.to_string());
        }
    }
    ("verdict_recorded".into(), "verdict recorded".into())
}

fn explore_status_phrase(st: PriorWorkStatus) -> &'static str {
    match st {
        PriorWorkStatus::JudgedSame => "Model judges: same",
        PriorWorkStatus::JudgedPartial => "Model judges: partly the same",
        PriorWorkStatus::JudgedOtherName => "Model judges: same under other names",
        PriorWorkStatus::AbstractOnly => "Model judges: overlap on abstract only",
        PriorWorkStatus::NoneInScope => "not found in the searched scope",
        PriorWorkStatus::Incomplete => "search incomplete",
    }
}
