//! Adversarial regression tests added from the independent F12 audit (H2, H3, H4, H5, M2, M5, M6).

use super::*;
use crate::lit_providers::LitProvider;
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use crate::quote_core::{quote_sha256, EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT};
use crate::search_coverage::{
    compute_coverage, CoverageInput, ExplorePreset, PaperReadStatus, PerspectiveKind,
    QueryCoverageRecord, QueryOutcomeStatus, IDEA_BRIEF_EVENT, IDEA_CANDIDATES_EVENT,
    IDEA_NOTE_EVENT, IDEA_PAPER_EVENT, IDEA_SEARCH_FINISHED_EVENT, IDEA_STARTED_EVENT,
    IDEA_STATED_EVENT,
};
use ledger::{Event, GENESIS_HASH};
use serde_json::json;

fn mk_event(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-audit-pw".into(),
        seq,
        ts_ms: 1_700_000_000_000 + seq as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: GENESIS_HASH.into(),
        event_hash: GENESIS_HASH.into(),
    }
}

fn minimal_passing_coverage() -> SearchCoverage {
    compute_coverage(&CoverageInput {
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
    })
}

#[test]
fn v9_origin_exclusion_matches_f2_aliases_for_overlaps_and_abstracts() {
    let orig_arxiv = PaperKey::from_arxiv("2301.00001").unwrap();
    let orig_doi = PaperKey::from_doi("10.1000/origin.alias").unwrap();
    let quote = "Origin statement proved by DOI alias.";
    let qsha = quote_sha256(quote);
    let fsha = "a".repeat(64);

    let events = vec![
        mk_event(1, HANDOFF_PACKET_EVENT, json!({
            "from": {"paper": {"key": orig_arxiv.as_str(), "ids": {"arxiv": "2301.00001", "doi": "10.1000/origin.alias"}}}
        })),
        mk_event(2, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(3, IDEA_BRIEF_EVENT, json!({
            "subclaims": [{"id": "sc-1"}],
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(4, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 0},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 0}
            ]
        })),
        mk_event(5, IDEA_PAPER_EVENT, json!({"key": orig_doi.as_str(), "status": "read_full"})),
        mk_event(6, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "src-1", "path": "src-1/main.tex", "start_line": 1, "end_line": 2,
            "file_sha256": fsha, "quote_sha256": qsha, "paper_key": orig_doi.as_str(),
            "quote": quote, "excerpt": quote, "subclaim_ids": ["sc-1"]
        })),
        mk_event(7, EVIDENCE_ABSTRACT_EVENT, json!({
            "paper_key": orig_doi.as_str(), "quote": quote, "quote_sha256": qsha,
            "start": 0, "end": quote.len(), "raw_blob": fsha, "provider": "crossref",
            "subclaim_ids": ["sc-1"]
        })),
        mk_event(8, IDEA_MAP_EVENT, json!({
            "overlaps": [{
                "subclaim_id": "sc-1", "paper_key": orig_doi.as_str(), "relation": "same",
                "evidence": ["xe-6"], "difference": "Exact alias of origin."
            }]
        })),
    ];

    let rep = compute_prior_work_from_events(&events);
    assert_eq!(rep.verdicts[0].status, PriorWorkStatus::NoneInScope);
    assert!(rep.verdicts[0].abstract_matches.is_empty());
    assert!(matches!(
        rep.verdicts[0].rejected_overlaps[0].reason,
        OverlapRejectionReason::OriginPaperExcluded
    ));
}

#[test]
fn hybrid_pdf_per_page_ocr_detection_from_source_ingested_pdf_pages() {
    let quote_p1 = "Clean digital text on page 1.";
    let quote_p2 = "Scanned text layer on page 2.";
    let fsha = "b".repeat(64);
    let events = vec![
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(2, IDEA_BRIEF_EVENT, json!({
            "subclaims": [{"id": "sc-1"}, {"id": "sc-2"}],
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(3, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 0},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 0}
            ]
        })),
        // Hybrid PDF: page 1 is text_layer (lines 1..=20), page 2 is scan_text_layer (lines 21..=40).
        mk_event(4, SOURCE_INGESTED_EVENT, json!({
            "source_id": "pdf-1",
            "pdf_extractor": "pdf-extract",
            "pdf_pages": [
                {"page": 1, "basis": "text_layer", "first_line": 1, "last_line": 20},
                {"page": 2, "basis": "scan_text_layer", "first_line": 21, "last_line": 40}
            ]
        })),
        // idea.paper omits text_basis, and F4 evidence.quote has no text_basis field.
        mk_event(5, IDEA_PAPER_EVENT, json!({"key": "arxiv:2401.00005", "status": "read_full"})),
        mk_event(6, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "pdf-1", "path": "pdf-1/pages.txt", "start_line": 5, "end_line": 6,
            "file_sha256": fsha, "quote_sha256": quote_sha256(quote_p1),
            "paper_key": "arxiv:2401.00005", "quote": quote_p1, "excerpt": quote_p1
        })),
        mk_event(7, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "pdf-1", "path": "pdf-1/pages.txt", "start_line": 25, "end_line": 26,
            "file_sha256": fsha, "quote_sha256": quote_sha256(quote_p2),
            "paper_key": "arxiv:2401.00005", "quote": quote_p2, "excerpt": quote_p2
        })),
        mk_event(8, IDEA_MAP_EVENT, json!({
            "overlaps": [
                {"subclaim_id": "sc-1", "paper_key": "arxiv:2401.00005", "relation": "same", "evidence": ["xe-6"]},
                {"subclaim_id": "sc-2", "paper_key": "arxiv:2401.00005", "relation": "same", "evidence": ["xe-7"]}
            ]
        })),
    ];

    let rep = compute_prior_work_from_events(&events);
    assert!(!rep.verdicts[0].has_ocr_evidence);
    assert!(!rep.verdicts[0].text.contains("[OCR source]"));
    assert!(rep.verdicts[1].has_ocr_evidence);
    assert!(rep.verdicts[1].text.contains("[OCR source]"));
}

#[test]
fn rejects_malformed_f4_events_and_zero_evidence_ids_and_dangling_xe() {
    let p1 = PaperKey::from_arxiv("2401.00001").unwrap();
    // Malformed events: evidence.quote missing QuoteRef, evidence.abstract missing raw_blob/start/end.
    let events = vec![
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(2, IDEA_BRIEF_EVENT, json!({
            "subclaims": [{"id": "sc-1"}],
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(3, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 0},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 0}
            ]
        })),
        mk_event(4, IDEA_PAPER_EVENT, json!({"key": p1.as_str(), "status": "read_full"})),
        mk_event(5, EVIDENCE_QUOTE_EVENT, json!({"paper_key": p1.as_str(), "quote": "Ungrounded quote."})),
        mk_event(6, EVIDENCE_ABSTRACT_EVENT, json!({"paper_key": p1.as_str(), "quote": "Ungrounded abs.", "subclaim_ids": ["sc-1"]})),
        mk_event(7, IDEA_MAP_EVENT, json!({
            "overlaps": [{"subclaim_id": "sc-1", "paper_key": p1.as_str(), "relation": "same", "evidence": ["xe-5"]}]
        })),
    ];
    let rep = compute_prior_work_from_events(&events);
    assert_eq!(rep.verdicts[0].status, PriorWorkStatus::NoneInScope);
    assert!(rep.verdicts[0].abstract_matches.is_empty());

    // Zero EvidenceId cannot serialize and is rejected in evaluate_prior_work; dangling xe-999 rejects overlap (M6).
    assert!(serde_json::to_string(&EvidenceId::FullText(0)).is_err());
    let input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into()],
        coverage: minimal_passing_coverage(),
        paper_reads: BTreeMap::from([(p1.clone(), PaperReadStatus::ReadFull)]),
        paper_text_basis: BTreeMap::new(),
        full_text_evidence: vec![FullTextEvidence {
            id: EvidenceId::FullText(10),
            paper_key: p1.clone(),
            quote: "Valid quote.".into(),
            excerpt: "Valid quote.".into(),
            quote_ref: None,
            text_basis: TextBasis::Tex,
            subclaim_ids: vec!["sc-1".into()],
            role: None,
        }],
        abstract_evidence: vec![],
        overlaps: vec![OverlapCandidate {
            subclaim_id: "sc-1".into(),
            paper_key: p1,
            relation: OverlapRelation::Same,
            evidence_ids: vec![EvidenceId::FullText(10), EvidenceId::FullText(999)],
            difference: String::new(),
            other_name: None,
        }],
        confirmed_subclaims: BTreeSet::new(),
        origin_paper: None,
    };
    let v = &evaluate_prior_work(&input)[0];
    assert_eq!(v.status, PriorWorkStatus::NoneInScope);
    assert!(matches!(
        v.rejected_overlaps[0].reason,
        OverlapRejectionReason::UnknownEvidenceId { evidence_id: EvidenceId::FullText(999) }
    ));
}

#[test]
fn stale_confirm_same_invalidated_on_idea_revision_or_changed_map() {
    let q1 = "Quote from paper A.";
    let q2 = "Quote from paper B.";
    let fsha = "c".repeat(64);
    let mut events = vec![
        mk_event(1, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(2, IDEA_BRIEF_EVENT, json!({
            "subclaims": [{"id": "sc-1"}],
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(3, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 0},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 0}
            ]
        })),
        mk_event(4, IDEA_PAPER_EVENT, json!({"key": "arxiv:2401.00001", "status": "read_full"})),
        mk_event(5, IDEA_PAPER_EVENT, json!({"key": "arxiv:2401.00002", "status": "read_full"})),
        mk_event(6, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "s1", "path": "s1/a.tex", "start_line": 1, "end_line": 1,
            "file_sha256": fsha, "quote_sha256": quote_sha256(q1), "paper_key": "arxiv:2401.00001",
            "quote": q1, "excerpt": q1
        })),
        mk_event(7, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "s2", "path": "s2/b.tex", "start_line": 1, "end_line": 1,
            "file_sha256": fsha, "quote_sha256": quote_sha256(q2), "paper_key": "arxiv:2401.00002",
            "quote": q2, "excerpt": q2
        })),
        mk_event(8, IDEA_MAP_EVENT, json!({
            "overlaps": [{"subclaim_id": "sc-1", "paper_key": "arxiv:2401.00001", "relation": "partial", "evidence": ["xe-6"]}]
        })),
        mk_event(9, IDEA_NOTE_EVENT, json!({"subclaim_id": "sc-1", "action": "confirm_same"})),
    ];
    assert!(compute_prior_work_from_events(&events).verdicts[0].confirmed_by_user);

    // New map replaces paper A (partial) with paper B (same) -> stale confirmation cleared (H5)!
    events.push(mk_event(10, IDEA_MAP_EVENT, json!({
        "overlaps": [{"subclaim_id": "sc-1", "paper_key": "arxiv:2401.00002", "relation": "same", "evidence": ["xe-7"]}]
    })));
    let after_remap = compute_prior_work_from_events(&events);
    assert_eq!(after_remap.verdicts[0].status, PriorWorkStatus::JudgedSame);
    assert!(!after_remap.verdicts[0].confirmed_by_user);

    // Confirm again, then revise idea (idea.stated) -> confirmation cleared (H5)!
    events.push(mk_event(11, IDEA_NOTE_EVENT, json!({"subclaim_id": "sc-1", "action": "confirm_same"})));
    assert!(compute_prior_work_from_events(&events).verdicts[0].confirmed_by_user);
    events.push(mk_event(12, IDEA_STATED_EVENT, json!({"rev": 2, "text": "Revised idea", "seeds": []})));
    assert!(!compute_prior_work_from_events(&events).verdicts[0].confirmed_by_user);
}

#[test]
fn mixed_relation_overlaps_isolate_primary_status_and_ocr_and_enforce_v8_difference() {
    let p_tex = PaperKey::from_arxiv("2401.00001").unwrap();
    let p_ocr = PaperKey::from_arxiv("2401.00002").unwrap();
    let p_bad = PaperKey::from_arxiv("2401.00003").unwrap();
    let input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into()],
        coverage: minimal_passing_coverage(),
        paper_reads: BTreeMap::from([
            (p_tex.clone(), PaperReadStatus::ReadFull),
            (p_ocr.clone(), PaperReadStatus::ReadFull),
            (p_bad.clone(), PaperReadStatus::ReadFull),
        ]),
        paper_text_basis: BTreeMap::from([(p_ocr.clone(), TextBasis::Ocr)]),
        full_text_evidence: vec![
            FullTextEvidence {
                id: EvidenceId::FullText(1),
                paper_key: p_tex.clone(),
                quote: "TeX proof of exact claim.".into(),
                excerpt: "TeX proof of exact claim.".into(),
                quote_ref: None,
                text_basis: TextBasis::Tex,
                subclaim_ids: vec!["sc-1".into()],
                role: None,
            },
            FullTextEvidence {
                id: EvidenceId::FullText(2),
                paper_key: p_ocr.clone(),
                quote: "Scanned partial result.".into(),
                excerpt: "Scanned partial result.".into(),
                quote_ref: None,
                text_basis: TextBasis::Ocr,
                subclaim_ids: vec!["sc-1".into()],
                role: None,
            },
            FullTextEvidence {
                id: EvidenceId::FullText(3),
                paper_key: p_bad.clone(),
                quote: "Third paper quote.".into(),
                excerpt: "Third paper quote.".into(),
                quote_ref: None,
                text_basis: TextBasis::Tex,
                subclaim_ids: vec!["sc-1".into()],
                role: None,
            },
        ],
        abstract_evidence: vec![],
        overlaps: vec![
            OverlapCandidate {
                subclaim_id: "sc-1".into(),
                paper_key: p_tex,
                relation: OverlapRelation::Same,
                evidence_ids: vec![EvidenceId::FullText(1)],
                difference: "Same statement in periodic geometry.".into(),
                other_name: None,
            },
            OverlapCandidate {
                subclaim_id: "sc-1".into(),
                paper_key: p_ocr,
                relation: OverlapRelation::Partial,
                evidence_ids: vec![EvidenceId::FullText(2)],
                difference: "Covers d = 2 only.".into(),
                other_name: None,
            },
            // Banned V8 phrase in model difference prose -> rejected (M5).
            OverlapCandidate {
                subclaim_id: "sc-1".into(),
                paper_key: p_bad,
                relation: OverlapRelation::Partial,
                evidence_ids: vec![EvidenceId::FullText(3)],
                difference: "This is a novel extension that has not been done.".into(),
                other_name: None,
            },
        ],
        confirmed_subclaims: BTreeSet::new(),
        origin_paper: Some(PaperRef {
            key: PaperKey::from_arxiv("2001.00001").unwrap(),
            version: None,
            ids: PaperIds::default(),
        }),
    };

    let v = &evaluate_prior_work(&input)[0];
    assert_eq!(v.status, PriorWorkStatus::JudgedSame);
    // Primary Same overlap is TeX -> top-level has_ocr_evidence is false; secondary Partial is separate (M2).
    assert!(!v.has_ocr_evidence);
    assert!(v.text.contains("Model judges: same as arxiv:2401.00001 based on matched full-text quote (xe-1)"));
    assert!(v.text.contains("Additional recorded overlap: partly the same as arxiv:2401.00002 (xe-2) [OCR source]"));
    assert!(matches!(
        v.rejected_overlaps[0].reason,
        OverlapRejectionReason::BannedProsePhrase { .. }
    ));
}

#[test]
fn rejects_dangling_and_cross_paper_abstract_ids_on_overlap_and_clears_stale_map_on_revision() {
    let p1 = PaperKey::from_arxiv("2401.00001").unwrap();
    let p2 = PaperKey::from_arxiv("2401.00002").unwrap();
    let mut input = PriorWorkInput {
        subclaim_ids: vec!["sc-1".into()],
        coverage: minimal_passing_coverage(),
        paper_reads: BTreeMap::from([(p1.clone(), PaperReadStatus::ReadFull)]),
        paper_text_basis: BTreeMap::new(),
        full_text_evidence: vec![FullTextEvidence {
            id: EvidenceId::FullText(10),
            paper_key: p1.clone(),
            quote: "Valid full-text quote.".into(),
            excerpt: "Valid full-text quote.".into(),
            quote_ref: None,
            text_basis: TextBasis::Tex,
            subclaim_ids: vec!["sc-1".into()],
            role: None,
        }],
        abstract_evidence: vec![AbstractEvidence {
            id: EvidenceId::Abstract(20),
            paper_key: p2,
            quote: "Abstract from another paper.".into(),
            verbatim: true,
            raw_blob: "d".repeat(64),
            provider: Some(LitProvider::Arxiv),
            subclaim_ids: vec![],
        }],
        overlaps: vec![OverlapCandidate {
            subclaim_id: "sc-1".into(),
            paper_key: p1.clone(),
            relation: OverlapRelation::Same,
            evidence_ids: vec![EvidenceId::FullText(10), EvidenceId::Abstract(999)],
            difference: String::new(),
            other_name: None,
        }],
        confirmed_subclaims: BTreeSet::new(),
        origin_paper: None,
    };
    // Dangling xa-999 alongside valid xe-10 must be rejected (R1).
    let v_dang = &evaluate_prior_work(&input)[0];
    assert_eq!(v_dang.status, PriorWorkStatus::NoneInScope);
    assert!(matches!(
        v_dang.rejected_overlaps[0].reason,
        OverlapRejectionReason::UnknownEvidenceId { evidence_id: EvidenceId::Abstract(999) }
    ));

    // Cross-paper xa-20 alongside valid xe-10 must be rejected (R1).
    input.overlaps[0].evidence_ids = vec![EvidenceId::FullText(10), EvidenceId::Abstract(20)];
    let v_cross = &evaluate_prior_work(&input)[0];
    assert_eq!(v_cross.status, PriorWorkStatus::NoneInScope);
    assert!(matches!(
        v_cross.rejected_overlaps[0].reason,
        OverlapRejectionReason::EvidencePaperMismatch { evidence_id: EvidenceId::Abstract(20) }
    ));

    // Origin alias discovered later via idea.candidates excludes DOI overlap (L1),
    // and idea.stated clears pre-revision overlaps so stale confirm_same cannot re-confirm (R2).
    let quote = "Origin paper discovered via candidate DOI.";
    let fsha = "e".repeat(64);
    let mut events = vec![
        mk_event(1, HANDOFF_PACKET_EVENT, json!({"from": {"paper": {"key": "arxiv:2301.00001"}}})),
        mk_event(2, IDEA_STARTED_EVENT, json!({"preset": "standard"})),
        mk_event(3, IDEA_BRIEF_EVENT, json!({
            "subclaims": [{"id": "sc-1"}],
            "perspectives": [{"id": "p-1", "kind": "exact"}, {"id": "p-2", "kind": "other_names"}]
        })),
        mk_event(4, IDEA_SEARCH_FINISHED_EVENT, json!({
            "queries": [
                {"qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "status": "ok", "hits": 1},
                {"qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "status": "ok", "hits": 0}
            ]
        })),
        mk_event(5, IDEA_CANDIDATES_EVENT, json!({
            "candidates": [{"key": "arxiv:2301.00001", "ids": {"arxiv": "2301.00001", "doi": "10.1000/orig.cand"}, "found_by": ["q-1"]}]
        })),
        mk_event(6, IDEA_PAPER_EVENT, json!({"key": "doi:10.1000/orig.cand", "status": "read_full"})),
        mk_event(7, EVIDENCE_QUOTE_EVENT, json!({
            "source_id": "s1", "path": "s1/a.tex", "start_line": 1, "end_line": 1,
            "file_sha256": fsha, "quote_sha256": quote_sha256(quote),
            "paper_key": "doi:10.1000/orig.cand", "quote": quote, "excerpt": quote
        })),
        mk_event(8, IDEA_MAP_EVENT, json!({
            "overlaps": [{"subclaim_id": "sc-1", "paper_key": "doi:10.1000/orig.cand", "relation": "same", "evidence": ["xe-7"]}]
        })),
    ];
    let rep = compute_prior_work_from_events(&events);
    assert!(matches!(
        rep.verdicts[0].rejected_overlaps[0].reason,
        OverlapRejectionReason::OriginPaperExcluded
    ));

    // Also test idea.stated clearing stale map & blocking stale confirm_same (R2).
    events.push(mk_event(9, IDEA_PAPER_EVENT, json!({"key": p1.as_str(), "status": "read_full"})));
    events.push(mk_event(10, EVIDENCE_QUOTE_EVENT, json!({
        "source_id": "s2", "path": "s2/b.tex", "start_line": 1, "end_line": 1,
        "file_sha256": fsha, "quote_sha256": quote_sha256(quote),
        "paper_key": p1.as_str(), "quote": quote, "excerpt": quote
    })));
    events.push(mk_event(11, IDEA_MAP_EVENT, json!({
        "overlaps": [{"subclaim_id": "sc-1", "paper_key": p1.as_str(), "relation": "same", "evidence": ["xe-10"]}]
    })));
    events.push(mk_event(12, IDEA_STATED_EVENT, json!({"rev": 2, "text": "Revised idea", "seeds": []})));
    events.push(mk_event(13, IDEA_NOTE_EVENT, json!({"subclaim_id": "sc-1", "action": "confirm_same", "map_seq": 11})));
    let after_rev = compute_prior_work_from_events(&events);
    assert!(after_rev.verdicts[0].overlaps.is_empty());
    assert!(!after_rev.verdicts[0].confirmed_by_user);
}
