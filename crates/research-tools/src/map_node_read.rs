//! Investigate v2 skeleton node reader (`read_map_node`, package V6).

use crate::investigation::anchor::{valid_id, SourceIndex, VerifiedFile, MAX_TOTAL_BYTES};
use crate::investigation_markers::{
    latest_investigation_marker, EVENT_INVESTIGATION_SKELETON, MARKER_TASK_REQUESTED,
};
use crate::map_defs::{
    read_map_node_def, DEFAULT_READ_NODE_MAX_CHARS, MAX_READ_NODE_MAX_CHARS,
    MIN_READ_NODE_MAX_CHARS,
};
use crate::skeleton::{PaperSkeleton, SkEdgeKind, SkEdgeRole, SkNode, SkNodeKind, Span, SymbolDef};
use crate::{resolve_sources_session_id, ResearchEnv};
use ledger::{Event, Ledger};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const MAX_SUGGESTED_NODES: usize = 8;

/// Load the pinned `investigation.skeleton` event and verified JSON blob for `source_id`.
///
/// Precedence: explicit `requested_skeleton_seq` > active `investigation.task_requested`
/// carrying `skeleton_seq` for `source_id` > newest `investigation.skeleton` for `source_id`.
/// Fails closed on missing events, source mismatch, or SHA-256 digest mismatch.
pub fn load_pinned_skeleton(
    ledger: &Ledger,
    events: &[Event],
    source_id: &str,
    requested_skeleton_seq: Option<u64>,
) -> Result<(u64, String, PaperSkeleton), String> {
    let pinned_seq = requested_skeleton_seq.or_else(|| {
        let marker = latest_investigation_marker(events)?;
        (marker.kind == MARKER_TASK_REQUESTED
            && marker.payload["source_id"].as_str() == Some(source_id))
            .then(|| marker.payload["skeleton_seq"].as_u64())?
    });

    let skel_event = match pinned_seq {
        Some(seq) => events
            .iter()
            .find(|e| e.seq == seq && e.kind == EVENT_INVESTIGATION_SKELETON)
            .ok_or_else(|| format!("Pinned investigation.skeleton seq={seq} for `{source_id}` not found."))?,
        None => events
            .iter()
            .rev()
            .find(|e| e.kind == EVENT_INVESTIGATION_SKELETON && e.payload["source_id"].as_str() == Some(source_id))
            .ok_or_else(|| format!("No investigation.skeleton exists for source `{source_id}` in this session."))?,
    };

    if let Some(ev_src) = skel_event.payload["source_id"].as_str().filter(|&s| s != source_id) {
        return Err(format!("Pinned skeleton seq={} belongs to `{ev_src}`, not `{source_id}`.", skel_event.seq));
    }

    let blob_hash = skel_event.payload["blob"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("investigation.skeleton seq={} is missing a valid `blob` hash.", skel_event.seq))?;
    let bytes = ledger.get_blob(blob_hash).map_err(|_| {
        format!("Skeleton blob `{blob_hash}` for seq={} could not be loaded or failed digest check.", skel_event.seq)
    })?;
    if ledger::sha256_hex(&bytes) != blob_hash {
        return Err(format!("Skeleton blob digest mismatch for seq={} (expected `{blob_hash}`).", skel_event.seq));
    }

    let skeleton: PaperSkeleton = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Skeleton blob for seq={} is not valid PaperSkeleton JSON: {e}", skel_event.seq))?;
    if skeleton.source_id != source_id {
        return Err(format!("Skeleton seq={} source_id `{}` does not match `{source_id}`.", skel_event.seq, skeleton.source_id));
    }

    Ok((skel_event.seq, blob_hash.to_string(), skeleton))
}

/// Read one node from the pinned paper skeleton with hash-verified line-numbered source text,
/// structural dependency edges, dependents, and symbols bounded to `max_chars`.
pub fn read_map_node(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    let session = resolve_sources_session_id(session).map_err(|e| e.to_string())?;
    tools::registry::check_args(&read_map_node_def().input_schema, args)?;

    let source_id = args["source_id"].as_str().filter(|s| valid_id(s, 64)).ok_or("Invalid source_id.")?;
    let node_query = args["node"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("node query must be non-empty.")?;

    let (_, events) = ledger
        .read_verified_session(session)
        .map_err(|_| "The session ledger or a recorded artifact could not be verified.")?;
    let (skeleton_seq, _, skeleton) = load_pinned_skeleton(ledger, &events, source_id, None)?;
    let receipt = find_source_receipt(&events, source_id, skeleton.source_seq)
        .ok_or_else(|| format!("Source `{source_id}` has no source.ingested receipt in this session."))?;

    let target = resolve_node(&skeleton, source_id, node_query)?;
    let include = parse_include(args.get("include"));
    let max_chars = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(DEFAULT_READ_NODE_MAX_CHARS)
        .clamp(MIN_READ_NODE_MAX_CHARS, MAX_READ_NODE_MAX_CHARS);

    let proof_nodes: Vec<&SkNode> = if target.kind == SkNodeKind::Proof {
        Vec::new()
    } else if let Some(target_tag) = target.tag.as_deref() {
        skeleton.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof && n.proof_of.as_deref() == Some(target_tag)).collect()
    } else {
        Vec::new()
    };

    let mut receipts = BTreeMap::new();
    receipts.insert(source_id, receipt);
    let mut index = SourceIndex {
        sandbox: env.sandbox(),
        session,
        primary_source_id: source_id,
        receipts,
        files: BTreeMap::new(),
        bytes_left: MAX_TOTAL_BYTES,
        pages: None,
        history: &events,
        active_marker_seq: None,
    };

    let mut spans_to_read: Vec<(&'static str, &Span)> = Vec::new();
    if include.statement {
        spans_to_read.extend(target.spans.iter().map(|s| ("statement", s)));
    }
    if include.proof {
        for pnode in &proof_nodes {
            spans_to_read.extend(pnode.spans.iter().map(|s| ("proof", s)));
        }
    }
    // Fail closed up front: verify SHA-256 and line bounds of every requested physical span.
    for &(_, span) in &spans_to_read {
        let file = index.load(source_id, &span.path)?;
        validate_span_bounds(file, span)?;
    }

    let tag_map: BTreeMap<&str, &SkNode> = skeleton.nodes.iter().filter_map(|n| Some((n.tag.as_deref()?, n))).collect();
    let mut base = Map::new();
    base.insert("source_id".into(), json!(source_id));
    base.insert("skeleton_seq".into(), json!(skeleton_seq));
    base.insert("tag".into(), json!(target.tag));
    base.insert("kind".into(), json!(target.kind));
    base.insert("number".into(), json!(target.number));
    base.insert("title".into(), json!(target.title));
    base.insert("spans".into(), json!(target.spans));

    if include.proof {
        let proof_val = proof_nodes.first().map_or(Value::Null, |first| {
            let all_spans: Vec<&Span> = proof_nodes.iter().flat_map(|p| p.spans.iter()).collect();
            json!({ "tag": first.tag, "spans": all_spans })
        });
        base.insert("proof".into(), proof_val);
    }
    if include.uses {
        base.insert("uses".into(), build_uses(&skeleton, target, &tag_map));
    }
    if include.used_by {
        base.insert("used_by".into(), build_used_by(&skeleton, target, &tag_map));
    }
    if include.symbols {
        let syms = collect_symbols(&skeleton, target, &proof_nodes, source_id, &mut index)?;
        base.insert("symbols".into(), json!(syms));
    }

    let (emitted_files, truncated) = deliver_spans(&base, &spans_to_read, source_id, &mut index, max_chars)?;
    base.insert("files".into(), Value::Array(emitted_files));
    base.insert("truncated".into(), Value::Bool(truncated));
    Ok(Value::Object(base))
}

#[derive(Clone, Copy)]
struct IncludeFlags {
    statement: bool,
    proof: bool,
    uses: bool,
    used_by: bool,
    symbols: bool,
}

fn parse_include(raw: Option<&Value>) -> IncludeFlags {
    let Some(items) = raw.and_then(Value::as_array) else {
        return IncludeFlags { statement: true, proof: true, uses: true, used_by: true, symbols: true };
    };
    let has = |k: &str| items.iter().any(|v| v.as_str() == Some(k));
    IncludeFlags {
        statement: has("statement"),
        proof: has("proof"),
        uses: has("uses"),
        used_by: has("used_by"),
        symbols: has("symbols"),
    }
}

fn find_source_receipt<'a>(events: &'a [Event], source_id: &str, source_seq: u64) -> Option<&'a Event> {
    let matches = |e: &&Event| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id);
    events
        .iter()
        .find(|e| e.seq == source_seq && matches(e))
        .or_else(|| events.iter().rev().find(matches))
}

fn validate_span_bounds(file: &VerifiedFile, span: &Span) -> Result<(), String> {
    let total = file.line_count();
    if span.start_line == 0 || span.end_line < span.start_line || span.end_line > total {
        return Err(format!("Skeleton span {}:{}-{} is outside file ({total} lines).", span.path, span.start_line, span.end_line));
    }
    Ok(())
}

fn deliver_spans(
    base: &Map<String, Value>,
    spans_to_read: &[(&'static str, &Span)],
    source_id: &str,
    index: &mut SourceIndex<'_>,
    max_chars: usize,
) -> Result<(Vec<Value>, bool), String> {
    let build_val = |files: &[Value], truncated: bool| -> Value {
        let mut obj = base.clone();
        obj.insert("files".into(), Value::Array(files.to_vec()));
        obj.insert("truncated".into(), Value::Bool(truncated));
        Value::Object(obj)
    };
    let fits = |val: &Value| tools::canon::canonical_json(val).chars().count() <= max_chars;

    let mut emitted: Vec<Value> = Vec::new();
    let mut truncated = false;

    for (idx, &(section, span)) in spans_to_read.iter().enumerate() {
        let is_last = idx + 1 == spans_to_read.len();
        let file = index.load(source_id, &span.path)?;
        let total_lines = span.end_line - span.start_line + 1;
        let make_block = |count: usize| -> Value {
            json!({
                "section": section,
                "source_id": source_id,
                "path": span.path,
                "file_sha256": file.sha256,
                "start_line": span.start_line,
                "end_line": span.start_line + count - 1,
                "line_truncated": false,
                "text": format_lines(file, span.start_line, count),
            })
        };

        let mut trial = emitted.clone();
        trial.push(make_block(total_lines));
        if fits(&build_val(&trial, !is_last)) {
            emitted = trial;
            continue;
        }

        truncated = true;
        let max_partial = if is_last { total_lines.saturating_sub(1) } else { total_lines };
        if max_partial == 0 {
            break;
        }
        let mut first_trial = emitted.clone();
        first_trial.push(make_block(1));
        if !fits(&build_val(&first_trial, true)) {
            break;
        }
        let (mut lo, mut hi) = (1usize, max_partial);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            let mut cand = emitted.clone();
            cand.push(make_block(mid));
            if fits(&build_val(&cand, true)) { lo = mid; } else { hi = mid - 1; }
        }
        emitted.push(make_block(lo));
        break;
    }

    Ok((emitted, truncated))
}

fn format_lines(file: &VerifiedFile, start_line: usize, count: usize) -> String {
    let mut out = String::new();
    for line_no in start_line..(start_line + count) {
        let _ = writeln!(&mut out, "{line_no}\t{}", file.line_str(line_no));
    }
    out
}

fn build_uses(skeleton: &PaperSkeleton, target: &SkNode, tag_map: &BTreeMap<&str, &SkNode>) -> Value {
    let (mut statement, mut proof, mut prose) = (Vec::new(), Vec::new(), Vec::new());
    if let Some(target_tag) = target.tag.as_deref() {
        for edge in skeleton.edges.iter().filter(|e| e.kind == SkEdgeKind::Uses && e.to == target_tag) {
            let src = tag_map.get(edge.from.as_str()).copied();
            let entry = json!({
                "tag": edge.from, "kind": src.map(|n| n.kind),
                "number": src.and_then(|n| n.number.as_deref()),
                "title": src.and_then(|n| n.title.as_deref()),
                "rule": edge.rule, "confidence": edge.confidence,
            });
            match edge.role {
                SkEdgeRole::Statement => statement.push(entry),
                SkEdgeRole::Proof => proof.push(entry),
                SkEdgeRole::Prose => prose.push(entry),
            }
        }
    }
    json!({ "statement": statement, "proof": proof, "prose": prose })
}

fn build_used_by(skeleton: &PaperSkeleton, target: &SkNode, tag_map: &BTreeMap<&str, &SkNode>) -> Value {
    let mut out = Vec::new();
    if let Some(target_tag) = target.tag.as_deref() {
        for edge in skeleton.edges.iter().filter(|e| e.kind == SkEdgeKind::Uses && e.from == target_tag) {
            let dst = tag_map.get(edge.to.as_str()).copied();
            out.push(json!({
                "tag": edge.to, "kind": dst.map(|n| n.kind),
                "number": dst.and_then(|n| n.number.as_deref()),
                "title": dst.and_then(|n| n.title.as_deref()),
                "role": edge.role, "rule": edge.rule, "confidence": edge.confidence,
            }));
        }
    }
    Value::Array(out)
}

fn collect_symbols<'a>(
    skeleton: &'a PaperSkeleton,
    target: &SkNode,
    proof_nodes: &[&SkNode],
    source_id: &str,
    index: &mut SourceIndex<'_>,
) -> Result<Vec<&'a SymbolDef>, String> {
    if skeleton.symbols.is_empty() {
        return Ok(Vec::new());
    }
    let tag_order: BTreeMap<&str, usize> = skeleton.nodes.iter().filter_map(|n| Some((n.tag.as_deref()?, n.order))).collect();
    let target_tag = target.tag.as_deref();
    let proof_tags: BTreeSet<&str> = proof_nodes.iter().filter_map(|p| p.tag.as_deref()).collect();
    let direct_uses: BTreeSet<&str> = target_tag
        .map(|tt| skeleton.edges.iter().filter(|e| e.kind == SkEdgeKind::Uses && e.to == tt).map(|e| e.from.as_str()).collect())
        .unwrap_or_default();

    let mut body_text = String::new();
    for span in target.spans.iter().chain(proof_nodes.iter().flat_map(|p| p.spans.iter())) {
        let file = index.load(source_id, &span.path)?;
        validate_span_bounds(file, span)?;
        body_text.push_str(file.lines_text(span.start_line, span.end_line));
        body_text.push('\n');
    }

    let (mut local_syms, mut local_norms, mut preceding) = (Vec::new(), BTreeSet::new(), BTreeMap::new());
    for sym in &skeleton.symbols {
        let in_node = sym.in_node.as_deref();
        let defined_here = (target_tag.is_some() && in_node == target_tag)
            || in_node.is_some_and(|t| proof_tags.contains(t))
            || target.spans.iter().any(|s| s.contains(&sym.span))
            || proof_nodes.iter().any(|p| p.spans.iter().any(|s| s.contains(&sym.span)));
        if defined_here {
            local_norms.insert(sym.norm.as_str());
            local_syms.push(sym);
            continue;
        }
        let precedes = match in_node {
            Some(tag) => tag_order.get(tag).is_some_and(|&ord| ord <= target.order),
            None => target.spans.last().is_none_or(|last| last.path != sym.span.path || sym.span.start_line <= last.end_line),
        };
        if precedes && ((!sym.symbol_tex.is_empty() && body_text.contains(&sym.symbol_tex)) || in_node.is_some_and(|t| direct_uses.contains(t))) {
            preceding.insert(sym.norm.as_str(), sym);
        }
    }
    local_syms.extend(preceding.into_iter().filter(|(norm, _)| !local_norms.contains(norm)).map(|(_, s)| s));
    Ok(local_syms)
}

fn resolve_node<'a>(skeleton: &'a PaperSkeleton, source_id: &str, query: &str) -> Result<&'a SkNode, String> {
    if let Some(node) = skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(query)) {
        return Ok(node);
    }
    let label_matches: Vec<&SkNode> = skeleton.nodes.iter().filter(|n| n.label.as_deref() == Some(query)).collect();
    if label_matches.len() == 1 { return Ok(label_matches[0]); }
    if label_matches.len() > 1 { return Err(format_ambiguous(query, &label_matches)); }

    let parsed = parse_printed_ref(query);
    if let Some((kind, ref want_num)) = parsed {
        let matches: Vec<&SkNode> = skeleton
            .nodes
            .iter()
            .filter(|n| n.kind == kind && n.number.as_deref().is_some_and(|num| normalize_ref_number(num) == *want_num))
            .collect();
        if matches.len() == 1 { return Ok(matches[0]); }
        if matches.len() > 1 { return Err(format_ambiguous(query, &matches)); }
    }
    Err(format_not_found(skeleton, source_id, query, parsed.map(|(k, _)| k)))
}

fn parse_printed_ref(query: &str) -> Option<(SkNodeKind, String)> {
    let trimmed = query.trim();
    if let Some(rest) = trimmed.strip_prefix('§') {
        let num = normalize_ref_number(rest);
        return (!num.is_empty()).then_some((SkNodeKind::Section, num));
    }
    let prefix_len = trimmed.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
    if prefix_len == 0 { return None; }
    let word = trimmed[..prefix_len].to_ascii_lowercase();
    let tail = &trimmed[prefix_len..];
    let rest = tail.strip_prefix('.').or_else(|| tail.starts_with(char::is_whitespace).then_some(tail))?;
    let kind = match word.as_str() {
        "theorem" | "thm" => SkNodeKind::Theorem,
        "lemma" | "lem" => SkNodeKind::Lemma,
        "proposition" | "prop" => SkNodeKind::Proposition,
        "corollary" | "cor" => SkNodeKind::Corollary,
        "definition" | "def" | "defn" => SkNodeKind::Definition,
        "assumption" | "assump" | "asm" => SkNodeKind::Assumption,
        "claim" => SkNodeKind::Claim,
        "conjecture" | "conj" => SkNodeKind::Conjecture,
        "example" | "ex" => SkNodeKind::Example,
        "remark" | "rem" | "rmk" => SkNodeKind::Remark,
        "section" | "sec" | "subsection" | "subsec" | "appendix" | "app" => SkNodeKind::Section,
        "equation" | "eq" | "eqn" => SkNodeKind::Equation,
        "figure" | "fig" => SkNodeKind::Figure,
        "table" | "tab" => SkNodeKind::Table,
        "algorithm" | "alg" => SkNodeKind::Algorithm,
        _ => return None,
    };
    let num = normalize_ref_number(rest);
    (!num.is_empty()).then_some((kind, num))
}

fn normalize_ref_number(raw: &str) -> String {
    let mut s = raw.trim();
    while let Some(inner) = s.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        s = inner.trim();
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

fn format_ambiguous(query: &str, matches: &[&SkNode]) -> String {
    let list = matches.iter().map(|n| format_node_brief(n)).collect::<Vec<_>>().join("; ");
    format!("Ambiguous node reference `{query}` matches {} nodes: {list}. Pass an exact tag.", matches.len())
}

fn format_not_found(skeleton: &PaperSkeleton, source_id: &str, query: &str, preferred: Option<SkNodeKind>) -> String {
    let mut picked: Vec<&SkNode> = Vec::with_capacity(MAX_SUGGESTED_NODES);
    if let Some(kind) = preferred {
        picked.extend(skeleton.nodes.iter().filter(|n| n.tag.is_some() && n.kind == kind).take(MAX_SUGGESTED_NODES));
    }
    for n in skeleton.nodes.iter().filter(|n| n.tag.is_some()) {
        if picked.len() >= MAX_SUGGESTED_NODES { break; }
        if !picked.iter().any(|p| p.order == n.order) { picked.push(n); }
    }
    if picked.is_empty() {
        return format!("Node `{query}` not found in skeleton for `{source_id}` (skeleton has no tagged nodes).");
    }
    let list = picked.iter().map(|n| format_node_brief(n)).collect::<Vec<_>>().join("; ");
    format!("Node `{query}` not found in skeleton for `{source_id}`. Available nodes: {list}")
}

fn format_node_brief(node: &SkNode) -> String {
    let tag = node.tag.as_deref().unwrap_or("<untagged>");
    let kind_val = serde_json::to_value(node.kind).ok();
    let kind = kind_val.as_ref().and_then(Value::as_str).unwrap_or("node");
    let mut out = format!("{tag} ({kind}");
    if let Some(num) = node.number.as_deref().filter(|s| !s.is_empty()) { let _ = write!(&mut out, " {num}"); }
    if let Some(title) = node.title.as_deref().filter(|s| !s.is_empty()) { let _ = write!(&mut out, ": {title}"); }
    if let Some(label) = node.label.as_deref().filter(|s| !s.is_empty()) { let _ = write!(&mut out, ", label={label}"); }
    out.push(')');
    out
}

#[cfg(test)]
#[path = "map_node_read_tests.rs"]
mod tests;
