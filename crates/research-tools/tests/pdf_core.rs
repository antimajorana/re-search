//! Integration tests for package F14a (PDF text core): legacy pre-F14a receipt compatibility,
//! form-feed reading files & page map, scan detection, background extraction jobs, page ranges,
//! and malformed/encrypted/oversized safety.

mod pdf_support;

use ledger::Ledger;
use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::ingest::{ingest_bytes, ingest_pdf_pages, IngestRequest};
use research_tools::paper_lines::read_lines;
use research_tools::pdf::{extract_pdf_bytes, verified_page_lines, PdfError, PdfExtractStatus};
use research_tools::pdf_match::match_pdf_quote;
use research_tools::pdf_pages::{
    extract_pdf_pages, page_of_line, PageBasis, PdfExtractJob, PdfExtractLayout,
    PdfExtractOptions, PdfExtractPlain, PdfExtractStage, PdfExtractor, DEFAULT_PDF_EXTRACTOR_ID,
    PAGE_BREAK_CHAR,
};
use research_tools::ResearchEnv;
use serde_json::json;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn legacy_base_commit_receipts_keep_opening_verifying_and_matching_quotes() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-legacy-pdf";

    let pdf_bytes = build_pdf(
        &[
            PageSpec::single_column(&["1. INTRODUCTION", "Legacy opening theorem on page one."]),
            PageSpec::single_column(&["2. PROOF", "Legacy closing estimate on page two."]),
        ],
        &PdfBuildOptions::default(),
    );

    // Reproduce the exact base-commit reading file format (`--- [Page N] ---\n...`) produced at
    // base commit e1517477 before F14a.
    let plain_pages = PdfExtractPlain
        .extract(&pdf_bytes, &PdfExtractOptions::default())
        .unwrap();
    let p1 = plain_pages.text.split(PAGE_BREAK_CHAR).next().unwrap().trim();
    let p2 = plain_pages.text.split(PAGE_BREAK_CHAR).nth(1).unwrap().trim();
    let legacy_text = format!("--- [Page 1] ---\n{p1}\n\n--- [Page 2] ---\n{p2}");
    let p1_last = 1 + p1.lines().count();
    let p2_first = p1_last + 2;
    let p2_last = p2_first + p2.lines().count();
    let legacy_page_lines = vec![(1usize, 1usize, p1_last), (2usize, p2_first, p2_last)];

    let raw_blob = ledger.put_blob(&pdf_bytes).unwrap();
    let text_sha = ledger::sha256_hex(legacy_text.as_bytes());

    // Subcase A: pre-F14a receipt WITH pdf_page_lines (no pdf_extractor, no pdf_pages).
    let src_with_lines = "legacy-a-1234567890abcdef";
    let path_a = format!("{src_with_lines}/paper.pdf.extracted.txt");
    let src_dir_a = tmp.path().join("sessions").join(sid).join("sources").join(src_with_lines);
    fs::create_dir_all(&src_dir_a).unwrap();
    fs::write(src_dir_a.join("paper.pdf.extracted.txt"), &legacy_text).unwrap();

    let receipt_a = json!({
        "source_id": src_with_lines,
        "origin": "upload",
        "title": "Legacy Paper",
        "format": "pdf",
        "status": "ok",
        "bytes": pdf_bytes.len(),
        "raw_blob": raw_blob,
        "primary_path": path_a,
        "assembled_path": null,
        "files": [path_a],
        "files_sha256": { path_a.clone(): text_sha },
        "page_count": 2,
        "pdf_page_lines": legacy_page_lines,
        "arxiv_cover_id": null
    });
    ledger.append(sid, "source.ingested", 1, &receipt_a).unwrap();

    // Subcase B: older pre-F14a receipt WITHOUT pdf_page_lines (reconstructs from raw_blob).
    let mut receipt_b = receipt_a.clone();
    receipt_b.as_object_mut().unwrap().remove("pdf_page_lines");
    let verified_a = verified_page_lines(&receipt_a, || Err("should not load raw".into())).unwrap();
    let verified_b = verified_page_lines(&receipt_b, || Ok(pdf_bytes.clone())).unwrap();
    assert_eq!(verified_a, legacy_page_lines);
    assert_eq!(verified_b, legacy_page_lines);

    // read_paper_lines opens the pre-F14a reading file, verifies hash, and reports page ranges.
    let read_out = read_lines(
        &env,
        &ledger,
        sid,
        &json!({ "source_id": src_with_lines, "start_line": 1, "end_line": p2_last }),
        14_000,
    )
    .unwrap();
    assert_eq!(read_out["a_meta"]["file_sha256"], text_sha);
    assert_eq!(read_out["a_meta"]["pages"].as_array().unwrap().len(), 2);

    // Quotes/anchors recorded against the pre-F14a reading file still verify via investigation::record.
    ledger.append(sid, "investigation.started", 1, &json!({})).unwrap();
    ledger
        .append(sid, "investigation.paper", 1, &json!({ "source_id": src_with_lines }))
        .unwrap();
    ledger
        .append(sid, "investigation.mapping_requested", 1, &json!({ "source_id": src_with_lines }))
        .unwrap();
    let quote_line = legacy_text
        .lines()
        .position(|l| l.contains("Legacy closing estimate on page two."))
        .unwrap()
        + 1;
    let map = json!({
        "title": "Legacy Map",
        "source_id": src_with_lines,
        "coverage": "paper",
        "limitations": [],
        "nodes": [{
            "id": "res1",
            "kind": "result",
            "main": true,
            "sentence": "Closing estimate on page two.",
            "anchors": [{
                "kind": "passage",
                "source_id": src_with_lines,
                "path": path_a,
                "start_line": quote_line,
                "end_line": quote_line,
                "excerpt": "Legacy closing estimate on page two."
            }]
        }],
        "edges": []
    });
    assert_eq!(
        research_tools::investigation::record(&env, &ledger, sid, &map).unwrap()["status"],
        "recorded"
    );

    ledger.verify_session(sid).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn form_feed_reading_file_page_map_and_pinned_extractor_in_receipt() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-f14a-ff";

    let pdf_bytes = build_pdf(
        &[
            PageSpec::single_column(&["Page one alpha line.", "Page one beta line."]),
            PageSpec::full_page_image_only(),
            PageSpec::single_column(&["Page three gamma line."]),
        ],
        &PdfBuildOptions::default(),
    );

    let ing = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "mixed.pdf",
            origin: "upload",
            raw_bytes: &pdf_bytes,
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let ledger = db.lock().unwrap();
    let events = ledger.read_session(sid).unwrap();
    let receipt = &events[0].payload;

    assert_eq!(receipt["pdf_extractor"].as_str(), Some(DEFAULT_PDF_EXTRACTOR_ID));
    let pdf_pages = receipt["pdf_pages"].as_array().expect("pinned pdf_pages");
    assert_eq!(pdf_pages.len(), 3);
    assert_eq!(pdf_pages[0]["basis"], "text_layer");
    assert_eq!(pdf_pages[1]["basis"], "none");
    assert_eq!(pdf_pages[2]["basis"], "text_layer");

    let primary = ing.primary_path.as_deref().unwrap();
    let disk_text = fs::read_to_string(
        tmp.path().join("sessions").join(sid).join("sources").join(primary),
    )
    .unwrap();
    assert_eq!(disk_text.matches(PAGE_BREAK_CHAR).count(), 2);

    let ranges = verified_page_lines(receipt, || Ok(pdf_bytes.clone())).unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].0, 1);
    assert_eq!(ranges[1].0, 3);
    assert_eq!(page_of_line(&ranges, ranges[0].1), Some(1));
    assert_eq!(page_of_line(&ranges, ranges[0].2 + 1), None);
    assert_eq!(page_of_line(&ranges, ranges[1].1), Some(3));

    let mut stripped = receipt.clone();
    stripped.as_object_mut().unwrap().remove("pdf_page_lines");
    assert_eq!(verified_page_lines(&stripped, || Ok(pdf_bytes.clone())).unwrap(), ranges);
}

#[test]
fn scan_detection_classifies_all_page_types_and_never_marks_scans_as_text_layer() {
    let pdf_bytes = build_pdf(
        &[
            PageSpec::single_column(&[
                "Section 1. Digital text layer",
                "This page has clean embedded digital text and no scan image.",
            ]),
            PageSpec::blank(),
            PageSpec::full_page_image_only(),
            PageSpec::scan_with_text(&["14"], false),
            PageSpec::scan_with_text(
                &[
                    "Theorem 2. Publisher scan with invisible OCR layer.",
                    "The decay estimate holds uniformly in volume.",
                ],
                true,
            ),
            PageSpec::scan_with_text(
                &[
                    "Proposition 3. Publisher scan with standard text over image.",
                    "Every spectral gap stays bounded away from zero.",
                ],
                false,
            ),
            PageSpec::tiled_scan_with_text(&[
                "Lemma 4. Tiled four-strip scan with embedded OCR text.",
                "Strip coverage sums above seventy percent of the page.",
            ]),
        ],
        &PdfBuildOptions::default(),
    );

    let doc = extract_pdf_pages(&pdf_bytes, &PdfExtractOptions::default()).unwrap();
    let bases: Vec<PageBasis> = doc.page_map.pages.iter().map(|p| p.basis).collect();
    assert_eq!(
        bases,
        vec![
            PageBasis::TextLayer,
            PageBasis::None,
            PageBasis::None,
            PageBasis::None,
            PageBasis::ScanTextLayer,
            PageBasis::ScanTextLayer,
            PageBasis::ScanTextLayer,
        ]
    );
    assert!(doc.page_map.has_scanned_pages());
    assert_eq!(doc.page_map.pages_needing_ocr(), vec![2, 3, 4]);
    assert!(doc.page_map.record(5).unwrap().metrics.has_invisible_text);
    assert!(doc.page_map.record(7).unwrap().metrics.image_coverage_pct >= 70);
}

#[test]
fn background_extract_job_progress_ingest_and_page_range_on_large_pdf() {
    let pages: Vec<PageSpec> = (1..=5)
        .map(|i| PageSpec::single_column(&[&format!("Chapter {i}. Synthetic page number {i}.")]))
        .collect();
    let pdf_bytes = build_pdf(&pages, &PdfBuildOptions::default());

    let job = PdfExtractJob::start("job-pdf-1", pdf_bytes.clone(), PdfExtractOptions::default());
    let doc = job.join().unwrap();
    assert_eq!(doc.page_count, 5);
    assert_eq!(doc.status, PdfExtractStatus::Ok);
    assert_eq!(doc.text.matches(PAGE_BREAK_CHAR).count(), 4);

    let sync_job = PdfExtractJob::run_sync(
        "job-pdf-sync",
        &pdf_bytes,
        &PdfExtractOptions::default(),
        &PdfExtractLayout,
    );
    let snap = sync_job.snapshot();
    assert_eq!(snap.pages_done, 5);
    assert_eq!(snap.total_pages, 5);
    assert!(matches!(snap.stage, PdfExtractStage::Done { page_count: 5, .. }));
    assert!(matches!(snap.history[0], PdfExtractStage::Queued));
    assert!(matches!(snap.history[1], PdfExtractStage::Loading));
    let page_events = snap
        .history
        .iter()
        .filter(|s| matches!(s, PdfExtractStage::ExtractingPage { .. }))
        .count();
    assert_eq!(page_events, 5);
    assert!(sync_job.is_finished());

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let ing = ingest_pdf_pages(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: "s-bg-job",
            file_name: "five_pages.pdf",
            origin: "upload",
            raw_bytes: &pdf_bytes,
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
        doc,
    )
    .unwrap();
    assert_eq!(ing.page_count, Some(5));
    let guard = db.lock().unwrap();
    guard.verify_session("s-bg-job").unwrap();
    assert_eq!(guard.verify_all().unwrap().unreferenced_blobs, 0);

    // 405-page PDF: default cap refuses it, user-picked page range 401..=403 succeeds cleanly.
    let mut big_pages: Vec<PageSpec> = (0..400).map(|_| PageSpec::blank()).collect();
    for p in 401..=405 {
        big_pages.push(PageSpec::single_column(&[&format!(
            "Appendix page {p} content."
        )]));
    }
    let big_pdf = build_pdf(&big_pages, &PdfBuildOptions::default());
    assert_eq!(
        extract_pdf_bytes(&big_pdf),
        Err(PdfError::TooManyPages { pages: 405, max: 400 })
    );

    let ranged = extract_pdf_pages(
        &big_pdf,
        &PdfExtractOptions::user_supplied().with_page_range(401, 403),
    )
    .unwrap();
    assert_eq!(ranged.page_count, 405);
    assert_eq!(ranged.page_map.pages.len(), 3);
    assert_eq!(ranged.page_map.pages[0].page, 401);
    assert_eq!(ranged.page_map.pages[2].page, 403);
    let m = match_pdf_quote(&ranged.text, &ranged.page_map, "Appendix page 402 content.")
        .expect("page 402 quote match");
    assert_eq!(m.start_page, Some(402));
}

#[test]
fn malformed_encrypted_and_huge_inputs_are_refused_without_panic() {
    let opts = PdfExtractOptions::default();

    assert!(matches!(extract_pdf_pages(b"", &opts), Err(PdfError::Malformed(_))));
    assert!(matches!(
        extract_pdf_pages(b"<html><body>404 Not Found</body></html>", &opts),
        Err(PdfError::Malformed(_))
    ));
    assert!(matches!(
        extract_pdf_pages(b"%PDF-1.4\ntruncated garbage", &opts),
        Err(PdfError::Malformed(_))
    ));
    let broken_root = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< >>\n%%EOF";
    assert!(extract_pdf_pages(broken_root, &opts).is_err());

    let encrypted = build_pdf(
        &[PageSpec::single_column(&["Secret page text"])],
        &PdfBuildOptions {
            encrypted_stub: true,
            ..PdfBuildOptions::default()
        },
    );
    assert_eq!(extract_pdf_pages(&encrypted, &opts), Err(PdfError::Encrypted));

    let mut small_cap = PdfExtractOptions::default();
    small_cap.max_bytes = 128;
    let valid = build_pdf(
        &[PageSpec::single_column(&["Hello bounded world"])],
        &PdfBuildOptions::default(),
    );
    assert!(matches!(
        extract_pdf_pages(&valid, &small_cap),
        Err(PdfError::TooLarge { max: 128, .. })
    ));

    let mut tiny_out = PdfExtractOptions::default();
    tiny_out.max_extracted_bytes = 4;
    assert_eq!(extract_pdf_pages(&valid, &tiny_out), Err(PdfError::OutputTooLarge));

    assert!(matches!(
        extract_pdf_pages(&valid, &PdfExtractOptions::default().with_page_range(0, 1)),
        Err(PdfError::InvalidPageRange { .. })
    ));
    assert!(matches!(
        extract_pdf_pages(&valid, &PdfExtractOptions::default().with_page_range(2, 3)),
        Err(PdfError::InvalidPageRange { .. })
    ));
}
