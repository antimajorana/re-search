//! Heuristic line-pattern matchers for theorem-like headings, proofs, QED markers, captions,
//! and margin equation numbers in PDF reading files (`pdf-1`, Investigate V17).

use super::model::SkNodeKind;
use crate::pdf_pages::{PageBasis, PageMap, PAGE_BREAK_CHAR};
use crate::quote_core::clip_to_chars;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub(crate) struct TheoremMatch {
    pub kind: SkNodeKind,
    pub number: Option<String>,
    pub title: Option<String>,
    pub heading_end_byte: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ProofMatch {
    pub target: Option<(SkNodeKind, String)>,
    pub title: Option<String>,
    pub heading_end_byte: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CaptionMatch {
    pub kind: SkNodeKind,
    pub number: String,
    pub title: Option<String>,
}

/// Maps a singular/plural/abbreviated structural keyword from PDF text to [`SkNodeKind`].
pub(crate) fn map_pdf_kind_word(word: &str, allow_non_statement: bool) -> Option<SkNodeKind> {
    let norm = word.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    match norm.trim_end_matches('.') {
        "theorem" | "theorems" | "thm" | "thms" | "main theorem" => Some(SkNodeKind::Theorem),
        "lemma" | "lemmas" | "lem" | "lems" => Some(SkNodeKind::Lemma),
        "proposition" | "propositions" | "prop" | "props" => Some(SkNodeKind::Proposition),
        "corollary" | "corollaries" | "cor" | "cors" => Some(SkNodeKind::Corollary),
        "conjecture" | "conjectures" | "conj" => Some(SkNodeKind::Conjecture),
        "claim" | "claims" | "observation" | "observations" => Some(SkNodeKind::Claim),
        "definition" | "definitions" | "defn" | "defns" | "def" => Some(SkNodeKind::Definition),
        "assumption" | "assumptions" | "assump" | "hypothesis" | "hypotheses" | "axiom" | "axioms" => {
            Some(SkNodeKind::Assumption)
        }
        "remark" | "remarks" | "rem" | "rmk" if allow_non_statement => Some(SkNodeKind::Remark),
        "example" | "examples" | "exmp" | "ex" if allow_non_statement => Some(SkNodeKind::Example),
        "algorithm" | "algorithms" | "alg" if allow_non_statement => Some(SkNodeKind::Algorithm),
        "section" | "sections" | "sec" | "secs" | "appendix" | "appendices" | "app"
            if allow_non_statement =>
        {
            Some(SkNodeKind::Section)
        }
        "equation" | "equations" | "eqn" | "eqns" | "eq" | "eqs" if allow_non_statement => {
            Some(SkNodeKind::Equation)
        }
        "figure" | "figures" | "fig" | "figs" if allow_non_statement => Some(SkNodeKind::Figure),
        "table" | "tables" | "tab" | "tabs" if allow_non_statement => Some(SkNodeKind::Table),
        _ => None,
    }
}

/// Matches a theorem-like heading (`Theorem 2.1.`, `Lemma 1.2 (Bound on L^2(M)).`,
/// `Theorem 2.1 [1] (Title).`) while rejecting prose sentences (`"Theorem 2.1 shows..."`)
/// and bare EOL wraps followed by lowercase prose (`"Theorem 2.1\nshows..."`).
pub(crate) fn match_theorem_heading(norm: &str, next_active: Option<&str>) -> Option<TheoremMatch> {
    static THM_RE: OnceLock<Regex> = OnceLock::new();
    let re = THM_RE.get_or_init(|| {
        Regex::new(concat!(
            r"^(?i:(Main\s+Theorem|Theorem|Lemma|Proposition|Corollary|Conjecture|Claim|Observation|",
            r"Definition|Assumption|Hypothesis|Axiom|Remark|Example|Algorithm))",
            r"(?:\s+([A-Z]?\d+(?:\.\d+)*[a-z]?|[IVX]+))?",
            r"(?:\s*\[[^\[\]]{1,60}\])?",
            r"(?:\s*\(((?:[^()]|\([^()]*\)){1,120})\))?",
            r"(?:\s*\[[^\[\]]{1,60}\])?",
            r"(?:\s*([.:—–])(?:\s|$)|\s*$)",
        ))
        .unwrap()
    });
    let cap = re.captures(norm)?;
    let kind = map_pdf_kind_word(&cap[1], true)?;
    if !kind.is_theorem_like() {
        return None;
    }
    let number = cap.get(2).map(|m| m.as_str().trim().to_string());
    let title = cap.get(3).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty());
    let has_delim = cap.get(4).is_some();
    let full_end = cap.get(0).map_or(0, |m| m.end());
    if number.is_none() && title.is_none() && !has_delim {
        return None;
    }
    // Without a trailing delimiter, reject wrapped prose like "Theorem 2.1\nshows that...".
    if !has_delim && next_starts_lowercase(next_active) {
        return None;
    }
    Some(TheoremMatch {
        kind,
        number,
        title,
        heading_end_byte: full_end,
    })
}

/// Matches proof headings (`Proof.`, `Proof of Theorem 2.1.`, `Proof of Theorem 2.1 and Lemma 2.2.`,
/// `Proof sketch.`) while rejecting prose (`"Proof techniques..."` or `"Proof\nof the..."`).
pub(crate) fn match_proof_heading(norm: &str, next_active: Option<&str>) -> Option<ProofMatch> {
    static PROOF_OF_RE: OnceLock<Regex> = OnceLock::new();
    static PROOF_MULTI_RE: OnceLock<Regex> = OnceLock::new();
    static PROOF_PLAIN_RE: OnceLock<Regex> = OnceLock::new();

    let of_re = PROOF_OF_RE.get_or_init(|| {
        Regex::new(concat!(
            r"^(?i:(?:Sketch\s+of\s+)?Proof\s+(?:\(\s*)?of\s+)",
            r"(?i:(Main\s+Theorem|Theorem|Thm\.?|Lemma|Lem\.?|Proposition|Prop\.?|Corollary|Cor\.?|",
            r"Conjecture|Conj\.?|Claim|Definition|Defn\.?|Assumption))",
            r"\s+([A-Z]?\d+(?:\.\d+)*[a-z]?|[IVX]+)",
            r"(?:\s*\))?",
            r"(?:\s*\((?:[^()]|\([^()]*\)){1,80}\))?",
            r"(?:\s*\[[^\[\]]{1,60}\])?",
            r"(?:\s*([.:—–])(?:\s|$)|\s*$)",
        ))
        .unwrap()
    });
    if let Some(cap) = of_re.captures(norm) {
        if cap.get(3).is_none() && next_starts_lowercase(next_active) {
            return None;
        }
        let kind = map_pdf_kind_word(&cap[1], false)?;
        let num = cap[2].trim().to_string();
        let full = cap.get(0).unwrap();
        let title = full.as_str().trim().trim_end_matches(['.', ':', '—', '–']).trim().to_string();
        return Some(ProofMatch {
            target: Some((kind, num)),
            title: (!title.is_empty()).then_some(title),
            heading_end_byte: full.end(),
        });
    }

    // Joint proof heading ("Proof of Theorem 2.1 and Proposition 2.2."): recognized as a Proof
    // node with `target: None` so rule 6.2 ("each proof proves at most one node") stays intact.
    let multi_re = PROOF_MULTI_RE.get_or_init(|| {
        Regex::new(concat!(
            r"^(?i:(?:Sketch\s+of\s+)?Proof\s+of\s+[^.:—–]{3,90}?\s+and\s+[^.:—–]{3,90}?)",
            r"(?:\s*[.:—–](?:\s|$))",
        ))
        .unwrap()
    });
    if let Some(cap) = multi_re.captures(norm) {
        let full = cap.get(0).unwrap();
        let title = cap[1].trim().to_string();
        return Some(ProofMatch {
            target: None,
            title: (!title.is_empty()).then_some(title),
            heading_end_byte: full.end(),
        });
    }

    let plain_re = PROOF_PLAIN_RE.get_or_init(|| {
        Regex::new(
            r"^(?i:(?:Sketch\s+of\s+proof|Proof\s+sketch|Proof))(?:\s*\(((?:[^()]|\([^()]*\)){1,80})\))?(?:\s*([.:—–])(?:\s|$)|\s*$)",
        )
        .unwrap()
    });
    let cap = plain_re.captures(norm)?;
    if cap.get(2).is_none() && next_starts_lowercase(next_active) {
        return None;
    }
    let full = cap.get(0).unwrap();
    let title = cap.get(1).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty());
    Some(ProofMatch {
        target: None,
        title,
        heading_end_byte: full.end(),
    })
}

fn next_starts_lowercase(next_active: Option<&str>) -> bool {
    next_active
        .and_then(|s| s.trim_start().chars().next())
        .is_some_and(char::is_lowercase)
}

/// True when a line ends with a conventional end-of-proof marker (`□`, `■`, `∎`, `Q.E.D.`).
pub(crate) fn has_qed_marker(norm: &str) -> bool {
    let t = norm.trim_end();
    t.ends_with('□')
        || t.ends_with('■')
        || t.ends_with('∎')
        || t.ends_with("Q.E.D.")
        || t.ends_with("QED")
        || t.ends_with("Q.E.D")
}

/// Matches figure and table captions (`Figure 1: ...`, `Fig. 2. ...`, `Table I: ...`).
pub(crate) fn match_caption_heading(norm: &str) -> Option<CaptionMatch> {
    static FIG_RE: OnceLock<Regex> = OnceLock::new();
    static TAB_RE: OnceLock<Regex> = OnceLock::new();
    let fig_re = FIG_RE.get_or_init(|| {
        Regex::new(r"^(?:FIG\.|Fig\.|Figure|FIGURE)\s+([A-Za-z]?\d+(?:\.\d+)?[a-z]?)\s*[:.\-–—]\s*(.*)$").unwrap()
    });
    let tab_re = TAB_RE.get_or_init(|| {
        Regex::new(r"^(?:TABLE|Table|TAB\.|Tab\.)\s+([A-Za-z]?\d+(?:\.\d+)?[a-z]?|[IVX]+)\s*[:.\-–—]\s*(.*)$").unwrap()
    });
    if let Some(cap) = fig_re.captures(norm) {
        let title = cap[2].trim();
        return Some(CaptionMatch {
            kind: SkNodeKind::Figure,
            number: cap[1].trim().to_string(),
            title: (!title.is_empty()).then(|| clip_to_chars(title, 140).trim_end().to_string()),
        });
    }
    if let Some(cap) = tab_re.captures(norm) {
        let title = cap[2].trim();
        return Some(CaptionMatch {
            kind: SkNodeKind::Table,
            number: cap[1].trim().to_string(),
            title: (!title.is_empty()).then(|| clip_to_chars(title, 140).trim_end().to_string()),
        });
    }
    None
}

/// Matches a margin equation number (`... = ... (3.4)` or standalone `(3.4)` after a math line)
/// while rejecting 4-digit years, prose wrapped after `"Eq. (3.4)"`, and lines above `min_start_line`.
pub(crate) fn match_margin_equation(
    lines: &[&str],
    active: &[bool],
    page_map: &PageMap,
    idx0: usize,
    min_start_line: usize,
) -> Option<(String, usize)> {
    static EQ_RE: OnceLock<Regex> = OnceLock::new();
    let eq_re = EQ_RE.get_or_init(|| Regex::new(r"\(([A-Za-z]?\d+(?:\.\d+)*[a-z]?)\)\s*$").unwrap());

    let trimmed = lines[idx0].trim();
    let cap = eq_re.captures(trimmed)?;
    let num = cap[1].to_string();
    if is_likely_year(&num) {
        return None;
    }
    let m = cap.get(0).unwrap();
    let before = &trimmed[..m.start()];
    let prefix = before.trim_end();
    let line_no = idx0 + 1;

    if prefix.is_empty() {
        let cur_page = page_map.page_of(line_no);
        let prev_idx = (0..idx0)
            .rev()
            .find(|&k| active[k] && !lines[k].trim().is_empty())?;
        let prev_line_no = prev_idx + 1;
        if prev_line_no < min_start_line {
            return None;
        }
        if cur_page.is_some() && page_map.page_of(prev_line_no) != cur_page {
            return None;
        }
        if !is_display_math_candidate(lines[prev_idx].trim()) {
            return None;
        }
        return Some((num, prev_line_no));
    }

    if ends_with_ref_cue(prefix) {
        return None;
    }
    let wide_gap = before.len().saturating_sub(prefix.len()) >= 3;
    if looks_math_line(prefix) || (wide_gap && !looks_like_plain_prose_sentence(prefix)) {
        Some((num, line_no))
    } else {
        None
    }
}

pub(crate) fn looks_math_line(s: &str) -> bool {
    s.contains("||")
        || [
            '=', '+', '−', '<', '>', '≤', '≥', '≠', '≈', '≡', '∼', '∑', '∫', '∏', '→', '↦', '⊗',
            '⊕', '∈', '⊂', '∂', '∇', '∝', '×', '±', '·', '∞', '∥', '∀', '∃', '^', '_', '\\',
        ]
        .iter()
        .any(|&ch| s.contains(ch))
}

/// Stricter check used for standalone `(N)` lines and bottom-of-page `(N)` footer guards:
/// requires math symbols AND rejects prose sentences containing an incidental `=` or `+`.
pub(crate) fn is_display_math_candidate(s: &str) -> bool {
    looks_math_line(s) && !looks_like_plain_prose_sentence(s)
}

fn looks_like_plain_prose_sentence(s: &str) -> bool {
    let prose_words = s
        .split_whitespace()
        .filter(|w| {
            let clean = w.trim_matches(|c: char| !c.is_alphabetic());
            clean.len() >= 2 && clean.chars().all(|c| c.is_ascii_alphabetic())
        })
        .count();
    prose_words >= 5 && (s.ends_with('.') || s.ends_with(',') || s.ends_with(';'))
}

fn ends_with_ref_cue(prefix: &str) -> bool {
    let last = prefix
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_end_matches([',', ';'])
        .to_ascii_lowercase();
    matches!(
        last.as_str(),
        "eq." | "eq" | "eqs." | "eqs" | "eqn." | "eqn" | "eqns." | "eqns"
            | "equation" | "equations" | "ref." | "ref" | "refs." | "refs"
            | "see" | "cf." | "by" | "from" | "via" | "in" | "of" | "to"
            | "and" | "or" | "with" | "using" | "item" | "case" | "step" | "part"
    )
}

fn is_likely_year(label: &str) -> bool {
    label.len() == 4
        && label.bytes().all(|b| b.is_ascii_digit())
        && (label.starts_with("19") || label.starts_with("20"))
}

/// Detects standalone page numbers (top-2 and bottom-2 non-empty lines per page) and repeated
/// or alternating odd/even running headers/footers.
pub(crate) fn detect_headers_footers(lines: &[&str], page_map: &PageMap) -> Vec<bool> {
    let mut flags = vec![false; lines.len()];
    let mut edge_counts: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut page_edges: Vec<(usize, usize, String)> = Vec::new();

    for rec in &page_map.pages {
        let (Some(first), Some(last)) = (rec.first_line, rec.last_line) else {
            continue;
        };
        if first == 0 || last < first || first > lines.len() {
            continue;
        }
        let lo = first - 1;
        let hi = last.min(lines.len()) - 1;
        let top2: Vec<usize> = (lo..=hi).filter(|&i| !lines[i].trim().is_empty()).take(2).collect();
        let bot2: Vec<usize> = (lo..=hi)
            .rev()
            .filter(|&i| !lines[i].trim().is_empty())
            .take(2)
            .collect();

        let mut inspected = BTreeSet::new();
        for (rank, &idx) in top2.iter().enumerate() {
            inspected.insert(idx);
            let t = lines[idx].trim();
            if is_standalone_page_number(t, false, lines, idx) {
                flags[idx] = true;
            } else if rank == 0 {
                record_header_sig(t, idx, rec.page, &mut edge_counts, &mut page_edges);
            }
        }
        for (rank, &idx) in bot2.iter().enumerate() {
            let t = lines[idx].trim();
            if is_standalone_page_number(t, true, lines, idx) {
                flags[idx] = true;
            } else if rank == 0 && !inspected.contains(&idx) {
                record_header_sig(t, idx, rec.page, &mut edge_counts, &mut page_edges);
            }
        }
    }

    for (idx, _page, sig) in page_edges {
        if edge_counts.get(&sig).is_some_and(|pages| pages.len() >= 2) {
            flags[idx] = true;
        }
    }
    flags
}

fn record_header_sig(
    trimmed: &str,
    idx: usize,
    page: usize,
    edge_counts: &mut BTreeMap<String, BTreeSet<usize>>,
    page_edges: &mut Vec<(usize, usize, String)>,
) {
    if looks_structural_keyword(trimmed) {
        return;
    }
    let sig = normalize_header_sig(trimmed);
    if sig.len() >= 3 && sig.len() <= 100 {
        edge_counts.entry(sig.clone()).or_default().insert(page);
        page_edges.push((idx, page, sig));
    }
}

fn is_standalone_page_number(trimmed: &str, is_bottom: bool, lines: &[&str], idx: usize) -> bool {
    static PAGE_NUM_RE: OnceLock<Regex> = OnceLock::new();
    let re = PAGE_NUM_RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)^(?:[-–—]\s*\d{1,5}\s*[-–—]|\d{1,5}|",
            r"(?:page|p\.?)\s*\d{1,5}(?:\s*(?:of|/)\s*\d{1,5})?|",
            r"\d{1,5}\s*/\s*\d{1,5}|[ivxlc]{1,6})$",
        ))
        .unwrap()
    });
    if re.is_match(trimmed) {
        return true;
    }
    static PAREN_FOOTER_RE: OnceLock<Regex> = OnceLock::new();
    let pf_re = PAREN_FOOTER_RE.get_or_init(|| Regex::new(r"^\(\d{1,5}\)$").unwrap());
    if is_bottom && pf_re.is_match(trimmed) {
        let prev_math = (0..idx)
            .rev()
            .find(|&p| !lines[p].trim().is_empty())
            .is_some_and(|p| is_display_math_candidate(lines[p].trim()));
        return !prev_math;
    }
    false
}

/// Normalizes a top/bottom edge line to detect both repeated and alternating odd/even running
/// headers (e.g., `"2  Paper Title"` on even pages and `"Paper Title  3"` on odd pages).
fn normalize_header_sig(trimmed: &str) -> String {
    static EDGE_NUM_RE: OnceLock<Regex> = OnceLock::new();
    let re = EDGE_NUM_RE.get_or_init(|| Regex::new(r"^(?:\d{1,5}\s+)(.+)$|^(.+?)(?:\s+\d{1,5})$").unwrap());
    let mut s = trimmed.trim().to_string();
    for _ in 0..2 {
        if let Some(cap) = re.captures(&s) {
            let stripped = cap.get(1).or_else(|| cap.get(2)).map_or("", |m| m.as_str().trim());
            if stripped.len() >= 3 {
                s = stripped.to_string();
                continue;
            }
        }
        break;
    }
    let mut out = String::with_capacity(s.len());
    let mut in_digits = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                out.push('#');
                in_digits = true;
            }
        } else {
            in_digits = false;
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn looks_structural_keyword(trimmed: &str) -> bool {
    let low = trimmed.to_ascii_lowercase();
    [
        "proof", "theorem", "lemma", "proposition", "corollary", "definition", "assumption",
        "conjecture", "claim", "remark", "example", "algorithm", "abstract", "references",
        "bibliography", "figure", "fig.", "table", "tab.", "appendix", "section",
    ]
    .iter()
    .any(|kw| low.starts_with(kw))
}

/// Marks which lines are eligible for structural detection (excluding `\x0c`, headers/footers,
/// and all lines belonging to `PageBasis::None` image-only pages).
pub(crate) fn compute_active_lines(
    lines: &[&str],
    page_map: &PageMap,
    header_footer: &[bool],
) -> Vec<bool> {
    let has_records = !page_map.pages.is_empty();
    lines
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            if raw.contains(PAGE_BREAK_CHAR) || header_footer[i] {
                return false;
            }
            if !has_records {
                return true;
            }
            page_map
                .record_of_line(i + 1)
                .is_some_and(|rec| rec.basis != PageBasis::None)
        })
        .collect()
}
