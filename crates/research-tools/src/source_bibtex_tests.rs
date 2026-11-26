//! Unit tests for [`crate::source_bibtex`] and [`crate::source_bibtex_parse`] (package F13b).
//!
//! All BibTeX entries and files are self-authored synthetic fixtures.

use super::*;
use crate::paper_sources::ScanContext;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn parses_bibtex_entries_macros_nested_braces_quotes_and_tex_accents() {
    let bib = r#"
% Leading comment
@comment{ignored entry}
@preamble{"\newcommand{\noop}[1]{}"}
@string{JPHYS = "Journal of {Synthetic} Physics"}
@string{Y2024 = 2024}

@article{schrodinger2024gauge,
  author = {Schr{\"o}dinger, Erwin and de Broglie, Louis and {Collaboration on {Gauge} Fields}},
  title = {Exact {SU(2)} {M\"obius} Identities in {L}attice Models -- Part {I}},
  journal = JPHYS # " A",
  year = Y2024,
  doi = {https://doi.org/10.9999/synth.2024.001},
  eprint = {2405.01234v2},
  archivePrefix = {arXiv},
  keywords = {gauge theory, lattice; topolog{\'i}a},
  groups = {Physics/Lattice, Core},
  abstract = {We study exact identities with {\'e}tude and 50\% precision.}
}

@book(noether1921ideal,
  author = "Noether, Emmy",
  title = "Idealtheorie in Ringbereichen",
  date = "1921-03-14",
  isbn = {978-0-306-40615-7}
)
"#;

    let (entries, warnings) = parse_bibtex(bib);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(entries.len(), 2);

    let e0 = &entries[0];
    assert_eq!(e0.entry_type, "article");
    assert_eq!(e0.citekey, "schrodinger2024gauge");
    assert_eq!(
        e0.fields.get("journal").map(String::as_str),
        Some("Journal of {Synthetic} Physics A")
    );
    assert_eq!(decode_tex_text(&e0.fields["journal"]), "Journal of Synthetic Physics A");
    assert_eq!(
        decode_tex_text(&e0.fields["title"]),
        "Exact SU(2) Möbius Identities in Lattice Models \u{2013} Part I"
    );
    let authors = split_bibtex_authors(&e0.fields["author"]);
    assert_eq!(
        authors,
        vec![
            "Schrödinger, Erwin",
            "de Broglie, Louis",
            "Collaboration on Gauge Fields"
        ]
    );
    assert_eq!(
        decode_tex_text(&e0.fields["abstract"]),
        "We study exact identities with étude and 50% precision."
    );

    let (ids0, ver0) = extract_bibtex_ids(&e0.fields);
    assert_eq!(ids0.doi.as_deref(), Some("10.9999/synth.2024.001"));
    assert_eq!(ids0.arxiv.as_deref(), Some("2405.01234"));
    assert_eq!(ver0.as_deref(), Some("v2"));

    let e1 = &entries[1];
    assert_eq!(e1.entry_type, "book");
    assert_eq!(e1.citekey, "noether1921ideal");
    let (ids1, _) = extract_bibtex_ids(&e1.fields);
    assert_eq!(ids1.isbn.as_deref(), Some("9780306406157"));
}

#[test]
fn parses_zotero_jabref_and_kandinsky_file_fields() {
    // Zotero Better BibTeX multi-attachment with escaped colons & semicolons
    let bbt = parse_bib_file_field(
        r"Full Text PDF:papers/a\:1.pdf:application/pdf;Supplement:papers/b\;2.pdf:application/pdf",
    );
    assert_eq!(bbt.len(), 2);
    assert_eq!(bbt[0].description, "Full Text PDF");
    assert_eq!(bbt[0].path, "papers/a:1.pdf");
    assert_eq!(bbt[0].mime_or_type, "application/pdf");
    assert_eq!(bbt[1].path, "papers/b;2.pdf");

    // JabRef format
    let jab = parse_bib_file_field(":rel/sub/paper.pdf:PDF;TeX Source:src/main.tex:text/x-tex");
    assert_eq!(jab.len(), 2);
    assert_eq!(jab[0].path, "rel/sub/paper.pdf");
    assert_eq!(jab[0].mime_or_type, "PDF");
    assert_eq!(jab[1].path, "src/main.tex");
    assert_eq!(jab[1].mime_or_type, "text/x-tex");

    // Windows drive letters (both escaped and unescaped)
    let win = parse_bib_file_field(r"Full Text:C\:/Papers/win1.pdf:application/pdf;C:/Papers/win2.pdf:application/pdf");
    assert_eq!(win.len(), 2);
    assert_eq!(win[0].path, "C:/Papers/win1.pdf");
    assert_eq!(win[1].path, "C:/Papers/win2.pdf");

    // Kandinsky refs.bib plain relative path
    let kand = parse_bib_file_field("refs/papers/k2024/k2024.pdf");
    assert_eq!(kand.len(), 1);
    assert_eq!(kand[0].path, "refs/papers/k2024/k2024.pdf");
}

#[test]
fn bibtex_adapter_resolves_attachments_and_refuses_escapes_and_symlinks() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path().join("bibroot");
    let ext = tmp.path().join("SharedPool");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(base.join("pdfs")).unwrap();
    fs::create_dir_all(&ext).unwrap();
    fs::create_dir_all(&outside).unwrap();

    fs::write(base.join("pdfs/local.pdf"), b"%PDF-1.4 local").unwrap();
    fs::write(ext.join("shared.pdf"), b"%PDF-1.4 shared").unwrap();
    fs::write(outside.join("secret.pdf"), b"%PDF-1.4 secret").unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join("secret.pdf"), base.join("pdfs/link.pdf")).unwrap();
    }

    assert_eq!(decode_tex_text(r"Foo \'{} Bar"), "Foo Bar");

    let bib_path = base.join("library.bib");
    fs::write(
        &bib_path,
        "@article{good2024,\n\
           author = {Ada Synth},\n\
           title = {Valid Local Attachment},\n\
           year = {2024},\n\
           doi = {10.9999/good.2024},\n\
           file = {Full Text:pdfs/local.pdf:application/pdf}\n\
         }\n\
         @article{ext2024,\n\
           author = {Ben Pool},\n\
           title = {External Root Attachment},\n\
           year = {2024},\n\
           file = {../SharedPool/shared.pdf}\n\
         }\n\
         @article{escape2024,\n\
           author = {Eve Escape},\n\
           title = {Escaping Path},\n\
           year = {2024},\n\
           file = {:../outside/secret.pdf:PDF}\n\
         }\n\
         @article{symlink2024,\n\
           author = {Sam Link},\n\
           title = {Symlink Path},\n\
           year = {2024},\n\
           file = {:pdfs/link.pdf:PDF}\n\
         }\n\
         @article{missing2024,\n\
           author = {Mia Gone},\n\
           title = {Missing File Entry},\n\
           year = {2024},\n\
           file = {:pdfs/gone.pdf:PDF}\n\
         }\n",
    )
    .unwrap();

    let canon_bib = canonicalize_bibtex_source_root(&bib_path).unwrap();
    let source = SourceRecord {
        id: "bib-test".into(),
        kind: SourceKind::Bibtex,
        root: portable_path_string(&canon_bib),
        settings: json!({ "external_pdf_roots": ["../SharedPool"] }),
        added_ms: 1,
        last_sync_ms: None,
        state: "pending_sync".into(),
    };
    let ctx = ScanContext::from_previous(&[], true, 100);
    let out = BibtexAdapter.scan(&source, &ctx).unwrap();
    assert_eq!(out.items.len(), 5);

    let good = out.items.iter().find(|i| i.item.external_id == "good2024").unwrap();
    assert_eq!(good.item.state, SourceItemState::Present);
    assert_eq!(good.item.files.len(), 1);
    assert!(good.item.files[0].sha256.is_some());
    assert_eq!(good.item.citekey.as_deref(), Some("good2024"));

    let ext_item = out.items.iter().find(|i| i.item.external_id == "ext2024").unwrap();
    assert_eq!(ext_item.item.state, SourceItemState::Present);
    assert_eq!(ext_item.item.files.len(), 1);

    let esc = out.items.iter().find(|i| i.item.external_id == "escape2024").unwrap();
    assert!(esc.item.files.is_empty(), "escaping path must be refused");
    assert!(out
        .warnings
        .iter()
        .any(|w| w.path.contains("escape2024") && w.reason.contains("escapes")));

    let sym = out.items.iter().find(|i| i.item.external_id == "symlink2024").unwrap();
    assert!(sym.item.files.is_empty(), "symlink must be refused");

    let miss = out.items.iter().find(|i| i.item.external_id == "missing2024").unwrap();
    assert_eq!(miss.item.state, SourceItemState::Missing);
    assert!(miss.item.files.is_empty());
}
