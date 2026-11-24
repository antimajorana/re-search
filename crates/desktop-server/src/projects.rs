//! Local research projects and their user-maintained records.

#[path = "project_backup.rs"]
pub(crate) mod backup;

use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::{DesktopState, SessionMeta};
use ledger::Event;
use research_tools::tex_scan::{normalize_doi, parse_bib_entries};
use research_tools::{validate_session_id, WorkspaceSandbox};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use tools::canon::random_hex;

const STORE_FILE: &str = "projects.json";
const MAX_STORE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROJECTS: usize = 128;
const MAX_SESSIONS: usize = 1024;
const MAX_ITEMS: usize = 256;
const MAX_SOURCES: usize = 256;

pub(crate) type ApiError = (u16, String);
type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Project {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) question: String,
    pub(crate) conventions: String,
    pub(crate) revision: u64,
    pub(crate) session_ids: Vec<String>,
    pub(crate) items: Vec<ProjectItem>,
    pub(crate) sources: Vec<ProjectSource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectItemKind {
    Finding,
    OpenQuestion,
    FailedApproach,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProjectItem {
    pub(crate) id: String,
    pub(crate) kind: ProjectItemKind,
    pub(crate) text: String,
    pub(crate) session_id: Option<String>,
    pub(crate) entry_id: Option<String>,
    pub(crate) source_id: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) quote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct ProjectSource {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) revision: u64,
    pub(crate) title: String,
    pub(crate) url: Option<String>,
    pub(crate) doi: Option<String>,
    pub(crate) bibtex: Option<String>,
    pub(crate) notes: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) quote: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) source_id: Option<String>,
}

struct ProjectStore {
    _lock: File,
    root: PathBuf,
    projects: Vec<Project>,
}

impl ProjectStore {
    fn open(state: &DesktopState) -> Result<Self> {
        let root = state.cfg.data_dir.canonicalize().map_err(io_error)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("projects.lock"))
            .map_err(io_error)?;
        lock.lock().map_err(io_error)?;
        let projects = read_projects(&root)?;
        Ok(Self {
            _lock: lock,
            root,
            projects,
        })
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.projects).map_err(|e| internal(e.to_string()))?;
        if bytes.len() > MAX_STORE_BYTES {
            return Err(conflict("the project store has reached its size limit"));
        }
        WorkspaceSandbox::atomic_write_in_root(&self.root, &[STORE_FILE.into()], &bytes)
            .map_err(|e| internal(format!("could not save projects: {e}")))?;
        #[cfg(unix)]
        let _ = File::open(&self.root).and_then(|directory| directory.sync_all());
        Ok(())
    }

    fn project_index(&self, id: &str) -> Result<usize> {
        self.projects
            .iter()
            .position(|project| project.id == id)
            .ok_or_else(|| not_found(format!("project `{id}` not found")))
    }

    fn session_owner(&self, session_id: &str) -> Option<&Project> {
        self.projects.iter().find(|project| {
            project
                .session_ids
                .iter()
                .any(|session| session == session_id)
        })
    }

    fn link_session(&mut self, index: usize, session_id: &str) -> Result<()> {
        let project = &mut self.projects[index];
        if project.session_ids.len() >= MAX_SESSIONS {
            return Err(conflict("the project has reached its session limit"));
        }
        project.session_ids.push(session_id.to_string());
        revise(project)
    }
}

fn io_error(error: io::Error) -> ApiError {
    internal(format!("project storage: {error}"))
}

fn read_projects(root: &Path) -> Result<Vec<Project>> {
    let file = match File::open(root.join(STORE_FILE)) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(io_error(err)),
    };
    let mut bytes = Vec::new();
    file.take((MAX_STORE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(internal("the project store exceeds its size limit"));
    }
    serde_json::from_slice(&bytes).map_err(|e| internal(format!("could not read projects: {e}")))
}

fn validate_id(id: &str) -> Result<()> {
    validate_session_id(id).map_err(|e| bad(e.to_string()))
}

fn read_body<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    serde_json::from_value(parse_json_body(body)?)
        .map_err(|e| bad(format!("invalid project request: {e}")))
}

fn project_res(status: u16, project: Project) -> ApiResult {
    Ok((status, json!(project)))
}

pub(crate) fn list_projects(state: &DesktopState) -> Result<Vec<Project>> {
    Ok(ProjectStore::open(state)?.projects)
}

pub(crate) fn load_project(state: &DesktopState, id: &str) -> Result<Option<Project>> {
    validate_id(id)?;
    Ok(list_projects(state)?
        .into_iter()
        .find(|project| project.id == id))
}

pub(crate) fn project_for_session(
    state: &DesktopState,
    session_id: &str,
) -> Result<Option<Project>> {
    validate_id(session_id)?;
    Ok(list_projects(state)?.into_iter().find(|project| {
        project
            .session_ids
            .iter()
            .any(|session| session == session_id)
    }))
}

fn get_project(state: &DesktopState, id: &str) -> Result<Project> {
    load_project(state, id)?.ok_or_else(|| not_found(format!("project `{id}` not found")))
}

/// Hold the shared project lock while reading or updating local project adjuncts.
pub(crate) fn with_project<T>(
    state: &DesktopState,
    id: &str,
    action: impl FnOnce(&Path, &Project) -> Result<T>,
) -> Result<T> {
    validate_id(id)?;
    let store = ProjectStore::open(state)?;
    let index = store.project_index(id)?;
    action(&store.root, &store.projects[index])
}

fn revise(project: &mut Project) -> Result<()> {
    project.revision = project
        .revision
        .checked_add(1)
        .ok_or_else(|| conflict("the project revision counter is exhausted"))?;
    Ok(())
}

fn edit_project(
    state: &DesktopState,
    id: &str,
    edit: impl FnOnce(&mut Project) -> Result<()>,
) -> Result<Project> {
    validate_id(id)?;
    let mut store = ProjectStore::open(state)?;
    let index = store.project_index(id)?;
    let mut project = store.projects[index].clone();
    edit(&mut project)?;
    if project != store.projects[index] {
        revise(&mut project)?;
        store.projects[index] = project.clone();
        store.save()?;
    }
    Ok(project)
}

pub(crate) fn attach_session(state: &DesktopState, id: &str, session_id: &str) -> Result<Project> {
    validate_id(id)?;
    validate_id(session_id)?;
    let mut store = ProjectStore::open(state)?;
    let index = store.project_index(id)?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    if let Some(owner) = store.session_owner(session_id) {
        if owner.id == id {
            return Ok(owner.clone());
        }
        return Err(conflict(format!(
            "session `{session_id}` already belongs to project `{}`",
            owner.id
        )));
    }
    store.link_session(index, session_id)?;
    store.save()?;
    Ok(store.projects[index].clone())
}

pub(crate) fn save_new_session(
    state: &DesktopState,
    meta: &SessionMeta,
    project_id: Option<&str>,
) -> Result<()> {
    // Serialize creation with other creators and with project attach. In particular a
    // competing request for the same ID must never roll back a session it did not create.
    let mut store = ProjectStore::open(state)?;
    if state.session_exists(&meta.id).map_err(internal)? {
        return Err(conflict(format!("session `{}` already exists", meta.id)));
    }
    if store.session_owner(&meta.id).is_some() {
        return Err(conflict(format!(
            "session `{}` is retained in a project",
            meta.id
        )));
    }
    if let Some(project_id) = project_id {
        validate_id(project_id)?;
        let index = store.project_index(project_id)?;
        store.link_session(index, &meta.id)?;
    }
    state.save_meta(meta).map_err(internal)?;
    if project_id.is_some() {
        if let Err(error) = store.save() {
            let tombstone = SessionMeta {
                deleted: true,
                ..meta.clone()
            };
            state.save_meta(&tombstone).map_err(internal)?;
            return Err(error);
        }
    }
    Ok(())
}

fn check_text(value: String, field: &str, max: usize) -> Result<String> {
    let value = value.trim();
    if value.chars().count() > max {
        return Err(bad(format!(
            "{field} must contain at most {max} characters"
        )));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
    {
        return Err(bad(format!("{field} contains control characters")));
    }
    Ok(value.to_string())
}

fn clear_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn required(value: String, field: &str) -> Result<String> {
    clear_empty(value).ok_or_else(|| bad(format!("{field} is required")))
}

fn check_optional(value: Option<String>, field: &str, max: usize) -> Result<Option<String>> {
    value.map(|text| check_text(text, field, max)).transpose()
}

fn check_reference(id: Option<&str>) -> Result<()> {
    if let Some(id) = id {
        validate_id(id)?;
    }
    Ok(())
}

fn check_backlinks(
    state: &DesktopState,
    project: &Project,
    session_id: Option<&str>,
    entry_id: Option<&str>,
    source_id: Option<&str>,
) -> Result<()> {
    let session_events = if let Some(session) = session_id {
        if !project.session_ids.iter().any(|id| id == session)
            || !state.session_exists(session).map_err(internal)?
        {
            return Err(bad(format!(
                "session `{session}` must belong to this project"
            )));
        }
        if entry_id.is_some() || source_id.is_some() {
            state
                .open_ledger()
                .map_err(internal)?
                .read_verified_session(session)
                .map_err(|e| {
                    bad(format!(
                        "could not verify backlinks in session `{session}`: {e}"
                    ))
                })?
                .1
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let has = |events: &[Event], kind: &str, field: &str, id: &str| {
        events
            .iter()
            .any(|event| event.kind == kind && event.payload[field].as_str() == Some(id))
    };
    if let Some(entry) = entry_id {
        if !has(&session_events, "proof.entry", "entry_id", entry) {
            return Err(bad(format!(
                "proof entry `{entry}` was not found in the linked session"
            )));
        }
    }
    if let Some(source) = source_id {
        if !has(&session_events, "source.ingested", "source_id", source) {
            return Err(bad(format!(
                "source `{source}` was not found in the linked session"
            )));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectInput {
    title: Option<String>,
    question: Option<String>,
    conventions: Option<String>,
    revision: Option<u64>,
}

impl ProjectInput {
    fn apply(self, project: &mut Project) -> Result<()> {
        if self
            .revision
            .is_some_and(|revision| revision != project.revision)
        {
            return Err(conflict("the project changed since it was loaded"));
        }
        if let Some(title) = self.title {
            project.title = required(check_text(title, "title", 256)?, "title")?;
        }
        if let Some(question) = self.question {
            project.question = check_text(question, "question", 16_000)?;
        }
        if let Some(conventions) = self.conventions {
            project.conventions = check_text(conventions, "conventions", 2000)?;
        }
        Ok(())
    }
}

pub(crate) fn handle_list(state: &DesktopState) -> ApiResult {
    Ok((200, json!({ "projects": list_projects(state)? })))
}

pub(crate) fn handle_create(state: &DesktopState, body: &[u8]) -> ApiResult {
    let input: ProjectInput = read_body(body)?;
    let mut store = ProjectStore::open(state)?;
    if store.projects.len() >= MAX_PROJECTS {
        return Err(conflict("the project store has reached its project limit"));
    }
    let mut project = Project {
        id: format!("p-{}", random_hex(12)),
        title: String::new(),
        question: String::new(),
        conventions: String::new(),
        revision: 1,
        session_ids: Vec::new(),
        items: Vec::new(),
        sources: Vec::new(),
    };
    input.apply(&mut project)?;
    if project.title.is_empty() {
        return Err(bad("title is required"));
    }
    store.projects.push(project.clone());
    store.save()?;
    project_res(201, project)
}

pub(crate) fn handle_get(state: &DesktopState, id: &str) -> ApiResult {
    project_res(200, get_project(state, id)?)
}

pub(crate) fn handle_put(state: &DesktopState, id: &str, body: &[u8]) -> ApiResult {
    let input: ProjectInput = read_body(body)?;
    project_res(
        200,
        edit_project(state, id, |project| input.apply(project))?,
    )
}

pub(crate) fn handle_attach(state: &DesktopState, id: &str, session_id: &str) -> ApiResult {
    project_res(200, attach_session(state, id, session_id)?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemInput {
    kind: ProjectItemKind,
    text: String,
    session_id: Option<String>,
    entry_id: Option<String>,
    source_id: Option<String>,
    location: Option<String>,
    quote: Option<String>,
}

pub(crate) fn handle_add_item(state: &DesktopState, id: &str, body: &[u8]) -> ApiResult {
    let input: ItemInput = read_body(body)?;
    let item = ProjectItem {
        id: format!("item-{}", random_hex(12)),
        kind: input.kind,
        text: required(check_text(input.text, "text", 4000)?, "text")?,
        session_id: input.session_id.and_then(clear_empty),
        entry_id: input.entry_id.and_then(clear_empty),
        source_id: input.source_id.and_then(clear_empty),
        location: check_optional(input.location, "location", 256)?.and_then(clear_empty),
        quote: check_optional(input.quote, "quote", 3000)?.and_then(clear_empty),
    };
    check_reference(item.session_id.as_deref())?;
    check_reference(item.entry_id.as_deref())?;
    check_reference(item.source_id.as_deref())?;
    let project = edit_project(state, id, |project| {
        if project.items.len() >= MAX_ITEMS {
            return Err(conflict("the project has reached its research item limit"));
        }
        let ingested_source = item
            .source_id
            .as_deref()
            .filter(|id| !project.sources.iter().any(|source| source.id == *id));
        check_backlinks(
            state,
            project,
            item.session_id.as_deref(),
            item.entry_id.as_deref(),
            ingested_source,
        )?;
        project.items.push(item);
        Ok(())
    })?;
    project_res(200, project)
}

pub(crate) fn handle_delete_item(state: &DesktopState, id: &str, item_id: &str) -> ApiResult {
    validate_id(item_id)?;
    let project = edit_project(state, id, |project| {
        let index = project
            .items
            .iter()
            .position(|item| item.id == item_id)
            .ok_or_else(|| not_found(format!("project item `{item_id}` not found")))?;
        project.items.remove(index);
        Ok(())
    })?;
    project_res(200, project)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInput {
    id: Option<String>,
    revision: Option<u64>,
    #[serde(default)]
    create_only: bool,
    title: Option<String>,
    url: Option<String>,
    doi: Option<String>,
    bibtex: Option<String>,
    notes: Option<String>,
    location: Option<String>,
    quote: Option<String>,
    session_id: Option<String>,
    source_id: Option<String>,
}

fn validated_url(url: String) -> Result<String> {
    if url.is_empty() {
        return Ok(url);
    }
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| bad("url must use HTTP or HTTPS"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || authority.is_empty()
        || authority.contains('@')
        || url
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '\\' | '<' | '>' | '"' | '\''))
    {
        return Err(bad(
            "url must be an HTTP or HTTPS address without credentials",
        ));
    }
    Ok(url)
}

fn validated_doi(doi: String) -> Result<String> {
    if doi.is_empty() {
        return Ok(doi);
    }
    normalize_doi(&doi)
        .filter(|id| !id.chars().any(|ch| ch.is_whitespace() || ch.is_control()))
        .ok_or_else(|| bad("invalid DOI"))
}

fn source_doi(doi: Option<&str>, url: Option<&str>, bibtex: Option<&str>) -> Option<String> {
    doi.filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| url.and_then(normalize_doi))
        .or_else(|| {
            let entries = parse_bib_entries(bibtex?);
            if entries.len() == 1 {
                return entries
                    .into_values()
                    .next()
                    .and_then(|entry| entry.doi_lower);
            }
            None
        })
}

fn set_optional(target: &mut Option<String>, value: Option<String>) {
    if let Some(value) = value {
        *target = clear_empty(value);
    }
}

impl SourceInput {
    fn checked(mut self) -> Result<Self> {
        if self.revision.is_some() && self.id.is_none() {
            return Err(bad("a paper revision requires its project source ID"));
        }
        if self.create_only && self.revision.is_some() {
            return Err(bad("a new paper cannot specify an existing revision"));
        }
        check_reference(self.id.as_deref())?;
        check_reference(self.session_id.as_deref().filter(|id| !id.is_empty()))?;
        check_reference(self.source_id.as_deref().filter(|id| !id.is_empty()))?;
        self.title = check_optional(self.title, "title", 256)?;
        self.url = check_optional(self.url, "url", 2048)?
            .map(validated_url)
            .transpose()?;
        self.doi = check_optional(self.doi, "doi", 512)?
            .map(validated_doi)
            .transpose()?;
        self.bibtex = check_optional(self.bibtex, "bibtex", 8192)?;
        self.notes = check_optional(self.notes, "notes", 4000)?;
        self.location = check_optional(self.location, "location", 256)?;
        self.quote = check_optional(self.quote, "quote", 3000)?;
        Ok(self)
    }

    fn apply(self, source: &mut ProjectSource) -> Result<()> {
        if let Some(title) = self.title {
            source.title = required(title, "title")?;
        }
        if source.title.is_empty() {
            return Err(bad("title is required"));
        }
        set_optional(&mut source.url, self.url);
        set_optional(&mut source.doi, self.doi);
        set_optional(&mut source.bibtex, self.bibtex);
        set_optional(&mut source.notes, self.notes);
        set_optional(&mut source.location, self.location);
        set_optional(&mut source.quote, self.quote);
        set_optional(&mut source.session_id, self.session_id);
        set_optional(&mut source.source_id, self.source_id);
        Ok(())
    }
}

fn source_matches(source: &ProjectSource, input: &SourceInput, doi: Option<&str>) -> bool {
    input.id.as_deref() == Some(source.id.as_str())
        || doi.is_some_and(|id| {
            source_doi(
                source.doi.as_deref(),
                source.url.as_deref(),
                source.bibtex.as_deref(),
            )
            .as_deref()
                == Some(id)
        })
        || input
            .url
            .as_deref()
            .is_some_and(|url| source.url.as_deref() == Some(url))
        || (input.source_id.is_some()
            && input.source_id == source.source_id
            && input.session_id == source.session_id)
        || (input.title.as_deref() == Some(source.title.as_str())
            && input.url.is_none()
            && source.url.is_none()
            && input.source_id.is_none()
            && source.source_id.is_none()
            && doi.is_none()
            && source.doi.is_none()
            && input.bibtex == source.bibtex)
}

pub(crate) fn handle_add_source(state: &DesktopState, id: &str, body: &[u8]) -> ApiResult {
    let input = read_body::<SourceInput>(body)?.checked()?;
    let doi = source_doi(
        input.doi.as_deref(),
        input.url.as_deref(),
        input.bibtex.as_deref(),
    );
    let project = edit_project(state, id, |project| {
        let mut matches = project
            .sources
            .iter()
            .enumerate()
            .filter_map(|(index, source)| {
                source_matches(source, &input, doi.as_deref()).then_some(index)
            });
        let matched = matches.next();
        if input.create_only && matched.is_some() {
            return Err(conflict(
                "This paper is already saved. Open it in the project library to edit it.",
            ));
        }
        if matches.next().is_some() {
            return Err(conflict(
                "the supplied identifiers refer to different project sources",
            ));
        }
        if matched.is_some_and(|index| {
            input
                .id
                .as_ref()
                .is_some_and(|id| id != &project.sources[index].id)
        }) {
            return Err(conflict("this source already has a different project ID"));
        }
        let original = matched.map(|index| &project.sources[index]);
        if let Some(revision) = input.revision {
            let saved =
                original.ok_or_else(|| not_found("this paper was removed from the project"))?;
            if saved.revision != revision {
                return Err(conflict("this paper changed since it was opened"));
            }
        }
        let mut source = match original {
            Some(source) => source.clone(),
            None if project.sources.len() < MAX_SOURCES => ProjectSource {
                id: input
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("src-{}", random_hex(12))),
                ..ProjectSource::default()
            },
            None => return Err(conflict("the project has reached its source limit")),
        };
        input.apply(&mut source)?;
        // Editing notes on a saved source does not recreate its historical backlink.
        if original.is_none_or(|old| {
            old.session_id != source.session_id || old.source_id != source.source_id
        }) {
            check_backlinks(
                state,
                project,
                source.session_id.as_deref(),
                None,
                source.source_id.as_deref(),
            )?;
        }
        if original != Some(&source) {
            source.revision = source
                .revision
                .checked_add(1)
                .ok_or_else(|| conflict("the paper revision counter is exhausted"))?;
        }
        match matched {
            Some(index) => project.sources[index] = source,
            None => project.sources.push(source),
        }
        Ok(())
    })?;
    project_res(200, project)
}

pub(crate) fn handle_delete_source(
    state: &DesktopState,
    id: &str,
    source_id: &str,
    revision: Option<&str>,
) -> ApiResult {
    validate_id(source_id)?;
    let expected = revision
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| bad("the paper revision must be an unsigned integer"))?;
    let project = edit_project(state, id, |project| {
        let index = project
            .sources
            .iter()
            .position(|source| source.id == source_id)
            .ok_or_else(|| not_found(format!("project source `{source_id}` not found")))?;
        if expected.is_some_and(|revision| revision != project.sources[index].revision) {
            return Err(conflict("this paper changed since it was opened"));
        }
        project.sources.remove(index);
        for item in &mut project.items {
            if item.source_id.as_deref() == Some(source_id) {
                item.source_id = None;
            }
        }
        Ok(())
    })?;
    project_res(200, project)
}

pub(crate) fn export_bibtex(
    state: &DesktopState,
    id: &str,
    format: Option<&str>,
) -> Result<Vec<u8>> {
    if format.is_some_and(|format| format != "bib") {
        return Err(bad("project export supports only format=bib"));
    }
    let project = get_project(state, id)?;
    let mut seen = HashSet::new();
    let entries: Vec<_> = project
        .sources
        .iter()
        .filter_map(|source| source.bibtex.as_deref())
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && seen.insert(*entry))
        .collect();
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    Ok(format!("{}\n", entries.join("\n\n")).into_bytes())
}
