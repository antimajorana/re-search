//! Explore P6-style PDF & scanned-paper evaluation helpers (F14a + F4 quote verification).

use ledger::{sha256_hex, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::lines_sent::{ContextSuppliedPayload, SuppliedRange, CONTEXT_SUPPLIED_EVENT};
use research_tools::pdf_match::match_pdf_quote;
use research_tools::pdf_pages::{
    extract_pdf_pages, PageBasis, PdfExtractOptions, DEFAULT_PDF_EXTRACTOR_ID, PAGE_BREAK_CHAR,
};
use research_tools::prior_work_wording::wording::TAG_OCR;
use research_tools::quote_core::{accept_quote, QuoteRejection};
use research_tools::sandbox::WorkspaceSandbox;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;
use tools::EVENT_SCHEMA_VERSION;

/// Ground-truth expectation for one sentence inside a synthetic Explore PDF fixture.
pub struct PdfSentenceExpectation {
    pub quote: &'static str,
    pub expected_page: usize,
    pub expected_basis: PageBasis,
}

/// Measured outcomes of running F14a + F4 quote verification on Explore PDF fixtures.
#[derive(Debug, Default, Clone)]
pub struct PdfEvalMetrics {
    pub sentences_total: u64,
    pub sentences_matched: u64,
    pub wrong_page_chips: u64,
    pub scanned_pages_checked: u64,
    pub scanned_reported_plain_text: u64,
    pub scanned_quotes_total: u64,
    pub scanned_quotes_tagged_ocr: u64,
    pub two_col_wrong_order_lines: u64,
    pub image_only_pages_needing_ocr: u64,
    pub lines_not_sent_rejections: u64,
    pub fabricated_quote_rejections: u64,
}

/// Evaluate one synthetic PDF fixture: F14a extraction, page-basis honesty, F14a `match_pdf_quote`,
/// F4 `accept_quote` positive + negative (`LinesNotSent`, `VerificationFailed`), physical page-chip
/// lookup (`page_map.page_of`, including `\x0c` separator and cross-page distinction), and OCR tagging.
pub fn evaluate_pdf_fixture(
    pdf_path: &Path,
    expected_page_bases: &[PageBasis],
    sentences: &[PdfSentenceExpectation],
    ordered_pairs: &[(&str, &str)],
    metrics: &mut PdfEvalMetrics,
) {
    let raw_bytes = fs::read(pdf_path).expect("read synthetic PDF fixture");
    assert!(
        !raw_bytes.contains(&b'\r'),
        "PDF fixture `{}` must be LF-only",
        pdf_path.display()
    );
    let doc = extract_pdf_pages(&raw_bytes, &PdfExtractOptions::user_supplied())
        .expect("F14a extract_pdf_pages");
    assert_eq!(doc.extractor_id, DEFAULT_PDF_EXTRACTOR_ID);
    assert_eq!(
        doc.page_map.page_count,
        expected_page_bases.len(),
        "page count mismatch for `{}`",
        pdf_path.display()
    );

    for (idx, expected_basis) in expected_page_bases.iter().enumerate() {
        let rec = &doc.page_map.pages[idx];
        assert_eq!(rec.page, idx + 1);
        if expected_basis.is_scan() {
            metrics.scanned_pages_checked += 1;
            if rec.basis == PageBasis::TextLayer {
                metrics.scanned_reported_plain_text += 1;
            }
        }
        assert_eq!(
            rec.basis, *expected_basis,
            "page {} basis mismatch in `{}`",
            rec.page,
            pdf_path.display()
        );
        if *expected_basis == PageBasis::None {
            assert!(rec.basis.needs_ocr());
            assert!(rec.first_line.is_none() && rec.last_line.is_none());
            metrics.image_only_pages_needing_ocr += 1;
        }
    }

    // Verify form-feed page-break line (`\x0c`) maps to `None` and page 1 != page 2 cleanly.
    let lines: Vec<&str> = doc.text.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if *line == PAGE_BREAK_CHAR.to_string() {
            assert_eq!(
                doc.page_map.page_of(idx + 1),
                None,
                "inter-page form-feed line {} must map to None",
                idx + 1
            );
        }
    }
    if let (Some(p1_last), Some(p2_first)) = (
        doc.page_map.pages.first().and_then(|p| p.last_line),
        doc.page_map.pages.get(1).and_then(|p| p.first_line),
    ) {
        assert_eq!(doc.page_map.page_of(p1_last), Some(1));
        assert_eq!(doc.page_map.page_of(p2_first), Some(2));
        assert_ne!(doc.page_map.page_of(p1_last), doc.page_map.page_of(p2_first));
    }

    // Check two-column reading order on the extracted page-broken text.
    for &(earlier, later) in ordered_pairs {
        let pos_a = lines.iter().position(|l| l.contains(earlier));
        let pos_b = lines.iter().position(|l| l.contains(later));
        match (pos_a, pos_b) {
            (Some(a), Some(b)) if a < b => {}
            _ => metrics.two_col_wrong_order_lines += 1,
        }
    }

    // Ingest through real `source.ingested` and verify quotes through F4 `accept_quote` + F14a `match_pdf_quote`.
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let sid = "s-eval-pdf";
    let file_name = pdf_path.file_name().unwrap().to_str().unwrap();
    let ing = ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id: sid, file_name, origin: "eval_pdf_fixture", raw_bytes: &raw_bytes,
            source_id_hint: None, section_filter: None, title: None,
        },
    )
    .expect("ingest PDF into session");

    let text_rel = ing
        .primary_path
        .clone()
        .or_else(|| ing.files.first().cloned())
        .expect("PDF ingestion must emit extracted reading file path");
    let reading_abs = data_dir.join("sessions").join(sid).join("sources").join(&text_rel);
    let reading_bytes = fs::read(&reading_abs).expect("read ingested extracted PDF text");
    let file_sha256 = sha256_hex(&reading_bytes);
    let reading_text = String::from_utf8(reading_bytes).expect("UTF-8 extracted PDF text");
    let reading_lines: Vec<&str> = reading_text.lines().collect();
    let total_lines = reading_lines.len().max(1);

    // Record F4 `context.supplied` for two callers:
    // - `caller_full`: credited for `1..=total_lines`
    // - `caller_partial`: credited only for `1..=1` (negative Rule C2 `LinesNotSent` test)
    let caller_full = "explore:reader:eval";
    let caller_partial = "explore:reader:partial";
    for (caller, end_line) in [(caller_full, total_lines), (caller_partial, 1usize)] {
        let payload = ContextSuppliedPayload::new(
            "pdf-eval-run",
            caller,
            vec![SuppliedRange {
                source_id: ing.source_id.clone(),
                path: text_rel.clone(),
                file_sha256: file_sha256.clone(),
                start_line: 1,
                end_line,
            }],
        )
        .expect("valid context.supplied payload");
        let val = serde_json::to_value(&payload).unwrap();
        ledger
            .lock()
            .unwrap()
            .append(sid, CONTEXT_SUPPLIED_EVENT, EVENT_SCHEMA_VERSION, &val)
            .expect("append context.supplied");
    }

    let events = ledger.lock().unwrap().read_session(sid).unwrap();
    assert!(events.iter().any(|e| e.kind == CONTEXT_SUPPLIED_EVENT));

    // Negative Rule C1 test: fabricated quote must fail with `QuoteRejection::VerificationFailed`.
    let fab = accept_quote(
        &sandbox, sid, &events, None, Some("pdf-eval-run"), caller_full,
        &ing.source_id, &text_rel, 1, total_lines.min(3),
        "fabricated sentence completely absent from this synthetic pdf",
    );
    assert!(matches!(fab, Err(QuoteRejection::VerificationFailed(_))));
    metrics.fabricated_quote_rejections += 1;

    let mut tested_not_sent = false;
    for s in sentences {
        metrics.sentences_total += 1;
        // Production F14a `match_pdf_quote` must agree on physical page and page basis.
        let pm = match_pdf_quote(&reading_text, &doc.page_map, s.quote)
            .unwrap_or_else(|| panic!("F14a match_pdf_quote missed `{}` in `{}`", s.quote, pdf_path.display()));
        assert_eq!(pm.start_page, Some(s.expected_page));
        assert_eq!(pm.start_page_basis, Some(s.expected_basis));

        // Negative Rule C2 test: quoting a real line > 1 with `caller_partial` (only line 1 sent) must return `LinesNotSent`.
        if !tested_not_sent && pm.start_line > 1 {
            let unsent = accept_quote(
                &sandbox, sid, &events, None, Some("pdf-eval-run"), caller_partial,
                &ing.source_id, &text_rel, pm.start_line, pm.end_line, s.quote,
            );
            assert!(
                matches!(unsent, Err(QuoteRejection::LinesNotSent { .. })),
                "expected LinesNotSent for caller_partial on line {}, got {unsent:?}",
                pm.start_line
            );
            metrics.lines_not_sent_rejections += 1;
            tested_not_sent = true;
        }

        let verified = accept_quote(
            &sandbox, sid, &events, None, Some("pdf-eval-run"), caller_full,
            &ing.source_id, &text_rel, pm.start_line, pm.end_line, s.quote,
        );
        match verified {
            Ok(m) => {
                metrics.sentences_matched += 1;
                let resolved_page = doc.page_map.page_of(m.quote_ref.start_line);
                if resolved_page != Some(s.expected_page) {
                    metrics.wrong_page_chips += 1;
                }
                if s.expected_basis.is_scan() {
                    metrics.scanned_quotes_total += 1;
                    // `pm.start_page_basis` is computed by F14a `match_pdf_quote` directly from `PageMap`.
                    assert!(pm.start_page_basis.is_some_and(PageBasis::is_scan));
                    let ocr_tag = pm.start_page_basis.filter(|b| b.is_scan()).map(|_| TAG_OCR);
                    if ocr_tag == Some(TAG_OCR) {
                        metrics.scanned_quotes_tagged_ocr += 1;
                    }
                }
            }
            Err(fault) => panic!("F4 accept_quote failed on `{}` in `{}`: {fault}", s.quote, pdf_path.display()),
        }
    }
    assert!(tested_not_sent, "expected at least one line > 1 sentence to verify Rule C2 LinesNotSent");
}
