// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! First-page PDF metadata candidates (title, authors, year, DOI, arXiv id, ISBN) with exact
//! 1-based line provenance and `/Info` dictionary reading (package F14a, 00-overview.md 2.11–2.12).
//!
//! Every candidate returned here is heuristic (`certain: false`) and carries the 1-based line
//! and page number it came from whenever derived from extracted page text.

use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::pdf_firstpage_norm::year_in;
use crate::pdf_pages::PAGE_BREAK_CHAR;
use percent_encoding::percent_decode_str;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

pub use crate::pdf_firstpage_norm::{
    decode_pdf_string, extract_pdf_info_candidates, fold_diacritics, is_junk_pdf_title,
    normalize_pdf_unicode, normalize_valid_isbn, parse_pdf_date_year,
};

const SCAN_PAGES_FOR_TITLE: usize = 5;
const SCAN_PAGES_FOR_IDS: usize = 5;
const MAX_LINES_PER_PAGE_SHAPE: usize = 25;

/// Where a metadata candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    FirstPageLine,
    IdentifierScan,
    PdfInfoDict,
    FileName,
}

/// A single heuristic metadata candidate with exact line provenance when found in page text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldCandidate<T> {
    pub value: T,
    /// 1-based starting line in the extracted reading file (`None` for `/Info` or filename).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// 1-based ending line when the candidate spans multiple lines (e.g. a wrapped title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    /// 1-based physical page number (`None` for `/Info` or filename).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    pub source: CandidateSource,
    /// Always `false`: code-extracted first-page metadata is never presented as certain.
    pub certain: bool,
}

impl<T> FieldCandidate<T> {
    pub fn from_line(value: T, page: usize, line: usize, source: CandidateSource) -> Self {
        Self {
            value,
            line: Some(line),
            end_line: Some(line),
            page: Some(page),
            source,
            certain: false,
        }
    }

    pub fn from_span(
        value: T,
        page: usize,
        start_line: usize,
        end_line: usize,
        source: CandidateSource,
    ) -> Self {
        Self {
            value,
            line: Some(start_line),
            end_line: Some(end_line),
            page: Some(page),
            source,
            certain: false,
        }
    }

    pub fn from_non_text(value: T, source: CandidateSource) -> Self {
        Self {
            value,
            line: None,
            end_line: None,
            page: None,
            source,
            certain: false,
        }
    }
}

/// Canonicalised arXiv identifier candidate (validated through F2 [`split_arxiv_id`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArxivCandidate {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Code-found metadata candidates for a PDF (all heuristic, `certain == false`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstPageCandidates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<FieldCandidate<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authors: Option<FieldCandidate<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<FieldCandidate<String>>,
    #[serde(default)]
    pub dois: Vec<FieldCandidate<String>>,
    #[serde(default)]
    pub arxiv_ids: Vec<FieldCandidate<ArxivCandidate>>,
    #[serde(default)]
    pub isbns: Vec<FieldCandidate<String>>,
}

fn furniture(l: &str) -> bool {
    let low = l.to_lowercase();
    [
        "arxiv", "doi", "http", "www.", "©", "copyright", "preprint", "issn", "isbn", "vol.",
        "volume ", "pp.",
    ]
    .iter()
    .any(|w| low.contains(w))
        || l.chars().filter(|c| c.is_alphabetic()).count() < 4
}

fn reads_as_words(l: &str) -> bool {
    let words: Vec<&str> = l.split_whitespace().collect();
    let singles = words
        .iter()
        .filter(|w| {
            w.chars().filter(|c| c.is_alphabetic()).count() == 1 && !matches!(**w, "a" | "A" | "I")
        })
        .count();
    words.len() >= 2 && singles * 4 <= words.len() && !words.iter().any(|w| w.len() > 30)
}

fn names_in(l: &str) -> Option<Vec<String>> {
    const PARTICLES: &[&str] = &[
        "de", "van", "von", "der", "den", "da", "di", "du", "la", "le", "del", "dos", "y",
    ];
    const PLACES: &[&str] = &[
        "university", "université", "universität", "institute", "institut", "department",
        "laboratory", "laboratoire", "school", "college", "center", "centre", "lecture",
        "abstract", "dated", "physics", "école", "faculty",
    ];
    let clean: String = l
        .chars()
        .filter(|c| {
            !c.is_ascii_digit()
                && !matches!(
                    c,
                    '*' | '†' | '‡' | '§' | '¶' | '(' | ')' | '\u{2217}' | '\u{22C6}' | '¹'
                        | '²' | '³' | '\u{2709}'
                )
                && !('\u{2070}'..='\u{2079}').contains(c)
        })
        .collect();
    let parts: Vec<String> = clean
        .split([',', ';', '&'])
        .flat_map(|p| p.split(" and "))
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect();
    let person = |p: &str| {
        let words: Vec<&str> = p.split_whitespace().collect();
        (2..=5).contains(&words.len())
            && words
                .iter()
                .all(|w| PARTICLES.contains(w) || w.chars().next().is_some_and(char::is_uppercase))
            && !words.iter().any(|w| PLACES.contains(&w.to_lowercase().as_str()))
            && words
                .last()
                .is_some_and(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2)
    };
    (!parts.is_empty() && parts.iter().all(|p| person(p))).then_some(parts)
}

fn is_cover_sheet(lines: &[(usize, String)]) -> bool {
    let low = lines
        .iter()
        .map(|(_, s)| s.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    [
        "warwick research archive portal", "wrap.warwick.ac.uk", "hal-", "downloaded from",
        "accepted manuscript", "ssrn", "researchgate", "institutional repository",
        "university repository", "this version is available at", "please cite this article as",
    ]
    .iter()
    .any(|m| low.contains(m))
}

fn has_month_year(low: &str) -> bool {
    const MONTHS: &[&str] = &[
        "january", "february", "march", "april", "may", "june", "july", "august", "september",
        "october", "november", "december", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep",
        "sept", "oct", "nov", "dec",
    ];
    let words: Vec<&str> = low
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.windows(2).any(|w| {
        MONTHS.contains(&w[0])
            && w[1].len() == 4
            && (w[1].starts_with("19") || w[1].starts_with("20"))
            && w[1].bytes().all(|b| b.is_ascii_digit())
    })
}

fn extract_page_year(
    lines: &[(usize, String)],
    title_indices: std::ops::Range<usize>,
    page: usize,
) -> Option<FieldCandidate<String>> {
    const SKIP: &[&str] = &["arxiv", "doi", "http", "www.", "issn", "vol.", "volume "];
    const EXPLICIT_CUES: &[&str] = &[
        "dated", "date:", "received", "submitted", "published", "last compiled", "compiled:",
        "summer term", "winter term", "spring term", "autumn term", "fall term",
    ];
    const COPYRIGHT_CUES: &[&str] = &["©", "copyright"];

    lines
        .iter()
        .take(20)
        .enumerate()
        .find_map(|(idx, (line_no, l))| {
            let low = l.to_lowercase();
            if SKIP.iter().any(|w| low.contains(w)) {
                return None;
            }
            let cue = EXPLICIT_CUES.iter().any(|w| low.contains(w))
                || (!title_indices.contains(&idx) && has_month_year(&low));
            cue.then(|| year_in(l))
                .flatten()
                .map(|y| FieldCandidate::from_line(y, page, *line_no, CandidateSource::FirstPageLine))
        })
        .or_else(|| {
            lines.iter().take(20).find_map(|(line_no, l)| {
                let low = l.to_lowercase();
                if SKIP.iter().any(|w| low.contains(w)) {
                    return None;
                }
                COPYRIGHT_CUES
                    .iter()
                    .any(|w| low.contains(w))
                    .then(|| year_in(l))
                    .flatten()
                    .map(|y| {
                        FieldCandidate::from_line(y, page, *line_no, CandidateSource::FirstPageLine)
                    })
            })
        })
}

fn split_numbered_pages(reading_text: &str) -> Vec<(usize, Vec<(usize, String)>)> {
    let mut pages: Vec<(usize, Vec<(usize, String)>)> = Vec::new();
    let mut cur_page = 1usize;
    pages.push((cur_page, Vec::new()));

    for (idx, raw_line) in reading_text.split('\n').enumerate() {
        let line_no = idx + 1;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.contains(PAGE_BREAK_CHAR) {
            let mut segs = line.split(PAGE_BREAK_CHAR).peekable();
            while let Some(seg) = segs.next() {
                let trimmed = normalize_pdf_unicode(seg).trim().to_string();
                if !trimmed.is_empty() {
                    if let Some((_, list)) = pages.last_mut() {
                        list.push((line_no, trimmed));
                    }
                }
                if segs.peek().is_some() {
                    cur_page += 1;
                    pages.push((cur_page, Vec::new()));
                }
            }
        } else {
            let trimmed = normalize_pdf_unicode(line).trim().to_string();
            if !trimmed.is_empty() {
                if let Some((_, list)) = pages.last_mut() {
                    list.push((line_no, trimmed));
                }
            }
        }
    }
    pages
}

struct ShapeReading {
    title: FieldCandidate<String>,
    authors: FieldCandidate<Vec<String>>,
    year: Option<FieldCandidate<String>>,
}

fn read_single_page_shape(
    page: usize,
    lines: &[(usize, String)],
    file_name: Option<&str>,
) -> Option<ShapeReading> {
    let head: &[(usize, String)] = &lines[..lines.len().min(MAX_LINES_PER_PAGE_SHAPE)];
    let at = head.iter().position(|(_, l)| !furniture(l))?;
    let start_line = head[at].0;
    let mut end_line = start_line;
    let mut title = head[at].1.clone();
    let mut next = at + 1;
    while next < head.len()
        && head[next].1.starts_with(|c: char| c.is_lowercase())
        && title.len() < 200
    {
        title = format!("{title} {}", head[next].1);
        end_line = head[next].0;
        next += 1;
    }
    let title = title.trim_end_matches(['.', ':', ';', ',', ' ']).to_string();
    if !reads_as_words(&title) || names_in(&title).is_some() {
        return None;
    }
    let (auth_line, authors) = head
        .iter()
        .skip(next)
        .take(5)
        .find_map(|(ln, l)| names_in(l).map(|a| (*ln, a)))?;
    let year = extract_page_year(head, at..next, page).or_else(|| {
        file_name.and_then(|f| {
            let decoded = percent_decode_str(f).decode_utf8_lossy();
            year_in(&decoded).map(|y| FieldCandidate::from_non_text(y, CandidateSource::FileName))
        })
    });
    Some(ShapeReading {
        title: FieldCandidate::from_span(
            title,
            page,
            start_line,
            end_line,
            CandidateSource::FirstPageLine,
        ),
        authors: FieldCandidate::from_line(
            authors,
            page,
            auth_line,
            CandidateSource::FirstPageLine,
        ),
        year,
    })
}

fn push_candidate<T>(
    value: T,
    page: usize,
    line_no: usize,
    source: CandidateSource,
    dest: &mut Vec<FieldCandidate<T>>,
) {
    dest.push(match source {
        CandidateSource::PdfInfoDict | CandidateSource::FileName => {
            FieldCandidate::from_non_text(value, source)
        }
        _ => FieldCandidate::from_line(value, page, line_no, source),
    });
}

#[derive(Default)]
pub(crate) struct IdSeenSets {
    pub dois: BTreeSet<String>,
    pub arxiv: BTreeSet<String>,
    pub isbns: BTreeSet<String>,
}

pub(crate) fn scan_line_identifiers(
    line: &str,
    page: usize,
    line_no: usize,
    source: CandidateSource,
    seen: &mut IdSeenSets,
    out: &mut FirstPageCandidates,
) {
    static ARXIV_RE: OnceLock<Regex> = OnceLock::new();
    static DOI_RE: OnceLock<Regex> = OnceLock::new();
    static ISBN_RE: OnceLock<Regex> = OnceLock::new();

    let arxiv_re = ARXIV_RE.get_or_init(|| {
        Regex::new(r"(?i)(?:arxiv\s*:\s*|arxiv\.org/(?:abs|pdf|e-print)/|10\.48550/arxiv\.)([0-9]{4}\.[0-9]{4,5}(?:v[1-9][0-9]*)?|[a-z-]+(?:\.[a-z-]+)?/[0-9]{7}(?:v[1-9][0-9]*)?)").unwrap()
    });
    let doi_re = DOI_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(10\.\d{4,9}/[^\s,\{\}"'<>\]\)]+)"#).unwrap()
    });
    let isbn_re = ISBN_RE.get_or_init(|| {
        Regex::new(r"(?i)\bisbn(?:-1[03])?\s*[:#]?\s*([0-9][0-9\- ]{8,16}[0-9Xx])\b").unwrap()
    });

    for cap in arxiv_re.captures_iter(line) {
        if let Ok((base, version)) = split_arxiv_id(&cap[1]) {
            if seen.arxiv.insert(base.clone()) {
                push_candidate(
                    ArxivCandidate { id: base, version },
                    page,
                    line_no,
                    source,
                    &mut out.arxiv_ids,
                );
            }
        }
    }

    for cap in doi_re.captures_iter(line) {
        if let Ok(doi) = canonical_doi(&cap[1]) {
            if let Some(rest) = doi.strip_prefix("10.48550/arxiv.") {
                if let Ok((base, version)) = split_arxiv_id(rest) {
                    if seen.arxiv.insert(base.clone()) {
                        push_candidate(
                            ArxivCandidate { id: base, version },
                            page,
                            line_no,
                            source,
                            &mut out.arxiv_ids,
                        );
                    }
                }
                continue;
            }
            if seen.dois.insert(doi.clone()) {
                push_candidate(doi, page, line_no, source, &mut out.dois);
            }
        }
    }

    for cap in isbn_re.captures_iter(line) {
        if let Some(isbn) = normalize_valid_isbn(&cap[1]) {
            if seen.isbns.insert(isbn.clone()) {
                push_candidate(isbn, page, line_no, source, &mut out.isbns);
            }
        }
    }
}

/// Extract heuristic title, authors, year, DOI, arXiv id, and ISBN candidates from the extracted
/// reading file (pages 1..=5, skipping blank pages and preferring content over repository cover
/// sheets). Every text-derived candidate records its exact 1-based line and page numbers.
pub fn extract_first_page_candidates(
    reading_text: &str,
    file_name: Option<&str>,
) -> FirstPageCandidates {
    let pages = split_numbered_pages(reading_text);
    let mut out = FirstPageCandidates::default();
    let mut seen = IdSeenSets::default();

    for (page_num, lines) in pages.iter().take(SCAN_PAGES_FOR_IDS) {
        for (line_no, text) in lines {
            scan_line_identifiers(
                text,
                *page_num,
                *line_no,
                CandidateSource::IdentifierScan,
                &mut seen,
                &mut out,
            );
        }
    }

    let mut cover_fallback: Option<ShapeReading> = None;
    let mut chosen: Option<ShapeReading> = None;
    for (page_num, lines) in pages.iter().take(SCAN_PAGES_FOR_TITLE) {
        let alpha_count: usize = lines
            .iter()
            .map(|(_, l)| l.chars().filter(|c| c.is_alphabetic()).count())
            .sum();
        if alpha_count < 20 {
            continue;
        }
        let is_cover = is_cover_sheet(lines);
        if let Some(shape) = read_single_page_shape(*page_num, lines, file_name) {
            if !is_cover {
                chosen = Some(shape);
                break;
            }
            if cover_fallback.is_none() {
                cover_fallback = Some(shape);
            }
        }
    }

    if let Some(shape) = chosen.or(cover_fallback) {
        out.title = Some(shape.title);
        out.authors = Some(shape.authors);
        out.year = shape.year;
    } else if let Some(fname) = file_name {
        let decoded = percent_decode_str(fname).decode_utf8_lossy();
        if let Some(y) = year_in(&decoded) {
            out.year = Some(FieldCandidate::from_non_text(y, CandidateSource::FileName));
        }
    }

    out
}

/// Merge text-extracted candidates (preferred) with `/Info` dictionary candidates.
pub fn merge_first_page_candidates(
    mut primary: FirstPageCandidates,
    supplementary: FirstPageCandidates,
) -> FirstPageCandidates {
    if primary.title.is_none() {
        primary.title = supplementary.title;
    }
    if primary.authors.is_none() {
        primary.authors = supplementary.authors;
    }
    if primary.year.is_none() {
        primary.year = supplementary.year;
    }
    let mut seen_dois: BTreeSet<String> = primary.dois.iter().map(|c| c.value.clone()).collect();
    for d in supplementary.dois {
        if seen_dois.insert(d.value.clone()) {
            primary.dois.push(d);
        }
    }
    let mut seen_ax: BTreeSet<String> = primary.arxiv_ids.iter().map(|c| c.value.id.clone()).collect();
    for a in supplementary.arxiv_ids {
        if seen_ax.insert(a.value.id.clone()) {
            primary.arxiv_ids.push(a);
        }
    }
    let mut seen_isbn: BTreeSet<String> = primary.isbns.iter().map(|c| c.value.clone()).collect();
    for i in supplementary.isbns {
        if seen_isbn.insert(i.value.clone()) {
            primary.isbns.push(i);
        }
    }
    primary
}

#[cfg(test)]
#[path = "pdf_firstpage_tests.rs"]
mod tests;
