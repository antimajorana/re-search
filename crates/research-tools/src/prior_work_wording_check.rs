//! Phrase-check (rule V8) and forbidden-word guards (rules V5 & S8) for `crate::prior_work_wording`.

use super::wording;
use crate::investigation::anchor::fold_with_map;
use crate::paper_key::PaperKey;

/// Rule V8 backstop: reject absolute-claim phrases in model-written prose while allowing single
/// words like `"new"` or `"first"` in scoped phrases such as `"first calculation"`.
pub fn check_model_prose_phrases(text: &str) -> Result<(), Vec<&'static str>> {
    let folded = normalize_for_phrase_check(text);
    let hits: Vec<&'static str> = wording::BANNED_MODEL_PROSE_PHRASES
        .iter()
        .copied()
        .filter(|phrase| contains_word_phrase(&folded, phrase))
        .collect();
    if hits.is_empty() { Ok(()) } else { Err(hits) }
}

/// Check whether `text` contains any forbidden generated prior-work word (`new`, `novel`,
/// `novelty`, `verified`, `proved`, `unexplored`), ignoring canonical F2 `PaperKey` tokens
/// (`doi:...`, `arxiv:...`, etc.) so opaque external identifiers do not cause false positives.
pub fn contains_forbidden_prior_work_word(text: &str) -> Option<&'static str> {
    let masked = mask_paper_key_tokens(text);
    let folded = normalize_for_phrase_check(&masked);
    wording::FORBIDDEN_GENERATED_WORDS
        .iter()
        .copied()
        .find(|w| contains_word_phrase(&folded, w))
}

fn mask_paper_key_tokens(text: &str) -> String {
    text.split_whitespace()
        .map(|tok| {
            let trimmed = tok.trim_matches(|c: char| {
                matches!(c, '(' | ')' | ',' | '.' | ';' | '"' | '\'' | '[' | ']')
            });
            if PaperKey::parse(trimmed).is_ok() { "PAPERKEY" } else { tok }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_for_phrase_check(text: &str) -> String {
    let mut expanded = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            // Strip zero-width format characters, soft hyphen, and combining diacritics (M5).
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}'
            | '\u{0300}'..='\u{036F}' => {}
            // Expand common typographic ligatures so `"ﬁrst to"` is detected.
            '\u{FB00}' => expanded.push_str("ff"),
            '\u{FB01}' => expanded.push_str("fi"),
            '\u{FB02}' => expanded.push_str("fl"),
            '\u{FB03}' => expanded.push_str("ffi"),
            '\u{FB04}' => expanded.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => expanded.push_str("st"),
            _ => expanded.push(c),
        }
    }
    let lower = expanded.to_lowercase();
    let replaced: String =
        lower.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    fold_with_map(&replaced).0
}

fn contains_word_phrase(normalized_haystack: &str, phrase: &str) -> bool {
    let needle = normalize_for_phrase_check(phrase);
    if needle.is_empty() {
        return false;
    }
    let h = normalized_haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = normalized_haystack[start..].find(&needle) {
        let abs = start + pos;
        let end = abs + n.len();
        let left_ok = abs == 0 || h[abs - 1] == b' ';
        let right_ok = end == h.len() || h[end] == b' ';
        if left_ok && right_ok {
            return true;
        }
        let step = normalized_haystack[abs..].chars().next().map_or(1, char::len_utf8);
        start = abs + step;
    }
    false
}
