//! Durable storage and portable checksummed backup/restore for non-rebuildable user marks
//! (`00-overview.md` section 8.2, decision 5; `explore.md` section 4.3).
//!
//! User marks are indexed in `<data_dir>/library/library.db` (`user_mark` table) for fast
//! querying, and durably mirrored in `<data_dir>/library/user_marks.json` (with a rolling
//! previous-generation copy at `<data_dir>/library/user_marks.json.bak`). Deleting and
//! rebuilding `library.db` restores every user mark from the sidecar file automatically.

use crate::library_schema::{
    MarkStatus, UserMark, UserMarksBackup, MAX_BACKUP_BYTES, MAX_BACKUP_MARKS, MAX_NOTE_CHARS,
    MAX_TAGS_PER_PAPER, MAX_TAG_CHARS, USER_MARKS_BACKUP_NAME, USER_MARKS_BACKUP_SCHEMA_VERSION,
    USER_MARKS_FILE_NAME,
};
use crate::paper_key::PaperKey;
use ledger::{canonical_json, sha256_hex};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_MARK_TMP: AtomicU64 = AtomicU64::new(0);

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Normalize and validate a user mark before persisting.
pub(crate) fn validate_and_normalize_mark(
    key: PaperKey,
    status: MarkStatus,
    tags: Vec<String>,
    note: String,
    updated_ms: i64,
) -> Result<UserMark, String> {
    let mut clean_tags = Vec::new();
    let mut seen_lower = BTreeSet::new();
    for raw in tags {
        let trimmed: String = raw
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > MAX_TAG_CHARS {
            return Err(format!("user mark tag exceeds {MAX_TAG_CHARS} characters"));
        }
        let lower = trimmed.to_ascii_lowercase();
        if seen_lower.insert(lower) {
            clean_tags.push(trimmed);
        }
    }
    if clean_tags.len() > MAX_TAGS_PER_PAPER {
        return Err(format!(
            "user mark has {} tags (maximum {MAX_TAGS_PER_PAPER})",
            clean_tags.len()
        ));
    }
    let clean_note: String = note
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    if clean_note.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("user mark note exceeds {MAX_NOTE_CHARS} characters"));
    }
    Ok(UserMark {
        key,
        status,
        tags: clean_tags,
        note: clean_note,
        updated_ms,
    })
}

/// Deterministic SHA-256 digest over the canonical JSON encoding of a sorted mark slice.
pub(crate) fn marks_digest(sorted_marks: &[UserMark]) -> Result<String, String> {
    let value = serde_json::to_value(sorted_marks).map_err(|e| e.to_string())?;
    let canon = canonical_json(&value);
    Ok(sha256_hex(canon.as_bytes()))
}

/// Build a portable, checksummed [`UserMarksBackup`] envelope from `marks`.
pub(crate) fn build_backup(marks: &[UserMark]) -> Result<UserMarksBackup, String> {
    if marks.len() > MAX_BACKUP_MARKS {
        return Err(format!(
            "user marks count {} exceeds backup limit {MAX_BACKUP_MARKS}",
            marks.len()
        ));
    }
    let mut sorted = marks.to_vec();
    sorted.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));
    let sha256 = marks_digest(&sorted)?;
    Ok(UserMarksBackup {
        schema_version: USER_MARKS_BACKUP_SCHEMA_VERSION,
        exported_ms: now_ms(),
        sha256,
        marks: sorted,
    })
}

/// Serialize `marks` as pretty-printed UTF-8 JSON inside a checksummed [`UserMarksBackup`].
pub(crate) fn export_backup_bytes(marks: &[UserMark]) -> Result<Vec<u8>, String> {
    let envelope = build_backup(marks)?;
    let bytes = serde_json::to_vec_pretty(&envelope).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(format!("user marks backup exceeds {MAX_BACKUP_BYTES} bytes"));
    }
    Ok(bytes)
}

/// Parse, validate, and verify the SHA-256 checksum of a [`UserMarksBackup`] payload.
pub(crate) fn parse_and_verify_backup(bytes: &[u8]) -> Result<UserMarksBackup, String> {
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(format!("user marks backup exceeds {MAX_BACKUP_BYTES} bytes"));
    }
    let env: UserMarksBackup =
        serde_json::from_slice(bytes).map_err(|e| format!("invalid user marks backup JSON: {e}"))?;
    if env.schema_version != USER_MARKS_BACKUP_SCHEMA_VERSION {
        return Err(format!(
            "unsupported user marks backup schema_version {} (expected {USER_MARKS_BACKUP_SCHEMA_VERSION})",
            env.schema_version
        ));
    }
    if env.marks.len() > MAX_BACKUP_MARKS {
        return Err(format!(
            "user marks backup has {} entries (maximum {MAX_BACKUP_MARKS})",
            env.marks.len()
        ));
    }
    let mut by_key = BTreeMap::new();
    for m in &env.marks {
        let normalized = validate_and_normalize_mark(
            m.key.clone(),
            m.status,
            m.tags.clone(),
            m.note.clone(),
            m.updated_ms,
        )?;
        if by_key.insert(normalized.key.to_string(), normalized).is_some() {
            return Err(format!("duplicate paper key `{}` in user marks backup", m.key));
        }
    }
    // Digest is computed over the serialized marks array in the envelope (canonical JSON).
    let expected = marks_digest(&env.marks)?;
    if expected != env.sha256 {
        return Err("user marks backup checksum mismatch (corrupted or tampered file)".into());
    }
    let marks = by_key.into_values().collect();
    Ok(UserMarksBackup {
        schema_version: env.schema_version,
        exported_ms: env.exported_ms,
        sha256: env.sha256,
        marks,
    })
}

pub(crate) fn primary_path(library_dir: &Path) -> PathBuf {
    library_dir.join(USER_MARKS_FILE_NAME)
}

pub(crate) fn backup_path(library_dir: &Path) -> PathBuf {
    library_dir.join(USER_MARKS_BACKUP_NAME)
}

/// Atomically persist `marks` to `<library_dir>/user_marks.json`, rotating any previous valid
/// primary copy to `<library_dir>/user_marks.json.bak` first.
pub(crate) fn save_sidecar(library_dir: &Path, marks: &[UserMark]) -> Result<(), String> {
    std::fs::create_dir_all(library_dir).map_err(|e| format!("cannot create library dir: {e}"))?;
    let bytes = export_backup_bytes(marks)?;
    let primary = primary_path(library_dir);
    let backup = backup_path(library_dir);

    if let Ok(prev) = std::fs::read(&primary) {
        if parse_and_verify_backup(&prev).is_ok() {
            atomic_replace_file(&backup, &prev)?;
        }
    }
    atomic_replace_file(&primary, &bytes)
}

/// Load user marks from `<library_dir>/user_marks.json`, falling back to
/// `<library_dir>/user_marks.json.bak` when the primary is missing or fails checksum verification.
pub(crate) fn load_sidecar(library_dir: &Path) -> Result<Option<Vec<UserMark>>, String> {
    let primary = primary_path(library_dir);
    let backup = backup_path(library_dir);

    let mut primary_err: Option<String> = None;
    match std::fs::read(&primary) {
        Ok(bytes) => match parse_and_verify_backup(&bytes) {
            Ok(env) => return Ok(Some(env.marks)),
            Err(e) => primary_err = Some(e),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => primary_err = Some(format!("cannot read `{}`: {e}", primary.display())),
    }

    match std::fs::read(&backup) {
        Ok(bytes) => match parse_and_verify_backup(&bytes) {
            Ok(env) => Ok(Some(env.marks)),
            Err(bak_err) => {
                if let Some(p_err) = primary_err {
                    Err(format!(
                        "user marks primary failed ({p_err}) and backup failed ({bak_err})"
                    ))
                } else {
                    Err(format!("user marks backup failed ({bak_err})"))
                }
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => match primary_err {
            Some(p_err) => Err(format!("user marks sidecar unreadable ({p_err}) and no backup found")),
            None => Ok(None),
        },
        Err(e) => Err(format!("cannot read `{}`: {e}", backup.display())),
    }
}

/// Cross-platform (Linux + Windows) temp-file + replace write.
fn atomic_replace_file(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("target `{}` has no parent directory", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create dir: {e}"))?;
    let (tmp, mut file) = loop {
        let seq = NEXT_MARK_TMP.fetch_add(1, Ordering::Relaxed);
        let tmp = target.with_extension(format!("tmp{}.{seq}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create temp file `{}`: {e}", tmp.display())),
        }
    };
    let write_res = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot write `{}`: {e}", target.display()));
    }
    if let Err(first_err) = std::fs::rename(&tmp, target) {
        // On Windows, renaming onto an existing destination fails with AlreadyExists/PermissionDenied.
        if target.exists() {
            let _ = std::fs::remove_file(target);
            if let Err(second_err) = std::fs::rename(&tmp, target) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!(
                    "cannot replace `{}`: {second_err} (initial: {first_err})",
                    target.display()
                ));
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("cannot move `{}` into place: {first_err}", target.display()));
        }
    }
    Ok(())
}
