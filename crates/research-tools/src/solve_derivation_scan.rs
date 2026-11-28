//! Low-level deterministic TeX display-math scanner for `solve_derivation` (package S17a).
//!
//! Pure string/line scanning with zero panics on untrusted UTF-8 input.

use regex::Regex;
use std::sync::OnceLock;

const MULTIROW_ENVS: &[&str] = &["align", "gather", "eqnarray", "alignat", "flalign"];
const NESTED_ENVS: &[&str] = &[
    "matrix", "cases", "pmatrix", "bmatrix", "Bmatrix", "vmatrix", "Vmatrix", "smallmatrix",
    "array", "split", "aligned", "gathered", "alignedat", "dcases", "rcases", "subarray",
    "multlined", "CD", "tabular",
];

pub(super) struct RawEqBlock {
    pub env: String,
    pub start_line: usize,
    pub end_line: usize,
    pub raw_tex: String,
    pub has_nonumber: bool,
    pub has_tag: bool,
}

enum ActiveDelim {
    Env {
        env_label: String,
        end_marker: String,
        multi_row: bool,
        open_line: usize,
        body: Vec<(usize, String)>,
    },
    Bracket {
        open_line: usize,
        body: Vec<(usize, String)>,
    },
    Dollar {
        open_line: usize,
        body: Vec<(usize, String)>,
    },
}

/// Mask `\begin{verbatim|comment|lstlisting}...\end{...}` lines with blanks (preserving line count).
pub(super) fn mask_skipped_envs(normalized: &str) -> String {
    static SKIP_RE: OnceLock<Regex> = OnceLock::new();
    let skip_re = SKIP_RE
        .get_or_init(|| Regex::new(r"\\begin\{(verbatim|comment|lstlisting)(\*)?\}").unwrap());
    let mut out = Vec::new();
    let mut skip_end: Option<String> = None;

    for raw_line in normalized.lines() {
        if let Some(ref end_tag) = skip_end {
            if let Some(pos) = raw_line.find(end_tag.as_str()) {
                let tail = &raw_line[pos + end_tag.len()..];
                skip_end = None;
                out.push(tail.to_string());
            } else {
                out.push(String::new());
            }
            continue;
        }
        let uncommented = strip_tex_comment(raw_line);
        if let Some(cap) = find_unescaped_regex(uncommented, skip_re) {
            let m = cap.get(0).unwrap();
            let end_tag = format!("\\end{{{}{}}}", &cap[1], cap.get(2).map_or("", |s| s.as_str()));
            let prefix = &uncommented[..m.start()];
            let after = &uncommented[m.end()..];
            if let Some(end_pos) = after.find(&end_tag) {
                out.push(format!("{prefix}{}", &after[end_pos + end_tag.len()..]));
            } else {
                skip_end = Some(end_tag);
                out.push(prefix.to_string());
            }
        } else {
            out.push(raw_line.to_string());
        }
    }
    out.join("\n")
}

/// Break escaped `\\begin{` (odd preceding backslashes) into `\\ begin{` in the scratch string
/// passed to read-only `investigation_outline_tex` while preserving 1-based line numbers.
pub(super) fn mask_escaped_begins_for_outline(masked: &str) -> String {
    let mut out = String::with_capacity(masked.len());
    let bytes = masked.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = masked[i..].find("\\begin{") {
        let pos = i + rel;
        if !is_unescaped_at(bytes, pos) {
            out.push_str(&masked[i..pos]);
            out.push_str(" begin{");
            i = pos + "\\begin{".len();
        } else {
            out.push_str(&masked[i..pos + "\\begin{".len()]);
            i = pos + "\\begin{".len();
        }
    }
    out.push_str(&masked[i..]);
    out
}

/// Extract display-math equation blocks from already-masked TeX text; errors on unclosed blocks.
pub(super) fn extract_display_equations(masked: &str) -> Result<Vec<RawEqBlock>, String> {
    static ENV_BEGIN_RE: OnceLock<Regex> = OnceLock::new();
    let env_begin_re = ENV_BEGIN_RE.get_or_init(|| {
        Regex::new(r"\\begin\{(equation|align|gather|eqnarray|multline|alignat|flalign|displaymath)(\*)?\}")
            .unwrap()
    });

    let mut out = Vec::new();
    let mut active: Option<ActiveDelim> = None;

    for (idx, raw_line) in masked.lines().enumerate() {
        let line_no = idx + 1;
        let mut rest = strip_tex_comment(raw_line);

        loop {
            if let Some(mut state) = active.take() {
                match &mut state {
                    ActiveDelim::Env { env_label, end_marker, multi_row, open_line, body } => {
                        if let Some(pos) = find_unescaped(rest, end_marker.as_str()) {
                            body.push((line_no, rest[..pos].to_string()));
                            flush_env_block(env_label, *multi_row, *open_line, body, &mut out);
                            rest = &rest[pos + end_marker.len()..];
                            continue;
                        }
                        body.push((line_no, rest.to_string()));
                        active = Some(state);
                        break;
                    }
                    ActiveDelim::Bracket { open_line, body } => {
                        if let Some(pos) = find_unescaped(rest, "\\]") {
                            body.push((line_no, rest[..pos].to_string()));
                            flush_single_block("bracket", Some(*open_line), body, &mut out);
                            rest = &rest[pos + 2..];
                            continue;
                        }
                        body.push((line_no, rest.to_string()));
                        active = Some(state);
                        break;
                    }
                    ActiveDelim::Dollar { open_line, body } => {
                        if let Some(pos) = find_unescaped(rest, "$$") {
                            body.push((line_no, rest[..pos].to_string()));
                            flush_single_block("dollar", Some(*open_line), body, &mut out);
                            rest = &rest[pos + 2..];
                            continue;
                        }
                        body.push((line_no, rest.to_string()));
                        active = Some(state);
                        break;
                    }
                }
            }

            let next_env = find_unescaped_regex(rest, env_begin_re)
                .map(|c| (c.get(0).unwrap().start(), c));
            let next_brk = find_unescaped(rest, "\\[");
            let next_dol = find_unescaped(rest, "$$");

            let earliest = [next_env.as_ref().map(|(p, _)| *p), next_brk, next_dol]
                .into_iter()
                .flatten()
                .min();
            let Some(pos) = earliest else { break };

            if next_env.as_ref().is_some_and(|(p, _)| *p == pos) {
                let (_, cap) = next_env.unwrap();
                let m = cap.get(0).unwrap();
                let base = &cap[1];
                let star = cap.get(2).map_or("", |s| s.as_str());
                let env_label = format!("{base}{star}");
                let mut after = &rest[m.end()..];
                if base == "alignat" {
                    after = strip_leading_braced_arg(after);
                }
                active = Some(ActiveDelim::Env {
                    end_marker: format!("\\end{{{env_label}}}"),
                    env_label,
                    multi_row: MULTIROW_ENVS.contains(&base),
                    open_line: line_no,
                    body: Vec::new(),
                });
                rest = after;
            } else if next_brk == Some(pos) {
                active = Some(ActiveDelim::Bracket { open_line: line_no, body: Vec::new() });
                rest = &rest[pos + 2..];
            } else {
                active = Some(ActiveDelim::Dollar { open_line: line_no, body: Vec::new() });
                rest = &rest[pos + 2..];
            }
        }
    }

    if let Some(unclosed) = active {
        let msg = match unclosed {
            ActiveDelim::Env { env_label, open_line, .. } => {
                format!("Unclosed `\\begin{{{env_label}}}` opened at line {open_line}.")
            }
            ActiveDelim::Bracket { open_line, .. } => {
                format!("Unclosed `\\[` display math opened at line {open_line}.")
            }
            ActiveDelim::Dollar { open_line, .. } => {
                format!("Unclosed `$$` display math opened at line {open_line}.")
            }
        };
        return Err(msg);
    }
    Ok(out)
}

fn flush_single_block(
    env: &str,
    open_line: Option<usize>,
    body: &[(usize, String)],
    out: &mut Vec<RawEqBlock>,
) {
    let mut start = None;
    let mut end = 0usize;
    let mut lines = Vec::new();
    for (ln, seg) in body {
        let t = seg.trim();
        if !t.is_empty() {
            if start.is_none() {
                start = Some(*ln);
            }
            end = *ln;
            lines.push(t);
        }
    }
    let raw_tex = lines.join("\n").trim().to_string();
    if let Some(first_body) = start {
        if !raw_tex.is_empty() {
            let start_line = open_line.map_or(first_body, |o| o.min(first_body));
            out.push(RawEqBlock {
                env: env.to_string(),
                start_line,
                end_line: end,
                has_nonumber: has_nonumber_or_notag(&raw_tex),
                has_tag: has_tex_tag(&raw_tex),
                raw_tex,
            });
        }
    }
}

fn flush_env_block(
    env: &str,
    multi_row: bool,
    open_line: usize,
    body: &[(usize, String)],
    out: &mut Vec<RawEqBlock>,
) {
    if !multi_row {
        flush_single_block(env, Some(open_line), body, out);
        return;
    }
    let stripped = strip_intertext_lines(body);
    let mut rows: Vec<Vec<(usize, String)>> = Vec::new();
    let mut cur: Vec<(usize, String)> = Vec::new();
    let mut nested = 0usize;
    let mut braces = 0usize;

    for (ln, seg) in &stripped {
        let bytes = seg.as_bytes();
        let (mut i, mut start) = (0usize, 0usize);
        while i < bytes.len() {
            if let Some(len) = match_nested_cmd(&seg[i..], "\\begin{") {
                nested += 1;
                i += len;
                continue;
            }
            if let Some(len) = match_nested_cmd(&seg[i..], "\\end{") {
                nested = nested.saturating_sub(1);
                i += len;
                continue;
            }
            if seg[i..].starts_with("\\\\") && nested == 0 && braces == 0 {
                cur.push((*ln, seg[start..i].to_string()));
                rows.push(std::mem::take(&mut cur));
                i += 2;
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
                if i < bytes.len() && bytes[i] == b'*' { i += 1; }
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
                if i < bytes.len() && bytes[i] == b'[' {
                    if let Some(rel_end) = seg[i + 1..].find(']') {
                        let arg = &seg[i + 1..i + 1 + rel_end];
                        if is_tex_length_spec(arg) {
                            i += rel_end + 2;
                        }
                    }
                }
                start = i;
                continue;
            }
            if seg[i..].starts_with("\\{") || seg[i..].starts_with("\\}") {
                i += 2;
                continue;
            }
            if bytes[i] == b'{' { braces += 1; i += 1; continue; }
            if bytes[i] == b'}' { braces = braces.saturating_sub(1); i += 1; continue; }
            i += seg[i..].chars().next().map_or(1, char::len_utf8);
        }
        if start < bytes.len() {
            cur.push((*ln, seg[start..].to_string()));
        }
    }
    rows.push(cur);

    for row in rows {
        flush_single_block(env, None, &row, out);
    }
}

/// Strip single- or multi-line `\intertext{...}` and `\shortintertext{...}` across body lines.
fn strip_intertext_lines(body: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut out = Vec::with_capacity(body.len());
    let mut skip_depth = 0usize;

    for (ln, seg) in body {
        let bytes = seg.as_bytes();
        let mut kept = String::with_capacity(seg.len());
        let mut i = 0usize;
        while i < bytes.len() {
            if skip_depth > 0 {
                if seg[i..].starts_with("\\{") || seg[i..].starts_with("\\}") {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'{' { skip_depth += 1; i += 1; continue; }
                if bytes[i] == b'}' { skip_depth -= 1; i += 1; continue; }
                i += seg[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            let cmd_len = if seg[i..].starts_with("\\intertext") && is_unescaped_at(bytes, i) {
                Some("\\intertext".len())
            } else if seg[i..].starts_with("\\shortintertext") && is_unescaped_at(bytes, i) {
                Some("\\shortintertext".len())
            } else {
                None
            };
            if let Some(clen) = cmd_len {
                let mut j = i + clen;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
                if j < bytes.len() && bytes[j] == b'{' {
                    skip_depth = 1;
                    i = j + 1;
                    continue;
                }
            }
            let ch = seg[i..].chars().next().unwrap();
            kept.push(ch);
            i += ch.len_utf8();
        }
        out.push((*ln, kept));
    }
    out
}

fn is_tex_length_spec(raw: &str) -> bool {
    static LEN_RE: OnceLock<Regex> = OnceLock::new();
    LEN_RE
        .get_or_init(|| {
            Regex::new(r"^\s*[+-]?(?:\d+(?:\.\d*)?|\.\d+)\s*(?:pt|pc|in|bp|cm|mm|dd|cc|sp|em|ex|mu)\s*$")
                .unwrap()
        })
        .is_match(raw)
}

fn has_nonumber_or_notag(s: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    find_unescaped_regex(s, RE.get_or_init(|| Regex::new(r"\\(?:nonumber|notag)\b").unwrap()))
        .is_some()
}

fn has_tex_tag(s: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    find_unescaped_regex(s, RE.get_or_init(|| Regex::new(r"\\tag\*?\s*\{").unwrap())).is_some()
}

fn match_nested_cmd(s: &str, cmd: &str) -> Option<usize> {
    let rest = s.strip_prefix(cmd)?;
    let close = rest.find('}')?;
    let inner = rest[..close].trim();
    let base = inner.strip_suffix('*').unwrap_or(inner);
    NESTED_ENVS.contains(&base).then_some(cmd.len() + close + 1)
}

pub(super) fn is_unescaped_at(bytes: &[u8], pos: usize) -> bool {
    let mut bs = 0usize;
    while bs < pos && bytes[pos - 1 - bs] == b'\\' {
        bs += 1;
    }
    bs.is_multiple_of(2)
}

fn find_unescaped(s: &str, needle: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut offset = 0usize;
    while let Some(rel) = s[offset..].find(needle) {
        let pos = offset + rel;
        if is_unescaped_at(bytes, pos) {
            return Some(pos);
        }
        offset = pos + 1;
    }
    None
}

fn find_unescaped_regex<'a>(s: &'a str, re: &Regex) -> Option<regex::Captures<'a>> {
    let bytes = s.as_bytes();
    let mut offset = 0usize;
    while let Some(cap) = re.captures(&s[offset..]) {
        let m = cap.get(0)?;
        let abs_start = offset + m.start();
        if is_unescaped_at(bytes, abs_start) {
            return Some(cap);
        }
        offset = abs_start + 1;
    }
    None
}

pub(super) fn strip_tex_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'%' && is_unescaped_at(b, i) {
            return &line[..i];
        }
    }
    line
}

fn strip_leading_braced_arg(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some((_, end_offset)) = extract_braced_span(trimmed) {
        return &trimmed[end_offset..];
    }
    s
}

/// Extract balanced `{...}` starting at index 0 (respecting `\{` and `\}`), returning
/// `(inner_content, byte_offset_after_closing_brace)`.
pub(super) fn extract_braced_span(s: &str) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut out = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if s[i..].starts_with("\\{") || s[i..].starts_with("\\}") {
            if depth > 0 {
                out.push_str(&s[i..i + 2]);
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            if depth > 0 {
                out.push('{');
            }
            depth += 1;
            i += 1;
            continue;
        }
        if bytes[i] == b'}' {
            depth -= 1;
            i += 1;
            if depth == 0 {
                return Some((out, i));
            }
            out.push('}');
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        if depth > 0 {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    None
}

pub(super) fn extract_first_tex_label(raw: &str) -> Option<String> {
    static LABEL_RE: OnceLock<Regex> = OnceLock::new();
    let re = LABEL_RE.get_or_init(|| Regex::new(r"\\label\s*\{").unwrap());
    let cap = find_unescaped_regex(raw, re)?;
    let brace_pos = cap.get(0)?.end() - 1;
    let (inner, _) = extract_braced_span(&raw[brace_pos..])?;
    let trimmed = inner.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
