//! BibTeX formatting and user-citekey resolution helpers for [`crate::source_export`]
//! (package F13c, `00-overview.md` 2.11 & 2.13).

use crate::library::Library;
use crate::paper_key::PaperKey;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Format selected library papers into BibTeX text, preferring stored user `source_item.citekey`
/// over synthesized keys and deduplicating keys deterministically.
pub fn format_library_papers_bibtex(
    lib: &Library,
    keys: &[PaperKey],
) -> Result<(String, Vec<String>), String> {
    if keys.is_empty() {
        return Err("at least one paper key is required for BibTeX export".into());
    }
    let mut used_keys = BTreeSet::new();
    let mut citekeys = Vec::with_capacity(keys.len());
    let mut out = String::new();

    for (idx, key) in keys.iter().enumerate() {
        let canonical = lib.resolve_key(key)?.unwrap_or_else(|| key.clone());
        let detail = lib
            .get_paper(&canonical)?
            .ok_or_else(|| format!("paper `{canonical}` not found in library"))?;

        let user_citekey = detail.source_items.iter().find_map(|it| {
            it.citekey
                .as_deref()
                .map(sanitize_bibtex_key)
                .filter(|k| !k.is_empty())
        });
        let base = user_citekey.unwrap_or_else(|| {
            fallback_bibtex_key(&canonical, &detail.paper.authors, detail.paper.year)
        });
        let citekey = deduplicate_bibtex_key(&base, &mut used_keys);
        citekeys.push(citekey.clone());

        if idx > 0 {
            out.push('\n');
        }
        let raw_bib = detail.paper.bibtex.trim();
        if !raw_bib.is_empty() {
            out.push_str(&rewrite_entry_citekey(raw_bib, &citekey));
            if !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        let venue = detail
            .paper
            .flags
            .venue
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let entry_type = if venue.is_some() { "article" } else { "misc" };
        let _ = writeln!(out, "@{entry_type}{{{citekey},");
        if !detail.paper.authors.is_empty() {
            let joined = detail
                .paper
                .authors
                .iter()
                .map(|a| escape_bibtex_value(a))
                .collect::<Vec<_>>()
                .join(" and ");
            let _ = writeln!(out, "  author = {{{joined}}},");
        }
        let title = if detail.paper.title.trim().is_empty() {
            canonical.as_str()
        } else {
            detail.paper.title.trim()
        };
        let _ = writeln!(out, "  title = {{{{{}}}}},", escape_bibtex_value(title));
        if let Some(y) = detail.paper.year {
            let _ = writeln!(out, "  year = {{{y}}},");
        }
        if let Some(v) = venue {
            let _ = writeln!(out, "  journal = {{{}}},", escape_bibtex_value(v));
        }
        if let Some(doi) = detail
            .paper
            .ids
            .doi
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let _ = writeln!(out, "  doi = {{{}}},", escape_bibtex_value(doi));
        }
        if let Some(ax) = detail
            .paper
            .ids
            .arxiv
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let ver = detail
                .source_items
                .iter()
                .find_map(|it| it.version.as_deref())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            let eprint = match ver {
                Some(v) => format!("{ax}{v}"),
                None => ax.to_string(),
            };
            let _ = writeln!(out, "  eprint = {{{}}},", escape_bibtex_value(&eprint));
            let _ = writeln!(out, "  archivePrefix = {{arXiv}},");
        }
        out.push_str("}\n");
    }

    Ok((out, citekeys))
}

fn sanitize_bibtex_key(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
        .collect()
}

fn fallback_bibtex_key(key: &PaperKey, authors: &[String], year: Option<i32>) -> String {
    let surname: String = authors
        .first()
        .map(|a| {
            let tok = a
                .split_once(',')
                .map(|(before, _)| before.trim())
                .unwrap_or_else(|| a.split_whitespace().last().unwrap_or(""));
            tok.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let digest = Sha256::digest(key.as_str().as_bytes());
    let short_hash = format!("{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2]);
    if !surname.is_empty() {
        match year {
            Some(y) => format!("{surname}{y}_{short_hash}"),
            None => format!("{surname}_{short_hash}"),
        }
    } else {
        let slug: String = key
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("ref_{slug}")
    }
}

fn deduplicate_bibtex_key(base: &str, used: &mut BTreeSet<String>) -> String {
    let clean = if base.is_empty() { "ref" } else { base };
    if used.insert(clean.to_string()) {
        return clean.to_string();
    }
    for n in 2usize.. {
        let cand = format!("{clean}_{n}");
        if used.insert(cand.clone()) {
            return cand;
        }
    }
    unreachable!()
}

fn rewrite_entry_citekey(raw: &str, new_key: &str) -> String {
    let Some(at) = raw.find('@') else {
        return raw.to_string();
    };
    let after_at = &raw[at + 1..];
    let Some(open_rel) = after_at.find(['{', '(']) else {
        return raw.to_string();
    };
    let open = at + 1 + open_rel;
    let rest = &raw[open + 1..];
    let Some(comma) = rest.find(',') else {
        return raw.to_string();
    };
    format!("{}{new_key}{}", &raw[..open + 1], &rest[comma..])
}

fn escape_bibtex_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '%' => out.push_str("\\%"),
            '&' => out.push_str("\\&"),
            '#' => out.push_str("\\#"),
            '$' => out.push_str("\\$"),
            '\r' | '\n' => out.push(' '),
            _ => out.push(ch),
        }
    }
    if out.ends_with('\\') {
        out.pop();
        out.push_str("\\textbackslash{}");
    }
    out
}
