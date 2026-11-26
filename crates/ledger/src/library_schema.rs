//! SQLite schema and forward-only migrations for `<data_dir>/library/library.db`
//! (shared package F10, extending F3's `fulltext` table).
//!
//! All names that rest on `00-overview.md` section 8.2, decision 5 (one global, rebuildable
//! library for all modes; user marks backed up) live in named constants here so any future
//! rename touches one place.

use crate::{now_ms, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use std::io;

/// Current SQLite `PRAGMA user_version` for `<data_dir>/library/library.db`.
pub const LIBRARY_SCHEMA_VERSION: u32 = 1;

/// Durable sidecar filename under `<data_dir>/library/` holding non-rebuildable user marks
/// (`00-overview.md` section 8.2, decision 5).
pub const USER_MARKS_FILE_NAME: &str = "user_marks.json";

/// Rolling previous-generation backup filename beside [`USER_MARKS_FILE_NAME`]
/// (`00-overview.md` section 8.2, decision 5).
pub const USER_MARKS_BACKUP_NAME: &str = "user_marks.json.bak";

/// Table names in `library.db` (`00-overview.md` sections 2.7, 2.11, 2.12; `explore.md` 4.3).
pub const PAPER_TABLE: &str = "paper";
pub const ALIAS_TABLE: &str = "alias";
pub const SOURCE_TABLE: &str = "source";
pub const SOURCE_ITEM_TABLE: &str = "source_item";
pub const PAPER_TEXT_TABLE: &str = "paper_text";
pub const SEEN_TABLE: &str = "seen";
pub const SKELETON_TABLE: &str = "skeleton";
pub const FINDING_TABLE: &str = "finding";
pub const SEARCH_CACHE_TABLE: &str = "search_cache";
pub const USER_MARK_TABLE: &str = "user_mark";
pub const PAPER_FTS_TABLE: &str = "paper_fts";
pub const SCHEMA_MIGRATIONS_TABLE: &str = "schema_migrations";

/// Value of `fulltext.origin` for files indexed in place from a user library (F13).
pub const ORIGIN_USER_LIBRARY: &str = "user_library";

/// Allowed `seen.role` values (`explore.md` 4.3 + `00-overview.md` 2.7).
pub const SEEN_ROLES: &[&str] = &[
    "candidate",
    "triaged",
    "read",
    "investigated",
    "cited_lookup",
    "solve_source",
];

/// Allowed `user_mark.status` values (`explore.md` 4.3).
pub const USER_MARK_STATUSES: &[&str] = &["unread", "skimmed", "key", "irrelevant"];

/// Allowed `source.kind` values (`00-overview.md` 2.11, F13).
pub const SOURCE_KINDS: &[&str] = &["kandinsky_vault", "folder", "bibtex", "zotero"];

/// Allowed `source_item.state` values (`00-overview.md` 2.11, F13).
pub const SOURCE_ITEM_STATES: &[&str] = &["present", "changed", "missing", "possible_duplicate"];

const CREATE_FULLTEXT_V1: &str = r#"
CREATE TABLE IF NOT EXISTS fulltext (
    paper_key     TEXT    NOT NULL,
    version       TEXT    NOT NULL,
    raw_blob      TEXT    NOT NULL CHECK (
        (origin = 'user_library' AND (raw_blob = '' OR (length(raw_blob) = 64 AND raw_blob NOT GLOB '*[^0-9a-f]*')))
        OR (origin <> 'user_library' AND length(raw_blob) = 64 AND raw_blob NOT GLOB '*[^0-9a-f]*')
    ),
    format        TEXT    NOT NULL,
    bytes         INTEGER NOT NULL CHECK (bytes >= 0),
    origin        TEXT    NOT NULL,
    fetched_ms    INTEGER NOT NULL,
    file_path     TEXT,
    file_size     INTEGER CHECK (file_size IS NULL OR file_size >= 0),
    file_mtime_ms INTEGER,
    file_sha256   TEXT CHECK (
        file_sha256 IS NULL
        OR (length(file_sha256) = 64 AND file_sha256 NOT GLOB '*[^0-9a-f]*')
    ),
    PRIMARY KEY (paper_key, version)
) STRICT, WITHOUT ROWID;
"#;

const UPGRADE_FULLTEXT_F3_TO_V1: &str = r#"
CREATE TABLE fulltext_v1_upgrade (
    paper_key     TEXT    NOT NULL,
    version       TEXT    NOT NULL,
    raw_blob      TEXT    NOT NULL CHECK (
        (origin = 'user_library' AND (raw_blob = '' OR (length(raw_blob) = 64 AND raw_blob NOT GLOB '*[^0-9a-f]*')))
        OR (origin <> 'user_library' AND length(raw_blob) = 64 AND raw_blob NOT GLOB '*[^0-9a-f]*')
    ),
    format        TEXT    NOT NULL,
    bytes         INTEGER NOT NULL CHECK (bytes >= 0),
    origin        TEXT    NOT NULL,
    fetched_ms    INTEGER NOT NULL,
    file_path     TEXT,
    file_size     INTEGER CHECK (file_size IS NULL OR file_size >= 0),
    file_mtime_ms INTEGER,
    file_sha256   TEXT CHECK (
        file_sha256 IS NULL
        OR (length(file_sha256) = 64 AND file_sha256 NOT GLOB '*[^0-9a-f]*')
    ),
    PRIMARY KEY (paper_key, version)
) STRICT, WITHOUT ROWID;

INSERT INTO fulltext_v1_upgrade (paper_key, version, raw_blob, format, bytes, origin, fetched_ms)
SELECT paper_key, version, raw_blob, format, bytes, origin, fetched_ms FROM fulltext;

DROP TABLE fulltext;
ALTER TABLE fulltext_v1_upgrade RENAME TO fulltext;
"#;

const SCHEMA_V1_REST: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER NOT NULL PRIMARY KEY CHECK (version >= 1),
    applied_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS paper (
    paper_key     TEXT    NOT NULL PRIMARY KEY,
    ids_json      TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(ids_json)),
    title         TEXT    NOT NULL DEFAULT '',
    authors_json  TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(authors_json)),
    year          INTEGER,
    abstract      TEXT    NOT NULL DEFAULT '',
    bibtex        TEXT    NOT NULL DEFAULT '',
    flags_json    TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(flags_json)),
    first_seen_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS alias (
    identifier TEXT NOT NULL PRIMARY KEY,
    paper_key  TEXT NOT NULL REFERENCES paper(paper_key) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS source (
    id            TEXT    NOT NULL PRIMARY KEY,
    kind          TEXT    NOT NULL CHECK (kind IN ('kandinsky_vault', 'folder', 'bibtex', 'zotero')),
    root          TEXT    NOT NULL,
    settings_json TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(settings_json)),
    added_ms      INTEGER NOT NULL,
    last_sync_ms  INTEGER,
    state         TEXT    NOT NULL DEFAULT 'idle'
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS source_item (
    source_id        TEXT    NOT NULL REFERENCES source(id) ON DELETE CASCADE,
    external_id      TEXT    NOT NULL,
    paper_key        TEXT,
    citekey          TEXT,
    ids_json         TEXT    NOT NULL DEFAULT '{}' CHECK (json_valid(ids_json)),
    version          TEXT,
    title            TEXT    NOT NULL DEFAULT '',
    authors_json     TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(authors_json)),
    year             INTEGER,
    item_kind        TEXT    NOT NULL DEFAULT '',
    collections_json TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(collections_json)),
    tags_json        TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(tags_json)),
    files_json       TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(files_json)),
    state            TEXT    NOT NULL CHECK (state IN ('present', 'changed', 'missing', 'possible_duplicate')),
    updated_ms       INTEGER NOT NULL,
    PRIMARY KEY (source_id, external_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS paper_text (
    paper_key       TEXT    NOT NULL,
    version         TEXT    NOT NULL DEFAULT '',
    file_sha256     TEXT    NOT NULL CHECK (length(file_sha256) = 64 AND file_sha256 NOT GLOB '*[^0-9a-f]*'),
    extractor       TEXT    NOT NULL,
    text_blob       TEXT    NOT NULL CHECK (length(text_blob) = 64 AND text_blob NOT GLOB '*[^0-9a-f]*'),
    page_basis_json TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(page_basis_json)),
    extracted_ms    INTEGER NOT NULL,
    PRIMARY KEY (paper_key, version, file_sha256, extractor)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS seen (
    paper_key  TEXT    NOT NULL,
    session_id TEXT    NOT NULL,
    role       TEXT    NOT NULL CHECK (role IN (
        'candidate', 'triaged', 'read',
        'investigated', 'cited_lookup', 'solve_source'
    )),
    seen_ms    INTEGER NOT NULL,
    PRIMARY KEY (paper_key, session_id, role)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS skeleton (
    paper_key   TEXT    NOT NULL,
    version     TEXT    NOT NULL DEFAULT '',
    builder     TEXT    NOT NULL,
    blob_sha256 TEXT    NOT NULL CHECK (length(blob_sha256) = 64 AND blob_sha256 NOT GLOB '*[^0-9a-f]*'),
    built_ms    INTEGER NOT NULL,
    PRIMARY KEY (paper_key, version, builder)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS finding (
    paper_key    TEXT    NOT NULL,
    session_id   TEXT    NOT NULL,
    evidence_seq INTEGER NOT NULL CHECK (evidence_seq >= 1),
    role         TEXT    NOT NULL DEFAULT '',
    gist         TEXT    NOT NULL DEFAULT '',
    quote        TEXT    NOT NULL DEFAULT '',
    cited_start  INTEGER,
    cited_end    INTEGER,
    recorded_ms  INTEGER NOT NULL,
    PRIMARY KEY (paper_key, session_id, evidence_seq)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS search_cache (
    service    TEXT    NOT NULL,
    url_sha256 TEXT    NOT NULL CHECK (length(url_sha256) = 64 AND url_sha256 NOT GLOB '*[^0-9a-f]*'),
    raw_blob   TEXT    NOT NULL CHECK (length(raw_blob) = 64 AND raw_blob NOT GLOB '*[^0-9a-f]*'),
    fetched_ms INTEGER NOT NULL,
    PRIMARY KEY (service, url_sha256)
) STRICT, WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS user_mark (
    paper_key  TEXT    NOT NULL PRIMARY KEY,
    status     TEXT    NOT NULL CHECK (status IN ('unread', 'skimmed', 'key', 'irrelevant')),
    tags_json  TEXT    NOT NULL DEFAULT '[]' CHECK (json_valid(tags_json)),
    note       TEXT    NOT NULL DEFAULT '',
    updated_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE VIRTUAL TABLE IF NOT EXISTS paper_fts USING fts5(
    paper_key UNINDEXED,
    title,
    authors,
    abstract,
    gists,
    full_text,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE INDEX IF NOT EXISTS idx_paper_year ON paper(year);
CREATE INDEX IF NOT EXISTS idx_paper_last_seen ON paper(last_seen_ms DESC);
CREATE INDEX IF NOT EXISTS idx_alias_paper_key ON alias(paper_key);
CREATE INDEX IF NOT EXISTS idx_fulltext_blob ON fulltext(raw_blob);
CREATE INDEX IF NOT EXISTS idx_fulltext_origin ON fulltext(origin);
CREATE INDEX IF NOT EXISTS idx_source_item_key ON source_item(paper_key);
CREATE INDEX IF NOT EXISTS idx_source_item_state ON source_item(state);
CREATE INDEX IF NOT EXISTS idx_paper_text_sha ON paper_text(file_sha256);
CREATE INDEX IF NOT EXISTS idx_seen_session ON seen(session_id);
CREATE INDEX IF NOT EXISTS idx_seen_role_recent ON seen(role, seen_ms DESC);
CREATE INDEX IF NOT EXISTS idx_finding_session ON finding(session_id);
"#;

type MigrationStep = (u32, fn(&Connection) -> Result<()>);

/// Ordered forward-only migration steps. Future schema versions append new entries here.
const MIGRATIONS: &[MigrationStep] = &[(1, migrate_0_to_1)];

/// Read `PRAGMA user_version` from `conn`.
pub fn current_schema_version(conn: &Connection) -> Result<u32> {
    let raw: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    u32::try_from(raw).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("library database has invalid user_version `{raw}`"),
        )
        .into()
    })
}

/// Apply all pending forward-only migrations up to [`LIBRARY_SCHEMA_VERSION`].
///
/// Fails closed if the database reports a schema version newer than this build supports.
pub fn migrate_library_db(conn: &mut Connection) -> Result<u32> {
    let mut version = current_schema_version(conn)?;
    if version > LIBRARY_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "library database schema version {version} is newer than supported version {LIBRARY_SCHEMA_VERSION}; refusing downgrade"
            ),
        )
        .into());
    }
    if version == LIBRARY_SCHEMA_VERSION {
        return Ok(version);
    }
    for &(target, step) in MIGRATIONS {
        if version < target {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            step(&tx)?;
            tx.execute(
                "INSERT OR REPLACE INTO schema_migrations (version, applied_ms) VALUES (?1, ?2)",
                params![i64::from(target), now_ms()],
            )?;
            tx.pragma_update(None, "user_version", i64::from(target))?;
            tx.commit()?;
            version = target;
        }
    }
    Ok(version)
}

fn migrate_0_to_1(conn: &Connection) -> Result<()> {
    let cols = table_columns(conn, "fulltext")?;
    if cols.is_empty() {
        conn.execute_batch(CREATE_FULLTEXT_V1)?;
    } else if !cols.iter().any(|c| c == "file_path") {
        // Database was created by F3 (`FulltextCache::open`) with the 7-column `fulltext` table.
        // Upgrade in place while preserving every cached row and keeping F3 compatibility.
        conn.execute_batch(UPGRADE_FULLTEXT_F3_TO_V1)?;
    }
    conn.execute_batch(SCHEMA_V1_REST)?;
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
