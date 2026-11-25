//! Honesty-lens audit proof tests for relocated label lookback + bare section heading
//! bypass (`"Appendix A"`, `"Section 1. Introduction"`, `"§2 Methods"`), and `excerpt_window`
//! duplicate-occurrence mis-centering after `label_extended`.

use ledger::Ledger;
use research_tools::investigation::record;
use research_tools::investigation_evidence::note_evidence;
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::json;
use std::sync::Mutex;

const SID: &str = "audit-honesty-relocated-appendix";
const READER_CALLER: &str = "worker:paper_reader:ollama:audit-honesty-relocated-appendix:r0";

fn setup_session(paper_text: &str) -> (tempfile::TempDir, ResearchEnv, Mutex<Ledger>, String) {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: paper_text.as_bytes(),
            source_id_hint: Some("paper1"),
            section_filter: None,
            title: Some("Audit Paper"),
        },
    )
    .unwrap();
    {
        let ledger = db.lock().unwrap();
        ledger
            .append(SID, "investigation.started", 1, &json!({}))
            .unwrap();
        ledger
            .append(
                SID,
                "investigation.paper",
                1,
                &json!({"source_id": src.source_id}),
            )
            .unwrap();
        ledger
            .append(
                SID,
                "investigation.mapping_requested",
                1,
                &json!({"source_id": src.source_id}),
            )
            .unwrap();
    }
    (tmp, env, db, src.source_id)
}

/// Finding F18 (major):
/// 1. `is_bare_section_name` (`investigation_anchor.rs:498-513`) only strips a leading
///    number/Roman/letter before a section keyword, returning `false` on `"Appendix A"`,
///    `"Section 1. Introduction"`, `"Sec. II. Results"`, `"§2 Methods"`, and `"II.A Discussion"`.
/// 2. `verify_pointer` (`investigation_anchor.rs:396`) applies `s.min(final_start - 3)` even when
///    `relocated == true`: citing `1..=52` with a 16-non-space-char quote relocated +12 lines to
///    line 64 and `label: "Appendix A"` at line 1 extends `final_start` 63 lines upward back to 1.
#[test]
fn audit_honesty_relocated_label_and_bare_appendix_must_not_ground_64_lines() {
    let mut lines = vec!["Appendix A".to_string()];
    for i in 2..=63 {
        lines.push(format!("Unquoted intermediate claim statement number {i}."));
    }
    lines.push("This concludes our argument here.".to_string());
    let paper_text = lines.join("\n") + "\n";
    let (_tmp, env, db, pid) = setup_session(&paper_text);

    // Relocated pointer via `note_evidence`: quote absent from 1..=52, relocated +12 lines to 64,
    // paired with bare section heading `"Appendix A"` at line 1.
    let ev_out = note_evidence(
        &env,
        &db,
        SID,
        READER_CALLER,
        &json!({
            "items": [{
                "source_id": pid,
                "start_line": 1,
                "end_line": 52,
                "quote": "concludes our argument here",
                "kind": "passage",
                "role": "result",
                "label": "Appendix A",
                "gist": "Conclusion"
            }]
        }),
    )
    .unwrap();

    // Bare `"Section 1. Introduction"` heading on line 1 in a second source must also be rejected
    // as `label_is_section_name` rather than extending a 64-line range.
    let mut sec_lines = vec!["Section 1. Introduction".to_string()];
    for i in 2..=63 {
        sec_lines.push(format!("Unquoted filler statement {i}."));
    }
    sec_lines.push("Final concluding bound holds here.".to_string());
    let sec_text = sec_lines.join("\n") + "\n";
    let (_tmp2, env2, db2, pid2) = setup_session(&sec_text);
    let ledger2 = db2.lock().unwrap();
    let map_res = record(
        &env2,
        &ledger2,
        SID,
        &json!({
            "source_id": pid2,
            "title": "Audit",
            "coverage": "paper",
            "limitations": [],
            "nodes": [{
                "id": "c1",
                "sentence": "Claim.",
                "kind": "result",
                "main": true,
                "anchors": [{
                    "source_id": pid2,
                    "start_line": 1,
                    "end_line": 64,
                    "quote": "Final concluding bound holds here.",
                    "kind": "passage",
                    "label": "Section 1. Introduction"
                }]
            }],
            "edges": []
        }),
    );

    assert_eq!(
        ev_out["noted"], 0,
        "relocated 16-char quote at line 64 with bare 'Appendix A' at line 1 grounded all 64 lines: {ev_out}"
    );
    assert!(
        map_res.is_err(),
        "bare 'Section 1. Introduction' bypassed is_bare_section_name and grounded 64 lines: {map_res:?}"
    );
}

/// Finding F18b (minor, sub-issue of `excerpt_window` in `investigation_anchor.rs:432`):
/// `verify_pointer` matches `quote` uniquely inside `[s, e]` (`lines 2..=4`, matching line 4),
/// then `label_extended` prepends line 1 (`final_start = 1`) which happens to repeat the phrase,
/// and `folded_full.find(&folded_quote)` re-searches from byte 0 of `1..=4`, centering the
/// 3,200-byte window on line 1 and slicing off line 4 (the actual cited conclusion line).
#[test]
fn audit_honesty_excerpt_window_must_center_on_cited_match_not_prepended_label_line() {
    let l1 = "Lemma 1 (spectral gap remains strictly positive).";
    let l2 = format!("Assume uniform ellipticity across the lattice: {}.", "x".repeat(1700));
    let l3 = format!("Assume finite-range coupling bounds hold everywhere: {}.", "y".repeat(1700));
    let l4 = "Therefore the spectral gap remains strictly positive.";
    let paper_text = format!("{l1}\n{l2}\n{l3}\n{l4}\n");
    let (_tmp, env, db, pid) = setup_session(&paper_text);
    let ledger = db.lock().unwrap();

    let res = record(
        &env,
        &ledger,
        SID,
        &json!({
            "source_id": pid,
            "title": "Audit",
            "coverage": "paper",
            "limitations": [],
            "nodes": [{
                "id": "n1",
                "sentence": "Conclusion holds.",
                "kind": "result",
                "main": true,
                "anchors": [{
                    "source_id": pid,
                    "start_line": 2,
                    "end_line": 4,
                    "quote": "spectral gap remains strictly positive",
                    "kind": "passage",
                    "label": "Lemma 1"
                }]
            }],
            "edges": []
        }),
    )
    .unwrap();
    assert_eq!(res["anchors_verified"], true);

    let events = ledger.read_session(SID).unwrap();
    let map_ev = events.iter().rfind(|e| e.kind == "investigation.map").unwrap();
    let excerpt = map_ev.payload["nodes"][0]["anchors"][0]["excerpt"]
        .as_str()
        .unwrap();
    assert!(
        excerpt.contains("Therefore the spectral gap remains strictly positive."),
        "excerpt_window centered on prepended label line 1 and sliced off cited conclusion line 4; tail={:?}",
        &excerpt[excerpt.len().saturating_sub(80)..]
    );
}
