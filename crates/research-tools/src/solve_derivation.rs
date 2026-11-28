//! Deterministic TeX-only derivation splitter for Solve check-a-derivation intake (package S17a).
//!
//! Pure deterministic Rust (`< 1` s, zero model calls, zero I/O). Reuses
//! `crate::investigation_outline::build_outline` (`investigation_outline_tex.rs`, read-only)
//! for section anchors and numbered equation labels, while extracting exact raw TeX blocks for
//! numbered and starred display-math environments (`equation`/`align`/`gather`/`eqnarray`/
//! `multline`/`alignat`/`flalign`/`displaymath`, `\[...\]`, `$$...$$`) and consecutive pairs.
//! PDF derivations are explicitly rejected (`00-overview.md` 2.12).

#[path = "solve_derivation_scan.rs"]
mod scan;

use crate::investigation_outline::{build_outline, OutlineItem, OutlineSection, PaperOutline};
use ledger::sha256_hex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

/// Maximum accepted TeX derivation size (2 MiB).
pub const MAX_DERIVATION_TEX_BYTES: usize = 2 * 1024 * 1024;

/// User-facing error when a PDF is passed to the TeX-only S17 intake.
pub const PDF_REJECT_MESSAGE: &str =
    "PDF derivations are not supported yet; provide a .tex file or paste LaTeX directly.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationEquation {
    pub index: usize,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tex_label: Option<String>,
    pub env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub raw_tex: String,
    pub source_tex_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsecutiveEquationPair {
    pub pair_index: usize,
    pub from_index: usize,
    pub to_index: usize,
    pub from_label: String,
    pub to_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationSplit {
    pub source_sha256: String,
    pub outline_style: String,
    pub section_count: usize,
    pub equations: Vec<DerivationEquation>,
    pub pairs: Vec<ConsecutiveEquationPair>,
    pub elapsed_ms: u64,
}

/// Fail-closed PDF check for S17 TeX-only intake (`00-overview.md` 2.12).
pub fn reject_if_pdf(file_name: Option<&str>, bytes: &[u8]) -> Result<(), String> {
    if let Some(name) = file_name {
        let is_pdf_ext = Path::new(name.trim())
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        if is_pdf_ext {
            return Err(PDF_REJECT_MESSAGE.to_string());
        }
    }
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let trimmed = stripped
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map_or(&[][..], |idx| &stripped[idx..]);
    if trimmed.starts_with(b"%PDF-") {
        return Err(PDF_REJECT_MESSAGE.to_string());
    }
    Ok(())
}

/// Validate raw bytes (rejecting PDF, oversize, NUL/control bytes, and invalid UTF-8) and split.
pub fn split_tex_derivation_bytes(
    file_name: Option<&str>,
    bytes: &[u8],
) -> Result<DerivationSplit, String> {
    reject_if_pdf(file_name, bytes)?;
    if bytes.len() > MAX_DERIVATION_TEX_BYTES {
        return Err(format!(
            "TeX derivation exceeds maximum size of {} bytes",
            MAX_DERIVATION_TEX_BYTES
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "TeX derivation must be valid UTF-8 text.".to_string())?;
    split_tex_derivation(text)
}

/// Deterministically split a LaTeX derivation into numbered equations and consecutive pairs.
pub fn split_tex_derivation(tex: &str) -> Result<DerivationSplit, String> {
    let started = Instant::now();
    reject_if_pdf(None, tex.as_bytes())?;
    if tex.len() > MAX_DERIVATION_TEX_BYTES {
        return Err(format!(
            "TeX derivation exceeds maximum size of {} bytes",
            MAX_DERIVATION_TEX_BYTES
        ));
    }
    validate_tex_control_chars(tex)?;
    let normalized = tex.strip_prefix('\u{FEFF}').unwrap_or(tex).replace("\r\n", "\n");
    if normalized.trim().is_empty() {
        return Err("TeX derivation is empty.".to_string());
    }

    let source_sha256 = sha256_hex(normalized.as_bytes());
    let masked = scan::mask_skipped_envs(&normalized);
    let outline = build_tex_outline_reusing_reader(&masked);
    let raw_blocks = scan::extract_display_equations(&masked)?;
    if raw_blocks.is_empty() {
        return Err(
            "No display equations found in the provided TeX derivation. Use equation/align/gather environments or \\[...\\] / $$...$$."
                .to_string(),
        );
    }

    let mut used_outline_eqs = vec![false; outline.equations.len()];
    let mut equations = Vec::with_capacity(raw_blocks.len());
    for (idx, blk) in raw_blocks.into_iter().enumerate() {
        let one_based = idx + 1;
        let tex_label = scan::extract_first_tex_label(&blk.raw_tex);
        let eligible_for_outline = !blk.has_nonumber || blk.has_tag;
        let outline_label = if eligible_for_outline {
            match_outline_equation(
                &outline.equations,
                &mut used_outline_eqs,
                blk.start_line,
                blk.end_line,
            )
        } else {
            None
        };
        let label = outline_label.unwrap_or_else(|| format!("eq-{one_based}"));
        let section = enclosing_section(&outline.sections, blk.start_line);
        let source_tex_sha256 = sha256_hex(blk.raw_tex.as_bytes());
        equations.push(DerivationEquation {
            index: one_based,
            label,
            tex_label,
            env: blk.env,
            section,
            start_line: blk.start_line,
            end_line: blk.end_line,
            raw_tex: blk.raw_tex,
            source_tex_sha256,
        });
    }

    let mut pairs = Vec::with_capacity(equations.len().saturating_sub(1));
    for win in equations.windows(2) {
        pairs.push(ConsecutiveEquationPair {
            pair_index: pairs.len() + 1,
            from_index: win[0].index,
            to_index: win[1].index,
            from_label: win[0].label.clone(),
            to_label: win[1].label.clone(),
        });
    }

    let section_count = outline
        .sections
        .iter()
        .filter(|s| matches!(s.kind.as_str(), "section" | "appendix"))
        .count();

    Ok(DerivationSplit {
        source_sha256,
        outline_style: "tex".to_string(),
        section_count,
        equations,
        pairs,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

fn validate_tex_control_chars(tex: &str) -> Result<(), String> {
    if tex.bytes().any(|b| b == 0) {
        return Err("TeX derivation must not contain NUL bytes.".to_string());
    }
    if tex.bytes().any(|b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t')) {
        return Err("TeX derivation contains disallowed ASCII control characters.".to_string());
    }
    Ok(())
}

/// Reuse `investigation_outline::build_outline` (read-only) while ensuring masked TeX fragments
/// always enter the TeX outline branch (`looks_like_tex`) with adjusted 1-based line numbers.
fn build_tex_outline_reusing_reader(masked: &str) -> PaperOutline {
    let outline_src = scan::mask_escaped_begins_for_outline(masked);
    let mut has_docclass = false;
    let mut has_begindoc = false;
    let mut sec_hits = 0usize;
    for line in outline_src.lines() {
        let t = scan::strip_tex_comment(line);
        if t.contains("\\documentclass") {
            has_docclass = true;
        }
        if t.contains("\\begin{document}") {
            has_begindoc = true;
        }
        if ["\\section", "\\subsection", "\\subsubsection", "\\chapter"]
            .iter()
            .any(|k| t.contains(k))
        {
            sec_hits += 1;
        }
    }
    if (has_docclass && has_begindoc) || sec_hits >= 2 {
        return build_outline(&outline_src, &[]);
    }
    let wrapped = format!(
        "\\documentclass{{article}}\n\\begin{{document}}\n{outline_src}\n\\end{{document}}\n"
    );
    let mut out = build_outline(&wrapped, &[]);
    out.total_lines = outline_src.lines().count();
    for sec in &mut out.sections {
        sec.start_line = sec.start_line.saturating_sub(2).max(1);
        sec.end_line = sec.end_line.saturating_sub(2).min(out.total_lines.max(1));
    }
    for eq in &mut out.equations {
        eq.line = eq.line.saturating_sub(2).max(1);
    }
    for cap in &mut out.captions {
        cap.line = cap.line.saturating_sub(2).max(1);
    }
    out
}

fn match_outline_equation(
    outline_eqs: &[OutlineItem],
    used: &mut [bool],
    start_line: usize,
    end_line: usize,
) -> Option<String> {
    for (idx, item) in outline_eqs.iter().enumerate() {
        if !used[idx] && item.line >= start_line && item.line <= end_line {
            used[idx] = true;
            return Some(item.label.clone());
        }
    }
    None
}

fn enclosing_section(sections: &[OutlineSection], line: usize) -> Option<String> {
    sections
        .iter()
        .filter(|s| s.start_line <= line && line <= s.end_line)
        .max_by_key(|s| (s.level, s.start_line))
        .map(|s| {
            let t = s.title.trim();
            if t.is_empty() { s.key.clone() } else { t.to_string() }
        })
}

#[cfg(test)]
#[path = "solve_derivation_tests.rs"]
mod tests;
