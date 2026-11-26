//! Shared Investigation request/finish marker table and bidirectional recorder gating (package V6, rule 6.3).
//!
//! All old and new Investigation recorders and the paper-reader evidence gate share
//! [`INVESTIGATION_REQUEST_MARKERS`] so an active V6 task (`investigation.task_requested`) closes
//! all four legacy recorders and `note_evidence`, while any legacy request or finish marker closes
//! all new V6+ task recorders.

use crate::paper_key::Mode;
use crate::tool_profile::{
    session_mode_from_events, MARKER_IDEA_STARTED, MARKER_INVESTIGATION_HANDOFF,
    MARKER_INVESTIGATION_STARTED, MARKER_SOLVE_STARTED,
};
use ledger::Event;
use serde::{Deserialize, Serialize};
use std::fmt;

// Legacy Investigation request/finish markers (preserved unchanged, investigate.md §4.4).
pub const MARKER_MAPPING_REQUESTED: &str = "investigation.mapping_requested";
pub const MARKER_MAPPING_FINISHED: &str = "investigation.mapping_finished";
pub const MARKER_BRANCH_REQUESTED: &str = "investigation.branch_requested";
pub const MARKER_BRANCH_FINISHED: &str = "investigation.branch_finished";
pub const MARKER_CONVENTIONS_RESUMED: &str = "investigation.conventions_resumed";
pub const MARKER_CONVENTIONS_FINISHED: &str = "investigation.conventions_finished";
pub const MARKER_QUESTION_REQUESTED: &str = "investigation.question_requested";
pub const MARKER_ANSWER: &str = "investigation.answer";
pub const MARKER_REQUESTED: &str = "investigation.requested";
pub const MARKER_RESULT: &str = "investigation.result";

// V6 Investigate task lifecycle markers (`investigate.md` §4.4; accepted in `00-overview.md` §8).
pub const MARKER_TASK_REQUESTED: &str = "investigation.task_requested";
pub const MARKER_TASK_FINISHED: &str = "investigation.task_finished";

// Session lifecycle markers that close any active request.
pub const MARKER_SESSION_END: &str = "session.end";
pub const MARKER_SESSION_ABORT: &str = "session.abort";

// V6+ Layer-2 record event kinds (`investigate.md` §4.4; accepted in `00-overview.md` §8).
pub const EVENT_INVESTIGATION_SKELETON: &str = "investigation.skeleton";
pub const EVENT_INVESTIGATION_READING: &str = "investigation.reading";
pub const EVENT_INVESTIGATION_CONTEXT: &str = "investigation.context";
pub const EVENT_INVESTIGATION_PRIOR_WORK: &str = "investigation.prior_work";
pub const EVENT_INVESTIGATION_NOTE: &str = "investigation.note";
pub const EVENT_INVESTIGATION_OVERRIDE: &str = "investigation.override";

/// Single shared list of every marker kind that opens or closes an Investigation turn/task
/// (`investigate.md` rule 6.3). Consumed by `investigation.rs`, `investigation_evidence.rs`,
/// `desktop-server/src/paper_reader.rs`, and all V6+ task recorders.
pub const INVESTIGATION_REQUEST_MARKERS: &[&str] = &[
    MARKER_MAPPING_REQUESTED,
    MARKER_MAPPING_FINISHED,
    MARKER_BRANCH_REQUESTED,
    MARKER_BRANCH_FINISHED,
    MARKER_CONVENTIONS_RESUMED,
    MARKER_CONVENTIONS_FINISHED,
    MARKER_QUESTION_REQUESTED,
    MARKER_ANSWER,
    MARKER_REQUESTED,
    MARKER_RESULT,
    MARKER_TASK_REQUESTED,
    MARKER_TASK_FINISHED,
    MARKER_SESSION_END,
    MARKER_SESSION_ABORT,
];

/// Returns `true` iff `kind` participates in the Investigation marker gate (`investigate.md` 6.3).
pub fn is_investigation_marker(kind: &str) -> bool {
    INVESTIGATION_REQUEST_MARKERS.contains(&kind)
}

/// Returns the newest marker event in `history`, if any.
pub fn latest_investigation_marker(history: &[Event]) -> Option<&Event> {
    history
        .iter()
        .rev()
        .find(|e| is_investigation_marker(&e.kind))
}

/// Returns the active Investigation slice (starting at the latest `investigation.started` marker)
/// and that marker event, failing closed if the session is not in Investigate mode.
pub fn active_investigation_slice(events: &[Event]) -> Result<(&[Event], &Event), String> {
    if session_mode_from_events(events) != Mode::Investigate {
        return Err("Session is not in an active Investigation.".into());
    }
    let idx = events
        .iter()
        .rposition(|e| {
            matches!(
                e.kind.as_str(),
                MARKER_INVESTIGATION_STARTED
                    | MARKER_INVESTIGATION_HANDOFF
                    | MARKER_IDEA_STARTED
                    | MARKER_SOLVE_STARTED
            )
        })
        .filter(|&i| events[i].kind == MARKER_INVESTIGATION_STARTED)
        .ok_or_else(|| "Session is not in an active Investigation.".to_string())?;
    Ok((&events[idx..], &events[idx]))
}

/// Task kinds recorded in `investigation.task_requested["task"]` (`investigate.md` §4.4, §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationTaskKind {
    Orient,
    Open,
    Context,
    PriorWork,
}

impl InvestigationTaskKind {
    pub const ALL: [Self; 4] = [Self::Orient, Self::Open, Self::Context, Self::PriorWork];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orient => "orient",
            Self::Open => "open",
            Self::Context => "context",
            Self::PriorWork => "prior_work",
        }
    }

    pub fn from_str_exact(s: &str) -> Option<Self> {
        match s {
            "orient" => Some(Self::Orient),
            "open" => Some(Self::Open),
            "context" => Some(Self::Context),
            "prior_work" => Some(Self::PriorWork),
            _ => None,
        }
    }

    /// Default model-call cap per task (`investigate.md` §5.3).
    pub fn default_call_cap(self) -> u32 {
        match self {
            Self::Orient => 8,
            Self::Open => 10,
            Self::Context => 8,
            Self::PriorWork => 10,
        }
    }
}

impl fmt::Display for InvestigationTaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The four legacy Investigation recorders (kept closed during any V6 task).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OldRecorderKind {
    Map,
    Summary,
    Branch,
    Conventions,
}

impl OldRecorderKind {
    pub const ALL: [Self; 4] = [Self::Map, Self::Summary, Self::Branch, Self::Conventions];

    pub fn tool_id(self) -> &'static str {
        match self {
            Self::Map => crate::investigation::RECORD_PAPER_MAP,
            Self::Summary => crate::investigation::RECORD_PAPER_SUMMARY,
            Self::Branch => crate::investigation::RECORD_PAPER_BRANCH,
            Self::Conventions => crate::investigation::RECORD_PAPER_CONVENTIONS,
        }
    }
}

/// The three new Investigation v2 recorders (`investigate.md` §5.1, rule 6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NewRecorderKind {
    MapReading,
    ContextCards,
    PriorWorkCheck,
}

impl NewRecorderKind {
    pub const ALL: [Self; 3] = [Self::MapReading, Self::ContextCards, Self::PriorWorkCheck];

    pub fn tool_id(self) -> &'static str {
        match self {
            Self::MapReading => crate::map_defs::RECORD_MAP_READING,
            Self::ContextCards => crate::map_defs::RECORD_CONTEXT_CARDS,
            Self::PriorWorkCheck => crate::map_defs::RECORD_PRIOR_WORK_CHECK,
        }
    }

    /// Whether this recorder is permitted while `task` is the active `task_requested`.
    pub fn accepts_task(self, task: InvestigationTaskKind) -> bool {
        match self {
            Self::MapReading => {
                matches!(task, InvestigationTaskKind::Orient | InvestigationTaskKind::Open)
            }
            Self::ContextCards => {
                matches!(task, InvestigationTaskKind::Context | InvestigationTaskKind::Open)
            }
            Self::PriorWorkCheck => matches!(task, InvestigationTaskKind::PriorWork),
        }
    }
}

/// Enforce the legacy recorder gate against the shared marker table (`investigate.md` rule 6.3).
pub fn check_old_recorder_gate(
    history: &[Event],
    recorder: OldRecorderKind,
) -> Result<&Event, String> {
    let expected = if recorder == OldRecorderKind::Branch {
        MARKER_BRANCH_REQUESTED
    } else {
        MARKER_MAPPING_REQUESTED
    };
    latest_investigation_marker(history)
        .filter(|e| {
            e.kind == expected
                || (recorder == OldRecorderKind::Conventions
                    && e.kind == MARKER_CONVENTIONS_RESUMED)
        })
        .ok_or_else(|| {
            if recorder == OldRecorderKind::Branch {
                "Select a result and start a focused branch before saving its dependencies.".into()
            } else {
                "Start mapping the paper before recording its overview. Claim checks use the existing map.".into()
            }
        })
}

/// Enforce the new V6+ task recorder gate (`investigate.md` rule 6.3).
///
/// Accepts ONLY when the newest marker in `history` is `investigation.task_requested`, its
/// `task` field matches `recorder`, and `marker.seq == request_seq`.
pub fn check_new_recorder_gate(
    history: &[Event],
    recorder: NewRecorderKind,
    request_seq: u64,
) -> Result<(&Event, InvestigationTaskKind), String> {
    let marker = latest_investigation_marker(history).ok_or_else(|| {
        format!(
            "No active Investigate task is open for `{}`.",
            recorder.tool_id()
        )
    })?;
    if marker.kind != MARKER_TASK_REQUESTED {
        return Err(format!(
            "`{}` cannot write while the latest marker is `{}`; start a matching Investigate task first.",
            recorder.tool_id(),
            marker.kind
        ));
    }
    let task_raw = marker.payload["task"].as_str().ok_or_else(|| {
        "Active `investigation.task_requested` marker is missing a valid `task` field.".to_string()
    })?;
    let task = InvestigationTaskKind::from_str_exact(task_raw).ok_or_else(|| {
        format!("Active `investigation.task_requested` marker has unknown task `{task_raw}`.")
    })?;
    if !recorder.accepts_task(task) {
        return Err(format!(
            "`{}` is not allowed during an active `{task}` task.",
            recorder.tool_id()
        ));
    }
    if marker.seq != request_seq {
        return Err(format!(
            "`{}` requires request_seq={} to match the active `{task}` task (got {request_seq}).",
            recorder.tool_id(),
            marker.seq
        ));
    }
    Ok((marker, task))
}

#[cfg(test)]
#[path = "investigation_markers_tests.rs"]
mod tests;
