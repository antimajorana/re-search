//! Session-local recovery snapshots. Workspace and file names are labels, never write destinations.
use crate::routes_session::{bad, conflict, internal, not_found, ApiResult};
use crate::state::DesktopState;
use research_tools::sandbox::is_symlink_or_junction;
use research_tools::{validate_session_id, WorkspaceSandbox};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::Path;
use tools::canon::random_hex;

const STORE: &str = "manuscript-drafts.json";
const MAX_STORE: usize = 16 * 1024 * 1024;
const MAX_TEXT: usize = 2 * 1024 * 1024;
type ErrMsg = (u16, String);

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Draft {
    text: String,
    original_content: String,
    file_hash: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    workspace: Option<String>,
    path: String,
    revision: String,
    draft: Draft,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    workspace: Option<String>,
    path: String,
    revision: Option<String>,
    draft: Option<Draft>,
}

fn io_error(err: impl std::fmt::Display) -> ErrMsg {
    internal(format!("Manuscript recovery storage: {err}"))
}

fn read(root: &Path, id: &str) -> Result<Vec<Record>, ErrMsg> {
    let mut path = root.to_owned();
    for part in ["sessions", id, STORE] {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(e)),
            Ok(meta) if is_symlink_or_junction(&path, &meta) => return Err(bad("Invalid manuscript storage path.")),
            Ok(_) => {}
        }
    }
    let mut bytes = Vec::new();
    File::open(path).map_err(io_error)?.take((MAX_STORE + 1) as u64)
        .read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() > MAX_STORE { return Err(bad("Manuscript recovery storage exceeds its limit.")); }
    serde_json::from_slice(&bytes).map_err(io_error)
}

pub(crate) fn handle(state: &DesktopState, id: &str, body: Option<&[u8]>) -> ApiResult {
    validate_session_id(id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(id).map_err(internal)? { return Err(not_found("Session not found.")); }
    let root = state.cfg.data_dir.canonicalize().map_err(io_error)?;
    let Some(body) = body else { return Ok((200, json!({ "drafts": read(&root, id)? }))); };
    if body.len() > MAX_STORE { return Err(bad("Recovery snapshot is too large.")); }
    let update: Update = serde_json::from_slice(body).map_err(|e| bad(e.to_string()))?;
    if update.path.len() > 4096 || update.workspace.as_ref().is_some_and(|s| s.len() > 4096) {
        return Err(bad("Recovery file label is too long."));
    }
    if let Some(d) = &update.draft {
        if d.text.len() > MAX_TEXT || d.original_content.len() > MAX_TEXT {
            return Err(bad("Recovery supports up to 2 MiB per manuscript."));
        }
        if d.file_hash.as_ref().is_some_and(|h| h.len() != 64 || !h.bytes().all(|c| c.is_ascii_hexdigit())) {
            return Err(bad("Invalid manuscript baseline hash."));
        }
    }
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(root.join("manuscript-drafts.lock")).map_err(io_error)?;
    lock.lock().map_err(io_error)?;
    let mut records = read(&root, id)?;
    let index = records.iter().position(|r| r.workspace == update.workspace && r.path == update.path);
    let current = index.map(|i| records[i].revision.as_str());
    if current != update.revision.as_deref() {
        return Err(conflict("Another window changed this recovery copy. Reopen the session before editing it."));
    }
    if let Some(i) = index { records.swap_remove(i); }
    let revision = update.draft.map(|draft| {
        let revision = random_hex(16);
        records.push(Record { workspace: update.workspace, path: update.path, revision: revision.clone(), draft });
        revision
    });
    let bytes = serde_json::to_vec(&records).map_err(io_error)?;
    if records.len() > 128 || bytes.len() > MAX_STORE {
        return Err(bad("Recovery storage for this session is full. Save or discard a draft and try again."));
    }
    WorkspaceSandbox::atomic_write_in_root(&root, &["sessions".into(), id.into(), STORE.into()], &bytes)
        .map_err(io_error)?;
    Ok((200, json!({ "revision": revision })))
}
