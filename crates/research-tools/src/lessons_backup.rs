//! F16b opt-in, SHA-256 checksummed portable backup and restore for user lessons
//! (`00-overview.md` 2.14, 8.2 #5).

use crate::lessons_capture::{now_ms, UserLesson, UserLessonsStore};
use ledger::{canonical_json, sha256_hex};
use serde::{Deserialize, Serialize};

/// Version of the portable F16b lessons backup envelope.
pub const LESSONS_B_BACKUP_SCHEMA_VERSION: u32 = 1;
/// Hard ceiling on restored F16b backup JSON payloads (16 MiB).
pub const MAX_LESSONS_B_BACKUP_BYTES: usize = 16 * 1024 * 1024;

/// Opt-in flag controlling portable lesson export (`00-overview.md` 2.14 & `draft-f16b-lessons.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptInLessonsExport {
    #[serde(default)]
    pub include_lessons: bool,
}

/// Checksummed portable backup envelope for F16b user lessons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserLessonsBackup {
    pub schema_version: u32,
    pub exported_ms: i64,
    pub checksum_sha256: String,
    pub lessons: Vec<UserLesson>,
}

pub fn compute_lessons_backup_checksum(lessons: &[UserLesson]) -> Result<String, String> {
    let val = serde_json::to_value(lessons)
        .map_err(|e| format!("Failed to serialize lessons for checksum: {e}"))?;
    let canon = canonical_json(&val);
    Ok(sha256_hex(canon.as_bytes()))
}

/// Export user lessons as SHA-256 checksummed JSON ONLY when `opt_in.include_lessons == true`.
pub fn export_lessons_backup(
    store: &UserLessonsStore,
    opt_in: OptInLessonsExport,
) -> Result<Option<Vec<u8>>, String> {
    if !opt_in.include_lessons {
        return Ok(None);
    }
    let mut lessons = store.list_lessons()?;
    lessons.sort_by(|a, b| a.created_ms.cmp(&b.created_ms).then_with(|| a.id.cmp(&b.id)));
    let checksum_sha256 = compute_lessons_backup_checksum(&lessons)?;
    let env = UserLessonsBackup {
        schema_version: LESSONS_B_BACKUP_SCHEMA_VERSION,
        exported_ms: now_ms(),
        checksum_sha256,
        lessons,
    };
    let bytes = serde_json::to_vec_pretty(&env)
        .map_err(|e| format!("Failed to encode lessons backup: {e}"))?;
    Ok(Some(bytes))
}

/// Verify checksum and validate every lesson before restoring into `store`.
pub fn restore_lessons_backup(
    store: &UserLessonsStore,
    bytes: &[u8],
    replace: bool,
) -> Result<usize, String> {
    if bytes.len() > MAX_LESSONS_B_BACKUP_BYTES {
        return Err(format!(
            "Lessons backup exceeds max size ({} bytes).",
            MAX_LESSONS_B_BACKUP_BYTES
        ));
    }
    let parsed: UserLessonsBackup =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid lessons backup JSON: {e}"))?;
    if parsed.schema_version != LESSONS_B_BACKUP_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported lessons backup schema version: {}",
            parsed.schema_version
        ));
    }
    let expected = compute_lessons_backup_checksum(&parsed.lessons)?;
    if expected != parsed.checksum_sha256 {
        return Err("Lessons backup checksum mismatch (corrupted or tampered file).".to_string());
    }
    let mut rows = Vec::with_capacity(parsed.lessons.len());
    for lesson in &parsed.lessons {
        rows.push(lesson.to_row()?);
    }
    store
        .db()
        .restore_user_lessons(&rows, replace)
        .map_err(|e| format!("Failed to restore user lessons: {e}"))
}
