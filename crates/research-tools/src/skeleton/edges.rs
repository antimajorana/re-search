//! Positional dependency, proof-link, and citation edge construction (Investigate V2).
//!
//! Adapted from the positional dependency-edge rules of **KnowTeX (Apache-2.0)**,
//! turning `\ref`, `\eqref`, `\cref`-family commands, and `"Proof of Theorem ..."`
//! headers into statement-role vs. proof-role dependency edges and explicit proof links
//! (per `investigate.md` section 11).

use super::bib::CiteOccurrence;
use super::flatten::{is_nested_tex_if, FlattenedTex, VERBATIM_ENVS};
use super::model::{
    EdgeConfidence, NumberBasis, SkEdge, SkEdgeKind, SkEdgeRole, SkEdgeRule, SkNode, SkNodeKind,
    SkeletonQuality, Span, MAX_SKELETON_EDGES,
};
use super::tex_lex::{skip_inline_verb, ProofHint};
use crate::tex_scan::strip_latex_comments_line;
use regex::Regex;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// Builds all V2 skeleton edges (`Proves`, `Uses`, `Cites`), populates `SkNode::proof_of`
/// and `SkNode::intro_refs`, deduplicates in reading order, and enforces [`MAX_SKELETON_EDGES`].
pub fn build_edges(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    proof_hints: &BTreeMap<usize, ProofHint>,
    flattened: &FlattenedTex,
    citations: &[CiteOccurrence],
    quality: &mut SkeletonQuality,
    quality_notes: &mut Vec<String>,
) -> Vec<SkEdge> {
    let (mut label_to_idx, mut kind_num_map, mut tag_to_idx) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for (idx, n) in nodes.iter().enumerate() {
        if let Some(label) = n.label.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            label_to_idx.entry(label.to_string()).and_modify(|target| *target = None).or_insert(Some(idx));
        }
        if let Some(num) = n.number.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            kind_num_map.entry((n.kind, num.to_string())).and_modify(|s| *s = None).or_insert(Some(idx));
        }
        if let Some(t) = n.tag.as_deref() { tag_to_idx.entry(t.to_string()).or_insert(idx); }
    }
    let mut edges = Vec::new();
    link_proofs(nodes, proof_hints, &label_to_idx, &kind_num_map, &mut edges);
    let (span_index, intro_set, active_lines) = (SpanIndex::build(nodes, parent_orders), build_intro_set(nodes, parent_orders), mask_active_lines(flattened));
    let mut ctx = EdgeCtx {
        parent_orders, tag_to_idx: &tag_to_idx, span_index: &span_index, intro_set: &intro_set,
        flattened, explicit_uses: BTreeSet::new(), edges: &mut edges,
    };
    ctx.scan_ref_uses(nodes, &label_to_idx, &active_lines);
    ctx.scan_text_mentions(nodes, &kind_num_map, &active_lines);
    ctx.emit_cite_edges(nodes, citations);
    let mut seen = BTreeSet::new();
    edges.retain(|e| seen.insert((e.from.clone(), e.to.clone(), e.kind, e.role, e.rule)));
    if edges.len() > MAX_SKELETON_EDGES {
        let total = edges.len();
        edges.truncate(MAX_SKELETON_EDGES);
        if *quality == SkeletonQuality::Full { *quality = SkeletonQuality::Partial; }
        quality_notes.push(format!("edge count ({total}) exceeded MAX_SKELETON_EDGES ({MAX_SKELETON_EDGES}); truncated"));
    }
    edges
}

fn first_span(n: &SkNode) -> Span { n.spans.first().cloned().unwrap_or_else(|| Span::new("", 1, 1)) }

fn link_proofs(
    nodes: &mut [SkNode],
    proof_hints: &BTreeMap<usize, ProofHint>,
    label_to_idx: &BTreeMap<String, Option<usize>>,
    kind_num_map: &BTreeMap<(SkNodeKind, String), Option<usize>>,
    edges: &mut Vec<SkEdge>,
) {
    for p_idx in 0..nodes.len() {
        if nodes[p_idx].kind != SkNodeKind::Proof { continue; }
        nodes[p_idx].proof_of = None;
        let Some(p_tag) = nodes[p_idx].tag.clone() else { continue };
        let resolved = match proof_hints.get(&p_idx) {
            Some(ProofHint::HeaderRef { label, .. }) => label_to_idx
                .get(label.trim()).copied().flatten().filter(|&t| t != p_idx)
                .map(|t| (t, SkEdgeRule::ProofHeader, EdgeConfidence::Explicit)),
            Some(ProofHint::HeaderMention { kind_word, target_text, .. }) => {
                map_kind_word(kind_word, false).and_then(|kind| {
                    let t = kind_num_map.get(&(kind, target_text.trim().to_string())).copied().flatten()?;
                    let conf = match nodes[t].number_basis {
                        Some(NumberBasis::Printed | NumberBasis::Confirmed) => EdgeConfidence::Explicit,
                        _ => EdgeConfidence::Heuristic,
                    };
                    (t != p_idx).then_some((t, SkEdgeRule::ProofHeader, conf))
                })
            }
            Some(ProofHint::Adjacent { target_order }) => (*target_order < nodes.len() && *target_order != p_idx)
                .then_some((*target_order, SkEdgeRule::ProofAdjacent, EdgeConfidence::Explicit)),
            Some(ProofHint::Unattached) | None => None,
        };
        if let Some((t_idx, rule, confidence)) = resolved {
            if let Some(t_tag) = nodes[t_idx].tag.clone() {
                nodes[p_idx].proof_of = Some(t_tag.clone());
                edges.push(SkEdge {
                    from: p_tag, to: t_tag, kind: SkEdgeKind::Proves, role: SkEdgeRole::Proof,
                    site: first_span(&nodes[p_idx]), rule, confidence,
                });
            }
        }
    }
}

fn map_kind_word(raw: &str, allow_non_statement: bool) -> Option<SkNodeKind> {
    let norm = raw.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    match norm.trim_end_matches('.') {
        "theorem" | "thm" | "main theorem" => Some(SkNodeKind::Theorem),
        "lemma" | "lem" => Some(SkNodeKind::Lemma),
        "proposition" | "prop" => Some(SkNodeKind::Proposition),
        "corollary" | "cor" => Some(SkNodeKind::Corollary),
        "conjecture" | "conj" => Some(SkNodeKind::Conjecture),
        "claim" => Some(SkNodeKind::Claim),
        "definition" | "defn" | "def" => Some(SkNodeKind::Definition),
        "assumption" | "assump" => Some(SkNodeKind::Assumption),
        "remark" | "rem" if allow_non_statement => Some(SkNodeKind::Remark),
        "example" | "ex" if allow_non_statement => Some(SkNodeKind::Example),
        "section" | "sec" if allow_non_statement => Some(SkNodeKind::Section),
        "equation" | "eqn" | "eq" if allow_non_statement => Some(SkNodeKind::Equation),
        "algorithm" | "alg" if allow_non_statement => Some(SkNodeKind::Algorithm),
        "figure" | "fig" if allow_non_statement => Some(SkNodeKind::Figure),
        "table" | "tab" if allow_non_statement => Some(SkNodeKind::Table),
        _ => None,
    }
}

fn build_intro_set(nodes: &[SkNode], parent_orders: &[Option<usize>]) -> BTreeSet<usize> {
    let seeds: BTreeSet<usize> = nodes.iter().enumerate().filter(|(_, n)| {
        n.kind == SkNodeKind::FrontMatter || (n.kind == SkNodeKind::Section && n.title.as_deref()
            .is_some_and(|t| {
                let l = t.to_ascii_lowercase();
                l.contains("introduction") || l.contains("intro") || l.contains("overview")
            }))
    }).map(|(i, _)| i).collect();
    let mut intro = seeds.clone();
    for idx in 0..nodes.len() {
        let mut cur = idx;
        for _ in 0..nodes.len() {
            let Some(p) = parent_orders.get(cur).copied().flatten().filter(|&p| p < nodes.len()) else { break };
            if seeds.contains(&p) { intro.insert(idx); break; }
            cur = p;
        }
    }
    intro
}

struct SpanIndex {
    by_path: BTreeMap<String, Vec<(usize, usize, usize)>>,
    depths: Vec<usize>,
    orders: Vec<usize>,
}

impl SpanIndex {
    fn build(nodes: &[SkNode], parent_orders: &[Option<usize>]) -> Self {
        let n = nodes.len();
        let depths = (0..n).map(|mut cur| {
            let mut d = 0usize;
            while d < n {
                let Some(p) = parent_orders.get(cur).copied().flatten().filter(|&p| p < n) else { break };
                d += 1; cur = p;
            }
            d
        }).collect();
        let mut by_path: BTreeMap<String, Vec<(usize, usize, usize)>> = BTreeMap::new();
        for (idx, node) in nodes.iter().enumerate() {
            for s in &node.spans { by_path.entry(s.path.clone()).or_default().push((s.start_line, s.end_line, idx)); }
        }
        by_path.values_mut().for_each(|v| v.sort_by_key(|&(start, _, _)| start));
        Self { by_path, depths, orders: nodes.iter().map(|node| node.order).collect() }
    }

    fn enclosing(&self, flattened: &FlattenedTex, flat_line: usize) -> Option<usize> {
        let origin = flattened.origin_of(flat_line)?;
        let spans = self.by_path.get(&origin.path)?;
        let upper = spans.partition_point(|&(start, _, _)| start <= origin.line);
        spans[..upper].iter().filter(|&&(_, end, _)| origin.line <= end)
            .max_by_key(|&&(s, e, i)| (self.depths[i], Reverse(e.saturating_sub(s) + 1), self.orders[i]))
            .map(|&(_, _, idx)| idx)
    }
}

fn mask_active_lines(flattened: &FlattenedTex) -> Vec<String> {
    let mut out = Vec::with_capacity(flattened.lines.len());
    let (mut in_verb, mut iffalse) = (None::<String>, 0usize);
    for fl in &flattened.lines {
        let raw = fl.text.as_str();
        let seg = if let Some(env) = in_verb.as_deref() {
            let pat = format!("\\end{{{env}}}");
            let Some(rel) = raw.find(&pat) else { out.push(String::new()); continue; };
            in_verb = None;
            &raw[rel + pat.len()..]
        } else { raw };
        out.push(mask_segment(&strip_latex_comments_line(seg), &mut in_verb, &mut iffalse));
    }
    out
}

fn mask_segment(code: &str, in_verb: &mut Option<String>, iffalse: &mut usize) -> String {
    let mut masked = code.as_bytes().to_vec();
    let (len, mut i) = (masked.len(), 0usize);
    while i < len {
        if masked[i] != b'\\' {
            let step = code[i..].chars().next().map_or(1, char::len_utf8);
            if *iffalse > 0 { masked[i..i + step].fill(b' '); }
            i += step; continue;
        }
        if i + 1 >= len || !masked[i + 1].is_ascii_alphabetic() {
            let end = (i + 1 + code[i + 1..].chars().next().map_or(0, char::len_utf8)).min(len);
            if *iffalse > 0 { masked[i..end].fill(b' '); }
            i = end; continue;
        }
        let mut k = i + 1;
        while k < len && masked[k].is_ascii_alphabetic() { k += 1; }
        let cmd = &code[i + 1..k];
        if *iffalse > 0 {
            masked[i..k].fill(b' ');
            if cmd == "fi" { *iffalse -= 1; }
            else if cmd == "else" && *iffalse == 1 { *iffalse = 0; }
            else if is_nested_tex_if(cmd) { *iffalse += 1; }
        } else if cmd == "verb" {
            let end = skip_inline_verb(code, k).min(len);
            if end > k { masked[i..end].fill(b' '); i = end; continue; }
        } else if cmd == "iffalse" {
            *iffalse = 1; masked[i..k].fill(b' ');
        } else if cmd == "begin" && code[k..].starts_with('{') {
            if let Some(rel) = code[k + 1..].find('}') {
                let env = code[k + 1..k + 1 + rel].trim();
                if VERBATIM_ENVS.contains(&env) {
                    let (after, pat) = (k + 1 + rel + 1, format!("\\end{{{env}}}"));
                    if let Some(re) = code[after..].find(&pat) {
                        let end = after + re + pat.len();
                        masked[i..end].fill(b' '); i = end; continue;
                    }
                    masked[i..len].fill(b' '); *in_verb = Some(env.to_string()); break;
                }
            }
        }
        i = k;
    }
    String::from_utf8(masked).unwrap_or_else(|_| code.to_string())
}

fn preceding_backslashes_even(bytes: &[u8], mut pos: usize) -> bool {
    let mut count = 0usize;
    while pos > 0 && bytes[pos - 1] == b'\\' { count += 1; pos -= 1; }
    count.is_multiple_of(2)
}

struct EdgeCtx<'a> {
    parent_orders: &'a [Option<usize>],
    tag_to_idx: &'a BTreeMap<String, usize>,
    span_index: &'a SpanIndex,
    intro_set: &'a BTreeSet<usize>,
    flattened: &'a FlattenedTex,
    explicit_uses: BTreeSet<(String, String, SkEdgeRole)>,
    edges: &'a mut Vec<SkEdge>,
}

impl<'a> EdgeCtx<'a> {
    fn proof_owner(&self, nodes: &[SkNode], idx: usize) -> usize {
        if nodes[idx].kind == SkNodeKind::Proof {
            if let Some(&t) = nodes[idx].proof_of.as_deref().and_then(|k| self.tag_to_idx.get(k)) { return t; }
        }
        idx
    }

    fn nearest_non_eq(&self, nodes: &[SkNode], mut cur: usize) -> Option<usize> {
        for _ in 0..nodes.len() {
            let p = self.parent_orders.get(cur).copied().flatten().filter(|&p| p < nodes.len())?;
            if nodes[p].kind != SkNodeKind::Equation { return Some(p); }
            cur = p;
        }
        None
    }

    fn resolve_user(&self, nodes: &[SkNode], enc: usize) -> (usize, Option<usize>, SkEdgeRole) {
        match nodes[enc].kind {
            SkNodeKind::Proof => (self.proof_owner(nodes, enc), None, SkEdgeRole::Proof),
            k if k.is_theorem_like() => (enc, None, SkEdgeRole::Statement),
            SkNodeKind::Equation => match self.nearest_non_eq(nodes, enc) {
                Some(a) if nodes[a].kind == SkNodeKind::Proof => {
                    let o = self.proof_owner(nodes, a);
                    (enc, (o != enc).then_some(o), SkEdgeRole::Proof)
                }
                Some(a) if nodes[a].kind.is_theorem_like() => (enc, Some(a), SkEdgeRole::Statement),
                Some(a) => (enc, Some(a), SkEdgeRole::Prose),
                None => (enc, None, SkEdgeRole::Prose),
            },
            _ => (enc, None, SkEdgeRole::Prose),
        }
    }

    fn push_use(&mut self, from: String, to: String, role: SkEdgeRole, site: &Span, rule: SkEdgeRule, conf: EdgeConfidence) {
        if from == to { return; }
        let key = (from.clone(), to.clone(), role);
        if conf == EdgeConfidence::Heuristic && self.explicit_uses.contains(&key) { return; }
        if conf == EdgeConfidence::Explicit { self.explicit_uses.insert(key); }
        self.edges.push(SkEdge { from, to, kind: SkEdgeKind::Uses, role, site: site.clone(), rule, confidence: conf });
    }

    fn emit_uses(&mut self, nodes: &mut [SkNode], target: usize, enc: usize, site: &Span, rule: SkEdgeRule, conf: EdgeConfidence) {
        let (direct_idx, lifted_idx, role) = self.resolve_user(nodes, enc);
        let (Some(target_tag), Some(direct_tag)) = (nodes[target].tag.clone(), nodes[direct_idx].tag.clone()) else { return };
        if self.intro_set.contains(&enc) && !self.intro_set.contains(&target) { nodes[target].intro_refs = true; }
        self.push_use(target_tag.clone(), direct_tag, role, site, rule, conf);
        if let Some(l_tag) = lifted_idx.and_then(|i| nodes[i].tag.clone()) {
            self.push_use(target_tag, l_tag, role, site, SkEdgeRule::Lifted, conf);
        }
        if nodes[target].kind == SkNodeKind::Equation {
            if let Some(anc) = self.nearest_non_eq(nodes, target) {
                let (owner_idx, eff_user) = (self.proof_owner(nodes, anc), lifted_idx.unwrap_or(direct_idx));
                if let (Some(o_tag), Some(u_tag)) = (nodes[owner_idx].tag.clone(), nodes[eff_user].tag.clone()) {
                    self.push_use(o_tag, u_tag, role, site, SkEdgeRule::Lifted, conf);
                }
            }
        }
    }

    fn scan_ref_uses(&mut self, nodes: &mut [SkNode], label_to_idx: &BTreeMap<String, Option<usize>>, active: &[String]) {
        static REF_RE: OnceLock<Regex> = OnceLock::new();
        let re = REF_RE.get_or_init(|| Regex::new(r"\\(eqref|cref|Cref|autoref|pageref|nameref|vref|fullref|ref)\*?\s*\{([^{}]*)\}").unwrap());
        for (idx0, line) in active.iter().enumerate() {
            if !line.contains('\\') { continue; }
            let Some(origin) = self.flattened.origin_of(idx0 + 1) else { continue };
            let (bytes, mut enc_cache) = (line.as_bytes(), None::<Option<usize>>);
            for caps in re.captures_iter(line) {
                if !preceding_backslashes_even(bytes, caps.get(0).unwrap().start()) { continue; }
                let rule = match &caps[1] {
                    "eqref" => SkEdgeRule::Eqref,
                    "cref" | "Cref" | "autoref" | "nameref" | "vref" | "fullref" => SkEdgeRule::Cref,
                    _ => SkEdgeRule::Ref,
                };
                let Some(enc) = *enc_cache.get_or_insert_with(|| self.span_index.enclosing(self.flattened, idx0 + 1)) else { continue };
                let site = Span::new(origin.path.clone(), origin.line, origin.line);
                for part in caps[2].split(',') {
                    if let Some(&Some(target)) = label_to_idx.get(part.trim()) {
                        self.emit_uses(nodes, target, enc, &site, rule, EdgeConfidence::Explicit);
                    }
                }
            }
        }
    }

    fn scan_text_mentions(&mut self, nodes: &mut [SkNode], kind_num_map: &BTreeMap<(SkNodeKind, String), Option<usize>>, active: &[String]) {
        static MENTION_RE: OnceLock<Regex> = OnceLock::new();
        let re = MENTION_RE.get_or_init(|| Regex::new(
            r"\b(?i:(theorem|thm\.?|lemma|lem\.?|proposition|prop\.?|corollary|cor\.?|definition|defn\.?|def\.?|assumption|assump\.?|conjecture|conj\.?|claim|remark|rem\.?|example|ex\.?|section|sec\.?|equation|eqn\.?|eq\.?|algorithm|alg\.?|figure|fig\.?|table|tab\.?))(?:[ \t~]+)(?:\(([A-Za-z0-9]+(?:[.\-][A-Za-z0-9]+)*)\)|([0-9]+(?:[.\-][A-Za-z0-9]+)*[A-Za-z]?|[A-Z]\.[0-9]+(?:[.\-][A-Za-z0-9]+)*|[A-Z])\b)",
        ).unwrap());
        for (idx0, line) in active.iter().enumerate() {
            if line.is_empty() { continue; }
            let Some(origin) = self.flattened.origin_of(idx0 + 1) else { continue };
            let (bytes, mut enc_cache) = (line.as_bytes(), None::<Option<usize>>);
            for caps in re.captures_iter(line) {
                let start = caps.get(0).unwrap().start();
                if start > 0 && bytes[start - 1] == b'\\' { continue; }
                let Some(kind) = map_kind_word(&caps[1], true) else { continue };
                let num = caps.get(2).or_else(|| caps.get(3)).map_or("", |m| m.as_str().trim());
                if num.is_empty() || num.starts_with('\\') { continue; }
                let Some(Some(target)) = kind_num_map.get(&(kind, num.to_string())).copied() else { continue };
                let Some(enc) = *enc_cache.get_or_insert_with(|| self.span_index.enclosing(self.flattened, idx0 + 1)) else { continue };
                let site = Span::new(origin.path.clone(), origin.line, origin.line);
                self.emit_uses(nodes, target, enc, &site, SkEdgeRule::TextMention, EdgeConfidence::Heuristic);
            }
        }
    }

    fn emit_cite_edges(&mut self, nodes: &[SkNode], citations: &[CiteOccurrence]) {
        let mut head_sites: BTreeSet<(String, String, SkEdgeRole, String, usize)> = BTreeSet::new();
        for n in nodes.iter().filter(|n| !n.head_cites.is_empty()) {
            let (to_tag, role) = match n.kind {
                SkNodeKind::Proof => (n.proof_of.clone().or_else(|| n.tag.clone()), SkEdgeRole::Proof),
                k if k.is_theorem_like() => (n.tag.clone(), SkEdgeRole::Statement),
                _ => (n.tag.clone(), SkEdgeRole::Prose),
            };
            let Some(to_tag) = to_tag else { continue };
            let site = first_span(n);
            for key in n.head_cites.iter().filter(|k| !k.is_empty()) {
                head_sites.insert((key.clone(), to_tag.clone(), role, site.path.clone(), site.start_line));
                self.edges.push(SkEdge {
                    from: key.clone(), to: to_tag.clone(), kind: SkEdgeKind::Cites, role,
                    site: site.clone(), rule: SkEdgeRule::HeadCite, confidence: EdgeConfidence::Explicit,
                });
            }
        }
        for occ in citations.iter().filter(|c| !c.key.is_empty()) {
            let Some(enc) = self.span_index.enclosing(self.flattened, occ.flat_line) else { continue };
            let (direct_idx, lifted_idx, role) = self.resolve_user(nodes, enc);
            let user_idx = match (nodes[enc].kind, role) {
                (SkNodeKind::Equation, SkEdgeRole::Proof | SkEdgeRole::Statement) => lifted_idx.unwrap_or(direct_idx),
                (SkNodeKind::Equation, SkEdgeRole::Prose) => enc,
                _ => direct_idx,
            };
            let Some(user_tag) = nodes[user_idx].tag.clone() else { continue };
            if head_sites.contains(&(occ.key.clone(), user_tag.clone(), role, occ.site.path.clone(), occ.site.start_line)) { continue; }
            self.edges.push(SkEdge {
                from: occ.key.clone(), to: user_tag, kind: SkEdgeKind::Cites, role,
                site: occ.site.clone(), rule: SkEdgeRule::Cite, confidence: EdgeConfidence::Explicit,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::flatten::{FlattenedLine, OriginEntry};
    use crate::skeleton::model::SkConfidence;

    fn mk_flat(lines: &[&str]) -> FlattenedTex {
        FlattenedTex {
            primary_path: "main.tex".into(), inlined_files: vec!["main.tex".into()], warnings: Vec::new(),
            lines: lines.iter().enumerate().map(|(i, s)| FlattenedLine {
                text: (*s).into(), origin: OriginEntry { path: "main.tex".into(), line: i + 1 },
            }).collect(),
        }
    }

    fn mk_node(order: usize, tag: &str, kind: SkNodeKind, s: usize, e: usize, lbl: Option<&str>, num: Option<&str>) -> SkNode {
        SkNode {
            tag: Some(tag.into()), kind, number: num.map(str::to_string), number_basis: None, env_name: None,
            title: None, label: lbl.map(str::to_string), parent: None, order, spans: vec![Span::new("main.tex", s, e)],
            proof_of: None, head_cites: Vec::new(), intro_refs: false, fp: "000000000000".into(),
            words: 0, eq_count: 0, read_minutes: 0.0, confidence: SkConfidence::TexEnv,
        }
    }

    fn has(edges: &[SkEdge], from: &str, to: &str, k: SkEdgeKind, role: SkEdgeRole, rule: SkEdgeRule, c: EdgeConfidence) -> bool {
        edges.iter().any(|e| e.from == from && e.to == to && e.kind == k && e.role == role && e.rule == rule && e.confidence == c)
    }

    #[test]
    fn statement_vs_proof_uses_and_proof_linking_modes() {
        let flat = mk_flat(&[
            r"\begin{definition}\label{def:1} X \end{definition}",
            r"\begin{lemma}\label{lem:1} Y \end{lemma}",
            r"\begin{theorem}\label{thm:1} By \ref{def:1} \end{theorem}",
            r"\begin{proof} Using \cref{lem:1} \end{proof}",
            r"\begin{proof}[Proof of \ref{thm:2}] detached ref \end{proof}",
            r"\begin{proof}[Proof of Lemma 2.1] detached mention \end{proof}",
            r"\begin{theorem}\label{thm:2} Z \end{theorem}",
        ]);
        let mut nodes = vec![
            mk_node(0, "n-def1", SkNodeKind::Definition, 1, 1, Some("def:1"), None),
            mk_node(1, "n-lem1", SkNodeKind::Lemma, 2, 2, Some("lem:1"), Some("2.1")),
            mk_node(2, "n-thm1", SkNodeKind::Theorem, 3, 3, Some("thm:1"), None),
            mk_node(3, "n-pf1", SkNodeKind::Proof, 4, 4, None, None),
            mk_node(4, "n-pf2", SkNodeKind::Proof, 5, 5, None, None),
            mk_node(5, "n-pf3", SkNodeKind::Proof, 6, 6, None, None),
            mk_node(6, "n-thm2", SkNodeKind::Theorem, 7, 7, Some("thm:2"), None),
        ];
        nodes[1].number_basis = Some(NumberBasis::Computed);
        let hints = BTreeMap::from([
            (3, ProofHint::Adjacent { target_order: 2 }),
            (4, ProofHint::HeaderRef { label: "thm:2".into(), heading: "Proof of \\ref{thm:2}".into() }),
            (5, ProofHint::HeaderMention { kind_word: "Lemma".into(), target_text: "2.1".into(), heading: "Proof of Lemma 2.1".into() }),
        ]);
        let (mut q, mut notes) = (SkeletonQuality::Full, Vec::new());
        let edges = build_edges(&mut nodes, &[None; 7], &hints, &flat, &[], &mut q, &mut notes);
        assert_eq!((nodes[3].proof_of.as_deref(), nodes[4].proof_of.as_deref(), nodes[5].proof_of.as_deref()), (Some("n-thm1"), Some("n-thm2"), Some("n-lem1")));
        assert!(has(&edges, "n-pf1", "n-thm1", SkEdgeKind::Proves, SkEdgeRole::Proof, SkEdgeRule::ProofAdjacent, EdgeConfidence::Explicit));
        assert!(has(&edges, "n-pf2", "n-thm2", SkEdgeKind::Proves, SkEdgeRole::Proof, SkEdgeRule::ProofHeader, EdgeConfidence::Explicit));
        assert!(has(&edges, "n-pf3", "n-lem1", SkEdgeKind::Proves, SkEdgeRole::Proof, SkEdgeRule::ProofHeader, EdgeConfidence::Heuristic));
        assert!(has(&edges, "n-def1", "n-thm1", SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::Ref, EdgeConfidence::Explicit));
        assert!(has(&edges, "n-lem1", "n-thm1", SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::Cref, EdgeConfidence::Explicit));
    }

    #[test]
    fn lifted_equation_text_mention_and_intro_refs() {
        let flat = mk_flat(&[
            r"\section{Introduction}", r"See Theorem 1 and \ref{thm:2}.",
            r"\begin{theorem}\label{thm:1}", r"\begin{equation}\label{eq:1} x=y \end{equation}", r"\end{theorem}",
            r"\begin{theorem}\label{thm:2} By Theorem 1 \end{theorem}",
            r"\begin{proof} By \eqref{eq:1}, % Theorem 1",
            r"\verb|Theorem 1| \iffalse Theorem 1 \fi \ref{thm:1} and Theorem 1 \end{proof}",
        ]);
        let mut nodes = vec![
            mk_node(0, "n-intro", SkNodeKind::Section, 1, 2, None, None),
            mk_node(1, "n-t1", SkNodeKind::Theorem, 3, 5, Some("thm:1"), Some("1")),
            mk_node(2, "n-eq1", SkNodeKind::Equation, 4, 4, Some("eq:1"), Some("1")),
            mk_node(3, "n-t2", SkNodeKind::Theorem, 6, 6, Some("thm:2"), Some("2")),
            mk_node(4, "n-pf", SkNodeKind::Proof, 7, 8, None, None),
        ];
        nodes[0].title = Some("Introduction".into());
        let hints = BTreeMap::from([(4, ProofHint::Adjacent { target_order: 3 })]);
        let (mut q, mut notes) = (SkeletonQuality::Full, Vec::new());
        let edges = build_edges(&mut nodes, &[None, None, Some(1), None, None], &hints, &flat, &[], &mut q, &mut notes);
        assert!(nodes[1].intro_refs && nodes[3].intro_refs && !nodes[0].intro_refs);
        assert!(has(&edges, "n-eq1", "n-t2", SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::Eqref, EdgeConfidence::Explicit));
        assert!(has(&edges, "n-t1", "n-t2", SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::Lifted, EdgeConfidence::Explicit));
        assert!(has(&edges, "n-t1", "n-t2", SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::TextMention, EdgeConfidence::Heuristic));
        assert!(!has(&edges, "n-t1", "n-t2", SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::TextMention, EdgeConfidence::Heuristic));
    }

    #[test]
    fn head_cite_dedup_and_edge_cap_truncation() {
        let flat = mk_flat(&[r"\begin{theorem}[\cite{K20}]", r"See \cite{K20} and \cite{M21}.", r"\end{theorem}"]);
        let mut t1 = mk_node(0, "n-t1", SkNodeKind::Theorem, 1, 3, None, None);
        t1.head_cites = vec!["K20".into()];
        let mut nodes = vec![t1];
        let cites = vec![
            CiteOccurrence { flat_line: 1, site: Span::new("main.tex", 1, 1), key: "K20".into(), locator: None },
            CiteOccurrence { flat_line: 2, site: Span::new("main.tex", 2, 2), key: "M21".into(), locator: None },
        ];
        let (mut q, mut notes) = (SkeletonQuality::Full, Vec::new());
        let edges = build_edges(&mut nodes, &[None], &BTreeMap::new(), &flat, &cites, &mut q, &mut notes);
        assert_eq!(edges.len(), 2);
        assert!(has(&edges, "K20", "n-t1", SkEdgeKind::Cites, SkEdgeRole::Statement, SkEdgeRule::HeadCite, EdgeConfidence::Explicit));
        assert!(has(&edges, "M21", "n-t1", SkEdgeKind::Cites, SkEdgeRole::Statement, SkEdgeRule::Cite, EdgeConfidence::Explicit));
        let big: Vec<CiteOccurrence> = (0..=MAX_SKELETON_EDGES)
            .map(|i| CiteOccurrence { flat_line: 2, site: Span::new("main.tex", 2, 2), key: format!("k-{i}"), locator: None }).collect();
        let (mut q2, mut notes2) = (SkeletonQuality::Full, Vec::new());
        let capped = build_edges(&mut nodes, &[None], &BTreeMap::new(), &flat, &big, &mut q2, &mut notes2);
        assert_eq!(capped.len(), MAX_SKELETON_EDGES);
        assert_eq!(q2, SkeletonQuality::Partial);
        assert!(!notes2.is_empty());
    }
}
