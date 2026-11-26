// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Five-signal duplicate detection across user paper sources (package F13a, `00-overview.md` 2.11).
//!
//! Signals ported from Kandinsky `inbox_dedupe.rs`:
//! 1. Normalized DOI (exact -> merges index rows; `10.48550/arxiv.*` also matched on arXiv id)
//! 2. Base arXiv id without version (exact -> merges index rows)
//! 3. PDF SHA-256 digest (exact -> merges index rows)
//! 4. Normalized ISBN-10/13 core (exact -> merges index rows)
//! 5. Fuzzy title (Jaccard >= 0.88 on folded ASCII tokens) + first author surname + year within 1
//!    (possible -> NEVER merges index rows; marks `possible_duplicate` only)

use crate::library::{LibraryIds, SourceItemRecord, SourceItemState};
use crate::paper_key::{canonical_doi, split_arxiv_id, PaperKey};
use crate::paper_sources::{derive_paper_key, FILE_ROLE_PDF};
use crate::pdf_firstpage::{fold_diacritics, normalize_valid_isbn};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Threshold for fuzzy title token Jaccard similarity (ported from Kandinsky `inbox_dedupe.rs`).
pub const FUZZY_TITLE_JACCARD_MIN: f64 = 0.88;

/// Which signal matched two source items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateSignal {
    Doi,
    Arxiv,
    PdfSha256,
    Isbn,
    TitleAuthorYear,
}

impl DuplicateSignal {
    pub fn is_exact(self) -> bool {
        !matches!(self, Self::TitleAuthorYear)
    }
}

/// Whether a duplicate group merges index rows (`Exact`) or only flags a warning (`Possible`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateConfidence {
    Exact,
    Possible,
}

/// Compact member reference inside a [`DuplicateGroup`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateMember {
    pub source_id: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citekey: Option<String>,
    pub title: String,
    pub item_kind: String,
}

/// One detected group of duplicate items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub id: String,
    pub confidence: DuplicateConfidence,
    pub signals: Vec<DuplicateSignal>,
    /// Canonical `PaperKey` for `Exact` groups (`None` for `Possible` groups or unkeyed exact ISBN groups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_key: Option<PaperKey>,
    /// Secondary `PaperKey`s to alias onto `canonical_key` (only for `Exact` groups).
    #[serde(default)]
    pub alias_keys: Vec<PaperKey>,
    /// Merged identifier union across members of an `Exact` group.
    #[serde(default)]
    pub merged_ids: LibraryIds,
    pub primary_index: usize,
    pub member_indices: Vec<usize>,
    pub members: Vec<DuplicateMember>,
}

/// Complete result of running 5-signal duplicate detection over a slice of [`SourceItemRecord`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DedupeAnalysis {
    pub exact_groups: Vec<DuplicateGroup>,
    pub possible_groups: Vec<DuplicateGroup>,
}

impl DedupeAnalysis {
    pub fn all_groups(&self) -> Vec<DuplicateGroup> {
        let mut all = self.exact_groups.clone();
        all.extend(self.possible_groups.iter().cloned());
        all
    }
}

/// Run Kandinsky's 5-signal duplicate detection without modifying any files or records.
pub fn find_duplicates(items: &[SourceItemRecord]) -> DedupeAnalysis {
    let n = items.len();
    if n < 2 {
        return DedupeAnalysis::default();
    }

    // Step 1: Exact signals (1..4) via union-find.
    let mut exact_uf = UnionFind::new(n);
    let mut exact_edges: Vec<(usize, usize, DuplicateSignal)> = Vec::new();
    let mut by_doi: HashMap<String, usize> = HashMap::new();
    let mut by_arxiv: HashMap<String, usize> = HashMap::new();
    let mut by_sha: HashMap<String, usize> = HashMap::new();
    let mut by_isbn: HashMap<String, usize> = HashMap::new();

    for (i, item) in items.iter().enumerate() {
        if item.state == SourceItemState::Missing {
            continue;
        }
        if let Some(raw_doi) = item.ids.doi.as_deref() {
            if let Ok(norm) = canonical_doi(raw_doi) {
                if let Some(ax_part) = norm.strip_prefix("10.48550/arxiv.") {
                    if let Ok((base, _)) = split_arxiv_id(ax_part) {
                        if let Some(&prev) = by_arxiv.get(&base) {
                            exact_uf.union(prev, i);
                            exact_edges.push((prev, i, DuplicateSignal::Arxiv));
                        } else {
                            by_arxiv.insert(base, i);
                        }
                    }
                }
                if let Some(&prev) = by_doi.get(&norm) {
                    exact_uf.union(prev, i);
                    exact_edges.push((prev, i, DuplicateSignal::Doi));
                } else {
                    by_doi.insert(norm, i);
                }
            }
        }
        if let Some(raw_ax) = item.ids.arxiv.as_deref() {
            if let Ok((base, _)) = split_arxiv_id(raw_ax) {
                if let Some(&prev) = by_arxiv.get(&base) {
                    if prev != i {
                        exact_uf.union(prev, i);
                        exact_edges.push((prev, i, DuplicateSignal::Arxiv));
                    }
                } else {
                    by_arxiv.insert(base, i);
                }
            }
        }
        if let Some(raw_isbn) = item.ids.isbn.as_deref() {
            if let Some(core) = normalize_valid_isbn(raw_isbn) {
                if let Some(&prev) = by_isbn.get(&core) {
                    exact_uf.union(prev, i);
                    exact_edges.push((prev, i, DuplicateSignal::Isbn));
                } else {
                    by_isbn.insert(core, i);
                }
            }
        }
        for f in &item.files {
            if f.role == FILE_ROLE_PDF {
                if let Some(sha) = f.sha256.as_deref().map(str::trim).filter(|s| s.len() == 64) {
                    let norm_sha = sha.to_ascii_lowercase();
                    if let Some(&prev) = by_sha.get(&norm_sha) {
                        if prev != i {
                            exact_uf.union(prev, i);
                            exact_edges.push((prev, i, DuplicateSignal::PdfSha256));
                        }
                    } else {
                        by_sha.insert(norm_sha, i);
                    }
                }
            }
        }
    }

    let exact_groups = build_exact_groups(items, &mut exact_uf, &exact_edges);

    // Step 2: Fuzzy signal (5: title Jaccard >= 0.88 + first author surname + year within 1).
    // Never links items that are already in the same exact duplicate group.
    let mut possible_uf = UnionFind::new(n);
    let mut has_possible = vec![false; n];
    let toks: Vec<BTreeSet<String>> = items.iter().map(|it| title_tokens(&it.title)).collect();
    let surnames: Vec<String> = items.iter().map(|it| first_surname(&it.authors)).collect();

    for i in 0..n {
        if items[i].state == SourceItemState::Missing
            || toks[i].is_empty()
            || surnames[i].is_empty()
        {
            continue;
        }
        for j in (i + 1)..n {
            if items[j].state == SourceItemState::Missing
                || toks[j].is_empty()
                || surnames[j].is_empty()
            {
                continue;
            }
            if exact_uf.find(i) == exact_uf.find(j) {
                continue;
            }
            if surnames[i] != surnames[j] {
                continue;
            }
            if jaccard(&toks[i], &toks[j]) < FUZZY_TITLE_JACCARD_MIN {
                continue;
            }
            let year_ok = match (items[i].year, items[j].year) {
                (Some(yi), Some(yj)) => (yi - yj).abs() <= 1,
                _ => false,
            };
            if year_ok {
                possible_uf.union(i, j);
                has_possible[i] = true;
                has_possible[j] = true;
            }
        }
    }

    let possible_groups = build_possible_groups(items, &mut possible_uf, &has_possible);
    DedupeAnalysis {
        exact_groups,
        possible_groups,
    }
}

fn build_exact_groups(
    items: &[SourceItemRecord],
    uf: &mut UnionFind,
    edges: &[(usize, usize, DuplicateSignal)],
) -> Vec<DuplicateGroup> {
    let mut buckets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..items.len() {
        buckets.entry(uf.find(i)).or_default().push(i);
    }
    let mut out = Vec::new();
    for (_, mut idxs) in buckets.into_iter().filter(|(_, v)| v.len() >= 2) {
        let member_set: BTreeSet<usize> = idxs.iter().copied().collect();
        let signals: Vec<DuplicateSignal> = edges
            .iter()
            .filter(|(a, b, _)| member_set.contains(a) && member_set.contains(b))
            .map(|(_, _, sig)| *sig)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        idxs.sort_by(|&a, &b| {
            primary_score(&items[b])
                .cmp(&primary_score(&items[a]))
                .then_with(|| items[a].source_id.cmp(&items[b].source_id))
                .then_with(|| items[a].external_id.cmp(&items[b].external_id))
        });

        let mut merged_ids = LibraryIds::default();
        let mut first_pdf_sha: Option<String> = None;
        let mut all_keys: BTreeSet<PaperKey> = BTreeSet::new();
        for &idx in &idxs {
            let it = &items[idx];
            // Prefer non-arXiv publisher DOI over `10.48550/arxiv.*` when merging preprint + journal.
            if let Some(d) = it.ids.doi.as_deref() {
                let is_arxiv_doi = d.to_ascii_lowercase().starts_with("10.48550/arxiv.");
                let cur_is_arxiv_doi = merged_ids
                    .doi
                    .as_deref()
                    .is_some_and(|x| x.to_ascii_lowercase().starts_with("10.48550/arxiv."));
                if merged_ids.doi.is_none() || (cur_is_arxiv_doi && !is_arxiv_doi) {
                    merged_ids.doi = Some(d.to_string());
                }
            }
            merged_ids.merge_from(&it.ids);
            if let Some(k) = &it.key {
                all_keys.insert(k.clone());
            }
            for alias_str in it.ids.canonical_aliases() {
                if let Ok(k) = PaperKey::parse(&alias_str) {
                    all_keys.insert(k);
                }
            }
            if first_pdf_sha.is_none() {
                first_pdf_sha = it
                    .files
                    .iter()
                    .find(|f| f.role == FILE_ROLE_PDF)
                    .and_then(|f| f.sha256.clone());
            }
        }

        let canonical_key = derive_paper_key(&merged_ids, first_pdf_sha.as_deref())
            .or_else(|| idxs.iter().find_map(|&i| items[i].key.clone()));
        let alias_keys: Vec<PaperKey> = all_keys
            .into_iter()
            .filter(|k| canonical_key.as_ref() != Some(k))
            .collect();

        let primary_index = idxs[0];
        let id = format!(
            "dup-exact-{}-{}",
            items[primary_index].source_id, items[primary_index].external_id
        );
        let members = idxs.iter().map(|&i| to_member(&items[i])).collect();
        out.push(DuplicateGroup {
            id,
            confidence: DuplicateConfidence::Exact,
            signals,
            canonical_key,
            alias_keys,
            merged_ids,
            primary_index,
            member_indices: idxs,
            members,
        });
    }
    out
}

fn build_possible_groups(
    items: &[SourceItemRecord],
    uf: &mut UnionFind,
    has_possible: &[bool],
) -> Vec<DuplicateGroup> {
    let mut buckets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, &flag) in has_possible.iter().enumerate() {
        if flag {
            buckets.entry(uf.find(i)).or_default().push(i);
        }
    }
    let mut out = Vec::new();
    for (_, mut idxs) in buckets.into_iter().filter(|(_, v)| v.len() >= 2) {
        idxs.sort_by(|&a, &b| {
            primary_score(&items[b])
                .cmp(&primary_score(&items[a]))
                .then_with(|| items[a].source_id.cmp(&items[b].source_id))
                .then_with(|| items[a].external_id.cmp(&items[b].external_id))
        });
        let primary_index = idxs[0];
        let id = format!(
            "dup-possible-{}-{}",
            items[primary_index].source_id, items[primary_index].external_id
        );
        let members = idxs.iter().map(|&i| to_member(&items[i])).collect();
        out.push(DuplicateGroup {
            id,
            confidence: DuplicateConfidence::Possible,
            signals: vec![DuplicateSignal::TitleAuthorYear],
            canonical_key: None,
            alias_keys: Vec::new(),
            merged_ids: LibraryIds::default(),
            primary_index,
            member_indices: idxs,
            members,
        });
    }
    out
}

fn to_member(item: &SourceItemRecord) -> DuplicateMember {
    DuplicateMember {
        source_id: item.source_id.clone(),
        external_id: item.external_id.clone(),
        key: item.key.clone(),
        citekey: item.citekey.clone(),
        title: item.title.clone(),
        item_kind: item.item_kind.clone(),
    }
}

/// Rank duplicate members so journal/book entries with non-arXiv DOIs and attached files
/// win as the primary display entry (ported from Kandinsky `inbox_dedupe.rs`).
fn primary_score(item: &SourceItemRecord) -> i32 {
    let kind_pts = match item.item_kind.as_str() {
        "journalArticle" | "book" | "thesis" | "conferencePaper" => 10,
        "preprint" => 4,
        _ => 1,
    };
    let doi_pts = if item
        .ids
        .doi
        .as_deref()
        .is_some_and(|d| !d.to_ascii_lowercase().starts_with("10.48550/arxiv."))
    {
        6
    } else {
        0
    };
    let cite_pts = if item.citekey.is_some() { 3 } else { 0 };
    let file_pts = (item.files.len().min(3) as i32) * 2;
    kind_pts + doi_pts + cite_pts + file_pts
}

fn norm_ascii_word(s: &str) -> String {
    fold_diacritics(s)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

pub(crate) fn title_tokens(title: &str) -> BTreeSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .map(norm_ascii_word)
        .filter(|w| !w.is_empty())
        .collect()
}

pub(crate) fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

pub(crate) fn first_surname(authors: &[String]) -> String {
    let Some(first) = authors.first() else {
        return String::new();
    };
    let token = if let Some((before_comma, _)) = first.split_once(',') {
        before_comma.trim()
    } else {
        first.split_whitespace().last().unwrap_or("")
    };
    norm_ascii_word(token)
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

#[cfg(test)]
#[path = "source_dedupe_tests.rs"]
mod tests;
