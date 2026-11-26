//! Numbered bibliography and citation extraction for the PDF skeleton (`pdf-1`, Investigate V17).
//!
//! Parses numbered reference entries (`[1] ...` or `1. ...`) from the bibliography span(s) of
//! the pinned F14a reading file, extracts canonical DOIs and arXiv IDs via `tex_scan` and F2
//! `paper_key`, and scans active body lines for numbered citations (`[12]`, `[1, 2]`, `[1-3]`,
//! `[12, Theorem 2.1]`) while guarding against math indexing, intervals, and ambiguous entries.

use super::model::{BibLocator, BibRef, SkNode, Span};
use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::pdf_firstpage::normalize_pdf_unicode;
use crate::tex_scan::{scan_arxiv_base_ids, scan_normalized_dois};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const MAX_RAW_CHARS: usize = 2_000;
const MAX_CITE_RANGE_EXPAND: usize = 20;
const MAX_UNLISTED_CITE_NUM: usize = 500;

/// One active numbered citation occurrence in the PDF reading file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PdfCiteOccurrence {
    pub line_no: usize,
    pub byte_start: usize,
    pub site: Span,
    pub key: String,
    pub locator: Option<String>,
}

/// Result of PDF bibliography and citation extraction.
#[derive(Debug, Clone)]
pub(crate) struct PdfBibResult {
    pub bib: Vec<BibRef>,
    pub citations: Vec<PdfCiteOccurrence>,
}

/// Extracts numbered bibliography entries, resolves DOIs/arXiv IDs, scans active body lines for
/// numbered citations, and populates `SkNode::head_cites` on nodes whose headings cite keys.
pub(crate) fn extract_pdf_bibliography(
    primary_path: &str,
    lines: &[&str],
    active_lines: &[bool],
    bib_ranges: &[(usize, usize)],
    nodes: &mut [SkNode],
    node_heading_end_bytes: &BTreeMap<usize, usize>,
) -> PdfBibResult {
    let (mut entries, ambiguous_keys) = parse_bib_sections(lines, active_lines, bib_ranges);
    let has_parsed_bib = !entries.is_empty() || !ambiguous_keys.is_empty();
    let known_keys: BTreeSet<String> = entries.keys().cloned().collect();

    let mut citations = Vec::new();
    for (idx0, raw_line) in lines.iter().enumerate() {
        let line_no = idx0 + 1;
        if !active_lines[idx0] || in_any_range(line_no, bib_ranges) {
            continue;
        }
        if !raw_line.contains('[') && !raw_line.contains('\u{FF3B}') {
            continue;
        }
        let norm_trimmed = normalize_pdf_unicode(raw_line.trim());
        let site = Span::new(primary_path, line_no, line_no);
        for hit in scan_line_citations(&norm_trimmed, has_parsed_bib, &known_keys, &ambiguous_keys) {
            let occ = PdfCiteOccurrence {
                line_no,
                byte_start: hit.byte_start,
                site: site.clone(),
                key: hit.key.clone(),
                locator: hit.locator.clone(),
            };
            citations.push(occ);

            let entry = entries
                .entry(hit.key.clone())
                .or_insert_with(|| empty_bib_ref(&hit.key));
            if !entry.cited_at.contains(&site) {
                entry.cited_at.push(site.clone());
            }
            if let Some(loc_text) = hit.locator {
                let loc = BibLocator { site: site.clone(), text: loc_text };
                if !entry.locators.contains(&loc) {
                    entry.locators.push(loc);
                }
            }
        }
    }

    // Populate `head_cites` for citations that sit inside a node's heading prefix on its start line.
    for (idx, node) in nodes.iter_mut().enumerate() {
        let Some(&head_end) = node_heading_end_bytes.get(&idx) else {
            continue;
        };
        let Some(first) = node.spans.first() else {
            continue;
        };
        for occ in citations.iter().filter(|c| c.line_no == first.start_line) {
            if occ.byte_start < head_end && !node.head_cites.contains(&occ.key) {
                node.head_cites.push(occ.key.clone());
            }
        }
    }

    // Sort bibliography entries numerically when numeric, then lexicographically.
    let mut bib: Vec<BibRef> = entries.into_values().collect();
    bib.sort_by(|a, b| {
        let ka = a.key.parse::<usize>().ok();
        let kb = b.key.parse::<usize>().ok();
        match (ka, kb) {
            (Some(na), Some(nb)) => na.cmp(&nb).then_with(|| a.key.cmp(&b.key)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.key.cmp(&b.key),
        }
    });

    PdfBibResult { bib, citations }
}

fn in_any_range(line_no: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(s, e)| (s..=e).contains(&line_no))
}

fn empty_bib_ref(key: &str) -> BibRef {
    BibRef {
        key: key.to_string(),
        raw: String::new(),
        arxiv_id: None,
        doi: None,
        title: None,
        cited_at: Vec::new(),
        locators: Vec::new(),
    }
}

fn parse_bib_sections(
    lines: &[&str],
    active_lines: &[bool],
    bib_ranges: &[(usize, usize)],
) -> (BTreeMap<String, BibRef>, BTreeSet<String>) {
    static BRACKET_RE: OnceLock<Regex> = OnceLock::new();
    static DOTTED_RE: OnceLock<Regex> = OnceLock::new();
    let bracket_re = BRACKET_RE.get_or_init(|| Regex::new(r"^\[(\d{1,4})\](?:\s+(.*)|\s*)$").unwrap());
    let dotted_re = DOTTED_RE.get_or_init(|| Regex::new(r"^(\d{1,4})\.\s+(.+)$").unwrap());

    let mut entries: BTreeMap<String, BibRef> = BTreeMap::new();
    let mut ambiguous: BTreeSet<String> = BTreeSet::new();

    for &(start, end) in bib_ranges {
        let mut raw_lines: Vec<String> = Vec::new();
        for ln in (start + 1)..=end.min(lines.len()) {
            if !active_lines[ln - 1] {
                continue;
            }
            let t = lines[ln - 1].trim();
            if !t.is_empty() {
                raw_lines.push(t.to_string());
            }
        }
        if raw_lines.is_empty() {
            continue;
        }

        let has_bracketed = raw_lines.iter().any(|l| bracket_re.is_match(l));
        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        let mut last_dotted = 0usize;

        for line in raw_lines {
            if has_bracketed {
                if let Some(cap) = bracket_re.captures(&line) {
                    let key_num: usize = cap[1].parse().unwrap_or(0);
                    if key_num >= 1 {
                        let rest = cap.get(2).map_or("", |m| m.as_str().trim());
                        let mut parts = Vec::new();
                        if !rest.is_empty() {
                            parts.push(rest.to_string());
                        }
                        grouped.push((key_num.to_string(), parts));
                        continue;
                    }
                }
            } else if let Some(cap) = dotted_re.captures(&line) {
                let key_num: usize = cap[1].parse().unwrap_or(0);
                let step_ok = (last_dotted == 0 && key_num == 1)
                    || (last_dotted > 0 && (key_num == last_dotted + 1 || key_num == last_dotted + 2));
                if step_ok {
                    last_dotted = key_num;
                    grouped.push((key_num.to_string(), vec![cap[2].trim().to_string()]));
                    continue;
                }
            }
            if let Some((_, parts)) = grouped.last_mut() {
                parts.push(line);
            }
        }

        for (key, parts) in grouped {
            let joined_raw = parts.join(" ");
            let id_scan_text = join_for_id_scan(&parts);
            let doi = extract_canonical_doi(&id_scan_text);
            let arxiv_id = extract_canonical_arxiv(&id_scan_text, doi.as_deref());
            let title = extract_pdf_bib_title(&joined_raw);

            if entries.contains_key(&key) {
                // Duplicate numeric key in the bibliography: mark ambiguous so citations never guess.
                ambiguous.insert(key);
                continue;
            }
            entries.insert(
                key.clone(),
                BibRef {
                    key,
                    raw: cap_raw(&joined_raw),
                    arxiv_id,
                    doi,
                    title,
                    cited_at: Vec::new(),
                    locators: Vec::new(),
                },
            );
        }
    }

    (entries, ambiguous)
}

/// Joins wrapped bibliography lines while repairing line-end hyphens/slashes/dots in DOIs and
/// arXiv IDs solely for identifier detection (stored text is never altered).
fn join_for_id_scan(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let norm = normalize_pdf_unicode(part.trim());
        if out.is_empty() {
            out.push_str(&norm);
            continue;
        }
        let next_alnum = norm.chars().next().is_some_and(char::is_alphanumeric);
        let in_id_token = out
            .split_whitespace()
            .last()
            .is_some_and(|tok| {
                let low = tok.to_ascii_lowercase();
                low.contains("10.") || low.contains("doi") || low.contains("arxiv")
            });
        let join_tight = out.ends_with('/')
            || out.ends_with(':')
            || ((out.ends_with('-') || (out.ends_with('.') && in_id_token)) && next_alnum);
        if join_tight {
            out.push_str(&norm);
        } else {
            out.push(' ');
            out.push_str(&norm);
        }
    }
    out
}

fn extract_canonical_doi(text: &str) -> Option<String> {
    scan_normalized_dois(text)
        .into_iter()
        .find_map(|cand| canonical_doi(&cand).ok())
}

fn extract_canonical_arxiv(text: &str, matched_doi: Option<&str>) -> Option<String> {
    if let Some(doi) = matched_doi {
        if let Some(rest) = doi.strip_prefix("10.48550/arxiv.") {
            if let Ok((base, _)) = split_arxiv_id(rest) {
                return Some(base);
            }
        }
    }
    // Strip matched non-arXiv DOIs before scanning bare arXiv IDs so numeric DOI suffixes cannot
    // masquerade as arXiv identifiers.
    let mut cleaned = text.to_string();
    for raw_doi in scan_normalized_dois(text) {
        if !raw_doi.starts_with("10.48550/arxiv.") {
            loop {
                let lower = cleaned.to_ascii_lowercase();
                let Some(pos) = lower.find(&raw_doi) else {
                    break;
                };
                let end = pos + raw_doi.len();
                if cleaned.is_char_boundary(pos) && cleaned.is_char_boundary(end) {
                    cleaned.replace_range(pos..end, " ");
                } else {
                    break;
                }
            }
        }
    }
    scan_arxiv_base_ids(&cleaned)
        .into_iter()
        .find_map(|cand| split_arxiv_id(&cand).ok().map(|(base, _)| base))
}

fn extract_pdf_bib_title(raw: &str) -> Option<String> {
    for (open, close) in [('"', '"'), ('\u{201C}', '\u{201D}')] {
        if let Some(start) = raw.find(open) {
            let rest = &raw[start + open.len_utf8()..];
            if let Some(end) = rest.find(close) {
                let candidate = rest[..end].trim().trim_end_matches([',', '.']).trim();
                if candidate.len() >= 4 && candidate.len() <= 240 {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

fn cap_raw(raw: &str) -> String {
    crate::quote_core::clip_to_chars(raw.trim(), MAX_RAW_CHARS)
        .trim_end()
        .to_string()
}

#[derive(Debug, Clone)]
struct LineCiteHit {
    byte_start: usize,
    key: String,
    locator: Option<String>,
}

fn scan_line_citations(
    raw_line: &str,
    has_parsed_bib: bool,
    known_keys: &BTreeSet<String>,
    ambiguous_keys: &BTreeSet<String>,
) -> Vec<LineCiteHit> {
    static BRACKET_RE: OnceLock<Regex> = OnceLock::new();
    let re = BRACKET_RE.get_or_init(|| Regex::new(r"\[([^\[\]]{1,80})\]").unwrap());

    let bytes = raw_line.as_bytes();
    let mut hits = Vec::new();
    for cap in re.captures_iter(raw_line) {
        let full = cap.get(0).unwrap();
        let start = full.start();
        if start > 0 {
            let prev_byte = bytes[start - 1];
            if prev_byte.is_ascii_alphanumeric() || matches!(prev_byte, b'_' | b')' | b']') {
                continue;
            }
        }
        let inner = normalize_pdf_unicode(cap[1].trim());
        if inner.is_empty() {
            continue;
        }
        let before_trimmed = raw_line[..start].trim_end();
        let after_trimmed = raw_line[full.end()..].trim_start();
        let math_interval = before_trimmed.chars().last().is_some_and(|c| {
            matches!(
                c,
                '∈' | '∉' | '⊂' | '⊆' | '=' | '<' | '>' | '≤' | '≥' | '+' | '−' | '×'
                    | '→' | '↦' | '∪' | '∩' | '⊕' | '⊗'
            )
        }) || after_trimmed.chars().next().is_some_and(|c| {
            matches!(c, '×' | '∩' | '∪' | '⊂' | '⊆' | '→' | '↦' | '=' | '≠')
        });

        if let Some((keys, locator)) = parse_bracket_citation(&inner, math_interval) {
            for k in keys {
                if ambiguous_keys.contains(&k) {
                    continue;
                }
                if has_parsed_bib {
                    if !known_keys.contains(&k) {
                        continue;
                    }
                } else {
                    let n: usize = k.parse().unwrap_or(0);
                    if n == 0 || n > MAX_UNLISTED_CITE_NUM {
                        continue;
                    }
                }
                hits.push(LineCiteHit {
                    byte_start: start,
                    key: k,
                    locator: locator.clone(),
                });
            }
        }
    }
    hits
}

fn parse_bracket_citation(inner: &str, math_interval_prefix: bool) -> Option<(Vec<String>, Option<String>)> {
    static LOCATOR_RE: OnceLock<Regex> = OnceLock::new();
    let loc_re = LOCATOR_RE.get_or_init(|| {
        Regex::new(concat!(
            r"^(\d{1,4})\s*[,;]\s*(",
            r"(?i:(?:theorem|thm\.?|lemma|lem\.?|proposition|prop\.?|corollary|cor\.?|",
            r"definition|defn\.?|def\.?|section|sec\.?|chapter|ch\.?|equation|eqn\.?|eq\.?|",
            r"remark|rem\.?|example|ex\.?|appendix|app\.?|figure|fig\.?|table|tab\.?|pp?\.?)\s+.+)",
            r")$",
        ))
        .unwrap()
    });

    if let Some(cap) = loc_re.captures(inner) {
        let num: usize = cap[1].parse().ok()?;
        if num == 0 {
            return None;
        }
        let loc = cap[2].trim().to_string();
        return Some((vec![num.to_string()], (!loc.is_empty()).then_some(loc)));
    }

    if math_interval_prefix {
        return None;
    }

    // Pure numeric list / range: `12`, `1, 2, 5`, `1-3`, `1–3`, `1, 3-5`.
    let mut keys = Vec::new();
    for part in inner.split(',') {
        let p = part.trim();
        if p.is_empty() {
            return None;
        }
        if let Some((a_s, b_s)) = split_range(p) {
            let a: usize = a_s.trim().parse().ok()?;
            let b: usize = b_s.trim().parse().ok()?;
            if a == 0 || b <= a || (b - a) > MAX_CITE_RANGE_EXPAND {
                return None;
            }
            for k in a..=b {
                keys.push(k.to_string());
            }
        } else {
            if !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n: usize = p.parse().ok()?;
            if n == 0 {
                return None;
            }
            keys.push(n.to_string());
        }
    }
    (!keys.is_empty()).then_some((keys, None))
}

fn split_range(s: &str) -> Option<(&str, &str)> {
    for sep in ['-', '\u{2013}', '\u{2014}'] {
        if let Some((a, b)) = s.split_once(sep) {
            if !a.contains(sep) && !b.contains(sep) {
                return Some((a, b));
            }
        }
    }
    None
}
