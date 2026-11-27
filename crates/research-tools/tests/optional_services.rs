//! Loopback mock + `FakeRateClock` integration tests for optional literature services
//! (package X19 Part 1: OpenAlex allowlisted through F3 gate; TheoremSearch stays disabled).
//!
//! Every fixture record below is invented purely for testing (no third-party paper text).

use ledger::Ledger;
use research_tools::http_client::{
    allowlisted_https_hosts, is_allowlisted_https_host, USER_AGENT,
};
use research_tools::library::Library;
use research_tools::lit_abstracts::{locate_abstract_quote, locate_abstract_quote_in_blob};
use research_tools::lit_openalex::reconstruct_abstract_inverted_index;
use research_tools::lit_providers::{
    run_query_and_record_with_gate, run_query_with_gate, CitationEdge, LitProvider, LitQuery,
    ProviderConfig, QueryRelation, QueryStatus, LITERATURE_FETCHED_EVENT,
};
use research_tools::net_rate::{resolve_bucket, FakeRateClock, NetRateGate, RateBucket, Service};
use research_tools::paper_key::PaperKey;
use research_tools::LiteratureEndpoints;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
struct CapturedReq {
    path: String,
    headers: HashMap<String, String>,
}

#[derive(Clone)]
struct MockReply {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl MockReply {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: "200 OK",
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.into(),
        }
    }
    fn status(status: &'static str, extra: Vec<(&str, &str)>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: extra
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            body: body.into(),
        }
    }
}

struct MockServer {
    port: u16,
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedReq>>>,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn spawn<F>(max_reqs: usize, handler: F) -> Self
    where
        F: Fn(&CapturedReq) -> MockReply + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let cap_bg = Arc::clone(&captured);
        let hits_bg = Arc::clone(&hits);
        let stop_bg = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            for _ in 0..max_reqs {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                if stop_bg.load(Ordering::Acquire) {
                    break;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let req = read_http_request(&mut stream);
                hits_bg.fetch_add(1, Ordering::AcqRel);
                cap_bg.lock().unwrap().push(req.clone());
                let reply = handler(&req);
                let mut raw = format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len()
                );
                for (k, v) in &reply.headers {
                    raw.push_str(&format!("{k}: {v}\r\n"));
                }
                raw.push_str("\r\n");
                let _ = stream.write_all(raw.as_bytes());
                let _ = stream.write_all(&reply.body);
                let _ = stream.flush();
            }
        });
        Self {
            port,
            base_url: format!("http://127.0.0.1:{port}"),
            captured,
            hits,
            stop,
            handle: Some(handle),
        }
    }

    fn captured(&self) -> Vec<CapturedReq> {
        self.captured.lock().unwrap().clone()
    }

    fn hit_count(&self) -> usize {
        self.hits.load(Ordering::Acquire)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> CapturedReq {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 32_768 {
                    break;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    CapturedReq { path, headers }
}

fn fake_gate() -> (Arc<FakeRateClock>, NetRateGate) {
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(Arc::clone(&clock) as Arc<dyn research_tools::net_rate::RateClock>);
    (clock, gate)
}

const OPENALEX_SEARCH_JSON: &str = r#"{
  "meta": {"count": 5, "page": 1, "per_page": 2},
  "results": [
    {
      "id": "https://openalex.org/W9100001001",
      "doi": "https://doi.org/10.9999/oa.toy.2025.01",
      "ids": {
        "openalex": "https://openalex.org/W9100001001",
        "doi": "https://doi.org/10.9999/oa.toy.2025.01"
      },
      "title": "Synthetic   Plaquette   Spectrum",
      "publication_year": 2025,
      "cited_by_count": 14,
      "is_retracted": false,
      "authorships": [
        {"author": {"display_name": "Ada Synthetic"}},
        {"raw_author_name": "Bram Fixture"}
      ],
      "abstract_inverted_index": {
        "chains.": [4],
        "bounds": [2],
        "We": [0],
        "on": [3],
        "prove": [1]
      },
      "primary_location": {"landing_page_url": "https://example.org/work/1"}
    },
    {
      "id": "https://openalex.org/W9100001002",
      "display_name": "OpenAlex Only Identifier Work",
      "publication_year": 2024,
      "cited_by_count": 3,
      "is_retracted": true,
      "authorships": [{"author": {"display_name": "Clara Mock"}}],
      "abstract_inverted_index": {
        "sparse": [100000000],
        "Order": [0],
        "preserved": [2]
      }
    }
  ]
}"#;

const OPENALEX_CITES_JSON: &str = r#"{
  "meta": {"count": 1},
  "results": [
    {
      "id": "https://openalex.org/W9100002001",
      "title": "Cited Synthetic Ancestor",
      "publication_year": 2022,
      "authorships": [{"author": {"display_name": "Dora Ancestor"}}]
    }
  ]
}"#;

const OPENALEX_CITED_BY_JSON: &str = r#"{
  "meta": {"count": 1},
  "results": [
    {
      "id": "https://openalex.org/W9100003001",
      "doi": "https://doi.org/10.9999/oa.followup.2026.01",
      "title": "Synthetic Followup Study",
      "publication_year": 2026,
      "authorships": [{"author": {"display_name": "Eli Followup"}}]
    }
  ]
}"#;

const CROSSREF_MINI_JSON: &str = r#"{
  "message": {
    "total-results": 1,
    "items": [
      {"DOI": "10.9999/cr.peer.2025.01", "title": ["Peer Crossref Fixture"]}
    ]
  }
}"#;

#[test]
fn five_field_literature_endpoints_serde_backward_compat_and_allowlist() {
    let legacy_five_field_json = r#"{
      "arxiv_api_base": "https://export.arxiv.org",
      "arxiv_eprint_base": "https://arxiv.org",
      "inspire_api_base": "https://inspirehep.net",
      "crossref_api_base": "https://api.crossref.org",
      "semantic_scholar_api_base": "https://api.semanticscholar.org"
    }"#;
    let parsed: LiteratureEndpoints =
        serde_json::from_str(legacy_five_field_json).expect("5-field JSON must deserialize");
    assert_eq!(parsed.openalex_api_base, "https://api.openalex.org");
    assert_eq!(parsed, LiteratureEndpoints::default());

    let loopback = LiteratureEndpoints::loopback_mock("http://127.0.0.1:18777/").unwrap();
    assert_eq!(loopback.openalex_api_base, "http://127.0.0.1:18777/openalex");
    assert_eq!(loopback.crossref_api_base, "http://127.0.0.1:18777");

    // OpenAlex allowlisted; TheoremSearch remains excluded.
    assert!(is_allowlisted_https_host("api.openalex.org"));
    assert!(is_allowlisted_https_host("API.OPENALEX.ORG"));
    assert!(allowlisted_https_hosts().contains(&"api.openalex.org"));
    assert!(!is_allowlisted_https_host("theoremsearch.com"));
    assert!(!is_allowlisted_https_host("api.theoremsearch.com"));
    assert!(LitProvider::OpenAlex.is_allowlisted());
    assert_eq!(LitProvider::OpenAlex.net_service(), Some(Service::OpenAlex));
    assert!(!LitProvider::TheoremSearch.is_allowlisted());
    assert_eq!(LitProvider::TheoremSearch.net_service(), None);
}

#[test]
fn openalex_search_citations_inverted_index_verbatim_false_and_ledger_rebuild() {
    let srv = MockServer::spawn(4, |req| {
        if req.path.contains("cited%5Fby%3A") || req.path.contains("cited_by:") {
            MockReply::ok(OPENALEX_CITES_JSON)
        } else if req.path.contains("cites%3A") || req.path.contains("cites:") {
            MockReply::ok(OPENALEX_CITED_BY_JSON)
        } else if req.path.starts_with("/openalex/works") {
            MockReply::ok(OPENALEX_SEARCH_JSON)
        } else {
            MockReply::status("404 Not Found", vec![], "missing")
        }
    });
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (_, gate) = fake_gate();
    let fake_key = "fake-oa-search-key-4421";
    let cfg = ProviderConfig {
        openalex_api_key: Some(fake_key.into()),
        ..ProviderConfig::default()
    };
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let session = "sess-x19-openalex";

    // 1. Free-text search + ledger record + Rule C3 (`verbatim == false`).
    let q_search = LitQuery::search(LitProvider::OpenAlex, "plaquette spectrum")
        .with_paging(2, 0)
        .with_tracking(Some("q-oa-1"), Some("run-x19"));
    let res = run_query_and_record_with_gate(&ep, &ledger, session, &q_search, &cfg, &gate)
        .expect("record search");
    assert_eq!(res.status, QueryStatus::Ok);
    assert_eq!(res.papers.len(), 2);
    assert_eq!(res.total_results, Some(5));
    assert!(res.has_more);
    assert!(!res.request_url.as_deref().unwrap().contains(fake_key));

    let p0 = &res.papers[0];
    assert_eq!(p0.paper_ref.key.as_str(), "doi:10.9999/oa.toy.2025.01");
    assert_eq!(p0.title, "Synthetic Plaquette Spectrum");
    assert_eq!(p0.authors, vec!["Ada Synthetic", "Bram Fixture"]);
    assert_eq!(p0.year, Some(2025));
    assert_eq!(p0.citation_count, Some(14));
    assert!(!p0.withdrawn_or_retracted);
    assert_eq!(p0.abstract_text.as_deref(), Some("We prove bounds on chains."));
    let aref0 = p0.abstract_ref.as_ref().expect("abstract_ref");
    assert!(!aref0.verbatim, "OpenAlex inverted-index abstract MUST have verbatim = false");
    assert_eq!(aref0.provider, "openalex");
    assert_eq!(aref0.raw_blob, res.raw_blob.clone().unwrap());

    // Rule C3: rebuilt OpenAlex abstracts can NEVER become `evidence.abstract`.
    let raw_bytes = ledger.get_blob(res.raw_blob.as_deref().unwrap()).unwrap();
    assert!(locate_abstract_quote(p0.abstract_text.as_deref().unwrap(), "prove bounds", aref0.verbatim).is_err());
    assert!(locate_abstract_quote_in_blob(LitProvider::OpenAlex, &raw_bytes, &p0.paper_ref.key, "prove bounds").is_err());

    let p1 = &res.papers[1];
    assert_eq!(p1.paper_ref.key.as_str(), "openalex:W9100001002");
    assert!(p1.withdrawn_or_retracted);
    assert_eq!(p1.abstract_text.as_deref(), Some("Order preserved sparse"));
    assert!(!p1.abstract_ref.as_ref().unwrap().verbatim);

    // 2. Explicit `filter:` query.
    let q_filter = LitQuery::search(LitProvider::OpenAlex, "filter:publication_year:2025").with_paging(2, 0);
    let r_filter = run_query_with_gate(&ep, &q_filter, &cfg, &gate);
    assert_eq!(r_filter.status, QueryStatus::Ok);

    // 3. `QueryRelation::Cites` (openalex:W...) & `QueryRelation::CitedBy` (doi:...).
    let src_oa = PaperKey::parse("openalex:W9100001001").unwrap();
    let r_cites = run_query_with_gate(
        &ep,
        &LitQuery::citation_relation(LitProvider::OpenAlex, QueryRelation::Cites { key: src_oa.clone() }),
        &cfg,
        &gate,
    );
    assert_eq!(r_cites.status, QueryStatus::Ok);
    assert_eq!(
        r_cites.edges,
        vec![CitationEdge {
            from: src_oa,
            to: PaperKey::parse("openalex:W9100002001").unwrap(),
            provider: LitProvider::OpenAlex,
        }]
    );

    let src_doi = PaperKey::parse("doi:10.9999/oa.toy.2025.01").unwrap();
    let r_cited_by = run_query_with_gate(
        &ep,
        &LitQuery::citation_relation(LitProvider::OpenAlex, QueryRelation::CitedBy { key: src_doi.clone() }),
        &cfg,
        &gate,
    );
    assert_eq!(r_cited_by.status, QueryStatus::Ok);
    assert_eq!(
        r_cited_by.edges,
        vec![CitationEdge {
            from: PaperKey::parse("doi:10.9999/oa.followup.2026.01").unwrap(),
            to: src_doi,
            provider: LitProvider::OpenAlex,
        }]
    );

    // Unsupported key kind returns `Unsupported` without touching the network.
    let r_unsup = run_query_with_gate(
        &ep,
        &LitQuery::citation_relation(
            LitProvider::OpenAlex,
            QueryRelation::Cites {
                key: PaperKey::parse("arxiv:2501.00011").unwrap(),
            },
        ),
        &cfg,
        &gate,
    );
    assert_eq!(r_unsup.status, QueryStatus::Unsupported);
    assert_eq!(srv.hit_count(), 4);

    // Verify User-Agent stays generic (Rule P8) and ledger never contains API key.
    for req in srv.captured() {
        assert_eq!(req.headers.get("user-agent").map(String::as_str), Some(USER_AGENT));
        assert!(!req.path.contains(fake_key));
    }
    ledger.verify_session(session).expect("ledger hash chain valid");
    let events = ledger.read_session(session).unwrap();
    let lit_ev = events.iter().find(|e| e.kind == LITERATURE_FETCHED_EVENT).unwrap();
    assert!(!lit_ev.payload.to_string().contains(fake_key));

    // Library rebuild ingests OpenAlex-only `openalex:W...` records cleanly.
    let mut lib = Library::open(tmp.path()).unwrap();
    lib.rebuild().unwrap();
    assert!(lib.get_paper(&PaperKey::parse("openalex:W9100001002").unwrap()).unwrap().is_some());

    // Direct sanity check on inverted-index edge cases (empty / malformed).
    assert_eq!(reconstruct_abstract_inverted_index(&json!({})), None);
    assert_eq!(reconstruct_abstract_inverted_index(&json!({"": [0], "bad": "not-arr"})), None);
}

#[test]
fn openalex_cross_origin_redirect_strips_bearer_header_and_redacts_errors() {
    let fake_key = "fake-oa-redirect-secret-5591";
    let srv_b = MockServer::spawn(1, |_| MockReply::ok(OPENALEX_SEARCH_JSON));
    let redirect_target = format!("{}/openalex/works?redirected=1", srv_b.base_url);
    let srv_a = MockServer::spawn(5, move |req| {
        if req.path.contains("doredirect") {
            MockReply::status("302 Found", vec![("Location", &redirect_target)], "")
        } else if req.path.contains("echoerror") {
            MockReply::status("401 Unauthorized", vec![], format!("bad key {fake_key}"))
        } else if req.path.contains("ratelimited") {
            MockReply::status("429 Too Many Requests", vec![("Retry-After", "7")], "slow down")
        } else if req.path.starts_with("/openalex/works") {
            MockReply::ok(OPENALEX_SEARCH_JSON)
        } else {
            MockReply::ok(CROSSREF_MINI_JSON)
        }
    });

    let mut ep = LiteratureEndpoints::loopback_mock(&srv_a.base_url).unwrap();
    // Permit `srv_b`'s loopback origin on another endpoint slot so cross-origin redirect is followed.
    ep.arxiv_eprint_base = srv_b.base_url.clone();

    let (_, gate) = fake_gate();
    let cfg = ProviderConfig {
        openalex_api_key: Some(fake_key.into()),
        ..ProviderConfig::default()
    };

    let r_direct = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, "direct"), &cfg, &gate);
    assert_eq!(r_direct.status, QueryStatus::Ok);
    assert!(!r_direct.request_url.as_deref().unwrap().contains(fake_key));

    // Another provider (Crossref) must NOT receive OpenAlex's Authorization header.
    let r_cr = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Crossref, "peer"), &cfg, &gate);
    assert_eq!(r_cr.status, QueryStatus::Ok);

    // Cross-origin redirect must strip `Authorization: Bearer <key>` (Rule P8).
    let r_redir = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, "doredirect"), &cfg, &gate);
    assert_eq!(r_redir.status, QueryStatus::Ok);

    // Provider error body echoing the key must NOT leak into `LitQueryResult.error`.
    let r_err = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, "echoerror"), &cfg, &gate);
    assert_eq!(r_err.status, QueryStatus::Failed);
    assert_eq!(r_err.http_status, Some(401));
    assert!(!r_err.error.as_deref().unwrap().contains(fake_key));

    // HTTP 429 parses `Retry-After` and returns `QueryStatus::RateLimited`.
    let r_429 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, "ratelimited"), &cfg, &gate);
    assert_eq!(r_429.status, QueryStatus::RateLimited);
    assert_eq!(r_429.http_status, Some(429));
    assert_eq!(r_429.retry_after_secs, Some(7));

    let a_reqs = srv_a.captured();
    assert_eq!(
        a_reqs[0].headers.get("authorization").map(String::as_str),
        Some("Bearer fake-oa-redirect-secret-5591")
    );
    assert!(!a_reqs[1].headers.contains_key("authorization"));
    assert_eq!(
        a_reqs[2].headers.get("authorization").map(String::as_str),
        Some("Bearer fake-oa-redirect-secret-5591")
    );
    let b_reqs = srv_b.captured();
    assert_eq!(b_reqs.len(), 1);
    assert!(
        !b_reqs[0].headers.contains_key("authorization"),
        "Authorization header leaked across cross-origin redirect!"
    );
}

#[test]
fn fake_rate_clock_paces_openalex_separately_from_crossref() {
    let srv = MockServer::spawn(13, |req| {
        if req.path.starts_with("/openalex/") {
            MockReply::ok(OPENALEX_SEARCH_JSON)
        } else if req.path.starts_with("/works") {
            MockReply::ok(CROSSREF_MINI_JSON)
        } else {
            MockReply::status("404 Not Found", vec![], "missing")
        }
    });
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (clock, gate) = fake_gate();
    let cfg = ProviderConfig::default();

    // Verify shared-port loopback bucket resolution puts `/openalex/works` on OpenAlex
    // and `/works` on Crossref.
    let oa_uri: ureq::http::Uri = format!("{}/openalex/works?search=a", srv.base_url).parse().unwrap();
    let cr_uri: ureq::http::Uri = format!("{}/works?query=a", srv.base_url).parse().unwrap();
    assert_eq!(resolve_bucket(&oa_uri, &ep, None), RateBucket::Service(Service::OpenAlex));
    assert_eq!(resolve_bucket(&cr_uri, &ep, None), RateBucket::Service(Service::Crossref));

    // Service::OpenAlex limit is 10 req / 1 s: first 10 calls incur zero wait.
    for i in 0..10 {
        let r = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, format!("q{i}")), &cfg, &gate);
        assert_eq!(r.status, QueryStatus::Ok);
    }
    assert_eq!(clock.total_slept(), Duration::ZERO);

    // 11th OpenAlex call in the same 1 s window sleeps 1 s on FakeRateClock.
    let r11 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, "q10"), &cfg, &gate);
    assert_eq!(r11.status, QueryStatus::Ok);
    assert_eq!(clock.total_slept(), Duration::from_secs(1));

    // Crossref has its own independent 1 req / 1 s bucket.
    let cr1 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Crossref, "c1"), &cfg, &gate);
    assert_eq!(cr1.status, QueryStatus::Ok);
    assert_eq!(clock.total_slept(), Duration::from_secs(1));
    let cr2 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Crossref, "c2"), &cfg, &gate);
    assert_eq!(cr2.status, QueryStatus::Ok);
    assert_eq!(clock.total_slept(), Duration::from_secs(2));
}

#[test]
fn theoremsearch_stays_disabled_with_zero_http_calls() {
    let srv = MockServer::spawn(1, |_| MockReply::ok("{}"));
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (clock, gate) = fake_gate();
    let cfg = ProviderConfig::default();

    let queries = [
        LitQuery::search(LitProvider::TheoremSearch, "compactness"),
        LitQuery::citation_relation(
            LitProvider::TheoremSearch,
            QueryRelation::Cites {
                key: PaperKey::parse("openalex:W9100001001").unwrap(),
            },
        ),
        LitQuery::citation_relation(
            LitProvider::TheoremSearch,
            QueryRelation::CitedBy {
                key: PaperKey::parse("doi:10.9999/oa.toy.2025.01").unwrap(),
            },
        ),
    ];
    for q in &queries {
        let res = run_query_with_gate(&ep, q, &cfg, &gate);
        assert_eq!(res.status, QueryStatus::Disabled);
        assert!(res.papers.is_empty());
        assert!(res.request_url.is_none());
    }
    assert_eq!(srv.hit_count(), 0, "TheoremSearch must make 0 HTTP calls");
    assert_eq!(clock.total_slept(), Duration::ZERO);
}
