//! Investigate V12 context tasks (`prerequisites` / `compare_hypotheses`), cited-node prompt
//! injection with F4 `context.supplied` crediting, and `mapview` context card projection.

use crate::investigation_cited::EVENT_INVESTIGATION_CITED_LOOKUP;
use crate::investigation_mapview_v7::attach_pdf_pages_to_anchor_obj;
use crate::investigation_mapview_v8::strip_excerpts;
use crate::investigation_skeleton::find_source_receipt;
use crate::investigation_task_context::render_quoted_block;
use crate::state::DesktopState;
use ledger::Event;
use research_tools::investigation_markers::{
    active_investigation_slice, InvestigationTaskKind, EVENT_INVESTIGATION_CONTEXT,
    EVENT_INVESTIGATION_SKELETON,
};
use research_tools::lines_sent::SuppliedRange;
use research_tools::map_context_defs::{
    ContextCardOrigin, MAX_CONTEXT_CARDS, MAX_GROUNDED_BODY_CHARS,
    MAX_MODEL_BACKGROUND_BODY_CHARS, MAX_MODEL_BACKGROUND_PER_NODE,
};
use research_tools::map_defs::RECORD_CONTEXT_CARDS;
use research_tools::map_fold::{fold_map, FoldedMap, FoldedNodeReading, VersionedValue};
use research_tools::map_model_nodes::collect_existing_model_nodes;
use research_tools::paper_key::PaperKey;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    build_html_skeleton_from_ingested, build_pdf_skeleton_from_ingested,
    build_tex_skeleton_from_ingested, derive_receipt_paper_identity, is_html_receipt,
    is_pdf_receipt, is_tex_receipt, load_ingested_html_source, load_ingested_pdf_source,
    load_ingested_tex_source, PaperSkeleton, SkEdgeKind, SkNode, CITED_STATUS_FOUND,
    PROVENANCE_OUTSIDE_PAPER,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub const FOCUS_PREREQUISITES: &str = "prerequisites";
pub const FOCUS_COMPARE_HYPOTHESES: &str = "compare_hypotheses";

const PROVENANCE_KEY_PAPER: &str = "from_paper";
const PROVENANCE_KEY_EXTERNAL: &str = PROVENANCE_OUTSIDE_PAPER;
const PROVENANCE_KEY_MODEL_BG: &str = "model_background";

/// Canonicalize known context-task focus values (`prerequisites` / `compare_hypotheses`).
pub fn normalize_context_focus(raw: Option<&str>) -> Option<&'static str> {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty())?;
    let key = trimmed.to_ascii_lowercase().replace(['-', ' '], "_");
    match key.as_str() {
        "prerequisites" | "prerequisite" => Some(FOCUS_PREREQUISITES),
        "compare_hypotheses" | "compare_hypothesis" | "compare" => Some(FOCUS_COMPARE_HYPOTHESES),
        _ => None,
    }
}

/// Canonicalize `focus` stored on a `context` task request when it matches a built-in focus mode.
pub(crate) fn canonicalize_task_focus(
    task: InvestigationTaskKind,
    focus: Option<String>,
) -> Option<String> {
    if task != InvestigationTaskKind::Context {
        return focus;
    }
    match normalize_context_focus(focus.as_deref()) {
        Some(norm) => Some(norm.to_string()),
        None => focus,
    }
}

pub(crate) fn is_compare_hypotheses(focus: Option<&str>) -> bool {
    normalize_context_focus(focus) == Some(FOCUS_COMPARE_HYPOTHESES)
}

/// Concise user-turn prompt stored on ledger when launching a `context` task.
pub(crate) fn build_context_user_prompt(tags: &[String], focus: Option<&str>) -> String {
    let tag_list = tags.join(", ");
    if is_compare_hypotheses(focus) {
        format!(
            "Compare hypotheses and assumptions for Investigate node(s): {tag_list} against \
             located cited results using matched source quotes."
        )
    } else {
        format!(
            "Gather grounded prerequisites and symbol definitions for Investigate node(s): \
             {tag_list} using matched source quotes."
        )
    }
}

/// Structured task preamble instructions appended for `InvestigationTaskKind::Context`.
pub(crate) fn append_context_task_instructions(
    out: &mut String,
    source_id: &str,
    request_seq: u64,
    tags: &[String],
    focus: Option<&str>,
    cap: u32,
) {
    let compare = is_compare_hypotheses(focus);
    if compare {
        let _ = writeln!(
            out,
            "Goal (Context — Compare hypotheses, cap {cap} model calls): Compare hypotheses and \
             assumptions between target node(s) ({targets}) and their located cited external \
             result(s), surfacing any stronger, missing, or mismatched assumptions.",
            targets = tags.join(", ")
        );
    } else {
        let _ = writeln!(
            out,
            "Goal (Context — Prerequisites, cap {cap} model calls): Explain the key symbols, \
             notations, and mathematical prerequisites needed to read target node(s) ({targets}).",
            targets = tags.join(", ")
        );
    }
    let _ = writeln!(
        out,
        "Recorder rules (`{RECORD_CONTEXT_CARDS}`, passing request_seq={request_seq}, \
         source_id=\"{source_id}\", target `tag`, and 1..={MAX_CONTEXT_CARDS} `cards`):\n\
         - Each card has `type` (`symbol`, `prerequisite`, or `cited_result`), `title`, `body`, \
         and `anchors` (plus optional `bib_key` and `hypotheses [{{text, anchor}}]`).\n\
         - Code assigns `origin`: `paper` when all anchors match `{source_id}`, `external` when \
         any anchor matches an external cited source (`body <= {MAX_GROUNDED_BODY_CHARS}` chars). \
         Never supply `origin` or app-only anchor digest/excerpt fields.\n\
         - Failed or unsent anchors drop their card rather than downgrading. Only `prerequisite` \
         cards may omit `anchors` (`origin: model_background`), capped at \
         `{MAX_MODEL_BACKGROUND_BODY_CHARS}` chars and `{MAX_MODEL_BACKGROUND_PER_NODE}` cards per node."
    );
    if compare {
        let _ = writeln!(
            out,
            "- For each located citation, record a `cited_result` card with its `bib_key`, at least \
             one matched anchor inside that located external node, and each external hypothesis in \
             `hypotheses [{{text, anchor}}]` anchored inside that external node; add `prerequisite` \
             or `symbol` cards for any side assumptions or gaps.\n\
             Tools: Cite lines from the <quoted_source_material> blocks below or inspect additional \
             primary/external lines with `read_map_node` / `read_paper_lines`."
        );
    } else {
        let _ = writeln!(
            out,
            "Tools: Use `read_map_node` or `read_paper_lines` if an earlier symbol definition or \
             dependency statement is not yet shown below, and cite exact matched quotes in `anchors`."
        );
    }
}

/// Sum accepted cards and rejected cards from `investigation.context` events for `request_seq`.
pub(crate) fn accumulate_context_event_counts(
    events: &[Event],
    request_seq: u64,
    saved_items: &mut u64,
    rejected_items: &mut u64,
) {
    for ev in events.iter().filter(|e| {
        e.kind == EVENT_INVESTIGATION_CONTEXT
            && e.seq > request_seq
            && e.payload["request_seq"].as_u64() == Some(request_seq)
    }) {
        *saved_items += ev.payload["cards"].as_array().map_or(0, Vec::len) as u64;
        *rejected_items += ev.payload["rejected"].as_array().map_or(0, Vec::len) as u64;
    }
}

struct CitedLookupEntry {
    bib_key: String,
    short_ref: Option<String>,
    status: String,
    external_source_id: Option<String>,
    external_tag: Option<String>,
    paper_key: Option<String>,
    version: Option<String>,
}

/// Append target node reading metadata and located external cited node text (`<quoted_source_material>`)
/// for `Context` tasks with `Compare hypotheses` focus, crediting every delivered external line.
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_compare_hypotheses_context(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    events: &[Event],
    skeleton: &PaperSkeleton,
    tags: &[String],
    max_chars: usize,
    instructions: &mut String,
    raw_ranges: &mut Vec<SuppliedRange>,
) -> Result<(), String> {
    let slice = active_investigation_slice(events)
        .map(|(s, _)| s)
        .unwrap_or(events);
    let folded = fold_map(skeleton, events, false);
    let models = collect_existing_model_nodes(skeleton, events);
    let sk_by_tag: BTreeMap<&str, &SkNode> = skeleton
        .nodes
        .iter()
        .filter_map(|n| Some((n.tag.as_deref()?, n)))
        .collect();

    let mut cited_bibs: BTreeSet<String> = BTreeSet::new();
    let mut target_roots: BTreeSet<String> = BTreeSet::new();
    for tag in tags {
        target_roots.insert(tag.clone());
        let root_tag = models.get(tag).map_or(tag.as_str(), |m| m.root_skeleton_tag.as_str());
        target_roots.insert(root_tag.to_string());
        if let Some(sk) = sk_by_tag.get(root_tag) {
            for k in &sk.head_cites {
                cited_bibs.insert(k.clone());
            }
            for e in skeleton
                .edges
                .iter()
                .filter(|e| e.kind == SkEdgeKind::Cites && e.from == root_tag)
            {
                cited_bibs.insert(e.to.clone());
            }
        }
    }

    // Add concise non-instruction metadata note of existing target hypotheses & gist.
    let mut meta = String::from(
        "\nTarget node reading metadata for comparison (metadata only, not raw source lines):\n",
    );
    let mut has_meta = false;
    for tag in tags {
        if let Some(r) = folded.nodes.get(tag) {
            if let Some(g) = &r.gist.current {
                let _ = writeln!(meta, "- [{tag}] gist: {}", g.value);
                has_meta = true;
            }
            if let Some(hyps) = &r.hypotheses.current {
                for (hi, h) in hyps.value.iter().enumerate() {
                    let _ = writeln!(meta, "  hypothesis {}: {}", hi + 1, h.text);
                    has_meta = true;
                }
            }
        }
    }

    let lookups = collect_relevant_lookups(slice, &target_roots, &cited_bibs);
    let mut unlocated_notes = String::new();
    let mut seen_ext: BTreeSet<(String, String)> = BTreeSet::new();
    let mut ext_targets: Vec<CitedLookupEntry> = Vec::new();
    for entry in lookups {
        if entry.status == CITED_STATUS_FOUND {
            if let (Some(sid), Some(etag)) = (&entry.external_source_id, &entry.external_tag) {
                if sid != &skeleton.source_id && seen_ext.insert((sid.clone(), etag.clone())) {
                    ext_targets.push(entry);
                }
            }
        } else {
            let _ = writeln!(
                unlocated_notes,
                "- Citation [{}] has not been located to a single external node (status: {}).",
                entry.bib_key, entry.status
            );
        }
    }
    if has_meta && instructions.chars().count() + meta.chars().count() < max_chars {
        instructions.push_str(&meta);
    }
    if !unlocated_notes.is_empty()
        && instructions.chars().count() + unlocated_notes.chars().count() + 64 < max_chars
    {
        instructions.push_str("\nUnlocated citations (metadata only; do not save `cited_result` for these):\n");
        instructions.push_str(&unlocated_notes);
    }

    for entry in ext_targets {
        let Some(ext_sid) = entry.external_source_id.as_deref() else { continue };
        let Some(ext_tag) = entry.external_tag.as_deref() else { continue };
        let Some((ext_skel, files, hashes)) =
            load_external_source_and_skeleton(sandbox, session_id, slice, events, &entry)?
        else {
            continue;
        };
        let Some(ext_node) = ext_skel.nodes.iter().find(|n| n.tag.as_deref() == Some(ext_tag)) else {
            continue;
        };
        let label = ext_node
            .number
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(ext_node.title.as_deref())
            .unwrap_or(ext_tag);
        let ref_note = entry.short_ref.as_deref().unwrap_or(&entry.bib_key);
        for span in &ext_node.spans {
            let Some(content) = files.get(&span.path) else { continue };
            let Some(file_sha256) = hashes.get(&span.path) else { continue };
            let lines: Vec<&str> = content.lines().map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
            if span.start_line == 0 || span.end_line < span.start_line || span.start_line > lines.len() {
                continue;
            }
            let start = span.start_line;
            let max_end = span.end_line.min(lines.len());
            let remaining = max_chars.saturating_sub(instructions.chars().count());
            let heading = format!(
                "Cited external result [{}] ({ref_note}) — {label} [{ext_tag}]",
                entry.bib_key
            );
            let Some((rendered, delivered_end)) = render_quoted_block(
                &heading,
                ext_sid,
                &span.path,
                file_sha256,
                &lines,
                start,
                max_end,
                remaining,
            ) else {
                continue;
            };
            instructions.push_str(&rendered);
            raw_ranges.push(SuppliedRange {
                source_id: ext_sid.to_string(),
                path: span.path.clone(),
                file_sha256: file_sha256.clone(),
                start_line: start,
                end_line: delivered_end,
            });
        }
    }
    Ok(())
}

fn collect_relevant_lookups(
    slice: &[Event],
    target_roots: &BTreeSet<String>,
    cited_bibs: &BTreeSet<String>,
) -> Vec<CitedLookupEntry> {
    let mut by_key: BTreeMap<String, CitedLookupEntry> = BTreeMap::new();
    for ev in slice.iter().filter(|e| e.kind == EVENT_INVESTIGATION_CITED_LOOKUP) {
        let Some(bib_key) = ev.payload["bib_key"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let ev_tag = ev.payload["tag"].as_str().unwrap_or("");
        if !target_roots.contains(ev_tag) && !cited_bibs.contains(bib_key) {
            continue;
        }
        let status = ev.payload["status"].as_str().unwrap_or("unresolved").to_string();
        by_key.insert(
            bib_key.to_string(),
            CitedLookupEntry {
                bib_key: bib_key.to_string(),
                short_ref: ev.payload["short_ref"].as_str().map(str::to_string),
                status,
                external_source_id: ev.payload["external_source_id"].as_str().map(str::to_string),
                external_tag: ev.payload["external_tag"].as_str().map(str::to_string),
                paper_key: ev.payload["resolved"]["paper_key"].as_str().map(str::to_string),
                version: ev.payload["resolved"]["version"].as_str().map(str::to_string),
            },
        );
    }
    by_key.into_values().collect()
}

type LoadedExternal = (PaperSkeleton, BTreeMap<String, String>, BTreeMap<String, String>);

fn load_external_source_and_skeleton(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    _slice: &[Event],
    events: &[Event],
    entry: &CitedLookupEntry,
) -> Result<Option<LoadedExternal>, String> {
    let Some(ext_sid) = entry.external_source_id.as_deref() else {
        return Ok(None);
    };
    let Some(receipt) = find_source_receipt(events, ext_sid, None) else {
        return Ok(None);
    };
    let key_override = entry.paper_key.as_deref().and_then(|s| PaperKey::parse(s).ok());
    let (paper_key, version) =
        derive_receipt_paper_identity(receipt, key_override, entry.version.clone())?;

    if is_tex_receipt(receipt) {
        let tex = load_ingested_tex_source(sandbox, session_id, receipt)?;
        let skel = build_tex_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?
            .0
            .skeleton;
        Ok(Some((skel, tex.files, tex.files_sha256)))
    } else if is_html_receipt(receipt) {
        let html = load_ingested_html_source(sandbox, session_id, receipt)?;
        let skel =
            build_html_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?
                .0
                .skeleton;
        let files = BTreeMap::from([(html.primary_path, html.html)]);
        Ok(Some((skel, files, html.files_sha256)))
    } else if is_pdf_receipt(receipt) {
        let pdf = load_ingested_pdf_source(sandbox, session_id, receipt)?;
        let skel = build_pdf_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?
            .0
            .skeleton;
        let files = BTreeMap::from([(pdf.primary_path, pdf.text)]);
        Ok(Some((skel, files, pdf.files_sha256)))
    } else {
        let _ = _slice.iter().any(|e| e.kind == EVENT_INVESTIGATION_SKELETON);
        Ok(None)
    }
}

/// Project folded `context_cards` directly from `FoldedMap` onto every node in `mv["nodes"]`.
pub(crate) fn project_folded_context_cards(mv: &mut Value, folded: &FoldedMap) {
    let Some(nodes) = mv.get_mut("nodes").and_then(Value::as_array_mut) else {
        return;
    };
    for node in nodes {
        let Some(tag) = node.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        let Some(r) = folded.nodes.get(&tag) else {
            continue;
        };
        if r.context_cards.is_empty() {
            continue;
        }
        node["context_cards"] = Value::Array(
            r.context_cards
                .iter()
                .map(serialize_folded_card)
                .collect(),
        );
    }
}

/// Enrich projected `context_cards` on `mv["nodes"]` with `short_ref` metadata and PDF page numbers.
pub(crate) fn enrich_context_cards_metadata(
    mv: &mut Value,
    state: &DesktopState,
    session_id: &str,
    events: &[Event],
) {
    let short_refs: BTreeMap<String, String> = active_investigation_slice(events)
        .map(|(s, _)| s)
        .unwrap_or(events)
        .iter()
        .filter(|e| e.kind == EVENT_INVESTIGATION_CITED_LOOKUP)
        .filter_map(|e| {
            let k = e.payload["bib_key"].as_str()?;
            let sr = e.payload["short_ref"].as_str()?;
            Some((k.to_string(), sr.to_string()))
        })
        .collect();

    let mut pdf_sources: BTreeSet<String> = BTreeSet::new();
    if let Some(nodes) = mv.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            let Some(cards) = node.get_mut("context_cards").and_then(Value::as_array_mut) else {
                continue;
            };
            for card in cards {
                if let Some(k) = card.get("bib_key").and_then(Value::as_str) {
                    if let Some(sr) = short_refs.get(k) {
                        card["short_ref"] = json!(sr);
                    }
                }
                collect_card_anchor_sources(card, &mut pdf_sources);
            }
        }
    }

    if pdf_sources.is_empty() {
        return;
    }
    let Ok(env) = state.research_env_for_session(session_id) else {
        return;
    };
    let sandbox = env.sandbox();
    let mut loaded_pdfs = Vec::new();
    for sid in pdf_sources {
        let Some(receipt) = find_source_receipt(events, &sid, None) else { continue };
        if !is_pdf_receipt(receipt) {
            continue;
        }
        if let Ok(pdf) = load_ingested_pdf_source(&sandbox, session_id, receipt) {
            loaded_pdfs.push(pdf);
        }
    }
    if loaded_pdfs.is_empty() {
        return;
    }
    if let Some(nodes) = mv.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            let Some(cards) = node.get_mut("context_cards").and_then(Value::as_array_mut) else {
                continue;
            };
            for card in cards {
                for pdf in &loaded_pdfs {
                    attach_pdf_to_card_anchors(card, pdf);
                }
            }
        }
    }
}

fn serialize_folded_card(vc: &VersionedValue<research_tools::map_context_defs::ContextCardRecord>) -> Value {
    let prov = match vc.value.origin {
        ContextCardOrigin::Paper => PROVENANCE_KEY_PAPER,
        ContextCardOrigin::External => PROVENANCE_KEY_EXTERNAL,
        ContextCardOrigin::ModelBackground => PROVENANCE_KEY_MODEL_BG,
    };
    let mut obj = serde_json::to_value(&vc.value).unwrap_or(Value::Null);
    strip_excerpts(&mut obj);
    if let Some(map) = obj.as_object_mut() {
        map.insert("seq".into(), json!(vc.seq));
        map.insert("request_seq".into(), json!(vc.request_seq));
        map.insert("skeleton_seq".into(), json!(vc.skeleton_seq));
        map.insert("stale".into(), json!(vc.stale));
        map.insert("provenance".into(), json!(prov));
    }
    obj
}

fn collect_card_anchor_sources(card: &Value, out: &mut BTreeSet<String>) {
    if let Some(anchors) = card.get("anchors").and_then(Value::as_array) {
        for a in anchors {
            if let Some(sid) = a.get("source_id").and_then(Value::as_str) {
                out.insert(sid.to_string());
            }
        }
    }
    if let Some(hyps) = card.get("hypotheses").and_then(Value::as_array) {
        for h in hyps {
            if let Some(sid) = h.get("anchor").and_then(|a| a.get("source_id")).and_then(Value::as_str) {
                out.insert(sid.to_string());
            }
        }
    }
}

fn attach_pdf_to_card_anchors(
    card: &mut Value,
    pdf: &research_tools::skeleton::IngestedPdfSource,
) {
    if let Some(anchors) = card.get_mut("anchors").and_then(Value::as_array_mut) {
        for a in anchors {
            attach_pdf_pages_to_anchor_obj(a, pdf);
        }
    }
    if let Some(hyps) = card.get_mut("hypotheses").and_then(Value::as_array_mut) {
        for h in hyps {
            if let Some(a) = h.get_mut("anchor") {
                attach_pdf_pages_to_anchor_obj(a, pdf);
            }
        }
    }
}

/// Expose folded node cards for tests or sibling callers without unused warnings.
#[allow(dead_code)]
pub(crate) fn folded_node_cards_len(r: &FoldedNodeReading) -> usize {
    r.context_cards.len()
}

#[cfg(test)]
#[path = "investigation_context_tasks_tests.rs"]
mod tests;
