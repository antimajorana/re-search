use super::*;
use crate::lines_sent::{
    ContextSuppliedPayload, SuppliedPageRange, SuppliedRange, CONTEXT_SUPPLIED_EVENT,
};
use crate::sandbox::SandboxRoot;
use ledger::Ledger;
use serde_json::json;

const RAW_PDF_SHA: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_SHA: &str = "2222222222222222222222222222222222222222222222222222222222222222";

struct VisionFixture {
    _dir: tempfile::TempDir,
    sandbox: WorkspaceSandbox,
    ledger: Ledger,
    session: String,
    source_id: String,
    primary_path: String,
    file_sha256: String,
}

impl VisionFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = WorkspaceSandbox::new(dir.path(), None);
        let ledger = Ledger::open(dir.path()).unwrap();
        let session = "s-pdf-vision".to_string();
        let source_id = "paper1".to_string();
        let primary_path = format!("{source_id}/paper1.pdf.extracted.txt");

        // Synthetic 4-page reading text (no third-party text):
        // Lines 1..=3: page 1 (text_layer; line 2 is a standalone display equation)
        // Line 4: form feed separator
        // Page 2: basis "none" (image-only, no lines)
        // Lines 5..=6: page 3 (scan_text_layer, contains fi ligature \u{fb01})
        // Line 7: form feed separator
        // Lines 8..=9: page 4 (ocr)
        let reading_text = concat!(
            "Synthetic Lemma 1.\n",
            "E = m c^2\n",
            "Inline note with short formula a + b = c.\n",
            "\x0c\n",
            "Scan equation result:\n",
            "The ef\u{fb01}ciency ratio is eta_0 = 7/12 + delta_1.\n",
            "\x0c\n",
            "OCR equation result:\n",
            "Entropy S(E, V) = k_B ln Omega(E, V) on page 4.\n",
        );

        let target = sandbox
            .resolve_for_write(SandboxRoot::Sources, &session, &primary_path)
            .unwrap();
        std::fs::create_dir_all(target.target_path.parent().unwrap()).unwrap();
        std::fs::write(&target.target_path, reading_text.as_bytes()).unwrap();
        let file_sha256 = ledger::sha256_hex(reading_text.as_bytes());

        let metrics = json!({
            "char_count": 40,
            "alpha_count": 20,
            "word_count": 8,
            "image_coverage_pct": 0,
            "has_invisible_text": false,
            "width_pt": 612,
            "height_pt": 792
        });

        ledger
            .append(
                &session,
                "source.ingested",
                1,
                &json!({
                    "source_id": source_id,
                    "origin": "upload:paper1.pdf",
                    "title": "Synthetic Vision Paper",
                    "format": "pdf",
                    "status": "ok",
                    "bytes": 128,
                    "raw_blob": RAW_PDF_SHA,
                    "primary_path": primary_path,
                    "files": [primary_path],
                    "files_sha256": { &primary_path: &file_sha256 },
                    "page_count": 4,
                    "pdf_page_lines": [[1, 1, 3], [3, 5, 6], [4, 8, 9]],
                    "pdf_extractor": "pdf-extract-layout/0.12.1",
                    "pdf_pages": [
                        { "page": 1, "basis": "text_layer", "first_line": 1, "last_line": 3, "metrics": metrics },
                        { "page": 2, "basis": "none", "metrics": metrics },
                        { "page": 3, "basis": "scan_text_layer", "first_line": 5, "last_line": 6, "metrics": metrics },
                        { "page": 4, "basis": "ocr", "first_line": 8, "last_line": 9, "metrics": metrics }
                    ]
                }),
            )
            .unwrap();

        Self {
            _dir: dir,
            sandbox,
            ledger,
            session,
            source_id,
            primary_path,
            file_sha256,
        }
    }

    fn events(&self) -> Vec<Event> {
        self.ledger.read_session(&self.session).unwrap()
    }

    fn supply_page_only(&self, run_id: &str, caller: &str, page: usize) {
        let payload = ContextSuppliedPayload::new_with_pages(
            run_id,
            caller,
            vec![],
            vec![SuppliedPageRange {
                source_id: self.source_id.clone(),
                raw_blob_sha256: RAW_PDF_SHA.into(),
                start_page: page,
                end_page: page,
                crop: Some([0, 0, 200, 80]),
            }],
        )
        .unwrap();
        self.ledger
            .append(&self.session, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(payload).unwrap())
            .unwrap();
    }

    fn supply_lines(&self, run_id: &str, caller: &str, start_line: usize, end_line: usize) {
        let payload = ContextSuppliedPayload::new(
            run_id,
            caller,
            vec![SuppliedRange {
                source_id: self.source_id.clone(),
                path: self.primary_path.clone(),
                file_sha256: self.file_sha256.clone(),
                start_line,
                end_line,
            }],
        )
        .unwrap();
        self.ledger
            .append(&self.session, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(payload).unwrap())
            .unwrap();
    }
}

#[test]
fn constants_and_opt_in_gate_are_off_by_default() {
    const { assert!(!DEFAULT_EQUATION_TRANSCRIPTION_ENABLED); }
    assert_eq!(MODEL_TRANSCRIBED_LABEL, "model-transcribed");
    assert_eq!(EVENT_PDF_TRANSCRIPTION, "pdf.transcription");

    assert!(!is_equation_transcription_opted_in(None, None));
    assert!(!is_equation_transcription_opted_in(Some(false), None));
    assert!(is_equation_transcription_opted_in(Some(true), None));
    assert!(is_equation_transcription_opted_in(None, Some(true)));
    assert!(is_equation_transcription_opted_in(Some(false), Some(true)));
    // Explicit per-request refusal overrides a saved setting.
    assert!(!is_equation_transcription_opted_in(Some(true), Some(false)));
}

#[test]
fn formula_transcription_enforces_label_and_validation() {
    let rec = FormulaTranscription::new(
        "paper1",
        RAW_PDF_SHA,
        1,
        Some([10, 20, 400, 120]),
        "gpt-4o",
        "E = m c^2",
        false,
    )
    .unwrap();
    assert_eq!(rec.label, MODEL_TRANSCRIBED_LABEL);
    let val = serde_json::to_value(&rec).unwrap();
    let rt: FormulaTranscription = serde_json::from_value(val).unwrap();
    assert_eq!(rt, rec);

    // Forging any non-`model-transcribed` label fails validation.
    for forged in ["matched", "verified", "ocr", ""] {
        let mut bad = rec.clone();
        bad.label = forged.into();
        assert!(bad.validate().is_err(), "forged label `{forged}` must be rejected");
    }

    // Unknown fields rejected.
    assert!(serde_json::from_value::<FormulaTranscription>(json!({
        "source_id": "paper1",
        "raw_blob_sha256": RAW_PDF_SHA,
        "page": 1,
        "model": "gpt-4o",
        "latex_draft": "E = m c^2",
        "label": "model-transcribed",
        "agreed_with_text_layer": false,
        "extra": 1
    }))
    .is_err());

    // Invalid page, crop, hash, model, draft rejected.
    assert!(FormulaTranscription::new("paper1", RAW_PDF_SHA, 0, None, "gpt-4o", "x=1", false).is_err());
    assert!(FormulaTranscription::new("paper1", RAW_PDF_SHA, 1, Some([0, 0, 0, 10]), "gpt-4o", "x=1", false).is_err());
    assert!(FormulaTranscription::new("paper1", "bad", 1, None, "gpt-4o", "x=1", false).is_err());
    assert!(FormulaTranscription::new("paper1", RAW_PDF_SHA, 1, None, " ", "x=1", false).is_err());
    assert!(FormulaTranscription::new("paper1", RAW_PDF_SHA, 1, None, "gpt-4o", "   ", false).is_err());
}

#[test]
fn gate_refuses_missing_or_disagreeing_text_layer_and_page_only_context() {
    let fx = VisionFixture::new();
    fx.supply_page_only("run-1", "vision-caller", 1);
    fx.supply_page_only("run-1", "vision-caller", 2);
    let events = fx.events();

    // 1. Image-only page (basis = "none") -> refused with NoTextLayer.
    let img_only = FormulaTranscription::new(
        &fx.source_id,
        RAW_PDF_SHA,
        2,
        Some([0, 0, 200, 80]),
        "gpt-4o",
        "E = m c^2",
        false,
    )
    .unwrap();
    assert!(!check_text_layer_agreement(
        &fx.sandbox, &fx.session, &events, &fx.source_id, RAW_PDF_SHA, 2, "E = m c^2"
    ).agreed);
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-1"), "vision-caller", &img_only
        ),
        Err(TranscriptionGateError::NoTextLayer { page: 2, .. })
    ));

    // 2. Disagreeing text layer on page 1 -> refused with TextLayerDisagrees.
    let wrong_eq = FormulaTranscription::new(
        &fx.source_id,
        RAW_PDF_SHA,
        1,
        None,
        "gpt-4o",
        "F = m a",
        false,
    )
    .unwrap();
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-1"), "vision-caller", &wrong_eq
        ),
        Err(TranscriptionGateError::TextLayerDisagrees { page: 1, .. })
    ));

    // 3. Cross-page formula (exists on page 1, claimed on page 3) -> refused.
    let cross_page = FormulaTranscription::new(
        &fx.source_id, RAW_PDF_SHA, 3, None, "gpt-4o", "E = m c^2", false,
    )
    .unwrap();
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-1"), "vision-caller", &cross_page
        ),
        Err(TranscriptionGateError::TextLayerDisagrees { page: 3, .. })
    ));

    // 4. Agreeing text layer on page 1, BUT only `page_ranges` supplied (no text `ranges`) ->
    //    refused with QuoteRejected(LinesNotSent).
    let matching_p1 = FormulaTranscription::new(
        &fx.source_id,
        RAW_PDF_SHA,
        1,
        Some([0, 0, 200, 80]),
        "gpt-4o",
        "$$E = m c^2$$",
        true,
    )
    .unwrap();
    let check = check_text_layer_agreement(
        &fx.sandbox, &fx.session, &events, &fx.source_id, RAW_PDF_SHA, 1, "$$E = m c^2$$",
    );
    assert!(check.agreed);
    assert_eq!(check.basis, Some(TextLayerBasis::TextLayer));
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-1"), "vision-caller", &matching_p1
        ),
        Err(TranscriptionGateError::QuoteRejected(QuoteRejection::LinesNotSent { .. }))
    ));

    // 5. Substring / single-char false positives rejected.
    assert!(!check_text_layer_agreement(
        &fx.sandbox, &fx.session, &events, &fx.source_id, RAW_PDF_SHA, 1, "E"
    ).agreed);
    assert!(!check_text_layer_agreement(
        &fx.sandbox, &fx.session, &events, &fx.source_id, RAW_PDF_SHA, 3, "eq"
    ).agreed);

    // 6. Mismatched raw_blob_sha256 fails closed.
    let wrong_blob = FormulaTranscription::new(
        &fx.source_id, OTHER_SHA, 1, None, "gpt-4o", "E = m c^2", true,
    )
    .unwrap();
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-1"), "vision-caller", &wrong_blob
        ),
        Err(TranscriptionGateError::RawBlobMismatch { .. })
    ));
}

#[test]
fn gate_accepts_only_when_pinned_text_scan_or_ocr_layer_agrees_and_lines_were_supplied() {
    let fx = VisionFixture::new();
    fx.supply_lines("run-2", "reader-1", 1, 9);
    let events = fx.events();

    // TextLayer on page 1: agrees with outer $$...$$ delimiters stripped and returns QuoteRef on line 2.
    let t1 = FormulaTranscription::new(
        &fx.source_id, RAW_PDF_SHA, 1, None, "claude-sonnet-4-5", "$$E = m c^2$$", true,
    )
    .unwrap();
    let v1 = verify_transcription_against_text_layer(
        &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-2"), "reader-1", &t1,
    )
    .unwrap();
    assert_eq!(v1.page, 1);
    assert_eq!(v1.basis, TextLayerBasis::TextLayer);
    assert!(!v1.basis.is_ocr_or_scan());
    assert_eq!(v1.verified.quote_ref.start_line, 2);
    assert_eq!(v1.verified.quote_ref.end_line, 2);
    assert_eq!(v1.verified.quote_ref.file_sha256, fx.file_sha256);

    // ScanTextLayer on page 3: ASCII "efficiency ratio is eta_0 = 7/12 + delta_1" matches stored
    // ligature "ef\u{fb01}ciency ratio is eta_0 = 7/12 + delta_1" without mutating stored text.
    let t3 = FormulaTranscription::new(
        &fx.source_id,
        RAW_PDF_SHA,
        3,
        None,
        "gpt-4o",
        "efficiency ratio is eta_0 = 7/12 + delta_1",
        true,
    )
    .unwrap();
    let v3 = verify_transcription_against_text_layer(
        &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-2"), "reader-1", &t3,
    )
    .unwrap();
    assert_eq!(v3.page, 3);
    assert_eq!(v3.basis, TextLayerBasis::ScanTextLayer);
    assert!(v3.basis.is_ocr_or_scan());
    assert_eq!(v3.verified.quote_ref.start_line, 6);
    assert!(v3.verified.excerpt.contains('\u{fb01}'));

    // Ocr layer on page 4: matches "S(E, V) = k_B ln Omega(E, V)".
    let t4 = FormulaTranscription::new(
        &fx.source_id,
        RAW_PDF_SHA,
        4,
        None,
        "gemini-2.5-pro",
        "S(E, V) = k_B ln Omega(E, V)",
        true,
    )
    .unwrap();
    let v4 = verify_transcription_against_text_layer(
        &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-2"), "reader-1", &t4,
    )
    .unwrap();
    assert_eq!(v4.page, 4);
    assert_eq!(v4.basis, TextLayerBasis::Ocr);
    assert!(v4.basis.is_ocr_or_scan());
    assert_eq!(v4.verified.quote_ref.start_line, 9);

    // Short inline substring (< 16 non-space chars inside a longer line) fails BOTH advisory
    // check_text_layer_agreement and verify_transcription_against_text_layer (parity with quote_core).
    assert!(!check_text_layer_agreement(
        &fx.sandbox, &fx.session, &events, &fx.source_id, RAW_PDF_SHA, 1, "a + b = c"
    ).agreed);
    let short_inline = FormulaTranscription::new(
        &fx.source_id, RAW_PDF_SHA, 1, None, "gpt-4o", "a + b = c", false,
    )
    .unwrap();
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-2"), "reader-1", &short_inline
        ),
        Err(TranscriptionGateError::TextLayerDisagrees { page: 1, .. })
    ));

    // Tampered disk file fails closed even when lines were previously supplied.
    let target = fx
        .sandbox
        .resolve_existing(SandboxRoot::Sources, &fx.session, &fx.primary_path, false)
        .unwrap();
    std::fs::write(&target.canonical_target, b"Tampered E = m c^2\n").unwrap();
    assert!(matches!(
        verify_transcription_against_text_layer(
            &fx.sandbox, &fx.session, &events, Some(&fx.ledger), Some("run-2"), "reader-1", &t1,
        ),
        Err(TranscriptionGateError::ReadingFileError(_))
    ));
}

#[test]
fn gate_rejects_ambiguous_duplicate_matches_and_treats_legacy_receipts_as_scan() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = WorkspaceSandbox::new(dir.path(), None);
    let ledger = Ledger::open(dir.path()).unwrap();
    let sid = "s-legacy-ambig";
    let src = "leg1";
    let path = format!("{src}/leg1.pdf.extracted.txt");
    let text = concat!(
        "Unique equation: alpha_0 + beta_0 = gamma_0.\n",
        "Repeated equation: X_1 + Y_1 = Z_1 + W_1.\n",
        "Repeated equation: X_1 + Y_1 = Z_1 + W_1.\n",
    );
    let dst = sandbox.resolve_for_write(SandboxRoot::Sources, sid, &path).unwrap();
    std::fs::create_dir_all(dst.target_path.parent().unwrap()).unwrap();
    std::fs::write(&dst.target_path, text.as_bytes()).unwrap();
    let sha = ledger::sha256_hex(text.as_bytes());

    // Pre-F14a legacy receipt: has pdf_page_lines and status="ok" but NO pdf_pages / basis.
    ledger
        .append(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": src,
                "origin": "upload:leg1.pdf",
                "format": "pdf",
                "status": "ok",
                "bytes": 64,
                "raw_blob": RAW_PDF_SHA,
                "primary_path": path,
                "files": [path],
                "files_sha256": { &path: &sha },
                "page_count": 1,
                "pdf_page_lines": [[1, 1, 3]]
            }),
        )
        .unwrap();
    let events = ledger.read_session(sid).unwrap();

    // Unique formula matches, and legacy receipt without scan metadata is conservatively ScanTextLayer.
    let uniq = check_text_layer_agreement(
        &sandbox, sid, &events, src, RAW_PDF_SHA, 1, "alpha_0 + beta_0 = gamma_0",
    );
    assert!(uniq.agreed);
    assert_eq!(uniq.basis, Some(TextLayerBasis::ScanTextLayer));

    // Duplicate formula on the same page fails closed as ambiguous.
    let dup = check_text_layer_agreement(
        &sandbox, sid, &events, src, RAW_PDF_SHA, 1, "X_1 + Y_1 = Z_1 + W_1",
    );
    assert!(!dup.agreed);
}

#[test]
fn gate_enforces_quote_core_parity_on_two_char_ligature_and_line_span_limits() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = WorkspaceSandbox::new(dir.path(), None);
    let ledger = Ledger::open(dir.path()).unwrap();
    let sid = "s-parity-edge";
    let src = "par1";
    let path = format!("{src}/par1.pdf.extracted.txt");

    // Line 1: 2-non-ws whole-line formula ("E0") -> quote_core requires >= 3.
    // Line 2: longer line containing an inline ligature substring ("\u{FB01}eld \u{FB03}ux = 12345")
    //         whose raw non-ws count is 13 (< 16) while normalized ligature expansion reaches 16.
    // Lines 3..=68: sparse multi-line formula spanning 66 lines (end - start = 65 >= 64).
    let mut text = String::from("E0\nPrefix text \u{FB01}eld \u{FB03}ux = 12345 suffix text\nStartFormula(\n");
    for _ in 0..64 {
        text.push('\n');
    }
    text.push_str(")EndFormula\n");
    let total_lines = text.lines().count();
    assert!(total_lines >= 68);

    let dst = sandbox.resolve_for_write(SandboxRoot::Sources, sid, &path).unwrap();
    std::fs::create_dir_all(dst.target_path.parent().unwrap()).unwrap();
    std::fs::write(&dst.target_path, text.as_bytes()).unwrap();
    let sha = ledger::sha256_hex(text.as_bytes());

    ledger
        .append(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": src,
                "origin": "upload:par1.pdf",
                "format": "pdf",
                "status": "ok",
                "bytes": 64,
                "raw_blob": RAW_PDF_SHA,
                "primary_path": path,
                "files": [path],
                "files_sha256": { &path: &sha },
                "page_count": 1,
                "pdf_pages": [{
                    "page": 1,
                    "basis": "text_layer",
                    "first_line": 1,
                    "last_line": total_lines
                }]
            }),
        )
        .unwrap();
    let supplied = ContextSuppliedPayload::new(
        "run-parity",
        "reader-1",
        vec![SuppliedRange {
            source_id: src.into(),
            path: path.clone(),
            file_sha256: sha,
            start_line: 1,
            end_line: total_lines,
        }],
    )
    .unwrap();
    ledger.append(sid, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(&supplied).unwrap()).unwrap();
    let events = ledger.read_session(sid).unwrap();

    for draft in ["E0", "field ffiux = 12345", "StartFormula( )EndFormula"] {
        let ag = check_text_layer_agreement(&sandbox, sid, &events, src, RAW_PDF_SHA, 1, draft);
        assert!(!ag.agreed, "advisory agreement must stay false for `{draft}` to match quote_core");
        let rec = FormulaTranscription::new(src, RAW_PDF_SHA, 1, None, "gpt-4o", draft, false).unwrap();
        assert!(
            matches!(
                verify_transcription_against_text_layer(
                    &sandbox, sid, &events, Some(&ledger), Some("run-parity"), "reader-1", &rec
                ),
                Err(TranscriptionGateError::TextLayerDisagrees { page: 1, .. })
            ),
            "gate must reject `{draft}`"
        );
    }
}

