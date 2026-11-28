//! Unit tests for X9 `idea_fetch_papers` handler, resolution order, F10 `paper_text` PDF cache,
//! Rule P3 fetch budget enforcement, and Rule C9 user-library ingestion resilience.

use super::*;
use crate::idea_tools::{
    idea_fetch_papers_def, register_explore_v1_tools, IDEA_FETCH_PAPERS,
};
use crate::library::{FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord};
use crate::net_rate::{FakeRateClock, NetRateGate};
use crate::quote_core::verify_quote;
use crate::{research_policy, LiteratureEndpoints};
use flate2::write::GzEncoder;
use flate2::Compression;
use ledger::{sha256_hex, Ledger};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router};

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "idea-fetch-test".into(),
        }
    }
}

fn build_minimal_pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len().max(1);
    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();
    let first_page_id = 4usize;
    let first_stream_id = first_page_id + n;
    let kids: String = (0..n)
        .map(|i| format!("{} 0 R", first_page_id + i))
        .collect::<Vec<_>>()
        .join(" ");
    objs.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    objs.push((2, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes()));
    objs.push((3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()));
    for (i, _) in pages.iter().enumerate() {
        let pid = first_page_id + i;
        let sid = first_stream_id + i;
        let dict = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {sid} 0 R >>"
        );
        objs.push((pid, dict.into_bytes()));
    }
    for (i, text) in pages.iter().enumerate() {
        let sid = first_stream_id + i;
        let escaped = text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
        let body = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET");
        let obj = format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len());
        objs.push((sid, obj.into_bytes()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objs.len() + 1];
    for (id, body) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    let total_xref = objs.len() + 1;
    writeln!(out, "xref\n0 {total_xref}\n0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(
        out,
        "trailer\n<< /Size {total_xref} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF"
    )
    .unwrap();
    out
}

fn gzip_bytes(raw: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(raw).unwrap();
    enc.finish().unwrap()
}

struct MockArxivServer {
    base_url: String,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockArxivServer {
    fn start(tex_gz: Vec<u8>, pdf_bytes: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let hits_bg = Arc::clone(&hits);
        let stop_bg = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !stop_bg.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hits_bg.fetch_add(1, Ordering::AcqRel);
                        let _ = stream.set_nonblocking(false);
                        let mut buf = [0u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let first = req.lines().next().unwrap_or("");
                        let (status, ct, body) = if first.contains("/e-print/2501.10001") {
                            ("200 OK", "application/x-eprint", tex_gz.clone())
                        } else if first.contains("/e-print/2501.10002") {
                            ("404 Not Found", "text/plain", b"not found".to_vec())
                        } else if first.contains("/pdf/2501.10002.pdf") {
                            ("200 OK", "application/pdf", pdf_bytes.clone())
                        } else {
                            ("404 Not Found", "text/plain", b"missing".to_vec())
                        };
                        let hdr = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{}", addr.port()),
            hits,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for MockArxivServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn idea_owned_paper_is_ingested_before_reading() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&vault_dir).unwrap();

    let owned_path = vault_dir.join("owned_majorana.tex");
    let tex = "\\section{Main Result}\nThe Lanczos coefficients satisfy b_n = alpha n in the volume-law phase.\n";
    std::fs::write(&owned_path, tex).unwrap();

    let key = PaperKey::parse("doi:10.5555/owned-majorana-2025").unwrap();
    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_source(&crate::library::SourceRecord {
        id: "src-vault".into(),
        kind: crate::library::SourceKind::Folder,
        root: vault_dir.to_string_lossy().to_string(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds { doi: Some("10.5555/owned-majorana-2025".into()), ..Default::default() },
        title: "Owned Paywalled Majorana Paper".into(),
        authors: vec!["Ada Example".into()],
        year: Some(2025),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, ..Default::default() },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_user_library_fulltext(&FulltextRecord {
        key: key.clone(),
        version: None,
        raw_blob: None,
        format: "tex".into(),
        bytes: tex.len() as u64,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 1,
        file_path: Some(owned_path.to_string_lossy().to_string()),
        file_size: Some(tex.len() as u64),
        file_mtime_ms: Some(1),
        file_sha256: Some(sha256_hex(tex.as_bytes())),
    })
    .unwrap();

    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-owned-c9";
    ledger
        .append_new_session(
            sid,
            &[("idea.started", 1, &json!({ "preset": "standard", "budget": { "max_papers": 6 } }))],
        )
        .unwrap();

    let env = Arc::new(ResearchEnv::new(&data_dir));
    let mut reg = Registry::new();
    register_explore_v1_tools(&mut reg, Arc::clone(&env), None);
    let router = Router::new(reg, research_policy(), ledger, sid, Arc::new(ApproveAll));

    let out = router
        .try_call(
            "host:idea:s-owned-c9",
            IDEA_FETCH_PAPERS,
            &json!({ "keys": [key.as_str()] }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else { panic!("expected Ok, got {out:?}") };
    assert_eq!(val["fetched"], 1);
    let paper = &val["papers"][0];
    assert_eq!(paper["status"], "ready");
    assert_eq!(paper["origin"], ORIGIN_USER_LIBRARY);
    assert_eq!(paper["text_basis"], "tex");
    let source_id = paper["source_id"].as_str().unwrap().to_string();
    let reading_path = paper["reading_path"].as_str().unwrap().to_string();

    // Rule C9: remove the external user_library file completely; quote check MUST still succeed
    // against the session's own pinned copy!
    std::fs::remove_file(&owned_path).unwrap();
    assert!(!owned_path.exists());

    let ledger_reopen = Ledger::open(&data_dir).unwrap();
    let events = ledger_reopen.read_session(sid).unwrap();
    let verified = verify_quote(
        &env.sandbox(),
        sid,
        &events,
        &source_id,
        &reading_path,
        2,
        2,
        "The Lanczos coefficients satisfy b_n = alpha n in the volume-law phase.",
    )
    .expect("Rule C9: quote check must use session-ingested copy after source file deletion");
    assert_eq!(verified.quote_ref.start_line, 2);
}

#[test]
fn idea_fetch_papers_resolves_eprint_then_pdf_caches_paper_text_and_enforces_budget() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let tex_src = b"\\section{Intro}\nKrylov complexity grows linearly with time in this synthetic TeX fixture.\n";
    let tex_gz = gzip_bytes(tex_src);
    let pdf_bytes = build_minimal_pdf(&[
        "Page one synthetic fixture discussing Lanczos coefficients on a lattice.",
        "Page two synthetic fixture concluding saturation in the area-law phase.",
    ]);
    let mock = MockArxivServer::start(tex_gz, pdf_bytes.clone());

    let mut env = ResearchEnv::new(&data_dir);
    env.network_endpoints = LiteratureEndpoints {
        arxiv_eprint_base: mock.base_url.clone(),
        arxiv_api_base: mock.base_url.clone(),
        crossref_api_base: mock.base_url.clone(),
        ..LiteratureEndpoints::default()
    };
    let env = Arc::new(env);
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));

    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-fetch-order";
    ledger
        .append_new_session(
            sid,
            &[("idea.started", 1, &json!({ "preset": "standard", "budget": { "max_papers": 2 } }))],
        )
        .unwrap();

    let mut reg = Registry::new();
    reg.register(
        idea_fetch_papers_def(),
        idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(Arc::clone(&gate))),
    );
    let router = Router::new(reg, research_policy(), ledger, sid, Arc::new(ApproveAll));

    // Fetch 3 keys with budget max_papers = 2: first -> arxiv_eprint, second -> arxiv_pdf, third -> stopped_at_cap.
    let out = router
        .try_call(
            "host:idea:s-fetch-order",
            IDEA_FETCH_PAPERS,
            &json!({ "keys": ["arxiv:2501.10001", "arxiv:2501.10002", "arxiv:2501.10003"] }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else { panic!("expected Ok, got {out:?}") };
    assert_eq!(val["fetched"], 2);
    assert_eq!(val["stopped_at_cap"], true);
    assert_eq!(val["papers"][0]["origin"], "arxiv_eprint");
    assert_eq!(val["papers"][0]["text_basis"], "tex");
    assert_eq!(val["papers"][1]["origin"], "arxiv_pdf");
    assert_eq!(val["papers"][1]["text_basis"], "pdf_text");
    assert_eq!(val["papers"][1]["pdf_text_cached"], false);
    assert_eq!(val["papers"][2]["status"], "stopped_at_cap");

    // Verify F10 paper_text cache was populated for arxiv:2501.10002 and reused in a new session
    // with ZERO additional network requests!
    let hits_before = mock.hits.load(Ordering::Acquire);
    let sid2 = "s-fetch-cached";
    let ledger2 = Ledger::open(&data_dir).unwrap();
    ledger2
        .append_new_session(
            sid2,
            &[("idea.started", 1, &json!({ "preset": "standard", "budget": { "max_papers": 6 } }))],
        )
        .unwrap();
    let mut reg2 = Registry::new();
    reg2.register(
        idea_fetch_papers_def(),
        idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(gate)),
    );
    let router2 = Router::new(reg2, research_policy(), ledger2, sid2, Arc::new(ApproveAll));
    let out2 = router2
        .try_call(
            "host:idea:s-fetch-cached",
            IDEA_FETCH_PAPERS,
            &json!({ "keys": ["arxiv:2501.10002"] }),
        )
        .unwrap();
    let CallOutcome::Ok(val2) = out2 else { panic!("expected Ok, got {out2:?}") };
    assert_eq!(val2["papers"][0]["origin"], "library");
    assert_eq!(val2["papers"][0]["pdf_text_cached"], true);
    assert_eq!(mock.hits.load(Ordering::Acquire), hits_before, "cached paper must make 0 network hits");
}

#[test]
fn idea_fetch_papers_quick_preset_refuses_all_fetches_at_cap() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-quick-zero";
    ledger
        .append_new_session(
            sid,
            &[("idea.started", 1, &json!({ "preset": "quick", "budget": { "max_papers": 0 } }))],
        )
        .unwrap();
    let env = Arc::new(ResearchEnv::new(&data_dir));
    let mut reg = Registry::new();
    register_explore_v1_tools(&mut reg, env, None);
    let router = Router::new(reg, research_policy(), ledger, sid, Arc::new(ApproveAll));

    let out = router
        .try_call(
            "host:idea:s-quick-zero",
            IDEA_FETCH_PAPERS,
            &json!({ "keys": ["arxiv:2501.10001"] }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else { panic!("expected Ok, got {out:?}") };
    assert_eq!(val["fetched"], 0);
    assert_eq!(val["stopped_at_cap"], true);
    assert_eq!(val["papers"][0]["status"], "stopped_at_cap");
}

#[test]
fn reconstruct_pdf_document_pages_preserves_warnings_and_pads_missing_chunks() {
    use crate::library::PageBasis as LibPageBasis;
    let doc = reconstruct_pdf_document_pages(
        "pdf-extract-layout/0.12.1+tesseract/5.3",
        "Line one\n\x0c\nLine three".to_string(),
        &[LibPageBasis::TextLayer, LibPageBasis::Ocr, LibPageBasis::None],
    );
    assert_eq!(doc.page_count, 3);
    assert_eq!(doc.page_map.pages.len(), 3);
    assert_eq!(doc.page_map.page_of(1), Some(1));
    assert_eq!(doc.page_map.page_of(3), Some(2));
    assert!(doc.warning.as_deref().unwrap_or("").contains("OCR needed"));

    let empty = reconstruct_pdf_document_pages(
        "pdf-extract-layout/0.12.1",
        String::new(),
        &[LibPageBasis::None, LibPageBasis::None],
    );
    assert_eq!(empty.page_count, 2);
    assert_eq!(empty.page_map.pages.len(), 2);
    assert_eq!(empty.status, crate::pdf::PdfExtractStatus::EmptyTextLayer);
}

#[test]
fn user_library_lookup_enforces_f13a_containment_and_skips_notes_and_missing() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault_dir = tmp.path().join("vault");
    let outside_dir = tmp.path().join("outside");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&vault_dir).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();

    let outside_file = outside_dir.join("secret.tex");
    std::fs::write(&outside_file, "Outside root text").unwrap();
    let note_file = vault_dir.join("note.md");
    std::fs::write(&note_file, "# Private note").unwrap();

    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_source(&crate::library::SourceRecord {
        id: "src-1".into(),
        kind: crate::library::SourceKind::Folder,
        root: vault_dir.to_string_lossy().to_string(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();

    let key = PaperKey::parse("doi:10.5555/f13a-guard").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds::default(),
        title: "Guarded Paper".into(),
        authors: vec![],
        year: Some(2025),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, ..Default::default() },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    // Register a note file and an out-of-root fulltext row; neither may be read.
    lib.upsert_source_item(&crate::library::SourceItemRecord {
        source_id: "src-1".into(),
        external_id: "item-1".into(),
        key: Some(key.clone()),
        citekey: None,
        ids: LibraryIds::default(),
        version: None,
        title: "Guarded Paper".into(),
        authors: vec![],
        year: Some(2025),
        item_kind: "article".into(),
        collections: vec![],
        tags: vec![],
        files: vec![crate::library::SourceItemFile {
            path: note_file.to_string_lossy().to_string(),
            role: crate::paper_sources::FILE_ROLE_NOTE.into(),
            size: 14,
            mtime_ms: 1,
            sha256: None,
        }],
        state: crate::library::SourceItemState::Present,
        updated_ms: 1,
    })
    .unwrap();
    lib.upsert_user_library_fulltext(&FulltextRecord {
        key: key.clone(),
        version: None,
        raw_blob: None,
        format: "tex".into(),
        bytes: 17,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 1,
        file_path: Some(outside_file.to_string_lossy().to_string()),
        file_size: Some(17),
        file_mtime_ms: Some(1),
        file_sha256: None,
    })
    .unwrap();

    let found = super::cache::lookup_user_library_file(&data_dir, &key, None).unwrap();
    assert!(found.is_none(), "must reject out-of-root path and FILE_ROLE_NOTE");
}
