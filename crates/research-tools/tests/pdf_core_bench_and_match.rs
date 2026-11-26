//! Integration tests for package F14a extractor measurement harness (Plain vs Layout on
//! self-authored single-column, two-column, ligature, hyphenation, and scan fixtures) and
//! first-page metadata + offline/loopback PaperKey matching.

mod pdf_support;

use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_firstpage::{
    extract_first_page_candidates, extract_pdf_info_candidates, merge_first_page_candidates,
    CandidateSource,
};
use research_tools::pdf_match::{
    lookup_candidates_with_fetcher, match_candidates_offline, MatchBasis,
    PaperFetchMetadataAdapter,
};
use research_tools::pdf_pages::{
    extract_pdf_pages, measure_extractor, ExpectedSentence, ExtractorFixture, OrderedSequence,
    PageBasis, PdfExtractLayout, PdfExtractOptions, PdfExtractPlain,
};
use research_tools::{LiteratureEndpoints, ResearchEnv};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

#[test]
fn extractor_comparison_harness_measures_plain_vs_layout_on_fixtures() {
    let single_col_pdf = build_pdf(
        &[
            PageSpec::single_column(&[
                "arXiv:2404.01010v1 [math-ph] 10 Apr 2024",
                "Variational Bounds for Lattice Models",
                "Ada Example and Clara Lin",
                "Abstract. We establish a \u{FB01}rst uni\u{00AD}form spectral bound.",
            ]),
            PageSpec::single_column(&[
                "1. Main Theorem",
                "The operator admits a sharp eigen-",
                "value gap and quasi-",
                "local decay along every \u{FB02}ow.",
                "We verify the su\u{FB03}cient condition directly.",
            ]),
            PageSpec::single_column(&[
                "2. Concluding Remarks",
                "All estimates extend to periodic boundary conditions.",
            ]),
        ],
        &PdfBuildOptions::default(),
    );

    let left_lines = [
        "1. Introduction to the model.",
        "Left alpha paragraph starts here.",
        "Left beta step builds the bound.",
        "Left gamma concludes column one.",
    ];
    let right_lines = [
        "2. Proof of the main lemma.",
        "Right delta starts column two.",
        "Right epsilon applies Cauchy.",
        "Right zeta completes the proof.",
    ];
    let two_col_seq = build_pdf(
        &[PageSpec::two_column(
            &["Two-Column Sequential Synthetic Article"],
            &left_lines,
            &right_lines,
            false,
        )],
        &PdfBuildOptions::default(),
    );
    let two_col_interleaved = build_pdf(
        &[PageSpec::two_column(
            &["Two-Column Interleaved Synthetic Article"],
            &left_lines,
            &right_lines,
            true,
        )],
        &PdfBuildOptions::default(),
    );
    let mixed_scan = build_pdf(
        &[
            PageSpec::single_column(&["Opening page with digital text layer."]),
            PageSpec::full_page_image_only(),
            PageSpec::scan_with_text(
                &["Scanned page three with publisher OCR text layer."],
                true,
            ),
        ],
        &PdfBuildOptions::default(),
    );

    let fixtures = vec![
        ExtractorFixture {
            name: "single_col_ligatures_hyphens".into(),
            pdf_bytes: single_col_pdf,
            expected_page_count: 3,
            expected_bases: vec![
                (1, PageBasis::TextLayer),
                (2, PageBasis::TextLayer),
                (3, PageBasis::TextLayer),
            ],
            expected_sentences: vec![
                ExpectedSentence {
                    text: "We establish a first uniform spectral bound.".into(),
                    expected_page: 1,
                    category: "ligature".into(),
                },
                ExpectedSentence {
                    text: "The operator admits a sharp eigenvalue gap".into(),
                    expected_page: 2,
                    category: "hyphenation".into(),
                },
                ExpectedSentence {
                    text: "gap and quasi-local decay along every flow.".into(),
                    expected_page: 2,
                    category: "hyphenation".into(),
                },
                ExpectedSentence {
                    text: "We verify the sufficient condition directly.".into(),
                    expected_page: 2,
                    category: "ligature".into(),
                },
                ExpectedSentence {
                    text: "All estimates extend to periodic boundary conditions.".into(),
                    expected_page: 3,
                    category: "plain".into(),
                },
            ],
            reading_order_checks: vec![OrderedSequence {
                page: 2,
                phrases: vec![
                    "1. Main Theorem".into(),
                    "sharp eigenvalue gap".into(),
                    "sufficient condition directly".into(),
                ],
            }],
        },
        ExtractorFixture {
            name: "two_col_sequential".into(),
            pdf_bytes: two_col_seq,
            expected_page_count: 1,
            expected_bases: vec![(1, PageBasis::TextLayer)],
            expected_sentences: vec![
                ExpectedSentence {
                    text: "Left alpha paragraph starts here.".into(),
                    expected_page: 1,
                    category: "two_col".into(),
                },
                ExpectedSentence {
                    text: "Right delta starts column two.".into(),
                    expected_page: 1,
                    category: "two_col".into(),
                },
            ],
            reading_order_checks: vec![OrderedSequence {
                page: 1,
                phrases: vec![
                    "Two-Column Sequential Synthetic Article".into(),
                    "Left alpha paragraph starts here.".into(),
                    "Left gamma concludes column one.".into(),
                    "Right delta starts column two.".into(),
                    "Right zeta completes the proof.".into(),
                ],
            }],
        },
        ExtractorFixture {
            name: "two_col_interleaved".into(),
            pdf_bytes: two_col_interleaved,
            expected_page_count: 1,
            expected_bases: vec![(1, PageBasis::TextLayer)],
            expected_sentences: vec![
                ExpectedSentence {
                    text: "Left beta step builds the bound.".into(),
                    expected_page: 1,
                    category: "two_col".into(),
                },
                ExpectedSentence {
                    text: "Right epsilon applies Cauchy.".into(),
                    expected_page: 1,
                    category: "two_col".into(),
                },
            ],
            reading_order_checks: vec![OrderedSequence {
                page: 1,
                phrases: vec![
                    "Two-Column Interleaved Synthetic Article".into(),
                    "Left alpha paragraph starts here.".into(),
                    "Left gamma concludes column one.".into(),
                    "Right delta starts column two.".into(),
                    "Right zeta completes the proof.".into(),
                ],
            }],
        },
        ExtractorFixture {
            name: "mixed_empty_and_scan".into(),
            pdf_bytes: mixed_scan,
            expected_page_count: 3,
            expected_bases: vec![
                (1, PageBasis::TextLayer),
                (2, PageBasis::None),
                (3, PageBasis::ScanTextLayer),
            ],
            expected_sentences: vec![
                ExpectedSentence {
                    text: "Opening page with digital text layer.".into(),
                    expected_page: 1,
                    category: "plain".into(),
                },
                ExpectedSentence {
                    text: "Scanned page three with publisher OCR text layer.".into(),
                    expected_page: 3,
                    category: "scan_text".into(),
                },
            ],
            reading_order_checks: vec![],
        },
    ];

    let opts = PdfExtractOptions::default();
    let plain_rep = measure_extractor(&PdfExtractPlain, &fixtures, &opts);
    let layout_rep = measure_extractor(&PdfExtractLayout, &fixtures, &opts);

    eprintln!("BENCH_PLAIN={}", serde_json::to_string(&plain_rep).unwrap());
    eprintln!("BENCH_LAYOUT={}", serde_json::to_string(&layout_rep).unwrap());

    assert_eq!(layout_rep.failures, 0);
    assert_eq!(layout_rep.total_wrong_page_links, 0);
    assert_eq!(layout_rep.total_scans_reported_as_text_layer, 0);
    assert_eq!(layout_rep.total_reading_order_errors, 0);
    assert_eq!(layout_rep.total_sentences_matched, layout_rep.total_sentences);

    let plain_interleaved = plain_rep
        .cases
        .iter()
        .find(|c| c.fixture == "two_col_interleaved")
        .unwrap();
    let layout_interleaved = layout_rep
        .cases
        .iter()
        .find(|c| c.fixture == "two_col_interleaved")
        .unwrap();
    assert!(
        plain_interleaved.reading_order_errors > 0,
        "expected PlainTextOutput to mix interleaved two-column rows"
    );
    assert_eq!(layout_interleaved.reading_order_errors, 0);
}

#[test]
fn first_page_candidates_info_dict_offline_and_loopback_fetcher_matching() {
    let pdf_doi = build_pdf(
        &[PageSpec::single_column(&[
            "Journal of Synthetic Physics, Vol. 7",
            "Boundary Spectrum of Periodic Chains",
            "Ada Example and Viktor Novak",
            "Dated: April 2024",
            "DOI: 10.1000/chain.2024.7",
        ])],
        &PdfBuildOptions::default(),
    );
    let doc = extract_pdf_pages(&pdf_doi, &PdfExtractOptions::default()).unwrap();
    let cands = extract_first_page_candidates(&doc.text, Some("paper.pdf"));
    assert_eq!(
        cands.title.as_ref().map(|t| t.value.as_str()),
        Some("Boundary Spectrum of Periodic Chains")
    );
    assert_eq!(cands.title.as_ref().and_then(|t| t.line), Some(2));
    assert_eq!(cands.dois[0].value, "10.1000/chain.2024.7");
    assert_eq!(cands.dois[0].line, Some(5));
    assert!(!cands.title.as_ref().unwrap().certain);

    let offline = match_candidates_offline(&cands, Some(&pdf_doi)).unwrap();
    assert_eq!(offline.paper.key.as_str(), "doi:10.1000/chain.2024.7");
    assert_eq!(offline.basis, MatchBasis::DoiCandidate);
    assert_eq!(offline.source_line, Some(5));

    let pdf_info = build_pdf(
        &[PageSpec::full_page_image_only()],
        &PdfBuildOptions {
            info_title_utf16: Some("Asymptotic Decay on Planar Lattices".into()),
            info_author: Some("Clara Lin; Viktor Novak".into()),
            info_subject: Some("Preprint arXiv:2403.00777v2".into()),
            info_creation_date: Some("D:20240310120000Z".into()),
            encrypted_stub: false,
        },
    );
    let info_cands = extract_pdf_info_candidates(&pdf_info);
    assert_eq!(
        info_cands.title.as_ref().map(|t| t.value.as_str()),
        Some("Asymptotic Decay on Planar Lattices")
    );
    assert_eq!(info_cands.title.as_ref().unwrap().source, CandidateSource::PdfInfoDict);
    assert_eq!(info_cands.arxiv_ids[0].value.id, "2403.00777");
    let merged = merge_first_page_candidates(extract_first_page_candidates("", None), info_cands);
    let m_info = match_candidates_offline(&merged, Some(&pdf_info)).unwrap();
    assert_eq!(m_info.paper.key.as_str(), "arxiv:2403.00777");

    let pdf_no_id = build_pdf(
        &[PageSpec::single_column(&[
            "Dispersion Relations for Synthetic Waves",
            "Ada Example",
            "Dated: January 2024",
        ])],
        &PdfBuildOptions::default(),
    );
    let doc_no_id = extract_pdf_pages(&pdf_no_id, &PdfExtractOptions::default()).unwrap();
    let cands_no_id = extract_first_page_candidates(&doc_no_id.text, None);
    let m_file = match_candidates_offline(&cands_no_id, Some(&pdf_no_id)).unwrap();
    assert_eq!(m_file.paper.key, PaperKey::from_file_bytes(&pdf_no_id));
    assert_eq!(m_file.basis, MatchBasis::FileBytesHash);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let srv = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut req_buf = [0u8; 2048];
        let _ = stream.read(&mut req_buf);
        let body = json!({
            "status": "ok",
            "message": {
                "items": [{
                    "DOI": "10.1000/waves.2024.9",
                    "title": ["Dispersion Relations for Synthetic Waves"],
                    "author": [{"given": "Ada", "family": "Example"}],
                    "published": {"date-parts": [[2024, 1]]}
                }]
            }
        })
        .to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&format!("http://127.0.0.1:{port}")).unwrap());
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(clock);
    let adapter = PaperFetchMetadataAdapter::new(&env, &gate);
    let resolved = lookup_candidates_with_fetcher(&cands_no_id, &adapter)
        .unwrap()
        .expect("Crossref exact match via loopback mock");
    srv.join().unwrap();

    assert_eq!(resolved.paper.key.as_str(), "doi:10.1000/waves.2024.9");
    assert_eq!(resolved.basis, MatchBasis::RemoteExactTitleAuthor);
    assert!(!resolved.confirmed);
}
