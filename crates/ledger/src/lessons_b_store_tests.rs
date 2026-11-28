//! Unit tests for F16b `user_lesson` SQLite store and v1 -> v2 migration.

use crate::lessons_b_store::UserLessonRow;
use crate::lessons_db::{LessonsDb, LESSONS_SCHEMA_VERSION};
use rusqlite::{params, Connection};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "re_search_lessons_b_{tag}_{}_{}",
            std::process::id(),
            id
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn sample_row(id: &str, text: &str, created_ms: i64) -> UserLessonRow {
    UserLessonRow {
        id: id.to_string(),
        text: text.to_string(),
        scope_kind: "mode_field".to_string(),
        scope_mode: Some("solve".to_string()),
        scope_field: Some("qft".to_string()),
        scope_paper_key: None,
        from_node_json: r#"{"session":"s1","mode":"solve","id":"step-1"}"#.to_string(),
        source_kind: "step_correction".to_string(),
        model_summary: false,
        created_ms,
        updated_ms: created_ms,
        uses: 0,
    }
}

#[test]
fn fresh_db_opens_at_v2_with_user_lesson_table() {
    let dir = TempDir::new("fresh_v2");
    let db = LessonsDb::open(&dir.path).expect("open fresh lessons.db");
    assert_eq!(db.db_path(), dir.path.join("library").join("lessons.db"));
    let ver: u32 = db
        .conn()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("read user_version");
    assert_eq!(ver, LESSONS_SCHEMA_VERSION);
    assert_eq!(LESSONS_SCHEMA_VERSION, 2);

    let row = sample_row("les-1", "Use mostly-minus metric signature (+,-,-,-).", 1000);
    db.insert_user_lesson(&row).expect("insert row");
    assert_eq!(db.get_user_lesson("les-1").unwrap(), Some(row));
}

#[test]
fn migrates_v1_to_v2_preserving_f16a_rows_and_refuses_downgrade() {
    let dir = TempDir::new("migrate_v1_v2");
    let library_dir = dir.path.join("library");
    fs::create_dir_all(&library_dir).unwrap();
    let db_path = library_dir.join("lessons.db");

    // Seed a legacy v1 DB with real F16a schema before opening with v2 LessonsDb.
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER NOT NULL PRIMARY KEY CHECK (version >= 1), applied_ms INTEGER NOT NULL) STRICT, WITHOUT ROWID;
             INSERT INTO schema_migrations (version, applied_ms) VALUES (1, 123);
             CREATE TABLE term_alias (
                 term_a TEXT NOT NULL, term_b TEXT NOT NULL,
                 decision TEXT NOT NULL CHECK (decision IN ('confirmed', 'rejected')),
                 origin_session TEXT NOT NULL DEFAULT '',
                 origin_node_json TEXT NOT NULL DEFAULT 'null' CHECK (json_valid(origin_node_json)),
                 updated_ms INTEGER NOT NULL,
                 PRIMARY KEY (term_a, term_b)
             ) STRICT, WITHOUT ROWID;
             CREATE TABLE symbol_default (
                 symbol TEXT NOT NULL, domain TEXT NOT NULL DEFAULT '',
                 assumptions_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(assumptions_json)),
                 dimensions TEXT, convention_note TEXT,
                 origin_session TEXT NOT NULL DEFAULT '', updated_ms INTEGER NOT NULL,
                 PRIMARY KEY (symbol, domain)
             ) STRICT, WITHOUT ROWID;
             CREATE TABLE convention_default (
                 conv_key TEXT NOT NULL, conv_value TEXT NOT NULL, domain TEXT NOT NULL DEFAULT '',
                 origin_session TEXT NOT NULL DEFAULT '', updated_ms INTEGER NOT NULL,
                 PRIMARY KEY (conv_key, domain)
             ) STRICT, WITHOUT ROWID;
             CREATE TABLE skeleton_override (
                 node_tag TEXT NOT NULL PRIMARY KEY, paper_key TEXT, kind_override TEXT,
                 end_line_override INTEGER CHECK (end_line_override IS NULL OR end_line_override >= 1),
                 note TEXT NOT NULL DEFAULT '', origin_session TEXT NOT NULL DEFAULT '',
                 updated_ms INTEGER NOT NULL
             ) STRICT, WITHOUT ROWID;
             CREATE TABLE ocr_overlay (
                 file_sha256 TEXT NOT NULL, line_no INTEGER NOT NULL CHECK (line_no >= 1),
                 corrected_text TEXT NOT NULL, original_sha256 TEXT,
                 origin_session TEXT NOT NULL DEFAULT '', updated_ms INTEGER NOT NULL,
                 PRIMARY KEY (file_sha256, line_no)
             ) STRICT, WITHOUT ROWID;
             CREATE TABLE ingested_cursor (
                 session_id TEXT NOT NULL PRIMARY KEY, last_seq INTEGER NOT NULL CHECK (last_seq >= 0),
                 updated_ms INTEGER NOT NULL
             ) STRICT, WITHOUT ROWID;
             INSERT INTO term_alias (term_a, term_b, decision, origin_session, origin_node_json, updated_ms)
             VALUES ('rg', 'renormalization group', 'confirmed', 's0', 'null', 100);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    let db = LessonsDb::open(&dir.path).expect("migrate v1 -> v2");
    let alias_count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM term_alias", [], |r| r.get(0))
        .unwrap();
    assert_eq!(alias_count, 1, "existing F16a rows preserved");

    let ver: u32 = db
        .conn()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(ver, 2);

    db.insert_user_lesson(&sample_row("les-2", "Check loop symmetry factor 1/2.", 2000))
        .unwrap();
    assert_eq!(db.list_user_lessons().unwrap().len(), 1);

    // Bump user_version higher than supported and confirm open refuses downgrade.
    drop(db);
    let conn = Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "user_version", 99_u32).unwrap();
    drop(conn);
    assert!(LessonsDb::open(&dir.path).is_err());
}

#[test]
fn crud_uses_and_restore_roundtrip() {
    let dir = TempDir::new("crud");
    let db = LessonsDb::open(&dir.path).unwrap();

    db.insert_user_lesson(&sample_row("les-a", "First note", 100))
        .unwrap();
    db.insert_user_lesson(&sample_row("les-b", "Second note", 200))
        .unwrap();

    let listed = db.list_user_lessons().unwrap();
    assert_eq!(
        listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["les-b", "les-a"]
    );

    assert!(db
        .update_user_lesson(
            "les-a",
            "First note edited (model)",
            "global",
            None,
            None,
            None,
            Some(true),
            300,
        )
        .unwrap());
    let edited = db.get_user_lesson("les-a").unwrap().unwrap();
    assert_eq!(edited.text, "First note edited (model)");
    assert_eq!(edited.scope_kind, "global");
    assert!(edited.model_summary);

    // Duplicate ids in one increment call must only increment once per id.
    let inc = db
        .increment_user_lesson_uses(&["les-a".into(), "les-a".into(), "les-b".into()])
        .unwrap();
    assert_eq!(inc, 2);
    assert_eq!(db.get_user_lesson("les-a").unwrap().unwrap().uses, 1);
    assert_eq!(db.get_user_lesson("les-b").unwrap().unwrap().uses, 1);

    assert!(db.delete_user_lesson("les-a").unwrap());
    assert!(!db.delete_user_lesson("les-a").unwrap());
    assert_eq!(db.list_user_lessons().unwrap().len(), 1);

    // Restore replacing existing rows with snapshot.
    let backup_rows = vec![UserLessonRow {
        uses: 4,
        ..sample_row("les-restored", "Keep gauge parameter xi symbolic.", 500)
    }];
    assert_eq!(db.restore_user_lessons(&backup_rows, true).unwrap(), 1);
    let restored = db.list_user_lessons().unwrap();
    assert_eq!(restored, backup_rows);

    // SQLite CHECK / validation rejects empty text.
    let mut bad = sample_row("les-bad", "   ", 600);
    assert!(db.insert_user_lesson(&bad).is_err());
    bad.text = "valid".into();
    bad.scope_kind = "invalid_scope".into();
    assert!(db.insert_user_lesson(&bad).is_err());

    // Ensure duplicate insert does not overwrite.
    let dup = sample_row("les-restored", "Other", 700);
    assert!(db.insert_user_lesson(&dup).is_err());
    assert_eq!(
        db.get_user_lesson("les-restored").unwrap().unwrap().text,
        "Keep gauge parameter xi symbolic."
    );
    let _ = params![1];
}
