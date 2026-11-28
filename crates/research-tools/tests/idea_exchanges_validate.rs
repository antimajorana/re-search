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

#[test]
fn x7_explore_v1_profile_freezes_four_tools_with_rule_p7_isolation() {
    use research_tools::idea_tools::EXPLORE_V1_TOOL_IDS;
    use research_tools::tool_profile::{lookup, PROFILE_EXPLORE_V1};

    let p = lookup(PROFILE_EXPLORE_V1).expect("explore/1 must be built");
    let ids: Vec<&str> = p.defs.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, EXPLORE_V1_TOOL_IDS);
    for d in &p.defs {
        assert_ne!(d.effect, policy::EffectClass::ExecuteCode);
        assert_ne!(d.effect, policy::EffectClass::WriteWorkspace);
    }
}

#[test]
fn x8_record_idea_triage_enforces_rule_c3_verbatim_vs_rebuilt_and_quick_preset_rule_v2() {
    use research_tools::idea_triage::{
        record_idea_triage, validate_triage_input, TriageCandidateContext, UI_FROM_ABSTRACT_LABEL,
    };
    use research_tools::lit_abstracts::AbstractRef;
    use research_tools::paper_key::PaperKey;
    use research_tools::quote_core::EVIDENCE_ABSTRACT_EVENT;

    let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2501.05555v1</id>
    <title>Synthetic Krylov Paper</title>
    <summary>Lanczos growth scales linearly in the volume-law phase and saturates in the area-law phase.</summary>
    <published>2025-01-12T00:00:00Z</published>
    <author><name>C. Author</name></author>
  </entry>
</feed>"#;
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-val-triage";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();
    let blob = ledger.put_blob(xml).unwrap();

    let key_v = PaperKey::parse("arxiv:2501.05555").unwrap();
    let key_r = PaperKey::parse("doi:10.1000/rebuilt").unwrap();
    let cands = vec![
        TriageCandidateContext {
            key: key_v.clone(),
            alias_keys: vec![],
            abstract_text: Some(
                "Lanczos growth scales linearly in the volume-law phase and saturates in the area-law phase.".into(),
            ),
            abstract_ref: Some(AbstractRef { raw_blob: blob.clone(), provider: "arxiv".into(), verbatim: true }),
        },
        TriageCandidateContext {
            key: key_r.clone(),
            alias_keys: vec![],
            abstract_text: Some("Rebuilt abstract from inverted index.".into()),
            abstract_ref: Some(AbstractRef { raw_blob: blob, provider: "openalex".into(), verbatim: false }),
        },
    ];
    let scs = vec!["sc-1".to_string()];

    // Rule C3: rebuilt (`verbatim: false`) abstract cannot emit evidence.abstract.
    let bad_rebuilt = json!({
        "items": [{
            "key": "doi:10.1000/rebuilt",
            "relevance": "related",
            "subclaim_ids": ["sc-1"],
            "why": "Looks related.",
            "abstract_quote": "Rebuilt abstract"
        }],
        "reading_list": []
    });
    assert!(validate_triage_input(&bad_rebuilt, &cands, &scs, 0, &ledger).is_err());

    // Quick preset (`max_papers = 0`) rejects non-empty reading_list (Rule V2).
    let bad_quick = json!({
        "items": [{
            "key": "arxiv:2501.05555",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Directly tests Lanczos scaling.",
            "abstract_quote": "Lanczos growth scales linearly in the volume-law phase"
        }],
        "reading_list": ["arxiv:2501.05555"]
    });
    assert!(validate_triage_input(&bad_quick, &cands, &scs, 0, &ledger).is_err());

    // Valid Quick-preset triage records verbatim `xa-<seq>` ONLY for the verbatim paper, zero for rebuilt.
    let good = json!({
        "items": [
            {
                "key": "arxiv:2501.05555",
                "relevance": "core",
                "subclaim_ids": ["sc-1"],
                "why": "Directly tests Lanczos scaling.",
                "abstract_quote": "Lanczos growth scales linearly in the volume-law phase"
            },
            {
                "key": "doi:10.1000/rebuilt",
                "relevance": "background",
                "subclaim_ids": [],
                "why": "Background from rebuilt abstract.",
                "abstract_quote": null
            }
        ],
        "reading_list": []
    });
    let out = record_idea_triage(&ledger, sid, 0, &cands, &scs, 0, Some(4), &good).unwrap();
    assert!(out.quick_abstract_only);
    assert_eq!(out.abstract_evidence_ids.len(), 1);
    assert!(out.items[0].evidence_id.as_deref().unwrap().starts_with("xa-"));
    assert_eq!(out.items[0].label_basis, UI_FROM_ABSTRACT_LABEL);
    assert_eq!(out.items[1].evidence_id, None);
    let evs = ledger.read_session(sid).unwrap();
    assert_eq!(evs.iter().filter(|e| e.kind == EVIDENCE_ABSTRACT_EVENT).count(), 1);
}

