// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Pure parsing and normalization helpers for [`crate::source_zotero::ZoteroAdapter`]
//! (`prefs.js`, pinned citation keys, identifier extraction, and Kandinsky identity folding).

use crate::library::LibraryIds;
use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::pdf_firstpage::normalize_valid_isbn;
use crate::source_bibtex::parse_arxiv_from_url;
use std::collections::BTreeMap;

/// Parsed values from a Zotero profile `prefs.js` file (ported from Kandinsky `zotero_import.rs`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ZoteroPrefs {
    pub data_dir: Option<String>,
    pub base_attachment_path: Option<String>,
    pub zotmoov_dst_dir: Option<String>,
    pub zotmoov_enable_subdir: bool,
    pub zotmoov_subdir_str: Option<String>,
}

pub(crate) fn extract_zotero_ids(fields: &BTreeMap<String, String>) -> (LibraryIds, Option<String>) {
    let mut ids = LibraryIds::default();
    let mut version: Option<String> = None;

    if let Some(raw_doi) = fields.get("DOI").or_else(|| fields.get("doi")) {
        if let Ok(norm) = canonical_doi(raw_doi) {
            if let Some(ax_part) = norm.strip_prefix("10.48550/arxiv.") {
                if let Ok((base, ver)) = split_arxiv_id(ax_part) {
                    ids.arxiv = Some(base);
                    version = ver;
                }
            }
            ids.doi = Some(norm);
        }
    }
    for key in ["archiveID", "url", "extra"] {
        if ids.arxiv.is_some() {
            break;
        }
        if let Some(val) = fields.get(key) {
            if let Some((base, ver)) = parse_arxiv_from_zotero_field(val) {
                ids.arxiv = Some(base);
                if ver.is_some() {
                    version = ver;
                }
            }
        }
    }
    if let Some(raw_isbn) = fields.get("ISBN").or_else(|| fields.get("isbn")) {
        if let Some(norm) = normalize_valid_isbn(raw_isbn) {
            ids.isbn = Some(norm);
        }
    }
    (ids, version)
}

fn parse_arxiv_from_zotero_field(raw: &str) -> Option<(String, Option<String>)> {
    if let Some(parsed) = parse_arxiv_from_url(raw) {
        return Some(parsed);
    }
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Ok(parsed) = split_arxiv_id(trimmed) {
            return Some(parsed);
        }
        let low = trimmed.to_ascii_lowercase();
        for prefix in ["arxiv:", "arxiv "] {
            if let Some(idx) = low.find(prefix) {
                let tail = trimmed[idx + prefix.len()..].trim();
                let tok = tail.split_whitespace().next().unwrap_or("");
                if let Ok(parsed) = split_arxiv_id(tok) {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

/// Sanitize a Zotero / Better BibTeX citation key (ported from Kandinsky `zotero_import.rs`).
pub fn sanitize_citekey(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(['.', ' ', '_'])
        .to_string()
}

/// Extract a pinned citation key from Zotero's `Extra` field (`Citation Key:` or `bibtex:`),
/// ported from Kandinsky `zotero_import.rs`.
pub fn extra_citekey(extra: &str) -> Option<String> {
    extra.lines().find_map(|l| {
        let t = l.trim();
        let low = t.to_ascii_lowercase();
        ["citation key:", "bibtex:"].into_iter().find_map(|p| {
            let rest_len = low.strip_prefix(p)?.len();
            let val = t[t.len() - rest_len..].split_whitespace().next()?;
            Some(sanitize_citekey(val)).filter(|c| !c.is_empty())
        })
    })
}

/// Parse Zotero `prefs.js` entries for dataDir and linked attachment paths
/// (ported from Kandinsky `zotero_import.rs`).
pub fn parse_prefs_js(text: &str) -> ZoteroPrefs {
    let mut p = ZoteroPrefs::default();
    for line in text.lines() {
        let Some((key, after)) = line
            .trim()
            .strip_prefix("user_pref(\"")
            .and_then(|r| r.split_once("\","))
        else {
            continue;
        };
        let val = after.trim().trim_end_matches(");").trim();
        let sv = || {
            val.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .map(unescape_js)
                .filter(|s| !s.trim().is_empty())
        };
        match key {
            "extensions.zotero.dataDir" => p.data_dir = sv(),
            "extensions.zotero.baseAttachmentPath" => p.base_attachment_path = sv(),
            "extensions.zotmoov.dst_dir" => p.zotmoov_dst_dir = sv(),
            "extensions.zotmoov.enable_subdir_move" => p.zotmoov_enable_subdir = val == "true",
            "extensions.zotmoov.subdir_str" => p.zotmoov_subdir_str = sv(),
            _ => {}
        }
    }
    p
}

fn unescape_js(s: &str) -> String {
    let (mut out, mut chars) = (String::with_capacity(s.len()), s.chars());
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some(_)) => match chars.next().unwrap() {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                o => out.push(o),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Fold Zotero rich-text markup (`<i>`, `<sub>`, `<span class="nocase">`), HTML entities,
/// accents, and PDF ligatures for title/author identity comparisons
/// (ported from Kandinsky `zotero_import.rs::ident_fold`).
pub fn ident_fold(t: &str) -> String {
    let (mut out, mut rest) = (String::with_capacity(t.len()), t);
    while let Some(c) = rest.chars().next() {
        let tail = &rest[c.len_utf8()..];
        let tag = (c == '<' && tail.starts_with(|n: char| n == '/' || n.is_ascii_alphabetic()))
            .then(|| tail.find('>'))
            .flatten();
        let entity = (c == '&')
            .then(|| tail.find(';'))
            .flatten()
            .filter(|&n| {
                (1..=8).contains(&n)
                    && tail[..n]
                        .chars()
                        .all(|x| x.is_ascii_alphanumeric() || x == '#')
            });
        if let Some(n) = tag.or(entity) {
            rest = &tail[n + 1..];
            continue;
        }
        rest = tail;
        for l in c.to_lowercase() {
            match l {
                'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
                'é' | 'è' | 'ë' | 'ê' | 'ē' | 'ė' | 'ę' | 'ě' => out.push('e'),
                'í' | 'ì' | 'ï' | 'î' | 'ī' | 'ı' => out.push('i'),
                'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' | 'ō' | 'ő' => out.push('o'),
                'ú' | 'ù' | 'ü' | 'û' | 'ū' | 'ů' | 'ű' => out.push('u'),
                'ý' | 'ÿ' => out.push('y'),
                'ñ' | 'ń' | 'ň' => out.push('n'),
                'ç' | 'ć' | 'č' => out.push('c'),
                'ś' | 'š' | 'ş' => out.push('s'),
                'ź' | 'ż' | 'ž' => out.push('z'),
                'ł' => out.push('l'),
                'ř' => out.push('r'),
                'ď' => out.push('d'),
                'ť' => out.push('t'),
                'ß' => out.push_str("ss"),
                'æ' => out.push_str("ae"),
                'œ' => out.push_str("oe"),
                'ﬀ' => out.push_str("ff"),
                'ﬁ' => out.push_str("fi"),
                'ﬂ' => out.push_str("fl"),
                'ﬃ' => out.push_str("ffi"),
                'ﬄ' => out.push_str("ffl"),
                l if l.is_alphanumeric() => out.push(l),
                _ => {}
            }
        }
    }
    out
}

/// Extract the first author's folded surname (ported from Kandinsky `zotero_import.rs`).
pub fn first_author_last(authors: &[String]) -> Option<String> {
    let a = authors.first()?;
    a.split(',')
        .next()
        .unwrap_or(a)
        .split_whitespace()
        .last()
        .map(ident_fold)
        .filter(|l| !l.is_empty())
}
