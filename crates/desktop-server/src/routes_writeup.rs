//! Authenticated F15 write-up HTTP route (`GET /api/sessions/{id}/writeup`).
//!
//! Strictly header-token authenticated via `AuthGuard::authorize_api` in `http_srv.rs`
//! (never listed in `auth.rs` stream-cookie routes). Serves deterministic TeX, Markdown, or
//! BibTeX write-ups with `Content-Disposition: attachment`, `Cache-Control: no-store`,
//! `X-Content-Type-Options: nosniff`, and `Content-Security-Policy: sandbox`.

use crate::http_srv::{write_bytes, write_json};
use crate::routes_session::{bad, conflict, internal, not_found};
use crate::state::DesktopState;
use export::writeup::WriteupOptions;
use export::{writeup_session_bytes, WriteupFormat};
use ledger::Ledger;
use research_tools::validate_session_id;
use serde_json::json;
use std::collections::HashMap;
use std::net::TcpStream;

/// Verified write-up response ready to stream over HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedWriteup {
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

/// Handle `GET /api/sessions/{id}/writeup?format=tex|md|bib&inline_marks=0|1`.
pub fn handle_writeup(
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) -> Result<ServedWriteup, (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !session_has_ledger_events_readonly(&state.cfg.data_dir, session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }

    let fmt_raw = query.get("format").map(String::as_str).unwrap_or("tex");
    let format = WriteupFormat::parse(fmt_raw).map_err(bad)?;
    let include_inline_badges = parse_inline_marks(query.get("inline_marks").map(String::as_str))?;

    let opts = WriteupOptions {
        include_inline_badges,
        bib_stem: format!("{session_id}-writeup"),
    };
    let (filename, bytes) =
        writeup_session_bytes(&state.cfg.data_dir, session_id, format, &opts)
            .map_err(map_writeup_error)?;

    let content_type = match format {
        WriteupFormat::Tex => "application/x-tex; charset=utf-8",
        WriteupFormat::Md => "text/markdown; charset=utf-8",
        WriteupFormat::Bib => "application/x-bibtex; charset=utf-8",
    };

    Ok(ServedWriteup {
        filename: safe_download_filename(&filename),
        content_type,
        bytes,
    })
}

/// Write a `GET /api/sessions/{id}/writeup` response onto `stream`.
pub fn serve_writeup(
    stream: &mut TcpStream,
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) {
    match handle_writeup(state, session_id, query) {
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
            write_json(stream, status, json!({ "error": message }), &[]);
        }
    }
}

fn parse_inline_marks(raw: Option<&str>) -> Result<bool, (u16, String)> {
    let Some(val) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(true);
    };
    match val.to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => Err(bad(format!(
            "invalid `inline_marks` value `{other}` (expected `0` or `1`)"
        ))),
    }
}

fn map_writeup_error(err: String) -> (u16, String) {
    if err.contains("write-up adapter not built yet") {
        (501, err)
    } else if err.contains("Gate G7") || err.contains("ledger verification failed") {
        conflict(err)
    } else {
        bad(err)
    }
}

fn safe_download_filename(raw: &str) -> String {
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
        "writeup.tex".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::safe_download_filename;

    #[test]
    fn safe_download_filename_strips_leading_dots() {
        assert_eq!(safe_download_filename(".hidden.tex"), "hidden.tex");
        assert_eq!(safe_download_filename(".."), "writeup.tex");
        assert_eq!(safe_download_filename("s-1-writeup.tex"), "s-1-writeup.tex");
    }
}
