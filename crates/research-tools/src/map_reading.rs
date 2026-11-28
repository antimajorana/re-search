//! Investigate v2 reading/edge recorder (`record_map_reading`, package V6).

#[path = "map_reading_validate.rs"]
pub(crate) mod validate;

use crate::investigation::anchor::{bounded, valid_id};
use crate::investigation_markers::{
    active_investigation_slice, check_new_recorder_gate, InvestigationTaskKind, NewRecorderKind,
    EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON,
};
use crate::lines_sent::lines_sent_with_ledger;
use crate::map_defs::{
    record_map_reading_def, IndexedFault, MapReadingEventPayload, ReadingRejected,
    MAX_READING_EDGES, MAX_READING_ITEMS, MAX_READING_LIMITATIONS,
};
use crate::map_model_nodes::{
    ancestor_chain, collect_existing_model_nodes, is_model_descendant_of_any, ModelNodeRef,
};
use crate::skeleton::{PaperSkeleton, SkEdgeKind, SkNode, SkNodeKind};
use crate::{resolve_sources_session_id, ResearchEnv};
use ledger::{Event, Ledger};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use validate::{validate_edge, validate_item, ReadingAnchorReport, ValidationCtx};

const MAX_LIMITATION_CHARS: usize = 500;
const APP_ONLY_FIELDS: &[&str] = &[
    "excerpt_basis",
    "cited_range",
    "range_basis",
    "excerpt_window",
    "file_sha256",
    "quote_sha256",
    "excerpt",
    "cues",
    "node_fp",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecordArgs {
    request_seq: u64,
    source_id: String,
    items: Vec<Value>,
    edges: Vec<Value>,
    limitations: Vec<String>,
    done: bool,
}

pub(crate) fn reject_app_only_fields_with(value: &Value, extra: &[&str]) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            for &field in APP_ONLY_FIELDS.iter().chain(extra.iter()) {
                if map.contains_key(field) {
                    return Err(format!(
                        "Field `{field}` is set by the app and must not be supplied on input."
                    ));
                }
            }
            for child in map.values() {
                reject_app_only_fields_with(child, extra)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                reject_app_only_fields_with(child, extra)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_app_only_fields(value: &Value) -> Result<(), String> {
    reject_app_only_fields_with(value, &[])
}

pub(crate) fn load_pinned_skeleton(
    ledger: &Ledger,
    history: &[Event],
    marker: &Event,
    source_id: &str,
) -> Result<(u64, PaperSkeleton), String> {
    let skeleton_seq = marker.payload["skeleton_seq"]
        .as_u64()
        .filter(|&seq| seq >= 1)
        .ok_or("Active task marker is missing a valid `skeleton_seq`.")?;
    let skel_ev = history
        .iter()
        .find(|e| e.seq == skeleton_seq && e.kind == EVENT_INVESTIGATION_SKELETON)
        .ok_or_else(|| format!("Pinned `investigation.skeleton` event at seq {skeleton_seq} is missing."))?;
    if skel_ev.payload["source_id"].as_str() != Some(source_id) {
        return Err("Pinned skeleton does not belong to the requested `source_id`.".into());
    }
    let blob_hash = skel_ev.payload["blob"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Pinned `investigation.skeleton` event is missing its `blob` hash.")?;
    let bytes = ledger
        .get_blob(blob_hash)
        .map_err(|_| "Pinned skeleton blob could not be read from the ledger.".to_string())?;
    if ledger::sha256_hex(&bytes) != blob_hash {
        return Err("Pinned skeleton blob hash mismatch; refusing to record against corrupt skeleton.".into());
    }
    let skeleton: PaperSkeleton = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Pinned skeleton JSON could not be parsed: {e}"))?;
    if skeleton.source_id != source_id {
        return Err("Deserialized skeleton `source_id` does not match the pinned paper.".into());
    }
    Ok((skeleton_seq, skeleton))
}

pub(crate) fn compute_open_scopes(
    marker: &Event,
    skeleton: &PaperSkeleton,
    nodes_by_tag: &BTreeMap<&str, &SkNode>,
    model_nodes: &BTreeMap<String, ModelNodeRef>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut item_scope: BTreeSet<String> = BTreeSet::new();
    let mut owning_roots: BTreeSet<String> = BTreeSet::new();
    let sk_tags: BTreeSet<&str> = nodes_by_tag.keys().copied().collect();

    if let Some(tags) = marker.payload.get("tags").and_then(Value::as_array) {
        for raw in tags.iter().filter_map(Value::as_str) {
            if nodes_by_tag.contains_key(raw) {
                item_scope.insert(raw.to_string());
                owning_roots.insert(raw.to_string());
            } else if let Some(m) = model_nodes.get(raw) {
                item_scope.insert(raw.to_string());
                owning_roots.insert(m.root_skeleton_tag.clone());
            }
        }
    }
    loop {
        let mut added = false;
        for (&tag, node) in nodes_by_tag {
            if item_scope.contains(tag) {
                continue;
            }
            let via_parent = node.parent.as_deref().is_some_and(|p| item_scope.contains(p));
            let via_proof = node.kind == SkNodeKind::Proof
                && node.proof_of.as_deref().is_some_and(|t| item_scope.contains(t));
            if via_parent || via_proof {
                item_scope.insert(tag.to_string());
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let base_roots = item_scope.clone();
    for mtag in model_nodes.keys() {
        if is_model_descendant_of_any(mtag, &base_roots, model_nodes) {
            item_scope.insert(mtag.clone());
        }
    }

    let mut edge_scope = item_scope.clone();
    // Allow edges whose `from` is an ancestor along the parent chain of a requested `m-` node.
    for tag in &item_scope {
        if let Some(chain) = ancestor_chain(tag, &sk_tags, model_nodes) {
            for anc in chain {
                edge_scope.insert(anc);
            }
        }
    }
    for edge in &skeleton.edges {
        if edge.kind == SkEdgeKind::Uses
            && (item_scope.contains(edge.to.as_str()) || owning_roots.contains(edge.to.as_str()))
            && nodes_by_tag.contains_key(edge.from.as_str())
        {
            edge_scope.insert(edge.from.clone());
        }
    }
    (item_scope, edge_scope)
}

pub(crate) fn rejected_outcome(f: &IndexedFault) -> Value {
    let mut obj = json!({
        "index": f.index,
        "status": "rejected",
        "code": f.code,
        "message": f.message,
    });
    if let Some(true_text) = &f.true_text {
        obj["true_text"] = json!(true_text);
    }
    obj
}

pub fn record_map_reading(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    caller: &str,
    args: &Value,
) -> Result<Value, String> {
    reject_app_only_fields(args)?;
    resolve_sources_session_id(session).map_err(|e| e.to_string())?;
    tools::registry::check_args(&record_map_reading_def().input_schema, args)?;

    let parsed: RawRecordArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("Invalid `record_map_reading` arguments: {e}"))?;
    if parsed.request_seq == 0 {
        return Err("`request_seq` must be >= 1.".into());
    }
    if !valid_id(&parsed.source_id, 64) {
        return Err("Provide a valid `source_id`.".into());
    }
    if parsed.items.len() > MAX_READING_ITEMS {
        return Err(format!("At most {MAX_READING_ITEMS} items may be recorded per call."));
    }
    if parsed.edges.len() > MAX_READING_EDGES {
        return Err(format!("At most {MAX_READING_EDGES} edges may be recorded per call."));
    }
    if parsed.limitations.len() > MAX_READING_LIMITATIONS
        || parsed.limitations.iter().any(|s| !bounded(s, MAX_LIMITATION_CHARS))
    {
        return Err(format!(
            "Provide at most {MAX_READING_LIMITATIONS} non-empty limitations (<= {MAX_LIMITATION_CHARS} chars each)."
        ));
    }

    let (verified, events) = ledger
        .read_verified_session(session)
        .map_err(|_| "The Investigation ledger or its saved artifacts could not be verified.".to_string())?;
    let (history, _start) = active_investigation_slice(&events)?;
    let (marker, task_kind) =
        check_new_recorder_gate(history, NewRecorderKind::MapReading, parsed.request_seq)?;

    let paper = history
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.paper")
        .ok_or("Select and pin a paper before recording map readings.")?;
    if paper.payload["source_id"].as_str() != Some(parsed.source_id.as_str())
        || marker.payload["source_id"].as_str() != Some(parsed.source_id.as_str())
    {
        return Err("`source_id` must match the active task and pinned Investigation paper.".into());
    }

    let (skeleton_seq, skeleton) =
        load_pinned_skeleton(ledger, history, marker, &parsed.source_id)?;
    let receipt_primary_path = events
        .iter()
        .rev()
        .find(|e| {
            e.kind == "source.ingested"
                && e.payload["source_id"].as_str() == Some(parsed.source_id.as_str())
        })
        .and_then(|e| e.payload["primary_path"].as_str());

    let mut nodes_by_tag: BTreeMap<&str, &SkNode> = BTreeMap::new();
    let mut proofs_for_target: BTreeMap<&str, Vec<&SkNode>> = BTreeMap::new();
    for node in &skeleton.nodes {
        if let Some(tag) = node.tag.as_deref() {
            nodes_by_tag.insert(tag, node);
        }
        if node.kind == SkNodeKind::Proof {
            if let Some(target) = node.proof_of.as_deref() {
                proofs_for_target.entry(target).or_default().push(node);
            }
        }
    }
    let bib_keys: BTreeSet<&str> = skeleton.bib.iter().map(|b| b.key.as_str()).collect();
    let model_nodes = collect_existing_model_nodes(&skeleton, &events);
    let (open_item_scope, open_edge_scope) = if task_kind == InvestigationTaskKind::Open {
        let (is, es) = compute_open_scopes(marker, &skeleton, &nodes_by_tag, &model_nodes);
        (Some(is), Some(es))
    } else {
        (None, None)
    };

    let marker_global_idx = events
        .iter()
        .position(|e| e.seq == marker.seq)
        .ok_or("Active task marker is missing from session events.")?;
    let task_events = &events[marker_global_idx..];
    let req_seq_str = parsed.request_seq.to_string();
    let sent = lines_sent_with_ledger(task_events, Some(ledger), Some(&req_seq_str));

    let mut ctx = ValidationCtx {
        sandbox: env.sandbox(),
        session,
        caller,
        source_id: &parsed.source_id,
        receipt_primary_path,
        events: &events,
        sent: &sent,
        skeleton: &skeleton,
        nodes_by_tag,
        proofs_for_target,
        bib_keys,
        model_nodes,
        temp_id_map: BTreeMap::new(),
        open_item_scope,
        open_edge_scope,
    };

    let mut accepted_items = Vec::new();
    let mut accepted_edges = Vec::new();
    let mut item_outcomes = Vec::with_capacity(parsed.items.len());
    let mut edge_outcomes = Vec::with_capacity(parsed.edges.len());
    let mut assigned_tags: BTreeMap<String, String> = BTreeMap::new();
    let mut rejected = ReadingRejected::default();
    let mut report = ReadingAnchorReport::default();

    for (idx, raw_item) in parsed.items.into_iter().enumerate() {
        match validate_item(&mut ctx, idx, raw_item, &mut report) {
            Ok(validated) => {
                let rec = validated.record;
                let mut outcome = json!({ "index": idx, "status": "accepted", "tag": rec.tag });
                if let Some(tid) = validated.temp_id {
                    assigned_tags.insert(tid.clone(), rec.tag.clone());
                    outcome["temp_id"] = json!(tid);
                }
                if let Some(ref parent) = rec.attach_to {
                    outcome["attach_to"] = json!(parent);
                }
                if let Some(ref mk) = rec.model_kind {
                    outcome["kind"] = json!(mk);
                }
                item_outcomes.push(outcome);
                accepted_items.push(rec);
            }
            Err(fault) => {
                item_outcomes.push(rejected_outcome(&fault));
                rejected.items.push(fault);
            }
        }
    }
    for (idx, raw_edge) in parsed.edges.into_iter().enumerate() {
        match validate_edge(&ctx, idx, raw_edge, &mut report) {
            Ok(rec) => {
                edge_outcomes.push(json!({
                    "index": idx,
                    "status": "accepted",
                    "from": rec.from,
                    "to": rec.to,
                    "role": rec.role,
                }));
                accepted_edges.push(rec);
            }
            Err(fault) => {
                edge_outcomes.push(rejected_outcome(&fault));
                rejected.edges.push(fault);
            }
        }
    }

    let anchor_report_val = report.to_value();
    let accepted_items_len = accepted_items.len();
    let accepted_edges_len = accepted_edges.len();
    let has_non_empty_limitations = parsed.limitations.iter().any(|s| !s.trim().is_empty());
    let should_append = !accepted_items.is_empty()
        || !accepted_edges.is_empty()
        || (parsed.done && has_non_empty_limitations);

    let reading_seq = if should_append {
        let limitations = parsed.limitations.into_iter().map(|s| s.trim().to_string()).collect();
        let payload = MapReadingEventPayload {
            schema_version: 1,
            request_seq: parsed.request_seq,
            source_id: parsed.source_id.clone(),
            skeleton_seq,
            done: parsed.done,
            items: accepted_items,
            edges: accepted_edges,
            limitations,
            rejected: rejected.clone(),
            anchor_report: anchor_report_val.clone(),
        };
        let payload_val = serde_json::to_value(&payload).map_err(|e| e.to_string())?;
        let appended = ledger
            .append_at_head(
                session,
                EVENT_INVESTIGATION_READING,
                1,
                &payload_val,
                &verified.head_hash,
            )
            .map_err(|e| e.to_string())?;
        Some(appended.seq)
    } else {
        None
    };

    let status = if reading_seq.is_some() { "recorded" } else { "no_valid_records" };
    Ok(json!({
        "status": status,
        "reading_seq": reading_seq,
        "request_seq": parsed.request_seq,
        "source_id": parsed.source_id,
        "skeleton_seq": skeleton_seq,
        "done": parsed.done,
        "accepted_items": accepted_items_len,
        "accepted_edges": accepted_edges_len,
        "assigned_tags": assigned_tags,
        "items": item_outcomes,
        "edges": edge_outcomes,
        "rejected_items": rejected.items,
        "rejected_edges": rejected.edges,
        "anchor_report": anchor_report_val,
    }))
}

#[cfg(test)]
#[path = "map_reading_tests.rs"]
mod tests;
