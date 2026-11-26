//! V7/V8/V10 `mapview` enrichment helpers (sibling of `investigation_mapview.rs`, kept <= 600 lines).
//!
//! Projects folded Layer-2 `role`, `pin`/`pinned`, `gist`, `purpose`, `authors_claim` (computed
//! via `status_badge::status_badge`), `notes`, additive change history (`changed_since_last`),
//! and recomputed V5 `skip_plan` + section dimming (Rule 6.14) onto the `mapview` JSON so
//! V13 (`investigation_wholemap_model.js`), V7 badges, V8 Block B, and V10 notes light up
//! consistently.

use research_tools::map_defs::ModelEdgeRole;
use research_tools::map_fold::{FoldedMap, FoldedNodeReading, ReaderPin};
use research_tools::pdf_pages::PageBasis;
use research_tools::skeleton::{
    compute_skip_plan, ExtraUseEdge, IngestedPdfSource, PaperSkeleton, SkEdgeRole, SkNode,
    SkNodeKind, SKIP_LABEL_UNREFERENCED,
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

    // Build extra use edges from non-stale folded model edges for Rule 6.14 skip-plan honesty.
    let extra_edges: Vec<ExtraUseEdge> = folded
        .edges
        .iter()
        .filter(|e| {
            !e.current.stale
                && sk_by_tag.contains_key(e.from.as_str())
                && sk_by_tag.contains_key(e.to.as_str())
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

    let mut sk_groups: BTreeMap<String, Value> = BTreeMap::new();
    let mut existing_node_ids: BTreeSet<String> = BTreeSet::new();
    if let Some(arr) = mv["nodes"].as_array_mut() {
        for item in arr.iter_mut() {
            let id = item["id"].as_str().unwrap_or("").to_string();
            existing_node_ids.insert(id.clone());
            sk_groups.insert(id.clone(), item["group"].clone());
            let sk_node = sk_by_tag.get(id.as_str()).copied();
            let reading = folded.nodes.get(&id);
            enrich_one_skeleton_node(item, sk_node, reading, &sk_by_tag, &dimmed_secs, skeleton.nodes.len());
        }
    }
    crate::investigation_mapview_v8::append_model_nodes_and_edges(
        mv,
        skeleton,
        folded,
        &sk_by_tag,
        &dimmed_secs,
        &sk_groups,
        &mut existing_node_ids,
    );

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
        if let Some(hyps_vv) = &r.hypotheses.current {
            let mut hv = serde_json::to_value(&hyps_vv.value).unwrap_or(Value::Null);
            crate::investigation_mapview_v8::strip_excerpts(&mut hv);
            item["hypotheses"] = hv;
        }
        if let Some(anch_vv) = &r.anchors.current {
            let mut av = serde_json::to_value(&anch_vv.value).unwrap_or(Value::Null);
            crate::investigation_mapview_v8::strip_excerpts(&mut av);
            item["anchors"] = av;
        }
        if !r.children.is_empty() {
            item["children"] = json!(r.children);
        }

        // Pure authors'-side badge with deterministic cues and heading-cite line (rule 6.6).
        let ac_vv = r.authors_claim.current.as_ref();
        let ac_stale = ac_vv.is_some_and(|v| v.stale);
        let badge = status_badge(ac_vv.map(|v| &v.value), head_cites);
        if badge.present {
            let mut ac_val = serde_json::to_value(&badge).unwrap_or(Value::Null);
            if let Some(obj) = ac_val.as_object_mut() {
                if ac_stale {
                    obj.insert("tone".into(), json!("muted"));
                    obj.insert("stale".into(), json!(true));
                }
                if let (Some(cur), Some(prev)) = (ac_vv, r.authors_claim.history.last()) {
                    if cur.value.label != prev.value.label
                        || cur.value.relation_note != prev.value.relation_note
                        || cur.value.attributed_to != prev.value.attributed_to
                    {
                        obj.insert("previous_label".into(), json!(prev.value.label));
                        obj.insert("changed".into(), json!(true));
                    }
                }
            }
            item["authors_claim"] = ac_val;
        }

        // Notes array preserved cleanly for V10 UI, F15 write-up, and V16 hand-off (Rule 6.12).
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
                    "tone": if ac_stale { "muted" } else { badge.tone.as_str() },
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

pub(crate) fn enclosing_section_tag<'a>(
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
        if cur.value.label != prev.value.label
            || cur.value.relation_note != prev.value.relation_note
            || cur.value.attributed_to != prev.value.attributed_to
        {
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

/// Attaches 1-based physical `page` (and `ocr: true` / `page_basis: "scan_text_layer"` when on a
/// scanned page) to V7 `authors_claim.quotes` entries anchored to `pdf`'s verified reading file,
/// failing closed whenever any line falls outside `pdf.page_map` or on `PageBasis::None`.
pub fn attach_v7_pdf_quote_pages(mv: &mut Value, pdf: &IngestedPdfSource) {
    let Some(nodes) = mv.get_mut("nodes").and_then(Value::as_array_mut) else {
        return;
    };
    for node in nodes {
        let Some(quotes) = node
            .get_mut("authors_claim")
            .and_then(|c| c.get_mut("quotes"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for q in quotes {
            let Some(obj) = q.as_object_mut() else { continue };
            if obj.get("source_id").and_then(Value::as_str) != Some(pdf.source_id.as_str())
                || obj.get("path").and_then(Value::as_str) != Some(pdf.primary_path.as_str())
            {
                continue;
            }
            let Some(start) = obj
                .get("start_line")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&n| n >= 1)
            else {
                continue;
            };
            let end = obj
                .get("end_line")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(start)
                .max(start);
            let mut min_p: Option<usize> = None;
            let mut max_p: Option<usize> = None;
            let mut has_scan = false;
            let mut valid = true;
            for line in start..=end {
                let Some(rec) = pdf.page_map.record_of_line(line) else {
                    valid = false;
                    break;
                };
                if rec.basis == PageBasis::None {
                    valid = false;
                    break;
                }
                if rec.basis == PageBasis::ScanTextLayer {
                    has_scan = true;
                }
                min_p = Some(min_p.map_or(rec.page, |m| m.min(rec.page)));
                max_p = Some(max_p.map_or(rec.page, |m| m.max(rec.page)));
            }
            if !valid {
                continue;
            }
            if let (Some(sp), Some(ep)) = (min_p, max_p) {
                obj.insert("page".into(), json!(sp));
                if ep > sp {
                    obj.insert("end_page".into(), json!(ep));
                }
                if has_scan {
                    obj.insert("ocr".into(), json!(true));
                    obj.insert("page_basis".into(), json!("scan_text_layer"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigation_mapview::build_mapview_json_with_fold;
    use research_tools::map_defs::{AuthorsClaimLabel, AuthorsClaimRecord, ReadingRole};
    use research_tools::map_fold::{fold_map, FieldSlot, VersionedValue};
    use research_tools::paper_key::PaperKey;
    use research_tools::skeleton::{
        build_tex_skeleton, SkNodeKind, TexSkeletonInput,
    };
    use std::collections::BTreeMap;

    fn sample_skeleton() -> PaperSkeleton {
        let tex = concat!(
            "\\documentclass{article}\n",
            "\\newtheorem{theorem}{Theorem}[section]\n",
            "\\begin{document}\n",
            "\\section{Main}\\label{sec:m}\n",
            "\\begin{theorem}[Main]\\label{thm:m}\n",
            "We prove exponential decay.\n",
            "\\end{theorem}\n",
            "\\end{document}\n",
        );
        let mut files = BTreeMap::new();
        files.insert("main.tex".into(), tex.into());
        build_tex_skeleton(&TexSkeletonInput {
            source_id: "src-1".into(),
            source_seq: 1,
            paper_key: PaperKey::from_arxiv("2403.00001").unwrap(),
            version: Some("v1".into()),
            files,
            primary_path: Some("main.tex".into()),
            files_sha256: None,
        })
        .skeleton
    }

    #[test]
    fn authors_claim_rerun_only_marks_changed_on_actual_diff_and_mutes_stale_badges() {
        let sk = sample_skeleton();
        let thm = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
        let tag = thm.tag.as_deref().unwrap();
        let mut folded = fold_map(&sk, &[], false);

        let claim_new = AuthorsClaimRecord {
            label: AuthorsClaimLabel::New,
            relation_note: Some("new bound".into()),
            attributed_to: vec![],
            anchors: vec![],
            cues: vec!["we prove".into()],
        };
        let mut slot = FieldSlot::default();
        slot.push(VersionedValue {
            seq: 2,
            request_seq: 1,
            skeleton_seq: 1,
            node_fp: thm.fp.clone(),
            stale: false,
            value: claim_new.clone(),
        });
        slot.push(VersionedValue {
            seq: 4,
            request_seq: 3,
            skeleton_seq: 1,
            node_fp: thm.fp.clone(),
            stale: false,
            value: claim_new.clone(),
        });

        let mut reading = FoldedNodeReading {
            tag: tag.into(),
            attach_to: None,
            model_kind: None,
            is_model_node: false,
            children: vec![],
            role: FieldSlot::default(),
            gist: FieldSlot::default(),
            purpose: FieldSlot::default(),
            hypotheses: FieldSlot::default(),
            anchors: FieldSlot::default(),
            authors_claim: slot,
            notes: vec![],
            pin: None,
            effective_is_main_result: false,
            any_stale: false,
        };
        folded.nodes.insert(tag.into(), reading.clone());
        let mut mv_same = build_mapview_json_with_fold(&sk, 1, "b", "h", None, Some(&folded));
        enrich_mapview_with_v7(&mut mv_same, &sk, &folded, true);
        let node_same = mv_same["nodes"].as_array().unwrap().iter().find(|n| n["id"] == tag).unwrap();
        assert!(node_same["authors_claim"].get("changed").is_none());
        assert!(node_same.get("changed_since_last").is_none());

        // Now push a genuinely different authors_claim + stale flag.
        reading.authors_claim.push(VersionedValue {
            seq: 6,
            request_seq: 5,
            skeleton_seq: 1,
            node_fp: "oldfp".into(),
            stale: true,
            value: AuthorsClaimRecord {
                label: AuthorsClaimLabel::Restated,
                relation_note: Some("restated".into()),
                attributed_to: vec![],
                anchors: vec![],
                cues: vec![],
            },
        });
        reading.role.push(VersionedValue {
            seq: 6,
            request_seq: 5,
            skeleton_seq: 1,
            node_fp: "oldfp".into(),
            stale: true,
            value: ReadingRole::MainResult,
        });
        reading.any_stale = true;
        folded.nodes.insert(tag.into(), reading.clone());
        let mut mv_diff = build_mapview_json_with_fold(&sk, 1, "b", "h", None, Some(&folded));
        enrich_mapview_with_v7(&mut mv_diff, &sk, &folded, false);
        let node_diff = mv_diff["nodes"].as_array().unwrap().iter().find(|n| n["id"] == tag).unwrap();
        assert_eq!(node_diff["authors_claim"]["changed"], json!(true));
        assert_eq!(node_diff["authors_claim"]["previous_label"], json!("new"));
        assert_eq!(node_diff["authors_claim"]["tone"], json!("muted"));
        assert_eq!(node_diff["authors_claim"]["stale"], json!(true));
        assert!(node_diff.get("role").is_none());
        assert_eq!(node_diff["effective_is_main_result"], json!(false));
        assert_eq!(mv_diff["orient_auto_start"], json!(false));

        // Changing only `attributed_to` must also flag `changed` and `changed_since_last`.
        reading.authors_claim.push(VersionedValue {
            seq: 7,
            request_seq: 6,
            skeleton_seq: 1,
            node_fp: "fp1".into(),
            stale: false,
            value: AuthorsClaimRecord {
                label: AuthorsClaimLabel::Restated,
                relation_note: Some("restated".into()),
                attributed_to: vec!["Toy98".into()],
                anchors: vec![],
                cues: vec![],
            },
        });
        folded.nodes.insert(tag.into(), reading);
        let mut mv_attr = build_mapview_json_with_fold(&sk, 1, "b", "h", None, Some(&folded));
        enrich_mapview_with_v7(&mut mv_attr, &sk, &folded, false);
        let node_attr = mv_attr["nodes"].as_array().unwrap().iter().find(|n| n["id"] == tag).unwrap();
        assert_eq!(node_attr["authors_claim"]["changed"], json!(true));
        assert!(node_attr["changed_since_last"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["field"] == "authors_claim"));
    }

    #[test]
    fn attach_v7_pdf_quote_pages_sets_page_and_ocr_and_skips_none_or_foreign() {
        use research_tools::pdf_pages::{PageMap, PageQualityMetrics, PageRecord};
        use research_tools::skeleton::IngestedPdfSource;
        use std::collections::BTreeMap;
        let metrics = PageQualityMetrics {
            char_count: 100, alpha_count: 80, word_count: 15,
            image_coverage_pct: 0, has_invisible_text: false, width_pt: 612, height_pt: 792,
        };
        let pdf = IngestedPdfSource {
            source_id: "src-pdf".into(),
            source_seq: 2,
            primary_path: "paper.txt".into(),
            text: "a\nb\nc\nd\ne\nf\n".into(),
            page_map: PageMap {
                page_count: 3,
                pages: vec![
                    PageRecord { page: 1, basis: PageBasis::TextLayer, first_line: Some(1), last_line: Some(2), metrics: metrics.clone(), reason: None },
                    PageRecord { page: 2, basis: PageBasis::ScanTextLayer, first_line: Some(3), last_line: Some(4), metrics: metrics.clone(), reason: None },
                    PageRecord { page: 3, basis: PageBasis::None, first_line: Some(5), last_line: Some(6), metrics, reason: None },
                ],
            },
            extractor_id: None,
            files_sha256: BTreeMap::new(),
            files_root_sha256: "0".repeat(64),
            extra_quality_notes: vec![],
        };
        let mut mv = json!({
            "nodes": [{
                "id": "n1",
                "authors_claim": {
                    "quotes": [
                        { "source_id": "src-pdf", "path": "paper.txt", "start_line": 1, "end_line": 2, "quote": "digital" },
                        { "source_id": "src-pdf", "path": "paper.txt", "start_line": 3, "end_line": 4, "quote": "scanned" },
                        { "source_id": "src-pdf", "path": "paper.txt", "start_line": 5, "end_line": 6, "quote": "none-page" },
                        { "source_id": "other", "path": "paper.txt", "start_line": 1, "end_line": 1, "quote": "foreign" }
                    ]
                }
            }]
        });
        attach_v7_pdf_quote_pages(&mut mv, &pdf);
        let qs = mv["nodes"][0]["authors_claim"]["quotes"].as_array().unwrap();
        assert_eq!(qs[0]["page"], 1);
        assert!(qs[0].get("ocr").is_none());
        assert_eq!(qs[1]["page"], 2);
        assert_eq!(qs[1]["ocr"], true);
        assert_eq!(qs[1]["page_basis"], "scan_text_layer");
        assert!(qs[2].get("page").is_none());
        assert!(qs[3].get("page").is_none());
    }
}
