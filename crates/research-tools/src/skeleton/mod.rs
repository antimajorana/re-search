//! Deterministic TeX skeleton core (Investigate packages V1 and V2).
//!
//! Builds a complete [`PaperSkeleton`] (nodes, permanent tags, simulated counters,
//! positional dependency edges, reading stats, and bibliography entries) from already
//! ingested TeX files (or an in-memory map) with zero model calls and zero file access
//! outside the ingested source.

pub mod bib;
pub mod edges;
pub mod flatten;
pub mod html;
mod html_bib;
mod html_classify;
mod html_edges;
mod html_entities;
mod html_finalize;
mod html_lex;
mod html_walk;
pub mod locate;
mod locate_parse;
pub mod model;
pub mod number;
pub mod pdf;
mod pdf_bib;
mod pdf_edges;
mod pdf_honesty;
mod pdf_patterns;
mod pdf_scan;
mod pdf_sections;
pub mod skip;
pub mod stats;
pub mod symbols;
pub mod tags;
pub mod tex_env;
mod tex_env_cmds;
mod tex_lex;
mod tex_math;
pub mod tex_theorems;

pub use bib::{extract_bibliography, CiteOccurrence};
pub use edges::build_edges;
pub use flatten::{
    flatten_tex_files, load_ingested_tex_source, select_primary_tex, FlattenedLine, FlattenedTex,
    IngestedTexSource, OriginEntry, MAX_FLATTENED_BYTES, MAX_FLATTENED_LINES, MAX_INCLUDE_DEPTH,
};
pub use html::{
    build_html_skeleton, build_html_skeleton_from_ingested, build_html_skeleton_timed,
    is_html_receipt, is_latexml_html_document, load_ingested_html_source, HtmlSkeletonInput,
    HtmlSkeletonOutput, IngestedHtmlSource,
};
pub use html_lex::MAX_HTML_BYTES;
pub use locate::{
    detect_tex_counter_drift, format_candidate_label, format_version_caveat, locate_in_skeleton,
    locate_in_skeleton_with_context, parse_locator, LocateCandidate, LocateContext, LocateResult,
    LocateStatus, ParsedLocator, CITED_STATUS_CANDIDATES, CITED_STATUS_FOUND,
    CITED_STATUS_LOCATOR_NOT_FOUND, CITED_STATUS_NO_TEX, CITED_STATUS_UNRESOLVED,
    MAX_LOCATE_CANDIDATES, NO_TEX_REASON_EMPTY_TEXT_LAYER, NO_TEX_REASON_NEEDS_FILE,
    PROVENANCE_OUTSIDE_PAPER,
};
pub use model::{
    compute_body_fp, compute_files_root_sha256, BibLocator, BibRef, Carry, CarryBasis, CarryMatch,
    EdgeConfidence, MacroDef, NumberBasis, PaperSkeleton, SectionStats, SkConfidence, SkEdge,
    SkEdgeKind, SkEdgeRole, SkEdgeRule, SkNode, SkNodeKind, SkeletonBuilder, SkeletonQuality, Span,
    SymbolDef, SymbolDefHow, MAX_SKELETON_EDGES, MAX_SKELETON_JSON_BYTES, MAX_SKELETON_NODES,
    NODE_FP_HEX_CHARS, SKELETON_SCHEMA_VERSION,
};
pub use number::assign_numbers;
pub use pdf::{
    build_pdf_skeleton, build_pdf_skeleton_from_ingested, build_pdf_skeleton_timed,
    load_ingested_pdf_source, match_pdf_quote_for_display, node_page_basis, page_range_for_node,
    page_range_for_span, render_formulas_as_math, IngestedPdfSource, PdfPageAvailability,
    PdfQuoteDisplay, PdfSkeletonInput, PdfSkeletonOutput, PdfTextOrigin,
    PDF_MODEL_TRANSCRIPTION_LABEL, PDF_OCR_QUOTE_LABEL, PDF_QUALITY_REASON_BASE,
    PDF_QUALITY_SCAN_TAG,
};
pub use skip::{
    compute_skip_plan, ExtraUseEdge, SectionNeed, SkipBasis, SkipPlan, SkipSectionEntry,
    SKIP_BASIS_EXPLICIT, SKIP_BASIS_WITH_MODEL, SKIP_LABEL_PROOF, SKIP_LABEL_STATEMENT,
    SKIP_LABEL_UNREFERENCED,
};
pub use stats::{compute_stats, count_tex_words, estimate_read_minutes, EQ_READ_MINUTES, MATH_READ_WORDS_PER_MIN};
pub use symbols::{
    extract_lhs_symbol, extract_symbols, lookup_nearest_symbol, normalize_symbol,
    symbol_lookup_keys, SymbolLookupResult, MAX_SKELETON_SYMBOLS, SYMBOL_FOUND_LABEL,
    SYMBOL_NOT_FOUND_LABEL,
};
pub use tags::{assign_tags, kind_class, KIND_CLASS_TABLE};
pub use tex_env::{scan_tex_stream, ProofHint, TexScanOutput};
pub use tex_theorems::{map_theorem_kind, TheoremDef, TheoremLookup, TheoremTable};

use crate::paper_key::PaperKey;
use crate::sandbox::WorkspaceSandbox;
use ledger::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Input to the pure in-memory TeX skeleton builder.
#[derive(Debug, Clone)]
pub struct TexSkeletonInput {
    pub source_id: String,
    pub source_seq: u64,
    pub paper_key: PaperKey,
    pub version: Option<String>,
    pub files: BTreeMap<String, String>,
    pub primary_path: Option<String>,
    /// Optional precomputed `files_sha256` from a `source.ingested` receipt.
    /// Computed from `files` via SHA-256 when `None`.
    pub files_sha256: Option<BTreeMap<String, String>>,
}

/// Complete build output: the section 4.1 [`PaperSkeleton`] plus structural metadata
/// consumed by downstream passes and tests.
#[derive(Debug, Clone)]
pub struct TexSkeletonOutput {
    pub skeleton: PaperSkeleton,
    /// For each node `i`, the 0-based `order` of its parent node, if any.
    pub parent_orders: Vec<Option<usize>>,
    /// For each proof node `i`, the V1 proof-target classification.
    pub proof_hints: BTreeMap<usize, ProofHint>,
    /// Theorem-like environment declarations discovered in the source.
    pub theorem_table: TheoremTable,
    /// In-memory flattened stream with physical line-origin map.
    pub flattened: FlattenedTex,
    /// All recorded non-fatal quality notes (also joined in `skeleton.quality_reason`).
    pub quality_notes: Vec<String>,
}

/// Wall-clock timing breakdown returned by [`build_tex_skeleton_timed`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildTiming {
    pub flatten_us: u64,
    pub scan_us: u64,
    pub total_us: u64,
    pub flattened_lines: usize,
    pub node_count: usize,
    pub json_bytes: usize,
}

impl BuildTiming {
    pub fn total_duration(&self) -> Duration {
        Duration::from_micros(self.total_us)
    }
}

/// Builds a TeX [`PaperSkeleton`] deterministically from in-memory files (no disk I/O, no model).
pub fn build_tex_skeleton(input: &TexSkeletonInput) -> TexSkeletonOutput {
    build_tex_skeleton_timed(input).0
}

/// Builds a TeX [`PaperSkeleton`] and records wall-clock timing for flattening and scanning.
pub fn build_tex_skeleton_timed(input: &TexSkeletonInput) -> (TexSkeletonOutput, BuildTiming) {
    let total_start = Instant::now();

    let files_sha256 = match &input.files_sha256 {
        Some(map) => map.clone(),
        None => input
            .files
            .iter()
            .map(|(p, content)| (p.clone(), ledger::sha256_hex(content.as_bytes())))
            .collect(),
    };
    let files_root_sha256 = compute_files_root_sha256(&files_sha256);

    let flatten_start = Instant::now();
    let flattened = flatten_tex_files(&input.files, input.primary_path.as_deref()).unwrap_or_else(|| {
        FlattenedTex {
            primary_path: input.primary_path.clone().unwrap_or_default(),
            lines: Vec::new(),
            inlined_files: Vec::new(),
            warnings: vec!["no TeX files available to flatten".to_string()],
        }
    });
    let flatten_us = flatten_start.elapsed().as_micros() as u64;

    let scan_start = Instant::now();
    let scan = scan_tex_stream(&flattened);
    let scan_us = scan_start.elapsed().as_micros() as u64;

    let mut quality = scan.quality;
    let mut quality_notes = scan.quality_notes;
    let mut nodes = scan.nodes;
    let mut parent_orders = scan.parent_orders;
    let mut proof_hints = scan.proof_hints;
    let theorem_table = scan.theorem_table;

    // V2 passes: permanent tags, simulated counters, bibliography, edges, and reading stats.
    assign_tags(&input.paper_key, &mut nodes, &parent_orders);
    assign_numbers(&mut nodes, &parent_orders, &theorem_table, &flattened);
    let (bib, citations) = extract_bibliography(&flattened, &input.files);
    let edges = build_edges(
        &mut nodes,
        &parent_orders,
        &proof_hints,
        &flattened,
        &citations,
        &mut quality,
        &mut quality_notes,
    );
    let stats = compute_stats(&mut nodes, &parent_orders, &flattened);
    let symbols = extract_symbols(&nodes, &parent_orders, &scan.macros, &flattened);

    let mut skeleton = PaperSkeleton {
        schema: SKELETON_SCHEMA_VERSION,
        builder: SkeletonBuilder::Tex1,
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
        symbols,
        macros: scan.macros,
        stats,
        carry: None,
    };

    // Enforce the 2 MiB JSON size limit of investigate.md section 4.1.
    let mut json_bytes = serde_json::to_vec(&skeleton).map(|v| v.len()).unwrap_or(0);
    if json_bytes > MAX_SKELETON_JSON_BYTES {
        quality = SkeletonQuality::Partial;
        quality_notes.push(format!(
            "JSON size ({json_bytes} bytes) exceeded {MAX_SKELETON_JSON_BYTES}-byte cap; truncated nodes"
        ));
        while json_bytes > MAX_SKELETON_JSON_BYTES && !skeleton.nodes.is_empty() {
            let cut = (skeleton.nodes.len() * 3) / 4;
            skeleton.nodes.truncate(cut);
            parent_orders.truncate(cut);
            proof_hints.retain(|&ord, _| ord < cut);
            let live_tags: BTreeSet<String> = skeleton
                .nodes
                .iter()
                .filter_map(|n| n.tag.clone())
                .collect();
            for node in &mut skeleton.nodes {
                if node.proof_of.as_ref().is_some_and(|t| !live_tags.contains(t)) {
                    node.proof_of = None;
                }
            }
            skeleton.edges.retain(|e| {
                live_tags.contains(&e.to)
                    && (e.kind == SkEdgeKind::Cites || live_tags.contains(&e.from))
            });
            for sym in &mut skeleton.symbols {
                if sym.in_node.as_ref().is_some_and(|t| !live_tags.contains(t)) {
                    sym.in_node = None;
                }
            }
            skeleton.stats = compute_stats(&mut skeleton.nodes, &parent_orders, &flattened);
            skeleton.quality = quality;
            skeleton.quality_reason = join_quality_notes(&quality_notes);
            json_bytes = serde_json::to_vec(&skeleton).map(|v| v.len()).unwrap_or(0);
        }
    }

    let total_us = total_start.elapsed().as_micros() as u64;
    let timing = BuildTiming {
        flatten_us,
        scan_us,
        total_us,
        flattened_lines: flattened.lines.len(),
        node_count: skeleton.nodes.len(),
        json_bytes,
    };

    let output = TexSkeletonOutput {
        skeleton,
        parent_orders,
        proof_hints,
        theorem_table,
        flattened,
        quality_notes,
    };
    (output, timing)
}

/// Loads verified physical files from a `source.ingested` ledger receipt under `sandbox` and
/// builds the TeX skeleton without accessing any path outside that receipt.
pub fn build_tex_skeleton_from_ingested(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    paper_key: PaperKey,
    version: Option<String>,
) -> Result<(TexSkeletonOutput, BuildTiming), String> {
    let ingested = load_ingested_tex_source(sandbox, session_id, receipt)?;
    let input = TexSkeletonInput {
        source_id: ingested.source_id,
        source_seq: ingested.source_seq,
        paper_key,
        version,
        files: ingested.files,
        primary_path: ingested.primary_path,
        files_sha256: Some(ingested.files_sha256),
    };
    Ok(build_tex_skeleton_timed(&input))
}

fn join_quality_notes(notes: &[String]) -> Option<String> {
    if notes.is_empty() {
        None
    } else {
        Some(notes.join("; "))
    }
}
