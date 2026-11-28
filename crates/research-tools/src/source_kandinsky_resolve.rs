// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Path resolution, identifier extraction, and unpacked arXiv TeX lookup helpers for
//! [`crate::source_kandinsky::KandinskyVaultAdapter`] (package F13a, `00-overview.md` 2.11).

use crate::library::{LibraryIds, SourceItemFile};
use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::paper_sources::{
    file_meta_without_reading, hash_contained_file, portable_path_string, resolve_contained_abs,
    resolve_contained_rel, FILE_ROLE_TEX, MAX_SOURCE_TEXT_BYTES,
};
use crate::pdf_firstpage::normalize_valid_isbn;
use crate::sandbox::{strip_verbatim_prefix, WorkspaceSandbox};
use crate::source_frontmatter::{FlatMap, FlatVal};
use crate::source_kandinsky::ExternalPdfRoot;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Strips optional `[[target#frag|alias]]` wikilink chrome from a stored `pdf` or `files` value
/// (ported from Kandinsky `external.rs::clean_ref_token`).
pub fn clean_ref_token(raw: &str) -> &str {
    let t = raw.trim().trim_matches(['"', '\'']);
    let inner = t
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(t);
    inner
        .split('|')
        .next()
        .unwrap_or(inner)
        .split('#')
        .next()
        .unwrap_or(inner)
        .trim()
}

pub(crate) fn clean_prefix(raw: &str) -> String {
    raw.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

pub(crate) fn clean_rel_dir(raw: &str) -> String {
    raw.trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

pub(crate) fn extract_entry_ids(props: &FlatMap) -> (LibraryIds, Option<String>) {
    let mut ids = LibraryIds::default();
    let mut version: Option<String> = get_scalar(props, "arxivVersion").and_then(|v| {
        let t = v.trim().trim_start_matches(['v', 'V']);
        (!t.is_empty() && t.chars().all(|c| c.is_ascii_digit())).then(|| format!("v{t}"))
    });

    if let Some(raw_doi) = get_scalar(props, "DOI").or_else(|| get_scalar(props, "doi")) {
        if let Ok(norm) = canonical_doi(&raw_doi) {
            if let Some(ax_part) = norm.strip_prefix("10.48550/arxiv.") {
                if let Ok((base, ver)) = split_arxiv_id(ax_part) {
                    ids.arxiv = Some(base);
                    if version.is_none() {
                        version = ver;
                    }
                }
            }
            ids.doi = Some(norm);
        }
    }
    for field in ["archiveID", "arxiv", "eprint"] {
        if let Some(raw_ax) = get_scalar(props, field) {
            if let Ok((base, ver)) = split_arxiv_id(&raw_ax) {
                ids.arxiv = Some(base);
                if version.is_none() {
                    version = ver;
                }
                break;
            }
        }
    }
    if let Some(raw_isbn) = get_scalar(props, "ISBN").or_else(|| get_scalar(props, "isbn")) {
        if let Some(norm) = normalize_valid_isbn(&raw_isbn) {
            ids.isbn = Some(norm);
        }
    }
    (ids, version)
}

pub(crate) fn collect_pdf_candidates(props: &FlatMap) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["pdf", "external_pdf", "files"] {
        for raw in get_list(props, key) {
            let cleaned = clean_ref_token(&raw).replace('\\', "/");
            if cleaned.is_empty() {
                continue;
            }
            if key == "files" && !cleaned.to_ascii_lowercase().ends_with(".pdf") {
                continue;
            }
            if !out.contains(&cleaned) {
                out.push(cleaned);
            }
        }
    }
    out
}

pub(crate) fn resolve_vault_pdf_ref(
    canon_vault: &Path,
    entry_dir: &Path,
    ext_roots: &[ExternalPdfRoot],
    raw_ref: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let norm = clean_ref_token(raw_ref).replace('\\', "/");
    if norm.is_empty() || norm.contains('\0') {
        return Err("empty or NUL PDF reference".into());
    }
    for (idx, ext) in ext_roots.iter().enumerate() {
        let alias = format!("@ext/{idx}");
        if let Some(sub) = norm
            .strip_prefix(&format!("{}/", ext.prefix))
            .or_else(|| norm.strip_prefix(&format!("{alias}/")))
        {
            let safe = resolve_contained_rel(&ext.canon_root, sub)?;
            return Ok((ext.canon_root.clone(), safe));
        }
    }
    if Path::new(&norm).is_absolute() {
        let abs = PathBuf::from(&norm);
        for ext in ext_roots {
            if let Ok(safe) = resolve_contained_abs(&ext.canon_root, &abs) {
                return Ok((ext.canon_root.clone(), safe));
            }
        }
        return Err(format!(
            "absolute PDF path `{raw_ref}` escapes registered roots"
        ));
    }
    // Check inside entry directory first, then vault root (both strictly contained inside canon_vault).
    if let Ok(safe) = resolve_contained_rel(entry_dir, &norm) {
        if WorkspaceSandbox::is_same_or_inside(canon_vault, &safe) {
            return Ok((canon_vault.to_path_buf(), safe));
        }
    }
    let safe = resolve_contained_rel(canon_vault, &norm)?;
    Ok((canon_vault.to_path_buf(), safe))
}

pub(crate) fn resolve_external_roots(
    canon_vault: &Path,
    configured: &[String],
) -> Vec<ExternalPdfRoot> {
    let mut out = Vec::new();
    for raw in configured {
        let prefix = clean_prefix(raw);
        if prefix.is_empty() || prefix.contains('\0') {
            continue;
        }
        if let Some(canon_root) = resolve_optional_dir(canon_vault, &prefix) {
            if !out.iter().any(|e: &ExternalPdfRoot| e.canon_root == canon_root) {
                out.push(ExternalPdfRoot { prefix, canon_root });
            }
        }
    }
    out
}

pub(crate) fn resolve_optional_dir(canon_vault: &Path, raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }
    let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)?;
        home.join(rest)
    } else {
        let clean = trimmed.replace('\\', "/");
        let p = Path::new(&clean);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            let mut lexical = canon_vault.to_path_buf();
            for comp in p.components() {
                match comp {
                    Component::CurDir => {}
                    Component::ParentDir => {
                        lexical.pop();
                    }
                    Component::Normal(c) => lexical.push(c),
                    _ => return None,
                }
            }
            lexical
        }
    };
    let canon = fs::canonicalize(&expanded).ok()?;
    canon.is_dir().then(|| strip_verbatim_prefix(&canon))
}

pub(crate) fn find_unpacked_arxiv_tex(
    ax_root: &Path,
    arxiv_id: &str,
    version: Option<&str>,
    hash_files: bool,
) -> Option<SourceItemFile> {
    let safe_id = arxiv_id.replace('/', "_");
    let candidates = match version {
        Some(v) => vec![format!("{safe_id}{v}"), safe_id.clone(), arxiv_id.to_string()],
        None => vec![safe_id.clone(), arxiv_id.to_string()],
    };
    for sub in candidates {
        let Ok(dir) = resolve_contained_rel(ax_root, &sub) else {
            continue;
        };
        if !dir.is_dir() {
            continue;
        }
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut tex_files: Vec<PathBuf> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if name.ends_with(".tex") {
                    resolve_contained_abs(ax_root, &p).ok()
                } else {
                    None
                }
            })
            .collect();
        tex_files.sort();
        // Prefer main.tex or ms.tex if present.
        let picked = tex_files
            .iter()
            .find(|p| {
                matches!(
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .as_deref(),
                    Some("main.tex" | "ms.tex" | "paper.tex")
                )
            })
            .or_else(|| tex_files.first())?;
        let (size, mtime_ms) = file_meta_without_reading(picked).ok()?;
        let sha256 = if hash_files {
            hash_contained_file(ax_root, picked, MAX_SOURCE_TEXT_BYTES).ok()
        } else {
            None
        };
        return Some(SourceItemFile {
            path: portable_path_string(picked),
            role: FILE_ROLE_TEX.to_string(),
            size,
            mtime_ms,
            sha256,
        });
    }
    None
}

pub(crate) fn get_scalar(map: &FlatMap, key: &str) -> Option<String> {
    map.get(key)
        .and_then(FlatVal::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn get_list(map: &FlatMap, key: &str) -> Vec<String> {
    map.get(key)
        .map(FlatVal::as_list)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub(crate) fn extract_year(raw: &str) -> Option<i32> {
    let bytes = raw.as_bytes();
    for win in bytes.windows(4) {
        if win.iter().all(|b| b.is_ascii_digit()) {
            if let Ok(s) = std::str::from_utf8(win) {
                if let Ok(y) = s.parse::<i32>() {
                    if (1800..=2100).contains(&y) {
                        return Some(y);
                    }
                }
            }
        }
    }
    None
}
