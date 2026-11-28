//! Unit tests for `pdf_match.rs` (PDF-normalized quote matching and offline/mock PaperKey matching).

use super::*;
use crate::pdf_firstpage::{extract_first_page_candidates, CandidateSource, FieldCandidate};
use crate::pdf_pages::{PageQualityMetrics, PageRecord};

fn sample_page_map() -> PageMap {
    let m = PageQualityMetrics {
        char_count: 100,
        alpha_count: 80,
        word_count: 15,
        image_coverage_pct: 0,
        has_invisible_text: false,
        width_pt: 612,
        height_pt: 792,
    };
    PageMap {
        page_count: 2,
        pages: vec![
            PageRecord {
                page: 1,
                basis: PageBasis::TextLayer,
                first_line: Some(1),
                last_line: Some(3),
                metrics: m.clone(),
                reason: None,
            },
            PageRecord {
                page: 2,
                basis: PageBasis::ScanTextLayer,
                first_line: Some(5),
                last_line: Some(7),
                metrics: m,
                reason: Some("full-page image with text layer".into()),
            },
        ],
    }
}

#[test]
fn quote_matching_ignores_ligatures_soft_hyphens_and_line_end_hyphens_without_mutating_stored() {
    let stored = concat!(
        "Theorem 1. The \u{FB01}eld \u{FB02}ow has a uni\u{00AD}form eigen-\n",
        "value bound on quasi-\n",
        "local operators.\n",
        "\x0c\n",
        "Section 2. A second re-\n",
        "sult holds on page two with \u{FB03}cient scaling.\n",
        "End of page two."
    );
    let map = sample_page_map();

    let m1 = match_pdf_quote(
        stored,
        &map,
        "The field flow has a uniform eigenvalue bound",
    )
    .expect("ligature + soft hyphen + line-end hyphen match");
    assert_eq!(m1.start_line, 1);
    assert_eq!(m1.end_line, 2);
    assert_eq!(m1.start_page, Some(1));
    assert_eq!(m1.end_page, Some(1));
    assert_eq!(m1.start_page_basis, Some(PageBasis::TextLayer));
    assert!(m1.stored_excerpt.contains('\u{FB01}'));
    assert!(m1.stored_excerpt.contains("eigen-\nvalue"));

    // Compound hyphen preserved across line break.
    let m2 = match_pdf_quote(stored, &map, "bound on quasi-local operators")
        .expect("compound hyphen across line break");
    assert_eq!(m2.start_line, 2);
    assert_eq!(m2.end_line, 3);
    assert_eq!(m2.start_page, Some(1));
    assert_eq!(m2.stored_excerpt, "bound on quasi-\nlocal operators");

    // Match on scanned page 2 preserves ScanTextLayer basis.
    let m3 = match_pdf_quote(stored, &map, "second result holds on page two with fficient scaling")
        .expect("page 2 match");
    assert_eq!(m3.start_page, Some(2));
    assert_eq!(m3.start_page_basis, Some(PageBasis::ScanTextLayer));
    assert!(quotes_match_pdf(stored, "uniform eigenvalue bound"));
    assert!(!quotes_match_pdf(stored, "completely absent phrase"));
}

#[test]
fn offline_matching_follows_f2_paper_key_precedence_and_file_fallback() {
    let both = extract_first_page_candidates(
        "arXiv:2401.00042v3 [hep-th]\nDOI: 10.1000/test.2024.1\nExample Paper\nAda Example\n",
        None,
    );
    let m_ax = match_candidates_offline(&both, Some(b"%PDF-1.4 dummy")).expect("arxiv match");
    assert_eq!(m_ax.paper.key.as_str(), "arxiv:2401.00042");
    assert_eq!(m_ax.paper.version.as_deref(), Some("v3"));
    assert_eq!(m_ax.paper.ids.doi.as_deref(), Some("10.1000/test.2024.1"));
    assert_eq!(m_ax.basis, MatchBasis::ArxivCandidate);
    assert_eq!(m_ax.source_line, Some(1));
    assert!(!m_ax.confirmed);

    let doi_only = extract_first_page_candidates("DOI: 10.1000/test.2024.2\n", None);
    let m_doi = match_candidates_offline(&doi_only, Some(b"%PDF-1.4 dummy")).expect("doi match");
    assert_eq!(m_doi.paper.key.as_str(), "doi:10.1000/test.2024.2");
    assert_eq!(m_doi.basis, MatchBasis::DoiCandidate);
    assert_eq!(m_doi.source_line, Some(1));

    let no_ids = extract_first_page_candidates("Untagged Notes\nAda Example\n", None);
    assert_eq!(match_candidates_offline(&no_ids, None), None);
    let raw = b"%PDF-1.4 synthetic-bytes";
    let m_file = match_candidates_offline(&no_ids, Some(raw)).expect("file hash fallback");
    assert_eq!(m_file.paper.key, PaperKey::from_file_bytes(raw));
    assert_eq!(m_file.basis, MatchBasis::FileBytesHash);
}

struct MockFetcher {
    hits: Vec<RemoteCandidate>,
}

impl MetadataFetcher for MockFetcher {
    fn lookup(&self, _query: &MetadataLookupQuery) -> Result<Vec<RemoteCandidate>, String> {
        Ok(self.hits.clone())
    }
}

#[test]
fn network_lookup_requires_exact_normalized_title_and_first_author_surname() {
    let cands = FirstPageCandidates {
        title: Some(FieldCandidate::from_line(
            "Spectral Stability of Coupled Oscillators".into(),
            1,
            2,
            CandidateSource::FirstPageLine,
        )),
        authors: Some(FieldCandidate::from_line(
            vec!["Ada Example".into(), "Viktor Novak".into()],
            1,
            3,
            CandidateSource::FirstPageLine,
        )),
        ..FirstPageCandidates::default()
    };

    let wrong_title = MockFetcher {
        hits: vec![RemoteCandidate {
            paper: PaperRef::from_ids(
                PaperIds { doi: Some("10.1000/wrong.1".into()), ..PaperIds::default() },
                None,
            )
            .unwrap(),
            title: "Spectral Stability of Uncoupled Oscillators".into(),
            authors: vec!["Ada Example".into()],
            year: Some("2024".into()),
        }],
    };
    assert_eq!(lookup_candidates_with_fetcher(&cands, &wrong_title).unwrap(), None);

    let wrong_author = MockFetcher {
        hits: vec![RemoteCandidate {
            paper: PaperRef::from_ids(
                PaperIds { doi: Some("10.1000/wrong.2".into()), ..PaperIds::default() },
                None,
            )
            .unwrap(),
            title: "Spectral Stability of Coupled Oscillators".into(),
            authors: vec!["Different Person".into()],
            year: Some("2024".into()),
        }],
    };
    assert_eq!(lookup_candidates_with_fetcher(&cands, &wrong_author).unwrap(), None);

    let right = MockFetcher {
        hits: vec![RemoteCandidate {
            paper: PaperRef::from_ids(
                PaperIds { doi: Some("10.1000/exact.2024".into()), ..PaperIds::default() },
                None,
            )
            .unwrap(),
            title: "Spectral stability of coupled oscillators!".into(),
            authors: vec!["Example, A.".into()],
            year: Some("2024".into()),
        }],
    };
    let hit = lookup_candidates_with_fetcher(&cands, &right)
        .unwrap()
        .expect("exact normalized match");
    assert_eq!(hit.paper.key.as_str(), "doi:10.1000/exact.2024");
    assert_eq!(hit.basis, MatchBasis::RemoteExactTitleAuthor);
    assert_eq!(hit.source_line, Some(2));
    assert!(!hit.confirmed);
}
