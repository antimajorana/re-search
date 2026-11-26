//! Deterministic first-query builder per literature service for Explore mode (package X1).
//!
//! Produces the exact query strings shown to the user on the Explore preview/setup screen
//! before anything is sent (`00-overview.md` 2.4, `explore.md` rule P4). Pure code with zero
//! I/O, zero network, and zero model calls.
//!
//! ## Query-syntax alignment and assumptions
//!
//! - **arXiv** (`crates/research-tools/src/arxiv.rs`): `fetch_atom_feed` treats bare arXiv IDs
//!   as `id_list=` lookups, and `build_arxiv_search_expr` preserves inline field prefixes
//!   (`all:`, `ti:`, `au:`, `abs:`, `cat:`, ...), `AND`/`OR`/`ANDNOT`, balanced quotes `"..."`,
//!   and balanced parentheses `(...)`. Every arXiv query built here conforms directly to
//!   `arxiv.rs`.
//! - **INSPIRE-HEP** (`crates/research-tools/src/literature.rs`): `fetch_source` percent-encodes
//!   `q` verbatim into `/api/literature?q={enc_q}&size={max_results}`. **Assumption** beyond
//!   `literature.rs`: INSPIRE-HEP's REST search accepts space-separated terms, double-quoted
//!   phrases `"..."`, `arXiv:<id>` / `doi:<doi>` identifier terms, and `t "..."` title queries.
//! - **Semantic Scholar** (`crates/research-tools/src/literature.rs`): `fetch_source` sends
//!   `query={enc_q}` verbatim to `/graph/v1/paper/search`. **Assumption** beyond `literature.rs`:
//!   S2 keyword search works best with plain unquoted keyword strings without boolean operators
//!   or field prefixes.
//! - **Crossref** (`crates/research-tools/src/literature.rs`): `fetch_source` sends `query={enc_q}`
//!   verbatim to `/works?query={enc_q}&rows={max_results}`. **Assumption** beyond `literature.rs`:
//!   Crossref `/works?query=` expects plain bibliographic/keyword text or bare DOIs without
//!   field prefixes.
//! - **TeX math translation** ([`math_token_to_search_text`]): **Assumption** that stripping LaTeX
//!   styling macros (`\mathcal{N}=4` -> `N=4`, `\mathrm{SU}(N)` -> `SU(N)`, `\mathbb{Z}_2` ->
//!   `Z_2`, `\operatorname{Tr}` -> `Tr`, `\alpha_s` -> `alpha_s`) yields higher recall across
//!   literature APIs than raw TeX markup.

use crate::idea_terms::ExtractedIdea;
use crate::paper_key::PaperKeyKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Maximum length in Unicode scalar values of one generated query (`explore.md` 4.1).
pub const MAX_QUERY_CHARS: usize = 300;

/// Maximum number of user-shown preview queries allowed without a second approval prompt
/// (`explore.md` rule P4: at most 6, arXiv and INSPIRE-HEP only).
pub const MAX_USER_PREVIEW_QUERIES: usize = 6;

/// Supported literature search services matching the string names in `arxiv.rs` / `literature.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryService { Arxiv, Inspire, SemanticScholar, Crossref }

impl QueryService {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arxiv => "arxiv",
            Self::Inspire => "inspire",
            Self::SemanticScholar => "semantic_scholar",
            Self::Crossref => "crossref",
        }
    }
}

/// Origin of a planned query (`explore.md` 4.1 `SearchPlan.origin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOrigin { User, Model, App }

/// One exact, user-visible query planned for execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedQuery {
    pub qid: String,
    pub provider: QueryService,
    pub q: String,
    pub label: String,
    pub origin: QueryOrigin,
}

/// Options controlling which services and caps [`first_queries_for`] uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstQueryOptions {
    pub services: Vec<QueryService>,
    pub max_queries: usize,
    pub categories: Vec<String>,
}

impl Default for FirstQueryOptions {
    fn default() -> Self {
        Self {
            services: vec![QueryService::Arxiv, QueryService::Inspire],
            max_queries: MAX_USER_PREVIEW_QUERIES,
            categories: Vec::new(),
        }
    }
}

/// Builds the default user-shown preview queries (rule P4: at most 6 queries across arXiv
/// and INSPIRE-HEP).
pub fn first_queries(extracted: &ExtractedIdea) -> Vec<PlannedQuery> {
    first_queries_for(extracted, &FirstQueryOptions::default())
}

/// Builds deterministic preview queries for the requested services and cap, interleaving
/// round-robin across services and deduplicating on `(provider, q)`.
pub fn first_queries_for(extracted: &ExtractedIdea, opts: &FirstQueryOptions) -> Vec<PlannedQuery> {
    if opts.max_queries == 0 || opts.services.is_empty() { return Vec::new(); }
    let mut seen_svc = BTreeSet::new();
    let services: Vec<QueryService> = opts.services.iter().copied().filter(|s| seen_svc.insert(*s)).collect();
    let pools: Vec<(QueryService, Vec<(String, String)>)> = services.iter().map(|&svc| {
        let list = match svc {
            QueryService::Arxiv => build_arxiv_queries(extracted, &opts.categories, opts.max_queries),
            QueryService::Inspire => build_inspire_queries(extracted, opts.max_queries),
            QueryService::SemanticScholar => build_semantic_scholar_queries(extracted, opts.max_queries),
            QueryService::Crossref => build_crossref_queries(extracted, opts.max_queries),
        };
        (svc, list)
    }).collect();

    let (mut out, mut seen_pairs) = (Vec::new(), BTreeSet::new());
    let max_pool = pools.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    for slot in 0..max_pool {
        for (svc, list) in &pools {
            if out.len() >= opts.max_queries { return out; }
            if let Some((q, label)) = list.get(slot) {
                let trimmed = q.trim();
                if !trimmed.is_empty() && trimmed.chars().count() <= MAX_QUERY_CHARS
                    && seen_pairs.insert((*svc, trimmed.to_string()))
                {
                    out.push(PlannedQuery {
                        qid: format!("q-{}", out.len() + 1),
                        provider: *svc,
                        q: trimmed.to_string(),
                        label: label.clone(),
                        origin: QueryOrigin::User,
                    });
                }
            }
        }
    }
    out
}

/// Translates a TeX math expression into a search-friendly token string.
///
/// Examples: `\mathcal{N}=4` -> `N=4`, `\mathrm{SU}(N)` -> `SU(N)`,
/// `\mathbb{Z}_2` -> `Z_2`, `\operatorname{Tr}` -> `Tr`, `\alpha_s` -> `alpha_s`.
pub fn math_token_to_search_text(tex: &str) -> String {
    translate_tex_inner(tex).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn translate_tex_inner(input: &str) -> String {
    const UNWRAP: &[&str] = &[
        "mathcal", "mathrm", "mathbb", "mathfrak", "mathbf", "mathsf", "mathtt", "mathit",
        "mathscr", "operatorname", "text", "textrm", "textbf", "textit", "mbox", "hat", "widehat",
        "tilde", "widetilde", "bar", "overline", "underline", "vec", "dot", "ddot", "bm", "boldsymbol",
    ];
    const DROP: &[&str] = &[
        "left", "right", "big", "Big", "bigg", "Bigg", "displaystyle", "textstyle", "scriptstyle",
        "limits", "nolimits",
    ];
    const SPACE: &[&str] = &["quad", "qquad", "enspace", "thinspace"];

    let bytes = input.as_bytes();
    let (mut out, mut i) = (String::with_capacity(input.len()), 0usize);
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if let Some(&next) = bytes.get(i + 1) {
                if !next.is_ascii_alphabetic() {
                    if matches!(next, b',' | b';' | b':' | b' ') { out.push(' '); }
                    else if matches!(next, b'{' | b'}' | b'_' | b'%' | b'&' | b'$' | b'#') { out.push(next as char); }
                    i += 2;
                    continue;
                }
            }
            let mut cmd_end = i + 1;
            while cmd_end < bytes.len() && bytes[cmd_end].is_ascii_alphabetic() { cmd_end += 1; }
            let cmd = &input[i + 1..cmd_end];
            i = cmd_end;

            if matches!(cmd, "frac" | "dfrac" | "tfrac") {
                if let Some((num, a1)) = read_brace_group(input, skip_ws(bytes, i)) {
                    if let Some((den, a2)) = read_brace_group(input, skip_ws(bytes, a1)) {
                        out.push_str(&translate_tex_inner(num));
                        out.push('/');
                        out.push_str(&translate_tex_inner(den));
                        i = a2;
                        continue;
                    }
                }
            } else if UNWRAP.contains(&cmd) {
                let p = skip_ws(bytes, i);
                if let Some((inner, after)) = read_brace_group(input, p) {
                    out.push_str(&translate_tex_inner(inner));
                    i = after;
                } else if let Some(ch) = input[p..].chars().next() {
                    out.push(ch);
                    i = p + ch.len_utf8();
                }
                continue;
            } else if DROP.contains(&cmd) {
                continue;
            } else if SPACE.contains(&cmd) {
                out.push(' ');
                continue;
            } else {
                let repl = match cmd {
                    "sim" | "approx" | "simeq" | "cong" => " ~ ",
                    "to" | "rightarrow" | "longrightarrow" | "mapsto" => " to ",
                    "infty" => "infinity",
                    "times" => "*",
                    "cdot" => " ",
                    _ => {
                        if out.chars().last().is_some_and(|c| c.is_ascii_alphanumeric()) { out.push(' '); }
                        cmd
                    }
                };
                out.push_str(repl);
                continue;
            }
        }
        if matches!(bytes[i], b'{' | b'}') { i += 1; continue; }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn skip_ws(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
    pos
}

fn read_brace_group(input: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = input.as_bytes();
    if bytes.get(start) != Some(&b'{') { return None; }
    let (mut depth, mut i) = (0usize, start);
    while i < bytes.len() {
        if bytes[i] == b'\\' { i = (i + 2).min(bytes.len()); continue; }
        if bytes[i] == b'{' { depth += 1; } else if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 { return Some((&input[start + 1..i], i + 1)); }
        }
        i += 1;
    }
    None
}

struct QueryPool { max_count: usize, out: Vec<(String, String)>, seen: BTreeSet<String> }

impl QueryPool {
    fn new(max_count: usize) -> Self { Self { max_count, out: Vec::new(), seen: BTreeSet::new() } }
    fn push(&mut self, q: String, label: &str) {
        if self.out.len() < self.max_count && !q.is_empty() && q.chars().count() <= MAX_QUERY_CHARS
            && self.seen.insert(q.clone())
        {
            self.out.push((q, label.to_string()));
        }
    }
}

struct QueryTerms {
    phrases: Vec<String>,
    propers: Vec<String>,
    math: Vec<String>,
    bigrams: Vec<String>,
    unigrams: Vec<String>,
}

fn collect_query_terms(extracted: &ExtractedIdea) -> QueryTerms {
    let mut seen = BTreeSet::new();
    let mut push = |target: &mut Vec<String>, raw: &str| {
        let clean = sanitize_plain_term(raw);
        if !clean.is_empty() && seen.insert(clean.to_ascii_lowercase()) { target.push(clean); }
    };
    let (mut phrases, mut propers, mut math, mut bigrams, mut unigrams) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for p in &extracted.quoted_phrases { push(&mut phrases, p); }
    for p in &extracted.proper_names { push(&mut propers, p); }
    for m in &extracted.math_tokens {
        let conv = math_token_to_search_text(m);
        if (2..=48).contains(&conv.chars().count()) && conv.chars().any(|c| c.is_ascii_alphanumeric()) {
            push(&mut math, &conv);
        }
    }
    for k in &extracted.keywords {
        if k.contains(' ') { push(&mut bigrams, k); } else { push(&mut unigrams, k); }
    }
    QueryTerms { phrases, propers, math, bigrams, unigrams }
}

fn sanitize_plain_term(raw: &str) -> String {
    let collapsed = raw.chars()
        .map(|c| if c.is_control() || c == '"' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace().collect::<Vec<_>>().join(" ");
    let (mut depth, mut balanced) = (0i32, true);
    for ch in collapsed.chars() {
        if ch == '(' { depth += 1; } else if ch == ')' { depth -= 1; if depth < 0 { balanced = false; break; } }
    }
    if balanced && depth == 0 { return collapsed; }
    collapsed.chars().filter(|&c| c != '(' && c != ')').collect::<String>()
        .split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pick_terms(sources: &[&[String]], limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for src in sources {
        for item in *src {
            if out.len() >= limit { return out; }
            let low = item.to_ascii_lowercase();
            let subsumed = out.iter().any(|e| {
                let el = e.to_ascii_lowercase();
                el == low || el.split_whitespace().any(|w| w == low)
            });
            if !subsumed { out.push(item.clone()); }
        }
    }
    out
}

fn focused_terms(qt: &QueryTerms, limit: usize) -> Vec<String> {
    pick_terms(&[&qt.phrases, &qt.propers, &qt.math, &qt.bigrams, &qt.unigrams], limit)
}

fn broad_terms(qt: &QueryTerms, limit: usize) -> Vec<String> {
    pick_terms(&[&qt.propers, &qt.unigrams, &qt.math, &qt.phrases], limit)
}

fn arxiv_clause(field: &str, term: &str, max_chars: usize) -> Option<String> {
    let clean = sanitize_plain_term(term);
    if clean.is_empty() { return None; }
    let quoted = clean.contains(' ') || clean.contains(':') || clean.contains('/');
    let overhead = field.len() + 1 + usize::from(quoted) * 2;
    if max_chars <= overhead { return None; }
    let body = truncate_word_boundary(&clean, max_chars - overhead);
    (!body.is_empty()).then(|| if quoted { format!("{field}:\"{body}\"") } else { format!("{field}:{body}") })
}

fn join_arxiv_clauses(clauses: &[String], cat_suffix: Option<&str>) -> Option<String> {
    let budget = MAX_QUERY_CHARS.saturating_sub(cat_suffix.map_or(0, |s| s.chars().count() + 7));
    let (mut kept, mut used) = (Vec::new(), 0usize);
    for c in clauses {
        let add = c.chars().count() + if kept.is_empty() { 0 } else { 5 };
        if used + add > budget { break; }
        used += add;
        kept.push(c.as_str());
    }
    if kept.is_empty() { return None; }
    let body = kept.join(" AND ");
    let full = match cat_suffix {
        Some(cat) if kept.len() > 1 => format!("({body}) AND {cat}"),
        Some(cat) => format!("{body} AND {cat}"),
        None => body,
    };
    (full.chars().count() <= MAX_QUERY_CHARS).then_some(full)
}

fn format_arxiv_cat_suffix(categories: &[String]) -> Option<String> {
    let valid: Vec<String> = categories.iter().filter_map(|c| {
        let t = c.trim().strip_prefix("cat:").unwrap_or(c.trim());
        (!t.is_empty() && t.len() <= 32 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.'))
            .then(|| format!("cat:{t}"))
    }).take(3).collect();
    match valid.len() { 0 => None, 1 => Some(valid[0].clone()), _ => Some(format!("({})", valid.join(" OR "))) }
}

/// Builds arXiv queries conforming to `crates/research-tools/src/arxiv.rs`.
pub fn build_arxiv_queries(extracted: &ExtractedIdea, categories: &[String], max_count: usize) -> Vec<(String, String)> {
    let mut pool = QueryPool::new(max_count);
    for pref in &extracted.paper_refs {
        if pool.out.len() >= 2.min(max_count) { break; }
        if pref.key.kind() == PaperKeyKind::Arxiv { pool.push(pref.key.value().to_string(), "arXiv id lookup"); }
        else if let Some(ax) = &pref.ids.arxiv { pool.push(ax.clone(), "arXiv id lookup"); }
    }
    let qt = collect_query_terms(extracted);
    let cat = format_arxiv_cat_suffix(categories);

    let focused: Vec<String> = focused_terms(&qt, 4).iter().filter_map(|t| arxiv_clause("all", t, 90)).collect();
    if let Some(q) = join_arxiv_clauses(&focused, cat.as_deref()) { pool.push(q, "arXiv focused terms"); }

    let mut title_clauses: Vec<String> = qt.phrases.iter().chain(&qt.bigrams).chain(&qt.propers).take(2)
        .filter_map(|t| arxiv_clause("ti", t, 90)).collect();
    if title_clauses.len() == 1 {
        if let Some(extra) = qt.propers.iter().chain(&qt.unigrams).find(|t| !title_clauses[0].to_ascii_lowercase().contains(&t.to_ascii_lowercase())) {
            if let Some(c) = arxiv_clause("all", extra, 90) { title_clauses.push(c); }
        }
    }
    if let Some(q) = join_arxiv_clauses(&title_clauses, cat.as_deref()) { pool.push(q, "arXiv title match"); }

    let broad: Vec<String> = broad_terms(&qt, 4).iter().filter_map(|t| arxiv_clause("all", t, 90)).collect();
    if let Some(q) = join_arxiv_clauses(&broad, cat.as_deref()) { pool.push(q, "arXiv broader keywords"); }
    pool.out
}

fn quote_if_spaced(term: &str) -> String {
    if term.contains(' ') { format!("\"{term}\"") } else { term.to_string() }
}

fn join_bounded_terms(terms: &[String], max_chars: usize) -> Option<String> {
    let mut out = String::new();
    for tok in terms {
        if tok.is_empty() { continue; }
        let needed = tok.chars().count() + usize::from(!out.is_empty());
        if out.chars().count() + needed > max_chars {
            if out.is_empty() { out.push_str(&truncate_word_boundary(tok, max_chars)); }
            break;
        }
        if !out.is_empty() { out.push(' '); }
        out.push_str(tok);
    }
    (!out.is_empty()).then_some(out)
}

/// Builds INSPIRE-HEP queries for `literature.rs`'s `/api/literature?q=` endpoint.
pub fn build_inspire_queries(extracted: &ExtractedIdea, max_count: usize) -> Vec<(String, String)> {
    let mut pool = QueryPool::new(max_count);
    for pref in &extracted.paper_refs {
        if pool.out.len() >= 2.min(max_count) { break; }
        match pref.key.kind() {
            PaperKeyKind::Arxiv => pool.push(format!("arXiv:{}", pref.key.value()), "INSPIRE id lookup"),
            PaperKeyKind::Doi => pool.push(format!("doi:{}", pref.key.value()), "INSPIRE id lookup"),
            _ => {}
        }
    }
    let qt = collect_query_terms(extracted);
    let focused: Vec<String> = focused_terms(&qt, 5).into_iter().map(|t| quote_if_spaced(&t)).collect();
    if let Some(q) = join_bounded_terms(&focused, MAX_QUERY_CHARS) { pool.push(q, "INSPIRE focused terms"); }
    if let Some(anchor) = qt.phrases.first().or(qt.bigrams.first()).or(qt.propers.first()) {
        let body = truncate_word_boundary(anchor, 120);
        if !body.is_empty() {
            let mut parts = vec![format!("t \"{body}\"")];
            if let Some(second) = qt.propers.iter().chain(&qt.math).chain(&qt.unigrams)
                .find(|t| !body.to_ascii_lowercase().contains(&t.to_ascii_lowercase()))
            {
                parts.push(quote_if_spaced(second));
            }
            if let Some(q) = join_bounded_terms(&parts, MAX_QUERY_CHARS) { pool.push(q, "INSPIRE title match"); }
        }
    }
    let broad: Vec<String> = broad_terms(&qt, 5).into_iter().map(|t| quote_if_spaced(&t)).collect();
    if let Some(q) = join_bounded_terms(&broad, MAX_QUERY_CHARS) { pool.push(q, "INSPIRE broader keywords"); }
    pool.out
}

/// Builds Semantic Scholar plain-text keyword queries for `/graph/v1/paper/search?query=`.
pub fn build_semantic_scholar_queries(extracted: &ExtractedIdea, max_count: usize) -> Vec<(String, String)> {
    let mut pool = QueryPool::new(max_count);
    let qt = collect_query_terms(extracted);
    if let Some(q) = join_bounded_terms(&focused_terms(&qt, 6), MAX_QUERY_CHARS) { pool.push(q, "Semantic Scholar focused terms"); }
    if let Some(q) = join_bounded_terms(&broad_terms(&qt, 6), MAX_QUERY_CHARS) { pool.push(q, "Semantic Scholar broader keywords"); }
    if let Some(pref) = extracted.paper_refs.first() {
        match pref.key.kind() {
            PaperKeyKind::Arxiv => pool.push(format!("arXiv:{}", pref.key.value()), "Semantic Scholar id lookup"),
            PaperKeyKind::Doi => pool.push(pref.key.value().to_string(), "Semantic Scholar id lookup"),
            _ => {}
        }
    }
    pool.out
}

/// Builds Crossref plain bibliographic/keyword queries for `/works?query=`.
pub fn build_crossref_queries(extracted: &ExtractedIdea, max_count: usize) -> Vec<(String, String)> {
    let mut pool = QueryPool::new(max_count);
    if let Some(doi_ref) = extracted.paper_refs.iter().find(|r| r.key.kind() == PaperKeyKind::Doi || r.ids.doi.is_some()) {
        let doi = doi_ref.ids.doi.as_deref().unwrap_or_else(|| doi_ref.key.value());
        pool.push(doi.to_string(), "Crossref DOI lookup");
    }
    let qt = collect_query_terms(extracted);
    if let Some(q) = join_bounded_terms(&focused_terms(&qt, 6), MAX_QUERY_CHARS) { pool.push(q, "Crossref focused terms"); }
    if let Some(q) = join_bounded_terms(&broad_terms(&qt, 6), MAX_QUERY_CHARS) { pool.push(q, "Crossref broader keywords"); }
    pool.out
}

fn truncate_word_boundary(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars { return s.to_string(); }
    let slice: String = s.chars().take(max_chars).collect();
    match slice.rfind(' ') { Some(idx) if idx > 0 => slice[..idx].trim_end().to_string(), _ => slice.trim_end().to_string() }
}

#[cfg(test)]
#[path = "idea_queries_tests.rs"]
mod tests;
