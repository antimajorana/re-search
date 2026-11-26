//! Integration tests for `/api/library/..` route handlers (`routes_library::handle_library`)
//! and `AuthGuard::authorize_api` coverage of library endpoints (package F10).
//!
//! All fixtures are synthetic and self-written; no network or model calls are made.

mod common;

use common::test_cfg;
use desktop_server::auth::{ApiCredentials, AuthGuard};
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::routes_library::handle_library;
use desktop_server::routes_session::handle_create_session;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use ledger::{FulltextCache, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SeenRecord, SeenRole,
};
use research_tools::paper_key::PaperKey;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const SYNTHETIC_TEX: &[u8] = br"\documentclass{article}
\title{Synthetic Route Fixture Paper}
\begin{document}
\section{Main}
A self-written synthetic lemma for desktop library route testing.
\end{document}
";

fn make_state(tmp: &TempDir) -> Arc<DesktopState> {
    let cfg = test_cfg(tmp.path(), None, None, None);
    let auth = AuthGuard::new(18_420).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    Arc::new(DesktopState::new(cfg, auth, sse, chatgpt))
}

fn qmap(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn seed_two_papers(state: &DesktopState) {
    let lib = Library::open(&state.cfg.data_dir).unwrap();
    let k_arxiv = PaperKey::parse("arxiv:2402.00123").unwrap();
    let k_doi = PaperKey::parse("doi:10.9999/route/2023.42").unwrap();

    lib.upsert_paper(&PaperRecord {
        key: k_arxiv.clone(),
        ids: LibraryIds {
            arxiv: Some("2402.00123".into()),
            doi: Some("10.9999/synth.2024.1".into()),
            ..Default::default()
        },
        title: "Synthetic Lattice Gauge Identities".into(),
        authors: vec!["Ada Example".into(), "Carl Synthetic".into()],
        year: Some(2024),
        abstract_text: "Discrete plaquette trace identities.".into(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1000,
        last_seen_ms: 5000,
    })
    .unwrap();
    lib.record_seen(&SeenRecord {
        key: k_arxiv,
        session_id: "s-inv".into(),
        role: SeenRole::Investigated,
        seen_ms: 5000,
    })
    .unwrap();

    lib.upsert_paper(&PaperRecord {
        key: k_doi.clone(),
        ids: LibraryIds {
            doi: Some("10.9999/route/2023.42".into()),
            ..Default::default()
        },
        title: "Toy Graph Dualities".into(),
        authors: vec!["Emmy Fixture".into()],
        year: Some(2023),
        abstract_text: "Finite graph partition dualities.".into(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1000,
        last_seen_ms: 3000,
    })
    .unwrap();
    lib.record_seen(&SeenRecord {
        key: k_doi,
        session_id: "s-solve".into(),
        role: SeenRole::SolveSource,
        seen_ms: 3000,
    })
    .unwrap();
}

#[test]
fn list_search_and_detail_routes_support_filters_aliases_and_slash_keys() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    seed_two_papers(&state);

    // Unfiltered list returns both papers ordered by last_seen_ms DESC.
    let (st, val) = handle_library(&state, "GET", &[], &qmap(&[]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["count"], 2);
    assert_eq!(val["papers"][0]["paper"]["key"], "arxiv:2402.00123");
    assert_eq!(val["papers"][1]["paper"]["key"], "doi:10.9999/route/2023.42");

    // Search by title, author, year, role, and alias key.
    let (st, val) = handle_library(
        &state,
        "GET",
        &[],
        &qmap(&[
            ("title", "lattice"),
            ("author", "synthetic"),
            ("year", "2024"),
            ("key", "doi:10.9999/synth.2024.1"),
            ("role", "investigated"),
        ]),
        &[],
    )
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["count"], 1);
    assert_eq!(val["papers"][0]["paper"]["key"], "arxiv:2402.00123");

    // Free-text FTS5 `q` search.
    let (st, val) = handle_library(&state, "GET", &[], &qmap(&[("q", "partition")]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["count"], 1);
    assert_eq!(val["papers"][0]["paper"]["key"], "doi:10.9999/route/2023.42");

    // Detail lookup for a key containing slashes (re-joined after http_srv percent-decoding).
    let (st, detail) =
        handle_library(&state, "GET", &["doi:10.9999", "route", "2023.42"], &qmap(&[]), &[])
            .unwrap();
    assert_eq!(st, 200);
    assert_eq!(detail["paper"]["key"], "doi:10.9999/route/2023.42");
    assert_eq!(detail["seen"][0]["role"], "solve_source");

    // Validation and 404 shapes match existing desktop routes.
    assert_eq!(
        handle_library(&state, "GET", &[], &qmap(&[("year", "not-a-year")]), &[])
            .unwrap_err()
            .0,
        400
    );
    assert_eq!(
        handle_library(&state, "GET", &[], &qmap(&[("key", "badkey")]), &[])
            .unwrap_err()
            .0,
        400
    );
    assert_eq!(
        handle_library(&state, "GET", &["arxiv:9901.00001"], &qmap(&[]), &[])
            .unwrap_err()
            .0,
        404
    );
    assert_eq!(
        handle_library(&state, "PUT", &["unknown"], &qmap(&[]), &[])
            .unwrap_err()
            .0,
        404
    );
}

#[test]
fn user_mark_write_delete_backup_and_restore_routes() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    seed_two_papers(&state);

    // Set mark on a slash-containing DOI key.
    let mark_body = serde_json::to_vec(&json!({
        "status": "key",
        "tags": ["dualities", "core"],
        "note": "Check theorem 2"
    }))
    .unwrap();
    let (st, val) = handle_library(
        &state,
        "POST",
        &["doi:10.9999", "route", "2023.42", "mark"],
        &qmap(&[]),
        &mark_body,
    )
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["ok"], true);
    assert_eq!(val["mark"]["status"], "key");
    assert_eq!(val["mark"]["tags"], json!(["dualities", "core"]));

    // Filter list by `status=key`.
    let (st, list) = handle_library(&state, "GET", &[], &qmap(&[("status", "key")]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(list["count"], 1);
    assert_eq!(list["papers"][0]["paper"]["key"], "doi:10.9999/route/2023.42");

    // Export backup via route.
    let (st, backup) =
        handle_library(&state, "GET", &["marks", "backup"], &qmap(&[]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(backup["marks"].as_array().unwrap().len(), 1);
    let backup_bytes = serde_json::to_vec(&backup).unwrap();

    // Delete mark via route.
    let (st, del) = handle_library(
        &state,
        "DELETE",
        &["doi:10.9999", "route", "2023.42", "mark"],
        &qmap(&[]),
        &[],
    )
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(del["deleted"], true);

    // Restore backup via route -> mark comes back.
    let (st, res) =
        handle_library(&state, "POST", &["marks", "restore"], &qmap(&[]), &backup_bytes).unwrap();
    assert_eq!(st, 200);
    assert_eq!(res["restored"], 1);

    // Invalid status returns 400.
    let bad_body = serde_json::to_vec(&json!({ "status": "bogus" })).unwrap();
    assert_eq!(
        handle_library(&state, "POST", &["arxiv:2402.00123", "mark"], &qmap(&[]), &bad_body)
            .unwrap_err()
            .0,
        400
    );
}

#[test]
fn attach_delete_and_rebuild_routes_work_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    // Create a real session via existing desktop session route.
    let create_body = serde_json::to_vec(&json!({ "title": "Library Attach Test" })).unwrap();
    let (st, created) = handle_create_session(&state, &create_body).unwrap();
    assert_eq!(st, 200);
    let session_id = created["id"].as_str().unwrap().to_string();

    // Seed a cached paper in `fulltext` + a ledger ingestion in another session.
    let cache = FulltextCache::open(&state.cfg.data_dir).unwrap();
    let cached = cache
        .put_bytes("arxiv:2402.00123", Some("v1"), "tex", "arxiv_eprint", SYNTHETIC_TEX)
        .unwrap();
    drop(cache);

    let env = ResearchEnv::new(&state.cfg.data_dir);
    let ledger_mutex = Mutex::new(Ledger::open(&state.cfg.data_dir).unwrap());
    ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id: "origin-sess",
            file_name: "2402.00123v1.tex",
            origin: "https://arxiv.org/e-print/2402.00123v1",
            raw_bytes: SYNTHETIC_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Synthetic Route Fixture Paper"),
        },
    )
    .unwrap();
    drop(ledger_mutex);

    // Rebuild index via `POST /api/library/rebuild`.
    let (st, rebuilt) = handle_library(&state, "POST", &["rebuild"], &qmap(&[]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(rebuilt["report"]["papers"], 1);

    // Attach paper to the created session via `POST /api/library/{key}/attach`.
    let attach_body = serde_json::to_vec(&json!({ "session_id": session_id })).unwrap();
    let (st, attached) = handle_library(
        &state,
        "POST",
        &["arxiv:2402.00123", "attach"],
        &qmap(&[]),
        &attach_body,
    )
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(attached["ingested"]["raw_blob"], cached.raw_blob);
    assert_eq!(attached["ingested"]["origin"], "library:arxiv:2402.00123");

    // Delete from library index via `DELETE /api/library/{key}` -> 200, then 404.
    let (st, del) =
        handle_library(&state, "DELETE", &["arxiv:2402.00123"], &qmap(&[]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(del["deleted"], "arxiv:2402.00123");
    assert_eq!(
        handle_library(&state, "DELETE", &["arxiv:2402.00123"], &qmap(&[]), &[])
            .unwrap_err()
            .0,
        404
    );

    // Rebuild restores it because ledger.db was untouched.
    let (st, rebuilt2) = handle_library(&state, "POST", &["rebuild"], &qmap(&[]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(rebuilt2["report"]["papers"], 1);
    let (st, got): (u16, Value) =
        handle_library(&state, "GET", &["arxiv:2402.00123"], &qmap(&[]), &[]).unwrap();
    assert_eq!(st, 200);
    assert_eq!(got["paper"]["title"], "Synthetic Route Fixture Paper");
}

#[test]
fn auth_guard_requires_header_token_and_loopback_origin_for_library_routes() {
    let auth = AuthGuard::new(18_420).unwrap();
    let cookie = format!("{}={}", auth.cookie_name(), "ignored");

    // Unauthenticated GET /api/library -> 401.
    let unauth = ApiCredentials {
        method: "GET",
        path: "/api/library",
        cookie: Some(&cookie),
        token_hdr: None,
        auth_hdr: None,
        origin: None,
        sec_fetch_site: Some("same-origin"),
    };
    assert_eq!(auth.authorize_api(&unauth).unwrap_err().0, 401);

    // Header token authorizes GET /api/library.
    let authed_get = ApiCredentials {
        method: "GET",
        path: "/api/library",
        cookie: None,
        token_hdr: Some(auth.token()),
        auth_hdr: None,
        origin: None,
        sec_fetch_site: None,
    };
    assert!(auth.authorize_api(&authed_get).is_ok());

    // Cross-origin mutation POST /api/library/{key}/mark is rejected with 403 even with token.
    let cross_origin = ApiCredentials {
        method: "POST",
        path: "/api/library/arxiv:2402.00123/mark",
        cookie: None,
        token_hdr: Some(auth.token()),
        auth_hdr: None,
        origin: Some("https://evil.example"),
        sec_fetch_site: None,
    };
    assert_eq!(auth.authorize_api(&cross_origin).unwrap_err().0, 403);
}
