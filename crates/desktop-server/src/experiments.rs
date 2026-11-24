//! Project experiments compare researcher-recorded observations from one frozen fork point.

use crate::projects::{load_project, ApiError, Project};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::{now_ms, DesktopState};
use ledger::{Event, Ledger};
use research_tools::{is_symlink_or_junction, validate_session_id, WorkspaceSandbox};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::PathBuf;
use tools::canon::random_hex;

const STORE_FILE: &str = "experiments.json";
const MAX_STORE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXPERIMENTS: usize = 64;
const MAX_BRANCHES: usize = 32;
const MAX_OBSERVATIONS: usize = 32;
const MAX_TOOL_RECEIPTS: usize = 32;
type Result<T> = std::result::Result<T, ApiError>;

#[derive(Serialize, Deserialize)]
struct Experiment {
    id: String,
    project_id: String,
    question: String,
    metric: Metric,
    budget: Budget,
    baseline: Baseline,
    created_ms: i64,
    revision: u64,
    branches: Vec<Branch>,
}

#[derive(Serialize, Deserialize)]
struct Metric {
    name: String,
    unit: String,
    direction: Direction,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Minimize,
    Maximize,
}

#[derive(Serialize, Deserialize)]
struct Budget {
    limit: f64,
    unit: String,
}

#[derive(Serialize, Deserialize)]
struct Baseline {
    session_id: String,
    at_seq: u64,
    event_hash: String,
    entry_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Branch {
    session_id: String,
    label: String,
    fork_seq: u64,
    fork_event_hash: String,
    registered_ms: i64,
    observations: Vec<Observation>,
}

#[derive(Serialize, Deserialize)]
struct Observation {
    status: Outcome,
    actual_budget: f64,
    score: Option<f64>,
    note: String,
    within_budget: bool,
    observed_ms: i64,
    evidence: Evidence,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Completed,
    Failed,
}

#[derive(Serialize, Deserialize)]
struct Evidence {
    seq: u64,
    event_hash: String,
    entry_id: Option<String>,
    tools: Vec<ToolReceipt>,
    tools_omitted: usize,
}

#[derive(Serialize, Deserialize)]
struct ToolReceipt {
    tool: String,
    call_id: String,
    request_seq: u64,
    result_seq: Option<u64>,
    result: ToolResult,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolResult {
    Pending,
    RecordedOk,
    Denied,
    Error,
}

#[derive(Deserialize)]
struct NewExperiment {
    question: String,
    metric: Metric,
    budget: Budget,
    baseline: Cutoff,
}

#[derive(Deserialize)]
struct Cutoff {
    session_id: String,
    at_seq: u64,
}

#[derive(Deserialize)]
struct NewBranch {
    session_id: String,
    label: String,
}

#[derive(Deserialize)]
struct NewObservation {
    status: Outcome,
    actual_budget: f64,
    score: Option<f64>,
    #[serde(default)]
    note: String,
    at_seq: Option<u64>,
}

struct Store {
    _lock: File,
    root: PathBuf,
    experiments: Vec<Experiment>,
}

impl Store {
    fn open(state: &DesktopState) -> Result<Self> {
        let root = state.cfg.data_dir.canonicalize().map_err(storage_error)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("experiments.lock"))
            .map_err(storage_error)?;
        lock.lock().map_err(storage_error)?;
        let target = root.join(STORE_FILE);
        let experiments = match fs::symlink_metadata(&target) {
            Ok(meta) => {
                if !meta.is_file() || is_symlink_or_junction(&target, &meta) {
                    return Err(internal("invalid experiment storage file"));
                }
                let mut bytes = Vec::new();
                File::open(target)
                    .map_err(storage_error)?
                    .take((MAX_STORE_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(storage_error)?;
                if bytes.len() > MAX_STORE_BYTES {
                    return Err(internal("the experiment store exceeds its size limit"));
                }
                serde_json::from_slice(&bytes)
                    .map_err(|err| internal(format!("could not load experiments: {err}")))?
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(storage_error(err)),
        };
        Ok(Self {
            _lock: lock,
            root,
            experiments,
        })
    }

    fn save(&self) -> Result<()> {
        let bytes =
            serde_json::to_vec(&self.experiments).map_err(|err| internal(err.to_string()))?;
        if bytes.len() > MAX_STORE_BYTES {
            return Err(conflict("the experiment store has reached its size limit"));
        }
        WorkspaceSandbox::atomic_write_in_root(&self.root, &[STORE_FILE.into()], &bytes)
            .map_err(|err| internal(format!("could not save experiments: {err}")))?;
        #[cfg(unix)]
        let _ = File::open(&self.root).and_then(|dir| dir.sync_all());
        Ok(())
    }

    fn get_mut(&mut self, project_id: &str, experiment_id: &str) -> Result<&mut Experiment> {
        self.experiments
            .iter_mut()
            .find(|experiment| {
                experiment.project_id == project_id && experiment.id == experiment_id
            })
            .ok_or_else(|| not_found("experiment not found in this project"))
    }
}

fn storage_error(err: io::Error) -> ApiError {
    internal(format!("experiment storage: {err}"))
}

fn read_body<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    serde_json::from_value(parse_json_body(body)?)
        .map_err(|err| bad(format!("invalid experiment request: {err}")))
}

fn get_project(state: &DesktopState, id: &str) -> Result<Project> {
    load_project(state, id)?.ok_or_else(|| not_found(format!("project `{id}` not found")))
}

fn linked_session(project: &Project, session_id: &str) -> Result<()> {
    validate_session_id(session_id).map_err(|err| bad(err.to_string()))?;
    if !project.session_ids.iter().any(|id| id == session_id) {
        return Err(not_found("session not linked to this project"));
    }
    Ok(())
}

fn required_text(text: &str, field: &str, limit: usize) -> Result<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > limit {
        return Err(bad(format!("{field} must contain 1 to {limit} characters")));
    }
    Ok(text.to_string())
}

fn verified_events(state: &DesktopState, session_id: &str) -> Result<Vec<Event>> {
    Ledger::open_existing(&state.cfg.data_dir)
        .and_then(|ledger| ledger.read_verified_session(session_id))
        .map(|(_, events)| events)
        .map_err(|err| bad(format!("could not verify session `{session_id}`: {err}")))
}

fn event_at(events: &[Event], seq: u64) -> Result<&Event> {
    let index = seq.checked_sub(1).and_then(|n| usize::try_from(n).ok());
    index
        .and_then(|i| events.get(i))
        .ok_or_else(|| bad("the ledger sequence does not exist"))
}

fn entry_id(event: &Event) -> Option<String> {
    if event.kind == "proof.entry" {
        event.payload["entry_id"].as_str().map(str::to_string)
    } else {
        None
    }
}

fn experiment_json(experiment: &Experiment) -> Value {
    let mut pending = 0;
    let mut failed = 0;
    let mut completed_over_budget = 0;
    let mut eligible = 0;
    let mut best_score = None;
    let mut best_session_ids = Vec::new();
    for branch in &experiment.branches {
        match branch.observations.last() {
            None => pending += 1,
            Some(observation) => match observation.status {
                Outcome::Failed => failed += 1,
                Outcome::Completed if !observation.within_budget => completed_over_budget += 1,
                Outcome::Completed => {
                    if let Some(score) = observation.score {
                        eligible += 1;
                        let better =
                            best_score.is_none_or(|best| match experiment.metric.direction {
                                Direction::Minimize => score < best,
                                Direction::Maximize => score > best,
                            });
                        if better {
                            best_score = Some(score);
                            best_session_ids.clear();
                        }
                        if best_score == Some(score) {
                            best_session_ids.push(&branch.session_id);
                        }
                    }
                }
            },
        }
    }
    let mut value = json!(experiment);
    value["comparison"] = json!({
        "completed_within_budget": eligible,
        "completed_over_budget": completed_over_budget,
        "failed": failed,
        "pending": pending,
        "best_score": best_score,
        "best_session_ids": best_session_ids,
    });
    value
}

pub(crate) fn handle_list(state: &DesktopState, project_id: &str) -> ApiResult {
    get_project(state, project_id)?;
    let store = Store::open(state)?;
    let experiments: Vec<_> = store
        .experiments
        .iter()
        .filter(|experiment| experiment.project_id == project_id)
        .map(experiment_json)
        .collect();
    Ok((200, json!({"experiments": experiments})))
}

pub(crate) fn experiment_context(
    state: &DesktopState,
    project_id: &str,
    session_id: &str,
) -> Result<Vec<Value>> {
    let store = Store::open(state)?;
    Ok(store
        .experiments
        .iter()
        .rev()
        .filter_map(|experiment| {
            if experiment.project_id != project_id {
                return None;
            }
            let branch = experiment
                .branches
                .iter()
                .find(|branch| branch.session_id == session_id)?;
            Some(json!({
                "id": experiment.id,
                "question": experiment.question.chars().take(600).collect::<String>(),
                "metric": experiment.metric,
                "budget": experiment.budget,
                "branch_label": branch.label,
            }))
        })
        .take(3)
        .collect())
}

pub(crate) fn handle_create(state: &DesktopState, project_id: &str, body: &[u8]) -> ApiResult {
    let project = get_project(state, project_id)?;
    let input: NewExperiment = read_body(body)?;
    linked_session(&project, &input.baseline.session_id)?;
    let question = required_text(&input.question, "experiment question", 4_000)?;
    let metric = Metric {
        name: required_text(&input.metric.name, "metric name", 128)?,
        unit: required_text(&input.metric.unit, "metric unit", 64)?,
        direction: input.metric.direction,
    };
    if !input.budget.limit.is_finite() || input.budget.limit <= 0.0 {
        return Err(bad("the budget limit must be finite and positive"));
    }
    let budget = Budget {
        limit: input.budget.limit,
        unit: required_text(&input.budget.unit, "budget unit", 64)?,
    };
    let events = verified_events(state, &input.baseline.session_id)?;
    let anchor = event_at(&events, input.baseline.at_seq)?;
    let baseline = Baseline {
        session_id: input.baseline.session_id,
        at_seq: anchor.seq,
        event_hash: anchor.event_hash.clone(),
        entry_id: entry_id(anchor),
    };
    let mut store = Store::open(state)?;
    if store
        .experiments
        .iter()
        .filter(|experiment| experiment.project_id == project_id)
        .count()
        >= MAX_EXPERIMENTS
    {
        return Err(conflict("the project has reached its experiment limit"));
    }
    let id = loop {
        let id = format!("exp-{}", random_hex(8));
        if !store
            .experiments
            .iter()
            .any(|experiment| experiment.id == id)
        {
            break id;
        }
    };
    let experiment = Experiment {
        id,
        project_id: project_id.to_string(),
        question,
        metric,
        budget,
        baseline,
        created_ms: now_ms(),
        revision: 1,
        branches: Vec::new(),
    };
    let value = experiment_json(&experiment);
    store.experiments.push(experiment);
    store.save()?;
    Ok((201, value))
}

pub(crate) fn handle_add_branch(
    state: &DesktopState,
    project_id: &str,
    experiment_id: &str,
    body: &[u8],
) -> ApiResult {
    let project = get_project(state, project_id)?;
    let input: NewBranch = read_body(body)?;
    linked_session(&project, &input.session_id)?;
    let label = required_text(&input.label, "branch label", 128)?;
    let events = verified_events(state, &input.session_id)?;
    let fork = events
        .iter()
        .rev()
        .find(|event| event.kind == "session.fork")
        .ok_or_else(|| bad("the session has no ledger-backed fork point"))?;
    let mut store = Store::open(state)?;
    let experiment = store.get_mut(project_id, experiment_id)?;
    let baseline = &experiment.baseline;
    if fork.payload["parent_session"].as_str() != Some(baseline.session_id.as_str())
        || fork.payload["parent_seq"].as_u64() != Some(baseline.at_seq)
        || fork.payload["parent_event_hash"].as_str() != Some(baseline.event_hash.as_str())
        || fork.seq != baseline.at_seq.saturating_add(1)
    {
        return Err(conflict(
            "the fork does not use the experiment's baseline cutoff",
        ));
    }
    if let Some(existing) = experiment
        .branches
        .iter()
        .find(|branch| branch.session_id == input.session_id)
    {
        if existing.label != label {
            return Err(conflict(
                "the fork already belongs to this experiment with a different label",
            ));
        }
        return Ok((200, experiment_json(experiment)));
    }
    if experiment.branches.len() >= MAX_BRANCHES {
        return Err(conflict("the experiment has reached its branch limit"));
    }
    experiment.branches.push(Branch {
        session_id: input.session_id,
        label,
        fork_seq: fork.seq,
        fork_event_hash: fork.event_hash.clone(),
        registered_ms: now_ms(),
        observations: Vec::new(),
    });
    experiment.revision += 1;
    let value = experiment_json(experiment);
    store.save()?;
    Ok((201, value))
}

fn collect_tool_receipts(events: &[Event]) -> (Vec<ToolReceipt>, usize) {
    let mut indices = HashMap::new();
    let mut receipts: Vec<ToolReceipt> = Vec::new();
    for event in events {
        let Some(call_id) = event.payload["call_id"].as_str() else {
            continue;
        };
        match event.kind.as_str() {
            "tool.request" => {
                if let Some(tool) = event.payload["tool"].as_str() {
                    indices.insert(call_id, receipts.len());
                    receipts.push(ToolReceipt {
                        tool: tool.chars().take(128).collect(),
                        call_id: call_id.chars().take(128).collect(),
                        request_seq: event.seq,
                        result_seq: None,
                        result: ToolResult::Pending,
                    });
                }
            }
            "tool.result" | "tool.denied" => {
                if let Some(&index) = indices.get(call_id) {
                    let receipt = &mut receipts[index];
                    receipt.result_seq = Some(event.seq);
                    receipt.result = if event.kind == "tool.denied" {
                        ToolResult::Denied
                    } else if event.payload["ok"].as_bool() == Some(true) {
                        ToolResult::RecordedOk
                    } else {
                        ToolResult::Error
                    };
                }
            }
            _ => {}
        }
    }
    let omitted = receipts.len().saturating_sub(MAX_TOOL_RECEIPTS);
    (receipts.split_off(omitted), omitted)
}

pub(crate) fn handle_branch_diff(
    state: &DesktopState,
    project_id: &str,
    experiment_id: &str,
    session_id: &str,
) -> ApiResult {
    let project = get_project(state, project_id)?;
    linked_session(&project, session_id)?;
    let (fork_seq, fork_hash) = {
        let mut store = Store::open(state)?;
        let branch = store
            .get_mut(project_id, experiment_id)?
            .branches
            .iter()
            .find(|branch| branch.session_id == session_id)
            .ok_or_else(|| not_found("fork not registered in this experiment"))?;
        (branch.fork_seq, branch.fork_event_hash.clone())
    };
    let ledger =
        Ledger::open_existing(&state.cfg.data_dir).map_err(|err| internal(err.to_string()))?;
    let report =
        export::diff::compute_fork_diff(&ledger, session_id, fork_seq, &fork_hash).map_err(bad)?;
    Ok((200, json!(report)))
}

pub(crate) fn handle_record_observation(
    state: &DesktopState,
    project_id: &str,
    experiment_id: &str,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let project = get_project(state, project_id)?;
    linked_session(&project, session_id)?;
    let input: NewObservation = read_body(body)?;
    if !input.actual_budget.is_finite() || input.actual_budget < 0.0 {
        return Err(bad("actual budget must be finite and nonnegative"));
    }
    if input.score.is_some_and(|score| !score.is_finite()) {
        return Err(bad("score must be finite"));
    }
    if matches!(input.status, Outcome::Completed) && input.score.is_none() {
        return Err(bad("a completed observation requires a score"));
    }
    let note = input.note.trim();
    if note.chars().count() > 4_000 {
        return Err(bad("observation notes may contain at most 4000 characters"));
    }

    let events = verified_events(state, session_id)?;
    let seq = input
        .at_seq
        .unwrap_or_else(|| events.last().map_or(0, |event| event.seq));
    let anchor = event_at(&events, seq)?;
    let mut store = Store::open(state)?;
    let experiment = store.get_mut(project_id, experiment_id)?;
    let branch = experiment
        .branches
        .iter_mut()
        .find(|branch| branch.session_id == session_id)
        .ok_or_else(|| not_found("fork not registered in this experiment"))?;
    let fork = event_at(&events, branch.fork_seq)?;
    if fork.event_hash != branch.fork_event_hash || seq < branch.fork_seq {
        return Err(conflict(
            "observation must be anchored at or after its registered fork event",
        ));
    }
    if branch.observations.len() >= MAX_OBSERVATIONS {
        return Err(conflict("this branch has reached its observation limit"));
    }
    let (tools, tools_omitted) =
        collect_tool_receipts(&events[branch.fork_seq as usize..seq as usize]);
    branch.observations.push(Observation {
        status: input.status,
        actual_budget: input.actual_budget,
        score: input.score,
        note: note.to_string(),
        within_budget: input.actual_budget <= experiment.budget.limit,
        observed_ms: now_ms(),
        evidence: Evidence {
            seq,
            event_hash: anchor.event_hash.clone(),
            entry_id: entry_id(anchor),
            tools,
            tools_omitted,
        },
    });
    experiment.revision += 1;
    let value = experiment_json(experiment);
    store.save()?;
    Ok((200, value))
}
