//! Canonical `.tex` entrypoint scoring, recursive `\input`/`\include` bundle
//! assembly (`depth <= 5`, cycle-safe), and section filtering.

use crate::tex_scan::strip_latex_comments_line;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const MAX_INCLUDE_DEPTH: usize = 5;
const MAX_ASSEMBLED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TexBundle {
    pub primary_path: String,
    pub primary_score: i32,
    pub assembled_tex: String,
    pub inlined_files: Vec<String>,
}

/// Scores `.tex` files and recursively inlines `\input` / `\include` (`depth <= 5`, cycle-safe).
pub fn assemble_tex_bundle(tex_files: &BTreeMap<String, String>) -> Option<TexBundle> {
    if tex_files.is_empty() {
        return None;
    }
    let stripped_map: BTreeMap<String, String> = tex_files
        .iter()
        .map(|(p, raw)| {
            let s = raw.lines().map(strip_latex_comments_line).collect::<Vec<_>>().join("\n");
            (p.clone(), s)
        })
        .collect();

    let mut referenced = BTreeSet::new();
    for (path, stripped) in &stripped_map {
        for (_, target) in find_tex_includes_in_line_block(stripped) {
            if let Some(resolved) = resolve_tex_include(path, &target, tex_files) {
                referenced.insert(resolved);
            }
        }
    }

    let mut best: Option<(i32, String)> = None;
    for (path, stripped) in &stripped_map {
        let score = score_tex_entrypoint(path, stripped, !referenced.contains(path));
        let replace = match &best {
            None => true,
            Some((best_score, best_path)) => score > *best_score || (score == *best_score && path < best_path),
        };
        if replace {
            best = Some((score, path.clone()));
        }
    }
    let (primary_score, primary_path) = best?;

    let mut stack = vec![primary_path.clone()];
    let mut inlined_files = Vec::new();
    let mut assembled = String::new();
    let mut capped = false;
    inline_tex_recursive(
        &primary_path,
        0,
        tex_files,
        &mut stack,
        &mut inlined_files,
        &mut assembled,
        &mut capped,
    );

    Some(TexBundle { primary_path, primary_score, assembled_tex: assembled, inlined_files })
}

pub fn score_tex_entrypoint(rel_path: &str, stripped_tex: &str, is_unreferenced: bool) -> i32 {
    let mut score = 0i32;
    if has_tex_macro(stripped_tex, "documentclass") {
        score += 1000;
    }
    if stripped_tex.contains("\\begin{document}") {
        score += 800;
    }
    let basename = rel_path.rsplit('/').next().unwrap_or(rel_path).to_ascii_lowercase();
    score += match basename.as_str() {
        "main.tex" => 300,
        "ms.tex" => 280,
        "paper.tex" => 260,
        "article.tex" => 240,
        "manuscript.tex" => 220,
        _ => 0,
    };
    if is_unreferenced {
        score += 150;
    }
    let depth = rel_path.bytes().filter(|&b| b == b'/').count() as i32;
    score - 50 * depth
}

fn has_tex_macro(text: &str, name: &str) -> bool {
    let needle = format!("\\{name}");
    for (idx, _) in text.match_indices(&needle) {
        let after = idx + needle.len();
        if after >= text.len() || !text.as_bytes()[after].is_ascii_alphabetic() {
            return true;
        }
    }
    false
}

fn push_bounded(out: &mut String, s: &str, capped: &mut bool) -> bool {
    if *capped {
        return false;
    }
    let rem = MAX_ASSEMBLED_BYTES.saturating_sub(out.len());
    if s.len() <= rem {
        out.push_str(s);
        if out.len() == MAX_ASSEMBLED_BYTES {
            *capped = true;
            return false;
        }
        true
    } else {
        let mut end = rem;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&s[..end]);
        *capped = true;
        false
    }
}

fn inline_tex_recursive(
    cur_path: &str,
    depth: usize,
    tex_files: &BTreeMap<String, String>,
    stack: &mut Vec<String>,
    inlined_files: &mut Vec<String>,
    out: &mut String,
    capped: &mut bool,
) {
    if *capped {
        return;
    }
    let Some(raw) = tex_files.get(cur_path) else { return };
    for raw_line in raw.lines() {
        if *capped {
            return;
        }
        let code_part = strip_latex_comments_line(raw_line);
        let includes = find_tex_includes_in_line_block(&code_part);
        if includes.is_empty() {
            if (!code_part.is_empty() || raw_line.is_empty())
                && (!push_bounded(out, &code_part, capped) || !push_bounded(out, "\n", capped))
            {
                return;
            }
            continue;
        }
        let mut cursor = 0usize;
        for ((span_start, span_end), target) in includes {
            if *capped || !push_bounded(out, &code_part[cursor..span_start], capped) {
                return;
            }
            cursor = span_end;
            match resolve_tex_include(cur_path, &target, tex_files) {
                Some(resolved) if stack.contains(&resolved) => {
                    let msg = format!("\n% [re-search: cycle detected in {resolved}]\n");
                    if !push_bounded(out, &msg, capped) {
                        return;
                    }
                }
                Some(resolved) if depth >= MAX_INCLUDE_DEPTH => {
                    let msg = format!("\n% [re-search: max include depth exceeded for {resolved}]\n");
                    if !push_bounded(out, &msg, capped) {
                        return;
                    }
                }
                Some(resolved) => {
                    if !inlined_files.contains(&resolved) {
                        inlined_files.push(resolved.clone());
                    }
                    let hdr = format!("\n% --- begin inlined: {resolved} ---\n");
                    if !push_bounded(out, &hdr, capped) {
                        return;
                    }
                    stack.push(resolved.clone());
                    inline_tex_recursive(&resolved, depth + 1, tex_files, stack, inlined_files, out, capped);
                    stack.pop();
                    let ftr = format!("% --- end inlined: {resolved} ---\n");
                    if !push_bounded(out, &ftr, capped) {
                        return;
                    }
                }
                None => {
                    if !push_bounded(out, &code_part[span_start..span_end], capped) {
                        return;
                    }
                }
            }
        }
        if !push_bounded(out, &code_part[cursor..], capped) || !push_bounded(out, "\n", capped) {
            return;
        }
    }
}

/// Scans `code` for active `\input` / `\include` commands where the next byte is NOT ASCII alphabetic
/// (avoiding `\includegraphics`, `\includeonly`, `\inputencoding`).
pub fn find_tex_includes_in_line_block(code: &str) -> Vec<((usize, usize), String)> {
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let mut bs = 0usize;
            let mut k = i;
            while k > 0 && bytes[k - 1] == b'\\' {
                bs += 1;
                k -= 1;
            }
            if bs.is_multiple_of(2) {
                let cmd_len = if bytes[i + 1..].starts_with(b"include") {
                    Some(8)
                } else if bytes[i + 1..].starts_with(b"input") {
                    Some(6)
                } else {
                    None
                };
                if let Some(clen) = cmd_len {
                    let after_cmd = i + clen;
                    if after_cmd < bytes.len() && !bytes[after_cmd].is_ascii_alphabetic() {
                        let mut p = after_cmd;
                        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                            p += 1;
                        }
                        if p < bytes.len() && bytes[p] == b'{' {
                            if let Some(rel_close) = code[p + 1..].find('}') {
                                let end = p + 1 + rel_close + 1;
                                let arg = code[p + 1..end - 1].trim().to_string();
                                if !arg.is_empty() {
                                    out.push(((i, end), arg));
                                }
                                i = end;
                                continue;
                            }
                        } else if p < bytes.len() {
                            let start_tok = p;
                            while p < bytes.len() && !bytes[p].is_ascii_whitespace() && bytes[p] != b'%' {
                                p += 1;
                            }
                            let arg = code[start_tok..p].trim().to_string();
                            if !arg.is_empty() {
                                out.push(((i, p), arg));
                                i = p;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

fn resolve_tex_include(cur_path: &str, raw_target: &str, tex_files: &BTreeMap<String, String>) -> Option<String> {
    let cleaned = raw_target.trim().trim_matches(['"', '\'']).replace('\\', "/");
    let parent = cur_path.rfind('/').map(|idx| &cur_path[..idx]).unwrap_or("");
    let mut candidates = Vec::new();
    for base in [parent, ""] {
        let joined = if base.is_empty() { cleaned.clone() } else { format!("{base}/{cleaned}") };
        candidates.push(joined.clone());
        if !joined.to_ascii_lowercase().ends_with(".tex") {
            candidates.push(format!("{joined}.tex"));
        }
    }
    for cand in candidates {
        let norm = normalize_rel_segments(&cand)?;
        if tex_files.contains_key(&norm) {
            return Some(norm);
        }
    }
    None
}

fn normalize_rel_segments(path: &str) -> Option<String> {
    let mut stack = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            stack.pop()?;
        } else {
            stack.push(seg);
        }
    }
    (!stack.is_empty()).then(|| stack.join("/"))
}

pub fn filter_tex_sections(tex: &str, filter: &str) -> String {
    static SEC_RE: OnceLock<Regex> = OnceLock::new();
    let re = SEC_RE.get_or_init(|| {
        Regex::new(r"^[^\S\r\n]*\\(part|chapter|section|subsection|subsubsection)\*?\s*(?:\[[^\]]*\])?\s*\{([^}]*)\}")
            .unwrap()
    });
    let needle = filter.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return tex.to_string();
    }

    let mut matched_blocks = Vec::new();
    let mut cur_block = String::new();
    let mut cur_matched = false;

    if needle == "abstract" {
        if let Some(start) = tex.find("\\begin{abstract}") {
            if let Some(rel_end) = tex[start..].find("\\end{abstract}") {
                matched_blocks.push(tex[start..start + rel_end + "\\end{abstract}".len()].to_string());
            }
        }
    }

    for line in tex.lines() {
        if let Some(caps) = re.captures(line) {
            if cur_matched && !cur_block.trim().is_empty() {
                matched_blocks.push(cur_block.clone());
            }
            cur_block.clear();
            cur_matched = caps[2].to_ascii_lowercase().contains(&needle);
        }
        if cur_matched {
            cur_block.push_str(line);
            cur_block.push('\n');
        }
    }
    if cur_matched && !cur_block.trim().is_empty() {
        matched_blocks.push(cur_block);
    }

    if !matched_blocks.is_empty() {
        return matched_blocks.join("\n");
    }
    // Fallback for short physics letters without matching `\section` headings: return
    // paragraphs containing `needle`, or the full TeX if none match.
    let paras: Vec<&str> = tex
        .split("\n\n")
        .filter(|p| p.to_ascii_lowercase().contains(&needle))
        .collect();
    if !paras.is_empty() {
        paras.join("\n\n")
    } else {
        tex.to_string()
    }
}
