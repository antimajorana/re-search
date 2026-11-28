//! Integration tests for deterministic Explore JSON & BibTeX export (package X17).
//!
//! Covers:
//! - Deterministic JSON and BibTeX exports over a hash-chain-verified Explore session
//! - F12 `Coverage` & Rule V7 `idea_verdict::compute` recomputation + fail-closed mismatch rejection
//! - Post-verdict Rule V6 user confirmation (`confirm_same`) reflected cleanly on export
//! - F4 quote re-matching and `<= 300`-char clipping of exported full-text & abstract quotes
//! - User `source_item.citekey` preference from F10/F13 `Library` + Rule S5 survival after `library.db` deletion
//! - Rule S8 zero `"verified"` / `"new"` / `"novel"` across all exported wording fields
//! - Fail-closed rejection on tampered source bytes, tampered ledger, and non-Explore sessions.

use export::idea::{
    build_idea_export, export_idea_bibtex, export_idea_bytes, export_idea_json, IdeaExportFormat,
};
use ledger::Ledger;
use research_tools::idea_verdict;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord, LIBRARY_DB_NAME, LIBRARY_DIR_NAME,
};
use research_tools::paper_key::PaperKey;
use research_tools::prior_work::{PriorWorkStatus, IDEA_VERDICT_EVENT};
use research_tools::prior_work_wording::contains_forbidden_prior_work_word;
use research_tools::quote_core::{
    EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT, MAX_DISPLAY_QUOTE_CHARS,
};
use research_tools::WorkspaceSandbox;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::tempdir;

const SYNTH_PAPER_KEY: &str = "arxiv:2409.10001";
const SYNTH_USER_CITEKEY: &str = "synthAuthor2024lattice";

fn make_long_synthetic_line() -> (String, String) {
    // Purely synthetic self-written >360-char source line containing a <=300-char matched quote.
    let matched = "In our synthetic lattice benchmark model, the discrete Lanczos recurrence coefficient sequence grows linearly across the ballistic sector while saturating rapidly inside the localized regime whenever dissipation exceeds the critical threshold.".to_string();
    let full_line = format!(
        "{matched} Furthermore, the asymptotic plateau height scales monotonically with finite chain size across every synthetic test geometry above sixty-four sites in dimensionless units."
    );
    (full_line, matched)
}

fn build_complete_explore_session(data_dir: &Path, session_id: &str) -> (String, String) {
    let ledger = Ledger::open(data_dir).unwrap();
    let (long_line, matched_quote) = make_long_synthetic_line();
    assert!(long_line.chars().count() > MAX_DISPLAY_QUOTE_CHARS);
    assert!(matched_quote.chars().count() <= MAX_DISPLAY_QUOTE_CHARS);

    ledger
        .append(
            session_id,
            "idea.started",
            1,
            &json!({
                "preset": "standard",
                "pipeline_version": 1,
                "budget": { "max_queries": 18, "max_screened": 40, "max_papers": 6 }
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.stated",
            1,
            &json!({
                "rev": 1,
                "text": "Test whether synthetic Lanczos coefficients saturate under strong dissipation.",
                "text_sha256": ledger::sha256_hex(b"Test whether synthetic Lanczos coefficients saturate under strong dissipation."),
                "fields": ["cond-mat.stat-mech"],
                "seeds": [SYNTH_PAPER_KEY],
                "author": "user"
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.brief",
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Examine synthetic Lanczos growth and saturation under dissipation.",
                "assumptions": ["finite 1D lattice"],
                "subclaims": [
                    { "id": "sc-1", "text": "Lanczos coefficients saturate under strong dissipation", "idea_quote": "Lanczos coefficients saturate" },
                    { "id": "sc-2", "text": "Threshold scaling persists at large system sizes", "idea_quote": "strong dissipation" }
                ],
                "terms": [{ "term": "Lanczos coefficients", "aliases": ["spread growth"] }],
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "question": "Exact match?", "queries": [{ "provider": "arxiv", "q": "lanczos saturation" }] },
                    { "id": "p-2", "kind": "other_names", "question": "Other names?", "queries": [{ "provider": "inspire", "q": "spread growth" }] }
                ],
                "questions": [],
                "exclusions": []
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.search_planned",
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "lanczos saturation", "origin": "user" },
                    { "qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "q": "spread growth", "origin": "model" }
                ]
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.search_finished",
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "provider": "arxiv", "status": "ok", "hits": 1, "found_papers": [{ "key": SYNTH_PAPER_KEY }] },
                    { "qid": "q-2", "provider": "inspire", "status": "ok", "hits": 1, "found_papers": [{ "key": SYNTH_PAPER_KEY }] }
                ]
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.candidates",
            1,
            &json!({
                "round": 1,
                "candidates": [{
                    "key": SYNTH_PAPER_KEY,
                    "ids": { "arxiv": "2409.10001", "doi": "10.9999/synth.2024.10001" },
                    "title": "Synthetic Lattice Benchmark of Dissipative Lanczos Growth",
                    "authors": ["Ada Synthetic", "B. Example"],
                    "year": 2024,
                    "found_by": ["q-1", "q-2"],
                    "user_seed": true
                }]
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.triage",
            1,
            &json!({
                "items": [{
                    "key": SYNTH_PAPER_KEY,
                    "relevance": "core",
                    "subclaim_ids": ["sc-1"],
                    "why": "Directly analyzes synthetic dissipative saturation."
                }],
                "reading_list": [SYNTH_PAPER_KEY]
            }),
        )
        .unwrap();

    // Ingest synthetic paper text strictly through `ingest_bytes` so F4 quote re-matching works.
    let paper_body = format!(
        "% Synthetic fixture document\n\\section{{Model}}\n{long_line}\n\\section{{Conclusion}}\nEnd of synthetic note.\n"
    );
    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let ledger_mutex = Mutex::new(Ledger::open(data_dir).unwrap());
    let ingested = ingest_bytes(
        &sandbox,
        &ledger_mutex,
        &IngestRequest {
            session_id,
            file_name: "synthetic_paper.tex",
            origin: &format!("library:{SYNTH_PAPER_KEY}"),
            raw_bytes: paper_body.as_bytes(),
            source_id_hint: Some("src-synth-1"),
            section_filter: None,
            title: Some("Synthetic Lattice Benchmark of Dissipative Lanczos Growth"),
        },
    )
    .unwrap();
    drop(ledger_mutex);

    let evs_ing = ledger.read_session(session_id).unwrap();
    let vq = research_tools::quote_core::verify_quote(
        &sandbox,
        session_id,
        &evs_ing,
        &ingested.source_id,
        &ingested.files[0],
        3,
        3,
        &matched_quote,
    )
    .unwrap();
    let rel_path = vq.quote_ref.path.clone();
    let src_id = vq.quote_ref.source_id.clone();

    ledger
        .append(
            session_id,
            "idea.paper",
            1,
            &json!({
                "key": SYNTH_PAPER_KEY,
                "version": "v1",
                "source_id": src_id,
                "source_seq": ingested.event_seq,
                "origin": "user_library",
                "text_basis": "tex",
                "status": "read_passages",
                "lines_sent": 5,
                "total_lines": 5
            }),
        )
        .unwrap();

    let ev_quote = ledger
        .append(
            session_id,
            EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "paper_key": SYNTH_PAPER_KEY,
                "source_id": vq.quote_ref.source_id,
                "path": vq.quote_ref.path,
                "start_line": vq.quote_ref.start_line,
                "end_line": vq.quote_ref.end_line,
                "file_sha256": vq.quote_ref.file_sha256,
                "quote_sha256": vq.quote_ref.quote_sha256,
                "quote": matched_quote,
                "excerpt": vq.excerpt,
                "source_seq": vq.source_seq,
                "role": "states_result",
                "gist": "Dissipation saturates the synthetic Lanczos coefficients.",
                "subclaim_ids": ["sc-1"]
            }),
        )
        .unwrap();
    let xe_id = format!("xe-{ev_quote}");

    let abs_text = "A".repeat(360);
    let abs_blob = ledger.put_blob(abs_text.as_bytes()).unwrap();
    ledger
        .append(
            session_id,
            EVIDENCE_ABSTRACT_EVENT,
            1,
            &json!({
                "paper_key": SYNTH_PAPER_KEY,
                "quote": abs_text,
                "verbatim": true,
                "provider": "arxiv",
                "raw_blob": abs_blob
            }),
        )
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.map",
            1,
            &json!({
                "nodes": [{
                    "id": "k-1",
                    "kind": "known_result",
                    "label": "Dissipative Lanczos saturation",
                    "statement": "Saturation occurs above the dissipation threshold.",
                    "evidence": [xe_id],
                    "papers": [SYNTH_PAPER_KEY],
                    "relation_to_idea": "partial"
                }],
                "edges": [],
                "overlaps": [{
                    "subclaim_id": "sc-1",
                    "paper_key": SYNTH_PAPER_KEY,
                    "relation": "partial",
                    "evidence": [xe_id],
                    "difference": "Uses a synthetic lattice with fixed dissipation."
                }],
                "dropped": []
            }),
        )
        .unwrap();

    let events_now = ledger.read_session(session_id).unwrap();
    let computed_verdict = idea_verdict::compute(&events_now, None);
    let verdict_val = serde_json::to_value(&computed_verdict).unwrap();
    ledger
        .append(session_id, IDEA_VERDICT_EVENT, 1, &verdict_val)
        .unwrap();

    ledger
        .append(
            session_id,
            "idea.directions",
            1,
            &json!({
                "directions": [{
                    "id": "r-1",
                    "title": "Measure finite-size scaling of saturation",
                    "concreteness": "concrete"
                }]
            }),
        )
        .unwrap();

    (src_id, rel_path)
}

fn seed_library_with_user_citekey(data_dir: &Path) {
    let lib = Library::open(data_dir).unwrap();
    let key = PaperKey::parse(SYNTH_PAPER_KEY).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds {
            arxiv: Some("2409.10001".into()),
            doi: Some("10.9999/synth.2024.10001".into()),
            ..Default::default()
        },
        title: "Synthetic Lattice Benchmark of Dissipative Lanczos Growth".into(),
        authors: vec!["Ada Synthetic".into(), "B. Example".into()],
        year: Some(2024),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags {
            owned: true,
            ..Default::default()
        },
        first_seen_ms: 1_000,
        last_seen_ms: 2_000,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "src-kandinsky".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/synthetic-vault".into(),
        settings: json!({}),
        added_ms: 1_000,
        last_sync_ms: Some(2_000),
        state: "ok".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "src-kandinsky".into(),
        external_id: SYNTH_USER_CITEKEY.into(),
        key: Some(key),
        citekey: Some(SYNTH_USER_CITEKEY.into()),
        ids: LibraryIds {
            arxiv: Some("2409.10001".into()),
            doi: Some("10.9999/synth.2024.10001".into()),
            ..Default::default()
        },
        version: Some("v1".into()),
        title: "Synthetic Lattice Benchmark of Dissipative Lanczos Growth".into(),
        authors: vec!["Ada Synthetic".into(), "B. Example".into()],
        year: Some(2024),
        item_kind: "article".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 2_000,
    })
    .unwrap();
}

#[test]
fn export_idea_json_and_bibtex_deterministic_with_coverage_citekey_clipping_and_s8() {
    let dir = tempdir().unwrap();
    let sid = "explore-export-1";
    build_complete_explore_session(dir.path(), sid);
    seed_library_with_user_citekey(dir.path());

    let json_a = export_idea_json(dir.path(), sid).unwrap();
    let json_b = export_idea_json(dir.path(), sid).unwrap();
    assert_eq!(json_a, json_b, "JSON export must be byte-for-byte deterministic");

    let parsed: Value = serde_json::from_str(&json_a).unwrap();
    assert_eq!(parsed["session_id"], sid);
    assert_eq!(parsed["preset"], "standard");
    assert_eq!(parsed["statement"]["rev"], 1);
    assert!(parsed["sharpened_idea"].is_object());
    assert_eq!(parsed["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["papers_read"][0]["origin"], "user_library");
    assert_eq!(parsed["papers_read"][0]["text_basis"], "tex");
    assert_eq!(parsed["directions"].as_array().unwrap().len(), 1);
    assert!(parsed["concept_map"]["nodes"].as_array().unwrap().len() == 1);
    assert!(parsed["coverage"]["gate"]["passed"].as_bool().unwrap());

    // Quotes & excerpts clipped to <= 300 Unicode scalar values.
    let mq = &parsed["matched_quotes"][0];
    assert!(mq["quote"].as_str().unwrap().chars().count() <= MAX_DISPLAY_QUOTE_CHARS);
    assert_eq!(mq["excerpt"].as_str().unwrap().chars().count(), MAX_DISPLAY_QUOTE_CHARS);
    let aq = &parsed["abstract_quotes"][0];
    assert_eq!(aq["quote"].as_str().unwrap().chars().count(), MAX_DISPLAY_QUOTE_CHARS);

    // Verdicts recomputed: sc-1 is judged_partial (unconfirmed), sc-2 is none_in_scope.
    let model = build_idea_export(dir.path(), sid).unwrap();
    let v_sc1 = model.verdicts.iter().find(|v| v.subclaim_id == "sc-1").unwrap();
    let v_sc2 = model.verdicts.iter().find(|v| v.subclaim_id == "sc-2").unwrap();
    assert_eq!(v_sc1.status, PriorWorkStatus::JudgedPartial);
    assert!(!v_sc1.confirmed_by_user);
    assert_eq!(v_sc2.status, PriorWorkStatus::NoneInScope);

    // Rule S8: zero "verified", "new", or "novel" in exported JSON wording fields.
    assert!(contains_forbidden_prior_work_word(&model.coverage_scope_summary).is_none());
    for v in &model.verdicts {
        assert!(contains_forbidden_prior_work_word(&v.chip).is_none());
        assert!(contains_forbidden_prior_work_word(&v.text).is_none());
    }
    for key in parsed.as_object().unwrap().keys() {
        assert!(!key.to_ascii_lowercase().contains("verified"));
        assert!(!key.to_ascii_lowercase().contains("novel"));
    }

    // BibTeX prefers user's `source_item.citekey` and cites searched scope in header.
    let bib_a = export_idea_bibtex(dir.path(), sid).unwrap();
    let bib_b = export_idea_bibtex(dir.path(), sid).unwrap();
    assert_eq!(bib_a, bib_b, "BibTeX export must be byte-for-byte deterministic");
    assert!(bib_a.contains(&format!("{{{SYNTH_USER_CITEKEY},")));
    assert!(bib_a.contains("% Searched scope:"));
    for header_line in bib_a.lines().filter(|l| l.starts_with('%') && !l.starts_with("% Session:")) {
        assert!(
            contains_forbidden_prior_work_word(header_line).is_none(),
            "forbidden wording in BibTeX header: {header_line}"
        );
    }

    // Rule S5: deleting library.db does not break JSON or BibTeX export, nor recreate library.db.
    let lib_db_path = dir.path().join(LIBRARY_DIR_NAME).join(LIBRARY_DB_NAME);
    assert!(lib_db_path.is_file());
    fs::remove_file(&lib_db_path).unwrap();
    assert!(!lib_db_path.exists());

    let json_after_del = export_idea_json(dir.path(), sid).unwrap();
    assert_eq!(json_after_del, json_a);
    let bib_after_del = export_idea_bibtex(dir.path(), sid).unwrap();
    assert!(bib_after_del.contains("% Searched scope:"));
    assert!(bib_after_del.contains("@misc{synthetic2024_"));
    assert!(
        !lib_db_path.exists(),
        "Rule S5: read-only export must not recreate deleted library.db"
    );
}

#[test]
fn post_verdict_confirm_same_passes_rule_v7_and_updates_exported_verdict() {
    let dir = tempdir().unwrap();
    let sid = "explore-confirm-1";
    build_complete_explore_session(dir.path(), sid);

    let ledger = Ledger::open(dir.path()).unwrap();
    ledger
        .append(
            sid,
            "idea.note",
            1,
            &json!({
                "target_kind": "verdict",
                "target_id": "sc-1",
                "action": "confirm_same",
                "comment": "Matches the synthetic benchmark"
            }),
        )
        .unwrap();

    let model = build_idea_export(dir.path(), sid).unwrap();
    let v_sc1 = model.verdicts.iter().find(|v| v.subclaim_id == "sc-1").unwrap();
    assert_eq!(v_sc1.status, PriorWorkStatus::JudgedPartial);
    assert!(v_sc1.confirmed_by_user);
    assert_eq!(v_sc1.chip, "found (partly)");
}

#[test]
fn rule_v7_rejects_mismatched_stored_verdict_fail_closed() {
    let dir = tempdir().unwrap();
    let sid = "explore-v7-mismatch";
    build_complete_explore_session(dir.path(), sid);

    let ledger = Ledger::open(dir.path()).unwrap();
    let events = ledger.read_session(sid).unwrap();
    let mut forged = idea_verdict::compute(&events, None);
    forged.verdicts[0].status = PriorWorkStatus::NoneInScope;
    forged.verdicts[0].chip = "not found in the searched scope".into();
    ledger
        .append(sid, IDEA_VERDICT_EVENT, 1, &serde_json::to_value(&forged).unwrap())
        .unwrap();

    let err_json = export_idea_json(dir.path(), sid).unwrap_err();
    assert!(err_json.contains("Rule V7"), "expected Rule V7 error, got: {err_json}");
    let err_bib = export_idea_bibtex(dir.path(), sid).unwrap_err();
    assert!(err_bib.contains("Rule V7"), "expected Rule V7 error, got: {err_bib}");
}

#[test]
fn export_rejects_tampered_source_file_and_non_explore_sessions() {
    let dir = tempdir().unwrap();
    let sid = "explore-tamper-1";
    let (src_id, rel_path) = build_complete_explore_session(dir.path(), sid);

    // Tamper with the ingested paper file on disk: F4 re-matching must fail closed.
    let (_src_id, rel_path) = (src_id, rel_path);
    let src_file = dir
        .path()
        .join("sessions")
        .join(sid)
        .join("sources")
        .join(&rel_path);
    fs::write(&src_file, b"tampered file bytes\n").unwrap();

    let err = export_idea_json(dir.path(), sid).unwrap_err();
    assert!(
        err.contains("F4 re-matching"),
        "expected F4 re-matching refusal on tampered source, got: {err}"
    );

    // Non-Explore session rejected cleanly.
    let research_sid = "research-only-1";
    let ledger = Ledger::open(dir.path()).unwrap();
    ledger
        .append(research_sid, "session.start", 1, &json!({ "model": "test" }))
        .unwrap();
    let mode_err = export_idea_bytes(dir.path(), research_sid, IdeaExportFormat::Json).unwrap_err();
    assert!(mode_err.contains("Explore mode"), "got: {mode_err}");
}
