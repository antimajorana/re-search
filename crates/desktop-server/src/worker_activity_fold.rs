//! Pure ledger projector for worker/exchange/verifier activity, blobs, and `run.control` v1
//! (`00-overview.md` §3.4). Performs zero model calls, zero network calls, and scrubs secrets.

#[path = "worker_activity_detail.rs"]
mod detail;

use agent::role_routing::RoleModelInput;
use agent::usage::{extract_usage, RoleUsageRollup, TokenUsage};
use detail::{build_job_detail, extract_end_usage, find_owner_job};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const EVENT_RUN_CONTROL: &str = "run.control";
pub const WORKER_SUMMARY_UNVERIFIED_LABEL: &str = "(worker model summary — unverified)";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunControlsView {
    pub focus_notes: Vec<Value>,
    pub pinned_items: Vec<String>,
    pub dismissed_items: Vec<Value>,
    pub pause_after_stage: bool,
    pub next_run_roles: BTreeMap<String, Option<RoleModelInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_frontier_fallback: Option<bool>,
    pub budget_delta: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub kind: String,
    pub id: String,
    pub blob: String,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobUsageView {
    pub calls: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub unknown_usage_calls: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerJobSummary {
    pub job_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub stage_or_task: String,
    pub role: String,
    pub provider: String,
    pub requested_model: String,
    pub used_model: String,
    pub routing_reason: String,
    pub calls_used: u64,
    pub usage: JobUsageView,
    pub status: String,
    pub disposition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition_reason: Option<String>,
    pub artifact_refs: Vec<ArtifactRef>,
    pub returned_item_ids: Vec<String>,
    pub pinned: bool,
    pub dismissed: bool,
    pub seq_start: u64,
    pub seq_end: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerJobDetail {
    pub job: WorkerJobSummary,
    pub artifact_missing: bool,
    pub request_blobs: Vec<Value>,
    pub response_blobs: Vec<Value>,
    pub worker_summary: Value,
    pub context_supplied: Vec<Value>,
    pub tool_receipts: Vec<Value>,
    pub returned_items: Vec<Value>,
    pub checks: Vec<Value>,
}

pub struct FoldedWorkerActivity {
    pub jobs: Vec<WorkerJobSummary>,
    pub detail: Option<WorkerJobDetail>,
    pub usage_rollup: Value,
    pub controls: RunControlsView,
    pub pinned_roles: BTreeMap<String, Value>,
}

#[derive(Default)]
pub(super) struct JobBuilder {
    pub(super) job_id: String,
    pub(super) kind: String,
    pub(super) run_id: Option<String>,
    pub(super) stage_or_task: String,
    pub(super) role: String,
    pub(super) provider: String,
    pub(super) requested_model: String,
    pub(super) used_model: String,
    pub(super) routing_reason: String,
    pub(super) calls_used: u64,
    pub(super) seen_turns: u64,
    pub(super) in_tok: u64,
    pub(super) out_tok: u64,
    pub(super) known_in: u64,
    pub(super) known_out: u64,
    pub(super) unknown_calls: u64,
    pub(super) ok: Option<bool>,
    pub(super) cancelled: bool,
    pub(super) error: Option<String>,
    pub(super) seq_start: u64,
    pub(super) seq_end: u64,
    pub(super) callers: BTreeSet<String>,
    pub(super) req_blobs: Vec<(u64, String)>,
    pub(super) resp_blobs: Vec<(u64, String)>,
    pub(super) summary_blob: Option<String>,
    pub(super) inline_summary: Option<String>,
    pub(super) item_ids: Vec<String>,
    pub(super) check_rows: Vec<Value>,
}

impl JobBuilder {
    fn record_usage(&mut self, u: TokenUsage) {
        if let Some(i) = u.input {
            self.in_tok = self.in_tok.saturating_add(i);
            self.known_in += 1;
        }
        if let Some(o) = u.output {
            self.out_tok = self.out_tok.saturating_add(o);
            self.known_out += 1;
        }
        if !u.is_fully_known() {
            self.unknown_calls += 1;
        }
    }

    fn push_item(&mut self, id: &str) {
        if !id.is_empty() && !self.item_ids.iter().any(|x| x == id) {
            self.item_ids.push(id.to_string());
        }
    }

    fn push_resp_blob(&mut self, seq: u64, hash: Option<&str>) {
        if let Some(h) = hash.filter(|s| !s.is_empty()) {
            self.resp_blobs.push((seq, h.to_string()));
        }
    }

    fn push_req_blob(&mut self, seq: u64, hash: Option<&str>) {
        if let Some(h) = hash.filter(|s| !s.is_empty()) {
            self.req_blobs.push((seq, h.to_string()));
        }
    }
}

pub fn fold_controls(events: &[Event]) -> RunControlsView {
    let mut view = RunControlsView::default();
    let mut pinned = BTreeSet::new();
    let mut dismissed: BTreeMap<String, Option<String>> = BTreeMap::new();

    for ev in events.iter().filter(|e| e.kind == EVENT_RUN_CONTROL) {
        let p = &ev.payload;
        match p.get("action").and_then(Value::as_str).unwrap_or("") {
            "focus_note" => {
                if let Some(note) = p.get("note").and_then(Value::as_str) {
                    view.focus_notes.push(json!({ "seq": ev.seq, "note": note }));
                }
            }
            "pin_worker_item" => {
                if let Some(id) = p.get("item_id").and_then(Value::as_str) {
                    pinned.insert(id.to_string());
                    dismissed.remove(id);
                }
            }
            "dismiss_worker_item" => {
                if let Some(id) = p.get("item_id").and_then(Value::as_str) {
                    let reason = p.get("reason").and_then(Value::as_str).map(str::to_string);
                    dismissed.insert(id.to_string(), reason);
                    pinned.remove(id);
                }
            }
            "pause_after_stage" => {
                view.pause_after_stage = p.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            }
            "next_run_roles" => {
                if let Some(obj) = p.get("roles").and_then(Value::as_object) {
                    for (k, v) in obj {
                        if v.is_null() {
                            view.next_run_roles.insert(k.clone(), None);
                        } else if let Ok(spec) = serde_json::from_value::<RoleModelInput>(v.clone()) {
                            view.next_run_roles.insert(k.clone(), Some(spec));
                        }
                    }
                }
                if let Some(allow) = p.get("allow_frontier_fallback").and_then(Value::as_bool) {
                    view.allow_frontier_fallback = Some(allow);
                }
            }
            "budget_delta" => {
                if let Some(d) = p.get("delta").and_then(Value::as_i64) {
                    view.budget_delta = view.budget_delta.saturating_add(d);
                }
            }
            _ => {}
        }
    }

    view.pinned_items = pinned.into_iter().collect();
    view.dismissed_items = dismissed
        .into_iter()
        .map(|(item_id, reason)| json!({ "item_id": item_id, "reason": reason }))
        .collect();
    view
}

pub fn fold_worker_activity(
    events: &[Event],
    ledger: &Ledger,
    secrets: &[String],
    detail_job_id: Option<&str>,
) -> FoldedWorkerActivity {
    let controls = fold_controls(events);
    let usage_rollup = RoleUsageRollup::from_events(events).to_json();
    let pinned_set: BTreeSet<&str> = controls.pinned_items.iter().map(String::as_str).collect();
    let dismissed_map: BTreeMap<&str, Option<&str>> = controls
        .dismissed_items
        .iter()
        .filter_map(|v| Some((v.get("item_id")?.as_str()?, v.get("reason").and_then(Value::as_str))))
        .collect();

    let mut order: Vec<String> = Vec::new();
    let mut jobs: BTreeMap<String, JobBuilder> = BTreeMap::new();
    let mut active_task_or_stage = String::from("session");
    let mut active_run_id: Option<String> = None;
    let (mut pending_ver, mut pending_strat, mut pending_nar) = (None, None, None);
    let mut pinned_roles: BTreeMap<String, Value> = BTreeMap::new();

    fn ensure_job<'a>(
        order: &mut Vec<String>,
        map: &'a mut BTreeMap<String, JobBuilder>,
        id: &str,
    ) -> &'a mut JobBuilder {
        if !map.contains_key(id) {
            order.push(id.to_string());
            map.insert(id.to_string(), JobBuilder { job_id: id.to_string(), ..Default::default() });
        }
        map.get_mut(id).expect("inserted")
    }

    for ev in events {
        let p = &ev.payload;
        match ev.kind.as_str() {
            "session.start" | "session.resume" | "investigation.task_requested" | "idea.started" | "idea.stage" | "solve.setup" => {
                if ev.kind == "investigation.task_requested" {
                    if let Some(t) = non_empty(p, &["task"]) { active_task_or_stage = t.to_string(); }
                    active_run_id = Some(ev.seq.to_string());
                } else if ev.kind == "idea.stage" {
                    if let Some(s) = non_empty(p, &["stage"]) { active_task_or_stage = s.to_string(); }
                    if let Some(r) = non_empty(p, &["request_id", "run_id"]) { active_run_id = Some(r.to_string()); }
                } else if let Some(r) = non_empty(p, &["run_id"]) {
                    active_run_id = Some(r.to_string());
                }
                if let Some(obj) = p.get("roles").and_then(Value::as_object) {
                    for (k, v) in obj { pinned_roles.insert(k.clone(), v.clone()); }
                }
            }
            "delegate.start" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("delegate-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                j.kind = "delegate".into();
                j.run_id = non_empty(p, &["run_id"]).map(str::to_string).or_else(|| active_run_id.clone());
                j.stage_or_task = non_empty(p, &["stage", "task_kind", "task"]).unwrap_or(&active_task_or_stage).to_string();
                j.role = non_empty(p, &["role", "worker"]).unwrap_or("worker").to_string();
                j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                j.requested_model = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                if j.used_model.is_empty() { j.used_model = j.requested_model.clone(); }
                j.routing_reason = non_empty(p, &["routing_reason"]).unwrap_or("unknown").to_string();
                j.seq_start = ev.seq;
                j.seq_end = ev.seq;
                if let Some(c) = non_empty(p, &["caller"]) { j.callers.insert(c.to_string()); }
            }
            "delegate.turn" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("delegate-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                if j.kind.is_empty() { j.kind = "delegate".into(); }
                if j.role.is_empty() { j.role = non_empty(p, &["role"]).unwrap_or("worker").to_string(); }
                if j.stage_or_task.is_empty() { j.stage_or_task = active_task_or_stage.clone(); }
                if j.seq_start == 0 { j.seq_start = ev.seq; }
                j.seq_end = ev.seq;
                j.seen_turns += 1;
                if let Some(m) = non_empty(p, &["used_model", "model"]) { j.used_model = m.to_string(); }
                j.record_usage(p.get("usage").map(extract_usage).unwrap_or_default());
                j.push_resp_blob(ev.seq, non_empty(p, &["raw_blob"]));
            }
            "delegate.end" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("delegate-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                if j.kind.is_empty() { j.kind = "delegate".into(); }
                if j.role.is_empty() { j.role = non_empty(p, &["worker", "role"]).unwrap_or("worker").to_string(); }
                if j.provider.is_empty() { j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string(); }
                if j.requested_model.is_empty() { j.requested_model = non_empty(p, &["model"]).unwrap_or("unknown").to_string(); }
                if let Some(m) = non_empty(p, &["used_model", "model"]) {
                    if j.used_model.is_empty() || j.used_model == "unknown" { j.used_model = m.to_string(); }
                }
                if j.routing_reason.is_empty() { j.routing_reason = non_empty(p, &["routing_reason"]).unwrap_or("unknown").to_string(); }
                if j.stage_or_task.is_empty() { j.stage_or_task = active_task_or_stage.clone(); }
                if j.seq_start == 0 { j.seq_start = ev.seq; }
                j.seq_end = ev.seq;
                let end_turns = p.get("turns").and_then(Value::as_u64).unwrap_or(0);
                j.calls_used = j.seen_turns.max(end_turns);
                if end_turns > j.seen_turns {
                    let missing = end_turns - j.seen_turns;
                    for _ in 0..missing {
                        let u = if j.seen_turns == 0 && missing == 1 { extract_end_usage(p).unwrap_or_default() } else { TokenUsage::default() };
                        j.record_usage(u);
                    }
                }
                j.ok = p.get("ok").and_then(Value::as_bool);
                j.cancelled = p.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
                j.error = p.get("error").and_then(Value::as_str).map(str::to_string);
                if let Some(b) = non_empty(p, &["raw_blob"]) { j.summary_blob = Some(b.to_string()); }
                for key in ["proof_entries", "handles_created"] {
                    if let Some(arr) = p.get(key).and_then(Value::as_array) {
                        for item in arr.iter().filter_map(Value::as_str) { j.push_item(item); }
                    }
                }
            }
            "explore.start" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("explore-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                j.kind = "explore".into();
                j.role = "explorer_worker".into();
                j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                j.requested_model = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                j.used_model = j.requested_model.clone();
                j.routing_reason = non_empty(p, &["routing_reason"]).unwrap_or("unknown").to_string();
                j.stage_or_task = non_empty(p, &["problem"]).unwrap_or("explore").to_string();
                j.seq_start = ev.seq;
                j.seq_end = ev.seq;
            }
            "explore.branch" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("explore-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                j.seq_end = ev.seq;
                let bt = p.get("turns").and_then(Value::as_u64).unwrap_or(0);
                j.seen_turns += bt;
                for idx in 0..bt {
                    let u = if idx == 0 && bt == 1 { extract_end_usage(p).unwrap_or_default() } else { TokenUsage::default() };
                    j.record_usage(u);
                }
                j.push_resp_blob(ev.seq, non_empty(p, &["raw_blob"]));
            }
            "explore.end" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("explore-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                if j.kind.is_empty() { j.kind = "explore".into(); }
                if j.role.is_empty() { j.role = "explorer_worker".into(); }
                j.seq_end = ev.seq;
                j.calls_used = j.seen_turns.max(p.get("turns").and_then(Value::as_u64).unwrap_or(0));
                j.ok = p.get("ok").and_then(Value::as_bool);
                if let Some(b) = non_empty(p, &["raw_blob"]) { j.summary_blob = Some(b.to_string()); }
            }
            "idea.exchange" => {
                let id = non_empty(p, &["job_id", "exchange_id", "call_id"]).map(str::to_string).unwrap_or_else(|| format!("exchange-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                j.kind = "exchange".into();
                j.run_id = non_empty(p, &["run_id"]).map(str::to_string).or_else(|| active_run_id.clone());
                j.stage_or_task = non_empty(p, &["stage", "exchange"]).unwrap_or(&active_task_or_stage).to_string();
                j.role = non_empty(p, &["role"]).unwrap_or("worker").to_string();
                j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                j.requested_model = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                j.used_model = non_empty(p, &["used_model", "model"]).unwrap_or(&j.requested_model).to_string();
                j.routing_reason = non_empty(p, &["routing_reason"]).unwrap_or("unknown").to_string();
                if j.seq_start == 0 { j.seq_start = ev.seq; }
                j.seq_end = ev.seq;
                j.calls_used += 1;
                j.record_usage(p.get("usage").map(extract_usage).or_else(|| extract_end_usage(p)).unwrap_or_default());
                j.ok = p.get("ok").and_then(Value::as_bool).or(Some(p.get("error").is_none()));
                j.error = p.get("error").and_then(Value::as_str).map(str::to_string);
                j.push_req_blob(ev.seq, non_empty(p, &["req_blob", "request_blob"]));
                j.push_resp_blob(ev.seq, non_empty(p, &["raw_blob", "response_blob"]));
                if let Some(arr) = p.get("returned_items").and_then(Value::as_array) {
                    for it in arr.iter().filter_map(Value::as_str) { j.push_item(it); }
                }
            }
            "verifier.request" => {
                let id = non_empty(p, &["call_id"]).map(str::to_string).unwrap_or_else(|| format!("verifier-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                j.kind = "verifier".into();
                j.role = non_empty(p, &["role"]).unwrap_or("verifier").to_string();
                j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                j.requested_model = non_empty(p, &["verifier_model", "model"]).unwrap_or("unknown").to_string();
                j.used_model = j.requested_model.clone();
                j.routing_reason = non_empty(p, &["routing_reason"]).unwrap_or("unknown").to_string();
                j.stage_or_task = non_empty(p, &["kind"]).unwrap_or("verify").to_string();
                j.seq_start = ev.seq;
                j.seq_end = ev.seq;
                j.calls_used += 1;
                j.push_req_blob(ev.seq, non_empty(p, &["req_blob"]));
                pending_ver = Some(id);
            }
            "proof.verification" | "claim.review" | "verifier.response" | "verifier.incomplete" | "verifier.cancelled" => {
                let id = pending_ver.take().unwrap_or_else(|| format!("verifier-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                if j.kind.is_empty() { j.kind = "verifier".into(); }
                if j.role.is_empty() { j.role = non_empty(p, &["role"]).unwrap_or("verifier").to_string(); }
                if j.provider.is_empty() { j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string(); }
                if let Some(m) = non_empty(p, &["used_model", "verifier_model", "model"]) {
                    if j.requested_model.is_empty() { j.requested_model = m.to_string(); }
                    j.used_model = m.to_string();
                }
                if j.stage_or_task.is_empty() { j.stage_or_task = non_empty(p, &["kind"]).unwrap_or("verify").to_string(); }
                if j.seq_start == 0 { j.seq_start = ev.seq; }
                j.seq_end = ev.seq;
                if j.calls_used == 0 { j.calls_used = 1; }
                j.record_usage(p.get("usage").map(extract_usage).unwrap_or_default());
                j.cancelled = ev.kind == "verifier.cancelled";
                j.ok = Some(ev.kind != "verifier.incomplete" && ev.kind != "verifier.cancelled");
                if let Some(sum) = p.get("summary").and_then(Value::as_str) { j.inline_summary = Some(sum.to_string()); }
                j.push_resp_blob(ev.seq, non_empty(p, &["raw_blob"]));
                if let Some(target) = non_empty(p, &["target_entry_id"]) { j.push_item(target); }
            }
            "strategist.request" | "narrator.request" => {
                let is_strat = ev.kind == "strategist.request";
                let role = if is_strat { "strategist" } else { "narrator" };
                let id = format!("{role}-{}", ev.seq);
                let j = ensure_job(&mut order, &mut jobs, &id);
                j.kind = role.into();
                j.role = role.into();
                j.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                j.requested_model = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                j.used_model = j.requested_model.clone();
                j.stage_or_task = if is_strat { "strategist" } else { "narrate" }.into();
                j.seq_start = ev.seq;
                j.seq_end = ev.seq;
                j.calls_used = 1;
                j.push_req_blob(ev.seq, non_empty(p, &["req_blob"]));
                if is_strat { pending_strat = Some(id); } else { pending_nar = Some(id); }
            }
            "strategist.consult" | "narrator.report" => {
                let is_strat = ev.kind == "strategist.consult";
                let role = if is_strat { "strategist" } else { "narrator" };
                let pending = if is_strat { pending_strat.take() } else { pending_nar.take() };
                let id = pending.unwrap_or_else(|| format!("{role}-{}", ev.seq));
                let j = ensure_job(&mut order, &mut jobs, &id);
                if j.kind.is_empty() { j.kind = role.into(); j.role = role.into(); }
                if j.seq_start == 0 { j.seq_start = ev.seq; j.calls_used = 1; }
                j.seq_end = ev.seq;
                j.ok = Some(true);
                j.record_usage(p.get("usage").map(extract_usage).unwrap_or_default());
                j.push_resp_blob(ev.seq, non_empty(p, &["raw_blob"]));
                if let Some(adv) = p.get("advice").and_then(Value::as_str) { j.inline_summary = Some(adv.to_string()); }
            }
            "investigation.evidence" | "evidence.quote" | "evidence.abstract" | "solve.expr" => {
                let item_id = match ev.kind.as_str() {
                    "investigation.evidence" => format!("ev-{}", ev.seq),
                    "evidence.quote" => format!("xe-{}", ev.seq),
                    "evidence.abstract" => format!("xa-{}", ev.seq),
                    _ => format!("{}@{}", p.get("name").and_then(Value::as_str).unwrap_or("expr"), p.get("version").and_then(Value::as_u64).unwrap_or(1)),
                };
                if let Some(j) = find_owner_job(&mut jobs, &order, p.get("caller").and_then(Value::as_str), ev.seq) {
                    j.push_item(&item_id);
                }
            }
            "solve.check" => {
                if let Some(j) = find_owner_job(&mut jobs, &order, p.get("caller").and_then(Value::as_str), ev.seq) {
                    j.check_rows.push(p.clone());
                }
            }
            _ => {}
        }
    }

    let mut summaries = Vec::with_capacity(order.len());
    let mut detail = None;
    for id in &order {
        let j = &jobs[id];
        let mut refs = Vec::new();
        for (_, h) in &j.req_blobs {
            refs.push(ArtifactRef { kind: "request_blob".into(), id: h[..h.len().min(12)].into(), blob: h.clone(), missing: ledger.get_blob(h).is_err() });
        }
        for (_, h) in &j.resp_blobs {
            refs.push(ArtifactRef { kind: "response_blob".into(), id: h[..h.len().min(12)].into(), blob: h.clone(), missing: ledger.get_blob(h).is_err() });
        }
        if let Some(h) = &j.summary_blob {
            refs.push(ArtifactRef { kind: "summary_blob".into(), id: "summary".into(), blob: h.clone(), missing: ledger.get_blob(h).is_err() });
        }

        let job_pinned = pinned_set.contains(j.job_id.as_str()) || j.item_ids.iter().any(|it| pinned_set.contains(it.as_str()));
        let job_dismiss_reason = dismissed_map.get(j.job_id.as_str()).copied().flatten();
        let all_dismissed = !j.item_ids.is_empty() && j.item_ids.iter().all(|it| dismissed_map.contains_key(it.as_str()));
        let job_dismissed = dismissed_map.contains_key(j.job_id.as_str()) || all_dismissed;

        let status = if j.cancelled {
            "cancelled"
        } else if j.ok == Some(true) {
            "done"
        } else if j.ok == Some(false) {
            if !j.item_ids.is_empty() { "partial" } else { "failed" }
        } else {
            "running"
        };
        let (disposition, disposition_reason) = if job_dismissed {
            ("rejected", job_dismiss_reason.map(str::to_string).or_else(|| Some("dismissed by reader".into())))
        } else if job_pinned {
            ("accepted", Some("pinned by reader".into()))
        } else {
            match status {
                "done" => ("accepted", None),
                "partial" => ("partial", j.error.clone().or_else(|| Some("partial worker output retained".into()))),
                "running" => ("pending", None),
                _ => ("rejected", j.error.clone()),
            }
        };

        let summary = WorkerJobSummary {
            job_id: j.job_id.clone(),
            kind: j.kind.clone(),
            run_id: j.run_id.clone(),
            stage_or_task: if j.stage_or_task.is_empty() { j.role.clone() } else { j.stage_or_task.clone() },
            role: j.role.clone(),
            provider: if j.provider.is_empty() { "unknown".into() } else { j.provider.clone() },
            requested_model: if j.requested_model.is_empty() { "unknown".into() } else { j.requested_model.clone() },
            used_model: if j.used_model.is_empty() { "unknown".into() } else { j.used_model.clone() },
            routing_reason: if j.routing_reason.is_empty() { "unknown".into() } else { j.routing_reason.clone() },
            calls_used: j.calls_used.max(j.seen_turns),
            usage: JobUsageView {
                calls: j.calls_used.max(j.seen_turns),
                input_tokens: (j.known_in > 0).then_some(j.in_tok),
                output_tokens: (j.known_out > 0).then_some(j.out_tok),
                unknown_usage_calls: j.unknown_calls,
            },
            status: status.into(),
            disposition: disposition.into(),
            disposition_reason,
            artifact_refs: refs,
            returned_item_ids: j.item_ids.clone(),
            pinned: job_pinned,
            dismissed: job_dismissed,
            seq_start: j.seq_start,
            seq_end: j.seq_end,
        };

        if detail_job_id == Some(j.job_id.as_str()) {
            detail = Some(build_job_detail(&summary, j, events, ledger, secrets, &pinned_set, &dismissed_map));
        }
        summaries.push(summary);
    }

    FoldedWorkerActivity { jobs: summaries, detail, usage_rollup, controls, pinned_roles }
}

fn non_empty<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| payload.get(*k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()))
}
