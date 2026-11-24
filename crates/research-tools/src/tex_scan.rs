//! Pure LaTeX/BibTeX lexical scanners and arXiv/DOI identifier normalizers.

use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanRef {
    pub line: u32,
    pub command: String,
    pub key: String,
}

#[derive(Debug, Clone, Default)]
pub struct BibEntryMeta {
    pub key: String,
    pub base_arxiv_id: Option<String>,
    pub doi_lower: Option<String>,
    pub marked_withdrawn: bool,
    pub marked_retracted: bool,
}

/// Strips unescaped `%` comments on one line while preserving `\%`, handling `\\`
/// line breaks (`\\%` IS a comment; `\\\%` is `\\` + literal `\%`), and skipping `\verb|...|`.
pub fn strip_latex_comments_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1..].starts_with(b"verb") && preceding_backslashes_even(bytes, i) {
            let mut j = i + 5;
            if j < bytes.len() && bytes[j] == b'*' {
                j += 1;
            }
            if j < bytes.len() && !bytes[j].is_ascii_alphabetic() && !bytes[j].is_ascii_whitespace() {
                let delim = bytes[j];
                j += 1;
                while j < bytes.len() && bytes[j] != delim {
                    j += 1;
                }
                i = (j + 1).min(bytes.len());
                continue;
            }
        }
        if bytes[i] == b'%' && preceding_backslashes_even(bytes, i) {
            return line[..i].to_string();
        }
        i += 1;
    }
    line.to_string()
}

/// Strips unescaped `%` comments while preserving exact 1-based line numbers (`\n` count).
pub fn strip_latex_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (idx, raw_line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let body = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        out.push_str(&strip_latex_comments_line(body));
    }
    out
}

fn preceding_backslashes_even(bytes: &[u8], pos: usize) -> bool {
    let mut bs = 0usize;
    let mut k = pos;
    while k > 0 && bytes[k - 1] == b'\\' {
        bs += 1;
        k -= 1;
    }
    bs.is_multiple_of(2)
}

fn is_active_command_backslash(bytes: &[u8], cmd_backslash_pos: usize) -> bool {
    preceding_backslashes_even(bytes, cmd_backslash_pos)
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn offset_to_line(starts: &[usize], offset: usize) -> u32 {
    match starts.binary_search(&offset) {
        Ok(idx) => (idx + 1) as u32,
        Err(idx) => idx as u32,
    }
}

pub fn extract_tex_labels(stripped_tex: &str) -> Vec<(String, u32)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\\label\s*\{([^{}\r\n]*)\}").unwrap());
    let starts = line_starts(stripped_tex);
    let bytes = stripped_tex.as_bytes();
    let mut out = Vec::new();
    for m in re.captures_iter(stripped_tex) {
        let full = m.get(0).unwrap();
        if !is_active_command_backslash(bytes, full.start()) {
            continue;
        }
        let key = m[1].trim();
        if !key.is_empty() {
            out.push((key.to_string(), offset_to_line(&starts, full.start())));
        }
    }
    out
}

pub fn extract_tex_refs(stripped_tex: &str) -> Vec<SpanRef> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\\((?:eq|auto|c|C|page)?ref)\*?\s*\{([^{}]*)\}").unwrap());
    let starts = line_starts(stripped_tex);
    let bytes = stripped_tex.as_bytes();
    let mut out = Vec::new();
    for caps in re.captures_iter(stripped_tex) {
        let full = caps.get(0).unwrap();
        if !is_active_command_backslash(bytes, full.start()) {
            continue;
        }
        let cmd = caps[1].to_string();
        let line = offset_to_line(&starts, full.start());
        let body = caps[2].trim();
        if body.is_empty() {
            out.push(SpanRef { line, command: cmd, key: String::new() });
            continue;
        }
        for part in body.split(',') {
            let key = part.trim();
            out.push(SpanRef { line, command: cmd.clone(), key: key.to_string() });
        }
    }
    out
}

pub fn extract_tex_citations(stripped_tex: &str) -> Vec<SpanRef> {
    static CMD_RE: OnceLock<Regex> = OnceLock::new();
    let re = CMD_RE.get_or_init(|| {
        Regex::new(r"\\(cite(?:p|t|author|year|alt|alp)?|parencite|textcite|autocite)\*?").unwrap()
    });
    let starts = line_starts(stripped_tex);
    let bytes = stripped_tex.as_bytes();
    let mut out = Vec::new();
    for caps in re.captures_iter(stripped_tex) {
        let full = caps.get(0).unwrap();
        if !is_active_command_backslash(bytes, full.start()) {
            continue;
        }
        let mut pos = full.end();
        if pos < bytes.len() && bytes[pos].is_ascii_alphabetic() {
            continue;
        }
        for _ in 0..2 {
            pos = skip_ws(bytes, pos);
            if pos < bytes.len() && bytes[pos] == b'[' {
                if let Some(after_opt) = skip_balanced(bytes, pos, b'[', b']') {
                    pos = after_opt;
                } else {
                    break;
                }
            }
        }
        pos = skip_ws(bytes, pos);
        if pos < bytes.len() && bytes[pos] == b'{' {
            if let Some(end_brace) = skip_balanced(bytes, pos, b'{', b'}') {
                let inner = stripped_tex[pos + 1..end_brace - 1].trim();
                let line = offset_to_line(&starts, full.start());
                let cmd = caps[1].to_string();
                if inner.is_empty() {
                    out.push(SpanRef { line, command: cmd, key: String::new() });
                } else {
                    for part in inner.split(',') {
                        let key = part.trim();
                        if key != "*" {
                            out.push(SpanRef { line, command: cmd.clone(), key: key.to_string() });
                        }
                    }
                }
            }
        }
    }
    out
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn skip_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == open {
            depth += 1;
        } else if bytes[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// Parses BibTeX entries (`@article{key,`, `@book(key,`), ignoring `@comment`, `@preamble`, `@string`, and `%` comments.
pub fn parse_bib_entries(bib_content: &str) -> BTreeMap<String, BibEntryMeta> {
    static ENTRY_RE: OnceLock<Regex> = OnceLock::new();
    static EPRINT_RE: OnceLock<Regex> = OnceLock::new();
    static DOI_FIELD_RE: OnceLock<Regex> = OnceLock::new();
    static NOTE_RE: OnceLock<Regex> = OnceLock::new();

    let stripped_bib = strip_latex_comments(bib_content);
    let entry_re = ENTRY_RE.get_or_init(|| {
        Regex::new(r"(?m)^[^\S\r\n]*@([A-Za-z]+)\s*([\{\(])\s*([^,\s\{\}\(\)]+)\s*,").unwrap()
    });
    let eprint_re = EPRINT_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:eprint|arxivid)\s*=\s*[\{"']?\s*([^,\}"'\s]+)"#).unwrap()
    });
    let doi_re = DOI_FIELD_RE.get_or_init(|| {
        Regex::new(r#"(?i)\bdoi\s*=\s*[\{"']?\s*([^,\}"'\s]+)"#).unwrap()
    });
    let note_re = NOTE_RE.get_or_init(|| Regex::new(r#"(?i)\bnote\s*=\s*[\{"]([^\}"]+)"#).unwrap());

    let mut out = BTreeMap::new();
    let matches: Vec<_> = entry_re.captures_iter(&stripped_bib).collect();
    for (idx, caps) in matches.iter().enumerate() {
        let entry_type = caps[1].to_ascii_lowercase();
        if matches!(entry_type.as_str(), "comment" | "preamble" | "string") {
            continue;
        }
        let key = caps[3].trim().to_string();
        let slice_start = caps.get(0).unwrap().start();
        let slice_end = matches
            .get(idx + 1)
            .map(|m| m.get(0).unwrap().start())
            .unwrap_or(stripped_bib.len());
        let block = &stripped_bib[slice_start..slice_end];

        let base_arxiv_id = eprint_re
            .captures(block)
            .and_then(|c| base_arxiv_id(&c[1]))
            .or_else(|| scan_arxiv_base_ids(block).into_iter().next());
        let doi_lower = doi_re.captures(block).and_then(|c| normalize_doi(&c[1]));
        let note_text = note_re.captures(block).map(|c| c[1].to_ascii_lowercase()).unwrap_or_default();

        out.insert(
            key.clone(),
            BibEntryMeta {
                key,
                base_arxiv_id,
                doi_lower,
                marked_withdrawn: note_text.contains("withdrawn"),
                marked_retracted: note_text.contains("retracted"),
            },
        );
    }
    out
}

pub fn extract_bibitem_keys(stripped_tex: &str) -> BTreeSet<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\\bibitem(?:\s*\[[^\]]*\])?\s*\{([^{}\s]+)\}").unwrap());
    let bytes = stripped_tex.as_bytes();
    re.captures_iter(stripped_tex)
        .filter_map(|c| {
            let full = c.get(0).unwrap();
            is_active_command_backslash(bytes, full.start()).then(|| c[1].trim().to_string())
        })
        .collect()
}

pub fn extract_tex_quote_blocks(stripped_tex: &str) -> Vec<(String, u32)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?s)\\begin\{(quote|quotation)\}(.*?)\\end\{(?:quote|quotation)\}").unwrap()
    });
    let starts = line_starts(stripped_tex);
    re.captures_iter(stripped_tex)
        .filter_map(|c| {
            let m = c.get(0).unwrap();
            let norm = normalize_ws(&c[2]);
            (25..=2000)
                .contains(&norm.chars().count())
                .then(|| (norm, offset_to_line(&starts, m.start())))
        })
        .collect()
}

pub fn normalize_ws(text: &str) -> String {
    let trimmed = text.trim().trim_matches(['"', '\u{201c}', '\u{201d}']);
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim_end().to_string()
}

pub fn normalize_arxiv_id(raw: &str) -> Option<String> {
    static POST2007_RE: OnceLock<Regex> = OnceLock::new();
    static PRE2007_RE: OnceLock<Regex> = OnceLock::new();
    let post = POST2007_RE.get_or_init(|| Regex::new(r"^(\d{4}\.\d{4,5})(?:[vV]([1-9]\d*))?$").unwrap());
    let pre = PRE2007_RE.get_or_init(|| {
        Regex::new(r"^([A-Za-z-]+(?:\.[A-Za-z-]+)?/\d{7})(?:[vV]([1-9]\d*))?$").unwrap()
    });

    let decoded = percent_encoding::percent_decode_str(raw.trim()).decode_utf8_lossy();
    let mut s = decoded.trim().trim_end_matches([',', '.', ';', ')', ']', '}']);
    for prefix in [
        "https://export.arxiv.org/abs/",
        "http://export.arxiv.org/abs/",
        "https://arxiv.org/abs/",
        "http://arxiv.org/abs/",
        "https://export.arxiv.org/pdf/",
        "http://export.arxiv.org/pdf/",
        "https://arxiv.org/pdf/",
        "http://arxiv.org/pdf/",
        "https://export.arxiv.org/e-print/",
        "http://export.arxiv.org/e-print/",
        "https://arxiv.org/e-print/",
        "http://arxiv.org/e-print/",
    ] {
        if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
            s = &s[prefix.len()..];
            break;
        }
    }
    if s.len() >= 6 && s[..6].eq_ignore_ascii_case("arxiv:") {
        s = s[6..].trim_start();
    }
    if s.len() > 4 && s[s.len() - 4..].eq_ignore_ascii_case(".pdf") {
        s = &s[..s.len() - 4];
    }
    if let Some(c) = post.captures(s) {
        return Some(match c.get(2) {
            Some(v) => format!("{}v{}", &c[1], v.as_str()),
            None => c[1].to_string(),
        });
    }
    if let Some(c) = pre.captures(s) {
        return Some(match c.get(2) {
            Some(v) => format!("{}v{}", &c[1], v.as_str()),
            None => c[1].to_string(),
        });
    }
    None
}

/// Strips version suffix `vK` and lowercases (`"2311.00001v2"` -> `"2311.00001"`, `"HEP-TH/9901001v2"` -> `"hep-th/9901001"`).
pub fn base_arxiv_id(raw: &str) -> Option<String> {
    let norm = normalize_arxiv_id(raw)?;
    let base = match norm.rfind('v') {
        Some(idx) if idx > 0 && norm[idx + 1..].bytes().all(|b| b.is_ascii_digit()) => &norm[..idx],
        _ => norm.as_str(),
    };
    Some(base.to_ascii_lowercase())
}

pub fn normalize_doi(raw: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(raw.trim()).decode_utf8_lossy();
    let mut s = decoded.trim().trim_end_matches([',', '.', ';', ')', ']', '}', '"', '\'']);
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
    ] {
        if s.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix)) {
            s = &s[prefix.len()..];
            break;
        }
    }
    if s.get(..4).is_some_and(|head| head.eq_ignore_ascii_case("doi:")) {
        s = s[4..].trim_start();
    }
    let slash = s.find('/')?;
    if !s.starts_with("10.") || slash < 7 || slash + 1 >= s.len() {
        return None;
    }
    if !s[3..slash].bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    Some(s.to_ascii_lowercase())
}

pub fn scan_arxiv_base_ids(text: &str) -> BTreeSet<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)(?:arxiv\s*:\s*|arxiv\.org/(?:abs|pdf|e-print)/)?(\d{4}\.\d{4,5}(?:v[1-9]\d*)?|[a-z-]+(?:\.[a-z-]+)?/\d{7}(?:v[1-9]\d*)?)").unwrap()
    });
    re.captures_iter(text).filter_map(|c| base_arxiv_id(&c[1])).collect()
}

pub fn scan_normalized_dois(text: &str) -> BTreeSet<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?i)\b(10\.\d{4,9}/[^\s,\{\}"'<>\]\)]+)"#).unwrap());
    re.captures_iter(text).filter_map(|c| normalize_doi(&c[1])).collect()
}
