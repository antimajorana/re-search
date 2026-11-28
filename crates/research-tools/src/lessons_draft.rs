//! F16b "Remember this?" prompt prefill (zero model calls, user's own words)
//! and explicit opt-in `(model)` summary drafting.

use crate::lessons_capture::{
    normalize_field_name, normalize_lesson_input, validate_lesson_text, validate_node_ref,
    LessonScope, LessonSourceKind, MAX_LESSON_CHARS, MODEL_SUMMARY_TAG,
};
use crate::paper_key::{Mode, NodeRef, PaperKey};
use crate::quote_core::clip_to_chars;
use serde::{Deserialize, Serialize};

/// System prompt used ONLY when the user clicks `"Draft summary (model)"`.
pub const DRAFT_SUMMARY_SYSTEM_PROMPT: &str = "\
Rewrite the user's correction into one concise, reusable physics/research note \
of at most 260 characters. Keep strictly to what the user corrected; do not invent \
equations, claims, or instructions. Return plain text only with no markdown fence.";

/// Event emitted into a correction path to prefill the `"Remember this?"` UI card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrectionOriginContext {
    pub from: NodeRef,
    #[serde(default)]
    pub source_kind: LessonSourceKind,
    pub user_words: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<PaperKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_summary: Option<String>,
}

/// Zero-model `"Remember this?"` card prefill payload (`00-overview.md` 8.2 #5, 8.3 #7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RememberPromptDraft {
    pub prefilled_text: String,
    pub clipped_from_user_words: bool,
    pub default_scope: LessonScope,
    pub allowed_scopes: Vec<LessonScope>,
    pub from: NodeRef,
    pub source_kind: LessonSourceKind,
    pub max_chars: usize,
    pub model_summary: bool,
}

/// Build `"Remember this?"` prefill purely from the user's own words (**0 model calls**).
pub fn build_remember_draft(ctx: &CorrectionOriginContext) -> Result<RememberPromptDraft, String> {
    validate_node_ref(&ctx.from)?;
    let cleaned = normalize_lesson_input(&ctx.user_words);
    if cleaned.is_empty() {
        return Err("Cannot prefill lesson from an empty correction.".to_string());
    }
    let clipped = clip_to_chars(&cleaned, MAX_LESSON_CHARS).trim().to_string();
    let clipped_from_user_words = clipped.chars().count() < cleaned.chars().count();

    let norm_field = normalize_field_name(ctx.field.as_deref());
    let default_scope = LessonScope::default_for_correction(ctx.from.mode, norm_field.as_deref());
    let mut allowed_scopes = vec![default_scope.clone()];
    let candidates = [
        Some(LessonScope::mode(ctx.from.mode)),
        norm_field.as_deref().map(LessonScope::field),
        ctx.paper_key.clone().map(LessonScope::paper),
        Some(LessonScope::global()),
    ];
    for c in candidates.into_iter().flatten() {
        if !allowed_scopes.contains(&c) {
            allowed_scopes.push(c);
        }
    }

    Ok(RememberPromptDraft {
        prefilled_text: clipped,
        clipped_from_user_words,
        default_scope,
        allowed_scopes,
        from: ctx.from.clone(),
        source_kind: ctx.source_kind,
        max_chars: MAX_LESSON_CHARS,
        model_summary: false,
    })
}

/// Build a single-turn user prompt ONLY when the user explicitly clicks `"Draft summary (model)"`.
pub fn build_summary_prompt(
    user_words: &str,
    mode: Mode,
    field: Option<&str>,
    target_summary: Option<&str>,
) -> Result<String, String> {
    let cleaned = normalize_lesson_input(user_words);
    if cleaned.is_empty() {
        return Err("Cannot draft summary from empty correction text.".to_string());
    }
    let field_txt = normalize_field_name(field).unwrap_or_else(|| "general".to_string());
    let target_txt = target_summary
        .map(normalize_lesson_input)
        .filter(|s| !s.is_empty())
        .map(|s| format!("\nCorrected step summary: {}", clip_to_chars(&s, 200)))
        .unwrap_or_default();
    Ok(format!(
        "Mode: {}\nField: {}{}\nUser's correction: {}\n\nWrite one reusable note <= 260 characters.",
        mode.as_str(),
        field_txt,
        target_txt,
        clip_to_chars(&cleaned, 600),
    ))
}

/// Normalize a model-generated draft summary, enforce `<= 300` Unicode chars including `" (model)"`.
pub fn finalize_model_summary_draft(raw_model_output: &str) -> Result<String, String> {
    let mut cleaned = normalize_lesson_input(raw_model_output);
    cleaned = cleaned
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string();
    if let Some(stripped) = cleaned.strip_suffix(MODEL_SUMMARY_TAG) {
        cleaned = stripped.trim().to_string();
    }
    if cleaned.is_empty() {
        return Err("Model summary draft was empty.".to_string());
    }
    let suffix = format!(" {MODEL_SUMMARY_TAG}");
    let body_cap = MAX_LESSON_CHARS.saturating_sub(suffix.chars().count());
    let body = clip_to_chars(&cleaned, body_cap).trim();
    if body.is_empty() {
        return Err("Model summary draft body was empty.".to_string());
    }
    let final_text = format!("{body}{suffix}");
    validate_lesson_text(&final_text)
}
