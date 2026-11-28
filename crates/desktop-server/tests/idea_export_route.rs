//! Integration tests for `GET /api/sessions/{id}/idea/export?format=json|bib` (package X17).
//!
//! Verifies strict header-token authentication (`401` without token or cookie-only; `403` on
//! foreign `Origin`), JSON & BibTeX export headers/content, Rule V7 fail-closed mismatch (`409`),
//! user `source_item.citekey` preference + Rule S5 survival after `library.db` deletion,
//! Rule S8 wording compliance, `404` on unknown session (strictly read-only), and `400` on
//! invalid format or non-Explore session.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
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
use research_tools::quote_core::{verify_quote, EVIDENCE_QUOTE_EVENT, MAX_DISPLAY_QUOTE_CHARS};
use research_tools::sandbox::WorkspaceSandbox;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

const SYNTH_KEY: &str = "arxiv:2410.20002";
const SYNTH_CITEKEY: &str = "synthRoute2024gauge";

fn header_val<'a>(hdrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hdrs.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn seed_explore_session(data_dir: &Path, sid: &str, forge_verdict: bool) {
    let key = PaperKey::parse(SYNTH_KEY).unwrap();
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds {
            arxiv: Some("2410.20002".into()),
            doi: Some("10.9999/synth.route.2024".into()),
            ..Default::default()
        },
        title: "Synthetic Gauge Chain Benchmark".into(),
        authors: vec!["Clara Route".into()],
        year: Some(2024),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, ..Default::default() },
        first_seen_ms: 100,
        last_seen_ms: 200,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "vault-route".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/synth-vault".into(),
        settings: json!({}),
        added_ms: 100,
        last_sync_ms: Some(200),
        state: "ok".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "vault-route".into(),
        external_id: SYNTH_CITEKEY.into(),
        key: Some(key),
        citekey: Some(SYNTH_CITEKEY.into()),
        ids: LibraryIds { arxiv: Some("2410.20002".into()), ..Default::default() },
        version: Some("v1".into()),
        title: "Synthetic Gauge Chain Benchmark".into(),
        authors: vec!["Clara Route".into()],
        year: Some(2024),
        item_kind: "article".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 200,
    })
    .unwrap();

    let ledger = Ledger::open(data_dir).unwrap();
    ledger
        .append_with_hash(
            sid,
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
        .append_with_hash(
            sid,
            "idea.stated",
            1,
            &json!({
                "rev": 1,
                "text": "Check gauge-invariant synthetic operator growth.",
                "text_sha256": ledger::sha256_hex(b"Check gauge-invariant synthetic operator growth."),
                "fields": ["hep-th"],
                "seeds": [SYNTH_KEY],
                "author": "user"
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "idea.brief",
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Synthetic gauge-invariant operator growth check.",
                "subclaims": [{ "id": "sc-1", "text": "Synthetic operator norm saturates", "idea_quote": "operator growth" }],
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "queries": [{ "provider": "arxiv", "q": "gauge operator growth" }] },
                    { "id": "p-2", "kind": "other_names", "queries": [{ "provider": "inspire", "q": "krylov gauge chain" }] }
                ]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "idea.search_planned",
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "gauge operator growth", "origin": "user" },
                    { "qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "q": "krylov gauge chain", "origin": "model" }
                ]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "idea.search_finished",
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "provider": "arxiv", "status": "ok", "hits": 1, "found_papers": [{ "key": SYNTH_KEY }] },
                    { "qid": "q-2", "provider": "inspire", "status": "ok", "hits": 1, "found_papers": [{ "key": SYNTH_KEY }] }
                ]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "idea.candidates",
            1,
            &json!({
                "round": 1,
                "candidates": [{
                    "key": SYNTH_KEY,
                    "ids": { "arxiv": "2410.20002" },
                    "title": "Synthetic Gauge Chain Benchmark",
                    "authors": ["Clara Route"],
                    "year": 2024,
                    "found_by": ["q-1", "q-2"],
                    "user_seed": true
                }]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "idea.triage",
            1,
            &json!({
                "items": [{ "key": SYNTH_KEY, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Direct gauge chain benchmark." }],
                "reading_list": [SYNTH_KEY]
            }),
        )
        .unwrap();

    let quote_line = "The synthetic gauge operator norm reaches a plateau proportional to chain size.";
    let tex_doc = format!("\\documentclass{{article}}\n\\begin{{document}}\n{quote_line}\n\\end{{document}}\n");
    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let mtx = Mutex::new(Ledger::open(data_dir).unwrap());
    let ing = ingest_bytes(
        &sandbox,
        &mtx,
        &IngestRequest {
            session_id: sid,
            file_name: "gauge_chain.tex",
            origin: &format!("library:{SYNTH_KEY}"),
            raw_bytes: tex_doc.as_bytes(),
            source_id_hint: Some("gauge_chain.tex"),
            section_filter: None,
            title: Some("Synthetic Gauge Chain Benchmark"),
        },
    )
    .unwrap();
    drop(mtx);

    let evs_ing = ledger.read_session(sid).unwrap();
    let vq = verify_quote(&sandbox, sid, &evs_ing, &ing.source_id, &ing.files[0], 3, 3, quote_line).unwrap();
    ledger
        .append_with_hash(
            sid,
            "idea.paper",
            1,
            &json!({
                "key": SYNTH_KEY,
                "version": "v1",
                "source_id": vq.quote_ref.source_id,
                "source_seq": ing.event_seq,
                "origin": "user_library",
                "text_basis": "tex",
                "status": "read_passages",
                "lines_sent": 4,
                "total_lines": 4
            }),
        )
        .unwrap();
    let eq = ledger
        .append_with_hash(
            sid,
            EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "paper_key": SYNTH_KEY,
                "source_id": vq.quote_ref.source_id,
                "path": vq.quote_ref.path,
                "start_line": vq.quote_ref.start_line,
                "end_line": vq.quote_ref.end_line,
                "file_sha256": vq.quote_ref.file_sha256,
                "quote_sha256": vq.quote_ref.quote_sha256,
                "quote": quote_line,
                "excerpt": vq.excerpt,
                "source_seq": vq.source_seq,
                "role": "states_result",
                "gist": "Gauge operator norm plateaus.",
                "subclaim_ids": ["sc-1"]
            }),
        )
        .unwrap();
    let xe_id = format!("xe-{}", eq.seq);
    ledger
        .append_with_hash(
            sid,
            "idea.map",
            1,
            &json!({
                "nodes": [{
                    "id": "k-1",
                    "kind": "known_result",
                    "label": "Gauge plateau",
                    "statement": "Plateau scales with chain size.",
                    "evidence": [xe_id],
                    "papers": [SYNTH_KEY],
                    "relation_to_idea": "same"
                }],
                "edges": [],
                "overlaps": [{
                    "subclaim_id": "sc-1",
                    "paper_key": SYNTH_KEY,
                    "relation": "same",
                    "evidence": [xe_id],
                    "difference": "Same synthetic lattice."
                }],
                "dropped": []
            }),
        )
        .unwrap();

    let evs = ledger.read_session(sid).unwrap();
    let mut verdict = idea_verdict::compute(&evs, None);
    if forge_verdict {
        verdict.verdicts[0].status = PriorWorkStatus::NoneInScope;
        verdict.verdicts[0].chip = "not found in the searched scope".into();
    }
    ledger
        .append_with_hash(sid, IDEA_VERDICT_EVENT, 1, &serde_json::to_value(&verdict).unwrap())
        .unwrap();
}

#[test]
fn idea_export_route_enforces_auth_formats_v7_s5_and_s8() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    seed_explore_session(&data_dir, "exp-rt-ok", false);
    seed_explore_session(&data_dir, "exp-rt-bad-v7", true);

    let ledger = Ledger::open(&data_dir).unwrap();
    ledger
        .append_with_hash("res-only", "session.start", 1, &json!({ "model": "test" }))
        .unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // 1. Strict header-token auth & loopback Host enforcement (401/403).
    let json_url = "/api/sessions/exp-rt-ok/idea/export?format=json";
    assert_eq!(raw_http(port, "GET", json_url, &[], b"").0, 401);
    assert_eq!(raw_http(port, "GET", json_url, &[("X-ReSearch-Token", "wrong")], b"").0, 401);
    assert_eq!(raw_http(port, "GET", json_url, &[("Cookie", cookie.as_str())], b"").0, 401);
    assert_eq!(
        raw_http(
            port,
            "GET",
            json_url,
            &[("X-ReSearch-Token", tok.as_str()), ("Host", "evil.example")],
            b""
        )
        .0,
        403
    );

    // 2. Bearer auth + JSON export headers & payload.
    let bearer = format!("Bearer {tok}");
    let (st, hdrs, body) = raw_http(port, "GET", json_url, &[("Authorization", bearer.as_str())], b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
    assert_eq!(header_val(&hdrs, "content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(header_val(&hdrs, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header_val(&hdrs, "content-security-policy"), Some("sandbox"));
    assert_eq!(header_val(&hdrs, "cache-control"), Some("no-store"));
    assert_eq!(
        header_val(&hdrs, "content-disposition"),
        Some("attachment; filename=\"exp-rt-ok-idea.json\"")
    );
    let val: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["session_id"], "exp-rt-ok");
    assert!(val["matched_quotes"][0]["quote"].as_str().unwrap().chars().count() <= MAX_DISPLAY_QUOTE_CHARS);
    assert!(contains_forbidden_prior_work_word(val["coverage_scope_summary"].as_str().unwrap()).is_none());

    // 3. BibTeX export prefers `source_item.citekey` and cites searched scope.
    let (st, hdrs, body) = raw_http(
        port,
        "GET",
        "/api/sessions/exp-rt-ok/idea/export?format=bib",
        &auth,
        b"",
    );
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
    let bib = String::from_utf8(body).unwrap();
    assert!(bib.contains(&format!("{{{SYNTH_CITEKEY},")));
    assert!(bib.contains("% Searched scope:"));
    assert_eq!(header_val(&hdrs, "content-type"), Some("application/x-bibtex; charset=utf-8"));
    assert_eq!(
        header_val(&hdrs, "content-disposition"),
        Some("attachment; filename=\"exp-rt-ok-idea.bib\"")
    );

    // 4. Rule S5: deleting library.db does not break the HTTP export endpoint.
    let lib_db = data_dir.join(LIBRARY_DIR_NAME).join(LIBRARY_DB_NAME);
    fs::remove_file(&lib_db).unwrap();
    let (st, _, body) = raw_http(
        port,
        "GET",
        "/api/sessions/exp-rt-ok/idea/export?format=bib",
        &auth,
        b"",
    );
    assert_eq!(st, 200);
    assert!(String::from_utf8(body).unwrap().contains("% Searched scope:"));
    assert!(!lib_db.exists());

    // 5. Rule V7 stored mismatch -> 409 Conflict.
    let (st, _, raw) = raw_http(
        port,
        "GET",
        "/api/sessions/exp-rt-bad-v7/idea/export?format=json",
        &auth,
        b"",
    );
    assert_eq!(st, 409, "{}", String::from_utf8_lossy(&raw));
    let err: Value = serde_json::from_slice(&raw).unwrap();
    assert!(err["error"].as_str().unwrap().contains("Rule V7"));

    // 6. Unknown session -> 404; invalid format or non-Explore mode -> 400.
    assert_eq!(
        raw_http(port, "GET", "/api/sessions/no-such-exp/idea/export?format=json", &auth, b"").0,
        404
    );
    assert_eq!(
        raw_http(port, "GET", "/api/sessions/exp-rt-ok/idea/export?format=xml", &auth, b"").0,
        400
    );
    assert_eq!(
        raw_http(port, "GET", "/api/sessions/res-only/idea/export?format=json", &auth, b"").0,
        400
    );

    srv.shutdown();
}

#[test]
fn idea_export_route_404_is_strictly_readonly_before_ledger_exists() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("fresh-idea-export");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];

    let (st, _, _) = raw_http(port, "GET", "/api/sessions/missing-1/idea/export?format=json", &auth, b"");
    assert_eq!(st, 404);
    assert!(!data_dir.join("ledger.db").exists());
    srv.shutdown();
}
