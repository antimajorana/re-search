use super::*;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn touch_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn real_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

#[test]
fn default_config_is_enabled_with_eng_and_serde_defaults() {
    let cfg = OcrLocalConfig::default();
    assert!(DEFAULT_OCR_ENABLED);
    assert!(cfg.enabled);
    assert_eq!(cfg.langs, vec!["eng".to_string()]);
    assert!(cfg.tesseract_path.is_none());
    assert!(cfg.tessdata_dir.is_none());

    let from_empty: OcrLocalConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(from_empty, cfg);

    let json = serde_json::to_value(&cfg).unwrap();
    assert_eq!(json["enabled"], serde_json::json!(true));
    assert_eq!(json["langs"], serde_json::json!(["eng"]));
    assert!(json.get("tesseract_path").is_none());
    assert!(json.get("tessdata_dir").is_none());
}

#[test]
fn missing_tesseract_binary_surfaces_install_prompt() {
    let cfg = OcrLocalConfig::default();
    let probe = OcrEnvProbe::default();
    let status = ocr_status_with_probe(&cfg, &probe);

    assert!(status.enabled);
    assert!(!status.available);
    assert!(status.tesseract_path.is_none());
    assert!(status.tessdata_dir.is_none());
    assert!(status.langs.is_empty());
    assert_eq!(status.missing_langs, vec!["eng".to_string()]);
    assert_eq!(status.message, "Install Tesseract to enable OCR");
}

#[test]
fn present_binary_with_missing_eng_traineddata_surfaces_install_prompt() {
    let tmp = tempdir().unwrap();
    let exe = tmp.path().join("bin").join("tesseract");
    let empty_td = tmp.path().join("empty_tessdata");
    touch_file(&exe, b"stub-binary");
    fs::create_dir_all(&empty_td).unwrap();

    let cfg = OcrLocalConfig {
        tesseract_path: Some(exe.clone()),
        tessdata_dir: Some(empty_td),
        ..OcrLocalConfig::default()
    };
    let status = ocr_status_with_probe(&cfg, &OcrEnvProbe::default());
    assert!(!status.available);
    assert_eq!(status.tesseract_path.as_deref(), Some(exe.to_string_lossy().as_ref()));
    assert!(status.tessdata_dir.is_none());
    assert_eq!(status.missing_langs, vec!["eng".to_string()]);
    assert_eq!(status.message, "Install Tesseract to enable OCR");

    let other_td = tmp.path().join("other_tessdata");
    touch_file(&other_td.join("deu.traineddata"), b"synthetic-deu");
    let cfg_other = OcrLocalConfig {
        tesseract_path: Some(exe),
        tessdata_dir: Some(other_td.clone()),
        ..OcrLocalConfig::default()
    };
    let status_other = ocr_status_with_probe(&cfg_other, &OcrEnvProbe::default());
    assert!(!status_other.available);
    assert_eq!(
        status_other.tessdata_dir.as_deref(),
        Some(other_td.to_string_lossy().as_ref())
    );
    assert_eq!(status_other.langs, vec!["deu".to_string()]);
    assert_eq!(status_other.missing_langs, vec!["eng".to_string()]);
    assert_eq!(status_other.message, "Install Tesseract to enable OCR");
}

#[test]
fn tesseract_binary_precedence_order() {
    let tmp = tempdir().unwrap();
    let explicit_exe = tmp.path().join("explicit").join("tesseract");
    let env_exe = tmp.path().join("env").join("tesseract");
    let path_dir = tmp.path().join("path_bin");
    let path_exe = path_dir.join(if cfg!(windows) {
        "tesseract.exe"
    } else {
        "tesseract"
    });
    let std_exe = tmp.path().join("standard").join("tesseract.exe");
    let td = tmp.path().join("tessdata");

    touch_file(&explicit_exe, b"1");
    touch_file(&env_exe, b"2");
    touch_file(&path_exe, b"3");
    touch_file(&std_exe, b"4");
    touch_file(&td.join("eng.traineddata"), b"eng-model");

    let mut probe = OcrEnvProbe {
        re_search_tesseract: Some(env_exe.clone()),
        path_dirs: vec![path_dir],
        extra_exe_candidates: vec![std_exe.clone()],
        ..OcrEnvProbe::default()
    };
    let mut cfg = OcrLocalConfig {
        tesseract_path: Some(explicit_exe.clone()),
        tessdata_dir: Some(td),
        ..OcrLocalConfig::default()
    };

    let s1 = ocr_status_with_probe(&cfg, &probe);
    assert!(s1.available);
    assert_eq!(s1.tesseract_path.as_deref(), Some(explicit_exe.to_string_lossy().as_ref()));

    cfg.tesseract_path = None;
    let s2 = ocr_status_with_probe(&cfg, &probe);
    assert!(s2.available);
    assert_eq!(s2.tesseract_path.as_deref(), Some(env_exe.to_string_lossy().as_ref()));

    probe.re_search_tesseract = None;
    let s3 = ocr_status_with_probe(&cfg, &probe);
    assert!(s3.available);
    assert_eq!(s3.tesseract_path.as_deref(), Some(path_exe.to_string_lossy().as_ref()));

    probe.path_dirs.clear();
    let s4 = ocr_status_with_probe(&cfg, &probe);
    assert!(s4.available);
    assert_eq!(s4.tesseract_path.as_deref(), Some(std_exe.to_string_lossy().as_ref()));
}

#[test]
fn tessdata_dir_precedence_and_prefix_subdir() {
    let tmp = tempdir().unwrap();
    let exe_dir = tmp.path().join("install").join("bin");
    let exe = exe_dir.join("tesseract");
    let explicit_td = tmp.path().join("explicit_td");
    let prefix_direct = tmp.path().join("prefix_direct");
    let prefix_parent = tmp.path().join("prefix_parent");
    let prefix_nested = prefix_parent.join("tessdata");
    let sibling_td = exe_dir.join("tessdata");
    let share_td = tmp.path().join("install").join("share").join("tessdata");
    let extra_td = tmp.path().join("extra_td");

    touch_file(&exe, b"bin");
    touch_file(&explicit_td.join("eng.traineddata"), b"a");
    touch_file(&prefix_direct.join("eng.traineddata"), b"b");
    touch_file(&prefix_nested.join("eng.traineddata"), b"c");
    touch_file(&sibling_td.join("eng.traineddata"), b"d");
    touch_file(&share_td.join("eng.traineddata"), b"e");
    touch_file(&extra_td.join("eng.traineddata"), b"f");

    let mut cfg = OcrLocalConfig {
        tesseract_path: Some(exe),
        tessdata_dir: Some(explicit_td.clone()),
        ..OcrLocalConfig::default()
    };
    let mut probe = OcrEnvProbe {
        tessdata_prefix: Some(prefix_direct.clone()),
        extra_tessdata_candidates: vec![extra_td.clone()],
        ..OcrEnvProbe::default()
    };

    let s1 = ocr_status_with_probe(&cfg, &probe);
    assert_eq!(s1.tessdata_dir.as_deref(), Some(explicit_td.to_string_lossy().as_ref()));

    cfg.tessdata_dir = None;
    let s2 = ocr_status_with_probe(&cfg, &probe);
    assert_eq!(s2.tessdata_dir.as_deref(), Some(prefix_direct.to_string_lossy().as_ref()));

    probe.tessdata_prefix = Some(prefix_parent);
    let s3 = ocr_status_with_probe(&cfg, &probe);
    assert_eq!(s3.tessdata_dir.as_deref(), Some(prefix_nested.to_string_lossy().as_ref()));

    probe.tessdata_prefix = None;
    let s4 = ocr_status_with_probe(&cfg, &probe);
    assert_eq!(s4.tessdata_dir.as_deref(), Some(sibling_td.to_string_lossy().as_ref()));

    fs::remove_dir_all(&sibling_td).unwrap();
    let s5 = ocr_status_with_probe(&cfg, &probe);
    assert_eq!(s5.tessdata_dir.as_deref(), Some(share_td.to_string_lossy().as_ref()));

    fs::remove_dir_all(&share_td).unwrap();
    let s6 = ocr_status_with_probe(&cfg, &probe);
    assert_eq!(s6.tessdata_dir.as_deref(), Some(extra_td.to_string_lossy().as_ref()));
}

#[test]
fn hash_local_traineddata_computes_real_sha256_and_rejects_bad_inputs() {
    let tmp = tempdir().unwrap();
    let td = tmp.path().join("tessdata");
    let mut eng_bytes = vec![0x5Au8; 70_000];
    eng_bytes.extend_from_slice(b"synthetic-eng-traineddata-tail");
    let deu_bytes = b"synthetic-deu-traineddata".to_vec();

    touch_file(&td.join("eng.traineddata"), &eng_bytes);
    touch_file(&td.join("deu.traineddata"), &deu_bytes);
    touch_file(&td.join("empty.traineddata"), b"");

    let prov = hash_local_traineddata(&td, &["eng".into(), "deu".into(), "eng".into()]).unwrap();
    assert_eq!(prov.len(), 2);
    assert_eq!(prov[0].lang, "eng");
    assert_eq!(prov[0].bytes, eng_bytes.len() as u64);
    assert_eq!(prov[0].sha256, real_sha256_hex(&eng_bytes));
    assert_eq!(prov[0].sha256.len(), 64);
    assert!(prov[0].sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    assert_eq!(prov[1].lang, "deu");
    assert_eq!(prov[1].bytes, deu_bytes.len() as u64);
    assert_eq!(prov[1].sha256, real_sha256_hex(&deu_bytes));

    assert!(hash_local_traineddata(&td, &["empty".into()]).is_err());
    assert!(hash_local_traineddata(&td, &["fra".into()]).is_err());
    for bad in ["../eng", "-l", "eng;rm", "", "a_very_long_language_code_25"] {
        assert!(
            hash_local_traineddata(&td, &[bad.to_string()]).is_err(),
            "expected `{bad}` to be rejected"
        );
    }
}

#[test]
fn disabled_config_reports_availability_honestly() {
    let tmp = tempdir().unwrap();
    let exe = tmp.path().join("tesseract");
    let td = tmp.path().join("tessdata");
    touch_file(&exe, b"bin");
    touch_file(&td.join("eng.traineddata"), b"model");

    let cfg_present = OcrLocalConfig {
        enabled: false,
        tesseract_path: Some(exe),
        tessdata_dir: Some(td),
        langs: vec!["eng".into()],
    };
    let status_present = ocr_status_with_probe(&cfg_present, &OcrEnvProbe::default());
    assert!(!status_present.enabled);
    assert!(status_present.available);
    assert_eq!(
        status_present.message,
        "Local Tesseract detected (OCR disabled in settings)."
    );

    let cfg_missing = OcrLocalConfig {
        enabled: false,
        ..OcrLocalConfig::default()
    };
    let status_missing = ocr_status_with_probe(&cfg_missing, &OcrEnvProbe::default());
    assert!(!status_missing.enabled);
    assert!(!status_missing.available);
    assert_eq!(status_missing.message, "Install Tesseract to enable OCR");
}

#[test]
fn format_ocr_engine_id_parses_versions_and_fallbacks() {
    assert_eq!(format_ocr_engine_id(Some("tesseract 5.3.4")), "tesseract/5.3.4");
    assert_eq!(
        format_ocr_engine_id(Some("tesseract v5.3.3.20231005")),
        "tesseract/5.3.3.20231005"
    );
    assert_eq!(
        format_ocr_engine_id(Some("tesseract 4.1.1\n leptonica-1.82.0")),
        "tesseract/4.1.1"
    );
    assert_eq!(format_ocr_engine_id(None), "tesseract/local");
    assert_eq!(format_ocr_engine_id(Some("")), "tesseract/local");
    assert_eq!(format_ocr_engine_id(Some("tesseract")), "tesseract/local");
    assert_eq!(format_ocr_engine_id(Some("unknown binary")), "tesseract/local");
}

#[test]
fn validate_effective_langs_and_version_line_sanitization() {
    assert_eq!(effective_langs(&[]).unwrap(), vec!["eng".to_string()]);
    assert_eq!(
        effective_langs(&["chi_sim".into(), "eng-fast".into(), "chi_sim".into()]).unwrap(),
        vec!["chi_sim".to_string(), "eng-fast".to_string()]
    );
    assert!(validate_ocr_lang("../eng").is_err());
    assert!(validate_ocr_lang("-l").is_err());
    assert!(validate_ocr_lang("eng;rm").is_err());

    let extracted = extract_version_line("", "leptonica\ntesseract  5.3.4\x07\t(build)");
    assert_eq!(extracted.as_deref(), Some("tesseract 5.3.4 (build)"));

    let long_line = format!("tesseract {}", "1".repeat(100));
    let capped = sanitize_version_line(&long_line).unwrap();
    assert_eq!(capped.chars().count(), MAX_VERSION_CHARS);
}
