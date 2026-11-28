//! Authenticated Explore export HTTP route (`GET /api/sessions/{id}/idea/export`, package X17).
//!
//! Strictly header-token authenticated via `AuthGuard::authorize_api` in `http_srv.rs`
//! (never listed in `auth.rs` stream-cookie routes). Serves deterministic Explore JSON or
//! BibTeX exports with `Content-Disposition: attachment`, `Cache-Control: no-store`,
//! `X-Content-Type-Options: nosniff`, and `Content-Security-Policy: sandbox`.

use crate::http_srv::{write_bytes, write_json};
use crate::routes_session::{bad, conflict, internal, not_found};
use crate::state::DesktopState;
use export::idea::{export_idea_bytes, IdeaExportFormat};
use ledger::Ledger;
use research_tools::validate_session_id;
use serde_json::json;
use std::collections::HashMap;
use std::net::TcpStream;

/// Verified Explore export response ready to stream over HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedIdeaExport {
    pub filename: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

fn session_has_ledger_events_readonly(
    data_dir: &std::path::Path,
    session_id: &str,
) -> Result<bool, String> {
    if !data_dir.join("ledger.db").is_file() {
        return Ok(false);
    }
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    Ok(!events.is_empty())
}

/// Handle `GET /api/sessions/{id}/idea/export?format=json|bib`.
pub fn handle_idea_export(
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) -> Result<ServedIdeaExport, (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let has_events = session_has_ledger_events_readonly(&state.cfg.data_dir, session_id).map_err(
        |e| {
            if e.contains("tamper") || e.contains("integrity") || e.contains("corrupt") {
                conflict(e)
            } else {
                internal(e)
            }
        },
    )?;
    if !has_events {
        return Err(not_found(format!("session `{session_id}` not found")));
    }

    let fmt_raw = query
        .get("format")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("json");
    let format = IdeaExportFormat::parse(fmt_raw).map_err(bad)?;

    let (filename, bytes) = export_idea_bytes(&state.cfg.data_dir, session_id, format)
        .map_err(map_idea_export_error)?;

    Ok(ServedIdeaExport {
        filename: safe_download_filename(&filename, format),
        content_type: format.content_type(),
        bytes,
    })
}

/// Write a `GET /api/sessions/{id}/idea/export` response onto `stream`.
pub fn serve_idea_export(
    stream: &mut TcpStream,
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) {
    match handle_idea_export(state, session_id, query) {
        Ok(served) => {
            let disp = format!("attachment; filename=\"{}\"", served.filename);
            write_bytes(
                stream,
                200,
                served.content_type,
                &served.bytes,
                &[
                    ("Content-Disposition", disp.as_str()),
                    ("Cache-Control", "no-store"),
                    ("Content-Security-Policy", "sandbox"),
                ],
            );
        }
        Err((status, message)) => {
            write_json(
                stream,
                status,
                json!({ "error": message }),
                &[("Cache-Control", "no-store")],
            );
        }
    }
}

fn map_idea_export_error(err: String) -> (u16, String) {
    if err.contains("Rule V7")
        || err.contains("Rule S8")
        || err.contains("Rule C3")
        || err.contains("F4 re-matching")
        || err.contains("ledger integrity check failed")
    {
        conflict(err)
    } else {
        bad(err)
    }
}

fn safe_download_filename(raw: &str, format: IdeaExportFormat) -> String {
    let cleaned: String = raw
        .trim_start_matches('.')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    let trimmed = cleaned.trim_start_matches('.');
    if trimmed.is_empty() {
        format!("explore-export.{}", format.extension())
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_download_filename_sanitizes_edges() {
        assert_eq!(
            safe_download_filename(".hidden.json", IdeaExportFormat::Json),
            "hidden.json"
        );
        assert_eq!(
            safe_download_filename("..", IdeaExportFormat::Bib),
            "explore-export.bib"
        );
        assert_eq!(
            safe_download_filename("exp-1-idea.json", IdeaExportFormat::Json),
            "exp-1-idea.json"
        );
    }
}
