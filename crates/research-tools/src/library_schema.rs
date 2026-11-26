//! Typed schema, domain enums, and record types for the shared paper library (package F10).
//!
//! Uses F2's [`PaperKey`] and [`PaperIds`] exclusively for paper identity (`00-overview.md`
//! section 2.3). Names tied to `00-overview.md` section 8.2, decision 5 (one global,
//! rebuildable library; user marks backed up) are re-exported from `ledger::library_schema`
//! or defined as single named constants here.

pub use crate::library_ext_types::{
    PageBasis, PaperTextRecord, SearchCacheRecord, SourceItemFile, SourceItemRecord,
    SourceItemState, SourceKind, SourceRecord,
};
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use ledger::library_db::{
    LibraryFindingRow, LibraryFulltextRow, LibraryPaperRow, LibrarySearchRow, LibrarySeenRow,
    LibrarySkeletonRow, LibraryUserMarkRow,
};
pub use ledger::{
    FULLTEXT_TABLE, LIBRARY_DB_NAME, LIBRARY_DIR_NAME, LIBRARY_SCHEMA_VERSION, ORIGIN_USER_LIBRARY,
    USER_MARKS_BACKUP_NAME, USER_MARKS_FILE_NAME,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Version of the portable JSON backup format for non-rebuildable user marks
/// (`00-overview.md` section 8.2, decision 5).
pub const USER_MARKS_BACKUP_SCHEMA_VERSION: u32 = 1;

/// Standard `fulltext.origin` values (`00-overview.md` 2.4, 2.7, 2.11; `explore.md` 4.3).
pub const ORIGIN_ARXIV_EPRINT: &str = "arxiv_eprint";
pub const ORIGIN_ARXIV_PDF: &str = "arxiv_pdf";
pub const ORIGIN_LEDGER: &str = "ledger";
pub const ORIGIN_UPLOAD: &str = "upload";

pub const DEFAULT_LIBRARY_LIMIT: usize = 50;
pub const MAX_LIBRARY_LIMIT: usize = 200;
pub const MAX_TAGS_PER_PAPER: usize = 32;
pub const MAX_TAG_CHARS: usize = 64;
pub const MAX_NOTE_CHARS: usize = 4_000;
pub const MAX_BACKUP_MARKS: usize = 50_000;
pub const MAX_BACKUP_BYTES: usize = 16 * 1024 * 1024;

/// Role of a paper in a session (`explore.md` 4.3 + `00-overview.md` 2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeenRole {
    Candidate,
    Triaged,
    Read,
    Investigated,
    CitedLookup,
    SolveSource,
}

impl SeenRole {
    pub const ALL: [SeenRole; 6] = [
        Self::Candidate,
        Self::Triaged,
        Self::Read,
        Self::Investigated,
        Self::CitedLookup,
        Self::SolveSource,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Triaged => "triaged",
            Self::Read => "read",
            Self::Investigated => "investigated",
            Self::CitedLookup => "cited_lookup",
            Self::SolveSource => "solve_source",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "candidate" => Ok(Self::Candidate),
            "triaged" => Ok(Self::Triaged),
            "read" => Ok(Self::Read),
            "investigated" => Ok(Self::Investigated),
            "cited_lookup" => Ok(Self::CitedLookup),
            "solve_source" => Ok(Self::SolveSource),
            other => Err(format!(
                "invalid seen role `{other}` (expected one of candidate, triaged, read, investigated, cited_lookup, solve_source)"
            )),
        }
    }
}

/// User-authored triage/reading status (`explore.md` 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkStatus {
    Unread,
    Skimmed,
    Key,
    Irrelevant,
}

impl MarkStatus {
    pub const ALL: [MarkStatus; 4] = [Self::Unread, Self::Skimmed, Self::Key, Self::Irrelevant];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Skimmed => "skimmed",
            Self::Key => "key",
            Self::Irrelevant => "irrelevant",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "unread" => Ok(Self::Unread),
            "skimmed" => Ok(Self::Skimmed),
            "key" => Ok(Self::Key),
            "irrelevant" => Ok(Self::Irrelevant),
            other => Err(format!(
                "invalid user mark status `{other}` (expected one of unread, skimmed, key, irrelevant)"
            )),
        }
    }
}

/// Known identifiers of a paper stored in `paper.ids_json`.
/// Superset of F2's [`PaperIds`] adding optional `openalex` and `isbn` (for F13).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryIds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arxiv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspire: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openalex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
}

impl LibraryIds {
    pub fn from_paper_ids(ids: &PaperIds) -> Self {
        Self {
            arxiv: ids.arxiv.clone(),
            doi: ids.doi.clone(),
            inspire: ids.inspire.clone(),
            s2: ids.s2.clone(),
            openalex: None,
            isbn: None,
        }
    }

    pub fn to_paper_ids(&self) -> PaperIds {
        PaperIds {
            arxiv: self.arxiv.clone(),
            doi: self.doi.clone(),
            inspire: self.inspire.clone(),
            s2: self.s2.clone(),
        }
    }

    pub fn merge_from(&mut self, other: &Self) {
        if self.arxiv.is_none() {
            self.arxiv = other.arxiv.clone();
        }
        if self.doi.is_none() {
            self.doi = other.doi.clone();
        }
        if self.inspire.is_none() {
            self.inspire = other.inspire.clone();
        }
        if self.s2.is_none() {
            self.s2 = other.s2.clone();
        }
        if self.openalex.is_none() {
            self.openalex = other.openalex.clone();
        }
        if self.isbn.is_none() {
            self.isbn = other.isbn.clone();
        }
    }

    /// Canonical `PaperKey` strings derivable from every identifier present here.
    pub fn canonical_aliases(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        if let Some(a) = &self.arxiv {
            if let Ok(k) = PaperKey::from_arxiv(a) {
                out.insert(k.to_string());
            }
        }
        if let Some(d) = &self.doi {
            if let Ok(k) = PaperKey::from_doi(d) {
                out.insert(k.to_string());
            }
        }
        if let Some(i) = &self.inspire {
            if let Ok(k) = PaperKey::from_inspire(i) {
                out.insert(k.to_string());
            }
        }
        if let Some(s) = &self.s2 {
            if let Ok(k) = PaperKey::from_s2(s) {
                out.insert(k.to_string());
            }
        }
        if let Some(o) = &self.openalex {
            if let Ok(k) = PaperKey::from_openalex(o) {
                out.insert(k.to_string());
            }
        }
        out.into_iter().collect()
    }
}

/// Flags carried in `paper.flags_json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperFlags {
    #[serde(default)]
    pub withdrawn: bool,
    #[serde(default)]
    pub retracted: bool,
    #[serde(default)]
    pub owned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
}

impl PaperFlags {
    pub fn merge_from(&mut self, other: &Self) {
        self.withdrawn |= other.withdrawn;
        self.retracted |= other.retracted;
        self.owned |= other.owned;
        if self.license.is_none() {
            self.license = other.license.clone();
        }
        if self.venue.is_none() {
            self.venue = other.venue.clone();
        }
    }
}

/// Typed `paper` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperRecord {
    pub key: PaperKey,
    #[serde(default)]
    pub ids: LibraryIds,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, rename = "abstract")]
    pub abstract_text: String,
    #[serde(default)]
    pub bibtex: String,
    #[serde(default)]
    pub flags: PaperFlags,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

impl PaperRecord {
    pub fn paper_ref(&self, version: Option<String>) -> PaperRef {
        PaperRef {
            key: self.key.clone(),
            version,
            ids: self.ids.to_paper_ids(),
        }
    }

    pub(crate) fn to_row(&self) -> Result<LibraryPaperRow, String> {
        Ok(LibraryPaperRow {
            paper_key: self.key.to_string(),
            ids_json: serde_json::to_string(&self.ids).map_err(|e| e.to_string())?,
            title: self.title.clone(),
            authors_json: serde_json::to_string(&self.authors).map_err(|e| e.to_string())?,
            year: self.year,
            abstract_text: self.abstract_text.clone(),
            bibtex: self.bibtex.clone(),
            flags_json: serde_json::to_string(&self.flags).map_err(|e| e.to_string())?,
            first_seen_ms: self.first_seen_ms,
            last_seen_ms: self.last_seen_ms,
        })
    }

    pub(crate) fn from_row(row: LibraryPaperRow) -> Result<Self, String> {
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            ids: serde_json::from_str(&row.ids_json).unwrap_or_default(),
            title: row.title,
            authors: serde_json::from_str(&row.authors_json).unwrap_or_default(),
            year: row.year,
            abstract_text: row.abstract_text,
            bibtex: row.bibtex,
            flags: serde_json::from_str(&row.flags_json).unwrap_or_default(),
            first_seen_ms: row.first_seen_ms,
            last_seen_ms: row.last_seen_ms,
        })
    }
}

/// Typed `fulltext` row (covers both F3 cached blobs and F13 `user_library` files).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulltextRecord {
    pub key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
    pub format: String,
    pub bytes: u64,
    pub origin: String,
    pub fetched_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_mtime_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_sha256: Option<String>,
}

impl FulltextRecord {
    pub(crate) fn to_row(&self) -> LibraryFulltextRow {
        LibraryFulltextRow {
            paper_key: self.key.to_string(),
            version: self.version.clone(),
            raw_blob: self.raw_blob.clone().unwrap_or_default(),
            format: self.format.clone(),
            bytes: self.bytes,
            origin: self.origin.clone(),
            fetched_ms: self.fetched_ms,
            file_path: self.file_path.clone(),
            file_size: self.file_size,
            file_mtime_ms: self.file_mtime_ms,
            file_sha256: self.file_sha256.clone(),
        }
    }

    pub(crate) fn from_row(row: LibraryFulltextRow) -> Result<Self, String> {
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            version: row.version,
            raw_blob: (!row.raw_blob.is_empty()).then_some(row.raw_blob),
            format: row.format,
            bytes: row.bytes,
            origin: row.origin,
            fetched_ms: row.fetched_ms,
            file_path: row.file_path,
            file_size: row.file_size,
            file_mtime_ms: row.file_mtime_ms,
            file_sha256: row.file_sha256,
        })
    }
}

/// Typed `seen` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenRecord {
    pub key: PaperKey,
    pub session_id: String,
    pub role: SeenRole,
    pub seen_ms: i64,
}

impl SeenRecord {
    pub(crate) fn to_row(&self) -> LibrarySeenRow {
        LibrarySeenRow {
            paper_key: self.key.to_string(),
            session_id: self.session_id.clone(),
            role: self.role.as_str().to_string(),
            seen_ms: self.seen_ms,
        }
    }

    pub(crate) fn from_row(row: LibrarySeenRow) -> Result<Self, String> {
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            session_id: row.session_id,
            role: SeenRole::parse(&row.role)?,
            seen_ms: row.seen_ms,
        })
    }
}

/// Typed `skeleton` row (`00-overview.md` 2.7: `skeleton(key, version, builder, blob_sha256)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkeletonRecord {
    pub key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub builder: String,
    pub blob_sha256: String,
    pub built_ms: i64,
}

impl SkeletonRecord {
    pub(crate) fn to_row(&self) -> LibrarySkeletonRow {
        LibrarySkeletonRow {
            paper_key: self.key.to_string(),
            version: self.version.clone(),
            builder: self.builder.clone(),
            blob_sha256: self.blob_sha256.clone(),
            built_ms: self.built_ms,
        }
    }

    pub(crate) fn from_row(row: LibrarySkeletonRow) -> Result<Self, String> {
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            version: row.version,
            builder: row.builder,
            blob_sha256: row.blob_sha256,
            built_ms: row.built_ms,
        })
    }
}

/// Typed `finding` row (`explore.md` 4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingRecord {
    pub key: PaperKey,
    pub session_id: String,
    pub evidence_seq: u64,
    pub role: String,
    pub gist: String,
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cited_range: Option<[usize; 2]>,
    pub recorded_ms: i64,
}

impl FindingRecord {
    pub(crate) fn to_row(&self) -> LibraryFindingRow {
        LibraryFindingRow {
            paper_key: self.key.to_string(),
            session_id: self.session_id.clone(),
            evidence_seq: self.evidence_seq,
            role: self.role.clone(),
            gist: self.gist.clone(),
            quote: self.quote.clone(),
            cited_start: self.cited_range.map(|r| r[0] as u64),
            cited_end: self.cited_range.map(|r| r[1] as u64),
            recorded_ms: self.recorded_ms,
        }
    }

    pub(crate) fn from_row(row: LibraryFindingRow) -> Result<Self, String> {
        let cited_range = match (row.cited_start, row.cited_end) {
            (Some(s), Some(e)) => Some([s as usize, e as usize]),
            _ => None,
        };
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            session_id: row.session_id,
            evidence_seq: row.evidence_seq,
            role: row.role,
            gist: row.gist,
            quote: row.quote,
            cited_range,
            recorded_ms: row.recorded_ms,
        })
    }
}

/// Typed `user_mark` row (non-rebuildable user data; persisted in `user_marks.json` and backed up).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMark {
    pub key: PaperKey,
    pub status: MarkStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: String,
    pub updated_ms: i64,
}

impl UserMark {
    pub(crate) fn to_row(&self) -> Result<LibraryUserMarkRow, String> {
        Ok(LibraryUserMarkRow {
            paper_key: self.key.to_string(),
            status: self.status.as_str().to_string(),
            tags_json: serde_json::to_string(&self.tags).map_err(|e| e.to_string())?,
            note: self.note.clone(),
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(row: LibraryUserMarkRow) -> Result<Self, String> {
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            status: MarkStatus::parse(&row.status)?,
            tags: serde_json::from_str(&row.tags_json).unwrap_or_default(),
            note: row.note,
            updated_ms: row.updated_ms,
        })
    }
}

/// Portable, checksummed backup envelope for user marks (`00-overview.md` 8.2 row 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMarksBackup {
    pub schema_version: u32,
    pub exported_ms: i64,
    pub sha256: String,
    pub marks: Vec<UserMark>,
}

/// Summary row returned by list and search operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperSummary {
    pub paper: PaperRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_mark: Option<UserMark>,
    pub has_fulltext: bool,
    pub owned: bool,
    pub seen_roles: Vec<SeenRole>,
    pub seen_sessions: Vec<String>,
    pub finding_count: usize,
    pub skeleton_count: usize,
}

impl PaperSummary {
    pub(crate) fn from_search_row(row: LibrarySearchRow) -> Result<Self, String> {
        let paper = PaperRecord::from_row(row.paper)?;
        let user_mark = row.user_mark.map(UserMark::from_row).transpose()?;
        let mut seen_roles = Vec::with_capacity(row.seen_roles.len());
        for role in row.seen_roles {
            seen_roles.push(SeenRole::parse(&role)?);
        }
        Ok(Self {
            paper,
            user_mark,
            has_fulltext: row.has_fulltext,
            owned: row.owned,
            seen_roles,
            seen_sessions: row.seen_sessions,
            finding_count: row.finding_count,
            skeleton_count: row.skeleton_count,
        })
    }
}

/// Full record returned by `GET /api/library/{key}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperDetail {
    pub paper: PaperRecord,
    pub aliases: Vec<String>,
    pub fulltext: Vec<FulltextRecord>,
    pub seen: Vec<SeenRecord>,
    pub skeletons: Vec<SkeletonRecord>,
    pub findings: Vec<FindingRecord>,
    pub paper_text: Vec<PaperTextRecord>,
    pub source_items: Vec<SourceItemRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_mark: Option<UserMark>,
}

/// Typed search/filter request for [`crate::library::Library::search`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryQuery {
    pub q: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i32>,
    pub key: Option<PaperKey>,
    pub status: Option<MarkStatus>,
    pub role: Option<SeenRole>,
    pub limit: Option<usize>,
    pub offset: usize,
}

/// Summary returned by [`crate::library::Library::rebuild`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildReport {
    pub sessions_scanned: usize,
    pub events_scanned: usize,
    pub papers: usize,
    pub aliases: usize,
    pub fulltext_entries: usize,
    pub seen_entries: usize,
    pub skeleton_entries: usize,
    pub finding_entries: usize,
    pub paper_text_entries: usize,
    pub user_marks_restored: usize,
    pub warnings: Vec<String>,
}
