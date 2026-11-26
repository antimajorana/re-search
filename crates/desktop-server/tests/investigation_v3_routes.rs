//! End-to-end HTTP dispatcher and auth tests for Investigate V3 routes
//! (`/api/sessions/{id}/investigation/{skeleton,mapview,node-text}`).

mod common;

use common::{raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::sync::Mutex;

const SAMPLE_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\section{Framework}\label{sec:fw}
\begin{lemma}[Positivity]\label{lem:pos}
For every $x \ne 0$, $Q(x) > 0$.
\end{lemma}
\begin{theorem}[Stability]\label{thm:stab}
Under Lemma~\ref{lem:pos}, trajectories remain bounded.
\end{theorem}
\begin{proof}
Combine Lemma~\ref{lem:pos} with conservation of $Q$.
\end{proof}
\end{document}
"#;

#[test]
fn v3_skeleton_routes_enforce_auth_pin_etag_old_session_open_and_zero_model_calls() {
    let dir = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.info.port;
    let tok = srv.info.token.clone();
    let tok_hdr = ("X-ReSearch-Token", tok.as_str());

    // 1. Create an Investigate session through the normal API entry point.
    let (st, _, created) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[tok_hdr, ("Content-Type", "application/json")],
        br#"{"mode":"investigate","title":"V3 HTTP check"}"#,
    );
    assert_eq!(st, 200, "create session failed: {}", String::from_utf8_lossy(&created));
    let created_json: Value = serde_json::from_slice(&created).unwrap();
    let sid = created_json["id"].as_str().unwrap().to_string();

    // 2. Auth guard rejects unauthenticated and foreign-Origin calls on all three V3 endpoints.
    for path in [
        format!("/api/sessions/{sid}/investigation/skeleton"),
        format!("/api/sessions/{sid}/investigation/mapview"),
        format!("/api/sessions/{sid}/investigation/node-text?tag=n-test"),
    ] {
        let (unauth, _, _) = raw_http(port, "GET", &path, &[], b"");
        assert_eq!(unauth, 401, "expected 401 without token on {path}");
    }
    let (bad_origin, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        &[
            tok_hdr,
            ("Origin", "https://evil.example"),
            ("Content-Type", "application/json"),
        ],
        br#"{"source_id":"s1"}"#,
    );
    assert_eq!(bad_origin, 403);

    // 3. Ingest a TeX source and pin+build its deterministic skeleton via POST .../skeleton.
    let env = ResearchEnv::new(dir.path().to_path_buf());
    let ledger = Mutex::new(Ledger::open(dir.path()).unwrap());
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: &sid,
            file_name: "stability.tex",
            origin: "https://arxiv.org/e-print/2402.00001v2",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Stability note"),
        },
    )
    .unwrap();
    drop(ledger);

    let pin_body = serde_json::to_vec(&json!({
        "source_id": ingested.source_id,
        "pin": true,
    }))
    .unwrap();
    let (pin_st, pin_hdrs, pin_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        &[tok_hdr, ("Content-Type", "application/json")],
        &pin_body,
    );
    assert_eq!(pin_st, 200, "pin skeleton failed: {}", String::from_utf8_lossy(&pin_raw));
    let pin_val: Value = serde_json::from_slice(&pin_raw).unwrap();
    let blob_etag = pin_val["etag"].as_str().unwrap().to_string();
    let skel_seq = pin_val["seq"].as_u64().unwrap();
    assert!(!blob_etag.is_empty());
    assert!(
        pin_hdrs
            .iter()
            .any(|(k, v)| k == "etag" && v == &format!("\"{blob_etag}\"")),
        "expected quoted ETag header, got {pin_hdrs:?}"
    );
    assert_eq!(pin_val["skeleton"]["paper_key"], "arxiv:2402.00001");
    assert_eq!(pin_val["skeleton"]["version"], "v2");

    // 4. GET /api/sessions/{id} surfaces skeleton_seq while keeping zero model.request events.
    let (sess_st, _, sess_raw) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &[tok_hdr], b"");
    assert_eq!(sess_st, 200);
    let sess_val: Value = serde_json::from_slice(&sess_raw).unwrap();
    assert_eq!(sess_val["investigation"]["skeleton_seq"].as_u64(), Some(skel_seq));
    let events = sess_val["events"].as_array().unwrap();
    assert!(
        !events.iter().any(|e| e["kind"] == "model.request"),
        "expected zero model.request events"
    );

    // 5. ETag conditional GET returns 304 Not Modified on skeleton and mapview.
    let quoted_blob = format!("\"{blob_etag}\"");
    let (sk_304, _, sk_304_body) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        &[tok_hdr, ("If-None-Match", quoted_blob.as_str())],
        b"",
    );
    assert_eq!(sk_304, 304);
    assert!(sk_304_body.is_empty());

    let (mv_st, mv_hdrs, mv_raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/mapview"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(mv_st, 200, "mapview failed: {}", String::from_utf8_lossy(&mv_raw));
    let mv_str = String::from_utf8_lossy(&mv_raw);
    assert!(!mv_str.to_ascii_lowercase().contains("verified"));
    let mv_val: Value = serde_json::from_slice(&mv_raw).unwrap();
    let head_etag = format!("\"{}\"", mv_val["etag"].as_str().unwrap());
    assert!(mv_hdrs.iter().any(|(k, v)| k == "etag" && v == &head_etag));
    let (mv_304, _, mv_304_body) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/mapview"),
        &[tok_hdr, ("If-None-Match", head_etag.as_str())],
        b"",
    );
    assert_eq!(mv_304, 304);
    assert!(mv_304_body.is_empty());

    // 6. GET .../node-text returns verbatim statement + proof text for Theorem 1.2.
    let thm_tag = mv_val["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "theorem")
        .and_then(|n| n["id"].as_str())
        .unwrap()
        .to_string();
    let (nt_st, _, nt_raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/node-text?tag={thm_tag}"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(nt_st, 200, "node-text failed: {}", String::from_utf8_lossy(&nt_raw));
    let nt_val: Value = serde_json::from_slice(&nt_raw).unwrap();
    assert!(nt_val["text"].as_str().unwrap().contains("trajectories remain bounded"));
    assert!(nt_val["proof_text"].as_str().unwrap().contains("conservation of $Q$"));

    // 7. Old session upgrade on open (investigate.md §8.4): pinned TeX without skeleton builds on GET mapview.
    let old_sid = "inv-old-upgrade";
    let ledger = Mutex::new(Ledger::open(dir.path()).unwrap());
    ledger
        .lock()
        .unwrap()
        .append(old_sid, "investigation.started", 1, &json!({}))
        .unwrap();
    let old_ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: old_sid,
            file_name: "old.tex",
            origin: "upload",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Old session"),
        },
    )
    .unwrap();
    ledger
        .lock()
        .unwrap()
        .append(
            old_sid,
            "investigation.paper",
            1,
            &json!({ "source_id": old_ing.source_id }),
        )
        .unwrap();
    drop(ledger);

    let (old_mv_st, _, old_mv_raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{old_sid}/investigation/mapview"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(old_mv_st, 200);
    let old_mv: Value = serde_json::from_slice(&old_mv_raw).unwrap();
    assert!(old_mv["skeleton_seq"].as_u64().is_some());
    assert!(!old_mv["nodes"].as_array().unwrap().is_empty());

    srv.shutdown();
}
