//! Validation tests for Explore exchange recorders & tools (packages X6, X7, X8).
//!
//! Zero live network calls, zero model calls, zero third-party paper text.

use ledger::Ledger;
use research_tools::idea_brief::{
    record_idea_brief, validate_and_build_idea_brief, EVENT_IDEA_BRIEF, MAX_ASSUMPTIONS,
    MAX_RESTATEMENT_CHARS, UI_SHARPENED_IDEA_LABEL,
};
use serde_json::json;
use tempfile::TempDir;

const SYNTH_IDEA: &str =
    "Measure Lanczos growth rate b_n ~ alpha*n in the volume-law phase and saturation in the area-law phase of a monitored Majorana lattice.";

fn valid_brief_payload() -> serde_json::Value {
    json!({
        "restatement": "Test whether Lanczos growth distinguishes volume-law from area-law dynamics in a monitored Majorana lattice.",
        "assumptions": ["Quadratic free-fermion trajectory"],
        "subclaims": [
            {
                "text": "b_n grows linearly in the volume-law phase",
                "idea_quote": "b_n ~ alpha*n in the volume-law phase"
            },
            {
                "text": "b_n saturates in the area-law phase",
                "idea_quote": "saturation in the area-law phase"
            }
        ],
        "terms": [
            { "term": "Lanczos growth", "aliases": ["Krylov complexity", "spread complexity"] }
        ],
        "perspectives": [
            {
                "kind": "exact",
                "question": "Has Lanczos growth been computed for monitored Majorana chains?",
                "queries": [
                    { "provider": "arxiv", "q": "Lanczos Majorana monitored chain" }
                ]
            },
            {
                "kind": "other_names",
                "question": "Is this studied under spread complexity or operator growth?",
                "queries": [
                    { "provider": "inspire", "q": "spread complexity entanglement transition" },
                    { "provider": "crossref", "q": "operator growth measurement transition" }
                ]
            }
        ],
        "questions": [
            {
                "question": "Projective or continuous measurements?",
                "options": ["projective", "continuous", "both"],
                "default": "both"
            }
        ],
        "exclusions": ["classical spin chains"]
    })
}

#[test]
fn x6_record_idea_brief_validates_bounds_exact_quotes_and_perspectives() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-val-brief";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();

    let (seq, brief) =
        record_idea_brief(&ledger, sid, 1, SYNTH_IDEA, Some(3), &valid_brief_payload())
            .expect("valid brief should record");
    assert!(seq >= 2);
    assert_eq!(brief.subclaims.len(), 2);
    assert_eq!(brief.subclaims[0].id, "sc-1");
    assert_eq!(brief.subclaims[1].id, "sc-2");
    assert_eq!(brief.perspectives[0].id, "p-1");
    assert_eq!(brief.perspectives[1].id, "p-2");
    assert_eq!(brief.questions[0].id, "q-1");
    assert_eq!(UI_SHARPENED_IDEA_LABEL, "Sharpened idea");

    let evs = ledger.read_session(sid).unwrap();
    let brief_ev = evs.iter().find(|e| e.kind == EVENT_IDEA_BRIEF).unwrap();
    assert_eq!(brief_ev.payload["request_seq"], 3);

    // Unknown fields rejected (`#[serde(deny_unknown_fields)]`).
    let mut with_extra = valid_brief_payload();
    with_extra["extra"] = json!(true);
    assert!(validate_and_build_idea_brief(1, SYNTH_IDEA, &with_extra).is_err());

    // Non-substring `idea_quote` rejected.
    let mut bad_quote = valid_brief_payload();
    bad_quote["subclaims"][0]["idea_quote"] = json!("b_n grows quadratically");
    let errs = validate_and_build_idea_brief(1, SYNTH_IDEA, &bad_quote).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("exact substring")), "{errs:?}");

    // Missing `other_names` perspective rejected, while `obstruction` remains optional.
    let mut no_other = valid_brief_payload();
    no_other["perspectives"][1]["kind"] = json!("method");
    let errs = validate_and_build_idea_brief(1, SYNTH_IDEA, &no_other).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("other_names")), "{errs:?}");

    // Restatement and assumption bounds enforced.
    let mut too_long = valid_brief_payload();
    too_long["restatement"] = json!("x".repeat(MAX_RESTATEMENT_CHARS + 5));
    too_long["assumptions"] = json!(vec!["a"; MAX_ASSUMPTIONS + 1]);
    let errs = validate_and_build_idea_brief(1, SYNTH_IDEA, &too_long).unwrap_err();
    assert!(errs.len() >= 2, "{errs:?}");
}
