//! Non-rebuildable local store for user corrections reused by code (package F16a,
//! `00-overview.md` 2.14).
//!
//! Backed by `<data_dir>/library/lessons.db` via [`ledger::LessonsDb`]. F16a adds no new way
//! to correct: it reads existing correction events from session ledgers (and F10 library marks
//! via [`crate::lessons_select`]), lets the user inspect/edit/delete every stored entry, and
//! involves **zero model calls** and **zero changes** to any badge, tier, verdict, or gate.

pub use crate::lessons_schema::*;
use crate::paper_key::{NodeRef, PaperKey};
use crate::paper_sources::{now_ms, sha256_hex};
use ledger::lessons_db::{LessonsDb, LessonsSnapshot};
use ledger::Event;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Handle to `<data_dir>/library/lessons.db`.
pub struct LessonsStore {
    db: LessonsDb,
}

impl LessonsStore {
    /// Open or create `<data_dir>/library/lessons.db`.
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        let db = LessonsDb::open(data_dir)
            .map_err(|e| format!("failed to open lessons database: {e}"))?;
        Ok(Self { db })
    }

    pub fn db_path(&self) -> PathBuf {
        self.db.db_path().to_path_buf()
    }

    pub fn schema_version(&self) -> Result<u32, String> {
        self.db.schema_version().map_err(|e| e.to_string())
    }

    // --- Term aliases ("same under another name" confirmed or rejected) ---

    pub fn record_term_alias(&self, record: &TermAliasRecord) -> Result<(), String> {
        let row = record.to_row()?;
        self.db.upsert_term_alias(&row).map_err(|e| e.to_string())
    }

    pub fn list_term_aliases(&self) -> Result<Vec<TermAliasRecord>, String> {
        self.db
            .list_term_aliases()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(TermAliasRecord::from_row)
            .collect()
    }

    pub fn delete_term_alias(&self, term_a: &str, term_b: &str) -> Result<bool, String> {
        let (a, b) = canonical_term_pair(term_a, term_b)?;
        self.db
            .delete_term_alias(&a, &b)
            .map_err(|e| e.to_string())
    }

    // --- Solve symbol & convention defaults ---

    pub fn record_symbol_default(&self, record: &SymbolDefault) -> Result<(), String> {
        let row = record.to_row()?;
        self.db
            .upsert_symbol_default(&row)
            .map_err(|e| e.to_string())
    }

    pub fn list_symbol_defaults(&self) -> Result<Vec<SymbolDefault>, String> {
        self.db
            .list_symbol_defaults()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SymbolDefault::from_row)
            .collect()
    }

    pub fn delete_symbol_default(&self, symbol: &str, domain: &str) -> Result<bool, String> {
        self.db
            .delete_symbol_default(symbol.trim(), &domain.trim().to_lowercase())
            .map_err(|e| e.to_string())
    }

    pub fn record_convention_default(&self, record: &ConventionDefault) -> Result<(), String> {
        let row = record.to_row()?;
        self.db
            .upsert_convention_default(&row)
            .map_err(|e| e.to_string())
    }

    pub fn list_convention_defaults(&self) -> Result<Vec<ConventionDefault>, String> {
        Ok(self
            .db
            .list_convention_defaults()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(ConventionDefault::from_row)
            .collect())
    }

    pub fn delete_convention_default(&self, key: &str, domain: &str) -> Result<bool, String> {
        self.db
            .delete_convention_default(key.trim(), &domain.trim().to_lowercase())
            .map_err(|e| e.to_string())
    }

    // --- Investigate skeleton reader overrides (pure skeleton stays unchanged) ---

    pub fn record_skeleton_override(&self, record: &SkeletonOverride) -> Result<(), String> {
        let row = record.to_row()?;
        self.db
            .upsert_skeleton_override(&row)
            .map_err(|e| e.to_string())
    }

    pub fn get_skeleton_override(&self, node_tag: &str) -> Result<Option<SkeletonOverride>, String> {
        self.db
            .get_skeleton_override(node_tag.trim())
            .map_err(|e| e.to_string())?
            .map(SkeletonOverride::from_row)
            .transpose()
    }

    pub fn list_skeleton_overrides(&self) -> Result<Vec<SkeletonOverride>, String> {
        self.db
            .list_skeleton_overrides()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SkeletonOverride::from_row)
            .collect()
    }

    pub fn delete_skeleton_override(&self, node_tag: &str) -> Result<bool, String> {
        self.db
            .delete_skeleton_override(node_tag.trim())
            .map_err(|e| e.to_string())
    }

    // --- OCR line correction overlays (pinned text stays unchanged) ---

    pub fn record_ocr_overlay(&self, record: &OcrOverlay) -> Result<(), String> {
        let row = record.to_row()?;
        self.db.upsert_ocr_overlay(&row).map_err(|e| e.to_string())
    }

    pub fn list_ocr_overlays(&self, file_sha256: Option<&str>) -> Result<Vec<OcrOverlay>, String> {
        let norm = file_sha256.map(|s| s.trim().to_ascii_lowercase());
        Ok(self
            .db
            .list_ocr_overlays(norm.as_deref())
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(OcrOverlay::from_row)
            .collect())
    }

    pub fn delete_ocr_overlay(&self, file_sha256: &str, line_no: usize) -> Result<bool, String> {
        if line_no == 0 {
            return Ok(false);
        }
        self.db
            .delete_ocr_overlay(&file_sha256.trim().to_ascii_lowercase(), line_no as i64)
            .map_err(|e| e.to_string())
    }

    // --- Idempotent ingestion from existing session ledger events (read-only on ledger) ---

    /// Scan `events` from `session_id` above the stored cursor and record any existing
    /// correction events into `lessons.db`. Never writes to the ledger or invokes a model.
    pub fn ingest_session_events(
        &self,
        session_id: &str,
        events: &[Event],
    ) -> Result<IngestCorrectionsReport, String> {
        let cursor = self
            .db
            .get_ingested_cursor(session_id)
            .map_err(|e| e.to_string())?;
        let mut report = IngestCorrectionsReport {
            session_id: session_id.to_string(),
            ..Default::default()
        };
        let mut max_seq = cursor;

        for ev in events {
            if ev.seq <= cursor {
                continue;
            }
            max_seq = max_seq.max(ev.seq);
            report.events_scanned += 1;
            let ts = ev.ts_ms.max(0);
            match ev.kind.as_str() {
                EVENT_IDEA_VERDICT_CONFIRMED
                | EVENT_IDEA_ALIAS_CONFIRMED
                | EVENT_IDEA_ALIAS_REJECTED
                | EVENT_IDEA_CORRECTION => {
                    if let Some(alias) = extract_term_alias(session_id, &ev.kind, &ev.payload, ts) {
                        self.record_term_alias(&alias)?;
                        report.term_aliases_recorded += 1;
                    }
                }
                EVENT_SOLVE_SETUP | EVENT_SOLVE_SETUP_REVISED | EVENT_SESSION_CONVENTIONS => {
                    let (syms, convs) = extract_solve_defaults(session_id, &ev.payload, ts);
                    for s in syms {
                        self.record_symbol_default(&s)?;
                        report.symbol_defaults_recorded += 1;
                    }
                    for c in convs {
                        self.record_convention_default(&c)?;
                        report.convention_defaults_recorded += 1;
                    }
                }
                EVENT_INVESTIGATION_NODE_OVERRIDE | EVENT_INVESTIGATION_PIN => {
                    if let Some(ov) = extract_skeleton_override(session_id, &ev.payload, ts) {
                        self.record_skeleton_override(&ov)?;
                        report.skeleton_overrides_recorded += 1;
                    }
                }
                EVENT_OCR_LINE_CORRECTED | EVENT_PAPER_OCR_CORRECTED => {
                    if let Some(ocr) = extract_ocr_overlay(session_id, &ev.payload, ts) {
                        self.record_ocr_overlay(&ocr)?;
                        report.ocr_overlays_recorded += 1;
                    }
                }
                _ => {}
            }
        }

        if max_seq > cursor {
            self.db
                .set_ingested_cursor(session_id, max_seq, now_ms())
                .map_err(|e| e.to_string())?;
        }
        Ok(report)
    }

    // --- Opt-in portable checksummed backup & restore ---

    pub fn export_backup(&self) -> Result<LessonsBackup, String> {
        let term_aliases = self.list_term_aliases()?;
        let symbol_defaults = self.list_symbol_defaults()?;
        let convention_defaults = self.list_convention_defaults()?;
        let skeleton_overrides = self.list_skeleton_overrides()?;
        let ocr_overlays = self.list_ocr_overlays(None)?;
        let sha256 = backup_checksum(
            &term_aliases,
            &symbol_defaults,
            &convention_defaults,
            &skeleton_overrides,
            &ocr_overlays,
        )?;
        Ok(LessonsBackup {
            schema_version: LESSONS_BACKUP_SCHEMA_VERSION,
            exported_ms: now_ms(),
            sha256,
            term_aliases,
            symbol_defaults,
            convention_defaults,
            skeleton_overrides,
            ocr_overlays,
        })
    }

    pub fn export_backup_bytes(&self) -> Result<Vec<u8>, String> {
        let backup = self.export_backup()?;
        serde_json::to_vec_pretty(&backup).map_err(|e| e.to_string())
    }

    pub fn restore_backup_bytes(&mut self, bytes: &[u8]) -> Result<usize, String> {
        if bytes.len() > MAX_LESSONS_BACKUP_BYTES {
            return Err(format!(
                "lessons backup exceeds {} byte limit",
                MAX_LESSONS_BACKUP_BYTES
            ));
        }
        let backup: LessonsBackup =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid lessons backup JSON: {e}"))?;
        if backup.schema_version != LESSONS_BACKUP_SCHEMA_VERSION {
            return Err(format!(
                "unsupported lessons backup schema_version {} (expected {})",
                backup.schema_version, LESSONS_BACKUP_SCHEMA_VERSION
            ));
        }
        let expected = backup_checksum(
            &backup.term_aliases,
            &backup.symbol_defaults,
            &backup.convention_defaults,
            &backup.skeleton_overrides,
            &backup.ocr_overlays,
        )?;
        if expected != backup.sha256 {
            return Err("lessons backup SHA-256 checksum mismatch".into());
        }

        let snap = LessonsSnapshot {
            term_aliases: backup
                .term_aliases
                .iter()
                .map(TermAliasRecord::to_row)
                .collect::<Result<Vec<_>, _>>()?,
            symbol_defaults: backup
                .symbol_defaults
                .iter()
                .map(SymbolDefault::to_row)
                .collect::<Result<Vec<_>, _>>()?,
            convention_defaults: backup
                .convention_defaults
                .iter()
                .map(ConventionDefault::to_row)
                .collect::<Result<Vec<_>, _>>()?,
            skeleton_overrides: backup
                .skeleton_overrides
                .iter()
                .map(SkeletonOverride::to_row)
                .collect::<Result<Vec<_>, _>>()?,
            ocr_overlays: backup
                .ocr_overlays
                .iter()
                .map(OcrOverlay::to_row)
                .collect::<Result<Vec<_>, _>>()?,
        };
        let count = snap.term_aliases.len()
            + snap.symbol_defaults.len()
            + snap.convention_defaults.len()
            + snap.skeleton_overrides.len()
            + snap.ocr_overlays.len();
        self.db
            .replace_snapshot(&snap)
            .map_err(|e| e.to_string())?;
        Ok(count)
    }
}

fn backup_checksum(
    term_aliases: &[TermAliasRecord],
    symbol_defaults: &[SymbolDefault],
    convention_defaults: &[ConventionDefault],
    skeleton_overrides: &[SkeletonOverride],
    ocr_overlays: &[OcrOverlay],
) -> Result<String, String> {
    let payload = serde_json::json!({
        "term_aliases": term_aliases,
        "symbol_defaults": symbol_defaults,
        "convention_defaults": convention_defaults,
        "skeleton_overrides": skeleton_overrides,
        "ocr_overlays": ocr_overlays,
    });
    let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    Ok(sha256_hex(&bytes))
}

fn extract_term_alias(
    session_id: &str,
    kind: &str,
    payload: &Value,
    ts_ms: i64,
) -> Option<TermAliasRecord> {
    let term_a = str_field(payload, &["term_a", "term", "idea_term", "source_term"])?;
    let term_b = str_field(payload, &["term_b", "other_name", "alias", "literature_term"])?;
    let (a, b) = canonical_term_pair(&term_a, &term_b).ok()?;

    let decision = match kind {
        EVENT_IDEA_ALIAS_CONFIRMED => TermAliasDecision::Confirmed,
        EVENT_IDEA_ALIAS_REJECTED => TermAliasDecision::Rejected,
        _ => {
            if let Some(d) = payload.get("decision").and_then(Value::as_str) {
                match d {
                    "confirmed" | "confirm" | "accept" | "same" => TermAliasDecision::Confirmed,
                    "rejected" | "reject" | "different" => TermAliasDecision::Rejected,
                    _ => return None,
                }
            } else if let Some(ok) = payload.get("confirmed").and_then(Value::as_bool) {
                if ok {
                    TermAliasDecision::Confirmed
                } else {
                    TermAliasDecision::Rejected
                }
            } else {
                let ck = payload.get("kind").and_then(Value::as_str)?;
                match ck {
                    "other_name_confirmed" => TermAliasDecision::Confirmed,
                    "other_name_rejected" => TermAliasDecision::Rejected,
                    _ => return None,
                }
            }
        }
    };
    let origin_node = payload
        .get("origin_node")
        .or_else(|| payload.get("node_ref"))
        .and_then(|v| serde_json::from_value::<NodeRef>(v.clone()).ok());

    Some(TermAliasRecord {
        term_a: a,
        term_b: b,
        decision,
        origin_session: session_id.to_string(),
        origin_node,
        updated_ms: ts_ms,
    })
}

fn extract_solve_defaults(
    session_id: &str,
    payload: &Value,
    ts_ms: i64,
) -> (Vec<SymbolDefault>, Vec<ConventionDefault>) {
    let domain = str_field(payload, &["domain", "field"]).unwrap_or_default();
    let mut syms = Vec::new();
    let mut convs = Vec::new();

    if let Some(arr) = payload.get("symbols").and_then(Value::as_array) {
        for item in arr {
            let Some(name) = str_field(item, &["symbol", "name"]) else {
                continue;
            };
            let assumptions = item
                .get("assumptions")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let dimensions = str_field(item, &["dimensions", "dimension", "units"]);
            let convention_note = str_field(item, &["convention", "convention_note", "note"]);
            syms.push(SymbolDefault {
                symbol: name,
                domain: domain.clone(),
                assumptions,
                dimensions,
                convention_note,
                origin_session: session_id.to_string(),
                updated_ms: ts_ms,
            });
        }
    }

    if let Some(obj) = payload.get("conventions").and_then(Value::as_object) {
        for (k, v) in obj {
            let val_str = match v {
                Value::String(s) => s.trim().to_string(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            if !k.trim().is_empty() && !val_str.is_empty() {
                convs.push(ConventionDefault {
                    key: k.trim().to_string(),
                    value: val_str,
                    domain: domain.clone(),
                    origin_session: session_id.to_string(),
                    updated_ms: ts_ms,
                });
            }
        }
    } else if let Some(text) = payload.get("conventions").and_then(Value::as_str) {
        let t = text.trim();
        if !t.is_empty() {
            convs.push(ConventionDefault {
                key: "conventions".to_string(),
                value: t.to_string(),
                domain,
                origin_session: session_id.to_string(),
                updated_ms: ts_ms,
            });
        }
    }

    (syms, convs)
}

fn extract_skeleton_override(
    session_id: &str,
    payload: &Value,
    ts_ms: i64,
) -> Option<SkeletonOverride> {
    let node_tag = str_field(payload, &["node_tag", "tag"])?;
    let kind_override = str_field(payload, &["kind_override", "kind"]);
    let end_line_override = payload
        .get("end_line_override")
        .or_else(|| payload.get("end_line"))
        .and_then(Value::as_u64)
        .filter(|&n| n >= 1)
        .map(|n| n as usize);
    let note = str_field(payload, &["note", "reason"]).unwrap_or_default();
    if kind_override.is_none() && end_line_override.is_none() && note.is_empty() {
        return None;
    }
    let paper_key = str_field(payload, &["paper_key", "key"])
        .and_then(|k| PaperKey::parse(&k).ok());
    Some(SkeletonOverride {
        node_tag,
        paper_key,
        kind_override,
        end_line_override,
        note,
        origin_session: session_id.to_string(),
        updated_ms: ts_ms,
    })
}

fn extract_ocr_overlay(
    session_id: &str,
    payload: &Value,
    ts_ms: i64,
) -> Option<OcrOverlay> {
    let file_sha256 = str_field(payload, &["file_sha256", "sha256"])?;
    let line_no = payload
        .get("line_no")
        .or_else(|| payload.get("line"))
        .and_then(Value::as_u64)
        .filter(|&n| n >= 1)? as usize;
    let corrected_text = str_field(payload, &["corrected_text", "text"])?;
    let original_sha256 = str_field(payload, &["original_sha256"]);
    Some(OcrOverlay {
        file_sha256,
        line_no,
        corrected_text,
        original_sha256,
        origin_session: session_id.to_string(),
        updated_ms: ts_ms,
        label: OCR_OVERLAY_LABEL.to_string(),
    })
}

fn str_field(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(Value::as_str).map(str::trim) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}
