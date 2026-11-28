//! Integration tests for Explore X10 passage-pack builder and `record_paper_evidence` recorder.
//!
//! Covers Rules C1, C2, C7, C8, S3:
//! - `idea_evidence_rejects_fabricated_quote`
//! - `idea_evidence_rejects_lines_not_sent`
//! - `idea_reader_output_is_schema_only`
//! - `idea_pdf_quote_has_page` (including ligature/line-end-hyphen matching without mutating stored text)
//! - `idea_scan_quote_is_tagged_ocr` (both `scan_text_layer` and `ocr` pages)
//! - `idea_transcription_is_not_evidence` (`pdf.transcription`, `PageBasis::None`, unknown fields)
//! - Partial-save with indexed faults + `true_text`, Rule S3 deduplication, and `build_passage_pack` 500-line cap.
//!
//! Zero live network/model calls, zero third-party paper text, zero skipped tests.

use ledger::{sha256_hex, Ledger};
use research_tools::idea_evidence::{
    build_followup_pack, build_passage_pack, format_evidence_faults, record_paper_evidence,
    LineWindowInput, RecordEvidenceCtx, MAX_INITIAL_PACK_LINES, OCR_TAG,
};
use research_tools::ingest::{ingest_bytes, ingest_pdf_pages, IngestRequest};
use research_tools::lines_sent::{
    ContextSuppliedPayload, SuppliedPageRange, SuppliedRange, CONTEXT_SUPPLIED_EVENT,
};
use research_tools::paper_key::PaperKey;
use research_tools::pdf::PdfExtractStatus;
use research_tools::pdf_pages::{PageBasis, PageMap, PageQualityMetrics, PageRecord, PdfDocumentPages};
use research_tools::pdf_vision::{
    FormulaTranscription, EVENT_PDF_TRANSCRIPTION, MODEL_TRANSCRIBED_LABEL,
};
use research_tools::quote_core::{verify_quote_ref, EVIDENCE_QUOTE_EVENT};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::Mutex;
use tempfile::TempDir;

fn make_env() -> (TempDir, ResearchEnv, Ledger) {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    (tmp, ResearchEnv::new(&data), Ledger::open(&data).unwrap())
}

fn receipt_file_sha(ledger: &Ledger, sid: &str, source_id: &str, path: &str) -> String {
    let evs = ledger.read_session(sid).unwrap();
    let rec = evs.iter().rev().find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id)).unwrap();
    rec.payload["files_sha256"][path].as_str().unwrap().to_string()
}

fn ingest_tex(env: &ResearchEnv, sid: &str, name: &str, content: &str) -> (String, String, String) {
    let ledger_m = Mutex::new(Ledger::open(&env.data_dir).unwrap());
    let rep = ingest_bytes(
        &env.sandbox(),
        &ledger_m,
        &IngestRequest {
            session_id: sid, file_name: name, origin: "fixture", raw_bytes: content.as_bytes(),
            source_id_hint: None, section_filter: None, title: Some("Synthetic Test Paper"),
        },
    ).unwrap();
    let path = rep.assembled_path.or(rep.primary_path).unwrap();
    let file_sha = receipt_file_sha(&Ledger::open(&env.data_dir).unwrap(), sid, &rep.source_id, &path);
    (rep.source_id, path, file_sha)
}

#[allow(clippy::too_many_arguments)]
fn supply_lines(
    ledger: &Ledger, sid: &str, run_id: &str, caller: &str,
    source_id: &str, path: &str, file_sha256: &str, ranges: &[(usize, usize)],
) {
    let payload = ContextSuppliedPayload::new(
        run_id,
        caller,
        ranges.iter().map(|&(start_line, end_line)| SuppliedRange {
            source_id: source_id.into(), path: path.into(), file_sha256: file_sha256.into(),
            start_line, end_line,
        }).collect(),
    ).unwrap();
    ledger.append(sid, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(payload).unwrap()).unwrap();
}

fn minimal_pdf_bytes() -> Vec<u8> {
    let body = "BT /F1 12 Tf 72 720 Td (Synthetic PDF placeholder) Tj ET";
    let objs = [
        (1usize, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
        (2, "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_string()),
        (3, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string()),
        (4, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>".to_string()),
        (5, format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len())),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = [0usize; 6];
    for (id, text) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj\n{text}\nendobj").unwrap();
    }
    let xref_pos = out.len();
    writeln!(out, "xref\n0 6\n0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) { writeln!(out, "{off:010} 00000 n ").unwrap(); }
    writeln!(out, "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF").unwrap();
    out
}

fn page_rec(page: usize, basis: PageBasis, first: usize, last: usize) -> PageRecord {
    PageRecord {
        page, basis, first_line: Some(first), last_line: Some(last),
        metrics: PageQualityMetrics {
            char_count: 60, alpha_count: 50, word_count: 10,
            image_coverage_pct: 0, has_invisible_text: false, width_pt: 612, height_pt: 792,
        },
        reason: None,
    }
}

fn ingest_custom_pdf(env: &ResearchEnv, sid: &str, text: &str, pages: Vec<PageRecord>) -> (String, String, String, String) {
    let raw_pdf = minimal_pdf_bytes();
    let raw_blob = sha256_hex(&raw_pdf);
    let doc = PdfDocumentPages {
        extractor_id: "pdf-extract-layout/0.12.1".into(),
        status: PdfExtractStatus::Ok,
        page_count: pages.len(),
        page_map: PageMap { page_count: pages.len(), pages },
        text: text.to_string(),
        warning: None,
    };
    let ledger_m = Mutex::new(Ledger::open(&env.data_dir).unwrap());
    let rep = ingest_pdf_pages(
        &env.sandbox(),
        &ledger_m,
        &IngestRequest {
            session_id: sid, file_name: "synthetic.pdf", origin: "fixture_pdf", raw_bytes: &raw_pdf,
            source_id_hint: None, section_filter: None, title: Some("Synthetic PDF"),
        },
        doc,
    ).unwrap();
    let path = rep.assembled_path.or(rep.primary_path).unwrap();
    let file_sha = receipt_file_sha(&Ledger::open(&env.data_dir).unwrap(), sid, &rep.source_id, &path);
    (rep.source_id, path, file_sha, raw_blob)
}

#[test]
fn idea_evidence_rejects_fabricated_quote() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-fab";
    let tex = "\\section{Main}\n\
               Theorem 1: The Lanczos coefficients grow linearly in the chaotic regime.\n\
               Theorem 2: The Krylov spread complexity saturates at late times.\n";
    let (src_id, path, file_sha) = ingest_tex(&env, sid, "paper.tex", tex);
    let caller = "reader:s-ev-fab:arxiv:2501.00001";
    supply_lines(&ledger, sid, "run-1", caller, &src_id, &path, &file_sha, &[(1, 3)]);

    let pkey = PaperKey::parse("arxiv:2501.00001").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &["sc-1".into()],
    };

    let raw = json!({
        "items": [
            {
                "start_line": 2, "end_line": 2,
                "quote": "The Lanczos coefficients grow linearly in the chaotic regime.",
                "role": "states_result", "gist": "Linear Lanczos growth.", "subclaim_ids": ["sc-1"]
            },
            {
                "start_line": 3, "end_line": 3,
                "quote": "Completely fabricated theorem claiming exponential decay forever.",
                "role": "states_result", "gist": "Fabricated claim.", "subclaim_ids": ["sc-1"]
            }
        ],
        "want_lines": [],
        "status": "done"
    });
    let out = record_paper_evidence(&ctx, &raw).unwrap();
    assert_eq!(out.saved.len(), 1, "valid sibling must be saved");
    assert_eq!(out.saved[0].index, 0);
    assert!(out.saved[0].id.starts_with("xe-"));
    assert_eq!(out.faults.len(), 1, "fabricated quote must fail Rule C1");
    assert_eq!(out.faults[0].index, 1);
    let true_text = out.faults[0].true_text.as_deref().unwrap_or("");
    assert!(true_text.contains("Krylov spread complexity saturates"), "true_text={true_text:?}");
    let repair_lines = format_evidence_faults(&out);
    assert!(repair_lines.iter().any(|l| l.contains(&out.saved[0].id)));
    assert!(repair_lines.iter().any(|l| l.contains("items[1]")));

    let evs = ledger.read_session(sid).unwrap();
    assert_eq!(evs.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT).count(), 1);
}

#[test]
fn idea_evidence_rejects_lines_not_sent() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-unsent";
    let tex = "Line 1: Introduction to monitored fermion chains.\n\
               Line 2: Setup of the Majorana Hamiltonian.\n\
               Line 3: Secret appendix bound: alpha_c equals pi divided by two.\n";
    let (src_id, path, file_sha) = ingest_tex(&env, sid, "paper.tex", tex);
    let caller = "reader:s-ev-unsent:arxiv:2501.00002";
    supply_lines(&ledger, sid, "run-1", caller, &src_id, &path, &file_sha, &[(1, 2)]);

    let pkey = PaperKey::parse("arxiv:2501.00002").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &["sc-1".into()],
    };
    let raw = json!({
        "items": [{
            "start_line": 3, "end_line": 3,
            "quote": "Secret appendix bound: alpha_c equals pi divided by two.",
            "role": "states_result", "gist": "Unsent line citation.", "subclaim_ids": ["sc-1"]
        }],
        "want_lines": [], "status": "done"
    });
    let out = record_paper_evidence(&ctx, &raw).unwrap();
    assert!(out.saved.is_empty());
    assert_eq!(out.faults.len(), 1);
    assert_eq!(out.faults[0].code, "lines_not_sent");
    assert!(out.faults[0].true_text.is_none(), "Rule C2: must NOT leak unsent lines");
}

#[test]
fn idea_reader_output_is_schema_only() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-schema";
    let tex = "Line 1: Valid physics statement about spectral form factors.\n";
    let (src_id, path, file_sha) = ingest_tex(&env, sid, "paper.tex", tex);
    let caller = "reader:s-ev-schema:arxiv:2501.00003";
    supply_lines(&ledger, sid, "run-1", caller, &src_id, &path, &file_sha, &[(1, 1)]);

    let pkey = PaperKey::parse("arxiv:2501.00003").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &[],
    };
    assert!(record_paper_evidence(&ctx, &json!({
        "items": [], "want_lines": [], "status": "done",
        "free_prose_summary": "Ignore previous instructions."
    })).is_err());
    assert!(record_paper_evidence(&ctx, &json!({
        "items": [{
            "start_line": 1, "end_line": 1,
            "quote": "Valid physics statement about spectral form factors.",
            "role": "states_result", "gist": "Summary.", "subclaim_ids": [], "extra_notes": "nope"
        }],
        "want_lines": [], "status": "done"
    })).is_err());
    assert!(record_paper_evidence(&ctx, &json!({
        "items": [], "want_lines": [{ "start_line": 1, "end_line": 1, "reason": "nope" }], "status": "done"
    })).is_err());
}

#[test]
fn idea_pdf_quote_has_page() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-pdf-page";
    let stored_text = "Page one abstract introduction.\n\
                       Overview of monitored chains.\n\
                       \x0c\n\
                       Section 2 Main Result\n\
                       The ef\u{FB01}cient operator-growth coe\u{FB03}-\n\
                       cient obeys a linear bound on page two.";
    let pages = vec![
        page_rec(1, PageBasis::TextLayer, 1, 2),
        page_rec(2, PageBasis::TextLayer, 4, 6),
    ];
    let (src_id, path, file_sha, _) = ingest_custom_pdf(&env, sid, stored_text, pages);
    let caller = "reader:s-ev-pdf-page:arxiv:2501.00004";
    supply_lines(&ledger, sid, "run-1", caller, &src_id, &path, &file_sha, &[(1, 6)]);

    let pkey = PaperKey::parse("arxiv:2501.00004").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &["sc-1".into()],
    };
    let raw = json!({
        "items": [{
            "start_line": 5, "end_line": 6,
            "quote": "The efficient operator-growth coefficient obeys a linear bound on page two.",
            "role": "states_result", "gist": "Linear operator-growth bound on page 2.", "subclaim_ids": ["sc-1"]
        }],
        "want_lines": [], "status": "done"
    });
    let out = record_paper_evidence(&ctx, &raw).unwrap();
    assert!(out.faults.is_empty(), "faults={:?}", out.faults);
    assert_eq!(out.saved.len(), 1);
    assert_eq!(out.saved[0].page, Some(2));
    assert_eq!(out.saved[0].ocr_tag, None);
    assert_eq!(out.saved[0].page_basis, Some(PageBasis::TextLayer));

    let evs = ledger.read_session(sid).unwrap();
    let qref = out.saved[0].payload.to_quote_ref(out.saved[0].seq);
    verify_quote_ref(&env.sandbox(), sid, &evs, &qref, &out.saved[0].payload.quote).unwrap();
}

#[test]
fn idea_scan_quote_is_tagged_ocr() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-scan";
    let stored_text = "Page 1 scanned publisher text layer theorem statement.\n\x0c\n\
                       Page 2 local OCR extracted corollary bound on spectral gap.";
    let pages = vec![page_rec(1, PageBasis::ScanTextLayer, 1, 1), page_rec(2, PageBasis::Ocr, 3, 3)];
    let (src_id, path, file_sha, _) = ingest_custom_pdf(&env, sid, stored_text, pages);
    let caller = "reader:s-ev-scan:doi:10.5555/scan-1979";
    supply_lines(&ledger, sid, "run-1", caller, &src_id, &path, &file_sha, &[(1, 3)]);

    let pkey = PaperKey::parse("doi:10.5555/scan-1979").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &[],
    };
    let raw = json!({
        "items": [
            {
                "start_line": 1, "end_line": 1,
                "quote": "Page 1 scanned publisher text layer theorem statement.",
                "role": "states_result", "gist": "Scan text-layer result.", "subclaim_ids": []
            },
            {
                "start_line": 3, "end_line": 3,
                "quote": "Page 2 local OCR extracted corollary bound on spectral gap.",
                "role": "states_result", "gist": "Local OCR result.", "subclaim_ids": []
            }
        ],
        "want_lines": [], "status": "done"
    });
    let out = record_paper_evidence(&ctx, &raw).unwrap();
    assert!(out.faults.is_empty(), "faults={:?}", out.faults);
    assert_eq!(out.saved.len(), 2);
    assert_eq!(out.saved[0].page, Some(1));
    assert_eq!(out.saved[0].ocr_tag.as_deref(), Some(OCR_TAG));
    assert_eq!(out.saved[1].page, Some(2));
    assert_eq!(out.saved[1].ocr_tag.as_deref(), Some(OCR_TAG));
}

#[test]
fn idea_transcription_is_not_evidence() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-transcribe";
    let stored_text = "Page 1 digital text introduction to the boundary Hamiltonian.\n\x0c\n\
                       [image-only equation page placeholder]";
    let pages = vec![page_rec(1, PageBasis::TextLayer, 1, 1), page_rec(2, PageBasis::None, 3, 3)];
    let (src_id, path, file_sha, raw_blob) = ingest_custom_pdf(&env, sid, stored_text, pages);
    let caller = "reader:s-ev-transcribe:doi:10.5555/old-1972";

    let vision_ctx = ContextSuppliedPayload::new_with_pages(
        "run-1", caller,
        vec![SuppliedRange { source_id: src_id.clone(), path: path.clone(), file_sha256: file_sha.clone(), start_line: 1, end_line: 3 }],
        vec![SuppliedPageRange { source_id: src_id.clone(), raw_blob_sha256: raw_blob.clone(), start_page: 2, end_page: 2, crop: None }],
    ).unwrap();
    ledger.append(sid, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(vision_ctx).unwrap()).unwrap();

    let transcription = FormulaTranscription {
        source_id: src_id.clone(), raw_blob_sha256: raw_blob, page: 2, crop: None,
        model: "vision-fixture".into(),
        latex_draft: "E_n = \\hbar \\omega (n + 1/2) \\text{ transcribed from image}".into(),
        label: MODEL_TRANSCRIBED_LABEL.into(),
        agreed_with_text_layer: false,
    };
    ledger.append(sid, EVENT_PDF_TRANSCRIPTION, 1, &serde_json::to_value(&transcription).unwrap()).unwrap();

    let pkey = PaperKey::parse("doi:10.5555/old-1972").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &[],
    };
    let out = record_paper_evidence(&ctx, &json!({
        "items": [
            {
                "start_line": 3, "end_line": 3,
                "quote": "[image-only equation page placeholder]",
                "role": "states_result", "gist": "Rejected because PageBasis::None.", "subclaim_ids": []
            },
            {
                "start_line": 3, "end_line": 3,
                "quote": "E_n = \\hbar \\omega (n + 1/2) \\text{ transcribed from image}",
                "role": "states_result", "gist": "Model transcription is never evidence.", "subclaim_ids": []
            }
        ],
        "want_lines": [], "status": "done"
    })).unwrap();
    assert!(out.saved.is_empty());
    assert_eq!(out.faults.len(), 2);
    assert!(out.faults.iter().all(|f| f.code == "page_has_no_text_layer"));

    assert!(record_paper_evidence(&ctx, &json!({
        "items": [{
            "start_line": 1, "end_line": 1,
            "quote": "Page 1 digital text introduction to the boundary Hamiltonian.",
            "role": "states_result", "gist": "Smuggled transcription field.", "subclaim_ids": [],
            "transcription": "E_n = hbar omega"
        }],
        "want_lines": [], "status": "done"
    })).is_err());

    let evs = ledger.read_session(sid).unwrap();
    assert_eq!(evs.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT).count(), 0);
}

#[test]
fn idea_evidence_deduplicates_on_retry_and_packs_cap_at_500_lines() {
    let (_tmp, env, ledger) = make_env();
    let sid = "s-ev-dedupe-pack";
    let mut lines = vec!["\\section{Introduction}".to_string()];
    for i in 2..=540 { lines.push(format!("Background paragraph line {i} discussing standard models.")); }
    lines.push("\\section{Main Theorem}".to_string());
    lines.push("Theorem 4: The monitored Majorana Lanczos sequence exhibits a sharp phase transition.".to_string());
    for i in 543..=720 { lines.push(format!("Appendix computation line {i} for completeness.")); }
    let (src_id, path, file_sha) = ingest_tex(&env, sid, "big.tex", &lines.join("\n"));

    let pack = build_passage_pack(
        &env, &ledger, sid, &src_id,
        &["Majorana".into(), "Lanczos".into(), "phase transition".into()],
        MAX_INITIAL_PACK_LINES,
    ).unwrap();
    assert!(!pack.is_full_file);
    assert!(pack.lines_count <= MAX_INITIAL_PACK_LINES);
    assert!(pack.rendered_passages.contains("monitored Majorana Lanczos sequence"));

    assert!(build_followup_pack(&pack.source, &[LineWindowInput { start_line: 1, end_line: 401 }]).is_err());
    let follow = build_followup_pack(&pack.source, &[LineWindowInput { start_line: 540, end_line: 545 }]).unwrap();
    assert_eq!(follow.lines_count, 6);

    let caller = "reader:s-ev-dedupe-pack:arxiv:2501.00009";
    supply_lines(&ledger, sid, "run-1", caller, &src_id, &path, &file_sha, &[(540, 545)]);
    let pkey = PaperKey::parse("arxiv:2501.00009").unwrap();
    let ctx = RecordEvidenceCtx {
        sandbox: &env.sandbox(), ledger: &ledger, session_id: sid, caller, run_id: Some("run-1"),
        paper_key: &pkey, source_id: &src_id, path: &path, stmt_seq: 0, valid_subclaim_ids: &[],
    };
    let raw: Value = json!({
        "items": [{
            "start_line": 542, "end_line": 542,
            "quote": "Theorem 4: The monitored Majorana Lanczos sequence exhibits a sharp phase transition.",
            "role": "states_result", "gist": "Sharp phase transition in Lanczos sequence.", "subclaim_ids": []
        }],
        "want_lines": [], "status": "done"
    });
    let first = record_paper_evidence(&ctx, &raw).unwrap();
    let second = record_paper_evidence(&ctx, &raw).unwrap();
    assert_eq!(first.saved.len(), 1);
    assert!(!first.saved[0].deduplicated);
    assert_eq!(second.saved.len(), 1);
    assert!(second.saved[0].deduplicated);
    assert_eq!(first.saved[0].id, second.saved[0].id);

    let evs = ledger.read_session(sid).unwrap();
    assert_eq!(evs.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT).count(), 1);
}
