//! LaTeXML HTML5 class recognition, heading/tag parsing, and fingerprint helpers (`html-1`, V18).

use super::html_edges::map_html_kind_word;
use super::html_lex::{scan_html_tokens, HtmlToken};
use super::model::{SkNodeKind, NODE_FP_HEX_CHARS};
use sha2::{Digest, Sha256};

const LATEXML_STRUCTURAL_CLASSES: &[&str] = &[
    "ltx_document",
    "ltx_page_main",
    "ltx_page_content",
    "ltx_abstract",
    "ltx_part",
    "ltx_chapter",
    "ltx_section",
    "ltx_subsection",
    "ltx_subsubsection",
    "ltx_appendix",
    "ltx_theorem",
    "ltx_proof",
    "ltx_equation",
    "ltx_equationgroup",
    "ltx_figure",
    "ltx_table",
    "ltx_bibliography",
    "ltx_bibitem",
    "ltx_para",
    "ltx_p",
    "ltx_title",
];

/// Returns true iff `html` parses with HTML structure and LaTeXML markers (`ltx_*` classes
/// outside `<script>`/`<style>`/comments, or `<meta name="generator" content="...LaTeXML...">`),
/// rejecting generic arXiv 404 or error pages cleanly.
pub fn is_latexml_html_document(html: &str) -> bool {
    if html.trim().is_empty() || html.as_bytes().contains(&0) {
        return false;
    }
    let lex = scan_html_tokens(html);
    for tok in &lex.tokens {
        let HtmlToken::StartTag { name, attrs, .. } = tok else { continue };
        if name == "meta" {
            let meta_name = HtmlToken::attr(attrs, "name").unwrap_or("");
            let content = HtmlToken::attr(attrs, "content").unwrap_or("");
            if meta_name.eq_ignore_ascii_case("generator")
                && content.to_ascii_lowercase().contains("latexml")
            {
                return true;
            }
        }
        if let Some(cls) = HtmlToken::attr(attrs, "class") {
            if cls.split_whitespace().any(|c| {
                LATEXML_STRUCTURAL_CLASSES.contains(&c) || c.starts_with("ltx_theorem_")
            }) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn compute_html_body_fp(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len().min(4096));
    for (idx, tok) in text.split_whitespace().enumerate() {
        if idx > 0 {
            normalized.push(' ');
        }
        normalized.push_str(tok);
    }
    let digest = Sha256::digest(normalized.as_bytes());
    let mut hex = String::with_capacity(NODE_FP_HEX_CHARS);
    for byte in &digest[..NODE_FP_HEX_CHARS / 2] {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

pub(crate) fn classify_section_level(classes: &[&str]) -> Option<u8> {
    if classes.contains(&"ltx_part") { Some(0) }
    else if classes.contains(&"ltx_chapter") { Some(1) }
    else if classes.contains(&"ltx_section") || classes.contains(&"ltx_appendix") { Some(2) }
    else if classes.contains(&"ltx_subsection") { Some(3) }
    else if classes.contains(&"ltx_subsubsection") { Some(4) }
    else { None }
}

pub(crate) fn classify_structural_node(
    classes: &[&str],
) -> Option<(SkNodeKind, Option<String>, Option<u8>)> {
    if classes.contains(&"ltx_abstract") {
        return Some((SkNodeKind::FrontMatter, Some("abstract".into()), None));
    }
    if classes.contains(&"ltx_bibliography") {
        return Some((SkNodeKind::Bibliography, Some("thebibliography".into()), None));
    }
    if let Some(lvl) = classify_section_level(classes) {
        let env = if classes.contains(&"ltx_appendix") { "appendix" }
            else if classes.contains(&"ltx_subsection") { "subsection" }
            else if classes.contains(&"ltx_subsubsection") { "subsubsection" }
            else if classes.contains(&"ltx_chapter") { "chapter" }
            else if classes.contains(&"ltx_part") { "part" }
            else { "section" };
        return Some((SkNodeKind::Section, Some(env.into()), Some(lvl)));
    }
    if classes.contains(&"ltx_proof") {
        return Some((SkNodeKind::Proof, Some("proof".into()), None));
    }
    if classes.contains(&"ltx_figure") {
        return Some((SkNodeKind::Figure, Some("figure".into()), None));
    }
    if classes.contains(&"ltx_table") && !classes.contains(&"ltx_eqn_table") {
        return Some((SkNodeKind::Table, Some("table".into()), None));
    }
    for &c in classes {
        if let Some(suffix) = c.strip_prefix("ltx_theorem_") {
            let kind = map_html_kind_word(suffix, true).unwrap_or(SkNodeKind::Theorem);
            return Some((kind, Some(suffix.to_ascii_lowercase()), None));
        }
    }
    if classes.contains(&"ltx_theorem") {
        return Some((SkNodeKind::Theorem, Some("theorem".into()), None));
    }
    if classes.contains(&"ltx_algorithm") {
        return Some((SkNodeKind::Algorithm, Some("algorithm".into()), None));
    }
    None
}

pub(crate) fn is_heading_for_node(kind: SkNodeKind, classes: &[&str]) -> bool {
    if matches!(kind, SkNodeKind::Figure | SkNodeKind::Table) {
        return classes.contains(&"ltx_caption");
    }
    if kind == SkNodeKind::Proof {
        return classes.contains(&"ltx_title_proof") || classes.contains(&"ltx_title");
    }
    if kind == SkNodeKind::Section {
        return classes.iter().any(|c| {
            matches!(
                *c,
                "ltx_title_section"
                    | "ltx_title_subsection"
                    | "ltx_title_subsubsection"
                    | "ltx_title_appendix"
                    | "ltx_title_chapter"
                    | "ltx_title_part"
                    | "ltx_title"
            )
        });
    }
    if kind.is_theorem_like() {
        return classes.contains(&"ltx_title_theorem") || classes.contains(&"ltx_title");
    }
    false
}

pub(crate) fn extract_local_fragment(href: &str) -> Option<&str> {
    href.strip_prefix('#').map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn non_empty_str(s: &str) -> Option<String> {
    let c = collapse_ws(s);
    (!c.is_empty()).then_some(c)
}

pub(crate) fn strip_equation_tag(raw: &str) -> Option<String> {
    let t = collapse_ws(raw);
    let inner = t.strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or(&t).trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

pub(crate) fn infer_theorem_kind_from_tag(raw_tag: &str) -> Option<SkNodeKind> {
    let head = collapse_ws(raw_tag);
    let first = head.split_whitespace().next()?;
    map_html_kind_word(first.trim_end_matches(['.', ':']), true)
}

pub(crate) fn extract_theorem_number(raw_tag: &str) -> Option<String> {
    let t = collapse_ws(raw_tag);
    if t.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = t.split_whitespace().collect();
    if let Some(first) = tokens.first() {
        if map_html_kind_word(first.trim_end_matches(['.', ':']), true).is_some() {
            let rest = tokens[1..].join(" ");
            let clean = rest.trim_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ':'));
            return (!clean.is_empty()).then(|| clean.to_string());
        }
    }
    let clean = t.trim_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ':'));
    (!clean.is_empty() && clean.chars().any(|c| c.is_ascii_digit())).then(|| clean.to_string())
}

pub(crate) fn clean_theorem_title(raw_title: &str) -> Option<String> {
    let t = collapse_ws(raw_title);
    let trimmed = t.trim().trim_end_matches(['.', ':']).trim();
    let unwrapped = trimmed
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(trimmed)
        .trim();
    (!unwrapped.is_empty()).then(|| unwrapped.to_string())
}

pub(crate) fn extract_section_number(raw_tag: &str) -> Option<String> {
    let t = collapse_ws(raw_tag);
    let tokens: Vec<&str> = t.split_whitespace().collect();
    let rest = match tokens.first() {
        Some(f)
            if matches!(
                f.trim_end_matches(['.', ':']).to_ascii_lowercase().as_str(),
                "section" | "appendix" | "chapter" | "part" | "§"
            ) =>
        {
            tokens[1..].join(" ")
        }
        _ => t,
    };
    let clean = rest.trim().trim_end_matches(['.', ':']).trim();
    (!clean.is_empty()).then(|| clean.to_string())
}

pub(crate) fn extract_float_number(raw_tag: &str) -> Option<String> {
    let t = collapse_ws(raw_tag);
    let tokens: Vec<&str> = t.split_whitespace().collect();
    let rest = match tokens.first() {
        Some(f)
            if matches!(
                f.trim_end_matches(['.', ':']).to_ascii_lowercase().as_str(),
                "figure" | "fig" | "table" | "tab"
            ) =>
        {
            tokens[1..].join(" ")
        }
        _ => t,
    };
    let clean = rest.trim().trim_end_matches(['.', ':']).trim();
    (!clean.is_empty()).then(|| clean.to_string())
}

pub(crate) fn parse_proof_header_mention(head: &str) -> Option<(SkNodeKind, String)> {
    let lower = head.to_ascii_lowercase();
    let after = lower.find("proof of ").map(|i| &head[i + 9..])?;
    let tokens: Vec<&str> = after.split_whitespace().collect();
    let kind = map_html_kind_word(tokens.first()?, false)?;
    let num = tokens.get(1)?.trim_end_matches(['.', ',', ':', ')']).trim();
    (!num.is_empty()).then(|| (kind, num.to_string()))
}
