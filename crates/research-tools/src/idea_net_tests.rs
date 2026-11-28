use super::*;
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use ledger::Event;
use serde_json::json;

fn pk(s: &str) -> PaperKey {
    PaperKey::parse(s).expect("valid test PaperKey")
}

fn edge(from: &str, to: &str, provider: LitProvider) -> CitationEdge {
    CitationEdge { from: pk(from), to: pk(to), provider }
}

fn meta(key: &str, title: &str, year: i32, cites: u64) -> CitationPaperMeta {
    CitationPaperMeta {
        key: Some(pk(key)),
        ids: PaperIds::default(),
        title: Some(title.into()),
        authors: vec!["Ada Example".into()],
        year: Some(year),
        citation_count: Some(cites),
        in_library: false,
        owned: false,
    }
}

fn make_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-net-test".into(),
        seq,
        ts_ms: (seq * 1000) as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: String::new(),
    }
}

#[test]
fn in_network_ranking_is_deterministic_and_invariant_to_input_order() {
    let edges_a = vec![
        edge("arxiv:2401.00001", "arxiv:2101.00010", LitProvider::Inspire),
        edge("arxiv:2401.00002", "arxiv:2101.00010", LitProvider::OpenAlex),
        edge("arxiv:2401.00003", "arxiv:2101.00010", LitProvider::SemanticScholar),
        edge("arxiv:2401.00001", "arxiv:2202.00020", LitProvider::Inspire),
        edge("arxiv:2202.00020", "arxiv:2101.00010", LitProvider::Inspire),
        // Duplicate from second provider must NOT inflate degree.
        edge("arxiv:2401.00001", "arxiv:2101.00010", LitProvider::SemanticScholar),
        // Self-loop must be ignored.
        edge("arxiv:2401.00001", "arxiv:2401.00001", LitProvider::Inspire),
    ];
    let mut edges_rev = edges_a.clone();
    edges_rev.reverse();
    let edges_rot: Vec<CitationEdge> = edges_a.iter().skip(3).chain(edges_a.iter().take(3)).cloned().collect();

    let papers_a = vec![
        meta("arxiv:2101.00010", "Foundational Lanczos Paper", 2021, 90),
        meta("arxiv:2202.00020", "Bridge Paper", 2022, 30),
        meta("arxiv:2401.00001", "Core Paper A", 2024, 15),
        meta("arxiv:2401.00002", "Core Paper B", 2024, 12),
        meta("arxiv:2401.00003", "Citing Paper C", 2024, 4),
    ];
    let mut papers_rev = papers_a.clone();
    papers_rev.reverse();

    let net_a = build(CitationNetInput {
        seeds: vec![pk("arxiv:2401.00001"), pk("arxiv:2401.00002")],
        core_papers: vec![pk("arxiv:2401.00001"), pk("arxiv:2401.00002")],
        read_papers: [pk("arxiv:2401.00001")].into_iter().collect(),
        origin_paper: None,
        edges: edges_a,
        papers: papers_a,
        hops: 1,
    });
    let net_rev = build(CitationNetInput {
        seeds: vec![pk("arxiv:2401.00002"), pk("arxiv:2401.00001")],
        core_papers: vec![pk("arxiv:2401.00002"), pk("arxiv:2401.00001")],
        read_papers: [pk("arxiv:2401.00001")].into_iter().collect(),
        origin_paper: None,
        edges: edges_rev,
        papers: papers_rev.clone(),
        hops: 1,
    });
    let net_rot = build(CitationNetInput {
        seeds: vec![pk("arxiv:2401.00001"), pk("arxiv:2401.00002")],
        core_papers: vec![pk("arxiv:2401.00002"), pk("arxiv:2401.00001")],
        read_papers: [pk("arxiv:2401.00001")].into_iter().collect(),
        origin_paper: None,
        edges: edges_rot,
        papers: papers_rev,
        hops: 1,
    });

    assert_eq!(net_a, net_rev);
    assert_eq!(net_a, net_rot);

    // 5 unique non-self edges; duplicate edge merged with both providers.
    assert_eq!(net_a.edges.len(), 5);
    let dup = net_a
        .edges
        .iter()
        .find(|e| e.from == pk("arxiv:2401.00001") && e.to == pk("arxiv:2101.00010"))
        .expect("merged edge");
    assert_eq!(dup.providers, vec![LitProvider::Inspire, LitProvider::SemanticScholar]);

    // Hand-counted degrees:
    // arxiv:2101.00010: in=4 (00001, 00002, 00003, 00020), out=0 -> rank 4
    // arxiv:2401.00001: in=0, out=2 (00010, 00020) -> rank 2
    // arxiv:2202.00020: in=1 (00001), out=1 (00010) -> rank 2 (higher in_degree than 00001!)
    assert_eq!(net_a.nodes[0].key, pk("arxiv:2101.00010"));
    assert_eq!(net_a.nodes[0].in_network_in_degree, 4);
    assert_eq!(net_a.nodes[0].in_network_out_degree, 0);
    assert_eq!(net_a.nodes[0].in_network_rank, 4);
    assert_eq!(net_a.nodes[1].key, pk("arxiv:2202.00020"));
    assert_eq!(net_a.nodes[1].in_network_rank, 2);
    assert_eq!(net_a.nodes[1].in_network_in_degree, 1);
    assert_eq!(net_a.nodes[2].key, pk("arxiv:2401.00001"));
    assert_eq!(net_a.nodes[2].in_network_rank, 2);
}

#[test]
fn unread_core_citations_exclude_read_and_origin_papers_across_aliases() {
    // Origin paper has both arXiv and DOI aliases; core papers cite it via its DOI alias!
    let origin = PaperRef {
        key: pk("arxiv:2001.00099"),
        version: Some("v2".into()),
        ids: PaperIds {
            arxiv: Some("2001.00099".into()),
            doi: Some("10.5555/origin-paper".into()),
            inspire: None,
            s2: None,
        },
    };
    let read_target = pk("arxiv:2105.00042");
    let unread_high = pk("arxiv:2201.00007");
    let unread_low = pk("arxiv:2201.00008");

    let net = build(CitationNetInput {
        seeds: vec![pk("arxiv:2401.00001"), pk("arxiv:2401.00002")],
        core_papers: vec![pk("arxiv:2401.00001"), pk("arxiv:2401.00002")],
        read_papers: [pk("arxiv:2401.00001"), read_target.clone()].into_iter().collect(),
        origin_paper: Some(origin),
        edges: vec![
            // Both core papers cite the origin paper via DOI alias -> MUST be excluded (Rule V9).
            edge("arxiv:2401.00001", "doi:10.5555/origin-paper", LitProvider::OpenAlex),
            edge("arxiv:2401.00002", "arxiv:2001.00099", LitProvider::Inspire),
            // Both core papers cite already-read paper -> MUST be excluded.
            edge("arxiv:2401.00001", "arxiv:2105.00042", LitProvider::Inspire),
            edge("arxiv:2401.00002", "arxiv:2105.00042", LitProvider::Inspire),
            // Both core papers cite unread_high -> MUST rank #1 in unread_core_citations.
            edge("arxiv:2401.00001", "arxiv:2201.00007", LitProvider::Inspire),
            edge("arxiv:2401.00002", "arxiv:2201.00007", LitProvider::Inspire),
            // One core paper cites unread_low -> MUST rank #2.
            edge("arxiv:2401.00002", "arxiv:2201.00008", LitProvider::SemanticScholar),
        ],
        papers: vec![
            meta("arxiv:2201.00007", "Synthetic Core Reference One", 2022, 40),
            meta("arxiv:2201.00008", "Synthetic Core Reference Two", 2022, 10),
        ],
        hops: 1,
    });

    let unread_keys: Vec<&PaperKey> = net.unread_core_citations.iter().map(|u| &u.key).collect();
    assert_eq!(unread_keys, vec![&unread_high, &unread_low]);
    assert_eq!(net.unread_core_citations[0].cited_by_core_count, 2);
    assert_eq!(net.unread_core_citations[1].cited_by_core_count, 1);

    // Origin node is collapsed to canonical arxiv key and marked is_origin = true.
    let origin_node = net
        .nodes
        .iter()
        .find(|n| n.key == pk("arxiv:2001.00099"))
        .expect("canonical origin node");
    assert!(origin_node.is_origin);
    assert!(net.nodes.iter().all(|n| n.key != pk("doi:10.5555/origin-paper")));
}

#[test]
fn build_from_events_folds_active_revision_and_prior_citation_net() {
    let events = vec![
        make_event(
            1,
            "handoff.packet",
            json!({
                "from": {
                    "paper": { "key": "arxiv:1901.00001", "ids": { "arxiv": "1901.00001" } }
                }
            }),
        ),
        make_event(2, "idea.stated", json!({ "rev": 1, "seeds": ["arxiv:2301.00001"] })),
        make_event(
            3,
            "idea.triage",
            json!({ "items": [{ "key": "arxiv:2301.00001", "relevance": "core" }] }),
        ),
        make_event(
            4,
            "idea.paper",
            json!({ "key": "arxiv:2301.00001", "status": "read_passages" }),
        ),
        make_event(
            5,
            EVENT_IDEA_CITATION_NET,
            json!({
                "seeds": ["arxiv:2301.00001"],
                "hops": 1,
                "provider": "inspire",
                "edges": [
                    { "from": "arxiv:2301.00001", "to": "arxiv:1901.00001", "provider": "inspire" },
                    { "from": "arxiv:2301.00001", "to": "arxiv:2203.00005", "provider": "inspire" }
                ]
            }),
        ),
    ];

    let net = build_from_events(
        &events,
        &[edge("arxiv:2501.00011", "arxiv:2301.00001", LitProvider::OpenAlex)],
        &[meta("arxiv:2501.00011", "Follow-up Paper", 2025, 3)],
        &[pk("arxiv:2301.00001")],
        1,
    );

    assert_eq!(net.seeds, vec![pk("arxiv:2301.00001")]);
    assert_eq!(net.hops, 1);
    assert_eq!(net.edges.len(), 3);
    assert_eq!(net.unread_core_citations.len(), 1);
    assert_eq!(net.unread_core_citations[0].key, pk("arxiv:2203.00005"));
}

#[test]
fn supported_citation_providers_match_f11_and_x19_capabilities() {
    assert_eq!(
        supported_citation_providers(&pk("arxiv:2401.00001"), "cited_by"),
        vec![LitProvider::Inspire, LitProvider::SemanticScholar]
    );
    assert_eq!(
        supported_citation_providers(&pk("doi:10.5555/demo"), "cited_by"),
        vec![LitProvider::Inspire, LitProvider::OpenAlex, LitProvider::SemanticScholar]
    );
    assert_eq!(
        supported_citation_providers(&pk("doi:10.5555/demo"), "cites"),
        vec![
            LitProvider::Inspire,
            LitProvider::OpenAlex,
            LitProvider::SemanticScholar,
            LitProvider::Crossref,
        ]
    );
    assert_eq!(
        supported_citation_providers(&pk("openalex:W12345"), "cited_by"),
        vec![LitProvider::OpenAlex]
    );
    assert!(supported_citation_providers(&pk("file:0123456789abcdef"), "cited_by").is_empty());
    assert_eq!(
        default_citation_provider(&pk("arxiv:2401.00001"), "cited_by"),
        Some(LitProvider::Inspire)
    );
}

#[test]
fn audit_m3_alias_reload_triage_downgrade_and_rev2_isolation() {
    // 1. Alias learned in round 1 persists on CitationNetNode.ids when reloaded via build_from_events.
    let paper_with_alias = CitationPaperMeta {
        key: Some(pk("arxiv:2209.00077")),
        ids: PaperIds {
            arxiv: Some("2209.00077".into()),
            doi: Some("10.5555/read-paper".into()),
            inspire: None,
            s2: None,
        },
        title: Some("Read Paper With DOI Alias".into()),
        authors: vec!["Ada Example".into()],
        year: Some(2022),
        citation_count: Some(15),
        in_library: false,
        owned: false,
    };
    let net1 = build(CitationNetInput {
        seeds: vec![pk("arxiv:2401.00001")],
        core_papers: vec![pk("arxiv:2401.00001")],
        read_papers: [pk("arxiv:2401.00001")].into_iter().collect(),
        origin_paper: None,
        edges: vec![edge("arxiv:2401.00001", "doi:10.5555/read-paper", LitProvider::OpenAlex)],
        papers: vec![paper_with_alias],
        hops: 1,
    });
    let persisted = serde_json::to_value(&net1).unwrap();
    // Later `idea.paper` marks the paper read via its arXiv key; reload has NO extra_papers!
    let reload_events = vec![
        make_event(1, "idea.stated", json!({ "rev": 1, "seeds": ["arxiv:2401.00001"] })),
        make_event(2, "idea.triage", json!({ "items": [{ "key": "arxiv:2401.00001", "relevance": "core" }] })),
        make_event(3, EVENT_IDEA_CITATION_NET, persisted),
        make_event(4, "idea.paper", json!({ "key": "doi:10.5555/read-paper", "status": "read_full" })),
    ];
    let reloaded = build_from_events(&reload_events, &[], &[], &[], 0);
    assert!(
        reloaded.unread_core_citations.is_empty(),
        "DOI-read paper must remain excluded via persisted CitationNetNode.ids alias: {:?}",
        reloaded.unread_core_citations
    );

    // 2. Triage downgrade (core -> related) removes paper from core_papers even if read.
    let downgrade_events = vec![
        make_event(1, "idea.stated", json!({ "rev": 1, "seeds": ["arxiv:2401.00001"] })),
        make_event(2, "idea.triage", json!({ "items": [
            { "key": "arxiv:2401.00001", "relevance": "core" },
            { "key": "arxiv:2401.00002", "relevance": "core" }
        ] })),
        make_event(3, "idea.triage", json!({ "items": [
            { "key": "arxiv:2401.00002", "relevance": "related" }
        ] })),
        make_event(4, "idea.paper", json!({ "key": "arxiv:2401.00002", "status": "read_passages" })),
    ];
    let net_dg = build_from_events(
        &downgrade_events,
        &[edge("arxiv:2401.00002", "arxiv:2101.00055", LitProvider::Inspire)],
        &[],
        &[],
        1,
    );
    assert!(
        net_dg.unread_core_citations.is_empty(),
        "downgraded `related` read paper must not generate unread_core_citations"
    );

    // 3. Revision 2 clears rev 1 citation net while preserving pre-rev handoff.packet origin exclusion.
    let rev2_events = vec![
        make_event(1, "handoff.packet", json!({
            "from": { "paper": { "key": "arxiv:1901.00001", "ids": { "arxiv": "1901.00001" } } }
        })),
        make_event(2, "idea.stated", json!({ "rev": 1, "seeds": ["arxiv:2301.00001"] })),
        make_event(3, EVENT_IDEA_CITATION_NET, json!({
            "seeds": ["arxiv:2301.00001"], "hops": 1, "provider": "inspire",
            "edges": [{ "from": "arxiv:2301.00001", "to": "arxiv:2101.00001", "provider": "inspire" }]
        })),
        make_event(4, "idea.stated", json!({ "rev": 2, "seeds": ["arxiv:2401.00009"] })),
    ];
    let net_r2 = build_from_events(
        &rev2_events,
        &[edge("arxiv:2401.00009", "arxiv:1901.00001", LitProvider::Inspire)],
        &[],
        &[pk("arxiv:2401.00009")],
        1,
    );
    assert_eq!(net_r2.edges.len(), 1, "rev 1 edges must not leak into rev 2");
    assert!(net_r2.unread_core_citations.is_empty(), "origin paper must stay excluded in rev 2");
}
