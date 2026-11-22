//! Integration tests for F7 (`arxiv_search`), F8 (`literature_search`),
//! F9 (`ledger_append`, `ledger_supersede`, `ledger_query`), and F10 (`check_expression_sanity`).

use flate2::write::GzEncoder;
use flate2::Compression;
use ledger::Ledger;
use research_tools::http_client::{fetch_bytes_allowlisted, MAX_EPRINT_BYTES, MAX_METADATA_BYTES};
use research_tools::{
    normalize_arxiv_id, read_proof_dag, register_research_tools, research_policy,
    run_arxiv_search, run_literature_search, ArxivSearchRequest, LiteratureEndpoints,
    LiteratureRequest, ProofStatus, ResearchEnv, ARXIV_SEARCH, CHECK_EXPRESSION_SANITY, FS_READ,
    LEDGER_APPEND, LEDGER_QUERY, LEDGER_SUPERSEDE, LITERATURE_SEARCH,
};
use serde_json::json;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tempfile::TempDir;
use tools::approval::{ApprovalReply, ApprovalRequest, Approver};
use tools::router::{CallOutcome, Router};
use tools::Registry;

/// `(session, tool_id, canonical_args)` of every approval the router asked for.
static ASKED: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());

/// Approves only `network-read` calls (which the policy always clamps to ask) and records them;
/// any other approval request means an allow-class tool prompted, which is a bug.
struct NetworkReadOnlyApprover;
impl Approver for NetworkReadOnlyApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        assert_eq!(
            req.effect,
            policy::EffectClass::NetworkRead,
            "only network-read tools may trigger an approval prompt here: {}",
            req.tool_id
        );
        ASKED
            .lock()
            .unwrap()
            .push((req.session.clone(), req.tool_id.clone(), req.canonical_args.clone()));
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "test".into() }
    }
}

fn asked_for(session: &str) -> Vec<(String, String)> {
    ASKED
        .lock()
        .unwrap()
        .iter()
        .filter(|(s, _, _)| s == session)
        .map(|(_, t, a)| (t.clone(), a.clone()))
        .collect()
}

#[derive(Clone)]
struct MockResp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl MockResp {
    fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self { status: 200, headers: vec![("Content-Type".into(), content_type.into())], body: body.into() }
    }
    fn redirect(location: &str) -> Self {
        Self { status: 302, headers: vec![("Location".into(), location.into())], body: Vec::new() }
    }
    fn not_found() -> Self {
        Self { status: 404, headers: vec![], body: Vec::new() }
    }
}

struct MockHttpServer {
    base_url: String,
    seen_paths: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockHttpServer {
    fn start<F>(handler: F) -> Self
    where
        F: Fn(&str, &str) -> MockResp + Send + Sync + 'static,
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
                        let req_text = String::from_utf8_lossy(&buf[..n]);
                        let path = req_text.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("/").to_string();
                        seen_cl.lock().unwrap().push(path.clone());
                        let resp = handler(&path, &req_text);
                        let mut head = format!("HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n", resp.status, resp.body.len());
                        for (k, v) in &resp.headers {
                            head.push_str(&format!("{k}: {v}\r\n"));
                        }
                        head.push_str("\r\n");
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&resp.body);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(std::time::Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        });
        Self { base_url, seen_paths, stop, handle: Some(handle) }
    }
}

impl Drop for MockHttpServer {
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
    for (p, data) in files {
        let mut hdr = tar::Header::new_gnu();
        hdr.set_size(data.len() as u64);
        hdr.set_mode(0o644);
        hdr.set_cksum();
        tb.append_data(&mut hdr, p, *data).unwrap();
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&tb.into_inner().unwrap()).unwrap();
    enc.finish().unwrap()
}

fn build_router(env: Arc<ResearchEnv>, session: &str) -> Router {
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    Router::new(reg, research_policy(), Ledger::open(&env.data_dir).unwrap(), session, Arc::new(NetworkReadOnlyApprover))
}

#[test]
fn f7_http_allowlist_redirects_and_arxiv_id_normalization() {
    assert_eq!(normalize_arxiv_id("arXiv:2401.12345v2").as_deref(), Some("2401.12345v2"));
    assert_eq!(normalize_arxiv_id("https://arxiv.org/abs/2401.12345").as_deref(), Some("2401.12345"));
    assert_eq!(normalize_arxiv_id("https://arxiv.org/pdf/hep-th/9905111v1.pdf").as_deref(), Some("hep-th/9905111v1"));
    assert!(normalize_arxiv_id("not-an-arxiv-id").is_none());

    let ep = LiteratureEndpoints::default();
    assert!(fetch_bytes_allowlisted("http://export.arxiv.org/api/query", &ep, MAX_METADATA_BYTES).is_err());
    assert!(fetch_bytes_allowlisted("https://evil.example/api", &ep, MAX_METADATA_BYTES).is_err());

    let srv = MockHttpServer::start(|path, req| {
        if !req.to_ascii_lowercase().contains("user-agent: re-search/0.0.1") {
            return MockResp { status: 400, headers: vec![], body: b"missing UA".to_vec() };
        }
        match path {
            "/hop0" => MockResp::redirect("/hop1"),
            "/hop1" => MockResp::redirect("/hop2"),
            "/hop2" => MockResp::redirect("/hop3"),
            "/hop3" => MockResp::redirect("/hop4"),
            "/hop4" => MockResp::ok("text/plain", b"too-far".to_vec()),
            "/ok0" => MockResp::redirect("/ok1"),
            "/ok1" => MockResp::ok("text/plain", b"final-ok".to_vec()),
            "/offhost" => MockResp::redirect("https://evil.example/steal"),
            "/big" => MockResp::ok("text/plain", vec![b'x'; MAX_METADATA_BYTES + 10]),
            _ => MockResp::not_found(),
        }
    });
    let mock_ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let ok = fetch_bytes_allowlisted(&format!("{}/ok0", srv.base_url), &mock_ep, MAX_METADATA_BYTES).unwrap();
    assert_eq!(ok, b"final-ok");
    assert!(fetch_bytes_allowlisted(&format!("{}/hop0", srv.base_url), &mock_ep, MAX_METADATA_BYTES).is_err());
    assert!(fetch_bytes_allowlisted(&format!("{}/offhost", srv.base_url), &mock_ep, MAX_METADATA_BYTES).is_err());
    assert!(fetch_bytes_allowlisted(&format!("{}/big", srv.base_url), &mock_ep, MAX_METADATA_BYTES).is_err());
    assert_eq!(MAX_EPRINT_BYTES, 32 * 1024 * 1024);
}

#[test]
fn f7_arxiv_atom_xxe_withdrawn_and_eprint_ingestion() {
    let tgz = build_targz(&[
        ("main.tex", br"\documentclass{article}\begin{document}\section{Intro}Base\input{sec}\end{document}"),
        ("sec.tex", br"\section{Derivation}Chern-Simons shift $k \to k+N$."),
    ]);
    let atom_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>http://arxiv.org/abs/2403.09876v3</id><updated>2024-05-10T12:00:00Z</updated><published>2024-03-14T08:30:00Z</published>
    <title>Chern-Simons Level Shifts</title><summary>We derive the exact level shift.</summary>
    <author><name>A. Researcher</name></author><author><name>B. Collaborator</name></author>
    <arxiv:doi>10.1103/PhysRevB.109.125101</arxiv:doi><arxiv:primary_category term="cond-mat.str-el"/>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2311.00001v2</id><updated>2024-01-02T00:00:00Z</updated><published>2023-11-01T00:00:00Z</published>
    <title>Flawed Conjecture</title><summary>This paper has been withdrawn by the authors.</summary>
    <author><name>C. Author</name></author><arxiv:comment>Withdrawn due to sign error</arxiv:comment><arxiv:primary_category term="hep-th"/>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2401.11111v1</id><updated>2024-02-01T00:00:00Z</updated><published>2024-01-20T00:00:00Z</published>
    <title>PDF Only Paper</title><summary>Pdf only summary.</summary><author><name>D. Writer</name></author><arxiv:primary_category term="hep-th"/>
  </entry>
</feed>"#.to_string();
    let xxe_xml = r#"<?xml version="1.0"?><!DOCTYPE feed [ <!ENTITY xxe SYSTEM "file:///etc/passwd"> ]><feed xmlns="http://www.w3.org/2005/Atom"><title>&xxe;</title></feed>"#;
    let queue = Arc::new(Mutex::new(VecDeque::from([
        MockResp::ok("application/atom+xml", atom_xml.into_bytes()),
        MockResp::ok("application/atom+xml", xxe_xml.as_bytes().to_vec()),
    ])));
    let q_cl = queue.clone();
    let tgz_cl = tgz.clone();
    let srv = MockHttpServer::start(move |path, _| {
        if path.starts_with("/api/query") {
            return q_cl.lock().unwrap().pop_front().unwrap_or_else(MockResp::not_found);
        }
        if path.starts_with("/e-print/2403.09876v3") {
            return MockResp::ok("application/x-eprint-tar", tgz_cl.clone());
        }
        if path.starts_with("/e-print/2401.11111v1") {
            return MockResp::ok("application/pdf", b"%PDF-1.4\n...".to_vec());
        }
        MockResp::not_found()
    });

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data_dir).with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap()));
    let session = "s-arxiv01";
    let router = build_router(env.clone(), session);

    let out = router
        .try_call("agent:test", ARXIV_SEARCH, &json!({"query": "Chern-Simons", "include_source": true, "section_filter": "Derivation"}))
        .unwrap();
    let papers = match out {
        CallOutcome::Ok(v) => v["papers"].as_array().unwrap().clone(),
        other => panic!("unexpected outcome: {other:?}"),
    };
    assert_eq!(papers.len(), 3);
    // network-read is clamped to ask: the call prompted once, and the prompt carried the query.
    let asks = asked_for(session);
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!(asks[0].0, ARXIV_SEARCH);
    assert!(asks[0].1.contains("Chern-Simons"), "{asks:?}");
    assert_eq!(papers[0]["id"], "2403.09876v3");
    assert_eq!(papers[0]["base_id"], "2403.09876");
    assert_eq!(papers[0]["withdrawn"], false);
    assert!(papers[0]["bibtex"].as_str().unwrap().starts_with("@article{"));
    assert_eq!(papers[0]["source"]["source_available"], true);
    let assembled = papers[0]["source"]["assembled_path"].as_str().unwrap().to_string();

    assert_eq!(papers[1]["withdrawn"], true);
    assert!(papers[1]["source"].is_null());
    assert_eq!(papers[2]["source"]["source_available"], false);
    assert_eq!(papers[2]["source"]["reason"], "pdf_only");
    assert!(!srv.seen_paths.lock().unwrap().iter().any(|p| p.contains("2311.00001")));

    let read_res = router.try_call("agent:test", FS_READ, &json!({"root": "sources", "path": assembled})).unwrap();
    assert!(matches!(read_res, CallOutcome::Ok(v) if v["content"].as_str().unwrap().contains("Chern-Simons shift")));

    let ldg = Ledger::open(&data_dir).unwrap();
    assert!(run_arxiv_search(
        &env,
        &ldg,
        session,
        &ArxivSearchRequest { query: "2403.09876v3".into(), field: Some("id".into()), max_results: Some(5), include_source: Some(false), section_filter: None },
    )
    .is_err());
    assert_eq!(ldg.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn f8_literature_search_inspire_crossref_and_s2() {
    let srv = MockHttpServer::start(|path, _| {
        if path.starts_with("/api/literature") {
            return MockResp::ok("application/json", serde_json::to_vec(&json!({
                "hits": {"total": 1, "hits": [{"id": "451647", "metadata": {
                    "titles": [{"title": "The Large N limit of SCFTs"}],
                    "authors": [{"full_name": "Maldacena, Juan Martin"}],
                    "texkeys": ["Maldacena:1997re"], "citation_count": 20150, "preprint_date": "1998-01-15",
                    "dois": [{"value": "10.4310/ATMP.1998.v2.n2.a1"}], "arxiv_eprints": [{"value": "hep-th/9711200"}]
                }}]}
            })).unwrap());
        }
        if path.starts_with("/works") {
            return MockResp::ok("application/json", serde_json::to_vec(&json!({
                "status": "ok", "message": {"items": [{
                    "DOI": "10.1038/s41586-020-2801-z", "title": ["RETRACTED ARTICLE: Room-temperature superconductivity"],
                    "author": [{"given": "E.", "family": "Snider"}], "published-print": {"date-parts": [[2020, 10]]},
                    "update-to": [{"type": "retraction", "DOI": "10.1038/s41586-022-05294-9"}],
                    "relation": {"is-retracted-by": [{"id-type": "doi", "id": "10.1038/s41586-022-05294-9"}]},
                    "is-referenced-by-count": 410
                }]}
            })).unwrap());
        }
        if path.starts_with("/graph/v1/paper/search") {
            return MockResp::ok("application/json", serde_json::to_vec(&json!({
                "total": 1, "data": [{
                    "paperId": "649def", "title": "Model for a Quantum Hall Effect",
                    "authors": [{"name": "F. D. M. Haldane"}], "year": 1988, "citationCount": 9850,
                    "externalIds": {"DOI": "10.1103/PhysRevLett.61.2015", "ArXiv": "2403.09876"},
                    "url": "https://www.semanticscholar.org/paper/649def"
                }]
            })).unwrap());
        }
        MockResp::not_found()
    });

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data_dir).with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap()));
    let session = "s-lit01";
    let router = build_router(env.clone(), session);

    let ins = router.try_call("agent:test", LITERATURE_SEARCH, &json!({"source": "inspire", "query": "Maldacena"})).unwrap();
    match ins {
        CallOutcome::Ok(v) => {
            assert_eq!(v["hits"][0]["source"], "inspire");
            assert_eq!(v["hits"][0]["citation_count"], 20150);
            assert_eq!(v["hits"][0]["arxiv_id"], "hep-th/9711200");
            assert!(v["hits"][0]["bibtex"].as_str().unwrap().contains("Maldacena:1997re"));
        }
        other => panic!("unexpected {other:?}"),
    }

    let cr = router.try_call("agent:test", LITERATURE_SEARCH, &json!({"source": "crossref", "query": "hydride"})).unwrap();
    match cr {
        CallOutcome::Ok(v) => {
            assert_eq!(v["hits"][0]["retracted"], true);
            assert_eq!(v["hits"][0]["year"], 2020);
            assert_eq!(v["hits"][0]["doi"], "10.1038/s41586-020-2801-z");
        }
        other => panic!("unexpected {other:?}"),
    }
    let asks = asked_for(session);
    assert_eq!(asks.len(), 2, "each literature_search call asks: {asks:?}");
    assert!(asks.iter().all(|(t, _)| t == LITERATURE_SEARCH));
    assert!(asks[0].1.contains("Maldacena") && asks[1].1.contains("hydride"), "{asks:?}");

    let ldg = Ledger::open(&data_dir).unwrap();
    let s2 = run_literature_search(
        &env,
        &ldg,
        session,
        &LiteratureRequest { source: Some("semantic_scholar".into()), query: "Haldane".into(), max_results: Some(3) },
    ).unwrap();
    assert_eq!(s2.len(), 1);
    assert_eq!(s2[0].citation_count, Some(9850));
    assert_eq!(s2[0].arxiv_id.as_deref(), Some("2403.09876"));
    assert_eq!(ldg.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn f9_proof_dag_acyclic_supersede_latex_repair_and_verification_state() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data_dir));
    let session = "s-dag01";
    let router = build_router(env.clone(), session);

    assert!(matches!(
        router.try_call("agent:test", LEDGER_APPEND, &json!({"entry_type": "bogus", "summary": "x", "details": "y"})).unwrap(),
        CallOutcome::Error(_)
    ));
    assert!(matches!(
        router.try_call("agent:test", LEDGER_APPEND, &json!({"entry_type": "derivation", "summary": "x", "details": "y", "depends_on": ["e-99"]})).unwrap(),
        CallOutcome::Error(_)
    ));

    let e1 = router.try_call("agent:test", LEDGER_APPEND, &json!({
        "entry_type": "assumption", "summary": "Coupling \x07lpha and \x08eta",
        "details": "Area \x0crac{1}{2} at angle \x09heta", "latex": "\x0crac{\x07lpha}{\x0dho}"
    })).unwrap();
    assert!(matches!(e1, CallOutcome::Ok(ref v) if v["entry_id"] == "e-1"));

    let sympy_seq = {
        let ldg = Ledger::open(&data_dir).unwrap();
        ldg.append(session, "tool.request", 1, &json!({"call_id": "sym1", "tool": "sympy_eval", "args": {"inline": "{}"}})).unwrap();
        ldg.append(session, "tool.result", 1, &json!({"call_id": "sym1", "ok": true, "result": {"inline": "{\"value\":\"42\"}"}})).unwrap()
    };
    assert_eq!(read_proof_dag(&Ledger::open(&data_dir).unwrap(), session).unwrap().unverified_computations, vec![sympy_seq]);

    let e2 = router.try_call("agent:test", LEDGER_APPEND, &json!({
        "entry_type": "derivation", "summary": "Step", "details": "From e-1", "depends_on": ["e-1"]
    })).unwrap();
    assert!(matches!(e2, CallOutcome::Ok(ref v) if v["entry_id"] == "e-2"));

    {
        let ldg = Ledger::open(&data_dir).unwrap();
        let b = ldg.put_blob(b"verdict").unwrap();
        ldg.append(session, "proof.verification", 1, &json!({
            "target_entry_id": "e-2", "kind": "full", "verdict": "accepted", "summary": "ok", "issues": [], "raw_blob": b
        })).unwrap();
        assert!(read_proof_dag(&ldg, session).unwrap().latest_full_verification_accepted);
    }

    assert!(matches!(
        router.try_call("agent:test", LEDGER_SUPERSEDE, &json!({"entry_id": "e-2", "replaced_by": "e-1", "reason": "not correction"})).unwrap(),
        CallOutcome::Error(_)
    ));

    let e3 = router.try_call("agent:test", LEDGER_APPEND, &json!({
        "entry_type": "correction", "summary": "Fix factor", "details": "Use 42", "depends_on": ["e-1"]
    })).unwrap();
    assert!(matches!(e3, CallOutcome::Ok(ref v) if v["entry_id"] == "e-3"));

    let e4 = router.try_call("agent:test", LEDGER_APPEND, &json!({
        "entry_type": "result", "summary": "Computed value", "details": "Matches sympy_eval",
        "depends_on": ["e-3"], "tool_call_ref": format!("sym1@seq:{sympy_seq}")
    })).unwrap();
    assert!(matches!(e4, CallOutcome::Ok(ref v) if v["entry_id"] == "e-4"));

    assert!(matches!(
        router.try_call("agent:test", LEDGER_SUPERSEDE, &json!({"entry_id": "e-2", "replaced_by": "e-3", "reason": "sign fix"})).unwrap(),
        CallOutcome::Ok(_)
    ));

    let q_all = router.try_call("agent:test", LEDGER_QUERY, &json!({})).unwrap();
    match q_all {
        CallOutcome::Ok(v) => {
            assert_eq!(v["entries"].as_array().unwrap().len(), 4);
            assert_eq!(v["latest_full_verification_accepted"], false);
            assert_eq!(v["unverified_computations"], json!([]));
        }
        other => panic!("unexpected {other:?}"),
    }
    let q_active = router.try_call("agent:test", LEDGER_QUERY, &json!({"include_superseded": false})).unwrap();
    assert!(matches!(q_active, CallOutcome::Ok(ref v) if v["entries"].as_array().unwrap().len() == 3));

    let dag = read_proof_dag(&Ledger::open(&data_dir).unwrap(), session).unwrap();
    assert_eq!(dag.entries[0].summary, r"Coupling \alpha and \beta");
    assert_eq!(dag.entries[0].latex.as_deref(), Some(r"\frac{\alpha}{\rho}"));
    assert_eq!(dag.entries[1].status, ProofStatus::Superseded);
    assert_eq!(dag.entries[1].superseded_by.as_deref(), Some("e-3"));
    assert_eq!(Ledger::open(&data_dir).unwrap().verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn f10_check_expression_sanity_pure_rust_and_python_module_syntax() {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path().join("data")));
    let router = build_router(env.clone(), "s-sanity01");

    let ok_trig = router.try_call("agent:test", CHECK_EXPRESSION_SANITY, &json!({
        "lhs": "sin(x)^2 + cos(x)^2", "rhs": "1", "variables": ["x"], "domain": "real"
    })).unwrap();
    match ok_trig {
        CallOutcome::Ok(v) => {
            assert_eq!(v["ok"], true);
            assert_eq!(v["numeric_eval_supported"], true);
            assert_eq!(v["samples_checked"], 5);
        }
        other => panic!("unexpected {other:?}"),
    }

    let ok_poly = router.try_call("agent:test", CHECK_EXPRESSION_SANITY, &json!({
        "lhs": "(x + y)^2", "rhs": "x^2 + 2*x*y + y^2", "variables": ["x", "y"]
    })).unwrap();
    assert!(matches!(ok_poly, CallOutcome::Ok(ref v) if v["ok"] == true));

    let bad_poly = router.try_call("agent:test", CHECK_EXPRESSION_SANITY, &json!({
        "lhs": "(x + y)^2", "rhs": "x^2 + y^2", "variables": ["x", "y"]
    })).unwrap();
    assert!(matches!(bad_poly, CallOutcome::Ok(ref v) if v["ok"] == false && v["numeric_match"] == false));

    let bad_delims = router.try_call("agent:test", CHECK_EXPRESSION_SANITY, &json!({
        "lhs": r"\frac{x}{y", "rhs": "x/y"
    })).unwrap();
    assert!(matches!(bad_delims, CallOutcome::Ok(ref v) if v["ok"] == false && v["delimiter_balanced"] == false));
}

/// Negative control for the network-read clamp: a refused search never reaches the network.
#[test]
fn network_read_refused_by_human_makes_no_request() {
    struct RefuseAll;
    impl Approver for RefuseAll {
        fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
            ApprovalReply { approved: false, digest: req.digest.clone(), by: "test".into() }
        }
    }
    let srv = MockHttpServer::start(|_, _| MockResp::not_found());
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let env = Arc::new(ResearchEnv::new(&data_dir).with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap()));
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let router = Router::new(reg, research_policy(), Ledger::open(&data_dir).unwrap(), "s-refuse01", Arc::new(RefuseAll));
    for (tool, args) in [
        (ARXIV_SEARCH, json!({"query": "secret workspace text"})),
        (LITERATURE_SEARCH, json!({"source": "crossref", "query": "secret workspace text"})),
    ] {
        let out = router.try_call("agent:test", tool, &args).unwrap();
        assert!(matches!(out, CallOutcome::Denied(_)), "{tool}: {out:?}");
    }
    assert!(srv.seen_paths.lock().unwrap().is_empty(), "a refused search must not touch the network");
}

#[test]
fn stage10_arxiv_inline_prefixes_single_gz_eprint_and_session_network_grant() {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(b"\\documentclass{article}\n\\begin{document}\n\\section{Intro}\nSpin chain.\n\\section{Split Property}\nBounded split entanglement entropy.\n\\end{document}").unwrap();
    let single_gz = gz.finish().unwrap();
    let atom_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <entry>
    <id>http://arxiv.org/abs/1109.5778v1</id><updated>2011-09-27T00:00:00Z</updated><published>2011-09-27T00:00:00Z</published>
    <title>Boundedness of entanglement entropy and split property of quantum spin chains</title>
    <summary>Split property holds when entanglement entropy is bounded.</summary>
    <author><name>Taku Matsui</name></author><arxiv:primary_category term="math-ph"/>
  </entry>
</feed>"#
        .to_string();
    let srv = MockHttpServer::start(move |path, _| {
        if path.starts_with("/api/query") {
            return MockResp::ok("application/atom+xml", atom_xml.as_bytes().to_vec());
        }
        if path.starts_with("/e-print/1109.5778v1") {
            return MockResp::ok("application/x-gzip", single_gz.clone());
        }
        if path.starts_with("/works") {
            return MockResp::ok("application/json", br#"{"status":"ok","message":{"items":[]}}"#.to_vec());
        }
        MockResp::not_found()
    });

    struct SessionLitApprover(Mutex<usize>);
    impl Approver for SessionLitApprover {
        fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
            ApprovalReply { approved: false, digest: req.digest.clone(), by: "unexpected".into() }
        }
        fn decide_ext(&self, req: &ApprovalRequest) -> tools::ApprovalOutcome {
            let mut n = self.0.lock().unwrap();
            *n += 1;
            let is_net = req.effect == policy::EffectClass::NetworkRead;
            tools::ApprovalOutcome {
                reply: ApprovalReply { approved: is_net, digest: req.digest.clone(), by: "human-ui".into() },
                approve_for_session: true,
            }
        }
    }

    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let data_dir = tmp.path().join("data");
    let env = Arc::new(
        ResearchEnv::with_workspace(&data_dir, &ws)
            .unwrap()
            .with_endpoints(LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap()),
    );
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let approver = Arc::new(SessionLitApprover(Mutex::new(0)));
    let router = Router::new(reg, research_policy(), Ledger::open(&data_dir).unwrap(), "s-matsui01", approver.clone());

    // 1. Inline `au:` + `ti:` query must NOT double-prefix `all:au:`, and single-file `.gz` with dot in ID gets `.tex` + `__filtered__.tex`.
    let out1 = router
        .try_call(
            "agent:test",
            ARXIV_SEARCH,
            &json!({"query": "au:Matsui AND ti:split", "include_source": true, "section_filter": "Split"}),
        )
        .unwrap();
    let papers = match out1 {
        CallOutcome::Ok(v) => v["papers"].as_array().unwrap().clone(),
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(papers.len(), 1);
    assert!(papers[0]["source"]["primary_path"].as_str().unwrap().ends_with("/arxiv_1109.5778v1.tex"));
    assert!(papers[0]["source"]["assembled_path"].as_str().unwrap().ends_with("/__filtered__.tex"));
    assert!(papers[0]["source"]["excerpt"].as_str().unwrap().contains("Bounded split entanglement entropy"));
    let seen = srv.seen_paths.lock().unwrap().clone();
    assert!(seen[0].contains("search_query=au%3AMatsui%20AND%20ti%3Asplit"), "{seen:?}");
    assert!(!seen[0].contains("all%3Aau"), "{seen:?}");

    // 2. Subsequent `arxiv_search` and `literature_search` reuse the session grant without re-prompting, while `fs_write` still prompts (and is denied here).
    assert_eq!(*approver.0.lock().unwrap(), 1);
    assert!(matches!(router.try_call("agent:test", ARXIV_SEARCH, &json!({"query": "author:Matsui"})).unwrap(), CallOutcome::Ok(_)));
    assert!(matches!(router.try_call("agent:test", LITERATURE_SEARCH, &json!({"source": "crossref", "query": "Matsui"})).unwrap(), CallOutcome::Ok(_)));
    assert_eq!(*approver.0.lock().unwrap(), 1, "network-read grant reused across literature tools");
    assert!(matches!(router.try_call("agent:test", "fs_write", &json!({"path": "x.tex", "content": "hi"})).unwrap(), CallOutcome::Denied(_)));
    assert_eq!(*approver.0.lock().unwrap(), 2, "fs_write must never reuse network-read grant");
}
