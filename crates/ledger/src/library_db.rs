//! Low-level SQLite storage layer for `<data_dir>/library/library.db` (package F10).
//!
//! Keeps `rusqlite` encapsulated inside the `ledger` crate so `research-tools` exposes a
//! typed API (`PaperKey`, enums, rebuild, user-mark backups) without taking a `rusqlite`
//! dependency.

use crate::fulltext_cache::{
    read_verified_blob, write_blob_atomic, LIBRARY_DB_NAME, LIBRARY_DIR_NAME,
};
pub use crate::library_rows::*;
use crate::library_schema::{current_schema_version, migrate_library_db, ORIGIN_USER_LIBRARY};
use crate::{is_sha256_hex, sha256_hex, Error, Result};
use rusqlite::{
    config::DbConfig, params, Connection, OptionalExtension, TransactionBehavior,
};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Handle to `<data_dir>/library/library.db`.
pub struct LibraryDb {
    pub(crate) conn: Connection,
    data_dir: PathBuf,
    library_dir: PathBuf,
    pub(crate) ledger_blob_dir: PathBuf,
    pub(crate) cache_blob_dir: PathBuf,
}

impl LibraryDb {
    /// Open or create `<data_dir>/library/library.db` and run forward-only migrations.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let library_dir = data_dir.join(LIBRARY_DIR_NAME);
        std::fs::create_dir_all(&library_dir)?;
        let db_path = library_dir.join(LIBRARY_DB_NAME);
        let mut conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        let _ = conn.pragma_update(None, "trusted_schema", "OFF");
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate_library_db(&mut conn)?;
        Ok(Self {
            conn,
            data_dir: data_dir.to_path_buf(),
            library_dir: library_dir.clone(),
            ledger_blob_dir: data_dir.join("blobs"),
            cache_blob_dir: library_dir.join("blobs"),
        })
    }

    pub fn schema_version(&self) -> Result<u32> {
        current_schema_version(&self.conn)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn library_dir(&self) -> &Path {
        &self.library_dir
    }

    pub fn db_path(&self) -> PathBuf {
        self.library_dir.join(LIBRARY_DB_NAME)
    }

    /// Store `bytes` in `<data_dir>/library/blobs/` unless already present in `<data_dir>/blobs/`.
    pub fn put_cache_blob(&self, bytes: &[u8]) -> Result<String> {
        let hash = sha256_hex(bytes);
        if read_verified_blob(&self.ledger_blob_dir, &hash)?.is_none() {
            write_blob_atomic(&self.cache_blob_dir, &hash, bytes)?;
        }
        Ok(hash)
    }

    /// Read a verified SHA-256 blob from `<data_dir>/blobs/` first, then `<data_dir>/library/blobs/`.
    pub fn read_verified_blob_any(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        if hash.is_empty() {
            return Ok(None);
        }
        if let Some(bytes) = read_verified_blob(&self.ledger_blob_dir, hash)? {
            return Ok(Some(bytes));
        }
        read_verified_blob(&self.cache_blob_dir, hash)
    }

    pub fn upsert_paper(&self, row: &LibraryPaperRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        self.conn.execute(
            "INSERT INTO paper (
                 paper_key, ids_json, title, authors_json, year,
                 abstract, bibtex, flags_json, first_seen_ms, last_seen_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(paper_key) DO UPDATE SET
                 ids_json = excluded.ids_json,
                 title = excluded.title,
                 authors_json = excluded.authors_json,
                 year = excluded.year,
                 abstract = excluded.abstract,
                 bibtex = excluded.bibtex,
                 flags_json = excluded.flags_json,
                 first_seen_ms = MIN(paper.first_seen_ms, excluded.first_seen_ms),
                 last_seen_ms = MAX(paper.last_seen_ms, excluded.last_seen_ms)",
            params![
                row.paper_key,
                row.ids_json,
                row.title,
                row.authors_json,
                row.year,
                row.abstract_text,
                row.bibtex,
                row.flags_json,
                row.first_seen_ms,
                row.last_seen_ms,
            ],
        )?;
        self.refresh_fts_for_paper(&row.paper_key)
    }

    pub fn get_paper(&self, paper_key: &str) -> Result<Option<LibraryPaperRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT paper_key, ids_json, title, authors_json, year,
                        abstract, bibtex, flags_json, first_seen_ms, last_seen_ms
                 FROM paper WHERE paper_key = ?1",
                params![paper_key],
                row_to_paper,
            )
            .optional()?;
        Ok(row)
    }

    pub fn put_alias(&self, identifier: &str, paper_key: &str) -> Result<()> {
        validate_non_empty("identifier", identifier)?;
        validate_non_empty("paper_key", paper_key)?;
        self.conn.execute(
            "INSERT INTO alias (identifier, paper_key) VALUES (?1, ?2)
             ON CONFLICT(identifier) DO UPDATE SET paper_key = excluded.paper_key",
            params![identifier, paper_key],
        )?;
        Ok(())
    }

    pub fn resolve_alias(&self, identifier: &str) -> Result<Option<String>> {
        let key = self
            .conn
            .query_row(
                "SELECT paper_key FROM alias WHERE identifier = ?1",
                params![identifier],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(key)
    }

    pub fn list_aliases_for_paper(&self, paper_key: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT identifier FROM alias WHERE paper_key = ?1 ORDER BY identifier ASC")?;
        let rows = stmt.query_map(params![paper_key], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Delete a paper from rebuildable index tables (`paper`, `alias`, `fulltext`, `seen`,
    /// `skeleton`, `finding`, `paper_text`, `paper_fts`). Leaves `user_mark` and `ledger.db` untouched.
    pub fn delete_paper_index_rows(&mut self, paper_key: &str) -> Result<bool> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let n_paper = tx.execute("DELETE FROM paper WHERE paper_key = ?1", params![paper_key])?;
        let n_ft = tx.execute("DELETE FROM fulltext WHERE paper_key = ?1", params![paper_key])?;
        let n_seen = tx.execute("DELETE FROM seen WHERE paper_key = ?1", params![paper_key])?;
        let n_skel = tx.execute("DELETE FROM skeleton WHERE paper_key = ?1", params![paper_key])?;
        let n_find = tx.execute("DELETE FROM finding WHERE paper_key = ?1", params![paper_key])?;
        let n_pt = tx.execute("DELETE FROM paper_text WHERE paper_key = ?1", params![paper_key])?;
        tx.execute("DELETE FROM paper_fts WHERE paper_key = ?1", params![paper_key])?;
        tx.commit()?;
        Ok(n_paper + n_ft + n_seen + n_skel + n_find + n_pt > 0)
    }

    pub fn upsert_fulltext(&self, row: &LibraryFulltextRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        validate_non_empty("format", &row.format)?;
        validate_non_empty("origin", &row.origin)?;
        if row.origin == ORIGIN_USER_LIBRARY {
            if !row.raw_blob.is_empty() && !is_sha256_hex(&row.raw_blob) {
                return Err(Error::BadHash(row.raw_blob.clone()));
            }
        } else if !is_sha256_hex(&row.raw_blob) {
            return Err(Error::BadHash(row.raw_blob.clone()));
        }
        if let Some(sha) = &row.file_sha256 {
            if !is_sha256_hex(sha) {
                return Err(Error::BadHash(sha.clone()));
            }
        }
        let ver = row.version.as_deref().map(str::trim).unwrap_or("");
        let bytes_i64 = i64::try_from(row.bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bytes overflow"))?;
        let file_size_i64 = row
            .file_size
            .map(i64::try_from)
            .transpose()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file_size overflow"))?;
        self.conn.execute(
            "INSERT INTO fulltext (
                 paper_key, version, raw_blob, format, bytes, origin, fetched_ms,
                 file_path, file_size, file_mtime_ms, file_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(paper_key, version) DO UPDATE SET
                 raw_blob = excluded.raw_blob,
                 format = excluded.format,
                 bytes = excluded.bytes,
                 origin = excluded.origin,
                 fetched_ms = excluded.fetched_ms,
                 file_path = excluded.file_path,
                 file_size = excluded.file_size,
                 file_mtime_ms = excluded.file_mtime_ms,
                 file_sha256 = excluded.file_sha256",
            params![
                row.paper_key,
                ver,
                row.raw_blob,
                row.format,
                bytes_i64,
                row.origin,
                row.fetched_ms,
                row.file_path,
                file_size_i64,
                row.file_mtime_ms,
                row.file_sha256,
            ],
        )?;
        Ok(())
    }

    pub fn list_fulltext_for_paper(&self, paper_key: &str) -> Result<Vec<LibraryFulltextRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT paper_key, version, raw_blob, format, bytes, origin, fetched_ms,
                    file_path, file_size, file_mtime_ms, file_sha256
             FROM fulltext
             WHERE paper_key = ?1
             ORDER BY (CASE WHEN raw_blob = '' THEN 1 ELSE 0 END) ASC,
                      (CASE WHEN format = 'pdf' THEN 1 ELSE 0 END) ASC,
                      fetched_ms DESC, version DESC",
        )?;
        let rows = stmt.query_map(params![paper_key], row_to_fulltext)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn record_seen(&self, row: &LibrarySeenRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        validate_non_empty("session_id", &row.session_id)?;
        self.conn.execute(
            "INSERT INTO seen (paper_key, session_id, role, seen_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(paper_key, session_id, role) DO UPDATE SET
                 seen_ms = MAX(seen.seen_ms, excluded.seen_ms)",
            params![row.paper_key, row.session_id, row.role, row.seen_ms],
        )?;
        Ok(())
    }

    pub fn list_seen_for_paper(&self, paper_key: &str) -> Result<Vec<LibrarySeenRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT paper_key, session_id, role, seen_ms
             FROM seen WHERE paper_key = ?1
             ORDER BY seen_ms DESC, session_id ASC, role ASC",
        )?;
        let rows = stmt.query_map(params![paper_key], |r| {
            Ok(LibrarySeenRow {
                paper_key: r.get(0)?,
                session_id: r.get(1)?,
                role: r.get(2)?,
                seen_ms: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn upsert_skeleton(&self, row: &LibrarySkeletonRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        validate_non_empty("builder", &row.builder)?;
        if !is_sha256_hex(&row.blob_sha256) {
            return Err(Error::BadHash(row.blob_sha256.clone()));
        }
        let ver = row.version.as_deref().map(str::trim).unwrap_or("");
        self.conn.execute(
            "INSERT INTO skeleton (paper_key, version, builder, blob_sha256, built_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(paper_key, version, builder) DO UPDATE SET
                 blob_sha256 = excluded.blob_sha256,
                 built_ms = excluded.built_ms",
            params![row.paper_key, ver, row.builder, row.blob_sha256, row.built_ms],
        )?;
        Ok(())
    }

    pub fn get_skeleton(
        &self,
        paper_key: &str,
        version: Option<&str>,
        builder: &str,
    ) -> Result<Option<LibrarySkeletonRow>> {
        let ver = version.map(str::trim).unwrap_or("");
        let row = self
            .conn
            .query_row(
                "SELECT paper_key, version, builder, blob_sha256, built_ms
                 FROM skeleton WHERE paper_key = ?1 AND version = ?2 AND builder = ?3",
                params![paper_key, ver, builder],
                row_to_skeleton,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_skeletons_for_paper(&self, paper_key: &str) -> Result<Vec<LibrarySkeletonRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT paper_key, version, builder, blob_sha256, built_ms
             FROM skeleton WHERE paper_key = ?1
             ORDER BY built_ms DESC, version DESC, builder ASC",
        )?;
        let rows = stmt.query_map(params![paper_key], row_to_skeleton)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn upsert_finding(&self, row: &LibraryFindingRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        validate_non_empty("session_id", &row.session_id)?;
        let seq = i64::try_from(row.evidence_seq)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "evidence_seq overflow"))?;
        self.conn.execute(
            "INSERT INTO finding (
                 paper_key, session_id, evidence_seq, role, gist, quote,
                 cited_start, cited_end, recorded_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(paper_key, session_id, evidence_seq) DO UPDATE SET
                 role = excluded.role,
                 gist = excluded.gist,
                 quote = excluded.quote,
                 cited_start = excluded.cited_start,
                 cited_end = excluded.cited_end,
                 recorded_ms = excluded.recorded_ms",
            params![
                row.paper_key,
                row.session_id,
                seq,
                row.role,
                row.gist,
                row.quote,
                row.cited_start.map(|v| v as i64),
                row.cited_end.map(|v| v as i64),
                row.recorded_ms,
            ],
        )?;
        self.refresh_fts_for_paper(&row.paper_key)
    }

    pub fn list_findings_for_paper(&self, paper_key: &str) -> Result<Vec<LibraryFindingRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT paper_key, session_id, evidence_seq, role, gist, quote,
                    cited_start, cited_end, recorded_ms
             FROM finding WHERE paper_key = ?1
             ORDER BY recorded_ms DESC, session_id ASC, evidence_seq ASC",
        )?;
        let rows = stmt.query_map(params![paper_key], |r| {
            let seq: i64 = r.get(2)?;
            let start: Option<i64> = r.get(6)?;
            let end: Option<i64> = r.get(7)?;
            Ok(LibraryFindingRow {
                paper_key: r.get(0)?,
                session_id: r.get(1)?,
                evidence_seq: seq.max(0) as u64,
                role: r.get(3)?,
                gist: r.get(4)?,
                quote: r.get(5)?,
                cited_start: start.and_then(|v| u64::try_from(v).ok()),
                cited_end: end.and_then(|v| u64::try_from(v).ok()),
                recorded_ms: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn upsert_user_mark(&self, row: &LibraryUserMarkRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        self.conn.execute(
            "INSERT INTO user_mark (paper_key, status, tags_json, note, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(paper_key) DO UPDATE SET
                 status = excluded.status,
                 tags_json = excluded.tags_json,
                 note = excluded.note,
                 updated_ms = excluded.updated_ms",
            params![row.paper_key, row.status, row.tags_json, row.note, row.updated_ms],
        )?;
        Ok(())
    }

    pub fn get_user_mark(&self, paper_key: &str) -> Result<Option<LibraryUserMarkRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT paper_key, status, tags_json, note, updated_ms
                 FROM user_mark WHERE paper_key = ?1",
                params![paper_key],
                row_to_user_mark,
            )
            .optional()?;
        Ok(row)
    }

    pub fn delete_user_mark(&self, paper_key: &str) -> Result<bool> {
        let n = self.conn.execute("DELETE FROM user_mark WHERE paper_key = ?1", params![paper_key])?;
        Ok(n > 0)
    }

    pub fn list_user_marks(&self) -> Result<Vec<LibraryUserMarkRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT paper_key, status, tags_json, note, updated_ms
             FROM user_mark ORDER BY paper_key ASC",
        )?;
        let rows = stmt.query_map([], row_to_user_mark)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn replace_all_user_marks(&mut self, marks: &[LibraryUserMarkRow]) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM user_mark", [])?;
        for row in marks {
            validate_non_empty("paper_key", &row.paper_key)?;
            tx.execute(
                "INSERT INTO user_mark (paper_key, status, tags_json, note, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![row.paper_key, row.status, row.tags_json, row.note, row.updated_ms],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("library field `{field}` must not be empty"),
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "library_db_tests.rs"]
mod tests;
