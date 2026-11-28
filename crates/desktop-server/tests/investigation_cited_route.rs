//! End-to-end Eval Set P5 & "Follow the argument" integration tests for Investigate V11
//! (`POST /api/sessions/{id}/investigation/cited`).
//!
//! Loopback mock HTTP server + fake rate-limit clock only: zero real network calls, zero sleeps,
//! zero model calls, strictly self-authored TeX and PDF 1.4 fixtures.

use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::investigation_cited::{handle_with_gate, EVENT_INVESTIGATION_CITED_LOOKUP};
use desktop_server::investigation_open::effective_fetch_limit;
use desktop_server::investigation_skeleton::{
    pin_source_and_build_skeleton, EVENT_INVESTIGATION_STARTED,
};
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use desktop_server::{start_desktop_server, DesktopConfig};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{Library, SeenRole};
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::skeleton::SkNodeKind;
use research_tools::{LiteratureEndpoints, ResearchEnv};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

const PAPER_A_TEX: &[u8] = concat!(
    "\\documentclass{article}\n",
    "\\newtheorem{theorem}{Theorem}[section]\n",
    "\\begin{document}\n",
    "\\section{Main Argument}\n",
    "\\begin{theorem}[Target Claim]\\label{thm:target}\n",
    "Uses \\cite[Theorem 2.1]{B26}, \\cite[Lemma 9]{BDrift}, \\cite[Theorem 2.1]{CPdf},\n",
    "\\cite[Lemma 3.1]{CPdf}, \\cite[Theorem 1]{DScan}, \\cite[Theorem 1]{Paywall},\n",
    "\\cite[Prop. 1]{BareRef}, and \\cite[Theorem 2.1]{Extra9}.\n",
    "\\end{theorem}\n",
    "\\begin{thebibliography}{9}\n",
    "\\bibitem{B26} B. Author, \\emph{Shared Counter Paper}, arXiv:2601.00055v2.\n",
    "\\bibitem{BDrift} D. Author, \\emph{Unshared Counter Drift}, arXiv:2601.00066v1.\n",
    "\\bibitem{CPdf} P. Author, \\emph{PDF Only Paper}, arXiv:2601.00077v1.\n",
    "\\bibitem{DScan} S. Author, \\emph{Image Scan Only}, arXiv:2601.00088v1.\n",
    "\\bibitem{Paywall} W. Author, \\emph{Closed Journal Note}, doi:10.1000/paywalled.99.\n",
    "\\bibitem{BareRef} U. Author, Unpublished lecture notes, 1982.\n",
    "\\bibitem{Extra9} E. Author, \\emph{Ninth External Paper}, arXiv:2601.00099v1.\n",
    "\\end{thebibliography}\n",
    "\\end{document}\n",
)
.as_bytes();

const PAPER_B_TEX: &[u8] = concat!(
    "\\documentclass{article}\n",
    "\\newtheorem{theorem}{Theorem}[section]\n",
    "\\newtheorem{lemma}[theorem]{Lemma}\n",
    "\\begin{document}\n",
    "\\section{Setup}\n",
    "\\begin{lemma}[Base Estimate]\\label{lem:bbase}\n",
    "PAPER_B_PREREQUISITE_LEMMA_1_1.\n",
    "\\end{lemma}\n",
    "\\section{Main}\n",
    "\\begin{theorem}[Spectral Gap Bound]\\label{thm:b21}\n",
    "By \\ref{lem:bbase}, PAPER_B_THEOREM_2_1_VERBATIM.\n",
    "\\end{theorem}\n",
    "\\end{document}\n",
)
.as_bytes();

const PAPER_B_DRIFT_TEX: &[u8] = concat!(
    "\\documentclass{article}\n",
    "\\newtheorem{theorem}{Theorem}\n",
    "\\newtheorem{lemma}{Lemma}\n",
    "\\begin{document}\n",
    "\\section{Drift}\n",
    "\\begin{theorem}Separate theorem counter one.\\end{theorem}\n",
    "\\begin{lemma}L1\\end{lemma}\\begin{lemma}L2\\end{lemma}\\begin{lemma}L3\\end{lemma}\n",
    "\\begin{lemma}L4\\end{lemma}\\begin{lemma}L5\\end{lemma}\\begin{lemma}L6\\end{lemma}\n",
    "\\begin{lemma}L7\\end{lemma}\\begin{lemma}L8\\end{lemma}\n",
    "\\begin{lemma}\\label{lem:drift9}OFF_BY_ONE_CANDIDATE_LEMMA_9.\\end{lemma}\n",
    "\\end{document}\n",
)
.as_bytes();

const PAPER_EXTRA9_TEX: &[u8] = concat!(
    "\\documentclass{article}\n",
    "\\newtheorem{theorem}{Theorem}[section]\n",
    "\\begin{document}\n",
    "\\section{Intro}\n\\section{Result}\n",
    "\\begin{theorem}\\label{thm:e9}NINTH_EXTERNAL_THEOREM_2_1.\\end{theorem}\n",
    "\\end{document}\n",
)
.as_bytes();

/// Minimal self-authored PDF 1.4 generator supporting digital text, scanned-with-OCR, and image-only pages.
fn build_minimal_pdf(pages: &[(Vec<&str>, bool, bool)]) -> Vec<u8> {
    // Tuple: (lines, has_fullpage_image, invisible_text)
    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();
    let kids: String = (0..pages.len())
        .map(|i| format!("{} 0 R", 5 + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objs.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    objs.push((
        2,
        format!("<< /Type /Pages /Count {} /Kids [ {kids} ] >>", pages.len()).into_bytes(),
    ));
    objs.push((
        3,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ));
    let mut img_obj = b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n".to_vec();
    img_obj.extend_from_slice(b"\xC0\xC0\xC0\nendstream");
    objs.push((4, img_obj));

    for (idx, (lines, full_img, invisible)) in pages.iter().enumerate() {
        let page_id = 5 + idx * 2;
        let stream_id = page_id + 1;
        let mut s = String::new();
        if *full_img {
            s.push_str("q 612 0 0 792 0 0 cm /Im1 Do Q\n");
        }
        if !lines.is_empty() {
            s.push_str("BT /F1 12 Tf ");
            if *invisible {
                s.push_str("3 Tr ");
            }
            let mut y = 720;
            for line in lines {
                let esc = line.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
                s.push_str(&format!("1 0 0 1 72 {y} Tm ({esc}) Tj "));
                y -= 16;
            }
            s.push_str("ET\n");
        }
        objs.push((
            page_id,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> /XObject << /Im1 4 0 R >> >> /Contents {stream_id} 0 R >>"
            )
            .into_bytes(),
        ));
        objs.push((
            stream_id,
            format!("<< /Length {} >>\nstream\n{s}endstream", s.len()).into_bytes(),
        ));
    }

    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objs.len() + 1];
    for (id, body) in &objs {
        offsets[*id] = pdf.len();
        pdf.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in &offsets[1..] {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

struct MockLitServer {
    port: u16,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockLitServer {
    fn start(routes: HashMap<String, (&'static str, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback mock");
        listener.set_nonblocking(true).expect("set nonblocking");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (h_hits, h_stop) = (Arc::clone(&hits), Arc::clone(&stop));

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
                            .unwrap_or("/");
                        h_hits.fetch_add(1, Ordering::SeqCst);

                        let (status, ct, body) = if let Some((ct, bytes)) = routes.get(path) {
                            ("200 OK", *ct, bytes.clone())
                        } else if path.starts_with("/api/literature") {
                            ("200 OK", "application/json", br#"{"hits":{"hits":[]}}"#.to_vec())
                        } else if path.starts_with("/works/") {
                            (
                                "200 OK",
                                "application/json",
                                br#"{"status":"ok","message":{"DOI":"10.1000/paywalled.99","title":["Closed Journal Note"]}}"#.to_vec(),
                            )
                        } else {
                            ("404 Not Found", "text/plain", b"not found".to_vec())
                        };
                        let resp = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.write_all(&body);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { port, hits, stop, handle: Some(handle) }
    }

    fn endpoints(&self) -> LiteratureEndpoints {
        LiteratureEndpoints::loopback_mock(&format!("http://127.0.0.1:{}", self.port))
            .expect("loopback mock endpoints")
    }
}

impl Drop for MockLitServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn mk_state(dir: &std::path::Path, endpoints: LiteratureEndpoints) -> DesktopState {
    let cfg = DesktopConfig {
        port: 0,
        data_dir: dir.to_path_buf(),
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: endpoints,
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(Default::default()),
        chatgpt_test: None,
    };
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    DesktopState::new(cfg, AuthGuard::new(0).unwrap(), Arc::new(SseHub::new()), chatgpt)
}

fn assert_no_verified_word(val: &Value, ctx: &str) {
    let serialized = serde_json::to_string(val).unwrap().to_ascii_lowercase();
    assert!(
        !serialized.contains("verified"),
        "forbidden word `verified` appeared in {ctx}: {serialized}"
    );
}

fn seed_paper_a_session(state: &DesktopState, dir: &std::path::Path, sid: &str) -> String {
    let env = ResearchEnv::new(dir.to_path_buf());
    let ledger = Mutex::new(state.open_ledger().unwrap());
    ledger.lock().unwrap().append(sid, EVENT_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    let src_a = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "paper_a.tex",
            origin: "upload",
            raw_bytes: PAPER_A_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Paper A"),
        },
    )
    .unwrap();
    drop(ledger);
    let rec_a = pin_source_and_build_skeleton(state, sid, &src_a.source_id, None, None, None).unwrap();
    rec_a
        .skeleton
        .nodes
        .iter()
        .find(|n| n.kind == SkNodeKind::Theorem)
        .unwrap()
        .tag
        .clone()
        .unwrap()
}

fn call_cited(state: &DesktopState, sid: &str, gate: &NetRateGate, payload: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&payload).unwrap();
    let (st, val) = handle_with_gate(state, sid, &body, gate).unwrap();
    assert_no_verified_word(&val, &format!("cited response for {payload}"));
    (st, val)
}

#[test]
fn eval_set_p5_tex_pdf_drift_caching_overlay_and_budget() {
    let pdf_c = build_minimal_pdf(&[
        (vec!["1 Introduction", "Overview of PDF-only estimates."], false, false),
        (vec!["Theorem 2.1. Every finite operator is bounded on page two.", "Proof. Immediate."], false, false),
        (vec!["Lemma 3.1. A scanned OCR estimate on page three."], true, true),
    ]);
    let pdf_empty_scan = build_minimal_pdf(&[(vec![], true, false)]);

    let mut routes = HashMap::new();
    routes.insert("/e-print/2601.00055v2".into(), ("application/x-tex", PAPER_B_TEX.to_vec()));
    routes.insert("/e-print/2601.00066v1".into(), ("application/x-tex", PAPER_B_DRIFT_TEX.to_vec()));
    routes.insert("/pdf/2601.00077v1.pdf".into(), ("application/pdf", pdf_c));
    routes.insert("/pdf/2601.00088v1.pdf".into(), ("application/pdf", pdf_empty_scan));
    routes.insert("/e-print/2601.00099v1".into(), ("application/x-tex", PAPER_EXTRA9_TEX.to_vec()));

    let mock = MockLitServer::start(routes);
    let tmp = TempDir::new().unwrap();
    let state = mk_state(tmp.path(), mock.endpoints());
    let gate = NetRateGate::new(Arc::new(FakeRateClock::new()));

    let sid = "inv-p5-a";
    let tag_a = seed_paper_a_session(&state, tmp.path(), sid);

    // 1. Confirmed TeX match (Paper B, single shared counter) -> Found, exact verbatim statement, F10 cached.
    let (st, res_b) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "B26", "locator": "Theorem 2.1"
    }));
    assert_eq!(st, 200);
    assert_eq!(res_b["status"], "found");
    assert_eq!(res_b["provenance"], "outside_paper");
    assert!(res_b["statement"]["text"].as_str().unwrap().contains("PAPER_B_THEOREM_2_1_VERBATIM"));
    assert_eq!(res_b["return_ref"]["id"], json!(tag_a));
    assert_eq!(res_b["return_ref"]["session"], json!(sid));
    let lib = Library::open(tmp.path()).unwrap();
    let key_b = PaperKey::from_arxiv("2601.00055").unwrap();
    assert!(lib.get_skeleton(&key_b, Some("v2"), "tex-1").unwrap().is_some());
    assert!(!lib.recent_by_role(SeenRole::CitedLookup, 10).unwrap().is_empty());

    // 2. Second lookup in a DIFFERENT session hits F10 fulltext + skeleton cache with ZERO loopback HTTP hits.
    let sid2 = "inv-p5-a2";
    let tag_a2 = seed_paper_a_session(&state, tmp.path(), sid2);
    let hits_before = mock.hits.load(Ordering::SeqCst);
    let (st2, res_b2) = call_cited(&state, sid2, &gate, json!({
        "tag": tag_a2, "bib_key": "B26", "locator": "Theorem 2.1"
    }));
    assert_eq!(st2, 200);
    assert_eq!(res_b2["status"], "found");
    assert_eq!(res_b2["from_cache"], json!(true));
    assert_eq!(res_b2["skeleton_cache_hit"], json!(true));
    assert_eq!(mock.hits.load(Ordering::SeqCst), hits_before, "F10 cache hit must make 0 HTTP calls");

    // 3. Cross-paper overlay & return: inspect Paper B's prerequisite Lemma 1.1 with zero HTTP/model calls.
    let ext_sid_b = res_b["external_source_id"].as_str().unwrap().to_string();
    let lem_b_tag = res_b["cited_view"]["mapview"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "lemma")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let hits_before_inspect = mock.hits.load(Ordering::SeqCst);
    let (st_ins, res_ins) = call_cited(&state, sid, &gate, json!({
        "action": "inspect_node", "external_source_id": ext_sid_b, "external_tag": lem_b_tag
    }));
    assert_eq!(st_ins, 200);
    assert!(res_ins["statement"]["text"].as_str().unwrap().contains("PAPER_B_PREREQUISITE_LEMMA_1_1"));
    assert_eq!(mock.hits.load(Ordering::SeqCst), hits_before_inspect);

    // 4. Off-by-one / unshared counter fixture (Rule 6.9) -> MUST yield `candidates`, NEVER `found`.
    let (st_dr, res_dr) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "BDrift", "locator": "Lemma 9"
    }));
    assert_eq!(st_dr, 200);
    assert_eq!(res_dr["status"], "candidates", "unconfirmed counter MUST never yield found");
    assert!(res_dr["statement"].is_null());
    assert!(!res_dr["candidates"].as_array().unwrap().is_empty());

    // 5. PDF-only cited Paper C (V17 pdf-1): MUST yield `candidates`, NEVER `found`, with exact pages & OCR flag.
    let (st_pdf1, res_pdf1) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "CPdf", "locator": "Theorem 2.1"
    }));
    assert_eq!(st_pdf1, 200);
    assert_eq!(res_pdf1["status"], "candidates", "PDF lookup MUST never yield found");
    assert_eq!(res_pdf1["builder"], "pdf-1");
    let c0 = &res_pdf1["candidates"][0];
    assert_eq!(c0["start_page"], json!(2));
    assert_eq!(c0["ocr"], json!(false));
    assert_eq!(c0["render_math"], json!(true));

    let (st_pdf2, res_pdf2) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "CPdf", "locator": "Lemma 3.1"
    }));
    assert_eq!(st_pdf2, 200);
    assert_eq!(res_pdf2["status"], "candidates");
    let c_scan = &res_pdf2["candidates"][0];
    assert_eq!(c_scan["start_page"], json!(3));
    assert_eq!(c_scan["ocr"], json!(true));
    assert_eq!(c_scan["render_math"], json!(false));

    // 6. Image-only PDF (`empty_text_layer`), paywalled DOI (`needs_file`), missing locator, unresolved.
    let (_, res_scan) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "DScan", "locator": "Theorem 1"
    }));
    assert_eq!(res_scan["status"], "no_tex");
    assert_eq!(res_scan["reason"], "empty_text_layer");

    let (_, res_pw) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "Paywall", "locator": "Theorem 1"
    }));
    assert_eq!(res_pw["status"], "no_tex");
    assert_eq!(res_pw["reason"], "needs_file");

    let (_, res_miss) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "B26", "locator": "Theorem 99"
    }));
    assert_eq!(res_miss["status"], "paper_found_locator_not_found");

    let (_, res_unres) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "BareRef", "locator": "Prop. 1"
    }));
    assert_eq!(res_unres["status"], "unresolved");

    // 7. Fetch budget (rule 6.16): top up to 8 fetched sources, verify 9th is refused until raise_budget.
    {
        let ledger = state.open_ledger().unwrap();
        let cur = research_tools::paper_fetch::fetched_sources(&ledger.read_session(sid).unwrap());
        for i in cur..8 {
            ledger
                .append(
                    sid,
                    "source.ingested",
                    1,
                    &json!({ "source_id": format!("dummy_fetched_{i}"), "origin": format!("https://arxiv.org/e-print/2601.9000{i}") }),
                )
                .unwrap();
        }
    }
    let hits_before_cap = mock.hits.load(Ordering::SeqCst);
    let (st_cap, res_cap) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "Extra9", "locator": "Theorem 2.1"
    }));
    assert_eq!(st_cap, 429);
    assert_eq!(res_cap["code"], "fetch_budget_exceeded");
    assert_eq!(res_cap["fetch_budget"]["used"], json!(8));
    assert_eq!(res_cap["fetch_budget"]["limit"], json!(8));
    assert_eq!(mock.hits.load(Ordering::SeqCst), hits_before_cap, "budget refusal must make 0 HTTP calls");

    let (st_raise, res_raise) = call_cited(&state, sid, &gate, json!({
        "action": "raise_budget", "new_limit": 16, "tag": tag_a
    }));
    assert_eq!(st_raise, 200);
    assert_eq!(res_raise["fetch_budget"]["limit"], json!(16));

    let (st_9th, res_9th) = call_cited(&state, sid, &gate, json!({
        "tag": tag_a, "bib_key": "Extra9", "locator": "Theorem 2.1"
    }));
    assert_eq!(st_9th, 200);
    assert_eq!(res_9th["status"], "found");
    assert!(res_9th["statement"]["text"].as_str().unwrap().contains("NINTH_EXTERNAL_THEOREM_2_1"));

    // Zero model.request events across the entire run, and cited_lookup events recorded.
    let all_events = state.open_ledger().unwrap().read_session(sid).unwrap();
    assert!(!all_events.iter().any(|e| e.kind == "model.request"));
    assert_eq!(effective_fetch_limit(&all_events), 16);
    assert!(all_events.iter().any(|e| e.kind == EVENT_INVESTIGATION_CITED_LOOKUP));
}

fn raw_http_post(port: u16, path: &str, token: Option<&str>, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect server");
    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(tok) = token {
        req.push_str(&format!("X-ReSearch-Token: {tok}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    let code = resp
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body_part = resp.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_string();
    (code, body_part)
}

#[test]
fn cited_http_route_requires_auth_and_dispatches_via_api_dispatch() {
    let tmp = TempDir::new().unwrap();
    let cfg = DesktopConfig {
        port: 0,
        data_dir: tmp.path().to_path_buf(),
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(Default::default()),
        chatgpt_test: None,
    };
    let srv = start_desktop_server(cfg).expect("start desktop server");
    let sid = "inv-http-auth";
    let state_tmp = mk_state(tmp.path(), LiteratureEndpoints::default());
    let tag_a = seed_paper_a_session(&state_tmp, tmp.path(), sid);

    let payload = json!({ "tag": tag_a, "bib_key": "BareRef", "locator": "Prop. 1" }).to_string();
    let path = format!("/api/sessions/{sid}/investigation/cited");

    let (unauth_status, _) = raw_http_post(srv.info.port, &path, None, &payload);
    assert!(
        matches!(unauth_status, 401 | 403),
        "unauthenticated POST .../investigation/cited must return 401/403, got {unauth_status}"
    );

    let (auth_status, auth_body) =
        raw_http_post(srv.info.port, &path, Some(&srv.info.token), &payload);
    assert_eq!(auth_status, 200, "unexpected body: {auth_body}");
    let parsed: Value = serde_json::from_str(&auth_body).unwrap();
    assert_eq!(parsed["status"], "unresolved");
    assert_no_verified_word(&parsed, "http auth response");
    srv.shutdown();
}
