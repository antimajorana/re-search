//! F13 (`source`, `source_item`), F14 (`paper_text`), and F11 (`search_cache`) typed
//! library records, re-exported from [`crate::library_schema`].

use crate::library_schema::LibraryIds;
use crate::paper_key::PaperKey;
use ledger::library_db::{
    LibraryPaperTextRow, LibrarySearchCacheRow, LibrarySourceItemRow, LibrarySourceRow,
};
use serde::{Deserialize, Serialize};

/// Kind of external paper source (`00-overview.md` 2.11, F13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    KandinskyVault,
    Folder,
    Bibtex,
    Zotero,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KandinskyVault => "kandinsky_vault",
            Self::Folder => "folder",
            Self::Bibtex => "bibtex",
            Self::Zotero => "zotero",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "kandinsky_vault" => Ok(Self::KandinskyVault),
            "folder" => Ok(Self::Folder),
            "bibtex" => Ok(Self::Bibtex),
            "zotero" => Ok(Self::Zotero),
            other => Err(format!("invalid source kind `{other}`")),
        }
    }
}

/// Sync state of an item inside a user paper source (`00-overview.md` 2.11, F13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceItemState {
    Present,
    Changed,
    Missing,
    PossibleDuplicate,
}

impl SourceItemState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Changed => "changed",
            Self::Missing => "missing",
            Self::PossibleDuplicate => "possible_duplicate",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "present" => Ok(Self::Present),
            "changed" => Ok(Self::Changed),
            "missing" => Ok(Self::Missing),
            "possible_duplicate" => Ok(Self::PossibleDuplicate),
            other => Err(format!("invalid source item state `{other}`")),
        }
    }
}

/// Per-page text extraction basis (`00-overview.md` 2.12, F14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageBasis {
    TextLayer,
    ScanTextLayer,
    None,
    Ocr,
}

/// Typed `paper_text` row (`00-overview.md` 2.7 & 2.12, F14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperTextRecord {
    pub key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub file_sha256: String,
    pub extractor: String,
    pub text_blob: String,
    #[serde(default)]
    pub page_basis: Vec<PageBasis>,
    pub extracted_ms: i64,
}

impl PaperTextRecord {
    pub(crate) fn to_row(&self) -> Result<LibraryPaperTextRow, String> {
        Ok(LibraryPaperTextRow {
            paper_key: self.key.to_string(),
            version: self.version.clone(),
            file_sha256: self.file_sha256.clone(),
            extractor: self.extractor.clone(),
            text_blob: self.text_blob.clone(),
            page_basis_json: serde_json::to_string(&self.page_basis).map_err(|e| e.to_string())?,
            extracted_ms: self.extracted_ms,
        })
    }

    pub(crate) fn from_row(row: LibraryPaperTextRow) -> Result<Self, String> {
        Ok(Self {
            key: PaperKey::parse(&row.paper_key)?,
            version: row.version,
            file_sha256: row.file_sha256,
            extractor: row.extractor,
            text_blob: row.text_blob,
            page_basis: serde_json::from_str(&row.page_basis_json).unwrap_or_default(),
            extracted_ms: row.extracted_ms,
        })
    }
}

/// Typed `source` row (`00-overview.md` 2.11, F13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub kind: SourceKind,
    pub root: String,
    #[serde(default)]
    pub settings: serde_json::Value,
    pub added_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_ms: Option<i64>,
    pub state: String,
}

impl SourceRecord {
    pub(crate) fn to_row(&self) -> Result<LibrarySourceRow, String> {
        let settings_json = if self.settings.is_null() {
            "{}".to_string()
        } else {
            serde_json::to_string(&self.settings).map_err(|e| e.to_string())?
        };
        Ok(LibrarySourceRow {
            id: self.id.clone(),
            kind: self.kind.as_str().to_string(),
            root: self.root.clone(),
            settings_json,
            added_ms: self.added_ms,
            last_sync_ms: self.last_sync_ms,
            state: self.state.clone(),
        })
    }

    pub(crate) fn from_row(row: LibrarySourceRow) -> Result<Self, String> {
        Ok(Self {
            id: row.id,
            kind: SourceKind::parse(&row.kind)?,
            root: row.root,
            settings: serde_json::from_str(&row.settings_json)
                .unwrap_or_else(|_| serde_json::json!({})),
            added_ms: row.added_ms,
            last_sync_ms: row.last_sync_ms,
            state: row.state,
        })
    }
}

/// One file attached to a [`SourceItemRecord`] (`00-overview.md` 2.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceItemFile {
    pub path: String,
    pub role: String,
    pub size: u64,
    pub mtime_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Typed `source_item` row (`00-overview.md` 2.11, F13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceItemRecord {
    pub source_id: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citekey: Option<String>,
    #[serde(default)]
    pub ids: LibraryIds,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default)]
    pub item_kind: String,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub files: Vec<SourceItemFile>,
    pub state: SourceItemState,
    pub updated_ms: i64,
}

impl SourceItemRecord {
    pub(crate) fn to_row(&self) -> Result<LibrarySourceItemRow, String> {
        Ok(LibrarySourceItemRow {
            source_id: self.source_id.clone(),
            external_id: self.external_id.clone(),
            paper_key: self.key.as_ref().map(ToString::to_string),
            citekey: self.citekey.clone(),
            ids_json: serde_json::to_string(&self.ids).map_err(|e| e.to_string())?,
            version: self.version.clone(),
            title: self.title.clone(),
            authors_json: serde_json::to_string(&self.authors).map_err(|e| e.to_string())?,
            year: self.year,
            item_kind: self.item_kind.clone(),
            collections_json: serde_json::to_string(&self.collections)
                .map_err(|e| e.to_string())?,
            tags_json: serde_json::to_string(&self.tags).map_err(|e| e.to_string())?,
            files_json: serde_json::to_string(&self.files).map_err(|e| e.to_string())?,
            state: self.state.as_str().to_string(),
            updated_ms: self.updated_ms,
        })
    }

    pub(crate) fn from_row(row: LibrarySourceItemRow) -> Result<Self, String> {
        let key = row.paper_key.as_deref().map(PaperKey::parse).transpose()?;
        Ok(Self {
            source_id: row.source_id,
            external_id: row.external_id,
            key,
            citekey: row.citekey,
            ids: serde_json::from_str(&row.ids_json).unwrap_or_default(),
            version: row.version,
            title: row.title,
            authors: serde_json::from_str(&row.authors_json).unwrap_or_default(),
            year: row.year,
            item_kind: row.item_kind,
            collections: serde_json::from_str(&row.collections_json).unwrap_or_default(),
            tags: serde_json::from_str(&row.tags_json).unwrap_or_default(),
            files: serde_json::from_str(&row.files_json).unwrap_or_default(),
            state: SourceItemState::parse(&row.state)?,
            updated_ms: row.updated_ms,
        })
    }
}

/// Typed `search_cache` row (`explore.md` 4.3, F11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCacheRecord {
    pub service: String,
    pub url_sha256: String,
    pub raw_blob: String,
    pub fetched_ms: i64,
}

impl From<LibrarySearchCacheRow> for SearchCacheRecord {
    fn from(r: LibrarySearchCacheRow) -> Self {
        Self {
            service: r.service,
            url_sha256: r.url_sha256,
            raw_blob: r.raw_blob,
            fetched_ms: r.fetched_ms,
        }
    }
}
