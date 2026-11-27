//! Solve mode reproducible ZIP bundle export (S19c).
//!
//! Exports `ledger.json` (with `solve.*` events), `notebook.ipynb`, `code/step_<seq>.py`
//! with protocol-2 `exprs_in` staging, `_re_search_runtime/checks/*`, `sources/*`,
//! `replay_manifest.json` (`solve_checks` for R19 replay), `run_all_checks.py`, and `REPRODUCE.md`
//! directly from `Ledger` + `SolveState` without calling `latex::load_session_export_data`.

use crate::latex::ReplayWorker;
use crate::notebook::{build_solve_notebook_bytes, resolve_slot_text, NotebookOptions};
use crate::zip::{
    collect_source_manifest, insert_runtime_files, recorded_turns, worker_environment,
    write_zip_entries,
};
use ledger::{sha256_hex, Event, Ledger};
use research_tools::sandbox::sanitize_rel_components;
use research_tools::solve_exprs::{load_handle_srepr, stage_exprs_in_from_state};
use research_tools::solve_model::{event_kinds, SolveCheck};
use research_tools::solve_state::SolveState;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

struct SolveSympyCall {
    seq: u64,
    result_seq: u64,
    call_id: String,
    code: String,
    exprs_in: BTreeMap<String, String>,
    result: Value,
    worker: ReplayWorker,
    worker_environment: Value,
    execution_order: u64,
}

pub(crate) fn build_solve_reproducible_zip(
    data_dir: &Path,
    ledger: &Ledger,
    session: &str,
    head_hash: &str,
    events: &[Event],
    include_all_runs: bool,
) -> Result<Vec<u8>, String> {
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    let nb_bytes = build_solve_notebook_bytes(
        ledger,
        head_hash,
        session,
        events,
        NotebookOptions { include_all_runs },
    )?;
    entries.insert("notebook.ipynb".into(), nb_bytes);

    let ledger_doc = json!({
        "session": session,
        "head_hash": head_hash,
        "mode": "solve",
        "events": events.iter().map(|e| json!({
            "seq": e.seq,
            "ts_ms": e.ts_ms,
            "kind": e.kind,
            "schema_version": e.schema_version,
            "payload": e.payload,
            "prev_hash": e.prev_hash,
            "event_hash": e.event_hash,
        })).collect::<Vec<Value>>(),
    });
    let mut ledger_bytes = serde_json::to_vec_pretty(&ledger_doc).map_err(|e| e.to_string())?;
    ledger_bytes.push(b'\n');
    entries.insert("ledger.json".into(), ledger_bytes);

    let (sympy_calls, solve_checks) = collect_solve_replay_items(ledger, events)?;
    let workspace_files = collect_workspace_files(ledger, events);

    let mut worker_indexes = BTreeMap::new();
    let mut workers: Vec<Vec<(SolveSympyCall, String, String)>> = Vec::new();
    for call in sympy_calls {
        let rel = format!("code/step_{}.py", call.seq);
        let code_sha = sha256_hex(call.code.as_bytes());
        entries.insert(rel.clone(), call.code.as_bytes().to_vec());
        let index = *worker_indexes.entry(worker_key(&call.worker)).or_insert_with(|| {
            workers.push(Vec::new());
            workers.len() - 1
        });
        workers[index].push((call, rel, code_sha));
    }

    let source_files = collect_source_manifest(data_dir, session, events, &mut entries)?;
    for steps in &mut workers {
        steps.sort_by_key(|(call, _, _)| call.execution_order);
    }
    let replay_workers: Vec<Vec<String>> = workers
        .iter()
        .map(|steps| steps.iter().map(|(_, path, _)| path.clone()).collect())
        .collect();
    entries.insert(
        "_re_search_runtime/replay_steps.json".into(),
        serde_json::to_vec_pretty(&replay_workers).map_err(|e| e.to_string())?,
    );
    let runtime_files = insert_runtime_files(&mut entries);
    entries.insert(
        "REPRODUCE.md".into(),
        build_solve_reproduce_md(session, head_hash, replay_workers.iter().map(Vec::len).sum(), solve_checks.len()).into_bytes(),
    );
    let document_files: Vec<Value> = [
        "notebook.ipynb",
        "ledger.json",
        "REPRODUCE.md",
        "_re_search_runtime/replay_steps.json",
    ]
    .into_iter()
    .map(|path| json!({"path": path, "sha256": sha256_hex(&entries[path])}))
    .collect();

    let manifest = json!({
        "schema_version": 1,
        "mode": "solve",
        "session": session,
        "ledger_head_sha256": head_hash,
        "turns": recorded_turns(events),
        "source_files": source_files,
        "workspace_files": workspace_files,
        "runtime_files": runtime_files,
        "document_files": document_files,
        "workers": workers.iter().map(|steps| {
            let lifetime = match &steps[0].0.worker {
                ReplayWorker::LegacyLeg(seq) => json!({"kind": "inferred_session_leg", "start_seq": seq}),
                ReplayWorker::Recorded { instance, epoch } => json!({"kind": "recorded", "instance": instance, "epoch": epoch}),
            };
            json!({
                "lifetime": lifetime,
                "steps": steps.iter().map(|(call, path, sha256)| {
                    let mut step = json!({
                        "path": path,
                        "sha256": sha256,
                        "request_seq": call.seq,
                        "result_seq": call.result_seq,
                        "call_id": call.call_id,
                        "worker_environment": worker_environment(&call.worker_environment),
                        "recorded_result": call.result,
                    });
                    if !call.exprs_in.is_empty() {
                        step["exprs_in"] = json!(call.exprs_in);
                    }
                    step
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "solve_checks": solve_checks,
    });
    entries.insert(
        "replay_manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    );

    write_zip_entries(entries)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum WorkerKey {
    Legacy(u64),
    Recorded(String, u64),
}

fn worker_key(w: &ReplayWorker) -> WorkerKey {
    match w {
        ReplayWorker::LegacyLeg(seq) => WorkerKey::Legacy(*seq),
        ReplayWorker::Recorded { instance, epoch } => WorkerKey::Recorded(instance.clone(), *epoch),
    }
}

struct PendingCall {
    seq: u64,
    code: String,
    exprs_in: BTreeMap<String, String>,
    leg_seq: u64,
}

fn collect_solve_replay_items(
    ledger: &Ledger,
    events: &[Event],
) -> Result<(Vec<SolveSympyCall>, Vec<Value>), String> {
    let mut inc_state = SolveState::new();
    let mut pending: BTreeMap<String, PendingCall> = BTreeMap::new();
    let mut calls = Vec::new();
    let mut checks = Vec::new();
    let mut leg_seq = 0u64;

    for ev in events {
        if matches!(ev.kind.as_str(), "session.start" | "session.resume") {
            leg_seq = ev.seq;
        }
        match ev.kind.as_str() {
            event_kinds::TOOL_REQUEST
                if ev.payload.get("tool").and_then(Value::as_str) == Some("sympy_eval") =>
            {
                if let Some(call_id) = ev.payload.get("call_id").and_then(Value::as_str) {
                    let raw = resolve_slot_text(ledger, ev.payload.get("args"));
                    if let Ok(args) = serde_json::from_str::<Value>(&raw) {
                        if let Some(code) = args["code"].as_str().filter(|s| !s.trim().is_empty()) {
                            let mut exprs_in = stage_exprs_in_from_state(ledger, &inc_state)?;
                            if let Some(explicit) = args.get("exprs_in").and_then(Value::as_object) {
                                for (k, v) in explicit {
                                    if let Some(s) = v.as_str() {
                                        exprs_in.insert(k.clone(), s.to_string());
                                    }
                                }
                            }
                            pending.insert(
                                call_id.to_string(),
                                PendingCall {
                                    seq: ev.seq,
                                    code: code.to_string(),
                                    exprs_in,
                                    leg_seq,
                                },
                            );
                        }
                    }
                }
            }
            "tool.denied" => {
                if let Some(cid) = ev.payload.get("call_id").and_then(Value::as_str) {
                    pending.remove(cid);
                }
            }
            event_kinds::TOOL_RESULT => {
                if let Some(cid) = ev.payload.get("call_id").and_then(Value::as_str) {
                    if let Some(req) = pending.remove(cid) {
                        if ev.payload.get("ok").and_then(Value::as_bool) == Some(true) {
                            let raw = resolve_slot_text(ledger, ev.payload.get("result"));
                            if let Ok(result) = serde_json::from_str::<Value>(&raw) {
                                if result.is_object() && result["error"].is_null() {
                                    let exec = &ev.payload["worker_execution"];
                                    let (worker, execution_order) = match (
                                        exec["instance"].as_str(),
                                        exec["epoch"].as_u64(),
                                        exec["request_id"].as_u64(),
                                    ) {
                                        (Some(inst), Some(ep), Some(rid)) => (
                                            ReplayWorker::Recorded {
                                                instance: inst.to_string(),
                                                epoch: ep,
                                            },
                                            rid,
                                        ),
                                        _ => (ReplayWorker::LegacyLeg(req.leg_seq), ev.seq),
                                    };
                                    calls.push(SolveSympyCall {
                                        seq: req.seq,
                                        result_seq: ev.seq,
                                        call_id: cid.to_string(),
                                        code: req.code,
                                        exprs_in: req.exprs_in,
                                        result,
                                        worker,
                                        worker_environment: exec["environment"].clone(),
                                        execution_order,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            event_kinds::SOLVE_CHECK => {
                if let Ok(chk) = serde_json::from_value::<SolveCheck>(ev.payload.clone()) {
                    checks.push(build_solve_check_manifest(ledger, &inc_state, &chk)?);
                }
            }
            _ => {}
        }
        inc_state.apply(ev);
    }
    calls.sort_by_key(|c| c.seq);
    Ok((calls, checks))
}

fn build_solve_check_manifest(
    ledger: &Ledger,
    state: &SolveState,
    chk: &SolveCheck,
) -> Result<Value, String> {
    let entry = chk
        .entry_id
        .as_deref()
        .and_then(|id| state.entry(id))
        .or_else(|| {
            state
                .entries
                .iter()
                .find(|e| e.claim_sha256.as_deref() == Some(chk.claim_sha256.as_str()))
        });
    let claim = entry.and_then(|e| e.claims.get(chk.claim_index));

    let mut handles_map = serde_json::Map::new();
    let mut handles_sha_map = serde_json::Map::new();
    let mut code_hashes = serde_json::Map::new();

    let mut full_params = chk.params.as_object().cloned().unwrap_or_default();
    if let Some(setup) = &state.setup {
        full_params.insert("symbols".into(), json!(setup.symbols));
        full_params.insert("dimensions".into(), json!(setup.dimensions));
    }
    if let Some(c) = claim {
        full_params
            .entry("relation".to_string())
            .or_insert_with(|| json!(c.relation));
        if let Some(domain) = &c.domain {
            full_params
                .entry("domain".to_string())
                .or_insert_with(|| json!(domain));
        }
        if let Some(order) = &c.order {
            full_params
                .entry("order".to_string())
                .or_insert_with(|| json!(order));
        }
        for (side, href, code_key) in [
            ("lhs", &c.lhs, "lhs_code_sha256"),
            ("rhs", &c.rhs, "rhs_code_sha256"),
        ] {
            if let Some(expr) = state.handles.get(href) {
                if let Some(srepr) = load_handle_srepr(ledger, expr)? {
                    handles_map.insert(side.into(), json!(srepr));
                    handles_sha_map.insert(side.into(), json!(expr.srepr_sha256));
                }
                if let Some(code_sha) = state
                    .receipts
                    .get(&expr.produced_by.call_id)
                    .and_then(|r| r.code_sha256.as_ref())
                {
                    code_hashes.insert(code_key.into(), json!(code_sha));
                }
            }
        }
    }

    Ok(json!({
        "check_id": chk.check_id,
        "entry_id": chk.entry_id,
        "claim_index": chk.claim_index,
        "claim_sha256": chk.claim_sha256,
        "math_sha256": chk.math_sha256,
        "name": chk.name,
        "registry_sha256": chk.registry_sha256,
        "params": Value::Object(full_params),
        "handles": Value::Object(handles_map),
        "handles_sha256": Value::Object(handles_sha_map),
        "code_hashes": Value::Object(code_hashes),
        "outcome": chk.outcome,
        "evidence_sha256": chk.evidence_sha256,
        "recorded_evidence": chk.evidence,
    }))
}

fn collect_workspace_files(ledger: &Ledger, events: &[Event]) -> Vec<Value> {
    let mut pending = BTreeMap::new();
    let mut reads = Vec::new();
    for ev in events {
        let Some(cid) = ev.payload.get("call_id").and_then(Value::as_str) else {
            continue;
        };
        match ev.kind.as_str() {
            event_kinds::TOOL_REQUEST if ev.payload["tool"] == "fs_read" => {
                pending.insert(cid.to_string(), ev.seq);
            }
            "tool.denied" => {
                pending.remove(cid);
            }
            event_kinds::TOOL_RESULT => {
                let Some(request_seq) = pending.remove(cid) else { continue };
                if ev.payload["ok"] != true {
                    continue;
                }
                let raw = resolve_slot_text(ledger, ev.payload.get("result"));
                let Ok(res) = serde_json::from_str::<Value>(&raw) else { continue };
                if res["root"] != "workspace" {
                    continue;
                }
                let Some(path) = res["path"].as_str() else { continue };
                let Ok((_, path)) = sanitize_rel_components(path, false) else { continue };
                let Some(hash) = res["file_sha256"].as_str().filter(|h| {
                    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                }) else {
                    continue;
                };
                reads.push(json!({
                    "path": path,
                    "sha256": hash,
                    "request_seq": request_seq,
                    "result_seq": ev.seq,
                }));
            }
            _ => {}
        }
    }
    reads
}

fn build_solve_reproduce_md(
    session: &str,
    head_hash: &str,
    step_count: usize,
    check_count: usize,
) -> String {
    format!(
        "# Solve Reproducibility Bundle: `{session}`\n\n\
         - **Chain Head SHA-256:** `{head_hash}`\n\
         - **Extracted SymPy Receipts:** {step_count}\n\
         - **Recorded Host Checks:** {check_count}\n\n\
         ## Reproduction Steps\n\n\
         1. Replay worker calculations and re-run host checks in `_re_search_runtime/checks` \
         (comparing `evidence_sha256` against recorded outcomes per rule R19):\n   ```bash\n   python3 run_all_checks.py\n   ```\n\
         2. Run `python3 run_all_checks.py --verify-only` to check archived file and handle digests without executing Python calculations.\n\
         3. Open `notebook.ipynb` with Jupyter or execute its cells sequentially with plain Python + SymPy; the ledger log is in `ledger.json`.\n"
    )
}
