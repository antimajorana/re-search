//! Focused synthetic tests for F13a sync integrity:
//! - Atomic multi-table sync commits and error rollback (no partial index with a `synced` marker)
//! - Interleaved and concurrent sync protection (old scans never overwrite newer updates)
//! - File reads kept outside SQLite write transactions
//! - Overlapping folder/vault source unregistration preserving shared paths, fallback copies,
//!   unrelated fulltext origins, and safe idempotent retries
//!
//! All fixture files and PDFs are self-written synthetic data; no real user notes or papers.

mod pdf_support;

use ledger::{FulltextCache, Ledger};
use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::library::{Library, LibraryQuery, MarkStatus, SourceKind, ORIGIN_USER_LIBRARY};
use research_tools::paper_key::PaperKey;
use research_tools::paper_sources::{
    attach_source_paper_to_session, sha256_hex, SOURCE_STATE_PENDING, SOURCE_STATE_SYNCED,
};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::source_sync::{
    commit_source_sync, prepare_source_sync, register_source, remove_source, sync_source,
    SyncOptions,
};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::{Barrier, Mutex};
use tempfile::TempDir;

fn synth_pdf(lines: &[&str]) -> Vec<u8> {
    build_pdf(&[PageSpec::single_column(lines)], &PdfBuildOptions::default())
}

fn write_vault_entry(vault: &Path, key: &str, frontmatter: &str) {
    let dir = vault.join("refs/papers").join(key);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{key}.md")),
        format!("---\n{frontmatter}\n---\nSynthetic body.\n"),
    )
    .unwrap();
}

fn hash_tree(root: &Path) -> String {
    let mut records = Vec::new();
    collect_tree(root, root, &mut records);
    records.sort();
    sha256_hex(records.join("\n").as_bytes())
}

fn collect_tree(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let p = entry.path();
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if p.is_dir() {
            out.push(format!("D:{rel}"));
            collect_tree(root, &p, out);
        } else {
            let bytes = fs::read(&p).unwrap();
            out.push(format!("F:{rel}:{}:{}", bytes.len(), sha256_hex(&bytes)));
        }
    }
}

fn find_source(lib: &Library, id: &str) -> research_tools::library::SourceRecord {
    lib.list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
}

#[test]
fn interrupted_sync_surfaces_error_rolls_back_atomically_and_recovers_on_retry() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let folder = tmp.path().join("papers");
    fs::create_dir_all(&folder).unwrap();

    fs::write(
        folder.join("alpha.pdf"),
        synth_pdf(&["Atomic Rollback Paper Alpha", "Ada Synth", "2025", "DOI: 10.9999/atomic.001"]),
    )
    .unwrap();
    fs::write(
        folder.join("beta.pdf"),
        synth_pdf(&["Atomic Rollback Paper Beta", "Lin Synth", "2025", "arXiv:2501.00404v1"]),
    )
    .unwrap();

    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::Folder, &folder, json!({}), None).unwrap();
    let opts = SyncOptions { dry_run: false, hash_files: true };

    // Prepare sync outside any write lock, then inject an alias FK failure midway through the batch.
    let mut prepared = prepare_source_sync(&lib, &src.id, &opts, None).unwrap();
    assert_eq!(prepared.report.added, 2);
    prepared.batch.aliases.push((
        "doi:10.9999/dangling.alias".to_string(),
        "doi:10.9999/does.not.exist".to_string(),
    ));

    // Error must surface and roll back every table — no partial items/papers and no `synced` marker.
    let err = commit_source_sync(&lib, prepared).unwrap_err();
    assert!(!err.is_empty(), "expected surfaced commit error");

    let live_src = find_source(&lib, &src.id);
    assert_eq!(live_src.state, SOURCE_STATE_PENDING);
    assert_eq!(live_src.last_sync_ms, None);
    assert!(lib.list_source_items_for_source(&src.id).unwrap().is_empty());
    let k_alpha = PaperKey::parse("doi:10.9999/atomic.001").unwrap();
    let k_beta = PaperKey::parse("arxiv:2501.00404").unwrap();
    assert!(lib.get_paper(&k_alpha).unwrap().is_none());
    assert!(lib.get_paper(&k_beta).unwrap().is_none());

    // Retrying a clean sync commits atomically and marks the source synced.
    let recovered = sync_source(&lib, &src.id, &opts).unwrap();
    assert_eq!(recovered.added, 2);
    let after = find_source(&lib, &src.id);
    assert_eq!(after.state, SOURCE_STATE_SYNCED);
    assert!(after.last_sync_ms.is_some());
    assert!(lib.get_paper(&k_alpha).unwrap().is_some());
    assert!(lib.get_paper(&k_beta).unwrap().is_some());
}

#[test]
fn interleaved_syncs_prevent_stale_scan_from_overwriting_newer_update_or_resurrecting_source() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("refs/papers")).unwrap();

    write_vault_entry(
        &vault,
        "race2025",
        "citekey: race2025\nitemType: journalArticle\npublished: 2025\nauthors:\n  - Vega, M.\ntitle: Initial Version One\nDOI: 10.9999/race.2025.1\npdf: paper.pdf",
    );
    fs::write(
        vault.join("refs/papers/race2025/paper.pdf"),
        synth_pdf(&["Initial Version One", "DOI: 10.9999/race.2025.1"]),
    )
    .unwrap();

    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::KandinskyVault, &vault, json!({}), None).unwrap();
    let r0 = sync_source(&lib, &src.id, &SyncOptions::default()).unwrap();
    assert_eq!(r0.added, 1);

    // Sync A scans an older edit outside any write lock.
    write_vault_entry(
        &vault,
        "race2025",
        "citekey: race2025\nitemType: journalArticle\npublished: 2025\nauthors:\n  - Vega, M.\ntitle: Stale Intermediate Revision\nDOI: 10.9999/race.2025.1\npdf: paper.pdf",
    );
    let stale_prepared =
        prepare_source_sync(&lib, &src.id, &SyncOptions::default(), None).unwrap();

    // While Sync A is between scan and commit, another edit lands and Sync B commits first.
    write_vault_entry(
        &vault,
        "race2025",
        "citekey: race2025\nitemType: journalArticle\npublished: 2025\nauthors:\n  - Vega, M.\ntitle: Authoritative Latest Revision\nDOI: 10.9999/race.2025.1\npdf: paper.pdf",
    );
    let r_newer = sync_source(&lib, &src.id, &SyncOptions::default()).unwrap();
    assert_eq!(r_newer.updated, 1);
    let newer_sync_ms = lib
        .list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == src.id)
        .unwrap()
        .last_sync_ms
        .unwrap();

    // Sync A now attempts to commit its older scan -> must fail with conflict, preserving Sync B.
    let err = commit_source_sync(&lib, stale_prepared).unwrap_err();
    assert!(
        err.contains("concurrent sync conflict"),
        "unexpected error: {err}"
    );

    let key = PaperKey::parse("doi:10.9999/race.2025.1").unwrap();
    let detail = lib.get_paper(&key).unwrap().unwrap();
    assert_eq!(detail.paper.title, "Authoritative Latest Revision");
    assert_eq!(
        detail.source_items[0].title,
        "Authoritative Latest Revision"
    );
    let fts_stale = lib
        .search(&LibraryQuery {
            q: Some("Intermediate".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(fts_stale.is_empty());
    let fts_newer = lib
        .search(&LibraryQuery {
            q: Some("Authoritative".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(fts_newer.len(), 1);
    let live_sync_ms = lib
        .list_sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == src.id)
        .unwrap()
        .last_sync_ms
        .unwrap();
    assert_eq!(live_sync_ms, newer_sync_ms);

    // Prepare a scan, then unregister the source before commit -> commit must fail and NOT resurrect it.
    let pre_delete = prepare_source_sync(&lib, &src.id, &SyncOptions::default(), None).unwrap();
    assert!(remove_source(&lib, &src.id).unwrap());
    let err = commit_source_sync(&lib, pre_delete).unwrap_err();
    assert!(
        err.contains("was removed"),
        "expected removed-source error, got: {err}"
    );
    assert!(lib.list_sources().unwrap().is_empty());
    assert!(lib.list_source_items_for_source(&src.id).unwrap().is_empty());
}

#[test]
fn concurrent_threaded_syncs_and_db_writes_outside_scan_stay_consistent() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let folder = tmp.path().join("concurrent_papers");
    fs::create_dir_all(&folder).unwrap();

    for i in 1..=4 {
        fs::write(
            folder.join(format!("paper_{i}.pdf")),
            synth_pdf(&[
                &format!("Concurrent Synthetic Study {i}"),
                "N. Sato",
                "2025",
                &format!("DOI: 10.9999/conc.2025.{i:02}"),
            ]),
        )
        .unwrap();
    }

    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::Folder, &folder, json!({}), None).unwrap();
    let src_id = src.id.clone();

    // Prove no write lock is held during scan: another connection writes a user mark while
    // `prepare_source_sync` has scanned and not yet committed.
    let prepared = prepare_source_sync(&lib, &src_id, &SyncOptions::default(), None).unwrap();
    let other_conn = Library::open(&data_dir).unwrap();
    let mark_key = PaperKey::parse("doi:10.9999/conc.2025.01").unwrap();
    other_conn
        .set_user_mark(&mark_key, MarkStatus::Key, vec!["core".into()], "checked".into())
        .unwrap();
    commit_source_sync(&lib, prepared).unwrap();
    assert_eq!(
        other_conn.get_user_mark(&mark_key).unwrap().unwrap().status,
        MarkStatus::Key
    );

    // Two threads concurrently run `sync_source` on the exact same source via separate handles.
    let barrier = Barrier::new(2);
    let results: Vec<Result<_, String>> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let dir_ref = &data_dir;
            let id_ref = &src_id;
            let bar_ref = &barrier;
            handles.push(s.spawn(move || {
                let thread_lib = Library::open(dir_ref).unwrap();
                bar_ref.wait();
                sync_source(
                    &thread_lib,
                    id_ref,
                    &SyncOptions {
                        dry_run: false,
                        hash_files: true,
                    },
                )
            }));
        }
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    assert!(ok_count >= 1, "at least one concurrent sync must succeed: {results:?}");
    for res in &results {
        if let Err(e) = res {
            assert!(
                e.contains("concurrent sync conflict"),
                "non-winning thread must report a clean conflict, got: {e}"
            );
        }
    }

    // Index remains intact and a follow-up sync sees 4 unchanged items.
    let follow = sync_source(&lib, &src_id, &SyncOptions::default()).unwrap();
    assert_eq!(follow.total_items, 4);
    assert_eq!(follow.unchanged, 4);
    assert_eq!(follow.added, 0);
}

#[test]
fn overlapping_folder_and_vault_unregister_preserves_shared_paths_tex_and_retries() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault = tmp.path().join("vault");
    let shared = tmp.path().join("SharedPapers");
    let nested = shared.join("nested");
    fs::create_dir_all(vault.join(".kandinsky/plugin-settings")).unwrap();
    fs::create_dir_all(vault.join("refs/papers")).unwrap();
    fs::create_dir_all(&nested).unwrap();

    fs::write(
        vault.join(".kandinsky/plugin-settings/catalog-papers.toml"),
        "dir = \"refs/papers\"\nexternal_pdf_roots = [\"../SharedPapers\"]\narxiv_source_dir = \".kandinsky/cache/arxiv-src\"\n",
    )
    .unwrap();

    // Shared PDF indexed by both vault (via external_pdf_roots) and folder-shared.
    let shared_pdf = shared.join("shared_overlap.pdf");
    fs::write(
        &shared_pdf,
        synth_pdf(&[
            "Overlapping Shared Path Study",
            "M. Patel",
            "2025",
            "DOI: 10.9999/overlap.shared",
        ]),
    )
    .unwrap();
    write_vault_entry(
        &vault,
        "shared2025",
        "citekey: shared2025\nitemType: journalArticle\npublished: 2025\nauthors:\n  - Patel, M.\ntitle: Overlapping Shared Path Study\nDOI: 10.9999/overlap.shared\nexternal_pdf: \"[[../SharedPapers/shared_overlap.pdf]]\"",
    );

    // Paper present as unpacked TeX + PDF in vault AND as a separate PDF in folder-shared.
    let tex_dir = vault.join(".kandinsky/cache/arxiv-src/2503.00010v1");
    fs::create_dir_all(&tex_dir).unwrap();
    fs::write(
        tex_dir.join("main.tex"),
        "\\documentclass{article}\n\\begin{document}\nSynthetic overlap TeX body.\n\\end{document}\n",
    )
    .unwrap();
    write_vault_entry(
        &vault,
        "texdual2025",
        "citekey: texdual2025\nitemType: preprint\npublished: 2025\nauthors:\n  - Solis, D.\ntitle: Dual Source TeX Preference Study\narchiveID: 2503.00010v1\npdf: vault_copy.pdf",
    );
    fs::write(
        vault.join("refs/papers/texdual2025/vault_copy.pdf"),
        synth_pdf(&["Dual Source TeX Preference Study", "arXiv:2503.00010v1"]),
    )
    .unwrap();
    fs::write(
        shared.join("2503.00010v1.pdf"),
        synth_pdf(&["Dual Source TeX Preference Study", "arXiv:2503.00010v1"]),
    )
    .unwrap();

    // Nested PDF indexed by both folder-shared (parent) and folder-nested (child).
    fs::write(
        nested.join("nested_overlap.pdf"),
        synth_pdf(&[
            "Nested Folder Overlap Paper",
            "K. Okafor",
            "2025",
            "DOI: 10.9999/overlap.nested",
        ]),
    )
    .unwrap();

    let vault_hash_before = hash_tree(&vault);
    let shared_hash_before = hash_tree(&shared);

    let lib = Library::open(&data_dir).unwrap();
    let s_vault = register_source(
        &lib,
        SourceKind::KandinskyVault,
        &vault,
        json!({}),
        Some("vault-overlap".into()),
    )
    .unwrap();
    let s_shared = register_source(
        &lib,
        SourceKind::Folder,
        &shared,
        json!({}),
        Some("folder-shared".into()),
    )
    .unwrap();
    let s_nested = register_source(
        &lib,
        SourceKind::Folder,
        &nested,
        json!({}),
        Some("folder-nested".into()),
    )
    .unwrap();

    sync_source(&lib, &s_vault.id, &SyncOptions::default()).unwrap();
    sync_source(&lib, &s_shared.id, &SyncOptions::default()).unwrap();
    sync_source(&lib, &s_nested.id, &SyncOptions::default()).unwrap();

    // Syncing PDF-only folder-shared after vault-overlap must NOT downgrade TeX to PDF.
    let dual_key = PaperKey::parse("arxiv:2503.00010").unwrap();
    let dual_before = lib.get_paper(&dual_key).unwrap().unwrap();
    assert_eq!(dual_before.fulltext.len(), 1);
    assert_eq!(dual_before.fulltext[0].format, "tex");

    // 1. Unregister vault-overlap (and retry safely).
    assert!(remove_source(&lib, &s_vault.id).unwrap());
    assert!(!remove_source(&lib, &s_vault.id).unwrap());

    // Shared path fulltext is preserved because folder-shared still indexes it.
    let shared_key = PaperKey::parse("doi:10.9999/overlap.shared").unwrap();
    let shared_detail = lib.get_paper(&shared_key).unwrap().unwrap();
    assert_eq!(shared_detail.fulltext.len(), 1);
    assert_eq!(shared_detail.fulltext[0].origin, ORIGIN_USER_LIBRARY);
    assert!(shared_detail.fulltext[0]
        .file_path
        .as_deref()
        .unwrap()
        .ends_with("SharedPapers/shared_overlap.pdf"));

    // Dual paper falls back to surviving folder-shared PDF copy after vault TeX source is unregistered.
    let dual_after_vault = lib.get_paper(&dual_key).unwrap().unwrap();
    assert_eq!(dual_after_vault.fulltext.len(), 1);
    assert_eq!(dual_after_vault.fulltext[0].format, "pdf");
    assert!(dual_after_vault.fulltext[0]
        .file_path
        .as_deref()
        .unwrap()
        .ends_with("SharedPapers/2503.00010v1.pdf"));

    // Attaching the preserved shared paper into a session still succeeds.
    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let ingested = attach_source_paper_to_session(
        &lib,
        &sandbox,
        &ledger,
        "s-overlap-attach",
        &shared_key,
        None,
        None,
    )
    .unwrap();
    assert!(!ingested.files.is_empty());

    // 2. Unregister folder-shared: shared_overlap is now unreferenced (pruned), nested stays (folder-nested).
    assert!(remove_source(&lib, &s_shared.id).unwrap());
    assert!(lib
        .get_paper(&shared_key)
        .unwrap()
        .unwrap()
        .fulltext
        .is_empty());
    let nested_key = PaperKey::parse("doi:10.9999/overlap.nested").unwrap();
    let nested_detail = lib.get_paper(&nested_key).unwrap().unwrap();
    assert_eq!(nested_detail.fulltext.len(), 1);
    assert!(nested_detail.fulltext[0]
        .file_path
        .as_deref()
        .unwrap()
        .ends_with("SharedPapers/nested/nested_overlap.pdf"));

    // 3. Unregister folder-nested: now nested_overlap is pruned too.
    assert!(remove_source(&lib, &s_nested.id).unwrap());
    assert!(lib
        .get_paper(&nested_key)
        .unwrap()
        .unwrap()
        .fulltext
        .is_empty());

    // User directories stayed strictly read-only throughout.
    assert_eq!(hash_tree(&vault), vault_hash_before);
    assert_eq!(hash_tree(&shared), shared_hash_before);
}

#[test]
fn sync_and_unregister_preserve_fulltext_from_unrelated_origins() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let folder = tmp.path().join("local_pdfs");
    fs::create_dir_all(&folder).unwrap();

    // Seed F3 FulltextCache with unrelated origins (`arxiv_eprint` and `arxiv_pdf`).
    let eprint_bytes = b"\\documentclass{article}\\begin{document}Origin eprint\\end{document}";
    let pdf_bytes = synth_pdf(&["Origin Arxiv PDF", "arXiv:2504.00021v1"]);
    {
        let cache = FulltextCache::open(&data_dir).unwrap();
        cache
            .put_bytes("arxiv:2504.00020", Some("v1"), "tex", "arxiv_eprint", eprint_bytes)
            .unwrap();
        cache
            .put_bytes("arxiv:2504.00021", Some("v1"), "pdf", "arxiv_pdf", &pdf_bytes)
            .unwrap();
    }

    // Local folder contains PDFs for the same two arXiv papers plus one local-only paper.
    fs::write(
        folder.join("2504.00020v1.pdf"),
        synth_pdf(&["Local PDF Copy Of Eprint", "arXiv:2504.00020v1"]),
    )
    .unwrap();
    fs::write(
        folder.join("2504.00021v1.pdf"),
        synth_pdf(&["Local PDF Copy Of Arxiv PDF", "arXiv:2504.00021v1"]),
    )
    .unwrap();
    fs::write(
        folder.join("local_only.pdf"),
        synth_pdf(&["Local Only Paper", "DOI: 10.9999/local.only.1"]),
    )
    .unwrap();

    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::Folder, &folder, json!({}), None).unwrap();
    let rep = sync_source(&lib, &src.id, &SyncOptions::default()).unwrap();
    assert_eq!(rep.added, 3);

    // Neither unrelated origin row was overwritten by `user_library`.
    let cache = FulltextCache::open(&data_dir).unwrap();
    let (e1, b1) = cache
        .get_bytes("arxiv:2504.00020", Some("v1"))
        .unwrap()
        .expect("arxiv_eprint row must remain intact after folder sync");
    assert_eq!(e1.origin, "arxiv_eprint");
    assert_eq!(b1, eprint_bytes);

    let (e2, b2) = cache
        .get_bytes("arxiv:2504.00021", Some("v1"))
        .unwrap()
        .expect("arxiv_pdf row must remain intact after folder sync");
    assert_eq!(e2.origin, "arxiv_pdf");
    assert_eq!(b2, pdf_bytes);

    let local_key = PaperKey::parse("doi:10.9999/local.only.1").unwrap();
    assert_eq!(lib.get_paper(&local_key).unwrap().unwrap().fulltext.len(), 1);

    // Unregistering the folder removes local-only user_library fulltext and preserves unrelated origins.
    assert!(remove_source(&lib, &src.id).unwrap());
    assert!(lib
        .get_paper(&local_key)
        .unwrap()
        .unwrap()
        .fulltext
        .is_empty());
    let (_, b1_after) = cache
        .get_bytes("arxiv:2504.00020", Some("v1"))
        .unwrap()
        .expect("arxiv_eprint row must survive source unregister");
    assert_eq!(b1_after, eprint_bytes);
    let (_, b2_after) = cache
        .get_bytes("arxiv:2504.00021", Some("v1"))
        .unwrap()
        .expect("arxiv_pdf row must survive source unregister");
    assert_eq!(b2_after, pdf_bytes);
}
