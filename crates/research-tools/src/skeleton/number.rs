//! LaTeX counter simulation for skeleton nodes (Investigate package V2).
//!
//! Scans the flattened TeX stream for `\appendix`, `\numberwithin` / `\counterwithin`,
//! and `\newenvironment` / `\renewenvironment` wrapper aliases (skipping comments,
//! verbatim environments, and `\iffalse...\fi` blocks), then simulates LaTeX section,
//! theorem, equation, figure, and table counters in document order.

use super::flatten::{is_nested_tex_if, FlattenedLine, FlattenedTex, VERBATIM_ENVS};
use super::model::{NumberBasis, SkNode, SkNodeKind, Span};
use super::tex_lex::{
    has_active_begin_document, next_char_boundary, read_balanced_group, skip_inline_verb,
    skip_ws_and_comments, GroupOutcome,
};
use super::tex_theorems::TheoremTable;
use crate::tex_scan::strip_latex_comments_line;
use std::collections::{BTreeMap, BTreeSet};

const LEVEL_NAMES: [&str; 5] = ["part", "chapter", "section", "subsection", "subsubsection"];

/// Assigns computed LaTeX numbers to `nodes` in document order while preserving
/// explicit printed tags (e.g. `\tag{S1}` with `NumberBasis::Printed`).
pub fn assign_numbers(
    nodes: &mut [SkNode],
    _parent_orders: &[Option<usize>],
    theorem_table: &TheoremTable,
    flattened: &FlattenedTex,
) {
    let dir = scan_directives(flattened, theorem_table);
    let mut origin_map: BTreeMap<(&str, usize), Vec<usize>> = BTreeMap::new();
    for (idx, fl) in flattened.lines.iter().enumerate() {
        origin_map.entry((fl.origin.path.as_str(), fl.origin.line)).or_default().push(idx + 1);
    }

    let mut st = CounterState::default();
    let mut next_nw = 0usize;
    if !nodes.iter().any(|n| !n.spans.is_empty()) {
        for (_, child, parent) in &dir.number_within {
            st.number_within.insert(child.clone(), parent.clone());
        }
        next_nw = dir.number_within.len();
    }

    let mut cursor = 0usize;
    for node in nodes.iter_mut() {
        if let Some(nf) =
            node.spans.first().and_then(|s| locate_span(s, &origin_map, flattened, cursor))
        {
            cursor = cursor.max(nf);
            if !st.in_appendix && dir.appendix_flat_line.is_some_and(|af| nf >= af) {
                st.in_appendix = true;
                st.levels[1..].fill(0);
            }
            while next_nw < dir.number_within.len() && dir.number_within[next_nw].0 <= nf {
                let (_, c, p) = &dir.number_within[next_nw];
                st.number_within.insert(c.clone(), p.clone());
                next_nw += 1;
            }
        }
        st.assign_node(node, theorem_table, &dir.wrapper_aliases);
    }
}

#[derive(Debug, Default)]
struct CounterState {
    /// `[part, chapter, section, subsection, subsubsection]`
    levels: [u32; 5],
    in_appendix: bool,
    /// Dynamic counters for `equation`, `figure`, `table`, and theorem root counters.
    counters: BTreeMap<String, u32>,
    number_within: BTreeMap<String, String>,
}

impl CounterState {
    fn format_level(&self, lvl: usize) -> String {
        match lvl {
            0 => self.levels[0].to_string(),
            1 => match self.levels[1] {
                0 => "0".into(),
                n if self.in_appendix => format_alpha(n),
                n => n.to_string(),
            },
            2 if self.levels[1] > 0 => format!("{}.{}", self.format_level(1), self.levels[2]),
            2 if self.in_appendix && self.levels[2] > 0 => format_alpha(self.levels[2]),
            2 => self.levels[2].to_string(),
            3 => format!("{}.{}", self.format_level(2), self.levels[3]),
            _ => format!("{}.{}", self.format_level(3), self.levels[4]),
        }
    }

    fn parent_of<'a>(
        nw: &'a BTreeMap<String, String>,
        name: &str,
        table: &'a TheoremTable,
    ) -> Option<&'a str> {
        nw.get(name)
            .map(String::as_str)
            .or_else(|| table.defs.get(name).and_then(|d| d.parent_counter.as_deref()))
    }

    fn step_level(&mut self, lvl: usize, table: &TheoremTable) -> String {
        self.levels[lvl] += 1;
        if lvl >= 1 {
            self.levels[lvl + 1..].fill(0);
            let parents = &LEVEL_NAMES[lvl..];
            for (name, count) in &mut self.counters {
                if Self::parent_of(&self.number_within, name, table)
                    .is_some_and(|p| parents.contains(&p))
                {
                    *count = 0;
                }
            }
        }
        self.format_level(lvl)
    }

    fn step_counter(&mut self, counter: &str, table: &TheoremTable) -> String {
        let entry = self.counters.entry(counter.to_string()).or_insert(0);
        *entry += 1;
        let n = *entry;
        let parent = Self::parent_of(&self.number_within, counter, table);
        match parent.and_then(|p| LEVEL_NAMES[1..].iter().position(|&k| k == p)) {
            Some(idx) => format!("{}.{n}", self.format_level(idx + 1)),
            None => n.to_string(),
        }
    }

    fn assign_node(
        &mut self,
        node: &mut SkNode,
        table: &TheoremTable,
        aliases: &BTreeMap<String, String>,
    ) {
        let computed = match node.kind {
            SkNodeKind::Section => node
                .env_name
                .as_deref()
                .filter(|e| !e.ends_with('*'))
                .and_then(|e| LEVEL_NAMES.iter().position(|&k| k == e))
                .map(|lvl| self.step_level(lvl, table)),
            SkNodeKind::Definition
            | SkNodeKind::Assumption
            | SkNodeKind::Theorem
            | SkNodeKind::Lemma
            | SkNodeKind::Proposition
            | SkNodeKind::Corollary
            | SkNodeKind::Conjecture
            | SkNodeKind::Claim
            | SkNodeKind::Example
            | SkNodeKind::Remark
            | SkNodeKind::Algorithm => node
                .env_name
                .as_deref()
                .and_then(|e| resolve_theorem_counter(e, table, aliases))
                .map(|root| match root.as_str() {
                    "subsection" => self.step_level(3, table),
                    "subsubsection" => self.step_level(4, table),
                    _ => self.step_counter(&root, table),
                }),
            SkNodeKind::Equation => {
                if node.number.is_some() && node.number_basis == Some(NumberBasis::Printed) {
                    return;
                }
                let env = node.env_name.as_deref().unwrap_or("");
                if env.ends_with('*') || env == "displaymath" {
                    None
                } else {
                    Some(self.step_counter("equation", table))
                }
            }
            SkNodeKind::Figure => Some(self.step_counter("figure", table)),
            SkNodeKind::Table => Some(self.step_counter("table", table)),
            SkNodeKind::FrontMatter | SkNodeKind::Proof | SkNodeKind::Bibliography => None,
        };
        node.number_basis = computed.as_ref().map(|_| NumberBasis::Computed);
        node.number = computed;
    }
}

fn resolve_theorem_counter(
    env_name: &str,
    table: &TheoremTable,
    aliases: &BTreeMap<String, String>,
) -> Option<String> {
    let starred = |n: &str| n.ends_with('*') || table.defs.get(n).is_some_and(|d| d.starred);
    if starred(env_name) {
        return None;
    }
    let mut cur = env_name.to_string();
    let mut visited = BTreeSet::from([cur.clone()]);
    loop {
        if let Some(inner) = aliases.get(&cur) {
            if starred(inner) {
                return None;
            }
            if visited.insert(inner.clone()) {
                cur = inner.clone();
                continue;
            }
        }
        if let Some(s) = table.defs.get(&cur).and_then(|d| d.shared_counter.as_deref()) {
            if visited.insert(s.to_string()) {
                cur = s.to_string();
                continue;
            }
        }
        return Some(cur);
    }
}

fn format_alpha(mut n: u32) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push((b'A' + ((n - 1) % 26) as u8) as char);
        n = (n - 1) / 26;
    }
    out.into_iter().rev().collect()
}

#[derive(Debug, Default)]
struct TexDirectives {
    appendix_flat_line: Option<usize>,
    number_within: Vec<(usize, String, String)>,
    wrapper_aliases: BTreeMap<String, String>,
}

fn scan_directives(flattened: &FlattenedTex, table: &TheoremTable) -> TexDirectives {
    let (mut stream, mut line_starts) = (String::new(), Vec::with_capacity(flattened.lines.len()));
    for (idx, fl) in flattened.lines.iter().enumerate() {
        if idx > 0 { stream.push('\n'); }
        line_starts.push(stream.len());
        stream.push_str(&strip_latex_comments_line(&fl.text));
    }
    let bytes = stream.as_bytes();
    let mut in_body = !has_active_begin_document(&stream);
    let mut dir = TexDirectives::default();
    let mut raw_wrappers: Vec<(String, Vec<String>)> = Vec::new();
    let (mut iffalse_depth, mut pending_else_fi, mut pos) = (0usize, 0usize, 0usize);

    while pos < bytes.len() {
        if bytes[pos] != b'\\' { pos = next_char_boundary(&stream, pos); continue; }
        let Some((cmd, cmd_end)) = read_cmd_name(&stream, pos) else {
            pos = next_char_boundary(&stream, (pos + 1).min(bytes.len()));
            continue;
        };
        if iffalse_depth > 0 {
            if cmd == "fi" { iffalse_depth -= 1; }
            else if cmd == "else" && iffalse_depth == 1 { (iffalse_depth, pending_else_fi) = (0, pending_else_fi + 1); }
            else if is_nested_tex_if(cmd) { iffalse_depth += 1; }
            pos = cmd_end;
            continue;
        }
        let flat_line = line_starts.partition_point(|&s| s <= pos.min(stream.len())).max(1);
        pos = match cmd {
            "verb" => skip_inline_verb(&stream, cmd_end),
            "iffalse" => { iffalse_depth = 1; cmd_end }
            "fi" if pending_else_fi > 0 => { pending_else_fi -= 1; cmd_end }
            "begin" => match read_braced(&stream, cmd_end, false) {
                Some((raw, np)) => {
                    let env = raw.trim();
                    if VERBATIM_ENVS.contains(&env) {
                        let m = format!("\\end{{{env}}}");
                        stream[np..].find(&m).map_or(bytes.len(), |r| np + r + m.len())
                    } else {
                        if env == "document" { in_body = true; }
                        else if env == "appendices" && in_body { dir.appendix_flat_line.get_or_insert(flat_line); }
                        np
                    }
                }
                None => cmd_end,
            },
            "appendix" if in_body => { dir.appendix_flat_line.get_or_insert(flat_line); cmd_end }
            "numberwithin" | "counterwithin" => {
                let (np, parsed) = parse_number_within(&stream, cmd_end, cmd == "counterwithin");
                if let Some((c, p)) = parsed { dir.number_within.push((flat_line, c, p)); }
                np.max(cmd_end)
            }
            "newenvironment" | "renewenvironment" => {
                let (np, parsed) = parse_newenvironment(&stream, cmd_end);
                if let Some(entry) = parsed { raw_wrappers.push(entry); }
                np.max(cmd_end)
            }
            _ => cmd_end,
        };
    }

    let names: BTreeSet<&str> = raw_wrappers.iter().map(|(o, _)| o.as_str()).collect();
    for (outer, cands) in &raw_wrappers {
        let inner = cands.iter().find(|c| *c != outer && table.lookup(c).is_some())
            .or_else(|| cands.iter().find(|c| *c != outer && names.contains(c.as_str())))
            .or_else(|| cands.iter().find(|c| *c != outer));
        if let Some(target) = inner { dir.wrapper_aliases.insert(outer.clone(), target.clone()); }
    }
    dir
}

fn read_cmd_name(text: &str, bs: usize) -> Option<(&str, usize)> {
    let b = text.as_bytes();
    if !b.get(bs + 1).is_some_and(u8::is_ascii_alphabetic) { return None; }
    let mut end = bs + 1;
    while b.get(end).is_some_and(u8::is_ascii_alphabetic) { end += 1; }
    Some((&text[bs + 1..end], end))
}

fn read_braced(text: &str, pos: usize, allow_par: bool) -> Option<(String, usize)> {
    let p = skip_ws_and_comments(text, pos);
    if text.as_bytes().get(p) != Some(&b'{') { return None; }
    match read_balanced_group(text, p, b'{', b'}', allow_par) {
        GroupOutcome::Closed { content, next_pos } => Some((content, next_pos)),
        GroupOutcome::Unclosed { .. } => None,
    }
}

fn parse_number_within(text: &str, cmd_end: usize, star: bool) -> (usize, Option<(String, String)>) {
    let p0 = if star && text.as_bytes().get(cmd_end) == Some(&b'*') { cmd_end + 1 } else { cmd_end };
    let Some((child, p1)) = read_braced(text, p0, false) else { return (cmd_end, None) };
    let Some((parent, p2)) = read_braced(text, p1, false) else { return (p1, None) };
    let (c, p) = (child.trim(), parent.trim());
    if c.is_empty() || p.is_empty() { (p2, None) } else { (p2, Some((c.into(), p.into()))) }
}

fn parse_newenvironment(text: &str, cmd_end: usize) -> (usize, Option<(String, Vec<String>)>) {
    let bytes = text.as_bytes();
    let p0 = if bytes.get(cmd_end) == Some(&b'*') { cmd_end + 1 } else { cmd_end };
    let Some((outer, p1)) = read_braced(text, p0, false) else { return (cmd_end, None) };
    let mut pos = skip_ws_and_comments(text, p1);
    for _ in 0..2 {
        if bytes.get(pos) == Some(&b'[') {
            match read_balanced_group(text, pos, b'[', b']', false) {
                GroupOutcome::Closed { next_pos, .. } => pos = skip_ws_and_comments(text, next_pos),
                GroupOutcome::Unclosed { next_pos, .. } => return (next_pos, None),
            }
        }
    }
    let Some((begin_body, p2)) = read_braced(text, pos, true) else { return (pos, None) };
    pos = read_braced(text, p2, true).map_or(p2, |(_, np)| np);
    let (outer, envs) = (outer.trim(), extract_begin_envs(&begin_body));
    if outer.is_empty() || envs.is_empty() { (pos, None) } else { (pos, Some((outer.into(), envs))) }
}

fn extract_begin_envs(body: &str) -> Vec<String> {
    let (bytes, mut out, mut pos) = (body.as_bytes(), Vec::new(), 0usize);
    while pos < bytes.len() {
        if bytes[pos] != b'\\' { pos = next_char_boundary(body, pos); continue; }
        let Some((cmd, end)) = read_cmd_name(body, pos) else {
            pos = next_char_boundary(body, (pos + 1).min(bytes.len()));
            continue;
        };
        if cmd == "begin" {
            if let Some((raw, np)) = read_braced(body, end, false) {
                if !raw.trim().is_empty() { out.push(raw.trim().to_string()); }
                pos = np;
                continue;
            }
        }
        pos = end;
    }
    out
}

fn locate_span(
    span: &Span,
    origin_map: &BTreeMap<(&str, usize), Vec<usize>>,
    flattened: &FlattenedTex,
    min_flat: usize,
) -> Option<usize> {
    if let Some(lines) = origin_map.get(&(span.path.as_str(), span.start_line)) {
        let i = lines.partition_point(|&fl| fl < min_flat);
        return lines.get(i).or_else(|| lines.first()).copied();
    }
    let start = min_flat.saturating_sub(1).min(flattened.lines.len());
    let hit = |fl: &FlattenedLine| {
        fl.origin.path == span.path && fl.origin.line >= span.start_line && fl.origin.line <= span.end_line
    };
    flattened.lines[start..].iter().position(hit).map(|off| start + off + 1)
        .or_else(|| flattened.lines.iter().position(hit).map(|i| i + 1))
}

#[cfg(test)]
mod tests {
    use super::super::flatten::flatten_tex_files;
    use super::super::tex_env::scan_tex_stream;
    use super::*;

    fn run(files: &[(&str, &str)]) -> Vec<SkNode> {
        let map: BTreeMap<String, String> =
            files.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        let flat = flatten_tex_files(&map, Some("main.tex")).expect("flatten");
        let mut scan = scan_tex_stream(&flat);
        assign_numbers(&mut scan.nodes, &scan.parent_orders, &scan.theorem_table, &flat);
        scan.nodes
    }

    #[test]
    fn shared_counters_and_unnumbered_theorems() {
        let tex = "\\documentclass{article}\n\\newtheorem{thm}{Theorem}[section]\n\
\\newtheorem{lem}[thm]{Lemma}\n\\newtheorem*{rem*}{Remark}\n\
\\declaretheorem[numbered=no, name=Note]{unnumnote}\n\\begin{document}\n\
\\section{Intro}\n\\begin{thm}A\\end{thm}\n\\begin{lem}B\\end{lem}\n\
\\begin{rem*}C\\end{rem*}\n\\begin{unnumnote}D\\end{unnumnote}\n\
\\section*{Ack}\n\\section{Next}\n\\begin{thm}E\\end{thm}\n\\end{document}";
        let got: Vec<_> = run(&[("main.tex", tex)])
            .into_iter()
            .map(|n| (n.kind, n.number, n.number_basis))
            .collect();
        let c = Some(NumberBasis::Computed);
        assert_eq!(got, vec![
            (SkNodeKind::Section, Some("1".into()), c),
            (SkNodeKind::Theorem, Some("1.1".into()), c),
            (SkNodeKind::Lemma, Some("1.2".into()), c),
            (SkNodeKind::Remark, None, None),
            (SkNodeKind::Remark, None, None),
            (SkNodeKind::Section, None, None),
            (SkNodeKind::Section, Some("2".into()), c),
            (SkNodeKind::Theorem, Some("2.1".into()), c),
        ]);
    }

    #[test]
    fn newenvironment_wrapper_shares_counter() {
        let tex = "\\documentclass{article}\n\\newtheorem{mainres}{Main Result}\n\
\\newenvironment{boxedres}{\\begin{mainres}}{\\end{mainres}}\n\\begin{document}\n\
\\begin{mainres}1\\end{mainres}\n\\begin{boxedres}2\\end{boxedres}\n\
\\begin{mainres}3\\end{mainres}\n\\end{document}";
        let nums: Vec<_> = run(&[("main.tex", tex)]).into_iter().map(|n| n.number).collect();
        assert_eq!(nums, vec![Some("1".into()), Some("2".into()), Some("3".into())]);
    }

    #[test]
    fn numberwithin_equation_figure_table_resets() {
        let tex = "\\documentclass{article}\n\\numberwithin{equation}{section}\n\
\\counterwithin{figure}{section}\n\\counterwithin*{table}{section}\n\\begin{document}\n\
\\section{First}\n\\begin{equation}\\label{e1}a=b\\end{equation}\n\
\\begin{figure}\\caption{F1}\\end{figure}\n\\begin{table}\\caption{T1}\\end{table}\n\
\\section{Second}\n\\begin{equation}\\label{e2}c=d\\end{equation}\n\
\\begin{figure}\\caption{F2}\\end{figure}\n\\begin{table}\\caption{T2}\\end{table}\n\\end{document}";
        let got: Vec<_> = run(&[("main.tex", tex)]).into_iter().map(|n| (n.kind, n.number)).collect();
        assert_eq!(got, vec![
            (SkNodeKind::Section, Some("1".into())), (SkNodeKind::Equation, Some("1.1".into())),
            (SkNodeKind::Figure, Some("1.1".into())), (SkNodeKind::Table, Some("1.1".into())),
            (SkNodeKind::Section, Some("2".into())), (SkNodeKind::Equation, Some("2.1".into())),
            (SkNodeKind::Figure, Some("2.1".into())), (SkNodeKind::Table, Some("2.1".into())),
        ]);
    }

    #[test]
    fn appendix_lettering_multifile_and_skipped_inactive_appendix() {
        let main = "\\documentclass{article}\n\\newtheorem{thm}{Theorem}[section]\n\\begin{document}\n\
% \\appendix\n\\begin{verbatim}\n\\appendix\n\\end{verbatim}\n\\iffalse\n\\appendix\n\\fi\n\
\\section{Main}\n\\subsection{Setup}\n\\appendix\n\\input{app.tex}\n\\end{document}";
        let app = "\\section{Proofs}\n\\subsection{Helper}\n\\subsection{Detail}\n\\begin{thm}T\\end{thm}";
        let got: Vec<_> = run(&[("main.tex", main), ("app.tex", app)])
            .into_iter()
            .map(|n| (n.kind, n.number))
            .collect();
        assert_eq!(got, vec![
            (SkNodeKind::Section, Some("1".into())), (SkNodeKind::Section, Some("1.1".into())),
            (SkNodeKind::Section, Some("A".into())), (SkNodeKind::Section, Some("A.1".into())),
            (SkNodeKind::Section, Some("A.2".into())), (SkNodeKind::Theorem, Some("A.1".into())),
        ]);
    }

    #[test]
    fn printed_tag_preserved_without_incrementing_counter() {
        let tex = "\\documentclass{article}\n\\begin{document}\n\
\\begin{equation}\\label{e1}x=1\\end{equation}\n\
\\begin{equation}\\label{es1}\\tag{S1}y=2\\end{equation}\n\
\\begin{equation}\\label{e2}z=3\\end{equation}\n\
\\begin{equation*}\\label{es}w=4\\end{equation*}\n\\end{document}";
        let got: Vec<_> =
            run(&[("main.tex", tex)]).into_iter().map(|n| (n.number, n.number_basis)).collect();
        assert_eq!(got, vec![
            (Some("1".into()), Some(NumberBasis::Computed)),
            (Some("S1".into()), Some(NumberBasis::Printed)),
            (Some("2".into()), Some(NumberBasis::Computed)),
            (None, None),
        ]);
    }
}
