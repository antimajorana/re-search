//! HTTP handlers for Investigate V15 sharing (`.rsmap` and standalone HTML export + fail-closed import).

use crate::investigation::{is_investigation, latest};
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use export::paper_map::{
    build_paper_map_export, serialize_paper_map_json, PaperMapError, PaperMapExport,
    PaperMapExportOptions,
};
use export::paper_map_html::render_paper_map_html;
use export::paper_map_import::import_paper_map;
use ledger::sha256_hex;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const EVENT_INVESTIGATION_EXPORTED: &str = "investigation.exported";
pub const EVENT_INVESTIGATION_IMPORTED: &str = "investigation.imported";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Rsmap,
    Html,
}

impl ExportFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rsmap => "rsmap",
            Self::Html => "html",
        }
    }
    fn content_type(self) -> &'static str {
        match self {
            Self::Rsmap => "application/json; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
        }
    }
}

fn parse_format(raw: Option<&str>) -> Result<ExportFormat, (u16, String)> {
    match raw.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("rsmap").to_ascii_lowercase().as_str() {
        "rsmap" => Ok(ExportFormat::Rsmap),
        "html" => Ok(ExportFormat::Html),
        "md" | "markdown" => Err(bad(
            "Markdown reading-notes export uses the F15 reading-notes write-up (`writeup_paper.rs`); `/investigation/export` supports `format=rsmap` or `format=html`.",
        )),
        other => Err(bad(format!("Unsupported export format `{other}`. Use `rsmap` or `html`."))),
    }
}

fn parse_bool_flag(query: &HashMap<String, String>, key: &str) -> Result<bool, (u16, String)> {
    match query.get(key).map(String::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("0") | Some("false") => Ok(false),
        Some("1") | Some("true") => Ok(true),
        Some(_) => Err(bad(format!("`{key}` must be `0` or `1`."))),
    }
}

fn sanitize_filename_stem(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let trimmed = clean.trim_matches(['-', '.']);
    if trimmed.is_empty() { "investigation-map".to_string() } else { trimmed.to_string() }
}

fn map_export_err(err: PaperMapError) -> (u16, String) {
    match err {
        PaperMapError::BadRequest(m) => bad(m),
        PaperMapError::NotFound(m) => not_found(m),
        PaperMapError::Internal(m) => internal(m),
    }
}

fn read_investigate_session(state: &DesktopState, session_id: &str) -> Result<ledger::Ledger, (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(|e| not_found(e.to_string()))?;
    if events.is_empty() || latest(&events, "session.created").is_none() && !is_investigation(&events) {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    if !is_investigation(&events) {
        return Err(bad("This endpoint is only available for Investigate sessions."));
    }
    Ok(ledger)
}

fn build_read_only_preview(map: &PaperMapExport) -> Value {
    json!({
        "paper_key": map.paper_key,
        "version": map.version,
        "title": map.title,
        "question": map.question,
        "builder": map.builder,
        "licence": map.licence,
        "open_licence": map.open_licence,
        "statements_included": map.statements_included,
        "notes_included": map.notes_included,
        "summary": map.skeleton.summary,
        "gists": map.gists,
        "authors_claims": map.authors_claims,
        "model_nodes": map.model_nodes,
        "model_edges": map.model_edges,
        "notes": map.notes,
        "context_cards": map.context_cards,
    })
}

/// Handle `GET /api/sessions/{id}/investigation/export?format=rsmap|html&include_statements=0|1&include_notes=0|1`.
pub fn handle_export(
    state: &Arc<DesktopState>,
    session_id: &str,
    query: &HashMap<String, String>,
) -> Result<(String, &'static str, Vec<u8>), (u16, String)> {
    let format = parse_format(query.get("format").map(String::as_str))?;
    let include_full_statements = parse_bool_flag(query, "include_statements")?;
    let include_notes = parse_bool_flag(query, "include_notes")?;

    let ledger = read_investigate_session(state, session_id)?;
    let exported = build_paper_map_export(
        &state.cfg.data_dir,
        session_id,
        PaperMapExportOptions { include_full_statements, include_notes },
    )
    .map_err(map_export_err)?;

    let bytes = match format {
        ExportFormat::Rsmap => serialize_paper_map_json(&exported).map_err(map_export_err)?,
        ExportFormat::Html => render_paper_map_html(&exported).into_bytes(),
    };
    let sha256 = sha256_hex(&bytes);
    let stem = sanitize_filename_stem(&exported.paper_key);
    let filename = format!("{stem}.{}", format.as_str());

    let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
    ledger
        .append(
            session_id,
            EVENT_INVESTIGATION_EXPORTED,
            1,
            &json!({
                "format": format.as_str(),
                "sha256": sha256,
                "statements_included": exported.statements_included,
                "notes_included": exported.notes_included,
            }),
        )
        .map_err(|e| internal(e.to_string()))?;

    Ok((filename, format.content_type(), bytes))
}

fn extract_rsmap_bytes(body: &[u8]) -> Result<Vec<u8>, (u16, String)> {
    let val = parse_json_body(body)?;
    if let Some(raw) = val.get("raw_json").and_then(Value::as_str) {
        return Ok(raw.as_bytes().to_vec());
    }
    if let Some(inner) = val.get("rsmap").or_else(|| val.get("map")) {
        if let Some(s) = inner.as_str() {
            return Ok(s.as_bytes().to_vec());
        }
        return serde_json::to_vec(inner).map_err(|e| bad(e.to_string()));
    }
    Ok(body.to_vec())
}

/// Handle `POST /api/sessions/{id}/investigation/import`.
pub fn handle_import(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    let rsmap_bytes = extract_rsmap_bytes(body)?;
    let ledger = read_investigate_session(state, session_id)?;
    let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;

    let report = import_paper_map(&state.cfg.data_dir, session_id, &rsmap_bytes).map_err(map_export_err)?;
    let mut payload = json!({
        "format": "rsmap",
        "sha256": report.sha256,
        "paper_key": report.paper_key,
        "version": report.version,
        "source_match": report.source_match,
        "read_only": report.read_only,
        "source_id": report.source_id,
        "skeleton_seq": report.skeleton_seq,
        "items_matched": report.items_matched,
        "items_dropped": report.items_dropped,
        "banner": report.banner,
    });
    if report.read_only {
        payload["preview"] = build_read_only_preview(&report.imported_map);
    }
    let seq = ledger
        .append(session_id, EVENT_INVESTIGATION_IMPORTED, 1, &payload)
        .map_err(|e| internal(e.to_string()))?;

    let mut resp = json!({
        "seq": seq,
        "format": "rsmap",
        "sha256": report.sha256,
        "paper_key": report.paper_key,
        "version": report.version,
        "source_match": report.source_match,
        "read_only": report.read_only,
        "source_id": report.source_id,
        "skeleton_seq": report.skeleton_seq,
        "items_matched": report.items_matched,
        "items_dropped": report.items_dropped,
        "banner": report.banner,
        "dropped_reasons": report.dropped_reasons,
    });
    if report.read_only {
        resp["preview"] = build_read_only_preview(&report.imported_map);
    }
    Ok((200, resp))
}
