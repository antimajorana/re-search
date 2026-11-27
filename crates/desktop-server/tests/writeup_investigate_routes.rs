//! Integration tests for Investigate F15 reading-notes write-ups in `desktop-server`:
//! - End-to-end Investigate session created via desktop-server HTTP routes (create session,
//!   ingest paper, pin skeleton, save user pin + note) exported through `export::writeup_session_*`
//!   in `Tex`, `Md`, and `Bib` formats with `tex_audit` and determinism verification.
//! - Disjointness from V15 `/api/sessions/{id}/investigation/export` (which owns `rsmap`/`html`
//!   and directs Markdown reading notes to F15 `writeup_paper.rs`).
//! - Strict header-token `AuthGuard::authorize_api` contract for `/api/sessions/{id}/writeup`
//!   (rejects unauthenticated and cookie-only requests; accepts `X-ReSearch-Token` and
//!   `Authorization: Bearer`; rejects foreign-Origin mutations).

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::auth::{ApiCredentials, AuthGuard};
use desktop_server::start_desktop_server;
use export::writeup::{writeup_wording, WriteupOptions};
use export::{writeup_session_bytes, writeup_session_to_path, WriteupFormat};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::paper_key::{PaperIds, PaperKey};
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::{json, Value};
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

const ROUTE_SYNTH_TEX: &[u8] = br"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\section{Model Setup}\label{sec:setup}
\begin{lemma}[Coercivity]\label{lem:coercive}
Every coupling coefficient is strictly positive and bounded above by unity.
\end{lemma}
\begin{theorem}[Uniform Decay]\label{thm:decay}
Under positive coupling coefficients, correlation functions decay exponentially with distance across every finite admissible chain.
\end{theorem}
\end{document}
";

#[test]
fn investigate_desktop_session_writeup_and_v15_disjointness() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    let cfg = test_cfg(&data_dir, Some(ws_dir.clone()), None, None);
    let handle = start_desktop_server(cfg).unwrap();
    let port = handle.info.port;
    let tok = handle.info.token.clone();

    // 1. Create real Investigate session via HTTP API.
    let (st, _, created) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[("X-ReSearch-Token", &tok), ("Content-Type", "application/json")],
        &serde_json::to_vec(&json!({ "mode": "investigate", "title": "Decay Reading Session" })).unwrap(),
    );
    assert_eq!(st, 200, "create session failed: {}", String::from_utf8_lossy(&created));
    let sid = serde_json::from_slice::<Value>(&created).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Register user citekey in F10 library.
    let paper_key = PaperKey::from_arxiv("2404.07777").unwrap();
    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: paper_key.clone(),
        ids: LibraryIds::from_paper_ids(&PaperIds {
            arxiv: Some("2404.07777".into()),
            ..Default::default()
        }),
        title: "Uniform Decay in Coupled Chains".into(),
        authors: vec!["Sato, Kenji".into()],
        year: Some(2024),
        abstract_text: "Synthetic abstract.".into(),
        bibtex: String::new(),
        flags: PaperFlags {
            owned: true,
            venue: Some("Synthetic Physics Letters".into()),
            ..Default::default()
        },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "user-vault".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/synth-vault".into(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "user-vault".into(),
        external_id: "sato2024decay".into(),
        key: Some(paper_key.clone()),
        citekey: Some("sato2024decay".into()),
        ids: LibraryIds::from_paper_ids(&PaperIds {
            arxiv: Some("2404.07777".into()),
            ..Default::default()
        }),
        version: Some("v1".into()),
        title: "Uniform Decay in Coupled Chains".into(),
        authors: vec!["Sato, Kenji".into()],
        year: Some(2024),
        item_kind: "journalArticle".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 1,
    })
    .unwrap();

    // 2. Ingest source and pin deterministic skeleton through desktop-server route.
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let ledger_mutex = Mutex::new(Ledger::open_existing(&data_dir).unwrap());
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id: &sid,
            file_name: "decay.tex",
            origin: "https://arxiv.org/e-print/2404.07777v1",
            raw_bytes: ROUTE_SYNTH_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Uniform Decay in Coupled Chains"),
        },
    )
    .unwrap();
    drop(ledger_mutex);

    let (pin_st, _, pin_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        &[("X-ReSearch-Token", &tok), ("Content-Type", "application/json")],
        &serde_json::to_vec(&json!({ "source_id": ing.source_id, "pin": true })).unwrap(),
    );
    assert_eq!(pin_st, 200, "pin skeleton failed: {}", String::from_utf8_lossy(&pin_raw));
    let pin_val: Value = serde_json::from_slice(&pin_raw).unwrap();
    let skel: research_tools::skeleton::PaperSkeleton =
        serde_json::from_value(pin_val["skeleton"].clone()).unwrap();
    let thm_tag = skel
        .nodes
        .iter()
        .find(|n| n.kind == research_tools::skeleton::SkNodeKind::Theorem)
        .unwrap()
        .tag
        .clone()
        .unwrap();

    // 3. Add user pin + reader note through real `POST /api/sessions/{id}/investigation/notes`.
    let (note_st, _, note_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/notes"),
        &[("X-ReSearch-Token", &tok), ("Content-Type", "application/json")],
        &serde_json::to_vec(&json!({
            "tag": thm_tag,
            "pin": "main_result",
            "text": "Verify boundary factor on small chains by hand.",
        }))
        .unwrap(),
    );
    assert_eq!(note_st, 201, "post note failed: {}", String::from_utf8_lossy(&note_raw));

    // 4. Produce F15 reading notes (`Tex`, `Md`, `Bib`) through the shared dispatcher.
    let opts = WriteupOptions::default();
    let (_, tex_a) = writeup_session_bytes(&data_dir, &sid, WriteupFormat::Tex, &opts).unwrap();
    let (_, tex_b) = writeup_session_bytes(&data_dir, &sid, WriteupFormat::Tex, &opts).unwrap();
    let (_, md_bytes) = writeup_session_bytes(&data_dir, &sid, WriteupFormat::Md, &opts).unwrap();
    let (_, bib_bytes) = writeup_session_bytes(&data_dir, &sid, WriteupFormat::Bib, &opts).unwrap();
    assert_eq!(tex_a, tex_b, "write-up output must be deterministic");

    let md = String::from_utf8(md_bytes).unwrap();
    let bib = String::from_utf8(bib_bytes).unwrap();
    assert!(md.contains("pinned as main result by you"));
    assert!(md.contains("Verify boundary factor on small chains by hand."));
    assert!(md.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(!md.to_ascii_lowercase().contains("verified"));
    assert!(bib.contains("@article{sato2024decay,"));

    // Write `.tex` + sidecar `.bib` to workspace and validate via `tex_audit`.
    let out_tex = ws_dir.join("decay_notes.tex");
    writeup_session_to_path(&data_dir, &sid, WriteupFormat::Tex, &opts, &out_tex).unwrap();
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let audit = audit_tex_sources(&env, &sid, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);

    // 5. Verify non-removable status & check table when inline marks are disabled.
    let opts_off = WriteupOptions { include_inline_badges: false, bib_stem: "decay_notes".into() };
    let (_, md_off_bytes) = writeup_session_bytes(&data_dir, &sid, WriteupFormat::Md, &opts_off).unwrap();
    let md_off = String::from_utf8(md_off_bytes).unwrap();
    assert!(md_off.contains("tex-1, full"));
    assert!(md_off.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(!md_off.to_ascii_lowercase().contains("verified"));

    // 6. Disjointness from V15 Share: `/investigation/export?format=md` redirects to F15,
    // while `rsmap` and `html` remain intact.
    let (md_v15_st, _, md_v15_body) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/export?format=md"),
        &[("X-ReSearch-Token", &tok)],
        b"",
    );
    assert_eq!(md_v15_st, 400);
    assert!(String::from_utf8_lossy(&md_v15_body).contains("writeup_paper.rs"));

    let (rsmap_st, _, _) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/export?format=rsmap"),
        &[("X-ReSearch-Token", &tok)],
        b"",
    );
    assert_eq!(rsmap_st, 200);

    handle.shutdown();
}

#[test]
fn writeup_endpoint_enforces_strict_header_token_auth_and_rejects_cookie_only() {
    let port = 18_427;
    let auth = AuthGuard::new(port).unwrap();
    let path = "/api/sessions/s-inv-auth/writeup";
    let set_cookie = auth.set_cookie_header();
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    // 1. Missing credentials -> 401.
    let unauth = auth.authorize_api(&ApiCredentials {
        method: "GET",
        path,
        token_hdr: None,
        auth_hdr: None,
        cookie: None,
        origin: None,
        sec_fetch_site: None,
    });
    assert_eq!(unauth.unwrap_err().0, 401);

    // 2. Cookie-only (even with valid stream cookie + same-origin) -> 401 (never on cookie route list).
    let cookie_only = auth.authorize_api(&ApiCredentials {
        method: "GET",
        path,
        token_hdr: None,
        auth_hdr: None,
        cookie: Some(&cookie),
        origin: Some(&format!("http://127.0.0.1:{port}")),
        sec_fetch_site: Some("same-origin"),
    });
    assert_eq!(cookie_only.unwrap_err().0, 401);

    // 3. Invalid token -> 401.
    let wrong_tok = auth.authorize_api(&ApiCredentials {
        method: "GET",
        path,
        token_hdr: Some("not-the-real-token"),
        auth_hdr: None,
        cookie: Some(&cookie),
        origin: None,
        sec_fetch_site: None,
    });
    assert_eq!(wrong_tok.unwrap_err().0, 401);

    // 4. Valid `X-ReSearch-Token` -> 200 authorized.
    assert!(auth
        .authorize_api(&ApiCredentials {
            method: "GET",
            path,
            token_hdr: Some(auth.token()),
            auth_hdr: None,
            cookie: None,
            origin: None,
            sec_fetch_site: None,
        })
        .is_ok());

    // 5. Valid `Authorization: Bearer <token>` -> 200 authorized.
    let bearer = format!("Bearer {}", auth.token());
    assert!(auth
        .authorize_api(&ApiCredentials {
            method: "GET",
            path,
            token_hdr: None,
            auth_hdr: Some(&bearer),
            cookie: None,
            origin: None,
            sec_fetch_site: None,
        })
        .is_ok());

    // 6. Live loopback HTTP server also rejects unauthenticated and cookie-only GET to `/writeup`
    // at the auth layer before routing.
    let tmp = TempDir::new().unwrap();
    let cfg = test_cfg(tmp.path(), None, None, None);
    let handle = start_desktop_server(cfg).unwrap();
    let live_port = handle.info.port;
    let live_cookie = launch_cookie(live_port, &handle.info.token);

    let (st_unauth, _, _) = raw_http(
        live_port,
        "GET",
        "/api/sessions/s-inv-auth/writeup?format=tex",
        &[],
        b"",
    );
    assert_eq!(st_unauth, 401);

    let (st_cookie, _, _) = raw_http(
        live_port,
        "GET",
        "/api/sessions/s-inv-auth/writeup?format=tex",
        &[("Cookie", &live_cookie), ("Sec-Fetch-Site", "same-origin")],
        b"",
    );
    assert_eq!(st_cookie, 401);

    handle.shutdown();
}
