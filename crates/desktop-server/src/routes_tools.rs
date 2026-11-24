//! Research tool endpoints: upload, workspace browsing, LaTeX patching, TeX audit,
//! literature search, proof DAG, chain verification, narration, side-talk, and export.

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::{decode_base64, parse_multipart_files, DesktopState, UploadedFile};
use crate::turn_runner::build_desktop_router;
use agent::{ask_sidetalk, narrate_session, read_sidetalk_thread};
use export::{export_session_bytes, ExportFormat};
use research_tools::fs_tools::{handle_fs_list, handle_fs_read};
use research_tools::write_tools::{workspace_edit_sha256, EDIT_BASE_CHANGED};
use research_tools::{
    ingest_bytes, read_proof_dag, run_arxiv_search, run_literature_search, run_tex_audit,
    validate_session_id, ArxivSearchRequest, AuditOptions, IngestRequest, LiteratureRequest,
    SandboxRoot, WorkspaceSandbox, FS_WRITE, LATEX_PATCH,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const MAX_UPLOAD_FILES: usize = 5;
const MAX_UPLOAD_BYTES_PER_FILE: usize = 10 * 1024 * 1024;
// Base64 expands each file to four bytes for every three input bytes, rounded up.
// The remaining space covers filenames and JSON or multipart framing.
pub(crate) const MAX_UPLOAD_BODY_BYTES: usize =
    MAX_UPLOAD_FILES * (MAX_UPLOAD_BYTES_PER_FILE.div_ceil(3) * 4) + 64 * 1024;

pub fn handle_upload(
    state: &DesktopState,
    session_id: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }

    let items = if let Some(ct) = content_type.filter(|c| c.to_ascii_lowercase().contains("multipart/form-data")) {
        parse_multipart_files(ct, body)?
    } else {
        parse_json_upload_files(body)?
    };

    if items.is_empty() {
        return Err(bad("upload contains no files"));
    }
    if items.len() > MAX_UPLOAD_FILES {
        return Err(bad(format!("at most {MAX_UPLOAD_FILES} files per upload")));
    }
    for (name, bytes) in &items {
        if name.trim().is_empty() {
            return Err(bad("uploaded file name must not be empty"));
        }
        if bytes.len() > MAX_UPLOAD_BYTES_PER_FILE {
            return Err((
                413,
                format!("file `{name}` exceeds 10 MiB upload limit"),
            ));
        }
    }

    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let sandbox = WorkspaceSandbox::new(env.data_dir.clone(), env.workspace_root.clone());
    let ledger = Mutex::new(state.open_ledger().map_err(internal)?);

    let mut ingested = Vec::new();
    let mut failed = Vec::new();
    for (index, (name, bytes)) in items.iter().enumerate() {
        let result = ingest_bytes(
            &sandbox,
            &ledger,
            &IngestRequest {
                session_id,
                file_name: name,
                origin: name,
                raw_bytes: bytes,
                source_id_hint: None,
                section_filter: None,
                title: None,
            },
        );
        match result {
            Ok(report) => ingested.push(report),
            Err(error) => failed.push(json!({ "index": index, "name": name, "error": error })),
        }
    }

    if let Some(first) = failed.first() {
        return Ok((400, json!({
            "ok": false,
            "error": first["error"],
            "ingested": ingested,
            "failed": failed,
        })));
    }
    Ok((200, json!({ "ok": true, "ingested": ingested, "failed": [] })))
}

pub fn handle_workspace(
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let root = query.get("root").map(String::as_str).unwrap_or("workspace");

    if let Some(path) = query.get("path").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return handle_workspace_file_inner(&env, session_id, root, path);
    }

    let ws_root = env.workspace_root.as_ref().map(|p| p.display().to_string());
    let res = if root == "workspace" && env.workspace_root.is_none() {
        json!({ "entries": [] })
    } else {
        match handle_fs_list(
            &env,
            session_id,
            &json!({ "root": root, "recursive": true, "max_entries": 1000 }),
        ) {
            Ok(v) => v,
            Err(msg) if msg.contains("no workspace") && root == "workspace" => {
                json!({ "entries": [] })
            }
            Err(e) => return Err(bad(e)),
        }
    };
    let entries = res.get("entries").cloned().unwrap_or_else(|| json!([]));
    Ok((
        200,
        json!({
            "workspace_root": ws_root,
            "root": root,
            "files": entries,
            "entries": entries,
        }),
    ))
}

pub fn handle_workspace_file(
    state: &DesktopState,
    session_id: &str,
    query: &HashMap<String, String>,
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let root = query.get("root").map(String::as_str).unwrap_or("workspace");
    let path = query
        .get("path")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("missing `path` query parameter"))?;
    if root == "sources" && query.contains_key("start_line") {
        let first = query.get("start_line").and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0)
            .ok_or_else(|| bad("Invalid source line number."))?;
        let last = query.get("end_line").map(|v|v.parse::<usize>()).transpose().map_err(|_| bad("Invalid end line."))?.unwrap_or(first);
        if last < first || last - first > 512 { return Err(bad("Select a passage of up to 513 lines.")); }
        return source_line_window(&env,session_id,path,first,last);
    }
    handle_workspace_file_inner(&env, session_id, root, path)
}

fn source_line_window(env: &research_tools::ResearchEnv, session: &str, path: &str, first: usize, last: usize) -> ApiResult {
    use research_tools::fs_tools::MAX_FS_READ_FILE_BYTES;
    use std::io::Read;
    let file = env.sandbox().resolve_existing(SandboxRoot::Sources,session,path,false).map_err(|e|bad(e.to_string()))?;
    if !file.metadata.is_file() { return Err(bad("Choose a saved source file.")); }
    let mut bytes = Vec::new();
    std::fs::File::open(file.canonical_target).and_then(|f| f.take(MAX_FS_READ_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e|bad(e.to_string()))?;
    if bytes.len() as u64 > MAX_FS_READ_FILE_BYTES { return Err(bad("Source exceeds the text reader limit.")); }
    let text = std::str::from_utf8(&bytes).map_err(|_|bad("The source has no readable text."))?;
    let start = first.saturating_sub(8).max(1);
    let end = last.saturating_add(8);
    let mut content = String::new();
    let mut reached = 0;
    for (idx,line) in text.split_inclusive('\n').enumerate().skip(start - 1) {
        reached = idx + 1;
        if reached > end { break; }
        if content.len().saturating_add(line.len()) > 1024 * 1024 { return Err(bad("The selected passage is too large to display.")); }
        content.push_str(line);
    }
    if reached < last { return Err(bad("The cited lines are outside this source.")); }
    Ok((200,json!({"path":path,"content":content,"file_sha256":ledger::sha256_hex(&bytes),
        "first_line":start,"truncated":reached>end,"partial":start>1 || reached>end})))
}

fn handle_workspace_file_inner(
    env: &research_tools::ResearchEnv,
    session_id: &str,
    root: &str,
    path: &str,
) -> ApiResult {
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    let mut file_sha256 = None;
    let truncated;
    loop {
        let part = handle_fs_read(
            env,
            session_id,
            &json!({ "root": root, "path": path, "offset": offset, "max_chars": 12000 }),
        )
        .map_err(bad)?;
        let part_digest = part["file_sha256"]
            .as_str()
            .ok_or_else(|| internal("file digest unavailable"))?;
        if file_sha256.as_deref().is_some_and(|first| first != part_digest) {
            return Err(bad("The file changed while its preview was being read."));
        }
        file_sha256.get_or_insert_with(|| part_digest.to_string());
        let text = part.get("content").and_then(Value::as_str).unwrap_or("");
        chunks.push(text.to_string());
        let more = part.get("truncated").and_then(Value::as_bool).unwrap_or(false);
        if !more || chunks.len() >= 16 {
            truncated = more;
            break;
        }
        offset = part
            .get("next_offset")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(offset + text.chars().count());
    }
    let full_text = chunks.concat();
    Ok((
        200,
        json!({
            "path": path,
            "root": root,
            "file_sha256": file_sha256,
            "content": full_text,
            "text": full_text,
            "truncated": truncated,
        }),
    ))
}

pub fn handle_latex_patch(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let path = val
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`path` is required"))?;

    let env = Arc::new(state.research_env_for_session(session_id).map_err(bad)?);
    if let Some(expected) = val.get("expected_workspace_root") {
        let current = env.workspace_root.as_ref().map(|p| p.display().to_string());
        if expected.as_str().is_none() || expected.as_str() != current.as_deref() {
            return Err((409, "The attached workspace changed. Reopen the manuscript before saving.".into()));
        }
    }
    let has_search_replace = val
        .get("search")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
        && val.get("replace").and_then(Value::as_str).is_some();

    let (tool_id, mut args) = if has_search_replace {
        (
            LATEX_PATCH,
            json!({
                "path": path,
                "search": val["search"],
                "replace": val["replace"],
                "replace_all": val.get("replace_all").and_then(Value::as_bool).unwrap_or(false),
            }),
        )
    } else {
        let content = val
            .get("content")
            .or_else(|| val.get("replace"))
            .and_then(Value::as_str)
            .ok_or_else(|| bad("`content` or (`search` + `replace`) is required"))?;
        (FS_WRITE, json!({ "path": path, "content": content }))
    };
    if let Some(expected) = val.get("expected_sha256") {
        let expected = match expected {
            Value::Null => None,
            Value::String(s) if s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()) => Some(s.to_ascii_lowercase()),
            _ => return Err(bad("`expected_sha256` must be a 64-digit SHA-256 hex digest or null")),
        };
        let current = workspace_edit_sha256(&env, path).map_err(bad)?;
        if expected != current {
            return Err((409, format!("{path} {EDIT_BASE_CHANGED}")));
        }
        args["expected_sha256"] = json!(expected);
    }

    let (router, _, _) = build_desktop_router(Arc::clone(state), env, session_id).map_err(internal)?;
    match router.try_call("user:canvas", tool_id, &args) {
        Ok(tools::CallOutcome::Ok(output)) => {
            Ok((200, json!({ "ok": true, "tool": tool_id, "output": output })))
        }
        Ok(tools::CallOutcome::Denied(reason)) => Err((403, reason)),
        Ok(tools::CallOutcome::Error(reason)) if reason.ends_with(EDIT_BASE_CHANGED) => Err((409, reason)),
        Ok(tools::CallOutcome::Error(reason)) => Err(bad(reason)),
        Err(reason) => Err(internal(reason)),
    }
}

pub fn handle_tex_audit(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let target_path = val
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut candidate_texts = Vec::new();
    if let Some(tex) = val.get("candidate_tex").and_then(Value::as_str) {
        let name = target_path.clone().unwrap_or_else(|| "candidate.tex".to_string());
        candidate_texts.push((name, tex.to_string()));
    }
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let ledger = state.open_ledger().map_err(internal)?;
    let report = run_tex_audit(
        &env,
        session_id,
        Some(&ledger),
        &AuditOptions {
            candidate_only: target_path.is_none() && !candidate_texts.is_empty(),
            target_path,
            candidate_texts,
            ..AuditOptions::default()
        },
    )
    .map_err(bad)?;
    Ok((200, serde_json::to_value(report).map_err(|e| internal(e.to_string()))?))
}

pub fn handle_lean_check(state: &Arc<DesktopState>, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let obj = val
        .as_object()
        .ok_or_else(|| bad("request body must be a JSON object"))?;
    let mut args = serde_json::Map::new();
    for key in ["path", "code", "allow_axioms"] {
        if let Some(v) = obj.get(key).filter(|v| !v.is_null()) {
            args.insert(key.to_string(), v.clone());
        }
    }
    let env = Arc::new(state.research_env_for_session(session_id).map_err(bad)?);
    let router =
        crate::turn_runner::build_lean_router(Arc::clone(state), env, session_id).map_err(internal)?;
    match router.try_call("user:desktop-lean", research_tools::LEAN_CHECK, &Value::Object(args)) {
        Ok(tools::CallOutcome::Ok(output)) => Ok((200, output)),
        Ok(tools::CallOutcome::Denied(reason)) => Err((403, reason)),
        Ok(tools::CallOutcome::Error(reason)) => Err(bad(reason)),
        Err(reason) => Err(internal(reason)),
    }
}

pub fn handle_literature(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let source = val.get("source").and_then(Value::as_str).unwrap_or("arxiv");
    let query = val
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`query` is required"))?
        .to_string();
    let max_results = val.get("max_results").and_then(Value::as_u64).map(|n| n as usize);
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let ledger = state.open_ledger().map_err(internal)?;

    if source.eq_ignore_ascii_case("arxiv") {
        let papers = run_arxiv_search(
            &env,
            &ledger,
            session_id,
            &ArxivSearchRequest {
                query,
                field: val.get("field").and_then(Value::as_str).map(str::to_string),
                max_results,
                include_source: val.get("include_source").and_then(Value::as_bool),
                section_filter: val
                    .get("section_filter")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
        )
        .map_err(bad)?;
        Ok((
            200,
            json!({
                "count": papers.len(),
                "papers": papers,
                "results": papers,
                "hits": papers,
            }),
        ))
    } else {
        let hits = run_literature_search(
            &env,
            &ledger,
            session_id,
            &LiteratureRequest {
                source: Some(source.to_string()),
                query,
                max_results,
            },
        )
        .map_err(bad)?;
        Ok((
            200,
            json!({
                "count": hits.len(),
                "hits": hits,
                "results": hits,
            }),
        ))
    }
}

pub(crate) fn handle_paper_passages(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let args = parse_json_body(body)?;
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let ledger = state.open_ledger().map_err(internal)?;
    let result = research_tools::paper_passages::run_search(&env, &ledger, session_id, &args)
        .map_err(bad)?;
    Ok((200, result))
}

pub(crate) fn handle_formal_library(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let args = parse_json_body(body)?;
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let result = research_tools::formal_library::handle_search_formal_library(&env, session_id, &args)
        .map_err(bad)?;
    Ok((200, result))
}

pub fn handle_dag(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let dag = read_proof_dag(&ledger, session_id).map_err(internal)?;
    Ok((200, serde_json::to_value(dag).map_err(|e| internal(e.to_string()))?))
}

pub fn handle_verify(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    match ledger.verify_session(session_id) {
        Ok(rep) => Ok((
            200,
            json!({
                "ok": true,
                "session": rep.session,
                "event_count": rep.event_count,
                "events_verified": rep.event_count,
                "head_hash": rep.head_hash,
                "chain_head": rep.head_hash,
                "blob_refs_verified": rep.blob_refs_verified,
                "blobs_verified": rep.blob_refs_verified,
            }),
        )),
        Err(err) => Err((400, format!("verification failed: {err}"))),
    }
}

pub use crate::agent_models::NEEDS_API_PROVIDER;

pub fn handle_narrate(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let (provider, model) =
        crate::agent_models::resolve_in_core_role_model(state, "narrator", Some(&val), true)?;
    let transport = state
        .build_transport(provider, &model)
        .map_err(bad)?;
    let ledger = state.open_ledger().map_err(internal)?;
    let report = narrate_session(&*transport, &ledger, session_id, &model).map_err(bad)?;
    Ok((
        200,
        json!({
            "narrative": format!("{}\n\n{}", report.walkthrough, report.derivation_sheet),
            "walkthrough": report.walkthrough,
            "derivation_sheet": report.derivation_sheet,
            "entry_count": report.entry_count,
            "raw_blob": report.raw_blob,
            "event_seq": report.event_seq,
        }),
    ))
}

pub fn handle_get_sidetalk(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let turns = read_sidetalk_thread(&ledger, session_id).map_err(internal)?;
    Ok((200, json!({ "session": session_id, "turns": turns })))
}

pub fn handle_post_sidetalk(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let _activity = crate::session_lifecycle::reserve_turn(state, session_id)?;
    let val = parse_json_body(body)?;
    let question = val
        .get("question")
        .or_else(|| val.get("prompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`question` is required"))?;
    let (provider, model) =
        crate::agent_models::resolve_in_core_role_model(state, "explain", Some(&val), true)?;
    let transport = state
        .build_transport(provider, &model)
        .map_err(bad)?;
    let env = state.research_env_for_session(session_id).map_err(bad)?;
    let ledger = state.open_ledger().map_err(internal)?;
    let turn = ask_sidetalk(&*transport, &ledger, &env, session_id, question, &model).map_err(bad)?;
    Ok((200, serde_json::to_value(turn).map_err(|e| internal(e.to_string()))?))
}

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
    let fmt = ExportFormat::parse(fmt_str).map_err(bad)?;
    let (filename, bytes) = export_session_bytes(&state.cfg.data_dir, session_id, fmt).map_err(bad)?;
    let content_type = match fmt {
        ExportFormat::Tex => "application/x-tex; charset=utf-8",
        ExportFormat::Html => "text/html; charset=utf-8",
        ExportFormat::Zip => "application/zip",
    };
    Ok((filename, content_type, bytes))
}

fn parse_json_upload_files(body: &[u8]) -> Result<Vec<UploadedFile>, (u16, String)> {
    let val = parse_json_body(body)?;
    let arr = val
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("expected JSON object with `files` array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("each upload item requires `name`"))?
            .to_string();
        let b64 = item
            .get("bytes_base64")
            .or_else(|| item.get("content_base64"))
            .and_then(Value::as_str)
            .ok_or_else(|| bad("each upload item requires `bytes_base64` or `content_base64`"))?;
        let bytes = decode_base64(b64).map_err(bad)?;
        out.push((name, bytes));
    }
    let _ = SandboxRoot::Sources;
    Ok(out)
}
