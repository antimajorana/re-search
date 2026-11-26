// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Unit tests for [`crate::zotero_reader`]: core Zotero schema extraction, Better BibTeX keys,
//! locked-database snapshot fallback, temp cleanup, and byte-identical read-only invariant.

use super::*;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "re-search-zot-test-{}-{stamp}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&p)?;
        Ok(Self(p))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap();
    let digest = Sha256::digest(&bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn create_synthetic_zotero_db(dir: &Path) {
    let conn = Connection::open(dir.join("zotero.sqlite")).unwrap();
    conn.execute_batch(
        "CREATE TABLE itemTypes(itemTypeID INT PRIMARY KEY, typeName TEXT);
         INSERT INTO itemTypes VALUES(1,'journalArticle'),(2,'attachment'),(3,'note'),(4,'annotation');
         CREATE TABLE items(itemID INT PRIMARY KEY, itemTypeID INT, key TEXT, dateAdded TEXT);
         INSERT INTO items VALUES
           (10,1,'ZITEM001','2026-01-01'),
           (11,1,'ZITEM002','2026-01-02'),
           (12,1,'ZTRASH99','2026-01-03'),
           (20,2,'ATTKEY01','2026-01-01'),
           (21,2,'ATTKEY02','2026-01-02'),
           (30,3,'NOTEKY01','2026-01-01');
         CREATE TABLE deletedItems(itemID INT PRIMARY KEY);
         INSERT INTO deletedItems VALUES(12);
         CREATE TABLE fields(fieldID INT PRIMARY KEY, fieldName TEXT);
         INSERT INTO fields VALUES(1,'title'),(2,'date'),(3,'DOI'),(4,'extra'),(5,'publicationTitle');
         CREATE TABLE itemDataValues(valueID INT PRIMARY KEY, value TEXT);
         INSERT INTO itemDataValues VALUES
           (1,'Synthetic Lattice Scaling'),
           (2,'2024-08-04'),
           (3,'10.9999/zot.2024.001'),
           (4,'Citation Key: synth2024pinned'),
           (5,'Journal of Synthetic Physics'),
           (6,'Synthetic Braiding Notes'),
           (7,'2023');
         CREATE TABLE itemData(itemID INT, fieldID INT, valueID INT);
         INSERT INTO itemData VALUES(10,1,1),(10,2,2),(10,3,3),(10,4,4),(10,5,5),(11,1,6),(11,2,7);
         CREATE TABLE creatorTypes(creatorTypeID INT PRIMARY KEY, creatorType TEXT);
         INSERT INTO creatorTypes VALUES(1,'author');
         CREATE TABLE creators(creatorID INT PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
         INSERT INTO creators VALUES(1,'Ada','Synth',0),(2,'','Synthetic Collaboration',1);
         CREATE TABLE itemCreators(itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT);
         INSERT INTO itemCreators VALUES(10,1,1,0),(11,2,1,0);
         CREATE TABLE tags(tagID INT PRIMARY KEY, name TEXT);
         INSERT INTO tags VALUES(1,'lattice'),(2,'scaling');
         CREATE TABLE itemTags(itemID INT, tagID INT);
         INSERT INTO itemTags VALUES(10,1),(10,2);
         CREATE TABLE collections(collectionID INT PRIMARY KEY, collectionName TEXT, parentCollectionID INT);
         INSERT INTO collections VALUES(1,'Physics',NULL),(2,'Condensed Matter',1);
         CREATE TABLE collectionItems(collectionID INT, itemID INT);
         INSERT INTO collectionItems VALUES(2,10);
         CREATE TABLE itemAttachments(itemID INT PRIMARY KEY, parentItemID INT, contentType TEXT, path TEXT);
         INSERT INTO itemAttachments VALUES
           (20,10,'application/pdf','attachments:Quantum/synth1.pdf'),
           (21,11,'application/pdf','storage:stored.pdf');",
    )
    .unwrap();
    drop(conn);

    let bbt = Connection::open(dir.join("better-bibtex.sqlite")).unwrap();
    bbt.execute_batch(
        "CREATE TABLE citationkey(itemID INT, itemKey TEXT, citationKey TEXT);
         INSERT INTO citationkey VALUES(11,'ZITEM002','collab2023bbt');",
    )
    .unwrap();
    drop(bbt);
}

#[test]
fn reads_zotero_items_collections_tags_attachments_and_bbt_readonly() {
    let tmp = TempDir::new().unwrap();
    let zot_dir = tmp.path().join("ZoteroData");
    fs::create_dir_all(&zot_dir).unwrap();
    create_synthetic_zotero_db(&zot_dir);

    let hash_before = sha256_file(&zot_dir.join("zotero.sqlite"));
    let bbt_before = sha256_file(&zot_dir.join("better-bibtex.sqlite"));

    let snap = read_zotero_library(&zot_dir).unwrap();
    assert!(!snap.used_snapshot);
    assert_eq!(snap.trash_skipped, 1);
    assert_eq!(snap.collections_found, 2);
    assert_eq!(snap.items.len(), 2);

    let i0 = &snap.items[0];
    assert_eq!(i0.zotero_key, "ZITEM001");
    assert_eq!(i0.item_type, "journalArticle");
    assert_eq!(i0.fields.get("title").map(String::as_str), Some("Synthetic Lattice Scaling"));
    assert_eq!(i0.creators.get("author"), Some(&vec!["Synth, Ada".to_string()]));
    assert_eq!(i0.collections, vec!["Physics/Condensed Matter".to_string()]);
    assert_eq!(i0.tags, vec!["lattice".to_string(), "scaling".to_string()]);
    assert_eq!(i0.attachments.len(), 1);
    assert_eq!(i0.attachments[0].attachment_key, "ATTKEY01");
    assert_eq!(i0.attachments[0].raw_path, "attachments:Quantum/synth1.pdf");

    let i1 = &snap.items[1];
    assert_eq!(i1.zotero_key, "ZITEM002");
    assert_eq!(
        i1.creators.get("author"),
        Some(&vec!["Synthetic Collaboration".to_string()])
    );
    assert_eq!(i1.bbt_citekey.as_deref(), Some("collab2023bbt"));
    assert_eq!(i1.attachments[0].raw_path, "storage:stored.pdf");

    assert_eq!(sha256_file(&zot_dir.join("zotero.sqlite")), hash_before);
    assert_eq!(sha256_file(&zot_dir.join("better-bibtex.sqlite")), bbt_before);
}

#[test]
fn locked_zotero_database_falls_back_to_snapshot_and_cleans_up_temp_dir() {
    let tmp = TempDir::new().unwrap();
    let zot_dir = tmp.path().join("ZoteroLocked");
    let snap_root = tmp.path().join("snap_scratch");
    fs::create_dir_all(&zot_dir).unwrap();
    fs::create_dir_all(&snap_root).unwrap();
    create_synthetic_zotero_db(&zot_dir);

    let db_path = zot_dir.join("zotero.sqlite");
    let hash_before = sha256_file(&db_path);

    // Hold an exclusive transaction on a second connection to simulate a running Zotero process.
    let locker = Connection::open(&db_path).unwrap();
    locker.execute_batch("BEGIN EXCLUSIVE;").unwrap();

    let snap = read_zotero_library_with_temp_root(&zot_dir, Some(&snap_root)).unwrap();
    assert!(
        snap.used_snapshot,
        "locked database must trigger isolated read-only snapshot fallback"
    );
    assert_eq!(snap.items.len(), 2);
    assert_eq!(snap.items[1].bbt_citekey.as_deref(), Some("collab2023bbt"));

    // Snapshot temp directory must be removed immediately after drop.
    let leftovers: Vec<_> = fs::read_dir(&snap_root).unwrap().flatten().collect();
    assert!(
        leftovers.is_empty(),
        "snapshot temp directory leaked: {leftovers:?}"
    );

    locker.execute_batch("ROLLBACK;").unwrap();
    drop(locker);

    assert_eq!(
        sha256_file(&db_path),
        hash_before,
        "source zotero.sqlite must remain byte-identical"
    );
}

#[cfg(unix)]
#[test]
fn refuses_symlink_zotero_sqlite() {
    let tmp = TempDir::new().unwrap();
    let real_dir = tmp.path().join("real");
    let link_dir = tmp.path().join("linked");
    fs::create_dir_all(&real_dir).unwrap();
    fs::create_dir_all(&link_dir).unwrap();
    create_synthetic_zotero_db(&real_dir);
    std::os::unix::fs::symlink(real_dir.join("zotero.sqlite"), link_dir.join("zotero.sqlite"))
        .unwrap();

    let err = read_zotero_library(&link_dir).unwrap_err();
    assert!(err.contains("symlink"), "unexpected error: {err}");
}

#[test]
fn wal_mode_zotero_database_reads_uncheckpointed_rows_via_snapshot_cleanly() {
    let tmp = TempDir::new().unwrap();
    let zot_dir = tmp.path().join("ZoteroWal");
    let snap_root = tmp.path().join("snap_wal");
    fs::create_dir_all(&zot_dir).unwrap();
    fs::create_dir_all(&snap_root).unwrap();
    create_synthetic_zotero_db(&zot_dir);

    let db_path = zot_dir.join("zotero.sqlite");
    let writer = Connection::open(&db_path).unwrap();
    writer
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA wal_autocheckpoint = 0;
             INSERT INTO items VALUES(13,1,'ZWAL003','2026-02-01');
             INSERT INTO itemDataValues VALUES(8,'WAL Uncheckpointed Paper');
             INSERT INTO itemData VALUES(13,1,8);",
        )
        .unwrap();

    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    assert!(
        fs::metadata(&wal_path).is_ok_and(|m| m.len() > 0),
        "expected non-empty -wal sidecar"
    );
    let db_hash_before = sha256_file(&db_path);
    let wal_hash_before = sha256_file(&wal_path);

    let snap = read_zotero_library_with_temp_root(&zot_dir, Some(&snap_root)).unwrap();
    assert!(snap.used_snapshot);
    assert_eq!(snap.items.len(), 3);
    assert!(snap
        .items
        .iter()
        .any(|i| i.zotero_key == "ZWAL003"
            && i.fields.get("title").map(String::as_str) == Some("WAL Uncheckpointed Paper")));

    let leftovers: Vec<_> = fs::read_dir(&snap_root).unwrap().flatten().collect();
    assert!(leftovers.is_empty(), "snapshot dir leaked: {leftovers:?}");
    assert_eq!(sha256_file(&db_path), db_hash_before);
    assert_eq!(sha256_file(&wal_path), wal_hash_before);
    drop(writer);
}
