//! Offline tamper verification for the Stage 3 hash-chained ledger and blobs.

use crate::{
    canonical_json, compute_event_hash, is_lower_hex, is_sha256_hex, sha256_hex, Error, Event, Ledger, Result,
    GENESIS_HASH,
};
use rusqlite::params;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Verification summary for a single session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVerifyReport {
    pub session: String,
    pub event_count: u64,
    pub blob_refs_verified: u64,
    pub head_hash: String,
}

/// Verification summary across all sessions and the blob directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyAllReport {
    pub sessions: Vec<SessionVerifyReport>,
    pub total_events: u64,
    pub blobs_verified: u64,
    pub unreferenced_blobs: u64,
}

impl Ledger {
    /// Verify one session's hash chain, canonical payloads, and fixed blob slots.
    pub fn verify_session(&self, session: &str) -> Result<SessionVerifyReport> {
        self.verify_session_collect(session, None)
    }

    /// Read the exact events and head hash verified in the same SQLite snapshot.
    pub fn read_verified_session(&self, session: &str) -> Result<(SessionVerifyReport, Vec<Event>)> {
        let mut events = Vec::new();
        let report = self.verify_session_collect(session, Some(&mut events))?;
        Ok((report, events))
    }

    fn verify_session_collect(
        &self,
        session: &str,
        events: Option<&mut Vec<Event>>,
    ) -> Result<SessionVerifyReport> {
        if self.legacy_unchained {
            return Err(Error::LegacyUnchainedSchema);
        }
        let mut referenced = HashSet::new();
        self.in_read_tx(|| {
            let report = self.verify_session_inner(session, &mut referenced, events)?;
            self.verify_schema_triggers()?;
            Ok(report)
        })
    }

    /// Verify every session in the ledger and every blob file under `blobs/`.
    pub fn verify_all(&self) -> Result<VerifyAllReport> {
        if self.legacy_unchained {
            return Err(Error::LegacyUnchainedSchema);
        }
        let mut referenced = HashSet::new();
        let (reports, total_events) = self.in_read_tx(|| {
            let mut stmt = self.conn.prepare("SELECT DISTINCT session FROM event ORDER BY session")?;
            let sessions = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(stmt);

            let mut reports = Vec::with_capacity(sessions.len());
            let mut total_events = 0u64;
            for session in sessions {
                let report = self.verify_session_inner(&session, &mut referenced, None)?;
                total_events += report.event_count;
                reports.push(report);
            }
            self.verify_schema_triggers()?;
            Ok((reports, total_events))
        })?;
        let (blobs_verified, unreferenced_blobs) = verify_blob_dir(&self.blob_dir, &referenced)?;
        Ok(VerifyAllReport {
            sessions: reports,
            total_events,
            blobs_verified,
            unreferenced_blobs,
        })
    }

    fn verify_schema_triggers(&self) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'event'")?;
        let present: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        for required in ["event_no_update", "event_no_delete", "event_insert_guard"] {
            if !present.contains(required) {
                return Err(Error::Tamper(format!("missing required ledger trigger `{required}`")));
            }
        }
        Ok(())
    }

    fn in_read_tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN DEFERRED")?;
        match f() {
            Ok(v) => {
                if let Err(e) = self.conn.execute_batch("COMMIT") {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(e.into());
                }
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn verify_session_inner(
        &self,
        session: &str,
        referenced: &mut HashSet<String>,
        mut events: Option<&mut Vec<Event>>,
    ) -> Result<SessionVerifyReport> {
        let mut stmt = self.conn.prepare(
            "SELECT session, seq, ts_ms, kind, schema_version, payload, prev_hash, event_hash
             FROM event WHERE session = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![session], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;

        let mut expected_seq: i64 = 1;
        let mut expected_prev = GENESIS_HASH.to_string();
        let mut blob_refs_verified = 0u64;

        for row in rows {
            let (row_session, seq, ts_ms, kind, ver_i64, payload_text, prev_hash, event_hash) = row?;
            if row_session != session {
                return Err(Error::Tamper(format!(
                    "session `{session}` query returned mismatched session `{row_session}`"
                )));
            }
            if seq != expected_seq {
                return Err(Error::Tamper(format!(
                    "session `{session}`: expected seq {expected_seq}, found {seq}"
                )));
            }
            let schema_version = u32::try_from(ver_i64).map_err(|_| {
                Error::Tamper(format!("session `{session}` seq {seq}: invalid schema_version {ver_i64}"))
            })?;
            if prev_hash != expected_prev {
                return Err(Error::Tamper(format!(
                    "session `{session}` seq {seq}: prev_hash mismatch (expected {expected_prev}, got {prev_hash})"
                )));
            }

            let parsed: Value = serde_json::from_str(&payload_text).map_err(|e| {
                Error::Tamper(format!("session `{session}` seq {seq}: invalid payload JSON: {e}"))
            })?;
            if canonical_json(&parsed) != payload_text {
                return Err(Error::Tamper(format!(
                    "session `{session}` seq {seq}: payload is not canonical JSON"
                )));
            }

            let expected_hash = compute_event_hash(
                &row_session,
                seq as u64,
                ts_ms,
                &kind,
                schema_version,
                &payload_text,
                &prev_hash,
            );
            if event_hash != expected_hash {
                return Err(Error::Tamper(format!(
                    "session `{session}` seq {seq}: event_hash mismatch (expected {expected_hash}, got {event_hash})"
                )));
            }

            blob_refs_verified += self.verify_event_blobs(session, seq, &kind, &parsed, referenced)?;
            if let Some(events) = events.as_mut() {
                events.push(Event {
                    session: row_session,
                    seq: seq as u64,
                    ts_ms,
                    kind,
                    schema_version,
                    payload: parsed,
                    prev_hash,
                    event_hash: event_hash.clone(),
                });
            }
            expected_seq += 1;
            expected_prev = event_hash;
        }

        if expected_seq == 1 {
            return Err(Error::UnknownSession(session.to_string()));
        }

        Ok(SessionVerifyReport {
            session: session.to_string(),
            event_count: (expected_seq - 1) as u64,
            blob_refs_verified,
            head_hash: expected_prev,
        })
    }

    fn verify_event_blobs(
        &self,
        session: &str,
        seq: i64,
        kind: &str,
        parsed: &Value,
        referenced: &mut HashSet<String>,
    ) -> Result<u64> {
        let mut verified = 0u64;
        if let Some(raw_val) = parsed.get("raw_blob") {
            let h = raw_val.as_str().ok_or_else(|| {
                Error::Tamper(format!("session `{session}` seq {seq}: raw_blob is not a string"))
            })?;
            self.verify_blob_ref(session, seq, "raw_blob", h, None)?;
            referenced.insert(h.to_string());
            verified += 1;
        }
        let slots: &[&str] = match kind {
            "tool.request" => &["args"],
            "tool.result" => &["result", "error"],
            _ => &[],
        };
        for &slot in slots {
            if let Some(slot_obj) = parsed.get(slot).and_then(Value::as_object) {
                if let Some(blob_val) = slot_obj.get("blob") {
                    let h = blob_val.as_str().ok_or_else(|| {
                        Error::Tamper(format!(
                            "session `{session}` seq {seq} slot `{slot}`: blob is not a string"
                        ))
                    })?;
                    self.verify_blob_ref(session, seq, slot, h, slot_obj.get("bytes"))?;
                    referenced.insert(h.to_string());
                    verified += 1;
                }
            }
        }
        Ok(verified)
    }

    fn verify_blob_ref(
        &self,
        session: &str,
        seq: i64,
        slot: &str,
        hash: &str,
        expected_bytes: Option<&Value>,
    ) -> Result<()> {
        if !is_sha256_hex(hash) {
            return Err(Error::Tamper(format!(
                "session `{session}` seq {seq} slot `{slot}`: invalid blob digest `{hash}`"
            )));
        }
        let bytes = self.get_blob(hash).map_err(|e| {
            Error::Tamper(format!(
                "session `{session}` seq {seq} slot `{slot}`: blob `{hash}` failed verification: {e}"
            ))
        })?;
        if let Some(bytes_val) = expected_bytes {
            let recorded = bytes_val.as_u64().ok_or_else(|| {
                Error::Tamper(format!(
                    "session `{session}` seq {seq} slot `{slot}`: `bytes` is not an unsigned integer"
                ))
            })?;
            if recorded != bytes.len() as u64 {
                return Err(Error::Tamper(format!(
                    "session `{session}` seq {seq} slot `{slot}`: blob `{hash}` byte count mismatch (recorded {recorded}, actual {})",
                    bytes.len()
                )));
            }
        }
        Ok(())
    }
}

fn verify_blob_dir(blob_dir: &Path, referenced: &HashSet<String>) -> Result<(u64, u64)> {
    if !blob_dir.exists() {
        return Ok((0, 0));
    }
    let mut verified = 0u64;
    let mut unreferenced = 0u64;
    let entries = std::fs::read_dir(blob_dir).map_err(|e| {
        Error::Tamper(format!("cannot read blob directory {}: {e}", blob_dir.display()))
    })?;
    for entry in entries {
        let entry = entry?;
        let prefix_os = entry.file_name();
        let prefix = prefix_os
            .to_str()
            .ok_or_else(|| Error::Tamper(format!("non-UTF8 entry in {}", blob_dir.display())))?;
        let ft = entry.file_type()?;
        if !ft.is_dir() || !is_lower_hex(prefix, 2) {
            return Err(Error::Tamper(format!("unexpected entry `{prefix}` in blob directory")));
        }
        for file_entry in std::fs::read_dir(entry.path())? {
            let file_entry = file_entry?;
            let name_os = file_entry.file_name();
            let name = name_os
                .to_str()
                .ok_or_else(|| Error::Tamper(format!("non-UTF8 entry in blobs/{prefix}")))?;
            if !file_entry.file_type()?.is_file() {
                return Err(Error::Tamper(format!("unexpected non-file entry `blobs/{prefix}/{name}`")));
            }
            if name.contains(".tmp") {
                continue;
            }
            if !is_sha256_hex(name) || &name[..2] != prefix {
                return Err(Error::Tamper(format!("invalid blob path `blobs/{prefix}/{name}`")));
            }
            let bytes = std::fs::read(file_entry.path())?;
            let actual = sha256_hex(&bytes);
            if actual != name {
                return Err(Error::Tamper(format!(
                    "blob `{name}` content hash mismatch (got {actual})"
                )));
            }
            verified += 1;
            if !referenced.contains(name) {
                unreferenced += 1;
            }
        }
    }
    Ok((verified, unreferenced))
}
