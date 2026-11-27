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
    let db_path = dir.path.join("library").join("lessons.db");
    let db = LessonsDb::open(&db_path).expect("open fresh lessons.db");
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
    let db_path = dir.path.join("lessons.db");

    // Seed a legacy v1 DB manually before opening with v2 LessonsDb.
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_ms INTEGER NOT NULL);
             INSERT INTO schema_migrations (version, applied_ms) VALUES (1, 123);
             CREATE TABLE term_alias (
                 id INTEGER PRIMARY KEY, scope_kind TEXT NOT NULL, scope_key TEXT NOT NULL,
                 alias TEXT NOT NULL, canonical TEXT NOT NULL, source TEXT NOT NULL,
                 session_id TEXT, created_ms INTEGER NOT NULL,
                 UNIQUE (scope_kind, scope_key, alias)
             );
             CREATE TABLE symbol_default (
                 id INTEGER PRIMARY KEY, scope_kind TEXT NOT NULL, scope_key TEXT NOT NULL,
                 symbol TEXT NOT NULL, assumptions_json TEXT NOT NULL, dimensions TEXT,
                 source TEXT NOT NULL, session_id TEXT, created_ms INTEGER NOT NULL,
                 UNIQUE (scope_kind, scope_key, symbol)
             );
             CREATE TABLE convention_default (
                 id INTEGER PRIMARY KEY, scope_kind TEXT NOT NULL, scope_key TEXT NOT NULL,
                 kind TEXT NOT NULL, key_name TEXT NOT NULL, value TEXT NOT NULL,
                 source TEXT NOT NULL, session_id TEXT, created_ms INTEGER NOT NULL,
                 UNIQUE (scope_kind, scope_key, kind, key_name)
             );
             CREATE TABLE skeleton_override (
                 id INTEGER PRIMARY KEY, paper_key TEXT NOT NULL, node_id TEXT NOT NULL,
                 field TEXT NOT NULL, value_json TEXT NOT NULL, source TEXT NOT NULL,
                 session_id TEXT, created_ms INTEGER NOT NULL,
                 UNIQUE (paper_key, node_id, field)
             );
             CREATE TABLE ocr_overlay (
                 id INTEGER PRIMARY KEY, paper_key TEXT NOT NULL, page_no INTEGER NOT NULL,
                 span_fp TEXT NOT NULL, corrected_text TEXT NOT NULL, source TEXT NOT NULL,
                 session_id TEXT, created_ms INTEGER NOT NULL,
                 UNIQUE (paper_key, page_no, span_fp)
             );
             CREATE TABLE ingested_cursor (
                 session_id TEXT NOT NULL, event_kind TEXT NOT NULL,
                 last_seq INTEGER NOT NULL, updated_ms INTEGER NOT NULL,
                 PRIMARY KEY (session_id, event_kind)
             );
             INSERT INTO term_alias (scope_kind, scope_key, alias, canonical, source, created_ms)
             VALUES ('global', '', 'RG', 'renormalization group', 'user_edit', 100);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    let db = LessonsDb::open(&db_path).expect("migrate v1 -> v2");
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
    assert!(LessonsDb::open(&db_path).is_err());
}

#[test]
fn crud_uses_and_restore_roundtrip() {
    let dir = TempDir::new("crud");
    let db = LessonsDb::open(&dir.path.join("lessons.db")).unwrap();

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
