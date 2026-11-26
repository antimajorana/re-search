//! TeX diacritic/accent decoding and BibTeX author-list splitting for
//! [`crate::source_bibtex`] (package F13b, `00-overview.md` 2.11).

const DIACRITICS: &[(char, char, char)] = &[
    ('\'', 'a', 'á'), ('\'', 'e', 'é'), ('\'', 'i', 'í'), ('\'', 'o', 'ó'), ('\'', 'u', 'ú'),
    ('\'', 'y', 'ý'), ('\'', 'c', 'ć'), ('\'', 'n', 'ń'), ('\'', 's', 'ś'), ('\'', 'z', 'ź'),
    ('\'', 'A', 'Á'), ('\'', 'E', 'É'), ('\'', 'I', 'Í'), ('\'', 'O', 'Ó'), ('\'', 'U', 'Ú'),
    ('"', 'a', 'ä'), ('"', 'e', 'ë'), ('"', 'i', 'ï'), ('"', 'o', 'ö'), ('"', 'u', 'ü'),
    ('"', 'y', 'ÿ'), ('"', 'A', 'Ä'), ('"', 'E', 'Ë'), ('"', 'I', 'Ï'), ('"', 'O', 'Ö'),
    ('"', 'U', 'Ü'), ('`', 'a', 'à'), ('`', 'e', 'è'), ('`', 'i', 'ì'), ('`', 'o', 'ò'),
    ('`', 'u', 'ù'), ('^', 'a', 'â'), ('^', 'e', 'ê'), ('^', 'i', 'î'), ('^', 'o', 'ô'),
    ('^', 'u', 'û'), ('~', 'a', 'ã'), ('~', 'n', 'ñ'), ('~', 'o', 'õ'), ('~', 'N', 'Ñ'),
    ('=', 'a', 'ā'), ('=', 'e', 'ē'), ('=', 'i', 'ī'), ('=', 'o', 'ō'), ('=', 'u', 'ū'),
    ('.', 'z', 'ż'), ('.', 'Z', 'Ż'), ('c', 'c', 'ç'), ('c', 'C', 'Ç'), ('c', 's', 'ş'),
    ('v', 'c', 'č'), ('v', 's', 'š'), ('v', 'z', 'ž'), ('v', 'r', 'ř'), ('v', 'e', 'ě'),
    ('v', 'n', 'ň'), ('v', 'C', 'Č'), ('v', 'S', 'Š'), ('v', 'Z', 'Ž'), ('r', 'a', 'å'),
    ('r', 'A', 'Å'), ('r', 'u', 'ů'), ('H', 'o', 'ő'), ('H', 'u', 'ű'), ('u', 'a', 'ă'),
    ('k', 'a', 'ą'), ('k', 'e', 'ę'),
];

/// Decode common BibTeX / TeX accents, special letters, escaped symbols, and grouping braces
/// for human-readable metadata normalization (title, authors, venue).
pub fn decode_tex_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' | '}' => i += 1,
            '~' => {
                out.push(' ');
                i += 1;
            }
            '-' if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == '-' => {
                out.push('\u{2014}');
                i += 3;
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                out.push('\u{2013}');
                i += 2;
            }
            '\\' if i + 1 < chars.len() => {
                i += 1;
                let cmd = chars[i];
                if matches!(cmd, '&' | '%' | '_' | '$' | '#' | '{' | '}') {
                    out.push(cmd);
                    i += 1;
                } else if matches!(cmd, '\'' | '"' | '`' | '^' | '~' | '=' | '.') {
                    i += 1;
                    if let Some((base, next_i)) = read_accent_arg(&chars, i) {
                        out.push(apply_diacritic(cmd, base));
                        i = next_i;
                    }
                } else if cmd.is_ascii_alphabetic() {
                    let start = i;
                    while i < chars.len() && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    if let Some(accent) = letter_accent_cmd(&word) {
                        if let Some((base, next_i)) = read_accent_arg(&chars, i) {
                            out.push(apply_diacritic(accent, base));
                            i = next_i;
                        }
                    } else if let Some(rep) = special_tex_word(&word) {
                        out.push_str(rep);
                        if i < chars.len()
                            && chars[i] == ' '
                            && chars.get(i + 1).is_some_and(|c| *c == '{' || *c == '}')
                        {
                            i += 1;
                        }
                    } else if matches!(
                        word.as_str(),
                        "emph" | "textit" | "textbf" | "textrm" | "texttt" | "text" | "mathrm"
                    ) {
                        // Drop formatting wrapper; inner text follows.
                    } else {
                        out.push('\\');
                        out.push_str(&word);
                    }
                } else {
                    out.push(cmd);
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_accent_arg(chars: &[char], mut i: usize) -> Option<(char, usize)> {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    if chars[i] == '{' {
        i += 1;
        let base = if i + 1 < chars.len() && chars[i] == '\\' && matches!(chars[i + 1], 'i' | 'j') {
            let b = chars[i + 1];
            i += 2;
            b
        } else {
            let b = *chars.get(i)?;
            i += 1;
            b
        };
        while i < chars.len() && chars[i] != '}' {
            i += 1;
        }
        Some((base, (i + 1).min(chars.len())))
    } else if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], 'i' | 'j') {
        Some((chars[i + 1], i + 2))
    } else {
        Some((chars[i], i + 1))
    }
}

fn letter_accent_cmd(word: &str) -> Option<char> {
    match word {
        "c" => Some('c'),
        "v" => Some('v'),
        "u" => Some('u'),
        "H" => Some('H'),
        "r" => Some('r'),
        "k" => Some('k'),
        "d" => Some('d'),
        "b" => Some('b'),
        _ => None,
    }
}

fn special_tex_word(word: &str) -> Option<&'static str> {
    match word {
        "ss" => Some("ß"),
        "ae" => Some("æ"),
        "AE" => Some("Æ"),
        "oe" => Some("œ"),
        "OE" => Some("Œ"),
        "aa" => Some("å"),
        "AA" => Some("Å"),
        "o" => Some("ø"),
        "O" => Some("Ø"),
        "l" => Some("ł"),
        "L" => Some("Ł"),
        "i" => Some("i"),
        "j" => Some("j"),
        "textendash" => Some("–"),
        "textemdash" => Some("—"),
        "LaTeX" => Some("LaTeX"),
        "TeX" => Some("TeX"),
        _ => None,
    }
}

fn apply_diacritic(cmd: char, base: char) -> char {
    DIACRITICS
        .iter()
        .find(|&&(c, b, _)| c == cmd && b == base)
        .map_or(base, |&(_, _, out)| out)
}

/// Split a BibTeX `author` / `editor` field on top-level ` and ` (brace-depth 0, case-insensitive)
/// and normalize each author name with [`decode_tex_text`].
pub fn split_bibtex_authors(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b if depth == 0 && b.is_ascii_whitespace() => {
                let ws_start = i;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i + 3 < bytes.len()
                    && bytes[i..i + 3].eq_ignore_ascii_case(b"and")
                    && bytes[i + 3].is_ascii_whitespace()
                {
                    parts.push(&raw[start..ws_start]);
                    i += 3;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    start = i;
                }
            }
            _ => i += 1,
        }
    }
    if start < raw.len() {
        parts.push(&raw[start..]);
    }
    parts
        .into_iter()
        .map(decode_tex_text)
        .filter(|a| !a.is_empty())
        .collect()
}

pub(crate) fn extract_bibtex_ids(
    fields: &std::collections::BTreeMap<String, String>,
) -> (crate::library::LibraryIds, Option<String>) {
    let mut ids = crate::library::LibraryIds::default();
    let mut version: Option<String> = None;

    if let Some(raw_doi) = fields.get("doi") {
        if let Ok(norm) = crate::paper_key::canonical_doi(raw_doi) {
            if let Some(ax_part) = norm.strip_prefix("10.48550/arxiv.") {
                if let Ok((base, ver)) = crate::paper_key::split_arxiv_id(ax_part) {
                    ids.arxiv = Some(base);
                    version = ver;
                }
            }
            ids.doi = Some(norm);
        }
    }

    if let Some(raw_ax) = fields.get("arxiv").or_else(|| fields.get("archiveid")) {
        if let Ok((base, ver)) = crate::paper_key::split_arxiv_id(raw_ax) {
            ids.arxiv = Some(base);
            if ver.is_some() {
                version = ver;
            }
        }
    }

    if ids.arxiv.is_none() {
        if let Some(eprint) = fields.get("eprint") {
            let prefix = fields
                .get("archiveprefix")
                .or_else(|| fields.get("eprinttype"))
                .map(|s| s.trim().to_ascii_lowercase());
            if prefix.as_deref().is_none_or(|p| p.is_empty() || p == "arxiv") {
                if let Ok((base, ver)) = crate::paper_key::split_arxiv_id(eprint) {
                    ids.arxiv = Some(base);
                    if ver.is_some() {
                        version = ver;
                    }
                }
            }
        }
    }

    if ids.arxiv.is_none() {
        if let Some(url) = fields.get("url") {
            if let Some((base, ver)) = parse_arxiv_from_url(url) {
                ids.arxiv = Some(base);
                if ver.is_some() {
                    version = ver;
                }
            }
        }
    }

    if let Some(raw_isbn) = fields.get("isbn") {
        if let Some(norm) = crate::pdf_firstpage::normalize_valid_isbn(raw_isbn) {
            ids.isbn = Some(norm);
        }
    }

    (ids, version)
}

pub(crate) fn parse_arxiv_from_url(raw: &str) -> Option<(String, Option<String>)> {
    let low = raw.to_ascii_lowercase();
    for marker in ["arxiv.org/abs/", "arxiv.org/pdf/", "arxiv.org/e-print/"] {
        if let Some(idx) = low.find(marker) {
            let tail = &raw[idx + marker.len()..];
            let token = tail
                .split(['?', '#', ' ', '\t', '\r', '\n'])
                .next()
                .unwrap_or("")
                .trim_end_matches('/')
                .trim_end_matches(".pdf");
            if let Ok(parsed) = crate::paper_key::split_arxiv_id(token) {
                return Some(parsed);
            }
        }
    }
    None
}

pub(crate) fn split_list_field(raw: Option<&String>) -> Vec<String> {
    let Some(text) = raw else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in text.split([',', ';']) {
        let item = decode_tex_text(part);
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

pub(crate) fn normalize_bib_item_kind(entry_type: &str) -> String {
    match entry_type.to_ascii_lowercase().as_str() {
        "article" => "journalArticle".to_string(),
        "inproceedings" | "conference" => "conferencePaper".to_string(),
        "book" | "inbook" | "incollection" => "book".to_string(),
        "phdthesis" | "mastersthesis" => "thesis".to_string(),
        "techreport" => "report".to_string(),
        "misc" | "unpublished" | "preprint" => "preprint".to_string(),
        other => other.to_string(),
    }
}
