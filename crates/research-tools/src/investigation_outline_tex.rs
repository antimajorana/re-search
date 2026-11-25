//! LaTeX section, equation, and caption outline extraction for [`super::build_outline`].

use super::{clip_chars, make_section, page_for_line, OutlineItem, OutlineSection};
use regex::Regex;
use std::sync::OnceLock;

pub(super) fn looks_like_tex(lines: &[&str]) -> bool {
    let mut count = 0usize;
    for line in lines {
        let t = strip_tex_comment(line).trim();
        if ["\\section", "\\subsection", "\\subsubsection", "\\chapter"]
            .iter()
            .any(|k| t.contains(k))
        {
            count += 1;
            if count >= 2 {
                return true;
            }
        }
    }
    false
}

pub(super) fn build_tex_outline(
    lines: &[&str],
    page_lines: &[(usize, usize, usize)],
) -> (Vec<OutlineSection>, String, Vec<OutlineItem>, Vec<OutlineItem>) {
    static CMD_RE: OnceLock<Regex> = OnceLock::new();
    static TAG_RE: OnceLock<Regex> = OnceLock::new();
    let cmd_re = CMD_RE.get_or_init(|| {
        Regex::new(r"\\(chapter|section|subsection|subsubsection)(\*)?\s*(?:\[[^\]]*\])?\s*\{")
            .unwrap()
    });
    let tag_re = TAG_RE.get_or_init(|| Regex::new(r"\\tag\{([^}]+)\}").unwrap());

    let (mut sections, mut equations, mut captions) = (Vec::new(), Vec::new(), Vec::new());
    let (mut sec_num, mut sub_num, mut subsub_num) = (0usize, 0usize, 0usize);
    let (mut eq_num, mut fig_num, mut tab_num) = (0usize, 0usize, 0usize);
    let mut in_appendix = false;
    let mut env_kind: Option<&str> = None;

    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let line = strip_tex_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.contains("\\appendix") && !line.contains("\\appendixname") {
            in_appendix = true;
            (sec_num, sub_num, subsub_num) = (0, 0, 0);
        }
        if line.contains("\\begin{abstract}") {
            sections.push(make_section("Abstract".into(), "Abstract".into(), 1, "front_matter", line_no));
        }
        if line.contains("\\begin{thebibliography}") || line.contains("\\bibliography{") {
            sections.push(make_section("References".into(), "References".into(), 1, "back_matter", line_no));
        }
        if let Some(cap) = cmd_re.captures(line) {
            let cmd = &cap[1];
            let starred = cap.get(2).is_some();
            let title = extract_braced(&line[cap.get(0).unwrap().end() - 1..])
                .unwrap_or_default()
                .trim()
                .to_string();
            let level = match cmd {
                "chapter" | "section" => 1,
                "subsection" => 2,
                _ => 3,
            };
            let (key, kind) = if starred {
                let k = if title.is_empty() { cmd.to_string() } else { title.clone() };
                let kind = if is_back_matter_name(&title) { "back_matter" } else { "section" };
                (k, kind)
            } else {
                let kind = if in_appendix { "appendix" } else { "section" };
                let parent = |n: usize| if in_appendix { appendix_letter(n) } else { n.to_string() };
                match level {
                    1 => {
                        sec_num += 1;
                        (sub_num, subsub_num) = (0, 0);
                        (parent(sec_num), kind)
                    }
                    2 => {
                        sub_num += 1;
                        subsub_num = 0;
                        (format!("{}.{sub_num}", parent(sec_num.max(1))), kind)
                    }
                    _ => {
                        subsub_num += 1;
                        (format!("{}.{}.{subsub_num}", parent(sec_num.max(1)), sub_num.max(1)), kind)
                    }
                }
            };
            sections.push(make_section(key, title, level, kind, line_no));
        }

        if ["\\begin{equation}", "\\begin{align}", "\\begin{gather}", "\\begin{eqnarray}"]
            .iter()
            .any(|e| line.contains(e))
        {
            let label = tag_re
                .captures(line)
                .map(|t| t[1].trim().to_string())
                .unwrap_or_else(|| {
                    eq_num += 1;
                    eq_num.to_string()
                });
            equations.push(OutlineItem {
                kind: "equation".into(),
                label,
                caption: clip_chars(line, 120),
                line: line_no,
                page: page_for_line(page_lines, line_no),
            });
        } else if let Some(t) = tag_re.captures(line) {
            equations.push(OutlineItem {
                kind: "equation".into(),
                label: t[1].trim().to_string(),
                caption: clip_chars(line, 120),
                line: line_no,
                page: page_for_line(page_lines, line_no),
            });
        }

        if line.contains("\\begin{figure") {
            env_kind = Some("figure");
        } else if line.contains("\\begin{table") {
            env_kind = Some("table");
        } else if line.contains("\\end{figure") || line.contains("\\end{table") {
            env_kind = None;
        }
        if let Some(pos) = line.find("\\caption") {
            let after = &line[pos + "\\caption".len()..];
            if let Some(brace) = after.find('{') {
                let text = extract_braced(&after[brace..]).unwrap_or_default();
                let kind = env_kind.unwrap_or("figure");
                let label = if kind == "table" {
                    tab_num += 1;
                    tab_num.to_string()
                } else {
                    fig_num += 1;
                    fig_num.to_string()
                };
                captions.push(OutlineItem {
                    kind: kind.into(),
                    label,
                    caption: clip_chars(text.trim(), 140),
                    line: line_no,
                    page: page_for_line(page_lines, line_no),
                });
            }
        }
    }

    (sections, "tex".into(), equations, captions)
}

fn strip_tex_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'%' && (i == 0 || b[i - 1] != b'\\') {
            return &line[..i];
        }
    }
    line
}

fn extract_braced(s: &str) -> Option<String> {
    let (mut depth, mut started, mut out) = (0usize, false, String::new());
    for ch in s.chars() {
        match ch {
            '{' => {
                if started {
                    out.push(ch);
                }
                started = true;
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(out);
                }
                out.push(ch);
            }
            _ if started => out.push(ch),
            _ => {}
        }
    }
    started.then_some(out)
}

fn appendix_letter(n: usize) -> String {
    let idx = n.saturating_sub(1);
    if idx < 26 { ((b'A' + idx as u8) as char).to_string() } else { format!("A{n}") }
}

fn is_back_matter_name(title: &str) -> bool {
    matches!(
        title.trim().to_ascii_lowercase().as_str(),
        "acknowledgments" | "acknowledgements" | "references" | "bibliography"
    )
}
