//! Bibliography extraction and section reading-time statistics for `html-1`
//! (Investigate package V18).

use super::html_edges::{proof_owner, resolve_user};
use super::model::{
    BibLocator, BibRef, EdgeConfidence, SectionStats, SkEdge, SkEdgeKind, SkEdgeRole, SkEdgeRule,
    SkNode, SkNodeKind, Span,
};
use super::stats::estimate_read_minutes;
use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::tex_scan::{scan_arxiv_base_ids, scan_normalized_dois};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BIB_RAW_CHARS: usize = 2_000;

#[derive(Debug, Clone)]
pub(crate) struct HtmlCiteOccurrence {
    pub(crate) enc_idx: usize,
    pub(crate) target_bib_ids: Vec<String>,
    pub(crate) raw_cite_text: String,
    pub(crate) site: Span,
    pub(crate) in_heading: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RawHtmlBibItem {
    pub(crate) html_id: Option<String>,
    pub(crate) tag_label: Option<String>,
    pub(crate) raw_text: String,
    pub(crate) title_hint: Option<String>,
    pub(crate) link_urls: Vec<String>,
}

pub(crate) fn extract_html_bib(
    bib_items: &[RawHtmlBibItem],
    cites: &[HtmlCiteOccurrence],
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    tag_to_idx: &BTreeMap<String, usize>,
    edges: &mut Vec<SkEdge>,
) -> Vec<BibRef> {
    let mut entries: BTreeMap<String, BibRef> = BTreeMap::new();
    let mut html_id_to_key: BTreeMap<String, String> = BTreeMap::new();

    for (idx, item) in bib_items.iter().enumerate() {
        let base_key = item
            .tag_label
            .as_deref()
            .map(clean_bib_tag)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                item.html_id
                    .as_deref()
                    .map(|id| id.strip_prefix("bib.").unwrap_or(id).trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| format!("bib{}", idx + 1));

        let mut key = base_key.clone();
        if entries.contains_key(&key) {
            if let Some(id) = item.html_id.as_deref().map(|s| s.strip_prefix("bib.").unwrap_or(s)) {
                if !id.is_empty() && !entries.contains_key(id) {
                    key = id.to_string();
                } else {
                    key = format!("{base_key}#{}", idx + 1);
                }
            } else {
                key = format!("{base_key}#{}", idx + 1);
            }
        }
        if let Some(ref html_id) = item.html_id {
            html_id_to_key.insert(html_id.clone(), key.clone());
        }

        let search_blob = if item.link_urls.is_empty() {
            item.raw_text.clone()
        } else {
            format!("{} {}", item.raw_text, item.link_urls.join(" "))
        };
        let arxiv_id = scan_arxiv_base_ids(&search_blob)
            .into_iter()
            .find_map(|c| split_arxiv_id(&c).ok().map(|(b, _)| b));
        let doi = scan_normalized_dois(&search_blob)
            .into_iter()
            .find_map(|c| canonical_doi(&c).ok());
        let raw = cap_bib_raw(&item.raw_text);
        let title = item.title_hint.clone().or_else(|| extract_quoted_title(&item.raw_text));
        entries.insert(
            key.clone(),
            BibRef {
                key,
                raw,
                arxiv_id,
                doi,
                title,
                cited_at: Vec::new(),
                locators: Vec::new(),
            },
        );
    }

    let mut head_sites: BTreeSet<(String, String, SkEdgeRole, String, usize)> = BTreeSet::new();
    for occ in cites {
        if occ.enc_idx >= nodes.len() {
            continue;
        }
        let (keys, locator) = resolve_cite_keys_and_locator(occ, &html_id_to_key);
        if keys.is_empty() {
            continue;
        }
        let (direct_idx, lifted_idx, mut role) =
            resolve_user(nodes, parent_orders, tag_to_idx, occ.enc_idx);
        let user_idx = match (nodes[occ.enc_idx].kind, role) {
            (SkNodeKind::Equation, SkEdgeRole::Proof | SkEdgeRole::Statement) => {
                let target = lifted_idx.unwrap_or(direct_idx);
                let _ = proof_owner; // keep helper cohesion documented
                target
            }
            (SkNodeKind::Equation, SkEdgeRole::Prose) => occ.enc_idx,
            _ => direct_idx,
        };
        if occ.in_heading && nodes[occ.enc_idx].kind == SkNodeKind::Proof {
            role = SkEdgeRole::Proof;
        }
        let Some(user_tag) = nodes[user_idx].tag.clone() else { continue };

        for key in keys {
            let bib = entries.entry(key.clone()).or_insert_with(|| BibRef {
                key: key.clone(),
                raw: String::new(),
                arxiv_id: None,
                doi: None,
                title: None,
                cited_at: Vec::new(),
                locators: Vec::new(),
            });
            if !bib.cited_at.contains(&occ.site) {
                bib.cited_at.push(occ.site.clone());
            }
            if let Some(ref loc_text) = locator {
                let loc = BibLocator { site: occ.site.clone(), text: loc_text.clone() };
                if !bib.locators.contains(&loc) {
                    bib.locators.push(loc);
                }
            }
            let site_key = (
                key.clone(),
                user_tag.clone(),
                role,
                occ.site.path.clone(),
                occ.site.start_line,
            );
            if occ.in_heading {
                if !nodes[occ.enc_idx].head_cites.contains(&key) {
                    nodes[occ.enc_idx].head_cites.push(key.clone());
                }
                head_sites.insert(site_key);
                edges.push(SkEdge {
                    from: key,
                    to: user_tag.clone(),
                    kind: SkEdgeKind::Cites,
                    role,
                    site: occ.site.clone(),
                    rule: SkEdgeRule::HeadCite,
                    confidence: EdgeConfidence::Explicit,
                });
            } else if !head_sites.contains(&site_key) {
                edges.push(SkEdge {
                    from: key,
                    to: user_tag.clone(),
                    kind: SkEdgeKind::Cites,
                    role,
                    site: occ.site.clone(),
                    rule: SkEdgeRule::Cite,
                    confidence: EdgeConfidence::Explicit,
                });
            }
        }
    }

    entries.into_values().collect()
}

fn clean_bib_tag(raw: &str) -> String {
    raw.trim()
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')', '.'])
        .trim()
        .to_string()
}

fn cap_bib_raw(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .char_indices()
        .nth(MAX_BIB_RAW_CHARS)
        .map_or_else(|| collapsed.clone(), |(i, _)| collapsed[..i].trim_end().to_string())
}

fn extract_quoted_title(raw: &str) -> Option<String> {
    for (open, close) in [('“', '”'), ('"', '"')] {
        if let Some(s) = raw.find(open) {
            let rest = &raw[s + open.len_utf8()..];
            if let Some(e) = rest.find(close) {
                let cand = rest[..e].trim().trim_end_matches(',').trim();
                if !cand.is_empty() {
                    return Some(cand.to_string());
                }
            }
        }
    }
    None
}

fn resolve_cite_keys_and_locator(
    occ: &HtmlCiteOccurrence,
    html_id_to_key: &BTreeMap<String, String>,
) -> (Vec<String>, Option<String>) {
    let mut keys: Vec<String> = occ
        .target_bib_ids
        .iter()
        .filter_map(|id| {
            html_id_to_key.get(id).cloned().or_else(|| {
                let s = id.strip_prefix("bib.").unwrap_or(id).trim();
                (!s.is_empty()).then(|| s.to_string())
            })
        })
        .collect();

    let inner = occ
        .raw_cite_text
        .trim()
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')'])
        .trim();

    let mut locator: Option<String> = None;
    if !inner.is_empty() {
        let parts: Vec<&str> = inner.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if keys.is_empty() {
            for part in &parts {
                if !looks_like_locator(part) {
                    keys.push((*part).to_string());
                }
            }
        }
        if let Some(last) = parts.last() {
            if parts.len() > 1 && looks_like_locator(last) {
                locator = Some((*last).to_string());
            } else if let Some((_, after_semi)) = inner.split_once(';') {
                let cand = after_semi.trim();
                if looks_like_locator(cand) {
                    locator = Some(cand.to_string());
                }
            }
        }
    }
    keys.dedup();
    (keys, locator)
}

fn looks_like_locator(s: &str) -> bool {
    let low = s.to_ascii_lowercase();
    [
        "theorem", "thm", "lemma", "lem", "prop", "proposition", "corollary", "cor", "section",
        "sec", "definition", "def", "defn", "remark", "rem", "eq.", "equation", "chapter", "ch.",
        "p.", "pp.", "§",
    ]
    .iter()
    .any(|kw| low.contains(kw))
}

pub(crate) fn compute_html_stats(
    nodes: &mut [SkNode],
    parent_orders: &[Option<usize>],
    node_plain_texts: &[String],
) -> Vec<SectionStats> {
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
        let words = node_plain_texts.get(i).map_or(0, |t| count_plain_words(t));
        node.words = words;
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

fn count_plain_words(text: &str) -> u32 {
    let n = text
        .split_whitespace()
        .filter(|tok| tok.chars().any(char::is_alphanumeric))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}
