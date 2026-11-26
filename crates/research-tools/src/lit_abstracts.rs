//! Literature abstract normalization, JATS/XML stripping, and rule-C3 quote verification (package F11).
//!
//! Enforces `explore.md` section 4.1 (`MAX_ABSTRACT_CHARS = 3000`, `AbstractRef`) and rule C3:
//! an abstract quote is accepted as evidence only when it is a whitespace-normalized substring
//! of a verbatim abstract parsed from a stored raw response blob. Rebuilt (non-verbatim)
//! abstracts can be displayed and triaged, but can never be evidence.

use crate::investigation::anchor::fold_with_map;
use crate::lit_providers::{parse_provider_response, LitProvider, QueryRelation};
use crate::paper_key::{PaperKey, PaperKeyKind, PaperRef};
use crate::quote_core::{clip_to_chars, quote_sha256};
use serde::{Deserialize, Serialize};

/// Maximum stored abstract length in Unicode scalar values (`explore.md` section 4.1).
pub const MAX_ABSTRACT_CHARS: usize = 3_000;

/// Provenance of a paper abstract (`explore.md` section 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractRef {
    pub raw_blob: String,
    pub provider: String,
    pub verbatim: bool,
}

/// Exact location of a matched quote inside a stored verbatim abstract (`explore.md` rule C3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractQuoteMatch {
    /// Inclusive UTF-8 byte offset in the stored abstract.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the stored abstract.
    pub end: usize,
    /// Exact slice `&abstract_text[start..end]`.
    pub matched_text: String,
    /// Canonical folded quote SHA-256 matching [`crate::quote_core::quote_sha256`].
    pub quote_sha256: String,
}

/// Abstract extracted from a stored raw provider response blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAbstract {
    pub text: String,
    pub verbatim: bool,
}

/// Collapse all whitespace runs (including newlines and tabs) to a single ASCII space and trim ends.
pub fn normalize_whitespace(text: &str) -> String {
    fold_with_map(text).0
}

/// Cap `text` to at most [`MAX_ABSTRACT_CHARS`] (3000) Unicode scalar values on a UTF-8 boundary.
pub fn cap_abstract(text: &str) -> String {
    clip_to_chars(text, MAX_ABSTRACT_CHARS).to_string()
}

/// Normalize a raw provider abstract: optionally strip JATS/XML tags and unescape XML entities,
/// normalize whitespace, and cap at [`MAX_ABSTRACT_CHARS`]. Returns `None` if empty after cleaning.
pub fn normalize_abstract(raw: &str, strip_xml: bool) -> Option<String> {
    let cleaned = if strip_xml {
        unescape_xml_entities(&strip_jats_xml(raw))
    } else {
        raw.to_string()
    };
    let folded = normalize_whitespace(&cleaned);
    if folded.is_empty() {
        return None;
    }
    Some(cap_abstract(&folded))
}

/// Strip JATS/XML tags from a Crossref abstract while preserving plain-text `<` inequalities
/// (such as `x < 1` or `a < b > c`) and inserting spaces across block-level tag boundaries.
pub fn strip_jats_xml(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0usize;

    while i < len {
        if bytes[i] != b'<' {
            let next_lt = raw[i..].find('<').map(|off| i + off).unwrap_or(len);
            out.push_str(&raw[i..next_lt]);
            i = next_lt;
            continue;
        }
        if raw[i..].starts_with("<!--") {
            if let Some(end_rel) = raw[i + 4..].find("-->") {
                i = i + 4 + end_rel + 3;
                continue;
            }
        }
        if raw[i..].starts_with("<![CDATA[") {
            if let Some(end_rel) = raw[i + 9..].find("]]>") {
                out.push_str(&raw[i + 9..i + 9 + end_rel]);
                i = i + 9 + end_rel + 3;
                continue;
            }
        }
        if let Some((tag_name, end_idx)) = parse_xml_tag(raw, i) {
            if is_block_tag(tag_name) && !out.is_empty() && !out.ends_with(char::is_whitespace) {
                out.push(' ');
            }
            i = end_idx;
            continue;
        }
        out.push('<');
        i += 1;
    }
    out
}

/// Parse an opening, closing, or self-closing XML tag starting at `start` (`raw[start] == '<'`).
/// Returns `(tag_name, byte_index_after_gt)` if valid, or `None` for plain-text `<` characters.
fn parse_xml_tag(raw: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut i = start + 1;
    if i >= len {
        return None;
    }
    let closing = bytes[i] == b'/';
    if closing {
        i += 1;
    }
    if i >= len || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    let name_start = i;
    while i < len
        && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-' | b'.' | b':'))
    {
        i += 1;
    }
    let tag_name = &raw[name_start..i];
    if i >= len {
        return None;
    }
    // After the tag name must come '>', '/', or whitespace (not e.g. `x<y+z>`).
    if closing {
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        return (i < len && bytes[i] == b'>').then_some((tag_name, i + 1));
    }
    if !matches!(bytes[i], b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r') {
        return None;
    }
    let mut quote: Option<u8> = None;
    while i < len {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some((tag_name, i + 1)),
                b'<' => return None, // unclosed tag before next '<', treat as plain text
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn is_block_tag(name: &str) -> bool {
    let local = name.rsplit(':').next().unwrap_or(name);
    matches!(
        local.to_ascii_lowercase().as_str(),
        "p" | "sec"
            | "title"
            | "list-item"
            | "list"
            | "br"
            | "div"
            | "abstract"
            | "caption"
            | "tr"
            | "td"
            | "th"
            | "disp-formula"
            | "blockquote"
    )
}

/// Unescape common XML, numeric (`&#NNN;`, `&#xHHH;`), and physics JATS character entities.
/// Unknown entities and bare `&` are preserved untouched.
pub fn unescape_xml_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        if let Some(semi) = after.find(';') {
            let token = &after[..semi];
            if token.len() <= 16 {
                if let Some(decoded) = decode_entity(token) {
                    out.push(decoded);
                    rest = &after[semi + 1..];
                    continue;
                }
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn decode_entity(token: &str) -> Option<char> {
    if let Some(hex) = token.strip_prefix("#x").or_else(|| token.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16).ok()?;
        return (code != 0).then(|| char::from_u32(code)).flatten();
    }
    if let Some(dec) = token.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        return (code != 0).then(|| char::from_u32(code)).flatten();
    }
    match token {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "minus" => Some('\u{2212}'),
        "times" => Some('\u{00D7}'),
        "plusmn" => Some('\u{00B1}'),
        "le" => Some('\u{2264}'),
        "ge" => Some('\u{2265}'),
        "ne" => Some('\u{2260}'),
        "approx" => Some('\u{2248}'),
        "infin" | "infty" => Some('\u{221E}'),
        "alpha" => Some('\u{03B1}'),
        "beta" => Some('\u{03B2}'),
        "gamma" => Some('\u{03B3}'),
        "delta" => Some('\u{03B4}'),
        "epsilon" => Some('\u{03B5}'),
        "lambda" => Some('\u{03BB}'),
        "mu" => Some('\u{03BC}'),
        "nu" => Some('\u{03BD}'),
        "pi" => Some('\u{03C0}'),
        "sigma" => Some('\u{03C3}'),
        "tau" => Some('\u{03C4}'),
        "phi" => Some('\u{03C6}'),
        "psi" => Some('\u{03C8}'),
        "omega" => Some('\u{03C9}'),
        "Gamma" => Some('\u{0393}'),
        "Delta" => Some('\u{0394}'),
        "Lambda" => Some('\u{039B}'),
        "Pi" => Some('\u{03A0}'),
        "Sigma" => Some('\u{03A3}'),
        "Phi" => Some('\u{03A6}'),
        "Psi" => Some('\u{03A8}'),
        "Omega" => Some('\u{03A9}'),
        _ => None,
    }
}

/// Verify and locate `quote` inside `abstract_text` (`explore.md` rule C3).
///
/// Rejects non-verbatim (rebuilt-from-index) abstracts unconditionally. Matches whitespace-folded
/// substrings and returns exact UTF-8 byte offsets `(start, end)` in `abstract_text`.
pub fn locate_abstract_quote(
    abstract_text: &str,
    quote: &str,
    verbatim: bool,
) -> Result<AbstractQuoteMatch, String> {
    if !verbatim {
        return Err(
            "non-verbatim (rebuilt) abstract cannot be used as evidence (explore.md rule C3)"
                .into(),
        );
    }
    let (folded_quote, _) = fold_with_map(quote);
    if folded_quote.is_empty() {
        return Err("abstract quote must not be empty".into());
    }
    let (folded_abs, orig_byte) = fold_with_map(abstract_text);
    if folded_abs.is_empty() {
        return Err("stored abstract is empty".into());
    }
    let pos = folded_abs
        .find(&folded_quote)
        .ok_or_else(|| "quote not found in stored verbatim abstract".to_string())?;
    let end_pos = pos + folded_quote.len();
    let start = orig_byte[pos];
    let end = orig_byte[end_pos - 1] + 1;
    let matched_text = abstract_text
        .get(start..end)
        .ok_or_else(|| "internal offset alignment error in abstract".to_string())?
        .to_string();
    Ok(AbstractQuoteMatch {
        start,
        end,
        matched_text,
        quote_sha256: quote_sha256(quote),
    })
}

/// Check whether `paper_ref` identifies `key`, either as its primary key or as one of its aliases.
pub fn paper_matches_key(paper_ref: &PaperRef, key: &PaperKey) -> bool {
    if &paper_ref.key == key {
        return true;
    }
    match key.kind() {
        PaperKeyKind::Arxiv => paper_ref.ids.arxiv.as_deref() == Some(key.value()),
        PaperKeyKind::Doi => paper_ref.ids.doi.as_deref() == Some(key.value()),
        PaperKeyKind::Inspire => paper_ref.ids.inspire.as_deref() == Some(key.value()),
        PaperKeyKind::S2 => paper_ref.ids.s2.as_deref() == Some(key.value()),
        PaperKeyKind::Openalex | PaperKeyKind::File => false,
    }
}

/// Parse a stored raw provider response blob and extract the normalized abstract for `paper_key`.
///
/// Supports `explore.md` rule C3 ("parsed from a stored raw response").
pub fn extract_abstract_from_blob(
    provider: LitProvider,
    raw_bytes: &[u8],
    paper_key: &PaperKey,
) -> Result<ExtractedAbstract, String> {
    let parsed = parse_provider_response(provider, raw_bytes, &QueryRelation::None, 0, None)?;
    let paper = parsed
        .papers
        .iter()
        .find(|p| p.paper_ref.key == *paper_key)
        .or_else(|| parsed.papers.iter().find(|p| paper_matches_key(&p.paper_ref, paper_key)))
        .ok_or_else(|| {
            format!("paper `{paper_key}` not found in raw `{provider}` response blob")
        })?;
    let text = paper
        .abstract_text
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("paper `{paper_key}` has no abstract in `{provider}` response blob"))?;
    let verbatim = paper.abstract_ref.as_ref().is_none_or(|r| r.verbatim);
    Ok(ExtractedAbstract {
        text: text.to_string(),
        verbatim,
    })
}

/// End-to-end rule-C3 helper: parse `raw_bytes` for `paper_key` and locate `quote` inside its
/// verbatim abstract.
pub fn locate_abstract_quote_in_blob(
    provider: LitProvider,
    raw_bytes: &[u8],
    paper_key: &PaperKey,
    quote: &str,
) -> Result<AbstractQuoteMatch, String> {
    let extracted = extract_abstract_from_blob(provider, raw_bytes, paper_key)?;
    locate_abstract_quote(&extracted.text, quote, extracted.verbatim)
}

#[cfg(test)]
#[path = "lit_abstracts_tests.rs"]
mod tests;
