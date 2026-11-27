//! F16b user-approved lessons SQLite storage (`user_lesson`, schema v2).
//!
//! Extends `library/lessons.db` forward-only from v1 (`F16a`) to v2 (`F16b`),
//! preserving all F16a rows untouched during migration.

use crate::lessons_db::LessonsDb;
use crate::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::io;

pub(crate) const USER_LESSON_SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS user_lesson (
    id TEXT PRIMARY KEY,
    text TEXT NOT NULL CHECK (length(trim(text)) > 0),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('global', 'mode', 'field', 'mode_field', 'paper')),
    scope_mode TEXT,
    scope_field TEXT,
    scope_paper_key TEXT,
    from_node_json TEXT NOT NULL CHECK (json_valid(from_node_json)),
    source_kind TEXT NOT NULL,
    model_summary INTEGER NOT NULL DEFAULT 0 CHECK (model_summary IN (0, 1)),
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    uses INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0)
);
CREATE INDEX IF NOT EXISTS idx_user_lesson_scope
    ON user_lesson (scope_kind, scope_mode, scope_field, scope_paper_key);
CREATE INDEX IF NOT EXISTS idx_user_lesson_created
    ON user_lesson (created_ms DESC, id ASC);
"#;

const ALLOWED_SCOPE_KINDS: &[&str] = &["global", "mode", "field", "mode_field", "paper"];

/// Raw persisted row in `user_lesson` (`lessons.db` schema v2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserLessonRow {
    pub id: String,
    pub text: String,
    pub scope_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_paper_key: Option<String>,
    pub from_node_json: String,
    pub source_kind: String,
    #[serde(default)]
    pub model_summary: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
    #[serde(default)]
    pub uses: u64,
}

pub(crate) fn migrate_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(USER_LESSON_SCHEMA_V2)?;
    Ok(())
}

fn invalid_input(msg: impl Into<String>) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::InvalidInput, msg.into()))
}

fn validate_row_fields(
    id: &str,
    text: &str,
    scope_kind: &str,
    from_node_json: &str,
    source_kind: &str,
    uses: u64,
) -> Result<i64> {
    if id.trim().is_empty() {
        return Err(invalid_input("user lesson id must be non-empty"));
    }
    if text.trim().is_empty() {
        return Err(invalid_input("user lesson text must be non-empty"));
    }
    if !ALLOWED_SCOPE_KINDS.contains(&scope_kind) {
        return Err(invalid_input(format!(
            "unsupported user lesson scope_kind: {scope_kind}"
        )));
    }
    if source_kind.trim().is_empty() {
        return Err(invalid_input("user lesson source_kind must be non-empty"));
    }
    serde_json::from_str::<serde_json::Value>(from_node_json)
        .map_err(|e| invalid_input(format!("invalid from_node_json: {e}")))?;
    i64::try_from(uses).map_err(|_| invalid_input("user lesson uses overflows i64"))
}

fn with_savepoint<T>(
    conn: &Connection,
    name: &str,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    conn.execute_batch(&format!("SAVEPOINT {name};"))?;
    match f(conn) {
        Ok(val) => {
            conn.execute_batch(&format!("RELEASE SAVEPOINT {name};"))?;
            Ok(val)
        }
        Err(err) => {
            let _ = conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {name}; RELEASE SAVEPOINT {name};"
            ));
            Err(err)
        }
    }
}

fn row_from_sqlite(r: &rusqlite::Row<'_>) -> rusqlite::Result<UserLessonRow> {
    let model_summary_int: i64 = r.get(8)?;
    let uses_int: i64 = r.get(11)?;
    Ok(UserLessonRow {
        id: r.get(0)?,
        text: r.get(1)?,
        scope_kind: r.get(2)?,
        scope_mode: r.get(3)?,
        scope_field: r.get(4)?,
        scope_paper_key: r.get(5)?,
        from_node_json: r.get(6)?,
        source_kind: r.get(7)?,
        model_summary: model_summary_int != 0,
        created_ms: r.get(9)?,
        updated_ms: r.get(10)?,
        uses: u64::try_from(uses_int).unwrap_or(0),
    })
}

impl LessonsDb {
    /// Insert a new F16b user-approved lesson row. Fails on duplicate `id`.
    pub fn insert_user_lesson(&self, row: &UserLessonRow) -> Result<()> {
        let uses_i64 = validate_row_fields(
            &row.id,
            &row.text,
            &row.scope_kind,
            &row.from_node_json,
            &row.source_kind,
            row.uses,
        )?;
        self.conn().execute(
            "INSERT INTO user_lesson (
                id, text, scope_kind, scope_mode, scope_field, scope_paper_key,
                from_node_json, source_kind, model_summary, created_ms, updated_ms, uses
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                row.id,
                row.text,
                row.scope_kind,
                row.scope_mode,
                row.scope_field,
                row.scope_paper_key,
                row.from_node_json,
                row.source_kind,
                if row.model_summary { 1_i64 } else { 0_i64 },
                row.created_ms,
                row.updated_ms,
                uses_i64,
            ],
        )?;
        Ok(())
    }

    /// Fetch a single F16b user lesson by `id`.
    pub fn get_user_lesson(&self, id: &str) -> Result<Option<UserLessonRow>> {
        let row = self
            .conn()
            .query_row(
                "SELECT id, text, scope_kind, scope_mode, scope_field, scope_paper_key,
                        from_node_json, source_kind, model_summary, created_ms, updated_ms, uses
                 FROM user_lesson
                 WHERE id = ?1",
                params![id],
                row_from_sqlite,
            )
            .optional()?;
        Ok(row)
    }

    /// List all F16b user lessons deterministically (`created_ms DESC, id ASC`).
    pub fn list_user_lessons(&self) -> Result<Vec<UserLessonRow>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, text, scope_kind, scope_mode, scope_field, scope_paper_key,
                    from_node_json, source_kind, model_summary, created_ms, updated_ms, uses
             FROM user_lesson
             ORDER BY created_ms DESC, id ASC",
        )?;
        let rows = stmt
            .query_map([], row_from_sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Update an existing F16b lesson's text, scope, and optional `model_summary` flag.
    #[allow(clippy::too_many_arguments)]
    pub fn update_user_lesson(
        &self,
        id: &str,
        text: &str,
        scope_kind: &str,
        scope_mode: Option<&str>,
        scope_field: Option<&str>,
        scope_paper_key: Option<&str>,
        model_summary: Option<bool>,
        updated_ms: i64,
    ) -> Result<bool> {
        let _ = validate_row_fields(id, text, scope_kind, "{}", "edit", 0)?;
        let changed = match model_summary {
            Some(flag) => self.conn().execute(
                "UPDATE user_lesson
                 SET text = ?2, scope_kind = ?3, scope_mode = ?4, scope_field = ?5,
                     scope_paper_key = ?6, model_summary = ?7, updated_ms = ?8
                 WHERE id = ?1",
                params![
                    id,
                    text,
                    scope_kind,
                    scope_mode,
                    scope_field,
                    scope_paper_key,
                    if flag { 1_i64 } else { 0_i64 },
                    updated_ms,
                ],
            )?,
            None => self.conn().execute(
                "UPDATE user_lesson
                 SET text = ?2, scope_kind = ?3, scope_mode = ?4, scope_field = ?5,
                     scope_paper_key = ?6, updated_ms = ?7
                 WHERE id = ?1",
                params![
                    id,
                    text,
                    scope_kind,
                    scope_mode,
                    scope_field,
                    scope_paper_key,
                    updated_ms,
                ],
            )?,
        };
        Ok(changed > 0)
    }

    /// Delete an F16b lesson by `id`. Returns `true` if a row existed.
    pub fn delete_user_lesson(&self, id: &str) -> Result<bool> {
        let changed = self
            .conn()
            .execute("DELETE FROM user_lesson WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    /// Increment `uses` once per unique id in `ids` inside a savepoint transaction.
    pub fn increment_user_lesson_uses(&self, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        with_savepoint(self.conn(), "f16b_inc_uses", |conn| {
            let mut stmt =
                conn.prepare("UPDATE user_lesson SET uses = uses + 1 WHERE id = ?1")?;
            let mut updated = 0usize;
            let mut seen = std::collections::BTreeSet::new();
            for id in ids {
                if !seen.insert(id.as_str()) {
                    continue;
                }
                updated += stmt.execute(params![id])?;
            }
            Ok(updated)
        })
    }

    /// Restore F16b user lessons from backup (merging or replacing existing rows cleanly).
    pub fn restore_user_lessons(&self, rows: &[UserLessonRow], replace: bool) -> Result<usize> {
        for r in rows {
            let _ = validate_row_fields(
                &r.id,
                &r.text,
                &r.scope_kind,
                &r.from_node_json,
                &r.source_kind,
                r.uses,
            )?;
        }
        with_savepoint(self.conn(), "f16b_restore", |conn| {
            if replace {
                conn.execute("DELETE FROM user_lesson", [])?;
            }
            let mut stmt = conn.prepare(
                "INSERT INTO user_lesson (
                    id, text, scope_kind, scope_mode, scope_field, scope_paper_key,
                    from_node_json, source_kind, model_summary, created_ms, updated_ms, uses
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                    text = excluded.text,
                    scope_kind = excluded.scope_kind,
                    scope_mode = excluded.scope_mode,
                    scope_field = excluded.scope_field,
                    scope_paper_key = excluded.scope_paper_key,
                    from_node_json = excluded.from_node_json,
                    source_kind = excluded.source_kind,
                    model_summary = excluded.model_summary,
                    created_ms = excluded.created_ms,
                    updated_ms = excluded.updated_ms,
                    uses = excluded.uses",
            )?;
            let mut count = 0usize;
            for row in rows {
                let uses_i64 = i64::try_from(row.uses)
                    .map_err(|_| invalid_input("user lesson uses overflows i64"))?;
                stmt.execute(params![
                    row.id,
                    row.text,
                    row.scope_kind,
                    row.scope_mode,
                    row.scope_field,
                    row.scope_paper_key,
                    row.from_node_json,
                    row.source_kind,
                    if row.model_summary { 1_i64 } else { 0_i64 },
                    row.created_ms,
                    row.updated_ms,
                    uses_i64,
                ])?;
                count += 1;
            }
            Ok(count)
        })
    }
}

#[cfg(test)]
#[path = "lessons_b_store_tests.rs"]
mod tests;
