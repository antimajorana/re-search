//! Magic-byte and structural MIME sniffing for Solve scratch artifacts (package S18 Part A).
//!
//! Ignores filename extensions completely; accepts only PNG, PDF, download-only SVG, and valid
//! UTF-8 JSON/CSV/plain text, rejecting executables, archives, HTML/scripts, and binary garbage.

pub const MIME_PNG: &str = "image/png";
pub const MIME_PDF: &str = "application/pdf";
pub const MIME_SVG: &str = "image/svg+xml";
pub const MIME_JSON: &str = "application/json";
pub const MIME_CSV: &str = "text/csv";
pub const MIME_PLAIN: &str = "text/plain";

pub(crate) const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
pub(crate) const PDF_MAGIC: &[u8] = b"%PDF-";

/// Detect MIME strictly from file magic bytes and content structure, ignoring any filename
/// extension. Returns `Ok(mime)` only for allowlisted formats (PNG, PDF, download-only SVG,
/// valid UTF-8 JSON/CSV/plain text) and `Err(reason)` for executables, archives, HTML, and binary
/// garbage.
pub fn sniff_artifact_mime(bytes: &[u8]) -> Result<&'static str, String> {
    if bytes.is_empty() {
        return Err("empty artifact file".into());
    }
    if let Some(kind) = detect_disallowed_binary_magic(bytes) {
        return Err(format!("disallowed binary or archive content ({kind})"));
    }
    if bytes.starts_with(PNG_MAGIC) {
        return Ok(MIME_PNG);
    }
    if bytes.starts_with(PDF_MAGIC) {
        return Ok(MIME_PDF);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "unsupported binary content (not PNG, PDF, or valid UTF-8 text)".to_string())?;
    if has_binary_control_chars(text) {
        return Err("binary control bytes in text artifact".into());
    }
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Err("blank text artifact".into());
    }
    if looks_like_html_or_script(trimmed) {
        return Err("disallowed HTML or script content".into());
    }
    if looks_like_svg(trimmed) {
        return Ok(MIME_SVG);
    }
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return Ok(MIME_JSON);
    }
    if looks_like_csv(trimmed) {
        return Ok(MIME_CSV);
    }
    Ok(MIME_PLAIN)
}

fn detect_disallowed_binary_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"MZ") || bytes.starts_with(b"ZM") {
        return Some("Windows/DOS executable");
    }
    if bytes.starts_with(b"\x7fELF") {
        return Some("ELF executable");
    }
    if bytes.len() >= 4
        && matches!(
            &bytes[..4],
            [0xfe, 0xed, 0xfa, 0xce]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
        )
    {
        return Some("Mach-O/fat binary");
    }
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
        || bytes.starts_with(b"PK\x01\x02")
    {
        return Some("ZIP archive");
    }
    if bytes.starts_with(b"\x1f\x8b")
        || bytes.starts_with(b"BZh")
        || bytes.starts_with(b"\xfd7zXZ\x00")
        || bytes.starts_with(b"\x28\xb5\x2f\xfd")
        || bytes.starts_with(b"7z\xbc\xaf\x27\x1c")
        || bytes.starts_with(b"Rar!\x1a\x07")
        || bytes.starts_with(b"!<arch>\n")
        || (bytes.len() > 262 && &bytes[257..262] == b"ustar")
    {
        return Some("compressed archive");
    }
    None
}

fn has_binary_control_chars(text: &str) -> bool {
    text.bytes()
        .any(|b| (b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r')) || b == 0x7f)
}

fn strip_xml_prolog_and_comments(s: &str) -> &str {
    let mut rest = s.trim_start();
    loop {
        if let Some(after_pi) = rest.strip_prefix("<?xml") {
            if let Some(end) = after_pi.find("?>") {
                rest = after_pi[end + 2..].trim_start();
                continue;
            }
        }
        if let Some(after_comment) = rest.strip_prefix("<!--") {
            if let Some(end) = after_comment.find("-->") {
                rest = after_comment[end + 3..].trim_start();
                continue;
            }
        }
        break;
    }
    rest
}

fn has_tag_prefix(lower: &str, tag: &str) -> bool {
    let Some(rest) = lower.strip_prefix(tag) else {
        return false;
    };
    rest.is_empty()
        || rest
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
}

fn looks_like_html_or_script(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("<script") || lower.contains("</script") {
        return true;
    }
    let after_prolog = strip_xml_prolog_and_comments(&lower);
    after_prolog.starts_with("<!doctype html")
        || has_tag_prefix(after_prolog, "<html")
        || has_tag_prefix(after_prolog, "<head")
        || has_tag_prefix(after_prolog, "<body")
        || has_tag_prefix(after_prolog, "<script")
        || has_tag_prefix(after_prolog, "<iframe")
        || has_tag_prefix(after_prolog, "<object")
        || has_tag_prefix(after_prolog, "<embed")
}

fn looks_like_svg(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    let mut rest = strip_xml_prolog_and_comments(&lower);
    if let Some(after_dt) = rest.strip_prefix("<!doctype svg") {
        if let Some(end) = after_dt.find('>') {
            rest = strip_xml_prolog_and_comments(&after_dt[end + 1..]);
        }
    }
    has_tag_prefix(rest, "<svg") && (rest.contains("</svg>") || rest.contains("/>"))
}

fn csv_field_count(line: &str) -> Option<usize> {
    let mut cols = 1usize;
    let mut in_quotes = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if in_quotes && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                i += 2;
                continue;
            }
            in_quotes = !in_quotes;
        } else if bytes[i] == b',' && !in_quotes {
            cols += 1;
        }
        i += 1;
    }
    if in_quotes { None } else { Some(cols) }
}

fn looks_like_csv(trimmed: &str) -> bool {
    let lines: Vec<&str> = trimmed.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    if lines[0].starts_with('<') || lines[0].starts_with('{') || lines[0].starts_with('[') {
        return false;
    }
    let Some(first_cols) = csv_field_count(lines[0]) else {
        return false;
    };
    if first_cols < 2 {
        return false;
    }
    lines.iter().skip(1).all(|l| csv_field_count(l) == Some(first_cols))
}
