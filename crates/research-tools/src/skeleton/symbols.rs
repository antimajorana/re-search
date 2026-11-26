//! Deterministic symbol-definition extraction and nearest-earlier lookup
//! (Investigate package V5: `investigate.md` §2, §3 Stage F, §4.1, §4.3).

use super::flatten::{is_nested_tex_if, FlattenedTex, VERBATIM_ENVS};
use super::model::{MacroDef, SkNode, Span, SymbolDef, SymbolDefHow};
use super::tex_lex::skip_inline_verb;
use crate::tex_scan::strip_latex_comments_line;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// Basis label for pattern/macro symbol hits (`investigate.md` §2, §4.3).
pub const SYMBOL_FOUND_LABEL: &str = "found by pattern";
/// Basis label when no definition precedes the reading position (`investigate.md` §2, §4.3).
pub const SYMBOL_NOT_FOUND_LABEL: &str = "No definition found above this line";
/// Cap on symbol definitions stored in `PaperSkeleton::symbols` (`investigate.md` §3 Stage F, §4.1).
pub const MAX_SKELETON_SYMBOLS: usize = 2_000;

/// Result of looking up the nearest earlier definition of a queried symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolLookupResult {
    pub query: String,
    pub norm: String,
    pub found: Option<SymbolDef>,
    pub basis_label: String,
}

/// Normalizes a raw TeX symbol: strips whitespace, math delimiters (`$`, `$$`, `\(`, `\)`,
/// `\[`, `\]`), layout commands (`\left`, `\right`, `\,`, `\;`, `\!`, `\quad`, `\qquad`,
/// `\ `), collapses internal whitespace, and unwraps redundant outer `{...}`.
pub fn normalize_symbol(raw: &str) -> String {
    let mut cur = collapse_ws(&strip_layout_commands(&strip_math_delimiters(raw)));
    loop {
        let next = collapse_ws(&strip_math_delimiters(strip_outer_braces(&cur)));
        if next == cur { return cur; }
        cur = next;
    }
}

/// Returns candidate lookup keys: `[norm]` plus, for a macro invocation like `\Lap{G}` or
/// `\Deg(v)`, the bare command stem (`\Lap`, `\Deg`).
pub fn symbol_lookup_keys(raw: &str) -> Vec<String> {
    let norm = normalize_symbol(raw);
    if norm.is_empty() { return Vec::new(); }
    let mut keys = vec![norm.clone()];
    if let Some(stem) = macro_command_stem(&norm).filter(|s| *s != norm) { keys.push(stem.to_string()); }
    keys
}

/// Extracts the LHS symbol from math such as `$G=(V,E)$`, `$f:V\to\R$`, `w_{uv}=w_{vu}>0`, or `K^* = \{...\}`.
pub fn extract_lhs_symbol(math_inner: &str) -> Option<String> {
    let stripped = strip_math_delimiters(math_inner);
    let cut = find_top_level_relation(&stripped).unwrap_or(stripped.len());
    let norm = normalize_symbol(&stripped[..cut]);
    (!norm.is_empty() && norm.len() <= 48 && norm.chars().any(|c| c.is_ascii_alphabetic())).then_some(norm)
}

/// Extracts `Newcommand` and `DefinePhrase` symbol definitions from `flattened`, skipping
/// comments, verbatim blocks, inline `\verb`, and `\iffalse...\fi`, deduplicated by `(norm, span)`.
pub fn extract_symbols(
    nodes: &[SkNode], parent_orders: &[Option<usize>], macros: &[MacroDef], flattened: &FlattenedTex,
) -> Vec<SymbolDef> {
    let _ = macros; // Synthetic MacroDefs not matched on a flattened line are skipped gracefully.
    let (span_index, active) = (SpanIndex::build(nodes, parent_orders), mask_active_lines(flattened));
    let has_doc = active.iter().any(|l| l.contains("\\begin{document}"));
    let (mut in_body, mut out, mut seen) = (!has_doc, Vec::new(), BTreeSet::<(String, Span)>::new());
    for (idx, line) in active.iter().enumerate() {
        if out.len() >= MAX_SKELETON_SYMBOLS { break; }
        let origin = &flattened.lines[idx].origin;
        let span = Span::new(origin.path.clone(), origin.line, origin.line);
        let in_node = span_index.enclosing(flattened, idx + 1).and_then(|i| nodes[i].tag.clone());
        let was_in_body = in_body;
        let (macro_slice, body_slice) = split_doc_line(line, has_doc, &mut in_body);
        let mut push = |sym: String, how: SymbolDefHow, owner: Option<String>| {
            if out.len() >= MAX_SKELETON_SYMBOLS { return; }
            let norm = normalize_symbol(&sym);
            if !norm.is_empty() && seen.insert((norm.clone(), span.clone())) {
                out.push(SymbolDef { symbol_tex: sym, norm, span: span.clone(), in_node: owner, how });
            }
        };
        let macro_owner = was_in_body.then(|| in_node.clone()).flatten();
        for cmd in scan_macro_defs(macro_slice) { push(cmd, SymbolDefHow::Newcommand, macro_owner.clone()); }
        if !body_slice.is_empty() {
            for sym in scan_define_phrases(body_slice) { push(sym, SymbolDefHow::DefinePhrase, in_node.clone()); }
        }
    }
    out
}

/// Finds the nearest earlier symbol definition before the given reading position.
pub fn lookup_nearest_symbol(
    symbols: &[SymbolDef], nodes: &[SkNode], flattened_order: Option<&FlattenedTex>,
    query_tex: &str, before_node_tag: Option<&str>, before_span: Option<&Span>,
) -> SymbolLookupResult {
    let (norm, keys) = (normalize_symbol(query_tex), symbol_lookup_keys(query_tex));
    let make = |found: Option<SymbolDef>| SymbolLookupResult {
        query: query_tex.to_string(), norm: norm.clone(),
        basis_label: if found.is_some() { SYMBOL_FOUND_LABEL } else { SYMBOL_NOT_FOUND_LABEL }.into(),
        found,
    };
    if keys.is_empty() { return make(None); }

    let tag_map: BTreeMap<&str, &SkNode> = nodes.iter().filter_map(|n| n.tag.as_deref().map(|t| (t, n))).collect();
    let t_node = before_node_tag.and_then(|t| tag_map.get(t).copied());
    let t_span = before_span.or_else(|| t_node.and_then(|n| n.spans.first()));
    let t_rank = t_node.map(|n| n.order + 1).or_else(|| t_span.and_then(|sp| {
        nodes.iter().filter(|n| n.spans.iter().any(|s| s.contains(sp))).max_by_key(|n| n.order).map(|n| n.order + 1)
    }));
    let fmap: Option<BTreeMap<&str, BTreeMap<usize, usize>>> = flattened_order.map(|f| {
        let mut m: BTreeMap<&str, BTreeMap<usize, usize>> = BTreeMap::new();
        for (i, fl) in f.lines.iter().enumerate() { m.entry(fl.origin.path.as_str()).or_default().entry(fl.origin.line).or_insert(i + 1); }
        m
    });
    let flat_of = |sp: &Span| fmap.as_ref().and_then(|m| m.get(sp.path.as_str())).and_then(|l| l.get(&sp.start_line)).copied();
    let (t_flat, bounded, mut best) = (t_span.and_then(flat_of), t_span.is_some() || t_rank.is_some(), None::<(usize, Option<usize>, usize, bool)>);

    for (idx, def) in symbols.iter().enumerate() {
        if !keys.iter().any(|k| k == &def.norm) { continue; }
        let d_rank = def.in_node.as_deref().and_then(|t| tag_map.get(t)).map(|n| n.order + 1).unwrap_or(0);
        let d_flat = flat_of(&def.span);
        let eq = def.how == SymbolDefHow::Newcommand && (def.in_node.is_none() || def.in_node.as_deref() == before_node_tag);
        if bounded && !is_before(def, d_flat, d_rank, t_flat, t_span, t_rank, eq) { continue; }
        let exact = def.norm == norm;
        if best.is_none_or(|(bi, bf, br, be)| match (d_flat, bf) {
            (Some(df), Some(bf)) => (df, exact, idx) > (bf, be, bi),
            _ if def.span.path == symbols[bi].span.path => (def.span.start_line, exact, idx) > (symbols[bi].span.start_line, be, bi),
            _ => (d_rank, def.span.start_line, exact, idx) > (br, symbols[bi].span.start_line, be, bi),
        }) { best = Some((idx, d_flat, d_rank, exact)); }
    }
    make(best.map(|(idx, _, _, _)| symbols[idx].clone()))
}

fn is_before(def: &SymbolDef, df: Option<usize>, dr: usize, tf: Option<usize>, tsp: Option<&Span>, tr: Option<usize>, eq: bool) -> bool {
    let cmp = |a: usize, b: usize| if eq { a <= b } else { a < b };
    match (df, tf) {
        (Some(d), Some(t)) => cmp(d, t),
        _ => match tsp {
            Some(sp) if def.span.path == sp.path => cmp(def.span.start_line, sp.start_line),
            Some(sp) => { let (l, r) = ((dr, def.span.start_line), (tr.unwrap_or(usize::MAX), sp.start_line)); if eq { l <= r } else { l < r } }
            None => tr.is_none_or(|r| cmp(dr, r)),
        },
    }
}

fn split_doc_line<'a>(line: &'a str, has_doc: bool, in_body: &mut bool) -> (&'a str, &'a str) {
    if !has_doc { return (line, line); }
    if !*in_body {
        let Some(p) = line.find("\\begin{document}") else { return (line, ""); };
        *in_body = true;
        let after = &line[p + 16..];
        return after.find("\\end{document}").map_or((line, after), |r| { *in_body = false; (&line[..p + 16 + r], &after[..r]) });
    }
    line.find("\\end{document}").map_or((line, line), |p| { *in_body = false; (&line[..p], &line[..p]) })
}

fn strip_math_delimiters(raw: &str) -> String {
    let mut s = raw.trim();
    loop {
        let next = s.strip_prefix("$$").and_then(|x| x.strip_suffix("$$")).filter(|_| even_bs(s.as_bytes(), s.len() - 2))
            .or_else(|| s.strip_prefix('$').and_then(|x| x.strip_suffix('$')).filter(|_| even_bs(s.as_bytes(), s.len() - 1)))
            .or_else(|| s.strip_prefix("\\(").and_then(|x| x.strip_suffix("\\)")))
            .or_else(|| s.strip_prefix("\\[").and_then(|x| x.strip_suffix("\\]")))
            .map(str::trim).unwrap_or(s);
        if next == s { return s.to_string(); }
        s = next;
    }
}

fn strip_layout_commands(s: &str) -> String {
    let (b, mut out, mut i) = (s.as_bytes(), String::with_capacity(s.len()), 0usize);
    while i < b.len() {
        if b[i] != b'\\' { let ch = s[i..].chars().next().unwrap_or(' '); out.push(ch); i += ch.len_utf8(); }
        else if i + 1 >= b.len() { out.push('\\'); break; }
        else if matches!(b[i + 1], b',' | b';' | b'!') { i += 2; }
        else if b[i + 1] == b' ' { out.push(' '); i += 2; }
        else if !b[i + 1].is_ascii_alphabetic() { let e = skip_esc(s, i); out.push_str(&s[i..e]); i = e; }
        else {
            let mut k = i + 1;
            while k < b.len() && b[k].is_ascii_alphabetic() { k += 1; }
            match &s[i + 1..k] { "left" | "right" => {} "quad" | "qquad" => out.push(' '), _ => out.push_str(&s[i..k]) }
            i = k;
        }
    }
    out
}

fn strip_outer_braces(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() < 2 || b[0] != b'{' || b[b.len() - 1] != b'}' { return s; }
    let (mut d, mut i) = (0usize, 0usize);
    while i < b.len() {
        if b[i] == b'\\' { i = skip_esc(s, i); continue; }
        if b[i] == b'{' { d += 1; } else if b[i] == b'}' { d = d.saturating_sub(1); if d == 0 && i + 1 < b.len() { return s; } }
        i += 1;
    }
    if d == 0 { s[1..b.len() - 1].trim() } else { s }
}

fn collapse_ws(s: &str) -> String { s.split_whitespace().collect::<Vec<_>>().join(" ") }

fn macro_command_stem(norm: &str) -> Option<&str> {
    let b = norm.as_bytes();
    if b.first() != Some(&b'\\') { return None; }
    let mut k = 1usize;
    while k < b.len() && b[k].is_ascii_alphabetic() { k += 1; }
    if k == 1 || k >= b.len() || matches!(&norm[1..k],
        "mathcal" | "mathbb" | "mathfrak" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt"
        | "mathscr" | "text" | "textrm" | "textbf" | "textit" | "operatorname" | "hat" | "bar"
        | "tilde" | "widehat" | "widetilde" | "overline" | "underline" | "dot" | "ddot" | "vec"
    ) { return None; }
    let r = norm[k..].trim_start();
    (r.starts_with('{') || r.starts_with('[') || r.starts_with('(')).then_some(&norm[..k])
}

fn find_top_level_relation(s: &str) -> Option<usize> {
    let (b, mut i, mut bd, mut pd) = (s.as_bytes(), 0usize, 0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && b[i + 1].is_ascii_alphabetic() => {
                let mut k = i + 1;
                while k < b.len() && b[k].is_ascii_alphabetic() { k += 1; }
                if bd == 0 && pd == 0 && matches!(&s[i + 1..k],
                    "in" | "notin" | "ni" | "subset" | "subseteq" | "supset" | "supseteq" | "coloneqq"
                    | "eqqcolon" | "equiv" | "colon" | "to" | "rightarrow" | "leftarrow" | "mapsto"
                    | "le" | "leq" | "ge" | "geq" | "ll" | "gg" | "ne" | "neq" | "sim" | "simeq"
                    | "approx" | "cong" | "propto" | "prec" | "succ" | "mid" | "perp" | "parallel"
                ) { return Some(i); }
                i = k;
            }
            b'\\' => i = skip_esc(s, i),
            b'{' => { bd += 1; i += 1; } b'}' => { bd = bd.saturating_sub(1); i += 1; }
            b'(' | b'[' => { pd += 1; i += 1; } b')' | b']' => { pd = pd.saturating_sub(1); i += 1; }
            b'=' | b':' | b'<' | b'>' | b',' if bd == 0 && pd == 0 => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn scan_macro_defs(line: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(
        r"\\(?:newcommand|renewcommand|providecommand|DeclareMathOperator)\*?\s*(?:\{\s*(\\[A-Za-z@]+)\s*\}|(\\[A-Za-z@]+))"
    ).unwrap());
    re.captures_iter(line).filter(|c| c.get(0).is_some_and(|m| even_bs(line.as_bytes(), m.start())))
        .filter_map(|c| c.get(1).or_else(|| c.get(2)).map(|g| g.as_str().to_string())).collect()
}

fn scan_define_phrases(line: &str) -> Vec<String> {
    static MATH_RE: OnceLock<Regex> = OnceLock::new();
    static PLAIN_RE: OnceLock<Regex> = OnceLock::new();
    static ASSIGN_RE: OnceLock<Regex> = OnceLock::new();
    let math_re = MATH_RE.get_or_init(|| Regex::new(r"(?i)\b(?:let|where|denote(?:\s+by)?|define|set|write|call)\s+\$([^$\r\n]{1,64})\$").unwrap());
    let plain_re = PLAIN_RE.get_or_init(|| Regex::new(r"(?i)\b(?:let|where|define|set)\s+([A-Za-z](?:_[A-Za-z0-9{}]+)?)\s+(?:be|denote|denotes|is|:=|=)").unwrap());
    let assign_re = ASSIGN_RE.get_or_init(|| Regex::new(r"^([^=\r\n]{1,32}?)\s*(?::=|\\coloneqq|\\equiv)").unwrap());
    let (mut out, b) = (Vec::new(), line.as_bytes());
    let ok = |s: usize| s == 0 || b[s - 1] != b'\\';
    out.extend(math_re.captures_iter(line).filter(|c| c.get(0).is_some_and(|m| ok(m.start()))).filter_map(|c| c.get(1).and_then(|g| extract_lhs_symbol(g.as_str()))));
    out.extend(plain_re.captures_iter(line).filter(|c| c.get(0).is_some_and(|m| ok(m.start()))).filter_map(|c| c.get(1).map(|g| g.as_str().to_string())));
    out.extend(extract_inline_math_spans(line).into_iter().filter_map(|s| assign_re.captures(s).and_then(|c| c.get(1)).and_then(|g| extract_lhs_symbol(g.as_str()))));
    out
}

fn extract_inline_math_spans(line: &str) -> Vec<&str> {
    let (b, mut spans, mut i) = (line.as_bytes(), Vec::new(), 0usize);
    while i < b.len() {
        if b[i] == b'\\' { i = skip_esc(line, i); continue; }
        if b[i] != b'$' { i += 1; continue; }
        let dbl = i + 1 < b.len() && b[i + 1] == b'$';
        let (start, mut j, mut closed) = (i + if dbl { 2 } else { 1 }, i + if dbl { 2 } else { 1 }, None);
        while j < b.len() {
            if b[j] == b'\\' { j = skip_esc(line, j); continue; }
            if dbl && j + 1 < b.len() && b[j] == b'$' && b[j + 1] == b'$' { closed = Some((j, j + 2)); break; }
            if !dbl && b[j] == b'$' { closed = Some((j, j + 1)); break; }
            j += 1;
        }
        let Some((end, next_i)) = closed else { break };
        spans.push(&line[start..end]); i = next_i;
    }
    spans
}

struct SpanIndex { by_path: BTreeMap<String, Vec<(usize, usize, usize)>>, depths: Vec<usize>, orders: Vec<usize> }
impl SpanIndex {
    fn build(nodes: &[SkNode], parents: &[Option<usize>]) -> Self {
        let n = nodes.len();
        let depths = (0..n).map(|mut c| {
            let mut d = 0;
            while d < n { let Some(p) = parents.get(c).copied().flatten().filter(|&p| p < n) else { break }; d += 1; c = p; }
            d
        }).collect();
        let mut by_path: BTreeMap<String, Vec<(usize, usize, usize)>> = BTreeMap::new();
        for (i, node) in nodes.iter().enumerate() { for s in &node.spans { by_path.entry(s.path.clone()).or_default().push((s.start_line, s.end_line, i)); } }
        by_path.values_mut().for_each(|v| v.sort_by_key(|&(s, _, _)| s));
        Self { by_path, depths, orders: nodes.iter().map(|n| n.order).collect() }
    }
    fn enclosing(&self, flattened: &FlattenedTex, flat_line: usize) -> Option<usize> {
        let o = flattened.origin_of(flat_line)?;
        let sp = self.by_path.get(&o.path)?;
        sp[..sp.partition_point(|&(s, _, _)| s <= o.line)].iter().filter(|&&(_, e, _)| o.line <= e)
            .max_by_key(|&&(s, e, i)| (self.depths[i], Reverse(e.saturating_sub(s) + 1), self.orders[i])).map(|&(_, _, i)| i)
    }
}

fn mask_active_lines(flattened: &FlattenedTex) -> Vec<String> {
    let (mut out, mut in_verb, mut iffalse) = (Vec::with_capacity(flattened.lines.len()), None::<String>, 0usize);
    for fl in &flattened.lines {
        let raw = fl.text.as_str();
        let seg = if let Some(env) = in_verb.as_deref() {
            let pat = format!("\\end{{{env}}}");
            let Some(rel) = raw.find(&pat) else { out.push(String::new()); continue; };
            in_verb = None; &raw[rel + pat.len()..]
        } else { raw };
        out.push(mask_segment(&strip_latex_comments_line(seg), &mut in_verb, &mut iffalse));
    }
    out
}

fn mask_segment(code: &str, in_verb: &mut Option<String>, iffalse: &mut usize) -> String {
    let (mut m, len, mut i) = (code.as_bytes().to_vec(), code.len(), 0usize);
    while i < len {
        if m[i] != b'\\' {
            let step = code[i..].chars().next().map_or(1, char::len_utf8);
            if *iffalse > 0 { m[i..i + step].fill(b' '); }
            i += step; continue;
        }
        if i + 1 >= len || !m[i + 1].is_ascii_alphabetic() {
            let end = skip_esc(code, i).min(len);
            if *iffalse > 0 { m[i..end].fill(b' '); }
            i = end; continue;
        }
        let mut k = i + 1;
        while k < len && m[k].is_ascii_alphabetic() { k += 1; }
        let cmd = &code[i + 1..k];
        if *iffalse > 0 {
            m[i..k].fill(b' ');
            if cmd == "fi" { *iffalse -= 1; } else if cmd == "else" && *iffalse == 1 { *iffalse = 0; } else if is_nested_tex_if(cmd) { *iffalse += 1; }
        } else if cmd == "verb" {
            let end = skip_inline_verb(code, k).min(len);
            if end > k { m[i..end].fill(b' '); i = end; continue; }
        } else if cmd == "iffalse" { *iffalse = 1; m[i..k].fill(b' '); }
        else if cmd == "begin" && code[k..].starts_with('{') {
            if let Some(rel) = code[k + 1..].find('}') {
                let env = code[k + 1..k + 1 + rel].trim();
                if VERBATIM_ENVS.contains(&env) {
                    let (after, pat) = (k + 2 + rel, format!("\\end{{{env}}}"));
                    if let Some(re) = code[after..].find(&pat) { let end = after + re + pat.len(); m[i..end].fill(b' '); i = end; continue; }
                    m[i..len].fill(b' '); *in_verb = Some(env.to_string()); break;
                }
            }
        }
        i = k;
    }
    String::from_utf8(m).unwrap_or_else(|_| code.to_string())
}

fn skip_esc(s: &str, pos: usize) -> usize {
    let n = pos + 1;
    if n >= s.len() { s.len() } else { n + s[n..].chars().next().map_or(0, char::len_utf8) }
}
fn even_bs(bytes: &[u8], mut pos: usize) -> bool {
    let mut c = 0usize;
    while pos > 0 && bytes[pos - 1] == b'\\' { c += 1; pos -= 1; }
    c.is_multiple_of(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::flatten::{FlattenedLine, OriginEntry};
    use crate::skeleton::model::{SkConfidence, SkNodeKind};

    fn make_flat(lines: &[&str]) -> FlattenedTex {
        FlattenedTex {
            primary_path: "main.tex".into(), inlined_files: vec!["main.tex".into()], warnings: Vec::new(),
            lines: lines.iter().enumerate().map(|(i, t)| FlattenedLine { text: (*t).into(), origin: OriginEntry { path: "main.tex".into(), line: i + 1 } }).collect(),
        }
    }
    fn make_sec(tag: &str, order: usize, start: usize, end: usize) -> SkNode {
        SkNode {
            tag: Some(tag.into()), kind: SkNodeKind::Section, number: None, number_basis: None, env_name: Some("section".into()),
            title: None, label: None, parent: None, order, spans: vec![Span::new("main.tex", start, end)], proof_of: None,
            head_cites: Vec::new(), intro_refs: false, fp: "000000000000".into(), words: 0, eq_count: 0, read_minutes: 0.0, confidence: SkConfidence::TexHeading,
        }
    }

    #[test]
    fn normalize_lhs_and_lookup_keys() {
        assert_eq!((normalize_symbol(" $F$ "), normalize_symbol(r"\Lap{G}"), normalize_symbol(r"\mathcal{H}_L")), ("F".into(), r"\Lap{G}".into(), r"\mathcal{H}_L".into()));
        assert_eq!((normalize_symbol(" $ w_{uv} $ "), normalize_symbol(r"\left( \, x \quad \right)"), normalize_symbol(r"{F}")), ("w_{uv}".into(), "( x )".into(), "F".into()));
        assert_eq!((normalize_symbol(r"{a}{b}"), normalize_symbol(r"\leftarrow")), ("{a}{b}".into(), r"\leftarrow".into()));
        assert_eq!((extract_lhs_symbol("$G=(V,E)$").as_deref(), extract_lhs_symbol(r"$f:V\to\R$").as_deref()), (Some("G"), Some("f")));
        assert_eq!((extract_lhs_symbol("w_{uv}=w_{vu}>0").as_deref(), extract_lhs_symbol(r"K^* = \{v \in V\}").as_deref(), extract_lhs_symbol(r"0 < \varepsilon < 1")), (Some("w_{uv}"), Some("K^*"), None));
        assert_eq!((symbol_lookup_keys(r"$\Lap{G}$"), symbol_lookup_keys(r"\mathcal{H}_L")), (vec![r"\Lap{G}".into(), r"\Lap".into()], vec![r"\mathcal{H}_L".into()]));
    }

    #[test]
    fn extract_symbols_macros_phrases_and_skips_inactive() {
        let flat = make_flat(&[
            r"\newcommand{\R}{\mathbb{R}}", r"\newcommand{\Lap}[1]{L(#1)}", r"\DeclareMathOperator*{\Deg}{deg}", r"% \newcommand{\Commented}{x}",
            r"\begin{document}", r"\section{Setup}", r"Let $G=(V,E)$ be a graph, let $f:V\to\R$, and where k_0 is an integer.",
            r"We set $K^* := \{v \in V : f(v) > 0\}$, where $w_{uv}=w_{vu}>0$, and $\alpha \coloneqq 1$.",
            r"Here \verb|Let $Z=1$| and % Let $Q=2$", r"\begin{verbatim}", r"\newcommand{\VerbCmd}{0} Let $V_0 = 1$", r"\end{verbatim}",
            r"\iffalse", r"\newcommand{\FalseCmd}{0} Let $F_0 = 1$", r"\fi", r"\end{document}",
        ]);
        let syms = extract_symbols(&[make_sec("n-sec1", 0, 6, 15)], &[None], &[MacroDef { name: r"\Ghost".into(), arity: 0, body: "0".into() }], &flat);
        let norms: Vec<(&str, SymbolDefHow)> = syms.iter().map(|s| (s.norm.as_str(), s.how)).collect();
        for exp in [(r"\R", SymbolDefHow::Newcommand), (r"\Lap", SymbolDefHow::Newcommand), (r"\Deg", SymbolDefHow::Newcommand), ("G", SymbolDefHow::DefinePhrase),
            ("f", SymbolDefHow::DefinePhrase), ("k_0", SymbolDefHow::DefinePhrase), ("K^*", SymbolDefHow::DefinePhrase), ("w_{uv}", SymbolDefHow::DefinePhrase), (r"\alpha", SymbolDefHow::DefinePhrase)] {
            assert!(norms.contains(&exp), "missing {exp:?}");
        }
        for bad in [r"\Commented", "Z", "Q", r"\VerbCmd", "V_0", r"\FalseCmd", "F_0", r"\Ghost"] { assert!(!syms.iter().any(|s| s.norm == bad), "unexpected `{bad}`"); }
    }

    #[test]
    fn lookup_nearest_symbol_scoping_and_redefinition() {
        let flat = make_flat(&[
            r"\newcommand{\Lap}[1]{L(#1)}", r"\begin{document}", r"\section{One}", r"Intro line before any definition.", r"Let $x = 1$ be the first value.",
            r"Use $x$ and $\Lap{G}$ here.", r"\section{Two}", r"Let $x = 2$ be the redefined value.", r"Use $x$ after redefinition.", r"\end{document}",
        ]);
        let nodes = vec![make_sec("n-sec1", 0, 3, 6), make_sec("n-sec2", 1, 7, 9)];
        let syms = extract_symbols(&nodes, &[None, None], &[], &flat);
        for ord in [Some(&flat), None] {
            let before = lookup_nearest_symbol(&syms, &nodes, ord, "x", Some("n-sec1"), Some(&Span::new("main.tex", 4, 4)));
            assert_eq!((before.found, before.basis_label.as_str()), (None, SYMBOL_NOT_FOUND_LABEL));
            let s1 = lookup_nearest_symbol(&syms, &nodes, ord, "$x$", Some("n-sec1"), Some(&Span::new("main.tex", 6, 6)));
            assert_eq!((s1.basis_label.as_str(), s1.found.as_ref().map(|d| d.span.start_line)), (SYMBOL_FOUND_LABEL, Some(5)));
            let s2 = lookup_nearest_symbol(&syms, &nodes, ord, "x", Some("n-sec2"), Some(&Span::new("main.tex", 9, 9)));
            assert_eq!((s2.basis_label.as_str(), s2.found.as_ref().map(|d| d.span.start_line)), (SYMBOL_FOUND_LABEL, Some(8)));
            let mac = lookup_nearest_symbol(&syms, &nodes, ord, r"$\Lap{G}$", Some("n-sec1"), Some(&Span::new("main.tex", 6, 6)));
            assert_eq!((mac.basis_label.as_str(), mac.found.as_ref().map(|d| d.norm.as_str())), (SYMBOL_FOUND_LABEL, Some(r"\Lap")));
        }
    }
}
