//! Section-heading detection for the PDF skeleton (`pdf-1`, V17).
//!
//! Selects dotted Arabic, APS Roman, appendix, numbered bibliography, and fixed section headings
//! from active PDF lines (including page ranges beginning after page 1).

use super::model::SkNodeKind;
use crate::pdf_firstpage::normalize_pdf_unicode;
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub(crate) struct SelectedSection {
    pub line_no: usize,
    pub kind: SkNodeKind,
    pub number: Option<String>,
    pub title: String,
    pub level: u8,
}

#[derive(Debug, Clone)]
enum SecCandKind {
    Roman(usize, String),
    Letter(u8, char),
    Dotted(Vec<usize>),
    Appendix(String, Option<Vec<usize>>),
    AppendixBanner,
    Fixed(&'static str, FixedSecRole),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixedSecRole {
    FrontMatter,
    Section,
    Bibliography,
    Appendix,
}

#[derive(Debug, Clone)]
struct SecCandidate {
    line_no: usize,
    kind: SecCandKind,
    title: String,
    ends_with_dot: bool,
}

/// Selects structural sections/front-matter/bibliography headings from active PDF lines.
pub(crate) fn select_sections(
    lines: &[&str],
    active: &[bool],
    starts_after_page_one: bool,
) -> Vec<SelectedSection> {
    let cands = scan_section_candidates(lines, active);
    let roman_count = count_valid_roman_chain(&cands, starts_after_page_one);
    let dotted_multi = cands
        .iter()
        .filter(|c| matches!(&c.kind, SecCandKind::Dotted(v) if v.len() >= 2))
        .count();
    let dotted_top = count_valid_dotted_top(lines, &cands, starts_after_page_one);

    if roman_count >= 2 && roman_count >= dotted_multi {
        assemble_aps_sections(&cands, starts_after_page_one)
    } else if dotted_multi >= 1 || dotted_top >= 2 || (starts_after_page_one && dotted_top >= 1) {
        assemble_dotted_sections(lines, &cands, starts_after_page_one)
    } else if roman_count >= 1 {
        assemble_aps_sections(&cands, starts_after_page_one)
    } else if dotted_top == 1 {
        assemble_dotted_sections(lines, &cands, starts_after_page_one)
    } else {
        assemble_fixed_sections(&cands)
    }
}

fn is_bib_heading_title(title: &str) -> bool {
    let clean = title.trim().trim_end_matches(':').trim().to_ascii_lowercase();
    matches!(clean.as_str(), "references" | "bibliography" | "works cited")
}

fn scan_section_candidates(lines: &[&str], active: &[bool]) -> Vec<SecCandidate> {
    static ROMAN_RE: OnceLock<Regex> = OnceLock::new();
    static LETTER_RE: OnceLock<Regex> = OnceLock::new();
    static APP_RE: OnceLock<Regex> = OnceLock::new();
    static APP_SUB_RE: OnceLock<Regex> = OnceLock::new();
    static DOTTED_RE: OnceLock<Regex> = OnceLock::new();

    let roman_re = ROMAN_RE.get_or_init(|| {
        Regex::new(r"^(I|II|III|IV|V|VI|VII|VIII|IX|X|XI|XII|XIII|XIV|XV|XVI|XVII|XVIII|XIX|XX)\.\s+(.+)$").unwrap()
    });
    let letter_re = LETTER_RE.get_or_init(|| Regex::new(r"^([A-Z])\.\s+(.+)$").unwrap());
    let app_re = APP_RE.get_or_init(|| {
        Regex::new(r"(?i)^appendix\s+([A-Z]|\d+)(?:\s*[:.\-–—]\s*(.*)|\s+(.+))?$").unwrap()
    });
    let app_sub_re = APP_SUB_RE.get_or_init(|| {
        Regex::new(r"^([A-Z])\.(\d+(?:\.\d+)*)\.?\s+(.+)$").unwrap()
    });
    let dotted_re = DOTTED_RE.get_or_init(|| Regex::new(r"^(\d+(?:\.\d+)*)\.?\s+(.+)$").unwrap());

    let mut out = Vec::new();
    let mut in_refs = false;

    for (idx, raw) in lines.iter().enumerate() {
        if !active[idx] {
            continue;
        }
        let line_no = idx + 1;
        let trimmed = normalize_pdf_unicode(raw.trim());
        if trimmed.is_empty() || trimmed.len() > 140 {
            continue;
        }
        if let Some((title, role)) = match_fixed_heading(&trimmed) {
            in_refs = role == FixedSecRole::Bibliography;
            out.push(SecCandidate {
                line_no,
                kind: SecCandKind::Fixed(title, role),
                title: title.to_string(),
                ends_with_dot: false,
            });
            continue;
        }
        if trimmed.eq_ignore_ascii_case("APPENDIX") || trimmed.eq_ignore_ascii_case("APPENDICES") {
            in_refs = false;
            out.push(SecCandidate {
                line_no,
                kind: SecCandKind::AppendixBanner,
                title: trimmed,
                ends_with_dot: false,
            });
            continue;
        }
        if let Some(cap) = app_re.captures(&trimmed) {
            let id = cap[1].to_uppercase();
            let title = cap.get(2).or_else(|| cap.get(3)).map_or("", |m| m.as_str().trim()).to_string();
            if title.is_empty() || looks_like_heading_title(&title, true) {
                in_refs = false;
                out.push(SecCandidate {
                    line_no,
                    kind: SecCandKind::Appendix(id, None),
                    title,
                    ends_with_dot: false,
                });
                continue;
            }
        }
        if in_refs {
            continue;
        }
        if let Some(cap) = app_sub_re.captures(&trimmed) {
            let letter = cap[1].to_string();
            let nums: Option<Vec<usize>> = cap[2].split('.').map(|s| s.parse().ok()).collect();
            let title = cap[3].trim();
            if let Some(nums) = nums.filter(|v| v.iter().all(|&n| (1..=50).contains(&n))) {
                if looks_like_heading_title(title, false) && !preceded_by_list_colon(lines, active, idx) {
                    out.push(SecCandidate {
                        line_no,
                        kind: SecCandKind::Appendix(letter, Some(nums)),
                        title: title.to_string(),
                        ends_with_dot: ends_with_sentence_punct(title),
                    });
                    continue;
                }
            }
        }
        if let Some(cap) = roman_re.captures(&trimmed) {
            let rom = &cap[1];
            let title = cap[2].trim();
            if is_bib_heading_title(title) {
                in_refs = true;
                out.push(SecCandidate {
                    line_no,
                    kind: SecCandKind::Fixed("References", FixedSecRole::Bibliography),
                    title: "References".into(),
                    ends_with_dot: false,
                });
                continue;
            }
            if let Some(val) = parse_roman(rom) {
                if looks_like_heading_title(title, false)
                    && !looks_like_citation_line(&trimmed)
                    && !preceded_by_list_colon(lines, active, idx)
                {
                    out.push(SecCandidate {
                        line_no,
                        kind: SecCandKind::Roman(val, rom.to_string()),
                        title: title.to_string(),
                        ends_with_dot: ends_with_sentence_punct(title),
                    });
                    continue;
                }
            }
        }
        if let Some(cap) = letter_re.captures(&trimmed) {
            let ch = cap[1].as_bytes()[0] as char;
            let title = cap[2].trim();
            if looks_like_heading_title(title, false)
                && !looks_like_citation_line(&trimmed)
                && !looks_like_author_or_prose_initial(title)
                && !preceded_by_list_colon(lines, active, idx)
            {
                out.push(SecCandidate {
                    line_no,
                    kind: SecCandKind::Letter(ch as u8 - b'A', ch),
                    title: title.to_string(),
                    ends_with_dot: ends_with_sentence_punct(title),
                });
                continue;
            }
        }
        if let Some(cap) = dotted_re.captures(&trimmed) {
            let nums: Option<Vec<usize>> = cap[1].split('.').map(|s| s.parse().ok()).collect();
            let title = cap[2].trim();
            if let Some(nums) = nums.filter(|v| v.len() <= 4 && v.iter().all(|&n| (1..=50).contains(&n))) {
                if nums.len() == 1 && is_bib_heading_title(title) {
                    in_refs = true;
                    out.push(SecCandidate {
                        line_no,
                        kind: SecCandKind::Fixed("References", FixedSecRole::Bibliography),
                        title: "References".into(),
                        ends_with_dot: false,
                    });
                    continue;
                }
                if looks_like_heading_title(title, false)
                    && !looks_like_citation_line(&trimmed)
                    && !preceded_by_list_colon(lines, active, idx)
                {
                    out.push(SecCandidate {
                        line_no,
                        kind: SecCandKind::Dotted(nums),
                        title: title.to_string(),
                        ends_with_dot: ends_with_sentence_punct(title),
                    });
                }
            }
        }
    }
    out
}

fn valid_step(last: usize, val: usize, allow_nonzero_start: bool) -> bool {
    (last == 0 && (val == 1 || (allow_nonzero_start && val >= 1)))
        || (last > 0 && (val == last + 1 || val == last + 2))
}

fn count_valid_roman_chain(cands: &[SecCandidate], allow_start: bool) -> usize {
    let (mut last, mut count) = (0usize, 0usize);
    for c in cands {
        if let SecCandKind::Roman(val, _) = &c.kind {
            if !c.ends_with_dot && valid_step(last, *val, allow_start) {
                last = *val;
                count += 1;
            }
        }
    }
    count
}

fn count_valid_dotted_top(lines: &[&str], cands: &[SecCandidate], allow_start: bool) -> usize {
    let tops: Vec<&SecCandidate> = cands
        .iter()
        .filter(|c| matches!(&c.kind, SecCandKind::Dotted(v) if v.len() == 1) && !c.ends_with_dot)
        .collect();
    if is_packed_list(lines, &tops) {
        return 0;
    }
    let (mut last, mut count) = (0usize, 0usize);
    for c in tops {
        if let SecCandKind::Dotted(v) = &c.kind {
            if valid_step(last, v[0], allow_start) {
                last = v[0];
                count += 1;
            }
        }
    }
    count
}

fn assemble_aps_sections(cands: &[SecCandidate], allow_start: bool) -> Vec<SelectedSection> {
    let mut out = Vec::new();
    let (mut last_roman, mut cur_roman, mut in_app_banner, mut cur_app) =
        (0usize, None::<String>, false, None::<String>);
    let mut cur_sub: Option<(String, u8)> = None;
    for c in cands {
        match &c.kind {
            SecCandKind::Fixed(title, role) => {
                if *role == FixedSecRole::Bibliography {
                    (cur_roman, cur_sub, cur_app, in_app_banner) = (None, None, None, false);
                }
                out.push(mk_sec(c.line_no, role_to_kind(*role), None, title, 1));
            }
            SecCandKind::AppendixBanner => {
                (in_app_banner, cur_roman, cur_sub, cur_app) = (true, None, None, None);
            }
            SecCandKind::Appendix(id, None) => {
                (in_app_banner, cur_roman, cur_sub, cur_app) = (false, None, None, Some(id.clone()));
                let title = if c.title.is_empty() { format!("Appendix {id}") } else { c.title.clone() };
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(id.clone()), &title, 1));
            }
            SecCandKind::Appendix(id, Some(nums))
                if !c.ends_with_dot && (cur_app.as_deref() == Some(id.as_str()) || in_app_banner) =>
            {
                let num = format!("{id}.{}", join_nums(nums));
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(num), &c.title, (nums.len() + 1).min(3) as u8));
            }
            SecCandKind::Roman(val, rom) if !c.ends_with_dot && valid_step(last_roman, *val, allow_start) => {
                last_roman = *val;
                (cur_roman, cur_sub, cur_app, in_app_banner) = (Some(rom.clone()), None, None, false);
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(rom.clone()), &c.title, 1));
            }
            SecCandKind::Letter(idx_val, ch) if !c.ends_with_dot => {
                if in_app_banner {
                    cur_app = Some(ch.to_string());
                    out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(ch.to_string()), &c.title, 1));
                } else if let Some(rom) = &cur_roman {
                    let expected = cur_sub.as_ref().map_or(0, |(_, p)| p + 1);
                    if *idx_val == expected || (cur_sub.is_some() && *idx_val == expected + 1) {
                        let key = format!("{rom}.{ch}");
                        cur_sub = Some((key.clone(), *idx_val));
                        out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(key), &c.title, 2));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn assemble_dotted_sections(
    lines: &[&str],
    cands: &[SecCandidate],
    allow_start: bool,
) -> Vec<SelectedSection> {
    let mut out = Vec::new();
    let (mut l1, mut l2, mut l3) = (0usize, 0usize, 0usize);
    let (mut in_app_banner, mut cur_app) = (false, None::<String>);
    let tops: Vec<&SecCandidate> = cands
        .iter()
        .filter(|c| matches!(&c.kind, SecCandKind::Dotted(v) if v.len() == 1) && !c.ends_with_dot)
        .collect();
    let allow_single = !is_packed_list(lines, &tops);

    for c in cands {
        match &c.kind {
            SecCandKind::Fixed(title, role) => {
                if *role == FixedSecRole::Bibliography {
                    (in_app_banner, cur_app) = (false, None);
                }
                out.push(mk_sec(c.line_no, role_to_kind(*role), None, title, 1));
            }
            SecCandKind::AppendixBanner => (in_app_banner, cur_app) = (true, None),
            SecCandKind::Appendix(id, None) => {
                (in_app_banner, cur_app) = (false, Some(id.clone()));
                let title = if c.title.is_empty() { format!("Appendix {id}") } else { c.title.clone() };
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(id.clone()), &title, 1));
            }
            SecCandKind::Appendix(id, Some(nums))
                if !c.ends_with_dot && (cur_app.as_deref() == Some(id.as_str()) || in_app_banner) =>
            {
                let num = format!("{id}.{}", join_nums(nums));
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(num), &c.title, (nums.len() + 1).min(3) as u8));
            }
            SecCandKind::Dotted(nums) if !c.ends_with_dot => match nums.as_slice() {
                [a] if allow_single && valid_step(l1, *a, allow_start) => {
                    (l1, l2, l3, cur_app, in_app_banner) = (*a, 0, 0, None, false);
                    out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(a.to_string()), &c.title, 1));
                }
                [a, b] if (l1 == *a && valid_step(l2, *b, false))
                    || (l1 == 0 && *a >= 1 && valid_step(l2, *b, allow_start)) =>
                {
                    (l1, l2, l3) = (*a, *b, 0);
                    out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(format!("{a}.{b}")), &c.title, 2));
                }
                [a, b, d] if l1 == *a && l2 == *b && valid_step(l3, *d, false) => {
                    l3 = *d;
                    out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(format!("{a}.{b}.{d}")), &c.title, 3));
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

fn assemble_fixed_sections(cands: &[SecCandidate]) -> Vec<SelectedSection> {
    let mut out = Vec::new();
    let (mut in_app_banner, mut cur_app) = (false, None::<String>);
    for c in cands {
        match &c.kind {
            SecCandKind::Fixed(title, role) => {
                if *role == FixedSecRole::Bibliography {
                    (in_app_banner, cur_app) = (false, None);
                }
                out.push(mk_sec(c.line_no, role_to_kind(*role), None, title, 1));
            }
            SecCandKind::AppendixBanner => (in_app_banner, cur_app) = (true, None),
            SecCandKind::Appendix(id, None) => {
                (in_app_banner, cur_app) = (false, Some(id.clone()));
                let title = if c.title.is_empty() { format!("Appendix {id}") } else { c.title.clone() };
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(id.clone()), &title, 1));
            }
            SecCandKind::Appendix(id, Some(nums))
                if !c.ends_with_dot && (cur_app.as_deref() == Some(id.as_str()) || in_app_banner) =>
            {
                let num = format!("{id}.{}", join_nums(nums));
                out.push(mk_sec(c.line_no, SkNodeKind::Section, Some(num), &c.title, (nums.len() + 1).min(3) as u8));
            }
            _ => {}
        }
    }
    out
}

fn mk_sec(line_no: usize, kind: SkNodeKind, number: Option<String>, title: &str, level: u8) -> SelectedSection {
    SelectedSection { line_no, kind, number, title: title.to_string(), level }
}

fn role_to_kind(role: FixedSecRole) -> SkNodeKind {
    match role {
        FixedSecRole::FrontMatter => SkNodeKind::FrontMatter,
        FixedSecRole::Section | FixedSecRole::Appendix => SkNodeKind::Section,
        FixedSecRole::Bibliography => SkNodeKind::Bibliography,
    }
}

fn is_packed_list(lines: &[&str], run: &[&SecCandidate]) -> bool {
    run.len() >= 2
        && run.windows(2).any(|p| {
            (p[0].line_no..p[1].line_no.saturating_sub(1))
                .all(|idx| lines.get(idx).is_none_or(|s| s.trim().is_empty()))
        })
}

fn preceded_by_list_colon(lines: &[&str], active: &[bool], idx: usize) -> bool {
    (0..idx)
        .rev()
        .filter(|&k| active[k])
        .map(|k| lines[k].trim())
        .find(|t| !t.is_empty())
        .is_some_and(|t| t.ends_with(':'))
}

fn looks_like_author_or_prose_initial(title: &str) -> bool {
    if title.contains("--") || title.contains('—') {
        return true;
    }
    let words: Vec<&str> = title.split_whitespace().collect();
    words.len() >= 2
        && words[1..]
            .iter()
            .any(|w| matches!(w.to_ascii_lowercase().as_str(), "showed" | "proved" | "demonstrated" | "found" | "argued" | "proposed" | "defined"))
}

fn looks_like_heading_title(title: &str, allow_short: bool) -> bool {
    let t = title.trim();
    if t.is_empty() || t.len() > 115 || (!allow_short && t.len() < 2) || t.split_whitespace().count() > 15 {
        return false;
    }
    let first = t.chars().next().unwrap_or(' ');
    if !(first.is_ascii_uppercase() || first.is_ascii_digit() || matches!(first, '$' | '(' | '\\')) {
        return false;
    }
    if let Some(pos) = t.find(". ") {
        let last = t[..pos].split_whitespace().last().unwrap_or("");
        if last.len() > 1
            && !matches!(
                last,
                "Eq" | "Eqs" | "Fig" | "Figs" | "Sec" | "App" | "Ref" | "No" | "al"
                    | "Thm" | "Lem" | "Prop" | "Cor" | "Def" | "Ch" | "Dr" | "St" | "vs"
            )
        {
            return false;
        }
    }
    true
}

fn ends_with_sentence_punct(title: &str) -> bool {
    let t = title.trim();
    if t.ends_with(',') || t.ends_with(';') || t.ends_with('?') {
        return true;
    }
    if let Some(stem) = t.strip_suffix('.') {
        let last = stem.split_whitespace().last().unwrap_or("");
        return !matches!(last, "etc" | "al" | "Eq" | "Fig" | "Sec" | "App");
    }
    false
}

fn looks_like_citation_line(line: &str) -> bool {
    ["et al.", "Phys. Rev.", "Rev. Mod. Phys.", "arXiv:", "doi:"].iter().any(|pat| line.contains(pat))
}

fn match_fixed_heading(trimmed: &str) -> Option<(&'static str, FixedSecRole)> {
    let clean = trimmed.trim_end_matches(':').trim();
    match clean.to_ascii_lowercase().as_str() {
        "abstract" => Some(("Abstract", FixedSecRole::FrontMatter)),
        "introduction" => Some(("Introduction", FixedSecRole::Section)),
        "preliminaries" => Some(("Preliminaries", FixedSecRole::Section)),
        "background" => Some(("Background", FixedSecRole::Section)),
        "conclusion" | "conclusions" => Some(("Conclusions", FixedSecRole::Section)),
        "discussion" => Some(("Discussion", FixedSecRole::Section)),
        "summary" => Some(("Summary", FixedSecRole::Section)),
        "results" => Some(("Results", FixedSecRole::Section)),
        "methods" => Some(("Methods", FixedSecRole::Section)),
        "acknowledgments" | "acknowledgements" => Some(("Acknowledgments", FixedSecRole::Section)),
        "references" | "bibliography" | "works cited" => Some(("References", FixedSecRole::Bibliography)),
        "supplemental material" | "supplementary information" | "supplemental proofs" => {
            Some(("Supplemental Material", FixedSecRole::Appendix))
        }
        "notes" | "data availability" | "funding" | "author contributions" | "competing interests" => {
            Some(("Notes", FixedSecRole::Section))
        }
        _ => None,
    }
}

fn parse_roman(s: &str) -> Option<usize> {
    const TABLE: &[(&str, usize)] = &[
        ("I", 1), ("II", 2), ("III", 3), ("IV", 4), ("V", 5),
        ("VI", 6), ("VII", 7), ("VIII", 8), ("IX", 9), ("X", 10),
        ("XI", 11), ("XII", 12), ("XIII", 13), ("XIV", 14), ("XV", 15),
        ("XVI", 16), ("XVII", 17), ("XVIII", 18), ("XIX", 19), ("XX", 20),
    ];
    TABLE.iter().find(|&&(k, _)| k == s).map(|&(_, v)| v)
}

fn join_nums(nums: &[usize]) -> String {
    nums.iter().map(usize::to_string).collect::<Vec<_>>().join(".")
}
