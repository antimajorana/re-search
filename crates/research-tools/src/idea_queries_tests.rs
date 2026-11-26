use super::*;
use crate::arxiv::normalize_arxiv_id;
use crate::idea_terms::extract;

fn assert_balanced_quotes_and_parens(q: &str) {
    assert_eq!(q.bytes().filter(|&b| b == b'"').count() % 2, 0, "unbalanced quotes in `{q}`");
    let mut depth = 0i32;
    for ch in q.chars() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            assert!(depth >= 0, "negative paren depth in `{q}`");
        }
    }
    assert_eq!(depth, 0, "unbalanced parens in `{q}`");
}

#[test]
fn service_wire_names_match_repo_conventions() {
    assert_eq!(QueryService::Arxiv.as_str(), "arxiv");
    assert_eq!(QueryService::Inspire.as_str(), "inspire");
    assert_eq!(QueryService::SemanticScholar.as_str(), "semantic_scholar");
    assert_eq!(QueryService::Crossref.as_str(), "crossref");
}

#[test]
fn default_first_queries_obey_rule_p4_and_are_deterministic() {
    let idea = extract(
        "Use \"Krylov complexity\" and Lanczos coefficients for the measurement-induced \
         entanglement transition in a monitored Majorana chain (see 2103.01234).",
    );
    let q1 = first_queries(&idea);
    let q2 = first_queries(&idea);
    assert_eq!(q1, q2);
    assert!(!q1.is_empty());
    assert!(q1.len() <= MAX_USER_PREVIEW_QUERIES);

    let mut seen = BTreeSet::new();
    for (idx, pq) in q1.iter().enumerate() {
        assert_eq!(pq.qid, format!("q-{}", idx + 1));
        assert_eq!(pq.origin, QueryOrigin::User);
        assert!(matches!(pq.provider, QueryService::Arxiv | QueryService::Inspire));
        assert!(!pq.q.is_empty());
        assert_eq!(pq.q, pq.q.trim());
        assert!(pq.q.chars().count() <= MAX_QUERY_CHARS);
        assert!(seen.insert((pq.provider, pq.q.clone())), "duplicate query: {pq:?}");
        if pq.provider == QueryService::Arxiv {
            assert_balanced_quotes_and_parens(&pq.q);
        }
    }
    assert!(q1.iter().any(|p| p.provider == QueryService::Arxiv));
    assert!(q1.iter().any(|p| p.provider == QueryService::Inspire));
}

#[test]
fn arxiv_queries_match_arxiv_rs_id_and_prefix_syntax() {
    let idea = extract(
        "Check \"Chern-Simons\" duality in AdS/CFT starting from arXiv:2103.01234v2 and hep-th/9711200.",
    );
    let cats = vec!["hep-th".to_string(), "cat:cond-mat.str-el".to_string()];
    let queries = build_arxiv_queries(&idea, &cats, 6);
    assert!(queries.len() >= 3);

    // First two queries are bare arXiv IDs accepted by `arxiv::normalize_arxiv_id`.
    assert_eq!(queries[0].0, "2103.01234");
    assert!(normalize_arxiv_id(&queries[0].0).is_some());
    assert_eq!(queries[1].0, "hep-th/9711200");
    assert!(normalize_arxiv_id(&queries[1].0).is_some());

    // Subsequent queries use arXiv inline prefixes and category filter.
    for (q, _) in &queries[2..] {
        assert!(q.contains("all:") || q.contains("ti:"), "expected arXiv prefix in `{q}`");
        assert!(q.contains("cat:hep-th") && q.contains("cat:cond-mat.str-el"), "missing cats in `{q}`");
        assert_balanced_quotes_and_parens(q);
        assert!(q.chars().count() <= MAX_QUERY_CHARS);
    }
}

#[test]
fn builds_queries_for_all_four_services() {
    let idea = extract(
        "Compute \"Schwinger-Dyson\" propagators in \\mathcal{N}=4 \\mathrm{SU}(N) Yang-Mills \
         referencing 10.1103/PhysRevD.90.084021 and 2103.01234.",
    );
    let opts = FirstQueryOptions {
        services: vec![
            QueryService::Arxiv,
            QueryService::Inspire,
            QueryService::SemanticScholar,
            QueryService::Crossref,
        ],
        max_queries: 12,
        categories: vec![],
    };
    let planned = first_queries_for(&idea, &opts);
    for svc in [
        QueryService::Arxiv,
        QueryService::Inspire,
        QueryService::SemanticScholar,
        QueryService::Crossref,
    ] {
        assert!(
            planned.iter().any(|p| p.provider == svc),
            "expected at least one query for {svc:?}"
        );
    }

    let inspire = build_inspire_queries(&idea, 6);
    assert!(inspire.iter().any(|(q, _)| q == "doi:10.1103/physrevd.90.084021"));
    assert!(inspire.iter().any(|(q, _)| q == "arXiv:2103.01234"));
    assert!(inspire.iter().any(|(q, _)| q.contains("N=4") || q.contains("SU(N)")));

    let s2 = build_semantic_scholar_queries(&idea, 4);
    assert!(!s2.is_empty());
    assert!(s2.iter().all(|(q, _)| !q.contains("all:") && !q.contains("ti:")));

    let cr = build_crossref_queries(&idea, 4);
    assert_eq!(cr[0].0, "10.1103/physrevd.90.084021");
    assert!(cr.iter().all(|(q, _)| !q.contains("all:") && !q.contains("ti:")));
}

#[test]
fn translates_tex_math_tokens_to_search_friendly_text() {
    assert_eq!(math_token_to_search_text(r"\mathcal{N}=4"), "N=4");
    assert_eq!(math_token_to_search_text(r"\mathrm{SU}(N)"), "SU(N)");
    assert_eq!(math_token_to_search_text(r"\mathbb{Z}_2"), "Z_2");
    assert_eq!(math_token_to_search_text(r"\operatorname{Tr}"), "Tr");
    assert_eq!(math_token_to_search_text(r"\alpha_s"), "alpha_s");
    assert_eq!(math_token_to_search_text(r"\frac{1}{2}"), "1/2");
    assert_eq!(math_token_to_search_text(r"\Lambda_{\text{QCD}}"), "Lambda_QCD");
    assert_eq!(math_token_to_search_text(r"b_n \sim \alpha n"), "b_n ~ alpha n");
}

#[test]
fn enforces_300_char_cap_even_on_huge_terms() {
    let huge_phrase = format!("\"{}\"", "entanglement ".repeat(40).trim());
    let idea = extract(&format!(
        "Study {huge_phrase} with Majorana Kitaev Lanczos Krylov Schwinger-Dyson Chern-Simons."
    ));
    let opts = FirstQueryOptions {
        services: vec![
            QueryService::Arxiv,
            QueryService::Inspire,
            QueryService::SemanticScholar,
            QueryService::Crossref,
        ],
        max_queries: 16,
        categories: vec!["hep-th".into(), "cond-mat.str-el".into()],
    };
    let queries = first_queries_for(&idea, &opts);
    assert!(!queries.is_empty());
    for pq in &queries {
        assert!(!pq.q.is_empty());
        assert!(pq.q.chars().count() <= MAX_QUERY_CHARS, "query exceeded 300 chars: {}", pq.q);
        if pq.provider == QueryService::Arxiv {
            assert_balanced_quotes_and_parens(&pq.q);
        }
    }
}

#[test]
fn empty_idea_produces_no_queries() {
    let empty = extract("   ...,,,   ");
    assert!(first_queries(&empty).is_empty());
    assert!(build_arxiv_queries(&empty, &["hep-th".into()], 4).is_empty());
    assert!(build_inspire_queries(&empty, 4).is_empty());
    assert!(build_semantic_scholar_queries(&empty, 4).is_empty());
    assert!(build_crossref_queries(&empty, 4).is_empty());
}
