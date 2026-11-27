//! Integration tests for package X3 (Explore session, stage machine, view, routes).
//!
//! Loopback mocks only; zero live network calls, zero model calls, zero third-party paper text.

mod common;

use common::{drain_http_request, launch_cookie, raw_http, test_cfg};
use desktop_server::idea::{
    append_explore_session_start, budget_snapshot, build_idea_statement, next_stage,
    preview_first_queries, run_first_search_with_gate, validate_recorder_stage, IdeaPreset,
    IdeaRecorder, IdeaStage, EVENT_IDEA_BRIEF, EVENT_IDEA_CANDIDATES, EVENT_IDEA_EXCHANGE,
    EVENT_IDEA_MAP, EVENT_IDEA_STAGE, EVENT_IDEA_TRIAGE,
};
use desktop_server::idea_view;
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::idea_terms::extract as extract_idea_terms;
use research_tools::library::{
    FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord, ORIGIN_USER_LIBRARY,
};
use research_tools::net_rate::{FakeRateClock, NetRateGate, RateClock};
use research_tools::paper_key::PaperKey;
use research_tools::LiteratureEndpoints;
use serde_json::{json, Value};
use std::io::Write;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

const SAMPLE_IDEA: &str =
    "Use Krylov complexity and Lanczos coefficients to detect the monitored Majorana chain transition; compare with arXiv:2405.01111.";

fn synthetic_arxiv_atom() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <opensearch:totalResults xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">2</opensearch:totalResults>
  <entry>
    <id>http://arxiv.org/abs/2405.01111v1</id>
    <published>2024-05-02T00:00:00Z</published>
    <title>Synthetic Krylov Complexity Fixture on Majorana Chains</title>
    <summary>Self-written fixture abstract discussing Krylov complexity and Lanczos growth in a toy Majorana lattice.</summary>
    <author><name>Ada Example</name></author>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2405.02222v1</id>
    <published>2024-05-03T00:00:00Z</published>
    <title>Synthetic Operator Spreading Benchmark</title>
    <summary>Self-written fixture abstract for operator spreading without measurements.</summary>
    <author><name>Bram Test</name></author>
  </entry>
</feed>"#
        .to_vec()
}

fn synthetic_inspire_json() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "hits": {
            "total": 1,
            "hits": [{
                "id": "990001",
                "metadata": {
                    "titles": [{ "title": "Synthetic INSPIRE Lanczos Growth Study" }],
                    "authors": [{ "full_name": "Claire Sample" }],
                    "earliest_date": "2024-04-10",
                    "arxiv_eprints": [{ "value": "2405.03333" }],
                    "abstracts": [{ "value": "Self-written INSPIRE fixture abstract on Lanczos coefficients and Krylov complexity." }],
                    "citation_count": 12
                }
            }]
        }
    }))
    .unwrap()
}

struct CountingMock {
    base_url: String,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CountingMock {
    fn start() -> Self {
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
                        let mut first_line = String::new();
                        {
                            use std::io::BufRead;
                            let mut reader = std::io::BufReader::new(&mut stream);
                            let _ = reader.read_line(&mut first_line);
                            loop {
                                let mut line = String::new();
                                if reader.read_line(&mut line).unwrap_or(0) == 0
                                    || line == "\r\n"
                                    || line == "\n"
                                {
                                    break;
                                }
                            }
                        }
                        let (ct, body) = if first_line.contains("/api/query") {
                            ("application/atom+xml; charset=utf-8", synthetic_arxiv_atom())
                        } else {
                            ("application/json; charset=utf-8", synthetic_inspire_json())
                        };
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { base_url: format!("http://{addr}"), hits, stop, handle: Some(handle) }
    }

    fn hit_count(&self) -> usize { self.hits.load(Ordering::Acquire) }
}

impl Drop for CountingMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}

fn seed_library_papers(data_dir: &std::path::Path) {
    let lib = Library::open(data_dir).unwrap();
    let owned_key = PaperKey::parse("arxiv:2405.01111").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: owned_key.clone(),
        ids: LibraryIds { arxiv: Some("2405.01111".into()), ..Default::default() },
        title: "Synthetic Krylov Complexity Fixture on Majorana Chains".into(),
        authors: vec!["Ada Example".into()],
        year: Some(2024),
        abstract_text: "Krylov complexity and Lanczos coefficients in a synthetic Majorana chain.".into(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, ..Default::default() },
        first_seen_ms: 1000,
        last_seen_ms: 2000,
    }).unwrap();
    lib.upsert_fulltext(&FulltextRecord {
        key: owned_key,
        version: Some("v1".into()),
        raw_blob: None,
        format: "pdf".into(),
        bytes: 1234,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 2000,
        file_path: Some("refs/papers/example/paper.pdf".into()),
        file_size: Some(1234),
        file_mtime_ms: Some(2000),
        file_sha256: None,
    }).unwrap();

    let cached_key = PaperKey::parse("arxiv:2405.09999").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: cached_key,
        ids: LibraryIds { arxiv: Some("2405.09999".into()), ..Default::default() },
        title: "Cached Non-Owned Monitored Chain Fixture".into(),
        authors: vec!["Devon Cached".into()],
        year: Some(2023),
        abstract_text: "Synthetic cached abstract on monitored Majorana chain entanglement.".into(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1000,
        last_seen_ms: 2000,
    }).unwrap();
}

#[test]
fn idea_preview_is_local_and_deterministic() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    seed_library_papers(&data_dir);
    let mock = CountingMock::start();
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, Some(&mock.base_url))).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];

    let req_body = serde_json::to_vec(&json!({
        "text": format!("  {SAMPLE_IDEA}  "),
        "fields": ["cat:cond-mat.str-el", "cond-mat.str-el"]
    })).unwrap();
    let (st1, _, b1) = raw_http(port, "POST", "/api/idea/preview", &auth, &req_body);
    let (st2, _, b2) = raw_http(port, "POST", "/api/idea/preview", &auth, &req_body);
    assert_eq!(st1, 200, "{}", String::from_utf8_lossy(&b1));
    assert_eq!(st2, 200);
    assert_eq!(b1, b2, "preview must be deterministic across identical calls");
    assert_eq!(mock.hit_count(), 0, "preview must make zero HTTP requests");

    let v: Value = serde_json::from_slice(&b1).unwrap();
    assert_eq!(v["focus_investigate"], false);
    assert_eq!(v["fields"], json!(["cond-mat.str-el"]), "fields normalized/deduped identically to create");
    assert!(!v["terms"].as_array().unwrap().is_empty());
    assert_eq!(v["paper_refs"][0]["key"], "arxiv:2405.01111");
    let queries = v["first_queries"].as_array().unwrap();
    assert!(!queries.is_empty() && queries.len() <= 6);
    for q in queries {
        let p = q["provider"].as_str().unwrap();
        assert!(p == "arxiv" || p == "inspire", "unexpected preview provider {p}");
    }
    let matches = v["library_matches"].as_array().unwrap();
    assert!(matches.iter().any(|m| m["paper"]["key"] == "arxiv:2405.01111" && m["owned"] == true));
    assert!(matches.iter().any(|m| m["paper"]["key"] == "arxiv:2405.09999" && m["owned"] == false));

    let bare = serde_json::to_vec(&json!({ "text": "2405.01111" })).unwrap();
    let (st3, _, b3) = raw_http(port, "POST", "/api/idea/preview", &auth, &bare);
    assert_eq!(st3, 200);
    assert_eq!(serde_json::from_slice::<Value>(&b3).unwrap()["focus_investigate"], true);

    let (st_l, _, list_raw) = raw_http(port, "GET", "/api/sessions", &auth, b"");
    assert_eq!(st_l, 200);
    assert_eq!(serde_json::from_slice::<Value>(&list_raw).unwrap()["sessions"].as_array().unwrap().len(), 0);
}

#[test]
fn idea_user_queries_are_exactly_shown_ones() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    seed_library_papers(&data_dir);
    let mock = CountingMock::start();
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, Some(&mock.base_url))).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];

    let prev_body = serde_json::to_vec(&json!({ "text": SAMPLE_IDEA })).unwrap();
    let (st, _, prev_raw) = raw_http(port, "POST", "/api/idea/preview", &auth, &prev_body);
    assert_eq!(st, 200);
    let preview: Value = serde_json::from_slice(&prev_raw).unwrap();
    let first_qs = preview["first_queries"].as_array().unwrap();
    let one_arxiv = first_qs.iter().find(|q| q["provider"] == "arxiv").unwrap();
    let one_inspire = first_qs.iter().find(|q| q["provider"] == "inspire").unwrap();

    // 0. Omitted or empty queries with offline=false -> 400 (no silent unshown search, Rule P4).
    for payload in [
        json!({ "mode": "explore", "text": SAMPLE_IDEA }),
        json!({ "mode": "explore", "text": SAMPLE_IDEA, "queries": [] }),
        json!({ "mode": "explore", "text": SAMPLE_IDEA, "queries": [{ "provider": "arxiv", "q": "tampered" }] }),
        json!({ "mode": "explore", "text": SAMPLE_IDEA, "queries": vec![json!({ "provider": "arxiv", "q": one_arxiv["q"] }); 7] }),
        json!({ "mode": "explore", "text": SAMPLE_IDEA, "queries": [{ "provider": "crossref", "q": one_arxiv["q"] }] }),
        json!({ "mode": "explore", "text": SAMPLE_IDEA, "queries": [{ "provider": "semantic_scholar", "q": one_arxiv["q"] }] }),
    ] {
        let body = serde_json::to_vec(&payload).unwrap();
        let (st, _, _) = raw_http(port, "POST", "/api/sessions", &auth, &body);
        assert_eq!(st, 400, "invalid Rule P4 payload must be rejected: {payload}");
    }
    assert_eq!(mock.hit_count(), 0);
    let (_, _, list_raw) = raw_http(port, "GET", "/api/sessions", &auth, b"");
    assert_eq!(serde_json::from_slice::<Value>(&list_raw).unwrap()["sessions"].as_array().unwrap().len(), 0);

    // Offline mode -> 0 network requests, library matches only.
    let off_req = serde_json::to_vec(&json!({
        "id": "s-idea-offline", "mode": "explore", "text": SAMPLE_IDEA, "offline": true, "preset": "quick"
    })).unwrap();
    let (st, _, off_raw) = raw_http(port, "POST", "/api/sessions", &auth, &off_req);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&off_raw));
    assert_eq!(mock.hit_count(), 0, "offline: true must send 0 network requests");
    let off_v: Value = serde_json::from_slice(&off_raw).unwrap();
    assert_eq!(off_v["mode"], "explore");
    assert_eq!(off_v["idea"]["empty_state"]["code"], "offline_library_only");
    assert!(!off_v["idea"]["candidates"].as_array().unwrap().is_empty());

    // Exact shown queries (1 arXiv + 1 INSPIRE) -> 200, no approval prompt, online + library merged.
    let ok_req = serde_json::to_vec(&json!({
        "id": "s-idea-online", "mode": "explore", "text": SAMPLE_IDEA, "preset": "standard",
        "queries": [
            { "provider": one_arxiv["provider"], "q": one_arxiv["q"] },
            { "provider": one_inspire["provider"], "q": one_inspire["q"] }
        ]
    })).unwrap();
    let (st, _, ok_raw) = raw_http(port, "POST", "/api/sessions", &auth, &ok_req);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&ok_raw));
    assert_eq!(mock.hit_count(), 2, "expected 1 arXiv + 1 INSPIRE request");

    let (st_g, _, get_raw) = raw_http(port, "GET", "/api/sessions/s-idea-online", &auth, b"");
    assert_eq!(st_g, 200);
    let sess: Value = serde_json::from_slice(&get_raw).unwrap();
    assert_eq!(sess["mode"], "explore");
    let idea = &sess["idea"];
    assert_eq!(idea["preset"], "standard");
    assert_eq!(idea["run_bar"]["modelCallsMax"], 24);
    assert_eq!(idea["map"], json!({ "nodes": [], "edges": [], "legend": [] }));
    let cands = idea["candidates"].as_array().unwrap();
    assert_eq!(cands.len(), 4, "expected 2 arXiv + 1 INSPIRE + 1 local library merged candidates");
    let owned_c = cands.iter().find(|c| c["key"] == "arxiv:2405.01111").unwrap();
    assert_eq!(owned_c["in_library"], true);
    assert_eq!(owned_c["owned"], true);
    assert_eq!(owned_c["read_badge"], "not read");
    let fresh_c = cands.iter().find(|c| c["key"] == "arxiv:2405.03333").unwrap();
    assert_eq!(fresh_c["in_library"], false);
    assert_eq!(fresh_c["owned"], false);
    assert!(cands.iter().any(|c| c["key"] == "arxiv:2405.09999" && c["in_library"] == true));
    let crossref = idea["where_i_looked"]["providers"].as_array().unwrap()
        .iter().find(|p| p["name"] == "crossref").unwrap();
    assert_eq!(crossref["state"], "not_attempted");

    // Ledger integrity + F10 seen recording + Rule S8 wording check on app-owned strings.
    Ledger::open(&data_dir).unwrap().verify_all().unwrap();
    let lib = Library::open(&data_dir).unwrap();
    let fresh_detail = lib.get_paper(&PaperKey::parse("arxiv:2405.03333").unwrap()).unwrap().unwrap();
    assert!(fresh_detail.seen.iter().any(|s| s.session_id == "s-idea-online"));
    for field in [
        idea["no_model_notice"].as_str().unwrap_or(""),
        idea["run_bar"]["cost_line"].as_str().unwrap_or(""),
        idea["run_bar"]["label"].as_str().unwrap_or(""),
    ] {
        let low = field.to_ascii_lowercase();
        assert!(!low.contains("verified") && !low.contains("novel") && !low.contains("new"), "Rule S8: {field}");
    }
}

#[test]
fn idea_recorder_refused_outside_stage_and_resumes_after_crash() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let stmt = build_idea_statement(1, SAMPLE_IDEA, &[], &[]).unwrap();
    append_explore_session_start(&ledger, "s-stage-test", IdeaPreset::Quick, false, &stmt).unwrap();

    let evs = ledger.read_session("s-stage-test").unwrap();
    assert_eq!(next_stage(&evs), Some(IdeaStage::Sharpen));
    for rec in [
        IdeaRecorder::RecordIdeaBrief, IdeaRecorder::RecordIdeaTriage,
        IdeaRecorder::RecordPaperEvidence, IdeaRecorder::RecordIdeaMap, IdeaRecorder::RecordIdeaDirections,
    ] {
        assert!(validate_recorder_stage(&evs, rec).is_err(), "{rec:?} must be refused when idle");
    }

    // Enter Sharpen (`requested`) -> only RecordIdeaBrief is allowed.
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({
        "stage": "sharpen", "state": "requested", "request_id": "sh-1"
    })).unwrap();
    let evs = ledger.read_session("s-stage-test").unwrap();
    assert!(validate_recorder_stage(&evs, IdeaRecorder::RecordIdeaBrief).is_ok());
    assert!(validate_recorder_stage(&evs, IdeaRecorder::RecordIdeaTriage).is_err());

    // Simulate crash (reopen with running=false while stage is `requested` — Rule S3).
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];
    let before_len = ledger.read_session("s-stage-test").unwrap().len();
    let (st, _, raw) = raw_http(port, "GET", "/api/sessions/s-stage-test", &auth, b"");
    assert_eq!(st, 200);
    let sess: Value = serde_json::from_slice(&raw).unwrap();
    let sharpen = sess["idea"]["stages"].as_array().unwrap()
        .iter().find(|s| s["id"] == "sharpen").unwrap();
    assert_eq!(sharpen["stored_state"], "interrupted");
    assert_eq!(sharpen["run_state_label"], "Interrupted");
    assert_eq!(sharpen["action"], "retry");
    assert_eq!(ledger.read_session("s-stage-test").unwrap().len(), before_len);

    // Finish Sharpen -> next_stage is post-Brief Search; failed Search blocks Triage recorder.
    ledger.append("s-stage-test", EVENT_IDEA_BRIEF, 1, &json!({ "idea_rev": 1 })).unwrap();
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({ "stage": "sharpen", "state": "finished" })).unwrap();
    assert_eq!(next_stage(&ledger.read_session("s-stage-test").unwrap()), Some(IdeaStage::Search));
    ledger.append("s-stage-test", EVENT_IDEA_CANDIDATES, 1, &json!({ "round": 1, "candidates": [] })).unwrap();
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({ "stage": "search", "state": "failed" })).unwrap();
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({ "stage": "triage", "state": "requested" })).unwrap();
    assert!(validate_recorder_stage(&ledger.read_session("s-stage-test").unwrap(), IdeaRecorder::RecordIdeaTriage).is_err());

    // Finish Search + Triage -> Quick preset skips Read straight to Map.
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({ "stage": "search", "state": "finished" })).unwrap();
    ledger.append("s-stage-test", EVENT_IDEA_TRIAGE, 1, &json!({ "items": [] })).unwrap();
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({ "stage": "triage", "state": "finished" })).unwrap();
    ledger.append("s-stage-test", EVENT_IDEA_EXCHANGE, 1, &json!({ "stage": "sharpen", "attempt": 1 })).unwrap();
    let evs = ledger.read_session("s-stage-test").unwrap();
    assert_eq!(next_stage(&evs), Some(IdeaStage::Map));
    assert_eq!(budget_snapshot(&evs, false).model_calls.used, 1);

    // Map stage active -> stop cancels it; revise resets next_stage to Sharpen and marks map_stale.
    ledger.append("s-stage-test", EVENT_IDEA_STAGE, 1, &json!({ "stage": "map", "state": "requested" })).unwrap();
    assert!(validate_recorder_stage(&ledger.read_session("s-stage-test").unwrap(), IdeaRecorder::RecordIdeaMap).is_ok());
    let (st_stop, _, stop_raw) = raw_http(port, "POST", "/api/sessions/s-stage-test/idea/stop", &auth, b"{}");
    assert_eq!(st_stop, 200);
    assert_eq!(serde_json::from_slice::<Value>(&stop_raw).unwrap()["cancelled"], json!(["map"]));
    ledger.append("s-stage-test", EVENT_IDEA_MAP, 1, &json!({ "nodes": [], "edges": [] })).unwrap();

    let rev_body = serde_json::to_vec(&json!({ "text": "Revised Krylov complexity question" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-stage-test/idea/revise", &auth, &rev_body);
    assert_eq!(st, 200);
    let rev_v: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(rev_v["idea"]["statement"]["rev"], 2);
    assert_eq!(rev_v["idea"]["map_stale"], true);
    assert_eq!(rev_v["idea"]["next_stage"], "sharpen");

    // Note, invalid continue type (400), and 501 ops for X6-X16.
    let note_body = serde_json::to_vec(&json!({
        "target_kind": "candidate", "target_id": "arxiv:2405.01111", "action": "key_paper"
    })).unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions/s-stage-test/idea/note", &auth, &note_body).0, 200);
    let bad_bump = serde_json::to_vec(&json!({ "extra_queries": -3 })).unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions/s-stage-test/idea/continue", &auth, &bad_bump).0, 400);

    for op in ["triage", "read", "map", "directions", "handoff", "more"] {
        let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/s-stage-test/idea/{op}"), &auth, b"{}");
        assert_eq!(st, 501, "op `{op}` must return 501 not built yet");
        let msg = serde_json::from_slice::<Value>(&raw).unwrap()["error"].as_str().unwrap().to_string();
        assert!(msg.contains("not built yet"), "op `{op}` message: {msg}");
    }
    let _ = idea_view::view(&ledger.read_session("s-stage-test").unwrap(), false);
}

#[test]
fn idea_requests_all_pass_the_gate_and_auth() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let body = serde_json::to_vec(&json!({ "text": SAMPLE_IDEA })).unwrap();

    assert_eq!(raw_http(port, "POST", "/api/idea/preview", &[], &body).0, 401);
    assert_eq!(raw_http(port, "POST", "/api/idea/preview", &[("Cookie", cookie.as_str())], &body).0, 401);
    assert_eq!(
        raw_http(
            port, "POST", "/api/idea/preview",
            &[("X-ReSearch-Token", tok.as_str()), ("Origin", "http://evil.invalid")], &body,
        ).0,
        403
    );
    assert_eq!(raw_http(port, "POST", "/api/sessions/s-any/idea/note", &[], b"{}").0, 401);

    // Rule P6: 2 arXiv queries through run_first_search_with_gate obey the 3s F3 rate gate.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let bg = thread::spawn(move || {
        for _ in 0..2 {
            let (mut s, _) = listener.accept().unwrap();
            drain_http_request(&mut s);
            let atom = synthetic_arxiv_atom();
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/atom+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                atom.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            let _ = s.write_all(&atom);
        }
    });

    let ledger = Ledger::open(&data_dir).unwrap();
    let stmt = build_idea_statement(1, SAMPLE_IDEA, &[], &[]).unwrap();
    append_explore_session_start(&ledger, "s-gate-test", IdeaPreset::Standard, false, &stmt).unwrap();
    let extracted = extract_idea_terms(&stmt.text);
    let arxiv_two: Vec<_> = preview_first_queries(&extracted, &[])
        .into_iter().filter(|q| q.provider.as_str() == "arxiv").take(2).collect();
    assert_eq!(arxiv_two.len(), 2);

    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(Arc::clone(&clock) as Arc<dyn RateClock>);
    let endpoints = LiteratureEndpoints::loopback_mock(&format!("http://{addr}")).unwrap();
    let cands = run_first_search_with_gate(
        &data_dir, &endpoints, &ledger, "s-gate-test", &stmt, &arxiv_two, false, &gate, None, |_, _| {},
    ).unwrap();
    bg.join().unwrap();
    assert!(!cands.is_empty());
    assert!(
        clock.total_slept() >= Duration::from_secs(3),
        "Rule P6: 2 arXiv requests must be spaced by >= 3s via F3 NetRateGate, got {:?}",
        clock.recorded_sleeps()
    );
}
