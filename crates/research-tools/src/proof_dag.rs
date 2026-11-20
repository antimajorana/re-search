//! Structured Proof-Graph DAG tools (`ledger_append`, `ledger_supersede`, `ledger_query`).

use crate::latex_repair::repair_under_escaped_latex;
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef, EVENT_SCHEMA_VERSION};

pub const LEDGER_APPEND: &str = "ledger_append";
pub const LEDGER_SUPERSEDE: &str = "ledger_supersede";
pub const LEDGER_QUERY: &str = "ledger_query";

const VALID_ENTRY_TYPES: &[&str] = &[
    "assumption",
    "derivation",
    "result",
    "symbolic_computation",
    "numerical_computation",
    "reference_lookup",
    "verification",
    "correction",
    "final_answer",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Pending,
    Accepted,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofEntry {
    pub entry_id: String,
    pub seq: u64,
    pub entry_type: String,
    pub status: ProofStatus,
    pub summary: String,
    pub details: String,
    pub latex: Option<String>,
    pub depends_on: Vec<String>,
    pub tool_call_ref: Option<String>,
    pub superseded_by: Option<String>,
    pub verification: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProofDag {
    pub session: String,
    pub entries: Vec<ProofEntry>,
    pub edges: Vec<(String, String)>,
    pub latest_full_verification_accepted: bool,
    pub unverified_computations: Vec<u64>,
}

pub fn ledger_append_def() -> ToolDef {
    use policy::EffectClass;
    ToolDef {
        id: LEDGER_APPEND.into(),
        description: "Append a structured proof-graph entry with acyclic depends_on references.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "entry_type": {
                    "type": "string",
                    "enum": VALID_ENTRY_TYPES
                },
                "summary": { "type": "string", "maxLength": 500 },
                "details": { "type": "string", "maxLength": 16000 },
                "latex": { "type": "string", "maxLength": 8000 },
                "depends_on": { "type": "array", "items": { "type": "string" } },
                "tool_call_ref": { "type": "string", "maxLength": 200 }
            },
            "required": ["entry_type", "summary", "details"],
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn ledger_supersede_def() -> ToolDef {
    use policy::EffectClass;
    ToolDef {
        id: LEDGER_SUPERSEDE.into(),
        description: "Supersede an earlier proof entry with a correction entry.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "entry_id": { "type": "string" },
                "replaced_by": { "type": "string" },
                "reason": { "type": "string", "maxLength": 2000 }
            },
            "required": ["entry_id", "replaced_by", "reason"],
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn ledger_query_def() -> ToolDef {
    use policy::EffectClass;
    ToolDef {
        id: LEDGER_QUERY.into(),
        description: "Query proof-graph entries and status for the current session.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "entry_id": { "type": "string" },
                "include_superseded": { "type": "boolean" }
            },
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry) {
    registry.register(ledger_append_def(), Arc::new(handle_append));
    registry.register(ledger_supersede_def(), Arc::new(handle_supersede));
    registry.register(ledger_query_def(), Arc::new(handle_query));
}

fn handle_append(ctx: &CallCtx<'_>, args: &Value) -> Result<Value, String> {
    let entry_type = args
        .get("entry_type")
        .and_then(Value::as_str)
        .ok_or("`entry_type` must be a string")?;
    if !VALID_ENTRY_TYPES.contains(&entry_type) {
        return Err(format!("invalid `entry_type` `{entry_type}`"));
    }
    let raw_summary = args
        .get("summary")
        .and_then(Value::as_str)
        .ok_or("`summary` must be a string")?;
    let raw_details = args
        .get("details")
        .and_then(Value::as_str)
        .ok_or("`details` must be a string")?;
    let raw_latex = args.get("latex").and_then(Value::as_str);
    let tool_call_ref = args
        .get("tool_call_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let depends_on: Vec<String> = match args.get("depends_on") {
        None => Vec::new(),
        Some(v) => {
            let arr = v.as_array().ok_or("`depends_on` must be an array of entry IDs")?;
            let mut seen = BTreeSet::new();
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or("all items in `depends_on` must be strings")?
                    .trim();
                if s.is_empty() {
                    return Err("`depends_on` must not contain empty entry IDs".into());
                }
                if seen.insert(s.to_string()) {
                    out.push(s.to_string());
                }
            }
            out
        }
    };

    let summary = repair_under_escaped_latex(raw_summary);
    let details = repair_under_escaped_latex(raw_details);
    let latex = raw_latex.map(repair_under_escaped_latex);

    let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
    let events = guard
        .read_session(ctx.session)
        .map_err(|e| format!("cannot read session `{}`: {e}", ctx.session))?;

    let mut existing_ids = BTreeSet::new();
    let mut entry_count = 0usize;
    for ev in &events {
        if ev.kind == "proof.entry" {
            entry_count += 1;
            if let Some(eid) = ev.payload.get("entry_id").and_then(Value::as_str) {
                existing_ids.insert(eid.to_string());
            }
        }
    }
    for dep in &depends_on {
        if !existing_ids.contains(dep) {
            return Err(format!("`depends_on` references unknown entry `{dep}`"));
        }
    }

    let mut next_num = entry_count + 1;
    let mut entry_id = format!("e-{next_num}");
    while existing_ids.contains(&entry_id) {
        next_num += 1;
        entry_id = format!("e-{next_num}");
    }

    let mut payload = json!({
        "entry_id": entry_id,
        "entry_type": entry_type,
        "summary": summary,
        "details": details,
        "depends_on": depends_on,
    });
    if let Some(lx) = &latex {
        payload["latex"] = Value::String(lx.clone());
    }
    if let Some(tref) = &tool_call_ref {
        payload["tool_call_ref"] = Value::String(tref.clone());
    }

    let seq = guard
        .append(ctx.session, "proof.entry", EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| format!("proof.entry append failed: {e}"))?;

    Ok(json!({
        "entry_id": entry_id,
        "seq": seq,
        "entry_type": entry_type,
        "status": "pending",
        "depends_on": depends_on,
    }))
}

fn handle_supersede(ctx: &CallCtx<'_>, args: &Value) -> Result<Value, String> {
    let entry_id = args
        .get("entry_id")
        .and_then(Value::as_str)
        .ok_or("`entry_id` must be a string")?
        .trim();
    let replaced_by = args
        .get("replaced_by")
        .and_then(Value::as_str)
        .ok_or("`replaced_by` must be a string")?
        .trim();
    let raw_reason = args
        .get("reason")
        .and_then(Value::as_str)
        .ok_or("`reason` must be a string")?;

    if entry_id.is_empty() || replaced_by.is_empty() {
        return Err("`entry_id` and `replaced_by` must not be empty".into());
    }
    if entry_id == replaced_by {
        return Err("an entry cannot supersede itself".into());
    }
    let reason = repair_under_escaped_latex(raw_reason);

    let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
    let events = guard
        .read_session(ctx.session)
        .map_err(|e| format!("cannot read session `{}`: {e}", ctx.session))?;

    let mut types_by_id: BTreeMap<String, String> = BTreeMap::new();
    for ev in &events {
        if ev.kind == "proof.entry" {
            if let (Some(eid), Some(et)) = (
                ev.payload.get("entry_id").and_then(Value::as_str),
                ev.payload.get("entry_type").and_then(Value::as_str),
            ) {
                types_by_id.insert(eid.to_string(), et.to_string());
            }
        }
    }

    if !types_by_id.contains_key(entry_id) {
        return Err(format!("unknown `entry_id` `{entry_id}`"));
    }
    let Some(repl_type) = types_by_id.get(replaced_by) else {
        return Err(format!("unknown `replaced_by` entry `{replaced_by}`"));
    };
    if repl_type != "correction" {
        return Err(format!(
            "`replaced_by` `{replaced_by}` must have entry_type `correction` (found `{repl_type}`)"
        ));
    }

    let seq = guard
        .append(
            ctx.session,
            "proof.supersede",
            EVENT_SCHEMA_VERSION,
            &json!({
                "entry_id": entry_id,
                "replaced_by": replaced_by,
                "reason": reason,
            }),
        )
        .map_err(|e| format!("proof.supersede append failed: {e}"))?;

    Ok(json!({
        "ok": true,
        "seq": seq,
        "entry_id": entry_id,
        "replaced_by": replaced_by,
    }))
}

fn handle_query(ctx: &CallCtx<'_>, args: &Value) -> Result<Value, String> {
    let target_id = args.get("entry_id").and_then(Value::as_str).map(str::trim);
    let include_superseded = args
        .get("include_superseded")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let dag = {
        let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        read_proof_dag(&guard, ctx.session)?
    };

    if let Some(eid) = target_id {
        let entry = dag
            .entries
            .into_iter()
            .find(|e| e.entry_id == eid)
            .ok_or_else(|| format!("proof entry `{eid}` not found"))?;
        return serde_json::to_value(entry).map_err(|e| e.to_string());
    }

    let entries: Vec<ProofEntry> = if include_superseded {
        dag.entries
    } else {
        dag.entries
            .into_iter()
            .filter(|e| e.status != ProofStatus::Superseded)
            .collect()
    };

    Ok(json!({
        "session": dag.session,
        "entries": entries,
        "edges": dag.edges,
        "latest_full_verification_accepted": dag.latest_full_verification_accepted,
        "unverified_computations": dag.unverified_computations,
    }))
}

pub fn read_proof_dag(ledger: &Ledger, session: &str) -> Result<ProofDag, String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("cannot read session `{session}`: {e}"))?;

    let mut entries: Vec<ProofEntry> = Vec::new();
    let mut index_by_id: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut latest_full_verification_accepted = false;
    let mut sympy_calls: BTreeMap<String, u64> = BTreeMap::new();
    let mut sympy_result_seqs: Vec<(String, u64)> = Vec::new();
    let mut cited_call_refs: BTreeSet<String> = BTreeSet::new();
    let mut cited_result_seqs: BTreeSet<u64> = BTreeSet::new();

    for ev in &events {
        let p = &ev.payload;
        match ev.kind.as_str() {
            "tool.request" => {
                if p.get("tool").and_then(Value::as_str) == Some("sympy_eval") {
                    if let Some(cid) = p.get("call_id").and_then(Value::as_str) {
                        sympy_calls.insert(cid.to_string(), ev.seq);
                    }
                }
            }
            "tool.result" => {
                if let Some(cid) = p.get("call_id").and_then(Value::as_str) {
                    if sympy_calls.contains_key(cid) {
                        sympy_result_seqs.push((cid.to_string(), ev.seq));
                        latest_full_verification_accepted = false;
                    }
                }
            }
            "proof.entry" => {
                let entry_id = p
                    .get("entry_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let entry_type = p
                    .get("entry_type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let depends_on: Vec<String> = p
                    .get("depends_on")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                for dep in &depends_on {
                    edges.push((dep.clone(), entry_id.clone()));
                }
                let tool_call_ref = p
                    .get("tool_call_ref")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if entry_type == "result" {
                    if let Some(tref) = &tool_call_ref {
                        cited_call_refs.insert(tref.clone());
                        collect_cited_seqs(tref, &mut cited_result_seqs);
                    }
                }
                let idx = entries.len();
                index_by_id.insert(entry_id.clone(), idx);
                entries.push(ProofEntry {
                    entry_id,
                    seq: ev.seq,
                    entry_type,
                    status: ProofStatus::Pending,
                    summary: p.get("summary").and_then(Value::as_str).unwrap_or_default().into(),
                    details: p.get("details").and_then(Value::as_str).unwrap_or_default().into(),
                    latex: p.get("latex").and_then(Value::as_str).map(str::to_string),
                    depends_on,
                    tool_call_ref,
                    superseded_by: None,
                    verification: None,
                });
                latest_full_verification_accepted = false;
            }
            "proof.supersede" => {
                if let (Some(eid), Some(repl)) = (
                    p.get("entry_id").and_then(Value::as_str),
                    p.get("replaced_by").and_then(Value::as_str),
                ) {
                    if let Some(&idx) = index_by_id.get(eid) {
                        entries[idx].status = ProofStatus::Superseded;
                        entries[idx].superseded_by = Some(repl.to_string());
                    }
                }
                latest_full_verification_accepted = false;
            }
            "proof.verification_attempt" => {
                latest_full_verification_accepted = false;
            }
            "proof.verification" => {
                let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
                let verdict = p.get("verdict").and_then(Value::as_str).unwrap_or("");
                if let Some(target_id) = p.get("target_entry_id").and_then(Value::as_str) {
                    if let Some(&idx) = index_by_id.get(target_id) {
                        entries[idx].verification = Some(p.clone());
                        if entries[idx].status != ProofStatus::Superseded {
                            entries[idx].status = match verdict {
                                "accepted" => ProofStatus::Accepted,
                                "rejected" => ProofStatus::Rejected,
                                _ => ProofStatus::Pending,
                            };
                        }
                    }
                }
                if kind == "full" {
                    latest_full_verification_accepted = verdict == "accepted";
                }
            }
            _ => {}
        }
    }

    let unverified_computations: Vec<u64> = sympy_result_seqs
        .into_iter()
        .filter_map(|(cid, seq)| {
            let cited = cited_result_seqs.contains(&seq)
                || cited_call_refs.contains(&cid)
                || cited_call_refs.iter().any(|r| r.contains(&cid));
            (!cited).then_some(seq)
        })
        .collect();

    Ok(ProofDag {
        session: session.to_string(),
        entries,
        edges,
        latest_full_verification_accepted,
        unverified_computations,
    })
}

fn collect_cited_seqs(tref: &str, out: &mut BTreeSet<u64>) {
    if let Ok(n) = tref.trim().parse::<u64>() {
        out.insert(n);
    }
    if let Some((_, tail)) = tref.rsplit_once(':') {
        if let Ok(n) = tail.trim().parse::<u64>() {
            out.insert(n);
        }
    }
}
