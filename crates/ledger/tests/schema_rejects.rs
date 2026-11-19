//! The append-only rules live in the database schema, not only in the crate's
//! API. These tests open a second, raw connection with default settings (none
//! of the crate's pragmas) and show that the schema still refuses to rewrite
//! history.

use ledger::{Ledger, GENESIS_HASH};
use rusqlite::Connection;
use serde_json::json;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ledger-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn setup(name: &str) -> (PathBuf, Ledger, Connection) {
    let dir = temp_dir(name);
    let ledger = Ledger::open(&dir).unwrap();
    assert_eq!(ledger.append("s1", "note", 1, &json!({"n": 1})).unwrap(), 1);
    assert_eq!(ledger.append("s1", "note", 1, &json!({"n": 2})).unwrap(), 2);
    let raw = Connection::open(dir.join("ledger.db")).unwrap();
    (dir, ledger, raw)
}

fn assert_unchanged(ledger: &Ledger) {
    let events = ledger.read_session("s1").unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload, json!({"n": 1}));
    assert_eq!(events[1].payload, json!({"n": 2}));
}

const INSERT: &str =
    "INSERT INTO event (session, seq, ts_ms, kind, schema_version, payload, prev_hash, event_hash) VALUES";

#[test]
fn append_and_read_round_trip() {
    let (_dir, ledger, raw) = setup("roundtrip");
    let user_ver: i64 = raw.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(user_ver, 3);
    assert_eq!(ledger.append("s2", "note", 3, &json!({"other": true})).unwrap(), 1);
    assert_eq!(ledger.append("s1", "note", 1, &json!({"n": 3})).unwrap(), 3);
    let s1 = ledger.read_session("s1").unwrap();
    assert_eq!(s1.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(s1[0].prev_hash, GENESIS_HASH);
    assert_eq!(s1[1].prev_hash, s1[0].event_hash);
    assert_eq!(s1[2].prev_hash, s1[1].event_hash);
    let s2 = ledger.read_session("s2").unwrap();
    assert_eq!((s2[0].kind.as_str(), s2[0].schema_version), ("note", 3));
    assert_eq!(s2[0].prev_hash, GENESIS_HASH);
    assert_eq!(ledger.sessions().unwrap().len(), 2);
}

#[test]
fn update_is_rejected() {
    let (_dir, ledger, raw) = setup("update");
    let err = raw.execute("UPDATE event SET payload = '{}' WHERE seq = 1", []).unwrap_err();
    assert!(err.to_string().contains("UPDATE rejected"), "{err}");
    assert_unchanged(&ledger);
}

#[test]
fn delete_is_rejected() {
    let (_dir, ledger, raw) = setup("delete");
    let err = raw.execute("DELETE FROM event WHERE seq = 2", []).unwrap_err();
    assert!(err.to_string().contains("DELETE rejected"), "{err}");
    let err = raw.execute("DELETE FROM event", []).unwrap_err();
    assert!(err.to_string().contains("DELETE rejected"), "{err}");
    assert_unchanged(&ledger);
}

#[test]
fn insert_or_replace_is_rejected() {
    let (_dir, ledger, raw) = setup("replace");
    for stmt in ["INSERT OR REPLACE INTO", "REPLACE INTO"] {
        let sql = format!(
            "{stmt} event (session, seq, ts_ms, kind, schema_version, payload, prev_hash, event_hash) \
             VALUES ('s1', 1, 0, 'note', 1, '{{}}', '{GENESIS_HASH}', '{GENESIS_HASH}')"
        );
        let err = raw.execute(&sql, []).unwrap_err();
        assert!(err.to_string().contains("max(seq)+1"), "{err}");
    }
    assert_unchanged(&ledger);
}

#[test]
fn out_of_order_seq_is_rejected() {
    let (_dir, ledger, raw) = setup("seq");
    let head = ledger.read_session("s1").unwrap().pop().unwrap().event_hash;
    // gap, repeat, zero, and a new session that does not start at 1
    for (session, seq) in [("s1", 4), ("s1", 2), ("s1", 0), ("new", 2)] {
        let sql = format!("{INSERT} ('{session}', {seq}, 0, 'note', 1, '{{}}', '{GENESIS_HASH}', '{GENESIS_HASH}')");
        let err = raw.execute(&sql, []).unwrap_err();
        assert!(
            err.to_string().contains("max(seq)+1"),
            "seq {seq} in {session} should fail on max(seq)+1, got: {err}"
        );
    }
    assert_unchanged(&ledger);
    // the next number in order with matching prev_hash is accepted, even on the raw connection
    raw.execute(
        &format!("{INSERT} ('s1', 3, 0, 'note', 1, '{{}}', '{head}', '{GENESIS_HASH}')"),
        [],
    )
    .unwrap();
}

#[test]
fn prev_hash_mismatch_and_bad_hex_are_rejected() {
    let (_dir, ledger, raw) = setup("prev_hash");
    let head = ledger.read_session("s1").unwrap().pop().unwrap().event_hash;
    let err = raw
        .execute(
            &format!("{INSERT} ('s1', 3, 0, 'note', 1, '{{}}', '{GENESIS_HASH}', '{GENESIS_HASH}')"),
            [],
        )
        .unwrap_err();
    assert!(err.to_string().contains("prev_hash mismatch"), "{err}");

    let alt = "a".repeat(64);
    let err = raw
        .execute(&format!("{INSERT} ('s_new', 1, 0, 'note', 1, '{{}}', '{alt}', '{GENESIS_HASH}')"), [])
        .unwrap_err();
    assert!(err.to_string().contains("prev_hash mismatch"), "{err}");

    for bad_hex in ["A".repeat(64), "short".into(), "g".repeat(64)] {
        assert!(
            raw.execute(
                &format!("{INSERT} ('s1', 3, 0, 'note', 1, '{{}}', '{head}', '{bad_hex}')"),
                [],
            )
            .is_err(),
            "bad event_hash `{bad_hex}` should be rejected"
        );
    }
    assert_unchanged(&ledger);
}

#[test]
fn invalid_json_and_wrong_types_are_rejected() {
    let (_dir, ledger, raw) = setup("json");
    let head = ledger.read_session("s1").unwrap().pop().unwrap().event_hash;
    assert!(raw
        .execute(
            &format!("{INSERT} ('s1', 3, 0, 'note', 1, 'not json', '{head}', '{GENESIS_HASH}')"),
            [],
        )
        .is_err());
    // STRICT table: a text value in an INTEGER column is refused
    assert!(raw
        .execute(
            &format!("{INSERT} ('s1', 3, 'soon', 'note', 1, '{{}}', '{head}', '{GENESIS_HASH}')"),
            [],
        )
        .is_err());
    assert_unchanged(&ledger);
}

#[test]
fn two_connections_interleave_without_collisions() {
    let (dir, a, _raw) = setup("interleave");
    let b = Ledger::open(&dir).unwrap();
    let mut seqs = Vec::new();
    for i in 0..10 {
        let l = if i % 2 == 0 { &a } else { &b };
        seqs.push(l.append("s1", "note", 1, &json!({"i": i})).unwrap());
    }
    assert_eq!(seqs, (3..13).collect::<Vec<u64>>());
    a.verify_session("s1").unwrap();
    b.verify_all().unwrap();
}

#[test]
fn blobs_are_content_addressed() {
    let (_dir, ledger, _raw) = setup("blobs");
    let h = ledger.put_blob(b"hello").unwrap();
    assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    assert_eq!(ledger.put_blob(b"hello").unwrap(), h);
    assert_eq!(ledger.get_blob(&h).unwrap(), b"hello");
    assert!(ledger.get_blob("../../etc/passwd").is_err());
}
