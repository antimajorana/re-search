//! Integration tests for `/api/settings/literature` (`lit_settings.rs`, package X19 Part 1).
//!
//! Verifies `AuthGuard::authorize_api` enforcement, persistence to `<data_dir>/lit_settings.json`,
//! key masking on GET/PUT, clearing, partial updates, `OPENALEX_API_KEY` env fallback,
//! polite-pool opt-in validation, zero leakage into ledger events or error messages, and
//! `0600` Unix permissions on the sidecar file.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::lit_settings::{
    load_settings, resolve_provider_config, settings_path, LitSettings, OPENALEX_API_KEY_ENV,
};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|e| panic!("invalid JSON `{}`: {e}", String::from_utf8_lossy(bytes)))
}

#[test]
fn auth_guard_protects_get_and_put_literature_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let cookie = launch_cookie(port, &token);

    // Unauthenticated GET & PUT must be rejected with 401.
    let (st_no_auth_get, _, _) = raw_http(port, "GET", "/api/settings/literature", &[], b"");
    assert_eq!(st_no_auth_get, 401);
    let (st_no_auth_put, _, _) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &[("Content-Type", "application/json")],
        br#"{"openalex_polite_pool":true}"#,
    );
    assert_eq!(st_no_auth_put, 401);

    // Wrong token must be rejected with 401.
    let (st_bad_tok, _, _) = raw_http(
        port,
        "GET",
        "/api/settings/literature",
        &[("X-ReSearch-Token", "wrong-token")],
        b"",
    );
    assert_eq!(st_bad_tok, 401);

    // Stream cookie alone is NEVER sufficient for `/api/settings/literature`.
    let (st_cookie_get, _, _) = raw_http(
        port,
        "GET",
        "/api/settings/literature",
        &[("Cookie", &cookie)],
        b"",
    );
    assert_eq!(st_cookie_get, 401);
    let (st_cookie_put, _, _) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &[("Cookie", &cookie), ("Content-Type", "application/json")],
        br#"{"openalex_polite_pool":true}"#,
    );
    assert_eq!(st_cookie_put, 401);

    // Cross-origin Origin header must be rejected even with valid token.
    let (st_bad_origin, _, _) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &[
            ("X-ReSearch-Token", &token),
            ("Origin", "https://evil.example"),
            ("Content-Type", "application/json"),
        ],
        br#"{"openalex_polite_pool":true}"#,
    );
    assert_eq!(st_bad_origin, 403);

    // Valid header token succeeds.
    let (st_ok, _, body) = raw_http(
        port,
        "GET",
        "/api/settings/literature",
        &[("X-ReSearch-Token", &token)],
        b"",
    );
    assert_eq!(st_ok, 200);
    let v = parse_json(&body);
    assert_eq!(v["openalex_api_key"], Value::Null);
    assert_eq!(v["openalex_api_key_configured"], false);
    assert_eq!(v["openalex_api_key_source"], "none");
    assert_eq!(v["openalex_polite_pool"], false);
    assert_eq!(v["openalex_mailto"], Value::Null);
}

#[test]
fn put_get_mask_clear_persist_permissions_and_never_touch_ledger() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let hdrs = [
        ("X-ReSearch-Token", token.as_str()),
        ("Content-Type", "application/json"),
    ];
    let fake_secret = "fake-openalex-user-key-7419";

    // Save key + polite pool opt-in + mailto.
    let put_body = serde_json::to_vec(&json!({
        "openalex_api_key": format!("  {fake_secret}  "),
        "openalex_polite_pool": true,
        "openalex_mailto": "researcher@example.org",
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "PUT", "/api/settings/literature", &hdrs, &put_body);
    assert_eq!(st, 200, "PUT failed: {}", String::from_utf8_lossy(&raw));
    let raw_str = String::from_utf8_lossy(&raw);
    assert!(
        !raw_str.contains(fake_secret),
        "PUT response leaked plaintext key: {raw_str}"
    );
    let v = parse_json(&raw);
    assert_eq!(v["openalex_api_key"], "****7419");
    assert_eq!(v["openalex_api_key_configured"], true);
    assert_eq!(v["openalex_api_key_source"], "saved");
    assert_eq!(v["openalex_polite_pool"], true);
    assert_eq!(v["openalex_mailto"], "researcher@example.org");

    // GET returns masked key only and matches persisted sidecar file.
    let (st_g, _, raw_g) = raw_http(
        port,
        "GET",
        "/api/settings/literature",
        &[("X-ReSearch-Token", token.as_str())],
        b"",
    );
    assert_eq!(st_g, 200);
    let raw_g_str = String::from_utf8_lossy(&raw_g);
    assert!(
        !raw_g_str.contains(fake_secret),
        "GET response leaked plaintext key: {raw_g_str}"
    );
    assert_eq!(parse_json(&raw_g), v);

    // Sidecar file exists at `<data_dir>/lit_settings.json` with mode 0600 on Unix.
    let path = settings_path(tmp.path());
    assert!(path.exists(), "expected sidecar at {}", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600 permissions, got {mode:o}");
    }
    let loaded = load_settings(tmp.path());
    assert_eq!(loaded.openalex_api_key.as_deref(), Some(fake_secret));
    assert!(loaded.openalex_polite_pool);
    assert_eq!(loaded.openalex_mailto.as_deref(), Some("researcher@example.org"));

    // Partial update (only polite pool) preserves existing saved key.
    let (st_p, _, raw_p) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &hdrs,
        br#"{"openalex_polite_pool":false}"#,
    );
    assert_eq!(st_p, 200);
    let vp = parse_json(&raw_p);
    assert_eq!(vp["openalex_api_key"], "****7419");
    assert_eq!(vp["openalex_api_key_configured"], true);
    assert_eq!(vp["openalex_polite_pool"], false);
    assert_eq!(vp["openalex_mailto"], "researcher@example.org");

    // Clearing via null removes the saved key and clears mailto via empty string.
    let (st_c, _, raw_c) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &hdrs,
        br#"{"openalex_api_key":null,"openalex_mailto":""}"#,
    );
    assert_eq!(st_c, 200);
    let vc = parse_json(&raw_c);
    assert_eq!(vc["openalex_api_key"], Value::Null);
    assert_eq!(vc["openalex_api_key_configured"], false);
    assert_eq!(vc["openalex_api_key_source"], "none");
    assert_eq!(vc["openalex_mailto"], Value::Null);

    // Verify zero ledger sessions or events were created by settings operations.
    let ledger = Ledger::open(tmp.path()).unwrap();
    assert!(
        ledger.sessions().unwrap().is_empty(),
        "literature settings must never write ledger sessions/events"
    );
    let dbg_settings = LitSettings {
        openalex_api_key: Some(fake_secret.into()),
        openalex_polite_pool: false,
        openalex_mailto: None,
    };
    assert!(
        !format!("{dbg_settings:?}").contains(fake_secret),
        "LitSettings Debug must redact key"
    );
}

#[test]
fn env_fallback_and_saved_override_via_hermetic_connection_env() {
    let tmp = tempfile::tempdir().unwrap();
    let env_secret = "fake-env-openalex-key-3051";
    let saved_secret = "fake-saved-openalex-key-8842";

    let mut cfg = test_cfg(tmp.path(), None, None, None);
    let mut env_map = BTreeMap::new();
    env_map.insert(OPENALEX_API_KEY_ENV.to_string(), env_secret.to_string());
    cfg.connection_env = Some(env_map);

    let srv = start_desktop_server(cfg.clone()).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let hdrs = [
        ("X-ReSearch-Token", token.as_str()),
        ("Content-Type", "application/json"),
    ];

    // With no saved key, GET reports masked env key with source "env".
    let (st, _, raw) = raw_http(
        port,
        "GET",
        "/api/settings/literature",
        &[("X-ReSearch-Token", token.as_str())],
        b"",
    );
    assert_eq!(st, 200);
    assert!(!String::from_utf8_lossy(&raw).contains(env_secret));
    let v = parse_json(&raw);
    assert_eq!(v["openalex_api_key"], "****3051");
    assert_eq!(v["openalex_api_key_configured"], true);
    assert_eq!(v["openalex_api_key_source"], "env");

    // Saving a key in sidecar overrides the env fallback.
    let (st2, _, raw2) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &hdrs,
        &serde_json::to_vec(&json!({ "openalex_api_key": saved_secret })).unwrap(),
    );
    assert_eq!(st2, 200);
    assert!(!String::from_utf8_lossy(&raw2).contains(saved_secret));
    let v2 = parse_json(&raw2);
    assert_eq!(v2["openalex_api_key"], "****8842");
    assert_eq!(v2["openalex_api_key_source"], "saved");

    // Clearing the saved key falls back to the env key again.
    let (st3, _, raw3) = raw_http(
        port,
        "PUT",
        "/api/settings/literature",
        &hdrs,
        br#"{"openalex_api_key":""}"#,
    );
    assert_eq!(st3, 200);
    let v3 = parse_json(&raw3);
    assert_eq!(v3["openalex_api_key"], "****3051");
    assert_eq!(v3["openalex_api_key_source"], "env");

    // Verify `resolve_provider_config` picks up the effective key without leaking in Debug.
    let auth = desktop_server::auth::AuthGuard::new(0).unwrap();
    let sse = std::sync::Arc::new(desktop_server::sse::SseHub::new());
    let chatgpt = desktop_server::chatgpt::ChatGptHub::new(&cfg).unwrap();
    let state = desktop_server::state::DesktopState::new(cfg, auth, sse, chatgpt);
    let pcfg = resolve_provider_config(&state);
    assert_eq!(pcfg.openalex_api_key.as_deref(), Some(env_secret));
    assert!(!format!("{pcfg:?}").contains(env_secret));
}

#[test]
fn put_rejects_malformed_inputs_without_echoing_secret() {
    let tmp = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(tmp.path(), None, None, None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let hdrs = [
        ("X-ReSearch-Token", token.as_str()),
        ("Content-Type", "application/json"),
    ];

    let bad_key = "secret-with-bad-space 9999";
    let cases = [
        br#"{}"#.to_vec(),
        br#"[]"#.to_vec(),
        br#"{"unknown_field":true}"#.to_vec(),
        br#"{"openalex_polite_pool":"yes"}"#.to_vec(),
        br#"{"openalex_api_key":123}"#.to_vec(),
        serde_json::to_vec(&json!({ "openalex_api_key": bad_key })).unwrap(),
        serde_json::to_vec(&json!({ "openalex_api_key": "line1\r\nX-Injected: 1" })).unwrap(),
        br#"{"openalex_mailto":"not-an-email"}"#.to_vec(),
        br#"{"openalex_mailto":"bad@example.org?inject=1"}"#.to_vec(),
    ];
    for body in cases {
        let (st, _, raw) = raw_http(port, "PUT", "/api/settings/literature", &hdrs, &body);
        assert_eq!(st, 400, "expected 400 for body `{}`", String::from_utf8_lossy(&body));
        let text = String::from_utf8_lossy(&raw);
        assert!(
            !text.contains("secret-with-bad-space") && !text.contains("X-Injected"),
            "error response echoed invalid secret: {text}"
        );
    }

    // No sidecar file was created by rejected writes.
    assert!(!settings_path(tmp.path()).exists());
}
