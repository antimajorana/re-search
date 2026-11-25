//! Honesty audit failing tests for `research-tools` anchor verification, evidence resolution,
//! conventions partial-acceptance finalization, and recorder schema consistency.

use ledger::Ledger;
use research_tools::investigation::{def, paper_conventions, record, record_conventions};
use research_tools::{
    ingest_bytes, register_reader_tools, research_policy, IngestRequest, ResearchEnv,
    NOTE_EVIDENCE,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tools::{Approver, CallOutcome, Registry, Router};

const SID: &str = "audit-honesty-rt";
const READER_CALLER: &str = "worker:paper_reader:anthropic:audit-honesty-rt:r0";

struct DenyApprover;
impl Approver for DenyApprover {
    fn decide(&self, req: &tools::ApprovalRequest) -> tools::ApprovalReply {
        tools::ApprovalReply {
            approved: false,
            digest: req.digest.clone(),
            by: "test".into(),
        }
    }
}

fn setup(paper_text: &str) -> (tempfile::TempDir, Arc<ResearchEnv>, Ledger, String, u64) {
    let tmp = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let paper = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: paper_text.as_bytes(),
            source_id_hint: None,
            section_filter: None,
            title: Some("Audit Paper"),
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger
        .append(SID, "investigation.started", 1, &json!({}))
        .unwrap();
    ledger
        .append(
            SID,
            "investigation.paper",
            1,
            &json!({"source_id": paper.source_id}),
        )
        .unwrap();
    let req = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": paper.source_id}),
        )
        .unwrap();
    (tmp, env, ledger, paper.source_id, req)
}

#[test]
fn label_lookback_must_not_defeat_narrowing_across_60_unquoted_lines() {
    let mut lines = vec!["Lemma 1.".to_string()];
    for i in 2..=60 {
        lines.push(format!(
            "UNQUOTED INTERMEDIATE CLAIM {i}: completely unrelated assertion."
        ));
    }
    lines.push("Therefore the ground state energy vanishes identically.".to_string());
    let paper_text = lines.join("\n") + "\n";

    let (_tmp, env, ledger, pid, _) = setup(&paper_text);
    let anchor = json!({
        "source_id": pid,
        "start_line": 1,
        "end_line": 61,
        "kind": "passage",
        "label": "Lemma 1.",
        "quote": "ground state energy vanishes identically"
    });
    let map = json!({
        "source_id": pid,
        "title": "Ground State",
        "coverage": "paper",
        "limitations": [],
        "nodes": [{
            "id": "n1",
            "kind": "result",
            "main": true,
            "sentence": "Energy vanishes.",
            "anchors": [anchor]
        }],
        "edges": []
    });

    if let Ok(_res) = record(&env, &ledger, SID, &map) {
        let saved = ledger.read_verified_session(SID).unwrap().1.pop().unwrap();
        let saved_anchor = &saved.payload["nodes"][0]["anchors"][0];
        let s = saved_anchor["start_line"].as_u64().unwrap();
        let e = saved_anchor["end_line"].as_u64().unwrap();
        let excerpt = saved_anchor["excerpt"].as_str().unwrap_or("");
        assert!(
            e - s <= 8 && !excerpt.contains("UNQUOTED INTERMEDIATE CLAIM"),
            "16+ char quote on line 61 plus label on line 1 grounded {} lines ({}..={}), \
             absorbing 59 unquoted lines into verified excerpt",
            e - s + 1,
            s,
            e
        );
    }
}

#[test]
fn label_extension_restoring_original_range_must_not_emit_false_cited_range_correction() {
    let mut lines = Vec::new();
    for i in 1..=9 {
        lines.push(format!("Introductory context line {i}."));
    }
    lines.push("Eq. (1)".to_string()); // line 10
    lines.push(String::new()); // line 11 (blank)
    lines.push(String::new()); // line 12 (blank)
    lines.push("The mass gap strictly increases with coupling.".to_string()); // line 13
    lines.push("Closing remark.".to_string()); // line 14
    let paper_text = lines.join("\n") + "\n";

    let (_tmp, env, ledger, pid, _) = setup(&paper_text);
    let anchor = json!({
        "source_id": pid,
        "start_line": 10,
        "end_line": 13,
        "kind": "equation",
        "label": "Eq. (1)",
        "quote": "mass gap strictly increases with coupling"
    });
    let map = json!({
        "source_id": pid,
        "title": "Mass Gap",
        "coverage": "paper",
        "limitations": [],
        "nodes": [{
            "id": "eq1",
            "kind": "equation",
            "main": true,
            "sentence": "Mass gap increases.",
            "anchors": [anchor]
        }],
        "edges": []
    });

    let res = record(&env, &ledger, SID, &map).unwrap();
    let saved = ledger.read_verified_session(SID).unwrap().1.pop().unwrap();
    let saved_anchor = &saved.payload["nodes"][0]["anchors"][0];
    assert_eq!(saved_anchor["start_line"], 10);
    assert_eq!(saved_anchor["end_line"], 13);
    assert!(
        saved_anchor.get("cited_range").is_none(),
        "final range 10..=13 equals cited 10..=13, yet cited_range was set to {:?} (basis: {:?}, report: {})",
        saved_anchor.get("cited_range"),
        saved_anchor.get("range_basis"),
        res["anchor_report"]
    );
}

#[test]
fn excerpt_window_must_contain_the_full_matched_quote_or_reject() {
    let padded_line = format!(
        "Alpha boundary condition{}implies beta scaling law.",
        " ".repeat(3_400)
    );
    let paper_text = format!("Title: Padding Test\n{padded_line}\n");
    let (_tmp, env, ledger, pid, _) = setup(&paper_text);

    let quote = "Alpha boundary condition implies beta scaling law";
    let map = json!({
        "source_id": pid,
        "title": "Padding",
        "coverage": "paper",
        "limitations": [],
        "nodes": [{
            "id": "n1",
            "kind": "result",
            "main": true,
            "sentence": "Alpha implies beta.",
            "anchors": [{
                "source_id": pid,
                "start_line": 2,
                "end_line": 2,
                "kind": "passage",
                "quote": quote
            }]
        }],
        "edges": []
    });

    if let Ok(_res) = record(&env, &ledger, SID, &map) {
        let saved = ledger.read_verified_session(SID).unwrap().1.pop().unwrap();
        let excerpt = saved.payload["nodes"][0]["anchors"][0]["excerpt"]
            .as_str()
            .unwrap_or("");
        let folded_excerpt = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            folded_excerpt.contains(quote),
            "stored excerpt window dropped part of the verbatim quote: got `{folded_excerpt}`"
        );
    }
}

#[test]
fn evidence_anchor_with_reader_label_before_excerpt_window_must_not_fail_label_not_found() {
    let filler = "x ".repeat(1_800); // 3,600 bytes on line 2
    let line2 = format!("Theorem 1: {filler}the winding number equals the edge state count.");
    let paper_text = format!("Title: Long Theorem Line\n{line2}\n");
    let (tmp, env, _ledger, pid, _) = setup(&paper_text);

    let mut reg = Registry::new();
    register_reader_tools(&mut reg, Arc::clone(&env));
    let reader_router = Router::new(
        reg,
        research_policy(),
        Ledger::open(tmp.path()).unwrap(),
        SID,
        Arc::new(DenyApprover),
    );
    let note_out = reader_router
        .try_call(
            READER_CALLER,
            NOTE_EVIDENCE,
            &json!({
                "items": [{
                    "source_id": pid,
                    "start_line": 2,
                    "end_line": 2,
                    "kind": "passage",
                    "role": "result",
                    "label": "Theorem 1",
                    "quote": "winding number equals the edge state count",
                    "gist": "Bulk-boundary correspondence."
                }]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(note_val) = note_out else {
        panic!("expected note_evidence Ok, got {note_out:?}");
    };
    assert_eq!(note_val["noted"], 1, "note_evidence output: {note_val}");
    let ev_id = note_val["results"][0]["evidence"]
        .as_str()
        .unwrap()
        .to_string();

    let rec_ledger = Ledger::open(tmp.path()).unwrap();
    let map = json!({
        "source_id": pid,
        "title": "Winding",
        "coverage": "paper",
        "limitations": [],
        "nodes": [{
            "id": "thm1",
            "kind": "result",
            "main": true,
            "sentence": "Winding number equals edge state count.",
            "anchors": [{
                "source_id": pid,
                "evidence": ev_id,
                "kind": "passage",
                "label": "Theorem 1",
                "quote": "winding number equals the edge state count"
            }]
        }],
        "edges": []
    });
    let res = record(&env, &rec_ledger, SID, &map);
    assert!(
        res.is_ok(),
        "resolve_evidence rejected the reader's own verified label `Theorem 1` because excerpt_window sliced it off: {res:?}"
    );
}

#[test]
fn conventions_later_batch_must_not_finalize_done_true_or_wipe_limitations_while_pending_items_remain()
{
    let paper_text = concat!(
        "Title: Conventions Audit\n",
        "Throughout this paper we set Planck constant hbar = 1.\n",
        "The metric signature is chosen to be (-,+,+,+).\n",
    );
    let (_tmp, env, ledger, pid, req) = setup(paper_text);

    // Batch 1: one valid convention + one convention with a fabricated quote + a model limitation.
    let b1 = json!({
        "request_seq": req,
        "source_id": pid,
        "batch_index": 1,
        "done": true,
        "coverage": "partial",
        "limitations": ["Appendix C equations are scanned images."],
        "items": [
            {
                "id": "hbar-unit",
                "category": "units",
                "label": "hbar = 1",
                "meaning": "Natural units with hbar = 1.",
                "scope": "Paper",
                "basis": "explicit",
                "anchors": [{
                    "source_id": pid,
                    "start_line": 2,
                    "end_line": 2,
                    "kind": "passage",
                    "quote": "Throughout this paper we set Planck constant hbar = 1"
                }]
            },
            {
                "id": "bad-sign",
                "category": "sign",
                "label": "sign convention",
                "meaning": "Fabricated convention.",
                "scope": "Paper",
                "basis": "explicit",
                "anchors": [{
                    "source_id": pid,
                    "start_line": 3,
                    "end_line": 3,
                    "kind": "passage",
                    "quote": "this quote does not exist anywhere in the paper"
                }]
            }
        ]
    });
    let r1 = record_conventions(&env, &ledger, SID, &b1).unwrap();
    assert_eq!(r1["done"], false);

    // Batch 2: model drops `bad-sign` without re-citing it and claims done=true, coverage="paper", limitations=[].
    let b2 = json!({
        "request_seq": req,
        "source_id": pid,
        "batch_index": 2,
        "done": true,
        "coverage": "paper",
        "limitations": [],
        "items": []
    });
    if record_conventions(&env, &ledger, SID, &b2).is_ok() {
        let events = ledger.read_verified_session(SID).unwrap().1;
        let catalog: Value = paper_conventions(&events, req).unwrap();
        let pending = catalog["pending_items"].as_array().unwrap();
        let limitations = catalog["limitations"].as_array().unwrap();
        assert!(
            !(catalog["done"] == true && !pending.is_empty()),
            "catalog finalized with done=true and coverage={} while pending_items={pending:?} remain unresolved",
            catalog["coverage"]
        );
        assert!(
            limitations
                .iter()
                .any(|l| l.as_str().unwrap_or("").contains("Appendix C")),
            "Batch 2 wiped Batch 1's reported limitation: got limitations={limitations:?}"
        );
    }
}

#[test]
fn recorder_anchor_schema_min_length_must_not_contradict_3_char_whole_line_quote_rule() {
    let map_def = def();
    let quote_min = map_def.input_schema["properties"]["nodes"]["items"]["properties"]["anchors"]
        ["items"]["properties"]["quote"]["minLength"]
        .as_u64()
        .unwrap();
    assert!(
        quote_min <= 3,
        "record_paper_* anchor_schema sets quote.minLength={quote_min}, blocking 3–11 char whole-line quotes \
         (such as `E = mc^2`) that verify_pointer, resolve_evidence, and note_evidence_def (minLength=3) allow"
    );
}
