//! Integration tests for package F10 (the global paper library).
//!
//! All fixtures are synthetic and self-written; no network calls or model calls are made.

use ledger::{sha256_hex, FulltextCache, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    FulltextRecord, Library, LibraryIds, LibraryQuery, MarkStatus, PageBasis, PaperDetail,
    PaperFlags, PaperRecord, PaperTextRecord, SeenRecord, SeenRole, SkeletonRecord,
    SourceItemFile, SourceItemRecord, SourceItemState, SourceKind, SourceRecord,
    LIBRARY_SCHEMA_VERSION, ORIGIN_USER_LIBRARY,
};
use research_tools::paper_key::PaperKey;
use research_tools::ResearchEnv;
use serde_json::json;
use std::sync::Mutex;
use tempfile::TempDir;

const SYNTHETIC_TEX_PAPER: &[u8] = br"\documentclass{article}
\title{Synthetic Lattice Gauge Identities}
\begin{document}
\section{Introduction}
We construct an exact discrete identity for toy plaquette operators.
\begin{theorem}
For every finite ring $Z_N$, the cyclic trace satisfies $T(a) = T(a^{-1})$.
\end{theorem}
\end{document}
";

const SYNTHETIC_SOLVE_NOTE: &[u8] = br"\documentclass{article}
\title{Toy Series Summation Reference}
\begin{document}
The finite harmonic tail obeys the synthetic bound $H_n - H_m \le (n-m)/m$.
\end{document}
";

const SYNTHETIC_ARXIV_ATOM: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>http://arxiv.org/abs/2402.00123v2</id>
    <published>2024-02-10T00:00:00Z</published>
    <updated>2024-02-14T00:00:00Z</updated>
    <title>Synthetic Lattice Gauge Identities</title>
    <summary>A self-written abstract discussing discrete plaquette symmetry bounds.</summary>
    <author><name>Ada Example</name></author>
    <author><name>Carl Synthetic</name></author>
    <arxiv:doi>10.9999/synth.2024.00123</arxiv:doi>
  </entry>
</feed>
"#;

const SYNTHETIC_INSPIRE_JSON: &[u8] = br#"{
  "hits": {
    "hits": [
      {
        "id": "987654",
        "metadata": {
          "titles": [{"title": "Toy Dualities on Finite Graphs"}],
          "authors": [{"full_name": "Emmy Fixture"}],
          "earliest_date": "2023-05-01",
          "abstracts": [{"value": "Synthetic graph duality notes for library tests."}],
          "dois": [{"value": "10.9999/graph.2023.77"}]
        }
      }
    ]
  }
}"#;

#[test]
fn f3_database_upgrades_in_place_and_remains_usable_by_fulltext_cache() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    // 1. Seed `<data_dir>/library/library.db` through F3's `FulltextCache`.
    let f3 = FulltextCache::open(data_dir).unwrap();
    let put = f3
        .put_bytes(
            "arxiv:2402.00123",
            Some("v2"),
            "tex",
            "arxiv_eprint",
            SYNTHETIC_TEX_PAPER,
        )
        .unwrap();
    drop(f3);

    // 2. Open with F10 `Library` -> upgrades in place to schema v1 without data loss.
    let lib = Library::open(data_dir).unwrap();
    assert_eq!(lib.schema_version().unwrap(), LIBRARY_SCHEMA_VERSION);
    let key = PaperKey::parse("arxiv:2402.00123").unwrap();
    let (ft, bytes) = lib
        .read_fulltext_bytes(&key, Some("v2"))
        .unwrap()
        .expect("F3 cached fulltext preserved after upgrade");
    assert_eq!(ft.raw_blob.as_deref(), Some(put.raw_blob.as_str()));
    assert_eq!(bytes, SYNTHETIC_TEX_PAPER);

    // 3. Add a `user_library` fulltext row (F13 shape, not yet hashed).
    lib.upsert_fulltext(&FulltextRecord {
        key: PaperKey::parse("doi:10.9999/owned.1").unwrap(),
        version: None,
        raw_blob: None,
        format: "pdf".into(),
        bytes: 1234,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 9_000,
        file_path: Some("refs/papers/owned2024/paper.pdf".into()),
        file_size: Some(1234),
        file_mtime_ms: Some(1_700_000_000_000),
        file_sha256: None,
    })
    .unwrap();
    drop(lib);

    // 4. F3 `FulltextCache` still opens, reads the original row, ignores unhashed user_library,
    //    and writes new entries without error.
    let f3_after = FulltextCache::open(data_dir).unwrap();
    let (_, got) = f3_after
        .get_bytes("arxiv:2402.00123", Some("v2"))
        .unwrap()
        .expect("F3 entry still readable by FulltextCache");
    assert_eq!(got, SYNTHETIC_TEX_PAPER);
    assert!(f3_after
        .get_bytes("doi:10.9999/owned.1", None)
        .unwrap()
        .is_none());
    let extra = f3_after
        .put_bytes(
            "arxiv:2405.00009",
            Some("v1"),
            "tex",
            "arxiv_eprint",
            SYNTHETIC_SOLVE_NOTE,
        )
        .unwrap();
    assert_eq!(extra.raw_blob, sha256_hex(SYNTHETIC_SOLVE_NOTE));
}

#[test]
fn delete_database_rebuild_and_compare() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();
    let env = ResearchEnv::new(data_dir);
    let sandbox = env.sandbox();
    let ledger_mutex = Mutex::new(Ledger::open(data_dir).unwrap());

    // Session 1 (Investigate): ingest arXiv TeX paper, record evidence and skeleton.
    ledger_mutex.lock().unwrap().append("sess-inv", "investigation.started", 1, &json!({})).unwrap();
    let inv_source = ingest_bytes(&sandbox, &ledger_mutex, &IngestRequest {
        session_id: "sess-inv", file_name: "2402.00123v2.tex",
        origin: "https://arxiv.org/e-print/2402.00123v2", raw_bytes: SYNTHETIC_TEX_PAPER,
        source_id_hint: Some("arxiv-2402-00123"), section_filter: None, title: None,
    }).unwrap();
    {
        let g = ledger_mutex.lock().unwrap();
        let skel_blob = g.put_blob(br#"{"nodes":[{"id":"n-synthetic"}]}"#).unwrap();
        g.append("sess-inv", "investigation.skeleton", 1, &json!({
            "source_id": inv_source.source_id, "builder": "tex-v1", "blob_sha256": skel_blob,
        })).unwrap();
        g.append("sess-inv", "investigation.evidence", 1, &json!({
            "source_id": inv_source.source_id, "role": "result",
            "gist": "Cyclic trace invariance on Z_N",
            "quote": "the cyclic trace satisfies $T(a) = T(a^{-1})$.",
            "start_line": 7, "end_line": 7,
        })).unwrap();
    }

    // Session 2 (Solve): ingest local file (gets `file:<16hex>` key) and add shared evidence.quote.
    ledger_mutex.lock().unwrap().append("sess-solve", "solve.started", 1, &json!({})).unwrap();
    let solve_source = ingest_bytes(&sandbox, &ledger_mutex, &IngestRequest {
        session_id: "sess-solve", file_name: "harmonic_note.tex", origin: "harmonic_note.tex",
        raw_bytes: SYNTHETIC_SOLVE_NOTE, source_id_hint: None, section_filter: None, title: None,
    }).unwrap();
    let expected_file_key = PaperKey::from_file_bytes(SYNTHETIC_SOLVE_NOTE);
    assert_eq!(expected_file_key.as_str(), format!("file:{}", &solve_source.raw_blob[..16]));
    ledger_mutex.lock().unwrap().append("sess-solve", "evidence.quote", 1, &json!({
        "source_id": solve_source.source_id, "role": "cited",
        "gist": "Harmonic tail upper bound", "quote": "H_n - H_m \\le (n-m)/m",
        "start_line": 4, "end_line": 4,
    })).unwrap();

    // Session 3 (Explore): literature.fetched blobs (arXiv Atom + INSPIRE JSON) + idea.triage.
    {
        let g = ledger_mutex.lock().unwrap();
        g.append("sess-explore", "idea.started", 1, &json!({})).unwrap();
        let arxiv_blob = g.put_blob(SYNTHETIC_ARXIV_ATOM).unwrap();
        g.append("sess-explore", "literature.fetched", 1, &json!({
            "provider": "arxiv", "query": "plaquette", "count": 1, "raw_blob": arxiv_blob,
        })).unwrap();
        let inspire_blob = g.put_blob(SYNTHETIC_INSPIRE_JSON).unwrap();
        g.append("sess-explore", "literature.fetched", 1, &json!({
            "provider": "inspire", "query": "graph duality", "count": 1, "raw_blob": inspire_blob,
        })).unwrap();
        g.append("sess-explore", "idea.triage", 1, &json!({
            "paper_key": "doi:10.9999/graph.2023.77", "status": "triaged",
        })).unwrap();
    }

    // Build initial index and record non-rebuildable user marks.
    let mut lib = Library::open(data_dir).unwrap();
    let arxiv_key = PaperKey::parse("arxiv:2402.00123").unwrap();
    let doi_key = PaperKey::parse("doi:10.9999/graph.2023.77").unwrap();
    let first_report = lib.rebuild().unwrap();
    assert_eq!(first_report.sessions_scanned, 3);
    assert_eq!(first_report.papers, 3);
    assert_eq!(first_report.skeleton_entries, 1);
    assert_eq!(first_report.finding_entries, 2);

    lib.set_user_mark(
        &arxiv_key,
        MarkStatus::Key,
        vec!["lattice".into(), "benchmark".into()],
        "Primary test fixture".into(),
    )
    .unwrap();
    lib.set_user_mark(
        &doi_key,
        MarkStatus::Skimmed,
        vec!["duality".into()],
        "Check graph lemmas".into(),
    )
    .unwrap();

    let snapshot_before = take_snapshot(&lib, &[&arxiv_key, &doi_key, &expected_file_key]);
    let db_path = lib.db_path();
    drop(lib);

    // Delete `library.db` (and any SQLite WAL/SHM sidecars) completely.
    std::fs::remove_file(&db_path).unwrap();
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    assert!(!db_path.exists());

    // Reopen and rebuild from ledger + blobs + user_marks.json.
    let mut rebuilt = Library::open(data_dir).unwrap();
    let second_report = rebuilt.rebuild().unwrap();
    assert_eq!(second_report.papers, first_report.papers);
    assert_eq!(second_report.aliases, first_report.aliases);
    assert_eq!(second_report.fulltext_entries, first_report.fulltext_entries);
    assert_eq!(second_report.seen_entries, first_report.seen_entries);
    assert_eq!(second_report.skeleton_entries, first_report.skeleton_entries);
    assert_eq!(second_report.finding_entries, first_report.finding_entries);
    assert_eq!(second_report.user_marks_restored, 2);

    let snapshot_after = take_snapshot(&rebuilt, &[&arxiv_key, &doi_key, &expected_file_key]);
    assert_eq!(snapshot_before, snapshot_after);

    // Verify all expected roles were reconstructed.
    let recent_inv = rebuilt.recent_by_role(SeenRole::Investigated, 10).unwrap();
    assert_eq!(recent_inv.len(), 1);
    assert_eq!(recent_inv[0].paper.key, arxiv_key);
    let recent_solve = rebuilt.recent_by_role(SeenRole::SolveSource, 10).unwrap();
    assert_eq!(recent_solve.len(), 1);
    assert_eq!(recent_solve[0].paper.key, expected_file_key);
}

#[derive(Debug, PartialEq, Eq)]
struct LibrarySnapshot {
    details: Vec<Option<PaperDetail>>,
    list_keys: Vec<String>,
    fts_plaquette_keys: Vec<String>,
    fts_cyclic_keys: Vec<String>,
    marks: Vec<research_tools::library::UserMark>,
}

fn take_snapshot(lib: &Library, keys: &[&PaperKey]) -> LibrarySnapshot {
    let details = keys.iter().map(|k| lib.get_paper(k).unwrap()).collect();
    let list_keys = lib
        .list(50, 0)
        .unwrap()
        .into_iter()
        .map(|s| s.paper.key.to_string())
        .collect();
    let fts_plaquette_keys = lib
        .search(&LibraryQuery {
            q: Some("plaquette".into()),
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .map(|s| s.paper.key.to_string())
        .collect();
    let fts_cyclic_keys = lib
        .search(&LibraryQuery {
            q: Some("cyclic".into()),
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .map(|s| s.paper.key.to_string())
        .collect();
    let marks = lib.list_user_marks().unwrap();
    LibrarySnapshot {
        details,
        list_keys,
        fts_plaquette_keys,
        fts_cyclic_keys,
        marks,
    }
}

#[test]
fn user_marks_sidecar_bak_recovery_and_portable_backup_restore() {
    let tmp = TempDir::new().unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    let k1 = PaperKey::parse("arxiv:2402.00123").unwrap();
    let k2 = PaperKey::parse("doi:10.9999/graph.2023.77").unwrap();

    lib.set_user_mark(&k1, MarkStatus::Key, vec!["alpha".into()], "First mark".into())
        .unwrap();
    // Second write rotates the first valid sidecar into `user_marks.json.bak`.
    lib.set_user_mark(&k2, MarkStatus::Irrelevant, vec!["offtopic".into()], "Skip".into())
        .unwrap();
    assert!(lib.user_marks_path().is_file());
    assert!(lib.user_marks_backup_path().is_file());

    // Portable backup export + tamper detection + clean restore on a fresh data_dir.
    let backup_bytes = lib.export_user_marks_backup_bytes().unwrap();
    let mut tampered = backup_bytes.clone();
    if let Some(pos) = tampered.windows(4).position(|w| w == b"Skip") {
        tampered[pos] = b'X';
    }
    let fresh_tmp = TempDir::new().unwrap();
    let mut fresh_lib = Library::open(fresh_tmp.path()).unwrap();
    assert!(fresh_lib.restore_user_marks_backup(&tampered).is_err());
    assert_eq!(fresh_lib.restore_user_marks_backup(&backup_bytes).unwrap(), 2);
    assert_eq!(fresh_lib.list_user_marks().unwrap(), lib.list_user_marks().unwrap());

    // Simulate primary `user_marks.json` corruption + deleted `library.db` -> recovers from `.bak`.
    let db_path = lib.db_path();
    let primary_path = lib.user_marks_path();
    drop(lib);
    std::fs::remove_file(&db_path).unwrap();
    std::fs::write(&primary_path, b"{corrupt json").unwrap();

    let recovered = Library::open(tmp.path()).unwrap();
    let marks = recovered.list_user_marks().unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].key, k1);
    assert_eq!(marks[0].status, MarkStatus::Key);
}

#[test]
fn search_by_title_author_year_key_alias_status_role_and_f13_f14_tables() {
    let tmp = TempDir::new().unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    let key = PaperKey::parse("arxiv:2402.00123").unwrap();
    let doi_alias = PaperKey::parse("doi:10.9999/synth.2024.00123").unwrap();

    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds {
            arxiv: Some("2402.00123".into()),
            doi: Some("10.9999/synth.2024.00123".into()),
            ..Default::default()
        },
        title: "Synthetic Lattice Gauge Identities".into(),
        authors: vec!["Ada Example".into(), "Carl Synthetic".into()],
        year: Some(2024),
        abstract_text: "Plaquette trace bounds.".into(),
        bibtex: "@article{example2024, title={Synthetic}}".into(),
        flags: PaperFlags::default(),
        first_seen_ms: 1000,
        last_seen_ms: 2000,
    })
    .unwrap();
    lib.record_seen(&SeenRecord {
        key: key.clone(),
        session_id: "s1".into(),
        role: SeenRole::CitedLookup,
        seen_ms: 2500,
    })
    .unwrap();
    lib.upsert_skeleton(&SkeletonRecord {
        key: key.clone(),
        version: Some("v2".into()),
        builder: "tex-v1".into(),
        blob_sha256: sha256_hex(b"{}"),
        built_ms: 2600,
    })
    .unwrap();
    lib.set_user_mark(&key, MarkStatus::Key, vec!["gauge".into()], String::new())
        .unwrap();

    // F13 source + source_item marks the paper `owned = true`.
    lib.upsert_source(&SourceRecord {
        id: "src-vault".into(),
        kind: SourceKind::KandinskyVault,
        root: "/vault".into(),
        settings: json!({ "refs_dir": "refs/papers" }),
        added_ms: 100,
        last_sync_ms: Some(200),
        state: "ok".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "src-vault".into(),
        external_id: "example2024".into(),
        key: Some(key.clone()),
        citekey: Some("example2024".into()),
        ids: LibraryIds { arxiv: Some("2402.00123".into()), ..Default::default() },
        version: Some("v2".into()),
        title: "Synthetic Lattice Gauge Identities".into(),
        authors: vec!["Ada Example".into()],
        year: Some(2024),
        item_kind: "article".into(),
        collections: vec!["lattice".into()],
        tags: vec!["core".into()],
        files: vec![SourceItemFile {
            path: "refs/papers/example2024/paper.pdf".into(),
            role: "pdf".into(),
            size: 1024,
            mtime_ms: 3000,
            sha256: None,
        }],
        state: SourceItemState::Present,
        updated_ms: 3000,
    })
    .unwrap();

    // F14 paper_text extends FTS5 to extracted full text.
    let extracted = b"UniqueFulltextTokenZeta appears only in extracted PDF text.";
    lib.upsert_paper_text(
        &PaperTextRecord {
            key: key.clone(),
            version: Some("v2".into()),
            file_sha256: sha256_hex(SYNTHETIC_TEX_PAPER),
            extractor: "test-pdf/1".into(),
            text_blob: sha256_hex(extracted),
            page_basis: vec![PageBasis::TextLayer],
            extracted_ms: 3100,
        },
        Some(extracted),
    )
    .unwrap();

    // Search by title, author, year, alias key, status, role, and full-text FTS5 token.
    let by_fields = lib
        .search(&LibraryQuery {
            title: Some("lattice gauge".into()),
            author: Some("synthetic".into()),
            year: Some(2024),
            key: Some(doi_alias.clone()),
            status: Some(MarkStatus::Key),
            role: Some(SeenRole::CitedLookup),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(by_fields.len(), 1);
    assert_eq!(by_fields[0].paper.key, key);
    assert!(by_fields[0].owned);
    assert_eq!(by_fields[0].skeleton_count, 1);

    let by_fulltext = lib
        .search(&LibraryQuery {
            q: Some("UniqueFulltextTokenZeta".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(by_fulltext.len(), 1);
    assert_eq!(by_fulltext[0].paper.key, key);

    // Detail lookup via alias resolves to canonical record.
    let detail = lib.get_paper(&doi_alias).unwrap().expect("alias lookup");
    assert_eq!(detail.paper.key, key);
    assert_eq!(detail.source_items.len(), 1);
    assert_eq!(detail.paper_text.len(), 1);
    assert!(lib.get_skeleton(&doi_alias, Some("v2"), "tex-v1").unwrap().is_some());
}

#[test]
fn library_attach_reingests_same_bytes_and_library_loss_does_not_break_verify() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();
    let env = ResearchEnv::new(data_dir);
    let sandbox = env.sandbox();

    // Put a paper into the library fulltext cache (simulating an earlier fetch).
    let cache = FulltextCache::open(data_dir).unwrap();
    let cached = cache
        .put_bytes(
            "arxiv:2402.00123",
            Some("v2"),
            "tex",
            "arxiv_eprint",
            SYNTHETIC_TEX_PAPER,
        )
        .unwrap();
    drop(cache);

    let lib = Library::open(data_dir).unwrap();
    let key = PaperKey::parse("arxiv:2402.00123").unwrap();
    let ledger_mutex = Mutex::new(Ledger::open(data_dir).unwrap());
    let ingested = lib
        .attach_to_session(&sandbox, &ledger_mutex, "sess-attach", &key, Some("v2"), None)
        .unwrap();
    assert_eq!(ingested.raw_blob, cached.raw_blob);
    assert_eq!(ingested.origin, "library:arxiv:2402.00123");

    let db_path = lib.db_path();
    drop(lib);

    // Delete `library.db` completely (Explore rule S5: library loss breaks nothing in sessions).
    std::fs::remove_file(&db_path).unwrap();
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));

    let verify_report = Ledger::open_existing(data_dir).unwrap().verify_all().unwrap();
    assert_eq!(verify_report.sessions.len(), 1);
    assert_eq!(verify_report.unreferenced_blobs, 0);

    // Verify materialized session source file still exists and matches the ledger's pinned SHA-256.
    let events = ledger_mutex.lock().unwrap().read_session("sess-attach").unwrap();
    let ev = events
        .iter()
        .find(|e| e.kind == "source.ingested")
        .expect("source.ingested event exists");
    let primary = ev.payload["primary_path"].as_str().unwrap();
    let expected_sha = ev.payload["files_sha256"][primary].as_str().unwrap();
    let resolved = sandbox
        .resolve_existing(
            research_tools::SandboxRoot::Sources,
            "sess-attach",
            primary,
            false,
        )
        .unwrap();
    let disk_bytes = std::fs::read(&resolved.canonical_target).unwrap();
    assert_eq!(sha256_hex(&disk_bytes), expected_sha);
    assert!(std::str::from_utf8(&disk_bytes)
        .unwrap()
        .contains("cyclic trace satisfies"));
}
