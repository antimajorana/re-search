// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! PDF Unicode/PUA normalization, diacritic folding, PDFDocEncoding/UTF-16 decoding,
//! junk-title filtering, date-year parsing, and ISBN check-digit validation (package F14a).

use crate::paper_key::split_arxiv_id;

/// Normalizes PDF-extracted text: expands NFKC ligatures, maps Adobe Corporate Use PUA glyphs,
/// converts Unicode spaces to ASCII space, and strips soft hyphens.
pub fn normalize_pdf_unicode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' => out.push_str("ft"),
            '\u{FB06}' => out.push_str("st"),
            '\u{F721}'..='\u{F77E}' if c != '\u{F760}' => {
                if let Some(ascii) = char::from_u32(c as u32 - 0xF700) {
                    out.push(ascii);
                }
            }
            '\u{FF01}'..='\u{FF5E}' => {
                if let Some(ascii) = char::from_u32(c as u32 - 0xFEE0) {
                    out.push(ascii);
                }
            }
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' => out.push(' '),
            '\u{00AD}' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Lowercases and folds Latin-1 / Latin Extended-A/B diacritics to ASCII while stripping combining
/// marks and preserving non-Latin alphanumerics.
pub fn fold_diacritics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        for lc in c.to_lowercase() {
            if ('\u{0300}'..='\u{036F}').contains(&lc) {
                continue;
            }
            match lc {
                'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
                'ć' | 'č' | 'ç' | 'ĉ' | 'ċ' => out.push('c'),
                'ď' | 'đ' => out.push('d'),
                'é' | 'è' | 'ë' | 'ê' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => out.push('e'),
                'ğ' | 'ĝ' | 'ġ' | 'ģ' => out.push('g'),
                'ħ' | 'ĥ' => out.push('h'),
                'í' | 'ì' | 'ï' | 'î' | 'ī' | 'ĭ' | 'į' | 'ı' => out.push('i'),
                'ĵ' => out.push('j'),
                'ķ' => out.push('k'),
                'ł' | 'ĺ' | 'ļ' | 'ľ' | 'ŀ' => out.push('l'),
                'ñ' | 'ń' | 'ņ' | 'ň' => out.push('n'),
                'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' | 'ō' | 'ŏ' | 'ő' => out.push('o'),
                'ŕ' | 'ŗ' | 'ř' => out.push('r'),
                'ś' | 'ŝ' | 'ş' | 'š' => out.push('s'),
                'ţ' | 'ť' | 'ŧ' => out.push('t'),
                'ú' | 'ù' | 'ü' | 'û' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => out.push('u'),
                'ŵ' => out.push('w'),
                'ý' | 'ÿ' | 'ŷ' => out.push('y'),
                'ź' | 'ż' | 'ž' => out.push('z'),
                'ß' => out.push_str("ss"),
                'æ' => out.push_str("ae"),
                'œ' => out.push_str("oe"),
                'ð' => out.push('d'),
                'þ' => out.push_str("th"),
                _ => out.push(lc),
            }
        }
    }
    out
}

fn pdf_doc_char(b: u8) -> char {
    const MAP: [char; 31] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}',
        '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}',
        '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}',
        '\u{0141}', '\u{0152}', '\u{0160}', '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}',
        '\u{0153}', '\u{0161}', '\u{017E}',
    ];
    if (0x80..=0x9E).contains(&b) {
        MAP[(b - 0x80) as usize]
    } else if b == 0xA0 {
        '\u{20AC}'
    } else {
        b as char
    }
}

/// Decode a PDF `/Info` string (UTF-16BE/LE with BOM, UTF-8, or PDFDocEncoding) and normalize it.
pub fn decode_pdf_string(bytes: &[u8]) -> String {
    let raw = if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_be_bytes(c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_string()
    } else {
        bytes.iter().map(|&b| pdf_doc_char(b)).collect()
    };
    normalize_pdf_unicode(&raw).trim().to_string()
}

pub(crate) fn year_in(s: &str) -> Option<String> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let y = s.get(i..i + 4)?;
        let edge = |j: Option<&u8>| j.is_none_or(|c| !c.is_ascii_digit());
        ((y.starts_with("19") || y.starts_with("20"))
            && y.bytes().all(|c| c.is_ascii_digit())
            && edge(i.checked_sub(1).and_then(|j| b.get(j)))
            && edge(b.get(i + 4)))
        .then(|| y.to_string())
    })
}

/// Extract a four-digit year (`19xx` or `20xx`) from a PDF date (`D:20240115...` or ISO date).
pub fn parse_pdf_date_year(raw: &str) -> Option<String> {
    let t = raw.trim();
    let s = t.strip_prefix("D:").or_else(|| t.strip_prefix("d:")).unwrap_or(t).trim();
    if s.len() >= 4 && s.is_char_boundary(4) {
        let y = &s[..4];
        if (y.starts_with("19") || y.starts_with("20")) && y.bytes().all(|b| b.is_ascii_digit()) {
            return Some(y.to_string());
        }
    }
    year_in(s)
}

/// True for placeholder or tool-generated PDF `/Info` titles ("Untitled", "dvips", "main.tex", ...).
pub fn is_junk_pdf_title(raw: &str) -> bool {
    let t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return true;
    }
    let low = t.to_ascii_lowercase();
    if [
        "microsoft word", "microsoft powerpoint", "microsoft excel",
        "powerpoint presentation", "dvips", "pdftex", "xelatex", "lualatex",
    ]
    .iter()
    .any(|p| low.starts_with(p))
    {
        return true;
    }
    if [
        ".tex", ".dvi", ".pdf", ".doc", ".docx", ".ps", ".eps", ".ppt", ".pptx", ".odt", ".rtf",
    ]
    .iter()
    .any(|ext| low.ends_with(ext))
    {
        return true;
    }
    if matches!(
        low.as_str(),
        "untitled" | "no title" | "latex" | "latex2e" | "tex" | "document" | "paper" | "thesis"
            | "dissertation" | "manuscript" | "preprint" | "article" | "book" | "report" | "notes"
            | "draft" | "abstract" | "summary" | "slides" | "poster" | "full text pdf"
    ) {
        return true;
    }
    split_arxiv_id(&t).is_ok()
}

/// Validate and normalize an ISBN-10 or ISBN-13 (returns bare digits / trailing `X`).
pub fn normalize_valid_isbn(raw: &str) -> Option<String> {
    let digits: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, 'x' | 'X'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if digits.len() == 13 && digits.bytes().all(|b| b.is_ascii_digit()) {
        if !(digits.starts_with("978") || digits.starts_with("979")) {
            return None;
        }
        let sum: u32 = digits
            .bytes()
            .enumerate()
            .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 3 })
            .sum();
        return sum.is_multiple_of(10).then_some(digits);
    }
    if digits.len() == 10 && digits[..9].bytes().all(|b| b.is_ascii_digit()) {
        let sum: u32 = digits
            .bytes()
            .enumerate()
            .map(|(i, b)| {
                let v = if b == b'X' { 10 } else { u32::from(b - b'0') };
                v * (10 - i as u32)
            })
            .sum();
        return sum.is_multiple_of(11).then_some(digits);
    }
    None
}

/// Extract supplementary heuristic metadata candidates from a PDF's `/Info` dictionary using
/// `pdf_extract::Document` (panic-shielded; returns empty candidates on malformed/encrypted input).
pub fn extract_pdf_info_candidates(pdf_bytes: &[u8]) -> crate::pdf_firstpage::FirstPageCandidates {
    use crate::pdf_firstpage::{
        scan_line_identifiers, CandidateSource, FieldCandidate, FirstPageCandidates, IdSeenSets,
    };
    use pdf_extract::{Document, Object};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let read = || -> Option<FirstPageCandidates> {
        let mut doc = Document::load_mem(pdf_bytes).ok()?;
        if doc.is_encrypted() && doc.decrypt("").is_err() {
            return None;
        }
        let info_obj = doc.trailer.get(b"Info").ok()?;
        let info = match info_obj {
            Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok()?,
            Object::Dictionary(d) => d,
            _ => return None,
        };
        let get_str = |key: &[u8]| -> Option<String> {
            let val = info.get(key).ok()?;
            let resolved = match val {
                Object::Reference(id) => doc.get_object(*id).ok()?,
                other => other,
            };
            match resolved {
                Object::String(bytes, _) => {
                    let s = decode_pdf_string(bytes);
                    (!s.is_empty()).then_some(s)
                }
                _ => None,
            }
        };

        let mut out = FirstPageCandidates::default();
        let mut seen = IdSeenSets::default();

        if let Some(t) = get_str(b"Title") {
            scan_line_identifiers(
                &t, 0, 0, CandidateSource::PdfInfoDict, &mut seen, &mut out,
            );
            if !is_junk_pdf_title(&t) {
                out.title = Some(FieldCandidate::from_non_text(t, CandidateSource::PdfInfoDict));
            }
        }
        if let Some(a) = get_str(b"Author") {
            let parsed = split_info_authors(&a);
            if !parsed.is_empty() {
                out.authors = Some(FieldCandidate::from_non_text(
                    parsed,
                    CandidateSource::PdfInfoDict,
                ));
            }
        }
        if let Some(d) = get_str(b"CreationDate").and_then(|s| parse_pdf_date_year(&s)) {
            out.year = Some(FieldCandidate::from_non_text(d, CandidateSource::PdfInfoDict));
        }
        for key in [b"Subject".as_slice(), b"Keywords".as_slice()] {
            if let Some(val) = get_str(key) {
                scan_line_identifiers(
                    &val, 0, 0, CandidateSource::PdfInfoDict, &mut seen, &mut out,
                );
            }
        }
        Some(out)
    };
    catch_unwind(AssertUnwindSafe(read))
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn split_info_authors(raw: &str) -> Vec<String> {
    let cleaned = raw.trim();
    if cleaned.is_empty() {
        return Vec::new();
    }
    if cleaned.contains(';') || cleaned.contains('&') || cleaned.to_ascii_lowercase().contains(" and ") {
        return cleaned
            .split([';', '&'])
            .flat_map(|p| p.split(" and "))
            .flat_map(|p| p.split(" AND "))
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
    }
    let parts: Vec<&str> = cleaned.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() == 2 && parts[0].split_whitespace().count() == 1 {
        return vec![format!("{}, {}", parts[0], parts[1])];
    }
    parts.into_iter().map(String::from).collect()
}
