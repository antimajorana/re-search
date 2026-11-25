//! Plain-text / PDF heading detection (APS, dotted Arabic, appendices, fixed names),
//! numbered-list guards, and equation/caption indexing for [`super::build_outline`].

use super::{make_section, OutlineSection};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
enum CandKind {
    Roman(usize, String),
    Letter(u8, char),
    Dotted(Vec<usize>),
    Appendix(String, Option<Vec<usize>>),
    AppendixBanner,
    Fixed(&'static str, &'static str),
}

#[derive(Debug, Clone)]
struct Candidate {
    line_no: usize,
    kind: CandKind,
    title: String,
    ends_with_dot: bool,
}

pub(super) fn build_plain_sections(lines: &[&str]) -> (Vec<OutlineSection>, String) {
    let candidates = scan_candidates(lines);
    let roman_count = count_valid_roman_chain(&candidates);
    let dotted_multi = candidates
        .iter()
        .filter(|c| matches!(&c.kind, CandKind::Dotted(v) if v.len() >= 2))
        .count();

    if roman_count >= 2 && roman_count >= dotted_multi {
        (assemble_aps(lines, &candidates), "aps".into())
    } else if dotted_multi >= 1 || count_valid_dotted_top(lines, &candidates) >= 2 {
        (assemble_dotted(lines, &candidates), "dotted".into())
    } else if roman_count >= 1 {
        (assemble_aps(lines, &candidates), "aps".into())
    } else {
        (assemble_fixed_and_appendices(&candidates), "mixed".into())
    }
}

fn scan_candidates(lines: &[&str]) -> Vec<Candidate> {
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
    let mut in_references = false;

    for (idx, raw) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.len() > 140 {
            continue;
        }
        if let Some((key, kind)) = match_fixed_heading(trimmed) {
            if key == "References" {
                in_references = true;
            }
            out.push(Candidate {
                line_no,
                kind: CandKind::Fixed(key, kind),
                title: key.to_string(),
                ends_with_dot: false,
            });
            continue;
        }
        if trimmed.eq_ignore_ascii_case("APPENDIX") || trimmed.eq_ignore_ascii_case("APPENDICES") {
            in_references = false;
            out.push(Candidate {
                line_no,
                kind: CandKind::AppendixBanner,
                title: trimmed.to_string(),
                ends_with_dot: false,
            });
            continue;
        }
        if let Some(cap) = app_re.captures(trimmed) {
            let id = cap[1].to_uppercase();
            let title = cap
                .get(2)
                .or_else(|| cap.get(3))
                .map(|m| m.as_str().trim())
                .unwrap_or("")
                .to_string();
            if title.is_empty() || looks_like_heading_title(&title, true) {
                in_references = false;
                out.push(Candidate {
                    line_no,
                    kind: CandKind::Appendix(id, None),
                    title,
                    ends_with_dot: false,
                });
                continue;
            }
        }
        if in_references {
            continue;
        }
        if let Some(cap) = app_sub_re.captures(trimmed) {
            let letter = cap[1].to_string();
            let nums: Option<Vec<usize>> = cap[2].split('.').map(|s| s.parse().ok()).collect();
            let title = cap[3].trim();
            if let Some(nums) = nums {
                if nums.iter().all(|&n| (1..=50).contains(&n))
                    && looks_like_heading_title(title, false)
                    && !preceded_by_list_colon(lines, idx)
                {
                    out.push(Candidate {
                        line_no,
                        kind: CandKind::Appendix(letter, Some(nums)),
                        title: title.to_string(),
                        ends_with_dot: ends_with_sentence_punct(title),
                    });
                    continue;
                }
            }
        }
        if let Some(cap) = roman_re.captures(trimmed) {
            let rom = &cap[1];
            let title = cap[2].trim();
            if let Some(val) = parse_roman(rom) {
                if looks_like_heading_title(title, false)
                    && !looks_like_citation_line(trimmed)
                    && !preceded_by_list_colon(lines, idx)
                {
                    out.push(Candidate {
                        line_no,
                        kind: CandKind::Roman(val, rom.to_string()),
                        title: title.to_string(),
                        ends_with_dot: ends_with_sentence_punct(title),
                    });
                    continue;
                }
            }
        }
        if let Some(cap) = letter_re.captures(trimmed) {
            let ch = cap[1].as_bytes()[0] as char;
            let title = cap[2].trim();
            if looks_like_heading_title(title, false)
                && !looks_like_citation_line(trimmed)
                && !preceded_by_list_colon(lines, idx)
                && !is_wrapped_sentence_item(lines, idx, title)
            {
                out.push(Candidate {
                    line_no,
                    kind: CandKind::Letter(ch as u8 - b'A', ch),
                    title: title.to_string(),
                    ends_with_dot: ends_with_sentence_punct(title),
                });
                continue;
            }
        }
        if let Some(cap) = dotted_re.captures(trimmed) {
            let nums: Option<Vec<usize>> = cap[1].split('.').map(|s| s.parse().ok()).collect();
            let title = cap[2].trim();
            if let Some(nums) = nums {
                if nums.len() <= 4
                    && nums.iter().all(|&n| (1..=50).contains(&n))
                    && looks_like_heading_title(title, false)
                    && !looks_like_citation_line(trimmed)
                    && !preceded_by_list_colon(lines, idx)
                    && !is_wrapped_sentence_item(lines, idx, title)
                {
                    out.push(Candidate {
                        line_no,
                        kind: CandKind::Dotted(nums),
                        title: title.to_string(),
                        ends_with_dot: ends_with_sentence_punct(title),
                    });
                }
            }
        }
    }
    out
}

fn count_valid_roman_chain(cands: &[Candidate]) -> usize {
    let (mut last, mut count) = (0usize, 0usize);
    for c in cands {
        if let CandKind::Roman(val, _) = &c.kind {
            if !c.ends_with_dot && valid_step(last, *val) {
                last = *val;
                count += 1;
            }
        }
    }
    count
}

fn count_valid_dotted_top(lines: &[&str], cands: &[Candidate]) -> usize {
    let tops: Vec<&Candidate> = cands
        .iter()
        .filter(|c| matches!(&c.kind, CandKind::Dotted(v) if v.len() == 1) && !c.ends_with_dot)
        .collect();
    if is_packed_list(lines, &tops) {
        return 0;
    }
    let (mut last, mut count) = (0usize, 0usize);
    for c in tops {
        if let CandKind::Dotted(v) = &c.kind {
            if valid_step(last, v[0]) {
                last = v[0];
                count += 1;
            }
        }
    }
    count
}

fn valid_step(last: usize, val: usize) -> bool {
    (last == 0 && val == 1) || (last > 0 && (val == last + 1 || val == last + 2))
}

fn assemble_aps(lines: &[&str], cands: &[Candidate]) -> Vec<OutlineSection> {
    let mut sections = Vec::new();
    let mut last_roman = 0usize;
    let mut cur_roman: Option<String> = None;
    let mut in_app_banner = false;
    let mut cur_app: Option<String> = None;
    let mut last_app_letter: Option<u8> = None;
    let mut cur_sub: Option<(String, u8)> = None;
    let mut l3_done_for_parent = false;

    let mut i = 0usize;
    while i < cands.len() {
        let c = &cands[i];
        match &c.kind {
            CandKind::Fixed(key, kind) => {
                if *kind == "back_matter" {
                    cur_roman = None;
                    cur_sub = None;
                    cur_app = None;
                    in_app_banner = false;
                    l3_done_for_parent = false;
                }
                sections.push(make_section((*key).into(), c.title.clone(), 1, kind, c.line_no));
            }
            CandKind::AppendixBanner => {
                in_app_banner = true;
                cur_roman = None;
                cur_sub = None;
                cur_app = None;
                last_app_letter = None;
                l3_done_for_parent = false;
            }
            CandKind::Appendix(id, None) => {
                in_app_banner = false;
                cur_roman = None;
                cur_sub = None;
                cur_app = Some(id.clone());
                l3_done_for_parent = false;
                if id.len() == 1 && id.as_bytes()[0].is_ascii_uppercase() {
                    last_app_letter = Some(id.as_bytes()[0] - b'A');
                }
                sections.push(make_section(id.clone(), c.title.clone(), 1, "appendix", c.line_no));
            }
            CandKind::Appendix(id, Some(nums)) => {
                if !c.ends_with_dot && (cur_app.as_deref() == Some(id.as_str()) || in_app_banner) {
                    let key = format!("{id}.{}", join_nums(nums));
                    sections.push(make_section(key, c.title.clone(), (nums.len() + 1).min(3) as u8, "appendix", c.line_no));
                }
            }
            CandKind::Roman(val, rom) => {
                if !c.ends_with_dot && valid_step(last_roman, *val) {
                    last_roman = *val;
                    cur_roman = Some(rom.clone());
                    cur_sub = None;
                    cur_app = None;
                    in_app_banner = false;
                    l3_done_for_parent = false;
                    sections.push(make_section(rom.clone(), c.title.clone(), 1, "section", c.line_no));
                }
            }
            CandKind::Letter(idx_val, ch) if !c.ends_with_dot => {
                if in_app_banner {
                    let expected = last_app_letter.map(|p| p + 1).unwrap_or(0);
                    if *idx_val == expected || (last_app_letter.is_some() && *idx_val == expected + 1) {
                        last_app_letter = Some(*idx_val);
                        cur_app = Some(ch.to_string());
                        cur_sub = None;
                        l3_done_for_parent = false;
                        sections.push(make_section(ch.to_string(), c.title.clone(), 1, "appendix", c.line_no));
                    }
                } else if let Some(rom) = &cur_roman {
                    let expected = cur_sub.as_ref().map(|(_, p)| p + 1).unwrap_or(0);
                    let step_ok = *idx_val == expected || (cur_sub.is_some() && *idx_val == expected + 1);
                    if step_ok && !is_letter_list(lines, cands, i) {
                        let key = format!("{rom}.{ch}");
                        cur_sub = Some((key.clone(), *idx_val));
                        l3_done_for_parent = false;
                        sections.push(make_section(key, c.title.clone(), 2, "section", c.line_no));
                    }
                }
            }
            CandKind::Dotted(nums) if nums.len() == 1 && !l3_done_for_parent => {
                if let Some((parent_key, level, kind)) = current_aps_parent(&cur_sub, &cur_app) {
                    let mut run = Vec::new();
                    let mut last_n = 0usize;
                    let mut j = i;
                    while j < cands.len() {
                        match &cands[j].kind {
                            CandKind::Dotted(v) if v.len() == 1 => {
                                if let Some(list_end) = packed_dotted_list_end(lines, cands, j) {
                                    j = list_end;
                                    continue;
                                }
                                if cands[j].ends_with_dot {
                                    j += 1;
                                    continue;
                                }
                                let n = v[0];
                                if n == 1 && last_n >= 1 {
                                    break; // Restarting at 1 inside a subsubsection is a body list.
                                }
                                if valid_step(last_n, n) {
                                    last_n = n;
                                    run.push(&cands[j]);
                                }
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    if !run.is_empty() && !is_packed_list(lines, &run) {
                        l3_done_for_parent = true;
                        for item in run {
                            if let CandKind::Dotted(v) = &item.kind {
                                sections.push(make_section(
                                    format!("{parent_key}.{}", v[0]),
                                    item.title.clone(),
                                    level,
                                    kind,
                                    item.line_no,
                                ));
                            }
                        }
                    }
                    if j > i {
                        i = j;
                        continue;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    sections
}

fn current_aps_parent<'a>(
    cur_sub: &'a Option<(String, u8)>,
    cur_app: &'a Option<String>,
) -> Option<(&'a str, u8, &'static str)> {
    if let Some((key, _)) = cur_sub {
        Some((key.as_str(), 3, "section"))
    } else if let Some(app) = cur_app {
        Some((app.as_str(), 2, "appendix"))
    } else {
        None
    }
}

fn assemble_dotted(lines: &[&str], cands: &[Candidate]) -> Vec<OutlineSection> {
    let mut sections = Vec::new();
    let (mut l1, mut l2, mut l3) = (0usize, 0usize, 0usize);
    let single_tops: Vec<&Candidate> = cands
        .iter()
        .filter(|c| matches!(&c.kind, CandKind::Dotted(v) if v.len() == 1) && !c.ends_with_dot)
        .collect();
    let allow_single = !is_packed_list(lines, &single_tops);

    for c in cands {
        match &c.kind {
            CandKind::Fixed(key, kind) => {
                sections.push(make_section((*key).into(), c.title.clone(), 1, kind, c.line_no));
            }
            CandKind::Appendix(id, None) => {
                sections.push(make_section(id.clone(), c.title.clone(), 1, "appendix", c.line_no));
            }
            CandKind::Appendix(id, Some(nums)) if !c.ends_with_dot => {
                let key = format!("{id}.{}", join_nums(nums));
                sections.push(make_section(key, c.title.clone(), (nums.len() + 1).min(3) as u8, "appendix", c.line_no));
            }
            CandKind::Dotted(nums) if !c.ends_with_dot => match nums.as_slice() {
                [a] if allow_single && valid_step(l1, *a) => {
                    l1 = *a;
                    l2 = 0;
                    l3 = 0;
                    sections.push(make_section(a.to_string(), c.title.clone(), 1, "section", c.line_no));
                }
                [a, b] if (l1 == *a || (l1 == 0 && *a >= 1)) && valid_step(l2, *b) => {
                    l1 = *a;
                    l2 = *b;
                    l3 = 0;
                    sections.push(make_section(format!("{a}.{b}"), c.title.clone(), 2, "section", c.line_no));
                }
                [a, b, d] if l1 == *a && l2 == *b && valid_step(l3, *d) => {
                    l3 = *d;
                    sections.push(make_section(format!("{a}.{b}.{d}"), c.title.clone(), 3, "section", c.line_no));
                }
                _ => {}
            },
            _ => {}
        }
    }
    sections
}

fn assemble_fixed_and_appendices(cands: &[Candidate]) -> Vec<OutlineSection> {
    let mut sections = Vec::new();
    for c in cands {
        match &c.kind {
            CandKind::Fixed(key, kind) => {
                sections.push(make_section((*key).into(), c.title.clone(), 1, kind, c.line_no));
            }
            CandKind::Appendix(id, None) => {
                sections.push(make_section(id.clone(), c.title.clone(), 1, "appendix", c.line_no));
            }
            CandKind::Appendix(id, Some(nums)) if !c.ends_with_dot => {
                let key = format!("{id}.{}", join_nums(nums));
                sections.push(make_section(key, c.title.clone(), (nums.len() + 1).min(3) as u8, "appendix", c.line_no));
            }
            _ => {}
        }
    }
    sections
}

fn no_prose_between(lines: &[&str], line_a: usize, line_b: usize) -> bool {
    (line_a..line_b.saturating_sub(1))
        .all(|idx| lines.get(idx).map(|s| s.trim().is_empty()).unwrap_or(true))
}

fn is_packed_list(lines: &[&str], run: &[&Candidate]) -> bool {
    run.len() >= 2 && run.windows(2).any(|p| no_prose_between(lines, p[0].line_no, p[1].line_no))
}

fn packed_dotted_list_end(lines: &[&str], cands: &[Candidate], start: usize) -> Option<usize> {
    let CandKind::Dotted(v0) = &cands[start].kind else { return None };
    if v0.len() != 1 {
        return None;
    }
    let (mut prev_n, mut k) = (v0[0], start + 1);
    while let Some(c) = cands.get(k) {
        let CandKind::Dotted(vk) = &c.kind else { break };
        if vk.len() != 1 || vk[0] != prev_n + 1 || !no_prose_between(lines, cands[k - 1].line_no, c.line_no) {
            break;
        }
        let adjacent = c.line_no == cands[k - 1].line_no + 1;
        let has_packed_next = cands.get(k + 1).is_some_and(|next| {
            matches!(&next.kind, CandKind::Dotted(vn) if vn.len() == 1 && vn[0] == vk[0] + 1)
                && no_prose_between(lines, c.line_no, next.line_no)
        });
        if !adjacent && !c.ends_with_dot && !has_packed_next {
            break;
        }
        prev_n = vk[0];
        k += 1;
    }
    (k >= start + 2).then_some(k)
}

fn is_letter_list(lines: &[&str], cands: &[Candidate], start: usize) -> bool {
    let mut run = vec![&cands[start]];
    for c in &cands[start + 1..] {
        if matches!(c.kind, CandKind::Letter(_, _)) {
            run.push(c);
        } else {
            break;
        }
    }
    is_packed_list(lines, &run)
}

fn preceded_by_list_colon(lines: &[&str], idx: usize) -> bool {
    (0..idx)
        .rev()
        .map(|k| lines[k].trim())
        .find(|t| !t.is_empty())
        .is_some_and(|t| t.ends_with(':'))
}

fn is_wrapped_sentence_item(lines: &[&str], idx: usize, title: &str) -> bool {
    let next = lines.get(idx + 1).map(|s| s.trim()).unwrap_or("");
    if !next.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        return false;
    }
    let last = title.split_whitespace().last().unwrap_or("").to_ascii_lowercase();
    matches!(
        last.as_str(),
        "of" | "the" | "and" | "that" | "which" | "in" | "for" | "to" | "with" | "by" | "from"
            | "a" | "an" | "is" | "are" | "be" | "as" | "on" | "at" | "or"
    )
}

fn looks_like_heading_title(title: &str, allow_short: bool) -> bool {
    let t = title.trim();
    if t.is_empty() || t.len() > 115 || (!allow_short && t.len() < 2) || t.split_whitespace().count() > 15 {
        return false;
    }
    let first = t.chars().next().unwrap();
    if !(first.is_ascii_uppercase() || first.is_ascii_digit() || matches!(first, '$' | '(' | '\\')) {
        return false;
    }
    if let Some(pos) = t.find(". ") {
        let last = t[..pos].split_whitespace().last().unwrap_or("");
        if last.len() > 1
            && !matches!(last, "Eq" | "Eqs" | "Fig" | "Figs" | "Sec" | "App" | "Ref" | "No" | "Dr" | "St" | "vs" | "al")
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
    ["et al.", "Phys. Rev.", "Rev. Mod. Phys.", "arXiv:", "doi:"]
        .iter()
        .any(|pat| line.contains(pat))
}

fn match_fixed_heading(trimmed: &str) -> Option<(&'static str, &'static str)> {
    let clean = trimmed.trim_end_matches(':').trim();
    match clean.to_ascii_lowercase().as_str() {
        "abstract" => Some(("Abstract", "front_matter")),
        "introduction" => Some(("Introduction", "section")),
        "conclusion" | "conclusions" => Some(("Conclusions", "section")),
        "discussion" => Some(("Discussion", "section")),
        "summary" => Some(("Summary", "section")),
        "results" => Some(("Results", "section")),
        "methods" => Some(("Methods", "section")),
        "acknowledgments" | "acknowledgements" => Some(("Acknowledgments", "back_matter")),
        "references" | "bibliography" => Some(("References", "back_matter")),
        "supplemental material" => Some(("Supplemental Material", "appendix")),
        _ => None,
    }
}

pub(super) fn parse_roman(s: &str) -> Option<usize> {
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
