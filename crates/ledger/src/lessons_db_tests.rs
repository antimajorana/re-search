use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct TestTempDir(PathBuf);

impl TestTempDir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "re-search-lessons-db-{tag}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opens_lessons_db_in_library_dir_and_refuses_downgrade() {
    let tmp = TestTempDir::new("downgrade");
    let db = LessonsDb::open(tmp.path()).unwrap();
    assert_eq!(db.schema_version().unwrap(), LESSONS_SCHEMA_VERSION);
    assert!(db.db_path().ends_with("library/lessons.db"));
    let path = db.db_path().to_path_buf();
    drop(db);

    // Simulate a newer schema version -> must fail closed.
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", LESSONS_SCHEMA_VERSION + 1)
        .unwrap();
    drop(conn);
    assert!(LessonsDb::open(tmp.path()).is_err());
}

#[test]
fn crud_and_snapshot_roundtrip_across_all_f16a_tables() {
    let tmp = TestTempDir::new("crud");
    let mut db = LessonsDb::open(tmp.path()).unwrap();
    let sha = "b".repeat(64);

    db.upsert_term_alias(&TermAliasRow {
        term_a: "chern number".into(),
        term_b: "tkkn invariant".into(),
        decision: "confirmed".into(),
        origin_session: "s1".into(),
        origin_node_json: "null".into(),
        updated_ms: 100,
    })
    .unwrap();
    db.upsert_symbol_default(&SymbolDefaultRow {
        symbol: "m".into(),
        domain: "qft".into(),
        assumptions_json: r#"["real","positive"]"#.into(),
        dimensions: Some("mass".into()),
        convention_note: Some("rest mass".into()),
        origin_session: "s1".into(),
        updated_ms: 100,
    })
    .unwrap();
    db.upsert_convention_default(&ConventionDefaultRow {
        conv_key: "metric_signature".into(),
        conv_value: "(-,+,+,+)".into(),
        domain: "qft".into(),
        origin_session: "s1".into(),
        updated_ms: 100,
    })
    .unwrap();
    db.upsert_skeleton_override(&SkeletonOverrideRow {
        node_tag: "n-abc1234567".into(),
        paper_key: Some("arxiv:2401.00001".into()),
        kind_override: Some("theorem".into()),
        end_line_override: Some(42),
        note: "proof ends at line 42".into(),
        origin_session: "s1".into(),
        updated_ms: 100,
    })
    .unwrap();
    db.upsert_ocr_overlay(&OcrOverlayRow {
        file_sha256: sha.clone(),
        line_no: 7,
        corrected_text: "E = mc^2".into(),
        original_sha256: None,
        origin_session: "s1".into(),
        updated_ms: 100,
    })
    .unwrap();

    let snap = db.export_snapshot().unwrap();
    assert_eq!(snap.term_aliases.len(), 1);
    assert_eq!(snap.symbol_defaults.len(), 1);
    assert_eq!(snap.convention_defaults.len(), 1);
    assert_eq!(snap.skeleton_overrides.len(), 1);
    assert_eq!(snap.ocr_overlays.len(), 1);

    // Delete one row and verify restore brings it back.
    assert!(db.delete_term_alias("chern number", "tkkn invariant").unwrap());
    assert!(db.list_term_aliases().unwrap().is_empty());
    db.replace_snapshot(&snap).unwrap();
    assert_eq!(db.export_snapshot().unwrap(), snap);

    // Invalid SHA-256 rejected by ocr_overlay.
    assert!(db
        .upsert_ocr_overlay(&OcrOverlayRow {
            file_sha256: "not-a-hash".into(),
            line_no: 1,
            corrected_text: "x".into(),
            original_sha256: None,
            origin_session: "s1".into(),
            updated_ms: 100,
        })
        .is_err());
}
