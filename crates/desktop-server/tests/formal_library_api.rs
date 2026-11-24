//! Manual Formal Library search uses the workspace attached to the requested session.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::sha256_hex;
use serde_json::{json, Value};
use std::fs;

#[test]
fn formal_library_search_follows_workspace_attachment() {
    let data = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let source_path = workspace
        .path()
        .join(".lake/packages/physlib/Physlib/Counting.lean");
    fs::create_dir_all(source_path.parent().unwrap()).unwrap();
    let source = "namespace Counting\ntheorem pairSize : 2 + 2 = 4 := by decide\nend Counting\n";
    fs::write(source_path, source).unwrap();

    let srv = start_desktop_server(test_cfg(data.path(), None, None, None)).unwrap();
    let headers = [("X-ReSearch-Token", srv.info.token.as_str())];
    let call = |method, path: &str, body| {
        let (status, _, bytes) = raw_http(
            srv.addr.port(),
            method,
            path,
            &headers,
            &serde_json::to_vec(&body).unwrap(),
        );
        (status, serde_json::from_slice::<Value>(&bytes).unwrap())
    };
    let (status, session) = call(
        "POST",
        "/api/sessions",
        json!({ "title": "Check counting" }),
    );
    assert_eq!(status, 200, "{session}");
    let sid = session["id"].as_str().unwrap();
    let search_path = format!("/api/sessions/{sid}/formal-library");
    let search = || {
        call(
            "POST",
            &search_path,
            json!({ "query": "Counting.pairSize" }),
        )
    };

    assert_eq!(search().0, 400);
    assert_eq!(
        call(
            "PUT",
            &format!("/api/sessions/{sid}/workspace"),
            json!({ "workspace_root": workspace.path() }),
        )
        .0,
        200
    );

    let (status, response) = search();
    assert_eq!(status, 200, "{response}");
    assert_eq!(response["status"], "lexical_unverified");
    let declaration = &response["results"][0];
    assert_eq!(declaration["qualified_name"], "Counting.pairSize");
    assert_eq!(
        declaration["path"],
        ".lake/packages/physlib/Physlib/Counting.lean"
    );
    assert_eq!(declaration["line"], 2);
    assert_eq!(declaration["signature"], "theorem pairSize : 2 + 2 = 4");
    assert_eq!(declaration["source_sha256"], sha256_hex(source.as_bytes()));
    assert_eq!(
        call(
            "POST",
            "/api/sessions/missing/formal-library",
            json!({"query": "pairSize"})
        )
        .0,
        404
    );
    let (asset_status, _, asset) = raw_http(
        srv.addr.port(),
        "GET",
        "/assets/formal_library.js",
        &headers,
        b"",
    );
    assert_eq!(asset_status, 200);
    assert!(String::from_utf8(asset)
        .unwrap()
        .contains("initFormalLibrary"));
    srv.shutdown();
}
