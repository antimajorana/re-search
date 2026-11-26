// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Filename parsing and lightweight TeX header extraction helpers for [`crate::source_folder`].

use crate::paper_key::{canonical_doi, split_arxiv_id};

#[derive(Debug, Default)]
pub(crate) struct ParsedFilename {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub arxiv: Option<String>,
    pub arxiv_version: Option<String>,
}

pub(crate) fn parse_filename(file_name: &str) -> ParsedFilename {
    let lower = file_name.to_ascii_lowercase();
    let stem = if lower.ends_with(".tar.gz") {
        &file_name[..file_name.len() - 7]
    } else if let Some(idx) = file_name.rfind('.') {
        &file_name[..idx]
    } else {
        file_name
    };

    let mut out = ParsedFilename::default();
    if let Ok((base, ver)) = split_arxiv_id(stem.trim()) {
        out.arxiv = Some(base.clone());
        out.arxiv_version = ver;
        out.title = format!("arXiv:{base}");
        return out;
    }

    // Pattern: "Author2024 - Title" or "Author (2024) - Title"
    if let Some((left, right)) = stem.split_once(" - ") {
        let right_t = right.trim();
        if !right_t.is_empty() {
            out.title = right_t.to_string();
        }
        let left_t = left.trim();
        if let Some((author_part, yr)) = split_author_and_year(left_t) {
            if !author_part.is_empty() {
                out.authors.push(author_part);
            }
            out.year = Some(yr);
        }
    }
    if out.title.is_empty() {
        out.title = stem.replace('_', " ").trim().to_string();
    }
    if out.year.is_none() {
        out.year = find_year_in_str(stem);
    }
    out
}

fn split_author_and_year(s: &str) -> Option<(String, i32)> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(3) {
        let win = &bytes[i..i + 4];
        if win.iter().all(|b| b.is_ascii_digit()) {
            if let Ok(yr) = std::str::from_utf8(win).unwrap_or("").parse::<i32>() {
                if (1800..=2100).contains(&yr) {
                    let author = s[..i]
                        .trim()
                        .trim_end_matches(['(', '[', ',', '-', '_'])
                        .trim()
                        .to_string();
                    return Some((author, yr));
                }
            }
        }
    }
    None
}

fn find_year_in_str(s: &str) -> Option<i32> {
    split_author_and_year(s).map(|(_, y)| y)
}

pub(crate) fn collections_from_rel_dir(rel_dir: Option<&str>) -> Vec<String> {
    let Some(dir) = rel_dir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    vec![dir.to_string()]
}

pub(crate) fn extract_tex_command(text: &str, cmd: &str) -> Option<String> {
    let start = text.find(cmd)? + cmd.len();
    let mut depth = 1usize;
    let mut end = start;
    for (off, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + off;
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 || end <= start {
        return None;
    }
    let cleaned = text[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}

pub(crate) fn find_arxiv_in_text(text: &str) -> Option<(String, Option<String>)> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("arxiv:")?;
    let rest = &text[idx + 6..];
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '-' | '_'))
        .collect();
    split_arxiv_id(&token).ok()
}

pub(crate) fn find_doi_in_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("doi:")?;
    let rest = text[idx + 4..].trim_start();
    let token: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '}' | ')' | ']' | ',' | ';'))
        .collect();
    canonical_doi(&token).ok()
}
