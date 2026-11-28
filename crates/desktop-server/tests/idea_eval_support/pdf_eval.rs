//! Explore P6-style PDF & scanned-paper evaluation helpers (F14a + F4 quote verification).

use ledger::{sha256_hex, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::lines_sent::{ContextSuppliedPayload, SuppliedRange, CONTEXT_SUPPLIED_EVENT};
use research_tools::pdf_pages::{
    extract_pdf_pages, PageBasis, PdfExtractOptions, DEFAULT_PDF_EXTRACTOR_ID,
};
use research_tools::prior_work_wording::wording::TAG_OCR;
use research_tools::quote_core::accept_quote;
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
    pub scanned_reported_plain_text: u64,
    pub scanned_quotes_total: u64,
    pub scanned_quotes_tagged_ocr: u64,
    pub two_col_wrong_order_lines: u64,
    pub image_only_pages_needing_ocr: u64,
}

/// Evaluate one synthetic PDF fixture: F14a extraction, page-basis honesty, F4 quote matching
/// with `context.supplied`, physical page-chip lookup (`page_map.page_of`), and OCR tagging.
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
        if expected_basis.is_scan() && rec.basis == PageBasis::TextLayer {
            metrics.scanned_reported_plain_text += 1;
        }
        assert_eq!(
            rec.basis, *expected_basis,
            "page {} basis mismatch in `{}`",
            rec.page,
            pdf_path.display()
        );
        if *expected_basis == PageBasis::None {
            assert!(rec.basis.needs_ocr());
            metrics.image_only_pages_needing_ocr += 1;
        }
    }

    // Check two-column reading order on the extracted page-broken text.
    let lines: Vec<&str> = doc.text.lines().collect();
    for &(earlier, later) in ordered_pairs {
        let pos_a = lines.iter().position(|l| l.contains(earlier));
        let pos_b = lines.iter().position(|l| l.contains(later));
        match (pos_a, pos_b) {
            (Some(a), Some(b)) if a < b => {}
            _ => metrics.two_col_wrong_order_lines += 1,
        }
    }

    // Ingest through the real session ingestion path (`source.ingested`) and verify quotes
    // through F4 `accept_quote` after recording `context.supplied`.
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
            session_id: sid,
            file_name,
            origin: "eval_pdf_fixture",
            raw_bytes: &raw_bytes,
            source_id_hint: None,
            section_filter: None,
            title: None,
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

    // Record F4 `context.supplied` so the reader caller is credited for the extracted reading file.
    let caller = "explore:reader:eval";
    {
        let payload = ContextSuppliedPayload::new(
            "pdf-eval-run",
            caller,
            vec![SuppliedRange {
                source_id: ing.source_id.clone(),
                path: text_rel.clone(),
                file_sha256,
                start_line: 1,
                end_line: total_lines,
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

    for s in sentences {
        metrics.sentences_total += 1;
        let line_1based = find_quote_line(&reading_lines, s.quote)
            .unwrap_or_else(|| panic!("quote `{}` not found in `{}`", s.quote, pdf_path.display()));
        let verified = accept_quote(
            &sandbox,
            sid,
            &events,
            None,
            Some("pdf-eval-run"),
            caller,
            &ing.source_id,
            &text_rel,
            line_1based,
            (line_1based + 1).min(total_lines),
            s.quote,
        );
        match verified {
            Ok(m) => {
                metrics.sentences_matched += 1;
                let resolved_page = doc.page_map.page_of(m.quote_ref.start_line);
                if resolved_page != Some(s.expected_page) {
                    metrics.wrong_page_chips += 1;
                }
                let rec = doc
                    .page_map
                    .record_of_line(m.quote_ref.start_line)
                    .expect("page record for matched line");
                assert_eq!(rec.basis, s.expected_basis);
                if s.expected_basis.is_scan() {
                    metrics.scanned_quotes_total += 1;
                    // Rule C8 / 00-overview.md 8.3 Q3: scanned quote carries OCR tag and disables math rendering.
                    let render_math = !rec.basis.is_scan();
                    assert!(!render_math, "scanned page quote must disable math rendering");
                    let ocr_tag = if rec.basis.is_scan() { Some(TAG_OCR) } else { None };
                    if ocr_tag == Some(TAG_OCR) {
                        metrics.scanned_quotes_tagged_ocr += 1;
                    }
                }
            }
            Err(fault) => {
                panic!(
                    "F4 accept_quote failed on `{}` in `{}`: {fault}",
                    s.quote,
                    pdf_path.display(),
                );
            }
        }
    }
}

fn find_quote_line(lines: &[&str], needle: &str) -> Option<usize> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let target = norm(needle);
    for (idx, line) in lines.iter().enumerate() {
        if norm(line).contains(&target) {
            return Some(idx + 1);
        }
        if idx + 1 < lines.len() {
            let joined = format!("{} {}", line, lines[idx + 1]);
            if norm(&joined).contains(&target) {
                return Some(idx + 1);
            }
        }
    }
    None
}
