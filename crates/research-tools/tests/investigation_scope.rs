use ledger::Ledger;
use research_tools::investigation::{
    paper_conventions, record, record_conventions, record_summary,
};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::sync::Mutex;

const SID: &str = "investigate-scope";
const PAPER: &str = "\
Abstract
We study twist defects in two dimensions.
I. INTRODUCTION
We denote the global symmetry group by G.
II. DISLOCATIONS
In Sec. II, b denotes the Burgers vector.
III. CONCLUSIONS
Throughout Appendix A, hbar equals one.
";

fn setup_session() -> (tempfile::TempDir, ResearchEnv, Ledger, research_tools::IngestedSource) {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let paper = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "defects.txt",
            origin: "upload",
            raw_bytes: PAPER.as_bytes(),
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger.append(SID, "investigation.started", 1, &json!({})).unwrap();
    ledger
        .append(SID, "investigation.paper", 1, &json!({"source_id": paper.source_id}))
        .unwrap();
    (tmp, env, ledger, paper)
}

fn line_anchor(paper: &research_tools::IngestedSource, line: usize, excerpt: &str) -> Value {
    json!({
        "source_id": paper.source_id,
        "path": paper.primary_path,
        "start_line": line,
        "end_line": line,
        "excerpt": excerpt,
        "kind": "passage"
    })
}

#[test]
fn batch_after_section_done_without_paper_marker_refused() {
    let (_tmp, env, ledger, paper) = setup_session();
    let req = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({
                "source_id": paper.source_id,
                "summary_requested": true,
                "conventions_requested": true,
                "conventions_scope": "section"
            }),
        )
        .unwrap();

    let a_sum = line_anchor(&paper, 2, "We study twist defects in two dimensions.");
    let a_b = line_anchor(&paper, 6, "In Sec. II, b denotes the Burgers vector.");
    let a_g = line_anchor(&paper, 4, "We denote the global symmetry group by G.");
    let a_h = line_anchor(&paper, 8, "Throughout Appendix A, hbar equals one.");

    record_summary(
        &env,
        &ledger,
        SID,
        &json!({
            "source_id": paper.source_id,
            "what": {"text": "Studies twist defects.", "anchors": [a_sum]},
            "why": {"unstated": true},
            "how": {"unstated": true}
        }),
    )
    .unwrap();

    let b1 = json!({
        "request_seq": req,
        "source_id": paper.source_id,
        "batch_index": 1,
        "done": true,
        "coverage": "paper",
        "limitations": [],
        "items": [{
            "id": "burgers-b",
            "category": "notation",
            "label": "b",
            "meaning": "Burgers vector.",
            "scope": "Section II",
            "basis": "explicit",
            "anchors": [a_b]
        }]
    });
    let r1 = record_conventions(&env, &ledger, SID, &b1).unwrap();
    assert_eq!(r1["done"], true);

    let events = ledger.read_verified_session(SID).unwrap().1;
    let cat = paper_conventions(&events, req).unwrap();
    assert_eq!(cat["coverage_scope"], "section");
    assert_eq!(cat["done"], true);

    // A second batch under the same mapping_requested marker must be refused.
    let b2 = json!({
        "request_seq": req,
        "source_id": paper.source_id,
        "batch_index": 2,
        "done": true,
        "coverage": "paper",
        "limitations": [],
        "items": [{
            "id": "sym-g",
            "category": "notation",
            "label": "G",
            "meaning": "Global symmetry group.",
            "scope": "Paper",
            "basis": "explicit",
            "anchors": [a_g]
        }]
    });
    assert!(record_conventions(&env, &ledger, SID, &b2)
        .unwrap_err()
        .contains("already finalized"));

    // Save map & close mapping, then append conventions_resumed WITHOUT scope="paper".
    record(
        &env,
        &ledger,
        SID,
        &json!({
            "source_id": paper.source_id,
            "title": "Twist defects",
            "coverage": "paper",
            "limitations": [],
            "nodes": [{"id": "n1", "kind": "result", "main": true, "sentence": "Twist defects are studied.", "anchors": [a_sum]}],
            "edges": []
        }),
    )
    .unwrap();
    ledger
        .append(SID, "investigation.mapping_finished", 1, &json!({"request_seq": req}))
        .unwrap();
    ledger
        .append(
            SID,
            "investigation.conventions_resumed",
            1,
            &json!({"for_request_seq": req, "source_id": paper.source_id}),
        )
        .unwrap();
    assert!(record_conventions(&env, &ledger, SID, &b2)
        .unwrap_err()
        .contains("already finalized"));

    // Now append conventions_resumed WITH scope="paper": batch 2 succeeds and upgrades coverage_scope.
    ledger
        .append(
            SID,
            "investigation.conventions_resumed",
            1,
            &json!({"for_request_seq": req, "source_id": paper.source_id, "scope": "paper"}),
        )
        .unwrap();
    // Re-sending identical batch 1 stays idempotent.
    assert_eq!(record_conventions(&env, &ledger, SID, &b1).unwrap(), r1);

    let r2 = record_conventions(&env, &ledger, SID, &b2).unwrap();
    assert_eq!(r2["batch_index"], 2);
    assert_eq!(r2["done"], true);

    let events = ledger.read_verified_session(SID).unwrap().1;
    let expanded = paper_conventions(&events, req).unwrap();
    assert_eq!(expanded["coverage_scope"], "paper");
    assert_eq!(expanded["items"].as_array().unwrap().len(), 2);

    // Once finalized at paper scope, a third batch is refused even under scope="paper".
    let b3 = json!({
        "request_seq": req,
        "source_id": paper.source_id,
        "batch_index": 3,
        "done": true,
        "coverage": "paper",
        "limitations": [],
        "items": [{
            "id": "hbar-one",
            "category": "units",
            "label": "hbar",
            "meaning": "Natural units.",
            "scope": "Appendix A",
            "basis": "explicit",
            "anchors": [a_h]
        }]
    });
    assert!(record_conventions(&env, &ledger, SID, &b3)
        .unwrap_err()
        .contains("already finalized"));
}
