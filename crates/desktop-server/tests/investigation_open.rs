//! Loopback-mock integration tests for Investigate V4 (`desktop_server::investigation_open`).
//!
//! Zero real network calls, zero real sleeps (uses `FakeRateClock`), zero model calls.

use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::investigation_open::{
    effective_fetch_limit, get_open_job, handle, start_open_job_with_gate, wait_for_open_job,
    OpenStep, DEFAULT_FETCH_LIMIT,
};
use desktop_server::investigation_skeleton::EVENT_INVESTIGATION_SKELETON;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use desktop_server::DesktopConfig;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::LiteratureEndpoints;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

struct MockLitServer {
    port: u16,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockLitServer {
    fn start(eprint_tex: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback mock");
        listener.set_nonblocking(true).expect("set nonblocking");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let h_hits = Arc::clone(&hits);
        let h_stop = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let atom_xml = concat!(
                r#"<?xml version="1.0" encoding="UTF-8"?>"#,
                r#"<feed xmlns="http://www.w3.org/2005/Atom">"#,
                r#"<entry>"#,
                r#"<id>http://arxiv.org/abs/2601.00055v2</id>"#,
                r#"<published>2026-01-05T00:00:00Z</published>"#,
                r#"<updated>2026-01-10T00:00:00Z</updated>"#,
                r#"<title>Toy Graph Decay Estimates</title>"#,
                r#"<summary>We prove a discrete decay bound on weighted graphs.</summary>"#,
                r#"<author><name>A. Researcher</name></author>"#,
                r#"<link rel="license" href="http://arxiv.org/licenses/nonexclusive-distrib/1.0/"/>"#,
                r#"</entry></feed>"#,
            );
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

                        let (status, ct, body): (&str, &str, Vec<u8>) =
                            if path.starts_with("/api/query") {
                                ("200 OK", "application/atom+xml", atom_xml.as_bytes().to_vec())
                            } else if path.starts_with("/e-print/2601.00055") {
                                ("200 OK", "application/x-tex", eprint_tex.clone())
                            } else if path.starts_with("/api/literature") {
                                ("200 OK", "application/json", br#"{"hits":{"hits":[]}}"#.to_vec())
                            } else if path.starts_with("/graph/v1/") {
                                ("404 Not Found", "application/json", br#"{"error":"not found"}"#.to_vec())
                            } else if path.starts_with("/works/") {
                                (
                                    "200 OK",
                                    "application/json",
                                    br#"{"status":"ok","message":{"DOI":"10.1000/paywalled.99","title":["Paywalled Note"]}}"#.to_vec(),
                                )
                            } else {
                                ("404 Not Found", "text/plain", b"not found".to_vec())
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
        Self { port, hits, stop, handle: Some(handle) }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
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

fn sample_tex_bytes() -> Vec<u8> {
    br#"\documentclass{article}
\usepackage{amsmath,amsthm}
\newtheorem{theorem}{Theorem}[section]
\newtheorem{lemma}[theorem]{Lemma}
\begin{document}
\section{Setup}\label{sec:setup}
\begin{lemma}[Base estimate]\label{lem:base}
For any unit vector $v$, $\|v\|_2 = 1$.
\end{lemma}
\begin{theorem}[Main decay]\label{thm:decay}
Using Lemma~\ref{lem:base}, the decay constant is positive.
\end{theorem}
\begin{proof}
Immediate from Lemma~\ref{lem:base}.
\end{proof}
\end{document}
"#
    .to_vec()
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
fn open_by_arxiv_id_fetches_tex_builds_skeleton_with_zero_model_calls() {
    let tmp = TempDir::new().unwrap();
    let mock = MockLitServer::start(sample_tex_bytes());
    let state = make_state(tmp.path(), mock.port);
    init_investigation_session(&state, "sess-open-arxiv");

    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let initial = start_open_job_with_gate(
        &state,
        "sess-open-arxiv",
        &json!({ "arxiv_id": "2601.00055v2" }),
        Some(gate),
    )
    .expect("start open job");
    assert_eq!(initial.session_id, "sess-open-arxiv");

    // Cross-session isolation: another session must not see this job.
    assert!(get_open_job("other-session", &initial.job_id).is_none());

    let done = wait_for_open_job("sess-open-arxiv", &initial.job_id, Duration::from_secs(10))
        .expect("job finishes");
    assert_eq!(done.status, "done", "unexpected snapshot: {done:?}");
    assert_eq!(done.step, OpenStep::Done);
    assert!(done.history.contains(&OpenStep::Metadata));
    assert!(done.history.contains(&OpenStep::Ingest));
    assert!(done.history.contains(&OpenStep::Skeleton));
    assert_eq!(
        done.metadata.as_ref().map(|m| m.title.as_str()),
        Some("Toy Graph Decay Estimates")
    );
    assert!(done.source_id.is_some());
    assert!(done.skeleton_seq.is_some());
    assert!(done.skeleton_blob.is_some());

    // Ledger must have source.ingested + investigation.paper + investigation.skeleton and ZERO model.request!
    let events = state.open_ledger().unwrap().read_session("sess-open-arxiv").unwrap();
    assert!(events.iter().any(|e| e.kind == "source.ingested"));
    assert!(events.iter().any(|e| e.kind == "investigation.paper"));
    assert!(events.iter().any(|e| e.kind == EVENT_INVESTIGATION_SKELETON));
    assert!(
        !events.iter().any(|e| e.kind == "model.request"),
        "open must make zero model calls"
    );
}

#[test]
fn open_by_paywalled_doi_returns_needs_file() {
    let tmp = TempDir::new().unwrap();
    let mock = MockLitServer::start(sample_tex_bytes());
    let state = make_state(tmp.path(), mock.port);
    init_investigation_session(&state, "sess-open-doi");

    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let initial = start_open_job_with_gate(
        &state,
        "sess-open-doi",
        &json!({ "doi": "10.1000/paywalled.99" }),
        Some(gate),
    )
    .expect("start doi job");
    let done = wait_for_open_job("sess-open-doi", &initial.job_id, Duration::from_secs(10))
        .expect("doi job finishes");
    assert_eq!(done.status, "needs_file", "expected needs_file, got: {done:?}");
    assert_eq!(done.step, OpenStep::NeedsFile);
    assert!(done.needs_file_reason.is_some());
}

#[test]
fn open_by_library_key_and_source_id_use_zero_network() {
    let tmp = TempDir::new().unwrap();
    let mock = MockLitServer::start(sample_tex_bytes());
    let state = make_state(tmp.path(), mock.port);
    init_investigation_session(&state, "sess-open-lib");

    // Seed the F10 library with a TeX blob directly (no network).
    let tex = sample_tex_bytes();
    let ledger = state.open_ledger().unwrap();
    let blob_sha = ledger.put_blob(&tex).unwrap();
    let key = PaperKey::parse("arxiv:2601.00077").unwrap();
    let lib = Library::open(&state.cfg.data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds { arxiv: Some("2601.00077".into()), ..Default::default() },
        title: "Library Cached Paper".into(),
        authors: vec!["A. Author".into()],
        year: Some(2026),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_fulltext(&FulltextRecord {
        key: key.clone(),
        version: Some("v1".into()),
        raw_blob: Some(blob_sha),
        format: "tex".into(),
        bytes: tex.len() as u64,
        origin: "library".into(),
        fetched_ms: 1,
        file_path: None,
        file_size: None,
        file_mtime_ms: None,
        file_sha256: None,
    })
    .unwrap();

    let (status, val) = handle(
        &state,
        "POST",
        "sess-open-lib",
        "open",
        &HashMap::new(),
        serde_json::to_vec(&json!({ "library_key": "arxiv:2601.00077" }))
            .unwrap()
            .as_slice(),
    )
    .expect("open from library");
    assert_eq!(status, 200);
    assert_eq!(val["job"]["status"], "done");
    assert!(val["job"]["skeleton_seq"].as_u64().is_some());
    assert_eq!(mock.hits(), 0, "library_key open must make ZERO network requests");

    // Also test opening an already-ingested source_id in another session with zero network.
    init_investigation_session(&state, "sess-open-src");
    let env = state.research_env_for_session("sess-open-src").unwrap();
    let l_mu = Mutex::new(state.open_ledger().unwrap());
    let ingested = ingest_bytes(
        &env.sandbox(),
        &l_mu,
        &IngestRequest {
            session_id: "sess-open-src",
            file_name: "local.tex",
            origin: "upload",
            raw_bytes: &tex,
            source_id_hint: Some("local"),
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let (st2, val2) = handle(
        &state,
        "POST",
        "sess-open-src",
        "open",
        &HashMap::new(),
        serde_json::to_vec(&json!({ "source_id": ingested.source_id }))
            .unwrap()
            .as_slice(),
    )
    .expect("open existing source_id");
    assert_eq!(st2, 200);
    assert_eq!(val2["job"]["status"], "done");
    assert!(val2["job"]["skeleton_seq"].as_u64().is_some());
    assert_eq!(mock.hits(), 0, "source_id open must make ZERO network requests");
}

#[test]
fn fetch_budget_blocks_ninth_external_fetch_until_raised_by_note() {
    let tmp = TempDir::new().unwrap();
    let mock = MockLitServer::start(sample_tex_bytes());
    let state = make_state(tmp.path(), mock.port);
    init_investigation_session(&state, "sess-budget");
    let ledger = state.open_ledger().unwrap();
    for i in 1..=DEFAULT_FETCH_LIMIT {
        ledger
            .append(
                "sess-budget",
                "source.ingested",
                1,
                &json!({
                    "source_id": format!("ext-{i}"),
                    "origin": format!("https://arxiv.org/e-print/2601.000{i:02}"),
                    "raw_blob": format!("{i:064x}"),
                }),
            )
            .unwrap();
    }
    let events = ledger.read_session("sess-budget").unwrap();
    assert_eq!(effective_fetch_limit(&events), DEFAULT_FETCH_LIMIT);

    // 9th external fetch is refused with 429 before any network call.
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let err = start_open_job_with_gate(
        &state,
        "sess-budget",
        &json!({ "arxiv_id": "2601.00055v2" }),
        Some(Arc::clone(&gate)),
    )
    .unwrap_err();
    assert_eq!(err.0, 429, "expected 429 fetch budget refusal, got {err:?}");
    assert_eq!(mock.hits(), 0);

    // Reader clicks to raise fetch limit (recorded as investigation.note) -> 9th fetch now succeeds.
    ledger
        .append("sess-budget", "investigation.note", 1, &json!({ "fetch_limit": 10 }))
        .unwrap();
    let events2 = ledger.read_session("sess-budget").unwrap();
    assert_eq!(effective_fetch_limit(&events2), 10);

    let snap = start_open_job_with_gate(
        &state,
        "sess-budget",
        &json!({ "arxiv_id": "2601.00055v2" }),
        Some(gate),
    )
    .expect("9th fetch allowed after raising limit");
    let done = wait_for_open_job("sess-budget", &snap.job_id, Duration::from_secs(10)).unwrap();
    assert_eq!(done.status, "done");
}
