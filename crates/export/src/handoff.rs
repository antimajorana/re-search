//! Shared cross-mode hand-off packet execution (package F7, `00-overview.md` section 4).
//!
//! Creates a new child session whose first events are, in exact order:
//! 1. Copies of the needed `source.ingested` events (verified SHA-256 via `fork::copy_session_sources`).
//! 2. One `handoff.packet` v1 event whose every `QuoteRef` has been re-matched against the child's
//!    own copied files and receipts via `research_tools::quote_core::verify_quote_ref` (Rule 1).
//! 3. The target mode's host-written marker (`investigation.started`, `idea.started`, or `solve.started`).
//!
//! The origin session receives one `handoff.sent` event (existing `investigation.linked` pattern).
//! Zero model calls and zero network calls happen here. The existing Investigation-to-Research
//! hand-off (`export::fork::research_from_paper`) is completely untouched.

pub use crate::handoff_types::*;
pub use crate::handoff_view::*;

use crate::fork::{copy_session_sources, generate_session_id};
use ledger::{Event, Ledger};
use research_tools::quote_core::{quote_sha256, verify_quote_ref, QuoteRef, EVIDENCE_QUOTE_EVENT};
use research_tools::{validate_session_id, SandboxRoot, WorkspaceSandbox};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Execute a cross-mode hand-over: copy required verified sources, re-match every `QuoteRef`
/// in the child session (dropping any quote that fails), write `[source.ingested..., handoff.packet,
/// <target_marker>]` as the child's first events, and append `handoff.sent` to the origin session.
pub fn execute_handoff(data_dir: &Path, req: &HandoffRequest) -> Result<HandoffReceipt, String> {
    req.packet.validate()?;
    let target_marker = req.packet.target_marker()?;
    let parent = req.packet.from.session.as_str();
    validate_session_id(parent).map_err(|e| e.to_string())?;

    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (_, parent_events) = ledger
        .read_verified_session(parent)
        .map_err(|e| format!("origin session `{parent}` failed verification: {e}"))?;

    let needed_source_ids = collect_needed_source_ids(&req.packet, &req.extra_source_ids);
    let copied_source_events: Vec<Event> = parent_events
        .iter()
        .filter(|ev| {
            ev.kind == "source.ingested"
                && ev.payload["source_id"]
                    .as_str()
                    .is_some_and(|id| needed_source_ids.contains(id))
        })
        .cloned()
        .collect();

    // Generate a unique child session id that does not exist in the ledger or on disk.
    let existing_sessions: BTreeSet<String> = ledger
        .sessions()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    let mut child = generate_session_id()?;
    while existing_sessions.contains(&child) || data_dir.join("sessions").join(&child).exists() {
        child = generate_session_id()?;
    }

    // Copy and hash-verify needed sources into `sessions/<child>/sources/` (RAII cleans up on error).
    let mut staged = copy_session_sources(data_dir, parent, &child, &copied_source_events)?;

    // Build a lookup of verbatim quote texts from caller map + origin evidence events.
    let quote_lookup = build_quote_text_lookup(&req.quote_texts, &parent_events);

    // Build the child's source receipt list with child-relative seq numbers (1..=N) for verify_quote_ref.
    let child_source_receipts: Vec<Event> = copied_source_events
        .iter()
        .enumerate()
        .map(|(idx, ev)| {
            let mut c = ev.clone();
            c.session = child.clone();
            c.seq = (idx + 1) as u64;
            c
        })
        .collect();

    // Rule 1: re-match every QuoteRef against the child's own receipts and copied files.
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let mut verified_packet = req.packet.clone();
    let mut quotes_matched = 0usize;
    let mut dropped_quotes = Vec::new();

    verified_packet.statement.quotes = reverify_quote_list(
        &sandbox,
        &child,
        &child_source_receipts,
        &quote_lookup,
        &req.packet.statement.quotes,
        "statement",
        &mut quotes_matched,
        &mut dropped_quotes,
    );

    for (idx, item) in verified_packet.context.iter_mut().enumerate() {
        let loc = format!("context[{idx}]");
        item.quotes = reverify_quote_list(
            &sandbox,
            &child,
            &child_source_receipts,
            &quote_lookup,
            &item.quotes,
            &loc,
            &mut quotes_matched,
            &mut dropped_quotes,
        );
    }

    if let HandoffAsk::Investigate(inv) = &mut verified_packet.ask {
        inv.earlier_quotes = reverify_quote_list(
            &sandbox,
            &child,
            &child_source_receipts,
            &quote_lookup,
            &inv.earlier_quotes,
            "ask.earlier_quotes",
            &mut quotes_matched,
            &mut dropped_quotes,
        );
    }

    let packet_val = serde_json::to_value(&verified_packet)
        .map_err(|e| format!("failed to serialize handoff.packet: {e}"))?;
    let marker_payload = json!({});

    // Exact first events in child: [copied source.ingested..., handoff.packet, <target_marker>].
    let mut batch: Vec<(&str, u32, &Value)> = copied_source_events
        .iter()
        .map(|ev| (ev.kind.as_str(), ev.schema_version, &ev.payload))
        .collect();
    batch.push((HANDOFF_PACKET_EVENT, HANDOFF_SCHEMA_VERSION, &packet_val));
    batch.push((target_marker, HANDOFF_SCHEMA_VERSION, &marker_payload));

    ledger
        .append_new_session(&child, &batch)
        .map_err(|e| format!("failed to write child hand-off session `{child}`: {e}"))?;
    if let Some(files) = &mut staged {
        files.committed = true;
    }

    let child_events = ledger.read_session(&child).map_err(|e| e.to_string())?;
    let packet_ev = child_events
        .iter()
        .find(|e| e.kind == HANDOFF_PACKET_EVENT)
        .ok_or_else(|| "handoff.packet missing after commit".to_string())?;

    // Record `handoff.sent` in the origin session (existing `investigation.linked` pattern).
    let sent_payload = json!({
        "child_session": child,
        "target_mode": verified_packet.target_mode,
        "from": verified_packet.from,
        "title": verified_packet.title,
        "ask_kind": verified_packet.ask.kind_name(),
        "packet_seq": packet_ev.seq,
        "packet_hash": packet_ev.event_hash,
        "quotes_matched": quotes_matched,
        "quotes_dropped": dropped_quotes.len(),
    });
    ledger
        .append(parent, HANDOFF_SENT_EVENT, HANDOFF_SCHEMA_VERSION, &sent_payload)
        .map_err(|e| format!("failed to record handoff.sent on `{parent}`: {e}"))?;

    Ok(HandoffReceipt {
        child_session: child,
        parent_session: parent.to_string(),
        target_mode: verified_packet.target_mode,
        target_marker: target_marker.to_string(),
        packet_seq: packet_ev.seq,
        packet_hash: packet_ev.event_hash.clone(),
        copied_sources: copied_source_events.len(),
        quotes_matched,
        quotes_dropped: dropped_quotes.len(),
        dropped_quotes,
        packet: verified_packet,
    })
}

fn collect_needed_source_ids(packet: &HandoffPacket, extra: &[String]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for q in &packet.statement.quotes {
        ids.insert(q.source_id.clone());
    }
    for item in &packet.context {
        for q in &item.quotes {
            ids.insert(q.source_id.clone());
        }
    }
    if let HandoffAsk::Investigate(inv) = &packet.ask {
        for q in &inv.earlier_quotes {
            ids.insert(q.source_id.clone());
        }
    }
    for sid in extra {
        let trimmed = sid.trim();
        if !trimmed.is_empty() {
            ids.insert(trimmed.to_string());
        }
    }
    ids
}

fn build_quote_text_lookup(
    caller_map: &BTreeMap<String, String>,
    parent_events: &[Event],
) -> BTreeMap<String, String> {
    let mut map = caller_map.clone();
    for ev in parent_events {
        if ev.kind == EVIDENCE_QUOTE_EVENT {
            if let (Some(hash), Some(q)) = (
                ev.payload["quote_sha256"].as_str(),
                ev.payload["quote"].as_str(),
            ) {
                map.entry(hash.to_string()).or_insert_with(|| q.to_string());
            }
        } else if ev.kind == "investigation.evidence" {
            if let Some(q) = ev.payload["quote"].as_str() {
                map.entry(quote_sha256(q)).or_insert_with(|| q.to_string());
            }
        } else if matches!(
            ev.kind.as_str(),
            "investigation.map" | "investigation.branch" | "investigation.summary"
        ) {
            collect_quotes_from_json(&ev.payload, &mut map);
        }
    }
    map
}

fn collect_quotes_from_json(val: &Value, map: &mut BTreeMap<String, String>) {
    match val {
        Value::Object(obj) => {
            if let Some(q) = obj.get("quote").and_then(Value::as_str) {
                if !q.trim().is_empty() {
                    map.entry(quote_sha256(q)).or_insert_with(|| q.to_string());
                }
            }
            for v in obj.values() {
                collect_quotes_from_json(v, map);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_quotes_from_json(v, map);
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn reverify_quote_list(
    sandbox: &WorkspaceSandbox,
    child_session: &str,
    child_receipts: &[Event],
    quote_lookup: &BTreeMap<String, String>,
    quotes: &[QuoteRef],
    location: &str,
    matched_count: &mut usize,
    dropped: &mut Vec<DroppedQuote>,
) -> Vec<QuoteRef> {
    let mut kept = Vec::new();
    for (idx, qref) in quotes.iter().enumerate() {
        let loc = format!("{location}.quotes[{idx}]");
        let candidate_text = quote_lookup
            .get(&qref.quote_sha256)
            .cloned()
            .or_else(|| read_matching_lines_from_child(sandbox, child_session, qref));

        let Some(text) = candidate_text else {
            dropped.push(DroppedQuote {
                location: loc,
                source_id: qref.source_id.clone(),
                path: qref.path.clone(),
                start_line: qref.start_line,
                end_line: qref.end_line,
                reason: "verbatim quote text unavailable or lines do not match quote_sha256 in child".into(),
            });
            continue;
        };

        match verify_quote_ref(sandbox, child_session, child_receipts, qref, &text) {
            Ok(verified) => {
                // Rule 1: parent evidence_seq never carries over to the child session.
                let mut clean = verified.quote_ref;
                clean.evidence_seq = None;
                *matched_count += 1;
                kept.push(clean);
            }
            Err(fault) => {
                dropped.push(DroppedQuote {
                    location: loc,
                    source_id: qref.source_id.clone(),
                    path: qref.path.clone(),
                    start_line: qref.start_line,
                    end_line: qref.end_line,
                    reason: fault.to_string(),
                });
            }
        }
    }
    kept
}

/// If the caller did not supply verbatim quote text separately, read lines `start_line..=end_line`
/// from the child's own copied file and accept it ONLY if its folded SHA-256 equals `qref.quote_sha256`.
fn read_matching_lines_from_child(
    sandbox: &WorkspaceSandbox,
    child_session: &str,
    qref: &QuoteRef,
) -> Option<String> {
    if qref.start_line == 0 || qref.end_line < qref.start_line {
        return None;
    }
    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, child_session, &qref.path, false)
        .ok()?;
    if !resolved.metadata.is_file() {
        return None;
    }
    let text = fs::read_to_string(&resolved.canonical_target).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    if qref.end_line > lines.len() {
        return None;
    }
    let slice = lines[qref.start_line - 1..qref.end_line].join("\n");
    (quote_sha256(&slice) == qref.quote_sha256).then_some(slice)
}

#[cfg(test)]
#[path = "handoff_tests.rs"]
mod tests;
