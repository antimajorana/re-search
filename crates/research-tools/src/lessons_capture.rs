//! F16b user-approved lessons capture, validation, "Remember this?" prefill,
//! opt-in model summary tagging, and checksummed portable backup/restore.
//!
//! Invariants (`00-overview.md` 2.14, 8.2 #5, 8.3 #7):
//! - Zero silent auto-save: every stored lesson is explicitly approved by the user.
//! - Zero automatic model calls on correction: the prefill uses the user's own words;
//!   an optional model summary is built only on explicit click and tagged `(model)`.
//! - Every lesson is bounded to `<= 300` Unicode chars and records its `from: NodeRef`.
//! - Backup / export is opt-in and SHA-256 checksummed.

use crate::paper_key::{Mode, NodeRef, PaperKey};
use crate::quote_core::clip_to_chars;
use ledger::{LessonsDb, UserLessonRow, LESSONS_DB_NAME};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::lessons_backup::{
    compute_lessons_backup_checksum, export_lessons_backup, restore_lessons_backup,
    OptInLessonsExport, UserLessonsBackup, LESSONS_B_BACKUP_SCHEMA_VERSION,
    MAX_LESSONS_B_BACKUP_BYTES,
};
pub use crate::lessons_draft::{
    build_remember_draft, build_summary_prompt, finalize_model_summary_draft,
    CorrectionOriginContext, RememberPromptDraft, DRAFT_SUMMARY_SYSTEM_PROMPT,
};

/// Hard Unicode scalar cap for every stored user lesson (`00-overview.md` 2.14 & Set K).
pub const MAX_LESSON_CHARS: usize = 300;
/// Suffix appended to opt-in model-drafted lesson summaries.
pub const MODEL_SUMMARY_TAG: &str = "(model)";
/// Maximum normalized field label length.
pub const MAX_FIELD_NAME_CHARS: usize = 120;

static ID_SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Strip ASCII control characters (`\r`, `\n`, `\t` become spaces) and trim edges.
pub fn normalize_lesson_input(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_space = false;
    for ch in raw.chars() {
        let mapped = match ch {
            '\r' | '\n' | '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if mapped == ' ' {
            if !out.is_empty() && !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(mapped);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Validate and normalize user-entered or edited lesson text (`1..=300` Unicode chars).
pub fn validate_lesson_text(raw: &str) -> Result<String, String> {
    let cleaned = normalize_lesson_input(raw);
    if cleaned.is_empty() {
        return Err("Lesson text cannot be empty.".to_string());
    }
    let count = cleaned.chars().count();
    if count > MAX_LESSON_CHARS {
        return Err(format!(
            "Lesson text exceeds {MAX_LESSON_CHARS} characters (got {count})."
        ));
    }
    Ok(cleaned)
}

/// Validate and normalize a physics/research field tag (trimmed lowercase).
pub fn normalize_field_name(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let cleaned = normalize_lesson_input(raw).to_lowercase();
    if cleaned.is_empty() {
        return None;
    }
    Some(clip_to_chars(&cleaned, MAX_FIELD_NAME_CHARS).to_string())
}

pub fn parse_mode_str(raw: &str) -> Result<Mode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "investigate" => Ok(Mode::Investigate),
        "explore" => Ok(Mode::Explore),
        "solve" => Ok(Mode::Solve),
        "research" => Ok(Mode::Research),
        other => Err(format!("Unsupported mode for lesson scope: {other}")),
    }
}

/// Scope kind reconciliation (`00-overview.md` 2.14 + 8.3 #7 compound `(mode, field)` default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonScopeKind {
    Global,
    Mode,
    Field,
    ModeField,
    Paper,
}

impl LessonScopeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Mode => "mode",
            Self::Field => "field",
            Self::ModeField => "mode_field",
            Self::Paper => "paper",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "global" => Ok(Self::Global),
            "mode" => Ok(Self::Mode),
            "field" => Ok(Self::Field),
            "mode_field" | "mode+field" => Ok(Self::ModeField),
            "paper" => Ok(Self::Paper),
            other => Err(format!("Invalid lesson scope kind: {other}")),
        }
    }
}

/// Applicability scope attached to a user-approved lesson.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LessonScope {
    pub kind: LessonScopeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<PaperKey>,
}

impl LessonScope {
    pub fn global() -> Self {
        Self { kind: LessonScopeKind::Global, mode: None, field: None, paper_key: None }
    }

    pub fn mode(mode: Mode) -> Self {
        Self { kind: LessonScopeKind::Mode, mode: Some(mode), field: None, paper_key: None }
    }

    pub fn field(field: impl Into<String>) -> Self {
        Self { kind: LessonScopeKind::Field, mode: None, field: Some(field.into()), paper_key: None }
    }

    pub fn mode_field(mode: Mode, field: impl Into<String>) -> Self {
        Self {
            kind: LessonScopeKind::ModeField,
            mode: Some(mode),
            field: Some(field.into()),
            paper_key: None,
        }
    }

    pub fn paper(paper_key: PaperKey) -> Self {
        Self { kind: LessonScopeKind::Paper, mode: None, field: None, paper_key: Some(paper_key) }
    }

    /// Owner decision (`00-overview.md` 8.3 #7): default to `"this mode and this field"` when
    /// a field is known, cleanly falling back to `"this mode"` if field is unknown.
    pub fn default_for_correction(mode: Mode, field: Option<&str>) -> Self {
        match normalize_field_name(field) {
            Some(f) => Self::mode_field(mode, f),
            None => Self::mode(mode),
        }
    }

    pub fn validated(&self) -> Result<Self, String> {
        let field = normalize_field_name(self.field.as_deref());
        match self.kind {
            LessonScopeKind::Global => Ok(Self::global()),
            LessonScopeKind::Mode => {
                let m = self.mode.ok_or("Mode scope requires `mode`.")?;
                Ok(Self::mode(m))
            }
            LessonScopeKind::Field => {
                let f = field.ok_or("Field scope requires a non-empty `field`.")?;
                Ok(Self::field(f))
            }
            LessonScopeKind::ModeField => {
                let m = self.mode.ok_or("Mode+field scope requires `mode`.")?;
                let f = field.ok_or("Mode+field scope requires a non-empty `field`.")?;
                Ok(Self::mode_field(m, f))
            }
            LessonScopeKind::Paper => {
                let pk = self.paper_key.clone().ok_or("Paper scope requires `paper_key`.")?;
                let parsed = PaperKey::parse(pk.as_str())?;
                Ok(Self::paper(parsed))
            }
        }
    }

    /// Higher number = more specific scope (`paper` > `mode_field` > `field` > `mode` > `global`).
    pub fn specificity(&self) -> u8 {
        match self.kind {
            LessonScopeKind::Paper => 5,
            LessonScopeKind::ModeField => 4,
            LessonScopeKind::Field => 3,
            LessonScopeKind::Mode => 2,
            LessonScopeKind::Global => 1,
        }
    }

    pub fn label(&self) -> String {
        match self.kind {
            LessonScopeKind::Global => "Global".to_string(),
            LessonScopeKind::Mode => {
                let m = self.mode.map(Mode::as_str).unwrap_or("mode");
                format!("{m} mode")
            }
            LessonScopeKind::Field => {
                let f = self.field.as_deref().unwrap_or("field");
                format!("Field: {f}")
            }
            LessonScopeKind::ModeField => {
                let m = self.mode.map(Mode::as_str).unwrap_or("mode");
                let f = self.field.as_deref().unwrap_or("field");
                format!("{m} + {f}")
            }
            LessonScopeKind::Paper => {
                let pk = self.paper_key.as_ref().map(PaperKey::as_str).unwrap_or("paper");
                format!("Paper: {pk}")
            }
        }
    }
}

/// Origin category for a user-approved lesson.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonSourceKind {
    #[default]
    StepCorrection,
    SkeletonCorrection,
    ExploreCorrection,
    ConventionCorrection,
    ManualNote,
}

impl LessonSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StepCorrection => "step_correction",
            Self::SkeletonCorrection => "skeleton_correction",
            Self::ExploreCorrection => "explore_correction",
            Self::ConventionCorrection => "convention_correction",
            Self::ManualNote => "manual_note",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "step_correction" => Self::StepCorrection,
            "skeleton_correction" => Self::SkeletonCorrection,
            "explore_correction" => Self::ExploreCorrection,
            "convention_correction" => Self::ConventionCorrection,
            "manual_note" => Self::ManualNote,
            _ => Self::StepCorrection,
        }
    }
}

/// Complete domain record for one F16b user-approved lesson.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserLesson {
    pub id: String,
    pub text: String,
    pub scope: LessonScope,
    pub from: NodeRef,
    #[serde(default)]
    pub source_kind: LessonSourceKind,
    #[serde(default)]
    pub model_summary: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
    #[serde(default)]
    pub uses: u64,
}

pub fn validate_node_ref(node: &NodeRef) -> Result<(), String> {
    if node.session.trim().is_empty() {
        return Err("Lesson `from.session` cannot be empty.".to_string());
    }
    if node.id.trim().is_empty() {
        return Err("Lesson `from.id` cannot be empty.".to_string());
    }
    if node.session.chars().any(char::is_control) || node.id.chars().any(char::is_control) {
        return Err("Lesson `from` identifiers cannot contain control characters.".to_string());
    }
    Ok(())
}

impl UserLesson {
    pub fn to_row(&self) -> Result<UserLessonRow, String> {
        let text = validate_lesson_text(&self.text)?;
        let scope = self.scope.validated()?;
        validate_node_ref(&self.from)?;
        let from_node_json = serde_json::to_string(&self.from)
            .map_err(|e| format!("Failed to encode lesson origin NodeRef: {e}"))?;
        Ok(UserLessonRow {
            id: self.id.clone(),
            text,
            scope_kind: scope.kind.as_str().to_string(),
            scope_mode: scope.mode.map(|m| m.as_str().to_string()),
            scope_field: scope.field.clone(),
            scope_paper_key: scope.paper_key.as_ref().map(|pk| pk.as_str().to_string()),
            from_node_json,
            source_kind: self.source_kind.as_str().to_string(),
            model_summary: self.model_summary,
            created_ms: self.created_ms,
            updated_ms: self.updated_ms,
            uses: self.uses,
        })
    }

    pub fn from_row(row: &UserLessonRow) -> Result<Self, String> {
        let kind = LessonScopeKind::parse(&row.scope_kind)?;
        let mode = row.scope_mode.as_deref().map(parse_mode_str).transpose()?;
        let paper_key = row
            .scope_paper_key
            .as_deref()
            .map(PaperKey::parse)
            .transpose()?;
        let scope = LessonScope {
            kind,
            mode,
            field: row.scope_field.clone(),
            paper_key,
        }
        .validated()?;
        let from: NodeRef = serde_json::from_str(&row.from_node_json)
            .map_err(|e| format!("Invalid stored `from_node_json` on {}: {e}", row.id))?;
        Ok(Self {
            id: row.id.clone(),
            text: validate_lesson_text(&row.text)?,
            scope,
            from,
            source_kind: LessonSourceKind::parse(&row.source_kind),
            model_summary: row.model_summary,
            created_ms: row.created_ms,
            updated_ms: row.updated_ms,
            uses: row.uses,
        })
    }
}

/// Explicit request payload to save an F16b lesson after user confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveLessonRequest {
    pub text: String,
    pub scope: LessonScope,
    pub from: NodeRef,
    #[serde(default)]
    pub source_kind: LessonSourceKind,
    #[serde(default)]
    pub model_summary: bool,
    #[serde(default)]
    pub created_ms: Option<i64>,
}

/// High-level store for F16b user-approved lessons in `<data_dir>/library/lessons.db`.
pub struct UserLessonsStore {
    db: LessonsDb,
}

impl UserLessonsStore {
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        let db_path = data_dir.join("library").join(LESSONS_DB_NAME);
        Self::open_at(&db_path)
    }

    pub fn open_at(db_path: &Path) -> Result<Self, String> {
        let db = LessonsDb::open(db_path).map_err(|e| format!("Failed to open lessons.db: {e}"))?;
        Ok(Self { db })
    }

    pub fn db(&self) -> &LessonsDb {
        &self.db
    }

    pub fn save_lesson(&self, req: SaveLessonRequest) -> Result<UserLesson, String> {
        let text = validate_lesson_text(&req.text)?;
        let scope = req.scope.validated()?;
        validate_node_ref(&req.from)?;
        let ts = req.created_ms.unwrap_or_else(now_ms);
        let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
        let mut h = Sha256::new();
        h.update(ts.to_le_bytes());
        h.update(seq.to_le_bytes());
        h.update(req.from.session.as_bytes());
        h.update(b":");
        h.update(req.from.id.as_bytes());
        h.update(b":");
        h.update(text.as_bytes());
        let digest = hex::encode(h.finalize());
        let id = format!("les-{ts}-{}", &digest[..12]);
        let lesson = UserLesson {
            id,
            text,
            scope,
            from: req.from,
            source_kind: req.source_kind,
            model_summary: req.model_summary,
            created_ms: ts,
            updated_ms: ts,
            uses: 0,
        };
        let row = lesson.to_row()?;
        self.db
            .insert_user_lesson(&row)
            .map_err(|e| format!("Failed to save user lesson: {e}"))?;
        Ok(lesson)
    }

    pub fn get_lesson(&self, id: &str) -> Result<Option<UserLesson>, String> {
        let row = self
            .db
            .get_user_lesson(id)
            .map_err(|e| format!("Failed to read user lesson {id}: {e}"))?;
        row.as_ref().map(UserLesson::from_row).transpose()
    }

    pub fn list_lessons(&self) -> Result<Vec<UserLesson>, String> {
        let rows = self
            .db
            .list_user_lessons()
            .map_err(|e| format!("Failed to list user lessons: {e}"))?;
        rows.iter().map(UserLesson::from_row).collect()
    }

    pub fn update_lesson(
        &self,
        id: &str,
        text: &str,
        scope: LessonScope,
        model_summary: Option<bool>,
    ) -> Result<UserLesson, String> {
        let clean_text = validate_lesson_text(text)?;
        let clean_scope = scope.validated()?;
        let updated_ms = now_ms();
        let found = self
            .db
            .update_user_lesson(
                id,
                &clean_text,
                clean_scope.kind.as_str(),
                clean_scope.mode.map(|m| m.as_str()),
                clean_scope.field.as_deref(),
                clean_scope.paper_key.as_ref().map(|pk| pk.as_str()),
                model_summary,
                updated_ms,
            )
            .map_err(|e| format!("Failed to update user lesson {id}: {e}"))?;
        if !found {
            return Err(format!("Lesson `{id}` not found."));
        }
        self.get_lesson(id)?
            .ok_or_else(|| format!("Lesson `{id}` disappeared after update."))
    }

    pub fn delete_lesson(&self, id: &str) -> Result<bool, String> {
        self.db
            .delete_user_lesson(id)
            .map_err(|e| format!("Failed to delete user lesson {id}: {e}"))
    }

    pub fn record_supplied_uses(&self, ids: &[String]) -> Result<usize, String> {
        self.db
            .increment_user_lesson_uses(ids)
            .map_err(|e| format!("Failed to update lesson usage counters: {e}"))
    }
}

mod hex {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        let mut s = String::with_capacity(bytes.as_ref().len() * 2);
        for &b in bytes.as_ref() {
            s.push(DIGITS[(b >> 4) as usize] as char);
            s.push(DIGITS[(b & 0x0f) as usize] as char);
        }
        s
    }
}

#[cfg(test)]
#[path = "lessons_capture_tests.rs"]
mod tests;
