//! Plain row structs and SQLite row mappers for [`crate::library_db::LibraryDb`].

use serde::{Deserialize, Serialize};

/// Row in `paper`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryPaperRow {
    pub paper_key: String,
    pub ids_json: String,
    pub title: String,
    pub authors_json: String,
    pub year: Option<i32>,
    pub abstract_text: String,
    pub bibtex: String,
    pub flags_json: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

/// Row in `fulltext` (extends F3's `FulltextEntry` with F13 `user_library` columns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFulltextRow {
    pub paper_key: String,
    pub version: Option<String>,
    pub raw_blob: String,
    pub format: String,
    pub bytes: u64,
    pub origin: String,
    pub fetched_ms: i64,
    pub file_path: Option<String>,
    pub file_size: Option<u64>,
    pub file_mtime_ms: Option<i64>,
    pub file_sha256: Option<String>,
}

/// Row in `source` (F13 schema reservation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySourceRow {
    pub id: String,
    pub kind: String,
    pub root: String,
    pub settings_json: String,
    pub added_ms: i64,
    pub last_sync_ms: Option<i64>,
    pub state: String,
}

/// Row in `source_item` (F13 schema reservation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySourceItemRow {
    pub source_id: String,
    pub external_id: String,
    pub paper_key: Option<String>,
    pub citekey: Option<String>,
    pub ids_json: String,
    pub version: Option<String>,
    pub title: String,
    pub authors_json: String,
    pub year: Option<i32>,
    pub item_kind: String,
    pub collections_json: String,
    pub tags_json: String,
    pub files_json: String,
    pub state: String,
    pub updated_ms: i64,
}

/// Row in `paper_text` (F14 schema reservation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryPaperTextRow {
    pub paper_key: String,
    pub version: Option<String>,
    pub file_sha256: String,
    pub extractor: String,
    pub text_blob: String,
    pub page_basis_json: String,
    pub extracted_ms: i64,
}

/// Row in `seen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySeenRow {
    pub paper_key: String,
    pub session_id: String,
    pub role: String,
    pub seen_ms: i64,
}

/// Row in `skeleton`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySkeletonRow {
    pub paper_key: String,
    pub version: Option<String>,
    pub builder: String,
    pub blob_sha256: String,
    pub built_ms: i64,
}

/// Row in `finding`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryFindingRow {
    pub paper_key: String,
    pub session_id: String,
    pub evidence_seq: u64,
    pub role: String,
    pub gist: String,
    pub quote: String,
    pub cited_start: Option<u64>,
    pub cited_end: Option<u64>,
    pub recorded_ms: i64,
}

/// Row in `search_cache`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySearchCacheRow {
    pub service: String,
    pub url_sha256: String,
    pub raw_blob: String,
    pub fetched_ms: i64,
}

/// Row in `user_mark`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryUserMarkRow {
    pub paper_key: String,
    pub status: String,
    pub tags_json: String,
    pub note: String,
    pub updated_ms: i64,
}

/// Filter parameters for [`crate::library_db::LibraryDb::search_papers`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibrarySearchParams {
    pub q: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i32>,
    pub paper_key: Option<String>,
    pub status: Option<String>,
    pub role: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

/// Projection returned by [`crate::library_db::LibraryDb::search_papers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySearchRow {
    pub paper: LibraryPaperRow,
    pub user_mark: Option<LibraryUserMarkRow>,
    pub has_fulltext: bool,
    pub owned: bool,
    pub seen_roles: Vec<String>,
    pub seen_sessions: Vec<String>,
    pub finding_count: usize,
    pub skeleton_count: usize,
}

/// Bulk snapshot written transactionally during a rebuild.
#[derive(Debug, Default)]
pub struct RebuildBatch {
    pub papers: Vec<LibraryPaperRow>,
    pub aliases: Vec<(String, String)>,
    pub fulltext: Vec<LibraryFulltextRow>,
    pub seen: Vec<LibrarySeenRow>,
    pub skeletons: Vec<LibrarySkeletonRow>,
    pub findings: Vec<LibraryFindingRow>,
    pub paper_text: Vec<LibraryPaperTextRow>,
}

/// Pre-scan snapshot of a `source` row used for optimistic concurrency (CAS) on sync commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSyncSnapshot {
    pub id: String,
    pub kind: String,
    pub root: String,
    pub settings_json: String,
    pub added_ms: i64,
    pub last_sync_ms: Option<i64>,
    pub state: String,
}

impl SourceSyncSnapshot {
    pub fn from_source_row(row: &LibrarySourceRow) -> Self {
        Self {
            id: row.id.clone(),
            kind: row.kind.clone(),
            root: row.root.clone(),
            settings_json: row.settings_json.clone(),
            added_ms: row.added_ms,
            last_sync_ms: row.last_sync_ms,
            state: row.state.clone(),
        }
    }
}

/// Merge strategy for one paper row inside [`SourceSyncBatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperUpdateMode {
    /// Insert if absent, otherwise merge incoming fields/IDs/owned flag into the live row.
    UpsertMerge,
    /// Insert only if the paper row does not currently exist in `paper`.
    InsertIfMissing,
    /// Only update `ids_json` and `flags.owned = true` if the paper row already exists.
    MergeIdsIfExists,
}

/// One paper update applied atomically inside [`crate::library_db::LibraryDb::apply_source_sync_batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSyncPaperUpdate {
    pub paper_key: String,
    pub ids_json: String,
    pub title: String,
    pub authors_json: String,
    pub year: Option<i32>,
    pub abstract_text: String,
    pub venue: Option<String>,
    pub seen_ms: i64,
    pub mode: PaperUpdateMode,
}

/// Complete atomic write batch for one F13a source sync commit.
#[derive(Debug, Clone)]
pub struct SourceSyncBatch {
    pub expected_source: SourceSyncSnapshot,
    pub upsert_source_items: Vec<LibrarySourceItemRow>,
    pub paper_updates: Vec<SourceSyncPaperUpdate>,
    pub aliases: Vec<(String, String)>,
    pub upsert_user_fulltext: Vec<LibraryFulltextRow>,
    pub stale_user_paths: Vec<String>,
    /// `(paper_key, version, file_path)` tuples whose key/version changed while the path may
    /// still be active under a different key; pruned only when no active source item matches.
    pub stale_user_fulltext_keys: Vec<(String, String, String)>,
    pub affected_fulltext_keys: Vec<(String, String)>,
    pub sync_ts_ms: i64,
    pub next_state: String,
}

pub(crate) fn row_to_paper(r: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryPaperRow> {
    Ok(LibraryPaperRow {
        paper_key: r.get(0)?,
        ids_json: r.get(1)?,
        title: r.get(2)?,
        authors_json: r.get(3)?,
        year: r.get(4)?,
        abstract_text: r.get(5)?,
        bibtex: r.get(6)?,
        flags_json: r.get(7)?,
        first_seen_ms: r.get(8)?,
        last_seen_ms: r.get(9)?,
    })
}

pub(crate) fn row_to_fulltext(r: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryFulltextRow> {
    let ver: String = r.get(1)?;
    let bytes: i64 = r.get(4)?;
    let file_size: Option<i64> = r.get(8)?;
    Ok(LibraryFulltextRow {
        paper_key: r.get(0)?,
        version: (!ver.is_empty()).then_some(ver),
        raw_blob: r.get(2)?,
        format: r.get(3)?,
        bytes: bytes.max(0) as u64,
        origin: r.get(5)?,
        fetched_ms: r.get(6)?,
        file_path: r.get(7)?,
        file_size: file_size.map(|v| v.max(0) as u64),
        file_mtime_ms: r.get(9)?,
        file_sha256: r.get(10)?,
    })
}

pub(crate) fn row_to_skeleton(r: &rusqlite::Row<'_>) -> rusqlite::Result<LibrarySkeletonRow> {
    let ver: String = r.get(1)?;
    Ok(LibrarySkeletonRow {
        paper_key: r.get(0)?,
        version: (!ver.is_empty()).then_some(ver),
        builder: r.get(2)?,
        blob_sha256: r.get(3)?,
        built_ms: r.get(4)?,
    })
}

pub(crate) fn row_to_user_mark(r: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryUserMarkRow> {
    Ok(LibraryUserMarkRow {
        paper_key: r.get(0)?,
        status: r.get(1)?,
        tags_json: r.get(2)?,
        note: r.get(3)?,
        updated_ms: r.get(4)?,
    })
}

pub(crate) fn row_to_source_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<LibrarySourceItemRow> {
    Ok(LibrarySourceItemRow {
        source_id: r.get(0)?,
        external_id: r.get(1)?,
        paper_key: r.get(2)?,
        citekey: r.get(3)?,
        ids_json: r.get(4)?,
        version: r.get(5)?,
        title: r.get(6)?,
        authors_json: r.get(7)?,
        year: r.get(8)?,
        item_kind: r.get(9)?,
        collections_json: r.get(10)?,
        tags_json: r.get(11)?,
        files_json: r.get(12)?,
        state: r.get(13)?,
        updated_ms: r.get(14)?,
    })
}
