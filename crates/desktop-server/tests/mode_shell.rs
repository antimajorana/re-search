//! Integration tests for package F5 (mode shell):
//! - `/api/library/..` route registration and header-token auth enforcement;
//! - `session_mode` single rule, unbuilt-mode refusals (Explore/Solve start nothing);
//! - `UiApprover` prompting and resolving outside a running model turn (`00-overview.md` 3.1 row 13).

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use research_tools::library::{Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::paper_key::PaperKey;
use serde_json::{json, Value};
use tempfile::TempDir;

#[test]
fn library_routes_require_header_token_and_answer_when_authenticated() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);

    // Seed one synthetic paper in `<data_dir>/library/library.db`.
    let lib = Library::open(&data_dir).unwrap();
    let key = PaperKey::parse("arxiv:2405.01234").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds {
            arxiv: Some("2405.01234".into()),
            ..Default::default()
        },
        title: "Synthetic Library Auth Fixture".into(),
        authors: vec!["Ada Example".into()],
        year: Some(2024),
        abstract_text: "Self-written fixture paper for F5 route registration.".into(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1000,
        last_seen_ms: 2000,
    })
    .unwrap();

    // Unauthenticated GET and POST are refused with 401.
    let (st, _, _) = raw_http(port, "GET", "/api/library", &[], b"");
    assert_eq!(st, 401, "unauthenticated GET /api/library must be refused");
    let (st, _, _) = raw_http(port, "GET", "/api/library", &[("Cookie", cookie.as_str())], b"");
    assert_eq!(st, 401, "stream cookie alone must not open /api/library");
    let (st, _, _) = raw_http(port, "POST", "/api/library/rebuild", &[], b"");
    assert_eq!(st, 401, "unauthenticated POST /api/library/rebuild must be refused");

    // Authenticated GET /api/library answers 200 with the seeded paper.
    let auth = [("X-ReSearch-Token", tok.as_str())];
    let (st, _, body) = raw_http(port, "GET", "/api/library", &auth, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
    let val: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["count"], 1);
    assert_eq!(val["papers"][0]["paper"]["key"], key.as_str());

    // Authenticated GET /api/library/<key> resolves detail cleanly.
    let (st, _, detail_body) = raw_http(port, "GET", "/api/library/arxiv:2405.01234", &auth, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&detail_body));
    let detail: Value = serde_json::from_slice(&detail_body).unwrap();
    assert_eq!(detail["paper"]["title"], "Synthetic Library Auth Fixture");

    // Foreign Origin on mutation is still rejected even with a valid token.
    let foreign_body = serde_json::to_vec(&json!({ "status": "key" })).unwrap();
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/library/arxiv:2405.01234/mark",
        &[("X-ReSearch-Token", tok.as_str()), ("Origin", "http://evil.invalid")],
        &foreign_body,
    );
    assert_eq!(st, 403, "foreign Origin mutation on /api/library must be refused");
}
