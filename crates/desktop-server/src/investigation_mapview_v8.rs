//! V8 model-created `m-` node and model-edge projection helpers for `investigation_mapview_v7`
//! (extracted cleanly so every source module stays <= 600 lines).

use crate::investigation_mapview_v7::{
    collect_field_changes, enclosing_section_tag, CHANGED_SINCE_LAST_TEXT,
};
use research_tools::map_fold::{FoldedMap, FoldedNodeReading};
use research_tools::skeleton::{PaperSkeleton, SkNode};
use research_tools::status_badge::BORDER_DASHED;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Appends folded V8 `m-` model nodes and model edges involving `m-` nodes to `mv`,
/// stripping raw excerpts so no unrequested paper text leaks into mapview JSON.
pub(crate) fn append_model_nodes_and_edges(
    mv: &mut Value,
    skeleton: &PaperSkeleton,
    folded: &FoldedMap,
    sk_by_tag: &BTreeMap<&str, &SkNode>,
    dimmed_secs: &BTreeSet<&str>,
    sk_groups: &BTreeMap<String, Value>,
    existing_node_ids: &mut BTreeSet<String>,
) {
    if let Some(arr) = mv["nodes"].as_array_mut() {
        // Append V8 model-created `m-` nodes (ordered stably by reading seq) with excerpts stripped.
        let mut model_entries: Vec<&FoldedNodeReading> = folded
            .nodes
            .values()
            .filter(|r| r.is_model_node && !existing_node_ids.contains(&r.tag))
            .collect();
        model_entries.sort_by_key(|r| first_reading_seq(r));
        for r in model_entries {
            let root_tag = resolve_root_skeleton_tag(&r.tag, sk_by_tag, &folded.nodes);
            let root_sk = root_tag.and_then(|rt| sk_by_tag.get(rt).copied());
            let group = root_tag
                .and_then(|rt| sk_groups.get(rt).cloned())
                .unwrap_or(Value::Null);
            let dim = !dimmed_secs.is_empty()
                && enclosing_section_tag(root_sk, sk_by_tag, skeleton.nodes.len())
                    .is_some_and(|st| dimmed_secs.contains(st));
            let node_val = build_model_node_json(r, root_tag, root_sk, group, dim);
            existing_node_ids.insert(r.tag.clone());
            arr.push(node_val);
        }
    }

    // Append folded model edges involving `m-` nodes (skeleton-to-skeleton edges are already emitted).
    if let Some(edges_arr) = mv["edges"].as_array_mut() {
        for me in &folded.edges {
            let involves_model = !sk_by_tag.contains_key(me.from.as_str())
                || !sk_by_tag.contains_key(me.to.as_str());
            if involves_model
                && existing_node_ids.contains(&me.from)
                && existing_node_ids.contains(&me.to)
            {
                let mut edge_val = json!({
                    "src": me.from,
                    "dst": me.to,
                    "kind": "uses",
                    "role": me.role,
                    "basis": "model",
                    "model_edge": true,
                    "edge_basis": me.current.value.basis,
                    "reason": me.current.value.reason,
                    "anchor": me.current.value.anchor,
                    "history": me.history,
                });
                strip_excerpts(&mut edge_val);
                edges_arr.push(edge_val);
            }
        }
    }
}

pub(crate) fn strip_excerpts(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.remove("excerpt");
            for child in map.values_mut() {
                strip_excerpts(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_excerpts(child);
            }
        }
        _ => {}
    }
}

fn build_model_node_json(
    r: &FoldedNodeReading,
    root_tag: Option<&str>,
    root_sk: Option<&SkNode>,
    group: Value,
    dim: bool,
) -> Value {
    let kind = r.model_kind.as_deref().unwrap_or("proof_step");
    let gist = r.gist.current.as_ref().map(|v| v.value.as_str()).unwrap_or("");
    let label = if !gist.is_empty() {
        let mut s: String = gist.chars().take(72).collect();
        if gist.chars().count() > 72 {
            s.push('…');
        }
        s
    } else {
        format!("{} ({})", kind.replace('_', " "), r.tag)
    };
    let fp = root_sk.map_or("", |n| n.fp.as_str());
    let spans = root_sk.map(|n| json!(n.spans)).unwrap_or_else(|| json!([]));
    let mut reading_val = serde_json::to_value(r).unwrap_or(Value::Null);
    strip_excerpts(&mut reading_val);

    let mut obj = json!({
        "id": r.tag,
        "kind": kind,
        "model_kind": kind,
        "model_node": true,
        "attach_to": r.attach_to,
        "parent": r.attach_to,
        "root_skeleton_tag": root_tag,
        "label": label,
        "group": group,
        "grade": "model",
        "badges": [{
            "text": format!("{} (model)", kind.replace('_', " ")),
            "icon": "model",
            "tone": "muted",
            "border": BORDER_DASHED,
        }],
        "flags": [],
        "stale": r.any_stale,
        "dim": dim,
        "fp": fp,
        "spans": spans,
        "children": r.children,
        "reading": reading_val,
    });
    if !gist.is_empty() {
        obj["gist"] = json!(gist);
    }
    if let Some(p) = &r.purpose.current {
        obj["purpose"] = json!(p.value);
    }
    if let Some(hyps) = &r.hypotheses.current {
        let mut hv = serde_json::to_value(&hyps.value).unwrap_or(Value::Null);
        strip_excerpts(&mut hv);
        obj["hypotheses"] = hv;
    }
    if let Some(anch) = &r.anchors.current {
        let mut av = serde_json::to_value(&anch.value).unwrap_or(Value::Null);
        strip_excerpts(&mut av);
        obj["anchors"] = av;
    }
    if !r.notes.is_empty() {
        obj["notes"] = json!(r.notes);
    }
    let changes = collect_field_changes(r);
    if !changes.is_empty() {
        obj["changed_since_last"] = json!({
            "label": CHANGED_SINCE_LAST_TEXT,
            "changes": changes,
        });
    }
    obj
}

fn first_reading_seq(r: &FoldedNodeReading) -> u64 {
    r.anchors
        .history
        .first()
        .or(r.anchors.current.as_ref())
        .map(|v| v.seq)
        .or_else(|| r.gist.history.first().or(r.gist.current.as_ref()).map(|v| v.seq))
        .unwrap_or(u64::MAX)
}

fn resolve_root_skeleton_tag<'a>(
    tag: &'a str,
    sk_by_tag: &BTreeMap<&'a str, &'a SkNode>,
    folded: &'a BTreeMap<String, FoldedNodeReading>,
) -> Option<&'a str> {
    let mut cur = tag;
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for _ in 0..64 {
        if let Some((&sk_tag, _)) = sk_by_tag.get_key_value(cur) {
            return Some(sk_tag);
        }
        if !seen.insert(cur) {
            return None;
        }
        cur = folded.get(cur)?.attach_to.as_deref()?;
    }
    None
}
