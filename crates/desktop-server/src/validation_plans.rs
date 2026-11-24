//! Frozen scalar validation protocols with local, withheld reference values.

use crate::projects::{project_for_session, with_project, ApiError, Project};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use ledger::{canonical_json, sha256_hex, Event, Ledger};
use research_tools::proof_dag::tool_reference_matches;
use research_tools::{validate_session_id, WorkspaceSandbox};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use tools::canon::random_hex;

const PLAN_STORE: &str = "validation_plans.json";
const MAX_STORE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROJECT_PLANS: usize = 64;
const PLAN_KIND: &str = "validation.plan.committed";
const EVALUATION_KIND: &str = "validation.plan.evaluated";
const SCHEMA_VERSION: u32 = 1;
const MAX_NUMBER_LENGTH: usize = 384;
const MAX_DECIMAL_EXPONENT: i32 = 512;

type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Axis {
    Resolution,
    Basis,
    Precision,
}

impl Axis {
    fn as_str(self) -> &'static str {
        match self {
            Self::Resolution => "resolution",
            Self::Basis => "basis",
            Self::Precision => "precision",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Protocol {
    session_id: String,
    claim: String,
    candidate_method: String,
    reference_method: String,
    axis: Axis,
    atol: String,
    rtol: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PrivateValues {
    salt: String,
    reference: String,
    known_pass: String,
    planted_fail: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StoredPlan {
    id: String,
    project_id: String,
    protocol: Protocol,
    values: PrivateValues,
}

impl StoredPlan {
    fn commitment_payload(&self) -> Value {
        // The salt stays in the local store along with the values. A commitment
        // by itself should not allow guesses against a low-entropy reference.
        let commitment = sha256_hex(canonical_json(&json!(self.values)).as_bytes());
        json!({
            "project_id": self.project_id,
            "plan_id": self.id,
            "protocol": self.protocol,
            "comparison": "abs(candidate - reference) <= atol + rtol * abs(reference)",
            "control_criteria": "known_pass must pass and planted_fail must fail the same comparison",
            "private_commitment": commitment,
        })
    }

    fn public_fields(&self, commit_seq: u64, commit_hash: &str) -> Value {
        json!({
            "id": self.id,
            "session_id": self.protocol.session_id,
            "claim": self.protocol.claim,
            "candidate_method": self.protocol.candidate_method,
            "reference_method": self.protocol.reference_method,
            "axis": self.protocol.axis,
            "atol": self.protocol.atol,
            "rtol": self.protocol.rtol,
            "status": "open",
            "commit_seq": commit_seq,
            "commit_hash": commit_hash,
            "private_commitment": self.commitment_payload()["private_commitment"],
            "evaluation": null,
        })
    }

    fn commit<'a>(&self, events: &'a [Event]) -> Result<Option<&'a Event>> {
        let mut commits = events.iter().filter(|event| {
            event.kind == PLAN_KIND && event.payload["plan_id"].as_str() == Some(&self.id)
        });
        let first = commits.next();
        if commits.next().is_some()
            || first.is_some_and(|event| event.payload != self.commitment_payload())
        {
            return Err(conflict(
                "validation plan does not match its recorded commitment",
            ));
        }
        Ok(first)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanInput {
    session_id: String,
    claim: String,
    candidate_method: String,
    reference_method: String,
    axis: Axis,
    atol: String,
    rtol: String,
    reference: String,
    known_pass: String,
    planted_fail: String,
}

fn checked_text(text: &str, name: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty()
        || text.chars().count() > 2000
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(bad(format!(
            "{name} must contain 1 to 2000 characters without control codes"
        )));
    }
    Ok(text.to_string())
}

impl PlanInput {
    fn validate(self, project: &Project) -> Result<StoredPlan> {
        validate_session_id(&self.session_id).map_err(|e| bad(e.to_string()))?;
        if !project.session_ids.contains(&self.session_id) {
            return Err(bad("validation requires an attached project session"));
        }
        let candidate_method = checked_text(&self.candidate_method, "candidate_method")?;
        let reference_method = checked_text(&self.reference_method, "reference_method")?;
        if candidate_method.eq_ignore_ascii_case(&reference_method) {
            return Err(bad(
                "describe a reference method independent of the candidate method",
            ));
        }
        let protocol = Protocol {
            session_id: self.session_id,
            claim: checked_text(&self.claim, "claim")?,
            candidate_method,
            reference_method,
            axis: self.axis,
            atol: self.atol.trim().to_string(),
            rtol: self.rtol.trim().to_string(),
        };
        let values = PrivateValues {
            salt: random_hex(32),
            reference: self.reference.trim().to_string(),
            known_pass: self.known_pass.trim().to_string(),
            planted_fail: self.planted_fail.trim().to_string(),
        };
        let plan = StoredPlan {
            id: format!("plan-{}", random_hex(12)),
            project_id: project.id.clone(),
            protocol,
            values,
        };
        let comparator = Comparator::new(&plan)?;
        if !comparator.known_pass_passed || !comparator.planted_fail_rejected {
            return Err(bad("controls must straddle the committed tolerance"));
        }
        Ok(plan)
    }
}

fn body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_value(parse_json_body(bytes)?)
        .map_err(|_| bad("validation request does not match its field schema"))
}

fn read_store(root: &Path) -> Result<Vec<StoredPlan>> {
    let file = match File::open(root.join(PLAN_STORE)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(internal(format!(
                "could not open validation plans: {error}"
            )))
        }
    };
    let mut bytes = Vec::new();
    file.take((MAX_STORE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| internal(format!("could not read validation plans: {error}")))?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(internal("validation plan storage exceeds its size limit"));
    }
    serde_json::from_slice(&bytes).map_err(|_| internal("validation plan storage is invalid"))
}

fn write_store(root: &Path, plans: &[StoredPlan]) -> Result<()> {
    let bytes =
        serde_json::to_vec(plans).map_err(|_| internal("could not encode validation plans"))?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(conflict(
            "validation plan storage has reached its size limit",
        ));
    }
    WorkspaceSandbox::atomic_write_in_root(root, &[PLAN_STORE.into()], &bytes)
        .map_err(|error| internal(format!("could not save validation plans: {error}")))?;
    #[cfg(unix)]
    let _ = File::open(root).and_then(|directory| directory.sync_all());
    Ok(())
}

fn verified_events(ledger: &Ledger, session: &str) -> Result<Vec<Event>> {
    match ledger.read_verified_session(session) {
        Ok((_, events)) => Ok(events),
        Err(ledger::Error::UnknownSession(_)) => Ok(Vec::new()),
        Err(error) => Err(conflict(format!(
            "could not verify validation session `{session}`: {error}"
        ))),
    }
}

pub(crate) fn backup_plans(root: &Path, id: &str) -> Result<Vec<StoredPlan>> {
    Ok(read_store(root)?.into_iter().filter(|plan| plan.project_id == id).collect())
}

pub(crate) fn restore_plans(root: &Path, project: &Project, rows: &[StoredPlan], ledger: &Ledger, apply: bool) -> Result<()> {
    let mut plans = read_store(root)?;
    let mut ids = HashSet::new();
    for row in rows {
        if row.project_id != project.id || !ids.insert(&row.id) || rows.len() > MAX_PROJECT_PLANS
            || !project.session_ids.contains(&row.protocol.session_id) {
            return Err(bad("invalid validation plan in backup"));
        }
        validate_session_id(&row.id).map_err(|e| bad(e.to_string()))?;
        let comparator = Comparator::new(row)?;
        if !comparator.known_pass_passed || !comparator.planted_fail_rejected {
            return Err(bad("archived validation controls do not straddle the committed tolerance"));
        }
        if snapshot_plan(row, &verified_events(ledger, &row.protocol.session_id)?)?.is_none() {
            return Err(bad("backup validation plan is missing its ledger commitment"));
        }
    }
    let matches: Vec<_> = plans.iter().filter(|row| row.project_id == project.id).collect();
    if !matches.is_empty() {
        if json!(matches) == json!(rows) { return Ok(()); }
        return Err(conflict("validation records already exist for this project"));
    }
    if rows.is_empty() { return Ok(()); }
    let imported = serde_json::from_value::<Vec<StoredPlan>>(json!(rows)).map_err(|e| bad(e.to_string()))?;
    plans.extend(imported);
    if serde_json::to_vec(&plans).map_err(|e| bad(e.to_string()))?.len() > MAX_STORE_BYTES {
        return Err(conflict("validation plan storage size limit exceeded"));
    }
    if apply { write_store(root, &plans)?; }
    Ok(())
}

fn snapshot_plan(plan: &StoredPlan, events: &[Event]) -> Result<Option<Value>> {
    let Some(commit) = plan.commit(events)? else {
        // A process may stop after writing the private record and before committing
        // its public ledger marker. Such a pending record never opens a plan.
        return Ok(None);
    };
    let mut public = plan.public_fields(commit.seq, &commit.event_hash);
    if let Some(evaluation) = evaluation_event(plan, commit, events)? {
        let mut result = evaluation.payload["evaluation"].clone();
        result["evaluation_seq"] = json!(evaluation.seq);
        result["evaluation_hash"] = json!(evaluation.event_hash);
        public["status"] = evaluation.payload["status"].clone();
        public["evaluation"] = result;
    }
    Ok(Some(public))
}

fn evaluation_event<'a>(
    plan: &StoredPlan,
    commit: &Event,
    events: &'a [Event],
) -> Result<Option<&'a Event>> {
    let mut results = events.iter().filter(|event| {
        event.kind == EVALUATION_KIND && event.payload["plan_id"].as_str() == Some(&plan.id)
    });
    let result = results.next();
    if results.next().is_some()
        || result.is_some_and(|event| {
            event.seq <= commit.seq
                || event.payload["commit_hash"].as_str() != Some(&commit.event_hash)
                || !matches!(
                    event.payload["status"].as_str(),
                    Some("accepted" | "rejected")
                )
                || !event.payload["evaluation"].is_object()
        })
    {
        return Err(conflict(
            "validation session contains an inconsistent evaluation",
        ));
    }
    Ok(result)
}

pub(crate) fn handle_create(state: &DesktopState, project_id: &str, bytes: &[u8]) -> ApiResult {
    let input: PlanInput = body(bytes)?;
    let plan = with_project(state, project_id, |root, project| {
        let mut plans = read_store(root)?;
        if plans
            .iter()
            .filter(|plan| plan.project_id == project_id)
            .count()
            >= MAX_PROJECT_PLANS
        {
            return Err(conflict(
                "the project has reached its validation plan limit",
            ));
        }
        let plan = input.validate(project)?;
        if !state
            .session_exists(&plan.protocol.session_id)
            .map_err(internal)?
        {
            return Err(not_found("validation session is unavailable"));
        }
        // Keep a new desktop turn from starting between this check and the
        // commit, so the next turn receives the frozen public protocol.
        let active_turns = state
            .active_turns
            .lock()
            .map_err(|_| internal("active turn lock poisoned"))?;
        if active_turns.contains(&plan.protocol.session_id) {
            return Err(conflict(
                "finish the current session turn before committing a validation plan",
            ));
        }
        let ledger = state.open_ledger().map_err(internal)?;
        verified_events(&ledger, &plan.protocol.session_id)?;
        plans.push(plan);
        write_store(root, &plans)?;
        let plan = plans
            .last()
            .ok_or_else(|| internal("validation plan was not stored"))?;
        let event = ledger.append_with_hash(
            &plan.protocol.session_id,
            PLAN_KIND,
            SCHEMA_VERSION,
            &plan.commitment_payload(),
        );
        match event {
            Ok(event) => Ok(plan.public_fields(event.seq, &event.event_hash)),
            Err(error) => {
                plans.pop();
                let _ = write_store(root, &plans);
                Err(internal(format!(
                    "could not commit validation plan: {error}"
                )))
            }
        }
    })?;
    Ok((201, plan))
}

fn public_plans(
    state: &DesktopState,
    root: &Path,
    project: &Project,
    session: Option<&str>,
) -> Result<Vec<Value>> {
    let ledger = state.open_ledger().map_err(internal)?;
    let mut sessions = HashMap::new();
    let mut public = Vec::new();
    for plan in read_store(root)? {
        if plan.project_id != project.id || session.is_some_and(|id| id != plan.protocol.session_id)
        {
            continue;
        }
        let events = match sessions.entry(plan.protocol.session_id.clone()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(verified_events(&ledger, &plan.protocol.session_id)?)
            }
        };
        if let Some(snapshot) = snapshot_plan(&plan, events)? {
            public.push(snapshot);
        }
    }
    Ok(public)
}

pub(crate) fn handle_list(state: &DesktopState, project_id: &str) -> ApiResult {
    let plans = with_project(state, project_id, |root, project| {
        public_plans(state, root, project, None)
    })?;
    Ok((200, json!({ "plans": plans })))
}

pub(crate) fn public_context(state: &DesktopState, session_id: &str) -> Result<Vec<Value>> {
    let Some(project) = project_for_session(state, session_id)? else {
        return Ok(Vec::new());
    };
    let plans = with_project(state, &project.id, |root, project| {
        public_plans(state, root, project, Some(session_id))
    })?;
    Ok(plans
        .into_iter()
        .rev()
        .filter(|plan| plan["status"] == "open")
        .take(3)
        .map(|plan| {
            // Only send public instructions, never held-out data or evaluated outcomes.
            json!({
                "id": plan["id"],
                "claim": plan["claim"],
                "candidate_method": plan["candidate_method"],
                "reference_method": plan["reference_method"],
                "axis": plan["axis"],
                "atol": plan["atol"],
                "rtol": plan["rtol"],
                "commit_seq": plan["commit_seq"],
            })
        })
        .collect())
}

fn with_plan<T>(
    state: &DesktopState,
    project_id: &str,
    plan_id: &str,
    action: impl FnOnce(&StoredPlan, &Ledger, &[Event], &Event) -> Result<T>,
) -> Result<T> {
    validate_session_id(plan_id).map_err(|error| bad(error.to_string()))?;
    with_project(state, project_id, |root, project| {
        let plans = read_store(root)?;
        let plan = plans
            .iter()
            .find(|plan| plan.id == plan_id && plan.project_id == project_id)
            .ok_or_else(|| not_found("validation plan was not found"))?;
        if !project.session_ids.contains(&plan.protocol.session_id) {
            return Err(conflict(
                "validation session is no longer attached to this project",
            ));
        }
        let ledger = state.open_ledger().map_err(internal)?;
        let events = verified_events(&ledger, &plan.protocol.session_id)?;
        let commit = plan
            .commit(&events)?
            .ok_or_else(|| not_found("validation plan was not committed"))?;
        action(plan, &ledger, &events, commit)
    })
}

#[derive(Serialize)]
struct Candidate<'a> {
    session_id: &'a str,
    entry_id: &'a str,
    entry_seq: u64,
    entry_hash: &'a str,
    entry_summary: &'a str,
    request_seq: u64,
    request_hash: &'a str,
    result_seq: u64,
    result_hash: &'a str,
    call_id: &'a str,
    axis: Axis,
    value: String,
}

fn result_value(slot: &Value, ledger: &Ledger) -> Option<Value> {
    if let Some(text) = slot["inline"].as_str() {
        serde_json::from_str(text).ok()
    } else if let Some(hash) = slot["blob"].as_str() {
        let bytes = ledger.get_blob(hash).ok()?;
        serde_json::from_slice(&bytes).ok()
    } else {
        slot.is_object().then(|| slot.clone())
    }
}

fn sample_level(sample: &Value, axis: Axis) -> Option<Magnitude> {
    let level = &sample["level"];
    if axis == Axis::Precision {
        let digits = level.as_u64()?;
        return (1..=256)
            .contains(&digits)
            .then(|| Decimal::parse(&digits.to_string()).unwrap().magnitude);
    }
    let level = Decimal::parse(level.as_str()?)?;
    let maximum = Decimal::parse("1e9").unwrap();
    (!level.negative
        && !level.magnitude.digits.is_empty()
        && level.magnitude.compare(&maximum.magnitude) != Ordering::Greater)
        .then_some(level.magnitude)
}

fn candidate_value(result: &Value, protocol: &Protocol) -> Option<String> {
    if result.get("value").is_none()
        || !result["error"].is_null()
        || result["validation_truncated"] == true
    {
        return None;
    }
    let report = &result["validation"];
    if report["schema_version"] != SCHEMA_VERSION
        || report["kind"] != "numerical_convergence"
        || report["axis"] != protocol.axis.as_str()
        || report["status"] != "within_tolerance"
        || report["stable"] != true
    {
        return None;
    }
    let reported_atol = Decimal::parse(report["atol"].as_str()?)?;
    let reported_rtol = Decimal::parse(report["rtol"].as_str()?)?;
    let atol = Decimal::parse(&protocol.atol)?;
    let rtol = Decimal::parse(&protocol.rtol)?;
    if reported_atol.negative
        || reported_rtol.negative
        || reported_atol.magnitude.compare(&atol.magnitude) == Ordering::Greater
        || reported_rtol.magnitude.compare(&rtol.magnitude) == Ordering::Greater
    {
        return None;
    }
    let samples = report["samples"].as_array()?;
    let tail = usize::try_from(report["tail"].as_u64()?).ok()?;
    if !(1..=31).contains(&tail)
        || !(tail + 1..=32).contains(&samples.len())
        || samples.iter().any(|sample| !sample["error"].is_null())
        || !samples
            .iter()
            .rev()
            .take(tail)
            .all(|sample| sample["passes"] == true)
    {
        return None;
    }
    let levels = samples
        .iter()
        .map(|sample| sample_level(sample, protocol.axis))
        .collect::<Option<Vec<_>>>()?;
    if !levels
        .windows(2)
        .all(|pair| pair[0].compare(&pair[1]) == Ordering::Less)
    {
        return None;
    }
    let values = samples
        .iter()
        .map(|sample| Decimal::parse(sample["value"].as_str()?))
        .collect::<Option<Vec<_>>>()?;
    let stable = values[values.len() - tail - 1..].windows(2).all(|pair| {
        let (previous, next) = (&pair[0], &pair[1]);
        let scale = if previous.magnitude.compare(&next.magnitude) == Ordering::Greater {
            &previous.magnitude
        } else {
            &next.magnitude
        };
        let limit = reported_atol
            .magnitude
            .add(&reported_rtol.magnitude.multiply(scale));
        previous.absolute_difference(next).compare(&limit) != Ordering::Greater
    });
    if !stable {
        return None;
    }
    samples.last()?["value"].as_str().map(str::to_string)
}

fn candidates<'a>(
    plan: &'a StoredPlan,
    ledger: &Ledger,
    events: &'a [Event],
    commit_seq: u64,
) -> Vec<Candidate<'a>> {
    let axis = plan.protocol.axis;
    let mut requests = HashMap::new();
    let mut results = Vec::new();
    let mut retired = HashSet::new();
    let mut rejected = HashSet::new();
    for event in events {
        let call = event.payload["call_id"].as_str();
        let entry = event.payload["entry_id"].as_str();
        match event.kind.as_str() {
            "tool.request" => {
                if let Some(call) = call {
                    requests.insert(call, event);
                }
            }
            "tool.result" | "tool.denied" => {
                let Some(call) = call else { continue };
                let Some(request) = requests.remove(call) else {
                    continue;
                };
                if event.kind == "tool.result" {
                    let value = if request.seq > commit_seq
                        && request.payload["tool"] == "sympy_eval"
                        && event.payload["ok"] == true
                    {
                        result_value(&event.payload["result"], ledger)
                            .and_then(|result| candidate_value(&result, &plan.protocol))
                    } else {
                        None
                    };
                    results.push((call, request, event, value));
                }
            }
            "proof.supersede" => {
                if let Some(entry) = entry {
                    retired.insert(entry);
                }
            }
            "proof.verification" => {
                if let Some(entry) = event.payload["target_entry_id"].as_str() {
                    match research_tools::effective_verification_verdict(&event.payload) {
                        Some("rejected") => {
                            rejected.insert(entry);
                        }
                        Some("accepted") => {
                            rejected.remove(entry);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    events
        .iter()
        .rev()
        .filter(|event| {
            event.seq > commit_seq
                && event.kind == "proof.entry"
                && matches!(
                    event.payload["entry_type"].as_str(),
                    Some("numerical_computation" | "result")
                )
        })
        .filter_map(|entry| {
            let entry_id = entry.payload["entry_id"].as_str()?;
            if retired.contains(entry_id) || rejected.contains(entry_id) {
                return None;
            }
            let reference = entry.payload["tool_call_ref"].as_str()?;
            let mut matches = results.iter().filter(|(call, _, result, _)| {
                result.seq < entry.seq && tool_reference_matches(reference, call, result.seq)
            });
            let (call, request, result, value) = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            Some(Candidate {
                session_id: &plan.protocol.session_id,
                entry_id,
                entry_seq: entry.seq,
                entry_hash: &entry.event_hash,
                entry_summary: entry.payload["summary"].as_str().unwrap_or_default(),
                request_seq: request.seq,
                request_hash: &request.event_hash,
                result_seq: result.seq,
                result_hash: &result.event_hash,
                call_id: call,
                axis,
                value: value.as_ref()?.clone(),
            })
        })
        .collect()
}

pub(crate) fn handle_candidates(
    state: &DesktopState,
    project_id: &str,
    plan_id: &str,
) -> ApiResult {
    with_plan(
        state,
        project_id,
        plan_id,
        |plan, ledger, events, commit| {
            let candidates = candidates(plan, ledger, events, commit.seq);
            Ok((
                200,
                json!({ "candidates": candidates.into_iter().take(100).collect::<Vec<_>>() }),
            ))
        },
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationInput {
    entry_id: String,
    result_seq: u64,
}

pub(crate) fn handle_evaluate(
    state: &DesktopState,
    project_id: &str,
    plan_id: &str,
    bytes: &[u8],
) -> ApiResult {
    let input: EvaluationInput = body(bytes)?;
    with_plan(
        state,
        project_id,
        plan_id,
        |plan, ledger, events, commit| {
            if let Some(evaluation) = evaluation_event(plan, commit, events)? {
                if evaluation.payload["evaluation"]["entry_id"] == input.entry_id
                    && evaluation.payload["evaluation"]["result_seq"] == input.result_seq
                {
                    return snapshot_plan(plan, events)
                        .map(|value| (200, value.unwrap_or(Value::Null)));
                }
                return Err(conflict(
                    "this validation plan already evaluated its frozen candidate",
                ));
            }
            let candidate = candidates(plan, ledger, events, commit.seq).into_iter()
            .find(|candidate| candidate.entry_id == input.entry_id && candidate.result_seq == input.result_seq)
            .ok_or_else(|| bad("select an eligible linked numerical result recorded after the plan was committed"))?;
            let comparator = Comparator::new(plan)?;
            let value = parse_decimal(&candidate.value, "candidate value")?;
            let accepted = comparator.known_pass_passed
                && comparator.planted_fail_rejected
                && comparator.matches(&value);
            let status = if accepted { "accepted" } else { "rejected" };
            let mut evaluation = json!(candidate);
            evaluation["known_pass_passed"] = json!(comparator.known_pass_passed);
            evaluation["planted_fail_rejected"] = json!(comparator.planted_fail_rejected);
            evaluation["snapshot_head_hash"] = json!(events.last().map(|event| &event.event_hash));
            let receipt = ledger
                .append_with_hash(
                    &plan.protocol.session_id,
                    EVALUATION_KIND,
                    SCHEMA_VERSION,
                    &json!({
                        "project_id": project_id,
                        "plan_id": plan_id,
                        "commit_hash": commit.event_hash,
                        "status": status,
                        "evaluation": evaluation,
                    }),
                )
                .map_err(|error| {
                    internal(format!("could not record validation evaluation: {error}"))
                })?;
            evaluation["evaluation_seq"] = json!(receipt.seq);
            evaluation["evaluation_hash"] = json!(receipt.event_hash);
            let mut public = plan.public_fields(commit.seq, &commit.event_hash);
            public["status"] = json!(status);
            public["evaluation"] = evaluation;
            Ok((200, public))
        },
    )
}

struct Comparator {
    reference: Decimal,
    threshold: Magnitude,
    known_pass_passed: bool,
    planted_fail_rejected: bool,
}

impl Comparator {
    fn new(plan: &StoredPlan) -> Result<Self> {
        let atol = parse_decimal(&plan.protocol.atol, "atol")?;
        let rtol = parse_decimal(&plan.protocol.rtol, "rtol")?;
        if atol.negative || rtol.negative {
            return Err(bad("atol and rtol must be nonnegative"));
        }
        let reference = parse_decimal(&plan.values.reference, "reference")?;
        let known_pass = parse_decimal(&plan.values.known_pass, "known_pass")?;
        let planted_fail = parse_decimal(&plan.values.planted_fail, "planted_fail")?;
        let threshold = atol
            .magnitude
            .add(&rtol.magnitude.multiply(&reference.magnitude));
        let mut comparison = Self {
            reference,
            threshold,
            known_pass_passed: false,
            planted_fail_rejected: false,
        };
        comparison.known_pass_passed = comparison.matches(&known_pass);
        comparison.planted_fail_rejected = !comparison.matches(&planted_fail);
        Ok(comparison)
    }

    fn matches(&self, value: &Decimal) -> bool {
        self.reference
            .absolute_difference(value)
            .compare(&self.threshold)
            != Ordering::Greater
    }
}

fn parse_decimal(text: &str, field: &str) -> Result<Decimal> {
    Decimal::parse(text).ok_or_else(|| bad(format!(
        "{field} must be a finite real decimal with at most {MAX_NUMBER_LENGTH} characters and exponent magnitude at most {MAX_DECIMAL_EXPONENT}"
    )))
}

/// Exact finite decimals suitable for comparing bounded mpmath output. Digits are
/// stored least-significant first, with their base-10 position tracked separately.
struct Decimal {
    negative: bool,
    magnitude: Magnitude,
}

struct Magnitude {
    digits: Vec<u8>,
    exponent: i32,
}

impl Decimal {
    fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_NUMBER_LENGTH {
            return None;
        }
        let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
        let (significand, exponent) = match unsigned.split_once(['e', 'E']) {
            Some((significand, exponent)) => (significand, exponent.parse::<i32>().ok()?),
            None => (unsigned, 0),
        };
        if !(-MAX_DECIMAL_EXPONENT..=MAX_DECIMAL_EXPONENT).contains(&exponent) {
            return None;
        }
        let (whole, fractional) = significand.split_once('.').unwrap_or((significand, ""));
        if (whole.is_empty() && fractional.is_empty())
            || !whole
                .bytes()
                .chain(fractional.bytes())
                .all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let digits = whole
            .bytes()
            .chain(fractional.bytes())
            .rev()
            .map(|byte| byte - b'0')
            .collect();
        let magnitude = Magnitude::new(digits, exponent - fractional.len() as i32);
        Some(Self {
            negative: text.starts_with('-') && !magnitude.digits.is_empty(),
            magnitude,
        })
    }

    fn absolute_difference(&self, other: &Self) -> Magnitude {
        if self.negative != other.negative {
            self.magnitude.add(&other.magnitude)
        } else if self.magnitude.compare(&other.magnitude) == Ordering::Less {
            other.magnitude.subtract(&self.magnitude)
        } else {
            self.magnitude.subtract(&other.magnitude)
        }
    }
}

impl Magnitude {
    fn new(mut digits: Vec<u8>, exponent: i32) -> Self {
        while digits.last() == Some(&0) {
            digits.pop();
        }
        Self {
            exponent: if digits.is_empty() { 0 } else { exponent },
            digits,
        }
    }

    fn high(&self) -> i32 {
        self.exponent + self.digits.len() as i32
    }

    fn digit(&self, position: i32) -> u8 {
        usize::try_from(position - self.exponent)
            .ok()
            .and_then(|index| self.digits.get(index))
            .copied()
            .unwrap_or(0)
    }

    fn compare(&self, other: &Self) -> Ordering {
        let low = self.exponent.min(other.exponent);
        let high = self.high().max(other.high());
        (low..high)
            .rev()
            .map(|position| self.digit(position))
            .cmp((low..high).rev().map(|position| other.digit(position)))
    }

    fn add(&self, other: &Self) -> Self {
        let low = self.exponent.min(other.exponent);
        let mut output = Vec::new();
        let mut carry = 0;
        for position in low..self.high().max(other.high()) {
            let sum = self.digit(position) + other.digit(position) + carry;
            output.push(sum % 10);
            carry = sum / 10;
        }
        output.push(carry);
        Self::new(output, low)
    }

    fn subtract(&self, other: &Self) -> Self {
        debug_assert_ne!(self.compare(other), Ordering::Less);
        let low = self.exponent.min(other.exponent);
        let mut output = Vec::new();
        let mut borrow = 0;
        for position in low..self.high().max(other.high()) {
            let difference = self.digit(position) as i16 - other.digit(position) as i16 - borrow;
            output.push(difference.rem_euclid(10) as u8);
            borrow = i16::from(difference < 0);
        }
        Self::new(output, low)
    }

    fn multiply(&self, other: &Self) -> Self {
        if self.digits.is_empty() || other.digits.is_empty() {
            return Self::new(Vec::new(), 0);
        }
        let mut buckets = vec![0_u32; self.digits.len() + other.digits.len()];
        for (i, a) in self.digits.iter().enumerate() {
            for (j, b) in other.digits.iter().enumerate() {
                buckets[i + j] += u32::from(*a) * u32::from(*b);
            }
        }
        let mut carry = 0;
        let mut output = Vec::with_capacity(buckets.len());
        for bucket in buckets {
            let value = bucket + carry;
            output.push((value % 10) as u8);
            carry = value / 10;
        }
        Self::new(output, self.exponent + other.exponent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol(atol: &str, rtol: &str) -> Protocol {
        Protocol {
            session_id: "s-example".into(),
            claim: "Refinement estimate".into(),
            candidate_method: "Grid integration".into(),
            reference_method: "Closed-form primitive".into(),
            axis: Axis::Resolution,
            atol: atol.into(),
            rtol: rtol.into(),
        }
    }

    #[test]
    fn relative_tolerance_uses_exact_signed_decimal_arithmetic() {
        let plan = StoredPlan {
            id: "plan-example".into(),
            project_id: "p-example".into(),
            protocol: protocol("0.25", "0.1"),
            values: PrivateValues {
                salt: String::new(),
                reference: "-2e1".into(),
                known_pass: "-17.75".into(),
                planted_fail: "-17.74999999999999999999999".into(),
            },
        };
        let comparator = Comparator::new(&plan).unwrap();
        assert!(comparator.known_pass_passed && comparator.planted_fail_rejected);
        assert!(comparator.matches(&Decimal::parse("-22.25").unwrap()));
        assert!(!comparator.matches(&Decimal::parse("20").unwrap()));
        assert!(!comparator.matches(&Decimal::parse("-22.25000000000000000001").unwrap()));
    }

    #[test]
    fn candidate_requires_sample_stability_under_the_frozen_tolerance() {
        let mut value = json!({
            "value": "recorded report",
            "validation": {
                "schema_version": 1, "kind": "numerical_convergence",
                "axis": "resolution", "status": "within_tolerance",
                "stable": true, "atol": "0.001", "rtol":"0", "tail": 1,
                "samples": [
                    { "level": "4", "value": "1", "error": null },
                    { "level": "8", "value": "1.01", "passes": true, "error": null },
                ]
            }
        });
        let protocol = protocol("0.001", "0");
        assert_eq!(candidate_value(&value, &protocol), None);
        value["validation"]["samples"][1]["value"] = json!("1.001");
        assert_eq!(candidate_value(&value, &protocol).as_deref(), Some("1.001"));
        value["validation"]["atol"] = json!("0.0001");
        assert_eq!(candidate_value(&value, &protocol), None);
    }

    #[test]
    fn candidate_requires_strictly_increasing_levels_in_the_axis_range() {
        let mut value = json!({
            "value": "recorded report",
            "validation": {
                "schema_version": 1, "kind": "numerical_convergence",
                "axis": "resolution", "status": "within_tolerance",
                "stable": true, "atol": "0.001", "rtol":"0", "tail": 1,
                "samples": [
                    { "level": "4", "value": "1", "error": null },
                    { "level": "1e9", "value": "1", "passes": true, "error": null },
                ]
            }
        });
        let mut protocol = protocol("0.001", "0");
        assert!(candidate_value(&value, &protocol).is_some());
        for invalid in [
            json!("4.0"),
            json!("3"),
            json!("1.000000001e9"),
            Value::Null,
        ] {
            value["validation"]["samples"][1]["level"] = invalid;
            assert!(candidate_value(&value, &protocol).is_none());
        }
        protocol.axis = Axis::Precision;
        value["validation"]["axis"] = json!("precision");
        value["validation"]["samples"][0]["level"] = json!(4);
        value["validation"]["samples"][1]["level"] = json!(256);
        assert!(candidate_value(&value, &protocol).is_some());
        for invalid in [json!(257), json!(2), json!(4.5), json!("8")] {
            value["validation"]["samples"][1]["level"] = invalid;
            assert!(candidate_value(&value, &protocol).is_none());
        }
    }
}
