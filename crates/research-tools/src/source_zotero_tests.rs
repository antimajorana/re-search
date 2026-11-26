// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.
//! Unit tests for [`crate::source_zotero`], including ported Kandinsky read-only tests
//! (`00-overview.md` 2.11 porting rule 7) and path-containment tests.

use super::*;
use std::collections::BTreeMap;

#[test]
fn identity_fold_drops_rich_text_markup_entities_and_accents() {
    assert_eq!(
        ident_fold("Superconductivity in MgB<sub>2</sub>"),
        ident_fold("Superconductivity in MgB2")
    );
    assert_eq!(
        ident_fold("<i>Ab initio</i> &amp; <span class=\"nocase\">DFT</span>"),
        "abinitiodft"
    );
    assert_eq!(
        ident_fold("Théorie des quanta"),
        ident_fold("The\u{301}orie  des quanta")
    );
    assert_eq!(ident_fold("Théorie"), "theorie");
    assert_eq!(ident_fold("a < b, c > d"), "abcd");
    assert_eq!(
        ident_fold("Eﬃcient classical simulation"),
        ident_fold("Efficient classical simulation")
    );
    assert_eq!(
        first_author_last(&["Schrödinger, Erwin".into()]),
        first_author_last(&["Erwin Schrodinger".into()])
    );
}

#[test]
fn parses_extra_citekey_and_prefs_js() {
    assert_eq!(
        extra_citekey("arXiv: 2401.00001\nCitation Key: synth2024pinned\n"),
        Some("synth2024pinned".to_string())
    );
    assert_eq!(
        extra_citekey("bibtex: legacy_key.2020"),
        Some("legacy_key.2020".to_string())
    );
    assert_eq!(extra_citekey("no citekey here"), None);

    let prefs = parse_prefs_js(
        "user_pref(\"extensions.zotero.dataDir\", \"C:\\\\Users\\\\u\\\\Zotero\");\n\
         user_pref(\"extensions.zotero.baseAttachmentPath\", \"/home/u/Papers\");\n\
         user_pref(\"extensions.zotmoov.dst_dir\", \"/home/u/ZotMoov\");\n\
         user_pref(\"extensions.zotmoov.enable_subdir_move\", true);\n\
         user_pref(\"extensions.zotmoov.subdir_str\", \"{%c}\");\n",
    );
    assert_eq!(prefs.data_dir.as_deref(), Some("C:\\Users\\u\\Zotero"));
    assert_eq!(prefs.base_attachment_path.as_deref(), Some("/home/u/Papers"));
    assert_eq!(prefs.zotmoov_dst_dir.as_deref(), Some("/home/u/ZotMoov"));
    assert!(prefs.zotmoov_enable_subdir);
    assert_eq!(prefs.zotmoov_subdir_str.as_deref(), Some("{%c}"));
}

#[test]
fn resolves_zotero_ids_including_arxiv_doi_and_url() {
    let mut f = BTreeMap::new();
    f.insert("DOI".into(), "10.48550/arXiv.2405.07777v2".into());
    f.insert("ISBN".into(), "978-0-306-40615-7".into());
    let (ids, ver) = extract_zotero_ids(&f);
    assert_eq!(ids.arxiv.as_deref(), Some("2405.07777"));
    assert_eq!(ver.as_deref(), Some("v2"));
    assert_eq!(ids.isbn.as_deref(), Some("9780306406157"));

    let mut f2 = BTreeMap::new();
    f2.insert("url".into(), "https://arxiv.org/abs/2501.00001v3".into());
    let (ids2, ver2) = extract_zotero_ids(&f2);
    assert_eq!(ids2.arxiv.as_deref(), Some("2501.00001"));
    assert_eq!(ver2.as_deref(), Some("v3"));
}

#[test]
fn zotero_attachment_resolution_refuses_traversal_and_outside_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let zot_dir = canonicalize_source_root(tmp.path()).unwrap();
    let base_att = zot_dir.join("LinkedPool");
    let outside = zot_dir.join("Outside");
    fs::create_dir_all(zot_dir.join("storage/ATT1")).unwrap();
    fs::create_dir_all(&base_att).unwrap();
    fs::create_dir_all(&outside).unwrap();

    fs::write(zot_dir.join("storage/ATT1/ok.pdf"), b"%PDF-1.4 ok").unwrap();
    fs::write(base_att.join("linked.pdf"), b"%PDF-1.4 linked").unwrap();
    fs::write(outside.join("secret.pdf"), b"%PDF-1.4 secret").unwrap();

    let ok_storage = ZoteroAttachmentRow {
        attachment_item_id: 1,
        attachment_key: "ATT1".into(),
        content_type: "application/pdf".into(),
        raw_path: "storage:ok.pdf".into(),
    };
    assert!(matches!(
        resolve_zotero_attachment(&zot_dir, Some(&base_att), &[], &ok_storage),
        ZotAttachResolve::Found { .. }
    ));

    let ok_linked = ZoteroAttachmentRow {
        attachment_item_id: 2,
        attachment_key: "ATT2".into(),
        content_type: "application/pdf".into(),
        raw_path: "attachments:linked.pdf".into(),
    };
    assert!(matches!(
        resolve_zotero_attachment(&zot_dir, Some(&base_att), &[], &ok_linked),
        ZotAttachResolve::Found { .. }
    ));

    let escape_linked = ZoteroAttachmentRow {
        attachment_item_id: 3,
        attachment_key: "ATT3".into(),
        content_type: "application/pdf".into(),
        raw_path: "attachments:../Outside/secret.pdf".into(),
    };
    assert!(matches!(
        resolve_zotero_attachment(&zot_dir, Some(&base_att), &[], &escape_linked),
        ZotAttachResolve::Refused(_)
    ));

    let escape_storage = ZoteroAttachmentRow {
        attachment_item_id: 4,
        attachment_key: "../Outside".into(),
        content_type: "application/pdf".into(),
        raw_path: "storage:secret.pdf".into(),
    };
    assert!(matches!(
        resolve_zotero_attachment(&zot_dir, Some(&base_att), &[], &escape_storage),
        ZotAttachResolve::Refused(_)
    ));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.join("secret.pdf"),
            zot_dir.join("storage/ATT1/sym.pdf"),
        )
        .unwrap();
        let sym_att = ZoteroAttachmentRow {
            attachment_item_id: 5,
            attachment_key: "ATT1".into(),
            content_type: "application/pdf".into(),
            raw_path: "storage:sym.pdf".into(),
        };
        assert!(matches!(
            resolve_zotero_attachment(&zot_dir, Some(&base_att), &[], &sym_att),
            ZotAttachResolve::Refused(_)
        ));
    }
}
