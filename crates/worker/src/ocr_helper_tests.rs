use super::*;

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    fn new(label: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "research-ocr-helper-{label}-{}-{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn sample_png() -> Vec<u8> {
    let mut bytes = PNG_SIGNATURE.to_vec();
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R']);
    bytes
}

fn valid_input(page: usize) -> OcrPageInput {
    OcrPageInput {
        page,
        png_bytes: sample_png(),
        dpi: 300,
        langs: vec!["eng".to_string()],
    }
}

#[test]
fn validate_ocr_input_accepts_valid_inputs_and_defaults_lang() {
    let mut input = valid_input(1);
    input.langs.clear();
    assert_eq!(validate_ocr_input(&input).unwrap(), "eng");

    input.dpi = MIN_OCR_DPI;
    input.langs = vec!["eng".into(), "deu".into(), "chi_sim".into(), "script-Latn".into()];
    assert_eq!(validate_ocr_input(&input).unwrap(), "eng+deu+chi_sim+script-Latn");

    input.dpi = MAX_OCR_DPI;
    input.langs = vec!["a".repeat(24)];
    assert_eq!(validate_ocr_input(&input).unwrap(), "a".repeat(24));
}

#[test]
fn validate_ocr_input_rejects_bad_page_png_dpi_and_langs() {
    let bad_page = valid_input(0);
    assert!(matches!(validate_ocr_input(&bad_page), Err(OcrHelperError::InvalidInput(_))));

    let mut empty_png = valid_input(1);
    empty_png.png_bytes.clear();
    assert!(matches!(validate_ocr_input(&empty_png), Err(OcrHelperError::InvalidInput(_))));

    let mut bad_header = valid_input(1);
    bad_header.png_bytes = b"JFIF\x00\x00\x00\x00extra".to_vec();
    assert!(matches!(validate_ocr_input(&bad_header), Err(OcrHelperError::InvalidInput(_))));

    let mut short_png = valid_input(1);
    short_png.png_bytes = b"\x89PNG".to_vec();
    assert!(matches!(validate_ocr_input(&short_png), Err(OcrHelperError::InvalidInput(_))));

    let mut oversized = valid_input(1);
    oversized.png_bytes = vec![0u8; MAX_OCR_INPUT_PNG_BYTES + 1];
    oversized.png_bytes[..8].copy_from_slice(PNG_SIGNATURE);
    assert!(matches!(validate_ocr_input(&oversized), Err(OcrHelperError::InvalidInput(_))));

    for bad_dpi in [0, MIN_OCR_DPI - 1, MAX_OCR_DPI + 1, u16::MAX] {
        let mut input = valid_input(1);
        input.dpi = bad_dpi;
        assert!(
            matches!(validate_ocr_input(&input), Err(OcrHelperError::InvalidInput(_))),
            "dpi {bad_dpi} should fail"
        );
    }

    let long_lang = "a".repeat(25);
    for bad_lang in [
        "",
        "-l",
        "--psm",
        "../eng",
        "..\\eng",
        "eng/deu",
        "eng+deu",
        "eng deu",
        "eng.traineddata",
        long_lang.as_str(),
    ] {
        let mut input = valid_input(1);
        input.langs = vec![bad_lang.to_string()];
        assert!(
            matches!(validate_ocr_input(&input), Err(OcrHelperError::InvalidInput(_))),
            "lang `{bad_lang}` should fail"
        );
    }
}

#[test]
fn fake_ocr_engine_records_calls_is_deterministic_and_propagates_errors() {
    let engine = FakeOcrEngine::new("fake/1.0")
        .with_page(1, "Theorem 1\r\nProof\0\rQED")
        .with_page_error(2, OcrHelperError::Timeout(Duration::from_secs(3)))
        .with_page_error(3, "simulated tesseract crash");

    assert_eq!(engine.engine_id(), "fake/1.0");
    assert_eq!(
        engine.containment(),
        Containment {
            backend: "fake-ocr",
            filesystem_restricted: true,
            network_restricted: true,
            process_tree_killed_with_core: true,
        }
    );

    let p1 = engine.ocr_page(&valid_input(1)).unwrap();
    assert_eq!(p1.text, "Theorem 1\nProof\nQED");
    assert_eq!(p1.engine_id, "fake/1.0");
    assert!(p1.containment.is_contained());

    let err2 = engine.ocr_page(&valid_input(2)).unwrap_err();
    assert_eq!(err2, OcrHelperError::Timeout(Duration::from_secs(3)));

    let err3 = engine.ocr_page(&valid_input(3)).unwrap_err();
    assert_eq!(
        err3,
        OcrHelperError::ProcessFailed {
            exit_code: Some(1),
            stderr: "simulated tesseract crash".into(),
        }
    );

    let p4 = engine.ocr_page(&valid_input(4)).unwrap();
    assert_eq!(p4.text, "");

    // Invalid inputs fail validation before recording a call.
    let bad = valid_input(0);
    assert!(matches!(engine.ocr_page(&bad), Err(OcrHelperError::InvalidInput(_))));

    let calls = engine.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0], FakeOcrCall { page: 1, dpi: 300, langs: vec!["eng".into()], png_len: 16 });
    assert_eq!(calls[1].page, 2);
    assert_eq!(calls[2].page, 3);
    assert_eq!(calls[3].page, 4);
}

#[test]
fn tesseract_cli_engine_missing_binary_or_tessdata_returns_unavailable() {
    let tmp = TestTempDir::new("missing-bin");
    let data_dir = tmp.path.join("data");
    let tessdata_dir = tmp.path.join("tessdata");
    fs::create_dir_all(&tessdata_dir).unwrap();

    let engine = TesseractCliEngine::new(TesseractCliConfig {
        tesseract_exe: tmp.path.join("no-such-tesseract"),
        tessdata_dir: tessdata_dir.clone(),
        data_dir: data_dir.clone(),
        engine_id: "tesseract/5.3.4".into(),
        page_timeout: DEFAULT_OCR_PAGE_TIMEOUT,
        max_output_bytes: MAX_OCR_OUTPUT_TEXT_BYTES,
    });

    assert_eq!(engine.engine_id(), "tesseract/5.3.4");
    #[cfg(windows)]
    assert_eq!(engine.containment().backend, "appcontainer-ocr");
    #[cfg(not(windows))]
    assert_eq!(engine.containment().backend, "isolated-tempdir");

    let err = engine.ocr_page(&valid_input(1)).unwrap_err();
    assert!(matches!(err, OcrHelperError::Unavailable(_)), "got {err:?}");
    assert!(err.to_string().contains("Install Tesseract"));

    // Existing binary path but missing tessdata directory also returns Unavailable.
    let dummy_bin = tmp.path.join("fake-tesseract");
    fs::write(&dummy_bin, b"not-a-real-binary").unwrap();
    let engine_no_tessdata = TesseractCliEngine::new(TesseractCliConfig {
        tesseract_exe: dummy_bin,
        tessdata_dir: tmp.path.join("missing-tessdata"),
        data_dir,
        engine_id: "tesseract/5.3.4".into(),
        page_timeout: DEFAULT_OCR_PAGE_TIMEOUT,
        max_output_bytes: MAX_OCR_OUTPUT_TEXT_BYTES,
    });
    let err2 = engine_no_tessdata.ocr_page(&valid_input(1)).unwrap_err();
    assert!(matches!(err2, OcrHelperError::Unavailable(_)), "got {err2:?}");
}

#[test]
fn normalize_ocr_text_and_read_bounded_text_enforce_limits() {
    assert_eq!(normalize_ocr_text("a\r\nb\rc\0d\ne"), "a\nb\ncd\ne");
    assert_eq!(normalize_ocr_text("∫ α + β\r\n"), "∫ α + β\n");

    let tmp = TestTempDir::new("bounded-read");
    let ok_file = tmp.path.join("result.txt");
    fs::write(&ok_file, b"line1\r\n\0line2").unwrap();

    // Exact byte count succeeds and normalizes content.
    let text = read_bounded_text(&ok_file, 13).unwrap();
    assert_eq!(text, "line1\nline2");

    // One byte over max_bytes returns OutputTooLarge.
    let too_large = read_bounded_text(&ok_file, 12).unwrap_err();
    assert_eq!(too_large, OcrHelperError::OutputTooLarge { max_bytes: 12 });
    assert!(too_large.to_string().contains("12"));

    // Missing output file reports ProcessFailed.
    let missing = read_bounded_text(&tmp.path.join("nope.txt"), 1024).unwrap_err();
    assert!(matches!(missing, OcrHelperError::ProcessFailed { exit_code: None, .. }));
}

#[test]
fn tesseract_cli_engine_rejects_missing_traineddata_file() {
    let tmp = TestTempDir::new("missing-traineddata");
    let data_dir = tmp.path.join("data");
    let tessdata_dir = tmp.path.join("tessdata");
    fs::create_dir_all(&tessdata_dir).unwrap();
    let fake_exe = tmp.path.join("tesseract-stub");
    fs::write(&fake_exe, b"stub").unwrap();

    let engine = TesseractCliEngine::new(TesseractCliConfig {
        tesseract_exe: fake_exe,
        tessdata_dir,
        data_dir,
        engine_id: "tesseract/test".into(),
        page_timeout: Duration::from_secs(5),
        max_output_bytes: 256,
    });
    let err = engine.ocr_page(&valid_input(1)).unwrap_err();
    assert!(
        matches!(err, OcrHelperError::Unavailable(_)),
        "expected Unavailable when eng.traineddata is absent, got {err:?}"
    );
}

#[test]
fn wait_with_timeout_returns_exit_kills_on_timeout_and_kills_on_error() {
    let mut polls = 0usize;
    let mut killed = false;
    let ok = wait_with_timeout(
        &mut polls,
        Duration::from_secs(1),
        |p| {
            *p += 1;
            Ok((*p >= 2).then_some(0u32))
        },
        |_| killed = true,
    )
    .unwrap();
    assert_eq!(ok, 0);
    assert!(!killed);

    let mut dummy = ();
    let mut timed_out_killed = false;
    let err = wait_with_timeout(
        &mut dummy,
        Duration::from_millis(25),
        |_| Ok(None::<u32>),
        |_| timed_out_killed = true,
    )
    .unwrap_err();
    assert_eq!(err, OcrHelperError::Timeout(Duration::from_millis(25)));
    assert!(timed_out_killed);

    let mut err_killed = false;
    let err = wait_with_timeout(
        &mut dummy,
        Duration::from_secs(1),
        |_| Err::<Option<u32>, _>(std::io::Error::other("poll failed")),
        |_| err_killed = true,
    )
    .unwrap_err();
    assert!(matches!(err, OcrHelperError::Spawn(_)));
    assert!(err_killed);
}

