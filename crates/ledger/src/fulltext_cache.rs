//! Minimal rebuildable full-text fetch cache under `<data_dir>/library/library.db`
//! (shared package F3; extended by F10).
//!
//! This cache is strictly an index and rebuildable cache, never evidence. Cached bytes
//! enter a session only through `research_tools::ingest::ingest_bytes`, which writes the
//! session's own hash-chained `source.ingested` event and pins the raw blob in the ledger.
//!
//! Blobs are read from `<data_dir>/blobs/` first (zero duplication when already ingested
//! into any session) and fall back to `<data_dir>/library/blobs/` so pre-ingest cache hits
//! do not leave unreferenced blobs in the ledger's evidence blob store. Every read checks
//! the SHA-256 digest against the stored `raw_blob` hash and treats missing or mismatched
//! files as a cache miss.

use crate::{is_sha256_hex, now_ms, sha256_hex, Error, Result};
use rusqlite::{config::DbConfig, params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_CACHE_TEMP: AtomicU64 = AtomicU64::new(0);

/// Subdirectory under `<data_dir>` for the shared library cache.
/// Provisional pending `00-overview.md` section 8, decision 5 (one library for all modes).
pub const LIBRARY_DIR_NAME: &str = "library";

/// SQLite database filename under `<data_dir>/<LIBRARY_DIR_NAME>/`.
/// Provisional pending `00-overview.md` section 8, decision 5.
pub const LIBRARY_DB_NAME: &str = "library.db";

/// SQLite table name for cached full-text paper pointers.
/// Provisional pending `00-overview.md` section 8, decision 5.
pub const FULLTEXT_TABLE: &str = "fulltext";

const FULLTEXT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS fulltext (
    paper_key  TEXT    NOT NULL,
    version    TEXT    NOT NULL,
    raw_blob   TEXT    NOT NULL CHECK (length(raw_blob) = 64 AND raw_blob NOT GLOB '*[^0-9a-f]*'),
    format     TEXT    NOT NULL,
    bytes      INTEGER NOT NULL CHECK (bytes >= 0),
    origin     TEXT    NOT NULL,
    fetched_ms INTEGER NOT NULL,
    PRIMARY KEY (paper_key, version)
) STRICT, WITHOUT ROWID;
"#;

/// One row of the rebuildable `fulltext` cache table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulltextEntry {
    pub paper_key: String,
    pub version: Option<String>,
    pub raw_blob: String,
    pub format: String,
    pub bytes: u64,
    pub origin: String,
    pub fetched_ms: i64,
}

/// Handle to `<data_dir>/library/library.db` for the minimal `fulltext` table.
pub struct FulltextCache {
    conn: Connection,
    ledger_blob_dir: PathBuf,
    cache_blob_dir: PathBuf,
}

impl FulltextCache {
    /// Open or create `<data_dir>/library/library.db` and ensure the `fulltext` table exists.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let lib_dir = data_dir.join(LIBRARY_DIR_NAME);
        std::fs::create_dir_all(&lib_dir)?;
        let db_path = lib_dir.join(LIBRARY_DB_NAME);
        let conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        let _ = conn.pragma_update(None, "trusted_schema", "OFF");
        conn.execute_batch(FULLTEXT_SCHEMA)?;
        Ok(Self {
            conn,
            ledger_blob_dir: data_dir.join("blobs"),
            cache_blob_dir: lib_dir.join("blobs"),
        })
    }

    /// Store `bytes` in the rebuildable cache blob store (unless already present in the
    /// ledger or cache blob store) and upsert the `fulltext` row for `(paper_key, version)`.
    pub fn put_bytes(
        &self,
        paper_key: &str,
        version: Option<&str>,
        format: &str,
        origin: &str,
        bytes: &[u8],
    ) -> Result<FulltextEntry> {
        validate_non_empty("paper_key", paper_key)?;
        validate_non_empty("format", format)?;
        validate_non_empty("origin", origin)?;
        let raw_blob = sha256_hex(bytes);
        let ledger_path = content_blob_path(&self.ledger_blob_dir, &raw_blob)?;
        if !ledger_path.is_file() {
            write_blob_atomic(&self.cache_blob_dir, &raw_blob, bytes)?;
        }
        self.upsert_row(paper_key, version, &raw_blob, format, bytes.len() as u64, origin)
    }

    /// Index an existing content-addressed ledger blob without copying its bytes.
    pub fn index_existing_blob(
        &self,
        paper_key: &str,
        version: Option<&str>,
        raw_blob: &str,
        format: &str,
        bytes: u64,
        origin: &str,
    ) -> Result<FulltextEntry> {
        validate_non_empty("paper_key", paper_key)?;
        validate_non_empty("format", format)?;
        validate_non_empty("origin", origin)?;
        if !is_sha256_hex(raw_blob) {
            return Err(Error::BadHash(raw_blob.to_string()));
        }
        self.upsert_row(paper_key, version, raw_blob, format, bytes, origin)
    }

    fn upsert_row(
        &self,
        paper_key: &str,
        version: Option<&str>,
        raw_blob: &str,
        format: &str,
        bytes: u64,
        origin: &str,
    ) -> Result<FulltextEntry> {
        let ver_col = version.map(str::trim).unwrap_or("");
        let fetched_ms = now_ms();
        let bytes_i64 = i64::try_from(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "blob size overflow"))?;
        self.conn.execute(
            "INSERT INTO fulltext (paper_key, version, raw_blob, format, bytes, origin, fetched_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(paper_key, version) DO UPDATE SET
                 raw_blob = excluded.raw_blob,
                 format = excluded.format,
                 bytes = excluded.bytes,
                 origin = excluded.origin,
                 fetched_ms = excluded.fetched_ms",
            params![paper_key, ver_col, raw_blob, format, bytes_i64, origin, fetched_ms],
        )?;
        Ok(FulltextEntry {
            paper_key: paper_key.to_string(),
            version: (!ver_col.is_empty()).then(|| ver_col.to_string()),
            raw_blob: raw_blob.to_string(),
            format: format.to_string(),
            bytes,
            origin: origin.to_string(),
            fetched_ms,
        })
    }

    /// Look up cache metadata for `(paper_key, version)`.
    ///
    /// When `version` is `Some(v)`, only that exact version matches. When `version` is
    /// `None`, returns the most recently cached entry for `paper_key` (preferring non-PDF
    /// TeX sources when timestamps tie).
    pub fn get_entry(&self, paper_key: &str, version: Option<&str>) -> Result<Option<FulltextEntry>> {
        let row = match version.map(str::trim).filter(|v| !v.is_empty()) {
            Some(ver) => self
                .conn
                .query_row(
                    "SELECT paper_key, version, raw_blob, format, bytes, origin, fetched_ms
                     FROM fulltext WHERE paper_key = ?1 AND version = ?2",
                    params![paper_key, ver],
                    row_to_entry,
                )
                .optional()?,
            None => self
                .conn
                .query_row(
                    "SELECT paper_key, version, raw_blob, format, bytes, origin, fetched_ms
                     FROM fulltext WHERE paper_key = ?1
                     ORDER BY (CASE WHEN raw_blob = '' THEN 1 ELSE 0 END) ASC,
                              (CASE WHEN format = 'pdf' THEN 1 ELSE 0 END) ASC,
                              fetched_ms DESC, version DESC
                     LIMIT 1",
                    params![paper_key],
                    row_to_entry,
                )
                .optional()?,
        };
        Ok(row)
    }

    /// Look up cached bytes for `(paper_key, version)`, verifying the SHA-256 digest.
    ///
    /// Checks `<data_dir>/blobs/` first and `<data_dir>/library/blobs/` second. Missing,
    /// unhashed (`user_library` with empty `raw_blob`), or digest-mismatched files are
    /// treated as a cache miss (`Ok(None)`) because this cache is rebuildable and never
    /// trusted without hash verification.
    pub fn get_bytes(
        &self,
        paper_key: &str,
        version: Option<&str>,
    ) -> Result<Option<(FulltextEntry, Vec<u8>)>> {
        let Some(entry) = self.get_entry(paper_key, version)? else {
            return Ok(None);
        };
        if entry.raw_blob.is_empty() {
            return Ok(None);
        }
        if let Some(bytes) = read_verified_blob(&self.ledger_blob_dir, &entry.raw_blob)? {
            return Ok(Some((entry, bytes)));
        }
        if let Some(bytes) = read_verified_blob(&self.cache_blob_dir, &entry.raw_blob)? {
            return Ok(Some((entry, bytes)));
        }
        Ok(None)
    }

    /// Remove cached index row(s) for `paper_key` (one version when `Some`, all when `None`).
    pub fn remove(&self, paper_key: &str, version: Option<&str>) -> Result<usize> {
        let count = match version.map(str::trim).filter(|v| !v.is_empty()) {
            Some(ver) => self.conn.execute(
                "DELETE FROM fulltext WHERE paper_key = ?1 AND version = ?2",
                params![paper_key, ver],
            )?,
            None => self.conn.execute(
                "DELETE FROM fulltext WHERE paper_key = ?1",
                params![paper_key],
            )?,
        };
        Ok(count)
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("fulltext cache field `{field}` must not be empty"),
        )
        .into());
    }
    Ok(())
}

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<FulltextEntry> {
    let ver: String = r.get(1)?;
    let bytes: i64 = r.get(4)?;
    Ok(FulltextEntry {
        paper_key: r.get(0)?,
        version: (!ver.is_empty()).then_some(ver),
        raw_blob: r.get(2)?,
        format: r.get(3)?,
        bytes: bytes.max(0) as u64,
        origin: r.get(5)?,
        fetched_ms: r.get(6)?,
    })
}

pub(crate) fn content_blob_path(blob_dir: &Path, hash: &str) -> Result<PathBuf> {
    if !is_sha256_hex(hash) {
        return Err(Error::BadHash(hash.to_string()));
    }
    Ok(blob_dir.join(&hash[..2]).join(hash))
}

pub(crate) fn read_verified_blob(blob_dir: &Path, hash: &str) -> Result<Option<Vec<u8>>> {
    let path = content_blob_path(blob_dir, hash)?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if sha256_hex(&bytes) != hash {
        return Ok(None);
    }
    Ok(Some(bytes))
}

pub(crate) fn write_blob_atomic(blob_dir: &Path, hash: &str, bytes: &[u8]) -> Result<()> {
    let path = content_blob_path(blob_dir, hash)?;
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let (tmp, mut file) = loop {
        let seq = NEXT_CACHE_TEMP.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!("tmp{}.{seq}", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let written = file.write_all(bytes);
    drop(file);
    if let Err(e) = written.and_then(|_| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        if !path.is_file() {
            return Err(e.into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "fulltext_cache_tests.rs"]
mod tests;
