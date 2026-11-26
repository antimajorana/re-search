//! V7/V8/V10 `mapview` enrichment helpers (sibling of `investigation_mapview.rs`, kept <= 600 lines).
//!
//! Projects folded Layer-2 `role`, `pin`/`pinned`, `gist`, `purpose`, `authors_claim` (computed
//! via `status_badge::status_badge`), `notes`, additive change history (`changed_since_last`),
//! and recomputed V5 `skip_plan` + section dimming (Rule 6.14) onto the `mapview` JSON so
//! V13 (`investigation_wholemap_model.js`), V7 badges, V8 Block B, and V10 notes light up
//! consistently.

use research_tools::map_defs::ModelEdgeRole;
use research_tools::map_fold::{FoldedMap, FoldedNodeReading, ReaderPin};
use research_tools::skeleton::{
    compute_skip_plan, ExtraUseEdge, PaperSkeleton, SkEdgeRole, SkNode, SkNodeKind,
    SKIP_LABEL_UNREFERENCED,
};
use research_tools::status_badge::{status_badge, BORDER_DASHED, BORDER_SOLID};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Badge text when a node is marked as a main result by the model (`investigate.md` §2, §3).
pub const BADGE_MAIN_RESULT_MODEL: &str = "Main result (model)";
/// Badge text when a node is pinned as a main result by the reader (`investigate.md` §4.3; `00-overview.md` §2.10).
pub const BADGE_MAIN_RESULT_USER: &str = "Main result (you)";
/// Indicator wording for additive re-run diffs (`investigate.md` §0.3 #4, rule 6.12).
pub const CHANGED_SINCE_LAST_TEXT: &str = "Changed since you last opened this";

/// Enriches a `mapview` JSON produced by `build_mapview_json_with_fold` with V7 authors' badges,
/// reader pins, effective main-result flags, additive diff indicators, reader notes, Orient
/// auto-start setting, and a recomputed V5 `skip_plan` honoring Rule 6.14.
pub fn enrich_mapview_with_v7(
    mv: &mut Value,
    skeleton: &PaperSkeleton,
    folded: &FoldedMap,
    orient_auto_start: bool,
) {
    let sk_by_tag: BTreeMap<&str, &SkNode> = skeleton
        .nodes
        .iter()
        .filter_map(|n| n.tag.as_deref().map(|t| (t, n)))
        .collect();

    // Collect effective main results (reader pins override model role, investigate.md §4.3).
    let mut main_result_tags: Vec<String> = Vec::new();
    for node in &skeleton.nodes {
        let Some(tag) = node.tag.as_deref() else {
            continue;
        };
        if folded
            .nodes
            .get(tag)
            .is_some_and(|r| r.effective_is_main_result)
        {
            main_result_tags.push(tag.to_string());
        }
    }

    // Build extra use edges from folded model edges for Rule 6.14 skip-plan honesty.
    let extra_edges: Vec<ExtraUseEdge> = folded
        .edges
        .iter()
        .filter(|e| {
            sk_by_tag.contains_key(e.from.as_str()) && sk_by_tag.contains_key(e.to.as_str())
        })
        .map(|e| ExtraUseEdge {
            from: e.from.clone(),
            to: e.to.clone(),
            role: match e.role {
                ModelEdgeRole::Statement => SkEdgeRole::Statement,
                ModelEdgeRole::Proof | ModelEdgeRole::Argument => SkEdgeRole::Proof,
            },
            model_inferred: true,
        })
        .collect();

    let plan = compute_skip_plan(skeleton, &main_result_tags, &extra_edges);
    let dimmed_secs: BTreeSet<&str> = if plan.empty {
        BTreeSet::new()
    } else {
        plan.sections
            .iter()
            .filter(|s| s.dim)
            .map(|s| s.section_tag.as_str())
            .collect()
    };

    if let Some(arr) = mv["nodes"].as_array_mut() {
        for item in arr.iter_mut() {
            let id = item["id"].as_str().unwrap_or("").to_string();
            let sk_node = sk_by_tag.get(id.as_str()).copied();
            let reading = folded.nodes.get(&id);
            enrich_one_skeleton_node(item, sk_node, reading, &sk_by_tag, &dimmed_secs, skeleton.nodes.len());
        }
    }

    mv["main_result_tags"] = json!(main_result_tags);
    mv["skip_plan"] = json!(plan);
    mv["orient_auto_start"] = json!(orient_auto_start);
}

fn enrich_one_skeleton_node(
    item: &mut Value,
    sk_node: Option<&SkNode>,
    reading: Option<&FoldedNodeReading>,
    sk_by_tag: &BTreeMap<&str, &SkNode>,
    dimmed_secs: &BTreeSet<&str>,
    max_hops: usize,
) {
    let head_cites: &[String] = sk_node.map_or(&[], |n| n.head_cites.as_slice());

    if let Some(r) = reading {
        if let Some(role_vv) = &r.role.current {
            if !role_vv.stale {
                item["role"] = json!(role_vv.value);
            }
        }
        if let Some(pin) = r.pin {
            item["pin"] = json!(pin);
            item["pinned"] = json!(pin == ReaderPin::MainResult);
            if pin == ReaderPin::NotMainResult {
                item["unpinned"] = json!(true);
            }
        }
        item["effective_is_main_result"] = json!(r.effective_is_main_result);

        if let Some(gist_vv) = &r.gist.current {
            item["gist"] = json!(gist_vv.value);
        }
        if let Some(purpose_vv) = &r.purpose.current {
            item["purpose"] = json!(purpose_vv.value);
        }

        // Pure authors'-side badge with deterministic cues and heading-cite line (rule 6.6).
        let ac_current = r.authors_claim.current.as_ref().map(|v| &v.value);
        let badge = status_badge(ac_current, head_cites);
        if badge.present {
            let mut ac_val = serde_json::to_value(&badge).unwrap_or(Value::Null);
            if let Some(prev) = r.authors_claim.history.last() {
                if let Some(obj) = ac_val.as_object_mut() {
                    obj.insert("previous_label".into(), json!(prev.value.label));
                    obj.insert("changed".into(), json!(true));
                }
            }
            item["authors_claim"] = ac_val;
        }

        // Notes array preserved cleanly for V10 UI, F15 write-up, and V16 hand-off.
        if !r.notes.is_empty() {
            item["notes"] = json!(r.notes);
        }

        // Additive history diff summary (rule 6.12).
        let changes = collect_field_changes(r);
        if !changes.is_empty() {
            item["changed_since_last"] = json!({
                "label": CHANGED_SINCE_LAST_TEXT,
                "changes": changes,
            });
        }

        // Append main-result and authors'-claim badges so V13 and node cards stay in sync.
        if let Some(badges) = item["badges"].as_array_mut() {
            if r.effective_is_main_result {
                let (text, icon, tone, border) = if r.pin == Some(ReaderPin::MainResult) {
                    (BADGE_MAIN_RESULT_USER, "pin", "ok", BORDER_SOLID)
                } else {
                    (BADGE_MAIN_RESULT_MODEL, "model", "info", BORDER_DASHED)
                };
                badges.push(json!({
                    "text": text, "icon": icon, "tone": tone, "border": border,
                }));
            }
            if badge.present {
                badges.push(json!({
                    "text": badge.text,
                    "icon": badge.icon,
                    "tone": badge.tone,
                    "border": badge.border,
                }));
            }
        }
    }

    // Apply Rule 6.14 dimming from recomputed skip plan.
    if !dimmed_secs.is_empty() {
        let sec_tag = enclosing_section_tag(sk_node, sk_by_tag, max_hops);
        if sec_tag.is_some_and(|st| dimmed_secs.contains(st)) {
            item["dim"] = json!(true);
            if let Some(flags) = item["flags"].as_array_mut() {
                let msg = json!(SKIP_LABEL_UNREFERENCED);
                if !flags.contains(&msg) {
                    flags.push(msg);
                }
            }
        }
    }
}

fn enclosing_section_tag<'a>(
    start: Option<&'a SkNode>,
    sk_by_tag: &BTreeMap<&'a str, &'a SkNode>,
    max_hops: usize,
) -> Option<&'a str> {
    let mut cur = start;
    let mut steps = 0usize;
    while let Some(n) = cur {
        if n.kind == SkNodeKind::Section {
            return n.tag.as_deref();
        }
        cur = n.parent.as_deref().and_then(|p| sk_by_tag.get(p).copied());
        steps += 1;
        if steps > max_hops {
            break;
        }
    }
    None
}

/// Summarizes fields that have at least one earlier value in their additive [`FieldSlot`] history
/// (`investigate.md` rule 6.12).
pub fn collect_field_changes(r: &FoldedNodeReading) -> Vec<Value> {
    let mut out = Vec::new();
    if let (Some(cur), Some(prev)) = (&r.role.current, r.role.history.last()) {
        if cur.value != prev.value {
            out.push(json!({
                "field": "role",
                "previous": prev.value,
                "current": cur.value,
                "previous_seq": prev.seq,
                "current_seq": cur.seq,
            }));
        }
    }
    if let (Some(cur), Some(prev)) = (&r.gist.current, r.gist.history.last()) {
        if cur.value != prev.value {
            out.push(json!({
                "field": "gist",
                "previous": prev.value,
                "current": cur.value,
                "previous_seq": prev.seq,
                "current_seq": cur.seq,
            }));
        }
    }
    if let (Some(cur), Some(prev)) = (&r.purpose.current, r.purpose.history.last()) {
        if cur.value != prev.value {
            out.push(json!({
                "field": "purpose",
                "previous": prev.value,
                "current": cur.value,
                "previous_seq": prev.seq,
                "current_seq": cur.seq,
            }));
        }
    }
    if let (Some(cur), Some(prev)) = (&r.authors_claim.current, r.authors_claim.history.last()) {
        if cur.value.label != prev.value.label || cur.value.relation_note != prev.value.relation_note {
            out.push(json!({
                "field": "authors_claim",
                "previous": prev.value.label,
                "current": cur.value.label,
                "previous_seq": prev.seq,
                "current_seq": cur.seq,
            }));
        }
    }
    out
}
