// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Pure reconciliation, moved-file detection by SHA-256, and source-id helpers for
//! [`crate::source_sync`] (package F13a, `00-overview.md` 2.11).

use crate::library::{
    FulltextRecord, LibraryIds, SourceItemRecord, SourceItemState, SourceKind, SourceRecord,
    ORIGIN_USER_LIBRARY,
};
use crate::paper_key::PaperKey;
use crate::paper_sources::{
    format_from_path_and_role, hash_contained_file, resolve_contained_abs, ScannedItem,
    FILE_ROLE_NOTE, FILE_ROLE_PDF, FILE_ROLE_TEX, SOURCE_STATE_SYNCED,
};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use ledger::library_db::{
    PaperUpdateMode, SourceSyncBatch, SourceSyncPaperUpdate, SourceSyncSnapshot,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) struct ReconciledEntry {
    pub(crate) item: SourceItemRecord,
    pub(crate) abstract_text: String,
    pub(crate) venue: Option<String>,
    pub(crate) dirty: bool,
}

pub(crate) struct ReconcileOutcome {
    pub(crate) items: Vec<ReconciledEntry>,
    pub(crate) added: usize,
    pub(crate) updated: usize,
    pub(crate) moved: usize,
    pub(crate) missing: usize,
    pub(crate) unchanged: usize,
}

pub(crate) fn reconcile_items(
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

pub(crate) fn validate_source_id(raw: &str) -> Result<String, String> {
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

pub(crate) fn derive_source_id(kind: SourceKind, root_str: &str) -> String {
    let prefix = match kind {
        SourceKind::KandinskyVault => "kandinsky",
        SourceKind::Folder => "folder",
        SourceKind::Bibtex => "bibtex",
        SourceKind::Zotero => "zotero",
    };
    let digest = crate::paper_sources::sha256_hex(format!("{prefix}:{root_str}").as_bytes());
    format!("{prefix}-{}", &digest[..12])
}

pub(crate) type ExactMergeSpec = (PaperKey, Vec<PaperKey>, LibraryIds);

pub(crate) fn build_source_sync_batch(
    source: &SourceRecord,
    previous: &[SourceItemRecord],
    entries: &[ReconciledEntry],
    exact_merges: &[ExactMergeSpec],
    ts: i64,
) -> Result<SourceSyncBatch, String> {
    let expected_source = SourceSyncSnapshot::from_source_row(&source.to_row()?);

    let mut upsert_source_items = Vec::new();
    let mut paper_updates = Vec::new();
    let mut alias_map: BTreeMap<String, String> = BTreeMap::new();
    let mut upsert_user_fulltext = Vec::new();
    let mut new_active_paths: BTreeSet<String> = BTreeSet::new();
    let mut new_active_key_ver_paths: BTreeSet<(String, String, String)> = BTreeSet::new();

    for entry in entries {
        if entry.dirty {
            upsert_source_items.push(entry.item.to_row()?);
        }
        if entry.item.state == SourceItemState::Missing {
            continue;
        }
        let entry_ver = entry
            .item
            .version
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        for f in &entry.item.files {
            if f.role != FILE_ROLE_NOTE && !f.path.is_empty() {
                let norm = normalize_sync_path(&f.path);
                new_active_paths.insert(norm.clone());
                if let Some(key) = &entry.item.key {
                    new_active_key_ver_paths.insert((key.to_string(), entry_ver.clone(), norm));
                }
            }
        }
        let Some(key) = &entry.item.key else {
            continue;
        };
        let ids_json = serde_json::to_string(&entry.item.ids).map_err(|e| e.to_string())?;
        let authors_json =
            serde_json::to_string(&entry.item.authors).map_err(|e| e.to_string())?;
        let mode = if entry.dirty {
            PaperUpdateMode::UpsertMerge
        } else {
            PaperUpdateMode::InsertIfMissing
        };
        paper_updates.push(SourceSyncPaperUpdate {
            paper_key: key.to_string(),
            ids_json,
            title: entry.item.title.clone(),
            authors_json,
            year: entry.item.year,
            abstract_text: entry.abstract_text.clone(),
            venue: entry.venue.clone(),
            seen_ms: ts,
            mode,
        });
        for alias in entry.item.ids.canonical_aliases() {
            if alias != key.as_str() {
                alias_map.insert(alias, key.to_string());
            }
        }

        if entry.dirty {
            if let Some(ft_row) = build_user_fulltext_row(key, &entry.item, ts) {
                upsert_user_fulltext.push(ft_row);
            }
        }
    }

    for (canon, aliases, merged_ids) in exact_merges {
        let ids_json = serde_json::to_string(merged_ids).map_err(|e| e.to_string())?;
        paper_updates.push(SourceSyncPaperUpdate {
            paper_key: canon.to_string(),
            ids_json,
            title: String::new(),
            authors_json: "[]".to_string(),
            year: None,
            abstract_text: String::new(),
            venue: None,
            seen_ms: ts,
            mode: PaperUpdateMode::MergeIdsIfExists,
        });
        for alias in aliases {
            if alias != canon {
                alias_map.insert(alias.to_string(), canon.to_string());
            }
        }
        for alias in merged_ids.canonical_aliases() {
            if alias != canon.as_str() {
                alias_map.insert(alias, canon.to_string());
            }
        }
    }

    let mut stale_paths_set: BTreeSet<String> = BTreeSet::new();
    let mut stale_key_ver_paths: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut affected_keys_set: BTreeSet<(String, String)> = BTreeSet::new();
    for prev in previous {
        let prev_ver = prev
            .version
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let mut had_stale = false;
        for f in &prev.files {
            if f.role != FILE_ROLE_NOTE && !f.path.is_empty() {
                let norm = normalize_sync_path(&f.path);
                if !new_active_paths.contains(&norm) {
                    stale_paths_set.insert(f.path.clone());
                    had_stale = true;
                } else if let Some(pk) = &prev.key {
                    let tuple = (pk.to_string(), prev_ver.clone(), norm);
                    if !new_active_key_ver_paths.contains(&tuple) {
                        stale_key_ver_paths.insert((
                            pk.to_string(),
                            prev_ver.clone(),
                            f.path.clone(),
                        ));
                        had_stale = true;
                    }
                }
            }
        }
        if had_stale {
            if let Some(pk) = &prev.key {
                affected_keys_set.insert((pk.to_string(), prev_ver));
            }
        }
    }

    Ok(SourceSyncBatch {
        expected_source,
        upsert_source_items,
        paper_updates,
        aliases: alias_map.into_iter().collect(),
        upsert_user_fulltext,
        stale_user_paths: stale_paths_set.into_iter().collect(),
        stale_user_fulltext_keys: stale_key_ver_paths.into_iter().collect(),
        affected_fulltext_keys: affected_keys_set.into_iter().collect(),
        sync_ts_ms: ts,
        next_state: SOURCE_STATE_SYNCED.to_string(),
    })
}

fn normalize_sync_path(raw: &str) -> String {
    let mut s = raw.trim().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    #[cfg(windows)]
    {
        s.make_ascii_lowercase();
    }
    s
}

fn build_user_fulltext_row(
    key: &PaperKey,
    item: &SourceItemRecord,
    ts: i64,
) -> Option<ledger::library_db::LibraryFulltextRow> {
    // Prefer TeX over PDF when both exist (`00-overview.md` 2.11).
    let file = item
        .files
        .iter()
        .find(|f| f.role == FILE_ROLE_TEX)
        .or_else(|| item.files.iter().find(|f| f.role == FILE_ROLE_PDF))?;
    let format = format_from_path_and_role(&file.path, &file.role);
    let rec = FulltextRecord {
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
    };
    Some(rec.to_row())
}
