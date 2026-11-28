// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Read-only Kandinsky vault adapter (`refs/papers/<citekey>/<citekey>.md`, legacy `info.yaml`,
//! `external_pdf_roots`, and unpacked `arxiv_source_dir` TeX sources) for package F13a
//! (`00-overview.md` 2.11).
//!
//! Only the reading half of Kandinsky's `entry.rs`, `external.rs`, and `external_roots.rs` is
//! ported here. No file inside a Kandinsky vault or external PDF root is ever written.

use crate::library::{
    SourceItemFile, SourceItemRecord, SourceItemState, SourceKind, SourceRecord,
};
use crate::paper_sources::{
    canonicalize_source_root, derive_paper_key, file_meta_without_reading, hash_contained_file,
    portable_path_string, read_contained_bytes, resolve_contained_abs, resolve_contained_rel,
    PaperSourceAdapter, ScanContext, ScanOutput, ScanWarning, ScannedItem, FILE_ROLE_NOTE,
    FILE_ROLE_PDF, MAX_SOURCE_TEXT_BYTES,
};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use crate::sandbox::is_symlink_or_junction;
use crate::source_frontmatter::{
    parse_flat_toml, parse_flat_yaml, split_frontmatter, FlatMap, FlatVal,
};
pub use crate::source_kandinsky_resolve::clean_ref_token;
use crate::source_kandinsky_resolve::{
    clean_prefix, clean_rel_dir, collect_pdf_candidates, extract_entry_ids, extract_year,
    find_unpacked_arxiv_tex, get_list, get_scalar, resolve_external_roots, resolve_optional_dir,
    resolve_vault_pdf_ref,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Default Kandinsky references directory inside a vault (`plugins/beta/catalog-papers/engine/src/settings.rs`).
pub const DEFAULT_KANDINSKY_REFS_DIR: &str = "refs/papers";
/// Default Kandinsky plugin settings file inside a vault (`crates/kandinsky-core/src/external_roots.rs`).
pub const KANDINSKY_SETTINGS_REL: &str = ".kandinsky/plugin-settings/catalog-papers.toml";
/// Default unpacked arXiv TeX cache directory inside a vault (`settings.rs`).
pub const DEFAULT_ARXIV_SOURCE_DIR: &str = ".kandinsky/cache/arxiv-src";
/// Legacy Kandinsky reference metadata file (`entry.rs`).
pub const LEGACY_INFO_YAML: &str = "info.yaml";

/// `(zotero_field, note_property)` rename table ported from Kandinsky `entry.rs::RENAMED`.
pub const RENAMED: &[(&str, &str)] = &[
    ("type", "itemType"),
    ("date", "published"),
    ("author", "authors"),
    ("tags", "keywords"),
];

/// Resolved read-only settings for a Kandinsky vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KandinskyVaultSettings {
    pub refs_dir: String,
    pub external_pdf_roots: Vec<String>,
    pub arxiv_source_dir: String,
}

impl Default for KandinskyVaultSettings {
    fn default() -> Self {
        Self {
            refs_dir: DEFAULT_KANDINSKY_REFS_DIR.to_string(),
            external_pdf_roots: Vec::new(),
            arxiv_source_dir: DEFAULT_ARXIV_SOURCE_DIR.to_string(),
        }
    }
}

/// Configured external PDF root resolved to a canonical directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalPdfRoot {
    pub prefix: String,
    pub canon_root: PathBuf,
}

/// Read-only adapter for a Kandinsky vault (`SourceKind::KandinskyVault`).
#[derive(Debug, Default, Clone, Copy)]
pub struct KandinskyVaultAdapter;

impl PaperSourceAdapter for KandinskyVaultAdapter {
    fn kind(&self) -> SourceKind {
        SourceKind::KandinskyVault
    }

    fn scan(&self, source: &SourceRecord, ctx: &ScanContext) -> Result<ScanOutput, String> {
        let canon_vault = canonicalize_source_root(Path::new(&source.root))?;
        let settings = load_vault_settings(&canon_vault, &source.settings);
        let ext_roots = resolve_external_roots(&canon_vault, &settings.external_pdf_roots);
        let arxiv_src_root = resolve_optional_dir(&canon_vault, &settings.arxiv_source_dir);

        let mut out = ScanOutput::default();
        out.roots_checked.push(canon_vault.clone());
        for er in &ext_roots {
            if !out.roots_checked.contains(&er.canon_root) {
                out.roots_checked.push(er.canon_root.clone());
            }
        }
        if let Some(ar) = &arxiv_src_root {
            if !out.roots_checked.contains(ar) {
                out.roots_checked.push(ar.clone());
            }
        }

        let refs_dir = match resolve_contained_rel(&canon_vault, &settings.refs_dir) {
            Ok(p) if p.is_dir() => p,
            Ok(_) => {
                return Err(format!(
                    "references directory `{}` is not a directory",
                    settings.refs_dir
                ));
            }
            Err(_) => {
                // Vault exists but has no refs/papers folder yet -> empty scan with warning.
                out.warnings.push(ScanWarning {
                    path: settings.refs_dir.clone(),
                    reason: "references directory not found inside vault".into(),
                });
                return Ok(out);
            }
        };

        let mut entry_dirs: Vec<(String, PathBuf)> = Vec::new();
        let rd = fs::read_dir(&refs_dir)
            .map_err(|e| format!("read_dir `{}`: {e}", refs_dir.display()))?;
        for dent in rd.flatten() {
            let name = dent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = dent.path();
            let Ok(lmeta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if is_symlink_or_junction(&path, &lmeta) {
                out.warnings.push(ScanWarning {
                    path: format!("{}/{name}", settings.refs_dir),
                    reason: "refused symlink or junction inside references folder".into(),
                });
                continue;
            }
            if !lmeta.is_dir() {
                continue;
            }
            match resolve_contained_abs(&canon_vault, &path) {
                Ok(safe_dir) => entry_dirs.push((name, safe_dir)),
                Err(reason) => out.warnings.push(ScanWarning {
                    path: format!("{}/{name}", settings.refs_dir),
                    reason,
                }),
            }
        }
        entry_dirs.sort_by(|a, b| a.0.cmp(&b.0));

        for (folder_key, entry_dir) in entry_dirs {
            if let Some(scanned) = scan_single_entry(
                &source.id,
                &canon_vault,
                &settings.refs_dir,
                &ext_roots,
                arxiv_src_root.as_deref(),
                &folder_key,
                &entry_dir,
                ctx,
                &mut out.warnings,
            ) {
                out.items.push(scanned);
            }
        }
        Ok(out)
    }
}

/// Return all canonical allowed roots (external PDF roots + arXiv TeX source dir) for a vault.
pub fn allowed_roots_for_vault(canon_vault: &Path, overrides: &Value) -> Vec<PathBuf> {
    let settings = load_vault_settings(canon_vault, overrides);
    let mut roots: Vec<PathBuf> = resolve_external_roots(canon_vault, &settings.external_pdf_roots)
        .into_iter()
        .map(|r| r.canon_root)
        .collect();
    if let Some(ar) = resolve_optional_dir(canon_vault, &settings.arxiv_source_dir) {
        if !roots.contains(&ar) {
            roots.push(ar);
        }
    }
    roots
}

/// Load vault settings from `.kandinsky/plugin-settings/catalog-papers.toml` overlaid with
/// explicit `source.settings` JSON overrides.
pub fn load_vault_settings(canon_vault: &Path, overrides: &Value) -> KandinskyVaultSettings {
    let mut cfg = KandinskyVaultSettings::default();
    if let Ok(toml_path) = resolve_contained_rel(canon_vault, KANDINSKY_SETTINGS_REL) {
        if let Ok(bytes) = read_contained_bytes(canon_vault, &toml_path, MAX_SOURCE_TEXT_BYTES) {
            if let Ok(text) = String::from_utf8(bytes) {
                let parsed = parse_flat_toml(&text);
                if let Some(d) = parsed.get("dir").and_then(FlatVal::as_str).map(clean_rel_dir).filter(|s| !s.is_empty()) {
                    cfg.refs_dir = d;
                }
                if let Some(roots) = parsed.get("external_pdf_roots") {
                    cfg.external_pdf_roots = roots
                        .as_list()
                        .into_iter()
                        .map(|s| clean_prefix(&s))
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                if let Some(ax) = parsed.get("arxiv_source_dir").and_then(FlatVal::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                    cfg.arxiv_source_dir = ax;
                }
            }
        }
    }
    if let Some(obj) = overrides.as_object() {
        for key in ["refs_dir", "dir"] {
            if let Some(d) = obj.get(key).and_then(Value::as_str).map(clean_rel_dir).filter(|s| !s.is_empty()) {
                cfg.refs_dir = d;
                break;
            }
        }
        if let Some(arr) = obj.get("external_pdf_roots").and_then(Value::as_array) {
            cfg.external_pdf_roots = arr
                .iter()
                .filter_map(Value::as_str)
                .map(clean_prefix)
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(ax) = obj.get("arxiv_source_dir").and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            cfg.arxiv_source_dir = ax;
        }
    }
    cfg
}

fn own_note_property(name: &str) -> bool {
    RENAMED.iter().any(|(z, n)| *z == name && *n != name)
}

/// Read an entry's Zotero-shaped metadata map from `info.yaml` or `<key>.md` frontmatter
/// strictly inside `canon_vault` (ported from Kandinsky `entry.rs::read`).
pub fn read_entry_metadata(
    canon_vault: &Path,
    entry_dir: &Path,
    folder_key: &str,
) -> Option<(PathBuf, FlatMap)> {
    let legacy_path = entry_dir.join(LEGACY_INFO_YAML);
    if legacy_path.exists() {
        let safe = resolve_contained_abs(canon_vault, &legacy_path).ok()?;
        let bytes = read_contained_bytes(canon_vault, &safe, MAX_SOURCE_TEXT_BYTES).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let map = parse_flat_yaml(&text);
        return (!map.is_empty()).then_some((safe, map));
    }
    let note_path = entry_dir.join(format!("{folder_key}.md"));
    let safe = resolve_contained_abs(canon_vault, &note_path).ok()?;
    let bytes = read_contained_bytes(canon_vault, &safe, MAX_SOURCE_TEXT_BYTES).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let fm = split_frontmatter(&text).0?;
    let props = parse_flat_yaml(fm);
    if !props.contains_key("citekey") && !props.contains_key("itemType") {
        return None;
    }
    let mut mapped = FlatMap::new();
    for (k, v) in props {
        if own_note_property(&k) {
            continue;
        }
        let z = RENAMED
            .iter()
            .find(|(_, n)| *n == k)
            .map_or(k.as_str(), |(z, _)| *z);
        mapped.insert(z.to_string(), v);
    }
    Some((safe, mapped))
}

#[allow(clippy::too_many_arguments)]
fn scan_single_entry(
    source_id: &str,
    canon_vault: &Path,
    refs_dir_rel: &str,
    ext_roots: &[ExternalPdfRoot],
    arxiv_src_root: Option<&Path>,
    folder_key: &str,
    entry_dir: &Path,
    ctx: &ScanContext,
    warnings: &mut Vec<ScanWarning>,
) -> Option<ScannedItem> {
    // Fast stat-only check before opening note or PDF files (cloud-drive friendly).
    let note_candidate = if entry_dir.join(LEGACY_INFO_YAML).exists() {
        entry_dir.join(LEGACY_INFO_YAML)
    } else {
        entry_dir.join(format!("{folder_key}.md"))
    };
    let safe_note = match resolve_contained_abs(canon_vault, &note_candidate) {
        Ok(p) => p,
        Err(reason) => {
            if note_candidate.symlink_metadata().is_ok() {
                warnings.push(ScanWarning {
                    path: format!("{refs_dir_rel}/{folder_key}"),
                    reason,
                });
            }
            return None;
        }
    };
    let note_meta = file_meta_without_reading(&safe_note).ok()?;
    let note_portable = portable_path_string(&safe_note);

    if let Some(prev) = ctx.previous_by_id.get(folder_key) {
        if can_reuse_previous_entry(prev, &note_portable, note_meta, ctx.hash_files) {
            return Some(ScannedItem {
                item: prev.clone(),
                abstract_text: String::new(),
                venue: None,
                reused_unchanged: true,
            });
        }
    }

    let (note_path, props) = read_entry_metadata(canon_vault, entry_dir, folder_key)?;
    let (note_size, note_mtime) = file_meta_without_reading(&note_path).ok()?;
    let citekey = get_scalar(&props, "citekey").unwrap_or_else(|| folder_key.to_string());
    let title = get_scalar(&props, "title").unwrap_or_else(|| citekey.clone());
    let authors = get_list(&props, "author");
    let year = get_scalar(&props, "date")
        .or_else(|| get_scalar(&props, "year"))
        .and_then(|d| extract_year(&d));
    let item_kind = get_scalar(&props, "type").unwrap_or_else(|| "journalArticle".into());
    let collections = get_list(&props, "collections");
    let tags = get_list(&props, "tags");
    let abstract_text = get_scalar(&props, "abstractNote").unwrap_or_default();
    let venue = get_scalar(&props, "publicationTitle").or_else(|| get_scalar(&props, "journal"));

    let (ids, version) = extract_entry_ids(&props);
    let mut files = vec![SourceItemFile {
        path: portable_path_string(&note_path),
        role: FILE_ROLE_NOTE.to_string(),
        size: note_size,
        mtime_ms: note_mtime,
        sha256: None,
    }];

    let prev_sha_by_path: std::collections::BTreeMap<&str, (&str, u64, i64)> = ctx
        .previous_by_id
        .get(folder_key)
        .map(|prev| {
            prev.files
                .iter()
                .filter_map(|f| Some((f.path.as_str(), (f.sha256.as_deref()?, f.size, f.mtime_ms))))
                .collect()
        })
        .unwrap_or_default();

    let mut pdf_sha_for_key: Option<String> = get_scalar(&props, "pdfSha256")
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));

    for raw_ref in collect_pdf_candidates(&props) {
        match resolve_vault_pdf_ref(canon_vault, entry_dir, ext_roots, &raw_ref) {
            Ok((root_for_file, abs_pdf)) => {
                if let Ok((size, mtime_ms)) = file_meta_without_reading(&abs_pdf) {
                    let portable = portable_path_string(&abs_pdf);
                    let mut sha = prev_sha_by_path
                        .get(portable.as_str())
                        .filter(|(_, ps, pm)| *ps == size && *pm == mtime_ms)
                        .map(|(h, _, _)| (*h).to_string())
                        .or_else(|| pdf_sha_for_key.clone());
                    if sha.is_none() && ctx.hash_files {
                        if let Ok(h) = hash_contained_file(&root_for_file, &abs_pdf, MAX_USER_PDF_BYTES) {
                            sha = Some(h);
                        }
                    }
                    if pdf_sha_for_key.is_none() {
                        pdf_sha_for_key = sha.clone();
                    }
                    files.push(SourceItemFile {
                        path: portable,
                        role: FILE_ROLE_PDF.to_string(),
                        size,
                        mtime_ms,
                        sha256: sha,
                    });
                }
            }
            Err(reason) => {
                warnings.push(ScanWarning {
                    path: format!("{refs_dir_rel}/{folder_key}: {raw_ref}"),
                    reason,
                });
            }
        }
    }

    // Check for unpacked arXiv TeX source (`arxiv_source_dir/<id>/`).
    if let (Some(ax_root), Some(ax_id)) = (arxiv_src_root, ids.arxiv.as_deref()) {
        if let Some(tex_file) = find_unpacked_arxiv_tex(ax_root, ax_id, version.as_deref(), ctx.hash_files) {
            files.push(tex_file);
        }
    }

    let key = derive_paper_key(&ids, pdf_sha_for_key.as_deref());
    Some(ScannedItem {
        item: SourceItemRecord {
            source_id: source_id.to_string(),
            external_id: folder_key.to_string(),
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
            state: SourceItemState::Present,
            updated_ms: ctx.now_ms,
        },
        abstract_text,
        venue,
        reused_unchanged: false,
    })
}

fn can_reuse_previous_entry(
    prev: &SourceItemRecord,
    note_path: &str,
    note_meta: (u64, i64),
    hash_files: bool,
) -> bool {
    if prev.state == SourceItemState::Missing {
        return false;
    }
    let Some(prev_note) = prev.files.iter().find(|f| f.role == FILE_ROLE_NOTE) else {
        return false;
    };
    if prev_note.path != note_path || (prev_note.size, prev_note.mtime_ms) != note_meta {
        return false;
    }
    for f in prev.files.iter().filter(|f| f.role != FILE_ROLE_NOTE) {
        let p = Path::new(&f.path);
        let Ok(cur) = file_meta_without_reading(p) else {
            return false;
        };
        if cur != (f.size, f.mtime_ms) {
            return false;
        }
        if hash_files && f.role == FILE_ROLE_PDF && f.sha256.is_none() {
            return false;
        }
    }
    true
}
