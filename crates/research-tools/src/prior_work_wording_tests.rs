use super::*;
use crate::lit_providers::LitProvider;
use crate::paper_key::PaperKey;
use crate::prior_work::{EvidenceId, OverlapRelation};
use crate::search_coverage::{
    compute_coverage, CorePaperGap, CoverageInput, ExplorePreset, GateFailure, LibraryCoverage,
    PerspectiveKind, ProviderCoverage, ProviderCoverageState, QueryCoverageRecord,
    QueryOutcomeStatus, UnresolvedCorePaper, NOT_SEARCHED_DEFAULT,
};

fn key(id: &str) -> PaperKey {
    PaperKey::from_arxiv(id).unwrap()
}

fn sample_coverage(preset: ExplorePreset, fail_all: bool) -> SearchCoverage {
    let mut input = CoverageInput {
        preset,
        provider_states: vec![
            (LitProvider::SemanticScholar, ProviderCoverageState::NotConfigured),
            (LitProvider::OpenAlex, ProviderCoverageState::Disabled),
            (LitProvider::Crossref, ProviderCoverageState::NotAttempted),
        ],
        perspectives: vec![
            ("p-1".into(), PerspectiveKind::Exact),
            ("p-2".into(), PerspectiveKind::OtherNames),
        ],
        library: LibraryCoverage {
            searched: true,
            papers: 412,
            with_full_text: 380,
            hits: 3,
        },
        ..Default::default()
    };
    if fail_all {
        input.queries = vec![
            QueryCoverageRecord {
                qid: "q-1".into(),
                provider: LitProvider::Arxiv,
                perspective_id: Some("p-1".into()),
                perspective_kind: Some(PerspectiveKind::Exact),
                status: QueryOutcomeStatus::Failed,
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
                status: QueryOutcomeStatus::RateLimited,
                hits: 0,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
        ];
        input.user_seeds = vec![key("2401.00009")];
    } else {
        input.queries = vec![
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
                hits: 4,
                cached: false,
                is_seed_lookup: false,
                found_papers: vec![],
            },
        ];
    }
    compute_coverage(&input)
}

#[test]
fn static_wording_constants_never_contain_forbidden_words() {
    for &c in wording::ALL_STATIC_CONSTANTS {
        assert_eq!(contains_forbidden_prior_work_word(c), None, "constant `{c}`");
        assert!(check_model_prose_phrases(c).is_ok(), "V8 hit on `{c}`");
    }
    for &item in NOT_SEARCHED_DEFAULT {
        assert_eq!(contains_forbidden_prior_work_word(item), None, "not_searched `{item}`");
    }
}

#[test]
fn exhaustive_property_generated_wording_never_calls_a_claim_new_or_novel() {
    let k1 = key("2401.00001");
    let k2 = key("2401.00002");

    let all_failures = vec![
        GateFailure::QuickPresetNoFullText,
        GateFailure::TooFewAnsweredServices { answered: 0, required: 2, answered_providers: vec![] },
        GateFailure::TooFewAnsweredServices {
            answered: 1,
            required: 2,
            answered_providers: vec![LitProvider::Arxiv],
        },
        GateFailure::AttemptedServiceFailed { provider: LitProvider::Inspire, queries_failed: 2 },
        GateFailure::AttemptedServiceRateLimited {
            provider: LitProvider::SemanticScholar,
            queries_rate_limited: 1,
        },
        GateFailure::MissingExactPerspective,
        GateFailure::MissingOtherNamesPerspective,
        GateFailure::InsufficientScreenedCandidates {
            screened: 5,
            required: 40,
            candidates_total: 60,
        },
        GateFailure::UnresolvedCorePapers {
            papers: vec![
                UnresolvedCorePaper { key: k1.clone(), reason: CorePaperGap::Unavailable },
                UnresolvedCorePaper { key: k2.clone(), reason: CorePaperGap::Failed },
                UnresolvedCorePaper { key: key("2401.00003"), reason: CorePaperGap::Unread },
            ],
        },
        GateFailure::MissedUserSeeds { missed: vec![k1.clone()] },
    ];
    for f in &all_failures {
        let s = format_gate_failure(f);
        assert_eq!(contains_forbidden_prior_work_word(&s), None, "failure text: {s}");
        assert!(check_model_prose_phrases(&s).is_ok(), "V8 hit on failure text: {s}");
    }

    for &state in &[
        ProviderCoverageState::Ok,
        ProviderCoverageState::Partial,
        ProviderCoverageState::Failed,
        ProviderCoverageState::RateLimited,
        ProviderCoverageState::NotConfigured,
        ProviderCoverageState::Disabled,
        ProviderCoverageState::NotAttempted,
    ] {
        for hits in [0, 1, 3] {
            for prov in LitProvider::ALL {
                let pc = ProviderCoverage {
                    provider: prov,
                    state,
                    queries_ok: usize::from(state.is_answered()),
                    queries_zero_results: usize::from(state.is_answered() && hits == 0),
                    queries_failed: usize::from(matches!(
                        state,
                        ProviderCoverageState::Failed | ProviderCoverageState::Partial
                    )),
                    queries_rate_limited: usize::from(matches!(
                        state,
                        ProviderCoverageState::RateLimited | ProviderCoverageState::Partial
                    )),
                    queries_skipped_budget: 0,
                    queries_unsupported: 0,
                    seed_lookup_queries_ok: 0,
                    cached: 0,
                    hits,
                };
                let s = format_provider_coverage(&pc);
                assert_eq!(contains_forbidden_prior_work_word(&s), None, "provider text: {s}");
            }
        }
    }

    for searched in [false, true] {
        let lib = format_library_coverage(&LibraryCoverage {
            searched,
            papers: 120,
            with_full_text: 90,
            hits: 4,
        });
        assert_eq!(contains_forbidden_prior_work_word(&lib), None, "lib text: {lib}");
    }

    for preset in ExplorePreset::ALL {
        for fail_all in [false, true] {
            let cov = sample_coverage(preset, fail_all);
            for status in PriorWorkStatus::ALL {
                for confirmed in [false, true] {
                    for ocr in [false, true] {
                        let overlaps = vec![AcceptedOverlap {
                            paper_key: k1.clone(),
                            relation: OverlapRelation::OtherName,
                            evidence_ids: vec![EvidenceId::FullText(10)],
                            difference: "Uses open boundary conditions.".into(),
                            other_name: Some("topological flux sector".into()),
                            ocr_sourced: ocr,
                        }];
                        let abs = vec![AbstractMatch {
                            paper_key: k2.clone(),
                            evidence_id: EvidenceId::Abstract(11),
                            quote: "We compute the exact entanglement entropy.".into(),
                            provider: Some(LitProvider::Arxiv),
                        }];
                        let chip = status_chip(status, confirmed);
                        let sent =
                            format_verdict_sentence(status, confirmed, &overlaps, &abs, &cov);
                        assert_eq!(contains_forbidden_prior_work_word(chip), None, "chip: {chip}");
                        assert_eq!(contains_forbidden_prior_work_word(&sent), None, "sent: {sent}");
                        assert!(check_model_prose_phrases(&sent).is_ok(), "V8 phrase in: {sent}");
                    }
                }
            }
        }
    }
}

#[test]
fn wording_discloses_scope_zero_results_vs_no_answer_and_failing_conditions() {
    let cov_ok = sample_coverage(ExplorePreset::Standard, false);
    let sent_ok = format_verdict_sentence(PriorWorkStatus::NoneInScope, false, &[], &[], &cov_ok);
    assert!(sent_ok.contains("Not found in the searched scope."));
    assert!(sent_ok.contains("arXiv: answered (0 results, 1 query)"));
    assert!(sent_ok.contains("INSPIRE-HEP: answered (4 hits, 1 query)"));
    assert!(sent_ok.contains("Semantic Scholar: not configured"));
    assert!(sent_ok.contains("OpenAlex: disabled"));
    assert!(sent_ok.contains("Crossref: no answer (not queried)"));
    assert!(sent_ok.contains("your library: 412 papers, 380 with full text, 3 hits"));
    assert!(sent_ok.contains("does not count toward the external service minimum"));
    assert!(sent_ok.contains("books and monographs"));

    let cov_fail = sample_coverage(ExplorePreset::Standard, true);
    let sent_fail =
        format_verdict_sentence(PriorWorkStatus::Incomplete, false, &[], &[], &cov_fail);
    assert!(sent_fail.contains("Unsatisfied conditions:"));
    assert!(sent_fail.contains("attempted service arXiv failed"));
    assert!(sent_fail.contains("attempted service INSPIRE-HEP was rate-limited"));
    assert!(sent_fail.contains("exact-match perspective had no answered query"));
    assert!(sent_fail.contains("alternative-name (other_names) perspective had no answered query"));
    assert!(sent_fail.contains("arxiv:2401.00009"));
}

#[test]
fn quick_failed_search_uses_search_incomplete_and_abstract_only_discloses_failing_gate() {
    // Quick preset when all services failed must NOT claim "No close match among 0 abstracts" (M5).
    let quick_fail = sample_coverage(ExplorePreset::Quick, true);
    let s_qf = format_verdict_sentence(PriorWorkStatus::Incomplete, false, &[], &[], &quick_fail);
    assert!(s_qf.starts_with("Search incomplete;"));
    assert!(!s_qf.contains("No close match among 0 abstracts"));

    // AbstractOnly on an incomplete search discloses failing gate conditions too (M5).
    let abs = vec![AbstractMatch {
        paper_key: PaperKey::from_doi("10.1000/new.2024").unwrap(),
        evidence_id: EvidenceId::Abstract(7),
        quote: "Exact scaling of the correlation length.".into(),
        provider: Some(LitProvider::Crossref),
    }];
    let s_abs =
        format_verdict_sentence(PriorWorkStatus::AbstractOnly, false, &[], &abs, &quick_fail);
    assert!(s_abs.contains("Search scope is also incomplete:"));
    assert!(s_abs.contains("attempted service arXiv failed"));
    // Real DOI containing "new" must not trigger a false positive in generated prose (M5).
    assert_eq!(contains_forbidden_prior_work_word(&s_abs), None);
}

#[test]
fn singular_pluralization_and_not_attempted_details() {
    let pc_one = ProviderCoverage {
        provider: LitProvider::Arxiv,
        state: ProviderCoverageState::Ok,
        queries_ok: 1,
        queries_zero_results: 0,
        queries_failed: 0,
        queries_rate_limited: 0,
        queries_skipped_budget: 0,
        queries_unsupported: 0,
        seed_lookup_queries_ok: 0,
        cached: 0,
        hits: 1,
    };
    assert_eq!(format_provider_coverage(&pc_one), "arXiv: answered (1 hit, 1 query)");

    let pc_na = ProviderCoverage {
        provider: LitProvider::Crossref,
        state: ProviderCoverageState::NotAttempted,
        queries_ok: 0,
        queries_zero_results: 0,
        queries_failed: 0,
        queries_rate_limited: 0,
        queries_skipped_budget: 2,
        queries_unsupported: 1,
        seed_lookup_queries_ok: 1,
        cached: 0,
        hits: 0,
    };
    assert_eq!(
        format_provider_coverage(&pc_na),
        "Crossref: no answer (2 skipped due to budget, 1 unsupported, 1 seed lookup)"
    );

    let lib_one = format_library_coverage(&LibraryCoverage {
        searched: true,
        papers: 1,
        with_full_text: 1,
        hits: 1,
    });
    assert!(lib_one.contains("your library: 1 paper, 1 with full text, 1 hit"));
}

#[test]
fn abstract_wording_clips_verbatim_quote_and_caps_at_possible_relevance() {
    let long_quote = "α".repeat(350);
    let m = AbstractMatch {
        paper_key: key("2401.00001"),
        evidence_id: EvidenceId::Abstract(42),
        quote: long_quote,
        provider: Some(LitProvider::Crossref),
    };
    let note = format_abstract_evidence_note(&m);
    assert!(note.contains("xa-42"));
    assert!(note.contains("Crossref"));
    assert!(note.contains("abstract only"));
    assert!(note.contains("possible relevance"));
    assert!(note.contains("full text not read"));
    assert!(!note.contains(&"α".repeat(301)));
    assert!(note.contains(&"α".repeat(300)));
}

#[test]
fn model_attribution_and_display_scale_labels() {
    assert_eq!(format_model_text("What differs", "d = 3 only."), "What differs (model): d = 3 only.");
    assert_eq!(format_model_text("Summary (model)", "Brief note."), "Summary (model): Brief note.");
    assert_eq!(display_grade_label(DisplayGrade::ByCode), "by code");
    assert_eq!(display_grade_label(DisplayGrade::MatchedQuoteFullText), "Found");
    assert_eq!(display_grade_label(DisplayGrade::MatchedQuoteScannedPage), "Found (OCR)");
    assert_eq!(display_grade_label(DisplayGrade::MatchedQuoteAbstract), "abstract only");
    assert_eq!(display_grade_border(DisplayGrade::MatchedQuoteFullText), BorderStyle::Solid);
    assert_eq!(display_grade_border(DisplayGrade::MatchedQuoteAbstract), BorderStyle::Dashed);
    assert_eq!(display_grade_border(DisplayGrade::ModelStatement), BorderStyle::Dashed);
    assert_eq!(display_grade_border(DisplayGrade::UserStatement), BorderStyle::Plain);
}

#[test]
fn idea_prose_rejects_absolute_phrases_and_unicode_bypasses_and_allows_first_calculation() {
    let bad_samples = [
        "This has not been done before.",
        "Such a construction has   never been studied.",
        "There is no prior work on this operator.",
        "Nobody has computed this invariant.",
        "We are the first to derive the anomaly.",
        "Obtained for the first time in 3+1 dimensions.",
        "A novel method for lattice fermions.",
        "An unexplored regime of the phase diagram.",
        // Unicode bypass attempts (M5): ZWSP, soft hyphen, combining mark, ligature.
        "A no\u{200b}vel method for lattice fermions.",
        "An un\u{00ad}explored regime.",
        "We are the \u{fb01}rst to derive this.",
        "Obtained for the \u{fb01}rst time.",
        "A nove\u{0301}l construction.",
    ];
    for sample in bad_samples {
        assert!(check_model_prose_phrases(sample).is_err(), "expected V8 rejection for `{sample}`");
    }

    let ok_samples = [
        "First calculation: evaluate the one-loop trace at N = 4.",
        "Introduce new coordinates (u, v) to diagonalize the metric.",
        "The author's claims of novelty in section 2 apply only to d = 2.",
        "Not found in the searched scope (arXiv and INSPIRE-HEP).",
    ];
    for sample in ok_samples {
        assert_eq!(check_model_prose_phrases(sample), Ok(()), "expected V8 acceptance for `{sample}`");
    }
}
