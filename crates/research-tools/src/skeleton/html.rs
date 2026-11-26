//! Deterministic HTML5 / LaTeXML skeleton builder (`html-1`, Investigate V18).
//!
//! Parses LaTeXML HTML5 documents (such as `https://arxiv.org/html/<id>`) using the
//! bounded zero-dependency lexer in [`super::html_lex`] and produces a section 4.1
//! [`PaperSkeleton`] with:
//! - `builder: SkeletonBuilder::Html1`, `confidence: SkConfidence::HtmlClass`,
//!   `number_basis: Some(NumberBasis::Html)` on numbered nodes.
//! - Permanent `n-...` V2 tags via [`super::tags::assign_tags`] (invariant to CRLF vs LF,
//!   HTML comments, and rendered tag renumbering).
//! - Physical 1-based source [`Span`](super::model::Span)s verifiable by
//!   `quote_core::verify_quote` and compatible with `locate_in_skeleton`.
//!
//! # Inferred LaTeXML Class Conventions (Assumptions)
//!
//! Because re:Search carries zero third-party LaTeXML fixtures or dependencies (Decision 12),
//! class recognition encodes standard arXiv/LaTeXML HTML5 conventions as explicit rules:
//! - Sections: `ltx_part`, `ltx_chapter`, `ltx_section`, `ltx_appendix`, `ltx_subsection`,
//!   `ltx_subsubsection`; front matter: `ltx_abstract`.
//! - Theorem-like blocks: `ltx_theorem` and `ltx_theorem_<kind>` (e.g. `ltx_theorem_lemma`,
//!   `ltx_theorem_definition`, `ltx_theorem_proposition`, `ltx_theorem_corollary`,
//!   `ltx_theorem_assumption`, `ltx_theorem_conjecture`, `ltx_theorem_claim`,
//!   `ltx_theorem_example`, `ltx_theorem_remark`, `ltx_theorem_algorithm`); fallback kind
//!   resolution checks `ltx_tag_theorem` / `ltx_title_theorem` leading kind words.
//! - Proofs: `ltx_proof` (linked via `ltx_title_proof` `ltx_ref`, heading kind+number, or
//!   nearest preceding provable statement under the same parent).
//! - Equations: `ltx_equation` / `ltx_equationgroup` emit `SkNodeKind::Equation` only when
//!   numbered/labelled (`ltx_tag_equation`, `ltx_tag_equationgroup`, `ltx_eqn_eqno`, or
//!   `ltx_tag`); unlabelled display equations increment `eq_count` on their parent node.
//! - Figures, tables, bibliography, refs/cites: `ltx_figure`, `ltx_table` (excluding
//!   layout `ltx_tabular` / `ltx_eqn_table`), `ltx_bibliography`, `ltx_bibitem`, `ltx_ref`,
//!   `ltx_cite`, `ltx_tag` / `ltx_tag_*`, `<math alttext="...">` (ignoring duplicate
//!   `<annotation>` / `<annotation-xml>` child text).

pub use super::html_classify::is_latexml_html_document;
use super::html_bib::compute_html_stats;
use super::html_edges::finalize_html_graph;
use super::html_lex::{scan_html_tokens, MAX_HTML_BYTES};
use super::html_walk::HtmlWalker;
use super::model::{
    compute_files_root_sha256, PaperSkeleton, SkEdgeKind, SkeletonBuilder, SkeletonQuality,
    MAX_SKELETON_JSON_BYTES, SKELETON_SCHEMA_VERSION,
};
use super::tags::assign_tags;
use super::BuildTiming;
use crate::paper_key::PaperKey;
use crate::sandbox::{SandboxRoot, WorkspaceSandbox};
use ledger::Event;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::time::Instant;

/// Verified physical HTML file loaded from one `source.ingested` ledger receipt.
#[derive(Debug, Clone)]
pub struct IngestedHtmlSource {
    pub source_id: String,
    pub source_seq: u64,
    pub primary_path: String,
    pub html: String,
    pub files_sha256: BTreeMap<String, String>,
    pub files_root_sha256: String,
}

/// Pure in-memory input to the deterministic `html-1` skeleton builder.
#[derive(Debug, Clone)]
pub struct HtmlSkeletonInput {
    pub source_id: String,
    pub source_seq: u64,
    pub paper_key: PaperKey,
    pub version: Option<String>,
    pub primary_path: String,
    pub html: String,
    pub files_sha256: Option<BTreeMap<String, String>>,
}

/// Complete output of the `html-1` skeleton builder.
#[derive(Debug, Clone)]
pub struct HtmlSkeletonOutput {
    pub skeleton: PaperSkeleton,
    pub parent_orders: Vec<Option<usize>>,
    pub quality_notes: Vec<String>,
    pub document_title: Option<String>,
}

/// True iff `receipt` is an ingested HTML source eligible for `html-1` skeleton building.
pub fn is_html_receipt(receipt: &Event) -> bool {
    if receipt.kind != "source.ingested" || receipt.payload["status"].as_str() != Some("ok") {
        return false;
    }
    let p = &receipt.payload;
    let has_tex = p["files"].as_array().is_some_and(|arr| {
        arr.iter().filter_map(|v| v.as_str()).any(|rel| {
            let low = rel.to_ascii_lowercase();
            low.ends_with(".tex") || low.ends_with(".ltx")
        })
    });
    if has_tex {
        return false;
    }
    let fmt = p["format"].as_str().unwrap_or("");
    matches!(fmt, "html" | "htm" | "xhtml")
        || p["primary_path"].as_str().is_some_and(has_html_extension)
}

fn has_html_extension(path: &str) -> bool {
    let low = path.to_ascii_lowercase();
    low.ends_with(".html") || low.ends_with(".htm") || low.ends_with(".xhtml")
}

/// Loads a hash-verified HTML file from a `source.ingested` ledger receipt under `sandbox`.
pub fn load_ingested_html_source(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
) -> Result<IngestedHtmlSource, String> {
    if receipt.kind != "source.ingested" {
        return Err(format!("expected `source.ingested` event, got `{}`", receipt.kind));
    }
    if !is_html_receipt(receipt) {
        return Err("source.ingested receipt is not an HTML source".to_string());
    }
    let data = &receipt.payload;
    let source_id = data["source_id"]
        .as_str()
        .ok_or_else(|| "source.ingested receipt missing `source_id`".to_string())?
        .to_string();
    let primary_path = data["primary_path"]
        .as_str()
        .ok_or_else(|| "source.ingested receipt missing `primary_path`".to_string())?
        .to_string();
    let files_arr = data["files"]
        .as_array()
        .ok_or_else(|| "source.ingested receipt missing `files` array".to_string())?;
    if !files_arr.iter().any(|v| v.as_str() == Some(primary_path.as_str())) {
        return Err(format!("`primary_path` `{primary_path}` is not listed in receipt `files`"));
    }
    let hashes_obj = data["files_sha256"]
        .as_object()
        .ok_or_else(|| "source.ingested receipt missing `files_sha256` map".to_string())?;
    let mut files_sha256 = BTreeMap::new();
    for (k, v) in hashes_obj {
        let digest = v
            .as_str()
            .ok_or_else(|| format!("non-string digest for `{k}` in files_sha256"))?;
        if digest.len() != 64 || !digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!("invalid sha256 digest for `{k}` in files_sha256"));
        }
        files_sha256.insert(k.clone(), digest.to_string());
    }
    let expected_sha = files_sha256
        .get(&primary_path)
        .ok_or_else(|| format!("files_sha256 missing digest for `{primary_path}`"))?;

    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, session_id, &primary_path, false)
        .map_err(|e| format!("cannot resolve `{primary_path}`: {e}"))?;
    if resolved.rel_path != primary_path || !resolved.metadata.is_file() {
        return Err(format!("ingested path `{primary_path}` is not a regular source file"));
    }
    if resolved.metadata.len() > MAX_HTML_BYTES as u64 {
        return Err(format!("`{primary_path}` exceeds {MAX_HTML_BYTES}-byte HTML cap"));
    }
    let mut raw_bytes = Vec::new();
    File::open(&resolved.canonical_target)
        .and_then(|f| f.take(MAX_HTML_BYTES as u64 + 1).read_to_end(&mut raw_bytes))
        .map_err(|e| format!("failed reading `{primary_path}`: {e}"))?;
    if raw_bytes.len() > MAX_HTML_BYTES {
        return Err(format!("`{primary_path}` exceeds {MAX_HTML_BYTES}-byte HTML cap"));
    }
    let actual_sha = ledger::sha256_hex(&raw_bytes);
    if &actual_sha != expected_sha {
        return Err(format!(
            "hash mismatch for `{primary_path}`: expected {expected_sha}, got {actual_sha}"
        ));
    }
    let html = String::from_utf8(raw_bytes)
        .map_err(|_| format!("`{primary_path}` is not valid UTF-8"))?;
    let files_root_sha256 = compute_files_root_sha256(&files_sha256);

    Ok(IngestedHtmlSource {
        source_id,
        source_seq: receipt.seq,
        primary_path,
        html,
        files_sha256,
        files_root_sha256,
    })
}

/// Builds an `html-1` [`PaperSkeleton`] deterministically from in-memory HTML (no model, no I/O).
pub fn build_html_skeleton(input: &HtmlSkeletonInput) -> HtmlSkeletonOutput {
    build_html_skeleton_timed(input).0
}

/// Builds an `html-1` [`PaperSkeleton`] and records timing metrics.
pub fn build_html_skeleton_timed(input: &HtmlSkeletonInput) -> (HtmlSkeletonOutput, BuildTiming) {
    let total_start = Instant::now();
    let files_sha256 = match &input.files_sha256 {
        Some(map) => map.clone(),
        None => BTreeMap::from([(
            input.primary_path.clone(),
            ledger::sha256_hex(input.html.as_bytes()),
        )]),
    };
    let files_root_sha256 = compute_files_root_sha256(&files_sha256);

    let scan_start = Instant::now();
    let lex = scan_html_tokens(&input.html);
    let mut walker = HtmlWalker::new(&input.primary_path, &lex.line_index, lex.warnings);
    walker.walk(&lex.tokens);
    walker.finish_all(lex.line_index.total_lines());
    let scan_us = scan_start.elapsed().as_micros() as u64;

    let mut quality_notes = walker.quality_notes;
    let mut quality = if walker.nodes.is_empty() {
        if quality_notes.is_empty() {
            quality_notes.push("no LaTeXML structural elements found in HTML".to_string());
        }
        SkeletonQuality::Weak
    } else if !quality_notes.is_empty() {
        SkeletonQuality::Partial
    } else {
        SkeletonQuality::Full
    };

    let mut nodes = walker.nodes;
    let mut parent_orders = walker.parent_orders;
    assign_tags(&input.paper_key, &mut nodes, &parent_orders);

    let (edges, bib, stats) = finalize_html_graph(
        &mut nodes,
        &parent_orders,
        &walker.html_id_to_node_idx,
        &walker.proof_hints,
        &walker.refs,
        &walker.cites,
        &walker.bib_items,
        &walker.prose_lines,
        &walker.node_plain_texts,
        &mut quality,
        &mut quality_notes,
    );

    let mut skeleton = PaperSkeleton {
        schema: SKELETON_SCHEMA_VERSION,
        builder: SkeletonBuilder::Html1,
        source_id: input.source_id.clone(),
        source_seq: input.source_seq,
        files_root_sha256,
        paper_key: input.paper_key.clone(),
        version: input.version.clone(),
        quality,
        quality_reason: join_quality_notes(&quality_notes),
        nodes,
        edges,
        bib,
        symbols: Vec::new(),
        macros: Vec::new(),
        stats,
        carry: None,
    };

    let mut node_plain_texts = walker.node_plain_texts;
    let mut json_bytes = serde_json::to_vec(&skeleton).map(|v| v.len()).unwrap_or(0);
    if json_bytes > MAX_SKELETON_JSON_BYTES {
        if skeleton.quality == SkeletonQuality::Full {
            skeleton.quality = SkeletonQuality::Partial;
        }
        quality_notes.push(format!(
            "JSON size ({json_bytes} bytes) exceeded {MAX_SKELETON_JSON_BYTES}-byte cap; truncated"
        ));
        while json_bytes > MAX_SKELETON_JSON_BYTES
            && (!skeleton.nodes.is_empty() || !skeleton.bib.is_empty())
        {
            if !skeleton.nodes.is_empty() {
                let cut = (skeleton.nodes.len() * 3) / 4;
                skeleton.nodes.truncate(cut);
                parent_orders.truncate(cut);
                node_plain_texts.truncate(cut);
            } else {
                let bcut = (skeleton.bib.len() * 3) / 4;
                skeleton.bib.truncate(bcut);
            }
            let live_tags: BTreeSet<String> =
                skeleton.nodes.iter().filter_map(|n| n.tag.clone()).collect();
            let live_bib: BTreeSet<String> = skeleton.bib.iter().map(|b| b.key.clone()).collect();
            for node in &mut skeleton.nodes {
                if node.proof_of.as_ref().is_some_and(|t| !live_tags.contains(t)) {
                    node.proof_of = None;
                }
            }
            skeleton.edges.retain(|e| {
                live_tags.contains(&e.to)
                    && if e.kind == SkEdgeKind::Cites {
                        live_bib.contains(&e.from)
                    } else {
                        live_tags.contains(&e.from)
                    }
            });
            skeleton.stats =
                compute_html_stats(&mut skeleton.nodes, &parent_orders, &node_plain_texts);
            skeleton.quality_reason = join_quality_notes(&quality_notes);
            json_bytes = serde_json::to_vec(&skeleton).map(|v| v.len()).unwrap_or(0);
        }
    }

    let timing = BuildTiming {
        flatten_us: 0,
        scan_us,
        total_us: total_start.elapsed().as_micros() as u64,
        flattened_lines: lex.line_index.total_lines(),
        node_count: skeleton.nodes.len(),
        json_bytes,
    };
    let output = HtmlSkeletonOutput {
        skeleton,
        parent_orders,
        quality_notes,
        document_title: walker.document_title,
    };
    (output, timing)
}

/// Loads a verified HTML file from a `source.ingested` ledger receipt and builds `html-1`.
pub fn build_html_skeleton_from_ingested(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    paper_key: PaperKey,
    version: Option<String>,
) -> Result<(HtmlSkeletonOutput, BuildTiming), String> {
    let ingested = load_ingested_html_source(sandbox, session_id, receipt)?;
    let input = HtmlSkeletonInput {
        source_id: ingested.source_id,
        source_seq: ingested.source_seq,
        paper_key,
        version,
        primary_path: ingested.primary_path,
        html: ingested.html,
        files_sha256: Some(ingested.files_sha256),
    };
    Ok(build_html_skeleton_timed(&input))
}

fn join_quality_notes(notes: &[String]) -> Option<String> {
    (!notes.is_empty()).then(|| notes.join("; "))
}
