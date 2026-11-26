//! Loopback-mock integration tests for Investigate V18 (`investigation_open_html`):
//! arXiv `/html/<id>` LaTeXML fallback between TeX e-print and PDF fallback, `Service::Arxiv`
//! rate gating (`FakeRateClock`), clean rejection of 404 / non-LaTeXML HTML to PDF,
//! `html-1` skeleton + mapview + verified node text, and zero model/live-network calls.

use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::investigation_open::{
    get_open_job, start_open_job_with_gate, wait_for_open_job, OpenStep,
};
use desktop_server::investigation_skeleton;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use desktop_server::DesktopConfig;
use ledger::{Event, FulltextCache};
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::LiteratureEndpoints;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

type RouteResponder = Box<dyn Fn(&str) -> (u16, &'static str, Vec<u8>) + Send + Sync>;

struct RoutedMock {
    port: u16,
    paths: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RoutedMock {
    fn start(responder: RouteResponder) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).expect("set nonblocking");
        let port = listener.local_addr().unwrap().port();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let h_paths = Arc::clone(&paths);
        let h_stop = Arc::clone(&stop);
        let responder = Arc::new(responder);

        let handle = thread::spawn(move || {
            while !h_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut buf = [0u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let path = req
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("/")
                            .to_string();
                        h_paths.lock().unwrap().push(path.clone());

                        let (code, ct, body) = responder(&path);
                        let status = match code {
                            200 => "200 OK",
                            404 => "404 Not Found",
                            _ => "500 Internal Server Error",
                        };
                        let hdr = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&body);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { port, paths, stop, handle: Some(handle) }
    }

    fn requested_paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

impl Drop for RoutedMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn atom_xml(id_ver: &str, title: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/{id_ver}</id>
    <published>2026-02-01T00:00:00Z</published>
    <updated>2026-02-02T00:00:00Z</updated>
    <title>{title}</title>
    <summary>Synthetic discrete harmonic estimates on finite graphs.</summary>
    <author><name>A. Synthetic</name></author>
  </entry>
</feed>"#
    )
    .into_bytes()
}

fn synthetic_latexml_html() -> Vec<u8> {
    br#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="generator" content="LaTeXML">
  <title>Synthetic Discrete Harmonic Estimates</title>
</head>
<body>
<article class="ltx_document">
  <h1 class="ltx_title ltx_title_document">Synthetic Discrete Harmonic Estimates</h1>
  <section class="ltx_section" id="S1">
    <h2 class="ltx_title ltx_title_section"><span class="ltx_tag ltx_tag_section">1 </span>Preliminaries</h2>
    <div class="ltx_theorem ltx_theorem_theorem" id="S1.Thm1">
      <h6 class="ltx_title ltx_title_theorem"><span class="ltx_tag ltx_tag_theorem">Theorem 1.1</span> (Contraction).</h6>
      <div class="ltx_para"><p class="ltx_p">Every balanced graph operator satisfies strict spectral contraction.</p></div>
    </div>
    <div class="ltx_proof" id="S1.P1">
      <h6 class="ltx_title ltx_title_proof"><span class="ltx_tag ltx_tag_proof">Proof.</span></h6>
      <div class="ltx_para"><p class="ltx_p">Follows by summing local edge quadratic forms directly.</p></div>
    </div>
  </section>
</article>
</body>
</html>"#
        .to_vec()
}

fn build_minimal_pdf(lines: &[&str]) -> Vec<u8> {
    let mut stream = String::from("BT\n/F1 12 Tf\n72 720 Td\n");
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            stream.push_str("0 -16 Td\n");
        }
        let esc = line.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
        stream.push_str(&format!("({esc}) Tj\n"));
    }
    stream.push_str("ET\n");

    let objs = [
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>\nendobj\n".to_string(),
        format!("4 0 obj\n<< /Length {} >>\nstream\n{stream}endstream\nendobj\n", stream.len()),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
    ];

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objs.len());
    for obj in &objs {
        offsets.push(out.len());
        out.extend_from_slice(obj.as_bytes());
    }
    let xref_pos = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n", objs.len() + 1)
            .as_bytes(),
    );
    out
}

fn make_state(data_dir: &std::path::Path, mock_port: u16) -> DesktopState {
    let cfg = DesktopConfig {
        port: 0,
        data_dir: data_dir.to_path_buf(),
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::loopback_mock(&format!(
            "http://127.0.0.1:{mock_port}"
        ))
        .expect("loopback mock endpoints"),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: None,
        chatgpt_test: None,
    };
    let auth = AuthGuard::new(0).expect("auth");
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).expect("chatgpt hub");
    DesktopState::new(cfg, auth, sse, chatgpt)
}

fn init_investigation_session(state: &DesktopState, session_id: &str) {
    let ledger = state.open_ledger().expect("open ledger");
    ledger
        .append(session_id, "investigation.started", 1, &json!({ "mode": "investigate" }))
        .expect("append started");
}

#[test]
fn missing_tex_falls_back_to_html_via_arxiv_gate_and_builds_html1_skeleton() {
    let tmp = TempDir::new().unwrap();
    let html_bytes = synthetic_latexml_html();
    let mock = RoutedMock::start(Box::new(move |path| {
        if path.starts_with("/api/query") {
            (200, "application/atom+xml", atom_xml("2601.00055v2", "Synthetic Discrete Harmonic Estimates"))
        } else if path.starts_with("/e-print/2601.00055") {
            (404, "text/plain", b"not found".to_vec())
        } else if path.starts_with("/html/2601.00055") {
            (200, "text/html; charset=utf-8", html_bytes.clone())
        } else if path.starts_with("/pdf/") {
            (200, "application/pdf", build_minimal_pdf(&["Should not fetch PDF"]))
        } else {
            (404, "text/plain", b"not found".to_vec())
        }
    }));

    let state = make_state(tmp.path(), mock.port);
    init_investigation_session(&state, "sess-html-fb");

    let clock = Arc::new(FakeRateClock::new());
    let gate = Arc::new(NetRateGate::new(Arc::clone(&clock) as _));
    let initial = start_open_job_with_gate(
        &state,
        "sess-html-fb",
        &json!({ "arxiv_id": "2601.00055v2" }),
        Some(gate),
    )
    .expect("start open job");

    let done = wait_for_open_job("sess-html-fb", &initial.job_id, Duration::from_secs(10)).unwrap();
    assert_eq!(done.step, OpenStep::Done, "failed: {:?}", done.error);
    assert_eq!(done.status, "done");
    assert_eq!(done.source_origin.as_deref(), Some("arxiv_html"));
    assert!(done.history.contains(&OpenStep::EprintTex));
    assert!(done.history.contains(&OpenStep::HtmlFallback));
    assert!(!done.history.contains(&OpenStep::PdfFallback));

    let requested = mock.requested_paths();
    assert!(requested.iter().any(|p| p.starts_with("/e-print/2601.00055v2")));
    assert!(requested.iter().any(|p| p.starts_with("/html/2601.00055v2")));
    assert!(!requested.iter().any(|p| p.starts_with("/pdf/")));
    assert!(
        clock.recorded_sleeps().len() >= 2,
        "expected >= 2 arXiv rate-gate sleeps (/api/query -> /e-print/ -> /html/), got {:?}",
        clock.recorded_sleeps()
    );

    let (st, sk_json) = investigation_skeleton::handle(
        &state,
        "GET",
        "sess-html-fb",
        "skeleton",
        &HashMap::new(),
        &[],
    )
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(sk_json["skeleton"]["builder"], "html-1");

    let (_, mv_json) = investigation_skeleton::handle(
        &state,
        "GET",
        "sess-html-fb",
        "mapview",
        &HashMap::new(),
        &[],
    )
    .unwrap();
    assert_eq!(mv_json["builder"], "html-1");
    assert!(
        mv_json["quality_line"].as_str().unwrap_or("").starts_with("Structure read from arXiv HTML"),
        "unexpected quality_line: {}",
        mv_json["quality_line"]
    );
    let nodes = mv_json["nodes"].as_array().unwrap();
    let thm = nodes.iter().find(|n| n["kind"] == "theorem").unwrap();
    let thm_tag = thm["id"].as_str().unwrap();

    let (nt_st, nt_json) = investigation_skeleton::handle(
        &state,
        "GET",
        "sess-html-fb",
        "node-text",
        &HashMap::from([("tag".into(), thm_tag.to_string())]),
        &[],
    )
    .unwrap();
    assert_eq!(nt_st, 200);
    assert!(nt_json["text"].as_str().unwrap().contains("strict spectral contraction"));
    assert!(nt_json["proof_text"].as_str().unwrap().contains("quadratic forms"));

    let events: Vec<Event> = state.open_ledger().unwrap().read_session("sess-html-fb").unwrap();
    assert!(!events.iter().any(|e| e.kind == "model.request"), "zero model calls expected");
    assert!(get_open_job("sess-html-fb", &initial.job_id).is_some());
}

#[test]
fn html_404_and_non_latexml_html_fall_through_to_pdf_fallback() {
    for (case_name, arxiv_id, html_status, html_body) in [
        ("404", "2601.00066", 404u16, b"not found".to_vec()),
        (
            "non_latexml",
            "2601.00077",
            200u16,
            b"<!DOCTYPE html><html><body><h1>HTML unavailable</h1><p>Use PDF.</p></body></html>".to_vec(),
        ),
    ] {
        let tmp = TempDir::new().unwrap();
        let pdf_bytes = build_minimal_pdf(&[
            "Synthetic Fallback Note",
            "1. Introduction",
            "Theorem 1.1. PDF fallback remains functional.",
        ]);
        let id_owned = arxiv_id.to_string();
        let mock = RoutedMock::start(Box::new(move |path| {
            if path.starts_with("/api/query") {
                (200, "application/atom+xml", atom_xml(&id_owned, "Synthetic Fallback Note"))
            } else if path.starts_with("/e-print/") {
                (404, "text/plain", b"no tex".to_vec())
            } else if path.starts_with("/html/") {
                (html_status, "text/html", html_body.clone())
            } else if path.starts_with("/pdf/") {
                (200, "application/pdf", pdf_bytes.clone())
            } else {
                (404, "text/plain", b"not found".to_vec())
            }
        }));

        let sid = format!("sess-pdf-{case_name}");
        let state = make_state(tmp.path(), mock.port);
        init_investigation_session(&state, &sid);
        let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));

        let init = start_open_job_with_gate(&state, &sid, &json!({ "arxiv_id": arxiv_id }), Some(gate))
            .unwrap();
        let done = wait_for_open_job(&sid, &init.job_id, Duration::from_secs(10)).unwrap();
        assert_eq!(done.step, OpenStep::Done, "{case_name} failed: {:?}", done.error);
        assert_eq!(done.source_origin.as_deref(), Some("arxiv_pdf"));
        assert!(done.history.contains(&OpenStep::HtmlFallback));
        assert!(done.history.contains(&OpenStep::PdfFallback));

        let (_, sk) = investigation_skeleton::handle(&state, "GET", &sid, "skeleton", &HashMap::new(), &[])
            .unwrap();
        assert_eq!(sk["skeleton"]["builder"], "pdf-1");

        let events = state.open_ledger().unwrap().read_session(&sid).unwrap();
        assert!(
            !events.iter().any(|e| e.kind == "source.ingested" && e.payload["format"] == "html"),
            "{case_name} must not ingest rejected HTML"
        );
    }
}

#[test]
fn non_arxiv_doi_and_fulltext_cache_preserve_existing_paths() {
    let tmp = TempDir::new().unwrap();
    let mock = RoutedMock::start(Box::new(|path| {
        if path.starts_with("/works/") {
            (200, "application/json", br#"{"status":"ok","message":{"DOI":"10.1000/closed.1","title":["Paywalled Paper"]}}"#.to_vec())
        } else {
            (404, "text/plain", b"not found".to_vec())
        }
    }));
    let state = make_state(tmp.path(), mock.port);
    init_investigation_session(&state, "sess-doi");
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));

    let doi_job = start_open_job_with_gate(
        &state,
        "sess-doi",
        &json!({ "doi": "10.1000/closed.1" }),
        Some(Arc::clone(&gate)),
    )
    .unwrap();
    let done_doi = wait_for_open_job("sess-doi", &doi_job.job_id, Duration::from_secs(10)).unwrap();
    assert_eq!(done_doi.step, OpenStep::NeedsFile);
    assert!(!mock.requested_paths().iter().any(|p| p.starts_with("/html/")));

    // Seed FulltextCache with TeX and confirm cache hit makes zero new network requests.
    let before_hits = mock.requested_paths().len();
    let cache = FulltextCache::open(tmp.path()).unwrap();
    cache
        .put_bytes(
            "arxiv:2601.00044",
            None,
            "tex",
            "library",
            b"\\documentclass{article}\n\\begin{document}\n\\section{Cache}\\label{s:c}\n\\end{document}\n",
        )
        .unwrap();
    init_investigation_session(&state, "sess-cache");
    let cached_job = start_open_job_with_gate(
        &state,
        "sess-cache",
        &json!({ "arxiv_id": "2601.00044" }),
        Some(gate),
    )
    .unwrap();
    let done_cached: Value = serde_json::to_value(
        wait_for_open_job("sess-cache", &cached_job.job_id, Duration::from_secs(10)).unwrap(),
    )
    .unwrap();
    assert_eq!(done_cached["step"], "done");
    assert_eq!(done_cached["source_origin"], "library");
    assert!(!mock.requested_paths()[before_hits..].iter().any(|p| p.starts_with("/html/") || p.starts_with("/e-print/")));
}
