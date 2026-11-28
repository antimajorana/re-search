//! F14b end-to-end PDF OCR integration tests and Set P6 real-Tesseract coverage measurement.

use ledger::Ledger;
use research_tools::ingest_ocr::{ingest_pdf_with_ocr, OcrIngestRequest};
use research_tools::library::Library;
use research_tools::library_schema::PageBasis as LibPageBasis;
use research_tools::paper_key::PaperKey;
use research_tools::pdf::verified_page_lines;
use research_tools::pdf_match::{match_pdf_quote, normalize_for_pdf_match};
use research_tools::pdf_ocr::{
    FakeOcrPageRunner, LocalTesseractPageRunner, OcrPageRunner, PdfOcrOptions,
};
use research_tools::pdf_ocr_models::{
    hash_local_traineddata, ocr_status, OcrEnvProbe, OcrLocalConfig, OcrModelProvenance,
    OCR_INSTALL_PROMPT,
};
use research_tools::pdf_pages::{extract_pdf_pages, PageBasis, PageMap, PdfExtractOptions};
use research_tools::pdf_render::RasterXObjectRenderer;
use research_tools::quote_core::{verify_quote, QuoteRef};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    load_ingested_pdf_source, match_pdf_quote_for_display, render_formulas_as_math,
};
use research_tools::solve_model::event_kinds;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{wording, DisplayGrade, Tier};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::{tempdir, TempDir};
use tools::EVENT_SCHEMA_VERSION;

#[derive(Debug, Deserialize)]
struct P6Manifest {
    fixtures: Vec<P6FixtureSpec>,
}

#[derive(Debug, Deserialize, Clone)]
struct P6FixtureSpec {
    id: String,
    file: String,
    #[allow(dead_code)]
    kind: String,
    page_count: usize,
    expected_initial_bases: Vec<PageBasis>,
    expected_post_ocr_bases: Vec<PageBasis>,
    ground_truth_sentences: Vec<GroundTruthSentence>,
}

#[derive(Debug, Deserialize, Clone)]
struct GroundTruthSentence {
    page: usize,
    text: String,
}

#[derive(Debug, Default)]
struct FixtureMetrics {
    id: String,
    pages_total: usize,
    pages_ocr_applied: usize,
    pages_skipped: usize,
    sentences_matched: usize,
    sentences_total: usize,
    cer_sum: f64,
    wrong_page_chip: usize,
    scanned_reported_text_layer: usize,
}

struct EvalEnv {
    _tmp: TempDir,
    sandbox: WorkspaceSandbox,
    ledger: Mutex<Ledger>,
    lib: Library,
}

impl EvalEnv {
    fn new() -> Self {
        let tmp = tempdir().expect("tempdir");
        let sandbox = WorkspaceSandbox::new(tmp.path().to_path_buf(), None);
        let ledger = Mutex::new(Ledger::open(tmp.path()).expect("open ledger"));
        let lib = Library::open(tmp.path()).expect("open library");
        Self { _tmp: tmp, sandbox, ledger, lib }
    }
}

fn p6_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p6_ocr")
}

fn fixture_path(spec: &P6FixtureSpec) -> PathBuf {
    if spec.id == "p5_10" {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/eval/p5/p5_10_pdf_scanned_ocr_and_image_only.pdf")
    } else {
        p6_dir().join(&spec.file)
    }
}

fn load_p6_manifest() -> P6Manifest {
    let raw = fs::read_to_string(p6_dir().join("manifest.json")).expect("read P6 manifest.json");
    serde_json::from_str(&raw).expect("parse P6 manifest.json")
}

fn p5_10_spec() -> P6FixtureSpec {
    P6FixtureSpec {
        id: "p5_10".into(),
        file: "p5_10_pdf_scanned_ocr_and_image_only.pdf".into(),
        kind: "p5_mixed_scan_and_image_only".into(),
        page_count: 3,
        expected_initial_bases: vec![PageBasis::TextLayer, PageBasis::ScanTextLayer, PageBasis::None],
        expected_post_ocr_bases: vec![PageBasis::TextLayer, PageBasis::ScanTextLayer, PageBasis::None],
        ground_truth_sentences: vec![
            GroundTruthSentence {
                page: 1,
                text: "The trace norm restricts functions to the boundary.".into(),
            },
            GroundTruthSentence {
                page: 2,
                text: "Every discrete boundary vector obeys the weighted trace bound on uniform meshes.".into(),
            },
        ],
    }
}

fn page_text_slice(text: &str, page_map: &PageMap, page: usize) -> String {
    let Some(rec) = page_map.record(page) else { return String::new() };
    let (Some(first), Some(last)) = (rec.first_line, rec.last_line) else { return String::new() };
    let lines: Vec<&str> = text.split('\n').collect();
    if first == 0 || last < first || last > lines.len() {
        return String::new();
    }
    lines[(first - 1)..last].join("\n")
}

/// Semi-global Levenshtein distance: min edits aligning `needle` against any substring of `haystack`.
fn substring_edit_distance(needle: &str, haystack: &str) -> usize {
    let n: Vec<char> = needle.chars().collect();
    let h: Vec<char> = haystack.chars().collect();
    if n.is_empty() { return 0; }
    if h.is_empty() { return n.len(); }
    let (mut prev, mut cur) = (vec![0usize; h.len() + 1], vec![0usize; h.len() + 1]);
    for (i, &nc) in n.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &hc) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(nc != hc);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.into_iter().min().unwrap_or(n.len())
}

fn sentence_cer_per_100(gt: &str, page_text: &str) -> f64 {
    let norm_gt = normalize_for_pdf_match(gt);
    let norm_page = normalize_for_pdf_match(page_text);
    let len = norm_gt.chars().count().max(1);
    (substring_edit_distance(&norm_gt, &norm_page) as f64 / len as f64) * 100.0
}

fn fake_provenance() -> OcrModelProvenance {
    let payload = b"p6-fake-eng-traineddata";
    OcrModelProvenance {
        lang: "eng".into(),
        sha256: ledger::sha256_hex(payload),
        bytes: payload.len() as u64,
    }
}

fn build_fake_runner_for_fixture(spec: &P6FixtureSpec) -> FakeOcrPageRunner {
    let mut by_page: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for s in &spec.ground_truth_sentences {
        if spec.expected_post_ocr_bases.get(s.page.saturating_sub(1)) == Some(&PageBasis::Ocr) {
            by_page.entry(s.page).or_default().push(s.text.clone());
        }
    }
    let pages = by_page.into_iter().map(|(p, lines)| (p, Ok(lines.join("\n")))).collect();
    FakeOcrPageRunner::new(pages)
        .with_engine_id("tesseract/fake-5.3.4")
        .with_provenance(vec![fake_provenance()])
}

fn append_cited_entry(
    ledger: &Mutex<Ledger>,
    session: &str,
    entry_id: &str,
    mut qref: QuoteRef,
    ocr: bool,
    details: &str,
) {
    let guard = ledger.lock().unwrap();
    let q_seq = guard
        .append(
            session,
            event_kinds::EVIDENCE_QUOTE,
            EVENT_SCHEMA_VERSION,
            &json!({
                "source_id": qref.source_id, "path": qref.path,
                "start_line": qref.start_line, "end_line": qref.end_line,
                "file_sha256": qref.file_sha256, "quote_sha256": qref.quote_sha256,
                "ocr": ocr,
            }),
        )
        .unwrap();
    qref.evidence_seq = Some(q_seq);
    guard
        .append(
            session,
            event_kinds::PROOF_ENTRY,
            EVENT_SCHEMA_VERSION,
            &json!({
                "entry_id": entry_id, "entry_type": "reference_lookup",
                "summary": entry_id, "details": details,
                "depends_on": [], "source_ref": qref,
            }),
        )
        .unwrap();
}

fn evaluate_fixture(
    env: &EvalEnv,
    session_id: &str,
    spec: &P6FixtureSpec,
    pdf_bytes: &[u8],
    runner: &dyn OcrPageRunner,
) -> FixtureMetrics {
    let initial = extract_pdf_pages(pdf_bytes, &PdfExtractOptions::user_supplied())
        .expect("initial extract_pdf_pages");
    let initial_bases: Vec<PageBasis> = initial.page_map.pages.iter().map(|r| r.basis).collect();
    assert_eq!(initial_bases, spec.expected_initial_bases, "{}: initial bases", spec.id);

    let renderer = RasterXObjectRenderer::default();
    let req = OcrIngestRequest {
        session_id,
        file_name: &spec.file,
        origin: &spec.file,
        raw_bytes: pdf_bytes,
        source_id_hint: Some(&spec.id),
        title: Some(&spec.id),
        ocr_opts: PdfOcrOptions::default(),
    };
    let ingested =
        ingest_pdf_with_ocr(&env.sandbox, &env.ledger, Some(&env.lib), &req, &renderer, Some(runner))
            .expect("ingest_pdf_with_ocr");
    let post_bases: Vec<PageBasis> = ingested.page_map.pages.iter().map(|r| r.basis).collect();
    assert_eq!(post_bases, spec.expected_post_ocr_bases, "{}: post-OCR bases", spec.id);

    let events = env.ledger.lock().unwrap().read_session(session_id).unwrap();
    let receipt = events.iter().find(|e| e.seq == ingested.source.event_seq).expect("receipt");
    let loaded = load_ingested_pdf_source(&env.sandbox, session_id, receipt).expect("load receipt");
    assert_eq!(loaded.page_map.page_count, spec.page_count);

    let mut m = FixtureMetrics {
        id: spec.id.clone(),
        pages_total: spec.page_count,
        pages_ocr_applied: ingested.pages_ocr_applied.len(),
        pages_skipped: spec.expected_initial_bases.iter().filter(|b| !b.needs_ocr()).count(),
        sentences_total: spec.ground_truth_sentences.len(),
        ..FixtureMetrics::default()
    };

    for gt in &spec.ground_truth_sentences {
        let expected_basis = spec.expected_post_ocr_bases[gt.page - 1];
        let page_txt = page_text_slice(&loaded.text, &loaded.page_map, gt.page);
        m.cer_sum += sentence_cer_per_100(&gt.text, &page_txt);

        if let Some(disp) = match_pdf_quote_for_display(
            &loaded.primary_path, &loaded.text, &loaded.page_map, &gt.text,
        ) {
            if disp.start_page != gt.page || disp.end_page != gt.page {
                m.wrong_page_chip += 1;
            }
            let scanned = matches!(expected_basis, PageBasis::Ocr | PageBasis::ScanTextLayer);
            if scanned && (!disp.is_ocr || disp.render_math) {
                m.scanned_reported_text_layer += 1;
            }
            if scanned {
                assert!(disp.is_ocr && !disp.render_math, "{}: scan/OCR honesty", spec.id);
                assert!(
                    disp.display_label.starts_with("matched in OCR text, p. "),
                    "{}: unexpected label `{}`", spec.id, disp.display_label
                );
            } else if expected_basis == PageBasis::TextLayer {
                assert!(!disp.is_ocr && disp.render_math, "{}: digital honesty", spec.id);
                assert_eq!(disp.display_label, format!("matched, p. {}", gt.page));
            }
            if disp.start_page == gt.page && disp.end_page == gt.page {
                m.sentences_matched += 1;
            }
        }
    }

    assert!(!render_formulas_as_math(PageBasis::Ocr));
    assert!(!render_formulas_as_math(PageBasis::ScanTextLayer));
    assert!(render_formulas_as_math(PageBasis::TextLayer));
    m
}

#[test]
fn f14b_ocr_pipeline_end_to_end_honesty_quotes_library_and_solve_flag() {
    let manifest = load_p6_manifest();
    let p6_05 = manifest.fixtures.iter().find(|f| f.id == "p6_05").unwrap();
    let p6_03 = manifest.fixtures.iter().find(|f| f.id == "p6_03").unwrap();
    let pdf_05 = fs::read(fixture_path(p6_05)).unwrap();
    let pdf_03 = fs::read(fixture_path(p6_03)).unwrap();

    let env = EvalEnv::new();
    let session = "s-f14b-e2e";
    let renderer = RasterXObjectRenderer::default();

    // Missing Tesseract stays honest and prompts installation.
    let err = LocalTesseractPageRunner::from_config_with_probe(
        &OcrLocalConfig::default(),
        &OcrEnvProbe::default(),
    )
    .unwrap_err();
    assert_eq!(err, OCR_INSTALL_PROMPT);

    let req_05 = OcrIngestRequest {
        session_id: session, file_name: &p6_05.file, origin: "upload:p6_05.pdf",
        raw_bytes: &pdf_05, source_id_hint: Some("p6-05"),
        title: Some("Mixed Digital and Scan Insert"), ocr_opts: PdfOcrOptions::default(),
    };
    let no_ocr = ingest_pdf_with_ocr(&env.sandbox, &env.ledger, None, &req_05, &renderer, None).unwrap();
    assert!(!no_ocr.ocr_available && no_ocr.ocr_status == "unavailable");
    assert_eq!(no_ocr.ocr_message.as_deref(), Some(OCR_INSTALL_PROMPT));
    assert_eq!(no_ocr.pages_remaining_none, vec![2]);
    assert_eq!(no_ocr.page_map.record(2).unwrap().basis, PageBasis::None);
    let prior_receipt = env.ledger.lock().unwrap().read_session(session).unwrap()[0].clone();

    // Selective OCR: p6_05 only sends page 2; p6_03 only sends page 3.
    let runner_05 = build_fake_runner_for_fixture(p6_05);
    let ing_05 =
        ingest_pdf_with_ocr(&env.sandbox, &env.ledger, Some(&env.lib), &req_05, &renderer, Some(&runner_05))
            .unwrap();
    assert_eq!(runner_05.calls(), vec![2]);
    assert_eq!(ing_05.pages_ocr_applied, vec![2]);
    assert!(ing_05.pages_remaining_none.is_empty());
    assert_eq!(ing_05.ocr_engine.as_deref(), Some("tesseract/fake-5.3.4"));
    assert_eq!(ing_05.ocr_models, vec![fake_provenance()]);

    let req_03 = OcrIngestRequest {
        session_id: session, file_name: &p6_03.file, origin: "upload:p6_03.pdf",
        raw_bytes: &pdf_03, source_id_hint: Some("p6-03"),
        title: Some("Publisher OCR Plus Appendix"), ocr_opts: PdfOcrOptions::default(),
    };
    let runner_03 = build_fake_runner_for_fixture(p6_03);
    let ing_03 =
        ingest_pdf_with_ocr(&env.sandbox, &env.ledger, Some(&env.lib), &req_03, &renderer, Some(&runner_03))
            .unwrap();
    assert_eq!(runner_03.calls(), vec![3]);
    assert_eq!(ing_03.pages_ocr_applied, vec![3]);
    assert!(ing_03.pages_remaining_none.is_empty());

    // Prior receipt untouched; new receipts pin OCR provenance and per-page basis.
    let events = env.ledger.lock().unwrap().read_session(session).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0], prior_receipt);
    let (rec_05, rec_03) = (&events[1], &events[2]);
    assert_eq!(rec_05.payload["ocr_engine"], "tesseract/fake-5.3.4");
    assert_eq!(rec_05.payload["ocr_models"], serde_json::to_value(vec![fake_provenance()]).unwrap());
    assert_eq!(rec_05.payload["pdf_pages"][0]["basis"], "text_layer");
    assert_eq!(rec_05.payload["pdf_pages"][1]["basis"], "ocr");
    assert_eq!(rec_05.payload["pdf_pages"][2]["basis"], "text_layer");
    assert_eq!(rec_03.payload["pdf_pages"][0]["basis"], "scan_text_layer");
    assert_eq!(rec_03.payload["pdf_pages"][1]["basis"], "scan_text_layer");
    assert_eq!(rec_03.payload["pdf_pages"][2]["basis"], "ocr");
    assert_eq!(
        verified_page_lines(&rec_05.payload, || Ok(pdf_05.clone())).unwrap(),
        ing_05.page_map.page_lines()
    );

    // Library caches paper_text with exact per-page bases and engine id.
    let lib_05 = env.lib.get_paper(&PaperKey::from_file_bytes(&pdf_05)).unwrap().unwrap();
    assert_eq!(
        lib_05.paper_text[0].page_basis,
        vec![LibPageBasis::TextLayer, LibPageBasis::Ocr, LibPageBasis::TextLayer]
    );
    assert!(lib_05.paper_text[0].extractor.contains("tesseract/fake-5.3.4"));
    let lib_03 = env.lib.get_paper(&PaperKey::from_file_bytes(&pdf_03)).unwrap().unwrap();
    assert_eq!(
        lib_03.paper_text[0].page_basis,
        vec![LibPageBasis::ScanTextLayer, LibPageBasis::ScanTextLayer, LibPageBasis::Ocr]
    );

    // Verify quotes via pdf_match, skeleton::pdf_honesty, and quote_core::verify_quote.
    let loaded_05 = load_ingested_pdf_source(&env.sandbox, session, rec_05).unwrap();
    let (dig_q, ocr_q) = (&p6_05.ground_truth_sentences[0].text, &p6_05.ground_truth_sentences[2].text);
    let raw_m = match_pdf_quote(&loaded_05.text, &loaded_05.page_map, ocr_q).unwrap();
    assert_eq!((raw_m.start_page, raw_m.start_page_basis), (Some(2), Some(PageBasis::Ocr)));

    let disp_ocr =
        match_pdf_quote_for_display(&loaded_05.primary_path, &loaded_05.text, &loaded_05.page_map, ocr_q)
            .unwrap();
    assert_eq!(disp_ocr.display_label, "matched in OCR text, p. 2");
    assert!(disp_ocr.is_ocr && !disp_ocr.render_math && disp_ocr.start_page == 2);

    let disp_dig =
        match_pdf_quote_for_display(&loaded_05.primary_path, &loaded_05.text, &loaded_05.page_map, dig_q)
            .unwrap();
    assert_eq!(disp_dig.display_label, "matched, p. 1");
    assert!(!disp_dig.is_ocr && disp_dig.render_math && disp_dig.start_page == 1);

    let loaded_03 = load_ingested_pdf_source(&env.sandbox, session, rec_03).unwrap();
    let disp_scan = match_pdf_quote_for_display(
        &loaded_03.primary_path, &loaded_03.text, &loaded_03.page_map,
        &p6_03.ground_truth_sentences[0].text,
    )
    .unwrap();
    assert_eq!(disp_scan.display_label, "matched in OCR text, p. 1");
    assert!(disp_scan.is_ocr && !disp_scan.render_math);
    let disp_app = match_pdf_quote_for_display(
        &loaded_03.primary_path, &loaded_03.text, &loaded_03.page_map,
        &p6_03.ground_truth_sentences[4].text,
    )
    .unwrap();
    assert_eq!(disp_app.display_label, "matched in OCR text, p. 3");
    assert!(disp_app.is_ocr && !disp_app.render_math);

    let v_ocr = verify_quote(
        &env.sandbox, session, &events, &ing_05.source.source_id, &loaded_05.primary_path,
        disp_ocr.span.start_line, disp_ocr.span.end_line, ocr_q,
    )
    .unwrap();
    let v_dig = verify_quote(
        &env.sandbox, session, &events, &ing_05.source.source_id, &loaded_05.primary_path,
        disp_dig.span.start_line, disp_dig.span.end_line, dig_q,
    )
    .unwrap();

    // Solve OCR flag propagation: OCR quote gets TAG_OCR + FLAG_OCR_SOURCE; digital quote does not.
    append_cited_entry(&env.ledger, session, "e-ocr", v_ocr.quote_ref, disp_ocr.is_ocr, ocr_q);
    append_cited_entry(&env.ledger, session, "e-dig", v_dig.quote_ref, disp_dig.is_ocr, dig_q);
    let solve = SolveState::from_events(&env.ledger.lock().unwrap().read_session(session).unwrap());

    let b_ocr = solve.badge("e-ocr").expect("badge for OCR entry");
    assert_eq!((b_ocr.effective, b_ocr.label.as_str()), (Tier::Cited, wording::BADGE_CITED));
    assert_eq!(b_ocr.grade, DisplayGrade::MatchedQuoteScannedPage);
    assert_eq!(b_ocr.provenance_tag.as_deref(), Some(wording::TAG_OCR));
    assert!(b_ocr.flags.contains(&wording::FLAG_OCR_SOURCE.to_string()));

    let b_dig = solve.badge("e-dig").expect("badge for digital entry");
    assert_eq!((b_dig.effective, b_dig.grade), (Tier::Cited, DisplayGrade::MatchedQuoteFullText));
    assert_eq!(b_dig.provenance_tag, None);
    assert!(!b_dig.flags.contains(&wording::FLAG_OCR_SOURCE.to_string()));
}

#[test]
fn set_p6_real_tesseract_ocr_coverage_measurement() {
    let manifest = load_p6_manifest();
    assert_eq!(manifest.fixtures.len(), 6);
    let mut specs = manifest.fixtures;
    specs.push(p5_10_spec());

    let env = EvalEnv::new();
    let cfg = OcrLocalConfig::default();
    let status = ocr_status(&cfg);

    if status.available {
        let runner = LocalTesseractPageRunner::from_config(&cfg).expect("real Tesseract runner");
        let prov = runner.model_provenance();
        let eng = prov.iter().find(|m| m.lang == "eng").expect("eng model provenance");
        assert_eq!(eng.sha256.len(), 64);
        assert!(eng.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert!(eng.bytes > 0);
        let runtime_hash = hash_local_traineddata(runner.tessdata_dir(), &["eng".into()]).unwrap();
        assert_eq!(runtime_hash[0], *eng);

        println!(
            "\n=== Set P6 Real-Tesseract Measurement (engine={}, eng.traineddata sha256={}..., {} bytes) ===",
            runner.engine_id(), &eng.sha256[..16], eng.bytes
        );
        println!(
            "{:<8} {:>5} {:>4} {:>5} {:>11} {:>10} {:>10} {:>12}",
            "id", "pages", "ocr", "skip", "matched", "cer/100", "wrong_pg", "scan_as_txt"
        );

        let mut rows = Vec::new();
        for spec in &specs {
            let pdf_bytes = fs::read(fixture_path(spec)).expect("read fixture PDF");
            let sid = format!("s-p6-{}", spec.id.replace('_', "-"));
            let row = evaluate_fixture(&env, &sid, spec, &pdf_bytes, &runner);
            let avg_cer = row.cer_sum / row.sentences_total.max(1) as f64;
            println!(
                "{:<8} {:>5} {:>4} {:>5} {:>5}/{:<5} {:>10.2} {:>10} {:>12}",
                row.id, row.pages_total, row.pages_ocr_applied, row.pages_skipped,
                row.sentences_matched, row.sentences_total, avg_cer,
                row.wrong_page_chip, row.scanned_reported_text_layer
            );
            rows.push(row);
        }

        let total_pages: usize = rows.iter().map(|r| r.pages_total).sum();
        let total_ocr: usize = rows.iter().map(|r| r.pages_ocr_applied).sum();
        let total_skip: usize = rows.iter().map(|r| r.pages_skipped).sum();
        let total_matched: usize = rows.iter().map(|r| r.sentences_matched).sum();
        let total_sents: usize = rows.iter().map(|r| r.sentences_total).sum();
        let total_cer: f64 = rows.iter().map(|r| r.cer_sum).sum();
        let wrong_pg: usize = rows.iter().map(|r| r.wrong_page_chip).sum();
        let scan_as_txt: usize = rows.iter().map(|r| r.scanned_reported_text_layer).sum();
        let match_rate = total_matched as f64 / total_sents.max(1) as f64;
        let avg_cer = total_cer / total_sents.max(1) as f64;

        println!(
            "{:<8} {:>5} {:>4} {:>5} {:>5}/{:<5} {:>10.2} {:>10} {:>12}",
            "TOTAL", total_pages, total_ocr, total_skip, total_matched, total_sents, avg_cer,
            wrong_pg, scan_as_txt
        );
        println!(
            "Aggregate sentence match rate: {:.1}% ({}/{}); avg CER/100: {:.2}\n",
            match_rate * 100.0, total_matched, total_sents, avg_cer
        );

        assert_eq!(wrong_pg, 0, "wrong_page_chip must be 0");
        assert_eq!(scan_as_txt, 0, "scanned_reported_text_layer must be 0");
        assert!(match_rate >= 0.80, "sentence match rate {match_rate:.3} < 0.80");
        assert!(avg_cer < 15.0, "avg CER/100 {avg_cer:.2} >= 15.0");
    } else {
        assert_eq!(LocalTesseractPageRunner::from_config(&cfg).unwrap_err(), OCR_INSTALL_PROMPT);
        let (mut wrong_pg, mut scan_as_txt) = (0usize, 0usize);
        for spec in &specs {
            let pdf_bytes = fs::read(fixture_path(spec)).expect("read fixture PDF");
            let sid = format!("s-p6-fb-{}", spec.id.replace('_', "-"));
            let fake = build_fake_runner_for_fixture(spec);
            let row = evaluate_fixture(&env, &sid, spec, &pdf_bytes, &fake);
            wrong_pg += row.wrong_page_chip;
            scan_as_txt += row.scanned_reported_text_layer;
        }
        assert_eq!(wrong_pg, 0);
        assert_eq!(scan_as_txt, 0);
    }
}
