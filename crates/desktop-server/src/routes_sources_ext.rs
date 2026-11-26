//! HTTP API handlers for opt-in sending papers back (`/api/sources/export/bibtex` and
//! `/api/sources/send/kandinsky`, package F13c, `00-overview.md` 2.11 & 8.3 decision 5).
//!
//! Security invariants:
//! - Both endpoints are strictly `POST` and sit behind `AuthGuard::authorize_api` (header token
//!   + loopback `Host` + same-origin `Origin` checks) in `http_srv.rs` / `api_dispatch.rs`.
//! - BibTeX export reuses F15's [`export::writeup_bib`] formatter (preferring the user's stored
//!   `source_item.citekey`) and writes atomically ONLY to a validated user-chosen path outside
//!   every registered read-only `PaperSource` root.
//! - Send to Kandinsky NEVER writes files into a Kandinsky vault or Zotero database, NEVER
//!   accepts a binary path from the request body, and NEVER invokes a shell.

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use export::writeup_bib::{render_bibtex, resolve_bib_entry};
use research_tools::library::Library;
use research_tools::paper_key::{PaperIds, PaperKey, PaperRef};
use research_tools::paper_sources::portable_path_string;
use research_tools::source_export::{
    build_kandinsky_capture_argv, build_kandinsky_deep_link, invoke_kandinsky_cli,
    resolve_kandinsky_target_for_paper, validate_kandinsky_capture_target,
    write_bibtex_export_atomic, KANDINSKY_DEFAULT_BINARY,
};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

const MAX_EXPORT_PAPERS: usize = 500;

/// Single dispatcher for F13c `/api/sources/export/..` and `/api/sources/send/..` endpoints.
pub fn handle_sources_ext(
    state: &DesktopState,
    method: &str,
    rest: &[&str],
    _query: &HashMap<String, String>,
    body: &[u8],
) -> ApiResult {
    match (method, rest) {
        ("POST", ["export", "bibtex"]) => handle_export_bibtex(state, body),
        ("POST", ["send", "kandinsky"]) => {
            handle_send_kandinsky_with_bin(state, body, Path::new(KANDINSKY_DEFAULT_BINARY))
        }
        (_, ["export", "bibtex"] | ["send", "kandinsky"]) => Err((
            405,
            format!("POST is required for opt-in action /api/sources/{}", rest.join("/")),
        )),
        _ => Err(not_found(format!(
            "unknown sources-ext endpoint: {method} /api/sources/{}",
            rest.join("/")
        ))),
    }
}

/// `POST /api/sources/export/bibtex`
///
/// Body: `{ "paper_keys": ["arxiv:...", ...], "output_path"?: "/abs/path/to/export.bib" }`.
/// Reuses F15's `writeup_bib` for BibTeX formatting and citekey preference.
pub fn handle_export_bibtex(state: &DesktopState, body: &[u8]) -> ApiResult {
    let val = parse_json_body(body)?;
    let raw_keys = val
        .get("paper_keys")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("`paper_keys` array is required"))?;
    if raw_keys.is_empty() {
        return Err(bad("`paper_keys` must contain at least one paper key"));
    }
    if raw_keys.len() > MAX_EXPORT_PAPERS {
        return Err(bad(format!(
            "`paper_keys` exceeds limit of {MAX_EXPORT_PAPERS} entries"
        )));
    }

    let mut keys = Vec::with_capacity(raw_keys.len());
    for item in raw_keys {
        let s = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad("each entry in `paper_keys` must be a non-empty string"))?;
        keys.push(PaperKey::parse(s).map_err(bad)?);
    }

    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let mut used_keys = BTreeSet::new();
    let mut entries = Vec::with_capacity(keys.len());

    for key in &keys {
        let canonical = lib
            .resolve_key(key)
            .map_err(internal)?
            .unwrap_or_else(|| key.clone());
        let detail = lib
            .get_paper(&canonical)
            .map_err(internal)?
            .ok_or_else(|| not_found(format!("paper `{canonical}` not found in library")))?;
        let version = detail
            .source_items
            .iter()
            .find(|it| it.ids.arxiv == detail.paper.ids.arxiv && it.version.is_some())
            .or_else(|| detail.source_items.iter().find(|it| it.version.is_some()))
            .and_then(|it| it.version.clone())
            .filter(|v| !v.trim().is_empty());
        let pref = PaperRef {
            key: canonical.clone(),
            version,
            ids: PaperIds {
                arxiv: detail.paper.ids.arxiv.clone(),
                doi: detail.paper.ids.doi.clone(),
                inspire: detail.paper.ids.inspire.clone(),
                s2: detail.paper.ids.s2.clone(),
            },
        };
        let entry = resolve_bib_entry(
            Some(&lib),
            &pref,
            &detail.paper.title,
            &detail.paper.authors,
            detail.paper.year,
            &mut used_keys,
        )
        .map_err(internal)?;
        entries.push(entry);
    }

    let citekeys: Vec<String> = entries.iter().map(|e| e.cite_key.clone()).collect();
    let bibtex = render_bibtex(&entries);

    let output_path = val
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(dest) = output_path {
        let (canon_dest, bytes_written) =
            write_bibtex_export_atomic(&lib, Path::new(dest), &bibtex).map_err(bad)?;
        return Ok((
            200,
            json!({
                "ok": true,
                "written": true,
                "output_path": portable_path_string(&canon_dest),
                "bytes_written": bytes_written,
                "count": citekeys.len(),
                "citekeys": citekeys,
                "bibtex": bibtex,
            }),
        ));
    }

    Ok((
        200,
        json!({
            "ok": true,
            "written": false,
            "count": citekeys.len(),
            "citekeys": citekeys,
            "bibtex": bibtex,
        }),
    ))
}

/// `POST /api/sources/send/kandinsky` with an explicit executable path (production always passes
/// `KANDINSKY_DEFAULT_BINARY`; tests may pass a self-compiled stub executable).
///
/// Body: `{ "paper_key"?: "arxiv:...", "target"?: "arXiv:...", "title"?: "...",
///          "mode"?: "deep_link" | "cli", "user_initiated"?: true }`.
pub fn handle_send_kandinsky_with_bin(
    state: &DesktopState,
    body: &[u8],
    kandinsky_bin: &Path,
) -> ApiResult {
    let val = parse_json_body(body)?;
    if val.get("user_initiated").and_then(Value::as_bool) != Some(true) {
        return Err(bad(
            "Send to Kandinsky is opt-in only and requires explicit `user_initiated: true` confirmation",
        ));
    }
    let mode = val
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("deep_link");
    if !matches!(mode, "deep_link" | "cli") {
        return Err(bad(format!(
            "invalid Kandinsky send mode `{mode}` (expected `deep_link` or `cli`)"
        )));
    }

    let explicit_title = val
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (target, resolved_title) = if let Some(raw_target) = val
        .get("target")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let validated = validate_kandinsky_capture_target(raw_target).map_err(bad)?;
        (validated, explicit_title)
    } else if let Some(raw_key) = val
        .get("paper_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let key = PaperKey::parse(raw_key).map_err(bad)?;
        let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
        let (validated, lib_title) = resolve_kandinsky_target_for_paper(&lib, &key)
            .map_err(|e| if e.contains("not found") { not_found(e) } else { bad(e) })?;
        let title = explicit_title.or_else(|| (!lib_title.trim().is_empty()).then_some(lib_title));
        (validated, title)
    } else {
        return Err(bad("either `target` or `paper_key` is required"));
    };

    let deep_link = build_kandinsky_deep_link(&target, resolved_title.as_deref());
    let argv = build_kandinsky_capture_argv(&target);

    if mode == "cli" {
        let outcome = invoke_kandinsky_cli(kandinsky_bin, &target)
            .map_err(|e| (502, e))?;
        let status = if outcome.success { 200 } else { 502 };
        return Ok((
            status,
            json!({
                "ok": outcome.success,
                "mode": "cli",
                "target": target.as_str(),
                "deep_link": deep_link,
                "argv": outcome.argv,
                "exit_code": outcome.exit_code,
                "stdout": outcome.stdout,
                "stderr": outcome.stderr,
            }),
        ));
    }

    Ok((
        200,
        json!({
            "ok": true,
            "mode": "deep_link",
            "target": target.as_str(),
            "deep_link": deep_link,
            "argv": argv,
        }),
    ))
}
