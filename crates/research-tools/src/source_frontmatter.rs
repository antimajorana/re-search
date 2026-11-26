//! Zero-dependency flat front-matter YAML and flat TOML reader for Kandinsky vault notes
//! (`<key>.md`, legacy `info.yaml`) and `.kandinsky/plugin-settings/catalog-papers.toml`
//! (package F13a, `00-overview.md` 2.11, porting rule 3).
//!
//! Supported YAML subset:
//! - Front-matter delimited by leading `---` and closing `---` (or `...`), LF or CRLF
//! - Flat `key: value` pairs with unquoted, single-quoted, or double-quoted scalars
//! - Inline sequences `key: [a, "b", 'c']`
//! - Block sequences (`key:` followed by indented `- item` lines)
//! - Block scalars (`|`, `|-`, `>`, `>-`) for multi-line fields such as `abstractNote`
//! - Full-line `#` comments and blank lines
//!
//! Explicitly NOT supported (recorded in the F13a report):
//! - Nested YAML mappings, flow mappings (`{k: v}`), anchors/aliases (`&a`/`*a`), merge keys
//!   (`<<`), YAML tags (`!!str`), or multi-document streams.

use std::collections::BTreeMap;

/// Value parsed by the zero-dependency flat YAML/TOML reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatVal {
    Scalar(String),
    List(Vec<String>),
}

impl FlatVal {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Scalar(s) => Some(s.as_str()),
            Self::List(_) => None,
        }
    }

    pub fn as_list(&self) -> Vec<String> {
        match self {
            Self::Scalar(s) => {
                let t = s.trim();
                if t.is_empty() {
                    Vec::new()
                } else {
                    vec![t.to_string()]
                }
            }
            Self::List(items) => items.clone(),
        }
    }
}

pub type FlatMap = BTreeMap<String, FlatVal>;

/// Split Markdown frontmatter (`---\n...\n---`) into `(Option<frontmatter>, body)`.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let trimmed = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = if let Some(r) = trimmed.strip_prefix("---\r\n") {
        r
    } else if let Some(r) = trimmed.strip_prefix("---\n") {
        r
    } else {
        return (None, text);
    };

    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let stripped = line.trim_end_matches(['\r', '\n']);
        if stripped == "---" || stripped == "..." {
            let fm = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (Some(fm), body);
        }
        offset += line.len();
    }
    if rest.trim_end_matches(['\r', '\n']) == "---" {
        return (Some(""), "");
    }
    (None, text)
}

/// Parse flat YAML properties (`key: value`, inline/block lists, and `|`/`>` block scalars).
pub fn parse_flat_yaml(yaml: &str) -> FlatMap {
    let mut map = FlatMap::new();
    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let raw_line = lines[i].trim_end_matches('\r');
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        // Only top-level (column 0) keys are parsed in the flat subset.
        if raw_line.starts_with(' ') || raw_line.starts_with('\t') {
            i += 1;
            continue;
        }
        let Some((key_raw, val_raw)) = split_yaml_kv(raw_line) else {
            i += 1;
            continue;
        };
        let key = unquote_scalar(key_raw.trim());
        if key.is_empty() {
            i += 1;
            continue;
        }
        let val_trim = val_raw.trim();

        if matches!(val_trim, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
            let folded = val_trim.starts_with('>');
            let (text, next_i) = parse_block_scalar(&lines, i + 1, folded);
            map.insert(key, FlatVal::Scalar(text));
            i = next_i;
            continue;
        }

        if val_trim.is_empty() {
            let (items, next_i) = parse_block_list(&lines, i + 1);
            if !items.is_empty() {
                map.insert(key, FlatVal::List(items));
            } else {
                map.insert(key, FlatVal::Scalar(String::new()));
            }
            i = next_i;
            continue;
        }

        if val_trim.starts_with('[') && val_trim.ends_with(']') {
            let items = parse_inline_list(&val_trim[1..val_trim.len() - 1]);
            map.insert(key, FlatVal::List(items));
            i += 1;
            continue;
        }

        let scalar = unquote_scalar(strip_inline_comment(val_trim));
        map.insert(key, FlatVal::Scalar(scalar));
        i += 1;
    }
    map
}

fn split_yaml_kv(line: &str) -> Option<(&str, &str)> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_double => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ':' if !in_single && !in_double => {
                let after = &line[idx + 1..];
                if after.is_empty() || after.starts_with(' ') || after.starts_with('\t') {
                    return Some((&line[..idx], after));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_block_scalar(lines: &[&str], start: usize, folded: bool) -> (String, usize) {
    let mut out = Vec::new();
    let mut indent: Option<usize> = None;
    let mut i = start;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line.trim().is_empty() {
            out.push(String::new());
            i += 1;
            continue;
        }
        let spaces = line.bytes().take_while(|b| *b == b' ').count();
        if spaces == 0 {
            break;
        }
        let base = *indent.get_or_insert(spaces);
        let cut = spaces.min(base);
        out.push(line[cut..].to_string());
        i += 1;
    }
    while out.last().is_some_and(|s| s.is_empty()) {
        out.pop();
    }
    let joined = if folded { out.join(" ") } else { out.join("\n") };
    (joined, i)
}

fn parse_block_list(lines: &[&str], start: usize) -> (Vec<String>, usize) {
    let mut items = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let val = unquote_scalar(strip_inline_comment(rest.trim()));
            if !val.is_empty() {
                items.push(val);
            }
            i += 1;
        } else if trimmed == "-" {
            i += 1;
        } else {
            break;
        }
    }
    (items, i)
}

pub(crate) fn parse_inline_list(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_double => {
                cur.push(ch);
                escaped = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                cur.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                cur.push(ch);
            }
            ',' if !in_single && !in_double => {
                let s = unquote_scalar(cur.trim());
                if !s.is_empty() {
                    items.push(s);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let tail = unquote_scalar(cur.trim());
    if !tail.is_empty() {
        items.push(tail);
    }
    items
}

fn strip_inline_comment(raw: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let bytes = raw.as_bytes();
    for (idx, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_double => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && (idx == 0 || bytes[idx - 1].is_ascii_whitespace()) => {
                return raw[..idx].trim_end();
            }
            _ => {}
        }
    }
    raw.trim_end()
}

pub(crate) fn unquote_scalar(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t[1..t.len() - 1].replace("''", "'");
    }
    t.to_string()
}

/// Parse flat `key = "value"` and `key = ["a", "b"]` settings from Kandinsky's
/// `.kandinsky/plugin-settings/catalog-papers.toml` (supports multi-line string arrays).
pub fn parse_flat_toml(toml: &str) -> FlatMap {
    let mut map = FlatMap::new();
    let lines: Vec<&str> = toml.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let line = strip_inline_comment(lines[i].trim());
        if line.is_empty() || line.starts_with('[') {
            i += 1;
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            i += 1;
            continue;
        };
        let key = k.trim().to_string();
        let mut val = v.trim().to_string();
        if val.starts_with('[') && !val.ends_with(']') {
            i += 1;
            while i < lines.len() {
                let next = strip_inline_comment(lines[i].trim());
                val.push(' ');
                val.push_str(next);
                if next.ends_with(']') {
                    break;
                }
                i += 1;
            }
        }
        let vt = val.trim();
        if vt.starts_with('[') && vt.ends_with(']') {
            map.insert(key, FlatVal::List(parse_inline_list(&vt[1..vt.len() - 1])));
        } else {
            map.insert(key, FlatVal::Scalar(unquote_scalar(vt)));
        }
        i += 1;
    }
    map
}

#[cfg(test)]
#[path = "source_frontmatter_tests.rs"]
mod tests;
