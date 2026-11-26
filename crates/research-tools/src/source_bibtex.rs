//! Read-only BibTeX paper source adapter (`SourceKind::Bibtex`, package F13b,
//! `00-overview.md` 2.11).
//!
//! Reads a user-chosen `.bib` file and resolves its `file` (and `pdf` / `local-url`) field
//! against the `.bib` directory and optional configured `external_pdf_roots`, supporting
//! Zotero Better BibTeX auto-export, JabRef, and Kandinsky `refs.bib`.
//! Needs no PDF first-page extraction for metadata and never modifies any file on disk.

use crate::library::{
    SourceItemFile, SourceItemRecord, SourceItemState, SourceKind, SourceRecord,
};
use crate::paper_sources::{
    canonicalize_source_root, derive_paper_key, file_meta_without_reading, hash_contained_file,
    portable_path_string, read_contained_bytes, resolve_contained_abs, resolve_contained_rel,
    PaperSourceAdapter, ScanContext, ScanOutput, ScanWarning, ScannedItem, FILE_ROLE_PDF,
    FILE_ROLE_TEX, MAX_SOURCE_TEXT_BYTES,
};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use crate::sandbox::is_symlink_or_junction;
pub use crate::source_bibtex_parse::{
    decode_tex_text, parse_bib_file_field, parse_bibtex, split_bibtex_authors, BibEntryRaw,
    BibFileField,
};
use crate::source_kandinsky::ExternalPdfRoot;
use crate::source_kandinsky_resolve::{clean_ref_token, extract_year, resolve_external_roots};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Read-only adapter for a BibTeX `.bib` file (`SourceKind::Bibtex`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BibtexAdapter;

/// Validate and canonicalize a `.bib` file path for registration as a `SourceKind::Bibtex` root.
///
/// Refuses empty/NUL paths, directories, non-`.bib` extensions, and symlinks or junctions on
/// the `.bib` file itself while canonicalizing its existing parent directory.
pub fn canonicalize_bibtex_source_root(path: &Path) -> Result<PathBuf, String> {
    let raw = path.to_string_lossy();
    if raw.trim().is_empty() || raw.contains('\0') {
        return Err("BibTeX source path must not be empty or contain NUL".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("BibTeX path `{}` must have a parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("BibTeX path `{}` has an invalid file name", path.display()))?;
    if !file_name.to_ascii_lowercase().ends_with(".bib") {
        return Err(format!(
            "BibTeX source `{}` must point to a `.bib` file",
            path.display()
        ));
    }
    let canon_parent = canonicalize_source_root(parent)?;
    let safe_file = resolve_contained_rel(&canon_parent, file_name)?;
    let md = fs::metadata(&safe_file)
        .map_err(|e| format!("cannot stat BibTeX file `{}`: {e}", safe_file.display()))?;
    if !md.is_file() {
        return Err(format!(
            "BibTeX source `{}` is not a regular file",
            safe_file.display()
        ));
    }
    Ok(safe_file)
}

/// Return all canonicalized attachment root directories allowed for a BibTeX source
/// (the `.bib` parent directory plus any configured `external_pdf_roots`).
pub fn allowed_roots_for_bibtex(bib_path: &Path, settings: &Value) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(parent) = bib_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return roots;
    };
    let Ok(bib_dir) = canonicalize_source_root(parent) else {
        return roots;
    };
    roots.push(bib_dir.clone());
    let configured = configured_external_roots(settings);
    for er in resolve_external_roots(&bib_dir, &configured) {
        if !roots.contains(&er.canon_root) {
            roots.push(er.canon_root);
        }
    }
    roots
}

fn configured_external_roots(settings: &Value) -> Vec<String> {
    settings
        .get("external_pdf_roots")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

impl PaperSourceAdapter for BibtexAdapter {
    fn kind(&self) -> SourceKind {
        SourceKind::Bibtex
    }

    fn scan(&self, source: &SourceRecord, ctx: &ScanContext) -> Result<ScanOutput, String> {
        let canon_bib = canonicalize_bibtex_source_root(Path::new(&source.root))?;
        let bib_dir = canon_bib
            .parent()
            .ok_or_else(|| "BibTeX file has no parent directory".to_string())?
            .to_path_buf();
        let ext_roots = resolve_external_roots(&bib_dir, &configured_external_roots(&source.settings));

        let mut out = ScanOutput::default();
        out.roots_checked.push(bib_dir.clone());
        for er in &ext_roots {
            if !out.roots_checked.contains(&er.canon_root) {
                out.roots_checked.push(er.canon_root.clone());
            }
        }

        let raw_bytes = read_contained_bytes(&bib_dir, &canon_bib, MAX_SOURCE_TEXT_BYTES)?;
        let raw_text = String::from_utf8_lossy(&raw_bytes);
        let (entries, parse_warnings) = parse_bibtex(&raw_text);
        let bib_label = portable_path_string(&canon_bib);
        for w in parse_warnings {
            out.warnings.push(ScanWarning {
                path: bib_label.clone(),
                reason: w,
            });
        }

        let mut seen_keys: BTreeMap<String, usize> = BTreeMap::new();
        for mut entry in entries {
            let raw_key = entry.citekey.trim().to_string();
            let count = seen_keys.entry(raw_key.clone()).or_insert(0);
            *count += 1;
            let external_id = if *count == 1 {
                raw_key.clone()
            } else {
                out.warnings.push(ScanWarning {
                    path: bib_label.clone(),
                    reason: format!("duplicate BibTeX citekey `{raw_key}` (#{})", *count),
                });
                format!("{raw_key}#{}", *count)
            };
            entry.citekey = raw_key;
            let scanned = build_bibtex_item(
                &source.id,
                &external_id,
                &bib_dir,
                &ext_roots,
                &entry,
                ctx,
                &mut out.warnings,
            );
            out.items.push(scanned);
        }

        Ok(out)
    }
}

fn build_bibtex_item(
    source_id: &str,
    external_id: &str,
    bib_dir: &Path,
    ext_roots: &[ExternalPdfRoot],
    entry: &BibEntryRaw,
    ctx: &ScanContext,
    warnings: &mut Vec<ScanWarning>,
) -> ScannedItem {
    let citekey = entry.citekey.trim().to_string();
    let prev = ctx.previous_by_id.get(external_id);
    let (ids, version) = extract_bibtex_ids(&entry.fields);

    let title = entry
        .fields
        .get("title")
        .map(|t| decode_tex_text(t))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| citekey.clone());

    let authors = entry
        .fields
        .get("author")
        .or_else(|| entry.fields.get("editor"))
        .map(|a| split_bibtex_authors(a))
        .unwrap_or_default();

    let year = entry
        .fields
        .get("year")
        .and_then(|y| extract_year(y))
        .or_else(|| entry.fields.get("date").and_then(|d| extract_year(d)));

    let venue = entry
        .fields
        .get("journal")
        .or_else(|| entry.fields.get("journaltitle"))
        .or_else(|| entry.fields.get("booktitle"))
        .map(|v| decode_tex_text(v))
        .filter(|v| !v.is_empty());

    let abstract_text = entry
        .fields
        .get("abstract")
        .map(|a| decode_tex_text(a))
        .unwrap_or_default();

    let tags = split_list_field(
        entry
            .fields
            .get("keywords")
            .or_else(|| entry.fields.get("tags")),
    );
    let collections = split_list_field(
        entry
            .fields
            .get("groups")
            .or_else(|| entry.fields.get("collections"))
            .or_else(|| entry.fields.get("mendeley-groups")),
    );
    let item_kind = normalize_bib_item_kind(&entry.entry_type);

    let mut files = Vec::new();
    let mut sha_for_key: Option<String> = None;
    let mut had_missing_in_root = false;

    for cand in collect_attachment_candidates(&entry.fields) {
        let Some(role) = attachment_role(&cand.path, &cand.mime_or_type) else {
            continue;
        };
        match resolve_bibtex_attachment(bib_dir, ext_roots, &cand.path) {
            AttachResolve::Found { root, path } => {
                let norm_path = portable_path_string(&path);
                if files.iter().any(|f: &SourceItemFile| f.path == norm_path) {
                    continue;
                }
                let Ok((size, mtime_ms)) = file_meta_without_reading(&path) else {
                    had_missing_in_root = true;
                    warnings.push(ScanWarning {
                        path: format!("{citekey}: {}", cand.path),
                        reason: "attached file could not be inspected".into(),
                    });
                    continue;
                };
                let prev_sha = prev.and_then(|p| {
                    p.files
                        .iter()
                        .find(|f| f.path == norm_path && f.size == size && f.mtime_ms == mtime_ms)
                        .and_then(|f| f.sha256.clone())
                });
                let max_bytes = if role == FILE_ROLE_PDF {
                    MAX_USER_PDF_BYTES
                } else {
                    MAX_SOURCE_TEXT_BYTES
                };
                let sha = if let Some(s) = prev_sha {
                    Some(s)
                } else if ctx.hash_files {
                    hash_contained_file(&root, &path, max_bytes).ok()
                } else {
                    None
                };
                if sha_for_key.is_none() && role == FILE_ROLE_PDF {
                    sha_for_key = sha.clone();
                }
                files.push(SourceItemFile {
                    path: norm_path,
                    role: role.to_string(),
                    size,
                    mtime_ms,
                    sha256: sha,
                });
            }
            AttachResolve::MissingInRoot(reason) => {
                had_missing_in_root = true;
                warnings.push(ScanWarning {
                    path: format!("{citekey}: {}", cand.path),
                    reason,
                });
            }
            AttachResolve::Refused(reason) => {
                warnings.push(ScanWarning {
                    path: format!("{citekey}: {}", cand.path),
                    reason,
                });
            }
        }
    }

    let state = if files.is_empty() && had_missing_in_root {
        SourceItemState::Missing
    } else {
        SourceItemState::Present
    };
    let key = derive_paper_key(&ids, sha_for_key.as_deref());

    ScannedItem {
        item: SourceItemRecord {
            source_id: source_id.to_string(),
            external_id: external_id.to_string(),
            key,
            citekey: Some(citekey),
            ids,
            version,
            title,
            authors,
            year,
            item_kind,
            collections,
            tags,
            files,
            state,
            updated_ms: ctx.now_ms,
        },
        abstract_text,
        venue,
        reused_unchanged: false,
    }
}

fn collect_attachment_candidates(fields: &BTreeMap<String, String>) -> Vec<BibFileField> {
    let mut out = Vec::new();
    for field_name in ["file", "files", "pdf", "local-url"] {
        if let Some(raw) = fields.get(field_name) {
            if field_name == "file" || field_name == "files" {
                out.extend(parse_bib_file_field(raw));
            } else {
                let cleaned = clean_ref_token(raw);
                if !cleaned.is_empty() {
                    out.push(BibFileField {
                        description: String::new(),
                        path: cleaned.to_string(),
                        mime_or_type: String::new(),
                    });
                }
            }
        }
    }
    out
}

fn attachment_role(path: &str, mime_or_type: &str) -> Option<&'static str> {
    let p_low = path.trim().to_ascii_lowercase();
    let m_low = mime_or_type.trim().to_ascii_lowercase();
    if p_low.ends_with(".pdf") || m_low == "pdf" || m_low == "application/pdf" {
        Some(FILE_ROLE_PDF)
    } else if p_low.ends_with(".tex")
        || p_low.ends_with(".tar.gz")
        || p_low.ends_with(".tgz")
        || m_low == "tex"
        || m_low == "text/x-tex"
        || m_low == "application/x-tex"
    {
        Some(FILE_ROLE_TEX)
    } else {
        None
    }
}

enum AttachResolve {
    Found { root: PathBuf, path: PathBuf },
    MissingInRoot(String),
    Refused(String),
}

fn resolve_bibtex_attachment(
    bib_dir: &Path,
    ext_roots: &[ExternalPdfRoot],
    raw_path: &str,
) -> AttachResolve {
    let trimmed = raw_path.trim();
    let stripped = strip_file_uri_prefix(trimmed);
    let norm = clean_ref_token(stripped).replace('\\', "/");
    if norm.is_empty() || norm.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return AttachResolve::Refused("empty or control-character attachment path".into());
    }

    // 1. Configured external PDF root prefix or `@ext/<idx>/` (Kandinsky `refs.bib` convention).
    for (idx, ext) in ext_roots.iter().enumerate() {
        let alias = format!("@ext/{idx}");
        if let Some(sub) = norm
            .strip_prefix(&format!("{}/", ext.prefix))
            .or_else(|| norm.strip_prefix(&format!("{alias}/")))
        {
            return resolve_rel_with_missing_check(&ext.canon_root, sub, raw_path);
        }
    }

    // 2. Absolute path (Unix `/...` or Windows `C:/...`): must be strictly inside bib_dir or ext_roots.
    if is_absolute_portable(&norm) {
        let abs = PathBuf::from(&norm);
        let all_roots = std::iter::once(bib_dir)
            .chain(ext_roots.iter().map(|e| e.canon_root.as_path()));
        for root in all_roots {
            if let Ok(safe) = resolve_contained_abs(root, &abs) {
                return if safe.is_file() {
                    AttachResolve::Found {
                        root: root.to_path_buf(),
                        path: safe,
                    }
                } else {
                    AttachResolve::Refused(format!("attachment `{raw_path}` is not a regular file"))
                };
            }
            if is_syntactically_inside_missing(root, &abs) {
                return AttachResolve::MissingInRoot(format!(
                    "attached file `{raw_path}` not found on disk"
                ));
            }
        }
        return AttachResolve::Refused(format!(
            "attachment path `{raw_path}` escapes registered roots or uses a symlink"
        ));
    }

    // 3. Relative path inside the `.bib` directory.
    resolve_rel_with_missing_check(bib_dir, &norm, raw_path)
}

fn resolve_rel_with_missing_check(root: &Path, rel: &str, display: &str) -> AttachResolve {
    // Reject `..`, leading `/`, or drive/stream colons before checking disk existence.
    let mut segs: Vec<&str> = Vec::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(os) => {
                let Some(s) = os.to_str() else {
                    return AttachResolve::Refused(format!("non-UTF-8 segment in `{display}`"));
                };
                if s.is_empty() || s == ".." || s.contains(':') {
                    return AttachResolve::Refused(format!("path `{display}` escapes registered root"));
                }
                segs.push(s);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return AttachResolve::Refused(format!("path `{display}` escapes registered root"));
            }
        }
    }
    if segs.is_empty() {
        return AttachResolve::Refused(format!("empty relative path `{display}`"));
    }
    match resolve_contained_rel(root, rel) {
        Ok(safe) => {
            if safe.is_file() {
                AttachResolve::Found {
                    root: root.to_path_buf(),
                    path: safe,
                }
            } else {
                AttachResolve::Refused(format!("attachment `{display}` is not a regular file"))
            }
        }
        Err(reason) => {
            // Check whether every existing prefix segment is a clean non-symlink inside `root`
            // and the failure is strictly a missing file (`NotFound`).
            let mut cur = root.to_path_buf();
            for (idx, seg) in segs.iter().enumerate() {
                let next = cur.join(seg);
                match fs::symlink_metadata(&next) {
                    Ok(lmeta) => {
                        if is_symlink_or_junction(&next, &lmeta) {
                            return AttachResolve::Refused(reason);
                        }
                        if idx + 1 < segs.len() && !lmeta.is_dir() {
                            return AttachResolve::Refused(reason);
                        }
                        cur = next;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return AttachResolve::MissingInRoot(format!(
                            "attached file `{display}` not found on disk"
                        ));
                    }
                    Err(_) => return AttachResolve::Refused(reason),
                }
            }
            AttachResolve::Refused(reason)
        }
    }
}

fn strip_file_uri_prefix(raw: &str) -> &str {
    let after = raw
        .strip_prefix("file://localhost")
        .or_else(|| raw.strip_prefix("file://"))
        .unwrap_or(raw);
    // Standard Windows file URI: `file:///C:/...` -> `C:/...`.
    if after.len() >= 4
        && after.as_bytes()[0] == b'/'
        && after.as_bytes()[1].is_ascii_alphabetic()
        && after.as_bytes()[2] == b':'
        && matches!(after.as_bytes()[3], b'/' | b'\\')
    {
        &after[1..]
    } else {
        after
    }
}

fn is_syntactically_inside_missing(root: &Path, cand: &Path) -> bool {
    let clean_root = crate::sandbox::strip_verbatim_prefix(root);
    let clean_cand = crate::sandbox::strip_verbatim_prefix(cand);
    let r_comps: Vec<_> = clean_root.components().collect();
    let c_comps: Vec<_> = clean_cand.components().collect();
    if c_comps.len() <= r_comps.len() {
        return false;
    }
    for (rc, cc) in r_comps.iter().zip(c_comps.iter()) {
        #[cfg(windows)]
        let eq = rc
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&cc.as_os_str().to_string_lossy());
        #[cfg(not(windows))]
        let eq = rc == cc;
        if !eq {
            return false;
        }
    }
    let r_len = r_comps.len();
    drop(r_comps);
    let mut cur = clean_root;
    for comp in &c_comps[r_len..] {
        let Component::Normal(os) = comp else {
            return false;
        };
        let next = cur.join(os);
        match fs::symlink_metadata(&next) {
            Ok(lmeta) => {
                if is_symlink_or_junction(&next, &lmeta) {
                    return false;
                }
                cur = next;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    false
}

fn is_absolute_portable(norm: &str) -> bool {
    norm.starts_with('/')
        || (norm.len() >= 3
            && norm.as_bytes()[0].is_ascii_alphabetic()
            && norm.as_bytes()[1] == b':'
            && norm.as_bytes()[2] == b'/')
}

pub(crate) use crate::source_bibtex_tex::{
    extract_bibtex_ids, normalize_bib_item_kind, parse_arxiv_from_url, split_list_field,
};

#[cfg(test)]
#[path = "source_bibtex_tests.rs"]
mod tests;
