//! Unit & security tests for [`crate::source_export`] (package F13c).
//!
//! Tests opt-in BibTeX export (user citekey preservation, escaping, atomic write, refusal inside
//! registered read-only source roots / `..` / symlinks) and Kandinsky capture-paper validation,
//! deep-link encoding, and argv-only execution using a self-compiled stub binary (zero shell).

use super::*;
use crate::library::{LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState};
use crate::source_sync::register_source;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn seed_library_paper(
    lib: &Library,
    data_dir: &Path,
    key_str: &str,
    title: &str,
    citekey: Option<&str>,
) -> PaperKey {
    let key = PaperKey::parse(key_str).unwrap();
    let mut ids = LibraryIds::default();
    if let Some(ax) = key_str.strip_prefix("arxiv:") {
        ids.arxiv = Some(ax.to_string());
    } else if let Some(d) = key_str.strip_prefix("doi:") {
        ids.doi = Some(d.to_string());
    }
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: ids.clone(),
        title: title.to_string(),
        authors: vec!["Synth, Ada".into(), "Tester, Ben".into()],
        year: Some(2024),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags {
            venue: Some("Synthetic & Letters 100%".into()),
            owned: true,
            ..Default::default()
        },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    if let Some(ck) = citekey {
        let src_dir = data_dir.join("seed_src");
        fs::create_dir_all(&src_dir).unwrap();
        let src = register_source(lib, SourceKind::Folder, &src_dir, json!({}), Some("seed-src".into()))
            .unwrap();
        lib.upsert_source_item(&SourceItemRecord {
            source_id: src.id,
            external_id: ck.to_string(),
            key: Some(key.clone()),
            citekey: Some(ck.to_string()),
            ids,
            version: Some("v2".into()),
            title: title.to_string(),
            authors: vec!["Synth, Ada".into()],
            year: Some(2024),
            item_kind: "journalArticle".into(),
            collections: vec![],
            tags: vec![],
            files: vec![],
            state: SourceItemState::Present,
            updated_ms: 1,
        })
        .unwrap();
    }
    key
}

#[test]
fn bibtex_export_preserves_user_citekey_escapes_and_refuses_source_roots() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let lib = Library::open(&data_dir).unwrap();

    let k1 = seed_library_paper(
        &lib,
        &data_dir,
        "arxiv:2405.01111",
        "Exact 100% {Gauge} & #1 $x$ Study",
        Some("ada2024custom"),
    );
    let k2 = seed_library_paper(
        &lib,
        &data_dir,
        "doi:10.9999/synth.export.2",
        "Second Synthetic Paper",
        None,
    );

    // Valid separate user-chosen output directory succeeds atomically.
    let out_dir = tmp.path().join("user_exports");
    fs::create_dir_all(&out_dir).unwrap();
    let out_file = out_dir.join("selected.bib");
    let res = export_library_bibtex_to_path(&lib, &[k1.clone(), k2.clone()], &out_file).unwrap();
    assert_eq!(res.count, 2);
    assert_eq!(res.citekeys[0], "ada2024custom");
    let written = fs::read_to_string(&out_file).unwrap();
    assert!(written.contains("@article{ada2024custom,"), "{written}");
    assert!(
        written.contains(r"Exact 100\% \{Gauge\} \& \#1 \$x\$ Study"),
        "BibTeX special chars must be escaped: {written}"
    );
    assert!(written.contains("eprint = {2405.01111v2}"), "{written}");
    assert!(written.contains("doi = {10.9999/synth.export.2}"), "{written}");

    // Refuses writing inside any registered read-only PaperSource root.
    let inside_src = data_dir.join("seed_src/evil.bib");
    let err_src = export_library_bibtex_to_path(&lib, &[k1.clone()], &inside_src).unwrap_err();
    assert!(err_src.contains("registered read-only paper source"), "{err_src}");

    // Refuses `..` traversal.
    let traversal = out_dir.join("../user_exports/tricky.bib");
    let err_dotdot = export_library_bibtex_to_path(&lib, &[k1.clone()], &traversal).unwrap_err();
    assert!(err_dotdot.contains(".."), "{err_dotdot}");

    // Refuses Kandinsky vault or Zotero dir even when unregistered.
    let fake_zot = tmp.path().join("fake_zotero");
    fs::create_dir_all(&fake_zot).unwrap();
    fs::write(fake_zot.join("zotero.sqlite"), b"stub").unwrap();
    let err_zot =
        export_library_bibtex_to_path(&lib, &[k1.clone()], &fake_zot.join("out.bib")).unwrap_err();
    assert!(err_zot.contains("Kandinsky vault or Zotero"), "{err_zot}");

    #[cfg(unix)]
    {
        let sym_target = out_dir.join("target.bib");
        let sym_link = out_dir.join("link.bib");
        fs::write(&sym_target, b"orig").unwrap();
        std::os::unix::fs::symlink(&sym_target, &sym_link).unwrap();
        let err_sym = export_library_bibtex_to_path(&lib, &[k1], &sym_link).unwrap_err();
        assert!(err_sym.contains("symlink"), "{err_sym}");
    }
}

#[test]
fn kandinsky_target_validator_accepts_valid_and_rejects_injection_and_bad_schemes() {
    assert_eq!(
        validate_kandinsky_capture_target("arxiv:2405.01234v2").unwrap().as_str(),
        "arXiv:2405.01234v2"
    );
    assert_eq!(
        validate_kandinsky_capture_target("doi:10.9999/Synth.2024.1").unwrap().as_str(),
        "doi:10.9999/synth.2024.1"
    );
    assert_eq!(
        validate_kandinsky_capture_target("https://arxiv.org/abs/2405.01234").unwrap().as_str(),
        "https://arxiv.org/abs/2405.01234"
    );

    let rejected = [
        "",
        "-e",
        "--config=/etc/passwd",
        "arXiv:2405.01234;rm -rf /",
        "arXiv:2405.01234|cat",
        "arXiv:$(whoami)",
        "arXiv:`id`",
        "arXiv:2405.01234\n--evil",
        "doi:10.1000/abc def",
        "http://arxiv.org/abs/2405.01234",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "kandinsky://capture-paper?target=x",
        "https://user@arxiv.org/abs/2405.01234",
    ];
    for bad in rejected {
        assert!(
            validate_kandinsky_capture_target(bad).is_err(),
            "target `{bad}` must be rejected"
        );
    }
}

#[test]
fn kandinsky_deep_link_encoding_and_argv_only_stub_execution() {
    let target = validate_kandinsky_capture_target("doi:10.9999/a+b/c:1").unwrap();
    let link = build_kandinsky_deep_link(&target, Some("Spin & Gauge: A/B?"));
    assert_eq!(
        link,
        "kandinsky://capture-paper?target=doi%3A10.9999%2Fa%2Bb%2Fc%3A1&title=Spin%20%26%20Gauge%3A%20A%2FB%3F"
    );

    // Compile a tiny cross-platform stub binary with rustc (no shell, works on Windows and Linux).
    let tmp = TempDir::new().unwrap();
    let stub_src = tmp.path().join("fake_kandinsky.rs");
    let stub_bin = tmp
        .path()
        .join(format!("fake_kandinsky{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &stub_src,
        "fn main() {\n\
           let args: Vec<String> = std::env::args().skip(1).collect();\n\
           println!(\"ARGC={}\", args.len());\n\
           for a in args { println!(\"ARG={a}\"); }\n\
         }\n",
    )
    .unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let comp = Command::new(rustc)
        .args(["--edition", "2021"])
        .arg(&stub_src)
        .arg("-o")
        .arg(&stub_bin)
        .output()
        .expect("compile fake_kandinsky stub");
    assert!(
        comp.status.success(),
        "rustc failed: {}",
        String::from_utf8_lossy(&comp.stderr)
    );

    let ax = validate_kandinsky_capture_target("arXiv:2405.01234v2").unwrap();
    let outcome = invoke_kandinsky_cli(&stub_bin, &ax).unwrap();
    assert!(outcome.success, "stub failed: {outcome:?}");
    assert_eq!(outcome.argv, vec!["capture-paper", "arXiv:2405.01234v2"]);
    assert!(outcome.stdout.contains("ARGC=2"), "{}", outcome.stdout);
    assert!(outcome.stdout.contains("ARG=capture-paper"), "{}", outcome.stdout);
    assert!(
        outcome.stdout.contains("ARG=arXiv:2405.01234v2"),
        "{}",
        outcome.stdout
    );
}
