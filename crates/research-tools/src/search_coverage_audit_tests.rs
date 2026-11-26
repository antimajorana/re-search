//! Adversarial regression tests added from the independent F12 audit (H1, M1, M3, M4, L1).

use super::*;
use crate::lit_providers::{LitProvider, LITERATURE_FETCHED_EVENT};
use crate::paper_key::PaperKey;
use ledger::{Event, GENESIS_HASH};
use serde_json::json;

fn mk_event(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-audit-cov".into(),
        seq,
        ts_ms: 1_700_000_000_000 + seq as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: GENESIS_HASH.into(),
        event_hash: GENESIS_HASH.into(),
    }
}

#[test]
fn literature_fetched_before_search_finished_preserves_hits_cached_and_inspire_aliases() {
    let seed_inspire = PaperKey::from_inspire("1900123").unwrap();
    let events = vec![
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(2, IDEA_STATED_EVENT, json!({"seeds": [seed_inspire.as_str()]})),
        mk_event(3, IDEA_BRIEF_EVENT, json!({
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(4, IDEA_SEARCH_PLANNED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "origin": "user", "relation": "none"},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "origin": "model", "relation": "none"}
            ]
        })),
        // literature.fetched arrives BEFORE idea.search_finished (normal F11 tool order).
        mk_event(5, LITERATURE_FETCHED_EVENT, json!({
            "provider": "arxiv",
            "qid": "q-1",
            "count": 3,
            "cached_from_ms": 1_700_000_000_000u64,
            "papers": [{"key": "arxiv:2401.00001", "base_id": "2401.00001"}]
        })),
        mk_event(6, LITERATURE_FETCHED_EVENT, json!({
            "provider": "inspire",
            "qid": "q-2",
            "count": 2,
            "hits": [{"key": "arxiv:2401.00001", "id": "1900123", "arxiv_id": "2401.00001"}]
        })),
        // idea.search_finished omits `hits` and `cached` (only records status).
        mk_event(7, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "provider": "arxiv", "status": "ok"},
                {"qid": "q-2", "provider": "inspire", "status": "ok"}
            ]
        })),
        mk_event(8, IDEA_CANDIDATES_EVENT, json!({
            "candidates": [{"key": "arxiv:2401.00001", "ids": {"arxiv": "2401.00001"}, "found_by": ["q-1", "q-2"]}]
        })),
        mk_event(9, IDEA_TRIAGE_EVENT, json!({"items": [{"key": "arxiv:2401.00001", "relevance": "core"}]})),
        mk_event(10, IDEA_PAPER_EVENT, json!({"key": "arxiv:2401.00001", "status": "read_full"})),
    ];

    let cov = compute_coverage_from_events(&events);
    assert!(cov.gate.passed, "unexpected failures: {:?}", cov.gate.unsatisfied);
    let arxiv = cov.providers.iter().find(|p| p.provider == LitProvider::Arxiv).unwrap();
    assert_eq!(arxiv.hits, 3);
    assert_eq!(arxiv.cached, 1);
    assert!(!arxiv.answered_with_zero_results());
    let inspire = cov.providers.iter().find(|p| p.provider == LitProvider::Inspire).unwrap();
    assert_eq!(inspire.hits, 2);
    assert_eq!(cov.known_paper_recall.found_by_search, vec![seed_inspire]);
    assert!(coverage_matches_recomputed(&cov, &events));
}

#[test]
fn direct_seed_lookup_never_satisfies_external_service_or_perspective_gate() {
    let input = CoverageInput {
        preset: ExplorePreset::Standard,
        perspectives: vec![
            ("p-1".into(), PerspectiveKind::Exact),
            ("p-2".into(), PerspectiveKind::OtherNames),
        ],
        queries: vec![
            QueryCoverageRecord {
                qid: "q-1".into(),
                provider: LitProvider::Arxiv,
                perspective_id: Some("p-1".into()),
                perspective_kind: Some(PerspectiveKind::Exact),
                status: QueryOutcomeStatus::Ok,
                hits: 1,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
            // Crossref ran ONLY a direct seed lookup (must not count as 2nd service or perspective).
            QueryCoverageRecord {
                qid: "q-seed".into(),
                provider: LitProvider::Crossref,
                perspective_id: Some("p-2".into()),
                perspective_kind: Some(PerspectiveKind::OtherNames),
                status: QueryOutcomeStatus::Ok,
                hits: 1,
                cached: false,
                is_seed_lookup: true,
                found_papers: vec![],
            },
        ],
        ..Default::default()
    };

    let cov = compute_coverage(&input);
    assert!(!cov.gate.passed);
    assert_eq!(cov.gate.answered_external_services, 1);
    assert!(!cov.gate.services_minimum_met);
    assert!(!cov.gate.other_names_perspective_answered);
    let cr = cov.providers.iter().find(|p| p.provider == LitProvider::Crossref).unwrap();
    assert_eq!(cr.state, ProviderCoverageState::NotAttempted);
    assert_eq!(cr.seed_lookup_queries_ok, 1);
    assert!(!cr.answered());
}

#[test]
fn explicit_partial_provider_state_always_fails_attempted_service_gate() {
    let input = CoverageInput {
        preset: ExplorePreset::Standard,
        provider_states: vec![(LitProvider::Arxiv, ProviderCoverageState::Partial)],
        perspectives: vec![
            ("p-1".into(), PerspectiveKind::Exact),
            ("p-2".into(), PerspectiveKind::OtherNames),
        ],
        queries: vec![
            QueryCoverageRecord {
                qid: "q-1".into(),
                provider: LitProvider::Arxiv,
                perspective_id: Some("p-1".into()),
                perspective_kind: Some(PerspectiveKind::Exact),
                status: QueryOutcomeStatus::Ok,
                hits: 1,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
            QueryCoverageRecord {
                qid: "q-2".into(),
                provider: LitProvider::Inspire,
                perspective_id: Some("p-2".into()),
                perspective_kind: Some(PerspectiveKind::OtherNames),
                status: QueryOutcomeStatus::Ok,
                hits: 1,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
        ],
        ..Default::default()
    };
    let cov = compute_coverage(&input);
    assert!(!cov.gate.passed);
    assert!(!cov.gate.no_attempted_service_failed);
    let arx = cov.providers.iter().find(|p| p.provider == LitProvider::Arxiv).unwrap();
    assert_eq!(arx.state, ProviderCoverageState::Partial);
}

#[test]
fn multi_round_triage_preserves_round1_core_and_merges_candidate_ids_and_rejects_zero_budget() {
    let doi_seed = PaperKey::from_doi("10.1000/round2.alias").unwrap();
    let events = vec![
        // budget.max_screened: 0 must be ignored (cannot bypass screening).
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard", "budget": {"max_screened": 0}})),
        mk_event(2, IDEA_STATED_EVENT, json!({"seeds": [doi_seed.as_str()]})),
        mk_event(3, IDEA_BRIEF_EVENT, json!({
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(4, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 1},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 1},
                {"qid": "q-3", "provider": "crossref", "status": "unsupported"},
                {"qid": "q-4", "provider": "semantic_scholar", "status": "skipped_budget"}
            ]
        })),
        // Round 1 candidate has only arxiv id; Round 2 adds DOI alias.
        mk_event(5, IDEA_CANDIDATES_EVENT, json!({
            "candidates": [
                {"key": "arxiv:2401.00001", "ids": {"arxiv": "2401.00001"}, "found_by": ["q-1"]},
                {"key": "arxiv:2401.00002", "ids": {"arxiv": "2401.00002"}, "found_by": ["q-2"]}
            ]
        })),
        mk_event(6, IDEA_CANDIDATES_EVENT, json!({
            "candidates": [
                {"key": "arxiv:2401.00001", "ids": {"doi": "10.1000/round2.alias"}, "found_by": ["q-2"]}
            ]
        })),
        // Round 1 triage marks 2401.00001 as core; Round 2 incrementally adds 2401.00002 as related.
        mk_event(7, IDEA_TRIAGE_EVENT, json!({
            "items": [{"key": "arxiv:2401.00001", "relevance": "core"}]
        })),
        mk_event(8, IDEA_TRIAGE_EVENT, json!({
            "items": [{"key": "arxiv:2401.00002", "relevance": "related"}]
        })),
        mk_event(9, IDEA_CITATION_NET_EVENT, json!({"seeds": []})),
    ];

    let cov = compute_coverage_from_events(&events);
    // Round 1 core paper MUST NOT be wiped by Round 2 triage; it's unread -> fails gate!
    assert!(!cov.gate.all_core_papers_resolved);
    assert_eq!(cov.unresolved_core.len(), 1);
    assert_eq!(cov.unresolved_core[0].reason, CorePaperGap::Unread);
    // unavailable_core holds only Unavailable papers, NOT Unread (L1).
    assert!(cov.unavailable_core.is_empty());
    // Merged DOI alias satisfies seed recall.
    assert!(cov.gate.all_seeds_found);
    // budget.max_screened: 0 was ignored -> required is min(40, 2) = 2.
    assert_eq!(cov.screened_required, 2);
    assert_eq!(cov.screened, 2);
    // citation_net with 0 seeds keeps hops == 0.
    assert_eq!(cov.citation_expansion, CitationExpansionCoverage { seeds: 0, hops: 0 });
    // Unsupported and SkippedBudget counters preserved.
    let cr = cov.providers.iter().find(|p| p.provider == LitProvider::Crossref).unwrap();
    assert_eq!(cr.queries_unsupported, 1);
    let s2 = cov.providers.iter().find(|p| p.provider == LitProvider::SemanticScholar).unwrap();
    assert_eq!(s2.queries_skipped_budget, 1);
}

#[test]
fn resharpened_brief_invalidates_stale_round1_perspective_bindings() {
    let events = vec![
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(2, IDEA_BRIEF_EVENT, json!({
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(3, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 0},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 0}
            ]
        })),
        // Second idea.brief replaces perspectives after revision: old perspective answers are stale!
        mk_event(4, IDEA_BRIEF_EVENT, json!({
            "perspectives": [{"id": "p-10", "kind": "exact"}, {"id": "p-11", "kind": "other_names"}]
        })),
    ];

    let cov = compute_coverage_from_events(&events);
    assert!(!cov.gate.passed);
    assert!(!cov.gate.exact_perspective_answered);
    assert!(!cov.gate.other_names_perspective_answered);
}
