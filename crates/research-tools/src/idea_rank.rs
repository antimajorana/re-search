//! Reciprocal-rank fusion and cross-service deduplication on F2 [`PaperKey`] with
//! transitive alias merging for Explore mode (package X1).
//!
//! Pure, deterministic code with zero I/O, zero network, and zero model calls.
//! Merging and scoring are strictly invariant under permutation of service lists,
//! order-independent across hits, and idempotent.

use crate::paper_key::{split_arxiv_id, PaperIds, PaperKey, PaperKeyKind, PaperRef};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// Standard Cormack-Clarke-Buettcher Reciprocal Rank Fusion smoothing constant (`k = 60`).
///
/// `explore.md` 4.1 specifies RRF over `(query, service)` lists without fixing a numeric `k`;
/// `60.0` is our chosen default (standard in IR rank fusion to dampen top-rank variance).
pub const RRF_K: f64 = 60.0;

/// Score bonus per distinct Explore perspective (`perspective_id`) that retrieved a paper.
///
/// `explore.md` 4.1 specifies "plus number of distinct perspectives" without fixing a numeric
/// weight; `0.02` is our chosen default (~1.2x a single rank-1 RRF contribution `1/61 ≈ 0.0164`,
/// rewarding cross-perspective corroboration without overriding strong rank agreement).
pub const PERSPECTIVE_BONUS: f64 = 0.02;

/// Weight applied to `ln(citations + 1)` in the candidate score.
///
/// `explore.md` 4.1 specifies "plus a small log(citations) term" without fixing a numeric
/// weight; `0.005` is our chosen default (`100` citations adds `~0.023`, `10_000` adds `~0.046`,
/// acting as a gentle prior/tie-breaker without burying recent zero-citation preprints).
pub const CITATION_LOG_WEIGHT: f64 = 0.005;

/// Maximum number of ranked candidates retained per search round (`explore.md` 4.1).
pub const MAX_CANDIDATES_PER_ROUND: usize = 200;

/// Maximum number of authors kept on a merged candidate (`explore.md` 4.1: `authors[<=8]`).
pub const MAX_AUTHORS: usize = 8;

/// Maximum abstract length in Unicode scalar values (`explore.md` 4.1: `<= 3,000`).
pub const MAX_ABSTRACT_CHARS: usize = 3_000;

/// Provenance of a candidate's selected abstract (`explore.md` 4.1 `Candidate.abstract_ref`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AbstractRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
    pub provider: String,
    /// `false` for abstracts rebuilt from an inverted index (e.g. OpenAlex); never evidence.
    pub verbatim: bool,
}

/// One raw paper hit returned by a service list prior to alias merging and fusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawHit {
    pub paper: PaperRef,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default)]
    pub abstract_verbatim: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_raw_blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u64>,
    #[serde(default)]
    pub withdrawn_or_retracted: bool,
    #[serde(default)]
    pub in_library: bool,
    #[serde(default)]
    pub owned: bool,
    #[serde(default)]
    pub user_seed: bool,
    #[serde(default)]
    pub cited_by_origin: bool,
}

/// One ordered list of hits produced by executing query `qid` on `provider`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedServiceList {
    pub qid: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective_id: Option<String>,
    pub hits: Vec<RawHit>,
}

/// Merged, deduplicated, and RRF-scored Explore candidate (`explore.md` 4.1 `Candidate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedCandidate {
    pub key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub ids: PaperIds,
    pub title: String,
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_ref: Option<AbstractRef>,
    pub found_by: Vec<String>,
    pub perspectives: Vec<String>,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u64>,
    pub withdrawn_or_retracted: bool,
    pub in_library: bool,
    pub owned: bool,
    pub user_seed: bool,
    pub cited_by_origin: bool,
}

impl RankedCandidate {
    /// Builds an F2 [`PaperRef`] carrying this candidate's canonical key, version, and aliases.
    pub fn paper_ref(&self) -> PaperRef {
        PaperRef {
            key: self.key.clone(),
            version: self.version.clone(),
            ids: self.ids.clone(),
        }
    }

    /// Converts this candidate back into a [`RawHit`] (useful for multi-round merging and
    /// idempotence checks).
    pub fn to_raw_hit(&self) -> RawHit {
        RawHit {
            paper: self.paper_ref(),
            title: self.title.clone(),
            authors: self.authors.clone(),
            year: self.year,
            abstract_text: self.abstract_text.clone(),
            abstract_verbatim: self.abstract_ref.as_ref().is_some_and(|r| r.verbatim),
            abstract_raw_blob: self.abstract_ref.as_ref().and_then(|r| r.raw_blob.clone()),
            citation_count: self.citation_count,
            withdrawn_or_retracted: self.withdrawn_or_retracted,
            in_library: self.in_library,
            owned: self.owned,
            user_seed: self.user_seed,
            cited_by_origin: self.cited_by_origin,
        }
    }
}

/// Merges alias-equivalent hits across all service lists and ranks them by RRF + perspectives
/// + `log(citations + 1)`, capped at [`MAX_CANDIDATES_PER_ROUND`] (200).
pub fn merge_and_rank(lists: &[RankedServiceList]) -> Vec<RankedCandidate> {
    merge_and_rank_with_cap(lists, Some(MAX_CANDIDATES_PER_ROUND))
}

/// Same as [`merge_and_rank`] with an explicit optional candidate cap (`None` = uncapped).
pub fn merge_and_rank_with_cap(
    lists: &[RankedServiceList],
    cap: Option<usize>,
) -> Vec<RankedCandidate> {
    struct Entry<'a> {
        list_idx: usize,
        rank_1_based: usize,
        hit: &'a RawHit,
        keys: Vec<PaperKey>,
    }

    let mut entries: Vec<Entry<'_>> = Vec::new();
    for (list_idx, list) in lists.iter().enumerate() {
        for (pos, hit) in list.hits.iter().enumerate() {
            entries.push(Entry {
                list_idx,
                rank_1_based: pos + 1,
                hit,
                keys: collect_canonical_keys(&hit.paper),
            });
        }
    }
    if entries.is_empty() {
        return Vec::new();
    }

    let mut uf = DisjointSet::new(entries.len());
    let mut key_owner: BTreeMap<PaperKey, usize> = BTreeMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        for k in &entry.keys {
            if let Some(&prev) = key_owner.get(k) {
                uf.union(prev, idx);
            } else {
                key_owner.insert(k.clone(), idx);
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for idx in 0..entries.len() {
        let root = uf.find(idx);
        groups.entry(root).or_default().push(idx);
    }

    let mut candidates: Vec<RankedCandidate> = Vec::with_capacity(groups.len());
    for member_indices in groups.into_values() {
        let mut all_keys: BTreeSet<PaperKey> = BTreeSet::new();
        let mut arxiv_vals: BTreeSet<String> = BTreeSet::new();
        let mut doi_vals: BTreeSet<String> = BTreeSet::new();
        let mut inspire_vals: BTreeSet<String> = BTreeSet::new();
        let mut s2_vals: BTreeSet<String> = BTreeSet::new();
        let mut versions: Vec<String> = Vec::new();

        let mut found_by_set: BTreeSet<String> = BTreeSet::new();
        let mut perspective_set: BTreeSet<String> = BTreeSet::new();
        let mut best_rank_per_list: BTreeMap<usize, usize> = BTreeMap::new();

        let mut best_title: Option<String> = None;
        let mut best_authors: Vec<String> = Vec::new();
        let mut best_year: Option<i32> = None;
        let mut best_abstract: Option<(String, AbstractRef)> = None;
        let mut max_citations: Option<u64> = None;

        let mut withdrawn_or_retracted = false;
        let mut in_library = false;
        let mut owned = false;
        let mut user_seed = false;
        let mut cited_by_origin = false;

        for idx in member_indices {
            let entry = &entries[idx];
            let list = &lists[entry.list_idx];
            let hit = entry.hit;

            for k in &entry.keys {
                all_keys.insert(k.clone());
                match k.kind() {
                    PaperKeyKind::Arxiv => {
                        arxiv_vals.insert(k.value().to_string());
                    }
                    PaperKeyKind::Doi => {
                        doi_vals.insert(k.value().to_string());
                    }
                    PaperKeyKind::Inspire => {
                        inspire_vals.insert(k.value().to_string());
                    }
                    PaperKeyKind::S2 => {
                        s2_vals.insert(k.value().to_string());
                    }
                    PaperKeyKind::Openalex | PaperKeyKind::File => {}
                }
            }
            if let Some(v) = hit.paper.version.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                versions.push(v.to_string());
            }
            if let Some(raw_ax) = hit.paper.ids.arxiv.as_deref() {
                if let Ok((_, Some(v))) = split_arxiv_id(raw_ax) {
                    versions.push(v);
                }
            }

            let qid = list.qid.trim();
            if !qid.is_empty() {
                found_by_set.insert(qid.to_string());
            }
            if let Some(pid) = list.perspective_id.as_deref().map(str::trim).filter(|s| !s.is_empty())
            {
                perspective_set.insert(pid.to_string());
            }

            best_rank_per_list
                .entry(entry.list_idx)
                .and_modify(|r| {
                    if entry.rank_1_based < *r {
                        *r = entry.rank_1_based;
                    }
                })
                .or_insert(entry.rank_1_based);

            let trimmed_title = hit.title.trim();
            if !trimmed_title.is_empty() {
                let cand_title = trimmed_title.to_string();
                if best_title
                    .as_deref()
                    .is_none_or(|cur| prefer_title(&cand_title, cur) == Ordering::Greater)
                {
                    best_title = Some(cand_title);
                }
            }

            let clean_authors: Vec<String> = hit
                .authors
                .iter()
                .map(|a| a.trim())
                .filter(|a| !a.is_empty())
                .take(MAX_AUTHORS)
                .map(str::to_string)
                .collect();
            if prefer_authors(&clean_authors, &best_authors) == Ordering::Greater {
                best_authors = clean_authors;
            }

            if let Some(y) = hit.year {
                best_year = Some(best_year.map_or(y, |cur| cur.min(y)));
            }

            if let Some(abs_raw) = hit.abstract_text.as_deref().map(str::trim).filter(|s| !s.is_empty())
            {
                let abs_text = truncate_chars(abs_raw, MAX_ABSTRACT_CHARS).to_string();
                let abs_ref = AbstractRef {
                    raw_blob: hit
                        .abstract_raw_blob
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    provider: list.provider.trim().to_string(),
                    verbatim: hit.abstract_verbatim,
                };
                if best_abstract.as_ref().is_none_or(|(cur_text, cur_ref)| {
                    prefer_abstract((&abs_text, &abs_ref), (cur_text, cur_ref)) == Ordering::Greater
                }) {
                    best_abstract = Some((abs_text, abs_ref));
                }
            }

            if let Some(c) = hit.citation_count {
                max_citations = Some(max_citations.map_or(c, |cur| cur.max(c)));
            }

            withdrawn_or_retracted |= hit.withdrawn_or_retracted;
            in_library |= hit.in_library;
            owned |= hit.owned;
            user_seed |= hit.user_seed;
            cited_by_origin |= hit.cited_by_origin;
        }

        // Canonical primary key follows F2's explicit kind preference order, then lex order.
        let key = all_keys
            .into_iter()
            .min_by(|a, b| a.kind().cmp(&b.kind()).then_with(|| a.as_str().cmp(b.as_str())))
            .expect("component always has at least one hit with a PaperKey");

        let ids = PaperIds {
            arxiv: arxiv_vals.into_iter().next(),
            doi: doi_vals.into_iter().next(),
            inspire: inspire_vals.into_iter().next(),
            s2: s2_vals.into_iter().next(),
        };
        let version = select_best_version(versions);

        let found_by: Vec<String> = found_by_set.into_iter().collect();
        let perspectives: Vec<String> = perspective_set.into_iter().collect();

        // Sort contributions before summing so floating-point addition order is strictly
        // invariant under any permutation of input `lists`.
        let mut contribs: Vec<(&str, &str, usize)> = best_rank_per_list
            .into_iter()
            .map(|(list_idx, rank)| {
                let list = &lists[list_idx];
                (list.qid.as_str(), list.provider.as_str(), rank)
            })
            .collect();
        contribs.sort_unstable();

        let rrf_score: f64 = contribs
            .iter()
            .map(|&(_, _, rank)| 1.0 / (RRF_K + rank as f64))
            .sum();
        let perspective_score = (perspectives.len() as f64) * PERSPECTIVE_BONUS;
        let citation_score = ((max_citations.unwrap_or(0) as f64) + 1.0).ln() * CITATION_LOG_WEIGHT;
        let score = rrf_score + perspective_score + citation_score;

        let (abstract_text, abstract_ref) = match best_abstract {
            Some((t, r)) => (Some(t), Some(r)),
            None => (None, None),
        };

        candidates.push(RankedCandidate {
            key,
            version,
            ids,
            title: best_title.unwrap_or_default(),
            authors: best_authors,
            year: best_year,
            abstract_text,
            abstract_ref,
            found_by,
            perspectives,
            score,
            citation_count: max_citations,
            withdrawn_or_retracted,
            in_library,
            owned,
            user_seed,
            cited_by_origin,
        });
    }

    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.key.as_str().cmp(b.key.as_str()))
    });
    if let Some(limit) = cap {
        candidates.truncate(limit);
    }
    candidates
}

/// Collects all canonical F2 [`PaperKey`]s carried by `paper` (primary key plus valid aliases).
pub fn collect_canonical_keys(paper: &PaperRef) -> Vec<PaperKey> {
    let mut keys = BTreeSet::new();
    keys.insert(paper.key.clone());
    if let Some(raw) = paper.ids.arxiv.as_deref() {
        if let Ok(k) = PaperKey::from_arxiv(raw) {
            keys.insert(k);
        }
    }
    if let Some(raw) = paper.ids.doi.as_deref() {
        if let Ok(k) = PaperKey::from_doi(raw) {
            keys.insert(k);
        }
    }
    if let Some(raw) = paper.ids.inspire.as_deref() {
        if let Ok(k) = PaperKey::from_inspire(raw) {
            keys.insert(k);
        }
    }
    if let Some(raw) = paper.ids.s2.as_deref() {
        if let Ok(k) = PaperKey::from_s2(raw) {
            keys.insert(k);
        }
    }
    keys.into_iter().collect()
}

fn select_best_version(versions: Vec<String>) -> Option<String> {
    versions.into_iter().max_by(|a, b| {
        let parse_v = |s: &str| -> Option<u64> { s.strip_prefix('v')?.parse::<u64>().ok() };
        match (parse_v(a), parse_v(b)) {
            (Some(na), Some(nb)) => na.cmp(&nb).then_with(|| a.cmp(b)),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => a.cmp(b),
        }
    })
}

fn prefer_title(cand: &str, cur: &str) -> Ordering {
    cand.chars()
        .count()
        .cmp(&cur.chars().count())
        .then_with(|| cur.cmp(cand))
}

fn prefer_authors(cand: &[String], cur: &[String]) -> Ordering {
    let cand_chars: usize = cand.iter().map(|s| s.chars().count()).sum();
    let cur_chars: usize = cur.iter().map(|s| s.chars().count()).sum();
    cand.len()
        .cmp(&cur.len())
        .then_with(|| cand_chars.cmp(&cur_chars))
        .then_with(|| cur.cmp(cand))
}

fn prefer_abstract(cand: (&str, &AbstractRef), cur: (&str, &AbstractRef)) -> Ordering {
    cand.1
        .verbatim
        .cmp(&cur.1.verbatim)
        .then_with(|| cand.1.raw_blob.is_some().cmp(&cur.1.raw_blob.is_some()))
        .then_with(|| cand.0.chars().count().cmp(&cur.0.chars().count()))
        .then_with(|| cur.0.cmp(cand.0))
        .then_with(|| cur.1.provider.cmp(&cand.1.provider))
        .then_with(|| cur.1.raw_blob.cmp(&cand.1.raw_blob))
}

fn truncate_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

struct DisjointSet {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect(), rank: vec![0; n] }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            let grand = self.parent[self.parent[x]];
            self.parent[x] = grand;
            x = grand;
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            Ordering::Less => self.parent[ra] = rb,
            Ordering::Greater => self.parent[rb] = ra,
            Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] = self.rank[ra].saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
#[path = "idea_rank_tests.rs"]
mod tests;
