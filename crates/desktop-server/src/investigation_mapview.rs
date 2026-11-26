//! Investigate mode mapview projection and hash-checked node text extraction (package V3).
//!
//! Projects a deterministic [`PaperSkeleton`] into the shared `GraphView` JSON shape
//! (`00-overview.md` §2.6) plus Investigate metadata without embedding node body prose
//! (`investigate.md` §5.2), and reads exact physical source spans on demand from the
//! hash-pinned `source.ingested` ledger receipt.

use ledger::Event;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    load_ingested_pdf_source, load_ingested_tex_source, node_page_basis, page_range_for_node,
    render_formulas_as_math, EdgeConfidence, IngestedPdfSource, PaperSkeleton, SkEdge, SkEdgeKind,
    SkNode, SkNodeKind, SkeletonBuilder, SkeletonQuality, Span, PDF_QUALITY_REASON_BASE,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Mode display name (`00-overview.md` §3.2, §8).
pub const MODE_DISPLAY_NAME: &str = "Investigate";
/// Deterministic code-grade badge label (`00-overview.md` §2.10, §3.2 row 20).
/// User-facing Investigate strings never use the word "verified" or "Verified".
pub const BADGE_BY_CODE: &str = "By code";
/// Quality banner prefix for a full TeX skeleton (`00-overview.md` §5.2).
pub const QUALITY_TEX_FULL: &str = "Structure read from TeX source";
/// Quality banner prefix for a partial TeX skeleton (`00-overview.md` §5.2).
pub const QUALITY_TEX_PARTIAL: &str = "Structure read from TeX source (partial)";
/// Quality banner prefix for a weak TeX skeleton (`00-overview.md` §5.2).
pub const QUALITY_TEX_WEAK: &str = "Structure read from TeX source (weak)";

const QUALITY_HTML: &str = "Structure read from arXiv HTML";
const QUALITY_PDF: &str = "Estimated from PDF text; theorem boundaries may be wrong";

/// Formats the header quality line describing how a [`PaperSkeleton`] was constructed
/// (`00-overview.md` §5.2; `investigate.md` §3).
pub fn format_quality_line(skeleton: &PaperSkeleton) -> String {
    let base = match (skeleton.builder, skeleton.quality) {
        (SkeletonBuilder::Tex1, SkeletonQuality::Full) => QUALITY_TEX_FULL,
        (SkeletonBuilder::Tex1, SkeletonQuality::Partial) => QUALITY_TEX_PARTIAL,
        (SkeletonBuilder::Tex1, SkeletonQuality::Weak) => QUALITY_TEX_WEAK,
        (SkeletonBuilder::Html1, _) => QUALITY_HTML,
        (SkeletonBuilder::Pdf1, _) => QUALITY_PDF,
    };
    let mut out = base.to_string();
    if let Some(ver) = non_empty(skeleton.version.as_deref()) {
        out.push_str(", ");
        out.push_str(ver);
    }
    if let Some(reason) = non_empty(skeleton.quality_reason.as_deref()) {
        let extra = if skeleton.builder == SkeletonBuilder::Pdf1 {
            let trimmed = if reason.len() >= PDF_QUALITY_REASON_BASE.len()
                && reason[..PDF_QUALITY_REASON_BASE.len()]
                    .eq_ignore_ascii_case(PDF_QUALITY_REASON_BASE)
            {
                reason[PDF_QUALITY_REASON_BASE.len()..].trim_start_matches([';', ' '])
            } else {
                reason
            };
            non_empty(Some(trimmed))
        } else {
            Some(reason)
        };
        if let Some(detail) = extra {
            out.push_str(" — ");
            out.push_str(detail);
        }
    }
    out
}

/// Projects a [`PaperSkeleton`] into the shared `GraphView` JSON shape (`00-overview.md` §2.6)
/// plus Investigate metadata, strictly omitting node body text (`investigate.md` §5.2).
pub fn build_mapview_json(
    skeleton: &PaperSkeleton,
    skeleton_seq: u64,
    skeleton_blob: &str,
    ledger_head: &str,
    fallback_note: Option<&str>,
) -> Value {
    build_mapview_json_with_fold(skeleton, skeleton_seq, skeleton_blob, ledger_head, fallback_note, None)
}

/// Projects a [`PaperSkeleton`] plus optional folded Layer-2 records (`map_fold::FoldedMap`)
/// into the shared `GraphView` JSON shape (`investigate.md` §4.3, §5.2).
pub fn build_mapview_json_with_fold(
    skeleton: &PaperSkeleton,
    skeleton_seq: u64,
    skeleton_blob: &str,
    ledger_head: &str,
    fallback_note: Option<&str>,
    folded: Option<&research_tools::map_fold::FoldedMap>,
) -> Value {
    let ids: Vec<String> = skeleton.nodes.iter().map(node_id).collect();
    let labels: Vec<String> = skeleton.nodes.iter().map(format_node_label).collect();
    let mut id_to_idx: HashMap<String, usize> = HashMap::with_capacity(ids.len());
    let mut node_tags: HashSet<String> = HashSet::with_capacity(ids.len());
    for (idx, id) in ids.iter().enumerate() {
        id_to_idx.entry(id.clone()).or_insert(idx);
        node_tags.insert(id.clone());
    }

    // Reverse lookup: target statement tag -> attached Proof node tag.
    let mut proof_for: HashMap<&str, String> = HashMap::new();
    for (idx, node) in skeleton.nodes.iter().enumerate() {
        if node.kind == SkNodeKind::Proof {
            if let Some(target) = node.proof_of.as_deref() {
                proof_for.entry(target).or_insert_with(|| ids[idx].clone());
            }
        }
    }

    // Collect sorted unique bibliography citations targeting each node.
    let mut cites_by_node: HashMap<&str, BTreeMap<String, &SkEdge>> = HashMap::new();
    for edge in &skeleton.edges {
        if edge.kind == SkEdgeKind::Cites {
            cites_by_node
                .entry(edge.to.as_str())
                .or_default()
                .entry(edge.from.clone())
                .or_insert(edge);
        }
    }

    let nodes: Vec<Value> = skeleton
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let id = &ids[idx];
            let group = resolve_node_group(&skeleton.nodes, idx, &labels, &id_to_idx);
            let flags: Vec<String> = if node.head_cites.is_empty() {
                Vec::new()
            } else {
                vec![format!("cites [{}] in its heading", node.head_cites.join(", "))]
            };
            let cites: Vec<Value> = cites_by_node
                .get(id.as_str())
                .map(|m| {
                    m.iter()
                        .map(|(k, e)| json!({ "key": k, "role": e.role, "rule": e.rule, "site": e.site }))
                        .collect()
                })
                .unwrap_or_default();
            let reading = folded.and_then(|f| f.nodes.get(id));
            let stale = reading.is_some_and(|r| r.any_stale);
            let mut obj = json!({
                "id": id, "kind": node.kind, "label": labels[idx],
                "group": group, "parent": node.parent, "grade": "code",
                "badges": [{ "text": BADGE_BY_CODE, "icon": "code", "tone": "ok" }],
                "flags": flags, "stale": stale, "dim": false,
                "order": node.order, "number": node.number, "number_basis": node.number_basis,
                "env_name": node.env_name, "title": node.title, "tex_label": node.label,
                "spans": node.spans, "proof_of": node.proof_of,
                "proof_tag": proof_for.get(id.as_str()).cloned(),
                "head_cites": node.head_cites, "cites": cites, "intro_refs": node.intro_refs,
                "fp": node.fp, "words": node.words, "eq_count": node.eq_count,
                "read_minutes": node.read_minutes, "confidence": node.confidence,
            });
            if let Some(r) = reading {
                let mut rv = serde_json::to_value(r).unwrap_or(Value::Null);
                strip_anchor_excerpts(&mut rv);
                obj["reading"] = rv;
            }
            obj
        })
        .collect();

    // Emit only node-to-node edges with live endpoints; Cites live on node.cites & bib.
    let mut edges: Vec<Value> = skeleton
        .edges
        .iter()
        .filter(|e| {
            matches!(e.kind, SkEdgeKind::Uses | SkEdgeKind::Proves | SkEdgeKind::Contains)
                && node_tags.contains(&e.from)
                && node_tags.contains(&e.to)
        })
        .map(|e| {
            let basis = match e.confidence {
                EdgeConfidence::Explicit => "code",
                EdgeConfidence::Heuristic => "model",
            };
            json!({
                "src": e.from, "dst": e.to, "kind": e.kind, "role": e.role,
                "basis": basis, "rule": e.rule, "confidence": e.confidence, "site": e.site,
            })
        })
        .collect();
    if let Some(f) = folded {
        for me in &f.edges {
            if node_tags.contains(&me.from) && node_tags.contains(&me.to) {
                let mut edge_val = json!({
                    "src": me.from, "dst": me.to, "kind": "uses", "role": me.role,
                    "basis": "model", "model_edge": true,
                    "edge_basis": me.current.value.basis,
                    "reason": me.current.value.reason,
                    "anchor": me.current.value.anchor,
                    "history": me.history,
                });
                strip_anchor_excerpts(&mut edge_val);
                edges.push(edge_val);
            }
        }
    }

    let tasks = folded
        .map(|f| serde_json::to_value(&f.tasks).unwrap_or_else(|_| json!([])))
        .unwrap_or_else(|| json!([]));
    let legend = json!([
        { "label": "By code (skeleton)", "icon": "code", "tone": "ok", "dashed": false },
        { "label": "Heuristic reference (not checked by code)", "icon": "model", "tone": "muted", "dashed": true },
    ]);

    let skip_plan = research_tools::skeleton::compute_skip_plan(skeleton, &[], &[]);
    json!({
        "etag": ledger_head, "skeleton_seq": skeleton_seq, "skeleton_blob": skeleton_blob,
        "source_id": skeleton.source_id, "source_seq": skeleton.source_seq,
        "paper_key": skeleton.paper_key, "version": skeleton.version,
        "builder": skeleton.builder, "quality": skeleton.quality,
        "quality_reason": skeleton.quality_reason, "quality_line": format_quality_line(skeleton),
        "nodes": nodes, "edges": edges, "legend": legend,
        "stats": skeleton.stats, "bib": skeleton.bib, "symbols": skeleton.symbols,
        "macros": skeleton.macros, "skip_plan": skip_plan, "tasks": tasks, "carry": skeleton.carry,
        "fallback_note": fallback_note,
    })
}

/// Builds the `GraphView` mapview JSON with a computed [`research_tools::skeleton::SkipPlan`]
/// for the given `main_result_tags` and optional extra/model edges, setting `dim: true` on
/// nodes belonging to unreferenced sections (Rule 6.14).
pub fn build_mapview_json_with_plan(
    skeleton: &PaperSkeleton,
    skeleton_seq: u64,
    skeleton_blob: &str,
    ledger_head: &str,
    main_result_tags: &[String],
    extra_edges: &[research_tools::skeleton::ExtraUseEdge],
    fallback_note: Option<&str>,
) -> Value {
    let mut mv = build_mapview_json(skeleton, skeleton_seq, skeleton_blob, ledger_head, fallback_note);
    let plan = research_tools::skeleton::compute_skip_plan(skeleton, main_result_tags, extra_edges);
    if !plan.empty {
        let dimmed_secs: BTreeSet<&str> = plan
            .sections
            .iter()
            .filter(|s| s.dim)
            .map(|s| s.section_tag.as_str())
            .collect();
        let node_by_tag: BTreeMap<&str, &SkNode> = skeleton
            .nodes
            .iter()
            .filter_map(|n| n.tag.as_deref().map(|t| (t, n)))
            .collect();
        if let Some(arr) = mv["nodes"].as_array_mut() {
            for item in arr.iter_mut() {
                let id = item["id"].as_str().unwrap_or("");
                let mut cur = node_by_tag.get(id).copied();
                let mut steps = 0usize;
                let mut sec_tag: Option<&str> = None;
                while let Some(n) = cur {
                    if n.kind == SkNodeKind::Section {
                        sec_tag = n.tag.as_deref();
                        break;
                    }
                    cur = n.parent.as_deref().and_then(|p| node_by_tag.get(p).copied());
                    steps += 1;
                    if steps > skeleton.nodes.len() {
                        break;
                    }
                }
                if sec_tag.is_some_and(|st| dimmed_secs.contains(st)) {
                    item["dim"] = json!(true);
                    if let Some(flags) = item["flags"].as_array_mut() {
                        let msg = json!(research_tools::skeleton::SKIP_LABEL_UNREFERENCED);
                        if !flags.contains(&msg) {
                            flags.push(msg);
                        }
                    }
                }
            }
        }
    }
    mv["skip_plan"] = json!(plan);
    mv
}

/// Attaches dynamic physical PDF page ranges, OCR basis, and `render_math` flags (`investigate.md`
/// rule 6.21) to a built `mapview` JSON without storing page numbers on [`SkNode`].
pub fn attach_pdf_page_metadata(mv: &mut Value, skeleton: &PaperSkeleton, pdf: &IngestedPdfSource) {
    let node_by_tag: HashMap<String, &SkNode> =
        skeleton.nodes.iter().map(|n| (node_id(n), n)).collect();
    if let Some(arr) = mv["nodes"].as_array_mut() {
        for item in arr.iter_mut() {
            let id = item["id"].as_str().unwrap_or("");
            if let Some(node) = node_by_tag.get(id) {
                let page_range = page_range_for_node(&pdf.page_map, node);
                let basis = node_page_basis(&pdf.page_map, node);
                item["page_range"] = json!(page_range);
                item["page_basis"] = json!(basis);
                item["render_math"] = json!(basis.is_some_and(render_formulas_as_math));
            }
        }
    }
    let image_only: Vec<usize> = pdf
        .page_map
        .pages
        .iter()
        .filter(|p| p.basis == research_tools::pdf_pages::PageBasis::None)
        .map(|p| p.page)
        .collect();
    mv["image_only_pages"] = json!(image_only);
}

/// Reads the exact hash-checked physical TeX or PDF lines for `tag` (and its attached proof, if any)
/// from the `source.ingested` receipt under `sandbox`.
pub fn read_verified_node_text(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    skeleton: &PaperSkeleton,
    tag: &str,
) -> Result<Value, String> {
    let node = skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(tag))
        .ok_or_else(|| format!("skeleton node `{tag}` not found"))?;
    let proof_node = skeleton.nodes.iter()
        .find(|p| p.kind == SkNodeKind::Proof && p.proof_of.as_deref() == Some(tag));
    if skeleton.builder == SkeletonBuilder::Pdf1 {
        let pdf = load_ingested_pdf_source(sandbox, session_id, receipt)?;
        let files = BTreeMap::from([(pdf.primary_path.clone(), pdf.text.clone())]);
        let text = extract_spans_text(&files, &node.spans)?;
        let (proof_tag, proof_spans, proof_text) = match proof_node {
            Some(p) => (p.tag.clone(), Some(p.spans.clone()), Some(extract_spans_text(&files, &p.spans)?)),
            None => (None, None, None),
        };
        let basis = node_page_basis(&pdf.page_map, node);
        return Ok(json!({
            "tag": tag, "kind": node.kind, "number": node.number, "title": node.title,
            "spans": node.spans, "text": text,
            "page_range": page_range_for_node(&pdf.page_map, node),
            "page_basis": basis,
            "render_math": basis.is_some_and(render_formulas_as_math),
            "proof_tag": proof_tag, "proof_spans": proof_spans, "proof_text": proof_text,
        }));
    }
    let ingested = load_ingested_tex_source(sandbox, session_id, receipt)?;
    let text = extract_spans_text(&ingested.files, &node.spans)?;
    let (proof_tag, proof_spans, proof_text) = match proof_node {
        Some(p) => (p.tag.clone(), Some(p.spans.clone()), Some(extract_spans_text(&ingested.files, &p.spans)?)),
        None => (None, None, None),
    };
    Ok(json!({
        "tag": tag, "kind": node.kind, "number": node.number, "title": node.title,
        "spans": node.spans, "text": text,
        "proof_tag": proof_tag, "proof_spans": proof_spans, "proof_text": proof_text,
    }))
}

fn non_empty(s: Option<&str>) -> Option<&str> { s.map(str::trim).filter(|t| !t.is_empty()) }

fn strip_anchor_excerpts(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.remove("excerpt");
            for child in map.values_mut() {
                strip_anchor_excerpts(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_anchor_excerpts(child);
            }
        }
        _ => {}
    }
}

fn node_id(node: &SkNode) -> String { node.tag.clone().unwrap_or_else(|| format!("ord-{}", node.order)) }

fn kind_display_name(kind: SkNodeKind) -> &'static str {
    use SkNodeKind::*;
    match kind {
        FrontMatter => "Front matter", Section => "Section", Definition => "Definition",
        Assumption => "Assumption", Theorem => "Theorem", Lemma => "Lemma",
        Proposition => "Proposition", Corollary => "Corollary", Conjecture => "Conjecture",
        Claim => "Claim", Proof => "Proof", Example => "Example", Remark => "Remark",
        Equation => "Equation", Figure => "Figure", Table => "Table",
        Algorithm => "Algorithm", Bibliography => "Bibliography",
    }
}

fn format_node_label(node: &SkNode) -> String {
    let num = non_empty(node.number.as_deref());
    let title = non_empty(node.title.as_deref());
    let head = match (node.kind, num) {
        (SkNodeKind::Equation, Some(n)) => format!("Eq. ({n})"),
        (SkNodeKind::Equation, None) => "Equation".to_string(),
        (SkNodeKind::Proof, _) => match title {
            Some(t) if t.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("proof")) => t.to_string(),
            Some(t) => format!("Proof: {t}"),
            None => "Proof".to_string(),
        },
        (k, Some(n)) => format!("{} {n}", kind_display_name(k)),
        (k, None) => kind_display_name(k).to_string(),
    };
    match title {
        Some(t) if node.kind != SkNodeKind::Proof => format!("{head}: {t}"),
        _ => head,
    }
}

fn resolve_node_group(
    nodes: &[SkNode], idx: usize, labels: &[String], id_to_idx: &HashMap<String, usize>,
) -> Option<String> {
    let mut cur = nodes[idx].parent.as_deref();
    for _ in 0..nodes.len() {
        let Some(&p_idx) = cur.and_then(|t| id_to_idx.get(t)) else { break };
        match nodes[p_idx].kind {
            SkNodeKind::Section => return Some(labels[p_idx].clone()),
            SkNodeKind::FrontMatter => return Some("Front matter".to_string()),
            SkNodeKind::Bibliography => return Some("Back matter".to_string()),
            _ => cur = nodes[p_idx].parent.as_deref(),
        }
    }
    match nodes[idx].kind {
        SkNodeKind::Section => Some(labels[idx].clone()),
        SkNodeKind::FrontMatter => Some("Front matter".to_string()),
        SkNodeKind::Bibliography => Some("Back matter".to_string()),
        _ => None,
    }
}

fn extract_spans_text(files: &BTreeMap<String, String>, spans: &[Span]) -> Result<String, String> {
    let mut segments = Vec::with_capacity(spans.len());
    for span in spans {
        if span.start_line == 0 || span.end_line < span.start_line {
            return Err(format!("invalid span {}:{}-{}", span.path, span.start_line, span.end_line));
        }
        let content = files.get(&span.path)
            .ok_or_else(|| format!("source file `{}` not present in ingested receipt", span.path))?;
        let lines: Vec<&str> = content.lines().map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        let start_idx = span.start_line.saturating_sub(1);
        if start_idx >= lines.len() {
            return Err(format!("span {}:{}-{} exceeds file length", span.path, span.start_line, span.end_line));
        }
        segments.push(lines[start_idx..span.end_line.min(lines.len())].join("\n"));
    }
    Ok(segments.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_tools::paper_key::PaperKey;
    use research_tools::sandbox::SandboxRoot;
    use research_tools::skeleton::{build_tex_skeleton, TexSkeletonInput};

    fn sample_tex() -> String {
        concat!(
            "\\documentclass{article}\n",
            "\\newtheorem{theorem}{Theorem}[section]\n",
            "\\newtheorem{lemma}[theorem]{Lemma}\n",
            "\\begin{document}\n",
            "\\section{Setup}\n",
            "\\begin{lemma}[\\cite{Toy98}]\n\\label{lem:base}\nUNIQUE_LEMMA_BODY_SENTINEL.\n\\end{lemma}\n",
            "\\begin{theorem}[Discrete maximum principle]\n\\label{thm:main}\n",
            "By \\ref{lem:base}, UNIQUE_THM_BODY_SENTINEL.\n\\end{theorem}\n",
            "\\begin{proof}\nFollows from Lemma 1.1 and \\cite{Alpha01}, UNIQUE_PROOF_SENTINEL.\n\\end{proof}\n",
            "\\begin{thebibliography}{9}\n\\bibitem{Toy98} Toy, 1998.\n\\bibitem{Alpha01} Alpha, 2001.\n\\end{thebibliography}\n",
            "\\end{document}\n",
        )
        .to_string()
    }

    fn build_sample_skeleton() -> PaperSkeleton {
        let mut files = BTreeMap::new();
        files.insert("main.tex".to_string(), sample_tex());
        build_tex_skeleton(&TexSkeletonInput {
            source_id: "src-1".into(),
            source_seq: 3,
            paper_key: PaperKey::from_arxiv("2401.00001").unwrap(),
            version: Some("v2".into()),
            files,
            primary_path: Some("main.tex".into()),
            files_sha256: None,
        })
        .skeleton
    }

    #[test]
    fn mapview_shape_edges_flags_and_no_body_or_forbidden_word() {
        let sk = build_sample_skeleton();
        let mv = build_mapview_json(&sk, 7, "blob-sha", "head-abc", None);
        assert_eq!(mv["etag"], "head-abc");
        assert_eq!(mv["skeleton_seq"], 7);
        assert_eq!(mv["quality_line"], "Structure read from TeX source, v2");
        assert_eq!(mv["skip_plan"]["empty"], json!(true));
        assert_eq!(mv["skip_plan"]["basis"], json!("explicit"));
        assert!(mv["symbols"].is_array());
        assert_eq!(mv["tasks"], json!([]));

        let serialized = serde_json::to_string(&mv).unwrap();
        assert!(!serialized.to_ascii_lowercase().contains("verified"));
        assert!(!serialized.contains("UNIQUE_LEMMA_BODY_SENTINEL"));
        assert!(!serialized.contains("UNIQUE_THM_BODY_SENTINEL"));
        assert!(!serialized.contains("UNIQUE_PROOF_SENTINEL"));

        let nodes = mv["nodes"].as_array().unwrap();
        let lem = nodes.iter().find(|n| n["kind"] == "lemma").unwrap();
        assert_eq!(lem["grade"], "code");
        assert_eq!(lem["group"], "Section 1: Setup");
        assert_eq!(lem["badges"][0]["text"], BADGE_BY_CODE);
        assert_eq!(lem["flags"], json!(["cites [Toy98] in its heading"]));
        assert_eq!(lem["cites"][0]["key"], "Toy98");

        let thm = nodes.iter().find(|n| n["kind"] == "theorem").unwrap();
        assert_eq!(thm["label"], "Theorem 1.2: Discrete maximum principle");
        assert!(thm["proof_tag"].as_str().is_some());
        assert_eq!(thm["cites"][0]["key"], "Alpha01");

        let edges = mv["edges"].as_array().unwrap();
        assert!(edges.iter().all(|e| e["kind"] != "cites"));
        assert!(edges.iter().any(|e| e["src"] == lem["id"] && e["dst"] == thm["id"] && e["basis"] == "code"));
        assert!(edges.iter().any(|e| e["src"] == lem["id"] && e["dst"] == thm["id"] && e["basis"] == "model"));
    }

    #[test]
    fn format_quality_line_covers_builders_qualities_and_suffixes() {
        let mut sk = build_sample_skeleton();
        sk.version = None;
        sk.quality_reason = None;
        sk.quality = SkeletonQuality::Full;
        assert_eq!(format_quality_line(&sk), QUALITY_TEX_FULL);

        sk.quality = SkeletonQuality::Partial;
        sk.version = Some("v3".into());
        assert_eq!(format_quality_line(&sk), "Structure read from TeX source (partial), v3");

        sk.quality = SkeletonQuality::Weak;
        sk.quality_reason = Some("unclosed env".into());
        assert_eq!(format_quality_line(&sk), "Structure read from TeX source (weak), v3 — unclosed env");

        sk.quality_reason = Some("   ".into());
        assert_eq!(format_quality_line(&sk), "Structure read from TeX source (weak), v3");

        sk.builder = SkeletonBuilder::Html1;
        sk.version = None;
        sk.quality_reason = None;
        assert_eq!(format_quality_line(&sk), "Structure read from arXiv HTML");

        sk.builder = SkeletonBuilder::Pdf1;
        assert_eq!(format_quality_line(&sk), "Estimated from PDF text; theorem boundaries may be wrong");
    }

    #[test]
    fn read_verified_node_text_reads_statement_and_attached_proof() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = WorkspaceSandbox::new(dir.path().to_path_buf(), None);
        let tex = sample_tex();
        let write = sandbox.resolve_for_write(SandboxRoot::Sources, "s1", "main.tex").unwrap();
        std::fs::write(&write.target_path, &tex).unwrap();
        let receipt = Event {
            session: "s1".into(),
            seq: 3,
            ts_ms: 0,
            kind: "source.ingested".into(),
            schema_version: 1,
            payload: json!({
                "source_id": "src-1",
                "primary_path": "main.tex",
                "files": ["main.tex"],
                "files_sha256": { "main.tex": ledger::sha256_hex(tex.as_bytes()) },
            }),
            prev_hash: String::new(),
            event_hash: String::new(),
        };
        let sk = build_sample_skeleton();
        let thm_tag = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap().tag.as_deref().unwrap();
        let out = read_verified_node_text(&sandbox, "s1", &receipt, &sk, thm_tag).unwrap();
        assert_eq!(out["tag"], thm_tag);
        assert!(out["text"].as_str().unwrap().contains("UNIQUE_THM_BODY_SENTINEL"));
        assert!(out["proof_text"].as_str().unwrap().contains("UNIQUE_PROOF_SENTINEL"));
        assert!(out["proof_tag"].as_str().is_some());
    }
}
