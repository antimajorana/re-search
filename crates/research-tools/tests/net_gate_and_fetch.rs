//! Loopback mock integration tests for package F3 (`net_rate`, `paper_fetch`,
//! `fetched_sources`, and the minimal `fulltext` cache).
//!
//! All fixture paper titles, authors, abstracts, TeX bodies, and PDFs are synthetic and
//! written specifically for these tests. Gate tests use `FakeRateClock` and never sleep
//! in real time.

use flate2::write::GzEncoder;
use flate2::Compression;
use ledger::Ledger;
use research_tools::arxiv::run_arxiv_search_with_gate;
use research_tools::literature::run_literature_search_with_gate;
use research_tools::net_rate::{FakeRateClock, NetRateGate, RateBucket, Service};
use research_tools::paper_fetch::{
    check_fetch_budget, fetch_with_gate, fetched_sources, parse_paper_input, FetchJob,
    FetchOutcome, FetchStage, FetchWant, SourceOrigin, STATUS_NEEDS_FILE,
};
use research_tools::paper_key::{PaperIds, PaperKey, PaperRef};
use research_tools::pdf::MAX_PDF_BYTES;
use research_tools::{ArxivSearchRequest, LiteratureEndpoints, LiteratureRequest, ResearchEnv};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tempfile::TempDir;

#[derive(Clone)]
struct MockResp {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl MockResp {
    fn ok(content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self { status: 200, content_type, body: body.into() }
    }
    fn not_found() -> Self {
        Self { status: 404, content_type: "text/plain", body: b"not found".to_vec() }
    }
}

struct LoopbackMock {
    base_url: String,
    seen_paths: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LoopbackMock {
    fn start<F>(handler: F) -> Self
    where
        F: Fn(&str) -> MockResp + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{addr}");
        let seen_paths = Arc::new(Mutex::new(Vec::new()));
        let seen_cl = seen_paths.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_cl = stop.clone();
        let handler = Arc::new(handler);

        let handle = thread::spawn(move || {
            while !stop_cl.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut buf = [0u8; 8192];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let path = req
                            .lines()
                            .next()
                            .unwrap_or("")
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("/")
                            .to_string();
                        seen_cl.lock().unwrap().push(path.clone());
                        let resp = handler(&path);
                        let head = format!(
                            "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            resp.status,
                            resp.content_type,
                            resp.body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&resp.body);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { base_url, seen_paths, stop, handle: Some(handle) }
    }

    fn paths(&self) -> Vec<String> {
        self.seen_paths.lock().unwrap().clone()
    }
}

impl Drop for LoopbackMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(self.base_url.trim_start_matches("http://"));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn build_targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tb = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(data.len() as u64);
        hdr.set_mode(0o644);
        hdr.set_cksum();
        tb.append_data(&mut hdr, name, *data).unwrap();
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&tb.into_inner().unwrap()).unwrap();
    enc.finish().unwrap()
}

fn build_symlink_targz() -> Vec<u8> {
    let mut tb = tar::Builder::new(Vec::new());
    let mut hdr = tar::Header::new_gnu();
    hdr.set_entry_type(tar::EntryType::Symlink);
    hdr.set_size(0);
    hdr.set_mode(0o777);
    hdr.set_cksum();
    tb.append_link(&mut hdr, "escape.tex", "/etc/passwd").unwrap();
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&tb.into_inner().unwrap()).unwrap();
    enc.finish().unwrap()
}

fn build_minimal_pdf(text: &str) -> Vec<u8> {
    let stream_body = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objs: Vec<(usize, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [5 0 R] /Count 1 >>".to_vec()),
        (3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
        (4, format!("<< /Length {} >>\nstream\n{stream_body}\nendstream", stream_body.len()).into_bytes()),
        (5, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".to_vec()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objs.len() + 1];
    for (id, body) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objs.len() + 1).unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF", objs.len() + 1).unwrap();
    out
}

fn sample_atom(id: &str, title: &str, doi: Option<&str>) -> Vec<u8> {
    let doi_tag = doi.map(|d| format!("<arxiv:doi>{d}</arxiv:doi>")).unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>https://arxiv.org/abs/{id}</id>
    <published>2021-03-01T00:00:00Z</published>
    <updated>2021-06-15T00:00:00Z</updated>
    <title>{title}</title>
    <summary>Synthetic test abstract for fixture verification.</summary>
    <author><name>A. Example</name></author>
    {doi_tag}
    <arxiv:primary_category term="math-ph"/>
    <link rel="license" href="https://creativecommons.org/licenses/by/4.0/"/>
  </entry>
</feed>"#
    )
    .into_bytes()
}

#[test]
fn gate_spacing_with_fake_clock_across_loopback_services() {
    let atom = sample_atom("2103.01234v2", "Synthetic Lattice Bound", None);
    let srv = LoopbackMock::start(move |path| {
        if path.starts_with("/api/query") {
            return MockResp::ok("application/atom+xml", atom.clone());
        }
        if path.starts_with("/api/literature") {
            return MockResp::ok("application/json", br#"{"hits":{"total":0,"hits":[]}}"#.to_vec());
        }
        if path.starts_with("/works") {
            return MockResp::ok("application/json", br#"{"status":"ok","message":{"items":[]}}"#.to_vec());
        }
        if path.starts_with("/graph/v1/") {
            return MockResp::ok("application/json", br#"{"total":0,"data":[]}"#.to_vec());
        }
        MockResp::not_found()
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(clock.clone());

    // Two arXiv searches: 2nd sleeps exactly 3 s.
    let req = ArxivSearchRequest {
        query: "synthetic".into(),
        field: None,
        max_results: Some(1),
        include_source: Some(false),
        section_filter: None,
    };
    run_arxiv_search_with_gate(&env, &ledger, "s-gate", &req, &gate).unwrap();
    run_arxiv_search_with_gate(&env, &ledger, "s-gate", &req, &gate).unwrap();
    assert_eq!(clock.recorded_sleeps(), vec![Duration::from_secs(3)]);

    // 16 INSPIRE searches: first 15 immediate, 16th waits 5 s.
    let ins_req = LiteratureRequest {
        source: Some("inspire".into()),
        query: "synthetic".into(),
        max_results: Some(1),
    };
    for _ in 0..15 {
        run_literature_search_with_gate(&env, &ledger, "s-gate", &ins_req, &gate).unwrap();
    }
    assert_eq!(clock.recorded_sleeps().len(), 1);
    run_literature_search_with_gate(&env, &ledger, "s-gate", &ins_req, &gate).unwrap();
    assert_eq!(clock.recorded_sleeps(), vec![Duration::from_secs(3), Duration::from_secs(5)]);

    // Crossref (1/s) and Semantic Scholar (1/s) are independent buckets.
    let cr_req = LiteratureRequest { source: Some("crossref".into()), query: "x".into(), max_results: Some(1) };
    let s2_req = LiteratureRequest { source: Some("semantic_scholar".into()), query: "x".into(), max_results: Some(1) };
    run_literature_search_with_gate(&env, &ledger, "s-gate", &cr_req, &gate).unwrap();
    run_literature_search_with_gate(&env, &ledger, "s-gate", &s2_req, &gate).unwrap();
    assert_eq!(clock.recorded_sleeps().len(), 2);
    run_literature_search_with_gate(&env, &ledger, "s-gate", &cr_req, &gate).unwrap();
    assert_eq!(clock.recorded_sleeps().last(), Some(&Duration::from_secs(1)));
}

#[test]
fn two_callers_sharing_one_arxiv_gate_never_exceed_three_second_spacing() {
    let atom = sample_atom("2103.02000v2", "Shared Gate Paper", None);
    let tgz = build_targz(&[("main.tex", br"\documentclass{article}\begin{document}\section{S}Hi.\end{document}")]);
    let srv = LoopbackMock::start(move |path| {
        if path.starts_with("/api/query") {
            return MockResp::ok("application/atom+xml", atom.clone());
        }
        if path.starts_with("/e-print/") {
            return MockResp::ok("application/x-gzip", tgz.clone());
        }
        MockResp::not_found()
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let clock = Arc::new(FakeRateClock::new());
    let gate = Arc::new(NetRateGate::new(clock.clone()));

    // Caller 1: arxiv_search; Caller 2: paper_fetch (metadata, then source).
    let req = ArxivSearchRequest {
        query: "2103.02000".into(),
        field: Some("id".into()),
        max_results: Some(1),
        include_source: Some(false),
        section_filter: None,
    };
    let paper = parse_paper_input("2103.02000v2").unwrap();

    run_arxiv_search_with_gate(&env, &ledger, "s-shared", &req, &gate).unwrap();
    let _ = fetch_with_gate(&env, &paper, FetchWant::Metadata, &gate).unwrap();
    let _ = fetch_with_gate(&env, &paper, FetchWant::Source, &gate).unwrap();
    run_arxiv_search_with_gate(&env, &ledger, "s-shared", &req, &gate).unwrap();

    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Arxiv)), 4);
    assert_eq!(
        clock.recorded_sleeps(),
        vec![Duration::from_secs(3), Duration::from_secs(3), Duration::from_secs(3)]
    );

    // Concurrent threads: one calls paper_fetch, the other calls arxiv_search on the same gate.
    let barrier = Arc::new(Barrier::new(2));
    let e1 = env.clone();
    let g1 = gate.clone();
    let b1 = barrier.clone();
    let p_other = parse_paper_input("2103.02001v1").unwrap();
    let t1 = thread::spawn(move || {
        b1.wait();
        fetch_with_gate(&e1, &p_other, FetchWant::Source, &g1).unwrap()
    });
    let e2 = env.clone();
    let g2 = gate.clone();
    let b2 = barrier.clone();
    let dir2 = tmp.path().to_path_buf();
    let t2 = thread::spawn(move || {
        let l2 = Ledger::open(&dir2).unwrap();
        b2.wait();
        run_arxiv_search_with_gate(&e2, &l2, "s-shared-2", &req, &g2).unwrap()
    });
    t1.join().unwrap();
    t2.join().unwrap();
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Arxiv)), 6);
}

#[test]
fn fetch_order_cache_hit_zero_requests_and_session_ingest_counter() {
    let tgz = build_targz(&[
        ("main.tex", br"\documentclass{article}\begin{document}\section{Intro}Synthetic body.\end{document}"),
    ]);
    let pdf = build_minimal_pdf("Synthetic PDF paper text.");
    let pdf_eprint = build_minimal_pdf("Eprint returned raw PDF directly.");
    let tgz_cl = tgz.clone();
    let pdf_cl = pdf.clone();
    let pdf_ep_cl = pdf_eprint.clone();

    let srv = LoopbackMock::start(move |path| match path {
        "/e-print/2103.01234v2" => MockResp::ok("application/x-gzip", tgz_cl.clone()),
        "/e-print/2103.05555v1" => MockResp::not_found(),
        "/pdf/2103.05555v1.pdf" => MockResp::ok("application/pdf", pdf_cl.clone()),
        "/e-print/2103.06666v1" => MockResp::ok("application/pdf", pdf_ep_cl.clone()),
        _ => MockResp::not_found(),
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap());
    let gate = NetRateGate::new(Arc::new(FakeRateClock::new()));
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());

    // 1. TeX e-print fetched first; PDF endpoint is never touched; zero session events written.
    let p_tex = parse_paper_input("2103.01234v2").unwrap();
    let fetched_tex = match fetch_with_gate(&env, &p_tex, FetchWant::Source, &gate).unwrap() {
        FetchOutcome::Source(s) => s,
        other => panic!("expected Source, got {other:?}"),
    };
    assert_eq!(fetched_tex.origin, SourceOrigin::ArxivEprint);
    assert_eq!(srv.paths(), vec!["/e-print/2103.01234v2"]);
    assert!(ledger.lock().unwrap().read_session("s-one").unwrap().is_empty());

    // Enters session s-one only through ingest_bytes.
    let ing1 = fetched_tex
        .ingest_into_session(&env.sandbox(), &ledger, "s-one", None)
        .unwrap();
    assert_eq!(ing1.status, "ok");
    let ev_one = ledger.lock().unwrap().read_session("s-one").unwrap();
    assert_eq!(fetched_sources(&ev_one), 1);

    // 2. Second fetch hits the library cache with ZERO network requests.
    let hits_before = srv.paths().len();
    let cached = match fetch_with_gate(&env, &p_tex, FetchWant::Source, &gate).unwrap() {
        FetchOutcome::Source(s) => s,
        other => panic!("expected cached Source, got {other:?}"),
    };
    assert_eq!(cached.origin, SourceOrigin::Library);
    assert_eq!(cached.raw_blob, fetched_tex.raw_blob);
    assert_eq!(srv.paths().len(), hits_before, "cache hit must make 0 HTTP requests");

    // Cached bytes enter session s-two cleanly via ingest_bytes.
    cached
        .ingest_into_session(&env.sandbox(), &ledger, "s-two", None)
        .unwrap();
    let ev_two = ledger.lock().unwrap().read_session("s-two").unwrap();
    assert_eq!(fetched_sources(&ev_two), 1);
    assert!(check_fetch_budget(&ev_two, 1).is_err());
    assert!(check_fetch_budget(&ev_two, 2).is_ok());

    // 3. Fallback order: /e-print/ 404 -> /pdf/ 200 -> cached on repeat.
    let p_pdf = parse_paper_input("2103.05555v1").unwrap();
    let fetched_pdf = match fetch_with_gate(&env, &p_pdf, FetchWant::Source, &gate).unwrap() {
        FetchOutcome::Source(s) => s,
        other => panic!("expected PDF fallback Source, got {other:?}"),
    };
    assert_eq!(fetched_pdf.origin, SourceOrigin::ArxivPdf);
    assert_eq!(
        srv.paths(),
        vec![
            "/e-print/2103.01234v2",
            "/e-print/2103.05555v1",
            "/pdf/2103.05555v1.pdf",
        ]
    );
    fetched_pdf
        .ingest_into_session(&env.sandbox(), &ledger, "s-one", None)
        .unwrap();
    assert_eq!(fetched_sources(&ledger.lock().unwrap().read_session("s-one").unwrap()), 2);

    // 4. Raw PDF served directly on /e-print/ avoids a second /pdf/ call.
    let p_ep_pdf = parse_paper_input("2103.06666v1").unwrap();
    let direct_pdf = match fetch_with_gate(&env, &p_ep_pdf, FetchWant::Source, &gate).unwrap() {
        FetchOutcome::Source(s) => s,
        other => panic!("expected direct PDF Source, got {other:?}"),
    };
    assert_eq!(direct_pdf.origin, SourceOrigin::ArxivPdf);
    assert_eq!(srv.paths().last().map(String::as_str), Some("/e-print/2103.06666v1"));

    // Ledger integrity: zero unreferenced blobs in <data_dir>/blobs/.
    assert_eq!(ledger.lock().unwrap().verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn doi_needs_file_doi_resolution_metadata_and_versions() {
    let tgz = build_targz(&[("paper.tex", br"\documentclass{article}\begin{document}Resolved.\end{document}")]);
    let atom_v3 = sample_atom("2103.07777v3", "Resolved Paper", Some("10.1000/has-arxiv.2"));
    let tgz_cl = tgz.clone();
    let atom_cl = atom_v3.clone();

    let srv = LoopbackMock::start(move |path| {
        if path.starts_with("/api/literature") && path.contains("has%2Darxiv") {
            return MockResp::ok(
                "application/json",
                serde_json::to_vec(&json!({
                    "hits": {"total": 1, "hits": [{"id": "9001", "metadata": {
                        "titles": [{"title": "Resolved Paper"}],
                        "authors": [{"full_name": "Example, A."}],
                        "dois": [{"value": "10.1000/has-arxiv.2"}],
                        "arxiv_eprints": [{"value": "2103.07777"}]
                    }}]}
                }))
                .unwrap(),
            );
        }
        if path.starts_with("/api/literature") {
            return MockResp::ok("application/json", br#"{"hits":{"total":0,"hits":[]}}"#.to_vec());
        }
        if path.starts_with("/works/") && path.contains("no%2Darxiv") {
            return MockResp::ok(
                "application/json",
                serde_json::to_vec(&json!({
                    "status": "ok",
                    "message": {
                        "DOI": "10.1000/no-arxiv.1",
                        "title": ["Publisher Only Article"],
                        "author": [{"given": "B.", "family": "Author"}],
                        "issued": {"date-parts": [[2022, 4]]},
                        "relation": {"is-retracted-by": [{"id": "10.1000/retraction"}]}
                    }
                }))
                .unwrap(),
            );
        }
        if path.starts_with("/api/query") {
            return MockResp::ok("application/atom+xml", atom_cl.clone());
        }
        if path == "/e-print/2103.07777" {
            return MockResp::ok("application/x-gzip", tgz_cl.clone());
        }
        MockResp::not_found()
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap());
    let gate = NetRateGate::new(Arc::new(FakeRateClock::new()));

    // DOI with no arXiv version returns needs_file for Source, but returns Crossref metadata.
    let no_ax = parse_paper_input("10.1000/no-arxiv.1").unwrap();
    let out = fetch_with_gate(&env, &no_ax, FetchWant::Source, &gate).unwrap();
    assert_eq!(out.status_str(), STATUS_NEEDS_FILE);
    assert!(matches!(out, FetchOutcome::NeedsFile { .. }));

    let meta_out = fetch_with_gate(&env, &no_ax, FetchWant::Metadata, &gate).unwrap();
    match meta_out {
        FetchOutcome::Metadata(m) => {
            assert_eq!(m.title, "Publisher Only Article");
            assert!(m.retracted);
        }
        other => panic!("expected Metadata, got {other:?}"),
    }

    // DOI that resolves through INSPIRE to arXiv fetches e-print and caches under both keys.
    let has_ax = parse_paper_input("10.1000/has-arxiv.2").unwrap();
    let src = match fetch_with_gate(&env, &has_ax, FetchWant::Source, &gate).unwrap() {
        FetchOutcome::Source(s) => s,
        other => panic!("expected Source, got {other:?}"),
    };
    assert_eq!(src.paper.key, PaperKey::parse("arxiv:2103.07777").unwrap());
    assert_eq!(src.origin, SourceOrigin::ArxivEprint);

    let reqs_after_first = srv.paths().len();
    let cached_by_doi = fetch_with_gate(&env, &has_ax, FetchWant::Source, &gate).unwrap();
    assert!(matches!(cached_by_doi, FetchOutcome::Source(ref s) if s.origin == SourceOrigin::Library));
    assert_eq!(srv.paths().len(), reqs_after_first);

    // Versions query returns v1..=v3 in one Atom request.
    let ax_ref = PaperRef::from_ids(
        PaperIds { arxiv: Some("2103.07777".into()), ..PaperIds::default() },
        None,
    )
    .unwrap();
    match fetch_with_gate(&env, &ax_ref, FetchWant::Versions, &gate).unwrap() {
        FetchOutcome::Versions { versions, .. } => {
            assert_eq!(versions.len(), 3);
            assert_eq!(versions[2].version, "v3");
            assert!(versions[2].latest);
        }
        other => panic!("expected Versions, got {other:?}"),
    }
}

#[test]
fn oversized_and_malformed_downloads_refused_without_panic() {
    let symlink_tgz = build_symlink_targz();
    let oversized_pdf = vec![b'X'; MAX_PDF_BYTES + 128];

    let srv = LoopbackMock::start(move |path| match path {
        "/e-print/2103.09001" => MockResp::ok("text/html", b"<!DOCTYPE html><html>Captcha</html>".to_vec()),
        "/e-print/2103.09002" => MockResp::ok("application/x-gzip", b"\x1f\x8b\x08\x00broken".to_vec()),
        "/e-print/2103.09003" => MockResp::ok("application/x-gzip", symlink_tgz.clone()),
        "/e-print/2103.09004" => MockResp::not_found(),
        "/pdf/2103.09004.pdf" => MockResp::ok("application/pdf", oversized_pdf.clone()),
        "/e-print/2103.09005" => MockResp::not_found(),
        "/pdf/2103.09005.pdf" => MockResp::ok("application/pdf", b"%PDF-1.4\ncorrupt-body".to_vec()),
        "/e-print/2103.09006" => MockResp::ok("application/octet-stream", Vec::new()),
        _ => MockResp::not_found(),
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap());
    let gate = NetRateGate::new(Arc::new(FakeRateClock::new()));

    for id in [
        "2103.09001",
        "2103.09002",
        "2103.09003",
        "2103.09004",
        "2103.09005",
        "2103.09006",
    ] {
        let p = parse_paper_input(id).unwrap();
        let err = fetch_with_gate(&env, &p, FetchWant::Source, &gate).unwrap_err();
        assert!(!err.is_empty(), "expected non-empty error for {id}");
        // Refused download must never poison the cache.
        let cache = ledger::FulltextCache::open(tmp.path()).unwrap();
        assert!(cache.get_entry(p.key.as_str(), None).unwrap().is_none());
    }
}

#[test]
fn fetch_job_runs_on_thread_and_records_progress_stages() {
    let tgz = build_targz(&[("doc.tex", br"\documentclass{article}\begin{document}Job ok.\end{document}")]);
    let srv = LoopbackMock::start(move |path| {
        if path == "/e-print/2103.08001v1" {
            return MockResp::ok("application/x-gzip", tgz.clone());
        }
        if path.starts_with("/api/literature") {
            return MockResp::ok("application/json", br#"{"hits":{"total":0,"hits":[]}}"#.to_vec());
        }
        MockResp::not_found()
    });

    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path())
        .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap());
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));

    let p = parse_paper_input("2103.08001v1").unwrap();
    let job = FetchJob::start_with_gate("job-1", env.clone(), p, FetchWant::Source, Some(gate.clone()));
    let outcome = job.join().unwrap();
    assert!(matches!(outcome, FetchOutcome::Source(s) if s.origin == SourceOrigin::ArxivEprint));

    // Also test synchronous job run for deterministic stage history verification.
    let p_doi = parse_paper_input("10.1000/job-none.9").unwrap();
    let sync_job = FetchJob::run_sync("job-2", &env, &p_doi, FetchWant::Source, &gate);
    assert!(sync_job.is_finished());
    let snap = sync_job.snapshot();
    assert_eq!(snap.job_id, "job-2");
    assert!(snap.history.contains(&FetchStage::Queued));
    assert!(snap.history.contains(&FetchStage::CheckingCache));
    assert!(snap.history.contains(&FetchStage::ResolvingId));
    assert!(matches!(snap.stage, FetchStage::NeedsFile { .. }));
    assert!(matches!(sync_job.join().unwrap(), FetchOutcome::NeedsFile { .. }));
}
