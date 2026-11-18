//! Append-only event ledger on SQLite, plus a content-addressed blob directory.
//!
//! The crate exposes `append` and read functions only; there is no raw SQL
//! access. The schema itself rejects UPDATE, DELETE, INSERT OR REPLACE and
//! out-of-order sequence numbers, so a second connection that bypasses this
//! crate is refused as well (see `tests/schema_rejects.rs`).
//!
//! This is an audit record, not a tamper-proof one: anyone who can write the
//! database file can drop the triggers or replace the file. v0 has no hash
//! chain.

use rusqlite::{config::DbConfig, params, Connection};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS event (
    session        TEXT    NOT NULL,
    seq            INTEGER NOT NULL CHECK (seq >= 1),
    ts_ms          INTEGER NOT NULL,
    kind           TEXT    NOT NULL,
    schema_version INTEGER NOT NULL,
    payload        TEXT    NOT NULL CHECK (json_valid(payload)),
    PRIMARY KEY (session, seq)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER IF NOT EXISTS event_no_update BEFORE UPDATE ON event
BEGIN SELECT RAISE(ABORT, 'ledger is append-only: UPDATE rejected'); END;

CREATE TRIGGER IF NOT EXISTS event_no_delete BEFORE DELETE ON event
BEGIN SELECT RAISE(ABORT, 'ledger is append-only: DELETE rejected'); END;

CREATE TRIGGER IF NOT EXISTS event_seq_in_order BEFORE INSERT ON event
WHEN NEW.seq <> (SELECT COALESCE(MAX(seq), 0) + 1 FROM event WHERE session = NEW.session)
BEGIN SELECT RAISE(ABORT, 'ledger seq must be max(seq)+1 for the session'); END;
"#;

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    BadHash(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "ledger sqlite error: {e}"),
            Error::Io(e) => write!(f, "ledger io error: {e}"),
            Error::Json(e) => write!(f, "ledger json error: {e}"),
            Error::BadHash(h) => write!(f, "not a sha-256 hex digest: {h}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sqlite(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub session: String,
    pub seq: u64,
    pub ts_ms: i64,
    pub kind: String,
    pub schema_version: u32,
    pub payload: Value,
}

pub struct Ledger {
    conn: Connection,
    blob_dir: PathBuf,
}

/// SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Ledger {
    /// Open (creating if needed) the ledger in `dir`: `dir/ledger.db` and
    /// `dir/blobs/`. Several processes may open the same directory.
    pub fn open(dir: &Path) -> Result<Ledger> {
        let blob_dir = dir.join("blobs");
        std::fs::create_dir_all(&blob_dir)?;
        let conn = Connection::open(dir.join("ledger.db"))?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "recursive_triggers", "ON")?;
        conn.pragma_update(None, "trusted_schema", "OFF")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Ledger { conn, blob_dir })
    }

    /// Append one event and return its sequence number (1, 2, 3, ... per
    /// session). The number is chosen inside a write transaction, so two
    /// processes appending to one session do not collide.
    pub fn append(&self, session: &str, kind: &str, schema_version: u32, payload: &Value) -> Result<u64> {
        let text = serde_json::to_string(payload)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let run = || -> Result<u64> {
            let seq: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM event WHERE session = ?1",
                params![session],
                |r| r.get(0),
            )?;
            self.conn.execute(
                "INSERT INTO event (session, seq, ts_ms, kind, schema_version, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![session, seq, now_ms(), kind, schema_version, text],
            )?;
            Ok(seq as u64)
        };
        match run() {
            Ok(seq) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(seq)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// All events of one session, in order.
    pub fn read_session(&self, session: &str) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT session, seq, ts_ms, kind, schema_version, payload
             FROM event WHERE session = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![session], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (session, seq, ts_ms, kind, ver, payload) = row?;
            out.push(Event {
                session,
                seq: seq as u64,
                ts_ms,
                kind,
                schema_version: ver as u32,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(out)
    }

    /// Session ids with their event counts, most recently started first.
    pub fn sessions(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT session, COUNT(*), MIN(ts_ms) AS t0 FROM event GROUP BY session ORDER BY t0 DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Store bytes in the blob directory and return their SHA-256 (hex).
    /// Storing the same bytes twice is a no-op.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        let hash = sha256_hex(bytes);
        let path = self.blob_path(&hash)?;
        if !path.exists() {
            std::fs::create_dir_all(path.parent().expect("blob path has a parent"))?;
            let tmp = path.with_extension(format!("tmp{}", std::process::id()));
            std::fs::write(&tmp, bytes)?;
            if let Err(e) = std::fs::rename(&tmp, &path) {
                let _ = std::fs::remove_file(&tmp);
                if !path.exists() {
                    return Err(e.into());
                }
            }
        }
        Ok(hash)
    }

    /// Read a blob back and check that it still hashes to its name.
    pub fn get_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let bytes = std::fs::read(self.blob_path(hash)?)?;
        if sha256_hex(&bytes) != hash {
            return Err(Error::BadHash(format!("blob {hash} does not match its content")));
        }
        Ok(bytes)
    }

    fn blob_path(&self, hash: &str) -> Result<PathBuf> {
        let ok = hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !ok {
            return Err(Error::BadHash(hash.to_string()));
        }
        Ok(self.blob_dir.join(&hash[..2]).join(hash))
    }
}
