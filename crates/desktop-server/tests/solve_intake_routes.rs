//! Integration tests for Solve S17a TeX-only check-a-derivation intake
//! (`POST /api/sessions/{id}/solve/transcribe` + `solve/ack { kind: "transcription" }`).

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::solve_derivation::{MAX_DERIVATION_TEX_BYTES, PDF_REJECT_MESSAGE};
use research_tools::solve_status::wording;
use serde_json::{json, Value};
use tempfile::TempDir;

fn create_solve_session(port: u16, tok: &str, sid: &str) {
    let auth = [("X-ReSearch-Token", tok)];
    let body = serde_json::to_vec(&json!({ "id": sid, "mode": "solve" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &body);
    assert_eq!(st, 200, "create solve session failed: {}", String::from_utf8_lossy(&raw));
}

#[test]
fn solve_transcribe_splits_tex_is_idempotent_supports_replace_and_confirms_by_exact_sha() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let _cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    create_solve_session(port, &tok, "s-s17-tex");

    let tex = r#"\documentclass{article}
\begin{document}
\section{Oscillator}
\begin{equation}
  H = \frac{p^2}{2m} + \frac{1}{2} m \omega^2 x^2 \label{eq:h0}
\end{equation}
\begin{align}
  E_0 &= \frac{1}{2}\hbar\omega \\
  E_1 &= \frac{3}{2}\hbar\omega
\end{align}
\end{document}
"#;

    let req_body = serde_json::to_vec(&json!({ "file_name": "../CON.tex", "tex": tex })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-s17-tex/solve/transcribe", &auth, &req_body);
    assert_eq!(st, 200, "transcribe failed: {}", String::from_utf8_lossy(&raw));
    let val: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(val["ok"], true);
    assert_eq!(val["idempotent"], false);
    assert_eq!(val["equation_count"], 3);
    assert_eq!(val["outline_style"], "tex");
    assert!(val["elapsed_ms"].as_u64().unwrap() < 1000);
    assert_eq!(val["given_source"]["source_id"], val["source_id"]);
    assert_eq!(val["given_source"]["file_sha256"], val["raw_blob"]);

    // Repeated identical transcribe is idempotent (no duplicate steps).
    let (st2, _, raw2) = raw_http(port, "POST", "/api/sessions/s-s17-tex/solve/transcribe", &auth, &req_body);
    assert_eq!(st2, 200);
    let val2: Value = serde_json::from_slice(&raw2).unwrap();
    assert_eq!(val2["idempotent"], true);
    assert_eq!(val2["view"]["steps"].as_array().unwrap().len(), 3);

    // Different derivation without replace=true returns 409 Conflict.
    let diff_tex = r#"\begin{equation} z = 1 \end{equation}"#;
    let diff_body = serde_json::to_vec(&json!({ "tex": diff_tex })).unwrap();
    let (st_conf, _, _) = raw_http(port, "POST", "/api/sessions/s-s17-tex/solve/transcribe", &auth, &diff_body);
    assert_eq!(st_conf, 409);

    // Verify 0 model calls and source.ingested recorded in ledger.
    let ledger = Ledger::open(&data_dir).unwrap();
    let events = ledger.read_session("s-s17-tex").unwrap();
    assert!(events.iter().any(|ev| ev.kind == "source.ingested"));
    assert_eq!(events.iter().filter(|ev| ev.kind == "model.request").count(), 0);

    // Consecutive pairs link e-1 -> e-2 -> e-3.
    let pairs = val["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0]["from_entry_id"], "e-1");
    assert_eq!(pairs[0]["to_entry_id"], "e-2");
    assert_eq!(pairs[1]["from_entry_id"], "e-2");
    assert_eq!(pairs[1]["to_entry_id"], "e-3");

    let view_steps = val["view"]["steps"].as_array().unwrap();
    assert_eq!(view_steps.len(), 3);
    assert_eq!(view_steps[0]["depends_on"], json!([]));
    assert_eq!(view_steps[1]["depends_on"], json!(["e-1"]));
    assert_eq!(view_steps[2]["depends_on"], json!(["e-2"]));
    for step in view_steps {
        assert_eq!(step["badge"]["own"], "unchecked");
        assert_eq!(step["badge"]["effective"], "unchecked");
        let flags = step["badge"]["flags"].as_array().unwrap();
        assert!(flags.iter().any(|f| f.as_str() == Some(wording::FLAG_TRANSCRIPTION_UNCONFIRMED)));
    }

    // Wrong entry_sha256 is rejected with 409 Conflict.
    let bad_ack = serde_json::to_vec(&json!({
        "entry_id": "e-1",
        "kind": "transcription",
        "entry_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-s17-tex/solve/ack", &auth, &bad_ack);
    assert_eq!(st, 409);

    // Confirm all steps using their exact entry_sha256 clears every "transcription unconfirmed" flag.
    let mut last_view = val["view"].clone();
    for step in view_steps {
        let ack_body = serde_json::to_vec(&json!({
            "entry_id": step["entry_id"],
            "kind": "transcription",
            "entry_sha256": step["entry_sha256"],
        }))
        .unwrap();
        let (st, _, ack_raw) = raw_http(port, "POST", "/api/sessions/s-s17-tex/solve/ack", &auth, &ack_body);
        assert_eq!(st, 200, "ack failed: {}", String::from_utf8_lossy(&ack_raw));
        let ack_val: Value = serde_json::from_slice(&ack_raw).unwrap();
        let badge_flags = ack_val["badge"]["flags"].as_array().unwrap();
        assert!(!badge_flags.iter().any(|f| f.as_str() == Some(wording::FLAG_TRANSCRIPTION_UNCONFIRMED)));
        last_view = ack_val["view"].clone();
    }
    for step in last_view["steps"].as_array().unwrap() {
        let flags = step["badge"]["flags"].as_array().unwrap();
        assert!(!flags.iter().any(|f| f.as_str() == Some(wording::FLAG_TRANSCRIPTION_UNCONFIRMED)));
    }

    // replace=true supersedes prior steps cleanly.
    let repl_body = serde_json::to_vec(&json!({ "tex": diff_tex, "replace": true })).unwrap();
    let (st_rep, _, raw_rep) = raw_http(port, "POST", "/api/sessions/s-s17-tex/solve/transcribe", &auth, &repl_body);
    assert_eq!(st_rep, 200);
    let rep_val: Value = serde_json::from_slice(&raw_rep).unwrap();
    let active: Vec<&Value> = rep_val["view"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["superseded_by"].is_null())
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["entry_id"], "e-4");
}

#[test]
fn solve_transcribe_rejects_pdf_raw_pdf_oversize_and_non_solve_sessions() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let _cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    create_solve_session(port, &tok, "s-s17-pdf");

    // 1. PDF filename rejected 400 without writing any ledger events.
    let pdf_ext = serde_json::to_vec(&json!({
        "file_name": "paper.pdf",
        "tex": "\\begin{equation}x=1\\end{equation}"
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-s17-pdf/solve/transcribe", &auth, &pdf_ext);
    assert_eq!(st, 400);
    assert!(String::from_utf8_lossy(&raw).contains(PDF_REJECT_MESSAGE));

    // 2. PDF magic header in JSON rejected 400.
    let pdf_magic = serde_json::to_vec(&json!({ "file_name": "spoofed.tex", "tex": "%PDF-1.7\n1 0 obj\n" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-s17-pdf/solve/transcribe", &auth, &pdf_magic);
    assert_eq!(st, 400);
    assert!(String::from_utf8_lossy(&raw).contains(PDF_REJECT_MESSAGE));

    // 3. Raw binary %PDF body rejected 400 with PDF_REJECT_MESSAGE (before JSON parse).
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-s17-pdf/solve/transcribe", &auth, b"%PDF-1.4\n...");
    assert_eq!(st, 400);
    assert!(String::from_utf8_lossy(&raw).contains(PDF_REJECT_MESSAGE));

    // 4. Oversize TeX (> 2 MiB) rejected 413 Payload Too Large.
    let huge_tex = format!("\\begin{{equation}}{}\\end{{equation}}", "x".repeat(MAX_DERIVATION_TEX_BYTES + 10));
    let huge_body = serde_json::to_vec(&json!({ "tex": huge_tex })).unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-s17-pdf/solve/transcribe", &auth, &huge_body);
    assert_eq!(st, 413);

    let ledger = Ledger::open(&data_dir).unwrap();
    let events = ledger.read_session("s-s17-pdf").unwrap();
    assert!(!events.iter().any(|ev| ev.kind == "source.ingested" || ev.kind == "proof.entry"));

    // 5. Non-Solve (research) session rejected 400.
    let body = serde_json::to_vec(&json!({ "id": "r-non-solve", "mode": "research" })).unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions", &auth, &body);
    assert_eq!(st, 200);
    let valid_tex = serde_json::to_vec(&json!({ "tex": "\\begin{equation}a=b\\end{equation}" })).unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/r-non-solve/solve/transcribe", &auth, &valid_tex);
    assert_eq!(st, 400);
}
