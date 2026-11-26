//! Bounded HTML5 named and numeric character reference decoder (`html-1`, V18).

use std::borrow::Cow;

const NAMED_ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{00A0}"),
    ("thinsp", "\u{2009}"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("ndash", "–"),
    ("mdash", "—"),
    ("minus", "−"),
    ("times", "×"),
    ("plusmn", "±"),
    ("pm", "±"),
    ("cdot", "·"),
    ("hellip", "…"),
    ("sect", "§"),
    ("para", "¶"),
    ("copy", "©"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("le", "≤"),
    ("leq", "≤"),
    ("ge", "≥"),
    ("geq", "≥"),
    ("ne", "≠"),
    ("neq", "≠"),
    ("approx", "≈"),
    ("equiv", "≡"),
    ("infin", "∞"),
    ("sum", "∑"),
    ("prod", "∏"),
    ("int", "∫"),
    ("part", "∂"),
    ("nabla", "∇"),
    ("forall", "∀"),
    ("exist", "∃"),
    ("isin", "∈"),
    ("notin", "∉"),
    ("sub", "⊂"),
    ("sube", "⊆"),
    ("cup", "∪"),
    ("cap", "∩"),
    ("rarr", "→"),
    ("larr", "←"),
    ("harr", "↔"),
    ("rArr", "⇒"),
    ("hArr", "⇔"),
];

/// Decodes common HTML5 named and numeric character references (`&amp;`, `&#...;`, `&#x...;`).
pub(crate) fn decode_html_entities(input: &str) -> Cow<'_, str> {
    if !input.contains('&') {
        return Cow::Borrowed(input);
    }
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((decoded, len)) = try_decode_entity(&input[i..]) {
                out.push_str(&decoded);
                i += len;
                continue;
            }
        }
        let step = input[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&input[i..i + step]);
        i += step;
    }
    Cow::Owned(out)
}

fn try_decode_entity(slice: &str) -> Option<(String, usize)> {
    let semi = slice[1..].find(';')?;
    if semi == 0 || semi > 16 {
        return None;
    }
    let body = &slice[1..1 + semi];
    let total_len = semi + 2;
    if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let code = u32::from_str_radix(hex, 16).ok()?;
        return Some((sanitize_numeric_char(code).to_string(), total_len));
    }
    if let Some(dec) = body.strip_prefix('#') {
        if dec.is_empty() || !dec.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let code = dec.parse::<u32>().ok()?;
        return Some((sanitize_numeric_char(code).to_string(), total_len));
    }
    NAMED_ENTITIES
        .iter()
        .find(|&&(k, _)| k == body)
        .map(|&(_, v)| (v.to_string(), total_len))
}

fn sanitize_numeric_char(code: u32) -> char {
    if code == 0 || code > 0x10FFFF {
        return '\u{FFFD}';
    }
    char::from_u32(code).unwrap_or('\u{FFFD}')
}
