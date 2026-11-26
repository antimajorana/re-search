//! Structural line scanner for the deterministic PDF skeleton (`pdf-1`, Investigate V17).
//!
//! Walks the pinned F14a reading file (without mutating it) using `pdf_sections` and
//! `pdf_patterns`, builds the section/theorem/proof/equation/caption/bibliography node tree,
//! splits spans around `PageBasis::None` (image-only) pages so no node ever claims an image-only
//! page, enforces strict parent-span containment and `MAX_SKELETON_NODES`, and computes content
//! fingerprints (`fp`) for V2 tag generation.

use super::model::{
    NumberBasis, SkConfidence, SkNode, SkNodeKind, Span, MAX_SKELETON_NODES, NODE_FP_HEX_CHARS,
};
use super::pdf_patterns::{
    compute_active_lines, detect_headers_footers, has_qed_marker, looks_math_line,
    match_caption_heading, match_margin_equation, match_proof_heading, match_theorem_heading,
};
use super::pdf_sections::{select_sections, SelectedSection};
use crate::pdf_firstpage::normalize_pdf_unicode;
use crate::pdf_pages::{PageBasis, PageMap};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Proof attachment hint collected during PDF structural scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PdfProofHint {
    HeaderMention { kind: SkNodeKind, number: String },
    Adjacent { target_order: usize },
    Unattached,
}

/// Output of the PDF structural pass consumed by bibliography, edge, tag, and stats passes.
#[derive(Debug, Clone)]
pub(crate) struct PdfScanOutput {
    pub nodes: Vec<SkNode>,
    pub parent_orders: Vec<Option<usize>>,
    pub proof_hints: BTreeMap<usize, PdfProofHint>,
    pub active_lines: Vec<bool>,
    pub bib_ranges: Vec<(usize, usize)>,
    pub node_heading_end_bytes: BTreeMap<usize, usize>,
    pub quality_notes: Vec<String>,
}

/// Computes `SkNode::fp` for PDF nodes: first 12 lower-case hex chars of SHA-256 over
/// whitespace-normalised text (preserving literal `%` signs; invariant to CRLF vs LF and `\x0c`).
pub(crate) fn compute_pdf_body_fp(raw_body: &str) -> String {
    let mut normalized = String::with_capacity(raw_body.len().min(4096));
    let mut first = true;
    for token in raw_body.split_whitespace() {
        if !first {
            normalized.push(' ');
        }
        normalized.push_str(token);
        first = false;
    }
    let digest = Sha256::digest(normalized.as_bytes());
    let mut hex = String::with_capacity(NODE_FP_HEX_CHARS);
    for byte in &digest[..NODE_FP_HEX_CHARS / 2] {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Scans the pinned reading file lines against `page_map` and builds structural PDF nodes.
pub(crate) fn scan_pdf_lines(
    primary_path: &str,
    lines: &[&str],
    page_map: &PageMap,
) -> PdfScanOutput {
    let n = lines.len();
    let header_footer_lines = detect_headers_footers(lines, page_map);
    let active_lines = compute_active_lines(lines, page_map, &header_footer_lines);
    let starts_after_page_one = page_map
        .pages
        .iter()
        .find(|p| p.basis != PageBasis::None)
        .is_some_and(|p| p.page > 1);

    let sections = select_sections(lines, &active_lines, starts_after_page_one);
    let sec_by_line: BTreeMap<usize, SelectedSection> =
        sections.into_iter().map(|s| (s.line_no, s)).collect();

    let mut nodes: Vec<SkNode> = Vec::new();
    let mut parent_orders: Vec<Option<usize>> = Vec::new();
    let mut proof_hints: BTreeMap<usize, PdfProofHint> = BTreeMap::new();
    let mut bib_ranges: Vec<(usize, usize)> = Vec::new();
    let mut node_heading_end_bytes: BTreeMap<usize, usize> = BTreeMap::new();
    let mut quality_notes: Vec<String> = Vec::new();
    let mut truncated_nodes = false;

    let first_heading_line = sec_by_line.keys().next().copied();
    let first_active = (1..=n).find(|&ln| active_lines[ln - 1] && !lines[ln - 1].trim().is_empty());
    let fm_limit = first_heading_line.map_or(n, |h| h.saturating_sub(1));
    let mut open_front_matter: Option<usize> = None;
    if let Some(start_ln) = first_active {
        let starts_with_explicit_abstract = first_heading_line.is_some_and(|h| {
            h == start_ln
                && sec_by_line
                    .get(&h)
                    .is_some_and(|s| s.kind == SkNodeKind::FrontMatter)
        });
        if !starts_with_explicit_abstract && start_ln <= fm_limit {
            if let Some(end_ln) = last_active_non_empty(lines, &active_lines, start_ln, fm_limit) {
                open_front_matter = push_node(
                    &mut nodes,
                    &mut parent_orders,
                    &mut truncated_nodes,
                    &mut quality_notes,
                    primary_path,
                    SkNodeKind::FrontMatter,
                    None,
                    Some("Front Matter".into()),
                    None,
                    start_ln,
                    end_ln,
                );
            }
        }
    }

    let mut sec_stack: Vec<(usize, u8, SkNodeKind)> = Vec::new();
    let mut open_block: Option<(usize, SkNodeKind, bool)> = None;
    let mut last_provable: Option<usize> = None;
    let mut in_bib: Option<usize> = None;
    let mut min_eq_start_line: usize = 1;

    let close_block = |block: &mut Option<(usize, SkNodeKind, bool)>,
                       end_before: usize,
                       nodes: &mut [SkNode]| {
        if let Some((idx, _, _)) = block.take() {
            let start = nodes[idx].spans[0].start_line;
            let end = last_active_non_empty(lines, &active_lines, start, end_before).unwrap_or(start);
            nodes[idx].spans[0].end_line = end.max(start);
        }
    };

    let close_sections_for = |stack: &mut Vec<(usize, u8, SkNodeKind)>,
                              min_level: u8,
                              end_before: usize,
                              nodes: &mut [SkNode]| {
        while let Some(&(idx, lvl, _)) = stack.last() {
            if lvl < min_level {
                break;
            }
            stack.pop();
            let start = nodes[idx].spans[0].start_line;
            let end = last_active_non_empty(lines, &active_lines, start, end_before).unwrap_or(start);
            nodes[idx].spans[0].end_line = end.max(start);
        }
    };

    for idx0 in 0..n {
        let line_no = idx0 + 1;
        if !active_lines[idx0] {
            continue;
        }
        let raw = lines[idx0];
        let trimmed = raw.trim();

        if trimmed.is_empty() {
            if let Some((_, kind, is_proof)) = open_block {
                if !is_proof
                    && kind.is_theorem_like()
                    && next_active_starts_plain_prose(lines, &active_lines, line_no)
                {
                    close_block(&mut open_block, line_no.saturating_sub(1), &mut nodes);
                    min_eq_start_line = line_no + 1;
                }
            }
            continue;
        }

        // 1. Section / FrontMatter / Bibliography heading.
        if let Some(sec) = sec_by_line.get(&line_no) {
            close_block(&mut open_block, line_no.saturating_sub(1), &mut nodes);
            min_eq_start_line = line_no + 1;
            if let Some(b_idx) = in_bib.take() {
                let start = nodes[b_idx].spans[0].start_line;
                let end = last_active_non_empty(lines, &active_lines, start, line_no.saturating_sub(1))
                    .unwrap_or(start);
                nodes[b_idx].spans[0].end_line = end.max(start);
                bib_ranges.push((start, nodes[b_idx].spans[0].end_line));
            }
            if sec.kind == SkNodeKind::FrontMatter {
                close_sections_for(&mut sec_stack, 1, line_no.saturating_sub(1), &mut nodes);
                if let Some(fm_idx) = open_front_matter {
                    nodes[fm_idx].spans[0].end_line = line_no;
                    continue;
                }
            } else {
                if let Some(fm_idx) = open_front_matter.take() {
                    let start = nodes[fm_idx].spans[0].start_line;
                    let end = last_active_non_empty(lines, &active_lines, start, line_no.saturating_sub(1))
                        .unwrap_or(start);
                    nodes[fm_idx].spans[0].end_line = end.max(start);
                }
                close_sections_for(&mut sec_stack, sec.level, line_no.saturating_sub(1), &mut nodes);
            }
            last_provable = None;

            let parent = if sec.kind == SkNodeKind::Section {
                sec_stack.last().map(|&(idx, _, _)| idx)
            } else {
                None
            };
            if let Some(new_idx) = push_node(
                &mut nodes,
                &mut parent_orders,
                &mut truncated_nodes,
                &mut quality_notes,
                primary_path,
                sec.kind,
                sec.number.clone(),
                (!sec.title.is_empty()).then(|| sec.title.clone()),
                parent,
                line_no,
                line_no,
            ) {
                node_heading_end_bytes.insert(new_idx, normalize_pdf_unicode(trimmed).len());
                if sec.kind == SkNodeKind::Bibliography {
                    in_bib = Some(new_idx);
                } else if sec.kind == SkNodeKind::FrontMatter {
                    open_front_matter = Some(new_idx);
                } else {
                    sec_stack.push((new_idx, sec.level, sec.kind));
                }
            }
            continue;
        }

        if let Some(b_idx) = in_bib {
            nodes[b_idx].spans[0].end_line = line_no;
            continue;
        }

        if let Some(fm_idx) = open_front_matter {
            nodes[fm_idx].spans[0].end_line = line_no;
        }

        let norm_line = normalize_pdf_unicode(trimmed);
        let next_active = next_active_non_empty(lines, &active_lines, line_no);
        let current_sec = sec_stack.last().map(|&(idx, _, _)| idx).or(open_front_matter);

        // 2. Proof heading (`Proof.`, `Proof of Theorem 2.1.`).
        if let Some(pm) = match_proof_heading(&norm_line, next_active) {
            close_block(&mut open_block, line_no.saturating_sub(1), &mut nodes);
            let hint = if let Some((kind, number)) = pm.target {
                PdfProofHint::HeaderMention { kind, number }
            } else if let Some(target_order) = last_provable.take() {
                PdfProofHint::Adjacent { target_order }
            } else {
                PdfProofHint::Unattached
            };
            if let Some(p_idx) = push_node(
                &mut nodes,
                &mut parent_orders,
                &mut truncated_nodes,
                &mut quality_notes,
                primary_path,
                SkNodeKind::Proof,
                None,
                pm.title,
                current_sec,
                line_no,
                line_no,
            ) {
                node_heading_end_bytes.insert(p_idx, pm.heading_end_byte);
                proof_hints.insert(p_idx, hint);
                if has_qed_marker(&norm_line) {
                    open_block = None;
                    min_eq_start_line = line_no + 1;
                } else {
                    open_block = Some((p_idx, SkNodeKind::Proof, true));
                    min_eq_start_line = line_no + 1;
                }
            }
            continue;
        }

        // 3. Theorem-like heading.
        if let Some(tm) = match_theorem_heading(&norm_line, next_active) {
            close_block(&mut open_block, line_no.saturating_sub(1), &mut nodes);
            if let Some(t_idx) = push_node(
                &mut nodes,
                &mut parent_orders,
                &mut truncated_nodes,
                &mut quality_notes,
                primary_path,
                tm.kind,
                tm.number,
                tm.title,
                current_sec,
                line_no,
                line_no,
            ) {
                node_heading_end_bytes.insert(t_idx, tm.heading_end_byte);
                last_provable = tm.kind.is_provable().then_some(t_idx);
                open_block = Some((t_idx, tm.kind, false));
                min_eq_start_line = line_no + 1;
            }
            continue;
        }

        // 4. Figure / Table caption.
        if let Some(cap) = match_caption_heading(&norm_line) {
            close_block(&mut open_block, line_no.saturating_sub(1), &mut nodes);
            let _ = push_node(
                &mut nodes,
                &mut parent_orders,
                &mut truncated_nodes,
                &mut quality_notes,
                primary_path,
                cap.kind,
                Some(cap.number),
                cap.title,
                current_sec,
                line_no,
                line_no,
            );
            min_eq_start_line = line_no + 1;
            continue;
        }

        // 5. Margin equation number.
        let eq_floor = open_block
            .map(|(i, _, _)| nodes[i].spans[0].start_line.max(min_eq_start_line))
            .unwrap_or(min_eq_start_line);
        if let Some((eq_num, eq_start_line)) =
            match_margin_equation(lines, &active_lines, page_map, idx0, eq_floor)
        {
            let eq_parent = open_block.map(|(i, _, _)| i).or(current_sec);
            let _ = push_node(
                &mut nodes,
                &mut parent_orders,
                &mut truncated_nodes,
                &mut quality_notes,
                primary_path,
                SkNodeKind::Equation,
                Some(eq_num),
                None,
                eq_parent,
                eq_start_line,
                line_no,
            );
            min_eq_start_line = line_no + 1;
            if let Some((b_idx, _, is_proof)) = open_block {
                nodes[b_idx].spans[0].end_line = line_no;
                if is_proof && has_qed_marker(&norm_line) {
                    open_block = None;
                }
            }
            continue;
        }

        // 6. Regular line inside an open block.
        if let Some((b_idx, _, is_proof)) = open_block {
            nodes[b_idx].spans[0].end_line = line_no;
            if is_proof && has_qed_marker(&norm_line) {
                open_block = None;
                min_eq_start_line = line_no + 1;
            }
        }
    }

    // Close all open containers at the last active line.
    close_block(&mut open_block, n, &mut nodes);
    if let Some(b_idx) = in_bib.take() {
        let start = nodes[b_idx].spans[0].start_line;
        let end = last_active_non_empty(lines, &active_lines, start, n).unwrap_or(start);
        nodes[b_idx].spans[0].end_line = end.max(start);
        bib_ranges.push((start, nodes[b_idx].spans[0].end_line));
    }
    if let Some(fm_idx) = open_front_matter.take() {
        let start = nodes[fm_idx].spans[0].start_line;
        let end = last_active_non_empty(lines, &active_lines, start, n).unwrap_or(start);
        nodes[fm_idx].spans[0].end_line = end.max(start);
    }
    close_sections_for(&mut sec_stack, 1, n, &mut nodes);

    // Guarantee parent span containment (`parent.span ⊇ child.span`) bottom-up before splitting.
    for i in (0..nodes.len()).rev() {
        if let Some(p_idx) = parent_orders[i] {
            let (cs, ce) = (nodes[i].spans[0].start_line, nodes[i].spans[0].end_line);
            let p_span = &mut nodes[p_idx].spans[0];
            if cs < p_span.start_line {
                p_span.start_line = cs;
            }
            if ce > p_span.end_line {
                p_span.end_line = ce;
            }
        }
    }

    // Split spans around `PageBasis::None` (image-only) pages so no node ever includes lines from
    // an image-only page; splitting preserves parent-span containment identically for all nodes.
    let none_line_mask = build_none_page_line_mask(n, page_map);
    for node in &mut nodes {
        let sp = &node.spans[0];
        let split = split_excluding_none_pages(primary_path, sp.start_line, sp.end_line, &none_line_mask);
        if !split.is_empty() {
            node.spans = split;
        }
        let body = slice_spans_text(lines, &node.spans);
        node.fp = compute_pdf_body_fp(&body);
    }

    PdfScanOutput {
        nodes,
        parent_orders,
        proof_hints,
        active_lines,
        bib_ranges,
        node_heading_end_bytes,
        quality_notes,
    }
}

fn build_none_page_line_mask(line_count: usize, page_map: &PageMap) -> Vec<bool> {
    let mut mask = vec![false; line_count];
    for rec in &page_map.pages {
        if rec.basis != PageBasis::None {
            continue;
        }
        if let (Some(first), Some(last)) = (rec.first_line, rec.last_line) {
            if first >= 1 && first <= line_count {
                let hi = last.min(line_count);
                for slot in &mut mask[(first - 1)..hi] {
                    *slot = true;
                }
            }
        }
    }
    mask
}

fn split_excluding_none_pages(
    primary_path: &str,
    start_line: usize,
    end_line: usize,
    none_mask: &[bool],
) -> Vec<Span> {
    if start_line == 0 || end_line < start_line || none_mask.is_empty() {
        return Vec::new();
    }
    let lo = start_line.min(none_mask.len());
    let hi = end_line.min(none_mask.len());
    let mut spans = Vec::new();
    let mut run_start: Option<usize> = None;
    for ln in lo..=hi {
        if none_mask[ln - 1] {
            if let Some(s) = run_start.take() {
                spans.push(Span::new(primary_path, s, ln - 1));
            }
        } else if run_start.is_none() {
            run_start = Some(ln);
        }
    }
    if let Some(s) = run_start {
        spans.push(Span::new(primary_path, s, hi));
    }
    spans
}

#[allow(clippy::too_many_arguments)]
fn push_node(
    nodes: &mut Vec<SkNode>,
    parent_orders: &mut Vec<Option<usize>>,
    truncated: &mut bool,
    quality_notes: &mut Vec<String>,
    primary_path: &str,
    kind: SkNodeKind,
    number: Option<String>,
    title: Option<String>,
    parent: Option<usize>,
    start_line: usize,
    end_line: usize,
) -> Option<usize> {
    if nodes.len() >= MAX_SKELETON_NODES {
        if !*truncated {
            *truncated = true;
            quality_notes.push(format!(
                "node count exceeded MAX_SKELETON_NODES ({MAX_SKELETON_NODES}); truncated"
            ));
        }
        return None;
    }
    let order = nodes.len();
    let number_basis = number.as_ref().map(|_| NumberBasis::Printed);
    nodes.push(SkNode {
        tag: None,
        kind,
        number,
        number_basis,
        env_name: None,
        title,
        label: None,
        parent: None,
        order,
        spans: vec![Span::new(primary_path, start_line, end_line.max(start_line))],
        proof_of: None,
        head_cites: Vec::new(),
        intro_refs: false,
        fp: String::new(),
        words: 0,
        eq_count: 0,
        read_minutes: 0.0,
        confidence: SkConfidence::PdfHeuristic,
    });
    parent_orders.push(parent);
    Some(order)
}

fn slice_spans_text(lines: &[&str], spans: &[Span]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for sp in spans {
        if sp.start_line == 0 || sp.end_line < sp.start_line {
            continue;
        }
        let s = sp.start_line.saturating_sub(1).min(lines.len().saturating_sub(1));
        let e = sp.end_line.saturating_sub(1).min(lines.len().saturating_sub(1));
        parts.push(lines[s..=e].join("\n"));
    }
    parts.join("\n")
}

fn last_active_non_empty(lines: &[&str], active: &[bool], start: usize, end: usize) -> Option<usize> {
    if start == 0 || end < start || lines.is_empty() {
        return None;
    }
    let lo = start.saturating_sub(1).min(lines.len().saturating_sub(1));
    let hi = end.saturating_sub(1).min(lines.len().saturating_sub(1));
    for i in (lo..=hi).rev() {
        if active[i] && !lines[i].trim().is_empty() {
            return Some(i + 1);
        }
    }
    None
}

fn next_active_non_empty<'a>(lines: &[&'a str], active: &[bool], after_line: usize) -> Option<&'a str> {
    for i in after_line..lines.len() {
        if active[i] && !lines[i].trim().is_empty() {
            return Some(lines[i]);
        }
    }
    None
}

fn next_active_starts_plain_prose(lines: &[&str], active: &[bool], after_line: usize) -> bool {
    let Some(raw) = next_active_non_empty(lines, active, after_line) else {
        return false;
    };
    let t = raw.trim();
    let first = t.chars().next().unwrap_or(' ');
    if first.is_lowercase() || matches!(first, '(' | '[' | '$' | '•' | '-') || looks_math_line(t) {
        return false;
    }
    first.is_uppercase()
}
