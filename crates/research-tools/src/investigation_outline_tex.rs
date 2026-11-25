//! LaTeX section, equation, and caption outline extraction for [`super::build_outline`].

use super::{clip_chars, make_section, page_for_line, OutlineItem, OutlineSection};
use regex::Regex;
use std::sync::OnceLock;

pub(super) fn looks_like_tex(lines: &[&str]) -> bool {
    let mut sec_count = 0usize;
    let mut has_documentclass = false;
    let mut has_begin_document = false;
    for line in lines {
        let t = strip_tex_comment(line).trim();
        if t.contains("\\documentclass") {
            has_documentclass = true;
        }
        if t.contains("\\begin{document}") {
            has_begin_document = true;
        }
        if ["\\section", "\\subsection", "\\subsubsection", "\\chapter"]
            .iter()
            .any(|k| t.contains(k))
        {
            sec_count += 1;
        }
        if sec_count >= 2 || (has_documentclass && has_begin_document) {
            return true;
        }
    }
    false
}

const MULTIROW_MATH_ENVS: &[&str] = &["align", "gather", "eqnarray", "alignat", "flalign"];
const NESTED_MATH_ENVS: &[&str] = &[
    "matrix", "cases", "pmatrix", "bmatrix", "Bmatrix", "vmatrix", "Vmatrix",
    "smallmatrix", "array", "split", "aligned", "gathered", "alignedat", "dcases", "rcases",
];

#[derive(Default)]
struct EqCounter {
    eq_num: usize,
    in_subequations: bool,
    sub_eq_letter: usize,
}

impl EqCounter {
    fn enter_subequations(&mut self) {
        if !self.in_subequations {
            self.eq_num += 1;
            self.in_subequations = true;
            self.sub_eq_letter = 0;
        }
    }

    fn exit_subequations(&mut self) {
        self.in_subequations = false;
        self.sub_eq_letter = 0;
    }

    fn next_label(&mut self) -> String {
        if self.in_subequations {
            let idx = self.sub_eq_letter;
            self.sub_eq_letter += 1;
            if idx < 26 {
                format!("{}{}", self.eq_num, (b'a' + idx as u8) as char)
            } else {
                format!("{}.{}", self.eq_num, idx + 1)
            }
        } else {
            self.eq_num += 1;
            self.eq_num.to_string()
        }
    }
}

struct MathEnvState {
    end_marker: String,
    starred: bool,
    multi_row: bool,
    begin_line: usize,
    begin_caption: String,
    /// `(line_no, body_segment_inside_env, full_trimmed_line)`
    body_lines: Vec<(usize, String, String)>,
}

#[derive(Default)]
struct MathRow {
    parts: Vec<(usize, String)>,
}

impl MathRow {
    fn push(&mut self, line_no: usize, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        if let Some((last_ln, last_s)) = self.parts.last_mut() {
            if *last_ln == line_no {
                last_s.push_str(chunk);
                return;
            }
        }
        self.parts.push((line_no, chunk.to_string()));
    }
}

fn match_nested_env_cmd(s: &str, cmd: &str) -> Option<usize> {
    let rest = s.strip_prefix(cmd)?;
    let close = rest.find('}')?;
    let inner = rest[..close].trim();
    let base = inner.strip_suffix('*').unwrap_or(inner);
    NESTED_MATH_ENVS.contains(&base).then_some(cmd.len() + close + 1)
}

fn split_multirow_body(body_lines: &[(usize, String, String)]) -> Vec<MathRow> {
    let mut rows: Vec<MathRow> = Vec::new();
    let mut cur = MathRow::default();
    let mut nested_depth = 0usize;

    for (line_no, seg, _) in body_lines {
        let bytes = seg.as_bytes();
        let mut i = 0usize;
        let mut seg_start = 0usize;
        while i < bytes.len() {
            if let Some(len) = match_nested_env_cmd(&seg[i..], "\\begin{") {
                nested_depth += 1;
                i += len;
                continue;
            }
            if let Some(len) = match_nested_env_cmd(&seg[i..], "\\end{") {
                nested_depth = nested_depth.saturating_sub(1);
                i += len;
                continue;
            }
            if nested_depth == 0 && seg[i..].starts_with("\\\\") {
                cur.push(*line_no, &seg[seg_start..i]);
                rows.push(std::mem::take(&mut cur));
                i += 2;
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'*' {
                    i += 1;
                }
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'[' {
                    if let Some(rel_end) = seg[i + 1..].find(']') {
                        let arg = &seg[i + 1..i + 1 + rel_end];
                        if !arg.contains('=') && !arg.contains("\\begin") {
                            i = i + 1 + rel_end + 1;
                        }
                    }
                }
                seg_start = i;
                continue;
            }
            i += seg[i..].chars().next().map_or(1, char::len_utf8);
        }
        if seg_start < bytes.len() {
            cur.push(*line_no, &seg[seg_start..]);
        }
    }
    rows.push(cur);
    rows
}

fn has_nonumber_or_notag(s: &str) -> bool {
    static NONUM_RE: OnceLock<Regex> = OnceLock::new();
    NONUM_RE
        .get_or_init(|| Regex::new(r"\\(?:nonumber|notag)\b").unwrap())
        .is_match(s)
}

fn extract_tags(s: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if s[i..].starts_with("\\tag") {
            let mut bs = 0usize;
            while bs < i && bytes[i - 1 - bs] == b'\\' {
                bs += 1;
            }
            if bs.is_multiple_of(2) {
                let mut j = i + 4;
                if j < bytes.len() && bytes[j] == b'*' {
                    j += 1;
                }
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'{' {
                    if let Some(content) = extract_braced(&s[j..]) {
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            tags.push(trimmed.to_string());
                        }
                        i = (j + content.len() + 2).min(bytes.len());
                        continue;
                    }
                }
            }
        }
        i += s[i..].chars().next().map_or(1, char::len_utf8);
    }
    tags
}

fn strip_required_braced_arg(s: &str) -> &str {
    let trimmed = s.trim_start();
    if trimmed.starts_with('{') {
        if let Some(inner) = extract_braced(trimmed) {
            let skip = (inner.len() + 2).min(trimmed.len());
            return &trimmed[skip..];
        }
    }
    s
}

fn push_eq(
    equations: &mut Vec<OutlineItem>,
    label: String,
    caption: String,
    line_no: usize,
    page_lines: &[(usize, usize, usize)],
) {
    equations.push(OutlineItem {
        kind: "equation".into(),
        label,
        caption,
        line: line_no,
        page: page_for_line(page_lines, line_no),
    });
}

fn flush_math_env(
    state: MathEnvState,
    counter: &mut EqCounter,
    equations: &mut Vec<OutlineItem>,
    page_lines: &[(usize, usize, usize)],
) {
    if state.multi_row && !state.starred {
        let rows = split_multirow_body(&state.body_lines);
        for row in rows {
            let joined = row
                .parts
                .iter()
                .map(|(_, s)| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let mut row_tags = Vec::new();
            for (ln, part) in &row.parts {
                for label in extract_tags(part) {
                    let cap_src = if part.trim().is_empty() { joined.as_str() } else { part.trim() };
                    row_tags.push((label, *ln, clip_chars(cap_src, 120)));
                }
            }
            if !row_tags.is_empty() {
                for (label, ln, caption) in row_tags {
                    push_eq(equations, label, caption, ln, page_lines);
                }
            } else if joined.is_empty() || has_nonumber_or_notag(&joined) {
                continue;
            } else {
                let first_line = row
                    .parts
                    .iter()
                    .find(|(_, s)| !s.trim().is_empty())
                    .map(|(ln, _)| *ln)
                    .unwrap_or(state.begin_line);
                let label = counter.next_label();
                push_eq(equations, label, clip_chars(&joined, 120), first_line, page_lines);
            }
        }
        return;
    }

    let mut tags = Vec::new();
    let mut suppressed = false;
    for (ln, seg, full) in &state.body_lines {
        for label in extract_tags(seg) {
            tags.push((label, *ln, clip_chars(full, 120)));
        }
        if has_nonumber_or_notag(seg) {
            suppressed = true;
        }
    }
    if !tags.is_empty() {
        for (label, ln, caption) in tags {
            push_eq(equations, label, caption, ln, page_lines);
        }
    } else if !state.starred && !suppressed {
        let label = counter.next_label();
        push_eq(equations, label, state.begin_caption, state.begin_line, page_lines);
    }
}

pub(super) fn build_tex_outline(
    lines: &[&str],
    page_lines: &[(usize, usize, usize)],
) -> (Vec<OutlineSection>, String, Vec<OutlineItem>, Vec<OutlineItem>) {
    static CMD_RE: OnceLock<Regex> = OnceLock::new();
    static MATH_BEGIN_RE: OnceLock<Regex> = OnceLock::new();
    let cmd_re = CMD_RE.get_or_init(|| {
        Regex::new(r"\\(chapter|section|subsection|subsubsection)(\*)?\s*(?:\[[^\]]*\])?\s*\{")
            .unwrap()
    });
    let math_begin_re = MATH_BEGIN_RE.get_or_init(|| {
        Regex::new(r"\\begin\{(equation|align|gather|eqnarray|multline|alignat|flalign)(\*)?\}")
            .unwrap()
    });

    let (mut sections, mut equations, mut captions) = (Vec::new(), Vec::new(), Vec::new());
    let (mut sec_num, mut sub_num, mut subsub_num) = (0usize, 0usize, 0usize);
    let (mut fig_num, mut tab_num) = (0usize, 0usize);
    let mut eq_counter = EqCounter::default();
    let mut in_appendix = false;
    let mut env_kind: Option<&str> = None;
    let mut active_math: Option<MathEnvState> = None;

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

        if line.contains("\\begin{subequations}") {
            eq_counter.enter_subequations();
        }

        let mut line_body_slice: Option<&str> = None;
        if active_math.is_none() {
            if let Some(cap) = math_begin_re.captures(line) {
                let m = cap.get(0).unwrap();
                let env_name = &cap[1];
                let starred = cap.get(2).is_some();
                let suffix = if starred { "*" } else { "" };
                let mut after = &line[m.end()..];
                if env_name == "alignat" {
                    after = strip_required_braced_arg(after);
                }
                line_body_slice = Some(after);
                active_math = Some(MathEnvState {
                    end_marker: format!("\\end{{{env_name}{suffix}}}"),
                    starred,
                    multi_row: MULTIROW_MATH_ENVS.contains(&env_name),
                    begin_line: line_no,
                    begin_caption: clip_chars(line, 120),
                    body_lines: Vec::new(),
                });
            }
        }

        if let Some(state) = active_math.as_mut() {
            let seg = line_body_slice.unwrap_or(line);
            if let Some(end_pos) = seg.find(&state.end_marker) {
                state.body_lines.push((line_no, seg[..end_pos].to_string(), line.to_string()));
                if let Some(finished) = active_math.take() {
                    flush_math_env(finished, &mut eq_counter, &mut equations, page_lines);
                }
            } else {
                state.body_lines.push((line_no, seg.to_string(), line.to_string()));
            }
        } else {
            for label in extract_tags(line) {
                push_eq(&mut equations, label, clip_chars(line, 120), line_no, page_lines);
            }
        }

        if line.contains("\\end{subequations}") {
            eq_counter.exit_subequations();
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

    if let Some(finished) = active_math.take() {
        flush_math_env(finished, &mut eq_counter, &mut equations, page_lines);
    }

    (sections, "tex".into(), equations, captions)
}

fn strip_tex_comment(line: &str) -> &str {
    let b = line.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'%' {
            let mut bs = 0usize;
            while bs < i && b[i - 1 - bs] == b'\\' {
                bs += 1;
            }
            if bs.is_multiple_of(2) {
                return &line[..i];
            }
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
