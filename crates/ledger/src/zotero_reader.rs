// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Strictly read-only SQLite reader for Zotero 6/7/8 (`zotero.sqlite`) and optional Better BibTeX
//! databases (`better-bibtex.sqlite`, `better-bibtex-search.sqlite`) for package F13b
//! (`00-overview.md` 2.11).
//!
//! Confines all `rusqlite` usage to `crates/ledger` (no `tokio`, `rmcp`, or `windows-sys`).
//! Never opens a user's live `zotero.sqlite` read-write: opens with `SQLITE_OPEN_READ_ONLY |
//! SQLITE_OPEN_NO_MUTEX` + `PRAGMA query_only = ON`, and when Zotero holds a write lock
//! (`SQLITE_BUSY` / `SQLITE_LOCKED`) or an active `-wal` file is present, copies the database
//! into an isolated temporary directory, reads the snapshot read-only, and removes the temporary
//! directory on drop.

use rusqlite::{config::DbConfig, Connection, ErrorCode, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One non-deleted attachment row belonging to a Zotero bibliographic item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoteroAttachmentRow {
    pub attachment_item_id: i64,
    pub attachment_key: String,
    pub content_type: String,
    pub raw_path: String,
}

/// Plain data record for one non-deleted Zotero bibliographic item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoteroItemRow {
    pub item_id: i64,
    pub zotero_key: String,
    pub item_type: String,
    pub date_added: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub creators: BTreeMap<String, Vec<String>>,
    pub collections: Vec<String>,
    pub tags: Vec<String>,
    pub attachments: Vec<ZoteroAttachmentRow>,
    pub bbt_citekey: Option<String>,
}

/// Complete read-only snapshot extracted from a Zotero data directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoteroLibrarySnapshot {
    pub items: Vec<ZoteroItemRow>,
    pub trash_skipped: usize,
    pub collections_found: usize,
    pub used_snapshot: bool,
}

/// Guard deleting the temporary snapshot directory on drop.
struct SnapDir(PathBuf);

impl Drop for SnapDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Opened read-only SQLite connection plus optional temporary snapshot guard.
///
/// Field order is significant: `conn` drops before `_snap` so Windows releases file handles
/// before `SnapDir::drop` deletes the temporary snapshot directory.
struct OpenedDb {
    conn: Connection,
    _snap: Option<SnapDir>,
    used_snapshot: bool,
}

static SNAP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Read a Zotero data directory containing `zotero.sqlite` strictly read-only.
pub fn read_zotero_library(data_dir: &Path) -> Result<ZoteroLibrarySnapshot, String> {
    read_zotero_library_with_temp_root(data_dir, None)
}

/// Read a Zotero data directory strictly read-only, placing any lock-fallback temporary snapshot
/// under `temp_root` (defaults to `std::env::temp_dir()`).
pub fn read_zotero_library_with_temp_root(
    data_dir: &Path,
    temp_root: Option<&Path>,
) -> Result<ZoteroLibrarySnapshot, String> {
    let db_path = data_dir.join("zotero.sqlite");
    ensure_regular_non_symlink(&db_path)?;

    let has_wal = sidecar_has_bytes(&db_path, "-wal");
    if !has_wal {
        if let Ok(direct) = open_direct_readonly(&db_path) {
            match query_zotero_conn(&direct.conn, false) {
                Ok(mut snap) => {
                    attach_bbt_keys(data_dir, temp_root, &mut snap.items);
                    return Ok(snap);
                }
                Err(ReadErr::Locked) => {
                    // Fall through to isolated snapshot copy below.
                }
                Err(ReadErr::Other(msg)) => return Err(msg),
            }
        }
    }

    let opened = open_snapshot_readonly(&db_path, temp_root)?;
    let mut snap = query_zotero_conn(&opened.conn, opened.used_snapshot).map_err(|e| match e {
        ReadErr::Locked => format!("{}: snapshot database unexpectedly locked", db_path.display()),
        ReadErr::Other(msg) => msg,
    })?;
    drop(opened);
    attach_bbt_keys(data_dir, temp_root, &mut snap.items);
    Ok(snap)
}

enum ReadErr {
    Locked,
    Other(String),
}

fn is_lock_error(err: &rusqlite::Error) -> bool {
    matches!(
        err.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn ensure_regular_non_symlink(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("{}: cannot inspect database file: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{}: refused symlink database file", path.display()));
    }
    if !meta.is_file() {
        return Err(format!("{}: database path is not a regular file", path.display()));
    }
    Ok(())
}

fn sidecar_has_bytes(db_path: &Path, suffix: &str) -> bool {
    let side = PathBuf::from(format!("{}{suffix}", db_path.display()));
    fs::symlink_metadata(&side)
        .map(|m| !m.file_type().is_symlink() && m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

fn harden_readonly_conn(conn: &Connection) -> Result<(), ReadErr> {
    let _ = conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true);
    conn.busy_timeout(Duration::from_millis(50))
        .map_err(|e| map_sqlite_err("busy_timeout", e))?;
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF;")
        .map_err(|e| map_sqlite_err("pragma query_only", e))?;
    Ok(())
}

fn map_sqlite_err(ctx: &str, err: rusqlite::Error) -> ReadErr {
    if is_lock_error(&err) {
        ReadErr::Locked
    } else {
        ReadErr::Other(format!("{ctx}: {err}"))
    }
}

fn open_direct_readonly(db_path: &Path) -> Result<OpenedDb, ReadErr> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db_path, flags)
        .map_err(|e| map_sqlite_err("open zotero.sqlite read-only", e))?;
    harden_readonly_conn(&conn)?;
    // Probe lock state before running the full query set.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .map_err(|e| map_sqlite_err("probe zotero.sqlite", e))?;
    Ok(OpenedDb {
        conn,
        _snap: None,
        used_snapshot: false,
    })
}

fn make_snap_dir(temp_root: Option<&Path>) -> Result<SnapDir, String> {
    let base = temp_root
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for _ in 0..32 {
        let n = SNAP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!(
            "re-search-zotero-{}-{stamp}-{n}",
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(SnapDir(dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create zotero snapshot temp dir: {e}")),
        }
    }
    Err("could not allocate unique zotero snapshot temp dir".into())
}

fn sqlite_ro_immutable_uri(path: &Path) -> String {
    let norm = path.to_string_lossy().replace('\\', "/");
    let lead = if norm.starts_with('/') { "" } else { "/" };
    let mut enc = String::with_capacity(norm.len());
    for b in norm.bytes() {
        if matches!(b, b'?' | b'#' | b'%' | b' ') {
            use std::fmt::Write as _;
            let _ = write!(&mut enc, "%{b:02X}");
        } else {
            enc.push(b as char);
        }
    }
    format!("file:{lead}{enc}?mode=ro&immutable=1")
}

fn open_snapshot_readonly(db_path: &Path, temp_root: Option<&Path>) -> Result<OpenedDb, String> {
    let snap = make_snap_dir(temp_root)?;
    let fname = db_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("zotero.sqlite");
    let dst = snap.0.join(fname);
    fs::copy(db_path, &dst).map_err(|e| format!("snapshot {}: {e}", db_path.display()))?;

    let mut copied_wal = false;
    for ext in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{ext}", db_path.display()));
        if let Ok(meta) = fs::symlink_metadata(&side) {
            if !meta.file_type().is_symlink() && meta.is_file() {
                let side_dst = PathBuf::from(format!("{}{ext}", dst.display()));
                let _ = fs::copy(&side, &side_dst);
                if ext == "-wal" && meta.len() > 0 {
                    copied_wal = true;
                }
            }
        }
    }

    let conn = if !copied_wal {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        Connection::open_with_flags(sqlite_ro_immutable_uri(&dst), flags)
    } else {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        Connection::open_with_flags(&dst, flags)
    }
    .map_err(|e| format!("open snapshot {}: {e}", db_path.display()))?;

    harden_readonly_conn(&conn).map_err(|e| match e {
        ReadErr::Locked => format!("{}: snapshot locked", db_path.display()),
        ReadErr::Other(msg) => msg,
    })?;

    let check: String = conn
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(|e| format!("corrupt sqlite snapshot {}: {e}", db_path.display()))?;
    if !check.eq_ignore_ascii_case("ok") {
        return Err(format!("{}: quick_check failed ({check})", db_path.display()));
    }

    Ok(OpenedDb {
        conn,
        _snap: Some(snap),
        used_snapshot: true,
    })
}

fn qvec<T>(
    conn: &Connection,
    sql: &str,
    mut f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, ReadErr> {
    let mut st = conn.prepare(sql).map_err(|e| map_sqlite_err("prepare", e))?;
    let rows = st
        .query_map([], |row| f(row))
        .map_err(|e| map_sqlite_err("query_map", e))?;
    let mut out = Vec::new();
    for item in rows {
        match item {
            Ok(v) => out.push(v),
            Err(e) if is_lock_error(&e) => return Err(ReadErr::Locked),
            Err(_) => {}
        }
    }
    Ok(out)
}

fn table_cols(conn: &Connection, table: &str) -> Result<HashSet<String>, ReadErr> {
    let cols = qvec(conn, &format!("PRAGMA table_info({table})"), |r| {
        r.get::<_, String>(1)
    })?;
    Ok(cols.into_iter().collect())
}

fn query_zotero_conn(conn: &Connection, used_snapshot: bool) -> Result<ZoteroLibrarySnapshot, ReadErr> {
    let deleted: HashSet<i64> = if !table_cols(conn, "deletedItems")?.is_empty() {
        qvec(conn, "SELECT itemID FROM deletedItems", |r| r.get::<_, i64>(0))?
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };

    let raw_cols: HashMap<i64, (String, Option<i64>)> =
        if !table_cols(conn, "collections")?.is_empty() {
            qvec(
                conn,
                "SELECT collectionID, collectionName, parentCollectionID FROM collections",
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        (
                            r.get::<_, String>(1)?.trim().to_string(),
                            r.get::<_, Option<i64>>(2)?,
                        ),
                    ))
                },
            )?
            .into_iter()
            .collect()
        } else {
            HashMap::new()
        };

    let mut col_paths: HashMap<i64, String> = HashMap::new();
    for &cid in raw_cols.keys() {
        let (mut segs, mut cur, mut seen) = (Vec::new(), Some(cid), HashSet::new());
        while let Some(id) = cur.filter(|id| seen.insert(*id)) {
            let Some((name, parent)) = raw_cols.get(&id) else {
                break;
            };
            if !name.is_empty() {
                segs.push(name.clone());
            }
            cur = *parent;
        }
        segs.reverse();
        if !segs.is_empty() {
            col_paths.insert(cid, segs.join("/"));
        }
    }

    let mut trash_skipped = 0usize;
    let mut bib_items = Vec::new();
    for (iid, zkey, tname, added) in qvec(
        conn,
        "SELECT i.itemID, i.key, it.typeName, i.dateAdded FROM items i \
         JOIN itemTypes it ON i.itemTypeID = it.itemTypeID ORDER BY i.itemID",
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        },
    )? {
        if matches!(tname.as_str(), "attachment" | "note" | "annotation") {
            continue;
        }
        if deleted.contains(&iid) {
            trash_skipped += 1;
        } else {
            bib_items.push((iid, zkey, tname, added));
        }
    }

    let fields_tbl = if !table_cols(conn, "fieldsCombined")?.is_empty() {
        "fieldsCombined"
    } else {
        "fields"
    };
    let mut item_fields: HashMap<i64, BTreeMap<String, String>> = HashMap::new();
    if !table_cols(conn, "itemData")?.is_empty() {
        let sql = format!(
            "SELECT id.itemID, f.fieldName, idv.value FROM itemData id \
             JOIN {fields_tbl} f ON id.fieldID = f.fieldID \
             JOIN itemDataValues idv ON id.valueID = idv.valueID"
        );
        for (iid, fname, val) in qvec(conn, &sql, |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })? {
            if !val.trim().is_empty() {
                item_fields.entry(iid).or_default().insert(fname, val);
            }
        }
    }

    let mut item_creators: HashMap<i64, BTreeMap<String, Vec<String>>> = HashMap::new();
    if !table_cols(conn, "itemCreators")?.is_empty() {
        let sql = "SELECT ic.itemID, ct.creatorType, c.firstName, c.lastName, c.fieldMode \
                   FROM itemCreators ic \
                   JOIN creators c ON ic.creatorID = c.creatorID \
                   JOIN creatorTypes ct ON ic.creatorTypeID = ct.creatorTypeID \
                   ORDER BY ic.itemID, ic.orderIndex";
        for (iid, role, first, last, mode) in qvec(conn, sql, |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })? {
            let (f, l) = (first.unwrap_or_default(), last.unwrap_or_default());
            let name = if mode == Some(1) || f.trim().is_empty() {
                l.trim().to_string()
            } else {
                format!("{}, {}", l.trim(), f.trim())
            };
            if !name.is_empty() {
                item_creators
                    .entry(iid)
                    .or_default()
                    .entry(role)
                    .or_default()
                    .push(name);
            }
        }
    }

    let mut item_tags: HashMap<i64, Vec<String>> = HashMap::new();
    if !table_cols(conn, "itemTags")?.is_empty() {
        for (iid, tag) in qvec(
            conn,
            "SELECT it.itemID, t.name FROM itemTags it JOIN tags t ON it.tagID = t.tagID",
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )? {
            let t = tag.trim().to_string();
            let list = item_tags.entry(iid).or_default();
            if !t.is_empty() && !list.contains(&t) {
                list.push(t);
            }
        }
    }

    let mut item_cols: HashMap<i64, Vec<String>> = HashMap::new();
    if !table_cols(conn, "collectionItems")?.is_empty() {
        for (cid, iid) in qvec(conn, "SELECT collectionID, itemID FROM collectionItems", |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })? {
            if let Some(p) = col_paths.get(&cid) {
                let list = item_cols.entry(iid).or_default();
                if !list.contains(p) {
                    list.push(p.clone());
                }
            }
        }
    }

    let mut item_atts: HashMap<i64, Vec<ZoteroAttachmentRow>> = HashMap::new();
    if !table_cols(conn, "itemAttachments")?.is_empty() {
        let sql = "SELECT ia.itemID, ia.parentItemID, i.key, COALESCE(ia.contentType, ''), \
                   COALESCE(ia.path, '') \
                   FROM itemAttachments ia JOIN items i ON ia.itemID = i.itemID";
        for (aid, pid, akey, ctype, raw_path) in qvec(conn, sql, |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            if let Some(parent_id) = pid.filter(|_| !deleted.contains(&aid)) {
                item_atts
                    .entry(parent_id)
                    .or_default()
                    .push(ZoteroAttachmentRow {
                        attachment_item_id: aid,
                        attachment_key: akey,
                        content_type: ctype,
                        raw_path,
                    });
            }
        }
    }

    let items = bib_items
        .into_iter()
        .map(|(iid, zkey, tname, date_added)| ZoteroItemRow {
            item_id: iid,
            zotero_key: zkey,
            item_type: tname,
            date_added,
            fields: item_fields.remove(&iid).unwrap_or_default(),
            creators: item_creators.remove(&iid).unwrap_or_default(),
            collections: item_cols.remove(&iid).unwrap_or_default(),
            tags: item_tags.remove(&iid).unwrap_or_default(),
            attachments: item_atts.remove(&iid).unwrap_or_default(),
            bbt_citekey: None,
        })
        .collect();

    Ok(ZoteroLibrarySnapshot {
        items,
        trash_skipped,
        collections_found: col_paths.len(),
        used_snapshot,
    })
}

fn attach_bbt_keys(data_dir: &Path, temp_root: Option<&Path>, items: &mut [ZoteroItemRow]) {
    let (by_id, by_key) = read_bbt_keys(data_dir, temp_root);
    if by_id.is_empty() && by_key.is_empty() {
        return;
    }
    for item in items {
        item.bbt_citekey = by_id
            .get(&item.item_id)
            .cloned()
            .or_else(|| by_key.get(&item.zotero_key).cloned());
    }
}

fn read_bbt_keys(
    data_dir: &Path,
    temp_root: Option<&Path>,
) -> (HashMap<i64, String>, HashMap<String, String>) {
    let (mut by_id, mut by_key) = (HashMap::new(), HashMap::new());
    for fname in ["better-bibtex.sqlite", "better-bibtex-search.sqlite"] {
        let path = data_dir.join(fname);
        if ensure_regular_non_symlink(&path).is_err() {
            continue;
        }
        let opened = if !sidecar_has_bytes(&path, "-wal") {
            match open_direct_readonly(&path) {
                Ok(db) => Ok(db),
                Err(ReadErr::Locked) => open_snapshot_readonly(&path, temp_root),
                Err(ReadErr::Other(_)) => continue,
            }
        } else {
            open_snapshot_readonly(&path, temp_root)
        };
        let Ok(db) = opened else {
            continue;
        };
        let Ok(cols) = table_cols(&db.conn, "citationkey") else {
            continue;
        };
        if cols.contains("itemID") && cols.contains("citationKey") {
            if let Ok(rows) = qvec(&db.conn, "SELECT itemID, citationKey FROM citationkey", |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            }) {
                for (id, ck) in rows {
                    let trimmed = ck.trim().to_string();
                    if !trimmed.is_empty() {
                        by_id.insert(id, trimmed);
                    }
                }
            }
        }
        if cols.contains("itemKey") && cols.contains("citationKey") {
            if let Ok(rows) =
                qvec(&db.conn, "SELECT itemKey, citationKey FROM citationkey", |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
            {
                for (ik, ck) in rows {
                    let trimmed = ck.trim().to_string();
                    if !trimmed.is_empty() {
                        by_key.insert(ik, trimmed);
                    }
                }
            }
        }
    }
    (by_id, by_key)
}

#[cfg(test)]
#[path = "zotero_reader_tests.rs"]
mod tests;
