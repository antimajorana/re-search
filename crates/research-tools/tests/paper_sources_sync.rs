//! Integration & property tests for package F13a (paper sources, sync, Kandinsky vault
//! adapter, plain folder adapter, 5-signal dedupe, path containment, and evaluation set B).
//!
//! All fixture text and synthetic PDFs are self-written; no third-party paper text is used.

mod pdf_support;

use ledger::Ledger;
use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::library::{Library, SourceItemState, SourceKind};
use research_tools::paper_key::PaperKey;
use research_tools::paper_sources::{
    attach_source_paper_to_session, resolve_contained_rel, sha256_hex, FILE_ROLE_TEX,
};
use research_tools::quote_core::verify_quote;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::source_dedupe::{DuplicateConfidence, DuplicateSignal};
use research_tools::source_sync::{register_source, remove_source, sync_source, SyncOptions};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

/// Deterministically hash an entire directory tree (relative paths, entry kinds, sizes, and
/// raw file contents; symlink targets are recorded without following).
fn hash_tree(root: &Path) -> String {
    let mut records: Vec<String> = Vec::new();
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
        let lmeta = fs::symlink_metadata(&p).unwrap();
        if lmeta.file_type().is_symlink() {
            let target = fs::read_link(&p).unwrap().to_string_lossy().into_owned();
            out.push(format!("L:{rel}->{target}"));
        } else if lmeta.is_dir() {
            out.push(format!("D:{rel}"));
            collect_tree(root, &p, out);
        } else {
            let bytes = fs::read(&p).unwrap();
            out.push(format!("F:{rel}:{}:{}", bytes.len(), sha256_hex(&bytes)));
        }
    }
}

fn synth_pdf(lines: &[&str]) -> Vec<u8> {
    build_pdf(&[PageSpec::single_column(lines)], &PdfBuildOptions::default())
}

fn write_vault_entry(vault: &Path, key: &str, frontmatter: &str, body: &str) {
    let dir = vault.join("refs/papers").join(key);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{key}.md")),
        format!("---\n{frontmatter}\n---\n{body}\n"),
    )
    .unwrap();
}

/// Build the fixed-tier evaluation set B Kandinsky vault (30 entries with planted cases):
/// - 22 normal entries
/// - 2 entries sharing a DOI (duplicate by DOI)
/// - 2 entries sharing identical PDF bytes (duplicate by file hash)
/// - 2 entries for the same paper as arXiv preprint (v2) + journal version
/// - 1 entry with an unpacked arXiv TeX source (preferred over PDF)
/// - 1 entry referencing a path that tries to escape the vault root (`../../../escape.pdf`)
fn build_eval_set_b_vault(root: &Path) {
    let vault = root.join("vault");
    let ext_pdfs = root.join("SharedPapers");
    fs::create_dir_all(vault.join(".kandinsky/plugin-settings")).unwrap();
    fs::create_dir_all(vault.join("refs/papers")).unwrap();
    fs::create_dir_all(&ext_pdfs).unwrap();

    fs::write(
        vault.join(".kandinsky/plugin-settings/catalog-papers.toml"),
        "dir = \"refs/papers\"\nexternal_pdf_roots = [\"../SharedPapers\"]\narxiv_source_dir = \".kandinsky/cache/arxiv-src\"\n",
    )
    .unwrap();

    // 22 distinct synthetic entries (entries 01..=22).
    for i in 1..=22 {
        let key = format!("synth2024p{i:02}");
        let pdf_name = format!("{key}.pdf");
        let fm = format!(
            "type: Literature\ncitekey: {key}\nitemType: journalArticle\npublished: 2024-01-{i:02}\nauthors:\n  - Author{i:02}, Pat\ntitle: Synthetic Lattice Study Number {i:02}\nDOI: 10.9999/evalb.2024.{i:03}\npdf: {pdf_name}\ncollections: [set-b]\n"
        );
        write_vault_entry(&vault, &key, &fm, &format!("# Study {i:02}\nSynthetic note."));
        let pdf = synth_pdf(&[
            &format!("Synthetic Lattice Study Number {i:02}"),
            &format!("Pat Author{i:02}"),
            &format!("DOI: 10.9999/evalb.2024.{i:03}"),
        ]);
        fs::write(vault.join("refs/papers").join(&key).join(&pdf_name), pdf).unwrap();
    }

    // 23 & 24: Duplicate by DOI (one in vault, one referencing external_pdf_roots).
    write_vault_entry(
        &vault,
        "dupdoi_a",
        "citekey: dupdoi_a\nitemType: journalArticle\npublished: 2023-05-01\nauthors:\n  - Vega, Mara\ntitle: Synthetic Gauge Anomaly Notes\nDOI: 10.9999/dup.doi.001\npdf: dupdoi_a.pdf",
        "Primary copy.",
    );
    fs::write(
        vault.join("refs/papers/dupdoi_a/dupdoi_a.pdf"),
        synth_pdf(&["Synthetic Gauge Anomaly Notes", "Mara Vega", "DOI: 10.9999/dup.doi.001"]),
    )
    .unwrap();
    fs::write(
        ext_pdfs.join("Vega2023.pdf"),
        synth_pdf(&["Synthetic Gauge Anomaly Notes", "Mara Vega", "10.9999/dup.doi.001"]),
    )
    .unwrap();
    write_vault_entry(
        &vault,
        "dupdoi_b",
        "citekey: dupdoi_b\nitemType: preprint\npublished: 2023-04-01\nauthors:\n  - Mara Vega\ntitle: Synthetic Gauge Anomaly Notes\nDOI: https://doi.org/10.9999/DUP.DOI.001\nexternal_pdf: \"[[../SharedPapers/Vega2023.pdf]]\"",
        "Duplicate DOI copy.",
    );

    // 25 & 26: Duplicate by identical PDF SHA-256 (no DOI/arXiv).
    let shared_bytes = synth_pdf(&[
        "Unindexed Local Lecture Notes on Winding Numbers",
        "N. Takahashi",
        "Self-written synthetic bytes for hash duplicate testing.",
    ]);
    let shared_sha = sha256_hex(&shared_bytes);
    write_vault_entry(
        &vault,
        "duphash_a",
        &format!(
            "citekey: duphash_a\nitemType: journalArticle\npublished: 2022\nauthors:\n  - Takahashi, N.\ntitle: Unindexed Local Lecture Notes on Winding Numbers\npdf: copy_one.pdf\npdfSha256: {shared_sha}"
        ),
        "Hash copy A.",
    );
    fs::write(vault.join("refs/papers/duphash_a/copy_one.pdf"), &shared_bytes).unwrap();
    write_vault_entry(
        &vault,
        "duphash_b",
        &format!(
            "citekey: duphash_b\nitemType: journalArticle\npublished: 2022\nauthors:\n  - Takahashi, N.\ntitle: Winding Numbers Lecture Notes (Local Copy)\npdf: copy_two.pdf\npdfSha256: {shared_sha}"
        ),
        "Hash copy B.",
    );
    fs::write(vault.join("refs/papers/duphash_b/copy_two.pdf"), &shared_bytes).unwrap();

    // 27 & 28: Same paper as preprint (arXiv v2) and published journal version.
    write_vault_entry(
        &vault,
        "preprint_pair",
        "citekey: preprint_pair\nitemType: preprint\npublished: 2024-02-10\nauthors:\n  - Lin, Mei\ntitle: Synthetic Boundary Modes in Finite Chains\narchiveID: arXiv:2402.09901v2\nDOI: 10.48550/arXiv.2402.09901\npdf: pre.pdf",
        "Preprint entry.",
    );
    fs::write(
        vault.join("refs/papers/preprint_pair/pre.pdf"),
        synth_pdf(&["Synthetic Boundary Modes", "arXiv:2402.09901v2"]),
    )
    .unwrap();
    write_vault_entry(
        &vault,
        "journal_pair",
        "citekey: journal_pair\nitemType: journalArticle\npublished: 2024-09-01\nauthors:\n  - Lin, Mei\ntitle: Synthetic Boundary Modes in Finite Chains\narchiveID: 2402.09901\nDOI: 10.9999/physrev.2024.9901\npdf: pub.pdf",
        "Published entry.",
    );
    fs::write(
        vault.join("refs/papers/journal_pair/pub.pdf"),
        synth_pdf(&["Synthetic Boundary Modes", "DOI: 10.9999/physrev.2024.9901"]),
    )
    .unwrap();

    // 29: Entry with unpacked arXiv TeX source (TeX must be preferred over PDF).
    write_vault_entry(
        &vault,
        "texpreferred2024",
        "citekey: texpreferred2024\nitemType: preprint\npublished: 2024-03-15\nauthors:\n  - Solis, D.\ntitle: Exact Sum Rule From TeX Source\narchiveID: 2403.07777v1\npdf: paper.pdf",
        "Has TeX unpacked.",
    );
    fs::write(
        vault.join("refs/papers/texpreferred2024/paper.pdf"),
        synth_pdf(&["Exact Sum Rule From TeX Source", "arXiv:2403.07777v1"]),
    )
    .unwrap();
    let tex_dir = vault.join(".kandinsky/cache/arxiv-src/2403.07777v1");
    fs::create_dir_all(&tex_dir).unwrap();
    fs::write(
        tex_dir.join("main.tex"),
        "\\documentclass{article}\n\\title{Exact Sum Rule From TeX Source}\n\\begin{document}\nSynthetic theorem statement: the spectral sum equals 7/12.\n\\end{document}\n",
    )
    .unwrap();

    // 30: Entry with a path attempting to escape the vault root via `..`.
    fs::write(root.join("outside_secret.pdf"), b"%PDF-1.4\noutside\n").unwrap();
    write_vault_entry(
        &vault,
        "escape_attempt",
        "citekey: escape_attempt\nitemType: journalArticle\npublished: 2024\nauthors:\n  - Tester, A.\ntitle: Escaping Path Entry\nDOI: 10.9999/escape.001\npdf: ../../../outside_secret.pdf",
        "Should refuse escaping PDF.",
    );
}

#[test]
fn property_readonly_eval_set_b_vault_sync_idempotency_edits_and_deletes() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    build_eval_set_b_vault(tmp.path());
    let vault = tmp.path().join("vault");
    let ext_pdfs = tmp.path().join("SharedPapers");

    let tree_before_vault = hash_tree(&vault);
    let tree_before_ext = hash_tree(&ext_pdfs);

    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::KandinskyVault, &vault, json!({}), None).unwrap();

    // First sync dry-run writes nothing to library.db.
    let preview = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: true,
            hash_files: true,
        },
    )
    .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.total_items, 30);
    assert!(lib.list_source_items_for_source(&src.id).unwrap().is_empty());

    // Real sync.
    let r1 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: true,
        },
    )
    .unwrap();
    assert_eq!(r1.total_items, 30);
    assert_eq!(r1.added, 30);
    assert_eq!(r1.updated, 0);
    assert_eq!(r1.missing, 0);
    assert_eq!(r1.with_tex, 1);
    // 3 exact duplicate groups: DOI pair, SHA-256 pair, preprint+journal pair.
    assert_eq!(r1.exact_duplicates, 3);
    // Escaping path warning recorded and file NOT attached.
    assert!(
        r1.warnings
            .iter()
            .any(|w| w.path.contains("escape_attempt") && w.reason.contains("escapes")),
        "warnings: {:?}",
        r1.warnings
    );
    let escape_item = r1
        .items
        .iter()
        .find(|i| i.external_id == "escape_attempt")
        .unwrap();
    assert!(
        escape_item.files.iter().all(|f| f.role != "pdf"),
        "escaping PDF must never be indexed"
    );

    // Verify exact duplicate merges registered aliases in F10 Library.
    let preprint_key = PaperKey::parse("arxiv:2402.09901").unwrap();
    let journal_doi_key = PaperKey::parse("doi:10.9999/physrev.2024.9901").unwrap();
    assert_eq!(
        lib.resolve_key(&journal_doi_key).unwrap(),
        Some(preprint_key.clone())
    );
    let detail = lib.get_paper(&preprint_key).unwrap().unwrap();
    assert!(detail.paper.flags.owned);
    assert_eq!(
        detail.paper.ids.doi.as_deref(),
        Some("10.9999/physrev.2024.9901")
    );

    // Second sync immediately after changes nothing.
    let r2 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r2.added, 0);
    assert_eq!(r2.updated, 0);
    assert_eq!(r2.moved, 0);
    assert_eq!(r2.missing, 0);
    assert_eq!(r2.unchanged, 30);

    // Property: entire vault and external PDF tree are byte-for-byte identical after dry-run + 2 syncs.
    assert_eq!(hash_tree(&vault), tree_before_vault);
    assert_eq!(hash_tree(&ext_pdfs), tree_before_ext);

    // Planted case: edited note updates metadata on next sync.
    let edited_note = vault.join("refs/papers/synth2024p01/synth2024p01.md");
    let new_content = fs::read_to_string(&edited_note)
        .unwrap()
        .replace("Synthetic Lattice Study Number 01", "Edited Lattice Title 01");
    fs::write(&edited_note, new_content).unwrap();

    // Planted case: deleted entry becomes `missing` (stays listed).
    fs::remove_dir_all(vault.join("refs/papers/synth2024p02")).unwrap();

    let r3 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r3.updated, 1);
    assert_eq!(r3.missing, 1);
    let edited_paper = lib
        .get_paper(&PaperKey::parse("doi:10.9999/evalb.2024.001").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(edited_paper.paper.title, "Edited Lattice Title 01");
    let missing_item = r3
        .items
        .iter()
        .find(|i| i.external_id == "synth2024p02")
        .unwrap();
    assert_eq!(missing_item.state, SourceItemState::Missing);

    // Another sync after delete changes nothing (missing stays missing, counted unchanged).
    let r4 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r4.added, 0);
    assert_eq!(r4.updated, 0);
    assert_eq!(r4.missing, 0);
    assert_eq!(r4.unchanged, 30);
}

#[test]
fn folder_adapter_moved_file_fuzzy_duplicate_and_readonly_property() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let folder = tmp.path().join("papers_folder");
    fs::create_dir_all(folder.join("quantum/chains")).unwrap();
    fs::create_dir_all(folder.join("archive")).unwrap();

    let movable_pdf = synth_pdf(&[
        "Movable Synthetic Paper on Discrete Curvature",
        "Elena Voss",
        "2024",
        "arXiv:2404.01111v1",
    ]);
    fs::write(folder.join("quantum/chains/movable.pdf"), &movable_pdf).unwrap();

    // Fuzzy possible duplicate (no shared DOI/arXiv/hash, same title + first author surname + year ± 1).
    fs::write(
        folder.join("quantum/chains/fuzzy_a.pdf"),
        synth_pdf(&[
            "Exact Partition Sums on Finite Graphs",
            "Carla Mendez",
            "Date: May 2022",
            "Abstract: Draft version A with distinct bytes for fuzzy matching.",
        ]),
    )
    .unwrap();
    fs::write(
        folder.join("archive/fuzzy_b.pdf"),
        synth_pdf(&[
            "Exact Partition Sums on Finite Graphs",
            "Carla Mendez",
            "Published: June 2023",
            "Abstract: Revised version B with different bytes for fuzzy matching.",
        ]),
    )
    .unwrap();

    let tree_before = hash_tree(&folder);
    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::Folder, &folder, json!({}), None).unwrap();

    // Sync twice with hash_files: true -> read-only and idempotent.
    let r1 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: true,
        },
    )
    .unwrap();
    assert_eq!(r1.total_items, 3);
    assert_eq!(r1.added, 3);
    assert_eq!(r1.possible_duplicates, 1);
    assert_eq!(r1.exact_duplicates, 0);
    assert_eq!(
        r1.duplicate_groups[0].confidence,
        DuplicateConfidence::Possible
    );
    assert_eq!(
        r1.duplicate_groups[0].signals,
        vec![DuplicateSignal::TitleAuthorYear]
    );
    let fuzzy_items: Vec<_> = r1
        .items
        .iter()
        .filter(|i| i.state == SourceItemState::PossibleDuplicate)
        .collect();
    assert_eq!(fuzzy_items.len(), 2, "fuzzy duplicates are flagged, never merged");
    assert_ne!(fuzzy_items[0].key, fuzzy_items[1].key);

    let r2 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r2.added, 0);
    assert_eq!(r2.updated, 0);
    assert_eq!(r2.moved, 0);
    assert_eq!(r2.missing, 0);
    assert_eq!(r2.unchanged, 3);
    assert_eq!(hash_tree(&folder), tree_before);

    // Planted case: move file to a new subfolder -> same item kept, moved == 1, missing == 0.
    fs::rename(
        folder.join("quantum/chains/movable.pdf"),
        folder.join("archive/movable_renamed.pdf"),
    )
    .unwrap();
    let r3 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r3.moved, 1);
    assert_eq!(r3.added, 0);
    assert_eq!(r3.missing, 0);
    let moved_item = r3
        .items
        .iter()
        .find(|i| i.external_id == "quantum/chains/movable.pdf")
        .expect("moved file keeps its original item identity");
    assert_eq!(moved_item.state, SourceItemState::Present);
    assert!(moved_item.files[0].path.ends_with("archive/movable_renamed.pdf"));
    assert_eq!(moved_item.collections, vec!["archive".to_string()]);

    // Subsequent sync after move is idempotent with zero reads.
    let r4 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r4.moved, 0);
    assert_eq!(r4.added, 0);
    assert_eq!(r4.updated, 0);
    assert_eq!(r4.missing, 0);
    assert_eq!(r4.unchanged, 3);
}

#[test]
fn path_containment_refuses_dotdot_and_symlinks_leaving_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("registered");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.pdf");
    fs::write(&secret, synth_pdf(&["Secret Outside Document"])).unwrap();

    let canon_root = fs::canonicalize(&root).unwrap();
    assert!(resolve_contained_rel(&canon_root, "../outside/secret.pdf").is_err());
    assert!(resolve_contained_rel(&canon_root, "sub/../../outside/secret.pdf").is_err());

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, root.join("link_escape.pdf")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dir_escape")).unwrap();
        assert!(resolve_contained_rel(&canon_root, "link_escape.pdf").is_err());
        assert!(resolve_contained_rel(&canon_root, "dir_escape/secret.pdf").is_err());

        let data_dir = tmp.path().join("data");
        let lib = Library::open(&data_dir).unwrap();
        let src = register_source(&lib, SourceKind::Folder, &root, json!({}), None).unwrap();
        let report = sync_source(&lib, &src.id, &SyncOptions::default()).unwrap();
        assert_eq!(report.total_items, 0, "symlinks must never be indexed");
        assert_eq!(report.warnings.len(), 2);
        assert!(report
            .warnings
            .iter()
            .all(|w| w.reason.contains("symlink")));
    }
}

#[test]
fn removing_source_and_deleting_library_db_never_breaks_session_quote_checks() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    build_eval_set_b_vault(tmp.path());
    let vault = tmp.path().join("vault");

    let lib = Library::open(&data_dir).unwrap();
    let src = register_source(&lib, SourceKind::KandinskyVault, &vault, json!({}), None).unwrap();
    let report = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: true,
        },
    )
    .unwrap();
    let tex_item = report
        .items
        .iter()
        .find(|i| i.external_id == "texpreferred2024")
        .unwrap();
    assert!(tex_item.files.iter().any(|f| f.role == FILE_ROLE_TEX));

    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let session_id = "s-f13a-quote";
    let key = PaperKey::parse("arxiv:2403.07777").unwrap();
    let ingested =
        attach_source_paper_to_session(&lib, &sandbox, &ledger, session_id, &key, Some("v1"), None)
            .unwrap();
    // Preferred TeX source was ingested, not PDF.
    assert!(
        ingested.files.iter().any(|f| f.ends_with(".tex")),
        "expected TeX source to be preferred: {:?}",
        ingested.files
    );
    let rel_path = ingested.files[0].clone();

    // Remove the source from the library, delete library.db, AND delete the source file on disk!
    assert!(remove_source(&lib, &src.id).unwrap());
    let db_path = lib.db_path();
    drop(lib);
    fs::remove_file(&db_path).unwrap();
    fs::remove_dir_all(&vault).unwrap();

    // Session ledger verification and quote verification still pass 100%.
    let events = ledger.lock().unwrap().read_session(session_id).unwrap();
    let verified = verify_quote(
        &sandbox,
        session_id,
        &events,
        &ingested.source_id,
        &rel_path,
        4,
        4,
        "Synthetic theorem statement: the spectral sum equals 7/12.",
    )
    .expect("quote check must still succeed after removing source and library.db");
    assert_eq!(verified.quote_ref.start_line, 4);
    assert!(ledger.lock().unwrap().verify_session(session_id).is_ok());
}
