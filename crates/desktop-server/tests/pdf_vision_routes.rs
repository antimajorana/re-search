//! Integration tests for `POST /api/sessions/{id}/sources/{source}/transcribe-equations` (F14c).
//!
//! Verifies header-token auth, off-by-default opt-in refusal (403), MIME/magic-byte validation,
//! fail-closed non-vision model refusal (400), F9 `run_budget` cap enforcement (429),
//! `context.supplied` `page_ranges` with zero `SentLines` credit, `pdf.transcription` labelled
//! `"model-transcribed"` with zero `evidence.quote` minting, and the decoupled `PageCropProvider`.

mod common;

use agent::vision::encode_base64;
use common::{launch_cookie, raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::routes_pdf_vision::{
    load_pdf_vision_settings, save_pdf_vision_settings, PdfVisionSettings,
};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::lines_sent::{lines_sent, CONTEXT_SUPPLIED_EVENT};
use research_tools::pdf_vision::{
    verify_transcription_against_text_layer, FormulaTranscription, TranscriptionGateError,
    EVENT_PDF_TRANSCRIPTION, MODEL_TRANSCRIBED_LABEL,
};
use research_tools::quote_core::{QuoteRejection, EVIDENCE_QUOTE_EVENT};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::TempDir;

fn sample_png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    bytes
}

fn sample_jpeg() -> Vec<u8> {
    vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0xFF, 0xD9]
}

fn seed_pdf_session(data_dir: &Path, sid: &str, source_id: &str) -> String {
    let ledger = Ledger::open(data_dir).unwrap();
    let raw_pdf = b"%PDF-1.4\n% synthetic test pdf\n%%EOF\n";
    let raw_blob = ledger.put_blob(raw_pdf).unwrap();

    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let primary_path = format!("{source_id}/paper.pdf.extracted.txt");
    let text = "Synthetic Header\nE = m c^2\n\x0c\n";
    let dst = sandbox
        .resolve_for_write(SandboxRoot::Sources, sid, &primary_path)
        .unwrap();
    std::fs::create_dir_all(dst.target_path.parent().unwrap()).unwrap();
    std::fs::write(&dst.target_path, text.as_bytes()).unwrap();
    let file_sha = ledger::sha256_hex(text.as_bytes());

    let metrics = json!({
        "char_count": 26, "alpha_count": 18, "word_count": 5,
        "image_coverage_pct": 0, "has_invisible_text": false,
        "width_pt": 612, "height_pt": 792
    });
    ledger
        .append_with_hash(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": source_id,
                "origin": "upload:paper.pdf",
                "title": "Synthetic Vision Paper",
                "format": "pdf",
                "status": "ok",
                "bytes": raw_pdf.len(),
                "raw_blob": raw_blob,
                "primary_path": primary_path,
                "files": [primary_path],
                "files_sha256": { &primary_path: &file_sha },
                "page_count": 2,
                "pdf_page_lines": [[1, 1, 2]],
                "pdf_extractor": "pdf-extract-layout/0.12.1",
                "pdf_pages": [
                    { "page": 1, "basis": "text_layer", "first_line": 1, "last_line": 2, "metrics": metrics },
                    { "page": 2, "basis": "none", "metrics": metrics }
                ]
            }),
        )
        .unwrap();
    raw_blob
}

fn write_fake_openai_key(data_dir: &Path) {
    let path = data_dir.join("providers.json");
    let body = json!({ "openai": "sk-fake-test-key-000000" });
    std::fs::write(path, serde_json::to_vec_pretty(&body).unwrap()).unwrap();
}

fn openai_chat_reply(text: &str) -> MockReply {
    let body = serde_json::to_vec(&json!({
        "id": "chatcmpl-test-1",
        "object": "chat.completion",
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 50, "completion_tokens": 12 }
    }))
    .unwrap();
    MockReply {
        status: 200,
        content_type: "application/json",
        body,
    }
}

fn create_session(port: u16, token: &str) -> String {
    let (st, _, body) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[("X-ReSearch-Token", token), ("Content-Type", "application/json")],
        br#"{"title":"Vision Route Test"}"#,
    );
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["id"].as_str().unwrap().to_string()
}

#[test]
fn route_enforces_auth_off_by_default_validation_fail_closed_and_budget() {
    let dir = TempDir::new().unwrap();
    write_fake_openai_key(dir.path());
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let cookie = launch_cookie(port, &token);
    let sid = create_session(port, &token);
    seed_pdf_session(dir.path(), &sid, "paper1");
    let path = format!("/api/sessions/{sid}/sources/paper1/transcribe-equations");
    let png_b64 = encode_base64(&sample_png());

    // Default settings are off.
    assert!(!load_pdf_vision_settings(dir.path()).equation_transcription_enabled);

    // 1. Unauthenticated and cookie-only requests are rejected with 401.
    let (st, _, _) = raw_http(port, "POST", &path, &[("Content-Type", "application/json")], b"{}");
    assert_eq!(st, 401);
    let (st, _, _) = raw_http(
        port,
        "POST",
        &path,
        &[("Cookie", &cookie), ("Content-Type", "application/json")],
        b"{}",
    );
    assert_eq!(st, 401);

    let auth = [("X-ReSearch-Token", token.as_str()), ("Content-Type", "application/json")];

    // 2. Off by default: missing `opt_in` refused with 403.
    let no_opt_in = serde_json::to_vec(&json!({
        "page": 1,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "gpt-4o"
    }))
    .unwrap();
    let (st, _, body) = raw_http(port, "POST", &path, &auth, &no_opt_in);
    assert_eq!(st, 403, "body={}", String::from_utf8_lossy(&body));

    // 3. Explicit `opt_in: false` overrides persisted enabled setting.
    save_pdf_vision_settings(dir.path(), &PdfVisionSettings { equation_transcription_enabled: true }).unwrap();
    let opt_out = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": false,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "gpt-4o"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &opt_out);
    assert_eq!(st, 403);
    save_pdf_vision_settings(dir.path(), &PdfVisionSettings { equation_transcription_enabled: false }).unwrap();

    // 4. Invalid MIME and spoofed magic bytes rejected with 400.
    let svg_req = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/svg+xml",
        "image_base64": encode_base64(b"<svg/>"),
        "provider": "openai",
        "model": "gpt-4o"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &svg_req);
    assert_eq!(st, 400);

    let spoof_req = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": encode_base64(&sample_jpeg()),
        "provider": "openai",
        "model": "gpt-4o"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &spoof_req);
    assert_eq!(st, 400);

    // 5. Non-vision model fails closed with 400 (never silently drops the image).
    let non_vision = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "o3-mini"
    }))
    .unwrap();
    let (st, _, body) = raw_http(port, "POST", &path, &auth, &non_vision);
    assert_eq!(st, 400, "body={}", String::from_utf8_lossy(&body));
    assert!(String::from_utf8_lossy(&body).contains("not vision-capable"));

    // 6. Out-of-bounds page rejected with 400.
    let oob_page = serde_json::to_vec(&json!({
        "page": 9,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "gpt-4o"
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &oob_page);
    assert_eq!(st, 400);

    // 7. Exhausted F9 run_budget refused with 429 before any ledger mutation.
    let ledger = Ledger::open(dir.path()).unwrap();
    ledger
        .append_with_hash(&sid, "model.request", 1, &json!({ "turn": 1, "run_id": "run-cap" }))
        .unwrap();
    let at_cap = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "gpt-4o",
        "run_id": "run-cap",
        "budget_cap": 1
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &at_cap);
    assert_eq!(st, 429);

    // Zero context.supplied / pdf.transcription / evidence.quote events written across all refusals.
    let evs = ledger.read_session(&sid).unwrap();
    assert!(!evs.iter().any(|e| e.kind == CONTEXT_SUPPLIED_EVENT));
    assert!(!evs.iter().any(|e| e.kind == EVENT_PDF_TRANSCRIPTION));
    assert!(!evs.iter().any(|e| e.kind == EVIDENCE_QUOTE_EVENT));
}

#[test]
fn opt_in_transcribe_records_page_ranges_and_model_transcribed_honestly() {
    let dir = TempDir::new().unwrap();
    write_fake_openai_key(dir.path());
    let mock = MockHttp::start(vec![
        openai_chat_reply("E = m c^2"),
        openai_chat_reply("H = p^2 / (2 m)"),
    ]);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(mock.base_url.clone()), None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let sid = create_session(port, &token);
    let raw_blob = seed_pdf_session(dir.path(), &sid, "paper1");
    let path = format!("/api/sessions/{sid}/sources/paper1/transcribe-equations");
    let auth = [("X-ReSearch-Token", token.as_str()), ("Content-Type", "application/json")];
    let png_b64 = encode_base64(&sample_png());

    // Call 1: explicit per-request opt_in on page 1 (text_layer matches "E = m c^2").
    let req1 = serde_json::to_vec(&json!({
        "page": 1,
        "crop": [12, 24, 320, 90],
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "gpt-4o",
        "run_id": "run-vis",
        "budget_cap": 2
    }))
    .unwrap();
    let (st, _, body1) = raw_http(port, "POST", &path, &auth, &req1);
    assert_eq!(st, 200, "body={}", String::from_utf8_lossy(&body1));
    let v1: Value = serde_json::from_slice(&body1).unwrap();
    assert_eq!(v1["label"], MODEL_TRANSCRIBED_LABEL);
    assert_eq!(v1["quotable"], false);
    assert_eq!(v1["requires_text_layer_agreement"], true);
    assert_eq!(v1["transcription"]["label"], MODEL_TRANSCRIBED_LABEL);
    assert_eq!(v1["transcription"]["latex_draft"], "E = m c^2");
    assert_eq!(v1["transcription"]["agreed_with_text_layer"], true);
    assert_eq!(v1["transcription"]["raw_blob_sha256"], raw_blob);
    assert_eq!(v1["budget"]["cap"], 2);
    assert_eq!(v1["budget"]["used"], 1);
    assert_eq!(v1["budget"]["remaining"], 1);

    // Call 2: persisted setting opt-in on image-only page 2 (basis = "none").
    save_pdf_vision_settings(dir.path(), &PdfVisionSettings { equation_transcription_enabled: true }).unwrap();
    let req2 = serde_json::to_vec(&json!({
        "page": 2,
        "image_media_type": "image/png",
        "image_base64": png_b64,
        "provider": "openai",
        "model": "gpt-4o",
        "run_id": "run-vis",
        "budget_cap": 2
    }))
    .unwrap();
    let (st, _, body2) = raw_http(port, "POST", &path, &auth, &req2);
    assert_eq!(st, 200, "body={}", String::from_utf8_lossy(&body2));
    let v2: Value = serde_json::from_slice(&body2).unwrap();
    assert_eq!(v2["transcription"]["label"], MODEL_TRANSCRIBED_LABEL);
    assert_eq!(v2["transcription"]["agreed_with_text_layer"], false);
    assert_eq!(v2["budget"]["used"], 2);
    assert_eq!(v2["budget"]["remaining"], 0);

    // Call 3: same run_id + budget_cap=2 now fails closed with 429.
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &req2);
    assert_eq!(st, 429);

    // Inspect ledger invariants.
    let ledger = Ledger::open(dir.path()).unwrap();
    let (_, events) = ledger.read_verified_session(&sid).unwrap();
    assert!(!events.iter().any(|e| e.kind == EVIDENCE_QUOTE_EVENT));

    let supplied: Vec<_> = events.iter().filter(|e| e.kind == CONTEXT_SUPPLIED_EVENT).collect();
    assert_eq!(supplied.len(), 2);
    assert_eq!(supplied[0].payload["ranges"], json!([]));
    assert_eq!(supplied[0].payload["page_ranges"][0]["start_page"], 1);
    assert_eq!(supplied[0].payload["page_ranges"][0]["crop"], json!([12, 24, 320, 90]));

    // `page_ranges` MUST grant zero credit to `SentLines::covers`.
    assert!(lines_sent(&events, Some("run-vis")).is_empty());

    let transcriptions: Vec<FormulaTranscription> = events
        .iter()
        .filter(|e| e.kind == EVENT_PDF_TRANSCRIPTION)
        .map(|e| serde_json::from_value(e.payload.clone()).unwrap())
        .collect();
    assert_eq!(transcriptions.len(), 2);
    assert_eq!(transcriptions[0].label, MODEL_TRANSCRIBED_LABEL);
    assert_eq!(transcriptions[1].label, MODEL_TRANSCRIBED_LABEL);

    // Even for page 1 where `agreed_with_text_layer == true`, text-layer gate refuses without
    // text lines supplied in `context.supplied` `ranges`!
    let sandbox = WorkspaceSandbox::new(dir.path(), None);
    assert!(matches!(
        verify_transcription_against_text_layer(
            &sandbox, &sid, &events, Some(&ledger), Some("run-vis"), "pdf_vision", &transcriptions[0]
        ),
        Err(TranscriptionGateError::QuoteRejected(QuoteRejection::LinesNotSent { .. }))
    ));

    mock.finish();
}

#[test]
fn decoupled_page_crop_provider_seam_works_without_inline_image() {
    use desktop_server::auth::AuthGuard;
    use desktop_server::chatgpt::ChatGptHub;
    use desktop_server::routes_pdf_vision::{handle_transcribe_with_provider, PageCropProvider};
    use desktop_server::sse::SseHub;
    use desktop_server::state::DesktopState;
    use std::sync::Arc;

    struct SyntheticCropProvider;
    impl PageCropProvider for SyntheticCropProvider {
        fn render_crop(
            &self,
            raw_pdf_bytes: &[u8],
            page: usize,
            _crop: Option<[u32; 4]>,
        ) -> Result<(String, Vec<u8>), String> {
            assert!(raw_pdf_bytes.starts_with(b"%PDF-"));
            assert_eq!(page, 1);
            Ok(("image/png".to_string(), sample_png()))
        }
    }

    let dir = TempDir::new().unwrap();
    write_fake_openai_key(dir.path());
    let mock = MockHttp::start(vec![openai_chat_reply("E = m c^2")]);
    let cfg = test_cfg(dir.path(), None, Some(mock.base_url.clone()), None);
    let auth = AuthGuard::new(0).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    let state = Arc::new(DesktopState::new(cfg, auth, sse, chatgpt));

    let sid = "s-crop-provider";
    let ledger = Ledger::open(dir.path()).unwrap();
    ledger.append_with_hash(sid, "session.meta", 1, &json!({ "id": sid, "title": "Crop" })).unwrap();
    seed_pdf_session(dir.path(), sid, "paper1");

    // Request omits image_media_type/image_base64; PageCropProvider supplies the PNG bytes.
    let body = serde_json::to_vec(&json!({
        "page": 1,
        "crop": [0, 0, 128, 64],
        "opt_in": true,
        "provider": "openai",
        "model": "gpt-4o"
    }))
    .unwrap();
    let (st, val) =
        handle_transcribe_with_provider(&state, sid, "paper1", &body, Some(&SyntheticCropProvider))
            .unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["label"], MODEL_TRANSCRIBED_LABEL);
    assert_eq!(val["transcription"]["latex_draft"], "E = m c^2");
    assert_eq!(val["transcription"]["agreed_with_text_layer"], true);
    assert_eq!(val["crop_image_media_type"], "image/png");
    assert_eq!(val["crop_image_base64"], encode_base64(&sample_png()));

    mock.finish();
}

#[test]
fn route_fails_closed_on_model_refusal_truncation_and_budget_cap_bypass() {
    use desktop_server::routes_pdf_vision::DEFAULT_VISION_BUDGET_CAP;

    let refusal_reply = MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&json!({
            "id": "chatcmpl-refuse",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "I cannot process this image." },
                "finish_reason": "content_filter"
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 5 }
        }))
        .unwrap(),
    };
    let truncated_reply = MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&json!({
            "id": "chatcmpl-trunc",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "E = m c^" },
                "finish_reason": "length"
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 1024 }
        }))
        .unwrap(),
    };

    let dir = TempDir::new().unwrap();
    write_fake_openai_key(dir.path());
    let mock = MockHttp::start(vec![refusal_reply, truncated_reply]);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(mock.base_url.clone()), None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let sid = create_session(port, &token);
    seed_pdf_session(dir.path(), &sid, "paper1");
    let path = format!("/api/sessions/{sid}/sources/paper1/transcribe-equations");
    let auth = [("X-ReSearch-Token", token.as_str()), ("Content-Type", "application/json")];
    let body = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": encode_base64(&sample_png()),
        "provider": "openai",
        "model": "gpt-4o",
        "budget_cap": 99999
    }))
    .unwrap();

    // 1. Model refusal -> 422, NEVER persisted as pdf.transcription.
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &body);
    assert_eq!(st, 422);

    // 2. Truncated MaxTokens -> 502, NEVER persisted as pdf.transcription.
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &body);
    assert_eq!(st, 502);

    let ledger = Ledger::open(dir.path()).unwrap();
    let evs = ledger.read_session(&sid).unwrap();
    assert!(!evs.iter().any(|e| e.kind == EVENT_PDF_TRANSCRIPTION));

    // 3. Session-wide main-loop model.request calls (without run_id) reaching DEFAULT_VISION_BUDGET_CAP
    //    cannot be bypassed by supplying a fresh run_id or inflated budget_cap=99999.
    for turn in 1..=DEFAULT_VISION_BUDGET_CAP {
        ledger.append_with_hash(&sid, "model.request", 1, &json!({ "turn": turn })).unwrap();
    }
    let bypass_attempt = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": encode_base64(&sample_png()),
        "provider": "openai",
        "model": "gpt-4o",
        "run_id": "fresh-bypass-run",
        "budget_cap": 99999
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", &path, &auth, &bypass_attempt);
    assert_eq!(st, 429);

    mock.finish();
}

#[test]
fn settings_refuse_symlinks_and_budget_remaining_clamps_to_session_cap() {
    use desktop_server::routes_pdf_vision::{
        load_pdf_vision_settings, save_pdf_vision_settings, PdfVisionSettings,
        DEFAULT_VISION_BUDGET_CAP, PDF_VISION_SETTINGS_FILE,
    };

    // 1. Settings sidecar refuses symlinks on load and save (Unix).
    #[cfg(unix)]
    {
        let dir = TempDir::new().unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, br#"{"equation_transcription_enabled":true}"#).unwrap();
        let link = dir.path().join(PDF_VISION_SETTINGS_FILE);
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(
            !load_pdf_vision_settings(dir.path()).equation_transcription_enabled,
            "symlinked settings must fail closed to default false"
        );
        let err = save_pdf_vision_settings(
            dir.path(),
            &PdfVisionSettings { equation_transcription_enabled: true },
        )
        .unwrap_err();
        assert!(err.contains("symlink"), "unexpected save error: {err}");
    }

    // 2. When session has 1 call left (63/64) and run_id requests cap 10, response clamps
    //    budget.remaining to 0 (matching session_budget.remaining) and next call returns 429.
    let ok_reply = MockReply {
        status: 200,
        content_type: "application/json",
        body: serde_json::to_vec(&json!({
            "id": "chatcmpl-last",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "E = m c^2" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        }))
        .unwrap(),
    };
    let dir = TempDir::new().unwrap();
    write_fake_openai_key(dir.path());
    let mock = MockHttp::start(vec![ok_reply]);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(mock.base_url.clone()), None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let sid = create_session(port, &token);
    seed_pdf_session(dir.path(), &sid, "paper1");

    let ledger = Ledger::open(dir.path()).unwrap();
    for turn in 1..DEFAULT_VISION_BUDGET_CAP {
        ledger.append_with_hash(&sid, "model.request", 1, &json!({ "turn": turn })).unwrap();
    }

    let path = format!("/api/sessions/{sid}/sources/paper1/transcribe-equations");
    let auth = [("X-ReSearch-Token", token.as_str()), ("Content-Type", "application/json")];
    let body = serde_json::to_vec(&json!({
        "page": 1,
        "opt_in": true,
        "image_media_type": "image/png",
        "image_base64": encode_base64(&sample_png()),
        "provider": "openai",
        "model": "gpt-4o",
        "run_id": "sub-run-1",
        "budget_cap": 10
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &path, &auth, &body);
    assert_eq!(st, 200);
    let parsed: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(parsed["budget"]["remaining"], 0);
    assert_eq!(parsed["session_budget"]["remaining"], 0);

    let (st2, _, _) = raw_http(port, "POST", &path, &auth, &body);
    assert_eq!(st2, 429);
    mock.finish();
}
