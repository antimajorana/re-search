// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! F10 library persistence helpers for [`crate::source_sync`].

use crate::library::{
    FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord,
    SourceItemState, SourceRecord, ORIGIN_USER_LIBRARY,
};
use crate::paper_key::PaperKey;
use crate::paper_sources::{
    format_from_path_and_role, FILE_ROLE_PDF, FILE_ROLE_TEX, SOURCE_STATE_SYNCED,
};

pub(crate) struct ReconciledEntry {
    pub item: SourceItemRecord,
    pub abstract_text: String,
    pub venue: Option<String>,
    pub dirty: bool,
}

pub(crate) fn persist_sync(
    lib: &Library,
    source: &SourceRecord,
    entries: &[ReconciledEntry],
    exact_merges: &[(PaperKey, Vec<PaperKey>, LibraryIds)],
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
