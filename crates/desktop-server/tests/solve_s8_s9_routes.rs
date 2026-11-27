//! Integration tests for Solve S8 (`attach-source`, `ack`) and S9 (`check`, `recheck`) routes.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::library::{FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::{PageBasis, PageQualityMetrics, PageRecord};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::solve_model::{
    compute_entry_sha256, event_kinds, StepEntryType, StepOrigin,
};
use research_tools::solve_status::wording;
use research_tools::solve_steps::run_step_record;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use tempfile::TempDir;

fn create_solve_session(port: u16, tok: &str, sid: &str) {
    let auth = [("X-ReSearch-Token", tok)];
    let body = serde_json::to_vec(&json!({ "id": sid, "mode": "solve" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &body);
    assert_eq!(st, 200, "create solve session failed: {}", String::from_utf8_lossy(&raw));
}

#[test]
fn solve_attach_source_library_and_ocr_pdf_page() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let _cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    create_solve_session(port, &tok, "s-s8-attach");

    // 1. Seed a library paper with a long line (>300 chars) to also test 300-char excerpt clip.
    let ledger = Ledger::open(&data_dir).unwrap();
    let long_line = format!("Equation (4.1) holds for all couplings {}", "a".repeat(320));
    let tex_bytes = format!("Intro line\n{long_line}\nClosing line\n").into_bytes();
    let blob_sha = ledger.put_blob(&tex_bytes).unwrap();
    let key = PaperKey::parse("arxiv:2501.00042").unwrap();
    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds { arxiv: Some("2501.00042".into()), ..Default::default() },
        title: "Long-Line Paper".into(),
        authors: vec!["A. Physicist".into()],
        year: Some(2025),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_fulltext(&FulltextRecord {
        key,
        version: Some("v1".into()),
        raw_blob: Some(blob_sha),
        format: "tex".into(),
        bytes: tex_bytes.len() as u64,
        origin: "library".into(),
        fetched_ms: 1,
        file_path: None,
        file_size: None,
        file_mtime_ms: None,
        file_sha256: None,
    })
    .unwrap();

    // Bad quote is rejected with 400.
    let bad_req = serde_json::to_vec(&json!({
        "library_key": "arxiv:2501.00042",
        "start_line": 2,
        "end_line": 2,
        "quote": "This text is not on line 2"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-s8-attach/solve/attach-source", &auth, &bad_req);
    assert_eq!(st, 400);

    // Matching library quote succeeds and clips excerpt <= 300 chars.
    let ok_req = serde_json::to_vec(&json!({
        "library_key": "arxiv:2501.00042",
        "start_line": 2,
        "end_line": 2,
        "quote": "Equation (4.1) holds for all couplings"
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-s8-attach/solve/attach-source", &auth, &ok_req);
    assert_eq!(st, 200, "attach-source failed: {}", String::from_utf8_lossy(&raw));
    let val: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(val["ok"], true);
    assert_eq!(val["ocr"], false);
    assert!(val["excerpt"].as_str().unwrap().chars().count() <= 300);
    assert_eq!(val["quote_ref"]["evidence_seq"], val["evidence_seq"]);

    // 2. Scanned PDF page ingestion marks `ocr: true` on `attach-source` & triggers OCR badge flag.
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let env = ResearchEnv::with_workspace(data_dir.clone(), ws).unwrap();
    let sandbox = env.sandbox();
    let pdf_src_id = "src-scan-1";
    let rel_path = format!("{pdf_src_id}/scan.pdf.extracted.txt");
    let extracted_text = "Page 1 line 1\nScanned theorem: Z_1 = 1 + g^2\nPage 1 line 3\n";
    let file_sha = ledger::sha256_hex(extracted_text.as_bytes());
    let canon_sources = sandbox.canonical_root(SandboxRoot::Sources, "s-s8-attach", true).unwrap().unwrap();
    WorkspaceSandbox::atomic_write_in_root(&canon_sources, &[pdf_src_id.into(), "scan.pdf.extracted.txt".into()], extracted_text.as_bytes()).unwrap();
    let pages = vec![PageRecord {
        page: 1,
        basis: PageBasis::ScanTextLayer,
        first_line: Some(1),
        last_line: Some(3),
        metrics: PageQualityMetrics {
            char_count: extracted_text.len(),
            alpha_count: 30,
            word_count: 10,
            image_coverage_pct: 95,
            has_invisible_text: true,
            width_pt: 612,
            height_pt: 792,
        },
        reason: Some("scanned page".into()),
    }];
    ledger
        .append_with_hash(
            "s-s8-attach",
            "source.ingested",
            1,
            &json!({
                "source_id": pdf_src_id,
                "origin": "upload:scan.pdf",
                "format": "pdf",
                "status": "ok",
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { &rel_path: &file_sha },
                "pdf_pages": pages,
            }),
        )
        .unwrap();

    let scan_req = serde_json::to_vec(&json!({
        "source_id": pdf_src_id,
        "start_line": 2,
        "end_line": 2,
        "quote": "Scanned theorem: Z_1 = 1 + g^2"
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-s8-attach/solve/attach-source", &auth, &scan_req);
    assert_eq!(st, 200, "scan attach failed: {}", String::from_utf8_lossy(&raw));
    let scan_val: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(scan_val["ocr"], true);
    assert_eq!(scan_val["page"], 1);

    // Cite the scanned quote through `step_record` after supplying the line (R17).
    ledger
        .append_with_hash(
            "s-s8-attach",
            event_kinds::CONTEXT_SUPPLIED,
            1,
            &json!({
                "run_id": "run-s8",
                "caller": "solver",
                "ranges": [{
                    "source_id": pdf_src_id,
                    "path": rel_path,
                    "file_sha256": file_sha,
                    "start_line": 2,
                    "end_line": 2
                }]
            }),
        )
        .unwrap();
    let step = run_step_record(
        &env,
        &ledger,
        "s-s8-attach",
        "solver",
        &json!({
            "entry_type": "reference_lookup",
            "summary": "Scanned citation",
            "details": "From scanned scan.pdf page 1",
            "source_ref": scan_val["quote_ref"]
        }),
    )
    .unwrap();
    assert_eq!(step["own_tier"], "cited");
    assert_eq!(step["badge"]["label"], wording::BADGE_CITED);
    let flags = step["badge"]["flags"].as_array().unwrap();
    assert!(flags.iter().any(|f| f == wording::FLAG_OCR_SOURCE), "flags={flags:?}");
}

#[test]
fn solve_ack_validates_hash_and_upgrades_tiers() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    create_solve_session(port, &tok, "s-s8-ack");
    let ledger = Ledger::open(&data_dir).unwrap();

    // e-1: unacknowledged solver assumption; e-2: unchecked derivation; e-3: superseded assumption.
    let e1_hash = compute_entry_sha256(None, "Assume g > 0", "Positive coupling");
    ledger
        .append_with_hash(
            "s-s8-ack",
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": StepEntryType::Assumption.as_str(),
                "summary": "Assume g > 0",
                "details": "Positive coupling",
                "depends_on": [],
                "origin": StepOrigin::Solver,
                "entry_sha256": e1_hash,
            }),
        )
        .unwrap();
    let e2_hash = compute_entry_sha256(None, "Manual step", "Algebra by hand");
    ledger
        .append_with_hash(
            "s-s8-ack",
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-2",
                "entry_type": StepEntryType::Derivation.as_str(),
                "summary": "Manual step",
                "details": "Algebra by hand",
                "depends_on": ["e-1"],
                "origin": StepOrigin::Solver,
                "entry_sha256": e2_hash,
            }),
        )
        .unwrap();
    let e3_hash = compute_entry_sha256(None, "Old assumption", "Replaced");
    ledger
        .append_with_hash(
            "s-s8-ack",
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-3",
                "entry_type": StepEntryType::Assumption.as_str(),
                "summary": "Old assumption",
                "details": "Replaced",
                "depends_on": [],
                "origin": StepOrigin::Solver,
                "entry_sha256": e3_hash,
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            "s-s8-ack",
            event_kinds::PROOF_SUPERSEDE,
            1,
            &json!({ "entry_id": "e-3", "replaced_by": "e-1", "reason": "duplicate" }),
        )
        .unwrap();

    // Wrong hash -> 409; superseded step -> 409; wrong kind on derivation -> 400.
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-s8-ack/solve/ack",
        &auth,
        &serde_json::to_vec(&json!({ "entry_id": "e-1", "kind": "assumption", "entry_sha256": "deadbeef" })).unwrap(),
    );
    assert_eq!(st, 409);

    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-s8-ack/solve/ack",
        &auth,
        &serde_json::to_vec(&json!({ "entry_id": "e-3", "kind": "assumption", "entry_sha256": e3_hash })).unwrap(),
    );
    assert_eq!(st, 409);

    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-s8-ack/solve/ack",
        &auth,
        &serde_json::to_vec(&json!({ "entry_id": "e-2", "kind": "assumption", "entry_sha256": e2_hash })).unwrap(),
    );
    assert_eq!(st, 400);

    // Valid assumption ack -> Given.
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions/s-s8-ack/solve/ack",
        &auth,
        &serde_json::to_vec(&json!({ "entry_id": "e-1", "kind": "assumption", "entry_sha256": e1_hash })).unwrap(),
    );
    assert_eq!(st, 200, "ack assumption failed: {}", String::from_utf8_lossy(&raw));
    let v1: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(v1["badge"]["effective"], "given");
    assert_eq!(v1["badge"]["label"], wording::BADGE_GIVEN);

    // Valid hand_checked ack -> Checked by you.
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions/s-s8-ack/solve/ack",
        &auth,
        &serde_json::to_vec(&json!({ "entry_id": "e-2", "kind": "hand_checked", "entry_sha256": e2_hash })).unwrap(),
    );
    assert_eq!(st, 200, "ack hand_checked failed: {}", String::from_utf8_lossy(&raw));
    let v2: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(v2["badge"]["effective"], "checked_by_you");
    assert_eq!(v2["badge"]["label"], wording::BADGE_CHECKED_BY_YOU);
}
