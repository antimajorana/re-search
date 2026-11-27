//! Focused tests for the F8 post-revision harness expansion:
//! `Set::{P6, B, W, K}`, `P4 -> C` measure updates, and the §7.3 measures.
//! Included by `eval_harness.rs`.

use super::eval_support::manifest::{parse_manifest, validate_manifest, validate_manifests, Set};
use super::eval_support::measures::{evaluate, judge, measure, GateVerdict, Outcome, Rule, MEASURES};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn manifest_json(set: &str, tier: &str, cases: Value) -> String {
    json!({ "format_version": 1, "set": set, "tier": tier, "cases": cases }).to_string()
}

fn errors_of(text: &str) -> Vec<String> {
    match parse_manifest(text) {
        Ok(m) => validate_manifest(&m),
        Err(e) => vec![e],
    }
}

#[test]
fn sets_p6_b_w_k_manifest_schemas_and_cross_references_validate() {
    assert_eq!(Set::ALL.len(), 13);
    assert_eq!(Set::P6.letter(), "P6");
    assert_eq!(Set::B.letter(), "B");
    assert_eq!(Set::W.letter(), "W");
    assert_eq!(Set::K.letter(), "K");

    // Set P6 accepts live DOI/Arxiv and fixed Fixture, and requires statements/sections/sampled_sentences.
    let p6_key = json!({ "statements": [], "sections": [], "sampled_sentences": [] });
    let p6_live = manifest_json(
        "P6",
        "live",
        json!([{
            "id": "P6-01",
            "mode": "investigate",
            "input": { "kind": "doi", "doi": "10.0000/p6.pdf" },
            "answer_key": p6_key,
            "notes": ""
        }]),
    );
    assert_eq!(errors_of(&p6_live), Vec::<String>::new());

    let p6_bad = manifest_json(
        "P6",
        "live",
        json!([{
            "id": "P6-02",
            "mode": "investigate",
            "input": { "kind": "fixture", "files": ["p6/paper.pdf"] },
            "answer_key": { "statements": [], "sections": [] },
            "notes": ""
        }]),
    );
    let p6_errs = errors_of(&p6_bad);
    assert!(p6_errs.iter().any(|e| e.contains("sampled_sentences")), "{p6_errs:?}");
    assert!(p6_errs.iter().any(|e| e.contains("never by a stored file")), "{p6_errs:?}");

    // Set B accepts LibraryFixture in fixed tier and Library in live tier.
    let b_key = json!({ "items": [], "planted_cases": [] });
    let b_fixed = manifest_json(
        "B",
        "fixed",
        json!([{
            "id": "B-01",
            "mode": "library",
            "input": {
                "kind": "library_fixture",
                "source_kind": "kandinsky_vault",
                "path": "b/vault",
                "auxiliary_paths": ["b/shared_pdfs"]
            },
            "answer_key": b_key,
            "notes": ""
        }]),
    );
    assert_eq!(errors_of(&b_fixed), Vec::<String>::new());
    let b_live_bad = manifest_json(
        "B",
        "live",
        json!([{
            "id": "B-02",
            "mode": "library",
            "input": { "kind": "library_fixture", "source_kind": "folder", "path": "b/folder" },
            "answer_key": b_key,
            "notes": ""
        }]),
    );
    assert!(errors_of(&b_live_bad).iter().any(|e| e.contains("live-tier library case")));

    // Set W requires compiles + check_table and validates case references across manifests.
    let p_m = parse_manifest(&manifest_json(
        "P",
        "live",
        json!([{
            "id": "P-01",
            "mode": "investigate",
            "input": { "kind": "arxiv", "id": "0000.00000" },
            "answer_key": { "statements": [], "sections": [], "uses_edges": [] },
            "notes": ""
        }]),
    ))
    .unwrap();
    let w_ok = parse_manifest(&manifest_json(
        "W",
        "live",
        json!([{
            "id": "W-01",
            "mode": "writeup",
            "input": { "kind": "writeup", "case_id": "P-01" },
            "answer_key": { "compiles": true, "check_table": [] },
            "notes": ""
        }]),
    ))
    .unwrap();
    assert_eq!(validate_manifests(&[p_m.clone(), w_ok]), Vec::<String>::new());

    let w_missing = parse_manifest(&manifest_json(
        "W",
        "live",
        json!([{
            "id": "W-02",
            "mode": "writeup",
            "input": { "kind": "writeup", "case_id": "Q-missing" },
            "answer_key": { "compiles": true, "check_table": [] },
            "notes": ""
        }]),
    ))
    .unwrap();
    let w_errs = validate_manifests(&[p_m.clone(), w_missing]);
    assert!(w_errs.iter().any(|e| e.contains("not a known case")), "{w_errs:?}");

    // Set K enforces <= 300-char lessons and validates target_case across manifests.
    let k_bad_lesson = manifest_json(
        "K",
        "fixed",
        json!([{
            "id": "K-01",
            "mode": "corrections",
            "input": {
                "kind": "correction",
                "scenario": "lesson_supplied",
                "lesson": "x".repeat(301),
                "scripts": ["k/pair-01.json"]
            },
            "answer_key": { "expected_effect": "shown" },
            "notes": ""
        }]),
    );
    assert!(errors_of(&k_bad_lesson).iter().any(|e| e.contains("at most 300 characters")));

    let k_ok = parse_manifest(&manifest_json(
        "K",
        "fixed",
        json!([{
            "id": "K-02",
            "mode": "corrections",
            "input": {
                "kind": "correction",
                "scenario": "lesson_supplied",
                "target_case": "P-01",
                "lesson": "Check domain before square root.",
                "scripts": ["k/pair-02.json"]
            },
            "answer_key": { "expected_effect": "shown" },
            "notes": ""
        }]),
    ))
    .unwrap();
    assert_eq!(validate_manifests(&[p_m, k_ok]), Vec::<String>::new());
}

#[test]
fn p4_measures_moved_to_c_and_new_section_7_3_measures_enforce_gate_rules() {
    assert_eq!(MEASURES.len(), 55);
    let pass_count = MEASURES.iter().filter(|d| d.rule != Rule::Recorded).count();
    let rec_count = MEASURES.iter().filter(|d| d.rule == Rule::Recorded).count();
    assert_eq!((pass_count, rec_count), (24, 31));

    // Deferred P4 replaced by C (P-to-I) & I across prior-work measures and quote measure set.
    for id in [
        "earlier_source_found",
        "none_in_scope_false_negative_rate",
        "judged_same_key_different",
        "judged_same_key_different_unreviewed",
    ] {
        let def = measure(id).unwrap();
        assert!(!def.applies_to.contains(&Set::P4), "{id} still references deferred P4");
        assert_eq!(def.applies_to, [Set::I, Set::C], "{id}");
    }
    let qa = measure("quotes_accepted_per_model_call").unwrap();
    assert!(!qa.applies_to.contains(&Set::P4));
    assert!(qa.applies_to.contains(&Set::P6) && qa.applies_to.contains(&Set::C));

    // All new MustBeZero pass-rule measures fail on 1.0, fail closed on fractional values, and pass on 0.0.
    let new_must_be_zero = [
        ("pdf_wrong_page_chip", Set::P6),
        ("pdf_scanned_reported_plain_text", Set::P6),
        ("sync_second_changes", Set::B),
        ("sync_planted_case_mismatches", Set::B),
        ("sync_source_folder_diffs", Set::B),
        ("sync_removed_source_quote_breaks", Set::B),
        ("writeup_failures", Set::W),
        ("lessons_gate_drift", Set::K),
    ];
    for (id, expected_set) in new_must_be_zero {
        let def = measure(id).unwrap_or_else(|| panic!("missing measure `{id}`"));
        assert_eq!(def.applies_to, [expected_set], "{id}");
        assert_eq!(def.rule, Rule::MustBeZero, "{id}");
        assert_eq!(judge(&def.rule, Some(0.0)), Outcome::Pass, "{id}");
        assert_eq!(judge(&def.rule, Some(1.0)), Outcome::Fail, "{id}");
        assert_eq!(judge(&def.rule, None), Outcome::NotMeasured, "{id}");
        assert!(evaluate(&BTreeMap::from([(id.to_string(), 0.5)])).is_err(), "{id}");
    }

    // All new Recorded measures stay RecordedOnly when given valid values and reject out-of-domain shares.
    for (id, expected_set) in [
        ("time_to_skeleton_pdf_s", Set::P6),
        ("pdf_sentence_match_share", Set::P6),
        ("pdf_scan_char_errors_per_100", Set::P6),
        ("pdf_two_col_wrong_order_lines", Set::P6),
        ("search_recall_with_library", Set::I),
        ("search_recall_without_library", Set::I),
        ("lessons_error_recurrence", Set::K),
    ] {
        let def = measure(id).unwrap_or_else(|| panic!("missing recorded measure `{id}`"));
        assert_eq!(def.applies_to, [expected_set], "{id}");
        assert_eq!(def.rule, Rule::Recorded, "{id}");
        let rep = evaluate(&BTreeMap::from([(id.to_string(), 0.5)])).unwrap();
        assert_eq!(rep.verdict, GateVerdict::Incomplete);
        assert_eq!(rep.outcome_of(id), Some(Outcome::RecordedOnly), "{id}");
        if def.unit == "share" {
            assert!(evaluate(&BTreeMap::from([(id.to_string(), 1.1)])).is_err(), "{id}");
        }
    }
}
