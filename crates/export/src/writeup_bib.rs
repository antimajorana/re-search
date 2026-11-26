//! BibTeX key resolution (preferring the user's own library citation key) and `.bib` rendering
//! for F15 write-ups (`00-overview.md` 2.11 & 2.13 rule 3).

use crate::writeup::BibEntry;
use research_tools::library::Library;
use research_tools::paper_key::PaperRef;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Resolve a [`BibEntry`] for `paper` using F10/F13's `Library`:
/// - Uses the user's own `source_item.citekey` when present (`00-overview.md` 2.13 rule 3)
/// - Otherwise makes a deterministic BibTeX key from first author surname, year, and key hash
/// - Avoids collisions against `used_keys` by appending a numeric suffix when needed
pub fn resolve_bib_entry(
    lib: Option<&Library>,
    paper: &PaperRef,
    fallback_title: &str,
    fallback_authors: &[String],
    fallback_year: Option<i32>,
    used_keys: &mut BTreeSet<String>,
) -> Result<BibEntry, String> {
    let detail = match lib {
        Some(l) => l.get_paper(&paper.key)?,
        None => None,
    };

    let user_citekey = detail.as_ref().and_then(|d| {
        d.source_items
            .iter()
            .find_map(|it| it.citekey.as_deref())
            .map(sanitize_cite_key)
            .filter(|k| !k.is_empty())
    });

    let title = detail
        .as_ref()
        .map(|d| d.paper.title.trim())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback_title.trim().to_string());

    let authors = detail
        .as_ref()
        .map(|d| d.paper.authors.clone())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| fallback_authors.to_vec());

    let year = detail
        .as_ref()
        .and_then(|d| d.paper.year)
        .or(fallback_year);

    let venue = detail
        .as_ref()
        .and_then(|d| d.paper.flags.venue.clone());

    let raw_bibtex = detail
        .as_ref()
        .map(|d| d.paper.bibtex.trim())
        .filter(|b| !b.is_empty())
        .map(str::to_string);

    let base_key = user_citekey.unwrap_or_else(|| make_fallback_cite_key(paper, &authors, year));
    let cite_key = deduplicate_key(&base_key, used_keys);

    Ok(BibEntry {
        cite_key,
        paper: paper.clone(),
        title,
        authors,
        year,
        venue,
        raw_bibtex,
    })
}

/// Render a deterministic `.bib` file containing one entry per [`BibEntry`].
pub fn render_bibtex(entries: &[BibEntry]) -> String {
    let mut out = String::new();
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if let Some(raw) = entry.raw_bibtex.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(&rewrite_bibtex_key(raw, &entry.cite_key));
            if !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        let entry_type = if entry.venue.is_some() {
            "article"
        } else {
            "misc"
        };
        let _ = writeln!(out, "@{entry_type}{{{},", entry.cite_key);
        if !entry.authors.is_empty() {
            let joined = entry
                .authors
                .iter()
                .map(|a| escape_bibtex_value(a))
                .collect::<Vec<_>>()
                .join(" and ");
            let _ = writeln!(out, "  author = {{{joined}}},");
        }
        let title = if entry.title.trim().is_empty() {
            entry.paper.key.as_str()
        } else {
            entry.title.as_str()
        };
        let _ = writeln!(out, "  title = {{{{{}}}}},", escape_bibtex_value(title));
        if let Some(y) = entry.year {
            let _ = writeln!(out, "  year = {{{y}}},");
        }
        if let Some(v) = entry.venue.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let _ = writeln!(out, "  journal = {{{}}},", escape_bibtex_value(v));
        }
        if let Some(doi) = entry.paper.ids.doi.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let _ = writeln!(out, "  doi = {{{}}},", escape_bibtex_value(doi));
        }
        if let Some(ax) = entry.paper.ids.arxiv.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let eprint = match entry.paper.version.as_deref() {
                Some(v) if !v.is_empty() => format!("{ax}{v}"),
                _ => ax.to_string(),
            };
            let _ = writeln!(out, "  eprint = {{{}}},", escape_bibtex_value(&eprint));
            let _ = writeln!(out, "  archivePrefix = {{arXiv}},");
        }
        out.push_str("}\n");
    }
    out
}

/// Sanitize a citation key to safe BibTeX key characters (`[A-Za-z0-9_:-]`).
pub fn sanitize_cite_key(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
        .collect()
}

fn make_fallback_cite_key(paper: &PaperRef, authors: &[String], year: Option<i32>) -> String {
    let surname: String = authors
        .first()
        .map(|a| {
            let tok = if let Some((before, _)) = a.split_once(',') {
                before.trim()
            } else {
                a.split_whitespace().last().unwrap_or("")
            };
            tok.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();

    let digest = Sha256::digest(paper.key.as_str().as_bytes());
    let short_hash = format!("{:02x}{:02x}{:02x}", digest[0], digest[1], digest[2]);

    if !surname.is_empty() {
        match year {
            Some(y) => format!("{surname}{y}_{short_hash}"),
            None => format!("{surname}_{short_hash}"),
        }
    } else {
        let slug: String = paper
            .key
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("ref_{slug}")
    }
}

fn deduplicate_key(base: &str, used: &mut BTreeSet<String>) -> String {
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

fn rewrite_bibtex_key(raw: &str, new_key: &str) -> String {
    let Some(open) = raw.find('{') else {
        return raw.to_string();
    };
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
    out
}
