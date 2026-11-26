//! Typed synthetic Zotero SQLite fixture writer for cross-crate integration tests (F13b).
//!
//! Creates fresh synthetic `zotero.sqlite` and `better-bibtex.sqlite` files from typed
//! [`ZoteroItemRow`] values (never raw SQL strings) and refuses to overwrite an existing
//! database file. Keeps `rusqlite` strictly confined to `crates/ledger`.

use crate::zotero_reader::ZoteroItemRow;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::BTreeMap;
use std::path::Path;

/// Holds an active `BEGIN EXCLUSIVE` lock on a test SQLite file until dropped.
pub struct ZoteroExclusiveLock {
    conn: Connection,
}

impl ZoteroExclusiveLock {
    /// Acquire an exclusive SQLite write lock on `db_path` (for locked-database fallback tests).
    pub fn acquire(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open lock conn: {e}"))?;
        conn.execute_batch("BEGIN EXCLUSIVE;")
            .map_err(|e| format!("begin exclusive: {e}"))?;
        Ok(Self { conn })
    }
}

impl Drop for ZoteroExclusiveLock {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("ROLLBACK;");
    }
}

/// Write a fresh synthetic `zotero.sqlite` (and optional `better-bibtex.sqlite`) inside `dir`.
///
/// Refuses to overwrite an existing `zotero.sqlite` to guarantee no real database is touched.
pub fn write_synthetic_zotero_dir(
    dir: &Path,
    items: &[ZoteroItemRow],
    deleted_item_ids: &[i64],
) -> Result<(), String> {
    let db_path = dir.join("zotero.sqlite");
    if db_path.exists() {
        return Err(format!(
            "refusing to overwrite existing database `{}`",
            db_path.display()
        ));
    }
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE itemTypes(itemTypeID INT PRIMARY KEY, typeName TEXT);
         CREATE TABLE items(itemID INT PRIMARY KEY, itemTypeID INT, key TEXT, dateAdded TEXT);
         CREATE TABLE deletedItems(itemID INT PRIMARY KEY);
         CREATE TABLE fields(fieldID INT PRIMARY KEY, fieldName TEXT);
         CREATE TABLE itemDataValues(valueID INT PRIMARY KEY, value TEXT);
         CREATE TABLE itemData(itemID INT, fieldID INT, valueID INT);
         CREATE TABLE creatorTypes(creatorTypeID INT PRIMARY KEY, creatorType TEXT);
         CREATE TABLE creators(creatorID INT PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
         CREATE TABLE itemCreators(itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT);
         CREATE TABLE tags(tagID INT PRIMARY KEY, name TEXT);
         CREATE TABLE itemTags(itemID INT, tagID INT);
         CREATE TABLE collections(collectionID INT PRIMARY KEY, collectionName TEXT, parentCollectionID INT);
         CREATE TABLE collectionItems(collectionID INT, itemID INT);
         CREATE TABLE itemAttachments(itemID INT PRIMARY KEY, parentItemID INT, contentType TEXT, path TEXT);",
    )
    .map_err(|e| e.to_string())?;

    let mut type_ids: BTreeMap<String, i64> = BTreeMap::new();
    let mut field_ids: BTreeMap<String, i64> = BTreeMap::new();
    let mut creator_type_ids: BTreeMap<String, i64> = BTreeMap::new();
    let mut tag_ids: BTreeMap<String, i64> = BTreeMap::new();
    let mut col_ids: BTreeMap<String, i64> = BTreeMap::new();
    let mut next_val_id = 1i64;
    let mut next_creator_id = 1i64;

    let ensure_type = |conn: &Connection, map: &mut BTreeMap<String, i64>, name: &str| -> Result<i64, String> {
        if let Some(&id) = map.get(name) {
            return Ok(id);
        }
        let id = (map.len() as i64) + 1;
        conn.execute(
            "INSERT INTO itemTypes(itemTypeID, typeName) VALUES(?1, ?2)",
            params![id, name],
        )
        .map_err(|e| e.to_string())?;
        map.insert(name.to_string(), id);
        Ok(id)
    };

    let att_type_id = ensure_type(&conn, &mut type_ids, "attachment")?;
    let mut bbt_entries: Vec<(i64, String, String)> = Vec::new();

    for item in items {
        let tid = ensure_type(&conn, &mut type_ids, &item.item_type)?;
        conn.execute(
            "INSERT INTO items(itemID, itemTypeID, key, dateAdded) VALUES(?1, ?2, ?3, ?4)",
            params![item.item_id, tid, item.zotero_key, item.date_added],
        )
        .map_err(|e| e.to_string())?;

        for (fname, fval) in &item.fields {
            let fid = if let Some(&id) = field_ids.get(fname) {
                id
            } else {
                let id = (field_ids.len() as i64) + 1;
                conn.execute(
                    "INSERT INTO fields(fieldID, fieldName) VALUES(?1, ?2)",
                    params![id, fname],
                )
                .map_err(|e| e.to_string())?;
                field_ids.insert(fname.clone(), id);
                id
            };
            let vid = next_val_id;
            next_val_id += 1;
            conn.execute(
                "INSERT INTO itemDataValues(valueID, value) VALUES(?1, ?2)",
                params![vid, fval],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO itemData(itemID, fieldID, valueID) VALUES(?1, ?2, ?3)",
                params![item.item_id, fid, vid],
            )
            .map_err(|e| e.to_string())?;
        }

        for (role, names) in &item.creators {
            let ctid = if let Some(&id) = creator_type_ids.get(role) {
                id
            } else {
                let id = (creator_type_ids.len() as i64) + 1;
                conn.execute(
                    "INSERT INTO creatorTypes(creatorTypeID, creatorType) VALUES(?1, ?2)",
                    params![id, role],
                )
                .map_err(|e| e.to_string())?;
                creator_type_ids.insert(role.clone(), id);
                id
            };
            for (idx, full) in names.iter().enumerate() {
                let cid = next_creator_id;
                next_creator_id += 1;
                let (first, last, mode) = if let Some((l, f)) = full.split_once(',') {
                    (f.trim().to_string(), l.trim().to_string(), 0i64)
                } else {
                    (String::new(), full.trim().to_string(), 1i64)
                };
                conn.execute(
                    "INSERT INTO creators(creatorID, firstName, lastName, fieldMode) VALUES(?1, ?2, ?3, ?4)",
                    params![cid, first, last, mode],
                )
                .map_err(|e| e.to_string())?;
                conn.execute(
                    "INSERT INTO itemCreators(itemID, creatorID, creatorTypeID, orderIndex) VALUES(?1, ?2, ?3, ?4)",
                    params![item.item_id, cid, ctid, idx as i64],
                )
                .map_err(|e| e.to_string())?;
            }
        }

        for tag in &item.tags {
            let tag_id = if let Some(&id) = tag_ids.get(tag) {
                id
            } else {
                let id = (tag_ids.len() as i64) + 1;
                conn.execute("INSERT INTO tags(tagID, name) VALUES(?1, ?2)", params![id, tag])
                    .map_err(|e| e.to_string())?;
                tag_ids.insert(tag.clone(), id);
                id
            };
            conn.execute(
                "INSERT INTO itemTags(itemID, tagID) VALUES(?1, ?2)",
                params![item.item_id, tag_id],
            )
            .map_err(|e| e.to_string())?;
        }

        for col_path in &item.collections {
            let mut parent: Option<i64> = None;
            let mut prefix = String::new();
            for seg in col_path.split('/').filter(|s| !s.is_empty()) {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(seg);
                let cid = if let Some(&id) = col_ids.get(&prefix) {
                    id
                } else {
                    let id = (col_ids.len() as i64) + 1;
                    conn.execute(
                        "INSERT INTO collections(collectionID, collectionName, parentCollectionID) VALUES(?1, ?2, ?3)",
                        params![id, seg, parent],
                    )
                    .map_err(|e| e.to_string())?;
                    col_ids.insert(prefix.clone(), id);
                    id
                };
                parent = Some(cid);
            }
            if let Some(leaf_id) = parent {
                conn.execute(
                    "INSERT INTO collectionItems(collectionID, itemID) VALUES(?1, ?2)",
                    params![leaf_id, item.item_id],
                )
                .map_err(|e| e.to_string())?;
            }
        }

        for att in &item.attachments {
            conn.execute(
                "INSERT INTO items(itemID, itemTypeID, key, dateAdded) VALUES(?1, ?2, ?3, NULL)",
                params![att.attachment_item_id, att_type_id, att.attachment_key],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO itemAttachments(itemID, parentItemID, contentType, path) VALUES(?1, ?2, ?3, ?4)",
                params![att.attachment_item_id, item.item_id, att.content_type, att.raw_path],
            )
            .map_err(|e| e.to_string())?;
        }

        if let Some(ck) = &item.bbt_citekey {
            bbt_entries.push((item.item_id, item.zotero_key.clone(), ck.clone()));
        }
    }

    for &del_id in deleted_item_ids {
        conn.execute(
            "INSERT INTO deletedItems(itemID) VALUES(?1)",
            params![del_id],
        )
        .map_err(|e| e.to_string())?;
    }
    drop(conn);

    if !bbt_entries.is_empty() {
        let bbt = Connection::open(dir.join("better-bibtex.sqlite")).map_err(|e| e.to_string())?;
        bbt.execute_batch("CREATE TABLE citationkey(itemID INT, itemKey TEXT, citationKey TEXT);")
            .map_err(|e| e.to_string())?;
        for (iid, zkey, ck) in bbt_entries {
            bbt.execute(
                "INSERT INTO citationkey(itemID, itemKey, citationKey) VALUES(?1, ?2, ?3)",
                params![iid, zkey, ck],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}
