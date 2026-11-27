//! Integration tests for `GET /api/sessions/{id}/solve/artifacts/{artifact_id}` (S18 Part A).

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::{sha256_hex, Ledger};
use research_tools::solve_artifacts::{
    harvest_scratch_artifacts, MIME_CSV, MIME_JSON, MIME_PDF, MIME_PLAIN, MIME_PNG, MIME_SVG,
};
use research_tools::solve_model::event_kinds;
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

fn header_val<'a>(hdrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hdrs.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[test]
fn solve_artifacts_route_enforces_auth_mode_integrity_and_safe_dispositions() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let scratch = tmp.path().join("scratch");
    fs::create_dir_all(scratch.join("artifacts")).unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // Create a Solve session and a non-Solve (Research) session.
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &auth,
        &serde_json::to_vec(&json!({ "id": "s-art-route", "mode": "solve" })).unwrap(),
    );
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));

    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &auth,
        &serde_json::to_vec(&json!({ "id": "s-res-route", "mode": "research" })).unwrap(),
    );
    assert_eq!(st, 200);

    // Harvest PNG, SVG, CSV, JSON, PDF, and plain text into the Solve session.
    let mut png_bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    png_bytes.extend_from_slice(b"\x00\x00\x00\rIHDR-mock");
    let svg_bytes = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"4\"/></svg>".to_vec();
    let csv_bytes = b"t,val\n0,1.0\n1,2.0\n".to_vec();
    let json_bytes = br#"{"chi2": 0.01}"#.to_vec();
    let pdf_bytes = b"%PDF-1.7\n%%EOF\n".to_vec();
    let txt_bytes = b"Summary report\n".to_vec();

    fs::write(scratch.join("artifacts/plot.png"), &png_bytes).unwrap();
    fs::write(scratch.join("artifacts/vector.svg"), &svg_bytes).unwrap();
    fs::write(scratch.join("artifacts/data.csv"), &csv_bytes).unwrap();
    fs::write(scratch.join("artifacts/metrics.json"), &json_bytes).unwrap();
    fs::write(scratch.join("artifacts/fig.pdf"), &pdf_bytes).unwrap();
    fs::write(scratch.join("artifacts/notes.txt"), &txt_bytes).unwrap();

    let ledger = Ledger::open(&data_dir).unwrap();
    let rep = harvest_scratch_artifacts(&ledger, "s-art-route", &scratch, "call-1", Some("e-2"))
        .unwrap();
    assert_eq!(rep.harvested.len(), 6);

    let by_mime: std::collections::BTreeMap<_, _> = rep
        .harvested
        .iter()
        .map(|h| (h.artifact.mime.as_str(), h.artifact.artifact_id.as_str()))
        .collect();

    // GET /solve/view exposes all 6 artifacts.
    let (st, _, view_raw) = raw_http(port, "GET", "/api/sessions/s-art-route/solve/view", &auth, b"");
    assert_eq!(st, 200);
    let view: Value = serde_json::from_slice(&view_raw).unwrap();
    assert_eq!(view["artifacts"].as_array().unwrap().len(), 6);

    // Auth enforcement: unauthenticated and cookie-only are rejected with 401.
    let png_path = format!("/api/sessions/s-art-route/solve/artifacts/{}", by_mime[MIME_PNG]);
    assert_eq!(raw_http(port, "GET", &png_path, &[], b"").0, 401);
    assert_eq!(
        raw_http(port, "GET", &png_path, &[("Cookie", cookie.as_str())], b"").0,
        401
    );

    // Bearer auth also works.
    let bearer_val = format!("Bearer {tok}");
    let (st, hdrs, body) = raw_http(
        port,
        "GET",
        &png_path,
        &[("Authorization", bearer_val.as_str())],
        b"",
    );
    assert_eq!(st, 200);
    assert_eq!(body, png_bytes);
    assert_eq!(header_val(&hdrs, "content-type"), Some(MIME_PNG));
    assert_eq!(header_val(&hdrs, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header_val(&hdrs, "content-security-policy"), Some("sandbox"));
    assert_eq!(
        header_val(&hdrs, "content-disposition"),
        Some("inline; filename=\"plot.png\"")
    );

    // SVG, CSV, JSON, PDF, and plain text MUST force Content-Disposition: attachment.
    for (mime, expected_bytes, expected_name) in [
        (MIME_SVG, svg_bytes.as_slice(), "vector.svg"),
        (MIME_CSV, csv_bytes.as_slice(), "data.csv"),
        (MIME_JSON, json_bytes.as_slice(), "metrics.json"),
        (MIME_PDF, pdf_bytes.as_slice(), "fig.pdf"),
        (MIME_PLAIN, txt_bytes.as_slice(), "notes.txt"),
    ] {
        let path = format!("/api/sessions/s-art-route/solve/artifacts/{}", by_mime[mime]);
        let (st, hdrs, body) = raw_http(port, "GET", &path, &auth, b"");
        assert_eq!(st, 200, "failed for {mime}");
        assert_eq!(body, expected_bytes);
        assert_eq!(header_val(&hdrs, "content-type"), Some(mime));
        assert_eq!(header_val(&hdrs, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header_val(&hdrs, "content-security-policy"), Some("sandbox"));
        assert_eq!(
            header_val(&hdrs, "content-disposition"),
            Some(format!("attachment; filename=\"{expected_name}\"").as_str())
        );
    }

    // Non-Solve session refuses artifact route; missing artifact returns 404.
    let (st, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-res-route/solve/artifacts/art-1",
        &auth,
        b"",
    );
    assert_eq!(st, 400);
    let (st, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-art-route/solve/artifacts/art-999",
        &auth,
        b"",
    );
    assert_eq!(st, 404);
    let (st, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-art-route/solve/artifacts/..",
        &auth,
        b"",
    );
    assert_eq!(st, 400);

    // Tampered / spoofed artifact event (HTML bytes claiming image/png) fails closed with 400.
    let html_bytes = b"<html><script>alert(1)</script></html>";
    let html_blob = ledger.put_blob(html_bytes).unwrap();
    ledger
        .append_with_hash(
            "s-art-route",
            event_kinds::SOLVE_ARTIFACT,
            1,
            &json!({
                "artifact_id": "art-evil",
                "name": "evil.png",
                "mime": MIME_PNG,
                "blob": html_blob,
                "sha256": sha256_hex(html_bytes),
                "bytes": html_bytes.len(),
                "call_id": "call-evil",
            }),
        )
        .unwrap();
    let (st, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-art-route/solve/artifacts/art-evil",
        &auth,
        b"",
    );
    assert_eq!(st, 400, "spoofed HTML claiming PNG must fail closed");

    // Corrupted blob file fails SHA-256 verification and returns 400.
    let png_id = by_mime[MIME_PNG];
    let png_hash = rep
        .harvested
        .iter()
        .find(|h| h.artifact.artifact_id == png_id)
        .unwrap()
        .artifact
        .blob
        .clone();
    let blob_path = data_dir.join("blobs").join(&png_hash[..2]).join(&png_hash);
    fs::write(&blob_path, b"corrupted").unwrap();
    let (st, _, _) = raw_http(port, "GET", &png_path, &auth, b"");
    assert_eq!(st, 400, "corrupted blob must fail SHA-256 verification");

    srv.shutdown();
}
