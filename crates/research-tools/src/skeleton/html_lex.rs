//! Bounded pure-Rust HTML5 tag, attribute, comment, raw-text, and entity lexer
//! (Investigate V18, Decision 12: zero external HTML parser dependencies).
//!
//! Tracks exact 1-based physical source line numbers compatible with
//! `investigation_anchor::VerifiedFile` and `quote_core::verify_quote`.

use super::html_entities::decode_html_entities;

pub const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_HTML_TOKENS: usize = 500_000;
pub const MAX_HTML_ATTRS_PER_TAG: usize = 128;

/// Standard HTML5 void elements that never require a closing tag or push onto the stack.
const VOID_HTML_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "menuitem",
    "meta", "param", "source", "track", "wbr",
];

pub(crate) fn is_void_html_tag(name_lower: &str) -> bool {
    VOID_HTML_TAGS.contains(&name_lower)
}

/// Physical 1-based line-start index over a UTF-8 HTML document.
#[derive(Debug, Clone)]
pub(crate) struct LineIndex {
    line_starts: Vec<usize>,
    total_bytes: usize,
}

impl LineIndex {
    pub(crate) fn new(src: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' && i + 1 < src.len() {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts, total_bytes: src.len() }
    }

    pub(crate) fn line_of_byte(&self, byte: usize) -> usize {
        if self.line_starts.is_empty() {
            return 1;
        }
        let clamped = byte.min(self.total_bytes);
        self.line_starts.partition_point(|&s| s <= clamped).max(1)
    }

    pub(crate) fn total_lines(&self) -> usize {
        self.line_starts.len().max(1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HtmlToken {
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
        is_void: bool,
        start_line: usize,
        end_line: usize,
    },
    EndTag {
        name: String,
        end_line: usize,
    },
    Text {
        decoded: String,
        start_line: usize,
        end_line: usize,
    },
}

impl HtmlToken {
    pub(crate) fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
        attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LexOutput {
    pub(crate) tokens: Vec<HtmlToken>,
    pub(crate) line_index: LineIndex,
    pub(crate) warnings: Vec<String>,
}

/// Scans `html` into bounded [`HtmlToken`]s with 1-based physical line coordinates.
///
/// - Skips HTML comments, `<!DOCTYPE ...>`, processing instructions, and `<script>` / `<style>`
///   raw-text bodies so embedded JS/CSS never produces synthetic tags or prose.
/// - Handles `<title>` in RCDATA mode (entity-decoded text until `</title>`, no inner tags).
/// - Treats unescaped `<` not starting a tag (common in inline math like `$a < b$`) as text.
pub(crate) fn scan_html_tokens(html: &str) -> LexOutput {
    let mut warnings = Vec::new();
    let scan_slice = if html.len() > MAX_HTML_BYTES {
        warnings.push(format!(
            "HTML input ({} bytes) exceeded {MAX_HTML_BYTES}-byte scan cap; truncated",
            html.len()
        ));
        let mut cut = MAX_HTML_BYTES;
        while cut > 0 && !html.is_char_boundary(cut) {
            cut -= 1;
        }
        &html[..cut]
    } else {
        html
    };

    let line_index = LineIndex::new(scan_slice);
    let bytes = scan_slice.as_bytes();
    let mut tokens = Vec::new();
    let mut prev_text_end: Option<usize> = None;
    let mut pos = 0usize;

    while pos < bytes.len() {
        if tokens.len() >= MAX_HTML_TOKENS {
            warnings.push(format!("HTML token cap ({MAX_HTML_TOKENS}) reached; truncated"));
            break;
        }

        let Some(rel_lt) = scan_slice[pos..].find('<') else {
            push_text_token(
                scan_slice,
                pos,
                bytes.len(),
                &line_index,
                &mut tokens,
                &mut prev_text_end,
            );
            break;
        };
        let lt_pos = pos + rel_lt;
        if lt_pos > pos {
            push_text_token(
                scan_slice,
                pos,
                lt_pos,
                &line_index,
                &mut tokens,
                &mut prev_text_end,
            );
            if tokens.len() >= MAX_HTML_TOKENS {
                warnings.push(format!("HTML token cap ({MAX_HTML_TOKENS}) reached; truncated"));
                break;
            }
        }

        if scan_slice[lt_pos..].starts_with("<!--") {
            prev_text_end = None;
            let after = lt_pos + 4;
            if let Some(end_rel) = scan_slice[after..].find("-->") {
                pos = after + end_rel + 3;
            } else {
                warnings.push(format!(
                    "unclosed HTML comment at line {}",
                    line_index.line_of_byte(lt_pos)
                ));
                break;
            }
            continue;
        }

        if scan_slice[lt_pos..].starts_with("<![CDATA[") {
            prev_text_end = None;
            let start = lt_pos + 9;
            if let Some(end_rel) = scan_slice[start..].find("]]>") {
                let end = start + end_rel;
                if end > start {
                    tokens.push(HtmlToken::Text {
                        decoded: scan_slice[start..end].to_string(),
                        start_line: line_index.line_of_byte(start),
                        end_line: line_index.line_of_byte(end.saturating_sub(1).max(start)),
                    });
                }
                pos = end + 3;
            } else {
                warnings.push(format!(
                    "unclosed CDATA section at line {}",
                    line_index.line_of_byte(lt_pos)
                ));
                break;
            }
            continue;
        }

        if bytes.get(lt_pos + 1).is_some_and(|&b| b == b'!' || b == b'?') {
            prev_text_end = None;
            pos = scan_slice[lt_pos + 2..]
                .find('>')
                .map_or(bytes.len(), |gt| lt_pos + 2 + gt + 1);
            continue;
        }

        if bytes.get(lt_pos + 1) == Some(&b'/') {
            prev_text_end = None;
            if bytes.get(lt_pos + 2).is_some_and(u8::is_ascii_alphabetic) {
                let (name, end_pos) = parse_end_tag(scan_slice, lt_pos);
                let end_line = line_index.line_of_byte(end_pos.saturating_sub(1).max(lt_pos));
                tokens.push(HtmlToken::EndTag { name, end_line });
                pos = end_pos;
            } else {
                pos = scan_slice[lt_pos + 2..]
                    .find('>')
                    .map_or(bytes.len(), |gt| lt_pos + 2 + gt + 1);
            }
            continue;
        }

        if bytes.get(lt_pos + 1).is_some_and(u8::is_ascii_alphabetic) {
            prev_text_end = None;
            let (name, attrs, self_closing, end_pos) =
                parse_start_tag(scan_slice, lt_pos, &mut warnings, &line_index);
            let is_void = is_void_html_tag(&name);
            let start_line = line_index.line_of_byte(lt_pos);
            let end_line = line_index.line_of_byte(end_pos.saturating_sub(1).max(lt_pos));
            let is_raw = !self_closing && matches!(name.as_str(), "script" | "style");
            let is_rcdata = !self_closing && name == "title";

            tokens.push(HtmlToken::StartTag {
                name: name.clone(),
                attrs,
                self_closing,
                is_void,
                start_line,
                end_line,
            });
            pos = end_pos;

            if is_raw {
                pos = skip_raw_text_element(scan_slice, pos, &name, &mut warnings, &line_index);
                let close_line = line_index.line_of_byte(pos.saturating_sub(1).max(end_pos));
                tokens.push(HtmlToken::EndTag { name, end_line: close_line });
            } else if is_rcdata {
                let (inner, next_pos) = read_rcdata_title(scan_slice, pos);
                if !inner.is_empty() {
                    let t_end = next_pos.min(bytes.len());
                    tokens.push(HtmlToken::Text {
                        decoded: decode_html_entities(inner).into_owned(),
                        start_line: line_index.line_of_byte(pos),
                        end_line: line_index.line_of_byte(t_end.saturating_sub(1).max(pos)),
                    });
                }
                let close_line = line_index.line_of_byte(next_pos.saturating_sub(1).max(pos));
                tokens.push(HtmlToken::EndTag { name, end_line: close_line });
                pos = next_pos;
            }
            continue;
        }

        // Literal `<` in prose/math (e.g. `$a < b$`) — emit as text and advance past `<`.
        push_text_token(
            scan_slice,
            lt_pos,
            lt_pos + 1,
            &line_index,
            &mut tokens,
            &mut prev_text_end,
        );
        pos = lt_pos + 1;
    }

    LexOutput { tokens, line_index, warnings }
}

fn push_text_token(
    src: &str,
    start_byte: usize,
    end_byte: usize,
    line_index: &LineIndex,
    out: &mut Vec<HtmlToken>,
    prev_text_end: &mut Option<usize>,
) {
    if end_byte <= start_byte {
        return;
    }
    let decoded = decode_html_entities(&src[start_byte..end_byte]).into_owned();
    let start_line = line_index.line_of_byte(start_byte);
    let end_line = line_index.line_of_byte(end_byte.saturating_sub(1).max(start_byte));
    if *prev_text_end == Some(start_byte) {
        if let Some(HtmlToken::Text { decoded: prev, end_line: prev_line, .. }) = out.last_mut() {
            prev.push_str(&decoded);
            *prev_line = end_line;
            *prev_text_end = Some(end_byte);
            return;
        }
    }
    *prev_text_end = Some(end_byte);
    out.push(HtmlToken::Text { decoded, start_line, end_line });
}

fn is_tag_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':')
}

fn parse_end_tag(src: &str, lt_pos: usize) -> (String, usize) {
    let bytes = src.as_bytes();
    let mut i = lt_pos + 2;
    let name_start = i;
    while i < bytes.len() && is_tag_name_byte(bytes[i]) {
        i += 1;
    }
    let name = src[name_start..i].to_ascii_lowercase();
    while i < bytes.len() && bytes[i] != b'>' {
        i += 1;
    }
    let end_pos = if bytes.get(i) == Some(&b'>') { i + 1 } else { bytes.len() };
    (name, end_pos)
}

fn parse_start_tag(
    src: &str,
    lt_pos: usize,
    warnings: &mut Vec<String>,
    line_index: &LineIndex,
) -> (String, Vec<(String, String)>, bool, usize) {
    let bytes = src.as_bytes();
    let mut i = lt_pos + 1;
    let name_start = i;
    while i < bytes.len() && is_tag_name_byte(bytes[i]) {
        i += 1;
    }
    let name = src[name_start..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    let mut attr_cap_noted = false;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'>' {
            return (name, attrs, self_closing, i + 1);
        }
        if bytes[i] == b'/' {
            i += 1;
            if bytes.get(i) == Some(&b'>') {
                self_closing = true;
                return (name, attrs, self_closing, i + 1);
            }
            continue;
        }

        let attr_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
        {
            i = next_char_boundary(src, i);
        }
        if i == attr_start {
            i = next_char_boundary(src, i);
            continue;
        }
        let attr_name = src[attr_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        let mut attr_val = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if let Some(&q @ (b'"' | b'\'')) = bytes.get(i) {
                i += 1;
                let val_start = i;
                while i < bytes.len() && bytes[i] != q {
                    i = next_char_boundary(src, i);
                }
                attr_val = decode_html_entities(&src[val_start..i]).into_owned();
                if bytes.get(i) == Some(&q) {
                    i += 1;
                } else {
                    warnings.push(format!(
                        "unclosed attribute quote in <{name}> at line {}",
                        line_index.line_of_byte(lt_pos)
                    ));
                }
            } else {
                let val_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i = next_char_boundary(src, i);
                }
                attr_val = decode_html_entities(&src[val_start..i]).into_owned();
            }
        }

        if attrs.len() < MAX_HTML_ATTRS_PER_TAG {
            attrs.push((attr_name, attr_val));
        } else if !attr_cap_noted {
            attr_cap_noted = true;
            warnings.push(format!(
                "attribute count on <{name}> exceeded {MAX_HTML_ATTRS_PER_TAG} at line {}",
                line_index.line_of_byte(lt_pos)
            ));
        }
    }

    warnings.push(format!(
        "unclosed start tag <{name}> at line {}",
        line_index.line_of_byte(lt_pos)
    ));
    (name, attrs, self_closing, bytes.len())
}

fn skip_raw_text_element(
    src: &str,
    start: usize,
    tag_lower: &str,
    warnings: &mut Vec<String>,
    line_index: &LineIndex,
) -> usize {
    let bytes = src.as_bytes();
    let tag_bytes = tag_lower.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        let Some(rel) = src[pos..].find("</") else { break };
        let name_start = pos + rel + 2;
        let name_end = name_start + tag_bytes.len();
        if name_end <= bytes.len()
            && bytes[name_start..name_end].eq_ignore_ascii_case(tag_bytes)
            && bytes
                .get(name_end)
                .is_some_and(|&b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
        {
            return src[name_end..]
                .find('>')
                .map_or(bytes.len(), |gt| name_end + gt + 1);
        }
        pos = name_start;
    }
    warnings.push(format!(
        "unclosed <{tag_lower}> raw-text element at line {}",
        line_index.line_of_byte(start)
    ));
    bytes.len()
}

fn read_rcdata_title(src: &str, start: usize) -> (&str, usize) {
    let bytes = src.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        let Some(rel) = src[pos..].find("</") else {
            return (&src[start..], bytes.len());
        };
        let cand = pos + rel;
        let name_start = cand + 2;
        let name_end = name_start + 5;
        if name_end <= bytes.len()
            && bytes[name_start..name_end].eq_ignore_ascii_case(b"title")
            && bytes
                .get(name_end)
                .is_some_and(|&b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
        {
            let next_pos = src[name_end..]
                .find('>')
                .map_or(bytes.len(), |gt| name_end + gt + 1);
            return (&src[start..cand], next_pos);
        }
        pos = name_start;
    }
    (&src[start..], bytes.len())
}

fn next_char_boundary(src: &str, pos: usize) -> usize {
    if pos >= src.len() {
        return src.len();
    }
    pos + src[pos..].chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_handles_void_unquoted_comments_scripts_entities_and_stray_lt() {
        let html = concat!(
            "<!DOCTYPE html>\n",
            "<html><head><title>A &amp; B &#x2264; C</title>\n",
            "<style>.ltx_theorem { color: red; }</style>\n",
            "<script>const fake = '<div class=\"ltx_theorem\">x</div>';</script></head>\n",
            "<body><!-- <section class=\"ltx_section\">comment</section> -->\n",
            "<br><img src=fig.png alt='hi &quot;x&quot;'>\n",
            "<p class=ltx_p>If a < b and c &gt; 0.</p>\n",
            "</body></html>",
        );
        let out = scan_html_tokens(html);
        assert!(out.warnings.is_empty(), "unexpected warnings: {:?}", out.warnings);
        let tags: Vec<(&str, bool)> = out
            .tokens
            .iter()
            .filter_map(|t| match t {
                HtmlToken::StartTag { name, is_void, .. } => Some((name.as_str(), *is_void)),
                _ => None,
            })
            .collect();
        assert!(tags.contains(&("br", true)));
        assert!(tags.contains(&("img", true)));
        assert!(!tags.contains(&("div", false)));
        assert!(!tags.contains(&("section", false)));

        let texts: String = out
            .tokens
            .iter()
            .filter_map(|t| match t {
                HtmlToken::Text { decoded, .. } => Some(decoded.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains("A & B ≤ C"));
        assert!(texts.contains("If a < b and c > 0."));
        assert!(!texts.contains("const fake"));
        assert!(!texts.contains("color: red"));
    }
}
