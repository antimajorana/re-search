//! Project persistence, ownership, provenance, and bibliography downloads.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use serde_json::{json, Value};
use std::sync::Barrier;
use tempfile::TempDir;

fn request(server: &RunningDesktopServer, method: &str, path: &str, body: Value) -> (u16, Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let port = server.addr.port();
    let headers = [("X-ReSearch-Token", server.info.token.as_str())];
    let (status, _, response) = raw_http(port, method, path, &headers, &bytes);
    (status, serde_json::from_slice(&response).unwrap())
}

fn create_project(server: &RunningDesktopServer, title: &str) -> Value {
    let (status, project) = request(
        server,
        "POST",
        "/api/projects",
        json!({
            "title": title,
            "question": "How does the spectrum change with the boundary condition?",
            "conventions": "Set hbar = 1"
        }),
    );
    assert_eq!(status, 201, "{project}");
    project
}

fn create_session(server: &RunningDesktopServer, id: &str) {
    let (status, value) = request(
        server,
        "POST",
        "/api/sessions",
        json!({ "id": id, "title": id }),
    );
    assert_eq!(status, 200, "{value}");
}

#[test]
fn session_creation_checks_project_and_retains_deleted_backlinks() {
    let temp = TempDir::new().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let project = create_project(&server, "Perturbation theory");
    let payload = |project_id: &Value| json!({"id": "s-perturbation", "project_id": project_id});
    let path = "/api/sessions/s-perturbation";

    assert_eq!(
        request(&server, "POST", "/api/sessions", payload(&json!("missing"))).0,
        404
    );
    assert_eq!(request(&server, "GET", path, Value::Null).0, 404);
    let (status, created) = request(&server, "POST", "/api/sessions", payload(&project["id"]));
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["project_id"], project["id"]);
    let (_, sessions) = request(&server, "GET", "/api/sessions", Value::Null);
    assert_eq!(sessions["sessions"][0]["project_id"], project["id"]);

    assert_eq!(request(&server, "DELETE", path, Value::Null).0, 200);
    assert_eq!(
        request(&server, "POST", "/api/sessions", payload(&project["id"])).0,
        409
    );
}

#[test]
fn competing_session_creators_cannot_overwrite_a_project_session() {
    let temp = TempDir::new().unwrap();
    let cfg = test_cfg(temp.path(), None, None, None);
    let first = start_desktop_server(cfg.clone()).unwrap();
    let second = start_desktop_server(cfg).unwrap();
    let a = create_project(&first, "First approach");
    let b = create_project(&second, "Second approach");
    let gate = Barrier::new(2);
    let create = |server: &RunningDesktopServer, project: &Value| {
        gate.wait();
        request(
            server,
            "POST",
            "/api/sessions",
            json!({
                "id": "s-common",
                "project_id": project["id"],
                "title": project["title"],
            }),
        )
        .0
    };
    let statuses = std::thread::scope(|scope| {
        let left = scope.spawn(|| create(&first, &a));
        let right = scope.spawn(|| create(&second, &b));
        [left.join().unwrap(), right.join().unwrap()]
    });
    let winner = match statuses {
        [200, 409] => a,
        [409, 200] => b,
        _ => panic!("unexpected statuses {statuses:?}"),
    };
    let (status, session) = request(&first, "GET", "/api/sessions/s-common", Value::Null);
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["project_id"], winner["id"]);
    assert_eq!(session["title"], winner["title"]);
}

#[test]
fn project_settings_and_single_session_ownership_survive_restart() {
    let temp = TempDir::new().unwrap();
    let cfg = test_cfg(temp.path(), None, None, None);
    let first = start_desktop_server(cfg.clone()).unwrap();
    let second = start_desktop_server(cfg.clone()).unwrap();
    let a = create_project(&first, "Spectrum");
    let b = create_project(&first, "Counterexamples");
    let ap = format!("/api/projects/{}", a["id"].as_str().unwrap());
    let bp = format!("/api/projects/{}", b["id"].as_str().unwrap());
    create_session(&first, "s-spectrum");

    let gate = Barrier::new(2);
    let attach = |server: &RunningDesktopServer, path: &str| {
        gate.wait();
        request(
            server,
            "PUT",
            &format!("{path}/sessions/s-spectrum"),
            json!({}),
        )
        .0
    };
    let statuses = std::thread::scope(|scope| {
        let x = scope.spawn(|| attach(&first, &ap));
        let y = scope.spawn(|| attach(&second, &bp));
        [x.join().unwrap(), y.join().unwrap()]
    });
    assert!(
        statuses == [200, 409] || statuses == [409, 200],
        "{statuses:?}"
    );
    let owner_path = if statuses[0] == 200 { &ap } else { &bp };
    let (_, owner) = request(&first, "GET", owner_path, Value::Null);
    assert_eq!(owner["session_ids"], json!(["s-spectrum"]));
    let (status, unchanged) = request(
        &second,
        "PUT",
        &format!("{owner_path}/sessions/s-spectrum"),
        json!({}),
    );
    assert_eq!(status, 200);
    assert_eq!(unchanged["revision"], owner["revision"]);

    let (status, changed) = request(
        &second,
        "PUT",
        owner_path,
        json!({
            "revision": owner["revision"], "conventions": "Set hbar = c = 1"
        }),
    );
    assert_eq!(status, 200, "{changed}");
    let (status, _) = request(
        &first,
        "PUT",
        owner_path,
        json!({
            "revision": owner["revision"], "title": "Stale title"
        }),
    );
    assert_eq!(status, 409);
    first.shutdown();
    second.shutdown();

    let reopened = start_desktop_server(cfg).unwrap();
    let (status, actual) = request(&reopened, "GET", owner_path, Value::Null);
    assert_eq!(status, 200);
    assert_eq!(actual, changed);
    let (_, list) = request(&reopened, "GET", "/api/projects", Value::Null);
    assert_eq!(list["projects"].as_array().unwrap().len(), 2);
    reopened.shutdown();
}

#[test]
fn project_library_and_items_validate_provenance_and_keep_stable_ids() {
    let temp = TempDir::new().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let project = create_project(&server, "Spectral gaps");
    let path = format!("/api/projects/{}", project["id"].as_str().unwrap());
    let items = format!("{path}/items");
    let sources = format!("{path}/sources");
    create_session(&server, "s-evidence");
    create_session(&server, "s-foreign");
    let (status, _) = request(
        &server,
        "PUT",
        &format!("{path}/sessions/s-evidence"),
        json!({}),
    );
    assert_eq!(status, 200);
    let ledger = Ledger::open(temp.path()).unwrap();
    for session in ["s-evidence", "s-foreign"] {
        ledger
            .append(
                session,
                "proof.entry",
                1,
                &json!({ "entry_id": "e-1", "kind": "result" }),
            )
            .unwrap();
    }
    ledger
        .append(
            "s-evidence",
            "source.ingested",
            1,
            &json!({ "source_id": "paper-abc" }),
        )
        .unwrap();

    for (session, entry) in [("s-foreign", "e-1"), ("s-evidence", "e-404")] {
        let (status, _) = request(
            &server,
            "POST",
            &items,
            json!({
                "kind": "finding", "text": "A claimed gap", "session_id": session, "entry_id": entry
            }),
        );
        assert_eq!(status, 400);
    }
    let (status, _) = request(
        &server,
        "POST",
        &sources,
        json!({
            "title": "Bad backlink", "session_id": "s-evidence", "source_id": "missing-paper"
        }),
    );
    assert_eq!(status, 400);
    for invalid in [
        json!({ "title": "Bad URL", "url": "javascript:alert(1)" }),
        json!({ "title": "Bad DOI", "doi": "aaaaaaaaaaaaaaaéaa" }),
    ] {
        assert_eq!(request(&server, "POST", &sources, invalid).0, 400);
    }
    let (status, created) = request(
        &server,
        "POST",
        &sources,
        json!({
            "title": "Spectral estimate",
            "doi": "10.1234/Gap",
            "bibtex": "@article{gap, title={Spectral estimate}}",
            "notes": "Check the constants",
            "session_id": "s-evidence",
            "source_id": "paper-abc"
        }),
    );
    assert_eq!(status, 200, "{created}");
    let source_id = created["sources"][0]["id"].as_str().unwrap();
    let (status, duplicate) = request(
        &server,
        "POST",
        &sources,
        json!({
            "title": "Spectral estimate revised", "url": "https://doi.org/10.1234/gap"
        }),
    );
    assert_eq!(status, 200, "{duplicate}");
    assert_eq!(duplicate["sources"].as_array().unwrap().len(), 1);
    assert_eq!(duplicate["sources"][0]["id"], source_id);
    assert_eq!(duplicate["sources"][0]["notes"], "Check the constants");
    let (status, cleared) = request(
        &server,
        "POST",
        &sources,
        json!({ "id": source_id, "notes": "" }),
    );
    assert_eq!(status, 200, "{cleared}");
    assert!(cleared["sources"][0]["notes"].is_null());

    let (status, linked) = request(
        &server,
        "POST",
        &items,
        json!({
            "kind": "finding",
            "text": "The smaller eigenvalue stays positive",
            "session_id": "s-evidence",
            "entry_id": "e-1",
            "source_id": source_id,
            "location": "Lemma 4.2",
            "quote": "For the mixed boundary condition"
        }),
    );
    assert_eq!(status, 200, "{linked}");
    let item_id = linked["items"][0]["id"].as_str().unwrap();
    assert_eq!(linked["items"][0]["source_id"], source_id);
    let (status, deleted) = request(
        &server,
        "DELETE",
        &format!("{sources}/{source_id}"),
        Value::Null,
    );
    assert_eq!(status, 200, "{deleted}");
    assert_eq!(deleted["sources"], json!([]));
    assert!(deleted["items"][0]["source_id"].is_null());
    assert_eq!(deleted["items"][0]["entry_id"], "e-1");
    let (status, deleted) = request(
        &server,
        "DELETE",
        &format!("{items}/{item_id}"),
        Value::Null,
    );
    assert_eq!(status, 200, "{deleted}");
    assert_eq!(deleted["items"], json!([]));
    server.shutdown();
}

#[test]
fn concurrent_paper_edits_preserve_newer_changes_and_deleted_records() {
    let temp = TempDir::new().unwrap();
    let cfg = test_cfg(temp.path(), None, None, None);
    let server = start_desktop_server(cfg.clone()).unwrap();
    let project = create_project(&server, "Spectral bounds");
    let papers = format!("/api/projects/{}/sources", project["id"].as_str().unwrap());
    let (_, project) = request(&server, "POST", &papers, json!({"title": "Original paper"}));
    let paper = &project["sources"][0];
    assert_eq!(paper["revision"], 1);
    let id = paper["id"].as_str().unwrap();
    let duplicate =
        json!({"create_only": true, "title": "Original paper", "notes": "Would replace notes"});
    assert_eq!(request(&server, "POST", &papers, duplicate).0, 409);
    server.shutdown();

    // Existing libraries did not persist per-paper revision counters.
    let file = temp.path().join("projects.json");
    let mut legacy: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    legacy[0]["sources"][0]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    std::fs::write(file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let server = start_desktop_server(cfg).unwrap();
    let patch = json!({"id": id, "revision": 0, "notes": "Using the fixed boundary"});
    let (status, updated) = request(&server, "POST", &papers, patch.clone());
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["sources"][0]["revision"], 1);
    let stale_delete = format!("{papers}/{id}?revision=0");
    assert_eq!(
        request(&server, "DELETE", &stale_delete, Value::Null).0,
        409
    );
    assert_eq!(request(&server, "POST", &papers, patch.clone()).0, 409);
    let noop = json!({"id": id, "revision": 1, "notes": "Using the fixed boundary"});
    assert_eq!(request(&server, "POST", &papers, noop).1, updated);
    assert_eq!(
        request(&server, "DELETE", &format!("{papers}/{id}"), Value::Null).0,
        200
    );
    assert_eq!(request(&server, "POST", &papers, patch).0, 404);
}

#[test]
fn bibtex_download_allows_only_the_expected_cookie_route() {
    let temp = TempDir::new().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let project = create_project(&server, "Bibliography");
    let path = format!("/api/projects/{}", project["id"].as_str().unwrap());
    let bibtex = "@article{paper, title={Boundary effects}}";
    let (status, _) = request(
        &server,
        "POST",
        &format!("{path}/sources"),
        json!({
            "title": "Boundary effects", "bibtex": bibtex
        }),
    );
    assert_eq!(status, 200);
    let port = server.addr.port();
    let cookie = launch_cookie(port, &server.info.token);
    let export = format!("{path}/export?format=bib");
    let cookie_headers = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];
    let (status, headers, bytes) = raw_http(port, "GET", &export, &cookie_headers, b"");
    assert_eq!(status, 200);
    assert!(headers
        .iter()
        .any(|(key, value)| key == "content-disposition"
            && value == "attachment; filename=\"project-references.bib\""));
    assert_eq!(String::from_utf8(bytes).unwrap(), format!("{bibtex}\n"));
    for (method, route) in [
        ("GET", path.as_str()),
        ("GET", "/api/projects"),
        ("POST", export.as_str()),
    ] {
        assert_eq!(raw_http(port, method, route, &cookie_headers, b"").0, 401);
    }
    let foreign_headers = [("Cookie", cookie.as_str()), ("Sec-Fetch-Site", "same-site")];
    assert_eq!(raw_http(port, "GET", &export, &foreign_headers, b"").0, 401);
    assert_eq!(raw_http(port, "GET", &export, &[], b"").0, 401);
    server.shutdown();
}
