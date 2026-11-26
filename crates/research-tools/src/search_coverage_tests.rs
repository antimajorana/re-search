use super::*;
use crate::lit_providers::{LitProvider, QueryStatus};
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use ledger::{Event, GENESIS_HASH};
use serde_json::json;

fn key_arxiv(id: &str) -> PaperKey {
    PaperKey::from_arxiv(id).unwrap()
}

fn key_doi(doi: &str) -> PaperKey {
    PaperKey::from_doi(doi).unwrap()
}

fn paper_ref_arxiv(id: &str, doi: Option<&str>) -> PaperRef {
    PaperRef::from_ids(
        PaperIds {
            arxiv: Some(id.to_string()),
            doi: doi.map(str::to_string),
            inspire: None,
            s2: None,
        },
        None,
    )
    .unwrap()
}

fn passing_standard_input() -> CoverageInput {
    let p1 = paper_ref_arxiv("2401.00001", Some("10.1000/test.1"));
    let p2 = paper_ref_arxiv("2401.00002", None);
    CoverageInput {
        preset: ExplorePreset::Standard,
        screened_limit_override: None,
        screened_count_override: None,
        provider_states: vec![
            (LitProvider::SemanticScholar, ProviderCoverageState::NotConfigured),
            (LitProvider::OpenAlex, ProviderCoverageState::Disabled),
        ],
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
                hits: 2,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![p1.clone()],
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
                found_papers: vec![p2.clone()],
            },
        ],
        library: LibraryCoverage {
            searched: true,
            papers: 412,
            with_full_text: 380,
            hits: 2,
        },
        candidates: vec![
            CandidateCoverageRecord {
                paper: p1.clone(),
                year: Some(2024),
                found_by: vec!["q-1".into()],
                user_seed: true,
            },
            CandidateCoverageRecord {
                paper: p2.clone(),
                year: Some(2022),
                found_by: vec!["q-2".into()],
                user_seed: false,
            },
        ],
        triage: vec![
            TriageCoverageRecord { key: p1.key.clone(), relevance: TriageRelevance::Core },
            TriageCoverageRecord { key: p2.key.clone(), relevance: TriageRelevance::Related },
        ],
        paper_reads: vec![PaperReadCoverageRecord {
            key: p1.key.clone(),
            status: PaperReadStatus::ReadPassages,
        }],
        dismissed_core: BTreeSet::new(),
        user_seeds: vec![p1.key.clone()],
        citation_expansion: CitationExpansionCoverage { seeds: 0, hops: 0 },
    }
}

fn mk_event(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-cov".into(),
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
fn preset_limits_match_explore_design_table() {
    let q = ExplorePreset::Quick.limits();
    assert_eq!((q.max_queries, q.max_screened, q.max_papers, q.calls_per_paper, q.typical_model_calls, q.max_model_calls), (8, 25, 0, 0, 3, 5));
    assert!(!ExplorePreset::Quick.allows_none_in_scope());

    let s = ExplorePreset::Standard.limits();
    assert_eq!((s.max_queries, s.max_screened, s.max_papers, s.calls_per_paper, s.typical_model_calls, s.max_model_calls), (18, 40, 6, 3, 15, 24));
    assert!(ExplorePreset::Standard.allows_none_in_scope());

    let t = ExplorePreset::Thorough.limits();
    assert_eq!((t.max_queries, t.max_screened, t.max_papers, t.calls_per_paper, t.typical_model_calls, t.max_model_calls), (36, 80, 12, 4, 35, 55));
    assert!(ExplorePreset::Thorough.allows_none_in_scope());
}

#[test]
fn query_status_from_f11_maps_every_variant() {
    assert_eq!(QueryOutcomeStatus::from(QueryStatus::Ok), QueryOutcomeStatus::Ok);
    assert_eq!(QueryOutcomeStatus::from(QueryStatus::Failed), QueryOutcomeStatus::Failed);
    assert_eq!(QueryOutcomeStatus::from(QueryStatus::RateLimited), QueryOutcomeStatus::RateLimited);
    assert_eq!(QueryOutcomeStatus::from(QueryStatus::NotConfigured), QueryOutcomeStatus::NotConfigured);
    assert_eq!(QueryOutcomeStatus::from(QueryStatus::Disabled), QueryOutcomeStatus::Disabled);
    assert_eq!(QueryOutcomeStatus::from(QueryStatus::Unsupported), QueryOutcomeStatus::Unsupported);
}

#[test]
fn search_coverage_gate_not_configured_is_listed_not_failed() {
    let cov = compute_coverage(&passing_standard_input());
    assert!(cov.gate.passed, "unexpected failures: {:?}", cov.gate.unsatisfied);
    assert!(cov.gate.unsatisfied.is_empty());
    assert_eq!(cov.gate.answered_external_services, 2);
    assert_eq!(cov.year_range, Some([2022, 2024]));
    assert!(cov.providers.iter().any(|p| {
        p.provider == LitProvider::SemanticScholar && p.state == ProviderCoverageState::NotConfigured
    }));
    assert!(cov.providers.iter().any(|p| {
        p.provider == LitProvider::OpenAlex && p.state == ProviderCoverageState::Disabled
    }));
    assert_eq!(cov.not_searched.len(), NOT_SEARCHED_DEFAULT.len());
}

#[test]
fn search_coverage_gate_quick_preset_never_passes() {
    let mut input = passing_standard_input();
    input.preset = ExplorePreset::Quick;
    let cov = compute_coverage(&input);
    assert!(!cov.gate.passed);
    assert!(!cov.gate.preset_allows_none_in_scope);
    assert_eq!(cov.gate.unsatisfied, vec![GateFailure::QuickPresetNoFullText]);
}

#[test]
fn search_coverage_gate_private_library_never_counts_as_external_service() {
    let mut input = passing_standard_input();
    // Keep only arXiv answering both perspectives; give private library 500 papers and 50 hits.
    input.queries[1].provider = LitProvider::Arxiv;
    input.library = LibraryCoverage {
        searched: true,
        papers: 500,
        with_full_text: 500,
        hits: 50,
    };
    let cov = compute_coverage(&input);
    assert!(!cov.gate.passed);
    assert!(!cov.gate.services_minimum_met);
    assert_eq!(cov.gate.answered_external_services, 1);
    assert_eq!(cov.library.papers, 500);
    assert_eq!(cov.library.hits, 50);
    assert!(cov.gate.unsatisfied.iter().any(|f| matches!(
        f,
        GateFailure::TooFewAnsweredServices { answered: 1, required: 2, .. }
    )));
}

#[test]
fn search_coverage_gate_fails_on_failed_or_rate_limited_service() {
    // Partial failure on an attempted service (1 ok + 1 failed) still fails gate.
    let mut failed = passing_standard_input();
    failed.queries.push(QueryCoverageRecord {
        qid: "q-3".into(),
        provider: LitProvider::Crossref,
        perspective_id: Some("p-1".into()),
        perspective_kind: Some(PerspectiveKind::Exact),
        status: QueryOutcomeStatus::Failed,
        hits: 0,
        cached: false,
        is_seed_lookup: false,
        found_papers: vec![],
    });
    let cov_f = compute_coverage(&failed);
    assert!(!cov_f.gate.passed);
    assert!(!cov_f.gate.no_attempted_service_failed);
    assert!(cov_f.gate.unsatisfied.contains(&GateFailure::AttemptedServiceFailed {
        provider: LitProvider::Crossref,
        queries_failed: 1,
    }));

    // Rate-limited query on INSPIRE (even though another INSPIRE query succeeded -> Partial).
    let mut rl = passing_standard_input();
    rl.queries.push(QueryCoverageRecord {
        qid: "q-4".into(),
        provider: LitProvider::Inspire,
        perspective_id: Some("p-2".into()),
        perspective_kind: Some(PerspectiveKind::OtherNames),
        status: QueryOutcomeStatus::RateLimited,
        hits: 0,
        cached: false,
        is_seed_lookup: false,
        found_papers: vec![],
    });
    let cov_r = compute_coverage(&rl);
    assert!(!cov_r.gate.passed);
    assert!(!cov_r.gate.no_attempted_service_failed);
    let ins = cov_r.providers.iter().find(|p| p.provider == LitProvider::Inspire).unwrap();
    assert_eq!(ins.state, ProviderCoverageState::Partial);
    assert!(cov_r.gate.unsatisfied.contains(&GateFailure::AttemptedServiceRateLimited {
        provider: LitProvider::Inspire,
        queries_rate_limited: 1,
    }));
}

#[test]
fn search_coverage_gate_requires_exact_and_other_names_perspectives() {
    let mut no_exact = passing_standard_input();
    no_exact.queries[0].perspective_id = Some("p-2".into());
    no_exact.queries[0].perspective_kind = Some(PerspectiveKind::OtherNames);
    let cov = compute_coverage(&no_exact);
    assert!(!cov.gate.passed);
    assert!(!cov.gate.exact_perspective_answered);
    assert!(cov.gate.other_names_perspective_answered);
    assert!(cov.gate.unsatisfied.contains(&GateFailure::MissingExactPerspective));

    let mut no_other = passing_standard_input();
    no_other.queries[1].perspective_id = Some("p-1".into());
    no_other.queries[1].perspective_kind = Some(PerspectiveKind::Exact);
    let cov2 = compute_coverage(&no_other);
    assert!(!cov2.gate.passed);
    assert!(cov2.gate.exact_perspective_answered);
    assert!(!cov2.gate.other_names_perspective_answered);
    assert!(cov2.gate.unsatisfied.contains(&GateFailure::MissingOtherNamesPerspective));
}

#[test]
fn search_coverage_gate_requires_enough_screened_candidates() {
    let mut under = passing_standard_input();
    // 2 candidates total, min(40, 2) = 2, but only 1 triaged.
    under.triage.pop();
    let cov = compute_coverage(&under);
    assert!(!cov.gate.passed);
    assert!(!cov.gate.enough_candidates_screened);
    assert!(cov.gate.unsatisfied.contains(&GateFailure::InsufficientScreenedCandidates {
        screened: 1,
        required: 2,
        candidates_total: 2,
    }));

    // With 50 candidates on Standard preset (max_screened = 40), screening 40 passes, 39 fails.
    let mut large = passing_standard_input();
    for i in 3..=50 {
        let p = paper_ref_arxiv(&format!("2401.{i:05}"), None);
        large.candidates.push(CandidateCoverageRecord {
            paper: p,
            year: Some(2024),
            found_by: vec!["q-1".into()],
            user_seed: false,
        });
    }
    large.screened_count_override = Some(39);
    assert!(!compute_coverage(&large).gate.enough_candidates_screened);
    large.screened_count_override = Some(40);
    assert!(compute_coverage(&large).gate.passed);
}

#[test]
fn search_coverage_gate_core_paper_read_or_explicit_dismissal() {
    let core_key = key_arxiv("2401.00001");

    // Unavailable core paper fails gate.
    let mut unavail = passing_standard_input();
    unavail.paper_reads[0].status = PaperReadStatus::Unavailable;
    let cov_u = compute_coverage(&unavail);
    assert!(!cov_u.gate.passed);
    assert!(!cov_u.gate.all_core_papers_resolved);
    assert_eq!(cov_u.unavailable_core, vec![core_key.clone()]);
    assert!(cov_u.gate.unsatisfied.contains(&GateFailure::UnresolvedCorePapers {
        papers: vec![UnresolvedCorePaper { key: core_key.clone(), reason: CorePaperGap::Unavailable }],
    }));

    // Explicit reader dismissal resolves unavailable core paper.
    unavail.dismissed_core.insert(core_key.clone());
    let cov_d = compute_coverage(&unavail);
    assert!(cov_d.gate.passed);
    assert_eq!(cov_d.dismissed_core, vec![core_key.clone()]);

    // NothingRelevant (full text read and found irrelevant) resolves core paper without dismissal.
    let mut nr = passing_standard_input();
    nr.paper_reads[0].status = PaperReadStatus::NothingRelevant;
    assert!(compute_coverage(&nr).gate.passed);

    // Failed reading or unread core paper fails gate.
    let mut unread = passing_standard_input();
    unread.paper_reads.clear();
    let cov_nr = compute_coverage(&unread);
    assert!(!cov_nr.gate.passed);
    assert!(cov_nr.gate.unsatisfied.contains(&GateFailure::UnresolvedCorePapers {
        papers: vec![UnresolvedCorePaper { key: core_key, reason: CorePaperGap::Unread }],
    }));
}

#[test]
fn search_coverage_gate_seed_recall_and_alias_matching() {
    // Missing user seed fails.
    let mut missed = passing_standard_input();
    let missing_seed = key_arxiv("2312.99999");
    missed.user_seeds.push(missing_seed.clone());
    let cov_m = compute_coverage(&missed);
    assert!(!cov_m.gate.passed);
    assert!(!cov_m.gate.all_seeds_found);
    assert_eq!(cov_m.known_paper_recall.missed, vec![missing_seed.clone()]);
    assert!(cov_m.gate.unsatisfied.contains(&GateFailure::MissedUserSeeds {
        missed: vec![missing_seed],
    }));

    // A direct seed lookup (`is_seed_lookup: true`) must NEVER credit search recall.
    let mut lookup_only = passing_standard_input();
    lookup_only.queries[0].is_seed_lookup = true;
    lookup_only.candidates[0].found_by = vec!["q-1".into()];
    let cov_l = compute_coverage(&lookup_only);
    assert!(!cov_l.gate.all_seeds_found);
    assert_eq!(cov_l.known_paper_recall.missed, vec![key_arxiv("2401.00001")]);

    // Seed given as DOI matches arXiv candidate carrying that DOI alias (F2 PaperRef).
    let mut doi_seed = passing_standard_input();
    doi_seed.user_seeds = vec![key_doi("10.1000/test.1")];
    let cov_a = compute_coverage(&doi_seed);
    assert!(cov_a.gate.passed);
    assert!(cov_a.gate.all_seeds_found);
    assert_eq!(cov_a.known_paper_recall.found_by_search, vec![key_doi("10.1000/test.1")]);
}

#[test]
fn distinguishes_zero_results_from_no_answer() {
    // Two services cleanly answered with 0 results on exact and other_names perspectives.
    let zero_ok = CoverageInput {
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
                hits: 0,
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
                hits: 0,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
        ],
        ..Default::default()
    };
    let cov_zero = compute_coverage(&zero_ok);
    assert!(cov_zero.gate.passed);
    assert!(cov_zero.zero_candidates_from_answered_search);
    assert!(cov_zero.providers.iter().all(|p| p.answered() && p.answered_with_zero_results() && !p.has_no_answer()));

    // Contrast: two services skipped / failed -> no answer!
    let no_answer = CoverageInput {
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
                status: QueryOutcomeStatus::SkippedBudget,
                hits: 0,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
            QueryCoverageRecord {
                qid: "q-2".into(),
                provider: LitProvider::Inspire,
                perspective_id: Some("p-2".into()),
                perspective_kind: Some(PerspectiveKind::OtherNames),
                status: QueryOutcomeStatus::Failed,
                hits: 0,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
        ],
        ..Default::default()
    };
    let cov_na = compute_coverage(&no_answer);
    assert!(!cov_na.gate.passed);
    assert!(!cov_na.zero_candidates_from_answered_search);
    assert!(cov_na.providers.iter().all(|p| !p.answered() && p.has_no_answer()));
    // Every unsatisfied condition is named simultaneously (no short-circuiting).
    assert!(cov_na.gate.unsatisfied.len() >= 4);
    assert!(cov_na.gate.unsatisfied.contains(&GateFailure::MissingExactPerspective));
    assert!(cov_na.gate.unsatisfied.contains(&GateFailure::MissingOtherNamesPerspective));
}

#[test]
fn search_coverage_recomputes_from_ledger_and_ignores_lessons_and_ranking_marks() {
    let base_events = vec![
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(2, IDEA_STATED_EVENT, json!({"seeds": ["arxiv:2401.00001"]})),
        mk_event(3, IDEA_BRIEF_EVENT, json!({
            "perspectives": [
                {"id": "p-1", "kind": "exact"},
                {"id": "p-2", "kind": "other_names"}
            ]
        })),
        mk_event(4, IDEA_SEARCH_PLANNED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "origin": "user", "relation": "none"},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "origin": "model", "relation": "none"}
            ]
        })),
        mk_event(5, IDEA_SEARCH_FINISHED_EVENT, json!({
            "providers": [{"provider": "semantic_scholar", "state": "not_configured"}],
            "library": {"searched": true, "papers": 100, "with_full_text": 80, "hits": 1},
            "queries": [
                {"qid": "q-1", "provider": "arxiv", "status": "ok", "hits": 1},
                {"qid": "q-2", "provider": "inspire", "status": "ok", "hits": 1}
            ]
        })),
        mk_event(6, IDEA_CANDIDATES_EVENT, json!({
            "candidates": [
                {"key": "arxiv:2401.00001", "ids": {"arxiv": "2401.00001"}, "year": 2024, "found_by": ["q-1"], "user_seed": true},
                {"key": "arxiv:2401.00002", "ids": {"arxiv": "2401.00002"}, "year": 2023, "found_by": ["q-2"], "user_seed": false}
            ]
        })),
        mk_event(7, IDEA_TRIAGE_EVENT, json!({
            "items": [
                {"key": "arxiv:2401.00001", "relevance": "core"},
                {"key": "arxiv:2401.00002", "relevance": "related"}
            ]
        })),
        mk_event(8, IDEA_PAPER_EVENT, json!({"key": "arxiv:2401.00001", "status": "read_full"})),
    ];

    let cov1 = compute_coverage_from_events(&base_events);
    assert!(cov1.gate.passed, "failures: {:?}", cov1.gate.unsatisfied);

    // Add F16 lessons and ranking marks: coverage and gate MUST be bit-for-bit identical (rule V10).
    let mut with_lessons = base_events.clone();
    with_lessons.push(mk_event(9, "lessons.supplied", json!({"run_id": "r1", "lesson_ids": ["l-1"]})));
    with_lessons.push(mk_event(10, IDEA_NOTE_EVENT, json!({"target": "arxiv:2401.00001", "action": "not_relevant"})));
    with_lessons.push(mk_event(11, IDEA_NOTE_EVENT, json!({"target": "arxiv:2401.00002", "action": "key_paper"})));
    let cov2 = compute_coverage_from_events(&with_lessons);
    assert_eq!(cov1, cov2);

    // Serde round-trip preserves exact structure.
    let json = serde_json::to_string(&cov1).unwrap();
    let roundtrip: SearchCoverage = serde_json::from_str(&json).unwrap();
    assert_eq!(roundtrip, cov1);
}
