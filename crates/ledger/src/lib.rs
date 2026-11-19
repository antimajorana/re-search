//! Append-only, hash-chained event ledger on SQLite, plus a content-addressed
//! blob directory.
//!
//! The crate exposes typed append, read, and offline verification methods
//! only; there is no raw SQL access. The schema itself rejects UPDATE, DELETE,
//! INSERT OR REPLACE, out-of-order sequence numbers, and `prev_hash`
//! mismatches on `INSERT` (while offline `verify*` checks full `event_hash`
//! preimages), so an unprivileged second connection is refused as well (see
//! `tests/schema_rejects.rs`).
//!
//! Stage 3 adds per-session SHA-256 hash chaining (`prev_hash` + `event_hash`)
//! over canonical JSON payloads and offline integrity checks (`verify_session`,
//! `verify_all`). Legacy v0 databases without hash columns remain readable for
//! `read_session` / `sessions` and fail closed with
//! `Error::LegacyUnchainedSchema` on `append*` / `verify*`.

pub mod verify;

pub use verify::{SessionVerifyReport, VerifyAllReport};

use rusqlite::{config::DbConfig, params, Connection, OpenFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Previous-hash anchor for `seq == 1` of every session.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS event (
    session        TEXT    NOT NULL,
    seq            INTEGER NOT NULL CHECK (seq >= 1),
    ts_ms          INTEGER NOT NULL,
    kind           TEXT    NOT NULL,
    schema_version INTEGER NOT NULL,
    payload        TEXT    NOT NULL CHECK (json_valid(payload)),
    prev_hash      TEXT    NOT NULL CHECK (length(prev_hash) = 64 AND prev_hash NOT GLOB '*[^0-9a-f]*'),
    event_hash     TEXT    NOT NULL CHECK (length(event_hash) = 64 AND event_hash NOT GLOB '*[^0-9a-f]*'),
    PRIMARY KEY (session, seq)
) STRICT, WITHOUT ROWID;

CREATE TRIGGER IF NOT EXISTS event_no_update BEFORE UPDATE ON event
BEGIN SELECT RAISE(ABORT, 'ledger is append-only: UPDATE rejected'); END;

CREATE TRIGGER IF NOT EXISTS event_no_delete BEFORE DELETE ON event
BEGIN SELECT RAISE(ABORT, 'ledger is append-only: DELETE rejected'); END;

CREATE TRIGGER IF NOT EXISTS event_insert_guard BEFORE INSERT ON event
BEGIN
    SELECT CASE
        WHEN NEW.seq <> (SELECT COALESCE(MAX(seq), 0) + 1 FROM event WHERE session = NEW.session)
            THEN RAISE(ABORT, 'ledger seq must be max(seq)+1 for the session')
        WHEN NEW.prev_hash IS NOT (
            CASE WHEN NEW.seq = 1
                THEN '0000000000000000000000000000000000000000000000000000000000000000'
                ELSE (SELECT event_hash FROM event WHERE session = NEW.session AND seq = NEW.seq - 1)
            END
        )
            THEN RAISE(ABORT, 'ledger prev_hash mismatch')
    END;
END;
"#;

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    BadHash(String),
    LegacyUnchainedSchema,
    UnknownSession(String),
    Tamper(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "ledger sqlite error: {e}"),
            Error::Io(e) => write!(f, "ledger io error: {e}"),
            Error::Json(e) => write!(f, "ledger json error: {e}"),
            Error::BadHash(h) => write!(f, "not a sha-256 hex digest: {h}"),
            Error::LegacyUnchainedSchema => {
                write!(f, "ledger uses legacy unchained v0 schema without hash chain columns")
            }
            Error::UnknownSession(s) => write!(f, "unknown ledger session: {s}"),
            Error::Tamper(msg) => write!(f, "ledger tamper detected: {msg}"),
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
    pub prev_hash: String,
    pub event_hash: String,
}

/// Receipt returned by [`Ledger::append_with_hash`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendedEvent {
    pub seq: u64,
    pub ts_ms: i64,
    pub prev_hash: String,
    pub event_hash: String,
}

pub struct Ledger {
    conn: Connection,
    blob_dir: PathBuf,
    legacy_unchained: bool,
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub(crate) fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn is_sha256_hex(s: &str) -> bool {
    is_lower_hex(s, 64)
}

/// SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

/// Compact JSON with object keys sorted by UTF-8 bytes, matching
/// `tools::canon::canonical_json`.
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(*k).expect("string serialization cannot fail"));
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// SHA-256 lowercase hex over 8-byte little-endian length-prefixed UTF-8 slices
/// identifying one event and its predecessor link.
pub fn compute_event_hash(
    session: &str,
    seq: u64,
    ts_ms: i64,
    kind: &str,
    schema_version: u32,
    payload_text: &str,
    prev_hash: &str,
) -> String {
    let seq_s = seq.to_string();
    let ts_s = ts_ms.to_string();
    let ver_s = schema_version.to_string();
    let fields = [
        "re-search.event.v1",
        session,
        seq_s.as_str(),
        ts_s.as_str(),
        kind,
        ver_s.as_str(),
        payload_text,
        prev_hash,
    ];
    let mut h = Sha256::new();
    for f in fields {
        h.update((f.len() as u64).to_le_bytes());
        h.update(f.as_bytes());
    }
    hex_lower(&h.finalize())
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
        Self::init_conn(conn, blob_dir)
    }

    /// Open an existing ledger in `dir` for inspection/verification without
    /// creating `dir`, `blobs/`, or `ledger.db` and without installing or
    /// modifying tables, triggers, or `PRAGMA user_version`.
    pub fn open_existing(dir: &Path) -> Result<Ledger> {
        let db_path = dir.join("ledger.db");
        if !db_path.is_file() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("ledger database not found: {}", db_path.display()),
            )));
        }
        let rw_flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let ro_flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&db_path, rw_flags)
            .or_else(|_| Connection::open_with_flags(&db_path, ro_flags))?;
        Self::init_existing_conn(conn, dir.join("blobs"))
    }

    fn configure_conn(conn: &Connection) -> Result<Vec<String>> {
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        let _ = conn.pragma_update(None, "recursive_triggers", "ON");
        let _ = conn.pragma_update(None, "trusted_schema", "OFF");
        let mut stmt = conn.prepare("PRAGMA table_info(event)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    fn init_conn(conn: Connection, blob_dir: PathBuf) -> Result<Ledger> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let cols = Self::configure_conn(&conn)?;
        let legacy_unchained = !cols.is_empty()
            && (!cols.iter().any(|c| c == "prev_hash") || !cols.iter().any(|c| c == "event_hash"));

        if !legacy_unchained {
            conn.execute_batch(SCHEMA)?;
            let user_ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if user_ver != 3 {
                conn.pragma_update(None, "user_version", 3)?;
            }
        }
        Ok(Ledger { conn, blob_dir, legacy_unchained })
    }

    fn init_existing_conn(conn: Connection, blob_dir: PathBuf) -> Result<Ledger> {
        let cols = Self::configure_conn(&conn)?;
        if cols.is_empty() {
            return Err(Error::Tamper("ledger database has no `event` table".into()));
        }
        let legacy_unchained =
            !cols.iter().any(|c| c == "prev_hash") || !cols.iter().any(|c| c == "event_hash");
        Ok(Ledger { conn, blob_dir, legacy_unchained })
    }

    /// Append one event inside an immediate write transaction, linking it into
    /// the session's SHA-256 hash chain and returning its sequence, timestamp,
    /// `prev_hash`, and `event_hash`.
    pub fn append_with_hash(
        &self,
        session: &str,
        kind: &str,
        schema_version: u32,
        payload: &Value,
    ) -> Result<AppendedEvent> {
        if self.legacy_unchained {
            return Err(Error::LegacyUnchainedSchema);
        }
        let payload_text = canonical_json(payload);
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let run = || -> Result<AppendedEvent> {
            let max_seq: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM event WHERE session = ?1",
                params![session],
                |r| r.get(0),
            )?;
            let seq = max_seq + 1;
            let seq_u64 = u64::try_from(seq)
                .map_err(|_| Error::Tamper(format!("invalid next sequence number {seq}")))?;
            let prev_hash = if seq == 1 {
                GENESIS_HASH.to_string()
            } else {
                self.conn.query_row(
                    "SELECT event_hash FROM event WHERE session = ?1 AND seq = ?2",
                    params![session, max_seq],
                    |r| r.get::<_, String>(0),
                )?
            };
            let ts_ms = now_ms();
            let event_hash = compute_event_hash(
                session,
                seq_u64,
                ts_ms,
                kind,
                schema_version,
                &payload_text,
                &prev_hash,
            );
            self.conn.execute(
                "INSERT INTO event (session, seq, ts_ms, kind, schema_version, payload, prev_hash, event_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![session, seq, ts_ms, kind, schema_version, payload_text, prev_hash, event_hash],
            )?;
            Ok(AppendedEvent { seq: seq_u64, ts_ms, prev_hash, event_hash })
        };
        match run() {
            Ok(ev) => {
                if let Err(e) = self.conn.execute_batch("COMMIT") {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(e.into());
                }
                Ok(ev)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Append one event and return its sequence number (1, 2, 3, ... per
    /// session). The number and hash chain link are chosen inside a write
    /// transaction, so two processes appending to one session do not collide.
    pub fn append(&self, session: &str, kind: &str, schema_version: u32, payload: &Value) -> Result<u64> {
        Ok(self.append_with_hash(session, kind, schema_version, payload)?.seq)
    }

    /// All events of one session, in order.
    pub fn read_session(&self, session: &str) -> Result<Vec<Event>> {
        let sql = if self.legacy_unchained {
            "SELECT session, seq, ts_ms, kind, schema_version, payload, '', ''
             FROM event WHERE session = ?1 ORDER BY seq"
        } else {
            "SELECT session, seq, ts_ms, kind, schema_version, payload, prev_hash, event_hash
             FROM event WHERE session = ?1 ORDER BY seq"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![session], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (session, seq, ts_ms, kind, ver, payload, prev_hash, event_hash) = row?;
            out.push(Event {
                session,
                seq: seq as u64,
                ts_ms,
                kind,
                schema_version: ver as u32,
                payload: serde_json::from_str(&payload)?,
                prev_hash,
                event_hash,
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
        if !is_sha256_hex(hash) {
            return Err(Error::BadHash(hash.to_string()));
        }
        Ok(self.blob_dir.join(&hash[..2]).join(hash))
    }
}
