use super::*;
use serde_json::json;
use tempfile::TempDir;

const IDEA_TEXT: &str =
    "Use Krylov complexity growth rate b_n ~ alpha*n in the volume-law phase and saturation in the area-law phase of a monitored Majorana chain.";

fn valid_brief_input() -> Value {
    json!({
        "restatement": "Test whether Lanczos growth rate distinguishes volume-law and area-law phases in a monitored Majorana chain.",
        "assumptions": ["Free-fermion quadratic trajectory", "Operator Krylov basis"],
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
            { "term": "Krylov complexity", "aliases": ["spread complexity", "operator growth"] }
        ],
        "perspectives": [
            {
                "kind": "exact",
                "question": "Has Krylov complexity been computed for a monitored Majorana chain?",
                "queries": [
                    { "provider": "arxiv", "q": "\"Krylov complexity\" \"Majorana\" measurement" },
                    { "provider": "inspire", "q": "Krylov complexity Majorana monitored" }
                ]
            },
            {
                "kind": "other_names",
                "question": "Is the same behaviour known under spread complexity or bi-Lanczos?",
                "queries": [
                    { "provider": "crossref", "q": "spread complexity measurement-induced transition" }
                ]
            }
        ],
        "questions": [
            {
                "question": "Projective or continuous monitoring?",
                "options": ["projective", "continuous", "both"],
                "default": "both"
            }
        ],
        "exclusions": ["classical cellular automata"]
    })
}

#[test]
fn valid_brief_assigns_sequential_ids_and_writes_ledger_event() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-brief-ok";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();

    let (seq, brief) =
        record_idea_brief(&ledger, sid, 1, IDEA_TEXT, Some(2), &valid_brief_input())
            .expect("valid brief must succeed");

    assert!(seq >= 2);
    assert_eq!(brief.idea_rev, 1);
    assert_eq!(brief.subclaims[0].id, "sc-1");
    assert_eq!(brief.subclaims[1].id, "sc-2");
    assert_eq!(brief.perspectives[0].id, "p-1");
    assert_eq!(brief.perspectives[1].id, "p-2");
    assert_eq!(brief.questions[0].id, "q-1");

    let events = ledger.read_session(sid).unwrap();
    let ev = events.iter().find(|e| e.kind == EVENT_IDEA_BRIEF).unwrap();
    assert_eq!(ev.payload["request_seq"], 2);
    assert_eq!(ev.payload["subclaims"][0]["id"], "sc-1");
    assert_eq!(ev.payload["perspectives"][1]["kind"], "other_names");
    assert!(!UI_SHARPENED_IDEA_LABEL.to_ascii_lowercase().contains("brief"));
}

#[test]
fn brief_rejects_unknown_fields_and_bad_subclaim_quote() {
    let mut bad_unknown = valid_brief_input();
    bad_unknown["injected_field"] = json!("nope");
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &bad_unknown).unwrap_err();
    assert!(errs[0].contains("unknown field"), "{errs:?}");

    let mut bad_quote = valid_brief_input();
    bad_quote["subclaims"][0]["idea_quote"] = json!("not a verbatim substring of idea");
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &bad_quote).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("exact substring")), "{errs:?}");
}

#[test]
fn brief_enforces_exact_and_other_names_perspectives_and_limits() {
    let mut missing_other = valid_brief_input();
    missing_other["perspectives"][1]["kind"] = json!("method");
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &missing_other).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("other_names")), "{errs:?}");

    let mut missing_exact = valid_brief_input();
    missing_exact["perspectives"][0]["kind"] = json!("obstruction");
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &missing_exact).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("`exact`")), "{errs:?}");

    let mut long_restatement = valid_brief_input();
    long_restatement["restatement"] = json!("a".repeat(MAX_RESTATEMENT_CHARS + 1));
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &long_restatement).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("1200")), "{errs:?}");

    let mut too_many_assumptions = valid_brief_input();
    too_many_assumptions["assumptions"] = json!(vec!["a"; MAX_ASSUMPTIONS + 1]);
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &too_many_assumptions).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("assumptions")), "{errs:?}");

    let mut bad_default = valid_brief_input();
    bad_default["questions"][0]["default"] = json!("unlisted");
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &bad_default).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("default")), "{errs:?}");
}

#[test]
fn brief_rejects_whitespace_only_entries_and_overlong_leaves() {
    let mut bad_opt = valid_brief_input();
    bad_opt["questions"][0]["options"] = json!(["projective", "   ", "both"]);
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &bad_opt).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("options")), "{errs:?}");

    let mut bad_alias = valid_brief_input();
    bad_alias["terms"][0]["aliases"] = json!(["spread complexity", "\t  "]);
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &bad_alias).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("aliases")), "{errs:?}");

    let mut bad_excl = valid_brief_input();
    bad_excl["exclusions"] = json!(["   "]);
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &bad_excl).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("exclusions")), "{errs:?}");

    let mut long_term = valid_brief_input();
    long_term["terms"][0]["term"] = json!("x".repeat(200));
    let errs = validate_and_build_idea_brief(1, IDEA_TEXT, &long_term).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("term")), "{errs:?}");
}

