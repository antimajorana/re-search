// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Read-only plain-folder paper source adapter (PDFs, TeX sources, arXiv archives) for package
//! F13a (`00-overview.md` 2.11).
//!
//! Uses F14a (`pdf_pages`, `pdf_firstpage`, `pdf_match`) for PDF identifier and first-page
//! metadata extraction, derives collections from subfolders, refuses symlinks/junctions or paths
//! leaving the registered root, and never writes into the folder.

use crate::library::{
    LibraryIds, SourceItemFile, SourceItemRecord, SourceItemState, SourceKind, SourceRecord,
};
use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::paper_sources::{
    canonicalize_source_root, derive_paper_key, file_meta_without_reading, portable_path_string,
    read_contained_bytes, sha256_hex, PaperSourceAdapter, ScanContext, ScanOutput, ScanWarning,
    ScannedItem, FILE_ROLE_PDF, FILE_ROLE_TEX, MAX_SOURCE_TEXT_BYTES,
};
use crate::pdf_firstpage::{
    extract_first_page_candidates, extract_pdf_info_candidates, merge_first_page_candidates,
};
use crate::pdf_match::match_candidates_offline;
use crate::pdf_pages::{extract_pdf_pages, PdfExtractOptions, MAX_USER_PDF_BYTES};
use crate::sandbox::{is_symlink_or_junction, strip_verbatim_prefix, WorkspaceSandbox};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Read-only adapter for a plain directory of papers (`SourceKind::Folder`).
#[derive(Debug, Default, Clone, Copy)]
pub struct FolderAdapter;

impl PaperSourceAdapter for FolderAdapter {
    fn kind(&self) -> SourceKind {
        SourceKind::Folder
    }

    fn scan(&self, source: &SourceRecord, ctx: &ScanContext) -> Result<ScanOutput, String> {
        let canon_root = canonicalize_source_root(Path::new(&source.root))?;
        let mut out = ScanOutput::default();
        out.roots_checked.push(canon_root.clone());

        let mut stack: Vec<(PathBuf, Option<String>)> = vec![(canon_root.clone(), None)];
        let mut visited_dirs: BTreeSet<PathBuf> = BTreeSet::new();
        visited_dirs.insert(canon_root.clone());

        while let Some((dir, rel_dir)) = stack.pop() {
            let rd = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) => {
                    out.warnings.push(ScanWarning {
                        path: rel_dir.unwrap_or_else(|| ".".into()),
                        reason: format!("cannot read directory: {e}"),
                    });
                    continue;
                }
            };
            let mut entries: Vec<(String, PathBuf)> = rd
                .flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));

            let mut subdirs = Vec::new();
            for (name, path) in entries {
                if name.starts_with('.') {
                    continue;
                }
                let rel_path = match &rel_dir {
                    Some(parent) => format!("{parent}/{name}"),
                    None => name.clone(),
                };
                let Ok(lmeta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if is_symlink_or_junction(&path, &lmeta) {
                    out.warnings.push(ScanWarning {
                        path: rel_path,
                        reason: "refused symlink or junction inside folder source".into(),
                    });
                    continue;
                }
                let canon_entry = match fs::canonicalize(&path) {
                    Ok(p) => strip_verbatim_prefix(&p),
                    Err(e) => {
                        out.warnings.push(ScanWarning {
                            path: rel_path,
                            reason: format!("cannot resolve path: {e}"),
                        });
                        continue;
                    }
                };
                if !WorkspaceSandbox::is_same_or_inside(&canon_root, &canon_entry) {
                    out.warnings.push(ScanWarning {
                        path: rel_path,
                        reason: "path escapes registered folder root".into(),
                    });
                    continue;
                }

                if lmeta.is_dir() {
                    if visited_dirs.insert(canon_entry.clone()) {
                        subdirs.push((canon_entry, Some(rel_path)));
                    }
                    continue;
                }
                if !lmeta.is_file() {
                    continue;
                }

                let Some(kind) = classify_supported_file(&name) else {
                    continue;
                };
                let scanned = scan_folder_file(
                    &source.id,
                    &canon_root,
                    &canon_entry,
                    &rel_path,
                    rel_dir.as_deref(),
                    &name,
                    kind,
                    ctx,
                    &mut out.warnings,
                );
                if let Some(item) = scanned {
                    out.items.push(item);
                }
            }
            // Push subdirs in reverse so alphabetical order is preserved on pop.
            for sub in subdirs.into_iter().rev() {
                stack.push(sub);
            }
        }

        out.items
            .sort_by(|a, b| a.item.external_id.cmp(&b.item.external_id));
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderFileKind {
    Pdf,
    Tex,
    Archive,
}

fn classify_supported_file(name: &str) -> Option<FolderFileKind> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        Some(FolderFileKind::Pdf)
    } else if lower.ends_with(".tex") {
        Some(FolderFileKind::Tex)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".zip") {
        Some(FolderFileKind::Archive)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_folder_file(
    source_id: &str,
    canon_root: &Path,
    canon_file: &Path,
    rel_path: &str,
    rel_dir: Option<&str>,
    file_name: &str,
    kind: FolderFileKind,
    ctx: &ScanContext,
    warnings: &mut Vec<ScanWarning>,
) -> Option<ScannedItem> {
    let (size, mtime_ms) = file_meta_without_reading(canon_file).ok()?;
    let portable = portable_path_string(canon_file);

    // Cloud-drive safe incremental check: match by current file path + size + mtime_ms
    // without opening or reading file content.
    if let Some(prev) = ctx.previous_by_path.get(&portable) {
        let matching_file = prev
            .files
            .iter()
            .find(|f| f.path == portable && f.size == size && f.mtime_ms == mtime_ms);
        if let Some(pf) = matching_file {
            if prev.state != SourceItemState::Missing && (!ctx.hash_files || pf.sha256.is_some()) {
                return Some(ScannedItem {
                    item: prev.clone(),
                    abstract_text: String::new(),
                    venue: None,
                    reused_unchanged: true,
                });
            }
        }
    }

    let collections = collections_from_rel_dir(rel_dir);
    let parsed_name = parse_filename(file_name);

    match kind {
        FolderFileKind::Pdf => Some(scan_pdf_file(
            source_id,
            canon_root,
            canon_file,
            rel_path,
            file_name,
            portable,
            size,
            mtime_ms,
            collections,
            &parsed_name,
            ctx,
            warnings,
        )),
        FolderFileKind::Tex => Some(scan_tex_file(
            source_id,
            canon_root,
            canon_file,
            rel_path,
            portable,
            size,
            mtime_ms,
            collections,
            &parsed_name,
            ctx,
            warnings,
        )),
        FolderFileKind::Archive => Some(scan_archive_file(
            source_id,
            canon_root,
            canon_file,
            rel_path,
            portable,
            size,
            mtime_ms,
            collections,
            &parsed_name,
            ctx,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_pdf_file(
    source_id: &str,
    canon_root: &Path,
    canon_file: &Path,
    rel_path: &str,
    file_name: &str,
    portable: String,
    size: u64,
    mtime_ms: i64,
    collections: Vec<String>,
    parsed_name: &ParsedFilename,
    ctx: &ScanContext,
    warnings: &mut Vec<ScanWarning>,
) -> ScannedItem {
    let mut ids = LibraryIds::default();
    let mut version = parsed_name.arxiv_version.clone();
    if let Some(ax) = &parsed_name.arxiv {
        ids.arxiv = Some(ax.clone());
    }
    let mut title = parsed_name.title.clone();
    let mut authors = parsed_name.authors.clone();
    let mut year = parsed_name.year;
    let mut sha256: Option<String> = None;

    let key = match read_contained_bytes(canon_root, canon_file, MAX_USER_PDF_BYTES) {
        Ok(bytes) => {
            if ctx.hash_files {
                sha256 = Some(sha256_hex(&bytes));
            }
            let opts = PdfExtractOptions::user_supplied().with_page_range(1, 5);
            let extracted = match extract_pdf_pages(&bytes, &opts) {
                Ok(doc) => Ok(doc),
                Err(crate::pdf::PdfError::InvalidPageRange { total_pages, .. })
                    if total_pages > 0 =>
                {
                    let clamped = PdfExtractOptions::user_supplied()
                        .with_page_range(1, total_pages.min(5));
                    extract_pdf_pages(&bytes, &clamped)
                }
                Err(e) => Err(e),
            };
            let mut candidates = match extracted {
                Ok(doc) => extract_first_page_candidates(&doc.text, Some(file_name)),
                Err(e) => {
                    warnings.push(ScanWarning {
                        path: rel_path.to_string(),
                        reason: format!("unreadable PDF text: {e}"),
                    });
                    Default::default()
                }
            };
            let info_cands = extract_pdf_info_candidates(&bytes);
            candidates = merge_first_page_candidates(candidates, info_cands);

            if let Some(ax) = candidates.arxiv_ids.first() {
                ids.arxiv = Some(ax.value.id.clone());
                if version.is_none() {
                    version = ax.value.version.clone();
                }
            }
            if let Some(doi) = candidates.dois.first() {
                ids.doi = Some(doi.value.clone());
            }
            if let Some(isbn) = candidates.isbns.first() {
                ids.isbn = Some(isbn.value.clone());
            }
            if let Some(t) = candidates.title.as_ref().map(|c| c.value.trim()).filter(|s| !s.is_empty()) {
                title = t.to_string();
            }
            if let Some(a) = candidates.authors.as_ref().filter(|c| !c.value.is_empty()) {
                authors = a.value.clone();
            }
            if let Some(y) = candidates.year.as_ref().and_then(|c| c.value.parse::<i32>().ok()) {
                year = Some(y);
            }

            let offline = match_candidates_offline(
                &candidates,
                if ctx.hash_files { Some(&bytes) } else { None },
            );
            derive_paper_key(&ids, sha256.as_deref())
                .or_else(|| offline.map(|m| m.paper.key))
        }
        Err(reason) => {
            warnings.push(ScanWarning {
                path: rel_path.to_string(),
                reason,
            });
            derive_paper_key(&ids, None)
        }
    };

    let item_kind = if ids.isbn.is_some() {
        "book"
    } else if ids.arxiv.is_some() && ids.doi.is_none() {
        "preprint"
    } else {
        "journalArticle"
    };

    ScannedItem {
        item: SourceItemRecord {
            source_id: source_id.to_string(),
            external_id: rel_path.to_string(),
            key,
            citekey: None,
            ids,
            version,
            title,
            authors,
            year,
            item_kind: item_kind.to_string(),
            collections,
            tags: Vec::new(),
            files: vec![SourceItemFile {
                path: portable,
                role: FILE_ROLE_PDF.to_string(),
                size,
                mtime_ms,
                sha256,
            }],
            state: SourceItemState::Present,
            updated_ms: ctx.now_ms,
        },
        abstract_text: String::new(),
        venue: None,
        reused_unchanged: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_tex_file(
    source_id: &str,
    canon_root: &Path,
    canon_file: &Path,
    rel_path: &str,
    portable: String,
    size: u64,
    mtime_ms: i64,
    collections: Vec<String>,
    parsed_name: &ParsedFilename,
    ctx: &ScanContext,
    warnings: &mut Vec<ScanWarning>,
) -> ScannedItem {
    let mut ids = LibraryIds::default();
    let mut version = parsed_name.arxiv_version.clone();
    if let Some(ax) = &parsed_name.arxiv {
        ids.arxiv = Some(ax.clone());
    }
    let mut title = parsed_name.title.clone();
    let mut authors = parsed_name.authors.clone();
    let mut sha256 = None;

    match read_contained_bytes(canon_root, canon_file, MAX_SOURCE_TEXT_BYTES) {
        Ok(bytes) => {
            if ctx.hash_files {
                sha256 = Some(sha256_hex(&bytes));
            }
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(16_384)]);
            if let Some(t) = extract_tex_command(&head, "\\title{") {
                title = t;
            }
            if let Some(a) = extract_tex_command(&head, "\\author{") {
                authors = a
                    .split("\\and")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            if ids.arxiv.is_none() {
                if let Some((base, ver)) = find_arxiv_in_text(&head) {
                    ids.arxiv = Some(base);
                    if version.is_none() {
                        version = ver;
                    }
                }
            }
            if ids.doi.is_none() {
                if let Some(doi) = find_doi_in_text(&head) {
                    ids.doi = Some(doi);
                }
            }
        }
        Err(reason) => warnings.push(ScanWarning {
            path: rel_path.to_string(),
            reason,
        }),
    }

    let key = derive_paper_key(&ids, sha256.as_deref());
    ScannedItem {
        item: SourceItemRecord {
            source_id: source_id.to_string(),
            external_id: rel_path.to_string(),
            key,
            citekey: None,
            ids,
            version,
            title,
            authors,
            year: parsed_name.year,
            item_kind: "preprint".to_string(),
            collections,
            tags: Vec::new(),
            files: vec![SourceItemFile {
                path: portable,
                role: FILE_ROLE_TEX.to_string(),
                size,
                mtime_ms,
                sha256,
            }],
            state: SourceItemState::Present,
            updated_ms: ctx.now_ms,
        },
        abstract_text: String::new(),
        venue: None,
        reused_unchanged: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_archive_file(
    source_id: &str,
    canon_root: &Path,
    canon_file: &Path,
    rel_path: &str,
    portable: String,
    size: u64,
    mtime_ms: i64,
    collections: Vec<String>,
    parsed_name: &ParsedFilename,
    ctx: &ScanContext,
) -> ScannedItem {
    let mut ids = LibraryIds::default();
    if let Some(ax) = &parsed_name.arxiv {
        ids.arxiv = Some(ax.clone());
    }
    let sha256 = if ctx.hash_files {
        read_contained_bytes(canon_root, canon_file, MAX_USER_PDF_BYTES)
            .ok()
            .map(|b| sha256_hex(&b))
    } else {
        None
    };
    let key = derive_paper_key(&ids, sha256.as_deref());
    ScannedItem {
        item: SourceItemRecord {
            source_id: source_id.to_string(),
            external_id: rel_path.to_string(),
            key,
            citekey: None,
            ids,
            version: parsed_name.arxiv_version.clone(),
            title: parsed_name.title.clone(),
            authors: parsed_name.authors.clone(),
            year: parsed_name.year,
            item_kind: "preprint".to_string(),
            collections,
            tags: Vec::new(),
            files: vec![SourceItemFile {
                path: portable,
                role: FILE_ROLE_TEX.to_string(),
                size,
                mtime_ms,
                sha256,
            }],
            state: SourceItemState::Present,
            updated_ms: ctx.now_ms,
        },
        abstract_text: String::new(),
        venue: None,
        reused_unchanged: false,
    }
}

#[derive(Debug, Default)]
struct ParsedFilename {
    title: String,
    authors: Vec<String>,
    year: Option<i32>,
    arxiv: Option<String>,
    arxiv_version: Option<String>,
}

fn parse_filename(file_name: &str) -> ParsedFilename {
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

fn collections_from_rel_dir(rel_dir: Option<&str>) -> Vec<String> {
    let Some(dir) = rel_dir.map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    vec![dir.to_string()]
}

fn extract_tex_command(text: &str, cmd: &str) -> Option<String> {
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

fn find_arxiv_in_text(text: &str) -> Option<(String, Option<String>)> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("arxiv:")?;
    let rest = &text[idx + 6..];
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '-' | '_'))
        .collect();
    split_arxiv_id(&token).ok()
}

fn find_doi_in_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find("doi:")?;
    let rest = text[idx + 4..].trim_start();
    let token: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '}' | ')' | ']' | ',' | ';'))
        .collect();
    canonical_doi(&token).ok()
}
