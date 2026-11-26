use super::*;
use crate::library::SourceItemFile;
use crate::paper_sources::FILE_ROLE_PDF;

#[allow(clippy::too_many_arguments)]
fn make_item(
    source_id: &str,
    external_id: &str,
    title: &str,
    authors: &[&str],
    year: Option<i32>,
    item_kind: &str,
    ids: LibraryIds,
    sha256: Option<&str>,
) -> SourceItemRecord {
    let key = derive_paper_key(&ids, sha256);
    SourceItemRecord {
        source_id: source_id.to_string(),
        external_id: external_id.to_string(),
        key,
        citekey: Some(external_id.to_string()),
        ids,
        version: None,
        title: title.to_string(),
        authors: authors.iter().map(|s| (*s).to_string()).collect(),
        year,
        item_kind: item_kind.to_string(),
        collections: Vec::new(),
        tags: Vec::new(),
        files: sha256
            .map(|h| {
                vec![SourceItemFile {
                    path: format!("/tmp/{external_id}.pdf"),
                    role: FILE_ROLE_PDF.to_string(),
                    size: 1024,
                    mtime_ms: 1000,
                    sha256: Some(h.to_string()),
                }]
            })
            .unwrap_or_default(),
        state: SourceItemState::Present,
        updated_ms: 1000,
    }
}

#[test]
fn detects_exact_duplicates_by_doi_arxiv_sha_and_isbn() {
    let sha = "a".repeat(64);
    let items = vec![
        // DOI duplicate pair
        make_item(
            "s1",
            "doi_a",
            "Synthetic Spin Chain Spectrum",
            &["Voss, Elena"],
            Some(2024),
            "journalArticle",
            LibraryIds {
                doi: Some("10.9999/SYNTH.2024.1".into()),
                ..Default::default()
            },
            None,
        ),
        make_item(
            "s2",
            "doi_b",
            "Synthetic Spin Chain Spectrum",
            &["Elena Voss"],
            Some(2024),
            "preprint",
            LibraryIds {
                doi: Some("https://doi.org/10.9999/synth.2024.1".into()),
                ..Default::default()
            },
            None,
        ),
        // arXiv preprint + journal version sharing arXiv id (and arXiv DOI)
        make_item(
            "s1",
            "preprint_v1",
            "Topological Boundary Flux",
            &["Okafor, K."],
            Some(2023),
            "preprint",
            LibraryIds {
                arxiv: Some("2305.01234".into()),
                doi: Some("10.48550/arXiv.2305.01234".into()),
                ..Default::default()
            },
            None,
        ),
        make_item(
            "s1",
            "journal_pub",
            "Topological Boundary Flux",
            &["Okafor, K."],
            Some(2024),
            "journalArticle",
            LibraryIds {
                arxiv: Some("2305.01234".into()),
                doi: Some("10.9999/jphys.2024.42".into()),
                ..Default::default()
            },
            None,
        ),
        // PDF SHA-256 duplicate pair (no identifiers)
        make_item(
            "s1",
            "hash_a",
            "Notes on Lattice Dualities",
            &["Sato, H."],
            Some(2021),
            "journalArticle",
            LibraryIds::default(),
            Some(&sha),
        ),
        make_item(
            "s2",
            "hash_b",
            "Copy of Lattice Dualities",
            &["Sato, H."],
            Some(2021),
            "journalArticle",
            LibraryIds::default(),
            Some(&sha),
        ),
        // ISBN duplicate pair
        make_item(
            "s1",
            "isbn_a",
            "Algebraic Geometry Primer",
            &["Nadia Chen"],
            Some(2020),
            "book",
            LibraryIds {
                isbn: Some("978-0-306-40615-7".into()),
                ..Default::default()
            },
            None,
        ),
        make_item(
            "s2",
            "isbn_b",
            "Algebraic Geometry Primer",
            &["Chen, Nadia"],
            Some(2020),
            "book",
            LibraryIds {
                isbn: Some("9780306406157".into()),
                ..Default::default()
            },
            None,
        ),
    ];

    let analysis = find_duplicates(&items);
    assert_eq!(analysis.exact_groups.len(), 4);
    assert!(analysis.possible_groups.is_empty());

    let arxiv_grp = analysis
        .exact_groups
        .iter()
        .find(|g| g.signals.contains(&DuplicateSignal::Arxiv))
        .expect("arxiv group");
    assert_eq!(
        arxiv_grp.canonical_key.as_ref().map(|k| k.as_str()),
        Some("arxiv:2305.01234")
    );
    // Journal entry wins primary over preprint and publisher DOI replaces arXiv DOI.
    assert_eq!(arxiv_grp.members[0].external_id, "journal_pub");
    assert_eq!(
        arxiv_grp.merged_ids.doi.as_deref(),
        Some("10.9999/jphys.2024.42")
    );
}

#[test]
fn fuzzy_title_author_year_is_possible_duplicate_and_never_merges() {
    let items = vec![
        make_item(
            "s1",
            "fuzzy_1",
            "Exact Partition Functions on Triangulated Surfaces",
            &["Mendez, Carla"],
            Some(2022),
            "journalArticle",
            LibraryIds::default(),
            None,
        ),
        make_item(
            "s2",
            "fuzzy_2",
            "Exact Partition Functions on Triangulated Surfaces",
            &["Carla Mendez"],
            Some(2023),
            "preprint",
            LibraryIds::default(),
            None,
        ),
        // Different surname -> not duplicate
        make_item(
            "s2",
            "other_author",
            "Exact Partition Functions on Triangulated Surfaces",
            &["Patel, R."],
            Some(2022),
            "journalArticle",
            LibraryIds::default(),
            None,
        ),
    ];

    let analysis = find_duplicates(&items);
    assert!(analysis.exact_groups.is_empty(), "fuzzy must never merge as exact");
    assert_eq!(analysis.possible_groups.len(), 1);
    let grp = &analysis.possible_groups[0];
    assert_eq!(grp.confidence, DuplicateConfidence::Possible);
    assert_eq!(grp.signals, vec![DuplicateSignal::TitleAuthorYear]);
    assert!(grp.canonical_key.is_none());
    assert_eq!(grp.members.len(), 2);
}
