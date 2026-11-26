//! Heuristic dependency edges, proof linking, citation edges, and reading stats for `pdf-1`
//! (Investigate V17).
//!
//! Every PDF edge uses `EdgeConfidence::Heuristic` and keeps `from` = prerequisite, `to` =
//! dependent, staying outside the confirmed skip plan. Ambiguous `(kind, printed_number)` targets
//! never emit edges or `proof_of` links.

use super::model::{
    EdgeConfidence, SectionStats, SkEdge, SkEdgeKind, SkEdgeRole, SkEdgeRule, SkNode, SkNodeKind,
    Span, MAX_SKELETON_EDGES,
};
use super::pdf_bib::PdfCiteOccurrence;
use super::pdf_patterns::map_pdf_kind_word;
use super::pdf_scan::PdfProofHint;
use super::stats::estimate_read_minutes;
use crate::pdf_firstpage::normalize_pdf_unicode;
use regex::Regex;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// Builds all heuristic PDF edges (`Proves`, `Uses`, `Cites`), populates `proof_of` and
/// `intro_refs`, deduplicates in reading order, and enforces [`MAX_SKELETON_EDGES`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pdf_edges(
    primary_path: &str,
    lines: &[&str],
    active_lines: &[bool],
    bib_ranges: &[(usize, usize)],
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    proof_hints: &BTreeMap<usize, PdfProofHint>,
    node_heading_end_bytes: &BTreeMap<usize, usize>,
    citations: &[PdfCiteOccurrence],
    quality_notes: &mut Vec<String>,
) -> Vec<SkEdge> {
    // Map `(kind, printed_number)` -> `Some(idx)` when unique, `None` when ambiguous.
    let mut kind_num_map: BTreeMap<(SkNodeKind, String), Option<usize>> = BTreeMap::new();
    let mut tag_to_idx: BTreeMap<String, usize> = BTreeMap::new();

    for (idx, n) in nodes.iter().enumerate() {
        if let Some(num) = n.number.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            kind_num_map
                .entry((n.kind, num.to_string()))
                .and_modify(|slot| *slot = None)
                .or_insert(Some(idx));
        }
        if let Some(tag) = n.tag.as_deref() {
            tag_to_idx.entry(tag.to_string()).or_insert(idx);
        }
    }

    let mut edges = Vec::new();
    link_pdf_proofs(nodes, proof_hints, &kind_num_map, &mut edges);

    let span_index = PdfSpanIndex::build(nodes, parent_orders);
    let intro_set = build_intro_set(nodes, parent_orders);

    // Map each node's start_line to the max heading_end_byte (in normalized trimmed coordinates)
    // so self-headings (`Theorem 2.1. By Lemma 1.2...`) are skipped while body mentions on the
    // same line are scanned.
    let mut line_skip_prefix: BTreeMap<usize, usize> = BTreeMap::new();
    for (&idx, &head_end) in node_heading_end_bytes {
        if let Some(first) = nodes.get(idx).and_then(|n| n.spans.first()) {
            line_skip_prefix
                .entry(first.start_line)
                .and_modify(|v| *v = (*v).max(head_end))
                .or_insert(head_end);
        }
    }

    let mut ctx = PdfEdgeCtx {
        parent_orders,
        tag_to_idx: &tag_to_idx,
        span_index: &span_index,
        intro_set: &intro_set,
        edges: &mut edges,
    };

    ctx.scan_text_mentions(
        primary_path,
        lines,
        active_lines,
        bib_ranges,
        &line_skip_prefix,
        &kind_num_map,
        nodes,
    );
    ctx.emit_cite_edges(nodes, citations);

    // Deduplicate in reading order.
    let mut seen = BTreeSet::new();
    edges.retain(|e| seen.insert((e.from.clone(), e.to.clone(), e.kind, e.role, e.rule)));

    if edges.len() > MAX_SKELETON_EDGES {
        let total = edges.len();
        edges.truncate(MAX_SKELETON_EDGES);
        quality_notes.push(format!(
            "edge count ({total}) exceeded MAX_SKELETON_EDGES ({MAX_SKELETON_EDGES}); truncated"
        ));
    }
    edges
}

fn first_span(n: &SkNode) -> Span {
    n.spans.first().cloned().unwrap_or_else(|| Span::new("", 1, 1))
}

fn link_pdf_proofs(
    nodes: &mut [SkNode],
    proof_hints: &BTreeMap<usize, PdfProofHint>,
    kind_num_map: &BTreeMap<(SkNodeKind, String), Option<usize>>,
    edges: &mut Vec<SkEdge>,
) {
    for p_idx in 0..nodes.len() {
        if nodes[p_idx].kind != SkNodeKind::Proof {
            continue;
        }
        nodes[p_idx].proof_of = None;
        let Some(p_tag) = nodes[p_idx].tag.clone() else {
            continue;
        };
        let resolved = match proof_hints.get(&p_idx) {
            Some(PdfProofHint::HeaderMention { kind, number }) => kind_num_map
                .get(&(*kind, number.trim().to_string()))
                .copied()
                .flatten()
                .filter(|&t| t != p_idx)
                .map(|t| (t, SkEdgeRule::ProofHeader)),
            Some(PdfProofHint::Adjacent { target_order }) => (*target_order < nodes.len()
                && *target_order != p_idx)
                .then_some((*target_order, SkEdgeRule::ProofAdjacent)),
            Some(PdfProofHint::Unattached) | None => None,
        };
        if let Some((t_idx, rule)) = resolved {
            if let Some(t_tag) = nodes[t_idx].tag.clone() {
                nodes[p_idx].proof_of = Some(t_tag.clone());
                edges.push(SkEdge {
                    from: p_tag,
                    to: t_tag,
                    kind: SkEdgeKind::Proves,
                    role: SkEdgeRole::Proof,
                    site: first_span(&nodes[p_idx]),
                    rule,
                    confidence: EdgeConfidence::Heuristic,
                });
            }
        }
    }
}

fn build_intro_set(nodes: &[SkNode], parent_orders: &[Option<usize>]) -> BTreeSet<usize> {
    let seeds: BTreeSet<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.kind == SkNodeKind::FrontMatter
                || (n.kind == SkNodeKind::Section
                    && n.title.as_deref().is_some_and(|t| {
                        let l = t.to_ascii_lowercase();
                        l.contains("introduction") || l.contains("intro") || l.contains("overview")
                    }))
        })
        .map(|(i, _)| i)
        .collect();

    let mut intro = seeds.clone();
    for idx in 0..nodes.len() {
        let mut cur = idx;
        for _ in 0..nodes.len() {
            let Some(p) = parent_orders.get(cur).copied().flatten().filter(|&p| p < nodes.len()) else {
                break;
            };
            if seeds.contains(&p) {
                intro.insert(idx);
                break;
            }
            cur = p;
        }
    }
    intro
}

struct PdfSpanIndex {
    spans: Vec<(usize, usize, usize)>,
    depths: Vec<usize>,
    orders: Vec<usize>,
}

impl PdfSpanIndex {
    fn build(nodes: &[SkNode], parent_orders: &[Option<usize>]) -> Self {
        let n = nodes.len();
        let depths = (0..n)
            .map(|mut cur| {
                let mut d = 0usize;
                while d < n {
                    let Some(p) = parent_orders.get(cur).copied().flatten().filter(|&p| p < n) else {
                        break;
                    };
                    d += 1;
                    cur = p;
                }
                d
            })
            .collect();
        let mut spans = Vec::with_capacity(n);
        for (idx, node) in nodes.iter().enumerate() {
            for s in &node.spans {
                spans.push((s.start_line, s.end_line, idx));
            }
        }
        spans.sort_by_key(|&(start, _, _)| start);
        Self { spans, depths, orders: nodes.iter().map(|n| n.order).collect() }
    }

    fn enclosing(&self, line_no: usize) -> Option<usize> {
        let upper = self.spans.partition_point(|&(start, _, _)| start <= line_no);
        self.spans[..upper]
            .iter()
            .filter(|&&(_, end, _)| line_no <= end)
            .max_by_key(|&&(s, e, i)| (self.depths[i], Reverse(e.saturating_sub(s) + 1), self.orders[i]))
            .map(|&(_, _, idx)| idx)
    }
}

struct PdfEdgeCtx<'a> {
    parent_orders: &'a [Option<usize>],
    tag_to_idx: &'a BTreeMap<String, usize>,
    span_index: &'a PdfSpanIndex,
    intro_set: &'a BTreeSet<usize>,
    edges: &'a mut Vec<SkEdge>,
}

impl<'a> PdfEdgeCtx<'a> {
    fn proof_owner(&self, nodes: &[SkNode], idx: usize) -> usize {
        if nodes[idx].kind == SkNodeKind::Proof {
            if let Some(&t) = nodes[idx].proof_of.as_deref().and_then(|k| self.tag_to_idx.get(k)) {
                return t;
            }
        }
        idx
    }

    fn nearest_non_eq(&self, nodes: &[SkNode], mut cur: usize) -> Option<usize> {
        for _ in 0..nodes.len() {
            let p = self.parent_orders.get(cur).copied().flatten().filter(|&p| p < nodes.len())?;
            if nodes[p].kind != SkNodeKind::Equation {
                return Some(p);
            }
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

    fn push_use(&mut self, from: String, to: String, role: SkEdgeRole, site: &Span, rule: SkEdgeRule) {
        if from == to {
            return;
        }
        self.edges.push(SkEdge {
            from,
            to,
            kind: SkEdgeKind::Uses,
            role,
            site: site.clone(),
            rule,
            confidence: EdgeConfidence::Heuristic,
        });
    }

    fn emit_uses(&mut self, nodes: &mut [SkNode], target: usize, enc: usize, site: &Span) {
        let (direct_idx, lifted_idx, role) = self.resolve_user(nodes, enc);
        let (Some(target_tag), Some(direct_tag)) = (nodes[target].tag.clone(), nodes[direct_idx].tag.clone()) else {
            return;
        };
        if self.intro_set.contains(&enc) && !self.intro_set.contains(&target) {
            nodes[target].intro_refs = true;
        }
        self.push_use(target_tag.clone(), direct_tag, role, site, SkEdgeRule::TextMention);
        if let Some(l_tag) = lifted_idx.and_then(|i| nodes[i].tag.clone()) {
            self.push_use(target_tag, l_tag, role, site, SkEdgeRule::Lifted);
        }
        if nodes[target].kind == SkNodeKind::Equation {
            if let Some(anc) = self.nearest_non_eq(nodes, target) {
                let owner_idx = self.proof_owner(nodes, anc);
                let eff_user = lifted_idx.unwrap_or(direct_idx);
                if let (Some(o_tag), Some(u_tag)) = (nodes[owner_idx].tag.clone(), nodes[eff_user].tag.clone()) {
                    self.push_use(o_tag, u_tag, role, site, SkEdgeRule::Lifted);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_text_mentions(
        &mut self,
        primary_path: &str,
        lines: &[&str],
        active_lines: &[bool],
        bib_ranges: &[(usize, usize)],
        line_skip_prefix: &BTreeMap<usize, usize>,
        kind_num_map: &BTreeMap<(SkNodeKind, String), Option<usize>>,
        nodes: &mut [SkNode],
    ) {
        static MENTION_RE: OnceLock<Regex> = OnceLock::new();
        static EXTRA_NUM_RE: OnceLock<Regex> = OnceLock::new();

        let re = MENTION_RE.get_or_init(|| {
            Regex::new(concat!(
                r"\b(?i:(theorems?|thms?\.?|lemmas?|lems?\.?|propositions?|props?\.?|",
                r"corollaries|corollary|cors?\.?|definitions?|defns?\.?|def\.?|",
                r"assumptions?|assump\.?|hypothes(?:is|es)|axioms?|conjectures?|conj\.?|",
                r"claims?|observations?|remarks?|rem\.?|examples?|ex\.?|",
                r"sections?|secs?\.?|appendi(?:x|ces)|app\.?|equations?|eqns?\.?|eqs?\.?|",
                r"algorithms?|alg\.?|figures?|figs?\.?|tables?|tab\.?))",
                r"(?:[ \t~]+)",
                r"(?:\(([A-Za-z]?\d+(?:\.\d+)*[a-z]?)\)|",
                r"([IVX]+\.[A-Z]|[A-Z]\.\d+(?:\.\d+)*[a-z]?|\d+(?:\.\d+)*[a-z]?|[IVX]+|[A-Z]))",
                r"(?:\.(?:[^0-9]|$)|[^A-Za-z0-9.\-]|$)",
            ))
            .unwrap()
        });
        let extra_re = EXTRA_NUM_RE.get_or_init(|| {
            Regex::new(concat!(
                r"^(?:\s*,\s*(?:and\s+)?|\s+and\s+)",
                r"(?:\(([A-Za-z]?\d+(?:\.\d+)*[a-z]?)\)|",
                r"([IVX]+\.[A-Z]|[A-Z]\.\d+(?:\.\d+)*[a-z]?|\d+(?:\.\d+)*[a-z]?|[IVX]+|[A-Z]))",
                r"(?:\.(?:[^0-9]|$)|[^A-Za-z0-9.\-]|$)",
            ))
            .unwrap()
        });

        for (idx0, raw_line) in lines.iter().enumerate() {
            let line_no = idx0 + 1;
            if !active_lines[idx0] || bib_ranges.iter().any(|&(s, e)| (s..=e).contains(&line_no)) {
                continue;
            }
            let norm_full = normalize_pdf_unicode(raw_line.trim());
            let skip_from = line_skip_prefix.get(&line_no).copied().unwrap_or(0);
            if skip_from >= norm_full.len() {
                continue;
            }
            let norm = if norm_full.is_char_boundary(skip_from) {
                &norm_full[skip_from..]
            } else {
                norm_full.as_str()
            };
            if norm.trim().is_empty() {
                continue;
            }

            let mut enc_cache: Option<Option<usize>> = None;
            let site = Span::new(primary_path, line_no, line_no);

            for caps in re.captures_iter(norm) {
                let Some(kind) = map_pdf_kind_word(&caps[1], true) else {
                    continue;
                };
                let first_m = caps.get(2).or_else(|| caps.get(3));
                let Some(first_m) = first_m else {
                    continue;
                };
                let mut nums = vec![first_m.as_str().trim().to_string()];

                // Handle chained plural mentions like `Eqs. (3.4) and (3.5)` or `Lemmas 1.1, 1.2`.
                let mut cursor = first_m.end();
                if norm.as_bytes().get(cursor) == Some(&b')') {
                    cursor += 1;
                }
                while let Some(ex) = extra_re.captures(&norm[cursor..]) {
                    let Some(nm) = ex.get(1).or_else(|| ex.get(2)) else {
                        break;
                    };
                    nums.push(nm.as_str().trim().to_string());
                    let mut next = cursor + nm.end();
                    if norm.as_bytes().get(next) == Some(&b')') {
                        next += 1;
                    }
                    cursor = next;
                }

                for num in nums {
                    let Some(Some(target)) = kind_num_map.get(&(kind, num)).copied() else {
                        continue;
                    };
                    let Some(enc) = *enc_cache.get_or_insert_with(|| self.span_index.enclosing(line_no)) else {
                        continue;
                    };
                    self.emit_uses(nodes, target, enc, &site);
                }
            }
        }
    }

    fn emit_cite_edges(&mut self, nodes: &[SkNode], citations: &[PdfCiteOccurrence]) {
        let mut head_sites: BTreeSet<(String, String, SkEdgeRole, usize)> = BTreeSet::new();
        for n in nodes.iter().filter(|n| !n.head_cites.is_empty()) {
            let (to_tag, role) = match n.kind {
                SkNodeKind::Proof => (n.proof_of.clone().or_else(|| n.tag.clone()), SkEdgeRole::Proof),
                k if k.is_theorem_like() => (n.tag.clone(), SkEdgeRole::Statement),
                _ => (n.tag.clone(), SkEdgeRole::Prose),
            };
            let Some(to_tag) = to_tag else {
                continue;
            };
            let site = first_span(n);
            for key in n.head_cites.iter().filter(|k| !k.is_empty()) {
                head_sites.insert((key.clone(), to_tag.clone(), role, site.start_line));
                self.edges.push(SkEdge {
                    from: key.clone(),
                    to: to_tag.clone(),
                    kind: SkEdgeKind::Cites,
                    role,
                    site: site.clone(),
                    rule: SkEdgeRule::HeadCite,
                    confidence: EdgeConfidence::Heuristic,
                });
            }
        }

        for occ in citations.iter().filter(|c| !c.key.is_empty()) {
            let Some(enc) = self.span_index.enclosing(occ.line_no) else {
                continue;
            };
            let (direct_idx, lifted_idx, role) = self.resolve_user(nodes, enc);
            let user_idx = match (nodes[enc].kind, role) {
                (SkNodeKind::Equation, SkEdgeRole::Proof | SkEdgeRole::Statement) => {
                    lifted_idx.unwrap_or(direct_idx)
                }
                (SkNodeKind::Equation, SkEdgeRole::Prose) => enc,
                _ => direct_idx,
            };
            let Some(user_tag) = nodes[user_idx].tag.clone() else {
                continue;
            };
            if head_sites.contains(&(occ.key.clone(), user_tag.clone(), role, occ.site.start_line)) {
                continue;
            }
            self.edges.push(SkEdge {
                from: occ.key.clone(),
                to: user_tag,
                kind: SkEdgeKind::Cites,
                role,
                site: occ.site.clone(),
                rule: SkEdgeRule::Cite,
                confidence: EdgeConfidence::Heuristic,
            });
        }
    }
}

/// Counts words in plain PDF text (preserving literal `%` signs unlike LaTeX comment stripping).
pub(crate) fn count_pdf_words(text: &str) -> u32 {
    let n = text
        .split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Populates `words` and `read_minutes` on every PDF node and returns per-section [`SectionStats`].
pub(crate) fn compute_pdf_stats(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    lines: &[&str],
    active_lines: &[bool],
) -> Vec<SectionStats> {
    let line_words: Vec<u32> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| if active_lines[i] { count_pdf_words(l) } else { 0 })
        .collect();

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
        let mut w = 0u32;
        for sp in &node.spans {
            if sp.start_line == 0 || sp.end_line < sp.start_line || line_words.is_empty() {
                continue;
            }
            let lo = sp.start_line.saturating_sub(1).min(line_words.len().saturating_sub(1));
            let hi = sp.end_line.saturating_sub(1).min(line_words.len().saturating_sub(1));
            for &lw in &line_words[lo..=hi] {
                w = w.saturating_add(lw);
            }
        }
        node.words = w;
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
        stats.push(SectionStats {
            section_tag: None,
            words: top_words,
            read_minutes: estimate_read_minutes(top_words, top_eqs),
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
