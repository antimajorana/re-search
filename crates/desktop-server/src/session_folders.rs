use crate::routes_session::{bad, conflict, internal, not_found, ApiResult};
use crate::state::DesktopState;
use research_tools::{validate_session_id, WorkspaceSandbox};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::Path;
use tools::canon::random_hex;

const STORE: &str = "session-folders.json";
const MAX_BYTES: usize = 4 * 1024 * 1024;
type Error = (u16, String);

#[derive(Default, Deserialize, Serialize)]
struct Catalog {
    revision: u64,
    folders: Vec<Folder>,
    members: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
struct Folder { id: String, name: String }

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Change {
    Create { name: String },
    Rename { id: String, name: String },
    Remove { id: String },
    Move { session: String, folder: Option<String> },
}

fn io_error(e: impl std::fmt::Display) -> Error { internal(format!("Session folders: {e}")) }

fn read(root: &Path) -> Result<Catalog, Error> {
    let path = root.join(STORE);
    let meta = match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Catalog::default()),
        other => other.map_err(io_error)?,
    };
    if !meta.is_file() || research_tools::sandbox::is_symlink_or_junction(&path, &meta) {
        return Err(io_error("invalid catalog file"));
    }
    let mut bytes = Vec::new();
    File::open(path).map_err(io_error)?.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() > MAX_BYTES { return Err(io_error("catalog exceeds its size limit")); }
    serde_json::from_slice(&bytes).map_err(io_error)
}

fn locked(root: &Path) -> Result<File, Error> {
    let guard = OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(root.join("session-folders.lock")).map_err(io_error)?;
    guard.lock().map_err(io_error)?;
    Ok(guard)
}

fn save(root: &Path, catalog: &mut Catalog) -> Result<(), Error> {
    catalog.revision = catalog.revision.checked_add(1).ok_or_else(|| io_error("revision limit"))?;
    let bytes = serde_json::to_vec(catalog).map_err(io_error)?;
    if bytes.len() > MAX_BYTES { return Err(conflict("Session folder catalog is full.")); }
    WorkspaceSandbox::atomic_write_in_root(root, &[STORE.into()], &bytes).map_err(io_error)?;
    Ok(())
}

fn name_for(input: &str, catalog: &Catalog, except: Option<&str>) -> Result<String, Error> {
    let name = input.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(bad("Use a folder name of 1 to 80 characters."));
    }
    if catalog.folders.iter().any(|f| Some(f.id.as_str()) != except && f.name.to_lowercase() == name.to_lowercase()) {
        return Err(bad("A folder already has that name."));
    }
    Ok(name.to_owned())
}

pub(crate) fn handle(state: &DesktopState, body: Option<&[u8]>) -> ApiResult {
    let root = state.cfg.data_dir.canonicalize().map_err(io_error)?;
    let Some(body) = body else { return Ok((200, json!(read(&root)?))); };
    let data = crate::routes_session::parse_json_body(body)?;
    let revision = data["revision"].as_u64().ok_or_else(|| bad("Missing folder revision."))?;
    let change: Change = serde_json::from_value(data).map_err(|e| bad(e.to_string()))?;
    let _guard = locked(&root)?;
    let mut catalog = read(&root)?;
    if catalog.revision != revision { return Err(conflict("Folders changed in another window. Review the list and try again.")); }
    let mut created = None;
    match change {
        Change::Create { name } => {
            if catalog.folders.len() >= 256 { return Err(bad("The session list supports up to 256 folders.")); }
            let name = name_for(&name, &catalog, None)?;
            let id = format!("f-{}", random_hex(12));
            catalog.folders.push(Folder { id: id.clone(), name });
            created = Some(id);
        }
        Change::Rename { id, name } => {
            let name = name_for(&name, &catalog, Some(&id))?;
            let folder = catalog.folders.iter_mut().find(|f| f.id == id).ok_or_else(|| not_found("Unknown session folder."))?;
            folder.name = name;
        }
        Change::Remove { id } => {
            let index = catalog.folders.iter().position(|f| f.id == id).ok_or_else(|| not_found("Unknown session folder."))?;
            catalog.folders.remove(index);
            catalog.members.retain(|_, folder| folder.as_str() != id.as_str());
        }
        Change::Move { session, folder } => {
            validate_session_id(&session).map_err(|e| bad(e.to_string()))?;
            if !state.session_exists(&session).map_err(internal)? { return Err(not_found("Unknown session.")); }
            if let Some(id) = folder {
                if !catalog.folders.iter().any(|f| f.id == id) { return Err(not_found("Unknown session folder.")); }
                catalog.members.insert(session, id);
            } else { catalog.members.remove(&session); }
        }
    }
    save(&root, &mut catalog)?;
    Ok((200, json!({ "catalog": catalog, "created": created })))
}

pub(crate) fn forget_session(state: &DesktopState, session: &str) -> Result<(), Error> {
    let root = state.cfg.data_dir.canonicalize().map_err(io_error)?;
    let _guard = locked(&root)?;
    let mut catalog = read(&root)?;
    if catalog.members.remove(session).is_some() { save(&root, &mut catalog)?; }
    Ok(())
}

pub(crate) fn backup_members(state: &DesktopState, sessions: &[String]) -> Result<BTreeMap<String, String>, Error> {
    let root = state.cfg.data_dir.canonicalize().map_err(io_error)?;
    let catalog = read(&root)?;
    let mut names = BTreeMap::new();
    for session in sessions {
        if let Some(id) = catalog.members.get(session) {
            let folder = catalog.folders.iter().find(|f| &f.id == id)
                .ok_or_else(|| io_error("session belongs to an unknown folder"))?;
            names.insert(session.clone(), folder.name.clone());
        }
    }
    Ok(names)
}

pub(crate) fn restore_members(state: &DesktopState, members: &BTreeMap<String, String>, apply: bool) -> Result<(), Error> {
    if members.is_empty() { return Ok(()); }
    let root = state.cfg.data_dir.canonicalize().map_err(io_error)?;
    let _guard = locked(&root)?;
    let mut catalog = read(&root)?;
    let mut changed = false;
    for (session, name) in members {
        let id = if let Some(folder) = catalog.folders.iter().find(|f| f.name.to_lowercase() == name.to_lowercase()) {
            folder.id.clone()
        } else {
            if catalog.folders.len() >= 256 { return Err(conflict("The backup would exceed the session folder limit.")); }
            let id = format!("f-{}", random_hex(12));
            let name = name_for(name, &catalog, None)?;
            catalog.folders.push(Folder { id: id.clone(), name });
            id
        };
        match catalog.members.get(session) {
            Some(current) if current != &id => return Err(conflict("The session already belongs to a different folder.")),
            Some(_) => (),
            None => { catalog.members.insert(session.clone(), id); changed = true; }
        }
    }
    if serde_json::to_vec(&catalog).map_err(io_error)?.len() >= MAX_BYTES { return Err(conflict("Session folder catalog is full.")); }
    if apply && changed { save(&root, &mut catalog)?; }
    Ok(())
}
