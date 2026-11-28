use super::*;
use crate::lit_abstracts::AbstractRef;
use ledger::Ledger;
use serde_json::json;
use tempfile::TempDir;

const ARXIV_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2501.01234v1</id>
    <title>Synthetic Krylov Complexity in Monitored Majorana Chains</title>
    <summary>We show that Lanczos coefficients b_n grow linearly in the volume-law phase and saturate in the area-law phase.</summary>
    <published>2025-01-10T00:00:00Z</published>
    <author><name>A. Author</name></author>
  </entry>
</feed>"#;

#[test]
fn idea_abstract_quote_must_be_in_blob_and_emits_deduped_xa_evidence() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-triage-c3";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();
    let blob = ledger.put_blob(ARXIV_XML).unwrap();

    let key = PaperKey::parse("arxiv:2501.01234").unwrap();
    let cands = vec![TriageCandidateContext {
        key: key.clone(),
        alias_keys: vec![],
        abstract_text: Some(
            "We show that Lanczos coefficients b_n grow linearly in the volume-law phase and saturate in the area-law phase.".into(),
        ),
        abstract_ref: Some(AbstractRef {
            raw_blob: blob.clone(),
            provider: "arxiv".into(),
            verbatim: true,
        }),
    }];
    let scs = vec!["sc-1".to_string(), "sc-2".to_string()];

    // Fabricated quote not in blob must fail closed (Rule C3).
    let bad = json!({
        "items": [{
            "key": "arxiv:2501.01234",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Directly tests b_n scaling.",
            "abstract_quote": "completely fabricated sentence not in raw xml"
        }],
        "reading_list": ["arxiv:2501.01234"]
    });
    let errs = validate_triage_input(&bad, &cands, &scs, 6, &ledger).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("Rule C3")), "{errs:?}");

    // Valid verbatim quote with extra whitespace succeeds, fills offsets, and emits `xa-<seq>`.
    let good = json!({
        "items": [{
            "key": "arxiv:2501.01234",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Directly studies Lanczos growth across phases.",
            "abstract_quote": "Lanczos   coefficients b_n grow linearly in the volume-law phase"
        }],
        "reading_list": ["arxiv:2501.01234"]
    });
    let out1 = record_idea_triage(&ledger, sid, 0, &cands, &scs, 6, Some(2), &good).unwrap();
    assert_eq!(out1.abstract_evidence_ids.len(), 1);
    let xa_id = &out1.abstract_evidence_ids[0];
    assert!(xa_id.starts_with("xa-"), "{xa_id}");
    assert_eq!(out1.items[0].evidence_id.as_deref(), Some(xa_id.as_str()));
    assert_eq!(out1.items[0].label_basis, UI_FROM_ABSTRACT_LABEL);
    assert!(!out1.quick_abstract_only);

    let events = ledger.read_session(sid).unwrap();
    let xa_ev = events.iter().find(|e| e.kind == EVIDENCE_ABSTRACT_EVENT).unwrap();
    assert_eq!(xa_ev.payload["paper_key"], "arxiv:2501.01234");
    assert_eq!(xa_ev.payload["raw_blob"], blob);
    assert!(xa_ev.payload["start"].as_u64().unwrap() < xa_ev.payload["end"].as_u64().unwrap());

    // Rule S3 deduplication: recording the same quote again reuses the exact same `xa-<seq>`.
    let out2 = record_idea_triage(&ledger, sid, 0, &cands, &scs, 6, Some(5), &good).unwrap();
    assert_eq!(out2.abstract_evidence_ids[0], *xa_id);
    let xa_count = ledger
        .read_session(sid)
        .unwrap()
        .iter()
        .filter(|e| e.kind == EVIDENCE_ABSTRACT_EVENT)
        .count();
    assert_eq!(xa_count, 1);
}

#[test]
fn idea_abstract_rebuilt_is_never_evidence_and_quick_preset_enforces_zero_reading_list() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-triage-rebuilt-quick";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();
    let blob = ledger.put_blob(ARXIV_XML).unwrap();

    let key = PaperKey::parse("arxiv:2501.01234").unwrap();
    let rebuilt_cands = vec![TriageCandidateContext {
        key: key.clone(),
        alias_keys: vec![],
        abstract_text: Some("Rebuilt abstract text from inverted index.".into()),
        abstract_ref: Some(AbstractRef {
            raw_blob: blob,
            provider: "openalex".into(),
            verbatim: false,
        }),
    }];

    // Citing an abstract_quote from a non-verbatim (rebuilt) abstract MUST be rejected (Rule C3).
    let with_quote = json!({
        "items": [{
            "key": "arxiv:2501.01234",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Rebuilt abstract appears relevant.",
            "abstract_quote": "Rebuilt abstract text"
        }],
        "reading_list": []
    });
    let errs = validate_triage_input(&with_quote, &rebuilt_cands, &["sc-1".into()], 0, &ledger)
        .unwrap_err();
    assert!(errs.iter().any(|e| e.contains("non-verbatim")), "{errs:?}");

    // Quick preset (`max_papers == 0`) with non-empty reading_list MUST be rejected (Rule V2).
    let quick_nonempty_rl = json!({
        "items": [{
            "key": "arxiv:2501.01234",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Rebuilt abstract appears relevant.",
            "abstract_quote": null
        }],
        "reading_list": ["arxiv:2501.01234"]
    });
    let errs_rl =
        validate_triage_input(&quick_nonempty_rl, &rebuilt_cands, &["sc-1".into()], 0, &ledger)
            .unwrap_err();
    assert!(errs_rl.iter().any(|e| e.contains("Quick preset")), "{errs_rl:?}");

    // Rebuilt abstract triaged WITHOUT abstract_quote in Quick preset succeeds with ZERO evidence.abstract.
    let quick_ok = json!({
        "items": [{
            "key": "arxiv:2501.01234",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Rebuilt abstract appears relevant.",
            "abstract_quote": null
        }],
        "reading_list": []
    });
    let out = record_idea_triage(&ledger, sid, 0, &rebuilt_cands, &["sc-1".into()], 0, Some(3), &quick_ok)
        .unwrap();
    assert!(out.quick_abstract_only);
    assert!(out.reading_list.is_empty());
    assert!(out.abstract_evidence_ids.is_empty());
    assert_eq!(out.items[0].evidence_id, None);
    assert_eq!(
        ledger
            .read_session(sid)
            .unwrap()
            .iter()
            .filter(|e| e.kind == EVIDENCE_ABSTRACT_EVENT)
            .count(),
        0
    );
}

#[test]
fn idea_triage_verifies_cross_provider_alias_blob_and_requires_full_candidate_coverage() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let cr_json = br#"{"message":{"items":[{"DOI":"10.1000/merged.1","title":["Merged Paper"],"abstract":"<jats:p>Cross-provider verbatim Lanczos bound.</jats:p>"}]}}"#;
    let blob = ledger.put_blob(cr_json).unwrap();
    let cands = vec![
        TriageCandidateContext {
            key: PaperKey::parse("arxiv:2501.09999").unwrap(),
            alias_keys: vec![PaperKey::parse("doi:10.1000/merged.1").unwrap()],
            abstract_text: Some("Cross-provider verbatim Lanczos bound.".into()),
            abstract_ref: Some(AbstractRef { raw_blob: blob, provider: "crossref".into(), verbatim: true }),
        },
        TriageCandidateContext {
            key: PaperKey::parse("arxiv:2501.08888").unwrap(),
            alias_keys: vec![],
            abstract_text: None,
            abstract_ref: None,
        },
    ];
    // Omitting second candidate must be rejected (Carbon X8 MEDIUM #3).
    let partial = json!({
        "items": [{
            "key": "arxiv:2501.09999",
            "relevance": "core",
            "subclaim_ids": ["sc-1"],
            "why": "Matches bound.",
            "abstract_quote": "Cross-provider verbatim Lanczos bound."
        }],
        "reading_list": ["arxiv:2501.09999"]
    });
    let errs = validate_triage_input(&partial, &cands, &["sc-1".into()], 6, &ledger).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("missing")), "{errs:?}");

    // Classifying both candidates succeeds via alias_keys lookup against crossref blob (Carbon X8 HIGH #3).
    let full = json!({
        "items": [
            {
                "key": "arxiv:2501.09999",
                "relevance": "core",
                "subclaim_ids": ["sc-1"],
                "why": "Matches bound.",
                "abstract_quote": "Cross-provider verbatim Lanczos bound."
            },
            {
                "key": "arxiv:2501.08888",
                "relevance": "unrelated",
                "subclaim_ids": [],
                "why": "No abstract match.",
                "abstract_quote": null
            }
        ],
        "reading_list": ["arxiv:2501.09999"]
    });
    let validated = validate_triage_input(&full, &cands, &["sc-1".into()], 6, &ledger).unwrap();
    assert!(validated.items[0].evidence_payload.is_some());
}
