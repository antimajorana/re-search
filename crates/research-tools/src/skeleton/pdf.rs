//! Deterministic PDF skeleton builder (`pdf-1`, Investigate package V17).
//!
//! Builds a section 4.1 [`PaperSkeleton`] from the pinned F14a reading file (`\x0c`-separated
//! pages) and its physical [`PageMap`] with zero model calls and zero network requests:
//! - Every node has [`SkConfidence::PdfHeuristic`], `label: None` (content-fingerprint V2 tags),
//!   and `number_basis: Some(NumberBasis::Printed)` whenever numbered.
//! - Every edge has [`EdgeConfidence::Heuristic`] in prerequisite-to-dependent order (`from` =
//!   prerequisite, `to` = dependent), keeping heuristic PDF links out of the confirmed skip plan.
//! - Quality is always [`SkeletonQuality::Partial`] or [`SkeletonQuality::Weak`], never `Full`,
//!   with a quality reason stating scan status (`"(scan)"`) when applicable (rule 6.21).

use super::model::{
    compute_files_root_sha256, PaperSkeleton, SkEdgeKind, SkNode, SkNodeKind, SkeletonBuilder,
    SkeletonQuality, MAX_SKELETON_JSON_BYTES, SKELETON_SCHEMA_VERSION,
};
use super::pdf_bib::extract_pdf_bibliography;
use super::pdf_edges::{build_pdf_edges, compute_pdf_stats};
use super::pdf_honesty::split_physical_lines;
use super::pdf_scan::scan_pdf_lines;
use super::tags::assign_tags;
use super::BuildTiming;
use crate::paper_key::PaperKey;
use crate::pdf_pages::{PageBasis, PageMap};
use crate::sandbox::WorkspaceSandbox;
use ledger::Event;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub use super::pdf_honesty::{
    load_ingested_pdf_source, match_pdf_quote_for_display, node_page_basis, page_range_for_node,
    page_range_for_span, render_formulas_as_math, IngestedPdfSource, PdfPageAvailability,
    PdfQuoteDisplay, PdfTextOrigin, PDF_MODEL_TRANSCRIPTION_LABEL, PDF_OCR_QUOTE_LABEL,
    PDF_QUALITY_REASON_BASE, PDF_QUALITY_SCAN_TAG,
};

/// Pure in-memory input to the deterministic `pdf-1` skeleton builder.
#[derive(Debug, Clone)]
pub struct PdfSkeletonInput {
    pub source_id: String,
    pub source_seq: u64,
    pub paper_key: PaperKey,
    pub version: Option<String>,
    /// Physical path of the pinned reading file inside `sources/`.
    pub primary_path: String,
    /// Exact pinned reading-file text (`\x0c`-separated pages from F14a). Never mutated.
    pub text: String,
    /// Physical page map from F14a (`pdf_pages` / `page_count`).
    pub page_map: PageMap,
    /// Optional pinned extractor id from `source.ingested`.
    pub extractor_id: Option<String>,
    /// Optional precomputed `files_sha256` map from `source.ingested`.
    pub files_sha256: Option<BTreeMap<String, String>>,
    /// Optional extra quality notes from receipt loading (e.g. legacy pre-F14a receipt).
    pub extra_quality_notes: Vec<String>,
}

/// Complete output of the `pdf-1` skeleton builder.
#[derive(Debug, Clone)]
pub struct PdfSkeletonOutput {
    pub skeleton: PaperSkeleton,
    /// For each node `i`, the 0-based `order` of its parent node, if any.
    pub parent_orders: Vec<Option<usize>>,
    /// Physical page map preserved from F14a for on-demand line-to-page lookup.
    pub page_map: PageMap,
    /// Per-page text/image/OCR availability summary.
    pub pages: Vec<PdfPageAvailability>,
    /// 1-based physical pages with `PageBasis::None` (image-only; available as images).
    pub image_only_pages: Vec<usize>,
    /// 1-based physical pages with `PageBasis::ScanTextLayer` (OCR basis; formulas stay text).
    pub scan_text_pages: Vec<usize>,
    /// All recorded non-fatal quality notes.
    pub quality_notes: Vec<String>,
}

/// Builds a `pdf-1` [`PaperSkeleton`] deterministically in memory (no disk I/O, no model).
pub fn build_pdf_skeleton(input: &PdfSkeletonInput) -> PdfSkeletonOutput {
    build_pdf_skeleton_timed(input).0
}

/// Builds a `pdf-1` [`PaperSkeleton`] and records wall-clock timing.
pub fn build_pdf_skeleton_timed(input: &PdfSkeletonInput) -> (PdfSkeletonOutput, BuildTiming) {
    let total_start = Instant::now();

    let files_sha256 = match &input.files_sha256 {
        Some(map) => map.clone(),
        None => BTreeMap::from([(
            input.primary_path.clone(),
            ledger::sha256_hex(input.text.as_bytes()),
        )]),
    };
    let files_root_sha256 = compute_files_root_sha256(&files_sha256);

    let lines: Vec<&str> = split_physical_lines(&input.text);
    let scan_start = Instant::now();
    let scan = scan_pdf_lines(&input.primary_path, &lines, &input.page_map);
    let scan_us = scan_start.elapsed().as_micros() as u64;

    let mut nodes = scan.nodes;
    let mut parent_orders = scan.parent_orders;
    let mut quality_notes = input.extra_quality_notes.clone();
    quality_notes.extend(scan.quality_notes);

    // Assign permanent content-fingerprint V2 tags (`fp:<12-hex>`) and wire `parent` tags.
    assign_tags(&input.paper_key, &mut nodes, &parent_orders);

    // Extract numbered bibliography entries (with DOI / arXiv IDs) and numbered citations.
    let bib_res = extract_pdf_bibliography(
        &input.primary_path,
        &lines,
        &scan.active_lines,
        &scan.bib_ranges,
        &mut nodes,
        &scan.node_heading_end_bytes,
    );

    // Build heuristic proof links, text_mention + lifted uses edges, and citation edges.
    let edges = build_pdf_edges(
        &input.primary_path,
        &lines,
        &scan.active_lines,
        &scan.bib_ranges,
        &mut nodes,
        &parent_orders,
        &scan.proof_hints,
        &scan.node_heading_end_bytes,
        &bib_res.citations,
        &mut quality_notes,
    );

    let stats = compute_pdf_stats(&mut nodes, &parent_orders, &lines, &scan.active_lines);

    let (pages, image_only_pages, scan_text_pages) = summarize_pages(&input.page_map);
    let quality = classify_pdf_quality(&nodes, &input.page_map, &image_only_pages);
    let quality_reason = build_quality_reason(
        &input.page_map,
        &image_only_pages,
        &scan_text_pages,
        &quality_notes,
    );

    let mut skeleton = PaperSkeleton {
        schema: SKELETON_SCHEMA_VERSION,
        builder: SkeletonBuilder::Pdf1,
        source_id: input.source_id.clone(),
        source_seq: input.source_seq,
        files_root_sha256,
        paper_key: input.paper_key.clone(),
        version: input.version.clone(),
        quality,
        quality_reason: Some(quality_reason),
        nodes,
        edges,
        bib: bib_res.bib,
        symbols: Vec::new(),
        macros: Vec::new(),
        stats,
        carry: None,
    };

    // Enforce the 2 MiB JSON size limit of investigate.md section 4.1 (never promotes to Full).
    let mut json_bytes = serde_json::to_vec(&skeleton).map(|v| v.len()).unwrap_or(0);
    if json_bytes > MAX_SKELETON_JSON_BYTES {
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
            } else {
                let bcut = (skeleton.bib.len() * 3) / 4;
                skeleton.bib.truncate(bcut);
            }
            let live_tags: BTreeSet<String> = skeleton
                .nodes
                .iter()
                .filter_map(|n| n.tag.clone())
                .collect();
            let live_bib: BTreeSet<String> =
                skeleton.bib.iter().map(|b| b.key.clone()).collect();
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
                compute_pdf_stats(&mut skeleton.nodes, &parent_orders, &lines, &scan.active_lines);
            skeleton.quality =
                classify_pdf_quality(&skeleton.nodes, &input.page_map, &image_only_pages);
            skeleton.quality_reason = Some(build_quality_reason(
                &input.page_map,
                &image_only_pages,
                &scan_text_pages,
                &quality_notes,
            ));
            json_bytes = serde_json::to_vec(&skeleton).map(|v| v.len()).unwrap_or(0);
        }
    }

    let total_us = total_start.elapsed().as_micros() as u64;
    let timing = BuildTiming {
        flatten_us: 0,
        scan_us,
        total_us,
        flattened_lines: lines.len(),
        node_count: skeleton.nodes.len(),
        json_bytes,
    };

    let output = PdfSkeletonOutput {
        skeleton,
        parent_orders,
        page_map: input.page_map.clone(),
        pages,
        image_only_pages,
        scan_text_pages,
        quality_notes,
    };
    (output, timing)
}

/// Loads a verified PDF reading file from a `source.ingested` ledger receipt under `sandbox` and
/// builds its `pdf-1` skeleton without touching any path outside that receipt.
pub fn build_pdf_skeleton_from_ingested(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    paper_key: PaperKey,
    version: Option<String>,
) -> Result<(PdfSkeletonOutput, BuildTiming), String> {
    let ingested = load_ingested_pdf_source(sandbox, session_id, receipt)?;
    let input = PdfSkeletonInput {
        source_id: ingested.source_id,
        source_seq: ingested.source_seq,
        paper_key,
        version,
        primary_path: ingested.primary_path,
        text: ingested.text,
        page_map: ingested.page_map,
        extractor_id: ingested.extractor_id,
        files_sha256: Some(ingested.files_sha256),
        extra_quality_notes: ingested.extra_quality_notes,
    };
    Ok(build_pdf_skeleton_timed(&input))
}

fn summarize_pages(page_map: &PageMap) -> (Vec<PdfPageAvailability>, Vec<usize>, Vec<usize>) {
    let mut pages = Vec::with_capacity(page_map.pages.len());
    let mut image_only = Vec::new();
    let mut scan_text = Vec::new();
    for rec in &page_map.pages {
        if rec.basis == PageBasis::None {
            image_only.push(rec.page);
        } else if matches!(rec.basis, PageBasis::ScanTextLayer | PageBasis::Ocr) {
            scan_text.push(rec.page);
        }
        pages.push(PdfPageAvailability {
            page: rec.page,
            basis: rec.basis,
            has_text: rec.basis != PageBasis::None && rec.first_line.is_some(),
            image_available: true,
            render_math: render_formulas_as_math(rec.basis),
        });
    }
    (pages, image_only, scan_text)
}

fn classify_pdf_quality(
    nodes: &[SkNode],
    page_map: &PageMap,
    image_only_pages: &[usize],
) -> SkeletonQuality {
    let extracted = page_map.pages.len();
    if nodes.is_empty() || (extracted > 0 && image_only_pages.len() == extracted) {
        return SkeletonQuality::Weak;
    }
    let sections = nodes.iter().filter(|n| n.kind == SkNodeKind::Section).count();
    let theorems = nodes.iter().filter(|n| n.kind.is_theorem_like()).count();
    let equations = nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count();

    // Never returns `SkeletonQuality::Full` (rule 6.21).
    if sections >= 2 || (sections >= 1 && (theorems >= 1 || equations >= 1)) || theorems >= 2 {
        SkeletonQuality::Partial
    } else {
        SkeletonQuality::Weak
    }
}

fn build_quality_reason(
    page_map: &PageMap,
    image_only_pages: &[usize],
    scan_text_pages: &[usize],
    notes: &[String],
) -> String {
    let mut parts = vec![PDF_QUALITY_REASON_BASE.to_string()];
    let total_extracted = page_map.pages.len();
    let none_count = image_only_pages.len();
    let scan_count = scan_text_pages.len();

    if total_extracted > 0 && none_count == total_extracted {
        parts.push(format!(
            "{PDF_QUALITY_SCAN_TAG} no usable text layer ({none_count} image-only page(s) available as images, OCR needed)"
        ));
    } else if scan_count > 0 && none_count > 0 {
        parts.push(format!(
            "{PDF_QUALITY_SCAN_TAG} {scan_count} scanned OCR page(s) and {none_count} image-only page(s) available as images"
        ));
    } else if scan_count > 0 {
        parts.push(format!(
            "{PDF_QUALITY_SCAN_TAG} {scan_count} of {total_extracted} extracted page(s) scanned with embedded OCR text"
        ));
    } else if none_count > 0 {
        parts.push(format!(
            "{PDF_QUALITY_SCAN_TAG} {none_count} of {total_extracted} extracted page(s) image-only (available as images, OCR needed)"
        ));
    }

    if let (Some(first), Some(last)) = (page_map.pages.first(), page_map.pages.last()) {
        if first.page > 1 || (page_map.page_count > 0 && total_extracted < page_map.page_count) {
            parts.push(format!(
                "extracted page range {}..={} of {}",
                first.page, last.page, page_map.page_count
            ));
        }
    }

    for note in notes {
        if !note.trim().is_empty() {
            parts.push(note.clone());
        }
    }
    parts.join("; ")
}

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod tests;
