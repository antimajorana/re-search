//! HTTP API handlers for F16b user-approved lessons UI & disclosure (`/api/lessons/notes..`,
//! `/api/lessons/supplied/{session_id}`).
//!
//! Backed by `<data_dir>/library/lessons.db` (v2 `user_lesson`).
//! - Never auto-saves corrections; only explicit user POST/PUT/DELETE mutates `lessons.db`.
//! - Never calls a model automatically; `POST /api/lessons/notes/draft-summary` runs ONLY on
//!   explicit click of `"Draft summary (model)"` and tags output with `"(model)"`.
//! - `GET /api/lessons/supplied/{session_id}` reads ONLY the frozen session ledger (`0` live DB reads).
//! - Backup export requires `?confirm=true`; restore verifies canonical SHA-256 checksum.

use crate::agent_models::resolve_in_core_role_model;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use agent::transport::{ContentBlock, Message, ModelRequest, Role, Transport};
use research_tools::lessons_backup::{
    export_lessons_backup, restore_lessons_backup, OptInLessonsExport, MAX_LESSONS_B_BACKUP_BYTES,
};
use research_tools::lessons_capture::{
    build_remember_draft, build_summary_prompt, finalize_model_summary_draft, parse_mode_str,
    validate_lesson_text, CorrectionOriginContext, LessonScope, LessonScopeKind, LessonSourceKind,
    SaveLessonRequest, UserLessonsStore, DRAFT_SUMMARY_SYSTEM_PROMPT, MAX_LESSON_CHARS,
};
use research_tools::lessons_prompt::{
    count_distinct_supplied_notes, read_supplied_lessons, supplied_notes_disclosure,
};
use research_tools::paper_key::{Mode, NodeRef, PaperKey};
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Delegate handler called from `routes_lessons::handle_lessons` for all F16b `/api/lessons/..` routes.
pub fn handle_lessons_ui(
    state: &DesktopState,
    method: &str,
    rest: &[&str],
    query: &HashMap<String, String>,
    body: &[u8],
) -> ApiResult {
    match (method, rest) {
        ("GET", ["notes"]) => handle_list_notes(state, query),
        ("POST", ["notes"]) => handle_create_note(state, body),
        ("POST", ["notes", "prefill"]) => handle_prefill_note(body),
        ("POST", ["notes", "draft-summary"]) => handle_draft_model_summary(state, body),
        ("GET", ["notes", "backup"]) => handle_export_notes_backup(state, query),
        ("POST", ["notes", "restore"]) => handle_restore_notes_backup(state, body),
        ("PUT" | "PATCH", ["notes", id]) => handle_update_note(state, id, body),
        ("DELETE", ["notes", id]) => handle_delete_note(state, id),
        ("GET", ["supplied", session_id]) => handle_supplied_for_session(state, session_id, query),
        _ => {
            let suffix = if rest.is_empty() {
                String::new()
            } else {
                format!("/{}", rest.join("/"))
            };
            Err(not_found(format!(
                "unknown lessons endpoint: {method} /api/lessons{suffix}"
            )))
        }
    }
}

fn handle_list_notes(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let store = UserLessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let mut lessons = store.list_lessons().map_err(internal)?;
    if let Some(mode_filter) = query.get("mode").map(String::as_str) {
        let mode = parse_mode_str(mode_filter).map_err(bad)?;
        lessons.retain(|l| l.scope.mode == Some(mode) || l.scope.kind == LessonScopeKind::Global);
    }
    Ok((
        200,
        json!({ "ok": true, "lessons": lessons, "max_chars": MAX_LESSON_CHARS }),
    ))
}

fn handle_prefill_note(body: &[u8]) -> ApiResult {
    let ctx: CorrectionOriginContext =
        serde_json::from_slice(body).map_err(|e| bad(format!("invalid prefill JSON: {e}")))?;
    let draft = build_remember_draft(&ctx).map_err(bad)?;
    Ok((200, json!({ "ok": true, "draft": draft })))
}

fn handle_create_note(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let text = val
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`text` is required"))?;
    let _ = validate_lesson_text(text).map_err(bad)?;
    let scope = parse_scope_json(
        val.get("scope")
            .ok_or_else(|| bad("`scope` object is required"))?,
    )?;
    let from: NodeRef = val
        .get("from")
        .cloned()
        .ok_or_else(|| bad("`from` NodeRef is required"))
        .and_then(|v| serde_json::from_value(v).map_err(|e| bad(format!("invalid `from`: {e}"))))?;
    let source_kind = val
        .get("source_kind")
        .and_then(Value::as_str)
        .map(LessonSourceKind::parse)
        .unwrap_or(LessonSourceKind::StepCorrection);
    let model_summary = val
        .get("model_summary")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let store = UserLessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let saved = store
        .save_lesson(SaveLessonRequest {
            text: text.to_string(),
            scope,
            from,
            source_kind,
            model_summary,
            created_ms: None,
        })
        .map_err(bad)?;
    Ok((200, json!({ "ok": true, "lesson": saved })))
}

fn handle_update_note(state: &DesktopState, id: &str, body: &[u8]) -> ApiResult {
    if id.trim().is_empty() {
        return Err(bad("lesson id must not be empty"));
    }
    let val = parse_json_body(body)?;
    let text = val
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`text` is required"))?;
    let _ = validate_lesson_text(text).map_err(bad)?;
    let scope = parse_scope_json(
        val.get("scope")
            .ok_or_else(|| bad("`scope` object is required"))?,
    )?;
    let model_summary = val.get("model_summary").and_then(Value::as_bool);

    let store = UserLessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    if store.get_lesson(id).map_err(internal)?.is_none() {
        return Err(not_found(format!("lesson `{id}` not found")));
    }
    let updated = store
        .update_lesson(id, text, scope, model_summary)
        .map_err(bad)?;
    Ok((200, json!({ "ok": true, "lesson": updated })))
}

fn handle_delete_note(state: &DesktopState, id: &str) -> ApiResult {
    if id.trim().is_empty() {
        return Err(bad("lesson id must not be empty"));
    }
    let store = UserLessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let deleted = store.delete_lesson(id).map_err(internal)?;
    if !deleted {
        return Err(not_found(format!("lesson `{id}` not found")));
    }
    Ok((200, json!({ "ok": true, "deleted": true, "id": id })))
}

fn handle_draft_model_summary(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let user_words = val
        .get("user_words")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`user_words` must be non-empty"))?;
    let mode = val
        .get("mode")
        .and_then(Value::as_str)
        .map(parse_mode_str)
        .transpose()
        .map_err(bad)?
        .unwrap_or(Mode::Solve);
    let field = val.get("field").and_then(Value::as_str);
    let target_summary = val.get("target_summary").and_then(Value::as_str);

    let (provider, model) = resolve_in_core_role_model(state, "solver", Some(&val), true)?;
    let transport = state
        .build_transport(provider, &model)
        .map_err(|e| (400, e))?;
    let drafted = draft_summary_with_transport(
        transport.as_ref(),
        &model,
        user_words,
        mode,
        field,
        target_summary,
    )?;
    Ok((
        200,
        json!({
            "ok": true,
            "text": drafted,
            "model_summary": true,
            "provider": provider.cli_name(),
            "model": model,
        }),
    ))
}

/// Shared single-turn summary builder (`POST /api/lessons/notes/draft-summary` and tests).
/// Never touches `lessons.db` and always tags output with `" (model)"` under `<= 300` chars.
pub fn draft_summary_with_transport(
    transport: &dyn Transport,
    model: &str,
    user_words: &str,
    mode: Mode,
    field: Option<&str>,
    target_summary: Option<&str>,
) -> Result<String, (u16, String)> {
    let user_prompt = build_summary_prompt(user_words, mode, field, target_summary).map_err(bad)?;
    let req = ModelRequest {
        model: model.to_string(),
        system: Some(DRAFT_SUMMARY_SYSTEM_PROMPT.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }],
        }],
        tools: Vec::new(),
        max_tokens: 180,
        reasoning_effort: None,
    };
    let prepared = transport
        .prepare_request(&req)
        .map_err(|e| (502, e.to_string()))?;
    let resp = transport
        .send_prepared(&prepared)
        .map_err(|e| (502, e.to_string()))?;
    let raw: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    finalize_model_summary_draft(&raw).map_err(bad)
}

fn handle_export_notes_backup(
    state: &DesktopState,
    query: &HashMap<String, String>,
) -> ApiResult {
    if query.get("confirm").map(String::as_str) != Some("true") {
        return Err(bad(
            "lessons backup export is opt-in: pass `?confirm=true` to export",
        ));
    }
    let store = UserLessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let bytes = export_lessons_backup(&store, OptInLessonsExport { include_lessons: true })
        .map_err(internal)?
        .ok_or_else(|| internal("opt-in backup returned empty output"))?;
    let parsed: Value = serde_json::from_slice(&bytes).map_err(|e| internal(e.to_string()))?;
    Ok((200, json!({ "ok": true, "backup": parsed })))
}

fn handle_restore_notes_backup(state: &DesktopState, body: &[u8]) -> ApiResult {
    if body.len() > MAX_LESSONS_B_BACKUP_BYTES {
        return Err(bad("backup payload exceeds maximum size"));
    }
    let val = parse_json_body(body)?;
    let replace = val
        .get("replace")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let backup_bytes = match val.get("backup") {
        Some(Value::String(s)) => s.as_bytes().to_vec(),
        Some(obj @ Value::Object(_)) => {
            serde_json::to_vec(obj).map_err(|e| bad(e.to_string()))?
        }
        Some(_) => return Err(bad("`backup` must be a JSON object or JSON string")),
        None => body.to_vec(),
    };
    let store = UserLessonsStore::open(&state.cfg.data_dir).map_err(internal)?;
    let restored = restore_lessons_backup(&store, &backup_bytes, replace).map_err(bad)?;
    Ok((200, json!({ "ok": true, "restored": restored })))
}

fn handle_supplied_for_session(
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let ledger = state.open_ledger().map_err(|e| internal(e.to_string()))?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let run_filter = query.get("run_id").map(String::as_str);
    let records = read_supplied_lessons(&events, Some(&ledger), run_filter);
    let count = count_distinct_supplied_notes(&records);
    let disclosure = supplied_notes_disclosure(count);
    Ok((
        200,
        json!({
            "ok": true,
            "session_id": session_id,
            "count": count,
            "disclosure": disclosure,
            "records": records,
        }),
    ))
}

fn parse_scope_json(val: &Value) -> Result<LessonScope, (u16, String)> {
    if let Some(kind_str) = val.as_str() {
        let kind = LessonScopeKind::parse(kind_str).map_err(bad)?;
        return if kind == LessonScopeKind::Global {
            Ok(LessonScope::global())
        } else {
            Err(bad("non-global scope must be a JSON object with parameters"))
        };
    }
    let kind_raw = val
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("`scope.kind` is required"))?;
    let kind = LessonScopeKind::parse(kind_raw).map_err(bad)?;
    let mode = val
        .get("mode")
        .and_then(Value::as_str)
        .map(parse_mode_str)
        .transpose()
        .map_err(bad)?;
    let field = val
        .get("field")
        .and_then(Value::as_str)
        .map(str::to_string);
    let paper_key = val
        .get("paper_key")
        .and_then(Value::as_str)
        .map(PaperKey::parse)
        .transpose()
        .map_err(bad)?;
    LessonScope { kind, mode, field, paper_key }
        .validated()
        .map_err(bad)
}
