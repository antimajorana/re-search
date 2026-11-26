//! Data types, constants, and span-matching helpers for the Investigate V9 legacy adapter.

use crate::investigation::list;
use research_tools::skeleton::{PaperSkeleton, SkNode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Reserved F2 legacy node prefix (`00-overview.md` §2.3, owner decisions 2026-11-25).
pub const LEGACY_NODE_PREFIX: &str = "legacy:";
/// Section heading for legacy nodes attached onto a skeleton node (`investigate.md` §8).
pub const LEGACY_SECTION_LABEL: &str = "Earlier map";
/// Drawer/section heading for legacy nodes lacking a skeleton span match (`investigate.md` §8).
pub const LEGACY_UNPLACED_LABEL: &str = "Unplaced (older map)";
/// Mandatory role on all legacy edges; never guess `"proof"` (`investigate.md` §0.2 #6, §8 #5).
pub const LEGACY_EDGE_ROLE: &str = "unknown (older map)";
/// Basis for legacy edges in GraphView (`00-overview.md` §2.6; dashed in UI).
pub const LEGACY_EDGE_BASIS: &str = "model";
/// Label replacing `"Find main results"` on old sessions (`investigate.md` §8 #6).
pub const LEGACY_REBUILD_LABEL: &str = "Rebuild earlier map";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyAnchor {
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyCorrectionNote {
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_seq: Option<u64>,
    pub raw_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_edge_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placed_tag: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyNodeEdge {
    pub id: String,
    pub direction: String,
    pub src: String,
    pub dst: String,
    pub peer_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_tag: Option<String>,
    pub kind: String,
    pub role: String,
    pub basis: String,
    pub dashed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyNode {
    pub id: String,
    pub raw_id: String,
    pub origin_seq: u64,
    pub origin_kind: String,
    pub sentence: String,
    pub kind: String,
    pub main: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<String>,
    pub anchors: Vec<LegacyAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placed_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<LegacyCorrectionNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<LegacyNodeEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyEdge {
    pub id: String,
    pub src: String,
    pub dst: String,
    pub raw_from: String,
    pub raw_to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_tag: Option<String>,
    pub kind: String,
    pub role: String,
    pub basis: String,
    pub dashed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_basis: Option<String>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub anchor: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyMapProjection {
    pub has_legacy: bool,
    pub fallback_old_view: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<String>,
    pub limitations: Vec<String>,
    pub answer_node_ids: Vec<String>,
    pub nodes: Vec<LegacyNode>,
    pub placed: BTreeMap<String, Vec<LegacyNode>>,
    pub unplaced: Vec<LegacyNode>,
    pub edges: Vec<LegacyEdge>,
    pub branches: Vec<Value>,
    pub corrections: Vec<LegacyCorrectionNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conventions: Option<Value>,
    pub activities: Vec<Value>,
    pub answers: Vec<Value>,
}

pub(crate) fn parse_legacy_anchor(val: &Value) -> LegacyAnchor {
    let start_line = val["start_line"].as_u64().unwrap_or(0) as usize;
    let end_line = val["end_line"].as_u64().unwrap_or(0) as usize;
    LegacyAnchor {
        source_id: val["source_id"].as_str().unwrap_or("").to_string(),
        path: val["path"].as_str().unwrap_or("").to_string(),
        start_line,
        end_line,
        kind: val["kind"].as_str().unwrap_or("passage").to_string(),
        label: opt_str(&val["label"]),
        quote: opt_str(&val["quote"]),
        excerpt: opt_str(&val["excerpt"]),
    }
}

/// Attaches to the innermost `SkNode` whose `spans` contain the legacy node's first anchor
/// `{path, start_line, end_line}`, minimizing `(span_line_count, SkNode.order, tag)`.
pub(crate) fn match_innermost_skeleton_node(
    skeleton: &PaperSkeleton,
    anchors: &[LegacyAnchor],
) -> Option<String> {
    let first = anchors.first()?;
    let anchor_path = norm_rel_path(&first.path);
    if anchor_path.is_empty() || first.start_line == 0 {
        return None;
    }
    if !first.source_id.is_empty()
        && !skeleton.source_id.is_empty()
        && first.source_id != skeleton.source_id
    {
        return None;
    }
    let start = first.start_line;
    let end = first.end_line.max(start);

    let mut best: Option<(usize, usize, String)> = None;
    for sk_node in &skeleton.nodes {
        let Some(tag) = sk_node.tag.as_deref() else {
            continue;
        };
        let Some(min_lines) = enclosing_span_min_lines(sk_node, &anchor_path, start, end) else {
            continue;
        };
        let candidate = (min_lines, sk_node.order, tag.to_string());
        if best.as_ref().is_none_or(|cur| candidate < *cur) {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, tag)| tag)
}

fn enclosing_span_min_lines(
    sk_node: &SkNode,
    anchor_path: &str,
    start: usize,
    end: usize,
) -> Option<usize> {
    sk_node
        .spans
        .iter()
        .filter(|span| {
            norm_rel_path(&span.path) == anchor_path
                && span.start_line <= start
                && end <= span.end_line
        })
        .map(|span| span.end_line.saturating_sub(span.start_line) + 1)
        .min()
}

pub(crate) fn strip_disallowed_keys(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.retain(|k, _| !k.to_ascii_lowercase().contains("verified"));
            for v in map.values_mut() {
                strip_disallowed_keys(v);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_disallowed_keys(item);
            }
        }
        _ => {}
    }
}

pub(crate) fn norm_rel_path(raw: &str) -> String {
    let slashed = raw.trim().replace('\\', "/");
    slashed.trim_start_matches("./").to_string()
}

pub(crate) fn opt_str(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn string_list(v: &Value) -> Vec<String> {
    list(v).iter().filter_map(opt_str).collect()
}
