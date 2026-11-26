use super::*;
use crate::fulltext_cache::FulltextCache;
use crate::library_schema::{LIBRARY_SCHEMA_VERSION, ORIGIN_USER_LIBRARY, SEEN_ROLES};
use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_data_dir(label: &str) -> PathBuf {
    let n = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "research-f10-ledger-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fts5_is_enabled_and_indexes_title_abstract_gists_and_full_text() {
    let dir = temp_data_dir("fts5");
    let db = LibraryDb::open(&dir).unwrap();
    assert_eq!(db.schema_version().unwrap(), LIBRARY_SCHEMA_VERSION);

    let paper = LibraryPaperRow {
        paper_key: "arxiv:2103.01234".into(),
        ids_json: r#"{"arxiv":"2103.01234"}"#.into(),
        title: "Modular Bootstrap in Two Dimensions".into(),
        authors_json: r#"["Ada Lovelace","Emmy Noether"]"#.into(),
        year: Some(2021),
        abstract_text: "We derive exact conformal bounds.".into(),
        bibtex: String::new(),
        flags_json: "{}".into(),
        first_seen_ms: 1000,
        last_seen_ms: 2000,
    };
    db.upsert_paper(&paper).unwrap();

    db.upsert_finding(&LibraryFindingRow {
        paper_key: "arxiv:2103.01234".into(),
        session_id: "sess-1".into(),
        evidence_seq: 5,
        role: "result".into(),
        gist: "Uniqueness of the extremal functional".into(),
        quote: "The extremal functional is unique.".into(),
        cited_start: Some(12),
        cited_end: Some(15),
        recorded_ms: 2100,
    })
    .unwrap();

    let extracted_text = b"Page 1\x0cPage 2: saddlepoint asymptotics for partition sums.";
    let text_blob = db.put_cache_blob(extracted_text).unwrap();
    let file_sha = sha256_hex(b"%PDF-1.4 synthetic fixture");
    db.upsert_paper_text(&LibraryPaperTextRow {
        paper_key: "arxiv:2103.01234".into(),
        version: Some("v1".into()),
        file_sha256: file_sha,
        extractor: "test-extractor/1".into(),
        text_blob,
        page_basis_json: r#"["text_layer","text_layer"]"#.into(),
        extracted_ms: 2200,
    })
    .unwrap();

    for term in ["Modular", "conformal", "extremal", "saddlepoint", "Noether"] {
        let hits = db
            .search_papers(&LibrarySearchParams {
                q: Some(term.into()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.len(), 1, "expected FTS5 hit for term `{term}`");
        assert_eq!(hits[0].paper.paper_key, "arxiv:2103.01234");
    }

    // Tricky punctuation and FTS5 operators must never cause SQLite syntax errors.
    let tricky = db
        .search_papers(&LibrarySearchParams {
            q: Some("Modular-Bootstrap (2021) OR foo:bar NOT \"bad".into()),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert!(tricky.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upgrades_f3_database_in_place_without_losing_rows() {
    let dir = temp_data_dir("f3-upgrade");

    // 1. Create database using F3's FulltextCache directly (PRAGMA user_version == 0).
    let f3_cache = FulltextCache::open(&dir).unwrap();
    let tex_bytes = b"\\documentclass{article}\\begin{document}F3 fixture\\end{document}";
    let put = f3_cache
        .put_bytes("arxiv:2401.00001", Some("v2"), "tex", "arxiv_eprint", tex_bytes)
        .unwrap();
    drop(f3_cache);

    // Confirm user_version starts at 0 before F10 opens the file.
    {
        let raw_conn = Connection::open(dir.join(LIBRARY_DIR_NAME).join(LIBRARY_DB_NAME)).unwrap();
        let ver: i64 = raw_conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ver, 0);
    }

    // 2. Open with LibraryDb -> upgrades in place to v1.
    let db = LibraryDb::open(&dir).unwrap();
    assert_eq!(db.schema_version().unwrap(), LIBRARY_SCHEMA_VERSION);
    let rows = db.list_fulltext_for_paper("arxiv:2401.00001").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].raw_blob, put.raw_blob);
    assert_eq!(rows[0].version.as_deref(), Some("v2"));
    assert_eq!(rows[0].origin, "arxiv_eprint");
    assert!(rows[0].file_path.is_none());

    // 3. Upgraded database can now store user_library rows with empty raw_blob.
    db.upsert_fulltext(&LibraryFulltextRow {
        paper_key: "doi:10.1000/owned-paper".into(),
        version: None,
        raw_blob: String::new(),
        format: "pdf".into(),
        bytes: 4096,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 5000,
        file_path: Some("vault/refs/papers/smith2020/paper.pdf".into()),
        file_size: Some(4096),
        file_mtime_ms: Some(1_700_000_000_000),
        file_sha256: None,
    })
    .unwrap();
    drop(db);

    // 4. F3 FulltextCache still reads the original row and safely treats unhashed user_library as miss.
    let f3_again = FulltextCache::open(&dir).unwrap();
    let (entry, bytes) = f3_again
        .get_bytes("arxiv:2401.00001", Some("v2"))
        .unwrap()
        .expect("F3 entry preserved after F10 upgrade");
    assert_eq!(entry.raw_blob, put.raw_blob);
    assert_eq!(bytes, tex_bytes);
    assert!(f3_again
        .get_bytes("doi:10.1000/owned-paper", None)
        .unwrap()
        .is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn database_created_by_f10_is_usable_by_f3_fulltext_cache() {
    let dir = temp_data_dir("f10-to-f3");
    let db = LibraryDb::open(&dir).unwrap();
    assert_eq!(db.schema_version().unwrap(), LIBRARY_SCHEMA_VERSION);
    drop(db);

    let cache = FulltextCache::open(&dir).unwrap();
    let payload = b"%PDF-1.4 created via F3 on F10 db";
    let saved = cache
        .put_bytes("arxiv:2501.12345", Some("v1"), "pdf", "arxiv_pdf", payload)
        .unwrap();
    let (got_entry, got_bytes) = cache
        .get_bytes("arxiv:2501.12345", None)
        .unwrap()
        .expect("cache hit");
    assert_eq!(got_entry.raw_blob, saved.raw_blob);
    assert_eq!(got_bytes, payload);
    assert_eq!(cache.remove("arxiv:2501.12345", Some("v1")).unwrap(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn forward_only_migration_refuses_newer_schema_version() {
    let dir = temp_data_dir("future-ver");
    let db = LibraryDb::open(&dir).unwrap();
    db.conn
        .pragma_update(None, "user_version", i64::from(LIBRARY_SCHEMA_VERSION + 10))
        .unwrap();
    drop(db);

    let err = LibraryDb::open(&dir).err().expect("must refuse downgrade");
    assert!(err.to_string().contains("refusing downgrade"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schema_check_constraints_enforce_roles_statuses_and_hashes() {
    let dir = temp_data_dir("checks");
    let db = LibraryDb::open(&dir).unwrap();

    for role in SEEN_ROLES {
        db.record_seen(&LibrarySeenRow {
            paper_key: "arxiv:2103.01234".into(),
            session_id: "s1".into(),
            role: (*role).into(),
            seen_ms: 100,
        })
        .unwrap();
    }
    assert!(db
        .record_seen(&LibrarySeenRow {
            paper_key: "arxiv:2103.01234".into(),
            session_id: "s1".into(),
            role: "bogus_role".into(),
            seen_ms: 100,
        })
        .is_err());

    assert!(db
        .upsert_user_mark(&LibraryUserMarkRow {
            paper_key: "arxiv:2103.01234".into(),
            status: "not_a_status".into(),
            tags_json: "[]".into(),
            note: String::new(),
            updated_ms: 100,
        })
        .is_err());

    assert!(db
        .upsert_skeleton(&LibrarySkeletonRow {
            paper_key: "arxiv:2103.01234".into(),
            version: Some("v1".into()),
            builder: "tex-v1".into(),
            blob_sha256: "not-a-sha256".into(),
            built_ms: 100,
        })
        .is_err());

    assert!(db
        .upsert_source(&LibrarySourceRow {
            id: "src-1".into(),
            kind: "unknown_kind".into(),
            root: "/vault".into(),
            settings_json: "{}".into(),
            added_ms: 100,
            last_sync_ms: None,
            state: "idle".into(),
        })
        .is_err());

    let _ = std::fs::remove_dir_all(&dir);
}
