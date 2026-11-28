//! Host route for additive reader pins (package V7) and reader notes (package V10)
//! (`POST /api/sessions/{id}/investigation/notes`, `investigate.md` §4.3–4.4, §5.2, rules 6.4, 6.12).
//!
//! Writes additive `investigation.note` events carrying `{ source_id, tag, node_fp, pin?, text? }`.
//! Validates `tag` against the pinned paper skeleton (or folded `m-` model nodes), rejects
//! stale `node_fp` mismatches with HTTP 409, and never overwrites or deletes earlier notes.

use crate::investigation::{is_investigation, latest};
use crate::investigation_skeleton::{EVENT_INVESTIGATION_PAPER, EVENT_INVESTIGATION_SKELETON};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use ledger::{Event, Ledger};
use research_tools::investigation_markers::EVENT_INVESTIGATION_NOTE;
use research_tools::map_fold::{fold_map, ReaderPin};
use research_tools::skeleton::PaperSkeleton;
use research_tools::validate_session_id;
use serde_json::{json, Value};

/// Maximum free-text reader note length in Unicode scalar values (aligned with F7 `MAX_USER_NOTE_CHARS`).
pub const MAX_NOTE_TEXT_CHARS: usize = 4_000;
const MAX_TAG_CHARS: usize = 64;
const MAX_FP_CHARS: usize = 64;
const MAX_NEARBY_TAGS: usize = 6;

/// HTTP handler for `POST /api/sessions/{id}/investigation/notes`.
///
/// Accepts JSON `{ tag, node_fp?, pin?, text? }` where at least one of `pin`
/// (`"main_result"` | `"not_main_result"`) or `text` (1..=4000 chars) must be present.
pub fn handle(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(storage)? {
        return Err(not_found("Session missing."));
    }
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else {
        return Err(bad("Notes request body must be a JSON object."));
    };
    for key in obj.keys() {
        if !matches!(key.as_str(), "tag" | "node_fp" | "pin" | "text") {
            return Err(bad(format!("Unknown field `{key}` in investigation note request.")));
        }
    }

    let tag = obj
        .get("tag")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().count() <= MAX_TAG_CHARS)
        .ok_or_else(|| bad("Provide a valid `tag` for the target node."))?;

    let supplied_fp = match obj.get("node_fp") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_FP_CHARS {
                return Err(bad("`node_fp` must be a non-empty string up to 64 characters."));
            }
            Some(trimmed.to_string())
        }
        Some(_) => return Err(bad("`node_fp` must be a string.")),
    };

    let pin = match obj.get("pin") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let parsed = serde_json::from_value::<ReaderPin>(v.clone()).map_err(|_| {
                bad("`pin` must be \"main_result\" or \"not_main_result\".")
            })?;
            Some(parsed)
        }
    };

    let text = match obj.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(bad("`text` must not be empty."));
            }
            if trimmed.chars().count() > MAX_NOTE_TEXT_CHARS {
                return Err(bad(format!(
                    "`text` must be at most {MAX_NOTE_TEXT_CHARS} characters."
                )));
            }
            Some(trimmed.to_string())
        }
        Some(_) => return Err(bad("`text` must be a string.")),
    };

    if pin.is_none() && text.is_none() {
        return Err(bad("Provide at least one of `pin` or `text`."));
    }

    let ledger = state.open_ledger().map_err(storage)?;
    let events = ledger.read_session(session_id).map_err(storage)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before saving notes or pins."));
    }

    let (source_id, skeleton_seq, skeleton) = load_pinned_skeleton(&ledger, &events)?;
    let current_fp = resolve_target_node_fp(&skeleton, &events, tag)?;
    if let Some(ref fp) = supplied_fp {
        if fp != &current_fp {
            return Err(conflict(format!(
                "Node `{tag}` fingerprint `{fp}` does not match pinned node fingerprint `{current_fp}`."
            )));
        }
    }

    let mut payload = json!({
        "source_id": source_id,
        "skeleton_seq": skeleton_seq,
        "tag": tag,
        "node_fp": current_fp,
    });
    if let Some(p) = pin {
        payload["pin"] = json!(p);
    }
    if let Some(ref t) = text {
        payload["text"] = json!(t);
    }

    let seq = {
        let _lock = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("ledger lock poisoned"))?;
        ledger
            .append(session_id, EVENT_INVESTIGATION_NOTE, 1, &payload)
            .map_err(storage)?
    };

    Ok((
        201,
        json!({
            "seq": seq,
            "source_id": source_id,
            "skeleton_seq": skeleton_seq,
            "tag": tag,
            "node_fp": current_fp,
            "pin": pin,
            "text": text,
        }),
    ))
}

fn resolve_target_node_fp(
    skeleton: &PaperSkeleton,
    events: &[Event],
    tag: &str,
) -> Result<String, (u16, String)> {
    if let Some(sk_node) = skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(tag)) {
        return Ok(sk_node.fp.clone());
    }
    let folded = fold_map(skeleton, events, false);
    if let Some(reading) = folded.nodes.get(tag) {
        if let Some(fp) = reading
            .anchors
            .current
            .as_ref()
            .map(|v| v.node_fp.as_str())
            .or_else(|| reading.gist.current.as_ref().map(|v| v.node_fp.as_str()))
            .or_else(|| reading.purpose.current.as_ref().map(|v| v.node_fp.as_str()))
            .or_else(|| reading.role.current.as_ref().map(|v| v.node_fp.as_str()))
            .filter(|s| !s.is_empty())
        {
            return Ok(fp.to_string());
        }
    }
    let valid_tags: Vec<&str> = skeleton
        .nodes
        .iter()
        .filter_map(|n| n.tag.as_deref())
        .take(MAX_NEARBY_TAGS)
        .collect();
    let hint = if valid_tags.is_empty() {
        String::new()
    } else {
        format!(" Valid tags include: {}.", valid_tags.join(", "))
    };
    Err(bad(format!(
        "Unknown node tag `{tag}` for this pinned paper.{hint}"
    )))
}

fn load_pinned_skeleton(
    ledger: &Ledger,
    events: &[Event],
) -> Result<(String, u64, PaperSkeleton), (u16, String)> {
    const NEED_SKEL: &str =
        "Build or pin the deterministic paper skeleton before saving notes or pins.";
    let source_id = latest(events, EVENT_INVESTIGATION_PAPER)
        .and_then(|e| e.payload["source_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad(NEED_SKEL))?
        .to_string();
    let skel_ev = events
        .iter()
        .rev()
        .find(|e| {
            e.kind == EVENT_INVESTIGATION_SKELETON
                && e.payload["source_id"].as_str() == Some(source_id.as_str())
        })
        .ok_or_else(|| bad(NEED_SKEL))?;
    let blob_hash = skel_ev.payload["blob"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| internal("stored investigation.skeleton missing `blob` hash"))?;
    let bytes = ledger
        .get_blob(blob_hash)
        .map_err(|e| internal(format!("failed to read skeleton blob `{blob_hash}`: {e}")))?;
    if ledger::sha256_hex(&bytes) != blob_hash {
        return Err(internal(format!(
            "skeleton blob hash mismatch for `{blob_hash}`"
        )));
    }
    let skeleton: PaperSkeleton = serde_json::from_slice(&bytes)
        .map_err(|e| internal(format!("failed to decode skeleton blob: {e}")))?;
    if skeleton.source_id != source_id {
        return Err(internal(
            "skeleton blob source_id does not match pinned paper",
        ));
    }
    Ok((source_id, skel_ev.seq, skeleton))
}

fn storage(err: impl ToString) -> (u16, String) {
    internal(err.to_string())
}

#[cfg(test)]
#[path = "investigation_notes_tests.rs"]
mod tests;
