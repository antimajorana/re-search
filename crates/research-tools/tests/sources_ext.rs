//! Integration & property tests for package F13b (BibTeX and Zotero read-only paper source
//! adapters, Set B planted cases, byte-identical immutability, evidence independence C9, and
//! path containment).
//!
//! All `.bib` entries, synthetic SQLite databases, and synthetic PDFs are self-authored.

mod pdf_support;

use ledger::Ledger;
use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::library::{Library, SourceItemState, SourceKind};
use research_tools::paper_key::PaperKey;
use research_tools::paper_sources::{attach_source_paper_to_session, sha256_hex};
use research_tools::quote_core::verify_quote;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::source_sync::{
    preview_source, register_source, remove_source, sync_source, SyncOptions,
};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

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

#[test]
fn set_b_bibtex_sync_planted_cases_immutability_and_evidence_independence() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let bib_dir = tmp.path().join("bib_source");
    let ext_dir = tmp.path().join("ext_pdfs");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(bib_dir.join("pdfs/old")).unwrap();
    fs::create_dir_all(bib_dir.join("pdfs/moved")).unwrap();
    fs::create_dir_all(&ext_dir).unwrap();
    fs::create_dir_all(&outside).unwrap();

    // Self-made synthetic PDFs.
    let quote_line = "Theorem 3.1: Synthetic curvature trace vanishes identically.";
    let main_pdf = synth_pdf(&[
        "Synthetic Curvature Trace Notes",
        "Ada Synth",
        quote_line,
    ]);
    let movable_pdf = synth_pdf(&["Movable BibTeX Paper", "Ben Mover", "Unique movable text 11"]);
    let deletable_pdf = synth_pdf(&["Deletable BibTeX Paper", "Cara Gone", "Ephemeral text 22"]);
    let dup_hash_pdf = synth_pdf(&["Shared Bytes Paper", "Dan Hash", "Identical hash bytes 33"]);
    let preprint_pdf = synth_pdf(&["Spectral Gap Preprint", "Eva Gap", "arXiv:2406.01234v1"]);
    let journal_pdf = synth_pdf(&["Spectral Gap Journal", "Eva Gap", "DOI: 10.9999/gap.2024.1"]);
    let secret_pdf = synth_pdf(&["Secret Outside File", "Eve Outside"]);

    fs::write(bib_dir.join("pdfs/main.pdf"), &main_pdf).unwrap();
    fs::write(bib_dir.join("pdfs/old/movable.pdf"), &movable_pdf).unwrap();
    fs::write(bib_dir.join("pdfs/deletable.pdf"), &deletable_pdf).unwrap();
    fs::write(bib_dir.join("pdfs/dup_hash_a.pdf"), &dup_hash_pdf).unwrap();
    fs::write(ext_dir.join("dup_hash_b.pdf"), &dup_hash_pdf).unwrap();
    fs::write(bib_dir.join("pdfs/preprint.pdf"), &preprint_pdf).unwrap();
    fs::write(bib_dir.join("pdfs/journal.pdf"), &journal_pdf).unwrap();
    fs::write(outside.join("secret.pdf"), &secret_pdf).unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.join("secret.pdf"),
            bib_dir.join("pdfs/escape_symlink.pdf"),
        )
        .unwrap();
    }

    let bib_path = bib_dir.join("refs.bib");
    let initial_bib = r#"
@article{synth2024main,
  author = {Synth, Ada},
  title = {Synthetic Curvature Trace Notes},
  year = {2024},
  doi = {10.9999/bib.main.001},
  file = {Full Text PDF:pdfs/main.pdf:application/pdf}
}
@article{move2024,
  author = {Mover, Ben},
  title = {Movable BibTeX Paper},
  year = {2024},
  file = {:pdfs/old/movable.pdf:PDF}
}
@article{del2024,
  author = {Gone, Cara},
  title = {Deletable BibTeX Paper},
  year = {2024},
  doi = {10.9999/bib.del.002},
  file = {:pdfs/deletable.pdf:PDF}
}
@article{dupdoi_a,
  author = {Doi, Alex},
  title = {Duplicate DOI Entry A},
  year = {2023},
  doi = {10.9999/bib.dup.100}
}
@article{dupdoi_b,
  author = {Doi, Alex},
  title = {Duplicate DOI Entry B},
  year = {2023},
  doi = {https://doi.org/10.9999/BIB.DUP.100}
}
@misc{duphash_a,
  author = {Hash, Dan},
  title = {Shared Bytes Copy A},
  year = {2022},
  file = {pdfs/dup_hash_a.pdf}
}
@misc{duphash_b,
  author = {Hash, Dan},
  title = {Shared Bytes Copy B},
  year = {2022},
  file = {../ext_pdfs/dup_hash_b.pdf}
}
@misc{gap2024preprint,
  author = {Gap, Eva},
  title = {Spectral Gap Preprint},
  year = {2024},
  eprint = {2406.01234v1},
  archivePrefix = {arXiv},
  file = {:pdfs/preprint.pdf:PDF}
}
@article{gap2024journal,
  author = {Gap, Eva},
  title = {Spectral Gap Journal},
  year = {2024},
  eprint = {2406.01234v2},
  archivePrefix = {arXiv},
  doi = {10.9999/gap.2024.1},
  file = {:pdfs/journal.pdf:PDF}
}
@article{escape_dotdot,
  author = {Escape, Eve},
  title = {Dotdot Escape},
  year = {2024},
  file = {:../outside/secret.pdf:PDF}
}
@article{escape_sym,
  author = {Escape, Sam},
  title = {Symlink Escape},
  year = {2024},
  file = {:pdfs/escape_symlink.pdf:PDF}
}
"#;
    fs::write(&bib_path, initial_bib).unwrap();

    let tree_before_bib = hash_tree(&bib_dir);
    let tree_before_ext = hash_tree(&ext_dir);

    let lib = Library::open(&data_dir).unwrap();
    let settings = json!({ "external_pdf_roots": ["../ext_pdfs"] });

    // Dry-run preview first writes nothing to library.db.
    let (_cand, preview) =
        preview_source(&lib, SourceKind::Bibtex, &bib_path, settings.clone(), None, true).unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.total_items, 11);
    assert!(lib.list_sources().unwrap().is_empty());

    let src = register_source(&lib, SourceKind::Bibtex, &bib_path, settings, None).unwrap();
    let r1 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: true,
        },
    )
    .unwrap();
    assert_eq!(r1.total_items, 11);
    assert_eq!(r1.added, 11);
    // 3 exact duplicate groups: DOI pair, SHA-256 pair, preprint+journal pair.
    assert_eq!(r1.exact_duplicates, 3);
    // Escapes refused.
    assert!(r1
        .items
        .iter()
        .find(|i| i.external_id == "escape_dotdot")
        .unwrap()
        .files
        .is_empty());
    assert!(r1
        .items
        .iter()
        .find(|i| i.external_id == "escape_sym")
        .unwrap()
        .files
        .is_empty());

    // Preprint + journal merged under arXiv canonical key with DOI aliased.
    let gap_arxiv = PaperKey::parse("arxiv:2406.01234").unwrap();
    let gap_doi = PaperKey::parse("doi:10.9999/gap.2024.1").unwrap();
    assert_eq!(lib.resolve_key(&gap_doi).unwrap(), Some(gap_arxiv));

    // Second consecutive sync changes zero index rows and leaves tree byte-identical.
    let r2 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!((r2.added, r2.updated, r2.moved, r2.missing, r2.unchanged), (0, 0, 0, 0, 11));
    assert_eq!(hash_tree(&bib_dir), tree_before_bib);
    assert_eq!(hash_tree(&ext_dir), tree_before_ext);

    // Evidence independence (C9): attach paper to session via ingest_bytes, verify quote, then
    // remove source and delete library.db; quote check must still pass.
    let main_key = PaperKey::parse("doi:10.9999/bib.main.001").unwrap();
    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(data_dir.clone(), None);
    let ingested = attach_source_paper_to_session(
        &lib,
        &sandbox,
        &ledger,
        "sess-bib-c9",
        &main_key,
        None,
        None,
    )
    .unwrap();
    let events_before = ledger.lock().unwrap().read_session("sess-bib-c9").unwrap();
    let text_path = ingested
        .files
        .iter()
        .find(|f| f.ends_with(".txt"))
        .unwrap_or(&ingested.files[0])
        .clone();
    let text_file = data_dir
        .join("sessions")
        .join("sess-bib-c9")
        .join("sources")
        .join(&text_path);
    let text_body = fs::read_to_string(&text_file).unwrap();
    let (line_no, matched_line) = text_body
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("Synthetic curvature trace vanishes"))
        .map(|(idx, l)| (idx + 1, l.trim().to_string()))
        .expect("quote line must exist in extracted PDF text");

    let q1 = verify_quote(
        &sandbox,
        "sess-bib-c9",
        &events_before,
        &ingested.source_id,
        &text_path,
        line_no,
        line_no,
        &matched_line,
    )
    .unwrap();
    assert!(!q1.excerpt.is_empty());

    // Now apply Set B mutations: moved PDF (recognized by SHA-256), edited .bib entry, deleted PDF.
    fs::rename(
        bib_dir.join("pdfs/old/movable.pdf"),
        bib_dir.join("pdfs/moved/movable_relocated.pdf"),
    )
    .unwrap();
    fs::remove_file(bib_dir.join("pdfs/deletable.pdf")).unwrap();
    let edited_bib = initial_bib
        .replace(
            "title = {Synthetic Curvature Trace Notes}",
            "title = {Edited Curvature Trace Notes}",
        )
        .replace("pdfs/old/movable.pdf", "pdfs/moved/movable_relocated.pdf");
    fs::write(&bib_path, edited_bib).unwrap();

    let r3 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(r3.moved, 1, "moved PDF must be recognized by SHA-256: {r3:?}");
    assert_eq!(r3.updated, 1, "edited .bib entry must update metadata: {r3:?}");
    assert_eq!(r3.missing, 1, "deleted PDF must mark item missing: {r3:?}");
    let del_item = r3.items.iter().find(|i| i.external_id == "del2024").unwrap();
    assert_eq!(del_item.state, SourceItemState::Missing);
    let edited_paper = lib.get_paper(&main_key).unwrap().unwrap();
    assert_eq!(edited_paper.paper.title, "Edited Curvature Trace Notes");

    // Re-sync after mutations is idempotent (0 changes).
    let r4 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!((r4.added, r4.updated, r4.moved, r4.missing, r4.unchanged), (0, 0, 0, 0, 11));

    // Remove source and delete library.db -> C9 quote check still passes.
    assert!(remove_source(&lib, &src.id).unwrap());
    drop(lib);
    fs::remove_dir_all(data_dir.join("library")).unwrap();
    let events_after = ledger.lock().unwrap().read_session("sess-bib-c9").unwrap();
    let q2 = verify_quote(
        &sandbox,
        "sess-bib-c9",
        &events_after,
        &ingested.source_id,
        &text_path,
        line_no,
        line_no,
        &matched_line,
    )
    .unwrap();
    assert_eq!(q1.quote_ref.quote_sha256, q2.quote_ref.quote_sha256);
}

#[test]
fn set_b_zotero_sync_storage_linked_bbt_locked_immutability_and_containment() {
    use ledger::zotero_fixture::{write_synthetic_zotero_dir, ZoteroExclusiveLock};
    use ledger::zotero_reader::{ZoteroAttachmentRow, ZoteroItemRow};
    use std::collections::BTreeMap;

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let zot_dir = tmp.path().join("ZoteroData");
    let linked_dir = tmp.path().join("LinkedPdfs");
    let outside = tmp.path().join("OutsideSecret");
    fs::create_dir_all(zot_dir.join("storage/ATT001")).unwrap();
    fs::create_dir_all(zot_dir.join("storage/ATT004")).unwrap();
    fs::create_dir_all(linked_dir.join("Quantum")).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let quote_line = "Lemma 2.2: Read-only Zotero snapshot preserves byte identity.";
    let storage_pdf = synth_pdf(&["Zotero Stored Synthetic Paper", "Ada Zotero", quote_line]);
    let linked_pdf = synth_pdf(&["Zotero Linked Synthetic Paper", "Ben Linked"]);
    let outside_pdf = synth_pdf(&["Outside Secret PDF", "Eve Outside"]);

    fs::write(zot_dir.join("storage/ATT001/stored.pdf"), &storage_pdf).unwrap();
    fs::write(linked_dir.join("Quantum/linked.pdf"), &linked_pdf).unwrap();
    fs::write(outside.join("secret.pdf"), &outside_pdf).unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.join("secret.pdf"),
            zot_dir.join("storage/ATT004/link_out.pdf"),
        )
        .unwrap();
    }

    let item1 = ZoteroItemRow {
        item_id: 10,
        zotero_key: "ZKEY001".into(),
        item_type: "journalArticle".into(),
        date_added: Some("2026-01-01".into()),
        fields: BTreeMap::from([
            ("title".into(), "Zotero Stored Synthetic Paper".into()),
            ("date".into(), "2024-02-10".into()),
            ("DOI".into(), "10.9999/zot.stored.001".into()),
            ("extra".into(), "Citation Key: zot2024pinned".into()),
            ("publicationTitle".into(), "Synthetic Letters".into()),
        ]),
        creators: BTreeMap::from([("author".into(), vec!["Zotero, Ada".into()])]),
        collections: vec!["Physics/Lattice".into()],
        tags: vec!["zotero-tag".into()],
        attachments: vec![ZoteroAttachmentRow {
            attachment_item_id: 100,
            attachment_key: "ATT001".into(),
            content_type: "application/pdf".into(),
            raw_path: "storage:stored.pdf".into(),
        }],
        bbt_citekey: Some("ignored_lower_priority".into()),
    };

    let item2 = ZoteroItemRow {
        item_id: 11,
        zotero_key: "ZKEY002".into(),
        item_type: "preprint".into(),
        date_added: Some("2026-01-02".into()),
        fields: BTreeMap::from([
            ("title".into(), "Zotero Linked Synthetic Paper".into()),
            ("date".into(), "2023".into()),
            ("url".into(), "https://arxiv.org/abs/2311.09999v2".into()),
        ]),
        creators: BTreeMap::from([("author".into(), vec!["Linked, Ben".into()])]),
        collections: vec!["Quantum".into()],
        tags: vec![],
        attachments: vec![ZoteroAttachmentRow {
            attachment_item_id: 101,
            attachment_key: "ATT002".into(),
            content_type: "application/pdf".into(),
            raw_path: "attachments:Quantum/linked.pdf".into(),
        }],
        bbt_citekey: Some("linked2023bbt".into()),
    };

    let item_escape = ZoteroItemRow {
        item_id: 12,
        zotero_key: "ZKEYESC".into(),
        item_type: "journalArticle".into(),
        date_added: None,
        fields: BTreeMap::from([("title".into(), "Escaping Zotero Attachment".into())]),
        creators: BTreeMap::new(),
        collections: vec![],
        tags: vec![],
        attachments: vec![
            ZoteroAttachmentRow {
                attachment_item_id: 102,
                attachment_key: "ATT003".into(),
                content_type: "application/pdf".into(),
                raw_path: "attachments:../OutsideSecret/secret.pdf".into(),
            },
            ZoteroAttachmentRow {
                attachment_item_id: 103,
                attachment_key: "ATT004".into(),
                content_type: "application/pdf".into(),
                raw_path: "storage:link_out.pdf".into(),
            },
        ],
        bbt_citekey: None,
    };

    let item_deleted = ZoteroItemRow {
        item_id: 13,
        zotero_key: "ZKEYDEL".into(),
        item_type: "journalArticle".into(),
        date_added: None,
        fields: BTreeMap::from([("title".into(), "Trashed Item".into())]),
        creators: BTreeMap::new(),
        collections: vec![],
        tags: vec![],
        attachments: vec![],
        bbt_citekey: None,
    };

    write_synthetic_zotero_dir(&zot_dir, &[item1, item2, item_escape, item_deleted], &[13])
        .unwrap();

    let tree_before_zot = hash_tree(&zot_dir);
    let tree_before_linked = hash_tree(&linked_dir);

    let lib = Library::open(&data_dir).unwrap();
    let settings = json!({ "base_attachment_path": linked_dir.to_string_lossy() });
    let src = register_source(&lib, SourceKind::Zotero, &zot_dir, settings, None).unwrap();

    // Hold an exclusive lock on zotero.sqlite during the first sync to exercise snapshot fallback!
    let lock = ZoteroExclusiveLock::acquire(&zot_dir.join("zotero.sqlite")).unwrap();
    let r1 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: true,
        },
    )
    .unwrap();
    drop(lock);

    assert_eq!(r1.total_items, 3, "deleted item must be skipped");
    assert_eq!(r1.added, 3);
    assert_eq!(r1.with_pdf, 2);

    let stored = r1.items.iter().find(|i| i.external_id == "ZKEY001").unwrap();
    assert_eq!(stored.citekey.as_deref(), Some("zot2024pinned"));
    assert_eq!(stored.collections, vec!["Physics/Lattice".to_string()]);
    assert_eq!(stored.files.len(), 1);

    let linked = r1.items.iter().find(|i| i.external_id == "ZKEY002").unwrap();
    assert_eq!(linked.citekey.as_deref(), Some("linked2023bbt"));
    assert_eq!(linked.version.as_deref(), Some("v2"));
    assert_eq!(
        linked.key.as_ref().map(|k| k.as_str()),
        Some("arxiv:2311.09999")
    );
    assert_eq!(linked.files.len(), 1);

    let esc = r1.items.iter().find(|i| i.external_id == "ZKEYESC").unwrap();
    assert!(esc.files.is_empty(), "escaping and symlinked attachments must be refused");

    // Second sync changes zero rows and source trees remain byte-identical.
    let r2 = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!((r2.added, r2.updated, r2.moved, r2.missing, r2.unchanged), (0, 0, 0, 0, 3));
    assert_eq!(hash_tree(&zot_dir), tree_before_zot);
    assert_eq!(hash_tree(&linked_dir), tree_before_linked);

    // Evidence independence (C9) for Zotero-attached paper.
    let z_key = PaperKey::parse("doi:10.9999/zot.stored.001").unwrap();
    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(data_dir.clone(), None);
    let ingested = attach_source_paper_to_session(
        &lib,
        &sandbox,
        &ledger,
        "sess-zot-c9",
        &z_key,
        None,
        None,
    )
    .unwrap();
    assert!(remove_source(&lib, &src.id).unwrap());
    drop(lib);
    fs::remove_dir_all(data_dir.join("library")).unwrap();

    let text_path = ingested
        .files
        .iter()
        .find(|f| f.ends_with(".txt"))
        .unwrap_or(&ingested.files[0])
        .clone();
    let text_file = data_dir
        .join("sessions")
        .join("sess-zot-c9")
        .join("sources")
        .join(&text_path);
    let text_body = fs::read_to_string(&text_file).unwrap();
    let (line_no, matched_line) = text_body
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("Read-only Zotero snapshot preserves byte identity"))
        .map(|(idx, l)| (idx + 1, l.trim().to_string()))
        .expect("quote line must exist in extracted Zotero PDF text");
    let events = ledger.lock().unwrap().read_session("sess-zot-c9").unwrap();
    verify_quote(
        &sandbox,
        "sess-zot-c9",
        &events,
        &ingested.source_id,
        &text_path,
        line_no,
        line_no,
        &matched_line,
    )
    .unwrap();
}
