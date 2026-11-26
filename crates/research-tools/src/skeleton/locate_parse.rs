//! Citation locator parser and TeX counter-drift detector for `skeleton::locate` (Investigate V11).

use super::flatten::{is_nested_tex_if, VERBATIM_ENVS};
use super::locate::{LocateCandidate, LocateContext, ParsedLocator, MAX_LOCATE_CANDIDATES};
use super::model::{NumberBasis, PaperSkeleton, SkNode, SkNodeKind, SkeletonBuilder};
use super::pdf_honesty::{node_page_basis, page_range_for_node, render_formulas_as_math};
use super::tex_theorems::TheoremTable;
use crate::pdf_pages::{page_of_line, PageBasis};
use crate::tex_scan::strip_latex_comments_line;
use std::collections::{BTreeMap, BTreeSet};

/// Parses a citation locator string into `(Option<SkNodeKind>, Option<String>, raw_locator, ambiguous)`.
///
/// Sets `ambiguous = true` when a locator names multiple distinct non-section statements
/// (e.g. `"Theorem 2.1, Lemma 3.2"`) or uses a prepositional reference without a number on
/// the primary keyword (e.g. `"Corollary to Theorem 2.1"`, `"Remark after Theorem 2.1"`),
/// ensuring Rule 6.9 never returns `Found` for the wrong statement.
pub fn parse_locator(raw: &str) -> ParsedLocator {
    let raw_locator = raw.trim().to_string();
    let cleaned = raw_locator.trim_matches(|c| matches!(c, '[' | ']' | ' ' | '\t' | '\n' | '\r'));
    if cleaned.is_empty() {
        return ParsedLocator { kind: None, number: None, raw_locator, ambiguous: false };
    }
    let mut best: Option<(u8, Option<SkNodeKind>, String, bool)> = None;
    let mut non_section_matches = 0usize;
    let mut any_ambiguous = false;

    for clause in cleaned.split([';', ',']) {
        if let Some(cand) = parse_locator_clause(clause) {
            if cand.3 {
                any_ambiguous = true;
            }
            if cand.1.is_some_and(|k| k != SkNodeKind::Section) {
                non_section_matches += 1;
            }
            if best.as_ref().is_none_or(|b| cand.0 > b.0) {
                best = Some(cand);
            }
        }
    }
    if best.is_none() {
        best = parse_locator_clause(cleaned);
        if let Some(ref c) = best {
            any_ambiguous |= c.3;
        }
    }
    let ambiguous = any_ambiguous || non_section_matches > 1;
    match best {
        Some((_, kind, num, _)) => ParsedLocator { kind, number: Some(num), raw_locator, ambiguous },
        None => ParsedLocator { kind: None, number: None, raw_locator, ambiguous },
    }
}

fn parse_locator_clause(clause: &str) -> Option<(u8, Option<SkNodeKind>, String, bool)> {
    let trimmed = clause.trim().trim_matches(|c| matches!(c, '[' | ']' | ' ' | '\t'));
    if trimmed.is_empty() {
        return None;
    }
    let tokens = tokenize_locator(trimmed);

    // Detect leading unnumbered statement keyword followed by a preposition + secondary statement,
    // e.g. "Corollary to Theorem 2.1", "Remark after Theorem 2.1", "Theorem in Section 2".
    let mut saw_unnumbered_kind = false;
    let mut best: Option<(u8, Option<SkNodeKind>, String, bool)> = None;
    let mut non_section_hits = 0usize;

    for (idx, tok) in tokens.iter().enumerate() {
        if let Some((kind, prio)) = match_kind_token(tok) {
            if let Some(num) = find_following_number(&tokens, idx + 1, kind) {
                if kind != SkNodeKind::Section {
                    non_section_hits += 1;
                }
                let amb = saw_unnumbered_kind || non_section_hits > 1;
                if best.as_ref().is_none_or(|b| prio > b.0) {
                    best = Some((prio, Some(kind), num, amb));
                } else if let Some(ref mut b) = best {
                    b.3 |= amb;
                }
            } else {
                saw_unnumbered_kind = true;
            }
        }
    }
    if best.is_some() {
        return best;
    }
    for tok in &tokens {
        let stripped = tok.trim_end_matches(['.', ',', ';', ':']);
        if let Some(inner) = stripped.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            if let Some(num) = normalize_number_token(inner, true) {
                return Some((20, Some(SkNodeKind::Equation), num, saw_unnumbered_kind));
            }
        }
    }
    for tok in &tokens {
        if let Some(num) = normalize_number_token(tok, false) {
            return Some((10, None, num, saw_unnumbered_kind));
        }
    }
    None
}

fn tokenize_locator(s: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    for ch in s.chars() {
        if ch == '§' {
            if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
            out.push("§".to_string());
        } else if ch.is_whitespace() || matches!(ch, '~' | '\u{00a0}') {
            if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

fn match_kind_token(tok: &str) -> Option<(SkNodeKind, u8)> {
    if tok == "§" { return Some((SkNodeKind::Section, 40)); }
    let clean = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '§').to_ascii_lowercase();
    match clean.as_str() {
        "theorem" | "theorems" | "thm" | "thms" => Some((SkNodeKind::Theorem, 80)),
        "lemma" | "lemmas" | "lem" | "lems" => Some((SkNodeKind::Lemma, 80)),
        "proposition" | "propositions" | "prop" | "props" => Some((SkNodeKind::Proposition, 80)),
        "corollary" | "corollaries" | "cor" | "cors" => Some((SkNodeKind::Corollary, 80)),
        "definition" | "definitions" | "def" | "defs" | "defn" | "defns" => Some((SkNodeKind::Definition, 80)),
        "assumption" | "assumptions" | "assump" | "hypothesis" | "hypotheses" | "hyp" | "axiom" | "axioms" => {
            Some((SkNodeKind::Assumption, 80))
        }
        "conjecture" | "conjectures" | "conj" => Some((SkNodeKind::Conjecture, 80)),
        "claim" | "claims" => Some((SkNodeKind::Claim, 80)),
        "remark" | "remarks" | "rem" | "rmk" => Some((SkNodeKind::Remark, 75)),
        "example" | "examples" | "ex" | "exm" => Some((SkNodeKind::Example, 75)),
        "algorithm" | "algorithms" | "alg" => Some((SkNodeKind::Algorithm, 75)),
        "equation" | "equations" | "eq" | "eqs" | "eqn" | "eqns" => Some((SkNodeKind::Equation, 70)),
        "figure" | "figures" | "fig" | "figs" => Some((SkNodeKind::Figure, 60)),
        "table" | "tables" | "tab" | "tabs" => Some((SkNodeKind::Table, 60)),
        "section" | "sections" | "sec" | "secs" | "sect" | "subsection" | "subsec"
        | "chapter" | "chap" | "ch" | "appendix" | "app" => Some((SkNodeKind::Section, 40)),
        _ => None,
    }
}

fn find_following_number(tokens: &[String], start: usize, kind: SkNodeKind) -> Option<String> {
    let mut i = start;
    while i < tokens.len() && i < start + 2 {
        let lower = tokens[i].trim_matches('.').to_ascii_lowercase();
        if matches!(lower.as_str(), "no" | "num" | "number" | "#") { i += 1; } else { break; }
    }
    normalize_number_token(tokens.get(i)?, kind == SkNodeKind::Equation)
}

fn normalize_number_token(raw: &str, allow_parens: bool) -> Option<String> {
    let mut s = raw.trim().trim_end_matches(['.', ',', ';', ':', ']']);
    if allow_parens {
        s = s.trim_start_matches('(').trim_end_matches(')');
    } else if s.starts_with('(') {
        return None;
    } else if let Some(idx) = s.find('(') {
        if s.ends_with(')') && idx > 0 { s = &s[..idx]; } else { return None; }
    }
    if s.is_empty() || s.len() > 24 || s.starts_with('.') || s.ends_with('.') {
        return None;
    }
    if !s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) {
        return None;
    }
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    let is_single_upper = s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase();
    let is_roman = matches!(s, "I" | "II" | "III" | "IV" | "V" | "VI" | "VII" | "VIII" | "IX" | "X");
    (has_digit || is_single_upper || is_roman).then(|| s.to_string())
}

/// Scans TeX files and optional [`TheoremTable`] for counter-drift hazards that make simulated
/// `NumberBasis::Computed` theorem numbers unconfirmed under Rule 6.9.
pub fn detect_tex_counter_drift(
    files: &BTreeMap<String, String>,
    theorem_table: Option<&TheoremTable>,
    nodes: &[SkNode],
) -> Option<String> {
    for content in files.values() {
        if let Some(reason) = scan_tex_counter_directives(content) {
            return Some(reason);
        }
    }
    let table = theorem_table?;
    let mut roots = BTreeSet::new();
    for def in table.defs.values().filter(|d| !d.starred && d.kind.is_theorem_like()) {
        roots.insert(resolve_root_counter(&def.env_name, table));
    }
    if roots.len() > 1 {
        return Some("multiple unshared theorem counters declared in preamble".into());
    }
    for n in nodes.iter().filter(|n| {
        n.kind.is_theorem_like() && n.number.is_some() && n.number_basis == Some(NumberBasis::Computed)
    }) {
        match n.env_name.as_deref() {
            Some(env) if !table.defs.contains_key(env) => {
                return Some(format!(
                    "theorem environment `{env}` is undeclared in TeX (counter sharing may live in an external class)"
                ));
            }
            None => {
                return Some("theorem-like node lacks environment name to confirm counter sharing".into());
            }
            _ => {}
        }
    }
    None
}

fn resolve_root_counter(env_name: &str, table: &TheoremTable) -> String {
    let mut cur = env_name.to_string();
    let mut visited = BTreeSet::from([cur.clone()]);
    while let Some(shared) = table.defs.get(&cur).and_then(|d| d.shared_counter.as_deref()) {
        if !visited.insert(shared.to_string()) { break; }
        cur = shared.to_string();
    }
    cur
}

/// Token-level scanner for counter-mutation directives across comments, verbatim, and `\iffalse...\fi`.
fn scan_tex_counter_directives(content: &str) -> Option<String> {
    let (mut in_verb, mut iffalse_depth): (Option<String>, usize) = (None, 0);
    for raw_line in content.split('\n') {
        let mut line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(ref env) = in_verb {
            let end_pat = format!("\\end{{{env}}}");
            if let Some(idx) = line.find(&end_pat) {
                line = &line[idx + end_pat.len()..];
                in_verb = None;
            } else {
                continue;
            }
        }
        let active = strip_latex_comments_line(line);
        let bytes = active.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] != b'\\' {
                i += 1;
                continue;
            }
            let Some((cmd, end)) = read_cmd_token(&active, i) else {
                i += 2;
                continue;
            };
            if iffalse_depth > 0 {
                if cmd == "fi" {
                    iffalse_depth -= 1;
                } else if cmd == "else" && iffalse_depth == 1 {
                    iffalse_depth = 0;
                } else if is_nested_tex_if(cmd) {
                    iffalse_depth += 1;
                }
                i = end;
                continue;
            }
            match cmd {
                "iffalse" => {
                    iffalse_depth = 1;
                    i = end;
                }
                "begin" => {
                    if let Some((env, next)) = read_simple_braced(&active, end) {
                        if VERBATIM_ENVS.contains(&env.trim()) {
                            in_verb = Some(env.trim().to_string());
                            break;
                        }
                        i = next;
                    } else {
                        i = end;
                    }
                }
                "setcounter" | "addtocounter" | "stepcounter" | "refstepcounter" | "counterwithout" => {
                    let after_star = if bytes.get(end) == Some(&b'*') { end + 1 } else { end };
                    if let Some((cname, next)) = read_simple_braced(&active, after_star) {
                        let name = cname.trim();
                        if !name.is_empty()
                            && !matches!(
                                name,
                                "tocdepth" | "secnumdepth" | "page" | "footnote" | "enumi" | "enumii"
                            )
                        {
                            return Some(format!("TeX modifies counter `{name}` via \\{cmd}"));
                        }
                        i = next;
                    } else {
                        i = end;
                    }
                }
                "renewcommand" | "def" | "edef" | "gdef" | "xdef" => {
                    let after_star = if bytes.get(end) == Some(&b'*') { end + 1 } else { end };
                    let rest = active[after_star..].trim_start();
                    let target = rest.strip_prefix('{').unwrap_or(rest).trim_start();
                    if target.starts_with("\\the")
                        && target.as_bytes().get(4).is_some_and(u8::is_ascii_alphabetic)
                    {
                        return Some(format!("TeX redefines counter formatting via \\{cmd}\\the..."));
                    }
                    i = end;
                }
                _ => i = end,
            }
        }
    }
    None
}

fn read_cmd_token(s: &str, bs: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    if !b.get(bs + 1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut end = bs + 1;
    while b.get(end).is_some_and(u8::is_ascii_alphabetic) {
        end += 1;
    }
    Some((&s[bs + 1..end], end))
}

fn read_simple_braced(s: &str, pos: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    let mut p = pos;
    while b.get(p).is_some_and(u8::is_ascii_whitespace) {
        p += 1;
    }
    if b.get(p) != Some(&b'{') {
        return None;
    }
    let start = p + 1;
    let close = s[start..].find('}')? + start;
    Some((&s[start..close], close + 1))
}

/// Formats a human-readable candidate label such as `"Theorem 2.1: Spectral Bound"` or `"Eq. (3.4)"`.
pub fn format_candidate_label(node: &SkNode) -> String {
    let num = node.number.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let title = node.title.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let kind_name = match node.kind {
        SkNodeKind::FrontMatter => "Front matter",
        SkNodeKind::Section => "Section",
        SkNodeKind::Definition => "Definition",
        SkNodeKind::Assumption => "Assumption",
        SkNodeKind::Theorem => "Theorem",
        SkNodeKind::Lemma => "Lemma",
        SkNodeKind::Proposition => "Proposition",
        SkNodeKind::Corollary => "Corollary",
        SkNodeKind::Conjecture => "Conjecture",
        SkNodeKind::Claim => "Claim",
        SkNodeKind::Proof => "Proof",
        SkNodeKind::Example => "Example",
        SkNodeKind::Remark => "Remark",
        SkNodeKind::Equation => "Equation",
        SkNodeKind::Figure => "Figure",
        SkNodeKind::Table => "Table",
        SkNodeKind::Algorithm => "Algorithm",
        SkNodeKind::Bibliography => "Bibliography",
    };
    let head = match (node.kind, num) {
        (SkNodeKind::Equation, Some(n)) => format!("Eq. ({n})"),
        (SkNodeKind::Equation, None) => "Equation".to_string(),
        (_, Some(n)) => format!("{kind_name} {n}"),
        (_, None) => kind_name.to_string(),
    };
    match title {
        Some(t) if node.kind != SkNodeKind::Proof => format!("{head}: {t}"),
        _ => head,
    }
}

pub(super) fn build_ranked_candidates(
    skeleton: &PaperSkeleton,
    primary: &[&SkNode],
    preferred_kind: Option<SkNodeKind>,
    ctx: &LocateContext<'_>,
) -> Vec<LocateCandidate> {
    let mut seen_orders = BTreeSet::new();
    let mut out = Vec::new();
    let mut push = |n: &SkNode| {
        if out.len() < MAX_LOCATE_CANDIDATES && seen_orders.insert(n.order) {
            out.push(node_to_candidate(skeleton, n, ctx));
        }
    };
    for n in primary { push(n); }
    if let Some(k) = preferred_kind {
        for n in &skeleton.nodes { if n.kind == k { push(n); } }
    }
    for n in &skeleton.nodes { if n.kind.is_theorem_like() { push(n); } }
    out
}

pub(super) fn collect_manual_candidates(
    skeleton: &PaperSkeleton,
    preferred_kind: Option<SkNodeKind>,
    ctx: &LocateContext<'_>,
) -> Vec<LocateCandidate> {
    let mut out = build_ranked_candidates(skeleton, &[], preferred_kind, ctx);
    if out.is_empty() {
        for n in &skeleton.nodes {
            if out.len() >= MAX_LOCATE_CANDIDATES { break; }
            if matches!(n.kind, SkNodeKind::Section | SkNodeKind::Equation) {
                out.push(node_to_candidate(skeleton, n, ctx));
            }
        }
    }
    out
}

fn node_to_candidate(
    skeleton: &PaperSkeleton,
    node: &SkNode,
    ctx: &LocateContext<'_>,
) -> LocateCandidate {
    let tag = node.tag.clone().unwrap_or_else(|| format!("ord-{}", node.order));
    let (start_page, end_page, page_basis, ocr, render_math) =
        if skeleton.builder == SkeletonBuilder::Pdf1 {
            if let Some(pm) = ctx.page_map {
                let range = page_range_for_node(pm, node);
                let basis = node_page_basis(pm, node);
                (
                    range.map(|r| r.0),
                    range.map(|r| r.1),
                    basis,
                    basis.is_some_and(PageBasis::is_scan),
                    basis.is_some_and(render_formulas_as_math),
                )
            } else if let Some(ranges) = ctx.page_lines {
                let range = page_range_from_lines(ranges, node);
                (range.map(|r| r.0), range.map(|r| r.1), None, false, false)
            } else {
                (None, None, None, false, false)
            }
        } else {
            (None, None, None, false, true)
        };
    LocateCandidate {
        tag,
        kind: node.kind,
        number: node.number.clone(),
        number_basis: node.number_basis,
        title: node.title.clone(),
        label: format_candidate_label(node),
        spans: node.spans.clone(),
        start_page,
        end_page,
        page_basis,
        ocr,
        render_math,
    }
}

fn page_range_from_lines(ranges: &[(usize, usize, usize)], node: &SkNode) -> Option<(usize, usize)> {
    let (mut lo, mut hi): (Option<usize>, Option<usize>) = (None, None);
    for span in &node.spans {
        for line in [span.start_line, span.end_line] {
            if let Some(p) = page_of_line(ranges, line) {
                lo = Some(lo.map_or(p, |c| c.min(p)));
                hi = Some(hi.map_or(p, |c| c.max(p)));
            }
        }
    }
    Some((lo?, hi?))
}
