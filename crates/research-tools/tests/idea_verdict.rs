//! Integration tests for Explore X11 pure verdict computation (`idea_verdict::compute`)
//! covering Rules V1–V10 and OCR-backed `judged_*` statuses (`00-overview.md` 8.3 Q3).
//!
//! Required named tests:
//! - `idea_verdict_judged_requires_read_paper`
//! - `idea_verdict_other_name_needs_term`
//! - `idea_verdict_abstract_cap`
//! - `idea_quick_never_claims_none_in_scope`
//! - `idea_verdict_text_is_app_authored`
//! - `idea_verdict_recomputes_from_ledger`
//! - `idea_verdict_excludes_origin_paper`
//! - `idea_origin_references_are_marked`
//! - `idea_verdict_ignores_lessons_and_marks`
//! - `idea_library_does_not_count_as_service`
//! - `idea_verdict_allows_ocr_judged_tagged_ocr`

use ledger::Event;
use research_tools::idea_verdict::{self, IdeaPresetLimits};
use research_tools::paper_key::PaperKey;
use research_tools::prior_work::PriorWorkStatus;
use research_tools::prior_work_wording::contains_forbidden_prior_work_word;
use research_tools::quote_core::{quote_sha256, EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT};
use research_tools::search_coverage::ExplorePreset;
use serde_json::{json, Value};

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-verd".into(),
        seq,
        ts_ms: seq as i64 * 1000,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: String::new(),
    }
}

fn sha64(c: char) -> String {
    std::iter::repeat_n(c, 64).collect()
}

fn ft_quote_ev(seq: u64, paper_key: &str, quote: &str, subclaims: &[&str], text_basis: Option<&str>) -> Event {
    let mut payload = json!({
        "source_id": "src-1",
        "path": "src-1/main.tex",
        "start_line": 10,
        "end_line": 12,
        "file_sha256": sha64('a'),
        "quote_sha256": quote_sha256(quote),
        "quote": quote,
        "excerpt": quote,
        "source_seq": 2,
        "caller": "reader:s-verd",
        "paper_key": paper_key,
        "role": "states_result",
        "subclaim_ids": subclaims,
        "gist": "Matched result."
    });
    if let Some(tb) = text_basis {
        payload.as_object_mut().unwrap().insert("text_basis".into(), json!(tb));
    }
    ev(seq, EVIDENCE_QUOTE_EVENT, payload)
}

fn abs_quote_ev(seq: u64, paper_key: &str, quote: &str, subclaims: &[&str]) -> Event {
    ev(
        seq,
        EVIDENCE_ABSTRACT_EVENT,
        json!({
            "paper_key": paper_key,
            "quote": quote,
            "quote_sha256": quote_sha256(quote),
            "start": 0,
            "end": quote.len(),
            "raw_blob": sha64('b'),
            "provider": "arxiv",
            "subclaim_ids": subclaims
        }),
    )
}

fn base_complete_search_events(preset: &str, max_papers: usize) -> Vec<Event> {
    vec![
        ev(1, "idea.started", json!({
            "preset": preset,
            "budget": { "max_queries": 18, "max_screened": 40, "max_papers": max_papers, "calls_per_paper": 3, "max_model_calls": 24 }
        })),
        ev(2, "idea.stated", json!({ "rev": 1, "text": "Does b_n grow linearly?", "seeds": [] })),
        ev(3, "idea.brief", json!({
            "idea_rev": 1,
            "subclaims": [{ "id": "sc-1", "text": "b_n grows linearly", "idea_quote": "b_n grow linearly" }],
            "perspectives": [
                { "id": "p-1", "kind": "exact", "question": "Exact?", "queries": [{ "provider": "arxiv", "q": "b_n linear" }] },
                { "id": "p-2", "kind": "other_names", "question": "Other?", "queries": [{ "provider": "inspire", "q": "Krylov growth" }] }
            ]
        })),
        ev(4, "idea.search_planned", json!({
            "round": 1,
            "queries": [
                { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "b_n linear", "origin": "model" },
                { "qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "q": "Krylov growth", "origin": "model" }
            ]
        })),
        ev(5, "idea.search_finished", json!({
            "round": 1,
            "queries": [
                { "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 1 },
                { "qid": "q-2", "provider": "inspire", "status": "ok", "count": 1 }
            ]
        })),
        ev(6, "idea.candidates", json!({
            "round": 1,
            "candidates": [{ "key": "arxiv:2501.00001", "title": "P1", "year": 2025 }]
        })),
        ev(7, "idea.triage", json!({
            "items": [{ "key": "arxiv:2501.00001", "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Relevant." }],
            "reading_list": if max_papers > 0 { json!(["arxiv:2501.00001"]) } else { json!([]) }
        })),
    ]
}

#[test]
fn idea_verdict_judged_requires_read_paper() {
    let mut events = base_complete_search_events("standard", 6);
    // Paper status is `unavailable` (NOT `read_full` or `read_passages`).
    events.push(ev(8, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "unavailable", "text_basis": "tex" })));
    events.push(ft_quote_ev(9, "arxiv:2501.00001", "Lanczos coefficients grow linearly in chaotic chains.", &["sc-1"], Some("tex")));
    events.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "same",
            "evidence": ["xe-9"], "difference": "Same bound."
        }]
    })));
    let out = idea_verdict::compute(&events, None);
    assert_ne!(out.verdicts[0].status, PriorWorkStatus::JudgedSame, "Rule V1: unread paper cannot support judged_same");

    // Once paper status is `read_passages`, judged_same is allowed!
    events.push(ev(11, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_passages", "text_basis": "tex" })));
    let out2 = idea_verdict::compute(&events, None);
    assert_eq!(out2.verdicts[0].status, PriorWorkStatus::JudgedSame);
}

#[test]
fn idea_verdict_other_name_needs_term() {
    let mut events = base_complete_search_events("standard", 6);
    events.push(ev(8, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_full", "text_basis": "tex" })));
    events.push(ft_quote_ev(9, "arxiv:2501.00001", "The operator spread complexity obeys a linear law.", &["sc-1"], Some("tex")));

    // Term NOT in quote -> rejected!
    let mut ev_missing = events.clone();
    ev_missing.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "other_name",
            "evidence": ["xe-9"], "difference": "Different term.",
            "other_name": { "term": "Krylov entropy" }
        }]
    })));
    let out_bad = idea_verdict::compute(&ev_missing, None);
    assert_ne!(out_bad.verdicts[0].status, PriorWorkStatus::JudgedOtherName);

    // Term present in quote -> accepted!
    events.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "other_name",
            "evidence": ["xe-9"], "difference": "Different term.",
            "other_name": { "term": "spread complexity" }
        }]
    })));
    let out_ok = idea_verdict::compute(&events, None);
    assert_eq!(out_ok.verdicts[0].status, PriorWorkStatus::JudgedOtherName);
}

#[test]
fn idea_verdict_abstract_cap() {
    let mut events = base_complete_search_events("standard", 6);
    events.push(abs_quote_ev(8, "arxiv:2501.00001", "Abstract states linear Lanczos growth.", &["sc-1"]));
    let out = idea_verdict::compute(&events, None);
    assert_eq!(out.verdicts[0].status, PriorWorkStatus::AbstractOnly, "Rule V2: abstract-only caps at abstract_only");
}

#[test]
fn idea_quick_never_claims_none_in_scope() {
    let events = base_complete_search_events("quick", 0);
    let out = idea_verdict::compute(&events, Some(IdeaPresetLimits::for_preset(ExplorePreset::Quick)));
    assert_eq!(out.verdicts[0].status, PriorWorkStatus::Incomplete, "Rule V2: Quick must NEVER produce none_in_scope");
    let text = out.verdicts[0].text.to_lowercase();
    assert!(
        text.contains("no close match among") && text.contains("nothing was read"),
        "Quick negative sentence must match Rule V2 wording; got: {}",
        out.verdicts[0].text
    );
}

#[test]
fn idea_verdict_text_is_app_authored() {
    let mut events = base_complete_search_events("standard", 6);
    events.push(ev(8, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_full", "text_basis": "tex" })));
    events.push(ft_quote_ev(9, "arxiv:2501.00001", "Lanczos coefficients grow linearly with slope alpha.", &["sc-1"], Some("tex")));
    events.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "same",
            "evidence": ["xe-9"], "difference": "Same slope in finite chain."
        }]
    })));
    let out = idea_verdict::compute(&events, None);
    let v = &out.verdicts[0];
    assert_eq!(contains_forbidden_prior_work_word(&v.text), None, "Rule V5/S8: forbidden word in verdict text: {}", v.text);
    assert_eq!(contains_forbidden_prior_work_word(&v.chip), None, "Rule V5/S8: forbidden word in chip: {}", v.chip);
    assert!(v.chip.starts_with("Model judges:"), "Rule V6: unconfirmed judged status must say Model judges");
}

#[test]
fn idea_verdict_recomputes_from_ledger() {
    let mut events = base_complete_search_events("standard", 6);
    events.push(ev(8, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_full", "text_basis": "tex" })));
    events.push(ft_quote_ev(9, "arxiv:2501.00001", "Lanczos coefficients grow linearly with slope alpha.", &["sc-1"], Some("tex")));
    events.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "partial",
            "evidence": ["xe-9"], "difference": "Restricted to periodic boundaries."
        }]
    })));
    let stored = idea_verdict::compute(&events, None);
    assert!(idea_verdict::matches_recomputed(&stored, &events, None), "Rule V7: recomputation must match stored payload");
}

#[test]
fn idea_verdict_excludes_origin_paper() {
    // Test `from.paper`, `ask.fields.origin_paper` with absent `from.paper`, and explicit `"paper": null` (Audit CRITICAL #1).
    for packet_payload in [
        json!({
            "from": { "session": "s-inv", "mode": "investigate", "id": "n-1", "paper": { "key": "arxiv:2501.00001", "ids": { "arxiv": "2501.00001" } } },
            "papers": [{ "key": "arxiv:2501.00001", "ids": {} }]
        }),
        json!({
            "from": { "session": "s-inv", "mode": "investigate", "id": "n-1", "paper": null },
            "ask": { "kind": "explore", "fields": { "origin_paper": { "key": "arxiv:2501.00001", "ids": { "arxiv": "2501.00001" } }, "seed_papers": [] } }
        }),
    ] {
        let mut events = vec![ev(1, "handoff.packet", packet_payload)];
        for mut e in base_complete_search_events("standard", 6) {
            e.seq += 1;
            events.push(e);
        }
        events.push(ev(9, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_full", "text_basis": "tex" })));
        events.push(ft_quote_ev(10, "arxiv:2501.00001", "Lanczos coefficients grow linearly in chaotic chains.", &["sc-1"], Some("tex")));
        events.push(abs_quote_ev(11, "arxiv:2501.00001", "Abstract from origin paper itself.", &["sc-1"]));
        events.push(ev(12, "idea.map", json!({
            "overlaps": [{
                "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "same",
                "evidence": ["xe-10"], "difference": "Same paper."
            }]
        })));
        let out = idea_verdict::compute(&events, None);
        assert_eq!(out.origin_paper.as_deref(), Some("arxiv:2501.00001"));
        assert_eq!(
            out.verdicts[0].status,
            PriorWorkStatus::NoneInScope,
            "Rule V9: origin paper must NEVER support judged_* or abstract_only"
        );
        assert!(out.verdicts[0].overlaps.is_empty());
        assert!(out.verdicts[0].abstract_matches.is_empty());
    }
}

#[test]
fn idea_origin_references_are_marked() {
    let events = vec![ev(
        1,
        "handoff.packet",
        json!({
            "from": { "session": "s-inv", "mode": "investigate", "id": "n-1", "paper": { "key": "arxiv:2501.00001", "ids": { "arxiv": "2501.00001" } } },
            "papers": [
                { "key": "arxiv:2501.00001", "ids": { "arxiv": "2501.00001" } },
                { "key": "arxiv:1901.00009", "ids": { "arxiv": "1901.00009", "doi": "10.5555/ref-1" } }
            ],
            "ask": {
                "kind": "explore",
                "fields": {
                    "seed_papers": [{ "key": "doi:10.5555/ref-2", "ids": { "doi": "10.5555/ref-2" } }]
                }
            }
        }),
    )];
    let cited = idea_verdict::origin_cited_keys(&events);
    assert!(!cited.contains(&PaperKey::parse("arxiv:2501.00001").unwrap()), "origin itself is excluded");
    assert!(cited.contains(&PaperKey::parse("arxiv:1901.00009").unwrap()));
    assert!(cited.contains(&PaperKey::parse("doi:10.5555/ref-1").unwrap()));
    assert!(cited.contains(&PaperKey::parse("doi:10.5555/ref-2").unwrap()));
}

#[test]
fn idea_verdict_ignores_lessons_and_marks() {
    let mut base = base_complete_search_events("standard", 6);
    base.push(ev(8, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_full", "text_basis": "tex" })));
    base.push(ft_quote_ev(9, "arxiv:2501.00001", "Lanczos coefficients grow linearly with slope alpha.", &["sc-1"], Some("tex")));
    base.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "partial",
            "evidence": ["xe-9"], "difference": "Periodic boundary only."
        }]
    })));
    let without_lessons = idea_verdict::compute(&base, None);

    let mut with_lessons = base.clone();
    with_lessons.push(ev(11, "lessons.supplied", json!({ "run_id": "r1", "lesson_ids": ["l-1"], "blob": sha64('c') })));
    with_lessons.push(ev(12, "idea.note", json!({ "target_kind": "candidate", "target_id": "arxiv:2501.00001", "action": "key_paper", "comment": "Important" })));
    with_lessons.push(ev(13, "idea.note", json!({ "target_kind": "candidate", "target_id": "arxiv:2501.00001", "action": "not_relevant" })));
    let after_lessons = idea_verdict::compute(&with_lessons, None);

    assert_eq!(without_lessons.verdicts, after_lessons.verdicts, "Rule V10: lessons & marks must not alter verdicts");
    assert_eq!(without_lessons.coverage, after_lessons.coverage, "Rule V10: lessons & marks must not alter coverage gate");
}

#[test]
fn idea_library_does_not_count_as_service() {
    // Only 1 external service answered (`arxiv`) + local library searched with hits -> gate MUST fail!
    let events = vec![
        ev(1, "idea.started", json!({ "preset": "standard", "budget": { "max_screened": 40, "max_papers": 6 } })),
        ev(2, "idea.stated", json!({ "rev": 1, "text": "Test idea.", "seeds": [] })),
        ev(3, "idea.brief", json!({
            "idea_rev": 1,
            "subclaims": [{ "id": "sc-1", "text": "Claim 1", "idea_quote": "Test idea" }],
            "perspectives": [
                { "id": "p-1", "kind": "exact", "question": "E?", "queries": [{ "provider": "arxiv", "q": "test" }] },
                { "id": "p-2", "kind": "other_names", "question": "O?", "queries": [{ "provider": "arxiv", "q": "alt" }] }
            ]
        })),
        ev(4, "idea.search_planned", json!({
            "round": 1,
            "queries": [
                { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "test", "origin": "model" },
                { "qid": "q-2", "perspective_id": "p-2", "provider": "arxiv", "q": "alt", "origin": "model" }
            ]
        })),
        ev(5, "idea.search_finished", json!({
            "round": 1,
            "library": { "searched": true, "papers": 50, "with_full_text": 40, "hits": 5 },
            "queries": [
                { "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 0 },
                { "qid": "q-2", "provider": "arxiv", "status": "ok", "count": 0 }
            ]
        })),
        ev(6, "idea.triage", json!({ "items": [], "reading_list": [] })),
    ];
    let out = idea_verdict::compute(&events, None);
    assert!(!out.coverage.gate.passed, "Rule V10: user library must NEVER count as a second literature service");
    assert_eq!(out.verdicts[0].status, PriorWorkStatus::Incomplete);
}

#[test]
fn idea_verdict_allows_ocr_judged_tagged_ocr() {
    let mut events = base_complete_search_events("standard", 6);
    events.push(ev(8, "idea.paper", json!({ "key": "arxiv:2501.00001", "status": "read_full", "text_basis": "ocr" })));
    events.push(ft_quote_ev(9, "arxiv:2501.00001", "Lanczos coefficients grow linearly in chaotic chains.", &["sc-1"], Some("ocr")));
    events.push(ev(10, "idea.map", json!({
        "overlaps": [{
            "subclaim_id": "sc-1", "paper_key": "arxiv:2501.00001", "relation": "same",
            "evidence": ["xe-9"], "difference": "Same scaling law from scanned 1979 paper."
        }]
    })));
    let out = idea_verdict::compute(&events, None);
    let v = &out.verdicts[0];
    assert_eq!(v.status, PriorWorkStatus::JudgedSame, "00-overview.md 8.3 Q3: OCR quotes may support judged_* status");
    assert!(v.has_ocr_evidence, "OCR-backed verdict must set has_ocr_evidence = true");
    assert!(v.overlaps[0].ocr_sourced, "OCR-backed overlap must set ocr_sourced = true");
    assert!(v.text.contains("OCR"), "verdict sentence must disclose OCR tag: {}", v.text);
}
