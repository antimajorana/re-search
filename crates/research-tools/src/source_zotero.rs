// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Read-only Zotero paper source adapter (`SourceKind::Zotero`, package F13b,
//! `00-overview.md` 2.11).
//!
//! Delegates all SQLite access to [`ledger::zotero_reader`] (keeping `rusqlite` strictly in
//! `crates/ledger`), resolves `storage:` and `attachments:` paths strictly inside allowed
//! roots via F13a's containment helpers, preserves pinned and Better BibTeX citation keys,
//! and maps items to F2 [`crate::paper_key::PaperKey`]. Never writes to Zotero or any vault.

use crate::library::{
    SourceItemFile, SourceItemRecord, SourceItemState, SourceKind, SourceRecord,
};
use crate::paper_sources::{
    canonicalize_source_root, derive_paper_key, file_meta_without_reading, hash_contained_file,
    portable_path_string, resolve_contained_abs, resolve_contained_rel, PaperSourceAdapter,
    ScanContext, ScanOutput, ScanWarning, ScannedItem, FILE_ROLE_PDF, FILE_ROLE_TEX,
    MAX_SOURCE_TEXT_BYTES,
};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use crate::sandbox::is_symlink_or_junction;
use crate::source_kandinsky_resolve::{extract_year, resolve_external_roots, resolve_optional_dir};
pub use crate::source_zotero_parse::{
    extra_citekey, first_author_last, ident_fold, parse_prefs_js, sanitize_citekey, ZoteroPrefs,
};
use crate::source_zotero_parse::extract_zotero_ids;
use ledger::zotero_reader::{read_zotero_library, ZoteroAttachmentRow, ZoteroItemRow};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Read-only adapter for a Zotero 6/7/8 data directory (`SourceKind::Zotero`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ZoteroAdapter;

/// Validate and canonicalize a Zotero data directory (or direct path to `zotero.sqlite`).
///
/// Always verifies that `zotero.sqlite` exists as a regular non-symlink file inside the
/// canonicalized data directory.
pub fn canonicalize_zotero_source_root(root: &Path) -> Result<PathBuf, String> {
    let raw = root.to_string_lossy();
    if raw.trim().is_empty() || raw.contains('\0') {
        return Err("Zotero source path must not be empty or contain NUL".into());
    }
    let candidate_dir = if root.is_file()
        && root
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("zotero.sqlite"))
    {
        root.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| format!("Zotero path `{}` has no parent directory", root.display()))?
    } else {
        root
    };
    let canon_dir = canonicalize_source_root(candidate_dir)?;
    let safe_db = resolve_contained_rel(&canon_dir, "zotero.sqlite").map_err(|e| {
        format!(
            "zotero.sqlite not found or invalid in `{}`: {e}",
            canon_dir.display()
        )
    })?;
    let md = fs::metadata(&safe_db)
        .map_err(|e| format!("cannot stat `{}`: {e}", safe_db.display()))?;
    if !md.is_file() {
        return Err(format!("`{}` is not a regular file", safe_db.display()));
    }
    Ok(canon_dir)
}

/// Return all canonicalized directories allowed for Zotero attachments (`data_dir`, optional
/// `base_attachment_path`, and optional `external_pdf_roots`).
pub fn allowed_roots_for_zotero(root: &Path, settings: &Value) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Ok(canon_dir) = canonicalize_zotero_source_root(root) else {
        return roots;
    };
    roots.push(canon_dir.clone());
    if let Some(base_att) = resolve_base_attachment_dir(&canon_dir, settings) {
        if !roots.contains(&base_att) {
            roots.push(base_att);
        }
    }
    let configured = configured_external_roots(settings);
    for er in resolve_external_roots(&canon_dir, &configured) {
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

fn resolve_base_attachment_dir(canon_dir: &Path, settings: &Value) -> Option<PathBuf> {
    if let Some(raw) = settings
        .get("base_attachment_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return resolve_optional_dir(canon_dir, raw);
    }
    if let Some(prefs_raw) = settings
        .get("prefs_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let p = Path::new(prefs_raw);
        if let Ok(meta) = fs::symlink_metadata(p) {
            if !meta.file_type().is_symlink() && meta.is_file() && meta.len() <= MAX_SOURCE_TEXT_BYTES as u64 {
                if let Ok(txt) = fs::read_to_string(p) {
                    let prefs = parse_prefs_js(&txt);
                    let raw = prefs.base_attachment_path.or(prefs.zotmoov_dst_dir)?;
                    return resolve_optional_dir(canon_dir, &raw);
                }
            }
        }
    }
    None
}

impl PaperSourceAdapter for ZoteroAdapter {
    fn kind(&self) -> SourceKind {
        SourceKind::Zotero
    }

    fn scan(&self, source: &SourceRecord, ctx: &ScanContext) -> Result<ScanOutput, String> {
        let canon_dir = canonicalize_zotero_source_root(Path::new(&source.root))?;
        let base_att = resolve_base_attachment_dir(&canon_dir, &source.settings);
        let ext_roots: Vec<PathBuf> =
            resolve_external_roots(&canon_dir, &configured_external_roots(&source.settings))
                .into_iter()
                .map(|e| e.canon_root)
                .collect();

        let mut out = ScanOutput::default();
        out.roots_checked.push(canon_dir.clone());
        if let Some(b) = &base_att {
            if !out.roots_checked.contains(b) {
                out.roots_checked.push(b.clone());
            }
        }
        for er in &ext_roots {
            if !out.roots_checked.contains(er) {
                out.roots_checked.push(er.clone());
            }
        }

        let snap = read_zotero_library(&canon_dir)?;
        for row in &snap.items {
            let scanned = build_zotero_item(
                &source.id,
                &canon_dir,
                base_att.as_deref(),
                &ext_roots,
                row,
                ctx,
                &mut out.warnings,
            );
            out.items.push(scanned);
        }
        Ok(out)
    }
}

fn build_zotero_item(
    source_id: &str,
    canon_dir: &Path,
    base_att: Option<&Path>,
    ext_roots: &[PathBuf],
    row: &ZoteroItemRow,
    ctx: &ScanContext,
    warnings: &mut Vec<ScanWarning>,
) -> ScannedItem {
    let prev = ctx.previous_by_id.get(&row.zotero_key);
    let (ids, version) = extract_zotero_ids(&row.fields);

    // Pinned Extra citation key takes precedence, then Better BibTeX citekey (`00-overview.md` 2.11).
    let citekey = row
        .fields
        .get("extra")
        .and_then(|e| extra_citekey(e))
        .or_else(|| {
            row.bbt_citekey
                .as_deref()
                .map(sanitize_citekey)
                .filter(|c| !c.is_empty())
        });

    let title = row
        .fields
        .get("title")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| citekey.clone().unwrap_or_else(|| row.zotero_key.clone()));

    let authors = row
        .creators
        .get("author")
        .or_else(|| row.creators.get("editor"))
        .cloned()
        .unwrap_or_default();

    let year = row.fields.get("date").and_then(|d| extract_year(d));

    let venue = ["publicationTitle", "proceedingsTitle", "bookTitle", "journalAbbreviation"]
        .into_iter()
        .find_map(|k| {
            row.fields
                .get(k)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    let abstract_text = row
        .fields
        .get("abstractNote")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut files = Vec::new();
    let mut pdf_sha_for_key: Option<String> = None;
    let mut had_missing_in_root = false;

    for att in &row.attachments {
        let Some(role) = zotero_attachment_role(att) else {
            continue;
        };
        match resolve_zotero_attachment(canon_dir, base_att, ext_roots, att) {
            ZotAttachResolve::Found { root, path } => {
                let norm_path = portable_path_string(&path);
                if files.iter().any(|f: &SourceItemFile| f.path == norm_path) {
                    continue;
                }
                let Ok((size, mtime_ms)) = file_meta_without_reading(&path) else {
                    had_missing_in_root = true;
                    warnings.push(ScanWarning {
                        path: format!("{}: {}", row.zotero_key, att.raw_path),
                        reason: "Zotero attachment could not be inspected".into(),
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
                if pdf_sha_for_key.is_none() && role == FILE_ROLE_PDF {
                    pdf_sha_for_key = sha.clone();
                }
                files.push(SourceItemFile {
                    path: norm_path,
                    role: role.to_string(),
                    size,
                    mtime_ms,
                    sha256: sha,
                });
            }
            ZotAttachResolve::MissingInRoot(reason) => {
                had_missing_in_root = true;
                warnings.push(ScanWarning {
                    path: format!("{}: {}", row.zotero_key, att.raw_path),
                    reason,
                });
            }
            ZotAttachResolve::Refused(reason) => {
                warnings.push(ScanWarning {
                    path: format!("{}: {}", row.zotero_key, att.raw_path),
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
    let key = derive_paper_key(&ids, pdf_sha_for_key.as_deref());

    ScannedItem {
        item: SourceItemRecord {
            source_id: source_id.to_string(),
            external_id: row.zotero_key.clone(),
            key,
            citekey,
            ids,
            version,
            title,
            authors,
            year,
            item_kind: row.item_type.clone(),
            collections: row.collections.clone(),
            tags: row.tags.clone(),
            files,
            state,
            updated_ms: ctx.now_ms,
        },
        abstract_text,
        venue,
        reused_unchanged: false,
    }
}

fn zotero_attachment_role(att: &ZoteroAttachmentRow) -> Option<&'static str> {
    let ct = att.content_type.trim().to_ascii_lowercase();
    let p = att.raw_path.trim().to_ascii_lowercase();
    if ct == "application/pdf" || p.ends_with(".pdf") {
        Some(FILE_ROLE_PDF)
    } else if ct == "text/x-tex"
        || ct == "application/x-tex"
        || p.ends_with(".tex")
        || p.ends_with(".tar.gz")
        || p.ends_with(".tgz")
    {
        Some(FILE_ROLE_TEX)
    } else {
        None
    }
}

enum ZotAttachResolve {
    Found { root: PathBuf, path: PathBuf },
    MissingInRoot(String),
    Refused(String),
}

fn resolve_zotero_attachment(
    canon_dir: &Path,
    base_att: Option<&Path>,
    ext_roots: &[PathBuf],
    att: &ZoteroAttachmentRow,
) -> ZotAttachResolve {
    let norm = att.raw_path.trim().replace('\\', "/");
    if norm.is_empty() || norm.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return ZotAttachResolve::Refused("empty or control-character Zotero attachment path".into());
    }
    if let Some(rel) = norm.strip_prefix("attachments:") {
        let Some(base) = base_att else {
            return ZotAttachResolve::Refused(
                "linked attachment requires configured `base_attachment_path`".into(),
            );
        };
        let clean_rel = rel.trim_start_matches('/');
        return resolve_rel_contained(base, clean_rel, &att.raw_path);
    }
    if let Some(raw_storage) = norm.strip_prefix("storage:") {
        // Validate attachment_key is a single clean segment (never `..` or slash).
        let akey = att.attachment_key.trim();
        if akey.is_empty()
            || akey.contains(['/', '\\', ':', '\0'])
            || akey == "."
            || akey == ".."
        {
            return ZotAttachResolve::Refused(format!(
                "unsafe Zotero attachment key `{akey}`"
            ));
        }
        let rel = format!("storage/{akey}/{}", raw_storage.trim_start_matches('/'));
        return resolve_rel_contained(canon_dir, &rel, &att.raw_path);
    }
    // Absolute path: allowed ONLY if strictly contained in canon_dir, base_att, or ext_roots.
    let abs = PathBuf::from(&norm);
    let is_abs = norm.starts_with('/')
        || (norm.len() >= 3
            && norm.as_bytes()[0].is_ascii_alphabetic()
            && norm.as_bytes()[1] == b':'
            && norm.as_bytes()[2] == b'/');
    if !is_abs {
        return ZotAttachResolve::Refused(format!(
            "unrecognized Zotero attachment path `{}`",
            att.raw_path
        ));
    }
    let all_roots = std::iter::once(canon_dir)
        .chain(base_att)
        .chain(ext_roots.iter().map(PathBuf::as_path));
    for root in all_roots {
        if let Ok(safe) = resolve_contained_abs(root, &abs) {
            return if safe.is_file() {
                ZotAttachResolve::Found {
                    root: root.to_path_buf(),
                    path: safe,
                }
            } else {
                ZotAttachResolve::Refused(format!(
                    "Zotero attachment `{}` is not a regular file",
                    att.raw_path
                ))
            };
        }
    }
    ZotAttachResolve::Refused(format!(
        "Zotero attachment `{}` escapes allowed roots or uses a symlink",
        att.raw_path
    ))
}

fn resolve_rel_contained(root: &Path, rel: &str, display: &str) -> ZotAttachResolve {
    let mut segs: Vec<&str> = Vec::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(os) => {
                let Some(s) = os.to_str() else {
                    return ZotAttachResolve::Refused(format!("non-UTF-8 segment in `{display}`"));
                };
                if s.is_empty() || s == ".." || s.contains(':') {
                    return ZotAttachResolve::Refused(format!(
                        "Zotero attachment `{display}` escapes registered root"
                    ));
                }
                segs.push(s);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return ZotAttachResolve::Refused(format!(
                    "Zotero attachment `{display}` escapes registered root"
                ));
            }
        }
    }
    if segs.is_empty() {
        return ZotAttachResolve::Refused(format!("empty Zotero attachment `{display}`"));
    }
    match resolve_contained_rel(root, rel) {
        Ok(safe) => {
            if safe.is_file() {
                ZotAttachResolve::Found {
                    root: root.to_path_buf(),
                    path: safe,
                }
            } else {
                ZotAttachResolve::Refused(format!(
                    "Zotero attachment `{display}` is not a regular file"
                ))
            }
        }
        Err(reason) => {
            let mut cur = root.to_path_buf();
            for (idx, seg) in segs.iter().enumerate() {
                let next = cur.join(seg);
                match fs::symlink_metadata(&next) {
                    Ok(lmeta) => {
                        if is_symlink_or_junction(&next, &lmeta) {
                            return ZotAttachResolve::Refused(reason);
                        }
                        if idx + 1 < segs.len() && !lmeta.is_dir() {
                            return ZotAttachResolve::Refused(reason);
                        }
                        cur = next;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return ZotAttachResolve::MissingInRoot(format!(
                            "Zotero attachment `{display}` not found on disk"
                        ));
                    }
                    Err(_) => return ZotAttachResolve::Refused(reason),
                }
            }
            ZotAttachResolve::Refused(reason)
        }
    }
}

#[cfg(test)]
#[path = "source_zotero_tests.rs"]
mod tests;
