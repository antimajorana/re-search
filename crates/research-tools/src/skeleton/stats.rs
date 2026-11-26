//! Per-node and per-section word counts and reading-time estimates (Investigate V2).

use super::flatten::FlattenedTex;
use super::model::{SectionStats, SkNode, SkNodeKind, Span};
use crate::tex_scan::strip_latex_comments_line;
use std::collections::{BTreeMap, BTreeSet};

pub const MATH_READ_WORDS_PER_MIN: f64 = 180.0;
pub const EQ_READ_MINUTES: f64 = 0.25;

const STRUCTURAL_ARG_CMDS: &[&str] =
    &["label", "ref", "eqref", "cref", "Cref", "autoref", "pageref", "begin", "end"];
const EXTRA_CITE_CMDS: &[&str] = &[
    "nocite", "parencite", "Parencite", "textcite", "Textcite", "autocite", "Autocite", "footcite",
    "Footcite", "fullcite", "smartcite", "supercite",
];

/// Counts prose words in TeX text after stripping comments, structural non-prose
/// command arguments (`\label`, `\ref`, `\begin`/`\end`, `\cite...`), and bare
/// control sequences while preserving prose inside formatting braces.
pub fn count_tex_words(text: &str) -> u32 {
    let stripped = text.lines().map(strip_latex_comments_line).collect::<Vec<_>>().join("\n");
    let bytes = stripped.as_bytes();
    let mut cleaned = String::with_capacity(stripped.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = skip_tex_command(&stripped, i);
            cleaned.push(' ');
        } else if matches!(bytes[i], b'{' | b'}' | b'~') {
            cleaned.push(' ');
            i += 1;
        } else {
            let ch = stripped[i..].chars().next().unwrap_or(' ');
            cleaned.push(ch);
            i += ch.len_utf8();
        }
    }
    let n = cleaned.split_whitespace().filter(|t| t.chars().any(char::is_alphanumeric)).count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Estimates reading time in minutes from prose word count and display equation count,
/// rounded to two decimal places.
pub fn estimate_read_minutes(words: u32, eq_count: u32) -> f64 {
    let mins =
        (f64::from(words) / MATH_READ_WORDS_PER_MIN) + (f64::from(eq_count) * EQ_READ_MINUTES);
    (mins * 100.0).round() / 100.0
}

/// Populates `words` and `read_minutes` on every node in `nodes` and returns
/// per-section [`SectionStats`] in document order (prepended by a `section_tag: None`
/// bucket if any non-section nodes have no `Section` ancestor).
pub fn compute_stats(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    flattened: &FlattenedTex,
) -> Vec<SectionStats> {
    let mut line_words: BTreeMap<&str, BTreeMap<usize, u32>> = BTreeMap::new();
    for fl in &flattened.lines {
        let by_line = line_words.entry(fl.origin.path.as_str()).or_default();
        by_line.entry(fl.origin.line).or_insert_with(|| count_tex_words(&fl.text));
    }

    let n = nodes.len();
    let (mut nearest_sec, mut sec_eq_desc) = (vec![None; n], vec![0u32; n]);
    for i in 0..n {
        let is_eq = nodes[i].kind == SkNodeKind::Equation;
        let (mut cur, mut steps) = (parent_orders.get(i).copied().flatten(), 0usize);
        while let Some(p) = cur.filter(|&p| p < n && steps < n) {
            if nodes[p].kind == SkNodeKind::Section {
                nearest_sec[i].get_or_insert(p);
                if is_eq {
                    sec_eq_desc[p] = sec_eq_desc[p].saturating_add(1);
                } else {
                    break;
                }
            }
            cur = parent_orders.get(p).copied().flatten();
            steps += 1;
        }
    }

    let mut sec_counts: BTreeMap<usize, BTreeMap<SkNodeKind, u32>> = BTreeMap::new();
    let mut top_counts: BTreeMap<SkNodeKind, u32> = BTreeMap::new();
    let (mut top_words, mut top_eqs, mut has_top) = (0u32, 0u32, false);
    for (i, node) in nodes.iter_mut().enumerate() {
        node.words = count_node_words(&node.spans, &line_words);
        let eqs = match node.kind {
            SkNodeKind::Equation => 1,
            SkNodeKind::Section => node.eq_count.saturating_add(sec_eq_desc[i]),
            _ => node.eq_count,
        };
        node.read_minutes = estimate_read_minutes(node.words, eqs);
        if let Some(sec_idx) = nearest_sec[i] {
            *sec_counts.entry(sec_idx).or_default().entry(node.kind).or_insert(0) += 1;
        } else if node.kind != SkNodeKind::Section {
            has_top = true;
            *top_counts.entry(node.kind).or_insert(0) += 1;
            if !parent_orders.get(i).copied().flatten().is_some_and(|p| p < n) {
                top_words = top_words.saturating_add(node.words);
            }
            top_eqs = top_eqs.saturating_add(eqs);
        }
    }

    let mut stats = Vec::new();
    if has_top {
        let read_minutes = estimate_read_minutes(top_words, top_eqs);
        stats.push(SectionStats {
            section_tag: None,
            words: top_words,
            read_minutes,
            counts: top_counts,
        });
    }
    for (i, node) in nodes.iter().enumerate().filter(|(_, n)| n.kind == SkNodeKind::Section) {
        stats.push(SectionStats {
            section_tag: node.tag.clone(),
            words: node.words,
            read_minutes: node.read_minutes,
            counts: sec_counts.remove(&i).unwrap_or_default(),
        });
    }
    stats
}

fn count_node_words(spans: &[Span], line_words: &BTreeMap<&str, BTreeMap<usize, u32>>) -> u32 {
    let mut seen: Option<BTreeSet<(&str, usize)>> = (spans.len() > 1).then(BTreeSet::new);
    let mut total = 0u32;
    for sp in spans.iter().filter(|sp| sp.start_line <= sp.end_line) {
        if let Some(m) = line_words.get(sp.path.as_str()) {
            for (&line, &w) in m.range(sp.start_line..=sp.end_line) {
                if seen.as_mut().is_none_or(|s| s.insert((sp.path.as_str(), line))) {
                    total = total.saturating_add(w);
                }
            }
        }
    }
    total
}

fn skip_tex_command(s: &str, backslash_pos: usize) -> usize {
    let bytes = s.as_bytes();
    let mut j = backslash_pos + 1;
    if j >= bytes.len() {
        return bytes.len();
    }
    if !bytes[j].is_ascii_alphabetic() {
        return j + s[j..].chars().next().map_or(0, char::len_utf8);
    }
    let cmd_start = j;
    while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
        j += 1;
    }
    let cmd = &s[cmd_start..j];
    if bytes.get(j) == Some(&b'*') {
        j += 1;
    }
    let skip_ws = |mut p: usize| {
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        p
    };
    if STRUCTURAL_ARG_CMDS.contains(&cmd) {
        return skip_delimited(s, skip_ws(j), b'{', b'}').unwrap_or(j);
    }
    if cmd.starts_with("cite") || cmd.starts_with("Cite") || EXTRA_CITE_CMDS.contains(&cmd) {
        let mut k = skip_ws(j);
        while let Some(after_opt) = skip_delimited(s, k, b'[', b']') {
            k = skip_ws(after_opt);
        }
        return skip_delimited(s, k, b'{', b'}').unwrap_or(j);
    }
    j
}

fn skip_delimited(s: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(start) != Some(&open) {
        return None;
    }
    let (mut depth, mut i) = (0usize, start);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            i += 1 + s[i + 1..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::flatten::{FlattenedLine, OriginEntry};
    use crate::skeleton::model::SkConfidence;

    #[rustfmt::skip]
    fn mk_node(order: usize, kind: SkNodeKind, tag: Option<&str>, spans: Vec<Span>) -> SkNode {
        SkNode {
            tag: tag.map(String::from), kind, number: None, number_basis: None, env_name: None,
            title: None, label: None, parent: None, order, spans, proof_of: None, head_cites: vec![],
            intro_refs: false, fp: String::new(), words: 0, eq_count: 0, read_minutes: 0.0,
            confidence: SkConfidence::TexEnv,
        }
    }

    fn mk_flat(lines: &[&str]) -> FlattenedTex {
        let lines = lines.iter().enumerate().map(|(i, text)| FlattenedLine {
            text: (*text).into(),
            origin: OriginEntry { path: "main.tex".into(), line: i + 1 },
        }).collect();
        FlattenedTex { primary_path: "main.tex".into(), lines, inlined_files: vec![], warnings: vec![] }
    }

    #[test]
    fn count_tex_words_and_read_minutes() {
        let tex = concat!(
            "\\begin{theorem}\\label{thm:main} % comment ignored words\n",
            "Let \\textbf{important} \\emph{lemma} hold, see \\ref{a}, \\eqref{b}, ",
            "\\cref{c}, \\Cref{d}, \\autoref{e}, \\pageref{f}, and \\cite[Thm. 1]{ns06}.\\\\",
            "Next\\,line 50\\% done.\n\\end{theorem}"
        );
        assert_eq!(count_tex_words(tex), 10);
        assert_eq!(estimate_read_minutes(0, 0), 0.0);
        assert_eq!(estimate_read_minutes(180, 0), 1.0);
        assert_eq!(estimate_read_minutes(90, 2), 1.0);
        assert_eq!(estimate_read_minutes(100, 1), 0.81);
    }

    #[test]
    fn compute_stats_nodes_front_matter_and_nested_sections() {
        let flat = mk_flat(&[
            "Abstract has four words.", "\\section{Intro}", "Intro prose here.",
            "\\begin{theorem}\\label{t1} One two three. \\end{theorem}",
            "\\subsection{Sub}", "\\begin{equation}\\label{e1} x = 1 \\end{equation}",
        ]);
        let sp = |a, b| vec![Span::new("main.tex", a, b)];
        let mut nodes = vec![
            mk_node(0, SkNodeKind::FrontMatter, Some("n-fm"), sp(1, 1)),
            mk_node(1, SkNodeKind::Section, Some("n-s1"), vec![Span::new("main.tex", 2, 5), Span::new("main.tex", 4, 6)]),
            mk_node(2, SkNodeKind::Theorem, Some("n-t1"), sp(4, 4)),
            mk_node(3, SkNodeKind::Section, Some("n-s2"), sp(5, 6)),
            mk_node(4, SkNodeKind::Equation, Some("n-e1"), sp(6, 6)),
        ];
        nodes[1].eq_count = 1;
        let stats = compute_stats(&mut nodes, &[None, None, Some(1), Some(1), Some(3)], &flat);
        assert_eq!((nodes[0].words, nodes[2].words, nodes[4].words), (4, 3, 2));
        assert_eq!(nodes[4].read_minutes, estimate_read_minutes(2, 1));
        assert_eq!((nodes[3].words, nodes[3].read_minutes), (3, estimate_read_minutes(3, 1)));
        assert_eq!((nodes[1].words, nodes[1].read_minutes), (10, estimate_read_minutes(10, 2)));
        assert_eq!(stats.len(), 3);
        assert_eq!((stats[0].section_tag.as_deref(), stats[0].words), (None, 4));
        assert_eq!(stats[0].counts.get(&SkNodeKind::FrontMatter), Some(&1));
        assert_eq!(stats[1].section_tag.as_deref(), Some("n-s1"));
        assert_eq!(stats[1].counts.get(&SkNodeKind::Theorem), Some(&1));
        assert_eq!(stats[1].counts.get(&SkNodeKind::Section), Some(&1));
        assert_eq!(stats[2].section_tag.as_deref(), Some("n-s2"));
        assert_eq!(stats[2].counts.get(&SkNodeKind::Equation), Some(&1));
    }
}
