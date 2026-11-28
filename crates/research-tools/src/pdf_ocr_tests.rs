//! Deterministic unit tests for F14b OCR pipeline, background OCR job, 64 MiB scan
//! ingestion, F10 Library caching, and quote/display honesty.

use super::*;
use crate::ingest_ocr::{
    ingest_pdf_with_ocr, pin_ocr_outcome_to_session, OcrIngestMeta, OcrIngestRequest,
};
use crate::library::Library;
use crate::library_schema::PageBasis as LibPageBasis;
use crate::paper_key::PaperKey;
use crate::pdf::verified_page_lines;
use crate::pdf_ocr_job::{PdfOcrJob, PdfOcrStage};
use crate::pdf_ocr_models::{OcrEnvProbe, OcrLocalConfig, OcrModelProvenance};
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use crate::pdf_render::{encode_grayscale_png, MapPageRenderer};
use crate::quote_core::verify_quote;
use crate::sandbox::WorkspaceSandbox;
use crate::skeleton::{load_ingested_pdf_source, match_pdf_quote_for_display};
use ledger::Ledger;
use std::io::Write;
use tempfile::tempdir;

fn stream_obj(dict_body: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict_body} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn build_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj", idx + 1).unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1).unwrap();
    for off in offsets {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF", objects.len() + 1).unwrap();
    out
}

/// 4-page synthetic PDF: p1 TextLayer, p2 None, p3 ScanTextLayer (`3 Tr`), p4 None.
fn build_mixed_four_page_pdf() -> Vec<u8> {
    let p1 = b"BT /F1 12 Tf 72 720 Td (Theorem 1.1 digital text layer statement on page one.) Tj ET";
    let img_only = b"q 612 0 0 792 0 0 cm /Im0 Do Q";
    let p3 = concat!(
        "q 612 0 0 792 0 0 cm /Im0 Do Q\n",
        "BT 3 Tr /F1 12 Tf 72 720 Td (Proposition 3.2 embedded publisher scan layer on page three.) Tj ET"
    )
    .as_bytes();
    let img = stream_obj(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0xff, 0x00, 0x00, 0xff],
    );
    let objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [5 0 R 7 0 R 9 0 R 11 0 R] /Count 4 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        img,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 6 0 R >>".to_vec(),
        stream_obj("", p1),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 8 0 R >>".to_vec(),
        stream_obj("", img_only),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> /XObject << /Im0 4 0 R >> >> /Contents 10 0 R >>".to_vec(),
        stream_obj("", p3),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 12 0 R >>".to_vec(),
        stream_obj("", img_only),
    ];
    build_pdf(&objs)
}

fn mixed_renderer() -> MapPageRenderer {
    let png = encode_grayscale_png(4, 4, &[210u8; 16]).unwrap();
    let mut pages = BTreeMap::new();
    pages.insert(2, png.clone());
    pages.insert(4, png);
    MapPageRenderer::new(pages)
}

fn dynamic_model_provenance() -> OcrModelProvenance {
    let synthetic_bytes = b"synthetic-eng-traineddata-v1";
    OcrModelProvenance {
        lang: "eng".to_string(),
        sha256: ledger::sha256_hex(synthetic_bytes),
        bytes: synthetic_bytes.len() as u64,
    }
}

fn mixed_fake_runner() -> FakeOcrPageRunner {
    let mut outputs = BTreeMap::new();
    outputs.insert(2, Ok("Lemma 2.1 scanned proof text.\r\nSecond OCR line on page two.\0".into()));
    outputs.insert(4, Ok("Corollary 4.3 closing remark on page four.".into()));
    FakeOcrPageRunner::new(outputs)
        .with_engine_id("tesseract/5.3.4")
        .with_provenance(vec![dynamic_model_provenance()])
}

fn page_slice_text(doc: &PdfDocumentPages, page: usize) -> String {
    let rec = doc.page_map.record(page).unwrap();
    let lines: Vec<&str> = doc.text.split('\n').collect();
    lines[(rec.first_line.unwrap() - 1)..rec.last_line.unwrap()].join("\n")
}

#[test]
fn mixed_pdf_ocrs_only_none_pages_and_preserves_text_and_scan_text_layers() {
    let pdf = build_mixed_four_page_pdf();
    let base = extract_pdf_pages(&pdf, &PdfExtractOptions::user_supplied()).unwrap();
    let base_bases: Vec<PageBasis> = base.page_map.pages.iter().map(|r| r.basis).collect();
    assert_eq!(base_bases, vec![PageBasis::TextLayer, PageBasis::None, PageBasis::ScanTextLayer, PageBasis::None]);
    let (base_p1, base_p3) = (page_slice_text(&base, 1), page_slice_text(&base, 3));

    let renderer = mixed_renderer();
    let runner = mixed_fake_runner();
    let opts = PdfOcrOptions::default();
    assert_eq!(opts.extract_opts.max_bytes, MAX_USER_PDF_BYTES);

    let mut progress = Vec::new();
    let outcome = extract_and_ocr_pdf_pages_with_progress(&pdf, &opts, &renderer, Some(&runner), &mut |p| {
        progress.push(p);
    })
    .unwrap();

    assert_eq!(runner.calls(), vec![2, 4]);
    assert_eq!(progress, vec![
        PdfOcrProgress { page: 2, done: 1, total_ocr_pages: 2 },
        PdfOcrProgress { page: 4, done: 2, total_ocr_pages: 2 },
    ]);
    assert!(outcome.ocr_available);
    assert_eq!(outcome.ocr_status, "applied");
    assert_eq!(outcome.ocr_message, None);
    assert_eq!(outcome.document.status, PdfExtractStatus::Ok);

    let final_bases: Vec<PageBasis> = outcome.document.page_map.pages.iter().map(|r| r.basis).collect();
    assert_eq!(final_bases, vec![PageBasis::TextLayer, PageBasis::Ocr, PageBasis::ScanTextLayer, PageBasis::Ocr]);
    assert_eq!(page_slice_text(&outcome.document, 1), base_p1);
    assert_eq!(page_slice_text(&outcome.document, 3), base_p3);
    assert_eq!(page_slice_text(&outcome.document, 2), "Lemma 2.1 scanned proof text.\nSecond OCR line on page two.");
    assert_eq!(page_slice_text(&outcome.document, 4), "Corollary 4.3 closing remark on page four.");

    let mut form_feeds = 0usize;
    for (idx, line) in outcome.document.text.split('\n').enumerate() {
        let ln = idx + 1;
        if line == PAGE_BREAK_CHAR.to_string() {
            form_feeds += 1;
            assert_eq!(outcome.document.page_map.page_of(ln), None);
        } else {
            let p = outcome.document.page_map.page_of(ln).unwrap();
            assert!((1..=4).contains(&p));
        }
    }
    assert_eq!(form_feeds, 3);

    let prov = outcome.provenance.unwrap();
    assert_eq!(prov.ocr_engine, "tesseract/5.3.4");
    assert_eq!(prov.pages_ocr_applied, vec![2, 4]);
    assert_eq!(prov.pages_ocr_skipped, vec![1, 3]);
    assert!(prov.pages_ocr_failed.is_empty());
    assert_eq!(prov.ocr_models, vec![dynamic_model_provenance()]);
}

#[test]
fn missing_runner_keeps_none_pages_honest_with_install_prompt() {
    let pdf = build_mixed_four_page_pdf();
    let renderer = mixed_renderer();
    let outcome = extract_and_ocr_pdf_pages(&pdf, &PdfOcrOptions::default(), &renderer, None).unwrap();

    assert!(!outcome.ocr_available);
    assert_eq!(outcome.ocr_status, "unavailable");
    assert_eq!(outcome.ocr_message.as_deref(), Some("Install Tesseract to enable OCR"));
    assert!(outcome.provenance.is_none());
    assert_eq!(outcome.document.page_map.pages_needing_ocr(), vec![2, 4]);
    assert_eq!(outcome.document.page_map.record(2).unwrap().basis, PageBasis::None);
    assert_eq!(outcome.document.page_map.record(4).unwrap().basis, PageBasis::None);

    let err = LocalTesseractPageRunner::from_config_with_probe(
        &OcrLocalConfig::default(),
        &OcrEnvProbe::default(),
    )
    .unwrap_err();
    assert_eq!(err, "Install Tesseract to enable OCR");
}

#[test]
fn disabled_config_never_calls_runner_and_keeps_none_pages() {
    let pdf = build_mixed_four_page_pdf();
    let renderer = mixed_renderer();
    let runner = mixed_fake_runner();
    let mut opts = PdfOcrOptions::default();
    opts.local_config.enabled = false;

    let outcome = extract_and_ocr_pdf_pages(&pdf, &opts, &renderer, Some(&runner)).unwrap();
    assert!(outcome.ocr_available);
    assert_eq!(outcome.ocr_status, "disabled");
    assert!(runner.calls().is_empty());
    assert!(outcome.provenance.is_none());
    assert_eq!(outcome.document.page_map.pages_needing_ocr(), vec![2, 4]);
}

#[test]
fn ocr_job_run_sync_background_and_cancellation() {
    let pdf = build_mixed_four_page_pdf();
    let renderer = mixed_renderer();
    let runner = mixed_fake_runner();

    let sync_job = PdfOcrJob::run_sync("ocr-sync-1", &pdf, &PdfOcrOptions::default(), &renderer, Some(&runner));
    assert!(sync_job.is_finished());
    let snap = sync_job.snapshot();
    assert_eq!((snap.job_id.as_str(), snap.pages_done, snap.total_ocr_pages), ("ocr-sync-1", 2, 2));
    assert_eq!(snap.history[0], PdfOcrStage::Queued);
    assert_eq!(snap.history[1], PdfOcrStage::ExtractingText);
    assert!(snap.history.contains(&PdfOcrStage::OcrPage { page: 2, done: 1, total_ocr_pages: 2 }));
    assert!(snap.history.contains(&PdfOcrStage::OcrPage { page: 4, done: 2, total_ocr_pages: 2 }));
    assert!(matches!(
        snap.stage,
        PdfOcrStage::Done { status: PdfExtractStatus::Ok, page_count: 4, pages_ocr_applied: 2, ocr_available: true, .. }
    ));
    assert_eq!(sync_job.join().unwrap().ocr_status, "applied");

    let bg_runner = Arc::new(mixed_fake_runner());
    let bg_calls = bg_runner.calls_handle();
    let bg_job = PdfOcrJob::start(
        "ocr-bg-1",
        pdf.clone(),
        PdfOcrOptions::default(),
        Arc::new(mixed_renderer()),
        Some(bg_runner),
    );
    assert_eq!(bg_job.join().unwrap().ocr_status, "applied");
    assert_eq!(*bg_calls.lock().unwrap(), vec![2, 4]);

    let cancel_opts = PdfOcrOptions {
        cancel: Some(Arc::new(AtomicBool::new(true))),
        ..PdfOcrOptions::default()
    };
    let cancelled_runner = mixed_fake_runner();
    let cancelled_job = PdfOcrJob::run_sync("ocr-cancel-1", &pdf, &cancel_opts, &renderer, Some(&cancelled_runner));
    assert!(cancelled_job.is_finished());
    assert_eq!(cancelled_job.snapshot().stage, PdfOcrStage::Cancelled);
    assert!(cancelled_runner.calls().is_empty());
    assert_eq!(cancelled_job.join(), Err(PdfError::Cancelled));
}

#[test]
fn ingest_pdf_with_ocr_pins_provenance_caches_library_and_verifies_quotes() {
    let tmp = tempdir().unwrap();
    let sandbox = WorkspaceSandbox::new(tmp.path().to_path_buf(), None);
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let lib = Library::open(tmp.path()).unwrap();
    let session = "s-ocr-1";
    let pdf = build_mixed_four_page_pdf();
    let renderer = mixed_renderer();

    let base_req = OcrIngestRequest {
        session_id: session,
        file_name: "scanned-paper.pdf",
        origin: "upload:scanned-paper.pdf",
        raw_bytes: &pdf,
        source_id_hint: Some("scanned-paper"),
        title: Some("Scanned Paper"),
        ocr_opts: PdfOcrOptions::default(),
    };
    let prior = ingest_pdf_with_ocr(&sandbox, &ledger, None, &base_req, &renderer, None).unwrap();
    assert!(!prior.ocr_available);
    assert_eq!(prior.pages_remaining_none, vec![2, 4]);
    let prior_event = ledger.lock().unwrap().read_session(session).unwrap()[0].clone();

    let runner = mixed_fake_runner();
    let ingested = ingest_pdf_with_ocr(&sandbox, &ledger, Some(&lib), &base_req, &renderer, Some(&runner)).unwrap();
    assert_ne!(ingested.source.source_id, prior.source.source_id);
    assert!(ingested.ocr_available);
    assert_eq!(ingested.ocr_status, "applied");
    assert_eq!(ingested.ocr_engine.as_deref(), Some("tesseract/5.3.4"));
    assert_eq!(ingested.ocr_models, vec![dynamic_model_provenance()]);
    assert_eq!(ingested.pages_ocr_applied, vec![2, 4]);
    assert!(ingested.pages_remaining_none.is_empty());

    let events = ledger.lock().unwrap().read_session(session).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], prior_event);

    let receipt = &events[1];
    assert_eq!(receipt.payload["ocr_engine"], "tesseract/5.3.4");
    assert_eq!(receipt.payload["ocr_status"], "applied");
    assert_eq!(receipt.payload["ocr_models"], serde_json::to_value(vec![dynamic_model_provenance()]).unwrap());
    assert_eq!(receipt.payload["pdf_pages"][1]["basis"], "ocr");
    assert_eq!(receipt.payload["pdf_pages"][3]["basis"], "ocr");
    assert_eq!(verified_page_lines(&receipt.payload, || Ok(pdf.clone())).unwrap(), ingested.page_map.page_lines());

    let detail = lib.get_paper(&PaperKey::from_file_bytes(&pdf)).unwrap().unwrap();
    assert_eq!(detail.paper_text.len(), 1);
    assert_eq!(
        detail.paper_text[0].page_basis,
        vec![LibPageBasis::TextLayer, LibPageBasis::Ocr, LibPageBasis::ScanTextLayer, LibPageBasis::Ocr]
    );
    assert!(detail.paper_text[0].extractor.contains("tesseract/5.3.4"));

    let primary_path = ingested.source.primary_path.as_deref().unwrap();
    let p2_rec = ingested.page_map.record(2).unwrap();
    let verified = verify_quote(
        &sandbox, session, &events, &ingested.source.source_id, primary_path,
        p2_rec.first_line.unwrap(), p2_rec.last_line.unwrap(), "Lemma 2.1 scanned proof text",
    )
    .unwrap();
    assert_eq!(verified.quote_ref.source_id, ingested.source.source_id);

    let loaded = load_ingested_pdf_source(&sandbox, session, receipt).unwrap();
    let display = match_pdf_quote_for_display(
        &loaded.primary_path, &loaded.text, &loaded.page_map, "Lemma 2.1 scanned proof text",
    )
    .unwrap();
    assert_eq!(display.display_label, "matched in OCR text, p. 2");
    assert!(display.is_ocr);
    assert!(!display.render_math);
    assert_eq!(display.start_page, 2);
    assert_eq!(display.basis, PageBasis::Ocr);

    let precomputed = extract_and_ocr_pdf_pages(&pdf, &PdfOcrOptions::default(), &renderer, Some(&mixed_fake_runner())).unwrap();
    let pinned = pin_ocr_outcome_to_session(
        &sandbox,
        &ledger,
        Some(&lib),
        &OcrIngestMeta {
            session_id: session,
            file_name: "pinned-bg.pdf",
            origin: "job:ocr-bg-1",
            source_id_hint: Some("pinned-bg"),
            title: None,
        },
        &pdf,
        precomputed,
    )
    .unwrap();
    assert_eq!(pinned.ocr_status, "applied");
    assert_eq!(pinned.pages_ocr_applied, vec![2, 4]);
}

#[test]
fn empty_pdf_rejected_and_partial_page_range_does_not_overwrite_library_cache() {
    let tmp = tempdir().unwrap();
    let sandbox = WorkspaceSandbox::new(tmp.path().to_path_buf(), None);
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let lib = Library::open(tmp.path()).unwrap();
    let session = "s-ocr-range";
    let pdf = build_mixed_four_page_pdf();
    let renderer = mixed_renderer();
    let runner = mixed_fake_runner();

    let empty_req = OcrIngestRequest {
        session_id: session,
        file_name: "empty.pdf",
        origin: "upload:empty.pdf",
        raw_bytes: &[],
        source_id_hint: None,
        title: None,
        ocr_opts: PdfOcrOptions::default(),
    };
    let err = ingest_pdf_with_ocr(&sandbox, &ledger, Some(&lib), &empty_req, &renderer, Some(&runner)).unwrap_err();
    assert!(err.contains("empty"), "expected empty PDF rejection, got: {err}");

    let partial_opts = PdfOcrOptions {
        extract_opts: PdfExtractOptions::user_supplied().with_page_range(1, 2),
        ..PdfOcrOptions::default()
    };
    let partial_req = OcrIngestRequest {
        session_id: session,
        file_name: "partial.pdf",
        origin: "upload:partial.pdf",
        raw_bytes: &pdf,
        source_id_hint: Some("partial"),
        title: Some("Partial Slice"),
        ocr_opts: partial_opts,
    };
    let partial = ingest_pdf_with_ocr(&sandbox, &ledger, Some(&lib), &partial_req, &renderer, Some(&runner)).unwrap();
    assert_eq!(partial.page_map.pages.len(), 2);
    assert_eq!(partial.page_map.page_count, 4);
    assert!(
        lib.get_paper(&PaperKey::from_file_bytes(&pdf)).unwrap().is_none(),
        "partial page-range OCR must not populate or overwrite full-document library paper_text cache"
    );
}

