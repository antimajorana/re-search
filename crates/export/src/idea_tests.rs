use super::*;
use ledger::{Event, GENESIS_HASH};
use serde_json::json;

fn make_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-unit".into(),
        seq,
        ts_ms: 1000 + seq as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: GENESIS_HASH.into(),
        event_hash: GENESIS_HASH.into(),
    }
}

#[test]
fn format_parse_and_metadata() {
    assert_eq!(IdeaExportFormat::parse("json").unwrap(), IdeaExportFormat::Json);
    assert_eq!(IdeaExportFormat::parse("JSON").unwrap(), IdeaExportFormat::Json);
    assert_eq!(IdeaExportFormat::parse("bib").unwrap(), IdeaExportFormat::Bib);
    assert_eq!(IdeaExportFormat::parse("bibtex").unwrap(), IdeaExportFormat::Bib);
    assert!(IdeaExportFormat::parse("md").is_err());
    assert!(IdeaExportFormat::parse("tex").is_err());
    assert_eq!(IdeaExportFormat::Json.extension(), "json");
    assert_eq!(IdeaExportFormat::Bib.extension(), "bib");
    assert!(IdeaExportFormat::Json.content_type().starts_with("application/json"));
    assert!(IdeaExportFormat::Bib.content_type().starts_with("application/x-bibtex"));
}

#[test]
fn rule_s8_guard_rejects_forbidden_terms() {
    for bad in ["verified quote", "brand new result", "novel approach", "proved theorem"] {
        assert!(
            check_text_s8("test_field", bad).is_err(),
            "expected Rule S8 rejection for `{bad}`"
        );
    }
    for ok in [
        "Not found in the searched scope.",
        "Model judges: partly the same",
        "matched full-text quote (xe-12) from arxiv:2401.00001",
    ] {
        assert!(
            check_text_s8("test_field", ok).is_ok(),
            "unexpected Rule S8 rejection for `{ok}`"
        );
    }
}

#[test]
fn rule_c3_rejects_non_verbatim_and_empty_abstract_quotes() {
    let bad_rebuilt = vec![make_event(
        2,
        "evidence.abstract",
        json!({
            "paper_key": "arxiv:2401.00001",
            "quote": "Rebuilt inverted index snippet",
            "verbatim": false,
            "provider": "openalex",
            "raw_blob": "a".repeat(64)
        }),
    )];
    let err = extract::collect_abstract_quotes(&bad_rebuilt, 1).unwrap_err();
    assert!(err.contains("Rule C3"), "got: {err}");

    let bad_empty = vec![make_event(
        2,
        "evidence.abstract",
        json!({
            "paper_key": "arxiv:2401.00001",
            "quote": "   ",
            "verbatim": true,
            "provider": "arxiv",
            "raw_blob": "a".repeat(64)
        }),
    )];
    assert!(extract::collect_abstract_quotes(&bad_empty, 1).is_err());
}

#[test]
fn rule_v7_rejects_missing_verdict_after_map_and_unexpected_fields() {
    let events_missing = vec![
        make_event(1, "idea.started", json!({ "preset": "standard" })),
        make_event(2, "idea.stated", json!({ "rev": 1, "text": "Synthetic idea" })),
        make_event(3, "idea.map", json!({ "nodes": [], "edges": [], "overlaps": [] })),
    ];
    let err = recompute_and_check_rule_v7(&events_missing, 2).unwrap_err();
    assert!(err.contains("Rule V7"), "got: {err}");

    let computed = idea_verdict::compute(&events_missing, derive_preset_limits(&events_missing));
    let mut extra_val = serde_json::to_value(&computed).unwrap();
    extra_val.as_object_mut().unwrap().insert("tampered_extra".into(), json!(true));
    let mut with_extra = events_missing;
    with_extra.push(make_event(4, IDEA_VERDICT_EVENT, extra_val));
    let err2 = recompute_and_check_rule_v7(&with_extra, 2).unwrap_err();
    assert!(err2.contains("unexpected field"), "got: {err2}");
}

#[test]
fn fold_brief_and_answers_merges_edits_and_answer_maps() {
    let events = vec![
        make_event(1, "idea.stated", json!({ "rev": 1, "text": "Orig" })),
        make_event(2, "idea.brief", json!({ "restatement": "Initial", "assumptions": ["a1"] })),
        make_event(
            3,
            "idea.answer",
            json!({
                "answers": { "q1": "continuous" },
                "restatement": "Edited restatement",
                "assumptions": ["a1", "a2"]
            }),
        ),
    ];
    let (brief, answers) = extract::fold_brief_and_answers(&events, 1);
    let b = brief.unwrap();
    assert_eq!(b["restatement"], "Edited restatement");
    assert_eq!(b["assumptions"], json!(["a1", "a2"]));
    assert_eq!(answers.get("q1").map(String::as_str), Some("continuous"));
}
