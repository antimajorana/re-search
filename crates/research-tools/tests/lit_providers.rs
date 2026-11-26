//! Loopback mock integration tests for F11 (`lit_providers` and `lit_abstracts`).
//!
//! Every fixture paper below is short and invented purely for testing (no real titles,
//! authors, or abstracts).

use ledger::Ledger;
use research_tools::http_client::USER_AGENT;
use research_tools::library::Library;
use research_tools::lit_abstracts::{locate_abstract_quote, locate_abstract_quote_in_blob};
use research_tools::lit_providers::{
    run_query_and_record_with_gate, run_query_with_gate, CitationEdge, LitProvider, LitQuery,
    ProviderConfig, QueryRelation, QueryStatus, LITERATURE_FETCHED_EVENT,
};
use research_tools::net_rate::{FakeRateClock, NetRateGate, RateBucket, Service};
use research_tools::paper_key::PaperKey;
use research_tools::LiteratureEndpoints;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
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
    delay: Duration,
}

impl MockReply {
    fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: "200 OK",
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.into(),
            delay: Duration::ZERO,
        }
    }
    fn status(status: &'static str, extra: Vec<(&str, &str)>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: extra.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            body: body.into(),
            delay: Duration::ZERO,
        }
    }
}

struct MockServer {
    port: u16,
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedReq>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn spawn<F>(expected_reqs: usize, handler: F) -> Self
    where
        F: Fn(&CapturedReq) -> MockReply + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let cap_bg = Arc::clone(&captured);
        let stop_bg = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            for _ in 0..expected_reqs {
                let Ok((mut stream, _)) = listener.accept() else { break };
                if stop_bg.load(Ordering::Acquire) {
                    break;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let req = read_http_request(&mut stream);
                cap_bg.lock().unwrap().push(req.clone());
                let reply = handler(&req);
                if !reply.delay.is_zero() {
                    thread::sleep(reply.delay);
                }
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
            stop,
            handle: Some(handle),
        }
    }

    fn captured(&self) -> Vec<CapturedReq> {
        self.captured.lock().unwrap().clone()
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

fn read_http_request(stream: &mut std::net::TcpStream) -> CapturedReq {
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

const ARXIV_FEED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <opensearch:totalResults>3</opensearch:totalResults><opensearch:startIndex>0</opensearch:startIndex><opensearch:itemsPerPage>2</opensearch:itemsPerPage>
  <entry>
    <id>http://arxiv.org/abs/2501.00011v2</id><title>Synthetic Hexagonal   Dimer Modes</title>
    <summary>We prove an exact gap bound for hexagonal dimer chains.</summary>
    <published>2025-01-05T12:00:00Z</published><updated>2025-01-09T12:00:00Z</updated>
    <author><name>Ada Example</name></author><author><name>Bram Testman</name></author>
    <arxiv:doi>10.9999/hex.2025.11</arxiv:doi><arxiv:primary_category term="cond-mat.str-el"/>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2501.00012v1</id><title>Missing Optional Fields Paper</title>
    <summary></summary><arxiv:comment>Withdrawn by author due to typo</arxiv:comment>
  </entry>
</feed>"#;

const INSPIRE_SEARCH_JSON: &str = r#"{"hits":{"total":4,"hits":[
  {"id":"9100001","metadata":{"control_number":9100001,"texkeys":["Example:2024abc"],
    "titles":[{"title":"Fictional Gauge Duality on Rings"}],"authors":[{"full_name":"Example, Clara"}],
    "preprint_date":"2024-06-15","citation_count":17,"dois":[{"value":"10.9999/ring.2024.01"}],
    "arxiv_eprints":[{"value":"2406.00101"}],
    "abstracts":[{"value":"A finite ring duality maps twist defects to boundary shifts."}]}},
  {"id":"9100002","metadata":{"titles":[{"title":"Minimal Inspire Record"}],"withdrawn":true}}
]}}"#;

const S2_SEARCH_JSON: &str = r#"{"total":5,"offset":0,"next":2,"data":[
  {"paperId":"0123456789abcdef0123456789abcdef01234567","title":"Synthetic Krylov Chain Scaling",
   "authors":[{"name":"Dana Mock"}],"year":2025,
   "abstract":"Lanczos coefficients grow linearly in our toy monitored chain.","citationCount":9,
   "externalIds":{"DOI":"10.9999/krylov.2025.9","ArXiv":"2502.00202"},
   "url":"https://api.semanticscholar.org/paper/0123456789abcdef0123456789abcdef01234567"},
  {"paperId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","title":"S2 Only Identifier Paper","authors":[]}
]}"#;

const CROSSREF_SEARCH_JSON: &str = r#"{"message":{"total-results":10,"items":[
  {"DOI":"10.9999/crossref.2023.5","title":["Toy Plaquette Entanglement Bounds"],
   "author":[{"given":"Eli","family":"Placeholder"}],"issued":{"date-parts":[[2023,8,1]]},
   "is-referenced-by-count":21,
   "abstract":"<jats:p>We show <jats:italic>area-law</jats:italic> scaling for &alpha; &lt; 1.</jats:p>",
   "URL":"https://doi.org/10.9999/crossref.2023.5"},
  {"DOI":"10.9999/crossref.retracted.6","title":["Retracted Toy Note"],
   "relation":{"is-retracted-by":[{"id":"10.9999/notice.6"}]}}
]}}"#;

#[test]
fn all_four_services_parse_fields_abstracts_paging_and_pass_rate_gate() {
    let srv = MockServer::spawn(5, |req| {
        if req.path.starts_with("/api/query") {
            MockReply::ok("application/atom+xml", ARXIV_FEED_XML)
        } else if req.path.starts_with("/api/literature") {
            MockReply::ok("application/json", INSPIRE_SEARCH_JSON)
        } else if req.path.starts_with("/graph/v1/paper/search") {
            MockReply::ok("application/json", S2_SEARCH_JSON)
        } else if req.path.starts_with("/works") {
            MockReply::ok("application/json", CROSSREF_SEARCH_JSON)
        } else {
            MockReply::status("404 Not Found", vec![], "missing")
        }
    });
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (clock, gate) = fake_gate();
    let cfg = ProviderConfig::default();

    // 1. arXiv (plus a second call to verify fake-clock 3s spacing on F3 gate)
    let r_ax = run_query_with_gate(
        &ep,
        &LitQuery::search(LitProvider::Arxiv, "hexagonal dimer").with_paging(2, 0),
        &cfg,
        &gate,
    );
    assert_eq!(r_ax.status, QueryStatus::Ok);
    assert_eq!(r_ax.papers.len(), 2);
    assert_eq!(r_ax.total_results, Some(3));
    assert!(r_ax.has_more);
    assert_eq!(r_ax.papers[0].paper_ref.key.as_str(), "arxiv:2501.00011");
    assert_eq!(r_ax.papers[0].paper_ref.version.as_deref(), Some("v2"));
    assert_eq!(r_ax.papers[0].paper_ref.ids.doi.as_deref(), Some("10.9999/hex.2025.11"));
    assert_eq!(r_ax.papers[0].title, "Synthetic Hexagonal Dimer Modes");
    assert!(r_ax.papers[0].abstract_ref.as_ref().unwrap().verbatim);
    assert_eq!(r_ax.papers[0].abstract_ref.as_ref().unwrap().raw_blob, r_ax.raw_blob.clone().unwrap());
    assert!(r_ax.papers[1].abstract_text.is_none());
    assert!(r_ax.papers[1].withdrawn_or_retracted);

    let r_ax2 = run_query_with_gate(
        &ep,
        &LitQuery::search(LitProvider::Arxiv, "2501.00011v2").with_paging(2, 0),
        &cfg,
        &gate,
    );
    assert_eq!(r_ax2.status, QueryStatus::Ok);
    assert_eq!(clock.total_slept(), Duration::from_secs(3));
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Arxiv)), 2);

    // 2. INSPIRE-HEP (with paging offset=2, max=2 -> page=2)
    let r_ins = run_query_with_gate(
        &ep,
        &LitQuery::search(LitProvider::Inspire, "gauge duality").with_paging(2, 2),
        &cfg,
        &gate,
    );
    assert_eq!(r_ins.status, QueryStatus::Ok);
    assert_eq!(r_ins.papers.len(), 2);
    assert_eq!(r_ins.papers[0].paper_ref.key.as_str(), "arxiv:2406.00101");
    assert_eq!(r_ins.papers[0].paper_ref.ids.inspire.as_deref(), Some("9100001"));
    assert!(r_ins.papers[0].abstract_text.as_deref().unwrap().contains("twist defects"));
    assert_eq!(r_ins.papers[1].paper_ref.key.as_str(), "inspire:9100002");
    assert!(r_ins.papers[1].withdrawn_or_retracted);
    assert!(!r_ins.has_more); // offset 2 + 2 returned == total 4

    // 3. Semantic Scholar
    let r_s2 = run_query_with_gate(
        &ep,
        &LitQuery::search(LitProvider::SemanticScholar, "krylov").with_paging(2, 0),
        &cfg,
        &gate,
    );
    assert_eq!(r_s2.status, QueryStatus::Ok);
    assert_eq!(r_s2.papers.len(), 2);
    assert!(r_s2.has_more);
    assert_eq!(r_s2.papers[0].paper_ref.key.as_str(), "arxiv:2502.00202");
    assert_eq!(r_s2.papers[0].paper_ref.ids.s2.as_deref(), Some("0123456789abcdef0123456789abcdef01234567"));
    assert_eq!(r_s2.papers[1].paper_ref.key.as_str(), "s2:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(r_s2.papers[1].abstract_text.is_none());

    // 4. Crossref (JATS stripped abstract + retraction flag)
    let r_cr = run_query_with_gate(
        &ep,
        &LitQuery::search(LitProvider::Crossref, "plaquette").with_paging(2, 0),
        &cfg,
        &gate,
    );
    assert_eq!(r_cr.status, QueryStatus::Ok);
    assert_eq!(r_cr.papers.len(), 2);
    assert!(r_cr.has_more);
    assert_eq!(r_cr.papers[0].paper_ref.key.as_str(), "doi:10.9999/crossref.2023.5");
    assert_eq!(
        r_cr.papers[0].abstract_text.as_deref(),
        Some("We show area-law scaling for \u{03B1} < 1.")
    );
    assert!(r_cr.papers[1].withdrawn_or_retracted);

    // Every request carried generic User-Agent (rule P8) and hit its F3 service bucket.
    let reqs = srv.captured();
    assert_eq!(reqs.len(), 5);
    for req in &reqs {
        assert_eq!(req.headers.get("user-agent").map(String::as_str), Some(USER_AGENT));
    }
    assert!(reqs[2].path.contains("page=2"));
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Inspire)), 1);
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::SemanticScholar)), 1);
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Crossref)), 1);
}

#[test]
fn citation_relations_and_unsupported_and_disabled_openalex() {
    let srv = MockServer::spawn(5, |req| {
        if req.path.starts_with("/api/literature") {
            MockReply::ok("application/json", r#"{"hits":{"total":1,"hits":[{"id":"88001","metadata":{"titles":[{"title":"Target Inspire Paper"}],"dois":[{"value":"10.9999/t.1"}]}}]}}"#)
        } else if req.path.contains("/references") {
            MockReply::ok("application/json", r#"{"offset":0,"data":[{"citedPaper":{"paperId":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","title":"Cited Ref"}},{"citedPaper":{"paperId":null,"title":"Unresolved"}}]}"#)
        } else if req.path.contains("/citations") {
            MockReply::ok("application/json", r#"{"offset":0,"data":[{"citingPaper":{"paperId":"cccccccccccccccccccccccccccccccccccccccc","title":"Citing Work","externalIds":{"ArXiv":"2503.00303"}}}]}"#)
        } else if req.path.starts_with("/works/") {
            MockReply::ok("application/json", r#"{"message":{"DOI":"10.9999/origin.1","reference":[{"DOI":"10.9999/ref.1","article-title":"Ref One","author":"A. One","year":"2020"},{"unstructured":"No DOI ref"},{"DOI":"10.9999/ref.2","volume-title":"Ref Two","year":"2021"}]}}"#)
        } else {
            MockReply::status("404 Not Found", vec![], "")
        }
    });
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (_, gate) = fake_gate();
    let cfg = ProviderConfig::default();

    let seed_ins = PaperKey::from_inspire("9100001").unwrap();
    let r_ic = run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::Inspire, QueryRelation::Cites { key: seed_ins.clone() }), &cfg, &gate);
    assert_eq!(r_ic.status, QueryStatus::Ok);
    assert_eq!(r_ic.edges, vec![CitationEdge { from: seed_ins.clone(), to: PaperKey::from_doi("10.9999/t.1").unwrap(), provider: LitProvider::Inspire }]);

    let r_ib = run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::Inspire, QueryRelation::CitedBy { key: seed_ins.clone() }), &cfg, &gate);
    assert_eq!(r_ib.status, QueryStatus::Ok);
    assert_eq!(r_ib.edges, vec![CitationEdge { from: PaperKey::from_doi("10.9999/t.1").unwrap(), to: seed_ins, provider: LitProvider::Inspire }]);

    let seed_ax = PaperKey::from_arxiv("2501.00011").unwrap();
    let r_s2_ref = run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::SemanticScholar, QueryRelation::Cites { key: seed_ax.clone() }), &cfg, &gate);
    assert_eq!(r_s2_ref.status, QueryStatus::Ok);
    assert_eq!(r_s2_ref.papers.len(), 1); // unresolved null paperId skipped cleanly
    assert_eq!(r_s2_ref.edges[0].from, seed_ax);
    assert_eq!(r_s2_ref.edges[0].to.as_str(), "s2:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    let r_s2_cit = run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::SemanticScholar, QueryRelation::CitedBy { key: seed_ax.clone() }), &cfg, &gate);
    assert_eq!(r_s2_cit.status, QueryStatus::Ok);
    assert_eq!(r_s2_cit.edges[0].from.as_str(), "arxiv:2503.00303");
    assert_eq!(r_s2_cit.edges[0].to, seed_ax);

    let seed_doi = PaperKey::from_doi("10.9999/origin.1").unwrap();
    let r_cr_ref = run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::Crossref, QueryRelation::Cites { key: seed_doi.clone() }).with_paging(1, 0), &cfg, &gate);
    assert_eq!(r_cr_ref.status, QueryStatus::Ok);
    assert_eq!(r_cr_ref.papers.len(), 1);
    assert_eq!(r_cr_ref.total_results, Some(2));
    assert!(r_cr_ref.has_more);
    assert_eq!(r_cr_ref.edges[0], CitationEdge { from: seed_doi.clone(), to: PaperKey::from_doi("10.9999/ref.1").unwrap(), provider: LitProvider::Crossref });

    // Unsupported relations and disabled OpenAlex make ZERO HTTP requests.
    assert_eq!(run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::Arxiv, QueryRelation::Cites { key: seed_ax }), &cfg, &gate).status, QueryStatus::Unsupported);
    assert_eq!(run_query_with_gate(&ep, &LitQuery::citation_relation(LitProvider::Crossref, QueryRelation::CitedBy { key: seed_doi }), &cfg, &gate).status, QueryStatus::Unsupported);
    let r_oa = run_query_with_gate(&ep, &LitQuery::search(LitProvider::OpenAlex, "spin"), &cfg, &gate);
    assert_eq!(r_oa.status, QueryStatus::Disabled);
    assert!(r_oa.error.as_deref().unwrap().contains("not in http_client.rs"));
    assert_eq!(srv.captured().len(), 5);
}

#[test]
fn handles_429_retry_after_variants_5xx_malformed_xxe_and_timeout() {
    let srv = MockServer::spawn(12, |req| {
        if req.path.contains("ratenum") {
            MockReply::status("429 Too Many Requests", vec![("Retry-After", "5")], "slow down")
        } else if req.path.contains("ratedate") {
            MockReply::status("429 Too Many Requests", vec![("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT")], "")
        } else if req.path.contains("ratebad") {
            MockReply::status("429 Too Many Requests", vec![("Retry-After", "soon")], "")
        } else if req.path.contains("ratenone") {
            MockReply::status("429 Too Many Requests", vec![], "")
        } else if req.path.contains("err503") {
            MockReply::status("503 Service Unavailable", vec![], "upstream overloaded")
        } else if req.path.contains("err500") {
            MockReply::status("500 Internal Server Error", vec![], "boom")
        } else if req.path.contains("badjson") {
            MockReply::ok("application/json", "{not valid json")
        } else if req.path.contains("badxml") {
            MockReply::ok("application/atom+xml", "<feed><entry><unclosed></feed>")
        } else if req.path.contains("xxexml") {
            MockReply::ok("application/atom+xml", r#"<?xml version="1.0"?><!DOCTYPE foo [<!ENTITY xxe "bad">]><feed/>"#)
        } else {
            let mut r = MockReply::ok("application/json", r#"{"hits":{"hits":[]}}"#);
            r.delay = Duration::from_millis(600);
            r
        }
    });
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (_, gate) = fake_gate();
    let cfg = ProviderConfig { semantic_scholar_api_key: None, request_timeout: Some(Duration::from_millis(180)) };

    let r1 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::SemanticScholar, "ratenum"), &cfg, &gate);
    assert_eq!(r1.status, QueryStatus::RateLimited);
    assert_eq!(r1.http_status, Some(429));
    assert_eq!(r1.retry_after_secs, Some(5));

    let r2 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::SemanticScholar, "ratedate"), &cfg, &gate);
    assert_eq!(r2.status, QueryStatus::RateLimited);
    assert_eq!(r2.retry_after_secs, Some(0)); // past HTTP-date saturates to 0

    let r3 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::SemanticScholar, "ratebad"), &cfg, &gate);
    assert_eq!(r3.status, QueryStatus::RateLimited);
    assert_eq!(r3.retry_after_secs, None);
    assert_eq!(r3.retry_after_raw.as_deref(), Some("soon"));

    let r4 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Crossref, "ratenone"), &cfg, &gate);
    assert_eq!(r4.status, QueryStatus::RateLimited);
    assert_eq!(r4.retry_after_secs, None);
    assert_eq!(r4.retry_after_raw, None);

    let r5 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Inspire, "err503"), &cfg, &gate);
    assert_eq!(r5.status, QueryStatus::Failed);
    assert_eq!(r5.http_status, Some(503));

    let r6 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Crossref, "err500"), &cfg, &gate);
    assert_eq!(r6.status, QueryStatus::Failed);
    assert_eq!(r6.http_status, Some(500));

    for prov in [LitProvider::Inspire, LitProvider::SemanticScholar, LitProvider::Crossref] {
        let rb = run_query_with_gate(&ep, &LitQuery::search(prov, "badjson"), &cfg, &gate);
        assert_eq!(rb.status, QueryStatus::Failed);
        assert!(rb.error.as_deref().unwrap().contains("invalid JSON"));
    }

    let r8 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Arxiv, "badxml"), &cfg, &gate);
    assert_eq!(r8.status, QueryStatus::Failed);
    assert!(r8.error.as_deref().unwrap().contains("invalid arXiv Atom XML"));

    let r9 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Arxiv, "xxexml"), &cfg, &gate);
    assert_eq!(r9.status, QueryStatus::Failed);
    assert!(r9.error.as_deref().unwrap().contains("XXE"));

    let r10 = run_query_with_gate(&ep, &LitQuery::search(LitProvider::Inspire, "slowtimeout"), &cfg, &gate);
    assert_eq!(r10.status, QueryStatus::Failed);
    assert!(r10.timed_out, "expected timed_out=true, got err={:?}", r10.error);
}

#[test]
fn s2_api_key_sent_only_to_s2_and_stripped_on_cross_origin_redirect() {
    let srv_b = MockServer::spawn(1, |_| MockReply::ok("application/json", S2_SEARCH_JSON));
    let redirect_target = format!("{}/graph/v1/paper/search?redirected=1", srv_b.base_url);
    let srv_a = MockServer::spawn(3, move |req| {
        if req.path.contains("doredirect") {
            MockReply::status("302 Found", vec![("Location", &redirect_target)], "")
        } else if req.path.starts_with("/graph/v1/") {
            MockReply::ok("application/json", S2_SEARCH_JSON)
        } else {
            MockReply::ok("application/json", CROSSREF_SEARCH_JSON)
        }
    });
    let mut ep_with_b = LiteratureEndpoints::loopback_mock(&srv_a.base_url).unwrap();
    ep_with_b.arxiv_eprint_base = srv_b.base_url.clone();

    let (_, gate) = fake_gate();
    let cfg = ProviderConfig {
        semantic_scholar_api_key: Some("fake-test-s2-key-0001".into()),
        request_timeout: None,
    };

    let r_s2 = run_query_with_gate(&ep_with_b, &LitQuery::search(LitProvider::SemanticScholar, "direct"), &cfg, &gate);
    assert_eq!(r_s2.status, QueryStatus::Ok);
    assert!(!r_s2.request_url.as_deref().unwrap().contains("fake-test-s2-key-0001"));

    let r_cr = run_query_with_gate(&ep_with_b, &LitQuery::search(LitProvider::Crossref, "other"), &cfg, &gate);
    assert_eq!(r_cr.status, QueryStatus::Ok);

    let r_redir = run_query_with_gate(&ep_with_b, &LitQuery::search(LitProvider::SemanticScholar, "doredirect"), &cfg, &gate);
    assert_eq!(r_redir.status, QueryStatus::Ok);

    let a_reqs = srv_a.captured();
    assert_eq!(a_reqs[0].headers.get("x-api-key").map(String::as_str), Some("fake-test-s2-key-0001"));
    assert!(!a_reqs[1].headers.contains_key("x-api-key"));
    assert_eq!(a_reqs[2].headers.get("x-api-key").map(String::as_str), Some("fake-test-s2-key-0001"));
    let b_reqs = srv_b.captured();
    assert!(!b_reqs[0].headers.contains_key("x-api-key"), "x-api-key leaked across cross-origin redirect!");
}

#[test]
fn literature_fetched_records_qid_run_id_verifies_ledger_and_rebuilds_library() {
    let srv = MockServer::spawn(4, |req| {
        if req.path.starts_with("/api/query") {
            MockReply::ok("application/atom+xml", ARXIV_FEED_XML)
        } else if req.path.starts_with("/api/literature") {
            MockReply::ok("application/json", INSPIRE_SEARCH_JSON)
        } else if req.path.starts_with("/graph/v1/paper/search") {
            MockReply::ok("application/json", S2_SEARCH_JSON)
        } else {
            MockReply::ok("application/json", CROSSREF_SEARCH_JSON)
        }
    });
    let ep = LiteratureEndpoints::loopback_mock(&srv.base_url).unwrap();
    let (_, gate) = fake_gate();
    let cfg = ProviderConfig::default();
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let session = "sess-f11-compat";

    let queries = [
        (LitProvider::Arxiv, "hexagonal dimer", "q-1"),
        (LitProvider::Inspire, "gauge duality", "q-2"),
        (LitProvider::SemanticScholar, "krylov", "q-3"),
        (LitProvider::Crossref, "plaquette", "q-4"),
    ];
    for (prov, text, qid) in queries {
        let q = LitQuery::search(prov, text).with_tracking(Some(qid), Some("run-f11-1"));
        let res = run_query_and_record_with_gate(&ep, &ledger, session, &q, &cfg, &gate).unwrap();
        assert_eq!(res.status, QueryStatus::Ok);
        let blob_hash = res.raw_blob.as_deref().unwrap();
        let raw_bytes = ledger.get_blob(blob_hash).unwrap();
        let first_key = &res.papers[0].paper_ref.key;
        let quote_snippet = &res.papers[0].abstract_text.as_deref().unwrap()[..20];
        let matched = locate_abstract_quote_in_blob(prov, &raw_bytes, first_key, quote_snippet)
            .expect("rule C3 quote from stored blob");
        assert!(matched.end > matched.start);
        assert!(locate_abstract_quote(res.papers[0].abstract_text.as_deref().unwrap(), quote_snippet, false).is_err());
    }

    ledger.verify_session(session).expect("ledger hash chain valid");
    let events = ledger.read_session(session).unwrap();
    let lit_events: Vec<_> = events.iter().filter(|e| e.kind == LITERATURE_FETCHED_EVENT).collect();
    assert_eq!(lit_events.len(), 4);
    assert_eq!(lit_events[0].payload["qid"], "q-1");
    assert_eq!(lit_events[0].payload["run_id"], "run-f11-1");
    assert!(lit_events[0].payload["papers"].is_array());
    assert!(lit_events[1].payload["hits"].is_array());
    assert!(lit_events[2].payload["hits"].is_array());
    assert!(lit_events[3].payload["hits"].is_array());

    // Verify backwards compatibility with F10's existing `Library::rebuild` (`library_rebuild_lit`).
    let mut lib = Library::open(tmp.path()).unwrap();
    let report = lib.rebuild().unwrap();
    assert_eq!(report.sessions_scanned, 1);
    assert!(report.papers >= 7, "expected papers from all 4 providers, got {}", report.papers);
    for key_str in [
        "arxiv:2501.00011",
        "arxiv:2406.00101",
        "arxiv:2502.00202",
        "doi:10.9999/crossref.2023.5",
    ] {
        let key = PaperKey::parse(key_str).unwrap();
        assert!(lib.get_paper(&key).unwrap().is_some(), "library missing rebuilt paper {key_str}");
    }
}
