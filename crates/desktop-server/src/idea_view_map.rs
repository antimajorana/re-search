//! Map `GraphView`, verdict projection, and Rule V9 `cited_by_origin` candidate annotations
//! for `idea_view.rs` (package X11).
//!
//! Kept in a sibling module so `idea_view.rs` stays comfortably <= 600 lines.

use ledger::Event;
use research_tools::idea_map::{project_graph_view, IdeaMapPayload};
use research_tools::idea_verdict::{self, IdeaPresetLimits};
use research_tools::paper_key::PaperKey;
use research_tools::search_coverage::ExplorePreset;
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Annotate candidate views with `cited_by_origin: true` whenever the session was opened from a
/// `handoff.packet` whose origin bibliography includes the candidate (Rule V9).
pub fn annotate_origin_citations(candidates: &mut [Value], events: &[Event]) {
    let origin_cited = idea_verdict::origin_cited_keys(events);
    if origin_cited.is_empty() {
        return;
    }
    for cand in candidates.iter_mut() {
        let Some(obj) = cand.as_object_mut() else {
            continue;
        };
        let mut matched = obj
            .get("key")
            .and_then(Value::as_str)
            .and_then(|s| PaperKey::parse(s).ok())
            .is_some_and(|k| origin_cited.contains(&k));
        if !matched {
            if let Some(ids) = obj.get("ids").and_then(Value::as_object) {
                matched = ids_match_set(ids, &origin_cited);
            }
        }
        if matched {
            obj.insert("cited_by_origin".into(), Value::Bool(true));
        }
    }
}

fn ids_match_set(ids: &serde_json::Map<String, Value>, set: &BTreeSet<PaperKey>) -> bool {
    if let Some(a) = ids.get("arxiv").and_then(Value::as_str) {
        if PaperKey::from_arxiv(a).is_ok_and(|k| set.contains(&k)) { return true; }
    }
    if let Some(d) = ids.get("doi").and_then(Value::as_str) {
        if PaperKey::from_doi(d).is_ok_and(|k| set.contains(&k)) { return true; }
    }
    if let Some(i) = ids.get("inspire").and_then(Value::as_str) {
        if PaperKey::from_inspire(i).is_ok_and(|k| set.contains(&k)) { return true; }
    }
    if let Some(s) = ids.get("s2").and_then(Value::as_str) {
        if PaperKey::from_s2(s).is_ok_and(|k| set.contains(&k)) { return true; }
    }
    false
}

/// Project the latest `idea.map` payload into F6 `GraphView` (`{ nodes, edges, legend, overlaps,
/// dropped }`), preserving compatibility with legacy raw map JSON payloads.
pub fn build_explore_graph_view(
    latest_map: Option<&Value>,
    full_text_evidence: &[Value],
    events: &[Event],
) -> Value {
    let Some(raw) = latest_map else {
        return json!({ "nodes": [], "edges": [], "legend": [] });
    };
    let mut ocr_ids: BTreeSet<String> = full_text_evidence
        .iter()
        .filter(|e| e.get("ocr_tag").and_then(Value::as_str) == Some("OCR"))
        .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    // Also preserve OCR tagging for stale pre-revision maps or carried-over `not_in_current_map` nodes.
    let mut ocr_papers: BTreeSet<String> = BTreeSet::new();
    for ev in events {
        if ev.kind == "idea.paper" {
            if matches!(ev.payload["text_basis"].as_str(), Some("scan_text_layer" | "ocr")) {
                if let Some(k) = ev.payload["key"].as_str() { ocr_papers.insert(k.to_string()); }
            }
        } else if ev.kind == research_tools::quote_core::EVIDENCE_QUOTE_EVENT {
            let pk = ev.payload["paper_key"].as_str().unwrap_or("");
            let tb = ev.payload["text_basis"].as_str().unwrap_or("");
            if matches!(tb, "scan_text_layer" | "ocr") || ocr_papers.contains(pk) {
                ocr_ids.insert(format!("xe-{}", ev.seq));
            }
        }
    }
    if let Ok(typed) = serde_json::from_value::<IdeaMapPayload>(raw.clone()) {
        let mut projected = project_graph_view(&typed, &ocr_ids);
        if let Some(obj) = projected.as_object_mut() {
            obj.insert("idea_rev".into(), json!(typed.idea_rev));
        }
        return projected;
    }
    crate::idea_view_coverage::build_map_graph_view(Some(raw))
}

/// Project `verdicts` for `idea_view`: when an `idea.verdict` event exists in the active revision,
/// recompute purely over ledger events so post-map Rule V6 confirmation notes (`confirm_same` /
/// `reject_same`) update `confirmed_by_user` and `chip` immediately while preserving Rule V7 parity.
pub fn build_verdicts_view(
    latest_verdict: Option<&Value>,
    events: &[Event],
    preset: crate::idea::IdeaPreset,
    max_screened: u64,
    max_papers: usize,
) -> Value {
    let Some(stored) = latest_verdict else {
        return Value::Null;
    };
    let explore_preset = match preset {
        crate::idea::IdeaPreset::Quick => ExplorePreset::Quick,
        crate::idea::IdeaPreset::Standard => ExplorePreset::Standard,
        crate::idea::IdeaPreset::Thorough => ExplorePreset::Thorough,
    };
    let limits = IdeaPresetLimits {
        preset: explore_preset,
        max_screened: Some(max_screened as usize),
        max_papers: Some(max_papers),
    };
    let computed = idea_verdict::compute(events, Some(limits));
    if computed.verdicts.is_empty() && stored.get("verdicts").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        return stored.clone();
    }
    serde_json::to_value(&computed).unwrap_or_else(|_| stored.clone())
}
