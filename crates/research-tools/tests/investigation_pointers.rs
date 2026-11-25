use ledger::Ledger;
use research_tools::investigation::{
    paper_conventions, record, record_branch, record_conventions, record_summary,
};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::sync::Mutex;

const SID: &str = "investigate-pointers";

type SetupTuple = (tempfile::TempDir, ResearchEnv, Ledger, String, String, Option<(String, String)>, u64);

fn setup(paper_text: &str, ref_text: Option<&str>) -> SetupTuple {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let upload = |name: &str, text: &str| {
        ingest_bytes(
            &env.sandbox(),
            &db,
            &IngestRequest {
                session_id: SID,
                file_name: name,
                origin: "upload",
                raw_bytes: text.as_bytes(),
                source_id_hint: None,
                section_filter: None,
                title: None,
            },
        )
        .unwrap()
    };
    let paper = upload("paper.txt", paper_text);
    let reference = ref_text.map(|t| {
        let r = upload("ref.txt", t);
        (r.source_id, r.primary_path.unwrap())
    });
    let ledger = db.into_inner().unwrap();
    ledger.append(SID, "investigation.started", 1, &json!({})).unwrap();
    ledger.append(SID, "investigation.paper", 1, &json!({"source_id": paper.source_id})).unwrap();
    let req = ledger
        .append(SID, "investigation.mapping_requested", 1, &json!({"source_id": paper.source_id}))
        .unwrap();
    (tmp, env, ledger, paper.source_id, paper.primary_path.unwrap(), reference, req)
}

fn ptr(source_id: &str, start_line: usize, end_line: usize, quote: &str, kind: &str) -> Value {
    json!({"source_id": source_id, "start_line": start_line, "end_line": end_line, "quote": quote, "kind": kind})
}

const PAPER: &str = concat!(
    "Title: Scaling Laws in Lattice Models\n",
    "We assume strictly positive mass throughout the derivation.\n",
    "Equation (1) predicts a constant oscillation period.\n",
    "Figure 2 reports the measured oscillation period across samples.\n",
    "The experimental data support the scaling result [1].\n",
    "[1] Earlier measurement of the constant period.\n",
    "Throughout, d denotes the exterior derivative operator.\n",
    "We denote the oscillation frequency by the symbol f.\n",
    "In Appendix B, the symbol g denotes the coupling strength.\n",
    "The metric signature is chosen to be mostly plus.\n",
);

#[test]
fn map_node_one_good_one_bad_pointer_saved_with_report() {
    let (_tmp, env, ledger, pid, _, _, _) = setup(PAPER, None);
    let good = ptr(&pid, 2, 2, "assume strictly positive mass throughout", "passage");
    let bad = ptr(&pid, 2, 2, "completely fabricated nonexistent sentence", "passage");
    let map = json!({
        "source_id": pid, "title": "Scaling", "coverage": "paper", "limitations": [],
        "nodes": [{"id": "n1", "kind": "result", "main": true, "sentence": "Mass is positive.", "anchors": [good, bad]}],
        "edges": []
    });
    let res = record(&env, &ledger, SID, &map).unwrap();
    assert_eq!(res["anchor_basis"]["cited_lines"], 1);
    assert_eq!(res["anchor_report"]["dropped"].as_array().unwrap().len(), 1);
    assert_eq!(res["anchor_report"]["dropped"][0]["owner"], "nodes[n1].anchors[1]");
    assert_eq!(res["anchor_report"]["dropped"][0]["code"], "quote_not_found");
    let saved = ledger.read_verified_session(SID).unwrap().1.pop().unwrap();
    assert_eq!(saved.payload["nodes"][0]["anchors"].as_array().unwrap().len(), 1);
    assert_eq!(saved.payload["nodes"][0]["anchors"][0]["excerpt_basis"], "cited_lines");
    assert_eq!(saved.payload["anchor_report"], res["anchor_report"]);
}

#[test]
fn drop_leaving_no_primary_anchor_fails() {
    let (_tmp, env, ledger, pid, _, reference, _) = setup(PAPER, Some("Earlier external measurement of scaling.\n"));
    let (rid, rpath) = reference.unwrap();
    let bad_primary = ptr(&pid, 6, 6, "fabricated quote not in primary paper", "reference");
    let good_ref = json!({"source_id": rid, "path": rpath, "start_line": 1, "end_line": 1, "quote": "Earlier external measurement of scaling", "kind": "passage"});
    let main_anchor = ptr(&pid, 5, 5, "experimental data support the scaling result", "passage");
    let map = json!({
        "source_id": pid, "title": "Scaling", "coverage": "paper", "limitations": [],
        "nodes": [
            {"id": "res", "kind": "result", "main": true, "sentence": "Data support scaling.", "anchors": [main_anchor.clone()]},
            {"id": "ref1", "kind": "reference", "main": false, "sentence": "Earlier work cited.", "anchors": [bad_primary, good_ref]}
        ],
        "edges": [{"from": "ref1", "to": "res", "kind": "cites", "basis": "explicit", "reason": "Cites earlier work.", "anchor": main_anchor}]
    });
    let err: Value = serde_json::from_str(&record(&env, &ledger, SID, &map).unwrap_err()).unwrap();
    assert_eq!(err["error"], "anchors_failed");
    assert_eq!(err["faults"][0]["owner"], "nodes[ref1].anchors[0]");
    assert!(ledger.read_session(SID).unwrap().iter().all(|e| e.kind != "investigation.map"));
}

#[test]
fn bad_edge_pointer_fails() {
    let (_tmp, env, ledger, pid, _, _, _) = setup(PAPER, None);
    let a1 = ptr(&pid, 2, 2, "assume strictly positive mass throughout", "passage");
    let a2 = ptr(&pid, 3, 3, "Equation (1) predicts a constant oscillation", "equation");
    let bad_edge = ptr(&pid, 3, 3, "this edge quote is completely invented", "passage");
    let map = json!({
        "source_id": pid, "title": "Scaling", "coverage": "paper", "limitations": [],
        "nodes": [
            {"id": "a", "kind": "assumption", "main": true, "sentence": "Mass positive.", "anchors": [a1]},
            {"id": "p", "kind": "prediction", "main": true, "sentence": "Constant period.", "anchors": [a2]}
        ],
        "edges": [{"from": "a", "to": "p", "kind": "depends_on", "basis": "explicit", "reason": "Uses mass.", "anchor": bad_edge}]
    });
    let err: Value = serde_json::from_str(&record(&env, &ledger, SID, &map).unwrap_err()).unwrap();
    assert_eq!(err["error"], "anchors_failed");
    assert_eq!(err["faults"][0]["owner"], "edges[a->p].anchor");
}

#[test]
fn multiple_faults_one_error_capped_3500() {
    let long_lines: String = (1..=30).map(|i| format!("Line {i:02} {}", "x".repeat(160))).collect::<Vec<_>>().join("\n");
    let (_tmp, env, ledger, pid, _, _, _) = setup(&long_lines, None);
    let nodes: Vec<Value> = (0..20)
        .map(|i| json!({
            "id": format!("n{i}"), "kind": "result", "main": true, "sentence": format!("Claim {i}."),
            "anchors": [ptr(&pid, (i % 25) + 1, (i % 25) + 1, "totally missing nonexistent quote text here", "passage")]
        }))
        .collect();
    let map = json!({"source_id": pid, "title": "Many Faults", "coverage": "paper", "limitations": [], "nodes": nodes, "edges": []});
    let raw = record(&env, &ledger, SID, &map).unwrap_err();
    assert!(raw.chars().count() <= 3500, "len={}", raw.chars().count());
    let err: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(err["error"], "anchors_failed");
    let faults = err["faults"].as_array().unwrap();
    let more = err["more_faults"].as_u64().unwrap() as usize;
    assert_eq!(faults.len() + more, 20);
    assert!(faults.len() >= 2);
}

#[test]
fn legacy_fault_returns_legacy_message_even_with_pointers() {
    let (_tmp, env, ledger, pid, ppath, _, _) = setup(PAPER, None);
    let bad_ptr = ptr(&pid, 2, 2, "invented pointer text not in paper", "passage");
    let bad_legacy = json!({"source_id": pid, "path": ppath, "start_line": 3, "end_line": 3, "excerpt": "invented legacy excerpt", "kind": "passage"});
    let map = json!({
        "source_id": pid, "title": "Mixed", "coverage": "paper", "limitations": [],
        "nodes": [
            {"id": "n1", "kind": "result", "main": true, "sentence": "First.", "anchors": [bad_ptr]},
            {"id": "n2", "kind": "result", "main": true, "sentence": "Second.", "anchors": [bad_legacy]}
        ],
        "edges": []
    });
    let err = record(&env, &ledger, SID, &map).unwrap_err();
    assert!(err.contains("Excerpt does not match"), "got: {err}");
    assert!(!err.contains("anchors_failed"));
}

#[test]
fn conventions_three_items_one_bad_saved_two_done_false() {
    let (_tmp, env, ledger, pid, _, _, req) = setup(PAPER, None);
    let batch = json!({
        "request_seq": req, "source_id": pid, "batch_index": 1, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "ext-d", "category": "notation", "label": "d", "meaning": "Exterior derivative.", "scope": "Throughout", "basis": "explicit",
             "anchors": [ptr(&pid, 7, 7, "d denotes the exterior derivative operator", "passage")]},
            {"id": "freq-f", "category": "notation", "label": "f", "meaning": "Oscillation frequency.", "scope": "Main text", "basis": "explicit",
             "anchors": [ptr(&pid, 8, 8, "oscillation frequency by the symbol f", "passage")]},
            {"id": "bad-g", "category": "notation", "label": "g", "meaning": "Coupling strength.", "scope": "Appendix B", "basis": "explicit",
             "anchors": [ptr(&pid, 9, 9, "completely fabricated nonexistent quote", "passage")]}
        ]
    });
    let res = record_conventions(&env, &ledger, SID, &batch).unwrap();
    assert_eq!(res["items"], 2);
    assert_eq!(res["done"], false);
    assert_eq!(res["coverage"], "partial");
    assert_eq!(res["next_batch_index"], 2);
    assert_eq!(res["rejected_items"].as_array().unwrap().len(), 1);
    assert_eq!(res["rejected_items"][0]["id"], "bad-g");
    let saved = ledger.read_verified_session(SID).unwrap().1.pop().unwrap();
    assert_eq!(saved.payload["done_requested"], true);
    assert_eq!(saved.payload["coverage_reported"], "paper");
    assert!(saved.payload["limitations"][0].as_str().unwrap().contains("awaiting valid citations: bad-g"));
}

#[test]
fn rejected_item_resent_in_next_batch_no_label_collision() {
    let (_tmp, env, ledger, pid, _, _, req) = setup(PAPER, None);
    let b1 = json!({
        "request_seq": req, "source_id": pid, "batch_index": 1, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "ext-d", "category": "notation", "label": "d", "meaning": "Exterior derivative.", "scope": "Throughout", "basis": "explicit",
             "anchors": [ptr(&pid, 7, 7, "d denotes the exterior derivative operator", "passage")]},
            {"id": "bad-g", "category": "notation", "label": "g", "meaning": "Coupling strength.", "scope": "Appendix B", "basis": "explicit",
             "anchors": [ptr(&pid, 9, 9, "completely fabricated nonexistent quote", "passage")]}
        ]
    });
    let r1 = record_conventions(&env, &ledger, SID, &b1).unwrap();
    assert_eq!(r1["next_batch_index"], 2);
    let b2 = json!({
        "request_seq": req, "source_id": pid, "batch_index": 2, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "bad-g", "category": "notation", "label": "g", "meaning": "Coupling strength.", "scope": "Appendix B", "basis": "explicit",
             "anchors": [ptr(&pid, 9, 9, "symbol g denotes the coupling strength", "passage")]}
        ]
    });
    let r2 = record_conventions(&env, &ledger, SID, &b2).unwrap();
    assert_eq!(r2["done"], true);
    assert_eq!(r2["items"], 1);
    let events = ledger.read_verified_session(SID).unwrap().1;
    let catalog = paper_conventions(&events, req).unwrap();
    assert_eq!(catalog["items"].as_array().unwrap().len(), 2);
    assert_eq!(catalog["pending_items"], json!([]));
}

#[test]
fn identical_resend_returns_same_seq_and_rejected_items() {
    let (_tmp, env, ledger, pid, _, _, req) = setup(PAPER, None);
    let b1 = json!({
        "request_seq": req, "source_id": pid, "batch_index": 1, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "ext-d", "category": "notation", "label": "d", "meaning": "Exterior derivative.", "scope": "Throughout", "basis": "explicit",
             "anchors": [ptr(&pid, 7, 7, "d denotes the exterior derivative operator", "passage")]},
            {"id": "bad-g", "category": "notation", "label": "g", "meaning": "Coupling strength.", "scope": "Appendix B", "basis": "explicit",
             "anchors": [ptr(&pid, 9, 9, "completely fabricated nonexistent quote", "passage")]}
        ]
    });
    let r1 = record_conventions(&env, &ledger, SID, &b1).unwrap();
    let r2 = record_conventions(&env, &ledger, SID, &b1).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(r2["rejected_items"][0]["id"], "bad-g");
    let mut modified = b1.clone();
    modified["items"][0]["meaning"] = json!("Different meaning.");
    assert!(record_conventions(&env, &ledger, SID, &modified).unwrap_err().contains("already saved"));
}

#[test]
fn all_items_bad_no_append() {
    let (_tmp, env, ledger, pid, _, _, req) = setup(PAPER, None);
    let before = ledger.read_session(SID).unwrap().len();
    let batch = json!({
        "request_seq": req, "source_id": pid, "batch_index": 1, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "bad1", "category": "notation", "label": "x", "meaning": "X.", "scope": "All", "basis": "explicit",
             "anchors": [ptr(&pid, 2, 2, "totally invented text number one", "passage")]},
            {"id": "bad2", "category": "notation", "label": "y", "meaning": "Y.", "scope": "All", "basis": "explicit",
             "anchors": [ptr(&pid, 3, 3, "totally invented text number two", "passage")]}
        ]
    });
    let err: Value = serde_json::from_str(&record_conventions(&env, &ledger, SID, &batch).unwrap_err()).unwrap();
    assert_eq!(err["error"], "anchors_failed");
    assert_eq!(err["faults"].as_array().unwrap().len(), 2);
    assert_eq!(ledger.read_session(SID).unwrap().len(), before);
}

#[test]
fn pending_items_aggregated_across_batches() {
    let (_tmp, env, ledger, pid, _, _, req) = setup(PAPER, None);
    let b1 = json!({
        "request_seq": req, "source_id": pid, "batch_index": 1, "done": false, "coverage": "partial", "limitations": [],
        "items": [
            {"id": "ext-d", "category": "notation", "label": "d", "meaning": "Exterior derivative.", "scope": "Throughout", "basis": "explicit",
             "anchors": [ptr(&pid, 7, 7, "d denotes the exterior derivative operator", "passage")]},
            {"id": "miss-f", "category": "notation", "label": "f", "meaning": "Frequency.", "scope": "Main", "basis": "explicit",
             "anchors": [ptr(&pid, 8, 8, "fabricated quote for miss-f here", "passage")]},
            {"id": "miss-g", "category": "notation", "label": "g", "meaning": "Coupling.", "scope": "Appendix", "basis": "explicit",
             "anchors": [ptr(&pid, 9, 9, "fabricated quote for miss-g here", "passage")]}
        ]
    });
    record_conventions(&env, &ledger, SID, &b1).unwrap();
    let b2 = json!({
        "request_seq": req, "source_id": pid, "batch_index": 2, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "miss-f", "category": "notation", "label": "f", "meaning": "Frequency.", "scope": "Main", "basis": "explicit",
             "anchors": [ptr(&pid, 8, 8, "oscillation frequency by the symbol f", "passage")]},
            {"id": "miss-sig", "category": "sign", "label": "metric", "meaning": "Mostly plus.", "scope": "Throughout", "basis": "explicit",
             "anchors": [ptr(&pid, 10, 10, "fabricated quote for miss-sig here", "passage")]}
        ]
    });
    record_conventions(&env, &ledger, SID, &b2).unwrap();
    let events = ledger.read_verified_session(SID).unwrap().1;
    let catalog = paper_conventions(&events, req).unwrap();
    assert_eq!(catalog["pending_items"], json!(["miss-g", "miss-sig"]));
}

#[test]
fn summary_bad_part_reports_specific_error() {
    let (_tmp, env, ledger, pid, _, _, _) = setup(PAPER, None);
    let good_a = ptr(&pid, 2, 2, "assume strictly positive mass throughout", "passage");
    let bad = json!({
        "source_id": pid,
        "what": {"text": "What statement.", "anchors": [good_a]},
        "why": {"text": "Why statement.", "anchors": [], "bogus_field": 123},
        "how": {"unstated": true}
    });
    let err = record_summary(&env, &ledger, SID, &bad).unwrap_err();
    assert!(err.contains("why"), "got: {err}");
    assert!(err.contains("bogus_field"), "got: {err}");
    assert!(!err.contains("did not match any variant"));
}

#[test]
fn fabricated_quote_no_new_event() {
    let (_tmp, env, ledger, pid, _, _, _) = setup(PAPER, None);
    let before = ledger.read_session(SID).unwrap().len();
    let bad = ptr(&pid, 2, 3, "this fabricated sentence never appears anywhere", "passage");
    let summary = json!({
        "source_id": pid,
        "what": {"text": "Claim.", "anchors": [bad.clone()]},
        "why": {"unstated": true}, "how": {"unstated": true}
    });
    assert!(record_summary(&env, &ledger, SID, &summary).is_err());
    let map = json!({
        "source_id": pid, "title": "T", "coverage": "paper", "limitations": [],
        "nodes": [{"id": "n1", "kind": "result", "main": true, "sentence": "S.", "anchors": [bad]}],
        "edges": []
    });
    assert!(record(&env, &ledger, SID, &map).is_err());
    assert_eq!(ledger.read_session(SID).unwrap().len(), before);
}

#[test]
fn regression_five_live_failure_classes() {
    let paper = concat!(
        "Line 1 header.\n",
        "Line 2 filler.\n",
        "Line 3 filler.\n",
        "Line 4 filler.\n",
        "We consider two-dimensional (2d) quantum\n",           // line 5
        "lattice models with topological order.\n",             // line 6
        "Below, we demonstrate the fusion rule (52).\n",        // line 7
        "Line 8 filler.\n",
        "Line 9 filler.\n",
        "Line 10 filler.\n",
        "Therefore after mov-\n",                               // line 11
        "ing the 4 dislocations into the plaquette.\n",         // line 12
        "TABLE A2.\n",                                          // line 13
        "Cocycle table row one with group elements.\n",         // line 14
        "Cocycle table row two with abelian phases.\n",         // line 15
        "The exact sequence terminates at degree three.\n",     // line 16
        "\n",                                                   // line 17 (trailing blank)
    );
    let (_tmp, env, ledger, pid, _, _, req) = setup(paper, None);

    // Class 1 (range wider than exact quote) + Class 3 (shifted line numbers) in summary:
    let summary = json!({
        "source_id": pid,
        "what": {"text": "Studies 2d quantum lattice models.",
            "anchors": [ptr(&pid, 9, 11, "two-dimensional (2d) quantum lattice models", "passage")]},
        "why": {"unstated": true},
        "how": {"text": "Demonstrates fusion rule (52).",
            "anchors": [ptr(&pid, 1, 12, "demonstrate the fusion rule (52)", "passage")]}
    });
    let sres = record_summary(&env, &ledger, SID, &summary).unwrap();
    assert_eq!(sres["anchor_report"]["corrected"].as_array().unwrap().len(), 2);

    // Class 4 (one-character slip at hyphen break) on one anchor alongside a good anchor in map:
    let map = json!({
        "source_id": pid, "title": "Live Classes", "coverage": "paper", "limitations": [],
        "nodes": [{
            "id": "fusion", "kind": "result", "main": true, "sentence": "Fusion rule holds.",
            "anchors": [
                ptr(&pid, 7, 7, "demonstrate the fusion rule (52)", "equation"),
                ptr(&pid, 11, 12, "after mov- ning the 4 dislocations", "passage")
            ]
        }],
        "edges": []
    });
    let mres = record(&env, &ledger, SID, &map).unwrap();
    assert_eq!(mres["anchor_report"]["dropped"].as_array().unwrap().len(), 1);
    assert_eq!(mres["anchor_report"]["dropped"][0]["code"], "quote_not_found");

    // Class 2 (trailing blank line) + Class 5 (label one line above) in conventions:
    let mut table_anchor = ptr(&pid, 14, 15, "Cocycle table row one with group elements", "table");
    table_anchor["label"] = json!("TABLE A2.");
    let conv = json!({
        "request_seq": req, "source_id": pid, "batch_index": 1, "done": true, "coverage": "paper", "limitations": [],
        "items": [
            {"id": "cocycle", "category": "algebra", "label": "TABLE A2.", "meaning": "Cocycle axes.", "scope": "Appendix", "basis": "explicit",
             "anchors": [table_anchor]},
            {"id": "exact-seq", "category": "notation", "label": "exact sequence", "meaning": "Degree 3.", "scope": "Section V", "basis": "explicit",
             "anchors": [ptr(&pid, 16, 17, "exact sequence terminates at degree three", "passage")]}
        ]
    });
    let cres = record_conventions(&env, &ledger, SID, &conv).unwrap();
    assert_eq!(cres["done"], true);
    assert_eq!(cres["items"], 2);
    assert_eq!(cres["anchor_report"]["corrected"].as_array().unwrap().len(), 2);

    // Also verify branch recorder accepts pointer anchors and reports corrections.
    let map_seq = mres["map_seq"].as_u64().unwrap();
    let breq = ledger.append(SID, "investigation.branch_requested", 1, &json!({"map_seq": map_seq, "source_id": pid, "node_id": "fusion", "question": "How do dislocations fuse?"})).unwrap();
    let bres = record_branch(&env, &ledger, SID, &json!({
        "request_seq": breq, "source_id": pid,
        "nodes": [{"id": "disloc", "kind": "method", "main": false, "sentence": "Move 4 dislocations.", "anchors": [ptr(&pid, 11, 12, "ing the 4 dislocations into the plaquette", "passage")]}],
        "edges": [{"from": "disloc", "to": "fusion", "kind": "supports", "basis": "explicit", "reason": "Moves dislocations.", "anchor": ptr(&pid, 11, 12, "ing the 4 dislocations into the plaquette", "passage")}],
        "limitations": []
    })).unwrap();
    assert_eq!(bres["anchor_basis"]["cited_lines"], 2);
}
