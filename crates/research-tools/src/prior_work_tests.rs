use super::*;
use crate::lit_providers::LitProvider;
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use crate::quote_core::EVIDENCE_QUOTE_EVENT;
use crate::search_coverage::{
    compute_coverage, CandidateCoverageRecord, CoverageInput, ExplorePreset, LibraryCoverage,
    PaperReadCoverageRecord, PaperReadStatus, PerspectiveKind, QueryCoverageRecord,
    QueryOutcomeStatus, TriageCoverageRecord, TriageRelevance, IDEA_BRIEF_EVENT,
    IDEA_CANDIDATES_EVENT, IDEA_NOTE_EVENT, IDEA_PAPER_EVENT, IDEA_SEARCH_FINISHED_EVENT,
    IDEA_SEARCH_PLANNED_EVENT, IDEA_STARTED_EVENT, IDEA_STATED_EVENT, IDEA_TRIAGE_EVENT,
};
use ledger::{Event, GENESIS_HASH};
use serde_json::json;

fn key_arxiv(id: &str) -> PaperKey {
    PaperKey::from_arxiv(id).unwrap()
}

fn paper_ref_arxiv(id: &str) -> PaperRef {
    PaperRef::from_ids(
        PaperIds { arxiv: Some(id.to_string()), doi: None, inspire: None, s2: None },
        None,
    )
    .unwrap()
}

fn passing_coverage(preset: ExplorePreset) -> SearchCoverage {
    let p1 = paper_ref_arxiv("2401.00001");
    let p2 = paper_ref_arxiv("2401.00002");
    compute_coverage(&CoverageInput {
        preset,
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
        library: LibraryCoverage { searched: true, papers: 50, with_full_text: 40, hits: 1 },
        candidates: vec![
            CandidateCoverageRecord { paper: p1.clone(), year: Some(2024), found_by: vec!["q-1".into()], user_seed: false },
            CandidateCoverageRecord { paper: p2.clone(), year: Some(2023), found_by: vec!["q-2".into()], user_seed: false },
        ],
        triage: vec![
            TriageCoverageRecord { key: p1.key.clone(), relevance: TriageRelevance::Core },
            TriageCoverageRecord { key: p2.key.clone(), relevance: TriageRelevance::Related },
        ],
        paper_reads: vec![PaperReadCoverageRecord {
            key: p1.key,
            status: PaperReadStatus::ReadPassages,
        }],
        ..Default::default()
    })
}

fn mk_event(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-pw".into(),
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
fn status_wire_names_and_evidence_id_parsing() {
    let expected = [
        (PriorWorkStatus::JudgedSame, "\"judged_same\""),
        (PriorWorkStatus::JudgedPartial, "\"judged_partial\""),
        (PriorWorkStatus::JudgedOtherName, "\"judged_other_name\""),
        (PriorWorkStatus::AbstractOnly, "\"abstract_only\""),
        (PriorWorkStatus::NoneInScope, "\"none_in_scope\""),
        (PriorWorkStatus::Incomplete, "\"incomplete\""),
    ];
    for (st, wire) in expected {
        let s = serde_json::to_string(&st).unwrap();
        assert_eq!(s, wire);
        assert_eq!(serde_json::from_str::<PriorWorkStatus>(&s).unwrap(), st);
    }

    assert_eq!(EvidenceId::parse("xe-12").unwrap(), EvidenceId::FullText(12));
    assert_eq!(EvidenceId::parse("xa-7").unwrap(), EvidenceId::Abstract(7));
    assert!(EvidenceId::parse("xe-0").is_err());
    assert!(EvidenceId::parse("xe-01").is_err());
    assert!(EvidenceId::parse("e-1").is_err());
    let round: EvidenceId = serde_json::from_str(&serde_json::to_string(&EvidenceId::FullText(9)).unwrap()).unwrap();
    assert_eq!(round, EvidenceId::FullText(9));
}

#[test]
fn idea_verdict_judged_requires_read_paper_and_full_text_evidence() {
    let p1 = key_arxiv("2401.00001");
    let p2 = key_arxiv("2401.00002");
    let ft1 = FullTextEvidence {
        id: EvidenceId::FullText(10),
        paper_key: p1.clone(),
        quote: "Theorem 1 establishes the exact bound on the spectral gap.".into(),
        excerpt: "Theorem 1 establishes the exact bound on the spectral gap.".into(),
        quote_ref: None,
        text_basis: TextBasis::Tex,
        subclaim_ids: vec!["sc-1".into()],
        role: Some(EvidenceRole::StatesResult),
    };

    // Paper unread -> overlap rejected -> falls through to NoneInScope (since coverage passed).
    let mut input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into()],
        coverage: passing_coverage(ExplorePreset::Standard),
        paper_reads: BTreeMap::from([(p1.clone(), PaperReadStatus::Unavailable)]),
        paper_text_basis: BTreeMap::new(),
        full_text_evidence: vec![ft1.clone()],
        abstract_evidence: vec![],
        overlaps: vec![OverlapCandidate {
            subclaim_id: "sc-1".into(),
            paper_key: p1.clone(),
            relation: OverlapRelation::Same,
            evidence_ids: vec![EvidenceId::FullText(10)],
            difference: "Same bound under slightly weaker boundary conditions.".into(),
            other_name: None,
        }],
        confirmed_subclaims: BTreeSet::new(),
        origin_paper: None,
    };
    let v_unread = &evaluate_prior_work(&input)[0];
    assert_eq!(v_unread.status, PriorWorkStatus::NoneInScope);
    assert_eq!(v_unread.rejected_overlaps.len(), 1);
    assert!(matches!(
        v_unread.rejected_overlaps[0].reason,
        OverlapRejectionReason::PaperNotRead { status: Some(PaperReadStatus::Unavailable) }
    ));

    // Paper read_passages + mismatched paper key on evidence -> rejected.
    input.paper_reads.insert(p2.clone(), PaperReadStatus::ReadPassages);
    input.overlaps[0].paper_key = p2.clone();
    let v_mismatch = &evaluate_prior_work(&input)[0];
    assert_eq!(v_mismatch.status, PriorWorkStatus::NoneInScope);
    assert!(matches!(
        v_mismatch.rejected_overlaps[0].reason,
        OverlapRejectionReason::EvidencePaperMismatch { evidence_id: EvidenceId::FullText(10) }
    ));

    // Paper read_passages + matching full-text evidence -> JudgedSame, unconfirmed by default!
    input.paper_reads.insert(p1.clone(), PaperReadStatus::ReadPassages);
    input.overlaps[0].paper_key = p1;
    let v_ok = &evaluate_prior_work(&input)[0];
    assert_eq!(v_ok.status, PriorWorkStatus::JudgedSame);
    assert!(!v_ok.confirmed_by_user);
    assert_eq!(v_ok.display_grade, DisplayGrade::ModelStatement);
    assert_eq!(v_ok.border_style, BorderStyle::Dashed);
    assert!(v_ok.chip.starts_with("Model judges:"));
    assert!(v_ok.text.starts_with("Model judges:"));

    // Reader confirmation flips attribution and display grade.
    input.confirmed_subclaims.insert("sc-1".into());
    let v_conf = &evaluate_prior_work(&input)[0];
    assert_eq!(v_conf.status, PriorWorkStatus::JudgedSame);
    assert!(v_conf.confirmed_by_user);
    assert_eq!(v_conf.display_grade, DisplayGrade::MatchedQuoteFullText);
    assert_eq!(v_conf.border_style, BorderStyle::Solid);
    assert_eq!(v_conf.chip, "found (same)");
}

#[test]
fn idea_verdict_other_name_needs_term_in_full_text_quote() {
    let p1 = key_arxiv("2401.00001");
    let mut input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into()],
        coverage: passing_coverage(ExplorePreset::Standard),
        paper_reads: BTreeMap::from([(p1.clone(), PaperReadStatus::ReadFull)]),
        paper_text_basis: BTreeMap::new(),
        full_text_evidence: vec![FullTextEvidence {
            id: EvidenceId::FullText(15),
            paper_key: p1.clone(),
            quote: "We study the gauge-invariant flux sector on a torus.".into(),
            excerpt: "We study the gauge-invariant flux sector on a torus.".into(),
            quote_ref: None,
            text_basis: TextBasis::Tex,
            subclaim_ids: vec!["sc-1".into()],
            role: Some(EvidenceRole::StatesResult),
        }],
        abstract_evidence: vec![AbstractEvidence {
            id: EvidenceId::Abstract(16),
            paper_key: p1.clone(),
            quote: "This abstract mentions dual plaquette holonomy.".into(),
            verbatim: true,
            raw_blob: "a".repeat(64),
            provider: Some(LitProvider::Arxiv),
            subclaim_ids: vec![],
        }],
        overlaps: vec![OverlapCandidate {
            subclaim_id: "sc-1".into(),
            paper_key: p1.clone(),
            relation: OverlapRelation::OtherName,
            evidence_ids: vec![EvidenceId::FullText(15)],
            difference: "Called dual plaquette holonomy in that paper.".into(),
            other_name: Some("dual plaquette holonomy".into()),
        }],
        confirmed_subclaims: BTreeSet::new(),
        origin_paper: None,
    };

    // Term appears ONLY in abstract and model difference, NOT in full-text quote -> rejected!
    let v_bad = &evaluate_prior_work(&input)[0];
    assert_eq!(v_bad.status, PriorWorkStatus::NoneInScope);
    assert!(matches!(
        &v_bad.rejected_overlaps[0].reason,
        OverlapRejectionReason::OtherNameNotInFullTextQuote { term } if term == "dual plaquette holonomy"
    ));

    // Punctuation-only term -> rejected.
    input.overlaps[0].other_name = Some("---".into());
    assert_eq!(evaluate_prior_work(&input)[0].status, PriorWorkStatus::NoneInScope);

    // Term actually occurs in the full-text quote (different case / whitespace) -> accepted!
    input.overlaps[0].other_name = Some("Flux   Sector".into());
    let v_ok = &evaluate_prior_work(&input)[0];
    assert_eq!(v_ok.status, PriorWorkStatus::JudgedOtherName);
    assert_eq!(v_ok.chip, "Model judges: same under another name");
    assert!(v_ok.text.contains("Flux   Sector"));
}

#[test]
fn idea_verdict_abstract_cap_and_quick_never_claims_none_in_scope() {
    let p1 = key_arxiv("2401.00001");
    let mut input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into(), "sc-2".into()],
        coverage: passing_coverage(ExplorePreset::Quick),
        paper_reads: BTreeMap::new(),
        paper_text_basis: BTreeMap::new(),
        full_text_evidence: vec![],
        abstract_evidence: vec![
            AbstractEvidence {
                id: EvidenceId::Abstract(20),
                paper_key: p1.clone(),
                quote: "We compute the exact partition function of the planar dimer model.".into(),
                verbatim: true,
                raw_blob: "b".repeat(64),
                provider: Some(LitProvider::Arxiv),
                subclaim_ids: vec!["sc-1".into()],
            },
            // Rebuilt (non-verbatim) abstract on sc-2 must NEVER be evidence (rule C3).
            AbstractEvidence {
                id: EvidenceId::Abstract(21),
                paper_key: p1.clone(),
                quote: "Rebuilt inverted-index abstract.".into(),
                verbatim: false,
                raw_blob: "c".repeat(64),
                provider: Some(LitProvider::OpenAlex),
                subclaim_ids: vec!["sc-2".into()],
            },
        ],
        // Model attempts to claim `Same` citing only `xa-20` -> capped at AbstractOnly!
        overlaps: vec![OverlapCandidate {
            subclaim_id: "sc-1".into(),
            paper_key: p1,
            relation: OverlapRelation::Same,
            evidence_ids: vec![EvidenceId::Abstract(20)],
            difference: String::new(),
            other_name: None,
        }],
        confirmed_subclaims: BTreeSet::from(["sc-1".into()]),
        origin_paper: None,
    };

    let verdicts = evaluate_prior_work(&input);
    // sc-1: capped at AbstractOnly, never confirmed, dashed border.
    assert_eq!(verdicts[0].status, PriorWorkStatus::AbstractOnly);
    assert!(!verdicts[0].confirmed_by_user);
    assert_eq!(verdicts[0].display_grade, DisplayGrade::MatchedQuoteAbstract);
    assert_eq!(verdicts[0].border_style, BorderStyle::Dashed);
    assert!(verdicts[0].text.contains("possible relevance"));

    // sc-2: non-verbatim abstract ignored; Quick mode -> Incomplete, never NoneInScope!
    assert_eq!(verdicts[1].status, PriorWorkStatus::Incomplete);
    assert!(verdicts[1].text.contains("No close match among 2 abstracts; nothing was read"));

    // Switching preset to Standard with passing coverage makes sc-2 NoneInScope.
    input.coverage = passing_coverage(ExplorePreset::Standard);
    let verdicts_std = evaluate_prior_work(&input);
    assert_eq!(verdicts_std[1].status, PriorWorkStatus::NoneInScope);
}

#[test]
fn idea_verdict_excludes_origin_paper_and_tags_ocr_evidence() {
    let origin = key_arxiv("2401.00001");
    let scanned = key_arxiv("2401.00002");

    let input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into(), "sc-2".into()],
        coverage: passing_coverage(ExplorePreset::Standard),
        paper_reads: BTreeMap::from([
            (origin.clone(), PaperReadStatus::ReadFull),
            (scanned.clone(), PaperReadStatus::ReadFull),
        ]),
        paper_text_basis: BTreeMap::from([(scanned.clone(), TextBasis::Ocr)]),
        full_text_evidence: vec![
            FullTextEvidence {
                id: EvidenceId::FullText(30),
                paper_key: origin.clone(),
                quote: "Origin paper main theorem.".into(),
                excerpt: "Origin paper main theorem.".into(),
                quote_ref: None,
                text_basis: TextBasis::Tex,
                subclaim_ids: vec!["sc-1".into()],
                role: Some(EvidenceRole::StatesResult),
            },
            FullTextEvidence {
                id: EvidenceId::FullText(31),
                paper_key: scanned.clone(),
                quote: "Older scanned theorem matching subclaim 2.".into(),
                excerpt: "Older scanned theorem matching subclaim 2.".into(),
                quote_ref: None,
                text_basis: TextBasis::Ocr,
                subclaim_ids: vec!["sc-2".into()],
                role: Some(EvidenceRole::StatesResult),
            },
        ],
        abstract_evidence: vec![AbstractEvidence {
            id: EvidenceId::Abstract(32),
            paper_key: origin.clone(),
            quote: "Origin abstract must also be excluded.".into(),
            verbatim: true,
            raw_blob: "d".repeat(64),
            provider: Some(LitProvider::Arxiv),
            subclaim_ids: vec!["sc-1".into()],
        }],
        overlaps: vec![
            OverlapCandidate {
                subclaim_id: "sc-1".into(),
                paper_key: origin.clone(),
                relation: OverlapRelation::Same,
                evidence_ids: vec![EvidenceId::FullText(30)],
                difference: String::new(),
                other_name: None,
            },
            OverlapCandidate {
                subclaim_id: "sc-2".into(),
                paper_key: scanned,
                relation: OverlapRelation::Partial,
                evidence_ids: vec![EvidenceId::FullText(31)],
                difference: "Covers dimension d = 2 only.".into(),
                other_name: None,
            },
        ],
        confirmed_subclaims: BTreeSet::from(["sc-2".into()]),
        origin_paper: Some(origin_ref_from_key(origin)),
    };

    let verdicts = evaluate_prior_work(&input);
    // sc-1: origin paper excluded from BOTH full-text overlaps and abstract matches (rule V9).
    assert_eq!(verdicts[0].status, PriorWorkStatus::NoneInScope);
    assert!(verdicts[0].abstract_matches.is_empty());
    assert!(matches!(
        verdicts[0].rejected_overlaps[0].reason,
        OverlapRejectionReason::OriginPaperExcluded
    ));

    // sc-2: OCR-sourced full-text quote accepted, tagged OCR, confirmed -> MatchedQuoteScannedPage.
    assert_eq!(verdicts[1].status, PriorWorkStatus::JudgedPartial);
    assert!(verdicts[1].has_ocr_evidence);
    assert!(verdicts[1].confirmed_by_user);
    assert_eq!(verdicts[1].display_grade, DisplayGrade::MatchedQuoteScannedPage);
    assert!(verdicts[1].text.contains("[OCR source]"));
}

#[test]
fn compute_prior_work_from_events_recomputes_and_ignores_lessons() {
    let quote = "Exact equivalence of the two transfer matrices.";
    let qsha = crate::quote_core::quote_sha256(quote);
    let fsha = "a".repeat(64);
    let base = vec![
        mk_event(1, HANDOFF_PACKET_EVENT, json!({"from": {"paper": {"key": "arxiv:2301.00001"}}})),
        mk_event(2, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(3, IDEA_STATED_EVENT, json!({"seeds": []})),
        mk_event(4, IDEA_BRIEF_EVENT, json!({
            "subclaims": [{"id": "sc-1"}, {"id": "sc-2"}],
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(5, IDEA_SEARCH_PLANNED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "origin": "user", "relation": "none"},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "origin": "model", "relation": "none"}
            ]
        })),
        mk_event(6, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "provider": "arxiv", "status": "ok", "hits": 1},
                {"qid": "q-2", "provider": "inspire", "status": "ok", "hits": 1}
            ]
        })),
        mk_event(7, IDEA_CANDIDATES_EVENT, json!({
            "candidates": [
                {"key": "arxiv:2401.00001", "ids": {"arxiv": "2401.00001"}, "year": 2024, "found_by": ["q-1"]}
            ]
        })),
        mk_event(8, IDEA_TRIAGE_EVENT, json!({
            "items": [{"key": "arxiv:2401.00001", "relevance": "core"}]
        })),
        mk_event(9, IDEA_PAPER_EVENT, json!({
            "key": "arxiv:2401.00001", "status": "read_full", "text_basis": "tex"
        })),
        mk_event(10, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "src-1",
            "path": "src-1/main.tex",
            "start_line": 10,
            "end_line": 12,
            "file_sha256": fsha,
            "quote_sha256": qsha,
            "paper_key": "arxiv:2401.00001",
            "quote": quote,
            "excerpt": quote,
            "subclaim_ids": ["sc-1"]
        })),
        mk_event(11, IDEA_MAP_EVENT, json!({
            "overlaps": [{
                "subclaim_id": "sc-1",
                "paper_key": "arxiv:2401.00001",
                "relation": "same",
                "evidence": ["xe-10"],
                "difference": "Uses periodic boundary conditions."
            }]
        })),
    ];

    let r1 = compute_prior_work_from_events(&base);
    assert_eq!(r1.verdicts.len(), 2);
    assert_eq!(r1.verdicts[0].status, PriorWorkStatus::JudgedSame);
    assert!(!r1.verdicts[0].confirmed_by_user);
    assert_eq!(r1.verdicts[1].status, PriorWorkStatus::NoneInScope);
    assert!(report_matches_recomputed(&r1, &base));

    // Lessons and ranking marks must have zero effect (rule V10).
    let mut with_lessons = base.clone();
    with_lessons.push(mk_event(12, "lessons.supplied", json!({"run_id": "r1", "lesson_ids": ["l-1"]})));
    with_lessons.push(mk_event(13, IDEA_NOTE_EVENT, json!({"target": "arxiv:2401.00001", "action": "key_paper"})));
    assert_eq!(compute_prior_work_from_events(&with_lessons), r1);

    // Explicit confirm_same confirms sc-1, then reject_same reverts it (rule V6).
    let mut confirmed = base.clone();
    confirmed.push(mk_event(12, IDEA_NOTE_EVENT, json!({"subclaim_id": "sc-1", "action": "confirm_same"})));
    let r_conf = compute_prior_work_from_events(&confirmed);
    assert!(r_conf.verdicts[0].confirmed_by_user);
    assert_eq!(r_conf.verdicts[0].chip, "found (same)");

    confirmed.push(mk_event(13, IDEA_NOTE_EVENT, json!({"subclaim_id": "sc-1", "action": "reject_same"})));
    let r_rej = compute_prior_work_from_events(&confirmed);
    assert!(!r_rej.verdicts[0].confirmed_by_user);
    assert_eq!(r_rej.verdicts[0].chip, "Model judges: same");
}
