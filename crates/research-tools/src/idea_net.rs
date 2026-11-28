//! Deterministic citation-network builder for Explore mode (package X16).
//!
//! Re-codes the LocalCitationNetwork pattern (`explore.md` section 11) in pure Rust over F11
//! `cites` / `cited_by` [`CitationEdge`]s and F2 [`PaperKey`] / [`PaperRef`] identities:
//! - Collapses cross-provider identifier aliases to a single canonical F2 [`PaperKey`].
//! - Deduplicates citation edges across providers and ignores self-loops.
//! - Computes deterministic in-network rank (`in_degree + out_degree`), completely invariant
//!   to input edge, paper, or seed ordering.
//! - Identifies `"cited by your core papers but unread"` candidates while excluding already-read
//!   papers and any Rule V9 `handoff.packet.from.paper` origin paper (and its alias keys).

use crate::idea_verdict::origin_paper_from_events;
use crate::lit_providers::{CitationEdge, LitPaper, LitProvider};
use crate::paper_key::{PaperIds, PaperKey, PaperKeyKind, PaperRef};
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Ledger event kind for Explore citation networks (`explore.md` 4.2).
pub const EVENT_IDEA_CITATION_NET: &str = "idea.citation_net";

/// Metadata for a paper participating in a citation network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationPaperMeta {
    pub key: Option<PaperKey>,
    #[serde(default)]
    pub ids: PaperIds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u64>,
    #[serde(default)]
    pub in_library: bool,
    #[serde(default)]
    pub owned: bool,
}

impl CitationPaperMeta {
    pub fn from_lit_paper(p: &LitPaper) -> Self {
        Self {
            key: Some(p.paper_ref.key.clone()),
            ids: p.paper_ref.ids.clone(),
            title: (!p.title.trim().is_empty()).then(|| p.title.trim().to_string()),
            authors: p.authors.clone(),
            year: p.year,
            citation_count: p.citation_count,
            in_library: false,
            owned: false,
        }
    }
}

/// Pure input for [`build`].
#[derive(Debug, Clone, Default)]
pub struct CitationNetInput {
    pub seeds: Vec<PaperKey>,
    pub core_papers: Vec<PaperKey>,
    pub read_papers: BTreeSet<PaperKey>,
    pub origin_paper: Option<PaperRef>,
    pub edges: Vec<CitationEdge>,
    pub papers: Vec<CitationPaperMeta>,
    pub hops: usize,
}

/// Ranked paper node inside a [`CitationNetwork`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationNetNode {
    pub key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u64>,
    pub in_network_in_degree: usize,
    pub in_network_out_degree: usize,
    pub in_network_rank: usize,
    pub cited_by_core_count: usize,
    #[serde(default)]
    pub cited_by_core: Vec<PaperKey>,
    pub cites_core_count: usize,
    pub is_seed: bool,
    pub is_core: bool,
    pub is_read: bool,
    pub is_origin: bool,
    #[serde(default)]
    pub in_library: bool,
    #[serde(default)]
    pub owned: bool,
}

/// Deduplicated directed citation edge (`from` cites `to`) inside a [`CitationNetwork`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationNetEdge {
    pub from: PaperKey,
    pub to: PaperKey,
    pub provider: LitProvider,
    #[serde(default)]
    pub providers: Vec<LitProvider>,
}

/// Candidate paper cited by one or more core papers that has not yet been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadCoreCitation {
    pub key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u64>,
    pub cited_by_core_count: usize,
    pub cited_by_core: Vec<PaperKey>,
    pub in_network_rank: usize,
    pub in_network_in_degree: usize,
    pub in_network_out_degree: usize,
    #[serde(default)]
    pub in_library: bool,
    #[serde(default)]
    pub owned: bool,
}

/// Deterministic citation network ready to persist in `idea.citation_net` and project to UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationNetwork {
    pub seeds: Vec<PaperKey>,
    pub hops: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<LitProvider>,
    pub providers: Vec<LitProvider>,
    pub nodes: Vec<CitationNetNode>,
    pub edges: Vec<CitationNetEdge>,
    pub unread_core_citations: Vec<UnreadCoreCitation>,
}

/// Providers that support `"cites"` or `"cited_by"` for `key`, matching F11 & X19.
pub fn supported_citation_providers(key: &PaperKey, relation: &str) -> Vec<LitProvider> {
    if relation != "cites" && relation != "cited_by" {
        return Vec::new();
    }
    match key.kind() {
        PaperKeyKind::Inspire => vec![LitProvider::Inspire],
        PaperKeyKind::Arxiv => vec![LitProvider::Inspire, LitProvider::SemanticScholar],
        PaperKeyKind::Doi if relation == "cites" => vec![
            LitProvider::Inspire,
            LitProvider::OpenAlex,
            LitProvider::SemanticScholar,
            LitProvider::Crossref,
        ],
        PaperKeyKind::Doi => vec![
            LitProvider::Inspire,
            LitProvider::OpenAlex,
            LitProvider::SemanticScholar,
        ],
        PaperKeyKind::Openalex => vec![LitProvider::OpenAlex],
        PaperKeyKind::S2 => vec![LitProvider::SemanticScholar],
        PaperKeyKind::File => Vec::new(),
    }
}

/// Preferred single default provider for a citation-relation lookup on `key`.
pub fn default_citation_provider(key: &PaperKey, relation: &str) -> Option<LitProvider> {
    supported_citation_providers(key, relation).into_iter().next()
}

/// Pure deterministic citation-network builder (`explore.md` 5.3 `idea_net::build`).
pub fn build(input: CitationNetInput) -> CitationNetwork {
    let alias_map = build_alias_map(&input.papers, input.origin_paper.as_ref());
    let canon = |k: &PaperKey| alias_map.get(k).cloned().unwrap_or_else(|| k.clone());

    let seed_set: BTreeSet<PaperKey> = input.seeds.iter().map(canon).collect();
    let core_set: BTreeSet<PaperKey> = if input.core_papers.is_empty() {
        seed_set.clone()
    } else {
        input.core_papers.iter().map(canon).collect()
    };
    let read_set: BTreeSet<PaperKey> = input.read_papers.iter().map(canon).collect();
    let origin_set: BTreeSet<PaperKey> = input
        .origin_paper
        .as_ref()
        .map(|orig| paper_ref_keys(orig).into_iter().map(|k| canon(&k)).collect())
        .unwrap_or_default();

    // Sort papers before merging so metadata aggregation is invariant to input order.
    let mut sorted_papers = input.papers;
    sorted_papers.sort_by(|a, b| {
        meta_canonical_key(a, &alias_map)
            .cmp(&meta_canonical_key(b, &alias_map))
            .then_with(|| a.year.cmp(&b.year))
            .then_with(|| a.citation_count.cmp(&b.citation_count))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.authors.cmp(&b.authors))
    });
    let mut meta_by_key: BTreeMap<PaperKey, CitationPaperMeta> = BTreeMap::new();
    for pm in sorted_papers {
        let Some(ck) = meta_canonical_key(&pm, &alias_map) else { continue };
        let entry = meta_by_key.entry(ck.clone()).or_insert_with(|| CitationPaperMeta {
            key: Some(ck),
            ..CitationPaperMeta::default()
        });
        merge_meta(entry, &pm);
    }

    let mut edge_providers: BTreeMap<(PaperKey, PaperKey), BTreeSet<LitProvider>> = BTreeMap::new();
    let mut all_providers: BTreeSet<LitProvider> = BTreeSet::new();
    for edge in &input.edges {
        let (from, to) = (canon(&edge.from), canon(&edge.to));
        if from == to {
            continue;
        }
        all_providers.insert(edge.provider);
        edge_providers.entry((from, to)).or_default().insert(edge.provider);
    }

    let mut node_keys: BTreeSet<PaperKey> = seed_set.union(&core_set).cloned().collect();
    let mut in_neighbors: BTreeMap<PaperKey, BTreeSet<PaperKey>> = BTreeMap::new();
    let mut out_neighbors: BTreeMap<PaperKey, BTreeSet<PaperKey>> = BTreeMap::new();
    let mut edges = Vec::with_capacity(edge_providers.len());

    for ((from, to), provs) in edge_providers {
        node_keys.insert(from.clone());
        node_keys.insert(to.clone());
        out_neighbors.entry(from.clone()).or_default().insert(to.clone());
        in_neighbors.entry(to.clone()).or_default().insert(from.clone());
        let providers: Vec<LitProvider> = provs.into_iter().collect();
        edges.push(CitationNetEdge { from, to, provider: providers[0], providers });
    }

    let mut nodes = Vec::with_capacity(node_keys.len());
    let mut unread_core_citations = Vec::new();
    for key in node_keys {
        let in_set = in_neighbors.get(&key);
        let out_set = out_neighbors.get(&key);
        let in_deg = in_set.map_or(0, BTreeSet::len);
        let out_deg = out_set.map_or(0, BTreeSet::len);
        let rank = in_deg.saturating_add(out_deg);
        let cited_by_core: Vec<PaperKey> = in_set
            .map(|s| s.iter().filter(|src| core_set.contains(*src)).cloned().collect())
            .unwrap_or_default();
        let cites_core_count = out_set
            .map(|s| s.iter().filter(|dst| core_set.contains(*dst)).count())
            .unwrap_or(0);
        let (is_seed, is_core, is_read, is_origin) = (
            seed_set.contains(&key),
            core_set.contains(&key),
            read_set.contains(&key),
            origin_set.contains(&key),
        );
        let meta = meta_by_key.get(&key);
        let title = meta.and_then(|m| m.title.clone());
        let authors = meta.map(|m| m.authors.clone()).unwrap_or_default();
        let year = meta.and_then(|m| m.year);
        let citation_count = meta.and_then(|m| m.citation_count);
        let (in_library, owned) = meta.map_or((false, false), |m| (m.in_library, m.owned));

        if !cited_by_core.is_empty() && !is_read && !is_origin {
            unread_core_citations.push(UnreadCoreCitation {
                key: key.clone(),
                title: title.clone(),
                authors: authors.clone(),
                year,
                citation_count,
                cited_by_core_count: cited_by_core.len(),
                cited_by_core: cited_by_core.clone(),
                in_network_rank: rank,
                in_network_in_degree: in_deg,
                in_network_out_degree: out_deg,
                in_library,
                owned,
            });
        }
        nodes.push(CitationNetNode {
            key,
            title,
            authors,
            year,
            citation_count,
            in_network_in_degree: in_deg,
            in_network_out_degree: out_deg,
            in_network_rank: rank,
            cited_by_core_count: cited_by_core.len(),
            cited_by_core,
            cites_core_count,
            is_seed,
            is_core,
            is_read,
            is_origin,
            in_library,
            owned,
        });
    }

    nodes.sort_by(|a, b| {
        b.in_network_rank
            .cmp(&a.in_network_rank)
            .then_with(|| b.in_network_in_degree.cmp(&a.in_network_in_degree))
            .then_with(|| b.cited_by_core_count.cmp(&a.cited_by_core_count))
            .then_with(|| b.citation_count.cmp(&a.citation_count))
            .then_with(|| a.key.cmp(&b.key))
    });
    unread_core_citations.sort_by(|a, b| {
        b.cited_by_core_count
            .cmp(&a.cited_by_core_count)
            .then_with(|| b.in_network_rank.cmp(&a.in_network_rank))
            .then_with(|| b.citation_count.cmp(&a.citation_count))
            .then_with(|| a.key.cmp(&b.key))
    });

    let seeds: Vec<PaperKey> = seed_set.into_iter().collect();
    let hops = if seeds.is_empty() { 0 } else { input.hops.max(1) };
    let providers: Vec<LitProvider> = all_providers.into_iter().collect();
    CitationNetwork {
        seeds,
        hops,
        provider: providers.first().copied(),
        providers,
        nodes,
        edges,
        unread_core_citations,
    }
}

/// Fold the active revision's ledger events plus newly fetched edges/metadata into a
/// cumulative [`CitationNetwork`].
pub fn build_from_events(
    events: &[Event],
    extra_edges: &[CitationEdge],
    extra_papers: &[CitationPaperMeta],
    extra_seeds: &[PaperKey],
    extra_hops: usize,
) -> CitationNetwork {
    let stmt_seq = events.iter().rev().find(|e| e.kind == "idea.stated").map_or(0, |e| e.seq);
    let mut seeds: BTreeSet<PaperKey> = extra_seeds.iter().cloned().collect();
    let mut core_papers: BTreeSet<PaperKey> = BTreeSet::new();
    let mut read_papers: BTreeSet<PaperKey> = BTreeSet::new();
    let mut edges = extra_edges.to_vec();
    let mut papers = extra_papers.to_vec();
    let mut max_hops = extra_hops;

    for ev in events.iter().filter(|e| e.seq >= stmt_seq) {
        match ev.kind.as_str() {
            "idea.stated" => collect_key_array(&ev.payload["seeds"], &mut seeds),
            "idea.candidates" => {
                if let Some(arr) = ev.payload["candidates"].as_array() {
                    papers.extend(arr.iter().filter_map(parse_candidate_meta));
                }
            }
            "idea.triage" => {
                if let Some(arr) = ev.payload["items"].as_array() {
                    for it in arr.iter().filter(|i| i["relevance"].as_str() == Some("core")) {
                        if let Some(k) = it["key"].as_str().and_then(|s| PaperKey::parse(s).ok()) {
                            core_papers.insert(k);
                        }
                    }
                }
            }
            "idea.paper" => {
                let st = ev.payload["status"].as_str().unwrap_or("");
                if matches!(st, "read_full" | "read_passages" | "nothing_relevant") {
                    if let Some(k) = ev.payload["key"].as_str().and_then(|s| PaperKey::parse(s).ok()) {
                        read_papers.insert(k.clone());
                        if st != "nothing_relevant" {
                            core_papers.insert(k);
                        }
                    }
                }
            }
            EVENT_IDEA_CITATION_NET => {
                collect_key_array(&ev.payload["seeds"], &mut seeds);
                if let Some(h) = ev.payload["hops"].as_u64() {
                    max_hops = max_hops.max(h as usize);
                }
                parse_stored_citation_edges(&ev.payload, &mut edges);
                if let Some(arr) = ev.payload["nodes"].as_array() {
                    papers.extend(arr.iter().filter_map(parse_candidate_meta));
                }
            }
            _ => {}
        }
    }

    build(CitationNetInput {
        seeds: seeds.into_iter().collect(),
        core_papers: core_papers.into_iter().collect(),
        read_papers,
        origin_paper: origin_paper_from_events(events),
        edges,
        papers,
        hops: max_hops,
    })
}

fn collect_key_array(val: &Value, dst: &mut BTreeSet<PaperKey>) {
    if let Some(arr) = val.as_array() {
        for s in arr.iter().filter_map(Value::as_str) {
            if let Ok(k) = PaperKey::parse(s) {
                dst.insert(k);
            }
        }
    }
}

fn parse_stored_citation_edges(payload: &Value, dst: &mut Vec<CitationEdge>) {
    let Some(arr) = payload["edges"].as_array() else { return };
    let fallback = payload["provider"]
        .as_str()
        .and_then(LitProvider::parse)
        .unwrap_or(LitProvider::Inspire);
    for e in arr {
        let from = e["from"].as_str().or_else(|| e["citing"].as_str()).and_then(|s| PaperKey::parse(s).ok());
        let to = e["to"].as_str().or_else(|| e["cited"].as_str()).and_then(|s| PaperKey::parse(s).ok());
        let (Some(from), Some(to)) = (from, to) else { continue };
        let mut provs: Vec<LitProvider> = e["providers"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).filter_map(LitProvider::parse).collect())
            .unwrap_or_default();
        if provs.is_empty() {
            provs.push(e["provider"].as_str().and_then(LitProvider::parse).unwrap_or(fallback));
        }
        for provider in provs {
            dst.push(CitationEdge { from: from.clone(), to: to.clone(), provider });
        }
    }
}

fn parse_candidate_meta(v: &Value) -> Option<CitationPaperMeta> {
    let key = v["key"].as_str().and_then(|s| PaperKey::parse(s).ok())?;
    let ids: PaperIds = serde_json::from_value(v["ids"].clone()).unwrap_or_default();
    let title = v["title"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let authors = v["authors"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    Some(CitationPaperMeta {
        key: Some(key),
        ids,
        title,
        authors,
        year: v["year"].as_i64().map(|y| y as i32),
        citation_count: v["citation_count"].as_u64(),
        in_library: v["in_library"].as_bool().unwrap_or(false),
        owned: v["owned"].as_bool().unwrap_or(false),
    })
}

fn meta_keys(m: &CitationPaperMeta) -> Vec<PaperKey> {
    let mut keys = Vec::new();
    if let Some(ref k) = m.key {
        keys.push(k.clone());
    }
    collect_ids_keys(&m.ids, &mut keys);
    keys
}

fn paper_ref_keys(r: &PaperRef) -> Vec<PaperKey> {
    let mut keys = vec![r.key.clone()];
    collect_ids_keys(&r.ids, &mut keys);
    keys
}

fn collect_ids_keys(ids: &PaperIds, out: &mut Vec<PaperKey>) {
    if let Some(ref a) = ids.arxiv { if let Ok(k) = PaperKey::from_arxiv(a) { out.push(k); } }
    if let Some(ref d) = ids.doi { if let Ok(k) = PaperKey::from_doi(d) { out.push(k); } }
    if let Some(ref i) = ids.inspire { if let Ok(k) = PaperKey::from_inspire(i) { out.push(k); } }
    if let Some(ref s) = ids.s2 { if let Ok(k) = PaperKey::from_s2(s) { out.push(k); } }
}

fn canonical_order(a: &PaperKey, b: &PaperKey) -> std::cmp::Ordering {
    a.kind().cmp(&b.kind()).then_with(|| a.as_str().cmp(b.as_str()))
}

fn build_alias_map(papers: &[CitationPaperMeta], origin: Option<&PaperRef>) -> BTreeMap<PaperKey, PaperKey> {
    let mut parent: BTreeMap<PaperKey, PaperKey> = BTreeMap::new();
    fn find(parent: &mut BTreeMap<PaperKey, PaperKey>, k: &PaperKey) -> PaperKey {
        let mut cur = k.clone();
        let mut trail = Vec::new();
        while let Some(next) = parent.get(&cur) {
            if next == &cur { break; }
            trail.push(cur.clone());
            cur = next.clone();
        }
        for n in trail { parent.insert(n, cur.clone()); }
        cur
    }
    let mut union_group = |keys: Vec<PaperKey>| {
        if keys.is_empty() { return; }
        for k in &keys { parent.entry(k.clone()).or_insert_with(|| k.clone()); }
        let mut root = find(&mut parent, &keys[0]);
        for k in keys.iter().skip(1) {
            let other = find(&mut parent, k);
            if root != other {
                if canonical_order(&root, &other).is_le() {
                    parent.insert(other, root.clone());
                } else {
                    parent.insert(root, other.clone());
                    root = other;
                }
            }
        }
    };
    for pm in papers { union_group(meta_keys(pm)); }
    if let Some(orig) = origin { union_group(paper_ref_keys(orig)); }
    let all_keys: Vec<PaperKey> = parent.keys().cloned().collect();
    all_keys.into_iter().map(|k| { let r = find(&mut parent, &k); (k, r) }).collect()
}

fn meta_canonical_key(m: &CitationPaperMeta, alias_map: &BTreeMap<PaperKey, PaperKey>) -> Option<PaperKey> {
    let mut keys = meta_keys(m);
    keys.sort_by(canonical_order);
    let first = keys.into_iter().next()?;
    Some(alias_map.get(&first).cloned().unwrap_or(first))
}

fn merge_meta(dst: &mut CitationPaperMeta, src: &CitationPaperMeta) {
    if let Some(ref t) = src.title {
        if dst.title.as_ref().is_none_or(|d| t.len() > d.len()) {
            dst.title = Some(t.clone());
        }
    }
    if dst.authors.is_empty() && !src.authors.is_empty() { dst.authors = src.authors.clone(); }
    if dst.year.is_none() { dst.year = src.year; }
    dst.citation_count = match (dst.citation_count, src.citation_count) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    dst.in_library |= src.in_library;
    dst.owned |= src.owned;
}

#[cfg(test)]
#[path = "idea_net_tests.rs"]
mod tests;
