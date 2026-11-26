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

fn minimal_two_page_pdf() -> Vec<u8> {
    let p1 = b"BT /F1 11 Tf 40 720 Td (1. Setup) Tj 0 -24 Td (Lemma 1.1. Bounded energy holds.) Tj ET";
    let p2 = b"0 0 1 rg 0 0 100 100 re f";
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", p1.len()).as_bytes(), p1, b"\nendstream"].concat(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", p2.len()).as_bytes(), p2, b"\nendstream"].concat(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize];
    for (idx, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for off in &offsets[1..] {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len())
            .as_bytes(),
    );
    out
}

#[test]
fn v4_open_routes_and_v17_pdf_skeleton_work_over_http_with_zero_model_calls() {
    let dir = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.info.port;
    let tok = srv.info.token.clone();
    let tok_hdr = ("X-ReSearch-Token", tok.as_str());

    let (st, _, created) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[tok_hdr, ("Content-Type", "application/json")],
        br#"{"mode":"investigate","title":"V4/V17 PDF check"}"#,
    );
    assert_eq!(st, 200);
    let sid = serde_json::from_slice::<Value>(&created).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Auth guard covers /investigation/open and /investigation/open-status.
    let (unauth_open, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/open"),
        &[],
        br#"{"source_id":"s1"}"#,
    );
    assert_eq!(unauth_open, 401);
    let (unauth_status, _, _) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/open-status?job_id=j1"),
        &[],
        b"",
    );
    assert_eq!(unauth_status, 401);
    let (bad_origin, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/open"),
        &[tok_hdr, ("Origin", "https://evil.example"), ("Content-Type", "application/json")],
        br#"{"source_id":"s1"}"#,
    );
    assert_eq!(bad_origin, 403);

    // Ingest a 2-page PDF (page 1 text layer, page 2 image-only) and open via POST .../investigation/open.
    let pdf_bytes = minimal_two_page_pdf();
    let env = ResearchEnv::new(dir.path().to_path_buf());
    let ledger = Mutex::new(Ledger::open(dir.path()).unwrap());
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: &sid,
            file_name: "hybrid.pdf",
            origin: "upload",
            raw_bytes: &pdf_bytes,
            source_id_hint: None,
            section_filter: None,
            title: Some("Hybrid PDF"),
        },
    )
    .unwrap();
    drop(ledger);

    let open_body = serde_json::to_vec(&json!({ "source_id": ingested.source_id })).unwrap();
    let (open_st, _, open_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/open"),
        &[tok_hdr, ("Content-Type", "application/json")],
        &open_body,
    );
    assert_eq!(open_st, 200, "open failed: {}", String::from_utf8_lossy(&open_raw));
    let open_val: Value = serde_json::from_slice(&open_raw).unwrap();
    let job_id = open_val["job_id"].as_str().unwrap();
    assert_eq!(open_val["job"]["status"], "done");
    assert!(open_val["job"]["skeleton_seq"].as_u64().is_some());

    let (st_status, _, raw_status) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/open-status?job_id={job_id}"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(st_status, 200);
    let status_val: Value = serde_json::from_slice(&raw_status).unwrap();
    assert_eq!(status_val["job"]["status"], "done");

    let (mv_st, _, mv_raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/mapview"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(mv_st, 200);
    let mv: Value = serde_json::from_slice(&mv_raw).unwrap();
    assert_eq!(mv["builder"], "pdf-1");
    assert_ne!(mv["quality"], "full");
    let qline = mv["quality_line"].as_str().unwrap();
    assert!(qline.starts_with("Estimated from PDF text; theorem boundaries may be wrong"));
    assert_eq!(
        qline.matches("theorem boundaries may be wrong").count(),
        1,
        "quality_line must not duplicate PDF base reason: {qline}"
    );
    assert_eq!(mv["image_only_pages"], json!([2]));

    let nodes = mv["nodes"].as_array().unwrap();
    assert!(!nodes.is_empty());
    let first_tag = nodes[0]["id"].as_str().unwrap();
    assert_eq!(nodes[0]["page_range"], json!([1, 1]));
    assert_eq!(nodes[0]["render_math"], json!(true));

    let (nt_st, _, nt_raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/node-text?tag={first_tag}"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(nt_st, 200);
    let nt: Value = serde_json::from_slice(&nt_raw).unwrap();
    assert_eq!(nt["page_range"], json!([1, 1]));
    assert!(!nt["text"].as_str().unwrap().is_empty());

    let events = Ledger::open(dir.path()).unwrap().read_session(&sid).unwrap();
    assert!(!events.iter().any(|e| e.kind == "model.request"));

    srv.shutdown();
}
