//! Investigate mode V9 read-side legacy adapter (`investigate.md` §0.2 #6, §4.4, §8, §9 V9).
//!
//! Reads legacy Investigation ledger events (`investigation.map`, `investigation.branch`,
//! `investigation.summary`, `investigation.conventions`, `investigation.correction`,
//! `investigation.question_requested`/`answer`, `investigation.requested`/`result`) with zero
//! ledger mutations and zero model calls, mapping nodes to `legacy:<seq>:<id>` and attaching
//! them onto the innermost enclosing [`PaperSkeleton`] node under `"Earlier map"`, or collecting
//! unplaced / fallback-mode entries under `"Unplaced (older map)"`.

pub use crate::investigation_legacy_model::*;

use crate::investigation::{latest, list};
use crate::investigation_legacy_model::{
    match_innermost_skeleton_node, opt_str, parse_legacy_anchor, strip_disallowed_keys,
    string_list,
};
use crate::investigation_skeleton::{ensure_skeleton_on_open, fallback_note, open_and_read};
use crate::routes_session::ApiResult;
use crate::state::DesktopState;
use ledger::Event;
use research_tools::investigation::{expanded_map, paper_conventions};
use research_tools::skeleton::PaperSkeleton;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

struct CollectedMapInputs {
    source_id: Option<String>,
    map_seq: Option<u64>,
    title: Option<String>,
    coverage: Option<String>,
    limitations: Vec<String>,
    answer_node_ids: Vec<String>,
    raw_nodes: Vec<(u64, &'static str, Value)>,
    raw_edges: Vec<(u64, Value)>,
    branches: Vec<Value>,
}

/// Pure read-side adapter projecting legacy Investigation ledger records onto an optional
/// deterministic [`PaperSkeleton`]. Makes zero ledger writes and zero model calls.
pub fn adapt_legacy_investigation(
    events: &[Event],
    skeleton: Option<&PaperSkeleton>,
) -> LegacyMapProjection {
    let inputs = collect_map_inputs(events);
    let summary = latest(events, "investigation.summary").map(|ev| {
        let mut val = ev.payload.clone();
        val["seq"] = json!(ev.seq);
        strip_disallowed_keys(&mut val);
        val
    });
    let conventions = extract_conventions(events);
    let (activities, answers) = fold_activities_and_answers(events);

    let BuiltLegacyNodes {
        mut nodes,
        raw_id_to_legacy,
        node_to_tag,
    } = build_legacy_nodes(&inputs.raw_nodes, skeleton);
    let corrections = build_corrections(events, &raw_id_to_legacy, &node_to_tag);
    attach_corrections_to_nodes(&mut nodes, &corrections);

    let edges = build_legacy_edges(&inputs.raw_edges, &raw_id_to_legacy, &node_to_tag);
    attach_edges_to_nodes(&mut nodes, &edges);

    let mut placed: BTreeMap<String, Vec<LegacyNode>> = BTreeMap::new();
    let mut unplaced = Vec::new();
    for node in &nodes {
        if let Some(tag) = node.placed_tag.as_ref() {
            placed.entry(tag.clone()).or_default().push(node.clone());
        } else {
            unplaced.push(node.clone());
        }
    }

    let answer_node_ids: Vec<String> = inputs
        .answer_node_ids
        .into_iter()
        .map(|raw| raw_id_to_legacy.get(&raw).cloned().unwrap_or(raw))
        .collect();

    let has_legacy = !nodes.is_empty()
        || !edges.is_empty()
        || summary.is_some()
        || conventions.is_some()
        || !corrections.is_empty()
        || !inputs.branches.is_empty()
        || !activities.is_empty()
        || !answers.is_empty();

    let fallback_old_view = skeleton.is_none();
    let diagnostic_note = fallback_old_view.then(|| fallback_note(events).to_string());

    LegacyMapProjection {
        has_legacy,
        fallback_old_view,
        diagnostic_note,
        source_id: inputs.source_id,
        map_seq: inputs.map_seq,
        title: inputs.title,
        coverage: inputs.coverage,
        limitations: inputs.limitations,
        answer_node_ids,
        nodes,
        placed,
        unplaced,
        edges,
        branches: inputs.branches,
        corrections,
        summary,
        conventions,
        activities,
        answers,
    }
}

/// Attaches the V9 legacy projection onto an existing `GraphView` mapview JSON object.
/// For pure V2 sessions (`!has_legacy`), sets `mv["legacy"] = Value::Null`.
pub fn embed_legacy_into_mapview(
    mv: &mut Value,
    events: &[Event],
    skeleton: Option<&PaperSkeleton>,
) {
    let projection = adapt_legacy_investigation(events, skeleton);
    if !projection.has_legacy {
        mv["legacy"] = Value::Null;
        return;
    }
    if let Some(arr) = mv.get_mut("nodes").and_then(Value::as_array_mut) {
        for item in arr.iter_mut() {
            let Some(tag) = item["id"].as_str() else {
                continue;
            };
            if let Some(legacy_nodes) = projection.placed.get(tag) {
                item["earlier_map"] = serde_json::to_value(legacy_nodes).unwrap_or(Value::Null);
            }
        }
    }
    mv["legacy"] = serde_json::to_value(&projection).unwrap_or(Value::Null);
}

/// Handler for `GET /api/sessions/{id}/investigation/legacy`.
/// Ensures a deterministic skeleton is attached on open when source hashes still match,
/// otherwise leaves the ledger unchanged and returns `fallback_old_view: true`.
pub fn handle_get_legacy(state: &DesktopState, session_id: &str) -> ApiResult {
    let rec = ensure_skeleton_on_open(state, session_id)?;
    let (_, events) = open_and_read(state, session_id)?;
    let skel = rec.as_ref().map(|r| &r.skeleton);
    let projection = adapt_legacy_investigation(&events, skel);
    let body = serde_json::to_value(&projection)
        .map_err(|err| (500, format!("failed to serialize legacy projection: {err}")))?;
    Ok((200, body))
}

fn collect_map_inputs(events: &[Event]) -> CollectedMapInputs {
    let mut raw_nodes = Vec::new();
    let mut raw_edges = Vec::new();
    let mut branches = Vec::new();
    let mut absorbed_branches = BTreeSet::new();

    let mut source_id = None;
    let mut map_seq = None;
    let mut title = None;
    let mut coverage = None;
    let mut limitations = Vec::new();
    let mut answer_node_ids = Vec::new();

    if let Some(base_map) = latest(events, "investigation.map") {
        let expanded = expanded_map(events, base_map);
        source_id = opt_str(&expanded["source_id"]);
        map_seq = Some(base_map.seq);
        title = opt_str(&expanded["title"]);
        coverage = opt_str(&expanded["coverage"]);
        limitations = string_list(&expanded["limitations"]);
        answer_node_ids = string_list(&expanded["answer_node_ids"]);

        for n in list(&base_map.payload["nodes"]) {
            raw_nodes.push((base_map.seq, "map", n.clone()));
        }
        for e in list(&base_map.payload["edges"]) {
            raw_edges.push((base_map.seq, e.clone()));
        }
        for b in list(&expanded["branches"]) {
            branches.push(b.clone());
            if let Some(b_seq) = b["seq"].as_u64() {
                absorbed_branches.insert(b_seq);
                if let Some(b_ev) = events
                    .iter()
                    .find(|e| e.seq == b_seq && e.kind == "investigation.branch")
                {
                    collect_branch_items(b_ev, &mut raw_nodes, &mut raw_edges);
                }
            }
        }
    }

    for ev in events.iter().filter(|e| e.kind == "investigation.branch") {
        if absorbed_branches.contains(&ev.seq) {
            continue;
        }
        if source_id.is_none() {
            source_id = opt_str(&ev.payload["source_id"]);
        }
        collect_branch_items(ev, &mut raw_nodes, &mut raw_edges);
        let node_ids: Vec<Value> = list(&ev.payload["nodes"])
            .iter()
            .map(|n| n["id"].clone())
            .collect();
        branches.push(json!({
            "seq": ev.seq,
            "request_seq": ev.payload["request_seq"],
            "node_id": ev.payload["node_id"],
            "question": ev.payload["question"],
            "limitations": ev.payload["limitations"],
            "node_ids": node_ids,
        }));
    }

    CollectedMapInputs {
        source_id,
        map_seq,
        title,
        coverage,
        limitations,
        answer_node_ids,
        raw_nodes,
        raw_edges,
        branches,
    }
}

fn collect_branch_items(
    ev: &Event,
    raw_nodes: &mut Vec<(u64, &'static str, Value)>,
    raw_edges: &mut Vec<(u64, Value)>,
) {
    for n in list(&ev.payload["nodes"]) {
        raw_nodes.push((ev.seq, "branch", n.clone()));
    }
    for e in list(&ev.payload["edges"]) {
        raw_edges.push((ev.seq, e.clone()));
    }
}

struct BuiltLegacyNodes {
    nodes: Vec<LegacyNode>,
    raw_id_to_legacy: HashMap<String, String>,
    node_to_tag: HashMap<String, Option<String>>,
}

fn build_legacy_nodes(
    raw_nodes: &[(u64, &'static str, Value)],
    skeleton: Option<&PaperSkeleton>,
) -> BuiltLegacyNodes {
    let mut nodes = Vec::with_capacity(raw_nodes.len());
    let mut raw_id_to_legacy = HashMap::new();
    let mut node_to_tag = HashMap::new();

    for (seq, origin_kind, val) in raw_nodes {
        let raw_id = val["id"].as_str().unwrap_or("").trim().to_string();
        if raw_id.is_empty() {
            continue;
        }
        let legacy_id = format!("{LEGACY_NODE_PREFIX}{seq}:{raw_id}");
        let anchors: Vec<LegacyAnchor> = list(&val["anchors"]).iter().map(parse_legacy_anchor).collect();
        let placed_tag = skeleton.and_then(|sk| match_innermost_skeleton_node(sk, &anchors));
        raw_id_to_legacy.insert(raw_id.clone(), legacy_id.clone());
        node_to_tag.insert(legacy_id.clone(), placed_tag.clone());
        nodes.push(LegacyNode {
            id: legacy_id,
            raw_id,
            origin_seq: *seq,
            origin_kind: (*origin_kind).to_string(),
            sentence: val["sentence"].as_str().unwrap_or("").to_string(),
            kind: val["kind"].as_str().unwrap_or("claim").to_string(),
            main: val["main"].as_bool().unwrap_or(false),
            questions: string_list(&val["questions"]),
            anchors,
            placed_tag,
            corrections: Vec::new(),
            edges: Vec::new(),
        });
    }
    BuiltLegacyNodes { nodes, raw_id_to_legacy, node_to_tag }
}

fn build_corrections(
    events: &[Event],
    raw_id_to_legacy: &HashMap<String, String>,
    node_to_tag: &HashMap<String, Option<String>>,
) -> Vec<LegacyCorrectionNote> {
    let mut out = Vec::new();
    for ev in events.iter().filter(|e| e.kind == "investigation.correction") {
        let map_seq = ev.payload["map_seq"].as_u64();
        let raw_node_id = ev.payload["node_id"].as_str().unwrap_or("").trim().to_string();
        let raw_edge_from = opt_str(&ev.payload["edge_from"]);
        let node_id = resolve_node_ref(&raw_node_id, map_seq, raw_id_to_legacy, node_to_tag);
        let edge_from = raw_edge_from
            .as_deref()
            .and_then(|r| resolve_node_ref(r, map_seq, raw_id_to_legacy, node_to_tag));
        let placed_tag = node_id
            .as_ref()
            .and_then(|nid| node_to_tag.get(nid).cloned())
            .flatten();
        out.push(LegacyCorrectionNote {
            seq: ev.seq,
            map_seq,
            raw_node_id,
            node_id,
            raw_edge_from,
            edge_from,
            placed_tag,
            text: ev.payload["text"].as_str().unwrap_or("").to_string(),
        });
    }
    out
}

fn resolve_node_ref(
    raw_id: &str,
    preferred_seq: Option<u64>,
    raw_id_to_legacy: &HashMap<String, String>,
    node_to_tag: &HashMap<String, Option<String>>,
) -> Option<String> {
    if raw_id.is_empty() {
        return None;
    }
    if let Some(seq) = preferred_seq {
        let exact = format!("{LEGACY_NODE_PREFIX}{seq}:{raw_id}");
        if node_to_tag.contains_key(&exact) {
            return Some(exact);
        }
    }
    raw_id_to_legacy.get(raw_id).cloned()
}

fn attach_corrections_to_nodes(nodes: &mut [LegacyNode], corrections: &[LegacyCorrectionNote]) {
    for note in corrections {
        if let Some(nid) = note.node_id.as_deref() {
            if let Some(node) = nodes.iter_mut().find(|n| n.id == nid) {
                node.corrections.push(note.clone());
            }
        }
    }
}

fn build_legacy_edges(
    raw_edges: &[(u64, Value)],
    raw_id_to_legacy: &HashMap<String, String>,
    node_to_tag: &HashMap<String, Option<String>>,
) -> Vec<LegacyEdge> {
    let mut out = Vec::with_capacity(raw_edges.len());
    for (idx, (seq, val)) in raw_edges.iter().enumerate() {
        let raw_from = val["from"].as_str().unwrap_or("").trim().to_string();
        let raw_to = val["to"].as_str().unwrap_or("").trim().to_string();
        if raw_from.is_empty() || raw_to.is_empty() {
            continue;
        }
        let src = resolve_node_ref(&raw_from, Some(*seq), raw_id_to_legacy, node_to_tag)
            .unwrap_or_else(|| format!("{LEGACY_NODE_PREFIX}{seq}:{raw_from}"));
        let dst = resolve_node_ref(&raw_to, Some(*seq), raw_id_to_legacy, node_to_tag)
            .unwrap_or_else(|| format!("{LEGACY_NODE_PREFIX}{seq}:{raw_to}"));
        let raw_kind = val["kind"].as_str().unwrap_or("depends_on").trim();
        let kind = if raw_kind.is_empty() || raw_kind.eq_ignore_ascii_case("proof") {
            "depends_on".to_string()
        } else {
            raw_kind.to_string()
        };
        let mut anchor = val.get("anchor").cloned().unwrap_or(Value::Null);
        strip_disallowed_keys(&mut anchor);
        out.push(LegacyEdge {
            id: format!("legacy-edge:{seq}:{idx}"),
            src_tag: node_to_tag.get(&src).cloned().flatten(),
            dst_tag: node_to_tag.get(&dst).cloned().flatten(),
            src,
            dst,
            raw_from,
            raw_to,
            kind,
            role: LEGACY_EDGE_ROLE.to_string(),
            basis: LEGACY_EDGE_BASIS.to_string(),
            dashed: true,
            legacy_basis: opt_str(&val["basis"]),
            reason: val["reason"].as_str().unwrap_or("").to_string(),
            anchor,
        });
    }
    out
}

fn attach_edges_to_nodes(nodes: &mut [LegacyNode], edges: &[LegacyEdge]) {
    for node in nodes.iter_mut() {
        for edge in edges {
            if edge.src != node.id && edge.dst != node.id {
                continue;
            }
            let incoming = edge.dst == node.id;
            node.edges.push(LegacyNodeEdge {
                id: edge.id.clone(),
                direction: if incoming { "incoming" } else { "outgoing" }.to_string(),
                src: edge.src.clone(),
                dst: edge.dst.clone(),
                peer_id: if incoming { &edge.src } else { &edge.dst }.clone(),
                peer_tag: if incoming { &edge.src_tag } else { &edge.dst_tag }.clone(),
                kind: edge.kind.clone(),
                role: edge.role.clone(),
                basis: edge.basis.clone(),
                dashed: edge.dashed,
                reason: edge.reason.clone(),
            });
        }
    }
}

fn extract_conventions(events: &[Event]) -> Option<Value> {
    let latest_conv = latest(events, "investigation.conventions")?;
    let mut val = latest_conv.payload["request_seq"]
        .as_u64()
        .and_then(|seq| paper_conventions(events, seq))
        .unwrap_or_else(|| latest_conv.payload.clone());
    strip_disallowed_keys(&mut val);
    Some(val)
}

fn fold_activities_and_answers(events: &[Event]) -> (Vec<Value>, Vec<Value>) {
    let mut activities = Vec::new();
    let mut positions = HashMap::new();
    let mut answers = Vec::new();
    let mut answer_positions = HashMap::new();

    for ev in events {
        match ev.kind.as_str() {
            "investigation.requested" => {
                let mut row = ev.payload.clone();
                strip_disallowed_keys(&mut row);
                row["seq"] = json!(ev.seq);
                row["status"] = json!("interrupted");
                positions.insert(ev.seq, activities.len());
                activities.push(row);
            }
            "investigation.result" => {
                if let Some(&idx) = ev.payload["request_seq"].as_u64().and_then(|s| positions.get(&s)) {
                    let row = &mut activities[idx];
                    row["status"] = ev.payload["status"].clone();
                    row["result"] = ev.payload["text"].clone();
                    row["end_seq"] = json!(ev.seq);
                }
            }
            "investigation.question_requested" => {
                let row = json!({
                    "seq": ev.seq,
                    "question": ev.payload["question"],
                    "status": "interrupted",
                    "text": Value::Null,
                    "error": Value::Null,
                    "cites": [],
                });
                answer_positions.insert(ev.seq, answers.len());
                answers.push(row);
            }
            "investigation.answer" => {
                if let Some(&idx) = ev.payload["request_seq"].as_u64().and_then(|s| answer_positions.get(&s)) {
                    let row = &mut answers[idx];
                    row["status"] = ev.payload["status"].clone();
                    row["text"] = ev.payload["text"].clone();
                    row["error"] = ev.payload["error"].clone();
                    let mut cites = ev.payload.get("cites").cloned().unwrap_or_else(|| json!([]));
                    strip_disallowed_keys(&mut cites);
                    row["cites"] = cites;
                }
            }
            _ => {}
        }
    }
    (activities, answers)
}

#[cfg(test)]
#[path = "investigation_legacy_view_tests.rs"]
mod tests;
