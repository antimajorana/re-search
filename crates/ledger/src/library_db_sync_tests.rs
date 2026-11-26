use super::*;
use crate::library_rows::{LibrarySearchParams, LibrarySourceItemRow};
use crate::sha256_hex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SYNC_TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let n = SYNC_TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "research-f13a-sync-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_source(id: &str, kind: &str, root: &str) -> LibrarySourceRow {
    LibrarySourceRow {
        id: id.into(),
        kind: kind.into(),
        root: root.into(),
        settings_json: "{}".into(),
        added_ms: 1_000,
        last_sync_ms: None,
        state: "pending_sync".into(),
    }
}

fn make_item(
    source_id: &str,
    external_id: &str,
    paper_key: &str,
    version: Option<&str>,
    file_path: &str,
    role: &str,
    title: &str,
) -> LibrarySourceItemRow {
    LibrarySourceItemRow {
        source_id: source_id.into(),
        external_id: external_id.into(),
        paper_key: Some(paper_key.into()),
        citekey: None,
        ids_json: "{}".into(),
        version: version.map(str::to_string),
        title: title.into(),
        authors_json: r#"["Ada Synth"]"#.into(),
        year: Some(2025),
        item_kind: "preprint".into(),
        collections_json: "[]".into(),
        tags_json: "[]".into(),
        files_json: serde_json::json!([{
            "path": file_path,
            "role": role,
            "size": 1024,
            "mtime_ms": 2000,
            "sha256": null
        }])
        .to_string(),
        state: "present".into(),
        updated_ms: 2_000,
    }
}

fn make_user_ft(paper_key: &str, version: Option<&str>, path: &str, format: &str) -> LibraryFulltextRow {
    LibraryFulltextRow {
        paper_key: paper_key.into(),
        version: version.map(str::to_string),
        raw_blob: String::new(),
        format: format.into(),
        bytes: 1024,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 2_000,
        file_path: Some(path.into()),
        file_size: Some(1024),
        file_mtime_ms: Some(2_000),
        file_sha256: None,
    }
}

#[test]
fn apply_source_sync_batch_rolls_back_all_tables_on_alias_fk_error() {
    let dir = temp_dir("atomic-rollback");
    let db = LibraryDb::open(&dir).unwrap();
    let src = make_source("src-1", "folder", "/virtual/folder");
    db.upsert_source(&src).unwrap();

    let batch = SourceSyncBatch {
        expected_source: SourceSyncSnapshot::from_source_row(&src),
        upsert_source_items: vec![make_item(
            "src-1",
            "p1.pdf",
            "doi:10.9999/ok.1",
            None,
            "/virtual/folder/p1.pdf",
            "pdf",
            "Synthetic Rollback Test",
        )],
        paper_updates: vec![SourceSyncPaperUpdate {
            paper_key: "doi:10.9999/ok.1".into(),
            ids_json: r#"{"doi":"10.9999/ok.1"}"#.into(),
            title: "Synthetic Rollback Test".into(),
            authors_json: r#"["Ada Synth"]"#.into(),
            year: Some(2025),
            abstract_text: String::new(),
            venue: None,
            seen_ms: 2_000,
            mode: PaperUpdateMode::UpsertMerge,
        }],
        // Deliberately targets a non-existent paper_key -> FK violation must abort everything!
        aliases: vec![("arxiv:2501.00001".into(), "doi:10.9999/nonexistent".into())],
        upsert_user_fulltext: vec![make_user_ft(
            "doi:10.9999/ok.1",
            None,
            "/virtual/folder/p1.pdf",
            "pdf",
        )],
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 2_000,
        next_state: "synced".into(),
    };

    assert!(db.apply_source_sync_batch(&batch).is_err());

    // Zero partial rows committed and source marker must remain unsynced.
    assert!(db.list_source_items_for_source("src-1").unwrap().is_empty());
    assert!(db.get_paper("doi:10.9999/ok.1").unwrap().is_none());
    assert!(db.list_fulltext_for_paper("doi:10.9999/ok.1").unwrap().is_empty());
    assert!(db
        .search_papers(&LibrarySearchParams {
            q: Some("Rollback".into()),
            limit: 10,
            ..Default::default()
        })
        .unwrap()
        .is_empty());
    let live = db.list_sources().unwrap().pop().unwrap();
    assert_eq!(live.state, "pending_sync");
    assert_eq!(live.last_sync_ms, None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn apply_source_sync_batch_rejects_stale_snapshot_and_removed_source() {
    let dir = temp_dir("cas-conflict");
    let db = LibraryDb::open(&dir).unwrap();
    let src = make_source("src-cas", "folder", "/virtual/cas");
    db.upsert_source(&src).unwrap();
    let snap0 = SourceSyncSnapshot::from_source_row(&src);

    let newer_batch = SourceSyncBatch {
        expected_source: snap0.clone(),
        upsert_source_items: vec![make_item(
            "src-cas",
            "doc.pdf",
            "doi:10.9999/cas.1",
            None,
            "/virtual/cas/doc.pdf",
            "pdf",
            "Newer Title",
        )],
        paper_updates: vec![SourceSyncPaperUpdate {
            paper_key: "doi:10.9999/cas.1".into(),
            ids_json: r#"{"doi":"10.9999/cas.1"}"#.into(),
            title: "Newer Title".into(),
            authors_json: r#"["Ada Synth"]"#.into(),
            year: Some(2025),
            abstract_text: String::new(),
            venue: None,
            seen_ms: 5_000,
            mode: PaperUpdateMode::UpsertMerge,
        }],
        aliases: Vec::new(),
        upsert_user_fulltext: vec![make_user_ft(
            "doi:10.9999/cas.1",
            None,
            "/virtual/cas/doc.pdf",
            "pdf",
        )],
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 5_000,
        next_state: "synced".into(),
    };
    let committed_ts = db.apply_source_sync_batch(&newer_batch).unwrap();
    assert_eq!(committed_ts, 5_000);

    // Stale scan carrying snap0 attempts to commit older title -> must fail without overwriting.
    let stale_batch = SourceSyncBatch {
        expected_source: snap0,
        upsert_source_items: vec![make_item(
            "src-cas",
            "doc.pdf",
            "doi:10.9999/cas.1",
            None,
            "/virtual/cas/doc.pdf",
            "pdf",
            "Stale Title",
        )],
        paper_updates: vec![SourceSyncPaperUpdate {
            paper_key: "doi:10.9999/cas.1".into(),
            ids_json: r#"{"doi":"10.9999/cas.1"}"#.into(),
            title: "Stale Title".into(),
            authors_json: r#"["Ada Synth"]"#.into(),
            year: Some(2025),
            abstract_text: String::new(),
            venue: None,
            seen_ms: 4_000,
            mode: PaperUpdateMode::UpsertMerge,
        }],
        aliases: Vec::new(),
        upsert_user_fulltext: Vec::new(),
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 4_000,
        next_state: "synced".into(),
    };
    let err = db.apply_source_sync_batch(&stale_batch).unwrap_err();
    assert!(err.to_string().contains("concurrent sync conflict"));
    assert_eq!(
        db.get_paper("doi:10.9999/cas.1").unwrap().unwrap().title,
        "Newer Title"
    );

    // Next sync with same wall-clock timestamp advances last_sync_ms strictly monotonically.
    let snap1 = SourceSyncSnapshot::from_source_row(&db.list_sources().unwrap()[0]);
    let same_ms_batch = SourceSyncBatch {
        expected_source: snap1.clone(),
        upsert_source_items: Vec::new(),
        paper_updates: Vec::new(),
        aliases: Vec::new(),
        upsert_user_fulltext: Vec::new(),
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 5_000,
        next_state: "synced".into(),
    };
    let committed_ts2 = db.apply_source_sync_batch(&same_ms_batch).unwrap();
    assert_eq!(committed_ts2, 5_001);

    // Unregister source -> a pending commit must not resurrect it.
    let snap2 = SourceSyncSnapshot::from_source_row(&db.list_sources().unwrap()[0]);
    assert!(db.unregister_source_atomic("src-cas").unwrap());
    let resurrect_batch = SourceSyncBatch {
        expected_source: snap2,
        upsert_source_items: Vec::new(),
        paper_updates: Vec::new(),
        aliases: Vec::new(),
        upsert_user_fulltext: Vec::new(),
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 6_000,
        next_state: "synced".into(),
    };
    assert!(db.apply_source_sync_batch(&resurrect_batch).is_err());
    assert!(db.list_sources().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unregister_source_atomic_preserves_overlapping_paths_fallback_copies_and_other_origins() {
    let dir = temp_dir("unregister-overlap");
    let db = LibraryDb::open(&dir).unwrap();

    // Unrelated origin fulltext row (e.g. arxiv_eprint) must survive both sync and unregister.
    let blob_sha = sha256_hex(b"synthetic arxiv tex payload");
    db.upsert_fulltext(&LibraryFulltextRow {
        paper_key: "arxiv:2502.00001".into(),
        version: Some("v1".into()),
        raw_blob: blob_sha.clone(),
        format: "tex".into(),
        bytes: 27,
        origin: "arxiv_eprint".into(),
        fetched_ms: 1_000,
        file_path: None,
        file_size: None,
        file_mtime_ms: None,
        file_sha256: None,
    })
    .unwrap();

    let s1 = make_source("vault-1", "kandinsky_vault", "/virtual/vault");
    let s2 = make_source("folder-1", "folder", "/virtual/shared");
    db.upsert_source(&s1).unwrap();
    db.upsert_source(&s2).unwrap();

    // Both sources index:
    // - `/virtual/shared/same.pdf` (same file path!)
    // - `doi:10.9999/two.copies` (different file paths for same paper+version)
    // - `arxiv:2502.00001` v1 (already has `arxiv_eprint` fulltext row)
    let b1 = SourceSyncBatch {
        expected_source: SourceSyncSnapshot::from_source_row(&s1),
        upsert_source_items: vec![
            make_item(
                "vault-1",
                "k_same",
                "doi:10.9999/shared.path",
                None,
                "/virtual/shared/same.pdf",
                "pdf",
                "Shared Path Paper",
            ),
            make_item(
                "vault-1",
                "k_copy",
                "doi:10.9999/two.copies",
                None,
                "/virtual/vault/copy_a.pdf",
                "pdf",
                "Two Copies Paper",
            ),
            make_item(
                "vault-1",
                "k_arxiv",
                "arxiv:2502.00001",
                Some("v1"),
                "/virtual/vault/arxiv_local.pdf",
                "pdf",
                "Arxiv Origin Paper",
            ),
        ],
        paper_updates: Vec::new(),
        aliases: Vec::new(),
        upsert_user_fulltext: vec![
            make_user_ft("doi:10.9999/shared.path", None, "/virtual/shared/same.pdf", "pdf"),
            make_user_ft("doi:10.9999/two.copies", None, "/virtual/vault/copy_a.pdf", "pdf"),
            make_user_ft("arxiv:2502.00001", Some("v1"), "/virtual/vault/arxiv_local.pdf", "pdf"),
        ],
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 2_000,
        next_state: "synced".into(),
    };
    db.apply_source_sync_batch(&b1).unwrap();

    // Verify arxiv_eprint row was NOT overwritten by user_library sync.
    let ax_rows = db.list_fulltext_for_paper("arxiv:2502.00001").unwrap();
    assert_eq!(ax_rows.len(), 1);
    assert_eq!(ax_rows[0].origin, "arxiv_eprint");
    assert_eq!(ax_rows[0].raw_blob, blob_sha);

    let b2 = SourceSyncBatch {
        expected_source: SourceSyncSnapshot::from_source_row(&s2),
        upsert_source_items: vec![
            make_item(
                "folder-1",
                "same.pdf",
                "doi:10.9999/shared.path",
                None,
                "/virtual/shared/same.pdf",
                "pdf",
                "Shared Path Paper",
            ),
            make_item(
                "folder-1",
                "copy_b.pdf",
                "doi:10.9999/two.copies",
                None,
                "/virtual/shared/copy_b.pdf",
                "pdf",
                "Two Copies Paper",
            ),
        ],
        paper_updates: Vec::new(),
        aliases: Vec::new(),
        upsert_user_fulltext: vec![make_user_ft(
            "doi:10.9999/shared.path",
            None,
            "/virtual/shared/same.pdf",
            "pdf",
        )],
        stale_user_paths: Vec::new(),
        affected_fulltext_keys: Vec::new(),
        sync_ts_ms: 2_500,
        next_state: "synced".into(),
    };
    db.apply_source_sync_batch(&b2).unwrap();

    // Unregister vault-1:
    assert!(db.unregister_source_atomic("vault-1").unwrap());
    // Retry is idempotent and returns Ok(false).
    assert!(!db.unregister_source_atomic("vault-1").unwrap());

    // 1. Shared path is preserved because folder-1 still indexes it.
    let shared_ft = db.list_fulltext_for_paper("doi:10.9999/shared.path").unwrap();
    assert_eq!(shared_ft.len(), 1);
    assert_eq!(shared_ft[0].file_path.as_deref(), Some("/virtual/shared/same.pdf"));

    // 2. Two-copies paper re-pointed from vault-1's copy_a.pdf to surviving folder-1's copy_b.pdf.
    let copy_ft = db.list_fulltext_for_paper("doi:10.9999/two.copies").unwrap();
    assert_eq!(copy_ft.len(), 1);
    assert_eq!(copy_ft[0].file_path.as_deref(), Some("/virtual/shared/copy_b.pdf"));

    // 3. Unrelated origin arxiv_eprint is untouched.
    let ax_after = db.list_fulltext_for_paper("arxiv:2502.00001").unwrap();
    assert_eq!(ax_after.len(), 1);
    assert_eq!(ax_after[0].origin, "arxiv_eprint");
    assert_eq!(ax_after[0].raw_blob, blob_sha);

    // Unregister folder-1 too -> now both user_library rows are cleaned up, arxiv_eprint stays.
    assert!(db.unregister_source_atomic("folder-1").unwrap());
    assert!(db.list_fulltext_for_paper("doi:10.9999/shared.path").unwrap().is_empty());
    assert!(db.list_fulltext_for_paper("doi:10.9999/two.copies").unwrap().is_empty());
    assert_eq!(db.list_fulltext_for_paper("arxiv:2502.00001").unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
