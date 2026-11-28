use super::*;
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn arxiv_spaces_requests_three_seconds_apart_on_fake_clock() {
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(clock.clone());

    let g1 = gate.acquire_grant(&RateBucket::Service(Service::Arxiv));
    let g2 = gate.acquire_grant(&RateBucket::Service(Service::Arxiv));
    let g3 = gate.acquire_grant(&RateBucket::Service(Service::Arxiv));

    assert_eq!(g1.waited, Duration::ZERO);
    assert_eq!(g1.granted_at, Duration::ZERO);
    assert_eq!(g2.waited, Duration::from_secs(3));
    assert_eq!(g2.granted_at, Duration::from_secs(3));
    assert_eq!(g3.waited, Duration::from_secs(3));
    assert_eq!(g3.granted_at, Duration::from_secs(6));
    assert_eq!(
        clock.recorded_sleeps(),
        vec![Duration::from_secs(3), Duration::from_secs(3)]
    );

    // After 3 s of natural elapsed time, the next request runs immediately.
    clock.advance(Duration::from_secs(3));
    let g4 = gate.acquire_grant(&RateBucket::Service(Service::Arxiv));
    assert_eq!(g4.waited, Duration::ZERO);
    assert_eq!(g4.granted_at, Duration::from_secs(9));
}

#[test]
fn inspire_allows_fifteen_per_five_seconds_then_waits() {
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(clock.clone());

    for i in 0..15 {
        let wait = gate.acquire_service(Service::Inspire);
        assert_eq!(wait, Duration::ZERO, "request {i} should pass immediately");
    }
    assert!(clock.recorded_sleeps().is_empty());

    // 16th request at t=0 must wait 5 seconds for the earliest slot to expire.
    let wait16 = gate.acquire_service(Service::Inspire);
    assert_eq!(wait16, Duration::from_secs(5));
    assert_eq!(clock.current(), Duration::from_secs(5));

    // At t=5s, the remaining 14 slots from t=0 have also expired, so 14 more pass immediately.
    for i in 17..=30 {
        let wait = gate.acquire_service(Service::Inspire);
        assert_eq!(wait, Duration::ZERO, "request {i} at t=5s should pass immediately");
    }
    // 31st request at t=5s hits the window (15 slots at t=5s) and waits until t=10s.
    let wait31 = gate.acquire_service(Service::Inspire);
    assert_eq!(wait31, Duration::from_secs(5));
}

#[test]
fn semantic_scholar_and_crossref_are_one_per_second_and_independent() {
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(clock.clone());

    assert_eq!(gate.acquire_service(Service::SemanticScholar), Duration::ZERO);
    assert_eq!(gate.acquire_service(Service::Crossref), Duration::ZERO);
    assert_eq!(gate.acquire_service(Service::Arxiv), Duration::ZERO);

    // Second S2 call waits 1 s; Crossref at t=1s then has 0 wait.
    assert_eq!(gate.acquire_service(Service::SemanticScholar), Duration::from_secs(1));
    assert_eq!(gate.acquire_service(Service::Crossref), Duration::ZERO);
    assert_eq!(gate.acquire_service(Service::Crossref), Duration::from_secs(1));
}

#[test]
fn export_arxiv_and_arxiv_org_share_one_bucket() {
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate::new(clock.clone());
    let ep = LiteratureEndpoints::default();

    let api_uri: Uri = "https://export.arxiv.org/api/query?id_list=2103.01234"
        .parse()
        .unwrap();
    let eprint_uri: Uri = "https://arxiv.org/e-print/2103.01234".parse().unwrap();

    let g1 = gate.acquire_for_url(&api_uri, &ep, None);
    let g2 = gate.acquire_for_url(&eprint_uri, &ep, None);

    assert_eq!(g1.bucket, RateBucket::Service(Service::Arxiv));
    assert_eq!(g2.bucket, RateBucket::Service(Service::Arxiv));
    assert_eq!(g1.waited, Duration::ZERO);
    assert_eq!(g2.waited, Duration::from_secs(3));
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Arxiv)), 2);
}

#[test]
fn loopback_mock_classifies_logical_services_separately() {
    let ep = LiteratureEndpoints::loopback_mock("http://127.0.0.1:18080").unwrap();

    let arxiv_q: Uri = "http://127.0.0.1:18080/api/query?id_list=2103.01234".parse().unwrap();
    let arxiv_ep: Uri = "http://127.0.0.1:18080/e-print/2103.01234".parse().unwrap();
    let arxiv_pdf: Uri = "http://127.0.0.1:18080/pdf/2103.01234.pdf".parse().unwrap();
    let inspire: Uri = "http://127.0.0.1:18080/api/literature?q=test".parse().unwrap();
    let crossref: Uri = "http://127.0.0.1:18080/works?query=test".parse().unwrap();
    let openalex: Uri = "http://127.0.0.1:18080/openalex/works?search=test".parse().unwrap();
    let s2: Uri = "http://127.0.0.1:18080/graph/v1/paper/search?query=test".parse().unwrap();
    let generic: Uri = "http://127.0.0.1:18080/ok0".parse().unwrap();

    assert_eq!(resolve_bucket(&arxiv_q, &ep, None), RateBucket::Service(Service::Arxiv));
    assert_eq!(resolve_bucket(&arxiv_ep, &ep, None), RateBucket::Service(Service::Arxiv));
    assert_eq!(resolve_bucket(&arxiv_pdf, &ep, None), RateBucket::Service(Service::Arxiv));
    assert_eq!(resolve_bucket(&inspire, &ep, None), RateBucket::Service(Service::Inspire));
    assert_eq!(resolve_bucket(&crossref, &ep, None), RateBucket::Service(Service::Crossref));
    assert_eq!(resolve_bucket(&openalex, &ep, None), RateBucket::Service(Service::OpenAlex));
    assert_eq!(resolve_bucket(&s2, &ep, None), RateBucket::Service(Service::SemanticScholar));
    assert_eq!(resolve_bucket(&generic, &ep, None), RateBucket::LoopbackGeneric(18080));
    // Explicit hint overrides generic loopback paths.
    assert_eq!(
        resolve_bucket(&generic, &ep, Some(Service::Arxiv)),
        RateBucket::Service(Service::Arxiv)
    );
}

#[test]
fn distinct_loopback_ports_classify_by_endpoint_without_path_match() {
    let ep = LiteratureEndpoints {
        arxiv_api_base: "http://127.0.0.1:19001".into(),
        arxiv_eprint_base: "http://127.0.0.1:19001".into(),
        inspire_api_base: "http://127.0.0.1:19002".into(),
        crossref_api_base: "http://127.0.0.1:19003".into(),
        semantic_scholar_api_base: "http://127.0.0.1:19004".into(),
        openalex_api_base: "http://127.0.0.1:19005".into(),
    };
    let custom_arxiv: Uri = "http://127.0.0.1:19001/custom".parse().unwrap();
    let custom_inspire: Uri = "http://127.0.0.1:19002/custom".parse().unwrap();
    let custom_openalex: Uri = "http://127.0.0.1:19005/custom".parse().unwrap();
    assert_eq!(resolve_bucket(&custom_arxiv, &ep, None), RateBucket::Service(Service::Arxiv));
    assert_eq!(resolve_bucket(&custom_inspire, &ep, None), RateBucket::Service(Service::Inspire));
    assert_eq!(resolve_bucket(&custom_openalex, &ep, None), RateBucket::Service(Service::OpenAlex));
}

#[test]
fn concurrent_callers_sharing_one_arxiv_gate_get_strictly_spaced_grants() {
    let clock = Arc::new(FakeRateClock::new());
    let gate = Arc::new(NetRateGate::new(clock));
    let threads = 6usize;
    let barrier = Arc::new(Barrier::new(threads));

    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let g = gate.clone();
        let b = barrier.clone();
        handles.push(thread::spawn(move || {
            b.wait();
            g.acquire_grant(&RateBucket::Service(Service::Arxiv))
        }));
    }

    let mut granted: Vec<Duration> = handles
        .into_iter()
        .map(|h| h.join().unwrap().granted_at)
        .collect();
    granted.sort();

    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Arxiv)), threads as u64);
    for pair in granted.windows(2) {
        assert!(
            pair[1] >= pair[0] + Duration::from_secs(3),
            "grants must be spaced by at least 3 s: {granted:?}"
        );
    }
}

#[test]
fn process_wide_gate_counts_but_does_not_wait_for_a_loopback_mock() {
    let endpoints = LiteratureEndpoints::loopback_mock("http://127.0.0.1:9").unwrap();
    let gate = NetRateGate::process_wide();
    let uri: ureq::http::Uri = "http://127.0.0.1:9/api/query?id_list=1".parse().unwrap();
    let started = std::time::Instant::now();
    for _ in 0..3 {
        let grant = gate.acquire_for_url(&uri, &endpoints, Some(Service::Arxiv));
        assert_eq!(grant.waited, Duration::ZERO);
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(gate.acquired_count(&RateBucket::Service(Service::Arxiv)), 3);
}

#[test]
fn process_wide_gate_still_spaces_a_real_host() {
    // Reserve two arXiv slots back to back without sleeping: the second must be owed 3 s.
    let clock = Arc::new(FakeRateClock::new());
    let gate = NetRateGate { wait_on_loopback: false, ..NetRateGate::new(clock.clone()) };
    let endpoints = LiteratureEndpoints::default();
    let uri: ureq::http::Uri = "https://export.arxiv.org/api/query?id_list=1".parse().unwrap();
    assert_eq!(gate.acquire_for_url(&uri, &endpoints, None).waited, Duration::ZERO);
    assert_eq!(gate.acquire_for_url(&uri, &endpoints, None).waited, Duration::from_secs(3));
}
