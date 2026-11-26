//! Additive Layer-2 fold over a pinned [`PaperSkeleton`] and session events (package V6, rule 6.12).

use crate::investigation_markers::{
    is_investigation_marker, latest_investigation_marker, InvestigationTaskKind,
    EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON,
    MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED,
};
use crate::map_defs::{
    AuthorsClaimRecord, HypothesisRecord, MapReadingEventPayload, ModelEdgeRecord, ModelEdgeRole,
    ReadingRole, VerifiedMapAnchor,
};
use crate::map_model_nodes::is_model_node_tag;
use crate::run_budget::{
    count_model_calls, investigate_task_run_state, InvestigateTaskState, ModelCallBudget, RunScope,
    RunState, RunStateAction,
};
use crate::skeleton::{PaperSkeleton, SkNode};
use crate::tool_profile::MARKER_INVESTIGATION_STARTED;
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// One timestamped, fingerprint-tagged value stored inside an additive [`FieldSlot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionedValue<T> {
    pub seq: u64,
    pub request_seq: u64,
    pub skeleton_seq: u64,
    pub node_fp: String,
    pub stale: bool,
    pub value: T,
}

fn default_none<T>() -> Option<T> {
    None
}

fn default_vec<T>() -> Vec<T> {
    Vec::new()
}

/// Additive slot holding the latest value and chronological history of all earlier values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct FieldSlot<T> {
    #[serde(default = "default_none", skip_serializing_if = "Option::is_none")]
    pub current: Option<VersionedValue<T>>,
    #[serde(default = "default_vec")]
    pub history: Vec<VersionedValue<T>>,
}

impl<T> Default for FieldSlot<T> {
    fn default() -> Self {
        Self { current: None, history: Vec::new() }
    }
}

impl<T> FieldSlot<T> {
    /// Push a newer value as `current`, preserving any previous `current` in chronological `history`.
    pub fn push(&mut self, next: VersionedValue<T>) {
        if let Some(prev) = self.current.replace(next) {
            self.history.push(prev);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_none() && self.history.is_empty()
    }
}

/// Reader override pin on whether a node counts as a main result (`investigate.md` §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderPin {
    MainResult,
    NotMainResult,
}

/// One reader-authored note attached to a skeleton node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldedNote {
    pub seq: u64,
    pub tag: String,
    pub node_fp: String,
    pub stale: bool,
    pub text: String,
}

/// Folded additive Layer-2 reading state for a single node tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoldedNodeReading {
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_kind: Option<String>,
    #[serde(default)]
    pub is_model_node: bool,
    #[serde(default)]
    pub children: Vec<String>,
    pub role: FieldSlot<ReadingRole>,
    pub gist: FieldSlot<String>,
    pub purpose: FieldSlot<String>,
    pub hypotheses: FieldSlot<Vec<HypothesisRecord>>,
    pub anchors: FieldSlot<Vec<VerifiedMapAnchor>>,
    pub authors_claim: FieldSlot<AuthorsClaimRecord>,
    #[serde(default)]
    pub notes: Vec<FoldedNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<ReaderPin>,
    pub effective_is_main_result: bool,
    pub any_stale: bool,
}

impl FoldedNodeReading {
    fn new(tag: String) -> Self {
        let is_model_node = is_model_node_tag(&tag);
        Self {
            tag,
            attach_to: None,
            model_kind: None,
            is_model_node,
            children: Vec::new(),
            role: FieldSlot::default(),
            gist: FieldSlot::default(),
            purpose: FieldSlot::default(),
            hypotheses: FieldSlot::default(),
            anchors: FieldSlot::default(),
            authors_claim: FieldSlot::default(),
            notes: Vec::new(),
            pin: None,
            effective_is_main_result: false,
            any_stale: false,
        }
    }

    fn finalize(&mut self) {
        self.effective_is_main_result = match self.pin {
            Some(ReaderPin::MainResult) => true,
            Some(ReaderPin::NotMainResult) => false,
            None => self
                .role
                .current
                .as_ref()
                .is_some_and(|v| !v.stale && v.value == ReadingRole::MainResult),
        };
        self.any_stale = self.role.current.as_ref().is_some_and(|v| v.stale)
            || self.gist.current.as_ref().is_some_and(|v| v.stale)
            || self.purpose.current.as_ref().is_some_and(|v| v.stale)
            || self.hypotheses.current.as_ref().is_some_and(|v| v.stale)
            || self.anchors.current.as_ref().is_some_and(|v| v.stale)
            || self.authors_claim.current.as_ref().is_some_and(|v| v.stale)
            || self.notes.iter().any(|n| n.stale);
    }
}

/// Folded additive model edge keyed by `(from, to, role)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoldedModelEdge {
    pub from: String,
    pub to: String,
    pub role: ModelEdgeRole,
    pub current: VersionedValue<ModelEdgeRecord>,
    #[serde(default)]
    pub history: Vec<VersionedValue<ModelEdgeRecord>>,
}

/// Folded Investigate task summary with shared F9 `RunState` and `ModelCallBudget`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldedTask {
    pub request_seq: u64,
    pub task: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_seq: Option<u64>,
    pub stored_status: InvestigateTaskState,
    pub run_state: RunState,
    pub run_state_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_state_action: Option<RunStateAction>,
    pub model_calls: ModelCallBudget,
    pub saved_items: usize,
    pub saved_edges: usize,
    pub rejected_items: usize,
    pub rejected_edges: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Complete additive Layer-2 view derived purely from a [`PaperSkeleton`] and session events.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FoldedMap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_seq: Option<u64>,
    #[serde(default)]
    pub nodes: BTreeMap<String, FoldedNodeReading>,
    #[serde(default)]
    pub edges: Vec<FoldedModelEdge>,
    #[serde(default)]
    pub tasks: Vec<FoldedTask>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Default)]
struct TaskCounts {
    saved_items: usize,
    saved_edges: usize,
    rejected_items: usize,
    rejected_edges: usize,
}

#[derive(Default)]
struct FoldAcc {
    nodes: BTreeMap<String, FoldedNodeReading>,
    edges_map: BTreeMap<(String, String, ModelEdgeRole), FoldedModelEdge>,
    limitations: Vec<String>,
    task_counts: BTreeMap<u64, TaskCounts>,
    child_links: Vec<(String, String)>,
}

fn versioned<T>(seq: u64, request_seq: u64, skeleton_seq: u64, node_fp: &str, stale: bool, value: T) -> VersionedValue<T> {
    VersionedValue { seq, request_seq, skeleton_seq, node_fp: node_fp.to_string(), stale, value }
}

fn resolve_expected_fp<'a>(
    tag: &str,
    attach_hint: Option<&str>,
    nodes_by_tag: &BTreeMap<&'a str, &'a SkNode>,
    folded_nodes: &BTreeMap<String, FoldedNodeReading>,
) -> Option<&'a str> {
    if let Some(sk) = nodes_by_tag.get(tag) {
        return Some(sk.fp.as_str());
    }
    let mut cur = attach_hint.or_else(|| folded_nodes.get(tag).and_then(|n| n.attach_to.as_deref()));
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for _ in 0..64 {
        let next = cur?;
        if let Some(sk) = nodes_by_tag.get(next) {
            return Some(sk.fp.as_str());
        }
        if !seen.insert(next) {
            return None;
        }
        cur = folded_nodes.get(next).and_then(|n| n.attach_to.as_deref());
    }
    None
}

fn fold_reading_event(
    ev: &Event,
    skeleton: &PaperSkeleton,
    skeleton_seq: Option<u64>,
    nodes_by_tag: &BTreeMap<&str, &SkNode>,
    acc: &mut FoldAcc,
) {
    let Ok(payload) = serde_json::from_value::<MapReadingEventPayload>(ev.payload.clone()) else { return };
    if payload.source_id != skeleton.source_id { return; }

    let tc = acc.task_counts.entry(payload.request_seq).or_default();
    tc.saved_items = tc.saved_items.saturating_add(payload.items.len());
    tc.saved_edges = tc.saved_edges.saturating_add(payload.edges.len());
    tc.rejected_items = tc.rejected_items.saturating_add(payload.rejected.items.len());
    tc.rejected_edges = tc.rejected_edges.saturating_add(payload.rejected.edges.len());

    for lim in payload.limitations {
        let trimmed = lim.trim();
        if !trimmed.is_empty() { acc.limitations.push(trimmed.to_string()); }
    }

    for item in payload.items {
        let expected_fp = resolve_expected_fp(
            item.tag.as_str(),
            item.attach_to.as_deref(),
            nodes_by_tag,
            &acc.nodes,
        );
        let stale = expected_fp != Some(item.node_fp.as_str());
        let entry = acc.nodes.entry(item.tag.clone()).or_insert_with(|| FoldedNodeReading::new(item.tag.clone()));
        if let Some(parent) = item.attach_to {
            entry.is_model_node = true;
            acc.child_links.push((parent.clone(), item.tag.clone()));
            entry.attach_to = Some(parent);
        }
        if let Some(kind) = item.model_kind {
            entry.model_kind = Some(kind);
        }
        let (rseq, sseq, fp) = (payload.request_seq, payload.skeleton_seq, item.node_fp.as_str());
        if let Some(v) = item.role { entry.role.push(versioned(ev.seq, rseq, sseq, fp, stale, v)); }
        if let Some(v) = item.gist { entry.gist.push(versioned(ev.seq, rseq, sseq, fp, stale, v)); }
        if let Some(v) = item.purpose { entry.purpose.push(versioned(ev.seq, rseq, sseq, fp, stale, v)); }
        if !item.hypotheses.is_empty() { entry.hypotheses.push(versioned(ev.seq, rseq, sseq, fp, stale, item.hypotheses)); }
        if !item.anchors.is_empty() { entry.anchors.push(versioned(ev.seq, rseq, sseq, fp, stale, item.anchors)); }
        if let Some(v) = item.authors_claim { entry.authors_claim.push(versioned(ev.seq, rseq, sseq, fp, stale, v)); }
    }

    for edge in payload.edges {
        let from_exists = nodes_by_tag.contains_key(edge.from.as_str()) || acc.nodes.contains_key(edge.from.as_str());
        let to_exists = nodes_by_tag.contains_key(edge.to.as_str()) || acc.nodes.contains_key(edge.to.as_str());
        let to_fp = resolve_expected_fp(edge.to.as_str(), None, nodes_by_tag, &acc.nodes).unwrap_or("");
        let stale = !from_exists
            || !to_exists
            || skeleton_seq.is_some_and(|cur| payload.skeleton_seq != cur);
        let key = (edge.from.clone(), edge.to.clone(), edge.role);
        let vv = versioned(ev.seq, payload.request_seq, payload.skeleton_seq, to_fp, stale, edge);
        match acc.edges_map.get_mut(&key) {
            Some(existing) => {
                let prev = std::mem::replace(&mut existing.current, vv);
                existing.history.push(prev);
            }
            None => {
                acc.edges_map.insert(
                    key.clone(),
                    FoldedModelEdge { from: key.0, to: key.1, role: key.2, current: vv, history: Vec::new() },
                );
            }
        }
    }
}

fn fold_note_event(
    ev: &Event,
    skeleton: &PaperSkeleton,
    nodes_by_tag: &BTreeMap<&str, &SkNode>,
    nodes: &mut BTreeMap<String, FoldedNodeReading>,
) {
    if ev.payload.get("source_id").and_then(Value::as_str).is_some_and(|s| s != skeleton.source_id) {
        return;
    }
    let Some(tag) = ev.payload.get("tag").and_then(Value::as_str).filter(|s| !s.is_empty()) else { return };
    let current_fp = resolve_expected_fp(tag, None, nodes_by_tag, nodes);
    let raw_fp = ev.payload.get("node_fp").and_then(Value::as_str).unwrap_or("");
    let stale = if raw_fp.is_empty() { current_fp.is_none() } else { current_fp != Some(raw_fp) };
    let text = ev.payload.get("text").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let pin = ev.payload.get("pin").filter(|v| !v.is_null()).and_then(|v| serde_json::from_value::<ReaderPin>(v.clone()).ok());
    if text.is_none() && pin.is_none() { return; }
    let entry = nodes.entry(tag.to_string()).or_insert_with(|| FoldedNodeReading::new(tag.to_string()));
    if let Some(body) = text {
        entry.notes.push(FoldedNote { seq: ev.seq, tag: tag.to_string(), node_fp: raw_fp.to_string(), stale, text: body.to_string() });
    }
    if let Some(p) = pin { entry.pin = Some(p); }
}

fn fold_tasks(
    slice: &[Event],
    skeleton: &PaperSkeleton,
    running: bool,
    task_counts: &BTreeMap<u64, TaskCounts>,
) -> Vec<FoldedTask> {
    let latest_marker_seq = latest_investigation_marker(slice).map(|e| e.seq);
    let mut finishes: BTreeMap<u64, &Event> = BTreeMap::new();
    for ev in slice.iter().filter(|e| e.kind == MARKER_TASK_FINISHED) {
        if let Some(req_seq) = ev.payload.get("request_seq").and_then(Value::as_u64) {
            finishes.insert(req_seq, ev);
        }
    }

    let mut tasks = Vec::new();
    for ev in slice.iter().filter(|e| e.kind == MARKER_TASK_REQUESTED) {
        let source_id = ev.payload["source_id"].as_str().unwrap_or(&skeleton.source_id).to_string();
        if source_id != skeleton.source_id { continue; }
        let req_seq = ev.seq;
        let task = ev.payload["task"].as_str().unwrap_or_default().to_string();
        let tags = ev.payload.get("tags").and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let depth = ev.payload.get("depth").and_then(Value::as_str).map(str::to_string);
        let focus = ev.payload.get("focus").and_then(Value::as_str).map(str::to_string);
        let task_skel_seq = ev.payload.get("skeleton_seq").and_then(Value::as_u64);
        let cap = ev.payload.get("max_calls").or_else(|| ev.payload.get("call_cap")).and_then(Value::as_u64)
            .or_else(|| InvestigationTaskKind::from_str_exact(&task).map(|k| u64::from(k.default_call_cap())))
            .unwrap_or(8);

        let fin = finishes.get(&req_seq).copied().filter(|f| f.seq > req_seq);
        let (stored_status, error, scope) = match fin {
            Some(fin_ev) => {
                let st = fin_ev.payload.get("status").and_then(Value::as_str)
                    .and_then(InvestigateTaskState::from_stored).unwrap_or(InvestigateTaskState::Done);
                let err = fin_ev.payload.get("error").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
                (st, err, RunScope::SeqRange { start_exclusive: req_seq, end_inclusive: fin_ev.seq })
            }
            None => {
                let st = if running && latest_marker_seq == Some(req_seq) {
                    InvestigateTaskState::Requested
                } else {
                    InvestigateTaskState::Interrupted
                };
                let next_marker_seq = slice.iter()
                    .find(|other| other.seq > req_seq && is_investigation_marker(&other.kind))
                    .map(|other| other.seq);
                let sc = match next_marker_seq {
                    Some(end_seq) => RunScope::SeqRange { start_exclusive: req_seq, end_inclusive: end_seq },
                    None => RunScope::AfterSeq(req_seq),
                };
                (st, None, sc)
            }
        };

        let model_calls = ModelCallBudget::new(cap, count_model_calls(slice, scope));
        let run_state = investigate_task_run_state(stored_status);
        let counts = task_counts.get(&req_seq);
        tasks.push(FoldedTask {
            request_seq: req_seq, task, tags, depth, focus, source_id, skeleton_seq: task_skel_seq,
            stored_status, run_state, run_state_label: run_state.label().to_string(),
            run_state_action: run_state.action(), model_calls,
            saved_items: counts.map_or(0, |c| c.saved_items),
            saved_edges: counts.map_or(0, |c| c.saved_edges),
            rejected_items: counts.map_or(0, |c| c.rejected_items),
            rejected_edges: counts.map_or(0, |c| c.rejected_edges),
            error,
        });
    }
    tasks
}

/// Pure additive Layer-2 fold over a pinned [`PaperSkeleton`] and session ledger `events`.
pub fn fold_map(skeleton: &PaperSkeleton, events: &[Event], running: bool) -> FoldedMap {
    let slice = events.iter().rposition(|e| e.kind == MARKER_INVESTIGATION_STARTED).map_or(events, |i| &events[i..]);
    let skeleton_seq = slice.iter().rev().find(|e| {
        e.kind == EVENT_INVESTIGATION_SKELETON
            && e.payload.get("source_id").and_then(Value::as_str).is_none_or(|s| s == skeleton.source_id)
    }).map(|e| e.seq);

    let nodes_by_tag: BTreeMap<&str, &SkNode> = skeleton.nodes.iter().filter_map(|n| Some((n.tag.as_deref()?, n))).collect();
    let mut acc = FoldAcc::default();

    for ev in slice {
        match ev.kind.as_str() {
            EVENT_INVESTIGATION_READING => fold_reading_event(ev, skeleton, skeleton_seq, &nodes_by_tag, &mut acc),
            EVENT_INVESTIGATION_NOTE => fold_note_event(ev, skeleton, &nodes_by_tag, &mut acc.nodes),
            _ => {}
        }
    }

    // Link direct model-created child nodes onto their parent's `children` list in appearance order.
    for (parent, child) in acc.child_links {
        let entry = acc.nodes.entry(parent.clone()).or_insert_with(|| FoldedNodeReading::new(parent));
        if !entry.children.contains(&child) {
            entry.children.push(child);
        }
    }

    for node in acc.nodes.values_mut() { node.finalize(); }
    let edges = acc.edges_map.into_values().collect();
    let tasks = fold_tasks(slice, skeleton, running, &acc.task_counts);
    FoldedMap { skeleton_seq, nodes: acc.nodes, edges, tasks, limitations: acc.limitations }
}

#[cfg(test)]
#[path = "map_fold_tests.rs"]
mod tests;
