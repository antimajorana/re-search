//! Integration & security tests for F13c `/api/sources/export/bibtex` and
//! `/api/sources/send/kandinsky` routes, live HTTP auth + Origin enforcement, F15 BibTeX reuse,
//! read-only source-root refusal, and argv-only stub execution.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::routes_sources_ext::{handle_send_kandinsky_with_bin, handle_sources_ext};
use desktop_server::sse::SseHub;
use desktop_server::start_desktop_server;
use desktop_server::state::DesktopState;
use research_tools::library::{Library, SourceKind};
use research_tools::source_sync::{register_source, sync_source, SyncOptions};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

fn make_state(tmp: &TempDir) -> Arc<DesktopState> {
    let cfg = test_cfg(tmp.path(), None, None, None);
    let auth = AuthGuard::new(18_429).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    Arc::new(DesktopState::new(cfg, auth, sse, chatgpt))
}

fn empty_q() -> HashMap<String, String> {
    HashMap::new()
}

fn seed_bibtex_source(data_dir: &std::path::Path, base: &std::path::Path) -> std::path::PathBuf {
    let bib_dir = base.join("registered_bib");
    fs::create_dir_all(&bib_dir).unwrap();
    let bib_path = bib_dir.join("refs.bib");
    fs::write(
        &bib_path,
        "@article{userpin2024,\n\
           author = {Synth, Ada and Route, Ben},\n\
           title = {Synthetic 100\\% Field \\& Gauge Study},\n\
           journal = {Synthetic Physics},\n\
           year = {2024},\n\
           eprint = {2407.01234v2},\n\
           archivePrefix = {arXiv},\n\
           doi = {10.9999/route.2024.1}\n\
         }\n",
    )
    .unwrap();

    let lib = Library::open(data_dir).unwrap();
    let src = register_source(&lib, SourceKind::Bibtex, &bib_path, json!({}), None).unwrap();
    let rep = sync_source(
        &lib,
        &src.id,
        &SyncOptions {
            dry_run: false,
            hash_files: false,
        },
    )
    .unwrap();
    assert_eq!(rep.added, 1);
    bib_dir
}

#[test]
fn export_bibtex_route_preserves_citekey_writes_atomically_and_refuses_source_root() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    let bib_dir = seed_bibtex_source(&state.cfg.data_dir, tmp.path());

    // 1. Preview export (no output_path) returns BibTeX with user's citekey.
    let preview_body = serde_json::to_vec(&json!({
        "paper_keys": ["arxiv:2407.01234"]
    }))
    .unwrap();
    let (code, preview) =
        handle_sources_ext(&state, "POST", &["export", "bibtex"], &empty_q(), &preview_body)
            .unwrap();
    assert_eq!(code, 200);
    assert_eq!(preview["written"], Value::Bool(false));
    assert_eq!(preview["citekeys"], json!(["userpin2024"]));
    let bib_str = preview["bibtex"].as_str().unwrap();
    assert!(bib_str.contains("@article{userpin2024,"), "{bib_str}");
    assert!(bib_str.contains("eprint = {2407.01234v2}"), "{bib_str}");

    // 2. User-chosen separate output path writes atomically.
    let export_dir = tmp.path().join("exports");
    fs::create_dir_all(&export_dir).unwrap();
    let dest = export_dir.join("out.bib");
    let write_body = serde_json::to_vec(&json!({
        "paper_keys": ["arxiv:2407.01234"],
        "output_path": dest.to_string_lossy(),
    }))
    .unwrap();
    let (code, res) =
        handle_sources_ext(&state, "POST", &["export", "bibtex"], &empty_q(), &write_body)
            .unwrap();
    assert_eq!(code, 200);
    assert_eq!(res["written"], Value::Bool(true));
    assert_eq!(fs::read_to_string(&dest).unwrap(), bib_str);

    // 3. Refuses writing inside the registered read-only BibTeX source directory.
    let bad_dest = serde_json::to_vec(&json!({
        "paper_keys": ["arxiv:2407.01234"],
        "output_path": bib_dir.join("overwrite.bib").to_string_lossy(),
    }))
    .unwrap();
    let (st, msg) =
        handle_sources_ext(&state, "POST", &["export", "bibtex"], &empty_q(), &bad_dest)
            .unwrap_err();
    assert_eq!(st, 400);
    assert!(msg.contains("registered read-only paper source"), "{msg}");

    // 4. GET is rejected with 405 Method Not Allowed.
    let (st_get, _) =
        handle_sources_ext(&state, "GET", &["export", "bibtex"], &empty_q(), b"").unwrap_err();
    assert_eq!(st_get, 405);
}

#[test]
fn send_to_kandinsky_route_validates_rejects_injection_and_invokes_stub_argv_only() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    let _bib_dir = seed_bibtex_source(&state.cfg.data_dir, tmp.path());

    // 1. Deep-link mode from library paper_key.
    let dl_body = serde_json::to_vec(&json!({
        "paper_key": "arxiv:2407.01234",
        "mode": "deep_link",
        "user_initiated": true,
    }))
    .unwrap();
    let (code, dl) =
        handle_sources_ext(&state, "POST", &["send", "kandinsky"], &empty_q(), &dl_body).unwrap();
    assert_eq!(code, 200);
    assert_eq!(dl["target"], json!("arXiv:2407.01234v2"));
    assert_eq!(dl["argv"], json!(["capture-paper", "arXiv:2407.01234v2"]));
    let link = dl["deep_link"].as_str().unwrap();
    assert!(
        link.starts_with("kandinsky://capture-paper?target=arXiv%3A2407.01234v2"),
        "{link}"
    );

    // 2. Rejects non-user-initiated calls and injection payloads.
    let auto_body = serde_json::to_vec(&json!({
        "target": "arXiv:2407.01234",
        "user_initiated": false,
    }))
    .unwrap();
    assert_eq!(
        handle_sources_ext(&state, "POST", &["send", "kandinsky"], &empty_q(), &auto_body)
            .unwrap_err()
            .0,
        400
    );

    for bad_target in [
        "--config=/etc/passwd",
        "-e",
        "arXiv:2407.01234;echo pwn",
        "arXiv:2407.01234|sh",
        "arXiv:$(id)",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ] {
        let payload = serde_json::to_vec(&json!({ "target": bad_target })).unwrap();
        let (st, _) =
            handle_sources_ext(&state, "POST", &["send", "kandinsky"], &empty_q(), &payload)
                .unwrap_err();
        assert_eq!(st, 400, "must reject `{bad_target}`");
    }

    // 3. CLI mode with self-compiled cross-platform stub binary (never real Kandinsky or shell).
    let stub_src = tmp.path().join("stub_kandinsky.rs");
    let stub_bin = tmp
        .path()
        .join(format!("stub_kandinsky{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &stub_src,
        "fn main() {\n\
           let args: Vec<String> = std::env::args().skip(1).collect();\n\
           println!(\"OK:{}\", args.join(\"|\"));\n\
         }\n",
    )
    .unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let comp = Command::new(rustc)
        .args(["--edition", "2021"])
        .arg(&stub_src)
        .arg("-o")
        .arg(&stub_bin)
        .output()
        .unwrap();
    assert!(comp.status.success());

    let cli_body = serde_json::to_vec(&json!({
        "paper_key": "arxiv:2407.01234",
        "mode": "cli",
        "user_initiated": true,
    }))
    .unwrap();
    let (code, res) = handle_send_kandinsky_with_bin(&state, &cli_body, &stub_bin).unwrap();
    assert_eq!(code, 200);
    assert_eq!(res["ok"], Value::Bool(true));
    assert_eq!(
        res["stdout"].as_str().unwrap().trim(),
        "OK:capture-paper|arXiv:2407.01234v2"
    );
}

#[test]
fn sources_ext_routes_enforced_behind_live_http_auth_and_origin_checks() {
    let dir = TempDir::new().unwrap();
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.info.port;
    let tok = srv.info.token.clone();
    let tok_hdr = ("X-ReSearch-Token", tok.as_str());
    let json_hdr = ("Content-Type", "application/json");

    for path in ["/api/sources/export/bibtex", "/api/sources/send/kandinsky"] {
        // Missing token -> 401.
        let (st_unauth, _, _) = raw_http(port, "POST", path, &[json_hdr], b"{}");
        assert_eq!(st_unauth, 401, "expected 401 without token on {path}");

        // Cross-origin POST -> 403.
        let (st_origin, _, _) = raw_http(
            port,
            "POST",
            path,
            &[tok_hdr, json_hdr, ("Origin", "https://evil.example")],
            b"{}",
        );
        assert_eq!(st_origin, 403, "expected 403 on cross-origin POST to {path}");
    }

    // Valid token + valid target on live HTTP returns 200.
    let (st_ok, _, body_ok) = raw_http(
        port,
        "POST",
        "/api/sources/send/kandinsky",
        &[tok_hdr, json_hdr],
        br#"{"target":"arXiv:2407.01234","mode":"deep_link","user_initiated":true}"#,
    );
    assert_eq!(st_ok, 200, "unexpected live response: {}", String::from_utf8_lossy(&body_ok));
    let raw_str = String::from_utf8_lossy(&body_ok);
    assert!(raw_str.contains("kandinsky://capture-paper?target=arXiv%3A2407.01234"), "{raw_str}");

    // Static asset /assets/library_send_back.js is served cleanly.
    let (st_asset, _, _) = raw_http(port, "GET", "/assets/library_send_back.js", &[], b"");
    assert_eq!(st_asset, 200);

    srv.shutdown();
}
