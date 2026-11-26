//! Deterministic pure-Rust BibTeX parser, TeX accent decoder, author splitter, and
//! `file` field parser for [`crate::source_bibtex::BibtexAdapter`] (package F13b,
//! `00-overview.md` 2.11).
//!
//! Zero external dependencies; handles `@string` macros, `#` concatenation, nested `{...}`
//! and `"..."` values, TeX diacritics, and attachment formats from Zotero Better BibTeX,
//! JabRef, and Kandinsky `refs.bib`.

use std::collections::BTreeMap;

/// Raw parsed BibTeX entry before domain normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibEntryRaw {
    pub entry_type: String,
    pub citekey: String,
    pub fields: BTreeMap<String, String>,
}

/// One attachment record parsed from a BibTeX `file` (or `pdf` / `local-url`) field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibFileField {
    pub description: String,
    pub path: String,
    pub mime_or_type: String,
}

/// Parse a `.bib` text buffer into raw entries and non-fatal parse warnings.
pub fn parse_bibtex(input: &str) -> (Vec<BibEntryRaw>, Vec<String>) {
    let mut parser = BibParser::new(input);
    parser.parse_all()
}

struct BibParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    strings: BTreeMap<String, String>,
    warnings: Vec<String>,
}

impl<'a> BibParser<'a> {
    fn new(src: &'a str) -> Self {
        let mut strings = BTreeMap::new();
        for (k, v) in [
            ("jan", "January"),
            ("feb", "February"),
            ("mar", "March"),
            ("apr", "April"),
            ("may", "May"),
            ("jun", "June"),
            ("jul", "July"),
            ("aug", "August"),
            ("sep", "September"),
            ("oct", "October"),
            ("nov", "November"),
            ("dec", "December"),
        ] {
            strings.insert(k.to_string(), v.to_string());
        }
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            strings,
            warnings: Vec::new(),
        }
    }

    fn parse_all(&mut self) -> (Vec<BibEntryRaw>, Vec<String>) {
        let mut entries = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'@' {
                self.pos += 1;
                if let Some(entry) = self.parse_at_construct() {
                    entries.push(entry);
                }
            } else if b == b'%' {
                self.skip_to_eol();
            } else {
                self.pos += 1;
            }
        }
        (entries, std::mem::take(&mut self.warnings))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_to_eol(&mut self) {
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'\n' {
                break;
            }
        }
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else if b == b'%' {
                self.skip_to_eol();
            } else {
                break;
            }
        }
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.' | b'+' | b'/') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    fn parse_at_construct(&mut self) -> Option<BibEntryRaw> {
        self.skip_ws_and_comments();
        let raw_type = self.read_ident();
        if raw_type.is_empty() {
            return None;
        }
        let kind = raw_type.to_ascii_lowercase();
        self.skip_ws_and_comments();
        let open = self.peek()?;
        let close = match open {
            b'{' => b'}',
            b'(' => b')',
            _ => return None,
        };
        self.pos += 1;

        match kind.as_str() {
            "comment" | "preamble" => {
                self.skip_balanced(open, close);
                None
            }
            "string" => {
                self.parse_string_macro(close);
                None
            }
            _ => self.parse_entry(kind, close),
        }
    }

    fn skip_balanced(&mut self, open: u8, close: u8) {
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'\\' && self.peek().is_some() {
                self.pos += 1;
                continue;
            }
            if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    fn parse_string_macro(&mut self, close: u8) {
        self.skip_ws_and_comments();
        let name = self.read_ident().to_ascii_lowercase();
        self.skip_ws_and_comments();
        if self.peek() == Some(b'=') {
            self.pos += 1;
            if let Some(val) = self.parse_value_expr(close) {
                if !name.is_empty() {
                    self.strings.insert(name, val);
                }
            }
        }
        self.skip_until_close(close);
    }

    fn skip_until_close(&mut self, close: u8) {
        let mut brace_depth = 0usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'{' {
                brace_depth += 1;
            } else if b == b'}' && brace_depth > 0 {
                brace_depth -= 1;
            } else if b == close && brace_depth == 0 {
                return;
            }
        }
    }

    fn parse_entry(&mut self, entry_type: String, close: u8) -> Option<BibEntryRaw> {
        self.skip_ws_and_comments();
        let key_start = self.pos;
        while let Some(b) = self.peek() {
            if b == b',' || b == close || b.is_ascii_whitespace() {
                break;
            }
            self.pos += 1;
        }
        let citekey = self.src[key_start..self.pos].trim().to_string();
        self.skip_ws_and_comments();
        if self.peek() == Some(b',') {
            self.pos += 1;
        } else if self.peek() == Some(close) {
            self.pos += 1;
            if citekey.is_empty() {
                return None;
            }
            return Some(BibEntryRaw {
                entry_type,
                citekey,
                fields: BTreeMap::new(),
            });
        } else {
            self.warnings
                .push(format!("malformed BibTeX entry header near `{citekey}`"));
            self.skip_until_close(close);
            return None;
        }

        let mut fields = BTreeMap::new();
        loop {
            self.skip_ws_and_comments();
            match self.peek() {
                None => {
                    self.warnings
                        .push(format!("unterminated BibTeX entry `{citekey}`"));
                    break;
                }
                Some(b) if b == close => {
                    self.pos += 1;
                    break;
                }
                Some(b',') => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            let fname = self.read_ident().to_ascii_lowercase();
            if fname.is_empty() {
                self.warnings.push(format!(
                    "unexpected token in BibTeX entry `{citekey}`; skipping remainder"
                ));
                self.skip_until_close(close);
                break;
            }
            self.skip_ws_and_comments();
            if self.peek() != Some(b'=') {
                self.warnings.push(format!(
                    "missing `=` after field `{fname}` in BibTeX entry `{citekey}`"
                ));
                self.skip_until_close(close);
                break;
            }
            self.pos += 1;
            if let Some(val) = self.parse_value_expr(close) {
                fields.insert(fname, val);
            }
            self.skip_ws_and_comments();
            if self.peek() == Some(b',') {
                self.pos += 1;
            }
        }
        if citekey.is_empty() {
            return None;
        }
        Some(BibEntryRaw {
            entry_type,
            citekey,
            fields,
        })
    }

    fn parse_value_expr(&mut self, close: u8) -> Option<String> {
        let mut out = String::new();
        let mut got_any = false;
        loop {
            self.skip_ws_and_comments();
            let part = match self.peek()? {
                b'{' => self.parse_braced_value(),
                b'"' => self.parse_quoted_value(),
                b if b == b',' || b == close => break,
                _ => self.parse_bare_token(close),
            };
            if let Some(p) = part {
                out.push_str(&p);
                got_any = true;
            } else {
                break;
            }
            self.skip_ws_and_comments();
            if self.peek() == Some(b'#') {
                self.pos += 1;
            } else {
                break;
            }
        }
        got_any.then_some(out)
    }

    fn parse_braced_value(&mut self) -> Option<String> {
        if self.peek() != Some(b'{') {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            if b == b'\\' && self.pos + 1 < self.bytes.len() {
                self.pos += 2;
                continue;
            }
            if b == b'{' {
                depth += 1;
                self.pos += 1;
            } else if b == b'}' {
                depth -= 1;
                if depth == 0 {
                    let end = self.pos;
                    self.pos += 1;
                    return Some(self.src[start..end].to_string());
                }
                self.pos += 1;
            } else {
                self.pos += 1;
            }
        }
        Some(self.src[start..self.pos].to_string())
    }

    fn parse_quoted_value(&mut self) -> Option<String> {
        if self.peek() != Some(b'"') {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        let mut brace_depth = 0usize;
        while let Some(b) = self.peek() {
            if b == b'\\' && self.pos + 1 < self.bytes.len() {
                self.pos += 2;
                continue;
            }
            if b == b'{' {
                brace_depth += 1;
                self.pos += 1;
            } else if b == b'}' && brace_depth > 0 {
                brace_depth -= 1;
                self.pos += 1;
            } else if b == b'"' && brace_depth == 0 {
                let end = self.pos;
                self.pos += 1;
                return Some(self.src[start..end].to_string());
            } else {
                self.pos += 1;
            }
        }
        Some(self.src[start..self.pos].to_string())
    }

    fn parse_bare_token(&mut self, close: u8) -> Option<String> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || b == b',' || b == b'#' || b == close {
                break;
            }
            self.pos += 1;
        }
        let tok = self.src[start..self.pos].trim();
        if tok.is_empty() {
            return None;
        }
        let lower = tok.to_ascii_lowercase();
        if let Some(expanded) = self.strings.get(&lower) {
            Some(expanded.clone())
        } else {
            Some(tok.to_string())
        }
    }
}

pub use crate::source_bibtex_tex::{decode_tex_text, split_bibtex_authors};

/// Parse a BibTeX `file` field into attachment records.
///
/// Supports:
/// - Zotero Better BibTeX (`Description:/path/to/file.pdf:application/pdf`, `;`-separated,
///   escaped `\:`, `\;`, `\\`, and Windows drive prefixes `C\:/...` or `C:/...`).
/// - JabRef (`:rel/path.pdf:PDF` or `Desc:rel/path.pdf:PDF`).
/// - Kandinsky `refs.bib` / plain paths (`refs/papers/key/key.pdf` or `../Shared/foo.pdf`).
pub fn parse_bib_file_field(raw: &str) -> Vec<BibFileField> {
    let mut out = Vec::new();
    for segment in split_unescaped(raw.trim(), ';') {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        let cols = split_colon_preserving_windows_drive(seg);
        let (description, path, mime_or_type) = match cols.as_slice() {
            [only] => (String::new(), unescape_bib_file_part(only), String::new()),
            [first, second] => {
                let a = unescape_bib_file_part(first);
                let b = unescape_bib_file_part(second);
                if looks_like_mime_or_type(&b) {
                    (String::new(), a, b)
                } else {
                    (a, b, String::new())
                }
            }
            [first, second, third] => (
                unescape_bib_file_part(first),
                unescape_bib_file_part(second),
                unescape_bib_file_part(third),
            ),
            many => {
                let desc = unescape_bib_file_part(&many[0]);
                let mime = unescape_bib_file_part(many.last().unwrap_or(&String::new()));
                let mid = many[1..many.len() - 1]
                    .iter()
                    .map(|p| unescape_bib_file_part(p))
                    .collect::<Vec<_>>()
                    .join(":");
                (desc, mid, mime)
            }
        };
        let clean_path = path.trim().to_string();
        if !clean_path.is_empty() {
            out.push(BibFileField {
                description: description.trim().to_string(),
                path: clean_path,
                mime_or_type: mime_or_type.trim().to_string(),
            });
        }
    }
    out
}

fn split_unescaped(input: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == delim || next == '\\' || next == ':' {
                    cur.push(c);
                    cur.push(chars.next().unwrap());
                    continue;
                }
            }
            cur.push(c);
        } else if c == delim {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn split_colon_preserving_windows_drive(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], ':' | ';' | '\\') {
            cur.push(c);
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == ':' {
            let is_win_drive = cur.len() == 1
                && cur.chars().all(|ch| ch.is_ascii_alphabetic())
                && chars.get(i + 1).is_some_and(|n| matches!(*n, '/' | '\\'));
            if is_win_drive {
                cur.push(':');
                i += 1;
                continue;
            }
            out.push(std::mem::take(&mut cur));
            i += 1;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out
}

fn unescape_bib_file_part(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if matches!(next, ':' | ';') {
                    out.push(chars.next().unwrap());
                    continue;
                }
                if next == '\\' {
                    chars.next();
                    out.push('\\');
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

fn looks_like_mime_or_type(s: &str) -> bool {
    let low = s.trim().to_ascii_lowercase();
    matches!(
        low.as_str(),
        "pdf" | "tex" | "application/pdf" | "text/x-tex" | "application/x-tex" | "application/zip"
    ) || low.contains('/')
}
