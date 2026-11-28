//! Proof-link and dependency-edge passes for `html-1` (Investigate package V18).
//!
//! Keeps edge direction (`from` = prerequisite, `to` = dependent), rule 6.2 proof uniqueness,
//! lifted equation edges, and reading-time math identical to `tex-1`.

use super::html_bib::{compute_html_stats, extract_html_bib, HtmlCiteOccurrence, RawHtmlBibItem};
use super::model::{
    BibRef, EdgeConfidence, NumberBasis, SectionStats, SkEdge, SkEdgeKind, SkEdgeRole, SkEdgeRule,
    SkNode, SkNodeKind, SkeletonQuality, Span, MAX_SKELETON_EDGES,
};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub(crate) enum HtmlProofHint {
    HeaderRef { target_html_id: String },
    HeaderMention { kind: SkNodeKind, number: String },
    Adjacent { target_order: usize },
    Unattached,
}

#[derive(Debug, Clone)]
pub(crate) struct HtmlRefOccurrence {
    pub(crate) enc_idx: usize,
    pub(crate) target_html_id: String,
    pub(crate) site: Span,
    pub(crate) in_proof_header: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct HtmlTextLine {
    pub(crate) enc_idx: usize,
    pub(crate) site: Span,
    pub(crate) text: String,
}

pub(crate) fn map_html_kind_word(raw: &str, allow_non_statement: bool) -> Option<SkNodeKind> {
    let norm = raw.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    match norm.trim_end_matches('.') {
        "theorem" | "thm" | "main theorem" => Some(SkNodeKind::Theorem),
        "lemma" | "lem" => Some(SkNodeKind::Lemma),
        "proposition" | "prop" => Some(SkNodeKind::Proposition),
        "corollary" | "cor" => Some(SkNodeKind::Corollary),
        "conjecture" | "conj" => Some(SkNodeKind::Conjecture),
        "claim" => Some(SkNodeKind::Claim),
        "definition" | "defn" | "def" => Some(SkNodeKind::Definition),
        "assumption" | "assump" | "hypothesis" | "axiom" => Some(SkNodeKind::Assumption),
        "remark" | "rem" | "rmk" | "note" | "observation" if allow_non_statement => {
            Some(SkNodeKind::Remark)
        }
        "example" | "ex" | "exm" if allow_non_statement => Some(SkNodeKind::Example),
        "algorithm" | "alg" if allow_non_statement => Some(SkNodeKind::Algorithm),
        "section" | "sec" | "appendix" if allow_non_statement => Some(SkNodeKind::Section),
        "equation" | "eqn" | "eq" if allow_non_statement => Some(SkNodeKind::Equation),
        "figure" | "fig" if allow_non_statement => Some(SkNodeKind::Figure),
        "table" | "tab" if allow_non_statement => Some(SkNodeKind::Table),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_html_graph(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    html_id_to_node_idx: &BTreeMap<String, usize>,
    proof_hints: &BTreeMap<usize, HtmlProofHint>,
    refs: &[HtmlRefOccurrence],
    cites: &[HtmlCiteOccurrence],
    bib_items: &[RawHtmlBibItem],
    prose_lines: &[HtmlTextLine],
    node_plain_texts: &[String],
    quality: &mut SkeletonQuality,
    quality_notes: &mut Vec<String>,
) -> (Vec<SkEdge>, Vec<BibRef>, Vec<SectionStats>) {
    let mut kind_num_map: BTreeMap<(SkNodeKind, String), Option<usize>> = BTreeMap::new();
    let mut tag_to_idx: BTreeMap<String, usize> = BTreeMap::new();
    for (idx, n) in nodes.iter().enumerate() {
        if let Some(num) = n.number.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            kind_num_map
                .entry((n.kind, num.to_string()))
                .and_modify(|s| *s = None)
                .or_insert(Some(idx));
        }
        if let Some(t) = n.tag.as_deref() {
            tag_to_idx.insert(t.to_string(), idx);
        }
    }

    let mut edges = Vec::new();
    link_proofs(nodes, proof_hints, html_id_to_node_idx, &kind_num_map, &mut edges);

    let intro_set = build_intro_set(nodes, parent_orders);
    let mut explicit_uses: BTreeSet<(String, String, SkEdgeRole)> = BTreeSet::new();

    for r in refs.iter().filter(|r| !r.in_proof_header) {
        let Some(&target) = html_id_to_node_idx.get(&r.target_html_id) else {
            continue;
        };
        let rule = if nodes[target].kind == SkNodeKind::Equation {
            SkEdgeRule::Eqref
        } else {
            SkEdgeRule::Ref
        };
        emit_uses(
            nodes,
            parent_orders,
            &tag_to_idx,
            &intro_set,
            &mut explicit_uses,
            &mut edges,
            target,
            r.enc_idx,
            &r.site,
            rule,
            EdgeConfidence::Explicit,
        );
    }

    scan_text_mentions(
        nodes,
        parent_orders,
        &tag_to_idx,
        &intro_set,
        &kind_num_map,
        prose_lines,
        &mut explicit_uses,
        &mut edges,
    );

    let bib = extract_html_bib(bib_items, cites, nodes, parent_orders, &tag_to_idx, &mut edges);

    let mut seen = BTreeSet::new();
    edges.retain(|e| seen.insert((e.from.clone(), e.to.clone(), e.kind, e.role, e.rule)));
    if edges.len() > MAX_SKELETON_EDGES {
        let total = edges.len();
        edges.truncate(MAX_SKELETON_EDGES);
        if *quality == SkeletonQuality::Full {
            *quality = SkeletonQuality::Partial;
        }
        quality_notes.push(format!(
            "edge count ({total}) exceeded MAX_SKELETON_EDGES ({MAX_SKELETON_EDGES}); truncated"
        ));
    }

    let stats = compute_html_stats(nodes, parent_orders, node_plain_texts);
    (edges, bib, stats)
}

fn first_span(n: &SkNode) -> Span {
    n.spans.first().cloned().unwrap_or_else(|| Span::new("", 1, 1))
}

fn link_proofs(
    nodes: &mut [SkNode],
    proof_hints: &BTreeMap<usize, HtmlProofHint>,
    html_id_to_node_idx: &BTreeMap<String, usize>,
    kind_num_map: &BTreeMap<(SkNodeKind, String), Option<usize>>,
    edges: &mut Vec<SkEdge>,
) {
    for p_idx in 0..nodes.len() {
        if nodes[p_idx].kind != SkNodeKind::Proof {
            continue;
        }
        nodes[p_idx].proof_of = None;
        let Some(p_tag) = nodes[p_idx].tag.clone() else { continue };
        let resolved = match proof_hints.get(&p_idx) {
            Some(HtmlProofHint::HeaderRef { target_html_id }) => html_id_to_node_idx
                .get(target_html_id)
                .copied()
                .filter(|&t| t != p_idx && nodes[t].kind.is_provable())
                .map(|t| (t, SkEdgeRule::ProofHeader, EdgeConfidence::Explicit)),
            Some(HtmlProofHint::HeaderMention { kind, number }) => kind_num_map
                .get(&(*kind, number.trim().to_string()))
                .copied()
                .flatten()
                .filter(|&t| t != p_idx && nodes[t].kind.is_provable())
                .map(|t| {
                    let conf = match nodes[t].number_basis {
                        Some(NumberBasis::Html | NumberBasis::Confirmed | NumberBasis::Printed) => {
                            EdgeConfidence::Explicit
                        }
                        _ => EdgeConfidence::Heuristic,
                    };
                    (t, SkEdgeRule::ProofHeader, conf)
                }),
            Some(HtmlProofHint::Adjacent { target_order }) => {
                (*target_order < nodes.len() && *target_order != p_idx)
                    .then_some((*target_order, SkEdgeRule::ProofAdjacent, EdgeConfidence::Explicit))
            }
            Some(HtmlProofHint::Unattached) | None => None,
        };
        if let Some((t_idx, rule, confidence)) = resolved {
            if let Some(t_tag) = nodes[t_idx].tag.clone() {
                nodes[p_idx].proof_of = Some(t_tag.clone());
                edges.push(SkEdge {
                    from: p_tag,
                    to: t_tag,
                    kind: SkEdgeKind::Proves,
                    role: SkEdgeRole::Proof,
                    site: first_span(&nodes[p_idx]),
                    rule,
                    confidence,
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
            let Some(p) = parent_orders.get(cur).copied().flatten().filter(|&p| p < nodes.len())
            else {
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

pub(crate) fn proof_owner(nodes: &[SkNode], tag_to_idx: &BTreeMap<String, usize>, idx: usize) -> usize {
    if nodes[idx].kind == SkNodeKind::Proof {
        if let Some(&t) = nodes[idx].proof_of.as_deref().and_then(|k| tag_to_idx.get(k)) {
            return t;
        }
    }
    idx
}

fn nearest_non_eq(
    nodes: &[SkNode],
    parent_orders: &[Option<usize>],
    mut cur: usize,
) -> Option<usize> {
    for _ in 0..nodes.len() {
        let p = parent_orders.get(cur).copied().flatten().filter(|&p| p < nodes.len())?;
        if nodes[p].kind != SkNodeKind::Equation {
            return Some(p);
        }
        cur = p;
    }
    None
}

pub(crate) fn resolve_user(
    nodes: &[SkNode],
    parent_orders: &[Option<usize>],
    tag_to_idx: &BTreeMap<String, usize>,
    enc: usize,
) -> (usize, Option<usize>, SkEdgeRole) {
    match nodes[enc].kind {
        SkNodeKind::Proof => (proof_owner(nodes, tag_to_idx, enc), None, SkEdgeRole::Proof),
        k if k.is_theorem_like() => (enc, None, SkEdgeRole::Statement),
        SkNodeKind::Equation => match nearest_non_eq(nodes, parent_orders, enc) {
            Some(a) if nodes[a].kind == SkNodeKind::Proof => {
                let o = proof_owner(nodes, tag_to_idx, a);
                (enc, (o != enc).then_some(o), SkEdgeRole::Proof)
            }
            Some(a) if nodes[a].kind.is_theorem_like() => (enc, Some(a), SkEdgeRole::Statement),
            Some(a) => (enc, Some(a), SkEdgeRole::Prose),
            None => (enc, None, SkEdgeRole::Prose),
        },
        _ => (enc, None, SkEdgeRole::Prose),
    }
}

#[allow(clippy::too_many_arguments)]
fn push_use(
    explicit_uses: &mut BTreeSet<(String, String, SkEdgeRole)>,
    edges: &mut Vec<SkEdge>,
    from: String,
    to: String,
    role: SkEdgeRole,
    site: &Span,
    rule: SkEdgeRule,
    conf: EdgeConfidence,
) {
    if from == to {
        return;
    }
    let key = (from.clone(), to.clone(), role);
    if conf == EdgeConfidence::Heuristic && explicit_uses.contains(&key) {
        return;
    }
    if conf == EdgeConfidence::Explicit {
        explicit_uses.insert(key);
    }
    edges.push(SkEdge {
        from,
        to,
        kind: SkEdgeKind::Uses,
        role,
        site: site.clone(),
        rule,
        confidence: conf,
    });
}

#[allow(clippy::too_many_arguments)]
fn emit_uses(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    tag_to_idx: &BTreeMap<String, usize>,
    intro_set: &BTreeSet<usize>,
    explicit_uses: &mut BTreeSet<(String, String, SkEdgeRole)>,
    edges: &mut Vec<SkEdge>,
    target: usize,
    enc: usize,
    site: &Span,
    rule: SkEdgeRule,
    conf: EdgeConfidence,
) {
    let (direct_idx, lifted_idx, role) = resolve_user(nodes, parent_orders, tag_to_idx, enc);
    let (Some(target_tag), Some(direct_tag)) =
        (nodes[target].tag.clone(), nodes[direct_idx].tag.clone())
    else {
        return;
    };
    if intro_set.contains(&enc) && !intro_set.contains(&target) {
        nodes[target].intro_refs = true;
    }
    push_use(explicit_uses, edges, target_tag.clone(), direct_tag, role, site, rule, conf);
    if let Some(l_tag) = lifted_idx.and_then(|i| nodes[i].tag.clone()) {
        push_use(explicit_uses, edges, target_tag, l_tag, role, site, SkEdgeRule::Lifted, conf);
    }
    if nodes[target].kind == SkNodeKind::Equation {
        if let Some(anc) = nearest_non_eq(nodes, parent_orders, target) {
            let owner_idx = proof_owner(nodes, tag_to_idx, anc);
            let eff_user = lifted_idx.unwrap_or(direct_idx);
            if let (Some(o_tag), Some(u_tag)) =
                (nodes[owner_idx].tag.clone(), nodes[eff_user].tag.clone())
            {
                push_use(
                    explicit_uses,
                    edges,
                    o_tag,
                    u_tag,
                    role,
                    site,
                    SkEdgeRule::Lifted,
                    conf,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_text_mentions(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    tag_to_idx: &BTreeMap<String, usize>,
    intro_set: &BTreeSet<usize>,
    kind_num_map: &BTreeMap<(SkNodeKind, String), Option<usize>>,
    prose_lines: &[HtmlTextLine],
    explicit_uses: &mut BTreeSet<(String, String, SkEdgeRole)>,
    edges: &mut Vec<SkEdge>,
) {
    static MENTION_RE: OnceLock<Regex> = OnceLock::new();
    let re = MENTION_RE.get_or_init(|| {
        Regex::new(r"\b(?i:(theorem|thm\.?|lemma|lem\.?|proposition|prop\.?|corollary|cor\.?|definition|defn\.?|def\.?|assumption|assump\.?|conjecture|conj\.?|claim|remark|rem\.?|example|ex\.?|section|sec\.?|equation|eqn\.?|eq\.?|algorithm|alg\.?|figure|fig\.?|table|tab\.?))(?:[ \t\u{00a0}]+)(?:\(([A-Za-z0-9]+(?:[.\-][A-Za-z0-9]+)*)\)|([0-9]+(?:[.\-][A-Za-z0-9]+)*[A-Za-z]?|[A-Z]\.[0-9]+(?:[.\-][A-Za-z0-9]+)*|[A-Z])\b)").unwrap()
    });
    for line in prose_lines {
        if line.enc_idx >= nodes.len() {
            continue;
        }
        for caps in re.captures_iter(&line.text) {
            let Some(kind) = map_html_kind_word(&caps[1], true) else { continue };
            let num = caps.get(2).or_else(|| caps.get(3)).map_or("", |m| m.as_str().trim());
            if num.is_empty() {
                continue;
            }
            let Some(Some(target)) = kind_num_map.get(&(kind, num.to_string())).copied() else {
                continue;
            };
            emit_uses(
                nodes,
                parent_orders,
                tag_to_idx,
                intro_set,
                explicit_uses,
                edges,
                target,
                line.enc_idx,
                &line.site,
                SkEdgeRule::TextMention,
                EdgeConfidence::Heuristic,
            );
        }
    }
}
