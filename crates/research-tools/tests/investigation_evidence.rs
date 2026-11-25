//! WP7 tests: isolated paper-reader evidence store (`note_evidence`), reader router tools,
//! and recorder evidence resolution (`resolve_evidence`).

use ledger::Ledger;
use research_tools::investigation::record;
use research_tools::investigation_evidence::{
    note_evidence_with_hook, register_reader_tools, EVIDENCE_EVENT, NOTE_EVIDENCE,
};
use research_tools::{
    all_research_tool_defs, ingest_bytes, register_research_tools, research_policy, IngestRequest,
    IngestedSource, ResearchEnv,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, CallOutcome, Registry, Router};

const SID: &str = "inv-evidence-test";
const READER_CALLER: &str = "worker:paper_reader:anthropic:inv-evidence-test:r0";

struct AllowAll;
impl Approver for AllowAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "test".into(),
        }
    }
}

struct Fixture {
    tmp: TempDir,
    env: Arc<ResearchEnv>,
    paper: IngestedSource,
    other: IngestedSource,
    marker_seq: u64,
}

const SAMPLE_PAPER: &str = "\
Title: Topological Band Theory in One Dimension\n\
Abstract: We prove a bulk-boundary correspondence for chiral 1D chains.\n\
Section I. Introduction\n\
Throughout this paper, hbar = 1 and lattice spacing a = 1.\n\
Theorem 1: The winding number nu equals the count of zero-energy edge modes.\n\
Equation (1): H(k) = d_x(k) sigma_x + d_y(k) sigma_y.\n\
Section II. Proof of Theorem 1\n\
By Cauchy's argument principle, the phase winding determines the zero modes.\n";

fn setup_fixture(paper_text: &str) -> Fixture {
    let tmp = TempDir::new().unwrap();
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
            source_id_hint: Some("paper-main"),
            section_filter: None,
            title: Some("Topological Band Theory"),
        },
    )
    .unwrap();
    let other = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "other.txt",
            origin: "upload",
            raw_bytes: b"Another unrelated reference paper with different claims.\n",
            source_id_hint: Some("paper-ref"),
            section_filter: None,
            title: Some("Reference"),
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger.append(SID, "investigation.started", 1, &json!({})).unwrap();
    ledger
        .append(
            SID,
            "investigation.paper",
            1,
            &json!({"source_id": paper.source_id, "question": "What is Theorem 1?"}),
        )
        .unwrap();
    let marker_seq = ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": paper.source_id}),
        )
        .unwrap();
    Fixture {
        tmp,
        env,
        paper,
        other,
        marker_seq,
    }
}

fn reader_router(f: &Fixture) -> Router {
    let mut reg = Registry::new();
    register_reader_tools(&mut reg, Arc::clone(&f.env));
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    Router::new(reg, research_policy(), ledger, SID, Arc::new(AllowAll))
}

fn note_one(router: &Router, source_id: &str, start: usize, end: usize, quote: &str) -> String {
    let outcome = router
        .try_call(
            READER_CALLER,
            NOTE_EVIDENCE,
            &json!({
                "items": [{
                    "source_id": source_id,
                    "start_line": start,
                    "end_line": end,
                    "quote": quote,
                    "kind": "passage",
                    "role": "result",
                    "gist": "Theorem 1 links winding number to edge modes."
                }]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = outcome else {
        panic!("expected note_evidence Ok, got {outcome:?}");
    };
    assert_eq!(val["noted"], 1, "{val}");
    val["results"][0]["evidence"].as_str().unwrap().to_string()
}

fn map_with_evidence(source_id: &str, ev_id: &str, quote: &str) -> Value {
    json!({
        "source_id": source_id,
        "title": "Topological Band Theory",
        "coverage": "paper",
        "limitations": [],
        "nodes": [{
            "id": "thm1",
            "sentence": "The winding number equals the count of zero-energy edge modes.",
            "kind": "result",
            "main": true,
            "anchors": [{
                "source_id": source_id,
                "evidence": ev_id,
                "quote": quote,
                "kind": "passage"
            }]
        }],
        "edges": []
    })
}

#[test]
fn refused_without_active_marker_after_finished_after_session_end() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let args = json!({
        "items": [{
            "source_id": f.paper.source_id,
            "start_line": 5,
            "end_line": 5,
            "quote": "The winding number nu equals the count of zero-energy edge modes.",
            "kind": "passage",
            "role": "result",
            "gist": "Theorem 1 statement"
        }]
    });

    // After mapping_finished -> refused.
    ledger
        .append(
            SID,
            "investigation.mapping_finished",
            1,
            &json!({"request_seq": f.marker_seq}),
        )
        .unwrap();
    let out = router.try_call(READER_CALLER, NOTE_EVIDENCE, &args).unwrap();
    assert!(matches!(out, CallOutcome::Error(_)), "{out:?}");

    // Open a new marker then close with session.end -> refused.
    ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": f.paper.source_id}),
        )
        .unwrap();
    ledger
        .append(SID, "session.end", 1, &json!({"outcome": "completed"}))
        .unwrap();
    let out2 = router.try_call(READER_CALLER, NOTE_EVIDENCE, &args).unwrap();
    assert!(matches!(out2, CallOutcome::Error(_)), "{out2:?}");

    // Session without any investigation.started at all -> refused.
    let tmp2 = TempDir::new().unwrap();
    let env2 = Arc::new(ResearchEnv::new(tmp2.path()));
    let mut reg2 = Registry::new();
    register_reader_tools(&mut reg2, env2);
    let l2 = Ledger::open(tmp2.path()).unwrap();
    l2.append("plain-sess", "session.start", 1, &json!({})).unwrap();
    let r2 = Router::new(reg2, research_policy(), l2, "plain-sess", Arc::new(AllowAll));
    let out3 = r2.try_call(READER_CALLER, NOTE_EVIDENCE, &args).unwrap();
    assert!(matches!(out3, CallOutcome::Error(_)), "{out3:?}");
}

#[test]
fn refused_for_non_reader_caller() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let args = json!({
        "items": [{
            "source_id": f.paper.source_id,
            "start_line": 5,
            "end_line": 5,
            "quote": "The winding number nu equals the count of zero-energy edge modes.",
            "kind": "passage",
            "role": "result",
            "gist": "Theorem 1"
        }]
    });
    for bad_caller in [
        "agent:anthropic:inv-evidence-test",
        "worker:compute_worker:anthropic:inv-evidence-test",
        "worker:literature_worker:openai:inv-evidence-test",
        "user:desktop",
    ] {
        let out = router.try_call(bad_caller, NOTE_EVIDENCE, &args).unwrap();
        assert!(
            matches!(out, CallOutcome::Error(ref m) if m.contains("paper_reader")),
            "caller `{bad_caller}` should be refused, got {out:?}"
        );
    }
}

#[test]
fn batch_per_item_results() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let out = router
        .try_call(
            READER_CALLER,
            NOTE_EVIDENCE,
            &json!({
                "items": [
                    {
                        "source_id": f.paper.source_id,
                        "start_line": 5,
                        "end_line": 5,
                        "quote": "The winding number nu equals the count of zero-energy edge modes.",
                        "kind": "passage",
                        "role": "result",
                        "gist": "Theorem 1"
                    },
                    {
                        "source_id": f.paper.source_id,
                        "start_line": 5,
                        "end_line": 5,
                        "quote": "This sentence does not exist anywhere in the paper.",
                        "kind": "passage",
                        "role": "result",
                        "gist": "Fabricated claim"
                    },
                    {
                        "source_id": f.paper.source_id,
                        "start_line": 4,
                        "end_line": 4,
                        "quote": "hbar = 1 and lattice spacing a = 1.",
                        "kind": "passage",
                        "role": "convention",
                        "gist": "Natural units and unit lattice spacing."
                    }
                ]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else {
        panic!("expected Ok, got {out:?}");
    };
    assert_eq!(val["noted"], 2, "{val}");
    assert_eq!(val["failed"], 1, "{val}");
    assert_eq!(val["results"][0]["ok"], true);
    assert!(val["results"][0]["evidence"].as_str().unwrap().starts_with("ev-"));
    assert_eq!(val["results"][1]["ok"], false);
    assert_eq!(val["results"][1]["error"], "quote_not_found");
    assert_eq!(val["results"][2]["ok"], true);
    assert!(val["results"][2]["evidence"].as_str().unwrap().starts_with("ev-"));
}

#[test]
fn two_routers_session_changed_retried_not_surfaced() {
    let f = setup_fixture(SAMPLE_PAPER);
    let ledger_a = Mutex::new(Ledger::open(f.tmp.path()).unwrap());
    let ledger_b = Ledger::open(f.tmp.path()).unwrap();
    let args = json!({
        "items": [{
            "source_id": f.paper.source_id,
            "start_line": 5,
            "end_line": 5,
            "quote": "The winding number nu equals the count of zero-energy edge modes.",
            "kind": "passage",
            "role": "result",
            "gist": "Theorem 1"
        }]
    });
    // Interleave an append from a second connection on attempts 0 and 1 so `append_at_head`
    // hits `SessionChanged` twice and succeeds on retry attempt 2.
    let hook = |_idx: usize, attempt: usize| {
        if attempt < 2 {
            ledger_b
                .append(SID, "ledger.note", 1, &json!({"note": format!("concurrent-{attempt}")}))
                .unwrap();
        }
    };
    let val = note_evidence_with_hook(&f.env, &ledger_a, SID, READER_CALLER, &args, Some(&hook))
        .unwrap();
    assert_eq!(val["noted"], 1, "{val}");
    assert_eq!(val["failed"], 0, "{val}");
    assert!(val["results"][0]["evidence"].as_str().unwrap().starts_with("ev-"));
    Ledger::open(f.tmp.path()).unwrap().verify_session(SID).unwrap();
}

#[test]
fn evidence_pointing_at_tool_request_refused() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let _ev = note_one(
        &router,
        &f.paper.source_id,
        5,
        5,
        "The winding number nu equals the count of zero-energy edge modes.",
    );
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let events = ledger.read_session(SID).unwrap();
    let req_seq = events
        .iter()
        .find(|e| e.kind == "tool.request")
        .unwrap()
        .seq;
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(
            &f.paper.source_id,
            &format!("ev-{req_seq}"),
            "The winding number nu equals the count of zero-energy edge modes.",
        ),
    )
    .unwrap_err();
    assert!(err.contains("evidence_wrong_kind"), "{err}");
}

#[test]
fn evidence_from_earlier_started_refused() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let quote = "The winding number nu equals the count of zero-energy edge modes.";
    let old_ev = note_one(&router, &f.paper.source_id, 5, 5, quote);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    // Restart investigation with a new `investigation.started`.
    ledger.append(SID, "investigation.started", 1, &json!({})).unwrap();
    ledger
        .append(
            SID,
            "investigation.paper",
            1,
            &json!({"source_id": f.paper.source_id}),
        )
        .unwrap();
    ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": f.paper.source_id}),
        )
        .unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, &old_ev, quote),
    )
    .unwrap_err();
    assert!(err.contains("evidence_not_found"), "{err}");
}

#[test]
fn evidence_other_marker_refused() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let quote = "The winding number nu equals the count of zero-energy edge modes.";
    let ev1 = note_one(&router, &f.paper.source_id, 5, 5, quote);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    // Open a newer mapping_requested marker in the same Investigation.
    ledger
        .append(
            SID,
            "investigation.mapping_requested",
            1,
            &json!({"source_id": f.paper.source_id}),
        )
        .unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, &ev1, quote),
    )
    .unwrap_err();
    assert!(err.contains("evidence_marker_mismatch"), "{err}");
}

#[test]
fn unknown_seq_refused() {
    let f = setup_fixture(SAMPLE_PAPER);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(
            &f.paper.source_id,
            "ev-999999",
            "The winding number nu equals the count of zero-energy edge modes.",
        ),
    )
    .unwrap_err();
    assert!(err.contains("evidence_not_found"), "{err}");
}

#[test]
fn other_source_refused() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let quote = "Another unrelated reference paper with different claims.";
    let out = router
        .try_call(
            READER_CALLER,
            NOTE_EVIDENCE,
            &json!({
                "items": [{
                    "source_id": f.other.source_id,
                    "path": f.other.primary_path.as_ref().unwrap(),
                    "start_line": 1,
                    "end_line": 1,
                    "quote": quote,
                    "kind": "passage",
                    "role": "result",
                    "gist": "Other paper claim"
                }]
            }),
        )
        .unwrap();
    let CallOutcome::Ok(val) = out else {
        panic!("expected Ok, got {out:?}");
    };
    let other_ev = val["results"][0]["evidence"].as_str().unwrap();
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, other_ev, quote),
    )
    .unwrap_err();
    assert!(err.contains("evidence_source_mismatch"), "{err}");
}

#[test]
fn rewritten_file_refused() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let quote = "The winding number nu equals the count of zero-energy edge modes.";
    let ev_id = note_one(&router, &f.paper.source_id, 5, 5, quote);
    // Tamper with the ingested file on disk after noting evidence.
    let disk_path = f
        .tmp
        .path()
        .join("sessions")
        .join(SID)
        .join("sources")
        .join(f.paper.primary_path.as_ref().unwrap());
    std::fs::write(&disk_path, "Rewritten tampered file contents.\n").unwrap();

    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, &ev_id, quote),
    )
    .unwrap_err();
    assert!(
        err.contains("source_load_failed") || err.contains("changed since ingestion"),
        "{err}"
    );
}

#[test]
fn window_reslice_verified() {
    let long_prefix = "A".repeat(2000);
    let long_suffix = "B".repeat(2000);
    let quote = "Unique theorem statement in the middle of a very long line.";
    let long_line = format!("{long_prefix} {quote} {long_suffix}\n");
    let f = setup_fixture(&long_line);
    let router = reader_router(&f);
    let ev_id = note_one(&router, &f.paper.source_id, 1, 1, quote);

    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let events = ledger.read_session(SID).unwrap();
    let ev = events.iter().find(|e| e.kind == EVIDENCE_EVENT).unwrap();
    assert!(
        ev.payload["excerpt_window"].is_array(),
        "long line must produce excerpt_window: {}",
        ev.payload
    );
    assert!(ev.payload["excerpt"].as_str().unwrap().len() <= 3200);

    let receipt = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, &ev_id, quote),
    )
    .unwrap();
    let map_seq = receipt["map_seq"].as_u64().unwrap();
    let events_after = ledger.read_session(SID).unwrap();
    let map_ev = events_after.iter().find(|e| e.seq == map_seq).unwrap();
    let anchor = &map_ev.payload["nodes"][0]["anchors"][0];
    assert_eq!(anchor["excerpt_basis"], "evidence");
    assert_eq!(anchor["excerpt_window"], ev.payload["excerpt_window"]);
    assert!(anchor["excerpt"].as_str().unwrap().contains(quote));

    // Also verify tampered excerpt_window is rejected.
    let mut bad_payload = ev.payload.clone();
    bad_payload["excerpt_window"] = json!([0, 10]);
    let bad_seq = ledger
        .append(SID, EVIDENCE_EVENT, 1, &bad_payload)
        .unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, &format!("ev-{bad_seq}"), quote),
    )
    .unwrap_err();
    assert!(err.contains("evidence_excerpt_mismatch"), "{err}");
}

#[test]
fn recorder_quote_must_be_in_excerpt() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let ev_id = note_one(
        &router,
        &f.paper.source_id,
        5,
        5,
        "The winding number nu equals the count of zero-energy edge modes.",
    );
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let err = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(
            &f.paper.source_id,
            &ev_id,
            "A totally different quote not present in line 5.",
        ),
    )
    .unwrap_err();
    assert!(err.contains("evidence_quote_mismatch"), "{err}");
}

#[test]
fn accepted_with_basis_evidence() {
    let f = setup_fixture(SAMPLE_PAPER);
    let router = reader_router(&f);
    let quote = "The winding number nu equals the count of zero-energy edge modes.";
    let ev_id = note_one(&router, &f.paper.source_id, 5, 5, quote);
    let ledger = Ledger::open(f.tmp.path()).unwrap();
    let receipt = record(
        &f.env,
        &ledger,
        SID,
        &map_with_evidence(&f.paper.source_id, &ev_id, quote),
    )
    .unwrap();
    let map_seq = receipt["map_seq"].as_u64().unwrap();
    let events = ledger.read_session(SID).unwrap();
    let map_ev = events.iter().find(|e| e.seq == map_seq).unwrap();
    let anchor = &map_ev.payload["nodes"][0]["anchors"][0];
    assert_eq!(anchor["excerpt_basis"], "evidence");
    assert_eq!(anchor["evidence"], ev_id);
    assert_eq!(anchor["start_line"], 5);
    assert_eq!(anchor["end_line"], 5);
    assert_eq!(anchor["path"], f.paper.primary_path.as_ref().unwrap().as_str());
    assert!(anchor["excerpt"].as_str().unwrap().contains(quote));
    ledger.verify_session(SID).unwrap();
}

#[test]
fn main_registry_lacks_note_evidence() {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let mut main_reg = Registry::new();
    register_research_tools(&mut main_reg, Arc::clone(&env));
    assert!(
        main_reg.get(NOTE_EVIDENCE).is_none(),
        "note_evidence must NEVER be in the main registry"
    );
    assert!(
        !all_research_tool_defs().iter().any(|d| d.id == NOTE_EVIDENCE),
        "note_evidence must NEVER be in all_research_tool_defs"
    );

    let mut r_reg = Registry::new();
    register_reader_tools(&mut r_reg, env);
    let mut ids: Vec<&str> = r_reg.defs().iter().map(|d| d.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["note_evidence", "read_paper_lines", "search_paper_passages"]
    );
}
