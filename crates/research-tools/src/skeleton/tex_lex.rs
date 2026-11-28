//! Shared comment-safe TeX lexical helpers: balanced group reading, whitespace/comment
//! skipping, inline `\verb` skipping, citation extraction, and proof-heading classification.

use super::flatten::{is_nested_tex_if, VERBATIM_ENVS};
use crate::tex_scan::{
    extract_tex_citations, extract_tex_refs, strip_latex_comments, strip_latex_comments_line,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const MAX_GROUP_SCAN_BYTES: usize = 4_096;

/// V1 proof-target classification (consumed by V2 to populate `SkNode::proof_of` and `proves` edges).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProofHint {
    HeaderRef { label: String, heading: String },
    HeaderMention { kind_word: String, target_text: String, heading: String },
    Adjacent { target_order: usize },
    Unattached,
}

/// Result of reading a balanced `{...}` or `[...]` group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GroupOutcome {
    Closed { content: String, next_pos: usize },
    Unclosed { partial: String, next_pos: usize },
}

/// Advances `pos` by one UTF-8 character, clamped to `s.len()`.
pub(crate) fn next_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    pos + s[pos..].chars().next().map_or(1, char::len_utf8)
}

/// Skips ASCII whitespace and `%...\n` LaTeX comments starting at `pos`.
pub(crate) fn skip_ws_and_comments(text: &str, mut pos: usize) -> usize {
    let bytes = text.as_bytes();
    while pos < bytes.len() {
        if bytes[pos].is_ascii_whitespace() {
            pos += 1;
        } else if bytes[pos] == b'%' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
        } else {
            break;
        }
    }
    pos
}

/// Reads a balanced `{...}` or `[...]` group starting at `open_pos` (`bytes[open_pos] == open`).
///
/// Comment-safe (`%...\n`), escape-safe (`\{`, `\}`, `\[`, `\]`, `\\`, `\%`), and bounded so
/// unbalanced input never loops or swallows subsequent structural elements.
pub(crate) fn read_balanced_group(
    text: &str,
    open_pos: usize,
    open: u8,
    close: u8,
    allow_paragraph_break: bool,
) -> GroupOutcome {
    let bytes = text.as_bytes();
    debug_assert!(open_pos < bytes.len() && bytes[open_pos] == open);

    let mut pos = open_pos + 1;
    let mut depth = 1usize;
    let mut inner_brace_depth = 0usize;
    let mut out = String::new();
    let limit = (open_pos + MAX_GROUP_SCAN_BYTES).min(bytes.len());
    let first_line_end = text[open_pos + 1..]
        .find('\n')
        .map(|rel| open_pos + 1 + rel)
        .unwrap_or(bytes.len());

    while pos < limit {
        let b = bytes[pos];
        if b == b'\\' {
            if pos + 1 < limit {
                let next_ch = text[pos + 1..].chars().next().unwrap_or(' ');
                let end = pos + 1 + next_ch.len_utf8();
                out.push_str(&text[pos..end]);
                pos = end;
                continue;
            }
            out.push('\\');
            pos += 1;
            continue;
        }
        if b == b'%' {
            while pos < limit && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if !allow_paragraph_break && b == b'\n' {
            let mut look = pos + 1;
            while look < limit && matches!(bytes[look], b' ' | b'\t' | b'\r') {
                look += 1;
            }
            if look < limit && bytes[look] == b'\n' {
                break;
            }
        }
        if open == b'[' {
            if b == b'{' {
                inner_brace_depth += 1;
                out.push('{');
                pos += 1;
                continue;
            }
            if b == b'}' && inner_brace_depth > 0 {
                inner_brace_depth -= 1;
                out.push('}');
                pos += 1;
                continue;
            }
        }
        if b == open && (open == b'{' || inner_brace_depth == 0) {
            depth += 1;
            out.push(open as char);
            pos += 1;
            continue;
        }
        if b == close && (open == b'{' || inner_brace_depth == 0) {
            depth -= 1;
            if depth == 0 {
                return GroupOutcome::Closed { content: out, next_pos: pos + 1 };
            }
            out.push(close as char);
            pos += 1;
            continue;
        }
        let ch = text[pos..].chars().next().unwrap_or(' ');
        out.push(ch);
        pos += ch.len_utf8();
    }

    let partial = strip_latex_comments_line(&text[open_pos + 1..first_line_end])
        .trim()
        .to_string();
    GroupOutcome::Unclosed { partial, next_pos: first_line_end.max(open_pos + 1) }
}

/// Collapses internal whitespace in a LaTeX title/caption while preserving TeX math.
pub(crate) fn clean_tex_title(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Skips an inline `\verb<delim>...<delim>` (or `\verb*`) starting right after `\verb`.
pub(crate) fn skip_inline_verb(stream: &str, mut pos: usize) -> usize {
    let bytes = stream.as_bytes();
    if bytes.get(pos) == Some(&b'*') {
        pos += 1;
    }
    if pos >= bytes.len() || bytes[pos].is_ascii_alphabetic() || bytes[pos].is_ascii_whitespace() {
        return pos;
    }
    let delim = bytes[pos];
    pos += 1;
    while pos < bytes.len() && bytes[pos] != delim && bytes[pos] != b'\n' {
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == delim {
        pos += 1;
    }
    pos
}

/// Extracts unique citation keys in order of appearance from a raw heading string.
pub(crate) fn extract_unique_cites(raw: &str) -> Vec<String> {
    let stripped = strip_latex_comments(raw);
    let mut out = Vec::new();
    for cite in extract_tex_citations(&stripped) {
        if !cite.key.is_empty() && !out.contains(&cite.key) {
            out.push(cite.key);
        }
    }
    out
}

/// Classifies a proof environment as detached (by `\ref` or text mention) or adjacent.
pub(crate) fn classify_proof_hint(heading: Option<&str>, prev_provable: Option<usize>) -> ProofHint {
    if let Some(raw_h) = heading {
        let stripped = strip_latex_comments(raw_h);
        if let Some(r) = extract_tex_refs(&stripped).into_iter().find(|r| !r.key.is_empty()) {
            return ProofHint::HeaderRef { label: r.key, heading: raw_h.to_string() };
        }
        static MENTION_RE: OnceLock<Regex> = OnceLock::new();
        let re = MENTION_RE.get_or_init(|| {
            Regex::new(r"(?i)\b(?:proof\s+of\s+)?(?:the\s+)?(main\s+theorem|theorem|lemma|proposition|corollary|conjecture|claim)\s+([A-Za-z0-9]+(?:[.\-][A-Za-z0-9]+)*)").unwrap()
        });
        if let Some(caps) = re.captures(&stripped) {
            let kind_word = caps[1].to_ascii_lowercase();
            let target_text = caps[2].trim().to_string();
            return ProofHint::HeaderMention { kind_word, target_text, heading: raw_h.to_string() };
        }
    }
    match prev_provable {
        Some(target_order) => ProofHint::Adjacent { target_order },
        None => ProofHint::Unattached,
    }
}

/// Checks whether `stream` contains an active `\begin{document}` outside comments,
/// verbatim environments, and `\iffalse ... \fi` blocks.
pub(crate) fn has_active_begin_document(stream: &str) -> bool {
    let bytes = stream.as_bytes();
    let mut pos = 0usize;
    let mut iffalse_depth = 0usize;
    while pos < bytes.len() {
        if bytes[pos] == b'%' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if bytes[pos] == b'\\' {
            if pos + 1 >= bytes.len() {
                break;
            }
            if !bytes[pos + 1].is_ascii_alphabetic() {
                pos = next_char_boundary(stream, pos + 1);
                continue;
            }
            let mut end = pos + 1;
            while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                end += 1;
            }
            let cmd = &stream[pos + 1..end];
            if iffalse_depth > 0 {
                if cmd == "fi" {
                    iffalse_depth -= 1;
                } else if cmd == "else" && iffalse_depth == 1 {
                    iffalse_depth = 0;
                } else if is_nested_tex_if(cmd) {
                    iffalse_depth += 1;
                }
                pos = end;
                continue;
            }
            if cmd == "verb" {
                pos = skip_inline_verb(stream, end);
                continue;
            }
            if cmd == "iffalse" {
                iffalse_depth = 1;
                pos = end;
                continue;
            }
            if cmd == "begin" {
                let p = skip_ws_and_comments(stream, end);
                if bytes.get(p) == Some(&b'{') {
                    if let GroupOutcome::Closed { content, next_pos } =
                        read_balanced_group(stream, p, b'{', b'}', false)
                    {
                        let name = content.trim();
                        if name == "document" {
                            return true;
                        }
                        if VERBATIM_ENVS.contains(&name) {
                            let end_marker = format!("\\end{{{name}}}");
                            pos = stream[next_pos..]
                                .find(&end_marker)
                                .map(|rel| next_pos + rel + end_marker.len())
                                .unwrap_or(bytes.len());
                            continue;
                        }
                        pos = next_pos;
                        continue;
                    }
                }
            }
            pos = end;
            continue;
        }
        pos = next_char_boundary(stream, pos);
    }
    false
}

pub(crate) const MAX_MACROS: usize = 500;
const MAX_MACRO_BODY_CHARS: usize = 512;

/// Parses `\newcommand`, `\renewcommand`, `\providecommand`, or `\DeclareMathOperator`.
pub(crate) fn try_parse_macro_cmd(
    cmd: &str,
    text: &str,
    cmd_end: usize,
    macros: &mut Vec<super::model::MacroDef>,
) -> Option<(usize, Option<String>)> {
    if !matches!(cmd, "newcommand" | "renewcommand" | "providecommand" | "DeclareMathOperator") {
        return None;
    }
    let bytes = text.as_bytes();
    let mut pos = cmd_end;
    let is_star_op = cmd == "DeclareMathOperator" && bytes.get(pos) == Some(&b'*');
    if bytes.get(pos) == Some(&b'*') {
        pos += 1;
    }
    pos = skip_ws_and_comments(text, pos);
    let name = if bytes.get(pos) == Some(&b'{') {
        match read_balanced_group(text, pos, b'{', b'}', false) {
            GroupOutcome::Closed { content, next_pos } => {
                pos = skip_ws_and_comments(text, next_pos);
                content.trim().to_string()
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                return Some((next_pos, Some(format!("unclosed `{{` name in \\{cmd}"))));
            }
        }
    } else if bytes.get(pos) == Some(&b'\\') {
        let mut k = pos + 1;
        while k < bytes.len() && bytes[k].is_ascii_alphabetic() {
            k += 1;
        }
        if k == pos + 1 {
            return Some((cmd_end, None));
        }
        let n = text[pos..k].to_string();
        pos = skip_ws_and_comments(text, k);
        n
    } else {
        return Some((cmd_end, None));
    };

    let mut arity = 0u8;
    if cmd != "DeclareMathOperator" && bytes.get(pos) == Some(&b'[') {
        match read_balanced_group(text, pos, b'[', b']', false) {
            GroupOutcome::Closed { content, next_pos } => {
                arity = content.trim().parse::<u8>().unwrap_or(0).min(9);
                pos = skip_ws_and_comments(text, next_pos);
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                return Some((next_pos, Some(format!("unclosed `[` arity in \\{cmd}"))));
            }
        }
        if bytes.get(pos) == Some(&b'[') {
            match read_balanced_group(text, pos, b'[', b']', false) {
                GroupOutcome::Closed { next_pos, .. } => pos = skip_ws_and_comments(text, next_pos),
                GroupOutcome::Unclosed { next_pos, .. } => {
                    return Some((next_pos, Some(format!("unclosed `[` default in \\{cmd}"))));
                }
            }
        }
    }
    if bytes.get(pos) != Some(&b'{') {
        return Some((pos, None));
    }
    let body = match read_balanced_group(text, pos, b'{', b'}', true) {
        GroupOutcome::Closed { content, next_pos } => {
            pos = next_pos;
            content.trim().to_string()
        }
        GroupOutcome::Unclosed { next_pos, .. } => {
            return Some((next_pos, Some(format!("unclosed `{{` body in \\{cmd}{{{name}}}"))));
        }
    };

    if name.starts_with('\\') && macros.len() < MAX_MACROS && body.chars().count() <= MAX_MACRO_BODY_CHARS {
        let rendered_body = if cmd == "DeclareMathOperator" {
            if is_star_op { format!("\\operatorname*{{{body}}}") } else { format!("\\operatorname{{{body}}}") }
        } else {
            body
        };
        if let Some(existing) = macros.iter_mut().find(|m| m.name == name) {
            existing.arity = arity;
            existing.body = rendered_body;
        } else {
            macros.push(super::model::MacroDef { name, arity, body: rendered_body });
        }
    }
    Some((pos, None))
}
