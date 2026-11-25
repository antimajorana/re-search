//! Deterministic paper outline extraction and question-scope resolution for Investigation mode.

#[path = "investigation_outline_plain.rs"]
mod plain;
#[path = "investigation_outline_tex.rs"]
mod tex;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// A detected structural section, subsection, appendix, or fixed front/back-matter block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineSection {
    pub key: String,
    pub title: String,
    pub level: u8,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_end: Option<usize>,
}

/// An indexed equation, figure, or table anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineItem {
    pub kind: String,
    pub label: String,
    pub caption: String,
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
}

/// Deterministic outline built from the primary paper text and optional physical PDF page ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperOutline {
    pub total_lines: usize,
    pub style: String,
    pub weak: bool,
    pub sections: Vec<OutlineSection>,
    pub equations: Vec<OutlineItem>,
    pub captions: Vec<OutlineItem>,
}

/// A resolved target referenced by a reader question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeTarget {
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_end: Option<usize>,
}

/// Outcome of resolving a reader question against a [`PaperOutline`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScopeResolution {
    Resolved {
        label: String,
        targets: Vec<ScopeTarget>,
        ranges: Vec<[usize; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        front_matter: Option<[usize; 2]>,
    },
    Unresolved {
        reason: String,
    },
}

const ITEM_CONTEXT_RADIUS: usize = 20;

/// Build a deterministic structural outline (sections, appendices, equations, captions).
pub fn build_outline(text: &str, page_lines: &[(usize, usize, usize)]) -> PaperOutline {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let total_lines = lines.len();
    if total_lines == 0 {
        return PaperOutline {
            total_lines: 0,
            style: "weak".into(),
            weak: true,
            sections: Vec::new(),
            equations: Vec::new(),
            captions: Vec::new(),
        };
    }

    let is_tex = tex::looks_like_tex(&lines);
    let (mut sections, style, mut equations, mut captions) = if is_tex {
        tex::build_tex_outline(&lines, page_lines)
    } else {
        let (secs, st) = plain::build_plain_sections(&lines);
        (secs, st, Vec::new(), Vec::new())
    };

    finalize_ranges(&mut sections, total_lines, page_lines);
    if !is_tex {
        let ref_range = sections
            .iter()
            .find(|s| s.kind == "back_matter" && s.key == "References")
            .map(|s| (s.start_line, s.end_line));
        equations = extract_plain_equations(&lines, page_lines, ref_range);
        captions = extract_plain_captions(&lines, page_lines, ref_range);
    }

    let body_sections = sections
        .iter()
        .filter(|s| matches!(s.kind.as_str(), "section" | "appendix"))
        .count();
    let weak = body_sections < 2;
    let final_style = if weak { "weak".into() } else { style };

    PaperOutline {
        total_lines,
        style: final_style,
        weak,
        sections,
        equations,
        captions,
    }
}

/// Format a compact, deterministic outline string suitable for prompt injection (~1.5 KB cap).
pub fn format_outline_for_prompt(outline: &PaperOutline, max_bytes: usize) -> String {
    let mut out = String::new();
    let header = format!(
        "Paper outline ({} lines, style: {}):\n",
        outline.total_lines, outline.style
    );
    if header.len() > max_bytes {
        return String::new();
    }
    out.push_str(&header);
    let mut omitted = 0usize;
    for sec in &outline.sections {
        let indent = "  ".repeat(usize::from(sec.level.saturating_sub(1)));
        let pages = match (sec.page_start, sec.page_end) {
            (Some(a), Some(b)) if a == b => format!(", p.{a}"),
            (Some(a), Some(b)) => format!(", pp.{a}-{b}"),
            _ => String::new(),
        };
        let label = if sec.kind == "appendix" && !sec.key.contains('.') {
            format!("Appendix {}", sec.key)
        } else {
            sec.key.clone()
        };
        let title = clip_chars(&sec.title, 72);
        let line = if title.is_empty() || title.eq_ignore_ascii_case(&label) {
            format!("{indent}- {label} (L{}-{}{pages})\n", sec.start_line, sec.end_line)
        } else {
            format!("{indent}- {label}: {title} (L{}-{}{pages})\n", sec.start_line, sec.end_line)
        };
        if out.len() + line.len() + 64 > max_bytes {
            omitted += 1;
            continue;
        }
        out.push_str(&line);
    }
    if omitted > 0 {
        let note = format!("  ... ({omitted} more sections omitted)\n");
        if out.len() + note.len() <= max_bytes {
            out.push_str(&note);
        }
    }
    if !outline.equations.is_empty() {
        let labels: Vec<String> = outline
            .equations
            .iter()
            .take(16)
            .map(|e| format!("({})@L{}", e.label, e.line))
            .collect();
        let extra = outline.equations.len().saturating_sub(16);
        let suffix = if extra > 0 { format!(", +{extra} more") } else { String::new() };
        let line = format!("Equations: {}{suffix}\n", labels.join(", "));
        if out.len() + line.len() <= max_bytes {
            out.push_str(&line);
        }
    }
    if !outline.captions.is_empty() {
        let labels: Vec<String> = outline
            .captions
            .iter()
            .take(12)
            .map(|c| format!("{} {}@L{}", if c.kind == "table" { "Table" } else { "Fig." }, c.label, c.line))
            .collect();
        let extra = outline.captions.len().saturating_sub(12);
        let suffix = if extra > 0 { format!(", +{extra} more") } else { String::new() };
        let line = format!("Figures/Tables: {}{suffix}\n", labels.join(", "));
        if out.len() + line.len() <= max_bytes {
            out.push_str(&line);
        }
    }
    out
}

/// Resolve section, appendix, equation, figure, or table references in a reader question.
pub fn resolve_scope(outline: &PaperOutline, question: &str) -> ScopeResolution {
    let refs = extract_question_refs(question);
    if refs.is_empty() {
        return ScopeResolution::Unresolved {
            reason: "No section, appendix, equation, figure, or table reference found in the question.".into(),
        };
    }

    let mut targets = Vec::new();
    let mut seen = BTreeSet::new();
    let mut missing = Vec::new();
    for qref in refs {
        if let Some(target) = match_ref(outline, &qref) {
            let dedup = (target.kind.clone(), target.label.clone(), target.start_line, target.end_line);
            if seen.insert(dedup) {
                targets.push(target);
            }
        } else {
            missing.push(qref.raw);
        }
    }

    if targets.is_empty() {
        return ScopeResolution::Unresolved {
            reason: format!("Referenced target(s) not found in the paper outline: {}.", missing.join(", ")),
        };
    }

    let ranges = merge_ranges(targets.iter().map(|t| [t.start_line, t.end_line]).collect());
    let label = build_scope_label(&targets);
    let front_matter = front_matter_range(outline);
    ScopeResolution::Resolved { label, targets, ranges, front_matter }
}

fn extract_plain_equations(
    lines: &[&str],
    page_lines: &[(usize, usize, usize)],
    ref_range: Option<(usize, usize)>,
) -> Vec<OutlineItem> {
    static EQ_RE: OnceLock<Regex> = OnceLock::new();
    let eq_re = EQ_RE.get_or_init(|| Regex::new(r"\(([A-Za-z]?\d+(?:\.\d+)?[a-z]?)\)\s*$").unwrap());
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    for (idx, raw) in lines.iter().enumerate() {
        let line_no = idx + 1;
        if ref_range.is_some_and(|(a, b)| (a..=b).contains(&line_no)) {
            continue;
        }
        let trimmed = raw.trim();
        let Some(cap) = eq_re.captures(trimmed) else {
            continue;
        };
        let label = cap[1].to_string();
        if is_likely_year(&label) {
            continue;
        }
        let prefix = trimmed[..cap.get(0).unwrap().start()].trim();
        let mathy = prefix.is_empty()
            || trimmed.contains("   (")
            || ['=', '+', '−', '∑', '∫', '→', '⊗', '≤', '≥', '^', '_', '\\']
                .iter()
                .any(|&ch| prefix.contains(ch));
        if mathy && seen.insert(label.to_ascii_uppercase()) {
            items.push(OutlineItem {
                kind: "equation".into(),
                label,
                caption: clip_chars(trimmed, 120),
                line: line_no,
                page: page_for_line(page_lines, line_no),
            });
        }
    }
    items
}

fn extract_plain_captions(
    lines: &[&str],
    page_lines: &[(usize, usize, usize)],
    ref_range: Option<(usize, usize)>,
) -> Vec<OutlineItem> {
    static FIG_RE: OnceLock<Regex> = OnceLock::new();
    static TAB_RE: OnceLock<Regex> = OnceLock::new();
    let fig_re = FIG_RE.get_or_init(|| {
        Regex::new(r"^(?:FIG\.|Fig\.|Figure|FIGURE)\s+([A-Za-z]?\d+(?:\.\d+)?[a-z]?)\s*[:.\-–—]\s*(.*)$").unwrap()
    });
    let tab_re = TAB_RE.get_or_init(|| {
        Regex::new(r"^(?:TABLE|Table|TAB\.|Tab\.)\s+([A-Za-z]?\d+(?:\.\d+)?[a-z]?|[IVX]+)\s*[:.\-–—]\s*(.*)$").unwrap()
    });
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    for (idx, raw) in lines.iter().enumerate() {
        let line_no = idx + 1;
        if ref_range.is_some_and(|(a, b)| (a..=b).contains(&line_no)) {
            continue;
        }
        let trimmed = raw.trim();
        if let Some(cap) = fig_re.captures(trimmed) {
            let label = cap[1].to_string();
            if seen.insert(("figure", label.to_ascii_uppercase())) {
                items.push(OutlineItem {
                    kind: "figure".into(),
                    label,
                    caption: clip_chars(cap[2].trim(), 140),
                    line: line_no,
                    page: page_for_line(page_lines, line_no),
                });
            }
        } else if let Some(cap) = tab_re.captures(trimmed) {
            let label = cap[1].to_string();
            if seen.insert(("table", label.to_ascii_uppercase())) {
                items.push(OutlineItem {
                    kind: "table".into(),
                    label,
                    caption: clip_chars(cap[2].trim(), 140),
                    line: line_no,
                    page: page_for_line(page_lines, line_no),
                });
            }
        }
    }
    items
}

fn is_likely_year(label: &str) -> bool {
    label.len() == 4
        && label.bytes().all(|b| b.is_ascii_digit())
        && (label.starts_with("19") || label.starts_with("20"))
}

pub(super) fn make_section(
    key: String,
    title: String,
    level: u8,
    kind: &str,
    start_line: usize,
) -> OutlineSection {
    OutlineSection {
        key,
        title,
        level,
        kind: kind.into(),
        start_line,
        end_line: start_line,
        page_start: None,
        page_end: None,
    }
}

fn finalize_ranges(
    sections: &mut [OutlineSection],
    total_lines: usize,
    page_lines: &[(usize, usize, usize)],
) {
    for i in 0..sections.len() {
        let level = sections[i].level;
        let next = sections[i + 1..]
            .iter()
            .find(|s| s.level <= level)
            .map(|s| s.start_line.saturating_sub(1))
            .unwrap_or(total_lines);
        sections[i].end_line = next.max(sections[i].start_line);
        sections[i].page_start = page_for_line(page_lines, sections[i].start_line);
        sections[i].page_end = page_for_line(page_lines, sections[i].end_line);
    }
}

pub(super) fn page_for_line(page_lines: &[(usize, usize, usize)], line: usize) -> Option<usize> {
    page_lines
        .iter()
        .find(|&&(_, first, last)| (first..=last).contains(&line))
        .map(|&(p, _, _)| p)
}

pub(super) fn clip_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

// ---------------------------------------------------------------------------
// Question reference parser & matcher
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum RefKind {
    Section(String),
    Appendix(String),
    Equation(String),
    Figure(String),
    Table(String),
}

#[derive(Debug, Clone)]
struct QuestionRef {
    kind: RefKind,
    raw: String,
}

fn extract_question_refs(question: &str) -> Vec<QuestionRef> {
    static SEC_RE: OnceLock<Regex> = OnceLock::new();
    static APP_RE: OnceLock<Regex> = OnceLock::new();
    static EQ_RE: OnceLock<Regex> = OnceLock::new();
    static FIG_RE: OnceLock<Regex> = OnceLock::new();
    static TAB_RE: OnceLock<Regex> = OnceLock::new();

    let sec_re = SEC_RE.get_or_init(|| {
        Regex::new(
            r"(?:(?i:sections?|subsections?|subsubsections?|secs?\.?)|§)\s*([IVXivx]+(?:\.[A-Za-z](?:(?:\.|\s+)\d+)?|\s+[A-Za-z](?:\.|\s+)\d+|\s+[A-Zb-hj-z])?|\d+(?:\.\d+)*)\b"
        ).unwrap()
    });
    let app_re = APP_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:appendix|appendices|app\.?)\s+([A-Z](?:[.\s]+\d+(?:\.\d+)*)?|\d+(?:\.\d+)*)\b")
            .unwrap()
    });
    let eq_re = EQ_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:equations?|eqs?\.?)\s*\(?([A-Za-z]?\d+(?:\.\d+)?[a-z]?)\)?")
            .unwrap()
    });
    let fig_re = FIG_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:figures?|figs?\.?)\s+([A-Za-z]?\d+(?:\.\d+)?[a-z]?)\b").unwrap()
    });
    let tab_re = TAB_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:tables?|tabs?\.?)\s+([A-Za-z]?\d+(?:\.\d+)?[a-z]?|[IVX]+)\b").unwrap()
    });

    let mut found: Vec<(usize, QuestionRef)> = Vec::new();
    for cap in sec_re.captures_iter(question) {
        let m = cap.get(0).unwrap();
        let norm = normalize_section_key(&cap[1]);
        let first = norm.split('.').next().unwrap_or("");
        if first.chars().all(|c| c.is_ascii_digit()) || plain::parse_roman(first).is_some() {
            found.push((m.start(), QuestionRef { kind: RefKind::Section(norm), raw: m.as_str().trim().into() }));
        }
    }
    for cap in app_re.captures_iter(question) {
        let m = cap.get(0).unwrap();
        found.push((m.start(), QuestionRef { kind: RefKind::Appendix(normalize_section_key(&cap[1])), raw: m.as_str().trim().into() }));
    }
    for cap in eq_re.captures_iter(question) {
        let m = cap.get(0).unwrap();
        found.push((m.start(), QuestionRef { kind: RefKind::Equation(cap[1].trim().into()), raw: m.as_str().trim().into() }));
    }
    for cap in fig_re.captures_iter(question) {
        let m = cap.get(0).unwrap();
        found.push((m.start(), QuestionRef { kind: RefKind::Figure(cap[1].trim().into()), raw: m.as_str().trim().into() }));
    }
    for cap in tab_re.captures_iter(question) {
        let m = cap.get(0).unwrap();
        found.push((m.start(), QuestionRef { kind: RefKind::Table(cap[1].trim().into()), raw: m.as_str().trim().into() }));
    }
    found.sort_by_key(|(pos, _)| *pos);
    found.into_iter().map(|(_, r)| r).collect()
}

fn normalize_section_key(raw: &str) -> String {
    raw.split(|c: char| c == '.' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(".")
}

fn match_ref(outline: &PaperOutline, qref: &QuestionRef) -> Option<ScopeTarget> {
    match &qref.kind {
        RefKind::Section(key) => {
            let sec = outline.sections.iter().find(|s| s.kind == "section" && s.key.eq_ignore_ascii_case(key))?;
            Some(ScopeTarget {
                kind: "section".into(),
                label: format!("Section {}", sec.key),
                title: (!sec.title.is_empty()).then(|| sec.title.clone()),
                start_line: sec.start_line,
                end_line: sec.end_line,
                page_start: sec.page_start,
                page_end: sec.page_end,
            })
        }
        RefKind::Appendix(key) => {
            let sec = outline.sections.iter().find(|s| s.kind == "appendix" && s.key.eq_ignore_ascii_case(key))?;
            Some(ScopeTarget {
                kind: "appendix".into(),
                label: format!("Appendix {}", sec.key),
                title: (!sec.title.is_empty()).then(|| sec.title.clone()),
                start_line: sec.start_line,
                end_line: sec.end_line,
                page_start: sec.page_start,
                page_end: sec.page_end,
            })
        }
        RefKind::Equation(label) => {
            let item = outline.equations.iter().find(|e| e.label.eq_ignore_ascii_case(label))?;
            Some(item_target(outline, item, format!("Eq. ({})", item.label)))
        }
        RefKind::Figure(label) => {
            let item = outline.captions.iter().find(|c| c.kind == "figure" && c.label.eq_ignore_ascii_case(label))?;
            Some(item_target(outline, item, format!("Fig. {}", item.label)))
        }
        RefKind::Table(label) => {
            let item = outline.captions.iter().find(|c| c.kind == "table" && c.label.eq_ignore_ascii_case(label))?;
            Some(item_target(outline, item, format!("Table {}", item.label)))
        }
    }
}

fn item_target(outline: &PaperOutline, item: &OutlineItem, label: String) -> ScopeTarget {
    let max_line = outline.total_lines.max(item.line);
    ScopeTarget {
        kind: item.kind.clone(),
        label,
        title: (!item.caption.is_empty()).then(|| item.caption.clone()),
        start_line: item.line.saturating_sub(ITEM_CONTEXT_RADIUS).max(1),
        end_line: (item.line + ITEM_CONTEXT_RADIUS).min(max_line),
        page_start: item.page,
        page_end: item.page,
    }
}

fn merge_ranges(mut ranges: Vec<[usize; 2]>) -> Vec<[usize; 2]> {
    ranges.sort_by_key(|r| (r[0], r[1]));
    let mut merged: Vec<[usize; 2]> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut() {
            if r[0] <= last[1].saturating_add(1) {
                last[1] = last[1].max(r[1]);
                continue;
            }
        }
        merged.push(r);
    }
    merged
}

fn build_scope_label(targets: &[ScopeTarget]) -> String {
    if targets.len() == 1 {
        let t = &targets[0];
        format!("{} (lines {}–{})", t.label, t.start_line, t.end_line)
    } else {
        targets.iter().map(|t| t.label.as_str()).collect::<Vec<_>>().join(", ")
    }
}

fn front_matter_range(outline: &PaperOutline) -> Option<[usize; 2]> {
    if outline.total_lines == 0 {
        return None;
    }
    if let Some(abs) = outline.sections.iter().find(|s| s.kind == "front_matter") {
        return Some([1, abs.end_line.min(120)]);
    }
    let first_body = outline
        .sections
        .iter()
        .find(|s| s.kind == "section")
        .map(|s| s.start_line.saturating_sub(1))
        .unwrap_or(outline.total_lines);
    Some([1, first_body.clamp(1, 120).min(outline.total_lines)])
}
