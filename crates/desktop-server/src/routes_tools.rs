//! Research tool endpoints: upload, workspace browsing, LaTeX patching, TeX audit,
//! literature search, proof DAG, chain verification, narration, side-talk, and export.

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::{decode_base64, parse_multipart_files, DesktopState, UploadedFile};
use crate::turn_runner::build_desktop_router;
use agent::{ask_sidetalk, narrate_session, read_sidetalk_thread, ProviderKind};
use export::{export_session_bytes, ExportFormat};
use research_tools::fs_tools::{handle_fs_list, handle_fs_read};
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
    for (name, bytes) in &items {
        let report = ingest_bytes(
            &sandbox,
            &ledger,
            &IngestRequest {
                session_id,
                file_name: name,
                origin: name,
                raw_bytes: bytes,
                source_id_hint: None,
                section_filter: None,
            },
        )
        .map_err(bad)?;
        ingested.push(serde_json::to_value(report).map_err(|e| internal(e.to_string()))?);
    }

    Ok((200, json!({ "ok": true, "ingested": ingested })))
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
    let res = match handle_fs_list(
        &env,
        session_id,
        &json!({ "root": root, "recursive": true, "max_entries": 1000 }),
    ) {
        Ok(v) => v,
        Err(msg) if msg.contains("no workspace attached") && root == "workspace" => {
            json!({ "entries": [] })
        }
        Err(e) => return Err(bad(e)),
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
    handle_workspace_file_inner(&env, session_id, root, path)
}

fn handle_workspace_file_inner(
    env: &research_tools::ResearchEnv,
    session_id: &str,
    root: &str,
    path: &str,
) -> ApiResult {
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    let truncated;
    loop {
        let part = handle_fs_read(
            env,
            session_id,
            &json!({ "root": root, "path": path, "offset": offset, "max_chars": 12000 }),
        )
        .map_err(bad)?;
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
    let has_search_replace = val
        .get("search")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
        && val.get("replace").and_then(Value::as_str).is_some();

    let (tool_id, args) = if has_search_replace {
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

    let (router, _, _) = build_desktop_router(Arc::clone(state), env, session_id).map_err(internal)?;
    match router.try_call("user:canvas", tool_id, &args) {
        Ok(tools::CallOutcome::Ok(output)) => {
            Ok((200, json!({ "ok": true, "tool": tool_id, "output": output })))
        }
        Ok(tools::CallOutcome::Denied(reason)) => Err((403, reason)),
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
            target_path,
            candidate_texts,
            ..AuditOptions::default()
        },
    )
    .map_err(bad)?;
    Ok((200, serde_json::to_value(report).map_err(|e| internal(e.to_string()))?))
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

pub fn handle_narrate(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let ui = state.load_ui_settings();
    let provider_str = val
        .get("provider")
        .and_then(Value::as_str)
        .or(ui.default_provider.as_deref())
        .unwrap_or("anthropic");
    let provider = ProviderKind::parse(provider_str).map_err(bad)?;
    let model = val
        .get("model")
        .and_then(Value::as_str)
        .map(|m| provider.resolve_model(m))
        .unwrap_or_else(|| provider.default_model().to_string());
    let transport = state
        .build_transport(provider, &model, val.get("api_base").and_then(Value::as_str))
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
    let val = parse_json_body(body)?;
    let question = val
        .get("question")
        .or_else(|| val.get("prompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("`question` is required"))?;
    let ui = state.load_ui_settings();
    let provider_str = val
        .get("provider")
        .and_then(Value::as_str)
        .or(ui.default_provider.as_deref())
        .unwrap_or("anthropic");
    let provider = ProviderKind::parse(provider_str).map_err(bad)?;
    let model = val
        .get("model")
        .and_then(Value::as_str)
        .map(|m| provider.resolve_model(m))
        .unwrap_or_else(|| provider.default_model().to_string());
    let transport = state
        .build_transport(provider, &model, val.get("api_base").and_then(Value::as_str))
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
