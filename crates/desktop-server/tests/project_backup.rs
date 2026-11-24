//! A portable project backup must preserve exact evidence, isolate the project, and refuse collisions.
mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::{sha256_hex, Ledger};
use serde_json::{json, Value};
use std::fs;
use std::io::{Cursor, Read, Write};

fn request(server: &RunningDesktopServer, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let (code, _, bytes) = raw_http(
        server.addr.port(),
        method,
        path,
        &[("X-ReSearch-Token", &server.info.token)],
        body,
    );
    (code, bytes)
}

fn post(server: &RunningDesktopServer, path: &str, body: Value, code: u16) -> Value {
    let (status, bytes) = request(server, "POST", path, &serde_json::to_vec(&body).unwrap());
    assert_eq!(status, code, "{}", String::from_utf8_lossy(&bytes));
    serde_json::from_slice(&bytes).unwrap()
}

fn rewrite_manifest(zip: &[u8], change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut original = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    let mut entries = std::collections::BTreeMap::new();
    for idx in 0..original.len() {
        let mut file = original.by_index(idx).unwrap();
        let key = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        entries.insert(key, bytes);
    }
    let mut manifest = serde_json::from_slice(&entries["project.json"]).unwrap();
    change(&mut manifest);
    let encoded = serde_json::to_vec(&manifest).unwrap();
    entries.insert("project.sha256".into(), sha256_hex(&encoded).into_bytes());
    entries.insert("project.json".into(), encoded);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (key, bytes) in entries {
        writer
            .start_file(key, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn restore_scoped_project_and_exact_receipts_without_rebinding_workspaces() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let src = start_desktop_server(test_cfg(source.path(), None, None, None)).unwrap();
    let dst = start_desktop_server(test_cfg(target.path(), None, None, None)).unwrap();
    let project = post(&src, "/api/projects", json!({"title":"Gap review"}), 201);
    let pid = project["id"].as_str().unwrap();
    let sid = "s-restore-test";
    post(
        &src,
        "/api/sessions",
        json!({"id":sid,"project_id":pid,"workspace_root":workspace.path()}),
        200,
    );
    post(&src, "/api/sessions", json!({"id":"s-unrelated"}), 200);
    let ledger = Ledger::open(source.path()).unwrap();
    let text = b"Periodic boundary conditions";
    let hash = ledger.put_blob(text).unwrap();
    let file = "paper/boundary.txt";
    let directory = source.path().join("sessions").join(sid);
    fs::create_dir_all(directory.join("sources/paper")).unwrap();
    fs::write(directory.join("sources").join(file), text).unwrap();
    fs::write(directory.join("manuscript-drafts.json"), b"[]").unwrap();
    fs::write(
        directory.join("sidetalk.jsonl"),
        b"{\"text\":\"Check units\"}\n",
    )
    .unwrap();
    ledger.append(sid, "source.ingested", 1, &json!({
        "source_id":"paper", "files":[file], "primary_path":file, "files_sha256":{(file):hash}, "raw_blob":hash
    })).unwrap();
    ledger
        .append(
            sid,
            "proof.entry",
            1,
            &json!({"entry_id":"e-1","entry_type":"assumption","summary":"Periodic chain"}),
        )
        .unwrap();
    ledger
        .append(
            &format!("sidetalk:{sid}"),
            "model.text",
            1,
            &json!({"text":"Check units"}),
        )
        .unwrap();
    let unrelated = ledger.put_blob(b"another project only").unwrap();
    ledger
        .append(
            "s-unrelated",
            "model.text",
            1,
            &json!({"raw_blob":unrelated}),
        )
        .unwrap();
    let plans = format!("/api/projects/{pid}/validation-plans");
    let created = post(
        &src,
        &plans,
        json!({
            "session_id":sid,"claim":"Ground energy","candidate_method":"Diagonalize the Hamiltonian",
            "reference_method":"Exact finite chain","axis":"basis","atol":"1e-6","rtol":"0",
            "reference":"2","known_pass":"2","planted_fail":"3"
        }),
        201,
    );
    let exp = format!("/api/projects/{pid}/experiments");
    post(
        &src,
        &exp,
        json!({"question":"Convergence?","metric":{"name":"error","unit":"eV","direction":"minimize"},
        "budget":{"limit":4,"unit":"samples"},"baseline":{"session_id":sid,"at_seq":2}}),
        201,
    );
    let (status, backup) = request(
        &src,
        "GET",
        &format!("/api/projects/{pid}/export?format=backup"),
        &[],
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&backup));
    let info = zip::ZipArchive::new(Cursor::new(&backup)).unwrap();
    let foreign_object = format!("objects/{unrelated}");
    assert!(!info
        .file_names()
        .any(|name| name.unwrap() == foreign_object));
    let restore = |buf: &[u8]| request(&dst, "POST", "/api/projects/restore", buf);

    let rebound = rewrite_manifest(&backup, |m| {
        m["sessions"][sid]["workspace_root"] = json!(workspace.path())
    });
    assert_eq!(restore(&rebound).0, 400);
    let damaged = rewrite_manifest(&backup, |m| {
        m["ledgers"][sid][0]["payload"]["source_id"] = json!("forged")
    });
    assert_eq!(restore(&damaged).0, 400);
    let wrong_hash = rewrite_manifest(&backup, |m| {
        m["ledgers"][sid][1]["payload"]["summary"] = json!("incorrect")
    });
    assert_eq!(restore(&wrong_hash).0, 400);
    assert!(!target.path().join("sessions").join(sid).exists());
    assert!(Ledger::open(target.path())
        .unwrap()
        .sessions()
        .unwrap()
        .is_empty());
    let (code, bytes) = restore(&backup);
    assert_eq!(code, 201, "{}", String::from_utf8_lossy(&bytes));
    assert_eq!(restore(&backup).0, 409);

    let restored = Ledger::open(target.path()).unwrap();
    assert_eq!(
        restored.read_verified_session(sid).unwrap(),
        ledger.read_verified_session(sid).unwrap()
    );
    assert_eq!(
        restored
            .read_verified_session(&format!("sidetalk:{sid}"))
            .unwrap(),
        ledger
            .read_verified_session(&format!("sidetalk:{sid}"))
            .unwrap()
    );
    assert!(restored.get_blob(&unrelated).is_err());
    assert!(restored.read_session("s-unrelated").unwrap().is_empty());
    let directory = target.path().join("sessions").join(sid);
    assert_eq!(
        fs::read(directory.join("sources").join(file)).unwrap(),
        text
    );
    assert_eq!(
        fs::read(directory.join("manuscript-drafts.json")).unwrap(),
        b"[]"
    );
    let meta: Value =
        serde_json::from_slice(&fs::read(directory.join("meta.json")).unwrap()).unwrap();
    assert!(meta["workspace_root"].is_null());
    let (_, plan_bytes) = request(&dst, "GET", &plans, &[]);
    assert!(String::from_utf8(plan_bytes)
        .unwrap()
        .contains(created["commit_hash"].as_str().unwrap()));
    assert_eq!(
        request(&src, "GET", &exp, &[]),
        request(&dst, "GET", &exp, &[])
    );

    let marker = target
        .path()
        .join(format!(".restore-{}.pending", sha256_hex(pid.as_bytes())));
    fs::write(&marker, sha256_hex(&backup)).unwrap();
    assert_eq!(restore(&backup).0, 200);
    assert!(!marker.exists());
}
