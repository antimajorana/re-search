//! SQLite persistence for `<data_dir>/library/lessons.db` (package F16a, `00-overview.md` 2.14).
//!
//! Kept in a separate file from `library.db` because corrections and lessons are user data
//! (not rebuildable from the ledger, local only, included in backups, and exported only when
//! the user explicitly opts in — `00-overview.md` section 8.2 decision 5 & 8.3 decision 7).
//!
//! No `tokio`, `rmcp`, or `windows-sys` dependencies.

use crate::fulltext_cache::LIBRARY_DIR_NAME;
use crate::{is_sha256_hex, Error, Result};
use rusqlite::{
    config::DbConfig, params, Connection, OpenFlags, OptionalExtension, TransactionBehavior,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Filename of the non-rebuildable corrections and lessons store (`00-overview.md` 2.14).
pub const LESSONS_DB_NAME: &str = "lessons.db";

/// Current SQLite `PRAGMA user_version` for `lessons.db` (forward-only migrations).
pub const LESSONS_SCHEMA_VERSION: u32 = 2;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER NOT NULL PRIMARY KEY CHECK (version >= 1),
    applied_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS term_alias (
    term_a           TEXT    NOT NULL,
    term_b           TEXT    NOT NULL,
    decision         TEXT    NOT NULL CHECK (decision IN ('confirmed', 'rejected')),
    origin_session   TEXT    NOT NULL DEFAULT '',
    origin_node_json TEXT    NOT NULL DEFAULT 'null' CHECK (json_valid(origin_node_json)),
    updated_ms       INTEGER NOT NULL,
    PRIMARY KEY (term_a, term_b)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS symbol_default (
    symbol           TEXT    NOT NULL,
    domain           TEXT    NOT NULL DEFAULT '',
    assumptions_json TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(assumptions_json)),
    dimensions       TEXT,
    convention_note  TEXT,
    origin_session   TEXT    NOT NULL DEFAULT '',
    updated_ms       INTEGER NOT NULL,
    PRIMARY KEY (symbol, domain)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS convention_default (
    conv_key       TEXT    NOT NULL,
    conv_value     TEXT    NOT NULL,
    domain         TEXT    NOT NULL DEFAULT '',
    origin_session TEXT    NOT NULL DEFAULT '',
    updated_ms     INTEGER NOT NULL,
    PRIMARY KEY (conv_key, domain)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS skeleton_override (
    node_tag          TEXT    NOT NULL PRIMARY KEY,
    paper_key         TEXT,
    kind_override     TEXT,
    end_line_override INTEGER CHECK (end_line_override IS NULL OR end_line_override >= 1),
    note              TEXT    NOT NULL DEFAULT '',
    origin_session    TEXT    NOT NULL DEFAULT '',
    updated_ms        INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS ocr_overlay (
    file_sha256     TEXT    NOT NULL CHECK (length(file_sha256) = 64 AND file_sha256 NOT GLOB '*[^0-9a-f]*'),
    line_no         INTEGER NOT NULL CHECK (line_no >= 1),
    corrected_text  TEXT    NOT NULL,
    original_sha256 TEXT    CHECK (
        original_sha256 IS NULL
        OR (length(original_sha256) = 64 AND original_sha256 NOT GLOB '*[^0-9a-f]*')
    ),
    origin_session  TEXT    NOT NULL DEFAULT '',
    updated_ms      INTEGER NOT NULL,
    PRIMARY KEY (file_sha256, line_no)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS ingested_cursor (
    session_id TEXT    NOT NULL PRIMARY KEY,
    last_seq   INTEGER NOT NULL CHECK (last_seq >= 0),
    updated_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
"#;

pub use crate::lessons_rows::{
    ConventionDefaultRow, LessonsSnapshot, OcrOverlayRow, SkeletonOverrideRow, SymbolDefaultRow,
    TermAliasRow,
};

/// Handle to `<data_dir>/library/lessons.db`.
pub struct LessonsDb {
    conn: Connection,
    db_path: PathBuf,
}

impl LessonsDb {
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

impl LessonsDb {
    /// Open or create `<data_dir>/library/lessons.db`, running forward-only schema migrations.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let library_dir = data_dir.join(LIBRARY_DIR_NAME);
        fs::create_dir_all(&library_dir)?;
        let db_path = library_dir.join(LESSONS_DB_NAME);
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = Connection::open_with_flags(&db_path, flags)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        let _ = conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true);
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;",
        )?;
        migrate_lessons_db(&mut conn)?;
        Ok(Self { conn, db_path })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn schema_version(&self) -> Result<u32> {
        let v: u32 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        Ok(v)
    }

    pub fn upsert_term_alias(&self, row: &TermAliasRow) -> Result<()> {
        validate_non_empty("term_a", &row.term_a)?;
        validate_non_empty("term_b", &row.term_b)?;
        self.conn.execute(
            "INSERT INTO term_alias (term_a, term_b, decision, origin_session, origin_node_json, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(term_a, term_b) DO UPDATE SET
                 decision = excluded.decision,
                 origin_session = excluded.origin_session,
                 origin_node_json = excluded.origin_node_json,
                 updated_ms = excluded.updated_ms",
            params![
                row.term_a,
                row.term_b,
                row.decision,
                row.origin_session,
                row.origin_node_json,
                row.updated_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_term_aliases(&self) -> Result<Vec<TermAliasRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT term_a, term_b, decision, origin_session, origin_node_json, updated_ms
             FROM term_alias ORDER BY term_a ASC, term_b ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(TermAliasRow {
                term_a: r.get(0)?,
                term_b: r.get(1)?,
                decision: r.get(2)?,
                origin_session: r.get(3)?,
                origin_node_json: r.get(4)?,
                updated_ms: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_term_alias(&self, term_a: &str, term_b: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM term_alias WHERE term_a = ?1 AND term_b = ?2",
            params![term_a, term_b],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_symbol_default(&self, row: &SymbolDefaultRow) -> Result<()> {
        validate_non_empty("symbol", &row.symbol)?;
        self.conn.execute(
            "INSERT INTO symbol_default (
                 symbol, domain, assumptions_json, dimensions, convention_note, origin_session, updated_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(symbol, domain) DO UPDATE SET
                 assumptions_json = excluded.assumptions_json,
                 dimensions = excluded.dimensions,
                 convention_note = excluded.convention_note,
                 origin_session = excluded.origin_session,
                 updated_ms = excluded.updated_ms",
            params![
                row.symbol,
                row.domain,
                row.assumptions_json,
                row.dimensions,
                row.convention_note,
                row.origin_session,
                row.updated_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_symbol_defaults(&self) -> Result<Vec<SymbolDefaultRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT symbol, domain, assumptions_json, dimensions, convention_note, origin_session, updated_ms
             FROM symbol_default ORDER BY symbol ASC, domain ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SymbolDefaultRow {
                symbol: r.get(0)?,
                domain: r.get(1)?,
                assumptions_json: r.get(2)?,
                dimensions: r.get(3)?,
                convention_note: r.get(4)?,
                origin_session: r.get(5)?,
                updated_ms: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_symbol_default(&self, symbol: &str, domain: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM symbol_default WHERE symbol = ?1 AND domain = ?2",
            params![symbol, domain],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_convention_default(&self, row: &ConventionDefaultRow) -> Result<()> {
        validate_non_empty("conv_key", &row.conv_key)?;
        self.conn.execute(
            "INSERT INTO convention_default (conv_key, conv_value, domain, origin_session, updated_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(conv_key, domain) DO UPDATE SET
                 conv_value = excluded.conv_value,
                 origin_session = excluded.origin_session,
                 updated_ms = excluded.updated_ms",
            params![
                row.conv_key,
                row.conv_value,
                row.domain,
                row.origin_session,
                row.updated_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_convention_defaults(&self) -> Result<Vec<ConventionDefaultRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT conv_key, conv_value, domain, origin_session, updated_ms
             FROM convention_default ORDER BY conv_key ASC, domain ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ConventionDefaultRow {
                conv_key: r.get(0)?,
                conv_value: r.get(1)?,
                domain: r.get(2)?,
                origin_session: r.get(3)?,
                updated_ms: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_convention_default(&self, conv_key: &str, domain: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM convention_default WHERE conv_key = ?1 AND domain = ?2",
            params![conv_key, domain],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_skeleton_override(&self, row: &SkeletonOverrideRow) -> Result<()> {
        validate_non_empty("node_tag", &row.node_tag)?;
        self.conn.execute(
            "INSERT INTO skeleton_override (
                 node_tag, paper_key, kind_override, end_line_override, note, origin_session, updated_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(node_tag) DO UPDATE SET
                 paper_key = excluded.paper_key,
                 kind_override = excluded.kind_override,
                 end_line_override = excluded.end_line_override,
                 note = excluded.note,
                 origin_session = excluded.origin_session,
                 updated_ms = excluded.updated_ms",
            params![
                row.node_tag,
                row.paper_key,
                row.kind_override,
                row.end_line_override,
                row.note,
                row.origin_session,
                row.updated_ms,
            ],
        )?;
        Ok(())
    }

    pub fn get_skeleton_override(&self, node_tag: &str) -> Result<Option<SkeletonOverrideRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT node_tag, paper_key, kind_override, end_line_override, note, origin_session, updated_ms
                 FROM skeleton_override WHERE node_tag = ?1",
                params![node_tag],
                |r| {
                    Ok(SkeletonOverrideRow {
                        node_tag: r.get(0)?,
                        paper_key: r.get(1)?,
                        kind_override: r.get(2)?,
                        end_line_override: r.get(3)?,
                        note: r.get(4)?,
                        origin_session: r.get(5)?,
                        updated_ms: r.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_skeleton_overrides(&self) -> Result<Vec<SkeletonOverrideRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_tag, paper_key, kind_override, end_line_override, note, origin_session, updated_ms
             FROM skeleton_override ORDER BY node_tag ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SkeletonOverrideRow {
                node_tag: r.get(0)?,
                paper_key: r.get(1)?,
                kind_override: r.get(2)?,
                end_line_override: r.get(3)?,
                note: r.get(4)?,
                origin_session: r.get(5)?,
                updated_ms: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_skeleton_override(&self, node_tag: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM skeleton_override WHERE node_tag = ?1",
            params![node_tag],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_ocr_overlay(&self, row: &OcrOverlayRow) -> Result<()> {
        if !is_sha256_hex(&row.file_sha256) {
            return Err(Error::BadHash(row.file_sha256.clone()));
        }
        if let Some(orig) = &row.original_sha256 {
            if !is_sha256_hex(orig) {
                return Err(Error::BadHash(orig.clone()));
            }
        }
        self.conn.execute(
            "INSERT INTO ocr_overlay (
                 file_sha256, line_no, corrected_text, original_sha256, origin_session, updated_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(file_sha256, line_no) DO UPDATE SET
                 corrected_text = excluded.corrected_text,
                 original_sha256 = excluded.original_sha256,
                 origin_session = excluded.origin_session,
                 updated_ms = excluded.updated_ms",
            params![
                row.file_sha256,
                row.line_no,
                row.corrected_text,
                row.original_sha256,
                row.origin_session,
                row.updated_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_ocr_overlays(&self, file_sha256: Option<&str>) -> Result<Vec<OcrOverlayRow>> {
        let mut out = Vec::new();
        if let Some(sha) = file_sha256 {
            let mut stmt = self.conn.prepare(
                "SELECT file_sha256, line_no, corrected_text, original_sha256, origin_session, updated_ms
                 FROM ocr_overlay WHERE file_sha256 = ?1 ORDER BY line_no ASC",
            )?;
            let rows = stmt.query_map(params![sha], row_to_ocr_overlay)?;
            for r in rows {
                out.push(r?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT file_sha256, line_no, corrected_text, original_sha256, origin_session, updated_ms
                 FROM ocr_overlay ORDER BY file_sha256 ASC, line_no ASC",
            )?;
            let rows = stmt.query_map([], row_to_ocr_overlay)?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }

    pub fn delete_ocr_overlay(&self, file_sha256: &str, line_no: i64) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM ocr_overlay WHERE file_sha256 = ?1 AND line_no = ?2",
            params![file_sha256, line_no],
        )?;
        Ok(n > 0)
    }

    pub fn get_ingested_cursor(&self, session_id: &str) -> Result<u64> {
        let seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT last_seq FROM ingested_cursor WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(seq.unwrap_or(0).max(0) as u64)
    }

    pub fn set_ingested_cursor(&self, session_id: &str, last_seq: u64, updated_ms: i64) -> Result<()> {
        validate_non_empty("session_id", session_id)?;
        self.conn.execute(
            "INSERT INTO ingested_cursor (session_id, last_seq, updated_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                 last_seq = excluded.last_seq,
                 updated_ms = excluded.updated_ms",
            params![session_id, last_seq as i64, updated_ms],
        )?;
        Ok(())
    }

    pub fn export_snapshot(&self) -> Result<LessonsSnapshot> {
        Ok(LessonsSnapshot {
            term_aliases: self.list_term_aliases()?,
            symbol_defaults: self.list_symbol_defaults()?,
            convention_defaults: self.list_convention_defaults()?,
            skeleton_overrides: self.list_skeleton_overrides()?,
            ocr_overlays: self.list_ocr_overlays(None)?,
        })
    }

    /// Atomically replace all F16a correction rows from a verified backup snapshot.
    pub fn replace_snapshot(&mut self, snap: &LessonsSnapshot) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM term_alias", [])?;
        tx.execute("DELETE FROM symbol_default", [])?;
        tx.execute("DELETE FROM convention_default", [])?;
        tx.execute("DELETE FROM skeleton_override", [])?;
        tx.execute("DELETE FROM ocr_overlay", [])?;
        for r in &snap.term_aliases {
            tx.execute(
                "INSERT INTO term_alias (term_a, term_b, decision, origin_session, origin_node_json, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![r.term_a, r.term_b, r.decision, r.origin_session, r.origin_node_json, r.updated_ms],
            )?;
        }
        for r in &snap.symbol_defaults {
            tx.execute(
                "INSERT INTO symbol_default (symbol, domain, assumptions_json, dimensions, convention_note, origin_session, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![r.symbol, r.domain, r.assumptions_json, r.dimensions, r.convention_note, r.origin_session, r.updated_ms],
            )?;
        }
        for r in &snap.convention_defaults {
            tx.execute(
                "INSERT INTO convention_default (conv_key, conv_value, domain, origin_session, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![r.conv_key, r.conv_value, r.domain, r.origin_session, r.updated_ms],
            )?;
        }
        for r in &snap.skeleton_overrides {
            tx.execute(
                "INSERT INTO skeleton_override (node_tag, paper_key, kind_override, end_line_override, note, origin_session, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![r.node_tag, r.paper_key, r.kind_override, r.end_line_override, r.note, r.origin_session, r.updated_ms],
            )?;
        }
        for r in &snap.ocr_overlays {
            tx.execute(
                "INSERT INTO ocr_overlay (file_sha256, line_no, corrected_text, original_sha256, origin_session, updated_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![r.file_sha256, r.line_no, r.corrected_text, r.original_sha256, r.origin_session, r.updated_ms],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn row_to_ocr_overlay(r: &rusqlite::Row<'_>) -> rusqlite::Result<OcrOverlayRow> {
    Ok(OcrOverlayRow {
        file_sha256: r.get(0)?,
        line_no: r.get(1)?,
        corrected_text: r.get(2)?,
        original_sha256: r.get(3)?,
        origin_session: r.get(4)?,
        updated_ms: r.get(5)?,
    })
}

fn validate_non_empty(field: &str, val: &str) -> Result<()> {
    if val.trim().is_empty() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{field} must not be empty"),
        )));
    }
    Ok(())
}

fn migrate_lessons_db(conn: &mut Connection) -> Result<()> {
    let cur: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if cur > LESSONS_SCHEMA_VERSION {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "lessons.db schema version {cur} is newer than supported version {LESSONS_SCHEMA_VERSION}"
            ),
        )));
    }
    if cur == LESSONS_SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    if cur < 1 {
        tx.execute_batch(SCHEMA_V1)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations (version, applied_ms) VALUES (?1, ?2)",
            params![1_i64, now_ms],
        )?;
    }
    if cur < 2 {
        crate::lessons_b_store::migrate_v2(&tx)?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations (version, applied_ms) VALUES (?1, ?2)",
            params![2_i64, now_ms],
        )?;
    }
    tx.pragma_update(None, "user_version", LESSONS_SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "lessons_db_tests.rs"]
mod tests;
