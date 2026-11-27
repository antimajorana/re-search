//! Deterministic extraction helpers for Investigate V16 hand-off packet building
//! (closure traversal, symbol mapping, bibliography resolution, quotes, gists, notes).

use crate::investigation::latest;
use crate::investigation_cited_support::resolve_bib_to_paper_ref;
use crate::investigation_skeleton::{
    ensure_skeleton_on_open, is_pdf_receipt, SkeletonRecord, EVENT_INVESTIGATION_PAPER,
    EVENT_INVESTIGATION_SKELETON,
};
use crate::routes_session::internal;
use crate::state::DesktopState;
use export::handoff::{
    ContextRole, HandoffContextItem, ModelTextEntry, MAX_ASK_ITEMS, MAX_CONVENTIONS_CHARS,
    MAX_MODEL_TEXT_CHARS, MAX_MODEL_TEXT_ITEMS, MAX_PAPERS, MAX_STATEMENT_CHARS, MAX_USER_NOTES,
    MAX_USER_NOTE_CHARS,
};
use ledger::{Event, Ledger};
use research_tools::library::Library;
use research_tools::map_fold::{FoldedMap, FoldedNodeReading};
use research_tools::paper_key::PaperRef;
use research_tools::quote_core::{verify_quote, QuoteRef};
use research_tools::skeleton::{
    is_html_receipt, load_ingested_html_source, load_ingested_pdf_source,
    load_ingested_tex_source, PaperSkeleton, SkEdgeKind, SkEdgeRole, SkNode, SkNodeKind, Span,
};
use research_tools::solve_model::{SymbolSpec, TargetForm, TaskKind};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_ANCHOR_SPAN_LINES: usize = 64;
const MAX_VERIFIED_QUOTE_CHARS: usize = 300;
const CHUNK_TARGET_QUOTE_CHARS: usize = 280;
const MIN_WHOLE_LINE_NON_SPACE: usize = 3;
const MIN_QUOTE_NON_SPACE: usize = 16;

pub(super) fn solve_kind_and_form(kind: SkNodeKind) -> (TaskKind, TargetForm) {
    match kind {
        SkNodeKind::Equation => (TaskKind::Calculate, TargetForm::ClosedForm),
        SkNodeKind::Proof => (TaskKind::CheckDerivation, TargetForm::PerStepVerdict),
        _ => (TaskKind::Prove, TargetForm::Proof),
    }
}

pub(super) fn context_role_for_kind(kind: SkNodeKind) -> ContextRole {
    match kind {
        SkNodeKind::Definition => ContextRole::Definition,
        SkNodeKind::Assumption => ContextRole::Assumption,
        SkNodeKind::Equation => ContextRole::Uses,
        _ => ContextRole::KnownResult,
    }
}

pub(super) fn collect_statement_closure(skeleton: &PaperSkeleton, target: &SkNode) -> Vec<String> {
    let Some(target_tag) = target.tag.as_deref() else {
        return Vec::new();
    };
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    visited.insert(target_tag.to_string());

    if target.kind == SkNodeKind::Proof {
        if let Some(of_tag) = target.proof_of.as_deref() {
            if visited.insert(of_tag.to_string()) {
                queue.push_back(of_tag.to_string());
            }
        }
        for edge in &skeleton.edges {
            if edge.to == target_tag
                && matches!(edge.kind, SkEdgeKind::Uses | SkEdgeKind::Proves)
                && visited.insert(edge.from.clone())
            {
                queue.push_back(edge.from.clone());
            }
        }
    } else {
        queue.push_back(target_tag.to_string());
    }

    while let Some(cur) = queue.pop_front() {
        for edge in &skeleton.edges {
            if edge.to == cur
                && edge.kind == SkEdgeKind::Uses
                && edge.role == SkEdgeRole::Statement
                && visited.insert(edge.from.clone())
            {
                queue.push_back(edge.from.clone());
            }
        }
    }

    visited.remove(target_tag);
    let mut ordered: Vec<(usize, String)> = visited
        .into_iter()
        .filter_map(|tag| {
            skeleton
                .nodes
                .iter()
                .find(|n| n.tag.as_deref() == Some(tag.as_str()))
                .filter(|n| {
                    !matches!(
                        n.kind,
                        SkNodeKind::Section | SkNodeKind::FrontMatter | SkNodeKind::Bibliography
                    )
                })
                .map(|n| (n.order, tag))
        })
        .collect();
    ordered.sort_by_key(|(order, _)| *order);
    ordered.into_iter().map(|(_, tag)| tag).collect()
}

pub(super) fn collect_relevant_symbols(
    skeleton: &PaperSkeleton,
    target: &SkNode,
    closure_tags: &[String],
    stmt_text: &str,
    context: &[HandoffContextItem],
) -> Vec<SymbolSpec> {
    let mut scope_tags: BTreeSet<&str> = closure_tags.iter().map(String::as_str).collect();
    if let Some(t) = target.tag.as_deref() {
        scope_tags.insert(t);
    }
    let mut corpus = String::from(stmt_text);
    for item in context {
        corpus.push('\n');
        corpus.push_str(&item.text);
    }
    let mut seen = BTreeSet::new();
    let (mut primary, mut earlier) = (Vec::new(), Vec::new());
    for sym in &skeleton.symbols {
        let name = sym.symbol_tex.trim();
        if name.is_empty() {
            continue;
        }
        let norm_key = if sym.norm.is_empty() { name } else { sym.norm.as_str() };
        let node_order = sym
            .in_node
            .as_deref()
            .and_then(|t| skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(t)).map(|n| n.order));
        if node_order.is_some_and(|o| o > target.order) || !seen.insert(norm_key.to_string()) {
            continue;
        }
        let spec = SymbolSpec { name: name.to_string(), assumptions: Vec::new() };
        let in_scope = sym.in_node.as_deref().is_some_and(|t| scope_tags.contains(t));
        if in_scope || corpus.contains(norm_key) || corpus.contains(name) {
            primary.push(spec);
        } else {
            earlier.push(spec);
        }
    }
    primary.extend(earlier);
    primary.truncate(MAX_ASK_ITEMS);
    primary
}

pub(super) fn collect_seed_papers(
    state: &DesktopState,
    skeleton: &PaperSkeleton,
    target: &SkNode,
    origin: &PaperRef,
) -> Result<Vec<PaperRef>, (u16, String)> {
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let mut cited_here: BTreeSet<&str> = target.head_cites.iter().map(String::as_str).collect();
    if let Some(tag) = target.tag.as_deref() {
        for edge in &skeleton.edges {
            if edge.to == tag && edge.kind == SkEdgeKind::Cites {
                cited_here.insert(edge.from.as_str());
            }
        }
    }
    let (mut prioritized, mut remaining) = (Vec::new(), Vec::new());
    for bib in &skeleton.bib {
        let overlaps_span = bib.cited_at.iter().any(|c| target.spans.iter().any(|s| s.contains(c)));
        if cited_here.contains(bib.key.as_str()) || overlaps_span {
            prioritized.push(bib);
        } else {
            remaining.push(bib);
        }
    }
    let mut seen_keys = BTreeSet::new();
    seen_keys.insert(origin.key.clone());
    let mut seeds = Vec::new();
    for bib in prioritized.into_iter().chain(remaining) {
        if seeds.len() >= MAX_PAPERS.saturating_sub(1) {
            break;
        }
        let Some(pref) = resolve_bib_to_paper_ref(bib, &lib)? else {
            continue;
        };
        if seen_keys.insert(pref.key.clone()) {
            seeds.push(pref);
        }
    }
    Ok(seeds)
}

pub(super) fn collect_model_text(folded: &FoldedMap, target_tag: &str, closure_tags: &[String]) -> Vec<ModelTextEntry> {
    let mut out = Vec::new();
    if let Some(reading) = folded.nodes.get(target_tag) {
        push_reading_model_text(&mut out, None, reading);
    }
    for ctx_tag in closure_tags {
        if out.len() >= MAX_MODEL_TEXT_ITEMS {
            break;
        }
        if let Some(reading) = folded.nodes.get(ctx_tag) {
            push_reading_model_text(&mut out, Some(ctx_tag), reading);
        }
    }
    out.truncate(MAX_MODEL_TEXT_ITEMS);
    out
}

fn push_reading_model_text(out: &mut Vec<ModelTextEntry>, prefix_tag: Option<&str>, reading: &FoldedNodeReading) {
    if let Some(gist) = &reading.gist.current {
        let text = gist.value.trim();
        if !text.is_empty() {
            let label = prefix_tag
                .map(|t| format!("Investigate gist for {t} (model)"))
                .unwrap_or_else(|| "Investigate gist (model)".to_string());
            out.push(ModelTextEntry { label, text: clamp_chars(text, MAX_MODEL_TEXT_CHARS) });
        }
    }
    if let Some(purpose) = &reading.purpose.current {
        let text = purpose.value.trim();
        if !text.is_empty() && out.len() < MAX_MODEL_TEXT_ITEMS {
            let label = prefix_tag
                .map(|t| format!("Investigate purpose for {t} (model)"))
                .unwrap_or_else(|| "Investigate purpose (model)".to_string());
            out.push(ModelTextEntry { label, text: clamp_chars(text, MAX_MODEL_TEXT_CHARS) });
        }
    }
}

pub(super) fn collect_user_notes(folded: &FoldedMap, target_tag: &str, closure_tags: &[String]) -> Vec<String> {
    let mut notes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_from = |tag: &str, with_prefix: bool| {
        let Some(reading) = folded.nodes.get(tag) else { return; };
        for n in &reading.notes {
            let trimmed = n.text.trim();
            if trimmed.is_empty() || notes.len() >= MAX_USER_NOTES {
                continue;
            }
            let entry = if with_prefix {
                clamp_chars(&format!("[{tag}] {trimmed}"), MAX_USER_NOTE_CHARS)
            } else {
                clamp_chars(trimmed, MAX_USER_NOTE_CHARS)
            };
            if seen.insert(entry.clone()) {
                notes.push(entry);
            }
        }
    };
    push_from(target_tag, false);
    for ctx_tag in closure_tags {
        push_from(ctx_tag, true);
    }
    notes
}

pub(super) fn collect_conventions_summary(state: &DesktopState, session_id: &str, events: &[Event]) -> String {
    let mut parts = Vec::new();
    if let Some(meta) = state.load_meta(session_id) {
        if let Some(pinned) = meta.pinned_conventions.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(pinned.to_string());
        }
    }
    if let Some(req_seq) = latest(events, "investigation.conventions").and_then(|e| e.payload["request_seq"].as_u64()) {
        if let Some(items) = research_tools::investigation::paper_conventions(events, req_seq)
            .and_then(|c| c.get("items").and_then(Value::as_array).cloned())
        {
            for item in items {
                let term = item.get("term").and_then(Value::as_str).or_else(|| item.get("name").and_then(Value::as_str)).unwrap_or("");
                let meaning = item.get("convention").and_then(Value::as_str).or_else(|| item.get("definition").and_then(Value::as_str)).unwrap_or("");
                let line = match (term.trim(), meaning.trim()) {
                    ("", "") => continue,
                    (t, "") => t.to_string(),
                    ("", m) => m.to_string(),
                    (t, m) => format!("{t}: {m}"),
                };
                if !parts.contains(&line) {
                    parts.push(line);
                }
            }
        }
    }
    clamp_chars(&parts.join("\n"), MAX_CONVENTIONS_CHARS)
}

pub(super) fn load_source_file_map(
    sandbox: &research_tools::WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    _skeleton: &PaperSkeleton,
) -> Result<BTreeMap<String, String>, String> {
    if is_pdf_receipt(receipt) {
        let pdf = load_ingested_pdf_source(sandbox, session_id, receipt)?;
        return Ok(BTreeMap::from([(pdf.primary_path, pdf.text)]));
    }
    if is_html_receipt(receipt) {
        let html = load_ingested_html_source(sandbox, session_id, receipt)?;
        return Ok(BTreeMap::from([(html.primary_path, html.html)]));
    }
    Ok(load_ingested_tex_source(sandbox, session_id, receipt)?.files)
}

pub(super) fn build_verified_quotes_for_spans(
    sandbox: &research_tools::WorkspaceSandbox,
    session_id: &str,
    events: &[Event],
    source_id: &str,
    spans: &[Span],
    files: &BTreeMap<String, String>,
    quote_texts: &mut BTreeMap<String, String>,
) -> Vec<QuoteRef> {
    let mut quotes = Vec::new();
    for span in spans {
        let Some(file_text) = files.get(&span.path) else { continue; };
        let all_lines: Vec<&str> = file_text.lines().collect();
        if span.start_line == 0 || span.end_line < span.start_line || span.end_line > all_lines.len() {
            continue;
        }
        let window_end = span.end_line.min(span.start_line + MAX_ANCHOR_SPAN_LINES - 1);
        let Some((q_start, q_end, q_text)) = pick_verifiable_quote_slice(&all_lines, span.start_line, window_end) else {
            continue;
        };
        if let Ok(verified) = verify_quote(sandbox, session_id, events, source_id, &span.path, q_start, q_end, &q_text) {
            let mut clean = verified.quote_ref;
            clean.evidence_seq = None;
            quote_texts.entry(clean.quote_sha256.clone()).or_insert(q_text);
            quotes.push(clean);
        }
    }
    quotes
}

fn pick_verifiable_quote_slice(all_lines: &[&str], start_line: usize, end_line: usize) -> Option<(usize, usize, String)> {
    let full = all_lines[start_line - 1..end_line].join("\n");
    if full.chars().count() <= MAX_VERIFIED_QUOTE_CHARS
        && full.chars().filter(|c| !c.is_whitespace()).count() >= MIN_WHOLE_LINE_NON_SPACE
    {
        return Some((start_line, end_line, full));
    }
    let mut cur_end = end_line;
    while cur_end >= start_line {
        let cand = all_lines[start_line - 1..cur_end].join("\n");
        if cand.chars().count() <= MAX_VERIFIED_QUOTE_CHARS
            && cand.chars().filter(|c| !c.is_whitespace()).count() >= MIN_QUOTE_NON_SPACE
        {
            return Some((start_line, cur_end, cand));
        }
        if cur_end == start_line { break; }
        cur_end -= 1;
    }
    let line_idx = (start_line..=end_line).find(|&ln| {
        all_lines[ln - 1].chars().filter(|c| !c.is_whitespace()).count() >= MIN_QUOTE_NON_SPACE
    })?;
    let clipped = clamp_word_boundary(all_lines[line_idx - 1].trim(), CHUNK_TARGET_QUOTE_CHARS);
    (clipped.chars().filter(|c| !c.is_whitespace()).count() >= MIN_QUOTE_NON_SPACE)
        .then_some((line_idx, line_idx, clipped))
}

/// Parse a [`PaperSkeleton`] directly from an `investigation.skeleton` ledger event's blob.
pub fn parse_skeleton_from_event(ledger: &Ledger, ev: &Event) -> Result<PaperSkeleton, String> {
    let hash = ev.payload["blob"]
        .as_str()
        .or_else(|| ev.payload["blob_sha256"].as_str())
        .ok_or_else(|| "skeleton event missing `blob` hash".to_string())?;
    let bytes = ledger.get_blob(hash).map_err(|e| e.to_string())?;
    serde_json::from_slice::<PaperSkeleton>(&bytes).map_err(|e| e.to_string())
}

pub(super) fn extract_spans_text(files: &BTreeMap<String, String>, spans: &[Span]) -> Result<String, String> {
    let mut chunks = Vec::new();
    for span in spans {
        let text = files
            .get(&span.path)
            .ok_or_else(|| format!("missing source file `{}` for span", span.path))?;
        let lines: Vec<&str> = text.lines().collect();
        if span.start_line == 0 || span.end_line < span.start_line || span.end_line > lines.len() {
            return Err(format!("span `{}:{}-{}` out of range", span.path, span.start_line, span.end_line));
        }
        chunks.push(lines[span.start_line - 1..span.end_line].join("\n"));
    }
    Ok(chunks.join("\n"))
}

pub(super) fn load_readonly_skeleton(
    state: &DesktopState,
    ledger: &Ledger,
    session_id: &str,
    events: &[Event],
) -> Result<Option<SkeletonRecord>, (u16, String)> {
    let ev_opt = latest(events, EVENT_INVESTIGATION_PAPER)
        .and_then(|p| {
            p.payload["source_id"].as_str().and_then(|sid| {
                events.iter().rev().find(|e| {
                    e.kind == EVENT_INVESTIGATION_SKELETON && e.payload["source_id"].as_str() == Some(sid)
                })
            })
        })
        .or_else(|| latest(events, EVENT_INVESTIGATION_SKELETON));
    if let Some(ev) = ev_opt {
        let hash = ev.payload["blob"]
            .as_str()
            .or_else(|| ev.payload["blob_sha256"].as_str())
            .ok_or_else(|| internal("skeleton event missing blob hash"))?;
        let skeleton = parse_skeleton_from_event(ledger, ev).map_err(internal)?;
        return Ok(Some(SkeletonRecord {
            skeleton,
            blob_hash: hash.to_string(),
            event_seq: ev.seq,
            timing: None,
            rebuilt: false,
        }));
    }
    ensure_skeleton_on_open(state, session_id)
}

pub(super) fn non_empty_description(label: &str, text: &str) -> String {
    let preview = non_empty_preview(text, label);
    clamp_chars(&format!("{label}: {preview}"), MAX_STATEMENT_CHARS)
}

pub(super) fn non_empty_preview(text: &str, fallback: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() { fallback.to_string() } else { collapsed }
}

pub(super) fn clamp_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

fn clamp_word_boundary(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars).collect();
    if let Some(idx) = prefix.rfind(char::is_whitespace) {
        if prefix[..idx].chars().filter(|c| !c.is_whitespace()).count() >= MIN_QUOTE_NON_SPACE {
            return prefix[..idx].to_string();
        }
    }
    prefix
}
