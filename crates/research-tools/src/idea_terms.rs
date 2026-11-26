//! TeX-aware term and paper-identifier extraction for Explore mode (package X1).
//!
//! Pure, deterministic, order-stable code with zero I/O, zero network, and zero model calls.
//! All extracted paper identifiers are canonicalised exclusively through F2's [`PaperKey`]
//! and [`PaperRef`] constructors (`crate::paper_key`).

use crate::paper_key::{split_arxiv_id, PaperIds, PaperKey, PaperRef};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// Maximum number of Unicode scalar values accepted from raw idea text (`explore.md` 4.1).
pub const MAX_IDEA_TEXT_CHARS: usize = 4_000;

const MAX_INLINE_MATH_BYTES: usize = 240;
const MAX_QUOTED_PHRASE_BYTES: usize = 160;
const MAX_BIGRAMS: usize = 12;

/// Classification of one extracted term from raw idea text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermKind {
    QuotedPhrase,
    MathToken,
    ProperName,
    Keyword,
}

/// One extracted term paired with its kind, ordered by first appearance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedTerm {
    pub text: String,
    pub kind: TermKind,
}

/// Deterministic extraction output from a user's idea text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedIdea {
    pub paper_refs: Vec<PaperRef>,
    pub terms: Vec<ExtractedTerm>,
    pub quoted_phrases: Vec<String>,
    pub proper_names: Vec<String>,
    pub math_tokens: Vec<String>,
    pub keywords: Vec<String>,
}

/// Extracts canonical F2 paper references and structured TeX-aware terms from `raw`.
///
/// Inputs longer than [`MAX_IDEA_TEXT_CHARS`] are cleanly truncated on a UTF-8 char boundary.
/// Output order is deterministic and follows first-seen order in the source text.
pub fn extract(raw: &str) -> ExtractedIdea {
    let bounded = raw.char_indices().nth(MAX_IDEA_TEXT_CHARS).map_or(raw, |(i, _)| &raw[..i]);
    let no_comments = strip_comments_preserving_urls(bounded);
    let (paper_refs, masked) = extract_paper_refs_and_mask(&no_comments);
    let unwrapped = unwrap_formatting_macros(&masked);

    let mut acc = TermAccumulator::default();
    scan_terms(&unwrapped, &mut acc);
    ExtractedIdea {
        paper_refs,
        terms: acc.terms,
        quoted_phrases: acc.quoted_phrases,
        proper_names: acc.proper_names,
        math_tokens: acc.math_tokens,
        keywords: acc.keywords,
    }
}

fn preceding_backslashes_even(bytes: &[u8], pos: usize) -> bool {
    let (mut count, mut k) = (0usize, pos);
    while k > 0 && bytes[k - 1] == b'\\' {
        count += 1;
        k -= 1;
    }
    count.is_multiple_of(2)
}

fn is_url_percent_escape(line: &str, pos: usize) -> bool {
    let b = line.as_bytes();
    if pos + 2 >= b.len() || !b[pos + 1].is_ascii_hexdigit() || !b[pos + 2].is_ascii_hexdigit() {
        return false;
    }
    let start = line[..pos]
        .rfind(|c: char| c.is_whitespace() || matches!(c, '(' | '[' | '{' | '<' | '"' | '\''))
        .map_or(0, |i| i + 1);
    let head = &line[start..pos];
    head.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        || head.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

fn strip_comments_line(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1..].starts_with(b"verb") && preceding_backslashes_even(bytes, i) {
            let mut j = i + 5 + usize::from(bytes.get(i + 5) == Some(&b'*'));
            if j < bytes.len() && !bytes[j].is_ascii_alphabetic() && !bytes[j].is_ascii_whitespace() {
                let delim = bytes[j];
                j += 1;
                while j < bytes.len() && bytes[j] != delim { j += 1; }
                i = (j + 1).min(bytes.len());
                continue;
            }
        }
        if bytes[i] == b'%' && preceding_backslashes_even(bytes, i) && !is_url_percent_escape(line, i) {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn strip_comments_preserving_urls(text: &str) -> String {
    text.split('\n')
        .map(|l| strip_comments_line(l.strip_suffix('\r').unwrap_or(l)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdCandKind { Arxiv, Doi }

struct IdMatch<'a> { start: usize, end: usize, kind: IdCandKind, raw: &'a str }

fn trim_unmatched_closers<'a>(mut s: &'a str, trim_chars: &[char]) -> &'a str {
    loop {
        let before = s;
        s = s.trim_end_matches(trim_chars);
        for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
            if s.ends_with(close)
                && s.bytes().filter(|&b| b == close as u8).count()
                    > s.bytes().filter(|&b| b == open as u8).count()
            {
                s = &s[..s.len() - 1];
            }
        }
        if s == before { return s; }
    }
}

fn extract_paper_refs_and_mask(text: &str) -> (Vec<PaperRef>, String) {
    static RES: OnceLock<[Regex; 7]> = OnceLock::new();
    let [ax_url, doi_url, ax_pref, doi_pref, bare_doi, bare_ax_new, bare_ax_old] = RES.get_or_init(|| [
        Regex::new(r#"(?i)https?://(?:export\.)?arxiv\.org/(?:abs|pdf|e-print)/[^\s<>"']+"#).unwrap(),
        Regex::new(r#"(?i)https?://(?:dx\.)?doi\.org/10\.\d{4,9}(?:/|%2f)[^\s<>"']+"#).unwrap(),
        Regex::new(r"(?i)\barxiv\s*:\s*(?:\d{4}\.\d{4,5}(?:v[1-9]\d*)?|[a-z-]+(?:\.[a-z-]+)?/\d{7}(?:v[1-9]\d*)?)").unwrap(),
        Regex::new(r#"(?i)\bdoi\s*:\s*10\.\d{4,9}/[^\s,\{\}"'<>]+"#).unwrap(),
        Regex::new(r#"(?i)10\.\d{4,9}/[^\s,\{\}"'<>]+"#).unwrap(),
        Regex::new(r"\d{4}\.\d{4,5}(?:[vV][1-9]\d*)?").unwrap(),
        Regex::new(r"[A-Za-z-]+(?:\.[A-Za-z-]+)?/\d{7}(?:[vV][1-9]\d*)?").unwrap(),
    ]);

    const ID_TRIM: &[char] = &['.', ',', ';', ':', '!', '?', '\'', '"', '\\'];
    let bytes = text.as_bytes();
    let mut cands: Vec<IdMatch<'_>> = Vec::new();
    macro_rules! push_id {
        ($start:expr, $raw:expr, $kind:expr, $trim:expr) => {{
            let s = if $trim { trim_unmatched_closers($raw, ID_TRIM) } else { $raw };
            if !s.is_empty() {
                cands.push(IdMatch { start: $start, end: $start + s.len(), kind: $kind, raw: s });
            }
        }};
    }

    for m in ax_url.find_iter(text) { push_id!(m.start(), m.as_str(), IdCandKind::Arxiv, true); }
    for m in doi_url.find_iter(text) { push_id!(m.start(), m.as_str(), IdCandKind::Doi, true); }
    for m in ax_pref.find_iter(text) { push_id!(m.start(), m.as_str(), IdCandKind::Arxiv, false); }
    for m in doi_pref.find_iter(text) { push_id!(m.start(), m.as_str(), IdCandKind::Doi, true); }
    for m in bare_doi.find_iter(text) {
        if m.start() == 0 || !matches!(bytes[m.start() - 1], b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'/') {
            push_id!(m.start(), m.as_str(), IdCandKind::Doi, true);
        }
    }
    for m in bare_ax_new.find_iter(text) {
        let prev_bad = m.start() > 0 && matches!(bytes[m.start() - 1], b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'.' | b'/');
        let next_bad = m.end() < bytes.len() && (bytes[m.end()].is_ascii_alphanumeric()
            || (bytes[m.end()] == b'.' && bytes.get(m.end() + 1).is_some_and(u8::is_ascii_digit)));
        if !prev_bad && !next_bad { push_id!(m.start(), m.as_str(), IdCandKind::Arxiv, false); }
    }
    for m in bare_ax_old.find_iter(text) {
        let prev_bad = m.start() > 0 && matches!(bytes[m.start() - 1], b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'.' | b'/' | b'-');
        if !prev_bad && !(m.end() < bytes.len() && bytes[m.end()].is_ascii_alphanumeric()) {
            push_id!(m.start(), m.as_str(), IdCandKind::Arxiv, false);
        }
    }

    cands.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));
    let (mut refs, mut ref_idx, mut spans, mut cursor) = (Vec::<PaperRef>::new(), BTreeMap::<PaperKey, usize>::new(), Vec::new(), 0usize);
    for cand in cands {
        if cand.start < cursor { continue; }
        let parsed = match cand.kind {
            IdCandKind::Arxiv => split_arxiv_id(cand.raw).ok().and_then(|(base, version)| {
                let key = PaperKey::from_arxiv(&base).ok()?;
                PaperRef::from_ids(PaperIds { arxiv: Some(key.value().into()), ..Default::default() }, version).ok()
            }),
            IdCandKind::Doi => PaperKey::from_doi(cand.raw).ok().and_then(|key| {
                PaperRef::from_ids(PaperIds { doi: Some(key.value().into()), ..Default::default() }, None).ok()
            }),
        };
        let Some(paper_ref) = parsed else { continue };
        cursor = cand.end;
        spans.push((cand.start, cand.end));
        if let Some(&idx) = ref_idx.get(&paper_ref.key) {
            if refs[idx].version.is_none() && paper_ref.version.is_some() { refs[idx].version = paper_ref.version; }
        } else {
            ref_idx.insert(paper_ref.key.clone(), refs.len());
            refs.push(paper_ref);
        }
    }

    let (mut masked, mut pos) = (String::with_capacity(text.len()), 0usize);
    for (s, e) in spans {
        masked.push_str(&text[pos..s]);
        masked.push(' ');
        pos = e;
    }
    masked.push_str(&text[pos..]);
    (refs, masked)
}

const FORMATTING_WRAPPERS: &[&str] = &[
    "emph", "textbf", "textit", "textsl", "texttt", "textsc", "textrm", "textsf", "underline", "text", "mbox",
];
const IGNORED_PROSE_COMMANDS: &[&str] = &[
    "cite", "citep", "citet", "citeauthor", "citeyear", "citealt", "citealp", "parencite",
    "textcite", "autocite", "ref", "eqref", "autoref", "cref", "Cref", "pageref", "label",
    "hspace", "vspace", "footnote",
];
const IGNORED_ZERO_ARG_COMMANDS: &[&str] = &["noindent", "newline", "par", "medskip", "bigskip", "smallskip"];

fn skip_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if bytes.get(start) != Some(&open) { return None; }
    let (mut depth, mut i) = (0usize, start);
    while i < bytes.len() {
        if bytes[i] == b'\\' { i = (i + 2).min(bytes.len()); continue; }
        if bytes[i] == open { depth += 1; } else if bytes[i] == close {
            depth -= 1;
            if depth == 0 { return Some(i + 1); }
        }
        i += 1;
    }
    None
}

fn skip_ws_bytes(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
    pos
}

fn find_math_delim_end(text: &str, start: usize) -> Option<(usize, usize, usize)> {
    let rest = &text[start..];
    for (open, close) in [("$$", "$$"), ("\\[", "\\]"), ("\\(", "\\)")] {
        if let Some(after) = rest.strip_prefix(open) {
            let rel = after.find(close)?;
            return Some((start + open.len(), start + open.len() + rel, start + open.len() + rel + close.len()));
        }
    }
    if rest.starts_with('$') {
        let bytes = text.as_bytes();
        let limit = (start + 1 + MAX_INLINE_MATH_BYTES).min(bytes.len());
        for i in (start + 1)..limit {
            if bytes[i] == b'\n' && bytes.get(i + 1) == Some(&b'\n') { return None; }
            if bytes[i] == b'$' && preceding_backslashes_even(bytes, i) { return Some((start + 1, i, i + 1)); }
        }
        return None;
    }
    if let Some(after_begin) = rest.strip_prefix("\\begin{") {
        let close = after_begin.find('}')?;
        let env = &after_begin[..close];
        if matches!(env, "equation" | "equation*" | "align" | "align*" | "gather" | "gather*" | "multline" | "multline*" | "eqnarray" | "eqnarray*") {
            let end_tag = format!("\\end{{{env}}}");
            let inner_s = start + "\\begin{".len() + close + 1;
            let rel = text[inner_s..].find(&end_tag)?;
            return Some((inner_s, inner_s + rel, inner_s + rel + end_tag.len()));
        }
    }
    None
}

fn unwrap_formatting_macros(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (String::with_capacity(text.len()), 0usize);
    while i < bytes.len() {
        if (bytes[i] == b'$' || bytes[i] == b'\\') && preceding_backslashes_even(bytes, i) {
            if let Some((_, _, full_end)) = find_math_delim_end(text, i) {
                out.push_str(&text[i..full_end]);
                i = full_end;
                continue;
            }
        }
        if bytes[i] == b'\\' && preceding_backslashes_even(bytes, i) {
            let mut cmd_end = i + 1;
            while cmd_end < bytes.len() && bytes[cmd_end].is_ascii_alphabetic() { cmd_end += 1; }
            let cmd = &text[i + 1..cmd_end];
            if FORMATTING_WRAPPERS.contains(&cmd) {
                let brace = skip_ws_bytes(bytes, cmd_end);
                if let Some(after) = skip_balanced(bytes, brace, b'{', b'}') {
                    out.push_str(&unwrap_formatting_macros(&text[brace + 1..after - 1]));
                    i = after;
                    continue;
                }
            } else if IGNORED_PROSE_COMMANDS.contains(&cmd) {
                let mut pos = cmd_end + usize::from(bytes.get(cmd_end) == Some(&b'*'));
                for _ in 0..2 {
                    if let Some(after) = skip_balanced(bytes, skip_ws_bytes(bytes, pos), b'[', b']') { pos = after; }
                }
                if let Some(after) = skip_balanced(bytes, skip_ws_bytes(bytes, pos), b'{', b'}') {
                    out.push(' ');
                    i = after;
                    continue;
                }
            } else if IGNORED_ZERO_ARG_COMMANDS.contains(&cmd) {
                out.push(' ');
                i = cmd_end;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[derive(Default)]
struct TermAccumulator {
    terms: Vec<ExtractedTerm>,
    quoted_phrases: Vec<String>,
    proper_names: Vec<String>,
    math_tokens: Vec<String>,
    keywords: Vec<String>,
    seen_prose: BTreeSet<String>,
    seen_math: BTreeSet<String>,
    bigrams_emitted: usize,
}

impl TermAccumulator {
    fn push(&mut self, text: String, kind: TermKind) {
        let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() { return; }
        let is_new = match kind {
            TermKind::MathToken => self.seen_math.insert(cleaned.clone()),
            _ => self.seen_prose.insert(cleaned.to_ascii_lowercase()),
        };
        if !is_new { return; }
        self.terms.push(ExtractedTerm { text: cleaned.clone(), kind });
        match kind {
            TermKind::QuotedPhrase => self.quoted_phrases.push(cleaned),
            TermKind::MathToken => self.math_tokens.push(cleaned),
            TermKind::ProperName => self.proper_names.push(cleaned),
            TermKind::Keyword => self.keywords.push(cleaned),
        }
    }
}

fn scan_standalone_tex_expr(text: &str, start: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'\\') { return None; }
    let mut pos = start + 1;
    while pos < bytes.len() && bytes[pos].is_ascii_alphabetic() { pos += 1; }
    if pos == start + 1 { return None; }
    loop {
        let before = pos;
        if let Some(after) = skip_balanced(bytes, pos, b'{', b'}').or_else(|| skip_balanced(bytes, pos, b'[', b']')) {
            pos = after;
        } else if let Some(after) = skip_balanced(bytes, pos, b'(', b')') {
            let inner = &text[pos + 1..after - 1];
            if !inner.is_empty() && inner.len() <= 24 && !inner.contains(' ') { pos = after; }
        } else if matches!(bytes.get(pos), Some(b'_' | b'^')) {
            if let Some(after) = skip_balanced(bytes, pos + 1, b'{', b'}') {
                pos = after;
            } else if bytes.get(pos + 1) == Some(&b'\\') {
                let mut k = pos + 2;
                while k < bytes.len() && bytes[k].is_ascii_alphabetic() { k += 1; }
                if k > pos + 2 { pos = k; }
            } else if bytes.get(pos + 1).is_some_and(u8::is_ascii_alphanumeric) {
                pos += 2;
            }
        } else if matches!(bytes.get(pos), Some(b'=' | b'+' | b'-' | b'/' | b'*'))
            && bytes.get(pos + 1).is_some_and(|b| !b.is_ascii_whitespace())
        {
            pos += 1;
            let is_cmd = bytes.get(pos) == Some(&b'\\');
            pos += usize::from(is_cmd);
            while pos < bytes.len() && if is_cmd { bytes[pos].is_ascii_alphabetic() } else { bytes[pos].is_ascii_alphanumeric() } {
                pos += 1;
            }
        }
        if pos == before { break; }
    }
    let slice = text[start..pos].trim_end_matches(['.', ',', ';', ':', '!', '?']);
    (slice.len() > 1).then(|| (slice.to_string(), start + slice.len()))
}

fn find_quoted_phrase(text: &str, start: usize) -> Option<(String, usize)> {
    let rest = &text[start..];
    for (open, close) in [("``", "''"), ("\u{201c}", "\u{201d}"), ("\"", "\"")] {
        if let Some(after) = rest.strip_prefix(open) {
            let rel = after.find(close)?;
            return (rel > 0 && rel <= MAX_QUOTED_PHRASE_BYTES && !after[..rel].contains("\n\n"))
                .then(|| (after[..rel].to_string(), start + open.len() + rel + close.len()));
        }
    }
    None
}

fn scan_terms(text: &str, acc: &mut TermAccumulator) {
    let bytes = text.as_bytes();
    let (mut i, mut prose_start, mut at_sentence_start) = (0usize, 0usize, true);
    while i < bytes.len() {
        let hit = if (bytes[i] == b'$' || bytes[i] == b'\\') && preceding_backslashes_even(bytes, i) {
            find_math_delim_end(text, i)
                .map(|(s, e, end)| (text[s..e].to_string(), TermKind::MathToken, end))
                .or_else(|| (bytes[i] == b'\\').then(|| scan_standalone_tex_expr(text, i)).flatten()
                    .map(|(t, end)| (t, TermKind::MathToken, end)))
        } else if bytes[i] == b'"' || text[i..].starts_with("``") || text[i..].starts_with('\u{201c}') {
            find_quoted_phrase(text, i).map(|(p, end)| (p, TermKind::QuotedPhrase, end))
        } else {
            None
        };
        if let Some((tok, kind, end)) = hit {
            let _ = scan_prose_chunk(&text[prose_start..i], at_sentence_start, acc);
            acc.push(tok, kind);
            at_sentence_start = false;
            i = end;
            prose_start = i;
            continue;
        }
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    scan_prose_chunk(&text[prose_start..], at_sentence_start, acc);
}

fn scan_prose_chunk(chunk: &str, mut at_sentence_start: bool, acc: &mut TermAccumulator) -> bool {
    const TRIM: &[char] = &[',', '.', ';', ':', '!', '?', '\'', '"', '`', '\u{2018}', '\u{2019}'];
    let mut prev_kw: Option<String> = None;
    for raw_tok in chunk.split(|c: char| c.is_whitespace() || c == '~') {
        if raw_tok.is_empty() { continue; }
        let leading_punct = raw_tok.starts_with(['(', '[', '{', ',', ';', ':', '"', '\'']);
        let ends_sent = raw_tok.ends_with(['.', '!', '?']) && !is_abbreviation(raw_tok);
        let ends_clause = ends_sent || raw_tok.ends_with([',', ';', ':', ')', ']', '}']);
        let tok = trim_unmatched_closers(
            raw_tok.trim_start_matches(['(', '[', '{', ',', '.', ';', ':', '!', '?', '\'', '"', '`', '\u{2018}', '\u{2019}']),
            TRIM,
        );
        if tok.is_empty() {
            at_sentence_start |= ends_sent;
            if ends_clause { prev_kw = None; }
            continue;
        }
        if leading_punct { prev_kw = None; }
        if is_bare_math_token(tok) {
            acc.push(tok.to_string(), TermKind::MathToken);
            prev_kw = None;
        } else if let Some((text, kind)) = classify_prose_token(tok, at_sentence_start) {
            let kw = (kind == TermKind::Keyword).then(|| text.clone());
            acc.push(text, kind);
            if let (Some(prev), Some(curr)) = (&prev_kw, &kw) {
                if acc.bigrams_emitted < MAX_BIGRAMS && prev.chars().count() >= 4 && curr.chars().count() >= 4 {
                    acc.push(format!("{prev} {curr}"), TermKind::Keyword);
                    acc.bigrams_emitted += 1;
                }
            }
            prev_kw = kw;
        } else {
            prev_kw = None;
        }
        at_sentence_start = ends_sent;
        if ends_clause { prev_kw = None; }
    }
    at_sentence_start
}

fn is_abbreviation(tok: &str) -> bool {
    matches!(tok.to_ascii_lowercase().as_str(),
        "e.g." | "i.e." | "cf." | "eq." | "eqs." | "fig." | "figs." | "sec." | "al." | "ref." | "refs." | "vs.")
}

fn is_bare_math_token(tok: &str) -> bool {
    if let Some((lhs, rhs)) = tok.split_once(['_', '^', '=']) {
        let ok = |s: &str| !s.is_empty() && s.len() <= 12
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '*' | '/'));
        if ok(lhs) && ok(rhs) && !(lhs.chars().all(|c| c.is_ascii_digit()) && rhs.chars().all(|c| c.is_ascii_digit())) {
            return true;
        }
    }
    tok.split_once('(').and_then(|(h, r)| r.strip_suffix(')').map(|a| (h, a))).is_some_and(|(h, a)| {
        !h.is_empty() && h.len() <= 4 && h.bytes().all(|b| b.is_ascii_uppercase())
            && !a.is_empty() && a.len() <= 8 && a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b',' || b == b'+')
    })
}

fn classify_prose_token(tok: &str, at_sentence_start: bool) -> Option<(String, TermKind)> {
    if !tok.chars().any(char::is_alphabetic) { return None; }
    if tok.contains('/') && tok.chars().any(char::is_uppercase) {
        return Some((tok.to_string(), TermKind::ProperName));
    }
    if tok.contains('-') {
        let parts: Vec<&str> = tok.split('-').filter(|p| !p.is_empty()).collect();
        if parts.iter().filter(|p| p.chars().next().is_some_and(char::is_uppercase)).count() >= 2 {
            return Some((tok.to_string(), TermKind::ProperName));
        }
        let low = tok.to_ascii_lowercase();
        return (parts.len() >= 2 && low.chars().count() >= 4 && !is_stop_word(&low)).then_some((low, TermKind::Keyword));
    }
    let upper = tok.chars().filter(|c| c.is_uppercase()).count();
    if tok.chars().count() >= 2 && (upper >= 2 || tok.chars().skip(1).any(char::is_uppercase))
        && !matches!(tok, "II" | "III" | "IV" | "VI" | "VII" | "VIII" | "IX")
    {
        return Some((tok.to_string(), TermKind::ProperName));
    }
    let low = tok.to_ascii_lowercase();
    if is_stop_word(&low) { return None; }
    if tok.chars().next().is_some_and(char::is_uppercase) && tok.chars().count() >= 3
        && (!at_sentence_start || !is_common_sentence_opener(&low))
    {
        return Some((tok.to_string(), TermKind::ProperName));
    }
    (low.chars().count() >= 3).then_some((low, TermKind::Keyword))
}

const STOP_WORDS: &[&str] = &[
    "a", "about", "above", "across", "after", "again", "against", "all", "allow", "allows",
    "almost", "also", "although", "always", "among", "an", "and", "another", "any", "appear",
    "appears", "apply", "applying", "approach", "are", "around", "as", "assume", "assuming", "at",
    "be", "because", "become", "becomes", "been", "before", "behave", "behaves", "being", "below",
    "between", "both", "but", "by", "calculate", "called", "can", "cannot", "case", "check",
    "compare", "comparing", "compute", "consider", "could", "define", "defined", "depend",
    "depends", "derive", "determine", "did", "do", "does", "done", "during", "each", "either",
    "else", "enable", "enables", "even", "every", "exist", "exists", "expect", "explore", "extend",
    "extending", "find", "finds", "follow", "follows", "for", "found", "from", "further",
    "furthermore", "get", "gets", "give", "given", "gives", "goal", "had", "has", "have", "having",
    "hence", "here", "hold", "holds", "how", "however", "idea", "if", "implies", "imply", "in",
    "indicate", "indicates", "instead", "into", "investigate", "is", "it", "its", "itself", "just",
    "know", "known", "lead", "leads", "less", "let", "like", "look", "made", "make", "makes",
    "many", "may", "mean", "means", "method", "might", "more", "moreover", "most", "much", "must",
    "my", "need", "neither", "no", "nor", "not", "note", "notice", "now", "observe", "occur",
    "occurs", "of", "off", "often", "on", "once", "one", "only", "or", "other", "others", "our",
    "out", "over", "own", "paper", "papers", "part", "perhaps", "previous", "prior", "problem",
    "prove", "provide", "provides", "question", "rather", "recall", "relate", "relating", "remain",
    "remains", "result", "results", "same", "see", "seem", "seems", "seen", "serve", "serves",
    "set", "several", "should", "show", "shows", "since", "so", "some", "start", "still", "study",
    "such", "suggest", "suggests", "suppose", "take", "takes", "test", "than", "that", "the",
    "their", "them", "then", "there", "therefore", "these", "they", "this", "those", "though",
    "through", "thus", "to", "too", "try", "under", "until", "up", "upon", "us", "use", "used",
    "using", "varies", "vary", "very", "via", "want", "was", "way", "we", "well", "were", "what",
    "when", "where", "whereas", "whether", "which", "while", "who", "whose", "why", "will", "with",
    "within", "without", "work", "would", "written", "yield", "yields",
];

const COMMON_SENTENCE_OPENERS: &[&str] = &[
    "analytical", "anomalous", "asymptotic", "black", "bosonic", "bound", "bounds", "chain",
    "chaotic", "chiral", "classical", "closed", "complexity", "conformal", "continuous",
    "correlation", "correlations", "coupled", "critical", "dark", "dimensional", "direct",
    "discrete", "disordered", "driven", "dual", "dynamics", "early", "effective", "energy",
    "entanglement", "entropy", "ergodic", "exact", "extended", "fermion", "fermionic", "field",
    "finite", "first", "fractional", "free", "gauge", "general", "generalised", "generalized",
    "global", "gravitational", "growth", "high", "higher", "holographic", "infinite", "integrable",
    "interacting", "large", "late", "lattice", "limit", "limits", "linear", "local", "localised",
    "localized", "long", "low", "lower", "magnetic", "maximal", "measurement", "minimal", "mixed",
    "model", "modified", "monitored", "new", "nonlinear", "nonlocal", "novel", "numerical", "open",
    "operator", "optimal", "order", "perturbative", "phase", "pure", "quantum", "random", "rate",
    "recent", "renormalisation", "renormalization", "scaling", "scattering", "second", "short",
    "simple", "single", "small", "spectrum", "spin", "standard", "static", "stochastic", "strong",
    "superconducting", "symmetry", "system", "theory", "thermal", "three", "topological",
    "transition", "transport", "two", "universal", "universality", "weak",
];

fn is_stop_word(word: &str) -> bool { STOP_WORDS.binary_search(&word).is_ok() }
fn is_common_sentence_opener(word: &str) -> bool { COMMON_SENTENCE_OPENERS.binary_search(&word).is_ok() }

#[cfg(test)]
#[path = "idea_terms_tests.rs"]
mod tests;
