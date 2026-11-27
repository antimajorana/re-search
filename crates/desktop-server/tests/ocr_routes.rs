//! Integration tests for F14b authenticated OCR settings, OCR ingestion,
//! background OCR jobs, and verified PDF page-image / crop serving routes.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::pdf_ocr_models::OCR_INSTALL_PROMPT;
use research_tools::pdf_render::inspect_png_dimensions;
use serde_json::{json, Value};
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn header_val<'a>(hdrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hdrs.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 { u32::from(chunk[1]) } else { 0 };
        let b2 = if chunk.len() > 2 { u32::from(chunk[2]) } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((triple >> 6) & 0x3f) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(triple & 0x3f) as usize] as char } else { '=' });
    }
    out
}

fn stream_obj(dict_body: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict_body} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Builds a minimal self-authored 1-page scanned PDF containing a single uncompressed
/// 8-bit `/DeviceGray` raster Image XObject and no text layer (`PageBasis::None`).
fn build_scanned_gray_pdf(width: u32, height: u32) -> Vec<u8> {
    let pixels: Vec<u8> = (0..(width as usize * height as usize))
        .map(|i| ((i * 17 + 31) % 256) as u8)
        .collect();
    let img_dict = format!(
        "/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8"
    );
    let objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        stream_obj(&img_dict, &pixels),
        stream_obj("", b"q 612 0 0 792 0 0 cm /Im0 Do Q"),
    ];
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objs.len());
    for (idx, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj", idx + 1).unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objs.len() + 1).unwrap();
    for off in offsets {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
        objs.len() + 1
    )
    .unwrap();
    out
}

fn create_session(port: u16, token: &str, id: &str) {
    let body = serde_json::to_vec(&json!({ "id": id, "mode": "research" })).unwrap();
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[("X-ReSearch-Token", token), ("Content-Type", "application/json")],
        &body,
    );
    assert_eq!(st, 200, "create_session failed: {}", String::from_utf8_lossy(&raw));
}

fn configure_missing_tesseract(port: u16, token: &str, missing_exe: &str) -> Value {
    let body = serde_json::to_vec(&json!({
        "enabled": true,
        "tesseract_path": missing_exe,
        "tessdata_dir": null,
        "langs": ["eng"],
    }))
    .unwrap();
    let (st, _, raw) = raw_http(
        port,
        "PUT",
        "/api/settings/ocr",
        &[("X-ReSearch-Token", token), ("Content-Type", "application/json")],
        &body,
    );
    assert_eq!(st, 200, "PUT /api/settings/ocr failed: {}", String::from_utf8_lossy(&raw));
    serde_json::from_slice(&raw).unwrap()
}

#[test]
fn ocr_routes_reject_unauthenticated_and_cookie_only_requests() {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(&tmp.path().join("data"), None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    create_session(port, &tok, "s-ocr-auth");

    let endpoints = [
        ("GET", "/api/settings/ocr", &b""[..]),
        ("PUT", "/api/settings/ocr", &br#"{"enabled":true}"#[..]),
        ("POST", "/api/sessions/s-ocr-auth/ocr/ingest", &br#"{}"#[..]),
        ("GET", "/api/sessions/s-ocr-auth/ocr/jobs/ocr-deadbeef", &b""[..]),
        ("POST", "/api/sessions/s-ocr-auth/ocr/jobs/ocr-deadbeef/cancel", &b""[..]),
        ("GET", "/api/sessions/s-ocr-auth/sources/src-1/page-image?page=1", &b""[..]),
    ];
    for (method, path, body) in endpoints {
        let (st_none, _, _) = raw_http(port, method, path, &[], body);
        assert_eq!(st_none, 401, "unauthenticated {method} {path} must return 401");
        let (st_cookie, _, _) = raw_http(port, method, path, &[("Cookie", cookie.as_str())], body);
        assert_eq!(st_cookie, 401, "cookie-only {method} {path} must return 401");
    }

    let bearer = format!("Bearer {tok}");
    let (st, _, _) = raw_http(port, "GET", "/api/settings/ocr", &[("Authorization", bearer.as_str())], b"");
    assert_eq!(st, 200, "Bearer token must be accepted on GET /api/settings/ocr");
    srv.shutdown();
}

#[test]
fn ocr_settings_defaults_validation_and_persistence_without_mutating_receipts() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];
    create_session(port, &tok, "s-ocr-set");

    let ledger = Ledger::open(&data_dir).unwrap();
    let events_before = ledger.read_session("s-ocr-set").unwrap();

    // Defaults: enabled == true, langs == ["eng"].
    let (st, _, raw) = raw_http(port, "GET", "/api/settings/ocr", &[("X-ReSearch-Token", tok.as_str())], b"");
    assert_eq!(st, 200);
    let initial: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(initial["enabled"], json!(true));
    assert_eq!(initial["langs"], json!(["eng"]));
    assert!(initial["tesseract_path"].is_null());

    // Reject unknown keys, NUL paths, and invalid language tags with 400.
    for bad_payload in [
        json!({ "unknown_key": true }),
        json!({ "tesseract_path": "/bad/path\0exe" }),
        json!({ "tessdata_dir": "/bad/dir\0data" }),
        json!({ "langs": ["../evil"] }),
    ] {
        let (st, _, _) = raw_http(port, "PUT", "/api/settings/ocr", &auth, &serde_json::to_vec(&bad_payload).unwrap());
        assert_eq!(st, 400, "expected 400 for payload {bad_payload}");
    }

    // Setting a non-existent tesseract_path reports unavailable with the exact install prompt.
    let missing_exe = tmp.path().join("definitely-missing-tesseract").display().to_string();
    let put_res = configure_missing_tesseract(port, &tok, &missing_exe);
    assert_eq!(put_res["enabled"], json!(true));
    assert_eq!(put_res["tesseract_path"], json!(missing_exe));
    assert_eq!(put_res["status"]["available"], json!(false));
    assert_eq!(put_res["status"]["message"], json!(OCR_INSTALL_PROMPT));

    // Update enabled=false, langs=["eng", "deu"], clear tessdata_dir with empty string, then GET round-trip.
    let (st, _, raw) = raw_http(
        port,
        "PUT",
        "/api/settings/ocr",
        &auth,
        &serde_json::to_vec(&json!({ "enabled": false, "tessdata_dir": "", "langs": ["eng", "deu"] })).unwrap(),
    );
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&raw));
    let (st, _, raw) = raw_http(port, "GET", "/api/settings/ocr", &[("X-ReSearch-Token", tok.as_str())], b"");
    assert_eq!(st, 200);
    let fetched: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(fetched["enabled"], json!(false));
    assert_eq!(fetched["tesseract_path"], json!(missing_exe));
    assert!(fetched["tessdata_dir"].is_null());
    assert_eq!(fetched["langs"], json!(["eng", "deu"]));
    assert_eq!(fetched["status"]["available"], json!(false));
    assert_eq!(fetched["status"]["message"], json!(OCR_INSTALL_PROMPT));

    // Settings mutations must never alter existing session receipts.
    let events_after = ledger.read_session("s-ocr-set").unwrap();
    assert_eq!(events_before.len(), events_after.len());
    srv.shutdown();
}

#[test]
fn ocr_ingest_and_page_image_full_and_cropped_png() {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(&tmp.path().join("data"), None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    create_session(port, &tok, "s-ocr-img");
    let missing_exe = tmp.path().join("missing-tess").display().to_string();
    configure_missing_tesseract(port, &tok, &missing_exe);

    let pdf = build_scanned_gray_pdf(16, 12);
    let b64 = encode_base64(&pdf);

    // 1) JSON base64 ingest keeps PageBasis::None honestly when Tesseract is unavailable.
    let json_body = serde_json::to_vec(&json!({
        "file_name": "scanned_paper.pdf",
        "bytes_base64": b64,
        "title": "Scanned Paper",
    }))
    .unwrap();
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions/s-ocr-img/ocr/ingest",
        &[("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")],
        &json_body,
    );
    assert_eq!(st, 200, "JSON OCR ingest failed: {}", String::from_utf8_lossy(&raw));
    let ingested: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(ingested["ocr_available"], json!(false));
    assert_eq!(ingested["ocr_status"], json!("unavailable"));
    assert_eq!(ingested["ocr_message"], json!(OCR_INSTALL_PROMPT));
    assert_eq!(ingested["page_map"]["pages"][0]["basis"], json!("none"));
    assert_eq!(ingested["pages_remaining_none"], json!([1]));
    let source_id = ingested["source"]["source_id"].as_str().expect("source_id").to_string();

    // 2) Raw application/pdf ingest also works identically.
    let (st, _, raw) = raw_http(
        port,
        "POST",
        "/api/sessions/s-ocr-img/ocr/ingest?file_name=raw_scan.pdf",
        &[("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/pdf")],
        &pdf,
    );
    assert_eq!(st, 200, "raw PDF OCR ingest failed: {}", String::from_utf8_lossy(&raw));
    let raw_ingested: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(raw_ingested["ocr_available"], json!(false));
    assert_eq!(raw_ingested["ocr_message"], json!(OCR_INSTALL_PROMPT));
    assert_eq!(raw_ingested["page_map"]["pages"][0]["basis"], json!("none"));

    // 3) GET full page image PNG with security headers and exact 16x12 dimensions.
    let full_path = format!("/api/sessions/s-ocr-img/sources/{source_id}/page-image?page=1");
    let (st, hdrs, png) = raw_http(port, "GET", &full_path, &[("X-ReSearch-Token", tok.as_str())], b"");
    assert_eq!(st, 200);
    assert_eq!(header_val(&hdrs, "content-type"), Some("image/png"));
    assert_eq!(header_val(&hdrs, "cache-control"), Some("no-store"));
    assert_eq!(header_val(&hdrs, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header_val(&hdrs, "content-security-policy"), Some("sandbox"));
    assert_eq!(header_val(&hdrs, "x-research-pdf-page"), Some("1"));
    assert_eq!(header_val(&hdrs, "x-research-image-width"), Some("16"));
    assert_eq!(header_val(&hdrs, "x-research-image-height"), Some("12"));
    assert!(png.starts_with(PNG_SIG), "response must begin with PNG signature");
    assert_eq!(inspect_png_dimensions(&png).unwrap(), (16, 12));

    // 4) GET half-cropped page image PNG (0,0,0.5,0.5 -> 8x6).
    let crop_path = format!("/api/sessions/s-ocr-img/sources/{source_id}/page-image?page=1&crop=0,0,0.5,0.5");
    let (st, hdrs, cropped) = raw_http(port, "GET", &crop_path, &[("X-ReSearch-Token", tok.as_str())], b"");
    assert_eq!(st, 200);
    assert_eq!(header_val(&hdrs, "content-type"), Some("image/png"));
    assert_eq!(header_val(&hdrs, "x-research-image-width"), Some("8"));
    assert_eq!(header_val(&hdrs, "x-research-image-height"), Some("6"));
    assert!(cropped.starts_with(PNG_SIG));
    assert_eq!(inspect_png_dimensions(&cropped).unwrap(), (8, 6));

    // 5) Invalid page / crop parameters fail cleanly with 400 / 404.
    for (bad_query, expected) in [
        ("page=0", 400u16),
        ("page=1&crop=bad", 400),
        ("", 400),
        ("page=9", 404),
    ] {
        let p = format!("/api/sessions/s-ocr-img/sources/{source_id}/page-image?{bad_query}");
        let (st, _, _) = raw_http(port, "GET", &p, &[("X-ReSearch-Token", tok.as_str())], b"");
        assert_eq!(st, expected, "expected {expected} for query `{bad_query}`");
    }
    srv.shutdown();
}

#[test]
fn ocr_async_job_lifecycle_cross_session_isolation_and_cancel() {
    let tmp = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(&tmp.path().join("data"), None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];
    create_session(port, &tok, "s-ocr-job-a");
    create_session(port, &tok, "s-ocr-job-b");
    let missing_exe = tmp.path().join("missing-tess-async").display().to_string();
    configure_missing_tesseract(port, &tok, &missing_exe);

    let pdf = build_scanned_gray_pdf(12, 8);
    let body = serde_json::to_vec(&json!({
        "file_name": "async_scan.pdf",
        "bytes_base64": encode_base64(&pdf),
        "async": true,
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-ocr-job-a/ocr/ingest", &auth, &body);
    assert_eq!(st, 202, "async OCR ingest must return 202: {}", String::from_utf8_lossy(&raw));
    let started: Value = serde_json::from_slice(&raw).unwrap();
    let job_id = started["job_id"].as_str().expect("job_id").to_string();
    assert!(started["snapshot"].is_object());

    // Cross-session isolation: session B cannot read or cancel session A's job.
    let cross_get = format!("/api/sessions/s-ocr-job-b/ocr/jobs/{job_id}");
    assert_eq!(raw_http(port, "GET", &cross_get, &[("X-ReSearch-Token", tok.as_str())], b"").0, 404);
    let cross_cancel = format!("/api/sessions/s-ocr-job-b/ocr/jobs/{job_id}/cancel");
    assert_eq!(raw_http(port, "POST", &cross_cancel, &[("X-ReSearch-Token", tok.as_str())], b"").0, 404);

    // Poll until finished and confirm outcome is pinned into session A.
    let poll_path = format!("/api/sessions/s-ocr-job-a/ocr/jobs/{job_id}");
    let deadline = Instant::now() + Duration::from_secs(5);
    let finished_body = loop {
        let (st, _, raw) = raw_http(port, "GET", &poll_path, &[("X-ReSearch-Token", tok.as_str())], b"");
        assert_eq!(st, 200, "poll failed: {}", String::from_utf8_lossy(&raw));
        let val: Value = serde_json::from_slice(&raw).unwrap();
        if val["finished"] == json!(true) {
            break val;
        }
        assert!(Instant::now() < deadline, "timed out waiting for async OCR job");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(finished_body["error"].is_null(), "unexpected job error: {finished_body}");
    assert_eq!(finished_body["source"]["ocr_available"], json!(false));
    assert_eq!(finished_body["source"]["ocr_message"], json!(OCR_INSTALL_PROMPT));
    assert_eq!(finished_body["source"]["page_map"]["pages"][0]["basis"], json!("none"));
    let pinned_id = finished_body["source"]["source"]["source_id"].as_str().expect("pinned source_id");
    let img_path = format!("/api/sessions/s-ocr-job-a/sources/{pinned_id}/page-image?page=1");
    let (st, _, png) = raw_http(port, "GET", &img_path, &[("X-ReSearch-Token", tok.as_str())], b"");
    assert_eq!(st, 200);
    assert_eq!(inspect_png_dimensions(&png).unwrap(), (12, 8));

    // Start a second async job and cancel it cleanly.
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-ocr-job-a/ocr/ingest", &auth, &body);
    assert_eq!(st, 202);
    let job_id_2 = serde_json::from_slice::<Value>(&raw).unwrap()["job_id"].as_str().unwrap().to_string();
    let cancel_path = format!("/api/sessions/s-ocr-job-a/ocr/jobs/{job_id_2}/cancel");
    let (st, _, raw) = raw_http(port, "POST", &cancel_path, &[("X-ReSearch-Token", tok.as_str())], b"");
    assert_eq!(st, 200, "cancel failed: {}", String::from_utf8_lossy(&raw));
    let cancelled: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(cancelled["job_id"], json!(job_id_2));
    assert_eq!(cancelled["cancelled"], json!(true));
    assert!(cancelled["snapshot"].is_object());
    srv.shutdown();
}
