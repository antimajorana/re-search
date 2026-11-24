//! Stage 3 tests: SHA-256 hash chaining, canonical JSON roundtrip, offline
//! tamper detection, fixed-slot blob integrity, and legacy v0 fail-closed mode.

use ledger::{canonical_json, compute_event_hash, sha256_hex, Error, Ledger, GENESIS_HASH};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ledger-stage3-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn drop_triggers_and_open_raw(dir: &Path) -> Connection {
    let raw = Connection::open(dir.join("ledger.db")).unwrap();
    raw.execute_batch(
        "DROP TRIGGER IF EXISTS event_no_update;
         DROP TRIGGER IF EXISTS event_no_delete;
         DROP TRIGGER IF EXISTS event_insert_guard;",
    )
    .unwrap();
    raw
}

fn setup_three_events(name: &str) -> (PathBuf, Ledger) {
    let dir = fresh_dir(name);
    let ledger = Ledger::open(&dir).unwrap();
    for i in 1..=3 {
        ledger.append("s1", "note", 1, &json!({ "n": i })).unwrap();
    }
    (dir, ledger)
}

fn assert_tamper(res: ledger::Result<impl std::fmt::Debug>, ctx: &str) {
    match res {
        Err(Error::Tamper(_)) => {}
        other => panic!("{ctx}: expected Err(Error::Tamper), got {other:?}"),
    }
}

fn independent_event_hash(
    session: &str,
    seq: u64,
    ts_ms: i64,
    kind: &str,
    schema_version: u32,
    payload_text: &str,
    prev_hash: &str,
) -> String {
    let mut h = Sha256::new();
    for field in [
        "re-search.event.v1",
        session,
        &seq.to_string(),
        &ts_ms.to_string(),
        kind,
        &schema_version.to_string(),
        payload_text,
        prev_hash,
    ] {
        h.update((field.len() as u64).to_le_bytes());
        h.update(field.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn genesis_chain_canonical_json_and_float_roundtrip() {
    let dir = fresh_dir("genesis");
    let ledger = Ledger::open(&dir).unwrap();

    let v1: Value = serde_json::from_str(r#"{"b":1,"a":{"z":0.12345678901234568,"y":2}}"#).unwrap();
    let v2: Value = serde_json::from_str(r#"{"a":{"y":2,"z":0.12345678901234568},"b":1}"#).unwrap();
    assert_eq!(canonical_json(&v1), canonical_json(&v2));
    assert_eq!(canonical_json(&v1), r#"{"a":{"y":2,"z":0.12345678901234568},"b":1}"#);

    let e1 = ledger.append_with_hash("s1", "session.start", 1, &v1).unwrap();
    assert_eq!(e1.seq, 1);
    assert_eq!(e1.prev_hash, GENESIS_HASH);
    assert_eq!(
        e1.event_hash,
        independent_event_hash("s1", 1, e1.ts_ms, "session.start", 1, &canonical_json(&v1), GENESIS_HASH)
    );

    let e2 = ledger.append_with_hash("s1", "note", 2, &json!({"k\"": "line\nbreak"})).unwrap();
    assert_eq!(e2.seq, 2);
    assert_eq!(e2.prev_hash, e1.event_hash);

    let read = ledger.read_session("s1").unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].payload["a"]["z"].as_f64(), Some(0.12345678901234568));
    assert_eq!((read[0].prev_hash.as_str(), read[0].event_hash.as_str()), (GENESIS_HASH, e1.event_hash.as_str()));
    assert_eq!((read[1].prev_hash.as_str(), read[1].event_hash.as_str()), (e1.event_hash.as_str(), e2.event_hash.as_str()));

    let s_rep = ledger.verify_session("s1").unwrap();
    assert_eq!(s_rep.event_count, 2);
    assert_eq!(s_rep.blob_refs_verified, 0);
    assert_eq!(s_rep.head_hash, e2.event_hash);

    let all_rep = ledger.verify_all().unwrap();
    assert_eq!(all_rep.sessions.len(), 1);
    assert_eq!((all_rep.total_events, all_rep.blobs_verified, all_rep.unreferenced_blobs), (2, 0, 0));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_connection_interleaving_verifies_cleanly() {
    let dir = fresh_dir("multi-conn");
    let a = Ledger::open(&dir).unwrap();
    let b = Ledger::open(&dir).unwrap();
    for i in 0..8 {
        let conn = if i % 2 == 0 { &a } else { &b };
        conn.append("s1", "note", 1, &json!({ "step": i })).unwrap();
        conn.append("s2", "note", 1, &json!({ "step": i })).unwrap();
    }
    let rep1 = a.verify_session("s1").unwrap();
    assert_eq!(rep1.event_count, 8);
    let rep_all = b.verify_all().unwrap();
    assert_eq!(rep_all.sessions.len(), 2);
    assert_eq!(rep_all.total_events, 16);
    assert!(matches!(a.verify_session("missing"), Err(Error::UnknownSession(_))));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_session_append_is_atomic_and_rejects_existing_ids() {
    let dir = fresh_dir("new-session");
    let ledger = Ledger::open(&dir).unwrap();
    let raw = Connection::open(dir.join("ledger.db")).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER reject_test_event BEFORE INSERT ON event WHEN NEW.kind = 'reject'
         BEGIN SELECT RAISE(ABORT, 'test insert failure'); END;",
    )
    .unwrap();
    let payload = json!({"name": "branch"});

    assert!(ledger.append_new_session("s1", &[("session.start", 1, &payload), ("reject", 1, &payload)]).is_err());
    assert!(ledger.read_session("s1").unwrap().is_empty());

    let rows = [("session.start", 1, &payload), ("session.fork", 1, &payload)];
    ledger.append_new_session("s1", &rows).unwrap();
    assert!(matches!(
        ledger.append_new_session("s1", &rows),
        Err(Error::Io(e)) if e.kind() == ErrorKind::AlreadyExists
    ));
    let (report, events) = ledger.read_verified_session("s1").unwrap();
    assert_eq!(report.event_count, 2);
    assert_eq!(events[0].prev_hash, GENESIS_HASH);
    assert_eq!(events[1].prev_hash, events[0].event_hash);
    assert_eq!(report.head_hash, events[1].event_hash);

    drop(raw);
    drop(ledger);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_blob_writers_publish_only_whole_files() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Barrier,
    };

    let dir = fresh_dir("parallel-blob");
    let ledger = Ledger::open(&dir).unwrap();
    for byte in 0..8u8 {
        let contents = vec![byte; 512 * 1024];
        let hash = sha256_hex(&contents);
        let final_path = dir.join("blobs").join(&hash[..2]).join(&hash);
        let writers_finished = AtomicBool::new(false);
        let start = Barrier::new(8);
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                while !writers_finished.load(Ordering::Acquire) {
                    if let Ok(meta) = std::fs::metadata(&final_path) {
                        assert_eq!(meta.len(), contents.len() as u64);
                    }
                    std::thread::yield_now();
                }
            });
            let writers = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let writer = Ledger::open_existing(&dir);
                        start.wait();
                        assert_eq!(writer.unwrap().put_blob(&contents).unwrap(), hash);
                    })
                })
                .collect::<Vec<_>>();
            let writers_ok = writers.into_iter().map(|writer| writer.join().is_ok()).collect::<Vec<_>>();
            writers_finished.store(true, Ordering::Release);
            reader.join().unwrap();
            assert!(writers_ok.into_iter().all(|ok| ok));
        });
        assert_eq!(ledger.get_blob(&hash).unwrap(), contents);
    }
    drop(ledger);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn offline_tamper_mutations_all_rejected() {
    let cases: [(&str, &str); 7] = [
        ("payload", "UPDATE event SET payload = '{\"n\":99}' WHERE seq = 2"),
        ("whitespace", "UPDATE event SET payload = '{\"n\": 2}' WHERE seq = 2"),
        ("ts_ms", "UPDATE event SET ts_ms = ts_ms + 1 WHERE seq = 1"),
        ("kind", "UPDATE event SET kind = 'other' WHERE seq = 2"),
        (
            "prev_hash",
            "UPDATE event SET prev_hash = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE seq = 2",
        ),
        (
            "event_hash",
            "UPDATE event SET event_hash = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE seq = 3",
        ),
        ("del_prefix", "DELETE FROM event WHERE seq = 1"),
    ];

    for (tag, sql) in cases {
        let (dir, ledger) = setup_three_events(&format!("tamper-{tag}"));
        let raw = drop_triggers_and_open_raw(&dir);
        raw.execute(sql, []).unwrap();
        assert_tamper(ledger.verify_session("s1"), tag);
        assert_tamper(ledger.verify_all(), &format!("{tag}-all"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Deleted middle row.
    let (dir, ledger) = setup_three_events("tamper-del-mid");
    let raw = drop_triggers_and_open_raw(&dir);
    raw.execute("DELETE FROM event WHERE seq = 2", []).unwrap();
    assert_tamper(ledger.verify_session("s1"), "del_middle");
    let _ = std::fs::remove_dir_all(&dir);

    // Even if an attacker recomputes hashes over non-canonical whitespace, verification catches it.
    let (dir, ledger) = setup_three_events("tamper-rehashed-ws");
    let events = ledger.read_session("s1").unwrap();
    let raw = drop_triggers_and_open_raw(&dir);
    let bad_payload = r#"{"n": 2}"#;
    let h2 = compute_event_hash("s1", 2, events[1].ts_ms, "note", 1, bad_payload, &events[0].event_hash);
    let h3 = compute_event_hash("s1", 3, events[2].ts_ms, "note", 1, r#"{"n":3}"#, &h2);
    raw.execute(
        "UPDATE event SET payload = ?1, event_hash = ?2 WHERE seq = 2",
        rusqlite::params![bad_payload, h2],
    )
    .unwrap();
    raw.execute(
        "UPDATE event SET prev_hash = ?1, event_hash = ?2 WHERE seq = 3",
        rusqlite::params![h2, h3],
    )
    .unwrap();
    assert_tamper(ledger.verify_session("s1"), "rehashed_noncanonical");
    let _ = std::fs::remove_dir_all(&dir);

    // Dropped trigger without row edits is caught, and open_existing does not reinstall triggers or empty schemas.
    let (dir, _ledger) = setup_three_events("tamper-dropped-triggers");
    let _raw = drop_triggers_and_open_raw(&dir);
    let reopened = Ledger::open_existing(&dir).unwrap();
    assert_tamper(reopened.verify_session("s1"), "dropped_triggers_session");
    assert_tamper(reopened.verify_all(), "dropped_triggers_all");
    let _ = std::fs::remove_dir_all(&dir);

    let empty_dir = fresh_dir("tamper-empty-db");
    std::fs::write(empty_dir.join("ledger.db"), b"").unwrap();
    assert!(matches!(Ledger::open_existing(&empty_dir), Err(Error::Tamper(_))));
    let _ = std::fs::remove_dir_all(&empty_dir);
}

#[test]
fn blob_integrity_fixed_slots_and_orphans() {
    let dir = fresh_dir("blobs-ok");
    let ledger = Ledger::open(&dir).unwrap();

    let raw_bytes = b"{\"type\":\"assistant\"}";
    let req_bytes = b"{\"code\":\"1+1\"}";
    let res_bytes = b"{\"value\":\"2\"}";
    let err_bytes = b"worker crashed";
    let h_raw = ledger.put_blob(raw_bytes).unwrap();
    let h_req = ledger.put_blob(req_bytes).unwrap();
    let h_res = ledger.put_blob(res_bytes).unwrap();
    let h_err = ledger.put_blob(err_bytes).unwrap();

    ledger.append("s1", "model.text", 1, &json!({ "text": "hi", "raw_blob": h_raw })).unwrap();
    ledger
        .append(
            "s1",
            "tool.request",
            1,
            &json!({ "call_id": "c1", "args": { "blob": h_req, "bytes": req_bytes.len() } }),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "tool.result",
            1,
            &json!({ "call_id": "c1", "ok": true, "result": { "blob": h_res, "bytes": res_bytes.len() } }),
        )
        .unwrap();
    ledger
        .append(
            "s1",
            "tool.result",
            1,
            &json!({ "call_id": "c2", "ok": false, "error": { "blob": h_err, "bytes": err_bytes.len() } }),
        )
        .unwrap();
    // Model-controlled input containing a fake `blob` object MUST NOT trigger false tamper.
    ledger
        .append(
            "s1",
            "model.tool_use",
            1,
            &json!({ "id": "t1", "name": "sympy_eval", "input": { "blob": GENESIS_HASH, "bytes": 1 } }),
        )
        .unwrap();
    // Inline tool.request args (without `blob` key) MUST pass.
    ledger
        .append("s1", "tool.request", 1, &json!({ "call_id": "c3", "args": { "inline": "{}" } }))
        .unwrap();

    let s_rep = ledger.verify_session("s1").unwrap();
    assert_eq!(s_rep.event_count, 6);
    assert_eq!(s_rep.blob_refs_verified, 4);

    // Valid orphan blob + ignored .tmp file.
    let h_orphan = ledger.put_blob(b"unreferenced cli user line").unwrap();
    let tmp_path = dir.join("blobs").join(&h_orphan[..2]).join(format!("{h_orphan}.tmp999"));
    std::fs::write(&tmp_path, b"partial").unwrap();

    let all_rep = ledger.verify_all().unwrap();
    assert_eq!(all_rep.blobs_verified, 5);
    assert_eq!(all_rep.unreferenced_blobs, 1);

    // Corrupt orphan blob -> verify_all fails, verify_session still passes.
    let orphan_path = dir.join("blobs").join(&h_orphan[..2]).join(&h_orphan);
    std::fs::write(&orphan_path, b"corrupted orphan").unwrap();
    assert!(ledger.verify_session("s1").is_ok());
    assert_tamper(ledger.verify_all(), "corrupt_orphan");
    let _ = std::fs::remove_file(&orphan_path);

    // Corrupt referenced blob bytes -> verify_session & verify_all fail.
    let res_path = dir.join("blobs").join(&h_res[..2]).join(&h_res);
    std::fs::write(&res_path, b"tampered").unwrap();
    assert_tamper(ledger.verify_session("s1"), "corrupt_ref_blob");
    assert_tamper(ledger.verify_all(), "corrupt_ref_blob_all");

    // Missing referenced blob -> verify_session fails.
    std::fs::remove_file(&res_path).unwrap();
    assert_tamper(ledger.verify_session("s1"), "missing_ref_blob");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blob_byte_count_mismatch_rejected() {
    let dir = fresh_dir("blobs-bytes");
    let ledger = Ledger::open(&dir).unwrap();
    let payload = b"actual bytes";
    let h = ledger.put_blob(payload).unwrap();
    ledger
        .append(
            "s1",
            "tool.request",
            1,
            &json!({ "call_id": "c1", "args": { "blob": h, "bytes": payload.len() + 1 } }),
        )
        .unwrap();
    assert_tamper(ledger.verify_session("s1"), "bytes_mismatch");
    assert_tamper(ledger.verify_all(), "bytes_mismatch_all");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_v0_and_missing_dir_fail_closed() {
    let missing = fresh_dir("missing-root").join("does-not-exist");
    let err = match Ledger::open_existing(&missing) {
        Err(e) => e,
        Ok(_) => panic!("open_existing must fail when ledger.db is missing"),
    };
    assert!(matches!(err, Error::Io(ref io) if io.kind() == ErrorKind::NotFound), "{err}");
    assert!(!missing.exists(), "open_existing must not create missing dir");

    let dir = fresh_dir("legacy-v0");
    {
        let raw = Connection::open(dir.join("ledger.db")).unwrap();
        raw.execute_batch(
            "CREATE TABLE event (
                session TEXT NOT NULL,
                seq INTEGER NOT NULL CHECK (seq >= 1),
                ts_ms INTEGER NOT NULL,
                kind TEXT NOT NULL,
                schema_version INTEGER NOT NULL,
                payload TEXT NOT NULL CHECK (json_valid(payload)),
                PRIMARY KEY (session, seq)
            ) STRICT, WITHOUT ROWID;
            INSERT INTO event VALUES ('legacy-s1', 1, 1000, 'note', 1, '{\"ok\":true}');",
        )
        .unwrap();
    }

    let ledger = Ledger::open_existing(&dir).unwrap();
    assert_eq!(ledger.sessions().unwrap(), vec![("legacy-s1".to_string(), 1)]);
    let evs = ledger.read_session("legacy-s1").unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].prev_hash, "");
    assert_eq!(evs[0].event_hash, "");
    assert_eq!(evs[0].payload, json!({"ok": true}));

    assert!(matches!(ledger.append("legacy-s1", "note", 1, &json!({})), Err(Error::LegacyUnchainedSchema)));
    assert!(matches!(
        ledger.append_with_hash("legacy-s1", "note", 1, &json!({})),
        Err(Error::LegacyUnchainedSchema)
    ));
    assert!(matches!(ledger.verify_session("legacy-s1"), Err(Error::LegacyUnchainedSchema)));
    assert!(matches!(ledger.verify_all(), Err(Error::LegacyUnchainedSchema)));
    assert_eq!(sha256_hex(b"").len(), 64);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(missing.parent().unwrap());
}
