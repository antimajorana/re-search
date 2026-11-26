//! Unit tests for `pdf_pages.rs` (PageMap lookup, sanitization, extractor id dispatch,
//! determinism, and cooperative job cancellation).

use super::*;
use std::sync::atomic::AtomicBool;

fn dummy_metrics() -> PageQualityMetrics {
    PageQualityMetrics {
        char_count: 42,
        alpha_count: 35,
        word_count: 7,
        image_coverage_pct: 0,
        has_invisible_text: false,
        width_pt: 612,
        height_pt: 792,
    }
}

#[test]
fn page_map_lookup_handles_boundaries_separators_and_empty_pages() {
    let map = PageMap {
        page_count: 3,
        pages: vec![
            PageRecord {
                page: 1,
                basis: PageBasis::TextLayer,
                first_line: Some(1),
                last_line: Some(3),
                metrics: dummy_metrics(),
                reason: None,
            },
            PageRecord {
                page: 2,
                basis: PageBasis::None,
                first_line: None,
                last_line: None,
                metrics: dummy_metrics(),
                reason: Some("page has no extractable text".into()),
            },
            PageRecord {
                page: 3,
                basis: PageBasis::ScanTextLayer,
                first_line: Some(6),
                last_line: Some(8),
                metrics: dummy_metrics(),
                reason: Some("full-page image with text layer".into()),
            },
        ],
    };

    assert_eq!(map.page_of(0), None);
    assert_eq!(map.page_of(1), Some(1));
    assert_eq!(map.page_of(3), Some(1));
    assert_eq!(map.page_of(4), None); // separator line
    assert_eq!(map.page_of(5), None); // empty page 2 / separator
    assert_eq!(map.page_of(6), Some(3));
    assert_eq!(map.page_of(8), Some(3));
    assert_eq!(map.page_of(9), None);
    assert_eq!(map.page_lines(), vec![(1, 1, 3), (3, 6, 8)]);
    assert!(map.has_scanned_pages());
    assert_eq!(map.pages_needing_ocr(), vec![2]);
}

#[test]
fn sanitize_scrubs_nul_crlf_and_embedded_form_feeds() {
    let dirty = "  alpha\0beta\r\ngamma\rdelta\x0cepsilon  \n";
    let clean = sanitize_page_text(dirty);
    assert_eq!(clean, "alphabeta\ngamma\ndelta\nepsilon");
    assert!(!clean.contains('\0'));
    assert!(!clean.contains('\r'));
    assert!(!clean.contains(PAGE_BREAK_CHAR));
}

fn minimal_one_page_pdf(line: &str) -> Vec<u8> {
    use std::io::Write;
    let stream = format!("BT /F1 12 Tf 72 720 Td ({line}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [5 0 R] /Count 1 >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objs.len());
    for (idx, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj\n{body}\nendobj", idx + 1).unwrap();
    }
    let xref = out.len();
    writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objs.len() + 1).unwrap();
    for off in offsets {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF", objs.len() + 1).unwrap();
    out
}

#[test]
fn unknown_extractor_id_and_precancelled_job_fail_cleanly() {
    let err = extract_pdf_pages_by_id("unknown/0.0", b"%PDF-1.4\n", &PdfExtractOptions::default());
    assert!(matches!(err, Err(PdfError::Malformed(_))));

    let minimal = minimal_one_page_pdf("Deterministic extraction test");

    let cancelled_opts = PdfExtractOptions {
        cancel: Some(Arc::new(AtomicBool::new(true))),
        ..PdfExtractOptions::default()
    };
    assert_eq!(
        extract_pdf_pages(&minimal, &cancelled_opts),
        Err(PdfError::Cancelled)
    );

    let first = extract_pdf_pages(&minimal, &PdfExtractOptions::default()).unwrap();
    let second = extract_pdf_pages(&minimal, &PdfExtractOptions::default()).unwrap();
    assert_eq!(first, second);
    assert_eq!(PdfExtractOptions::user_supplied().max_bytes, MAX_USER_PDF_BYTES);
}
