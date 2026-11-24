use ledger::Ledger;
use research_tools::investigation::record;
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::sync::Mutex;

const SID: &str = "investigate-map";
const PAPER:&str="A test of scaling\nWe assume positive mass.\nEquation (1) predicts a constant period.\nFigure 2 reports the measured period.\nThe data support the scaling result [1].\n[1] An earlier experiment.\n";

fn upload(
    env: &ResearchEnv,
    ledger: &Mutex<Ledger>,
    name: &str,
    text: &str,
) -> research_tools::IngestedSource {
    ingest_bytes(
        &env.sandbox(),
        ledger,
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
}
fn anchor(src: &research_tools::IngestedSource, line: usize, excerpt: &str, kind: &str) -> Value {
    json!({"source_id":src.source_id,"path":src.primary_path,"start_line":line,"end_line":line,"excerpt":excerpt,"kind":kind})
}
fn shape(
    src: &research_tools::IngestedSource,
    reference: &research_tools::IngestedSource,
) -> Value {
    let a = anchor(src, 2, "We assume positive mass.", "passage");
    let eq = anchor(
        src,
        3,
        "Equation (1) predicts a constant period.",
        "equation",
    );
    let obs = anchor(src, 4, "Figure 2 reports the measured period.", "figure");
    let result = anchor(
        src,
        5,
        "The data support the scaling result [1].",
        "passage",
    );
    let cite = anchor(src, 6, "[1] An earlier experiment.", "reference");
    let ref_anchor = anchor(reference, 1, "An earlier measurement.", "passage");
    let node = |id: &str, kind: &str, text: &str, main: bool, anchor: &Value| json!({"id":id,"kind":kind,"sentence":text,"main":main,"anchors":[anchor]});
    let edge = |from: &str, to: &str, kind: &str, basis: &str, reason: &str, anchor: &Value| json!({"from":from,"to":to,"kind":kind,"basis":basis,"reason":reason,"anchor":anchor});
    let mut reference_node = node(
        "ref",
        "reference",
        "An earlier measurement is cited.",
        false,
        &cite,
    );
    reference_node["anchors"]
        .as_array_mut()
        .unwrap()
        .push(ref_anchor);
    json!({"source_id":src.source_id,"title":"Scaling","coverage":"partial","limitations":["The error model is not described."],
        "nodes":[node("a","assumption","Mass is positive.",false,&a),node("p","prediction","The period should be constant.",false,&eq),
        node("obs","measurement","The period was measured.",false,&obs),node("result","conclusion","The experiment supports scaling.",true,&result),reference_node],
        "edges":[edge("a","p","depends_on","inferred","Positive mass sets the regime for the prediction.",&eq),
        edge("obs","result","supports","explicit","The authors use the data for this conclusion.",&result),
        edge("ref","result","cites","explicit","The paper cites the earlier experiment at the conclusion.",&result)]})
}

#[test]
fn map_is_grounded_and_scoped_to_the_selected_paper_and_session_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let paper = upload(&env, &db, "Scaling.txt", PAPER);
    let other = upload(&env, &db, "Earlier.txt", "An earlier measurement.");
    let map = shape(&paper, &other);
    let ledger = db.lock().unwrap();
    assert!(record(&env, &ledger, SID, &map).is_err());
    ledger
        .append(SID, "investigation.started", 1, &json!({}))
        .unwrap();
    assert!(record(&env, &ledger, SID, &map).is_err());
    ledger
        .append(
            SID,
            "investigation.paper",
            1,
            &json!({"source_id":paper.source_id}),
        )
        .unwrap();
    assert!(record(&env, &ledger, SID, &map).is_err());
    let request = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id":paper.source_id}),
        )
        .unwrap();
    let report = record(&env, &ledger, SID, &map).unwrap();
    let seq = report["map_seq"].as_u64().unwrap();
    assert_eq!(record(&env, &ledger, SID, &map).unwrap()["map_seq"], seq);
    let (_, events) = ledger.read_verified_session(SID).unwrap();
    let maps: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "investigation.map")
        .collect();
    assert_eq!(maps.len(), 1);
    let saved = &maps[0].payload;
    assert_eq!(saved["request_seq"], request);
    assert_eq!(saved["coverage_basis"], "model_reported");
    assert_eq!(saved["anchors_verified"], true);
    let proof = &saved["nodes"][0]["anchors"][0];
    assert_eq!(proof["source_seq"], paper.event_seq);
    assert_eq!(proof["file_sha256"], ledger::sha256_hex(PAPER.as_bytes()));
    assert_eq!(
        saved["nodes"][4]["anchors"][1]["source_seq"],
        other.event_seq
    );
    assert_eq!(saved["edges"][0]["basis"], "inferred");
    let mut other_map = map.clone();
    other_map["title"] = json!("Attempt to overwrite this review");
    assert!(record(&env, &ledger, SID, &other_map)
        .unwrap_err()
        .contains("already saved"));
    ledger
        .append(
            SID,
            "investigation.mapping_finished",
            1,
            &json!({"request_seq":request,"status":"recorded"}),
        )
        .unwrap();
    assert!(record(&env, &ledger, SID, &map)
        .unwrap_err()
        .contains("Start mapping"));
    let request = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id":paper.source_id}),
        )
        .unwrap();
    let bad = |edit: fn(&mut Value)| {
        let mut invalid = map.clone();
        edit(&mut invalid);
        assert!(record(&env, &ledger, SID, &invalid).is_err(), "{invalid}");
    };
    bad(|m| m["source_id"] = json!("another-paper"));
    bad(|m| m["nodes"][0]["anchors"][0]["excerpt"] = json!("an invented quote"));
    bad(|m| m["nodes"][0]["anchors"][0]["start_line"] = json!(1));
    bad(|m| m["nodes"][0]["anchors"][0]["end_line"] = json!(345));
    bad(|m| m["nodes"][2]["anchors"][0]["label"] = json!("Figure 99"));
    bad(|m| m["nodes"][0]["anchors"][0]["path"] = json!("../secret"));
    bad(|m| m["nodes"][0]["anchors"] = json!([]));
    bad(|m| m["nodes"][1]["id"] = json!("a"));
    bad(|m| m["edges"][0]["from"] = json!("missing"));
    bad(|m| m["edges"][2]["kind"] = json!("supports"));
    bad(|m| m["limitations"] = json!([]));
    bad(|m| {
        let mut back = m["edges"][0].clone();
        back["from"] = json!("p");
        back["to"] = json!("a");
        m["edges"].as_array_mut().unwrap().push(back);
    });
    bad(|m| m["nodes"][0]["anchors"] = json!([m["nodes"][4]["anchors"][1].clone()]));
    let path = env
        .sandbox()
        .resolve_existing(
            research_tools::SandboxRoot::Sources,
            SID,
            paper.primary_path.as_deref().unwrap(),
            false,
        )
        .unwrap();
    std::fs::write(path.canonical_target, b"source replaced on disk").unwrap();
    assert!(record(&env, &ledger, SID, &map)
        .unwrap_err()
        .contains("changed since ingestion"));
    ledger
        .append(
            SID,
            "investigation.mapping_finished",
            1,
            &json!({"request_seq":request,"status":"recorded"}),
        )
        .unwrap();
    assert!(record(&env, &ledger, SID, &map)
        .unwrap_err()
        .contains("Start mapping"));
    ledger
        .append(SID, "investigation.handoff", 1, &json!({}))
        .unwrap();
    assert!(record(&env, &ledger, SID, &map)
        .unwrap_err()
        .contains("only be recorded in an Investigation"));
    assert_eq!(
        ledger
            .read_session(SID)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "investigation.map")
            .count(),
        1
    );
}
#[test]
fn concurrent_request_closure_prevents_a_stale_map_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let recorder = Ledger::open(tmp.path()).unwrap();
    let started = recorder
        .append_with_hash(SID, "investigation.mapping_requested", 1, &json!({}))
        .unwrap();
    let desktop = Ledger::open(tmp.path()).unwrap();
    desktop
        .append(
            SID,
            "investigation.mapping_finished",
            1,
            &json!({"request_seq":started.seq}),
        )
        .unwrap();
    assert!(matches!(
        recorder.append_at_head(SID, "investigation.map", 1, &json!({}), &started.event_hash),
        Err(ledger::Error::SessionChanged(_))
    ));
    assert_eq!(recorder.verify_session(SID).unwrap().event_count, 2);
}

#[test]
fn selected_pdf_pages_apply_to_every_primary_anchor_and_allow_a_linked_reference() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let paper = upload(&env, &db, "paper.txt", PAPER);
    let external = upload(&env, &db, "ref.txt", "An earlier measurement.");
    let ledger = db.lock().unwrap();
    let mut receipt = ledger
        .read_session(SID)
        .unwrap()
        .iter()
        .find(|e| e.seq == paper.event_seq)
        .unwrap()
        .payload
        .clone();
    // Model the extractor's trusted physical-page boundaries; literal text in the source is irrelevant.
    receipt["format"] = json!("pdf");
    receipt["page_count"] = json!(3);
    receipt["pdf_page_lines"] = json!([[1, 1, 1], [2, 2, 6], [3, 7, 7]]);
    ledger.append(SID, "source.ingested", 1, &receipt).unwrap();
    ledger
        .append(SID, "investigation.started", 1, &json!({}))
        .unwrap();
    ledger
        .append(
            SID,
            "investigation.paper",
            1,
            &json!({"source_id":paper.source_id,"page_start":2,"page_end":2}),
        )
        .unwrap();
    ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id":paper.source_id}),
        )
        .unwrap();
    let valid = shape(&paper, &external);
    let outside = anchor(&paper, 1, "A test of scaling", "passage");
    let mut wrong = valid.clone();
    wrong["nodes"][0]["anchors"][0] = outside.clone();
    assert!(record(&env, &ledger, SID, &wrong)
        .unwrap_err()
        .contains("within selected PDF pages"));
    wrong = valid.clone();
    wrong["edges"][1]["anchor"] = outside;
    assert!(record(&env, &ledger, SID, &wrong)
        .unwrap_err()
        .contains("within selected PDF pages"));
    wrong = valid.clone();
    wrong["nodes"][0]["anchors"][0]["end_line"] = json!(7);
    assert!(record(&env, &ledger, SID, &wrong)
        .unwrap_err()
        .contains("within selected PDF pages"));
    let result = record(&env, &ledger, SID, &valid).unwrap();
    let saved = ledger.read_verified_session(SID).unwrap().1.pop().unwrap();
    assert_eq!(saved.seq, result["map_seq"].as_u64().unwrap());
    assert_eq!(saved.payload["page_start"], 2);
    assert_eq!(saved.payload["page_end"], 2);
}
