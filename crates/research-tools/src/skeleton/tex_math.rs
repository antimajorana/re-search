//! Math-environment and display-math parsing helpers for `tex_env.rs`.
//!
//! Splits multi-row math environments (`align`, `gather`, `eqnarray`, `alignat`, `flalign`,
//! `IEEEeqnarray`) at top-level `\\` while respecting nested math environments (`cases`,
//! `matrix`, `split`, ...) and balanced braces, finds `\label` and `\tag`, and locates
//! closing `\]` and `$$` delimiters safely.

use super::tex_lex::{
    next_char_boundary, read_balanced_group, skip_ws_and_comments, GroupOutcome,
};
use crate::tex_scan::{extract_tex_labels, strip_latex_comments};

pub(crate) const SINGLE_MATH_ENVS: &[&str] =
    &["equation", "equation*", "multline", "multline*", "displaymath"];

pub(crate) const MULTIROW_MATH_ENVS: &[&str] = &[
    "align",
    "align*",
    "gather",
    "gather*",
    "eqnarray",
    "eqnarray*",
    "alignat",
    "alignat*",
    "flalign",
    "flalign*",
    "IEEEeqnarray",
    "IEEEeqnarray*",
];

/// One labelled equation extracted from a math environment or display-math block.
#[derive(Debug, Clone)]
pub(crate) struct ExtractedEquation {
    pub label: String,
    pub printed_tag: Option<String>,
    pub raw_slice_start: usize,
    pub raw_slice_end: usize,
}

/// Result of inspecting one math environment or display-math block.
#[derive(Debug, Clone, Default)]
pub(crate) struct MathFlush {
    pub labelled: Vec<ExtractedEquation>,
    pub unlabelled_count: u32,
}

/// Extracts labelled equation slices and counts unlabelled equations inside `stream[body_start..body_end]`.
pub(crate) fn inspect_math_body(
    stream: &str,
    body_start: usize,
    body_end: usize,
    multi_row: bool,
) -> MathFlush {
    let mut out = MathFlush::default();
    let raw_body = stream.get(body_start..body_end).unwrap_or("");
    if multi_row {
        for (r_start, r_end) in split_math_rows(raw_body, body_start) {
            let (t_start, t_end) = trim_ws_range(stream, r_start, r_end);
            if t_start >= t_end {
                continue;
            }
            let slice = stream.get(t_start..t_end).unwrap_or("");
            let stripped = strip_latex_comments(slice);
            if stripped.trim().is_empty() {
                continue;
            }
            if let Some((label, _)) = extract_tex_labels(&stripped).into_iter().next() {
                out.labelled.push(ExtractedEquation {
                    label,
                    printed_tag: extract_first_tag(&stripped),
                    raw_slice_start: t_start,
                    raw_slice_end: t_end,
                });
            } else {
                out.unlabelled_count += 1;
            }
        }
        return out;
    }

    let stripped = strip_latex_comments(raw_body);
    if stripped.trim().is_empty() {
        return out;
    }
    if let Some((label, _)) = extract_tex_labels(&stripped).into_iter().next() {
        out.labelled.push(ExtractedEquation {
            label,
            printed_tag: extract_first_tag(&stripped),
            raw_slice_start: body_start,
            raw_slice_end: body_end,
        });
    } else {
        out.unlabelled_count += 1;
    }
    out
}

/// Finds the matching `\]` for `\[` starting at `start` (comment- and escape-safe).
pub(crate) fn find_closing_display_bracket(stream: &str, start: usize) -> Option<usize> {
    let bytes = stream.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        if bytes[pos] == b'%' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if bytes[pos] == b'\\' {
            if bytes.get(pos + 1) == Some(&b']') {
                return Some(pos);
            }
            pos = (pos + 2).min(bytes.len());
            continue;
        }
        pos += 1;
    }
    None
}

/// Finds the matching `$$` starting at `start` (comment- and escape-safe).
pub(crate) fn find_closing_double_dollar(stream: &str, start: usize) -> Option<usize> {
    let bytes = stream.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        if bytes[pos] == b'%' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if bytes[pos] == b'\\' {
            pos = (pos + 2).min(bytes.len());
            continue;
        }
        if bytes[pos] == b'$' && bytes.get(pos + 1) == Some(&b'$') {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

fn trim_ws_range(stream: &str, mut start: usize, mut end: usize) -> (usize, usize) {
    let bytes = stream.as_bytes();
    while start < end && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start, end)
}

fn split_math_rows(raw_body: &str, base_offset: usize) -> Vec<(usize, usize)> {
    let bytes = raw_body.as_bytes();
    let mut rows = Vec::new();
    let mut row_start = 0usize;
    let mut i = 0usize;
    let mut nested = 0usize;
    let mut braces = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'%' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if raw_body[i..].starts_with("\\begin{") {
            nested += 1;
            i += 7;
            continue;
        }
        if raw_body[i..].starts_with("\\end{") {
            nested = nested.saturating_sub(1);
            i += 5;
            continue;
        }
        if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'\\') {
            if nested == 0 && braces == 0 {
                rows.push((base_offset + row_start, base_offset + i));
                i += 2;
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'*') {
                    i += 1;
                }
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'[') {
                    if let GroupOutcome::Closed { next_pos, .. } =
                        read_balanced_group(raw_body, i, b'[', b']', false)
                    {
                        i = next_pos;
                    }
                }
                row_start = i;
            } else {
                i += 2;
            }
            continue;
        }
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == b'{' {
            braces += 1;
        } else if bytes[i] == b'}' {
            braces = braces.saturating_sub(1);
        }
        i += 1;
    }
    if row_start < bytes.len() {
        rows.push((base_offset + row_start, base_offset + bytes.len()));
    }
    rows
}

fn extract_first_tag(stripped: &str) -> Option<String> {
    let bytes = stripped.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if stripped[i..].starts_with("\\tag") {
            let mut j = i + 4;
            if bytes.get(j) == Some(&b'*') {
                j += 1;
            }
            if bytes.get(j).is_some_and(|b| b.is_ascii_alphabetic()) {
                i += 4;
                continue;
            }
            j = skip_ws_and_comments(stripped, j);
            if bytes.get(j) == Some(&b'{') {
                if let GroupOutcome::Closed { content, .. } =
                    read_balanced_group(stripped, j, b'{', b'}', false)
                {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
        i = next_char_boundary(stripped, i);
    }
    None
}
