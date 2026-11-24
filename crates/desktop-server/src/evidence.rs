//! Claim-linked evidence from the verified session ledger and researcher citations.

use crate::projects::{project_for_session, Project};
use crate::routes_session::{bad, internal, not_found, ApiResult};
use crate::state::DesktopState;
use agent::transport::GENERIC_KEY_ENV_VARS;
use agent::{ProviderKeyStore, SecretKey};
use ledger::{Event, Ledger};
use research_tools::proof_dag::tool_reference_matches;
use research_tools::{sandbox::sanitize_rel_components, validate_session_id};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_ENVELOPE_BYTES: usize = 256 * 1024;
const MAX_ENTRY_SUPPORT_BYTES: usize = 128 * 1024;
const MAX_ENTRIES: usize = 16;
const MAX_SUPPORT: usize = 12;
const MAX_REVIEWS: usize = 3;
const TEXT_LIMIT: usize = 1_200;
const VALIDATION_ROWS: usize = 8;

pub(crate) fn handle_session_evidence(
    state: &DesktopState,
    session_id: &str,
    entry_id: Option<&str>,
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if entry_id.is_some_and(|id| id.is_empty() || id.len() > 200) {
        return Err(bad("invalid proof entry ID"));
    }
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let (report, events) = match ledger.read_verified_session(session_id) {
        Ok(snapshot) => snapshot,
        Err(ledger::Error::UnknownSession(_)) if entry_id.is_none() => {
            return Ok((
                200,
                json!({
                    "session": session_id, "ledger_verified": false, "head_hash": null,
                    "entries": [], "truncated": false,
                }),
            ));
        }
        Err(ledger::Error::UnknownSession(_)) => return Err(not_found("proof entry not found")),
        Err(_) => {
            return Err(bad(
                "The session ledger or a recorded artifact could not be verified.",
            ))
        }
    };
    let project = project_for_session(state, session_id)?;
    let scrub = EvidenceText::from_state(state);
    let evidence = SessionEvidence::new(&ledger, &events, project.as_ref(), &scrub);
    let targets = if let Some(id) = entry_id {
        vec![
            evidence
                .nodes
                .get(id)
                .ok_or_else(|| not_found("proof entry not found"))?
                .event,
        ]
    } else {
        events
            .iter()
            .rev()
            .filter(|e| e.kind == "proof.entry")
            .take(MAX_ENTRIES + 1)
            .collect()
    };

    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut truncated = false;
    for event in targets {
        let Some(id) = event.payload["entry_id"]
            .as_str()
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let entry = evidence.describe_entry(session_id, id);
        bytes += serde_json::to_vec(&entry)
            .map_err(|e| internal(e.to_string()))?
            .len();
        if entries.len() >= MAX_ENTRIES || bytes > MAX_ENVELOPE_BYTES {
            truncated = true;
            break;
        }
        entries.push(entry);
    }
    Ok((
        200,
        json!({
            "session": session_id,
            "ledger_verified": true,
            "head_hash": report.head_hash,
            "entries": entries,
            "truncated": truncated,
        }),
    ))
}

#[derive(Default)]
struct EvidenceText {
    credentials: Vec<SecretKey>,
}

impl EvidenceText {
    fn from_state(state: &DesktopState) -> Self {
        let mut credentials = Vec::new();
        if let Ok(store) = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir) {
            credentials.extend(store.providers.values().filter_map(|provider| {
                SecretKey::new(provider.api_key.as_deref()?.to_string()).ok()
            }));
        }
        let provider_keys = crate::connections::API_PROVIDERS
            .iter()
            .flat_map(|provider| provider.own_key_env_vars().iter().copied());
        let other_keys = [
            "ANTHROPIC_AUTH_TOKEN",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "RE_SEARCH_CORE_TOKEN",
        ];
        credentials.extend(
            provider_keys
                .chain(GENERIC_KEY_ENV_VARS)
                .chain(other_keys)
                .filter_map(|key| SecretKey::new(state.env_var(key)?.into_string().ok()?).ok()),
        );
        Self { credentials }
    }

    fn clean(&self, text: &str, limit: usize) -> String {
        // Redact before clipping so a cut through a token cannot expose a prefix of it.
        let mut clean = text.to_string();
        for key in &self.credentials {
            clean = key.redact_from(&clean);
        }
        for prefix in ["sk-", "AIza", "ghp_", "github_pat_", "xoxb-"] {
            let mut start = 0;
            while let Some(offset) = clean[start..].find(prefix) {
                let left = start + offset;
                let length = clean.as_bytes()[left..]
                    .iter()
                    .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-' || **b == b'_')
                    .count();
                if length >= prefix.len() + 16 {
                    clean.replace_range(left..left + length, "[REDACTED]");
                    start = left + "[REDACTED]".len();
                } else {
                    start = left + length;
                }
            }
        }
        clean.retain(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'));
        if clean.len() > limit {
            clean.truncate(clean.floor_char_boundary(limit));
            clean.push('…');
        }
        clean
    }

    fn field(&self, value: &Value, limit: usize) -> Option<String> {
        value
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|text| self.clean(text, limit))
    }

    fn path(&self, value: &Value) -> Option<String> {
        let (_, path) = sanitize_rel_components(value.as_str()?, false).ok()?;
        Some(self.clean(&path, 256))
    }

    fn validation_scalar(&self, value: &Value, limit: usize) -> Option<Value> {
        match value {
            Value::String(s) if !s.is_empty() => Some(Value::String(self.clean(s, limit))),
            Value::Number(_) | Value::Bool(_) => Some(value.clone()),
            _ => None,
        }
    }

    fn validation_point(&self, point: &Value) -> Option<Value> {
        if let Some(coords) = point.as_object() {
            let coords = coords
                .iter()
                .take(12)
                .filter_map(|(name, value)| {
                    Some((self.clean(name, 72), self.validation_scalar(value, 100)?))
                })
                .collect();
            Some(Value::Object(coords))
        } else {
            self.validation_scalar(point, 180)
        }
    }

    fn numeric_value(&self, value: &Value) -> Option<Value> {
        if value.is_object() {
            Some(json!({
                "real": self.validation_scalar(&value["real"], 160)?,
                "imag": self.validation_scalar(&value["imag"], 160)?,
            }))
        } else {
            self.validation_scalar(value, 160)
        }
    }

    fn validation_rows(&self, rows: &Value, keys: &[&str], keep_tail: bool) -> Vec<Value> {
        let Some(rows) = rows.as_array() else {
            return Vec::new();
        };
        let mut selected: Vec<_> = rows.iter().filter(|row| row.is_object()).collect();
        if selected.len() > VALIDATION_ROWS && keep_tail {
            let tail_start = selected.len() - (VALIDATION_ROWS - 1);
            let omitted_error = selected[1..tail_start]
                .iter()
                .find(|row| row["error"].is_object())
                .copied();
            selected.drain(1..tail_start);
            if let Some(error) = omitted_error {
                selected[0] = error;
            }
        } else if selected.len() > VALIDATION_ROWS {
            // Keep the first counterexample visible even if it falls after the preview.
            if let Some(counterexample) = selected[VALIDATION_ROWS..]
                .iter()
                .find(|row| row["status"] == "counterexample")
                .copied()
            {
                selected[VALIDATION_ROWS - 1] = counterexample;
            }
            selected.truncate(VALIDATION_ROWS);
        }
        selected
            .into_iter()
            .map(|row| {
                Value::Object(
                    keys.iter()
                        .filter_map(|key| {
                            let field = match *key {
                                "point" => self.validation_point(&row[key]),
                                "value" => self.numeric_value(&row[key]),
                                "error" if row[key].is_object() => Some(json!({
                                    "type": self.field(&row[key]["type"], 72),
                                    "message": self.field(&row[key]["message"], 300),
                                })),
                                _ => self.validation_scalar(&row[key], 160),
                            }?;
                            Some(((*key).to_string(), field))
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn validation(&self, value: &Value) -> Option<Value> {
        if value["schema_version"].as_u64() != Some(1) {
            return None;
        }
        let kind = value["kind"].as_str()?;
        let status = value["status"].as_str()?;
        match kind {
            "symbolic_identity" => {
                let status = match status {
                    "symbolic_agreement" | "counterexample" | "undetermined" => status,
                    _ => return None,
                };
                let assumptions = value["assumptions"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .take(12)
                    .filter_map(|(symbol, flags)| {
                        let clean = flags
                            .as_object()?
                            .iter()
                            .filter(|(_, value)| value.is_boolean())
                            .take(12)
                            .map(|(key, value)| (self.clean(key, 72), value.clone()))
                            .collect();
                        Some((self.clean(symbol, 72), Value::Object(clean)))
                    })
                    .collect::<serde_json::Map<_, _>>();
                let samples = self.validation_rows(
                    &value["samples"],
                    &["point", "lhs", "rhs", "status"],
                    false,
                );
                let limits = self.validation_rows(
                    &value["limits"],
                    &["point", "direction", "lhs", "rhs", "status"],
                    false,
                );
                let rows_omitted = ["samples", "limits"].into_iter().any(|key| {
                    value[key]
                        .as_array()
                        .is_some_and(|rows| rows.len() > VALIDATION_ROWS)
                });
                Some(json!({
                    "schema_version": 1,
                    "kind": kind,
                    "status": status,
                    "lhs": self.validation_scalar(&value["lhs"], 500),
                    "rhs": self.validation_scalar(&value["rhs"], 500),
                    "residual": self.validation_scalar(&value["residual"], 500),
                    "assumptions": assumptions,
                    "domain": {
                        "requested": self.field(&value["domain"]["requested"], 350),
                        "effective": self.field(&value["domain"]["effective"], 350),
                        "common": self.field(&value["domain"]["common"], 350),
                        "status": self.field(&value["domain"]["status"], 72),
                    },
                    "samples": samples,
                    "limits": limits,
                    "dimensions": {
                        "status": self.field(&value["dimensions"]["status"], 72),
                        "lhs": self.field(&value["dimensions"]["lhs"], 160),
                        "rhs": self.field(&value["dimensions"]["rhs"], 160),
                        "error": self.field(&value["dimensions"]["error"], 350),
                    },
                    "rows_omitted": rows_omitted,
                }))
            }
            "numerical_convergence" => {
                let axis = match value["axis"].as_str()? {
                    axis @ ("resolution" | "basis" | "precision") => axis,
                    _ => return None,
                };
                let status = match status {
                    "within_tolerance"
                    | "not_stable"
                    | "outside_reference_tolerance"
                    | "insufficient_data"
                    | "evaluation_failed" => status,
                    _ => return None,
                };
                Some(json!({
                    "schema_version": 1,
                    "kind": kind,
                    "axis": axis,
                    "status": status,
                    "stable": value["stable"].as_bool(),
                    "within_tolerance": value["within_tolerance"].as_bool(),
                    "reference_within_tolerance": value["reference_within_tolerance"].as_bool(),
                    "tail": value["tail"].as_u64(),
                    "analysis_dps": value["analysis_dps"].as_u64(),
                    "atol": self.validation_scalar(&value["atol"], 160),
                    "rtol": self.validation_scalar(&value["rtol"], 160),
                    "reference": self.numeric_value(&value["reference"]),
                    "samples": self.validation_rows(&value["samples"], &[
                        "level", "work_dps", "value", "delta", "threshold", "passes",
                        "reference_error", "reference_threshold", "reference_passes", "error"
                    ], true),
                    "rows_omitted": value["samples"].as_array().is_some_and(|rows| rows.len() > VALIDATION_ROWS),
                }))
            }
            _ => None,
        }
    }
}

struct Node<'a> {
    event: &'a Event,
    retired: bool,
    reviews: VecDeque<&'a Event>,
    validation_decisions: VecDeque<&'a Event>,
}

struct ToolCall<'a> {
    request: &'a Event,
    outcome: Option<&'a Event>,
}

struct SessionEvidence<'a> {
    ledger: &'a Ledger,
    nodes: HashMap<&'a str, Node<'a>>,
    tools: HashMap<&'a str, ToolCall<'a>>,
    sources: HashMap<&'a str, &'a Event>,
    source_receipts: HashMap<u64, &'a Event>,
    project: Option<&'a Project>,
    scrub: &'a EvidenceText,
}

impl<'a> SessionEvidence<'a> {
    fn new(
        ledger: &'a Ledger,
        events: &'a [Event],
        project: Option<&'a Project>,
        scrub: &'a EvidenceText,
    ) -> Self {
        let mut index = Self {
            ledger,
            nodes: HashMap::new(),
            tools: HashMap::new(),
            sources: HashMap::new(),
            source_receipts: HashMap::new(),
            project,
            scrub,
        };
        for event in events {
            let data = &event.payload;
            let entry_id = data["entry_id"].as_str().unwrap_or_default();
            let call_id = data["call_id"].as_str().unwrap_or_default();
            match event.kind.as_str() {
                "proof.entry" if !entry_id.is_empty() => {
                    index.nodes.insert(
                        entry_id,
                        Node {
                            event,
                            retired: false,
                            reviews: VecDeque::new(),
                            validation_decisions: VecDeque::new(),
                        },
                    );
                }
                "proof.supersede" => {
                    if let Some(node) = index.nodes.get_mut(entry_id) {
                        node.retired = true;
                    }
                }
                "proof.verification" => {
                    if let Some(node) = data["target_entry_id"]
                        .as_str()
                        .and_then(|id| index.nodes.get_mut(id))
                    {
                        if node.reviews.len() == MAX_REVIEWS {
                            node.reviews.pop_front();
                        }
                        node.reviews.push_back(event);
                    }
                }
                "validation.plan.evaluated" => {
                    let candidate = &data["evaluation"];
                    if let Some(node) = candidate["entry_id"]
                        .as_str()
                        .and_then(|id| index.nodes.get_mut(id))
                    {
                        if candidate["entry_hash"] == node.event.event_hash {
                            if node.validation_decisions.len() == MAX_REVIEWS {
                                node.validation_decisions.pop_front();
                            }
                            node.validation_decisions.push_back(event);
                        }
                    }
                }
                "tool.request" if !call_id.is_empty() => {
                    index.tools.insert(
                        call_id,
                        ToolCall {
                            request: event,
                            outcome: None,
                        },
                    );
                }
                "tool.result" | "tool.denied" => {
                    if let Some(call) = index.tools.get_mut(call_id) {
                        call.outcome = Some(event);
                    }
                }
                "source.ingested" => {
                    if let Some(id) = data["source_id"].as_str() {
                        index.sources.insert(id, event);
                        index.source_receipts.insert(event.seq, event);
                    }
                }
                _ => {}
            }
        }
        index
    }

    fn describe_entry(&self, session_id: &str, id: &str) -> Value {
        let node = &self.nodes[id];
        let mut support = Vec::new();
        let mut seen = HashSet::new();
        let mut todo = VecDeque::from([id]);
        let mut truncated = false;
        while let Some(current) = todo.pop_front() {
            if !seen.insert(current) {
                continue;
            }
            let Some(via) = self.nodes.get(current) else {
                continue;
            };
            let historically_invalid = via.retired
                || via
                    .reviews
                    .back()
                    .is_some_and(|r| r.payload["verdict"] == "rejected");
            if current != id && historically_invalid {
                continue;
            }
            if let Some(item) = self.describe_tool(via.event) {
                support.push(item);
            }
            support.extend(via.validation_decisions.iter().rev().filter_map(|event| {
                let decision = &event.payload;
                let status = match decision["status"].as_str()? {
                    "accepted" => "frozen_checks_passed",
                    "rejected" => "frozen_checks_failed",
                    _ => return None,
                };
                Some(json!({
                    "kind": "validation_decision",
                    "status": status,
                    "via_entry_id": current,
                    "result_seq": event.seq,
                    "plan_id": self.scrub.field(&decision["plan_id"], 72),
                    "commit_hash": sha256(&decision["commit_hash"]),
                    "candidate_value": self.scrub.field(&decision["evaluation"]["value"], 384),
                    "known_pass_passed": decision["evaluation"]["known_pass_passed"].as_bool(),
                    "planted_fail_rejected": decision["evaluation"]["planted_fail_rejected"].as_bool(),
                }))
            }));
            if let Some(project) = self.project {
                support.extend(
                    self.researcher_citations(project, session_id, current)
                        .take(MAX_SUPPORT + 1),
                );
            }
            if support.len() > MAX_SUPPORT {
                support.truncate(MAX_SUPPORT);
                truncated = true;
                break;
            }
            if let Some(dependencies) = via.event.payload["depends_on"].as_array() {
                todo.extend(dependencies.iter().filter_map(Value::as_str));
            }
        }
        let mut bytes = 0;
        let visible = support
            .iter()
            .take_while(|record| {
                bytes += serde_json::to_vec(record).map_or(0, |encoded| encoded.len());
                bytes <= MAX_ENTRY_SUPPORT_BYTES
            })
            .count();
        if visible < support.len() {
            support.truncate(visible);
            truncated = true;
        }
        let reviews: Vec<_> = node
            .reviews
            .iter()
            .rev()
            .map(|event| {
                let data = &event.payload;
                let issues: Vec<_> = data["issues"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(4)
                    .filter_map(|issue| {
                        self.scrub.field(issue.get("message").unwrap_or(issue), 280)
                    })
                    .collect();
                json!({
                    "seq": event.seq,
                    "scope": match data["kind"].as_str() {
                        Some("step") => "step",
                        Some("full") => "full",
                        _ => "unknown",
                    },
                    "verdict": match data["verdict"].as_str() {
                        Some("accepted") => "accepted",
                        Some("rejected") => "rejected",
                        _ => "unknown",
                    },
                    "summary": self.scrub.field(&data["summary"], 700),
                    "answer_sha256": sha256(&data["answer_sha256"]),
                    "issues": issues,
                })
            })
            .collect();
        json!({
            "entry_id": self.scrub.clean(id, 200),
            "seq": node.event.seq,
            "retired": node.retired,
            "model_reviews": reviews,
            "support": support,
            "truncated": truncated,
        })
    }

    fn describe_tool(&self, entry: &Event) -> Option<Value> {
        let reference = entry.payload["tool_call_ref"].as_str()?.trim();
        if reference.is_empty() {
            return None;
        }
        let via = self.scrub.field(&entry.payload["entry_id"], 200);
        let call = self.tools.get(reference).or_else(|| {
            self.tools.iter().find_map(|(id, call)| {
                call.outcome
                    .filter(|outcome| tool_reference_matches(reference, id, outcome.seq))
                    .map(|_| call)
            })
        });
        let Some(call) = call.filter(|call| call.request.seq < entry.seq) else {
            return Some(json!({
                "kind": "unavailable", "status": "unknown_reference", "via_entry_id": via,
                "tool_call_ref": self.scrub.clean(reference, 200),
            }));
        };
        let name = call.request.payload["tool"].as_str().unwrap_or_default();
        let outcome = call.outcome.filter(|event| event.seq < entry.seq);
        let result = outcome
            .filter(|ev| ev.kind == "tool.result" && ev.payload["ok"].as_bool() == Some(true))
            .and_then(|ev| self.recorded_value(&ev.payload["result"]));
        let mut item = json!({
            "kind": match name {
                "sympy_eval" => "computation",
                "lean_check" => "lean",
                "doc_ingest" => "source_ingested",
                "fs_read" => "source_read",
                "search_paper_passages" => "paper_passages",
                "search_formal_library" => "formal_library",
                "arxiv_search" | "literature_search" => "literature",
                _ => "unavailable",
            },
            "tool": self.scrub.clean(name, 50),
            "tool_call_ref": self.scrub.field(&call.request.payload["call_id"], 200),
            "via_entry_id": via,
            "request_seq": call.request.seq,
            "result_seq": outcome.map(|event| event.seq),
            "status": match outcome {
                None => "result_unavailable",
                Some(event) if event.kind == "tool.denied" => "declined",
                Some(event) if event.payload["ok"].as_bool() != Some(true) => "failed",
                Some(_) if result.is_none() => "result_unavailable",
                Some(_) => "recorded",
            },
        });
        let Some(result) = result else {
            return Some(item);
        };
        match name {
            "sympy_eval" => {
                item["status"] = json!(
                    if result.get("value").is_some() && result["error"].is_null() {
                        "completed"
                    } else {
                        "failed"
                    }
                );
                item["value"] = json!(self.scrub.field(&result["value"], TEXT_LIMIT));
                item["latex"] = json!(self.scrub.field(&result["latex"], TEXT_LIMIT));
                item["stdout"] = json!(self.scrub.field(&result["stdout"], TEXT_LIMIT));
                item["stderr"] = json!(self.scrub.field(&result["stderr"], TEXT_LIMIT));
                item["error"] = json!(self.scrub.field(&result["error"]["message"], 500));
                item["worker_truncated"] = json!(result["truncated"].as_bool().unwrap_or(false));
                item["validation_truncated"] =
                    json!(result["validation_truncated"].as_bool().unwrap_or(false));
                if item["status"] == "completed" {
                    if let Some(validation) = self.scrub.validation(&result["validation"]) {
                        item["validation"] = validation;
                    }
                }
            }
            "lean_check" => {
                let args = self
                    .recorded_value(&call.request.payload["args"])
                    .unwrap_or(Value::Null);
                let axioms: Vec<_> = result["axioms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(12)
                    .filter_map(|axiom| self.scrub.field(axiom, 100))
                    .collect();
                item["status"] = json!(match result["status"].as_str() {
                    Some(
                        status @ ("verified" | "failed" | "source_changed" | "rejected_sorry"
                        | "rejected_axiom" | "lean_not_found" | "timeout"),
                    ) => status,
                    _ => "result_unavailable",
                });
                item["path"] = json!(self.scrub.path(&args["path"]));
                item["code"] = json!(self.scrub.field(&args["code"], TEXT_LIMIT));
                item["source_sha256"] = json!(sha256(&result["source_sha256"]));
                item["axioms"] = json!(axioms);
                item["error"] = json!(self.scrub.field(&result["error"], 500));
            }
            "fs_read" => {
                item["path"] = json!(self.scrub.path(&result["path"]));
                item["root"] = json!(match result["root"].as_str() {
                    Some(root @ ("workspace" | "sources")) => Some(root),
                    _ => None,
                });
                for key in [
                    "start_line",
                    "end_line",
                    "offset",
                    "end_offset",
                    "match_count",
                ] {
                    item[key] = json!(result[key].as_u64());
                }
                item["excerpt"] = json!(self.scrub.field(&result["content"], TEXT_LIMIT));
                if item["root"] == "sources" {
                    if let Some(link) = result["path"]
                        .as_str()
                        .and_then(|path| self.source_read_link(path, &result, outcome))
                    {
                        item["reader"] = link;
                    }
                }
            }
            "doc_ingest" => {
                let source = match result["event_seq"].as_u64() {
                    Some(seq) => self.source_receipts.get(&seq),
                    None => result["source_id"]
                        .as_str()
                        .and_then(|id| self.sources.get(id)),
                };
                if let Some(source) = source.filter(|source| {
                    outcome.is_some_and(|event| source.seq < event.seq)
                        && source.payload["source_id"] == result["source_id"]
                        && sha256(&source.payload["raw_blob"]).is_some()
                        && source.payload["raw_blob"] == result["raw_blob"]
                }) {
                    item["source"] = self.describe_source(source);
                    item["excerpt"] = json!(self.scrub.field(&result["excerpt"], TEXT_LIMIT));
                } else {
                    item["status"] = json!("source_record_unavailable");
                }
            }
            "search_paper_passages" => {
                let records: Vec<_> = result["passages"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(6)
                    .filter_map(|hit| {
                        let seq = hit["source_seq"].as_u64()?;
                        let source = self.source_receipts.get(&seq)?;
                        let path = hit["path"].as_str()?;
                        let start = hit["start_line"].as_u64()?;
                        let end = hit["end_line"].as_u64()?;
                        let file_hash = sha256(&hit["file_sha256"])?;
                        if start == 0
                            || end < start
                            || outcome.is_none_or(|ev| seq >= ev.seq)
                            || source.payload["source_id"] != hit["source_id"]
                            || source.payload["raw_blob"] != hit["source_sha256"]
                            || source.payload["files_sha256"][path] != hit["file_sha256"]
                        {
                            return None;
                        }
                        Some(json!({
                            "title": self.scrub.field(&source.payload["title"], 180),
                            "path": self.scrub.path(&hit["path"])?,
                            "file_sha256": file_hash,
                            "start_line": start,
                            "end_line": end,
                            "excerpt": self.scrub.field(&hit["excerpt"], TEXT_LIMIT)?,
                            "reader": self.reader_link(source, path, file_hash, Some((start, end))),
                        }))
                    })
                    .collect();
                item["status"] = json!(if !records.is_empty() {
                    "matches"
                } else if result["status"] == "no_match" {
                    "no_match"
                } else {
                    "source_record_unavailable"
                });
                item["passages"] = json!(records);
            }
            "search_formal_library" => {
                let declarations: Vec<_> = result["results"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(5)
                    .filter_map(|record| {
                        Some(json!({
                            "name": self.scrub.field(&record["qualified_name"], 180)
                                .or_else(|| self.scrub.field(&record["name"], 180))?,
                            "module": self.scrub.field(&record["module"], 180),
                            "path": self.scrub.path(&record["path"])?,
                            "line": record["line"].as_u64(),
                            "signature": self.scrub.field(&record["signature"], TEXT_LIMIT)?,
                            "source_sha256": sha256(&record["source_sha256"])?,
                        }))
                    })
                    .collect();
                item["status"] = json!(if !declarations.is_empty() {
                    "declarations_found"
                } else if result["status"] == "library_not_found" {
                    "library_not_found"
                } else {
                    "no_match"
                });
                item["declarations"] = json!(declarations);
            }
            "arxiv_search" | "literature_search" => {
                let hits = result["hits"]
                    .as_array()
                    .or_else(|| result["papers"].as_array());
                let records: Vec<_> = hits.into_iter().flatten().take(4).map(|hit| json!({
                    "title": self.scrub.field(&hit["title"], 180),
                    "arxiv_id": self.scrub.field(
                        if name == "arxiv_search" { &hit["id"] } else { &hit["arxiv_id"] }, 120
                    ),
                    "doi": self.scrub.field(&hit["doi"], 180),
                    "withdrawn": hit["withdrawn"].as_bool().unwrap_or(false),
                    "retracted": hit["retracted"].as_bool().unwrap_or(false),
                })).collect();
                item["records"] = json!(records);
            }
            _ => item["status"] = json!("unsupported_reference"),
        }
        Some(item)
    }

    fn researcher_citations<'b>(
        &'b self,
        project: &'b Project,
        session: &'b str,
        id: &'b str,
    ) -> impl Iterator<Item = Value> + 'b {
        project.items.iter().filter(move |item| {
            item.session_id.as_deref() == Some(session) && item.entry_id.as_deref() == Some(id)
        }).filter_map(move |item| {
            let source_id = item.source_id.as_deref();
            let source = source_id.and_then(|id| project.sources.iter().find(|source| source.id == id));
            let ingested_id = match source {
                Some(source) if source.session_id.as_deref().is_none_or(|owner| owner == session) => {
                    source.source_id.as_deref()
                }
                Some(_) => None,
                None => source_id,
            };
            let ledger_source = ingested_id.and_then(|id| self.sources.get(id));
            let location = item.location.as_deref().or_else(|| source.and_then(|s| s.location.as_deref()));
            let quote = item.quote.as_deref().or_else(|| source.and_then(|s| s.quote.as_deref()));
            if source_id.is_none() && location.is_none() && quote.is_none() {
                return None;
            }
            Some(json!({
                "kind": "citation",
                "status": "researcher_recorded",
                "via_entry_id": self.scrub.clean(id, 200),
                "source_id": source_id.map(|id| self.scrub.clean(id, 200)),
                "source_title": source.map(|s| self.scrub.clean(&s.title, 200)),
                "source_doi": source.and_then(|s| s.doi.as_deref()).map(|doi| self.scrub.clean(doi, 180)),
                "location": location.map(|s| self.scrub.clean(s, 300)),
                "quote": quote.map(|s| self.scrub.clean(s, TEXT_LIMIT)),
                "source": ledger_source.map(|event| self.describe_source(event)),
            }))
        })
    }

    fn describe_source(&self, source: &Event) -> Value {
        let data = &source.payload;
        let primary = data["assembled_path"]
            .as_str()
            .map_or(&data["primary_path"], |_| &data["assembled_path"]);
        let file_sha256 = primary
            .as_str()
            .and_then(|path| sha256(&data["files_sha256"][path]));
        let reader = primary
            .as_str()
            .zip(file_sha256)
            .and_then(|(path, digest)| self.reader_link(source, path, digest, None));
        json!({
            "seq": source.seq,
            "source_id": self.scrub.field(&data["source_id"], 200),
            "title": self.scrub.field(&data["title"], 200),
            "format": self.scrub.field(&data["format"], 50),
            "status": self.scrub.field(&data["status"], 80),
            "pages": data["page_count"].as_u64(),
            "raw_sha256": sha256(&data["raw_blob"]),
            "file_sha256": file_sha256,
            "path": self.scrub.path(primary),
            "reader": reader,
        })
    }

    fn source_read_link(
        &self,
        path: &str,
        result: &Value,
        outcome: Option<&Event>,
    ) -> Option<Value> {
        let source_id = path.split_once('/')?.0;
        let digest = sha256(&result["file_sha256"])?;
        let before = outcome?.seq;
        let source = self
            .source_receipts
            .values()
            .filter(|source| {
                source.seq < before
                    && source.payload["source_id"] == source_id
                    && source.payload["files_sha256"][path] == digest
            })
            .max_by_key(|source| source.seq)?;
        let lines = result["start_line"]
            .as_u64()
            .zip(result["end_line"].as_u64())
            .filter(|(start, end)| *start > 0 && end >= start);
        self.reader_link(source, path, digest, lines)
    }

    fn reader_link(
        &self,
        source: &Event,
        path: &str,
        digest: &str,
        lines: Option<(u64, u64)>,
    ) -> Option<Value> {
        let id = source.payload["source_id"].as_str()?;
        let (_, clean_path) = sanitize_rel_components(path, false).ok()?;
        // Only send a usable locator when sanitization preserves its exact identity.
        if clean_path != path
            || path.split_once('/')?.0 != id
            || self.scrub.clean(path, 256) != path
            || self.scrub.clean(id, 200) != id
        {
            return None;
        }
        Some(json!({
            "source_id": id,
            "path": path,
            "file_sha256": digest,
            "start_line": lines.map(|(start, _)| start),
            "end_line": lines.map(|(_, end)| end),
        }))
    }

    fn recorded_value(&self, slot: &Value) -> Option<Value> {
        if let Some(text) = slot["inline"].as_str().or_else(|| slot.as_str()) {
            serde_json::from_str(text).ok()
        } else if let Some(hash) = slot["blob"].as_str() {
            serde_json::from_slice(&self.ledger.get_blob(hash).ok()?).ok()
        } else {
            slot.as_object().map(|_| slot.clone())
        }
    }
}

fn sha256(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
