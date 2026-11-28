//! Live HTTP route, strict `AuthGuard::authorize_api`, security headers, determinism,
//! fail-closed tamper refusal, and `tex_audit` integration tests for Explore F15 write-ups
//! (`GET /api/sessions/{id}/writeup`).
//!
//! Zero third-party text, zero network calls, zero model calls, zero `#[ignore]` tests.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::auth::{ApiCredentials, AuthGuard};
use desktop_server::start_desktop_server;
use export::writeup::writeup_wording;
use ledger::Ledger;
use research_tools::idea_brief::EVENT_IDEA_BRIEF;
use research_tools::idea_map::EVENT_IDEA_MAP;
use research_tools::idea_verdict;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::paper_key::{PaperIds, PaperKey, PaperRef};
use research_tools::prior_work::{PriorWorkStatus, IDEA_VERDICT_EVENT};
use research_tools::prior_work_wording::wording as pw_wording;
use research_tools::quote_core::{verify_quote, EVIDENCE_QUOTE_EVENT};
use research_tools::search_coverage::{
    IDEA_CANDIDATES_EVENT, IDEA_NOTE_EVENT, IDEA_PAPER_EVENT, IDEA_SEARCH_FINISHED_EVENT,
    IDEA_SEARCH_PLANNED_EVENT, IDEA_STARTED_EVENT, IDEA_STATED_EVENT, IDEA_TRIAGE_EVENT,
};
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

const ROUTE_EXPLORE_TEX: &[u8] = br"\documentclass{article}
\begin{document}
\section{Spin Chain Growth}
Every finite chain with uniform exchange exhibits linear Krylov coefficient growth up to saturation.
\end{document}
";

fn seed_complete_explore_session(data_dir: &Path, sid: &str) -> PaperRef {
    let paper = PaperRef::from_ids(
        PaperIds { arxiv: Some("2408.09123".into()), ..Default::default() },
        Some("v1".into()),
    )
    .unwrap();
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: paper.key.clone(),
        ids: LibraryIds::from_paper_ids(&paper.ids),
        title: "Uniform Exchange Spin Chain Growth".into(),
        authors: vec!["Kato, Yuki".into()],
        year: Some(2024),
        abstract_text: "Synthetic abstract.".into(),
        bibtex: String::new(),
        flags: PaperFlags {
            owned: true,
            venue: Some("Synthetic Spin Letters".into()),
            ..Default::default()
        },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "vault-exp".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/synth-vault".into(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "vault-exp".into(),
        external_id: "kato2024spin".into(),
        key: Some(paper.key.clone()),
        citekey: Some("kato2024spin".into()),
        ids: LibraryIds::from_paper_ids(&paper.ids),
        version: Some("v1".into()),
        title: "Uniform Exchange Spin Chain Growth".into(),
        authors: vec!["Kato, Yuki".into()],
        year: Some(2024),
        item_kind: "journalArticle".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 1,
    })
    .unwrap();

    let ledger = Ledger::open(data_dir).unwrap();
    let idea_text = "Check linear Krylov coefficient growth in uniform exchange spin chains.";
    ledger
        .append_with_hash(
            sid,
            IDEA_STARTED_EVENT,
            1,
            &json!({
                "preset": "standard",
                "pipeline_version": 1,
                "offline": false,
                "library": { "searched": true, "papers": 15, "with_full_text": 12, "hits": 1 }
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            IDEA_STATED_EVENT,
            1,
            &json!({
                "rev": 1,
                "text": idea_text,
                "text_sha256": ledger::sha256_hex(idea_text.as_bytes()),
                "fields": ["quant-ph"],
                "seeds": [],
                "author": "user",
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Linear Krylov coefficient growth in uniform spin chains.",
                "assumptions": ["Uniform nearest-neighbor exchange"],
                "subclaims": [
                    { "id": "sc-1", "text": "Krylov coefficients grow linearly up to saturation.", "idea_quote": "linear Krylov coefficient growth" },
                    { "id": "sc-2", "text": "Subleading oscillation vanishes in the periodic limit.", "idea_quote": "uniform exchange spin chains" }
                ],
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "question": "Exact?", "queries": [{ "provider": "arxiv", "q": "Krylov spin chain" }] },
                    { "id": "p-2", "kind": "other_names", "question": "Aliases?", "queries": [{ "provider": "inspire", "q": "Lanczos spin growth" }] }
                ],
                "terms": [],
                "questions": [],
                "exclusions": [],
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            IDEA_SEARCH_PLANNED_EVENT,
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "Krylov spin chain", "origin": "model" },
                    { "qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "q": "Lanczos spin growth", "origin": "model" }
                ]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            IDEA_SEARCH_FINISHED_EVENT,
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 1 },
                    { "qid": "q-2", "provider": "inspire", "status": "ok", "count": 1 }
                ]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            IDEA_CANDIDATES_EVENT,
            1,
            &json!({
                "round": 1,
                "candidates": [{
                    "key": paper.key,
                    "title": "Uniform Exchange Spin Chain Growth",
                    "authors": ["Kato, Yuki"],
                    "year": 2024,
                    "ids": { "arxiv": "2408.09123" }
                }]
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            IDEA_TRIAGE_EVENT,
            1,
            &json!({
                "items": [{ "key": paper.key, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Direct." }],
                "reading_list": [paper.key]
            }),
        )
        .unwrap();

    let sandbox = research_tools::sandbox::WorkspaceSandbox::new(data_dir, None);
    let mtx = Mutex::new(Ledger::open(data_dir).unwrap());
    let ing = ingest_bytes(
        &sandbox,
        &mtx,
        &IngestRequest {
            session_id: sid,
            file_name: "kato.tex",
            origin: &format!("library:{}", paper.key),
            raw_bytes: ROUTE_EXPLORE_TEX,
            source_id_hint: Some("kato.tex"),
            section_filter: None,
            title: Some("Uniform Exchange Spin Chain Growth"),
        },
    )
    .unwrap();
    drop(mtx);
    ledger
        .append_with_hash(
            sid,
            IDEA_PAPER_EVENT,
            1,
            &json!({
                "key": paper.key,
                "source_id": ing.source_id,
                "status": "read_full",
                "text_basis": "tex",
                "lines_sent": 5,
                "total_lines": 5,
                "origin": "library"
            }),
        )
        .unwrap();
    let quote = "Every finite chain with uniform exchange exhibits linear Krylov coefficient growth up to saturation.";
    let evs = ledger.read_session(sid).unwrap();
    let vq = verify_quote(&sandbox, sid, &evs, &ing.source_id, &ing.files[0], 4, 4, quote).unwrap();
    let xe_seq = ledger
        .append_with_hash(
            sid,
            EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "source_id": vq.quote_ref.source_id,
                "path": vq.quote_ref.path,
                "start_line": vq.quote_ref.start_line,
                "end_line": vq.quote_ref.end_line,
                "file_sha256": vq.quote_ref.file_sha256,
                "quote_sha256": vq.quote_ref.quote_sha256,
                "quote": quote,
                "excerpt": vq.excerpt,
                "source_seq": vq.source_seq,
                "caller": format!("reader:{sid}:{}", paper.key),
                "paper_key": paper.key,
                "role": "states_result",
                "subclaim_ids": ["sc-1"],
                "gist": "Linear Krylov growth in uniform spin chains."
            }),
        )
        .unwrap()
        .seq;

    let map_seq = ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_MAP,
            1,
            &json!({
                "idea_rev": 1,
                "nodes": [],
                "edges": [],
                "overlaps": [{
                    "subclaim_id": "sc-1",
                    "paper_key": paper.key,
                    "relation": "same",
                    "evidence": [format!("xe-{xe_seq}")],
                    "difference": "Considers open boundaries without periodic twist."
                }],
                "dropped": []
            }),
        )
        .unwrap()
        .seq;
    let evs_map = ledger.read_session(sid).unwrap();
    let mut verdict = idea_verdict::compute(&evs_map, None);
    verdict.map_seq = Some(map_seq);
    ledger
        .append_with_hash(sid, IDEA_VERDICT_EVENT, 1, &serde_json::to_value(&verdict).unwrap())
        .unwrap();
    ledger
        .append_with_hash(sid, "idea.stage", 1, &json!({ "stage": "map", "state": "finished" }))
        .unwrap();
    paper
}

fn find_hdr(hdrs: &[(String, String)], name: &str) -> Option<String> {
    hdrs.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

#[test]
fn explore_writeup_http_routes_determinism_headers_tex_audit_and_tamper_refusal() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    let sid = "s-exp-http-ok";
    seed_complete_explore_session(&data_dir, sid);

    let cfg = test_cfg(&data_dir, Some(ws_dir.clone()), None, None);
    let handle = start_desktop_server(cfg).unwrap();
    let port = handle.info.port;
    let tok = handle.info.token.clone();
    let bearer = format!("Bearer {tok}");

    // 1. Live GET /api/sessions/{id}/writeup?format=tex (byte-deterministic + security headers).
    let (st1, hdrs1, tex1) = raw_http(
        port, "GET", &format!("/api/sessions/{sid}/writeup?format=tex"), &[("X-ReSearch-Token", &tok)], b"",
    );
    let (st2, _, tex2) = raw_http(
        port, "GET", &format!("/api/sessions/{sid}/writeup?format=tex"), &[("Authorization", &bearer)], b"",
    );
    assert_eq!(st1, 200, "{}", String::from_utf8_lossy(&tex1));
    assert_eq!(st2, 200);
    assert_eq!(tex1, tex2, "live Explore TeX write-up must be byte-deterministic");
    assert_eq!(find_hdr(&hdrs1, "content-type").as_deref(), Some("application/x-tex; charset=utf-8"));
    assert_eq!(
        find_hdr(&hdrs1, "content-disposition").as_deref(),
        Some(format!("attachment; filename=\"{sid}-writeup.tex\"").as_str())
    );
    assert_eq!(find_hdr(&hdrs1, "cache-control").as_deref(), Some("no-store"));
    assert_eq!(find_hdr(&hdrs1, "content-security-policy").as_deref(), Some("sandbox"));
    assert_eq!(find_hdr(&hdrs1, "x-content-type-options").as_deref(), Some("nosniff"));

    let tex_str = String::from_utf8(tex1.clone()).unwrap();
    assert!(tex_str.contains(r"\cite{kato2024spin}"));
    assert!(tex_str.contains(pw_wording::CHIP_JUDGED_SAME_UNCONFIRMED));
    assert!(tex_str.contains(pw_wording::CHIP_NONE_IN_SCOPE));
    assert!(tex_str.contains(writeup_wording::COVERAGE_HEADING));
    assert!(tex_str.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(!tex_str.to_ascii_lowercase().contains("verified"));

    // 2. Live format=bib + tex_audit on HTTP-served .tex and .bib.
    let (bib_st, bib_hdrs, bib_body) = raw_http(
        port, "GET", &format!("/api/sessions/{sid}/writeup?format=bib"), &[("X-ReSearch-Token", &tok)], b"",
    );
    assert_eq!(bib_st, 200);
    assert_eq!(find_hdr(&bib_hdrs, "content-type").as_deref(), Some("application/x-bibtex; charset=utf-8"));
    let bib_str = String::from_utf8(bib_body).unwrap();
    assert!(bib_str.contains("@article{kato2024spin,"));

    fs::write(ws_dir.join(format!("{sid}-writeup.tex")), &tex1).unwrap();
    fs::write(ws_dir.join(format!("{sid}-writeup.bib")), bib_str.as_bytes()).unwrap();
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let audit = audit_tex_sources(&env, sid, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed on live HTTP output: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);

    // 3. `inline_marks=0` drops inline marks while keeping status, Where I looked, and check table.
    let (md_st, md_hdrs, md_body) = raw_http(
        port, "GET", &format!("/api/sessions/{sid}/writeup?format=md&inline_marks=0"), &[("Authorization", &bearer)], b"",
    );
    assert_eq!(md_st, 200);
    assert_eq!(find_hdr(&md_hdrs, "content-type").as_deref(), Some("text/markdown; charset=utf-8"));
    let md_str = String::from_utf8(md_body).unwrap();
    assert!(!md_str.contains("*(model)*"));
    assert!(md_str.contains("**Status:**"));
    assert!(md_str.contains(writeup_wording::COVERAGE_HEADING));
    assert!(md_str.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(!md_str.to_ascii_lowercase().contains("verified"));

    // 4. Post-verdict `confirm_same` note upgrades chip live via HTTP.
    ledger
        .append_with_hash(
            sid,
            IDEA_NOTE_EVENT,
            1,
            &json!({ "target_kind": "verdict", "target_id": "sc-1", "action": "confirm_same", "comment": "" }),
        )
        .unwrap();
    let (conf_st, _, conf_body) = raw_http(
        port, "GET", &format!("/api/sessions/{sid}/writeup?format=md"), &[("X-ReSearch-Token", &tok)], b"",
    );
    assert_eq!(conf_st, 200);
    let conf_md = String::from_utf8(conf_body).unwrap();
    assert!(conf_md.contains(pw_wording::CHIP_JUDGED_SAME_CONFIRMED), "{conf_md}");

    // 5. Rule V7 tampered verdict returns 400 Bad Request.
    let sid_bad = "s-exp-http-v7";
    let mut forged = idea_verdict::compute(&ledger.read_session(sid).unwrap(), None);
    forged.verdicts[0].status = PriorWorkStatus::Incomplete;
    ledger.append_with_hash(sid_bad, IDEA_STARTED_EVENT, 1, &json!({ "preset": "standard" })).unwrap();
    ledger.append_with_hash(sid_bad, IDEA_STATED_EVENT, 1, &json!({ "rev": 1, "text": "Bad verdict." })).unwrap();
    ledger
        .append_with_hash(
            sid_bad,
            EVENT_IDEA_BRIEF,
            1,
            &json!({ "idea_rev": 1, "restatement": "Bad.", "subclaims": [{ "id": "sc-1", "text": "C", "idea_quote": "Bad" }], "perspectives": [] }),
        )
        .unwrap();
    ledger
        .append_with_hash(sid_bad, IDEA_VERDICT_EVENT, 1, &serde_json::to_value(&forged).unwrap())
        .unwrap();
    let (v7_st, _, v7_raw) = raw_http(
        port, "GET", &format!("/api/sessions/{sid_bad}/writeup?format=tex"), &[("X-ReSearch-Token", &tok)], b"",
    );
    assert_eq!(v7_st, 400, "{}", String::from_utf8_lossy(&v7_raw));
    let v7_val: Value = serde_json::from_slice(&v7_raw).unwrap();
    assert!(v7_val["error"].as_str().unwrap().contains("Rule V7"));

    // 6. Strict `AuthGuard::authorize_api` enforcement (unit + live HTTP).
    let auth = AuthGuard::new(port).unwrap();
    let path = format!("/api/sessions/{sid}/writeup");
    let cookie = launch_cookie(port, &tok);
    assert_eq!(
        auth.authorize_api(&ApiCredentials {
            method: "GET", path: &path, token_hdr: None, auth_hdr: None,
            cookie: Some(&cookie), origin: Some(&format!("http://127.0.0.1:{port}")),
            sec_fetch_site: Some("same-origin"),
        })
        .unwrap_err()
        .0,
        401
    );
    assert!(auth
        .authorize_api(&ApiCredentials {
            method: "GET", path: &path, token_hdr: Some(auth.token()), auth_hdr: None,
            cookie: None, origin: None, sec_fetch_site: None,
        })
        .is_ok());
    assert_eq!(raw_http(port, "GET", &format!("{path}?format=tex"), &[], b"").0, 401);
    assert_eq!(
        raw_http(
            port, "GET", &format!("{path}?format=tex"),
            &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")], b""
        )
        .0,
        401
    );

    let _ = PaperKey::from_arxiv("2408.09123").unwrap();
    handle.shutdown();
}
