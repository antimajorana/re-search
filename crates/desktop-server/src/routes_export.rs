//! Session export endpoint (`GET /api/sessions/{id}/export?format=...`) extracted from
//! `routes_tools.rs` as Checkpoint S19a and extended for `.ipynb` notebooks in S19c.

use crate::routes_session::{bad, internal, not_found};
use crate::state::DesktopState;
use export::{export_session_bytes_with_options, ExportFormat};
use research_tools::validate_session_id;
use std::collections::HashMap;

pub fn handle_export(
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) -> Result<(String, &'static str, Vec<u8>), (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let fmt_str = query.get("format").map(String::as_str).unwrap_or("tex");
    let fmt_norm = fmt_str.trim().to_ascii_lowercase();
    if matches!(fmt_norm.as_str(), "explain_md" | "explain_html") {
        return crate::explain_export::export_explain_conversation(state, session_id, &fmt_norm);
    }
    let include_all_runs = query
        .get("include_all_runs")
        .is_some_and(|raw| matches!(raw.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"));
    let fmt = ExportFormat::parse(fmt_str).map_err(bad)?;
    let (filename, bytes) =
        export_session_bytes_with_options(&state.cfg.data_dir, session_id, fmt, include_all_runs)
            .map_err(bad)?;
    let content_type = match fmt {
        ExportFormat::Tex => "application/x-tex; charset=utf-8",
        ExportFormat::Html => "text/html; charset=utf-8",
        ExportFormat::Zip => "application/zip",
        ExportFormat::Notebook => "application/x-ipynb+json; charset=utf-8",
    };
    Ok((filename, content_type, bytes))
}
