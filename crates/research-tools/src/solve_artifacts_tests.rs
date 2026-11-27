use super::*;
use crate::solve_artifacts_sniff::PNG_MAGIC;
use crate::solve_model::event_kinds;
use crate::solve_state::SolveState;
use ledger::Ledger;
use serde_json::json;
use tempfile::TempDir;

fn open_solve_session() -> (TempDir, Ledger, PathBuf, &'static str) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let scratch = tmp.path().join("scratch");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&scratch).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-art-test";
    ledger.append(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    (tmp, ledger, scratch, sid)
}

#[test]
fn sniff_artifact_mime_ignores_spoofed_extensions_and_rejects_dangerous_content() {
    // Valid PNG, PDF, SVG, JSON, CSV, plain text.
    let mut png = PNG_MAGIC.to_vec();
    png.extend_from_slice(b"\x00\x00\x00\rIHDR\x00\x00\x00\x01");
    assert_eq!(sniff_artifact_mime(&png).unwrap(), MIME_PNG);
    assert_eq!(sniff_artifact_mime(b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n").unwrap(), MIME_PDF);
    assert_eq!(
        sniff_artifact_mime(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"4\"/></svg>").unwrap(),
        MIME_SVG
    );
    assert_eq!(
        sniff_artifact_mime(b"<?xml version=\"1.0\"?><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"><svg></svg>")
            .unwrap(),
        MIME_SVG
    );
    assert_eq!(sniff_artifact_mime(br#"{"residual": 1e-12, "points": [1, 2]}"#).unwrap(), MIME_JSON);
    assert_eq!(sniff_artifact_mime(b"x,y\n1.0,2.0\n3.0,4.0\n").unwrap(), MIME_CSV);
    assert_eq!(sniff_artifact_mime(b"convergence summary line\n").unwrap(), MIME_PLAIN);

    // Explicit rejections: executables, archives, HTML/scripts, binary garbage, empty.
    assert!(sniff_artifact_mime(b"").is_err());
    assert!(sniff_artifact_mime(b"MZ\x90\x00\x03\x00").is_err());
    assert!(sniff_artifact_mime(b"\x7fELF\x02\x01\x01\x00").is_err());
    assert!(sniff_artifact_mime(b"\xcf\xfa\xed\xfe\x07\x00\x00\x01").is_err());
    assert!(sniff_artifact_mime(b"\xfe\xed\xfa\xce\x00\x00").is_err());
    assert!(sniff_artifact_mime(b"\xca\xfe\xba\xbe\x00\x00").is_err());
    assert!(sniff_artifact_mime(b"PK\x03\x04\x14\x00\x00\x00").is_err());
    assert!(sniff_artifact_mime(b"\x1f\x8b\x08\x00\x00\x00").is_err());
    assert!(sniff_artifact_mime(b"<!DOCTYPE html><html><body>xss</body></html>").is_err());
    assert!(sniff_artifact_mime(b"<html><head></head><body>hi</body></html>").is_err());
    assert!(sniff_artifact_mime(b"<script>alert(1)</script>").is_err());
    assert!(sniff_artifact_mime(b"<svg><script>alert(1)</script></svg>").is_err());
    assert!(sniff_artifact_mime(b"\xff\xfe\x00\x01\x99\xaa\xbb").is_err());
    assert!(sniff_artifact_mime(b"hello\x00world").is_err());
}

#[test]
fn harvest_sniffs_magic_bytes_stores_blobs_and_folds_into_solve_state() {
    let (_tmp, ledger, scratch, sid) = open_solve_session();
    let art_dir = scratch.join("artifacts");
    fs::create_dir_all(art_dir.join("nested")).unwrap();

    // Spoofed extensions must be ignored: magic bytes govern `mime`.
    let mut png = PNG_MAGIC.to_vec();
    png.extend_from_slice(b"fake-png-ihdr");
    fs::write(art_dir.join("plot.txt"), &png).unwrap();
    fs::write(art_dir.join("nested/table.png"), b"x,y\n1,2\n3,4\n").unwrap();
    fs::write(art_dir.join("vector.svg"), b"<svg viewBox=\"0 0 10 10\"><path d=\"M0 0L10 10\"/></svg>").unwrap();
    fs::write(art_dir.join("report.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
    fs::write(art_dir.join("metrics.json"), br#"{"ok": true}"#).unwrap();

    // Disallowed files become warnings without crashing the harvest.
    fs::write(art_dir.join("malware.png"), b"MZ\x90\x00not-a-png").unwrap();
    fs::write(art_dir.join("page.csv"), b"<!doctype html><html><body>bad</body></html>").unwrap();
    fs::write(art_dir.join("archive.txt"), b"PK\x03\x04zip").unwrap();
    fs::write(art_dir.join("garbage.dat"), b"\x00\x01\xff\xfe").unwrap();

    let report = harvest_scratch_artifacts(&ledger, sid, &scratch, "call-1", Some("e-3")).unwrap();
    assert_eq!(report.harvested.len(), 5);
    assert_eq!(report.warnings.len(), 4);
    assert_eq!(report.total_artifacts_in_run, 5);

    let events = ledger.read_verified_session(sid).unwrap().1;
    let state = SolveState::from_events(&events);
    assert_eq!(state.artifacts.len(), 5);

    let by_name: std::collections::BTreeMap<_, _> =
        state.artifacts.iter().map(|a| (a.name.as_str(), a)).collect();
    assert_eq!(by_name["plot.txt"].mime, MIME_PNG);
    assert_eq!(by_name["nested/table.png"].mime, MIME_CSV);
    assert_eq!(by_name["vector.svg"].mime, MIME_SVG);
    assert_eq!(by_name["report.pdf"].mime, MIME_PDF);
    assert_eq!(by_name["metrics.json"].mime, MIME_JSON);
    assert_eq!(by_name["plot.txt"].call_id, "call-1");
    assert_eq!(by_name["plot.txt"].entry_id.as_deref(), Some("e-3"));
    assert_eq!(ledger.get_blob(&by_name["plot.txt"].blob).unwrap(), png);

    // Re-running harvest on unchanged files is idempotent (no duplicate events).
    let rerun = harvest_scratch_artifacts(&ledger, sid, &scratch, "call-2", None).unwrap();
    assert!(rerun.harvested.is_empty());
    assert_eq!(rerun.total_artifacts_in_run, 5);
}

#[test]
fn harvest_enforces_16_file_and_20_mib_per_run_caps() {
    let (_tmp, ledger, scratch, sid) = open_solve_session();
    let art_dir = scratch.join("artifacts");
    fs::create_dir_all(&art_dir).unwrap();

    for i in 0..18 {
        fs::write(art_dir.join(format!("f_{i:02}.txt")), format!("line {i}\n")).unwrap();
    }
    let rep = harvest_scratch_artifacts(&ledger, sid, &scratch, "call-cap", None).unwrap();
    assert_eq!(rep.harvested.len(), MAX_ARTIFACTS_PER_RUN);
    assert_eq!(rep.warnings.len(), 2);
    assert!(rep.warnings[0].reason.contains("16"));

    // Test 20 MiB total cap on a fresh session.
    let (_tmp2, ledger2, scratch2, sid2) = open_solve_session();
    let art_dir2 = scratch2.join("artifacts");
    fs::create_dir_all(&art_dir2).unwrap();
    let chunk_11m = vec![b'a'; 11 * 1024 * 1024];
    fs::write(art_dir2.join("a_11m.txt"), &chunk_11m).unwrap();
    fs::write(art_dir2.join("b_11m.txt"), &chunk_11m).unwrap();
    fs::write(art_dir2.join("c_small.txt"), b"fits\n").unwrap();

    let rep2 = harvest_scratch_artifacts(&ledger2, sid2, &scratch2, "call-bytes", None).unwrap();
    assert_eq!(rep2.harvested.len(), 2);
    assert_eq!(rep2.harvested[0].artifact.name, "a_11m.txt");
    assert_eq!(rep2.harvested[1].artifact.name, "c_small.txt");
    assert_eq!(rep2.warnings.len(), 1);
    assert_eq!(rep2.warnings[0].name, "b_11m.txt");
    assert!(rep2.warnings[0].reason.contains("byte cap"));
}

#[test]
fn harvest_and_stage_reject_traversal_absolute_paths_and_symlinks() {
    for bad in ["", "../escape.csv", "a/../b.csv", "/etc/passwd", "C:\\x.csv", "CON.csv", "a/CON.txt", ".re-search-tmp-1"] {
        assert!(validate_artifact_rel_path(bad).is_err(), "`{bad}` must be rejected");
    }
    assert!(validate_artifact_rel_path("sub/data.csv").is_ok());

    let (tmp, ledger, scratch, sid) = open_solve_session();
    let art_dir = scratch.join("artifacts");
    fs::create_dir_all(&art_dir).unwrap();
    let secret = tmp.path().join("secret.txt");
    fs::write(&secret, b"top-secret\n").unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, art_dir.join("leak.txt")).unwrap();
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_file(&secret, art_dir.join("leak.txt"));

    #[cfg(unix)]
    {
        let rep = harvest_scratch_artifacts(&ledger, sid, &scratch, "call-sym", None).unwrap();
        assert!(rep.harvested.is_empty());
        assert_eq!(rep.warnings.len(), 1);
        assert!(rep.warnings[0].reason.contains("symlink"));
    }

    // Also refuse symlinked `scratch/artifacts` directory itself.
    let (_tmp2, ledger2, scratch2, sid2) = open_solve_session();
    let outside_dir = tmp.path().join("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("x.txt"), b"x\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_dir, scratch2.join("artifacts")).unwrap();
        let err = harvest_scratch_artifacts(&ledger2, sid2, &scratch2, "call-symdir", None).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
    }
}

#[test]
fn stage_readonly_inputs_copies_safely_sets_readonly_and_leaves_originals_untouched() {
    let (tmp, _ledger, scratch, _sid) = open_solve_session();
    let user_dir = tmp.path().join("user_data");
    fs::create_dir_all(&user_dir).unwrap();

    let csv_orig = user_dir.join("measurements.csv");
    let json_orig = user_dir.join("params.json");
    let npz_orig = user_dir.join("grid.npz");
    fs::write(&csv_orig, b"t,v\n0,1.5\n1,2.5\n").unwrap();
    fs::write(&json_orig, br#"{"dt": 0.01}"#).unwrap();
    fs::write(&npz_orig, b"PK\x03\x04npz-fixture-bytes").unwrap();

    let orig_csv_readonly = fs::metadata(&csv_orig).unwrap().permissions().readonly();
    assert!(!orig_csv_readonly);

    let staged = stage_readonly_inputs(
        &scratch,
        &[
            StagedInputSpec { name: "measurements.csv".into(), source_path: csv_orig.clone() },
            StagedInputSpec { name: "nested/params.json".into(), source_path: json_orig.clone() },
            StagedInputSpec { name: "grid.npz".into(), source_path: npz_orig.clone() },
        ],
    )
    .unwrap();
    assert_eq!(staged.len(), 3);
    assert_eq!(staged[0].name, "measurements.csv");
    assert_eq!(staged[1].name, "nested/params.json");
    assert_eq!(fs::read(&staged[0].staged_path).unwrap(), b"t,v\n0,1.5\n1,2.5\n");
    assert_eq!(fs::read(&staged[2].staged_path).unwrap(), b"PK\x03\x04npz-fixture-bytes");

    for item in &staged {
        assert!(fs::metadata(&item.staged_path).unwrap().permissions().readonly());
        // Writing to a staged file must fail with PermissionDenied (for non-root users).
        assert!(fs::OpenOptions::new().write(true).open(&item.staged_path).is_err());
    }
    // Original user files remain untouched and writable.
    assert!(!fs::metadata(&csv_orig).unwrap().permissions().readonly());
    assert_eq!(fs::read(&csv_orig).unwrap(), b"t,v\n0,1.5\n1,2.5\n");

    // Re-staging updated content succeeds even though prior staged files were read-only.
    fs::write(&csv_orig, b"t,v\n0,9.0\n").unwrap();
    let restaged = stage_readonly_inputs(
        &scratch,
        &[StagedInputSpec { name: "measurements.csv".into(), source_path: csv_orig.clone() }],
    )
    .unwrap();
    assert_eq!(fs::read(&restaged[0].staged_path).unwrap(), b"t,v\n0,9.0\n");

    // Unsafe input names and duplicate names are rejected.
    assert!(stage_readonly_inputs(
        &scratch,
        &[StagedInputSpec { name: "../escape.csv".into(), source_path: csv_orig.clone() }]
    )
    .is_err());
    assert!(stage_readonly_inputs(
        &scratch,
        &[
            StagedInputSpec { name: "a.csv".into(), source_path: csv_orig.clone() },
            StagedInputSpec { name: "a.csv".into(), source_path: json_orig },
        ]
    )
    .is_err());
}
