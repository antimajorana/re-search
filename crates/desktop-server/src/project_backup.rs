//! Portable, project-scoped backups. Payload files are content-addressed and never extracted by ZIP path.
use super::{bad, conflict, internal, not_found, ApiResult, Project, ProjectStore, Result, SessionMeta, MAX_PROJECTS};
use crate::{experiments, validation_plans};
use crate::state::DesktopState;
use ledger::{sha256_hex, Event, Ledger};
use research_tools::{is_symlink_or_junction, sandbox::sanitize_rel_components, validate_session_id, WorkspaceSandbox};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

pub(crate) const MAX_ARCHIVE: usize = 60 * 1024 * 1024;
const MAX_CONTENT: usize = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 16384;
const FORMAT: &str = "reSearch-project-v1";

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    project: Project,
    sessions: BTreeMap<String, SessionMeta>,
    ledgers: BTreeMap<String, Vec<Event>>,
    files: BTreeMap<String, String>,
    experiments: Vec<experiments::Experiment>,
    validation_plans: Vec<validation_plans::StoredPlan>,
}

fn invalid(error: impl std::fmt::Display) -> super::ApiError { bad(format!("Project backup: {error}")) }
fn storage(error: impl std::fmt::Display) -> super::ApiError { internal(format!("Project storage: {error}")) }
fn is_digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn components(rel: &str) -> Result<Vec<String>> {
    let (parts, canonical) = sanitize_rel_components(rel, false).map_err(invalid)?;
    if canonical != rel || parts.iter().any(|p| p.contains(['<','>','"','|','?','*'])) {
        return Err(invalid("invalid portable filename"));
    }
    Ok(parts)
}

// Check each component before reading any user-controlled sidecar.
fn checked_path(root: &Path, rel: &str) -> Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for part in components(rel)? {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if is_symlink_or_junction(&path, &meta) => return Err(invalid("linked files cannot be backed up or restored")),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(storage(e)),
        }
    }
    Ok(Some(path))
}

fn read_limited(file: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes).map_err(invalid)?;
    if bytes.len() > limit { return Err(invalid("expanded backup exceeds the 256 MiB limit")); }
    Ok(bytes)
}

fn recorded_sources(events: &[Event]) -> Result<BTreeMap<String, Option<String>>> {
    let mut files = BTreeMap::new();
    for event in events.iter().filter(|e| e.kind == "source.ingested") {
        let p = &event.payload;
        let paths = p["files"].as_array().into_iter().flatten().chain([&p["primary_path"], &p["assembled_path"]]);
        for path in paths.filter_map(|v| v.as_str()) {
            let parts = components(path)?;
            if parts.len() < 2 || Some(parts[0].as_str()) != p["source_id"].as_str() {
                return Err(invalid("source does not belong to its recorded source ID"));
            }
            let hash = p.get("files_sha256").map(|hashes| hashes[path].as_str().filter(|s| is_digest(s))
                .map(str::to_owned).ok_or_else(|| invalid("missing source digest"))).transpose()?;
            files.insert(path.to_string(), hash);
        }
    }
    Ok(files)
}

#[derive(Default)]
struct Objects {
    bytes: BTreeMap<String, Vec<u8>>,
    size: usize,
}

impl Objects {
    fn add(&mut self, bytes: Vec<u8>) -> Result<String> {
        let hash = sha256_hex(&bytes);
        if !self.bytes.contains_key(&hash) {
            self.size += bytes.len();
            if self.size > MAX_CONTENT || self.bytes.len() + 3 > MAX_ENTRIES {
                return Err(invalid("project exceeds the backup size or entry limit"));
            }
            self.bytes.insert(hash.clone(), bytes);
        }
        Ok(hash)
    }
    fn file(&mut self, path: &Path) -> Result<String> {
        let file = fs::File::open(path).map_err(storage)?;
        self.add(read_limited(file, MAX_CONTENT)?)
    }
}

pub(crate) fn export(state: &DesktopState, id: &str) -> Result<Vec<u8>> {
    super::with_project(state, id, |root, project| {
        let ledger = state.open_ledger().map_err(storage)?;
        let mut data = Manifest {
            format: FORMAT.into(), project: project.clone(), sessions: BTreeMap::new(), ledgers: BTreeMap::new(),
            files: BTreeMap::new(), experiments: experiments::backup_experiments(state, id)?,
            validation_plans: validation_plans::backup_plans(root, id)?,
        };
        let mut objects = Objects::default();
        for sid in &project.session_ids {
            validate_session_id(sid).map_err(invalid)?;
            if crate::session_lifecycle::is_turn_running(state, sid) {
                return Err(conflict("Wait for the project's active research turn to finish, then back up again."));
            }
            for key in [sid.clone(), format!("sidetalk:{sid}")] {
                match ledger.backup_session(&key) {
                    Ok((events, hashes)) => {
                        for hash in hashes {
                            if !objects.bytes.contains_key(&hash) {
                                objects.add(ledger.get_blob(&hash).map_err(storage)?)?;
                            }
                        }
                        data.ledgers.insert(key, events);
                    }
                    Err(ledger::Error::UnknownSession(_)) => (),
                    Err(e) => return Err(invalid(e)),
                }
            }
            let mut meta: SessionMeta = match checked_path(root, &format!("sessions/{sid}/meta.json"))? {
                Some(path) => serde_json::from_slice(&read_limited(fs::File::open(path).map_err(storage)?, MAX_CONTENT)?).map_err(invalid)?,
                None => SessionMeta { id: sid.clone(), title: sid.clone(), deleted: !data.ledgers.contains_key(sid), ..Default::default() },
            };
            meta.workspace_root = None;
            if !meta.deleted {
                let rows = data.ledgers.get(sid).map(Vec::as_slice).unwrap_or_default();
                for (path, expected) in recorded_sources(rows)? {
                    let rel = format!("sessions/{sid}/sources/{path}");
                    let file = checked_path(root, &rel)?.ok_or_else(|| invalid(format!("missing recorded source {path}")))?;
                    let hash = objects.file(&file)?;
                    if expected.is_some_and(|h| h != hash) { return Err(invalid(format!("source changed: {path}"))); }
                    data.files.insert(rel, hash);
                }
                for name in ["sidetalk.jsonl", "manuscript-drafts.json"] {
                    let rel = format!("sessions/{sid}/{name}");
                    if let Some(path) = checked_path(root, &rel)? { data.files.insert(rel, objects.file(&path)?); }
                }
            }
            data.sessions.insert(sid.clone(), meta);
        }
        let manifest = serde_json::to_vec(&data).map_err(storage)?;
        if manifest.len() + objects.size > MAX_CONTENT { return Err(invalid("expanded backup exceeds 256 MiB")); }
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("project.json", options).map_err(storage)?;
        archive.write_all(&manifest).map_err(storage)?;
        archive.start_file("project.sha256", options).map_err(storage)?;
        archive.write_all(sha256_hex(&manifest).as_bytes()).map_err(storage)?;
        for (hash, bytes) in objects.bytes {
            archive.start_file(format!("objects/{hash}"), options).map_err(storage)?;
            archive.write_all(&bytes).map_err(storage)?;
        }
        let bytes = archive.finish().map_err(storage)?.into_inner();
        if bytes.len() > MAX_ARCHIVE { return Err(invalid("compressed project backup exceeds 60 MiB")); }
        Ok(bytes)
    })
}

fn unpack(body: &[u8]) -> Result<(Manifest, Objects)> {
    if body.len() > MAX_ARCHIVE { return Err(invalid("project backup exceeds 60 MiB")); }
    let mut archive = zip::ZipArchive::new(Cursor::new(body)).map_err(invalid)?;
    if archive.len() > MAX_ENTRIES { return Err(invalid("too many entries")); }
    let mut names = BTreeSet::new();
    let mut manifest = Vec::new();
    let mut checksum = Vec::new();
    let mut objects = Objects::default();
    let mut remaining = MAX_CONTENT;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(invalid)?;
        let name = file.name().map_err(invalid)?.into_owned();
        if !names.insert(name.clone()) { return Err(invalid("duplicate archive entry")); }
        if file.size() > remaining as u64 { return Err(invalid("expanded backup exceeds 256 MiB")); }
        let bytes = read_limited(&mut file, remaining)?;
        remaining -= bytes.len();
        match name.as_str() {
            "project.json" => manifest = bytes,
            "project.sha256" => checksum = bytes,
            _ => {
                let expected = name.strip_prefix("objects/").filter(|s| is_digest(s)).ok_or_else(|| invalid("unrecognized ZIP entry"))?;
                if objects.add(bytes)? != expected { return Err(invalid("damaged backup object")); }
            }
        }
    }
    if checksum != sha256_hex(&manifest).as_bytes() { return Err(invalid("project manifest checksum mismatch")); }
    let data: Manifest = serde_json::from_slice(&manifest).map_err(invalid)?;
    if data.format != FORMAT { return Err(invalid("unsupported project backup version")); }
    Ok((data, objects))
}

fn validate(data: &Manifest, objects: &Objects) -> Result<()> {
    let project = &data.project;
    validate_session_id(&project.id).map_err(invalid)?;
    components(&project.id)?;
    if project.session_ids.len() > super::MAX_SESSIONS || project.items.len() > super::MAX_ITEMS || project.sources.len() > super::MAX_SOURCES
        || project.title.trim().is_empty() || project.title.chars().count() > 256 || project.revision == 0 {
        return Err(invalid("invalid project metadata"));
    }
    let ids: BTreeSet<_> = project.session_ids.iter().collect();
    if ids.len() != project.session_ids.len() || ids != data.sessions.keys().collect() {
        return Err(invalid("project and session IDs do not agree"));
    }
    for source in &project.sources {
        if let Some(url) = &source.url { super::validated_url(url.clone())?; }
    }
    let mut allowed = BTreeMap::new();
    for (sid, meta) in &data.sessions {
        validate_session_id(sid).map_err(invalid)?;
        components(sid)?;
        if sid != &meta.id { return Err(invalid("session metadata ID mismatch")); }
        if !meta.deleted {
            let events = data.ledgers.get(sid).map(Vec::as_slice).unwrap_or_default();
            for (path, expected) in recorded_sources(events)? {
                let rel = format!("sessions/{sid}/sources/{path}");
                if !data.files.contains_key(&rel) { return Err(invalid("referenced source file is missing")); }
                allowed.insert(rel, expected);
            }
            for name in ["sidetalk.jsonl", "manuscript-drafts.json"] { allowed.insert(format!("sessions/{sid}/{name}"), None); }
        }
    }
    let mut case_names = BTreeSet::new();
    for (rel, hash) in &data.files {
        let expected = allowed.get(rel).ok_or_else(|| invalid("unexpected session file"))?;
        if !case_names.insert(rel.to_lowercase()) || !objects.bytes.contains_key(hash)
            || expected.as_ref().is_some_and(|expected| expected != hash) {
            return Err(invalid("damaged or ambiguous session file"));
        }
    }
    for key in data.ledgers.keys() {
        let sid = key.strip_prefix("sidetalk:").unwrap_or(key);
        if !data.sessions.contains_key(sid) { return Err(invalid("ledger belongs to a different project")); }
    }
    Ok(())
}

pub(crate) fn restore(state: &DesktopState, body: &[u8]) -> ApiResult {
    let (mut data, objects) = unpack(body)?;
    validate(&data, &objects)?;
    let mut store = ProjectStore::open(state)?;
    let id = &data.project.id;
    if store.projects.iter().any(|p| &p.id == id) { return Err(conflict("This project already exists. Restore into a separate data directory to keep both copies.")); }
    if store.projects.len() >= MAX_PROJECTS { return Err(conflict("the project limit has been reached")); }
    let stage = tempfile::Builder::new().prefix(".restore-").tempdir_in(&store.root).map_err(storage)?;
    let staged = Ledger::open(stage.path()).map_err(storage)?;
    let mut refs = BTreeSet::new();
    for (hash, bytes) in &objects.bytes {
        if staged.put_blob(bytes).map_err(storage)? != *hash { return Err(invalid("object checksum mismatch")); }
    }
    staged.restore_sessions(&data.ledgers).map_err(invalid)?;
    for (sid, events) in &data.ledgers {
        if !events.is_empty() { refs.extend(staged.backup_session(sid).map_err(invalid)?.1); }
    }
    if objects.bytes.keys().any(|h| !refs.contains(h) && !data.files.values().any(|f| f == h)) {
        return Err(invalid("unreferenced backup object"));
    }

    // Materialize in a private tree before publishing anything. Workspace bindings and process grants
    // are local capabilities, so the restored session keeps only its scientific record and settings.
    for (sid, meta) in &mut data.sessions {
        meta.workspace_root = None;
        let parts = components(&format!("sessions/{sid}/meta.json"))?;
        WorkspaceSandbox::atomic_write_in_root(stage.path(), &parts, &serde_json::to_vec(meta).map_err(storage)?).map_err(storage)?;
    }
    for (rel, hash) in &data.files {
        WorkspaceSandbox::atomic_write_in_root(stage.path(), &components(rel)?, &objects.bytes[hash]).map_err(storage)?;
    }
    experiments::restore_experiments(state, &data.project, &data.experiments, false)?;
    validation_plans::restore_plans(&store.root, &data.project, &data.validation_plans, &staged, false)?;
    let db = state.open_ledger().map_err(storage)?;
    let marker_name = format!(".restore-{}.pending", sha256_hex(id.as_bytes()));
    let archive_hash = sha256_hex(body);
    let resuming = match checked_path(&store.root, &marker_name)? {
        Some(path) if fs::read(path).map_err(storage)? == archive_hash.as_bytes() => true,
        Some(_) => return Err(conflict("An interrupted restore is waiting for its original backup file.")),
        None => false,
    };
    checked_path(&store.root, "sessions")?;
    for sid in &data.project.session_ids {
        if store.session_owner(sid).is_some() || crate::session_lifecycle::is_turn_running(state, sid) {
            return Err(conflict(format!("Session {sid} already belongs to this installation.")));
        }
        let dest = checked_path(&store.root, &format!("sessions/{sid}"))?;
        if !resuming && dest.is_some() { return Err(conflict(format!("Session {sid} already exists."))); }
        for key in [sid.clone(), format!("sidetalk:{sid}")] {
            let rows = db.read_session(&key).map_err(storage)?;
            if !rows.is_empty() && (!resuming || data.ledgers.get(&key) != Some(&rows)) {
                return Err(conflict(format!("Session {sid} has existing research data.")));
            }
        }
        if resuming && dest.is_some() {
            let meta = format!("sessions/{sid}/meta.json");
            let expected = serde_json::to_vec(&data.sessions[sid]).map_err(storage)?;
            let current = checked_path(&store.root, &meta)?.ok_or_else(|| conflict("existing session conflicts with restore"))?;
            if fs::read(current).map_err(storage)? != expected { return Err(conflict("restored session has since changed")); }
            for (rel, hash) in data.files.range(format!("sessions/{sid}/")..) {
                if !rel.starts_with(&format!("sessions/{sid}/")) { break; }
                let path = checked_path(&store.root, rel)?.ok_or_else(|| conflict("restored files have since changed"))?;
                if sha256_hex(&read_limited(fs::File::open(path).map_err(storage)?, MAX_CONTENT)?) != *hash {
                    return Err(conflict("restored files have since changed"));
                }
            }
        }
    }
    store.projects.push(data.project.clone());
    if serde_json::to_vec(&store.projects).map_err(storage)?.len() > super::MAX_STORE_BYTES { return Err(conflict("project storage size limit exceeded")); }
    // The shared project lock serializes publishers. The marker makes a process or disk interruption
    // retryable with the exact same validated archive. Existing sessions are never overwritten.
    if !resuming {
        WorkspaceSandbox::atomic_write_in_root(&store.root, std::slice::from_ref(&marker_name), archive_hash.as_bytes()).map_err(storage)?;
    }
    for hash in refs { db.put_blob(&objects.bytes[&hash]).map_err(storage)?; db.get_blob(&hash).map_err(storage)?; }
    db.restore_sessions(&data.ledgers).map_err(invalid)?;
    fs::create_dir_all(store.root.join("sessions")).map_err(storage)?;
    for sid in data.sessions.keys() {
        let rel = format!("sessions/{sid}");
        if checked_path(&store.root, &rel)?.is_none() {
            fs::rename(stage.path().join(&rel), store.root.join(&rel)).map_err(storage)?;
        }
    }
    experiments::restore_experiments(state, &data.project, &data.experiments, true)?;
    validation_plans::restore_plans(&store.root, &data.project, &data.validation_plans, &staged, true)?;
    store.save()?;
    let _ = fs::remove_file(store.root.join(marker_name));
    Ok((201, json!({"id": id, "title": data.project.title, "sessions": data.sessions.len()})))
}


