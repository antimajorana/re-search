// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! One-way read-only source registration, dry-run preview, incremental sync, moved-file
//! detection by SHA-256, missing-file retention, and atomic F10 library indexing (package F13a,
//! `00-overview.md` 2.11).

use crate::library::{Library, SourceItemRecord, SourceItemState, SourceKind, SourceRecord};
use crate::paper_sources::{
    canonicalize_source_root, now_ms, portable_path_string, PaperSourceAdapter, ScanContext,
    ScanWarning, TitleLookupHook, FILE_ROLE_PDF, FILE_ROLE_TEX, SOURCE_STATE_PENDING,
};
use crate::source_dedupe::{find_duplicates, DuplicateGroup};
use crate::source_folder::FolderAdapter;
use crate::source_kandinsky::KandinskyVaultAdapter;
use crate::source_sync_reconcile::{
    build_source_sync_batch, derive_source_id, reconcile_items, validate_source_id, ExactMergeSpec,
};
use ledger::library_db::SourceSyncBatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

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

/// Result of the read-only scan + reconciliation phase (outside any SQLite write transaction).
///
/// Pass to [`commit_source_sync`] to apply the batch atomically with optimistic concurrency
/// checking against the pre-scan `source` snapshot.
#[derive(Debug, Clone)]
pub struct PreparedSourceSync {
    pub report: SyncReport,
    pub batch: SourceSyncBatch,
}

/// Construct a validated candidate [`SourceRecord`] in memory without writing to `library.db`.
fn build_candidate_source(
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
    Ok(SourceRecord {
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
    })
}

/// Register a new read-only paper source in F10's `source` table without writing to user folders.
pub fn register_source(
    lib: &Library,
    kind: SourceKind,
    root: &Path,
    settings: Value,
    custom_id: Option<String>,
) -> Result<SourceRecord, String> {
    let rec = build_candidate_source(kind, root, settings, custom_id)?;
    lib.upsert_source(&rec)?;
    Ok(rec)
}

/// Run a pure read-only first-sync preview for a candidate source without writing or deleting
/// anything in `library.db` (`00-overview.md` 2.11 sync rule 5).
pub fn preview_source(
    lib: &Library,
    kind: SourceKind,
    root: &Path,
    settings: Value,
    custom_id: Option<String>,
    hash_files: bool,
) -> Result<(SourceRecord, SyncReport), String> {
    let rec = build_candidate_source(kind, root, settings, custom_id)?;
    let prepared = scan_and_prepare(
        lib,
        &rec,
        &[],
        &SyncOptions {
            dry_run: true,
            hash_files,
        },
    )?;
    Ok((rec, prepared.report))
}

/// Atomically remove a registered source and its indexed rows from `library.db`.
///
/// Preserves any `fulltext` path still indexed by another registered source and every `fulltext`
/// row from unrelated origins (`origin <> 'user_library'`). Never touches user files or sessions,
/// and is safe to retry (returns `Ok(false)` when already removed).
pub fn remove_source(lib: &Library, source_id: &str) -> Result<bool, String> {
    lib.unregister_source(source_id)
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
    title_lookup: Option<&dyn TitleLookupHook>,
) -> Result<SyncReport, String> {
    let prepared = prepare_source_sync(lib, source_id, opts, title_lookup)?;
    if opts.dry_run {
        return Ok(prepared.report);
    }
    commit_source_sync(lib, prepared)
}

/// Phase 1 of source sync: snapshot the `source` row, scan user files read-only outside any
/// SQLite write transaction, reconcile moved/missing items, run 5-signal dedupe, and build
/// the atomic [`SourceSyncBatch`]. Writes nothing to `library.db`.
pub fn prepare_source_sync(
    lib: &Library,
    source_id: &str,
    opts: &SyncOptions,
    _title_lookup: Option<&dyn TitleLookupHook>,
) -> Result<PreparedSourceSync, String> {
    let source = lib
        .list_sources()?
        .into_iter()
        .find(|s| s.id == source_id)
        .ok_or_else(|| format!("paper source `{source_id}` not found"))?;
    let previous = lib.list_source_items_for_source(source_id)?;
    scan_and_prepare(lib, &source, &previous, opts)
}

/// Phase 2 of source sync: atomically commit a [`PreparedSourceSync`] batch inside one
/// `BEGIN IMMEDIATE` transaction, rejecting stale scans if the source changed or was removed.
pub fn commit_source_sync(
    lib: &Library,
    prepared: PreparedSourceSync,
) -> Result<SyncReport, String> {
    lib.apply_source_sync_batch(&prepared.batch)?;
    Ok(prepared.report)
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

fn scan_and_prepare(
    lib: &Library,
    source: &SourceRecord,
    previous: &[SourceItemRecord],
    opts: &SyncOptions,
) -> Result<PreparedSourceSync, String> {
    let ts = now_ms();
    let ctx = ScanContext::from_previous(previous, opts.hash_files, ts);

    // All filesystem reads, PDF parsing, and SHA-256 hashing happen here outside any DB write lock.
    let scan = match source.kind {
        SourceKind::KandinskyVault => KandinskyVaultAdapter.scan(source, &ctx)?,
        SourceKind::Folder => FolderAdapter.scan(source, &ctx)?,
        other => {
            return Err(format!(
                "source kind `{}` is not supported in F13a (planned for F13b)",
                other.as_str()
            ));
        }
    };

    let reconciled = reconcile_items(previous, scan.items, &scan.roots_checked, ts);

    // Combine this source's items with items from other registered sources for cross-source dedupe.
    let other_items: Vec<SourceItemRecord> = lib
        .list_all_source_items()?
        .into_iter()
        .filter(|it| it.source_id != source.id)
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
    let mut exact_merges: Vec<ExactMergeSpec> = Vec::new();
    for grp in &dedupe.exact_groups {
        let touches_mine = grp.member_indices.iter().any(|&idx| idx < mine_count);
        if let Some(canon) = &grp.canonical_key {
            for &idx in &grp.member_indices {
                if idx < mine_count {
                    let entry = &mut final_entries[idx];
                    let prev_key = entry.item.key.clone();
                    let prev_ids = entry.item.ids.clone();
                    let prev_state = entry.item.state;
                    entry.item.key = Some(canon.clone());
                    entry.item.ids.merge_from(&grp.merged_ids);
                    if let Some(merged_doi) = grp.merged_ids.doi.as_deref() {
                        let merged_is_arxiv = merged_doi
                            .to_ascii_lowercase()
                            .starts_with("10.48550/arxiv.");
                        let cur_is_arxiv = entry
                            .item
                            .ids
                            .doi
                            .as_deref()
                            .is_some_and(|d| d.to_ascii_lowercase().starts_with("10.48550/arxiv."));
                        if cur_is_arxiv && !merged_is_arxiv {
                            entry.item.ids.doi = Some(merged_doi.to_string());
                        }
                    }
                    if entry.item.state == SourceItemState::PossibleDuplicate {
                        entry.item.state = SourceItemState::Present;
                    }
                    if entry.item.key != prev_key
                        || entry.item.ids != prev_ids
                        || entry.item.state != prev_state
                    {
                        entry.dirty = true;
                    }
                }
            }
            if touches_mine {
                exact_merges.push((canon.clone(), grp.alias_keys.clone(), grp.merged_ids.clone()));
            }
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

    let batch = build_source_sync_batch(source, previous, &final_entries, &exact_merges, ts)?;

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
    let report = SyncReport {
        source_id: source.id.clone(),
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
    };
    Ok(PreparedSourceSync { report, batch })
}
