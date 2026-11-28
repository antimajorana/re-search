//! Orient continuation validation, saved-reading summary, and net rejection helpers
//! (sibling of `investigation_task_context.rs`, kept <= 600 lines).

use crate::investigation_skeleton::EVENT_INVESTIGATION_STARTED;
use ledger::{Event, Ledger};
use research_tools::investigation_markers::{
    InvestigationTaskKind, EVENT_INVESTIGATION_READING, MARKER_TASK_FINISHED,
    MARKER_TASK_REQUESTED,
};
use research_tools::map_defs::RECORD_MAP_READING;
use research_tools::skeleton::PaperSkeleton;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Validates that `seq` refers to a `partial` or `stopped` Orient task in the current
/// Investigation slice, returning `Ok(seq)` or HTTP 400.
pub(crate) fn resolve_continuation(
    events: &[Event],
    task: InvestigationTaskKind,
    seq: u64,
) -> Result<u64, (u16, String)> {
    if task != InvestigationTaskKind::Orient {
        return Err((400, "`continue_from` is only supported for `orient` tasks.".into()));
    }
    let slice = events
        .iter()
        .rposition(|e| e.kind == EVENT_INVESTIGATION_STARTED)
        .map_or(events, |i| &events[i..]);
    let req = slice
        .iter()
        .find(|e| e.seq == seq && e.kind == MARKER_TASK_REQUESTED)
        .ok_or_else(|| {
            (
                400,
                format!("`continue_from` seq {seq} does not refer to an Investigate task request."),
            )
        })?;
    if req.payload["task"].as_str() != Some(InvestigationTaskKind::Orient.as_str()) {
        return Err((400, format!("`continue_from` seq {seq} is not an `orient` task.")));
    }
    let fin = slice
        .iter()
        .rev()
        .find(|e| {
            e.kind == MARKER_TASK_FINISHED
                && e.seq > seq
                && e.payload["request_seq"].as_u64() == Some(seq)
        })
        .ok_or_else(|| (400, format!("`continue_from` seq {seq} has not finished yet.")))?;
    let status = fin.payload["status"].as_str().unwrap_or("");
    if !matches!(status, "partial" | "stopped") {
        return Err((
            400,
            format!(
                "`continue_from` seq {seq} finished with status `{status}`; only `partial` or `stopped` Orient runs can be continued."
            ),
        ));
    }
    Ok(seq)
}

/// Parses optional `continue_from` JSON value and validates it via [`resolve_continuation`].
pub(crate) fn parse_and_resolve_continue_from(
    raw: Option<&Value>,
    events: &[Event],
    task: InvestigationTaskKind,
) -> Result<Option<u64>, (u16, String)> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let seq = v
                .as_u64()
                .filter(|&n| n >= 1)
                .ok_or_else(|| {
                    (400, "`continue_from` must be a positive integer sequence number.".into())
                })?;
            resolve_continuation(events, task, seq).map(Some)
        }
    }
}

/// Computes net `unresolved_rejected_items` for `request_seq` using `map_fold::fold_map` so
/// `investigation.task_finished.counts` matches `mv.tasks[*].unresolved_rejected_items`.
pub(crate) fn compute_unresolved_rejected(
    ledger: &Ledger,
    events: &[Event],
    request_seq: u64,
    fallback: u64,
) -> u64 {
    let Some(skel_ev) = events
        .iter()
        .rev()
        .find(|e| e.kind == crate::investigation_skeleton::EVENT_INVESTIGATION_SKELETON)
    else {
        return fallback;
    };
    let Some(blob) = skel_ev.payload["blob"].as_str() else {
        return fallback;
    };
    let Ok(bytes) = ledger.get_blob(blob) else {
        return fallback;
    };
    let Ok(skeleton) = serde_json::from_slice::<PaperSkeleton>(&bytes) else {
        return fallback;
    };
    let folded = research_tools::map_fold::fold_map(&skeleton, events, false);
    folded
        .tasks
        .iter()
        .find(|t| t.request_seq == request_seq)
        .map_or(fallback, |t| t.unresolved_rejected_items as u64)
}

/// Builds bounded metadata-only continuation summary of already saved readings plus an
/// optional note if the previous Orient run ended with an undispatched final `record_map_reading`.
pub(crate) fn build_orient_continuation_note(
    skeleton: &PaperSkeleton,
    events: &[Event],
    prev_seq: u64,
    char_budget: usize,
) -> String {
    if char_budget < 96 {
        return String::new();
    }
    let folded = research_tools::map_fold::fold_map(skeleton, events, false);
    let unsaved_note = if has_undispatched_final_reading_call(events, prev_seq) {
        format!(
            "Note: The previous Orient run (request_seq={prev_seq}) had an unsaved final \
             `record_map_reading` call before hitting its cap; submit any remaining items.\n"
        )
    } else {
        String::new()
    };
    let mut out = String::from(
        "\nAlready saved readings for this paper (metadata, not source lines; do not redo):\n",
    );
    let reserve_unsaved = unsaved_note.chars().count();
    let mut saved_count = 0usize;
    for (tag, reading) in &folded.nodes {
        let role = reading
            .role
            .current
            .as_ref()
            .filter(|v| !v.stale)
            .map(|v| format!(" role={:?}", v.value))
            .unwrap_or_default();
        let claim = reading
            .authors_claim
            .current
            .as_ref()
            .map(|v| format!(" claim={:?}", v.value.label))
            .unwrap_or_default();
        let gist = reading
            .gist
            .current
            .as_ref()
            .map(|v| {
                let s: String = v.value.chars().take(120).collect();
                format!(" gist=\"{s}\"")
            })
            .unwrap_or_default();
        if role.is_empty() && claim.is_empty() && gist.is_empty() {
            continue;
        }
        let line = format!("- [{tag}]{role}{claim}{gist}\n");
        if out.chars().count() + line.chars().count() + reserve_unsaved + 36 > char_budget {
            let _ = writeln!(out, "... (more saved readings omitted)");
            break;
        }
        out.push_str(&line);
        saved_count += 1;
    }
    if saved_count == 0 {
        out.push_str("- (none saved yet)\n");
    }
    if !unsaved_note.is_empty() && out.chars().count() + reserve_unsaved <= char_budget {
        out.push_str(&unsaved_note);
    }
    out
}

fn has_undispatched_final_reading_call(events: &[Event], prev_seq: u64) -> bool {
    let fin_seq = events
        .iter()
        .find(|e| {
            e.kind == MARKER_TASK_FINISHED
                && e.seq > prev_seq
                && e.payload["request_seq"].as_u64() == Some(prev_seq)
        })
        .map_or(u64::MAX, |e| e.seq);
    let mut reading_uses: Vec<(u64, Option<&str>)> = Vec::new();
    let mut dispatched: BTreeSet<&str> = BTreeSet::new();
    let mut reading_seqs: Vec<u64> = Vec::new();
    for ev in events.iter().filter(|e| e.seq > prev_seq && e.seq <= fin_seq) {
        match ev.kind.as_str() {
            "model.tool_use" if ev.payload["name"].as_str() == Some(RECORD_MAP_READING) => {
                reading_uses.push((ev.seq, ev.payload["id"].as_str().filter(|s| !s.is_empty())));
            }
            "model.response" => {
                if let Some(arr) = ev.payload["content"].as_array() {
                    for b in arr {
                        if b["type"].as_str() == Some("tool_use")
                            && b["name"].as_str() == Some(RECORD_MAP_READING)
                        {
                            reading_uses.push((ev.seq, b["id"].as_str().filter(|s| !s.is_empty())));
                        }
                    }
                }
            }
            "tool.result" | "tool.denied" => {
                if let Some(id) = ev.payload["call_id"].as_str() {
                    dispatched.insert(id);
                }
            }
            "model.tool_result" => {
                if let Some(id) = ev.payload["tool_use_id"].as_str() {
                    dispatched.insert(id);
                }
            }
            EVENT_INVESTIGATION_READING => reading_seqs.push(ev.seq),
            _ => {}
        }
    }
    reading_uses.into_iter().any(|(seq, id)| match id {
        Some(cid) => !dispatched.contains(cid),
        None => !reading_seqs.iter().any(|&rseq| rseq > seq),
    })
}
