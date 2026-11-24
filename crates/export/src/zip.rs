//! Reproducible ZIP bundle export (F15): `report.tex`, `references.bib`,
//! `report.html`, `ledger.json`, `code/step_<seq>.py`, `sources/*`,
//! `run_all_checks.py`, and `REPRODUCE.md`.

use crate::html::render_html_document;
use crate::latex::{
    read_source_file, recorded_sources, render_latex_document, ReplayWorker, SessionExportData,
    SympyCall,
};
use ledger::{sha256_hex, Event};
use research_tools::{sandbox::sanitize_rel_components, SandboxRoot, WorkspaceSandbox};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

fn fixed_zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
}

pub(crate) fn build_reproducible_zip(
    data_dir: &Path,
    data: &SessionExportData,
) -> Result<Vec<u8>, String> {
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    let tex = render_latex_document(data, "references");
    let html = render_html_document(data);
    entries.insert("report.tex".into(), tex.into_bytes());
    entries.insert("references.bib".into(), data.bibtex.as_bytes().to_vec());
    entries.insert("report.html".into(), html.into_bytes());

    let ledger_doc = json!({
        "session": data.session,
        "head_hash": data.head_hash,
        "model": data.model,
        "policy_version": data.policy_version,
        "events": data.events.iter().map(|e| json!({
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

    let mut worker_indexes = BTreeMap::new();
    let mut workers: Vec<Vec<(&SympyCall, String, String)>> = Vec::new();
    for call in &data.sympy_calls {
        let rel = format!("code/step_{}.py", call.seq);
        let code = call.code.as_bytes();
        entries.insert(rel.clone(), code.to_vec());
        let index = *worker_indexes.entry(&call.worker).or_insert_with(|| {
            workers.push(Vec::new());
            workers.len() - 1
        });
        workers[index].push((call, rel, sha256_hex(code)));
    }

    let source_files =
        collect_source_manifest(data_dir, &data.session, &data.events, &mut entries)?;
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
    let runtime: [(&str, &[u8]); 5] = [
        ("run_all_checks.py", include_bytes!("replay_checks.py")),
        (
            "_re_search_runtime/worker.py",
            include_bytes!("../../../python/worker.py"),
        ),
        (
            "_re_search_runtime/solver_calc.py",
            include_bytes!("../../../python/solver_calc.py"),
        ),
        (
            "_re_search_runtime/symbolic_validation.py",
            include_bytes!("../../../python/symbolic_validation.py"),
        ),
        (
            "_re_search_runtime/numerical_validation.py",
            include_bytes!("../../../python/numerical_validation.py"),
        ),
    ];
    let runtime_files: Vec<_> = runtime
        .iter()
        .map(|(path, bytes)| {
            entries.insert((*path).into(), bytes.to_vec());
            json!({"path": path, "sha256": sha256_hex(bytes)})
        })
        .collect();
    entries.insert("REPRODUCE.md".into(), build_reproduce_md(data).into_bytes());
    let document_files: Vec<_> = [
        "report.tex",
        "report.html",
        "references.bib",
        "ledger.json",
        "REPRODUCE.md",
        "_re_search_runtime/replay_steps.json",
    ]
    .into_iter()
    .map(|path| json!({"path": path, "sha256": sha256_hex(&entries[path])}))
    .collect();
    let manifest = json!({
        "schema_version": 1,
        "session": data.session,
        "ledger_head_sha256": data.head_hash,
        "turns": recorded_turns(&data.events),
        "source_files": source_files,
        "workspace_files": data.workspace_files,
        "runtime_files": runtime_files,
        "document_files": document_files,
        "workers": workers.iter().map(|steps| {
            let lifetime = match &steps[0].0.worker {
                ReplayWorker::LegacyLeg(seq) => json!({"kind": "inferred_session_leg", "start_seq": seq}),
                ReplayWorker::Recorded { instance, epoch } => json!({"kind": "recorded", "instance": instance, "epoch": epoch}),
            };
            json!({
                "lifetime": lifetime,
                "steps": steps.iter().map(|(call, path, sha256)| json!({
                    "path": path,
                    "sha256": sha256,
                    "request_seq": call.seq,
                    "result_seq": call.result_seq,
                    "call_id": call.call_id,
                    "worker_environment": worker_environment(&call.worker_environment),
                    "recorded_result": call.result,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    });
    entries.insert(
        "replay_manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    );

    let buf = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(buf);
    let opts = fixed_zip_options();
    for (name, bytes) in entries {
        writer
            .start_file(&name, opts)
            .map_err(|e| format!("zip start_file `{name}` failed: {e}"))?;
        writer
            .write_all(&bytes)
            .map_err(|e| format!("zip write `{name}` failed: {e}"))?;
    }
    let cursor = writer
        .finish()
        .map_err(|e| format!("zip finish failed: {e}"))?;
    Ok(cursor.into_inner())
}

fn worker_environment(raw: &Value) -> Value {
    let mut env = serde_json::Map::new();
    if let Some(protocol) = raw["protocol"].as_u64() {
        env.insert("protocol".into(), json!(protocol));
    }
    for field in [
        "python",
        "sympy",
        "numpy",
        "mpmath",
        "platform",
        "worker_sha256",
        "solver_calc_sha256",
        "symbolic_validation_sha256",
        "numerical_validation_sha256",
    ] {
        if let Some(value) = raw[field]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
        {
            env.insert(field.into(), json!(value));
        }
    }
    if env.is_empty() {
        Value::Null
    } else {
        Value::Object(env)
    }
}

fn recorded_turns(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter(|event| matches!(event.kind.as_str(), "session.start" | "session.resume"))
        .map(|event| {
            let revision = &event.payload["workspace_revision"];
            let revision = revision["git_commit"]
                .as_str()
                .filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(|commit| json!({"git_commit": commit, "dirty": revision["dirty"].as_bool()}));
            json!({
                "seq": event.seq,
                "kind": event.kind,
                "route": event.payload["route"].as_str(),
                "model": event.payload["model"].as_str(),
                "policy_version": event.payload["policy_version"].as_str(),
                "workspace_revision": revision,
                "cli_version": event.payload["cli_version_detected"].as_str(),
                "tool_definitions": event.payload["tool_defs"].as_array().map(|tools| {
                    tools.iter().filter_map(|tool| Some(json!({
                        "id": tool["id"].as_str()?,
                        "sha256": tool["def_hash"].as_str()?,
                    }))).collect::<Vec<_>>()
                }),
            })
        })
        .collect()
}

fn collect_source_manifest(
    data_dir: &Path,
    session: &str,
    events: &[Event],
    entries: &mut BTreeMap<String, Vec<u8>>,
) -> Result<Vec<Value>, String> {
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let root = sandbox
        .canonical_root(SandboxRoot::Sources, session, false)
        .map_err(|e| e.to_string())?;
    let by_path = recorded_sources(events)?;

    let Some(root) = root else {
        return Ok(by_path.into_iter().map(|(path, revisions)| {
            json!({"path": format!("sources/{path}"), "status": "unavailable", "revisions": revisions})
        }).collect());
    };
    if by_path.is_empty() {
        // Old sessions can predate ingestion receipts. Keep their materialized sources and
        // distinguish the export-time hashes from hashes captured during ingestion.
        collect_sources_into_map(&root, &root, entries)?;
        return Ok(entries.iter().filter(|(path, _)| path.starts_with("sources/")).map(|(path, bytes)| {
            json!({"path": path, "status": "unrecorded", "bytes": bytes.len(), "sha256": sha256_hex(bytes), "revisions": []})
        }).collect());
    }

    Ok(by_path.into_iter().map(|(path, revisions)| {
        let name = format!("sources/{path}");
        let Some(bytes) = read_source_file(&sandbox, session, &path) else {
            return json!({"path": name, "status": "unavailable", "revisions": revisions});
        };
        let sha256 = sha256_hex(&bytes);
        let expected = revisions.last().and_then(|latest| latest.recorded_sha256);
        if expected.is_some_and(|hash| hash != sha256) {
            return json!({"path": name, "status": "hash_mismatch", "available_sha256": sha256, "revisions": revisions});
        }
        let n = bytes.len();
        entries.insert(name.clone(), bytes);
        json!({
            "path": name,
            "status": if expected.is_some() { "matches_recorded" } else { "unrecorded" },
            "bytes": n,
            "sha256": sha256,
            "revisions": revisions,
        })
    }).collect())
}

fn collect_sources_into_map(
    base: &Path,
    cur: &Path,
    out: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let rd = fs::read_dir(cur).map_err(|e| e.to_string())?;
    let mut items: Vec<_> = rd.filter_map(Result::ok).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if research_tools::is_symlink_or_junction(&path, &meta) {
            continue;
        }
        if meta.is_dir() {
            collect_sources_into_map(base, &path, out)?;
        } else if meta.is_file() {
            let relative_path = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy();
            let (_, rel) =
                sanitize_rel_components(&relative_path, false).map_err(|e| e.to_string())?;
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            out.insert(format!("sources/{rel}"), bytes);
        }
    }
    Ok(())
}

fn build_reproduce_md(data: &SessionExportData) -> String {
    format!(
        "# Reproducibility Bundle: `{}`\n\n\
         - **Chain Head SHA-256**: `{}`\n\
         - **Model**: `{}`\n\
         - **Policy Version**: `{}`\n\
         - **Extracted Checks**: {}\n\n\
         ## Verification Steps\n\n\
         1. Replay successful computations from a trusted bundle with Python and SymPy installed. \
         The runner checks archived hashes before executing the code locally, shares a Python namespace within each recorded worker lifetime, \
         and compares supported values exactly or compares complete recorded stdout for print-only calculations. \
         It reports steps without comparable recorded output and differences in recorded runtime versions. \
         The worker records explicit seeds for Python random and NumPy's legacy generator when calculations call `set_random_seed(seed)`. \
         Other generators and seeding methods are outside that record. \
         Older ledgers group checks by session start or resume because they lack worker restart records. \
         Later steps that relied on side effects of a failed calculation may need to be rewritten.\n   ```bash\n   python3 run_all_checks.py\n   ```\n\
         2. Run `python3 run_all_checks.py --verify-only` to check archived file hashes without evaluating calculations. \
         Inspect source revisions, turn and worker provenance, and recorded results in `replay_manifest.json`. \
         Missing and changed sources are listed and omitted. For sources without recorded hashes, the listed digests describe the exported files. \
         Git revisions identify recorded commits and include a dirty flag when captured. \
         The working tree is outside the bundle. Its state during earlier calculations may be unknown. \
         Workspace file digests describe the bytes observed by recorded file reads. The tool output covers only the returned text or matches. \
         The session audit log is in `ledger.json`.\n\
         3. Compile `report.tex` (with `references.bib`) or open `report.html`.\n\n\
         The runner executes Python calculations using the local interpreter's privileges. \
         Provider calls, network retrieval, Lean, and LaTeX compilation are outside replay.\n",
        data.session,
        data.head_hash,
        data.model,
        data.policy_version,
        data.sympy_calls.len()
    )
}
