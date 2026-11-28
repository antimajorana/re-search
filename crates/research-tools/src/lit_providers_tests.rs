use super::url::{parse_imf_fixdate_unix_secs, parse_retry_after_secs_at};
use super::*;

#[test]
fn provider_wire_names_and_allowlist_flags() {
    for p in LitProvider::ALL {
        let json = serde_json::to_string(&p).unwrap();
        let back: LitProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(LitProvider::parse(p.as_str()), Some(p));
    }
    assert!(LitProvider::Arxiv.is_allowlisted());
    assert!(LitProvider::Inspire.is_allowlisted());
    assert!(LitProvider::SemanticScholar.is_allowlisted());
    assert!(LitProvider::Crossref.is_allowlisted());
    assert!(LitProvider::OpenAlex.is_allowlisted());
    assert!(!LitProvider::TheoremSearch.is_allowlisted());
}

#[test]
fn provider_config_debug_redacts_api_key() {
    let cfg = ProviderConfig {
        semantic_scholar_api_key: Some("fake-unit-secret-token-xyz".into()),
        openalex_api_key: Some("fake-unit-openalex-secret-abc".into()),
        request_timeout: Some(Duration::from_secs(2)),
    };
    let dbg = format!("{cfg:?}");
    assert!(!dbg.contains("fake-unit-secret-token-xyz"));
    assert!(!dbg.contains("fake-unit-openalex-secret-abc"));
    assert!(dbg.contains("<redacted>"));
}

#[test]
fn retry_after_parses_seconds_imf_date_and_garbage() {
    assert_eq!(parse_retry_after_secs_at("5", 1_000), Some(5));
    assert_eq!(parse_retry_after_secs_at(" 120 ", 1_000), Some(120));
    assert_eq!(parse_retry_after_secs_at("0", 1_000), Some(0));
    // 2026-10-21T07:28:00Z = 1792567680
    let target = parse_imf_fixdate_unix_secs("Wed, 21 Oct 2026 07:28:00 GMT").unwrap();
    assert_eq!(target, 1_792_567_680);
    assert_eq!(
        parse_retry_after_secs_at("Wed, 21 Oct 2026 07:28:00 GMT", target - 45),
        Some(45)
    );
    // Past HTTP-date saturates at 0.
    assert_eq!(
        parse_retry_after_secs_at("Wed, 21 Oct 2026 07:28:00 GMT", target + 100),
        Some(0)
    );
    // Unparseable values return None.
    assert_eq!(parse_retry_after_secs_at("soon", 1_000), None);
    assert_eq!(parse_retry_after_secs_at("", 1_000), None);
}

#[test]
fn url_builders_cover_paging_ids_and_relations() {
    let ep = LiteratureEndpoints::default();

    // arXiv bare id -> id_list
    let q_id = LitQuery::search(LitProvider::Arxiv, "2501.00012v2").with_paging(5, 10);
    let url = build_request_url(&ep, &q_id, 5).unwrap();
    assert!(url.starts_with("https://export.arxiv.org/api/query?id_list="));
    assert!(url.contains("start=10&max_results=5"));

    // arXiv text search -> search_query=all:...
    let q_txt = LitQuery::search(LitProvider::Arxiv, "dimer model").with_paging(8, 16);
    let url = build_request_url(&ep, &q_txt, 8).unwrap();
    assert!(url.contains("search_query=all%3Adimer%20model"));
    assert!(url.contains("start=16&max_results=8"));

    // INSPIRE 1-indexed page and citation relations
    let q_ins = LitQuery::search(LitProvider::Inspire, "lattice").with_paging(10, 20);
    let url = build_request_url(&ep, &q_ins, 10).unwrap();
    assert!(url.contains("size=10&page=3&fields="));

    let recid = PaperKey::from_inspire("1234567").unwrap();
    let q_cites = LitQuery::citation_relation(
        LitProvider::Inspire,
        QueryRelation::Cites { key: recid.clone() },
    );
    assert!(build_request_url(&ep, &q_cites, 10)
        .unwrap()
        .contains("q=citedby%3Arecid%3A1234567"));

    let q_cited_by =
        LitQuery::citation_relation(LitProvider::Inspire, QueryRelation::CitedBy { key: recid });
    assert!(build_request_url(&ep, &q_cited_by, 10)
        .unwrap()
        .contains("q=refersto%3Arecid%3A1234567"));

    // Semantic Scholar search + references + citations
    let ax = PaperKey::from_arxiv("2501.00012").unwrap();
    let q_s2_refs = LitQuery::citation_relation(
        LitProvider::SemanticScholar,
        QueryRelation::Cites { key: ax.clone() },
    );
    assert!(build_request_url(&ep, &q_s2_refs, 5)
        .unwrap()
        .contains("/graph/v1/paper/ARXIV%3A2501%2E00012/references?limit=5&offset=0"));

    let q_s2_cits = LitQuery::citation_relation(
        LitProvider::SemanticScholar,
        QueryRelation::CitedBy { key: ax },
    );
    assert!(build_request_url(&ep, &q_s2_cits, 5)
        .unwrap()
        .contains("/graph/v1/paper/ARXIV%3A2501%2E00012/citations?limit=5&offset=0"));

    // Crossref Cites requires DOI; CitedBy is unsupported
    let doi = PaperKey::from_doi("10.9999/synth.1").unwrap();
    let q_cr_cites = LitQuery::citation_relation(
        LitProvider::Crossref,
        QueryRelation::Cites { key: doi.clone() },
    );
    assert!(build_request_url(&ep, &q_cr_cites, 5)
        .unwrap()
        .starts_with("https://api.crossref.org/works/10%2E9999%2Fsynth%2E1"));
    let q_cr_cited_by =
        LitQuery::citation_relation(LitProvider::Crossref, QueryRelation::CitedBy { key: doi });
    assert_eq!(
        build_request_url(&ep, &q_cr_cited_by, 5).unwrap_err().0,
        QueryStatus::Unsupported
    );

    // OpenAlex active (X19 Part 1) & TheoremSearch stays disabled
    let q_oa = LitQuery::search(LitProvider::OpenAlex, "spectral gap").with_paging(5, 10);
    let oa_url = build_request_url(&ep, &q_oa, 5).unwrap();
    assert!(oa_url.starts_with("https://api.openalex.org/works?search="));
    assert!(oa_url.contains("per-page=5&page=3"));
    let q_ts = LitQuery::search(LitProvider::TheoremSearch, "spectral gap");
    assert_eq!(
        build_request_url(&ep, &q_ts, 5).unwrap_err().0,
        QueryStatus::Disabled
    );
}

#[test]
fn s2_empty_zero_total_without_data_array_is_valid_empty() {
    let raw = br#"{"total": 0, "offset": 0}"#;
    let parsed = parse_provider_response(
        LitProvider::SemanticScholar,
        raw,
        &QueryRelation::None,
        0,
        Some(10),
    )
    .expect("zero-hit S2 response");
    assert!(parsed.papers.is_empty());
    assert_eq!(parsed.total_results, Some(0));
    assert!(!parsed.has_more);
}
