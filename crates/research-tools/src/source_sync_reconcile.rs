// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Pure reconciliation, moved-file detection by SHA-256, and source-id helpers for
//! [`crate::source_sync`] (package F13a, `00-overview.md` 2.11).

use crate::library::{SourceItemRecord, SourceItemState, SourceKind};
use crate::paper_sources::{
    hash_contained_file, resolve_contained_abs, ScannedItem, FILE_ROLE_NOTE,
};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
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
