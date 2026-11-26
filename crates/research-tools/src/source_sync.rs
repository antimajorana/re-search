// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! One-way read-only source registration, dry-run preview, incremental sync, moved-file
//! detection by SHA-256, missing-file retention, and F10 library indexing (package F13a,
//! `00-overview.md` 2.11).

use crate::library::{
    FulltextRecord, Library, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState,
    SourceKind, SourceRecord, ORIGIN_USER_LIBRARY,
};
use crate::paper_key::PaperKey;
use crate::paper_sources::{
    canonicalize_source_root, format_from_path_and_role, hash_contained_file, now_ms,
    portable_path_string, resolve_contained_abs, PaperSourceAdapter, ScanContext, ScanWarning,
    ScannedItem, TitleLookupHook, FILE_ROLE_NOTE, FILE_ROLE_PDF, FILE_ROLE_TEX,
    SOURCE_STATE_PENDING, SOURCE_STATE_SYNCED,
};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use crate::source_dedupe::{find_duplicates, DuplicateGroup};
use crate::source_folder::FolderAdapter;
use crate::source_kandinsky::KandinskyVaultAdapter;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Options controlling [`sync_source`].
#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    /// When `true`, scans and reconciles without writing anything to `library.db`.
    pub dry_run: bool,
    /// When `true`, hashes unhashed PDF/TeX files inside the source root (for background indexing
    /// or initial move-tracking). Default `false` avoids reading unchanged cloud-drive files.
    pub hash_files: bool,
}

/// Summary report returned by [`sync_source`] (and previewed on first sync before accepting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub source_id: String,
    pub kind: SourceKind,
    pub dry_run: bool,
    pub total_items: usize,
    pub with_pdf: usize,
    pub with_tex: usize,
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub missing: usize,
    pub unchanged: usize,
    pub exact_duplicates: usize,
    pub possible_duplicates: usize,
    pub unreadable: usize,
    pub warnings: Vec<ScanWarning>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub items: Vec<SourceItemRecord>,
}

/// Register a new read-only paper source in F10's `source` table without writing to disk.
pub fn register_source(
    lib: &Library,
    kind: SourceKind,
    root: &Path,
    settings: Value,
    custom_id: Option<String>,
) -> Result<SourceRecord, String> {
    if !matches!(kind, SourceKind::KandinskyVault | SourceKind::Folder) {
        return Err(format!(
            "source kind `{}` is not supported in F13a (planned for F13b)",
            kind.as_str()
        ));
    }
    let canon_root = canonicalize_source_root(root)?;
    let root_str = portable_path_string(&canon_root);
    let id = match custom_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(explicit) => validate_source_id(&explicit)?,
        None => derive_source_id(kind, &root_str),
    };
    let rec = SourceRecord {
        id,
        kind,
        root: root_str,
        settings: if settings.is_null() {
            serde_json::json!({})
        } else {
            settings
        },
        added_ms: now_ms(),
        last_sync_ms: None,
        state: SOURCE_STATE_PENDING.to_string(),
    };
    lib.upsert_source(&rec)?;
    Ok(rec)
}

/// Remove a registered source and its indexed rows from `library.db`.
/// Never touches any file on disk or any session ledger/blob.
pub fn remove_source(lib: &Library, source_id: &str) -> Result<bool, String> {
    let items = lib.list_source_items_for_source(source_id)?;
    let mut paths = Vec::new();
    for item in &items {
        for f in &item.files {
            if f.role != FILE_ROLE_NOTE {
                paths.push(f.path.clone());
            }
        }
    }
    if !paths.is_empty() {
        let _ = lib.delete_user_library_fulltext_for_paths(&paths)?;
    }
    lib.delete_source(source_id)
}

/// Run a read-only sync (or dry-run preview) for one registered source.
pub fn sync_source(
    lib: &Library,
    source_id: &str,
    opts: &SyncOptions,
) -> Result<SyncReport, String> {
    sync_source_with_hook(lib, source_id, opts, None)
}

/// Run a read-only sync with an optional approval-gated [`TitleLookupHook`] (unused/`None` in F13a).
pub fn sync_source_with_hook(
    lib: &Library,
    source_id: &str,
    opts: &SyncOptions,
    _title_lookup: Option<&dyn TitleLookupHook>,
) -> Result<SyncReport, String> {
    let source = lib
        .list_sources()?
        .into_iter()
        .find(|s| s.id == source_id)
        .ok_or_else(|| format!("paper source `{source_id}` not found"))?;

    let previous = lib.list_source_items_for_source(source_id)?;
    let ts = now_ms();
    let ctx = ScanContext::from_previous(&previous, opts.hash_files, ts);

    let scan = match source.kind {
        SourceKind::KandinskyVault => KandinskyVaultAdapter.scan(&source, &ctx)?,
        SourceKind::Folder => FolderAdapter.scan(&source, &ctx)?,
        other => {
            return Err(format!(
                "source kind `{}` is not supported in F13a (planned for F13b)",
                other.as_str()
            ));
        }
    };

    let reconciled = reconcile_items(&previous, scan.items, &scan.roots_checked, ts);

    // Combine this source's items with items from other registered sources for cross-source dedupe.
    let other_items: Vec<SourceItemRecord> = lib
        .list_all_source_items()?
        .into_iter()
        .filter(|it| it.source_id != source_id)
        .collect();
    let mine_count = reconciled.items.len();
    let mut combined: Vec<SourceItemRecord> = reconciled
        .items
        .iter()
        .map(|entry| entry.item.clone())
        .collect();
    combined.extend(other_items);

    let dedupe = find_duplicates(&combined);

    // Apply exact dedupe canonical keys and possible-duplicate flags to our items.
    let mut final_entries = reconciled.items;
    let mut exact_merges: Vec<(PaperKey, Vec<PaperKey>, crate::library::LibraryIds)> = Vec::new();
    for grp in &dedupe.exact_groups {
        if let Some(canon) = &grp.canonical_key {
            for &idx in &grp.member_indices {
                if idx < mine_count {
                    final_entries[idx].item.key = Some(canon.clone());
                    final_entries[idx].item.ids.merge_from(&grp.merged_ids);
                    if final_entries[idx].item.state == SourceItemState::PossibleDuplicate {
                        final_entries[idx].item.state = SourceItemState::Present;
                    }
                }
            }
            exact_merges.push((canon.clone(), grp.alias_keys.clone(), grp.merged_ids.clone()));
        }
    }

    let mut possible_indices = BTreeSet::new();
    for grp in &dedupe.possible_groups {
        for &idx in &grp.member_indices {
            if idx < mine_count && final_entries[idx].item.state != SourceItemState::Missing {
                possible_indices.insert(idx);
                if final_entries[idx].item.state != SourceItemState::PossibleDuplicate {
                    final_entries[idx].item.state = SourceItemState::PossibleDuplicate;
                    final_entries[idx].dirty = true;
                }
            }
        }
    }
    // Clear stale possible_duplicate flag if no longer a possible duplicate.
    for (idx, entry) in final_entries.iter_mut().enumerate() {
        if entry.item.state == SourceItemState::PossibleDuplicate && !possible_indices.contains(&idx)
        {
            entry.item.state = SourceItemState::Present;
            entry.dirty = true;
        }
    }

    if !opts.dry_run {
        persist_sync(lib, &source, &final_entries, &exact_merges, ts)?;
    }

    let items_out: Vec<SourceItemRecord> = final_entries.into_iter().map(|e| e.item).collect();
    let with_pdf = items_out
        .iter()
        .filter(|it| {
            it.state != SourceItemState::Missing
                && it.files.iter().any(|f| f.role == FILE_ROLE_PDF)
        })
        .count();
    let with_tex = items_out
        .iter()
        .filter(|it| {
            it.state != SourceItemState::Missing
                && it.files.iter().any(|f| f.role == FILE_ROLE_TEX)
        })
        .count();

    let unreadable = scan.warnings.len();
    Ok(SyncReport {
        source_id: source.id,
        kind: source.kind,
        dry_run: opts.dry_run,
        total_items: items_out.len(),
        with_pdf,
        with_tex,
        added: reconciled.added,
        updated: reconciled.updated,
        moved: reconciled.moved,
        missing: reconciled.missing,
        unchanged: reconciled.unchanged,
        exact_duplicates: dedupe.exact_groups.len(),
        possible_duplicates: dedupe.possible_groups.len(),
        unreadable,
        warnings: scan.warnings,
        duplicate_groups: dedupe.all_groups(),
        items: items_out,
    })
}

/// Sync all registered sources that F13a supports.
pub fn sync_all_sources(lib: &Library, opts: &SyncOptions) -> Result<Vec<SyncReport>, String> {
    let mut reports = Vec::new();
    for src in lib.list_sources()? {
        if matches!(src.kind, SourceKind::KandinskyVault | SourceKind::Folder) {
            reports.push(sync_source(lib, &src.id, opts)?);
        }
    }
    Ok(reports)
}

struct ReconciledEntry {
    item: SourceItemRecord,
    abstract_text: String,
    venue: Option<String>,
    dirty: bool,
}

struct ReconcileOutcome {
    items: Vec<ReconciledEntry>,
    added: usize,
    updated: usize,
    moved: usize,
    missing: usize,
    unchanged: usize,
}

fn reconcile_items(
    previous: &[SourceItemRecord],
    scanned: Vec<ScannedItem>,
    roots_checked: &[PathBuf],
    now_ms: i64,
) -> ReconcileOutcome {
    let mut prev_by_id: BTreeMap<String, SourceItemRecord> = previous
        .iter()
        .map(|it| (it.external_id.clone(), it.clone()))
        .collect();
    let mut prev_id_by_path: BTreeMap<String, String> = BTreeMap::new();
    for it in previous {
        for f in &it.files {
            if f.role != FILE_ROLE_NOTE {
                prev_id_by_path.insert(f.path.clone(), it.external_id.clone());
            }
        }
    }

    let mut matched_prev_ids: BTreeSet<String> = BTreeSet::new();
    let mut out_entries: Vec<ReconciledEntry> = Vec::new();
    let mut unmatched_new: Vec<ScannedItem> = Vec::new();
    let mut added = 0usize;
    let mut updated = 0usize;
    let mut moved = 0usize;
    let mut missing = 0usize;
    let mut unchanged = 0usize;

    // Pass 1: Match by external_id or existing file path.
    for mut sc in scanned {
        let matched_id = if prev_by_id.contains_key(&sc.item.external_id) {
            Some(sc.item.external_id.clone())
        } else {
            sc.item
                .files
                .iter()
                .filter(|f| f.role != FILE_ROLE_NOTE)
                .find_map(|f| prev_id_by_path.get(&f.path).cloned())
        };

        if let Some(pid) = matched_id {
            matched_prev_ids.insert(pid.clone());
            let prev = prev_by_id.remove(&pid).unwrap();
            // Keep stable external_id for items matched by current path after an earlier move.
            sc.item.external_id = pid;
            if sc.reused_unchanged && prev.state != SourceItemState::Missing {
                unchanged += 1;
                out_entries.push(ReconciledEntry {
                    item: prev,
                    abstract_text: String::new(),
                    venue: None,
                    dirty: false,
                });
            } else if same_item_content(&prev, &sc.item) && prev.state != SourceItemState::Missing {
                unchanged += 1;
                let mut kept = sc.item;
                kept.updated_ms = prev.updated_ms;
                out_entries.push(ReconciledEntry {
                    item: kept,
                    abstract_text: sc.abstract_text,
                    venue: sc.venue,
                    dirty: false,
                });
            } else {
                updated += 1;
                out_entries.push(ReconciledEntry {
                    item: sc.item,
                    abstract_text: sc.abstract_text,
                    venue: sc.venue,
                    dirty: true,
                });
            }
        } else {
            unmatched_new.push(sc);
        }
    }

    // Pass 2: Detect moved files by SHA-256 (same SHA-256 at a new path keeps the existing item).
    let mut absent_prev: Vec<SourceItemRecord> = prev_by_id.into_values().collect();
    let mut consumed_absent: BTreeSet<String> = BTreeSet::new();

    for mut cand in unmatched_new {
        let cand_file = cand
            .item
            .files
            .iter()
            .find(|f| f.role != FILE_ROLE_NOTE)
            .cloned();
        let mut matched_prev_idx: Option<usize> = None;

        if let Some(cf) = cand_file {
            // Only compute hash for a candidate if an absent item has a known SHA-256 with matching size.
            let mut cand_sha = cf.sha256.clone();
            for (idx, abs_item) in absent_prev.iter().enumerate() {
                if consumed_absent.contains(&abs_item.external_id) {
                    continue;
                }
                let Some(prev_sha) = primary_file_sha_and_size(abs_item) else {
                    continue;
                };
                if prev_sha.1 != cf.size {
                    continue;
                }
                if cand_sha.is_none() {
                    cand_sha = hash_in_any_root(roots_checked, Path::new(&cf.path));
                }
                if cand_sha.as_deref() == Some(prev_sha.0) {
                    matched_prev_idx = Some(idx);
                    break;
                }
            }
            if let Some(sha) = cand_sha {
                if let Some(f) = cand.item.files.iter_mut().find(|f| f.path == cf.path) {
                    f.sha256 = Some(sha);
                }
            }
        }

        if let Some(idx) = matched_prev_idx {
            let mut prev = absent_prev[idx].clone();
            consumed_absent.insert(prev.external_id.clone());
            // Keep existing item identity; update path/files/collections/state.
            prev.files = cand.item.files;
            prev.collections = cand.item.collections;
            if prev.key.is_none() {
                prev.key = cand.item.key;
            }
            if prev.title.is_empty() {
                prev.title = cand.item.title;
            }
            prev.state = SourceItemState::Present;
            prev.updated_ms = now_ms;
            moved += 1;
            out_entries.push(ReconciledEntry {
                item: prev,
                abstract_text: cand.abstract_text,
                venue: cand.venue,
                dirty: true,
            });
        } else {
            added += 1;
            out_entries.push(ReconciledEntry {
                item: cand.item,
                abstract_text: cand.abstract_text,
                venue: cand.venue,
                dirty: true,
            });
        }
    }

    // Pass 3: Remaining absent items become `Missing` (stay listed for sessions that refer to them).
    absent_prev.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    for mut abs_item in absent_prev {
        if consumed_absent.contains(&abs_item.external_id) {
            continue;
        }
        if abs_item.state == SourceItemState::Missing {
            unchanged += 1;
            out_entries.push(ReconciledEntry {
                item: abs_item,
                abstract_text: String::new(),
                venue: None,
                dirty: false,
            });
        } else {
            abs_item.state = SourceItemState::Missing;
            abs_item.updated_ms = now_ms;
            missing += 1;
            out_entries.push(ReconciledEntry {
                item: abs_item,
                abstract_text: String::new(),
                venue: None,
                dirty: true,
            });
        }
    }

    out_entries.sort_by(|a, b| a.item.external_id.cmp(&b.item.external_id));
    ReconcileOutcome {
        items: out_entries,
        added,
        updated,
        moved,
        missing,
        unchanged,
    }
}

fn persist_sync(
    lib: &Library,
    source: &SourceRecord,
    entries: &[ReconciledEntry],
    exact_merges: &[(PaperKey, Vec<PaperKey>, crate::library::LibraryIds)],
    ts: i64,
) -> Result<(), String> {
    for entry in entries {
        if entry.dirty {
            lib.upsert_source_item(&entry.item)?;
        }
        if entry.item.state == SourceItemState::Missing {
            continue;
        }
        let Some(key) = &entry.item.key else {
            continue;
        };
        if entry.dirty || lib.get_paper(key)?.is_none() {
            upsert_paper_from_entry(lib, key, entry, ts)?;
        }
        if entry.dirty {
            upsert_user_fulltext_from_item(lib, key, &entry.item, ts)?;
        }
    }

    // Register exact-duplicate aliases in F10's `alias` table and merge their IDs into the canonical `paper`.
    for (canon, aliases, merged_ids) in exact_merges {
        if let Some(detail) = lib.get_paper(canon)? {
            let mut paper = detail.paper;
            paper.ids.merge_from(merged_ids);
            paper.flags.owned = true;
            lib.upsert_paper(&paper)?;
        }
        for alias in aliases {
            let _ = lib.put_alias(alias, canon);
        }
    }

    let mut updated_src = source.clone();
    updated_src.last_sync_ms = Some(ts);
    updated_src.state = SOURCE_STATE_SYNCED.to_string();
    lib.upsert_source(&updated_src)
}

fn upsert_paper_from_entry(
    lib: &Library,
    key: &PaperKey,
    entry: &ReconciledEntry,
    ts: i64,
) -> Result<(), String> {
    let existing = lib.get_paper(key)?.map(|d| d.paper);
    let mut ids = existing
        .as_ref()
        .map(|p| p.ids.clone())
        .unwrap_or_default();
    ids.merge_from(&entry.item.ids);
    let mut flags = existing
        .as_ref()
        .map(|p| p.flags.clone())
        .unwrap_or_else(PaperFlags::default);
    flags.owned = true;
    if flags.venue.is_none() {
        flags.venue = entry.venue.clone();
    }

    let title = if !entry.item.title.trim().is_empty() {
        entry.item.title.clone()
    } else {
        existing.as_ref().map(|p| p.title.clone()).unwrap_or_default()
    };
    let authors = if !entry.item.authors.is_empty() {
        entry.item.authors.clone()
    } else {
        existing
            .as_ref()
            .map(|p| p.authors.clone())
            .unwrap_or_default()
    };
    let year = entry
        .item
        .year
        .or_else(|| existing.as_ref().and_then(|p| p.year));
    let abstract_text = if !entry.abstract_text.trim().is_empty() {
        entry.abstract_text.clone()
    } else {
        existing
            .as_ref()
            .map(|p| p.abstract_text.clone())
            .unwrap_or_default()
    };
    let bibtex = existing
        .as_ref()
        .map(|p| p.bibtex.clone())
        .unwrap_or_default();
    let first_seen_ms = existing.as_ref().map(|p| p.first_seen_ms).unwrap_or(ts);

    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids,
        title,
        authors,
        year,
        abstract_text,
        bibtex,
        flags,
        first_seen_ms,
        last_seen_ms: ts,
    })
}

fn upsert_user_fulltext_from_item(
    lib: &Library,
    key: &PaperKey,
    item: &SourceItemRecord,
    ts: i64,
) -> Result<(), String> {
    // Prefer TeX over PDF when both exist (`00-overview.md` 2.11).
    let chosen = item
        .files
        .iter()
        .find(|f| f.role == FILE_ROLE_TEX)
        .or_else(|| item.files.iter().find(|f| f.role == FILE_ROLE_PDF));
    let Some(file) = chosen else {
        return Ok(());
    };
    let format = format_from_path_and_role(&file.path, &file.role);
    lib.upsert_fulltext(&FulltextRecord {
        key: key.clone(),
        version: item.version.clone(),
        raw_blob: None,
        format,
        bytes: file.size,
        origin: ORIGIN_USER_LIBRARY.to_string(),
        fetched_ms: ts,
        file_path: Some(file.path.clone()),
        file_size: Some(file.size),
        file_mtime_ms: Some(file.mtime_ms),
        file_sha256: file.sha256.clone(),
    })
}

fn primary_file_sha_and_size(item: &SourceItemRecord) -> Option<(&str, u64)> {
    item.files
        .iter()
        .find(|f| f.role != FILE_ROLE_NOTE && f.sha256.is_some())
        .and_then(|f| Some((f.sha256.as_deref()?, f.size)))
}

fn hash_in_any_root(roots: &[PathBuf], path: &Path) -> Option<String> {
    for root in roots {
        if resolve_contained_abs(root, path).is_ok() {
            if let Ok(h) = hash_contained_file(root, path, MAX_USER_PDF_BYTES) {
                return Some(h);
            }
        }
    }
    None
}

fn same_item_content(a: &SourceItemRecord, b: &SourceItemRecord) -> bool {
    a.key == b.key
        && a.citekey == b.citekey
        && a.ids == b.ids
        && a.version == b.version
        && a.title == b.title
        && a.authors == b.authors
        && a.year == b.year
        && a.item_kind == b.item_kind
        && a.collections == b.collections
        && a.tags == b.tags
        && a.files == b.files
}

fn validate_source_id(raw: &str) -> Result<String, String> {
    if raw.len() > 64
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "invalid source id `{raw}` (allowed: 1..=64 ASCII alphanumeric, '-', '_', '.')"
        ));
    }
    Ok(raw.to_string())
}

fn derive_source_id(kind: SourceKind, root_str: &str) -> String {
    let prefix = match kind {
        SourceKind::KandinskyVault => "kandinsky",
        SourceKind::Folder => "folder",
        SourceKind::Bibtex => "bibtex",
        SourceKind::Zotero => "zotero",
    };
    let digest = crate::paper_sources::sha256_hex(format!("{prefix}:{root_str}").as_bytes());
    format!("{prefix}-{}", &digest[..12])
}
